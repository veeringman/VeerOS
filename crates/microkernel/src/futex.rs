//! Kernel futex (fast userspace mutex) implementation.
//!
//! A futex is a synchronization primitive where most operations happen in
//! userspace (via atomic compare-and-swap), and the kernel is only invoked
//! for the slow path: waiting and waking.
//!
//! # Design
//!
//! - The kernel maintains a fixed-size table of wait queues, keyed by the
//!   address of the futex word in user memory.
//! - `SYS_FUTEX_WAIT(addr, expected)` atomically checks `*addr == expected`,
//!   and if so, blocks the calling task.
//! - `SYS_FUTEX_WAKE(addr, count)` wakes up to `count` tasks waiting on `addr`.
//!
//! Because we run in M-mode (no MMU), addresses are physical and unique.

use crate::task::{BlockReason, Scheduler, TaskState, MAX_TASKS};

/// Maximum number of distinct futex addresses that can have active waiters.
const MAX_FUTEX_WAITERS: usize = 32;

/// A single entry in the futex wait table.
#[derive(Debug, Clone, Copy)]
struct FutexWaiter {
    /// The address being waited on (0 = slot free).
    addr: usize,
    /// Task ID of the waiter.
    task_id: usize,
}

impl FutexWaiter {
    const fn empty() -> Self {
        Self { addr: 0, task_id: usize::MAX }
    }
}

/// Kernel futex subsystem — manages wait queues keyed by address.
pub struct FutexTable {
    waiters: [FutexWaiter; MAX_FUTEX_WAITERS],
}

impl FutexTable {
    pub const fn new() -> Self {
        Self {
            waiters: [FutexWaiter::empty(); MAX_FUTEX_WAITERS],
        }
    }

    /// Atomically check `*addr == expected` and block the task if true.
    ///
    /// Returns `true` if the task was blocked, `false` if `*addr != expected`
    /// (spurious wakeup avoidance — caller should retry).
    ///
    /// # Safety
    /// `addr` must be a valid, aligned pointer to a `usize` in task memory.
    pub unsafe fn wait(
        &mut self,
        sched: &mut Scheduler,
        task_id: usize,
        addr: usize,
        expected: usize,
    ) -> bool {
        // Atomic check: read the futex word and compare.
        // In M-mode with interrupts disabled during syscall, this is atomic.
        let actual = unsafe { (addr as *const usize).read_volatile() };
        if actual != expected {
            return false; // value changed — don't block
        }

        // Find a free waiter slot.
        for slot in self.waiters.iter_mut() {
            if slot.addr == 0 {
                slot.addr = addr;
                slot.task_id = task_id;
                // Block the task.
                if task_id < MAX_TASKS {
                    sched.tasks[task_id].state = TaskState::Blocked;
                    sched.tasks[task_id].block_reason = BlockReason::Futex;
                }

                // Priority inheritance: boost any existing waiters' holder.
                // Heuristic: find a non-blocked task that also recently
                // touched this futex address (simple: boost all active tasks
                // that have lower priority than us).
                let waiter_pri = if task_id < MAX_TASKS {
                    sched.tasks[task_id].priority
                } else {
                    0
                };
                // Scan other waiters on same address — if we find one that
                // previously was woken (now Ready/Running), it's holding the
                // lock. Alternatively, this is called from futex_wait so the
                // kernel doesn't know the holder. We'll use a lightweight
                // approach: the caller can pass a hint via the futex word
                // value. For now, boost any Running task with lower priority.
                // This is a best-effort heuristic for single-core.
                if waiter_pri > 0 {
                    let cur_running = sched.current;
                    if cur_running != task_id && cur_running < MAX_TASKS {
                        let holder = &mut sched.tasks[cur_running];
                        if holder.state == TaskState::Running && holder.priority < waiter_pri {
                            holder.priority = waiter_pri;
                        }
                    }
                }

                return true;
            }
        }

        // Table full — don't block (caller will spin).
        false
    }

    /// Wake up to `count` tasks waiting on `addr`.
    ///
    /// Returns the number of tasks actually woken.
    pub fn wake(&mut self, sched: &mut Scheduler, addr: usize, count: usize) -> usize {
        let mut woken = 0;
        for slot in self.waiters.iter_mut() {
            if woken >= count {
                break;
            }
            if slot.addr == addr {
                let tid = slot.task_id;
                if tid < MAX_TASKS && sched.tasks[tid].state == TaskState::Blocked
                    && sched.tasks[tid].block_reason == BlockReason::Futex
                {
                    sched.tasks[tid].state = TaskState::Ready;
                    sched.tasks[tid].block_reason = BlockReason::None;
                    // Restore base priority (undo any inheritance boost).
                    sched.tasks[tid].priority = sched.tasks[tid].base_priority;
                    woken += 1;
                }
                // Free the slot regardless.
                *slot = FutexWaiter::empty();
            }
        }
        woken
    }
}

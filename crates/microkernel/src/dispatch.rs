//! Kernel-side syscall dispatcher.
//!
//! Called from the trap handler when `mcause` indicates an environment call
//! (ecall). Reads the syscall number from `a7` (x17) in the saved context,
//! dispatches to the appropriate handler, and writes return values back
//! into `a0`/`a1` of the saved context.
//!
//! The dispatcher receives mutable references to all kernel subsystems so
//! it can service any syscall without global state lookups.

use arch::TaskContext;
use crate::ipc::{Ipc, Message};
use crate::task::Scheduler;
use crate::alloc::Heap;
use crate::syscall::*;

/// Result of a syscall dispatch — tells the trap handler what to do next.
pub enum SyscallAction {
    /// Return to the same task (normal case).
    Resume,
    /// The task yielded or was blocked — a context switch is needed.
    Reschedule,
    /// The task exited — pick a new task.
    TaskExited,
}

/// Dispatch a syscall from the saved trap context.
///
/// # Arguments
/// - `ctx` — pointer to the saved `TaskContext` on the trap stack
/// - `sched` — the kernel scheduler
/// - `ipc` — the IPC mailbox array
/// - `heap` — the kernel heap (for SYS_ALLOC / SYS_FREE)
/// - `console_write` — callback to write a byte to the console
/// - `console_read` — callback to read a byte from the console
///
/// # Safety
/// `ctx` must be a valid pointer to a `TaskContext`. The caller must hold
/// exclusive access to `sched`, `ipc`, and `heap`.
pub unsafe fn dispatch(
    ctx: *mut TaskContext,
    sched: &mut Scheduler,
    ipc: &mut Ipc,
    heap: &mut Heap,
    console_write: fn(u8),
    console_read: fn() -> u8,
) -> SyscallAction {
    let c = unsafe { &mut *ctx };

    // Skip the ecall instruction (4 bytes) so we return to the next one.
    c.pc += 4;

    // RISC-V register mapping:
    //   a7 = x17 = gpr[17] — syscall number
    //   a0 = x10 = gpr[10] — arg0 / ret0
    //   a1 = x11 = gpr[11] — arg1 / ret1
    //   a2 = x12 = gpr[12] — arg2
    //   a3 = x13 = gpr[13] — arg3
    let nr = c.gpr[17];
    let a0 = c.gpr[10];
    let a1 = c.gpr[11];
    let a2 = c.gpr[12];
    let a3 = c.gpr[13];

    match nr {
        // ── Task control ────────────────────────────────────────
        SYS_YIELD => {
            // Mark current task Ready so the scheduler picks something else.
            let cur = sched.current;
            if cur < sched.tasks.len() {
                use crate::task::TaskState;
                if sched.tasks[cur].state == TaskState::Running {
                    sched.tasks[cur].state = TaskState::Ready;
                }
            }
            SyscallAction::Reschedule
        }

        SYS_EXIT => {
            let cur = sched.current;
            if cur < sched.tasks.len() {
                use crate::task::TaskState;
                sched.tasks[cur].state = TaskState::Free;
            }
            let _exit_code = a0;
            SyscallAction::TaskExited
        }

        SYS_TASK_ID => {
            c.gpr[10] = sched.current;
            SyscallAction::Resume
        }

        SYS_TASK_PRIORITY => {
            let cur = sched.current;
            c.gpr[10] = if cur < sched.tasks.len() {
                sched.tasks[cur].priority as usize
            } else {
                0
            };
            SyscallAction::Resume
        }

        SYS_TASK_COUNT => {
            use crate::task::TaskState;
            let count = sched.tasks.iter()
                .filter(|t| t.state != TaskState::Free)
                .count();
            c.gpr[10] = count;
            SyscallAction::Resume
        }

        // ── IPC ─────────────────────────────────────────────────
        SYS_IPC_SEND => {
            let dest = a0 as u8;
            let opcode = a1 as u8;
            let msg = Message {
                sender: 0, // filled by ipc.send()
                opcode,
                arg0: a2,
                arg1: a3,
            };
            let from = sched.current as u8;
            let ok = ipc.send(sched, from, dest, msg);
            c.gpr[10] = ok as usize;
            SyscallAction::Resume
        }

        SYS_IPC_RECV => {
            let tid = sched.current as u8;
            match ipc.recv(sched, tid) {
                Some(msg) => {
                    // Pack sender + opcode into a0, arg0 into a1.
                    c.gpr[10] = (msg.sender as usize) | ((msg.opcode as usize) << 8);
                    c.gpr[11] = msg.arg0;
                    SyscallAction::Resume
                }
                None => {
                    // Task was blocked by ipc.recv() — need to reschedule.
                    // When the task is unblocked by a send(), the scheduler
                    // will resume it. We need to re-execute the ecall so
                    // the recv sees the message, so rewind pc.
                    c.pc -= 4; // re-execute the ecall on wakeup
                    sched.save_current_context(c);
                    SyscallAction::Reschedule
                }
            }
        }

        SYS_IPC_POLL => {
            let tid = sched.current as u8;
            c.gpr[10] = ipc.has_message(tid) as usize;
            SyscallAction::Resume
        }

        // ── I/O / Console ───────────────────────────────────────
        SYS_WRITE_BYTE => {
            console_write(a0 as u8);
            SyscallAction::Resume
        }

        SYS_WRITE_BUF => {
            let ptr = a0 as *const u8;
            let len = a1;
            // Safety: the user task is responsible for passing a valid
            // pointer within its own memory. On M-mode-only systems there
            // is no MMU to enforce this.
            for i in 0..len {
                let byte = unsafe { ptr.add(i).read() };
                console_write(byte);
            }
            SyscallAction::Resume
        }

        SYS_READ_BYTE => {
            let byte = console_read();
            c.gpr[10] = byte as usize;
            SyscallAction::Resume
        }

        // ── Time ────────────────────────────────────────────────
        SYS_TICK => {
            let ticks = sched.ticks;
            c.gpr[10] = ticks as usize;           // low 32 bits
            c.gpr[11] = (ticks >> 32) as usize;   // high 32 bits
            SyscallAction::Resume
        }

        SYS_SLEEP => {
            // Simple sleep: record the wake-up tick, block the task.
            // The tick handler will unblock it when ticks >= target.
            let cur = sched.current;
            if cur < sched.tasks.len() {
                use crate::task::TaskState;
                sched.tasks[cur].state = TaskState::Blocked;
                // Store the target tick in the context's unused gpr[0]
                // (x0 is hardwired to zero in RISC-V, so gpr[0] is free
                // for kernel bookkeeping in the saved context).
                let target = sched.ticks.saturating_add(a0 as u64);
                c.gpr[0] = target as usize;
            }
            sched.save_current_context(c);
            SyscallAction::Reschedule
        }

        // ── Memory ──────────────────────────────────────────────
        SYS_ALLOC => {
            let size = a0;
            match heap.alloc(size) {
                Ok(ptr) => c.gpr[10] = ptr as usize,
                Err(_) => c.gpr[10] = 0,
            }
            SyscallAction::Resume
        }

        SYS_FREE => {
            let ptr = a0 as *mut u8;
            let size = a1;
            if !ptr.is_null() {
                unsafe { heap.free(ptr, size) };
            }
            SyscallAction::Resume
        }

        // ── Debug ───────────────────────────────────────────────
        SYS_PANIC => {
            let ptr = a0 as *const u8;
            let len = a1;
            // Print the panic message to console.
            for b in b"PANIC(user): " {
                console_write(*b);
            }
            for i in 0..len {
                let byte = unsafe { ptr.add(i).read() };
                console_write(byte);
            }
            console_write(b'\r');
            console_write(b'\n');
            // Kill the task.
            let cur = sched.current;
            if cur < sched.tasks.len() {
                use crate::task::TaskState;
                sched.tasks[cur].state = TaskState::Free;
            }
            SyscallAction::TaskExited
        }

        SYS_PLATFORM_NAME => {
            // Returns a pointer + length. The string lives in .rodata
            // so it's safe for the user to read.
            c.gpr[10] = 0; // not easily available without platform ref
            c.gpr[11] = 0;
            SyscallAction::Resume
        }

        // ── Unknown ─────────────────────────────────────────────
        _ => {
            // Unknown syscall — return -1 (usize::MAX) in a0.
            c.gpr[10] = usize::MAX;
            SyscallAction::Resume
        }
    }
}

/// Check sleeping tasks and wake any whose target tick has been reached.
///
/// Call this from the timer tick handler, after incrementing `sched.ticks`.
pub fn wake_sleepers(sched: &mut Scheduler) {
    use crate::task::TaskState;
    let now = sched.ticks;
    for task in sched.tasks.iter_mut() {
        if task.state == TaskState::Blocked {
            // gpr[0] holds the wakeup tick (set by SYS_SLEEP).
            // If it's 0, the task is blocked on IPC, not sleep.
            let target = task.context.gpr[0] as u64;
            if target > 0 && now >= target {
                task.context.gpr[0] = 0; // clear the sleep marker
                task.state = TaskState::Ready;
            }
        }
    }
}

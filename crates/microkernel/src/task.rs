//! Minimal cooperative/preemptive task scheduler.
//!
//! This module provides:
//! - A fixed-size task table (TCBs) held in static storage.
//! - Round-robin scheduling for the `Minimal` distribution.
//! - Priority-aware round-robin for the `RealTime` distribution.
//! - A `tick()` entry point the timer ISR calls each period.

use arch::{MemPerms, SavedContext, TaskContext, TaskMemRegion, TaskRegions, MAX_TASK_REGIONS};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum number of concurrent tasks.
/// 64-bit architectures have more address space and RAM, so we allow more tasks.
#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
pub const MAX_TASKS: usize = 64;

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
pub const MAX_TASKS: usize = 16;

/// Default stack size per task (bytes). Boards can override at link time.
pub const DEFAULT_STACK_SIZE: usize = 2048;

/// Stack guard size in bytes.  PMP/MPU will deny access to this region
/// below the stack, trapping stack overflow (enforced in U-mode).
pub const STACK_GUARD_SIZE: usize = 64;

// ---------------------------------------------------------------------------
// Task state
// ---------------------------------------------------------------------------

/// Lifecycle state of a task slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// Slot is unused.
    Free,
    /// Task is ready to run.
    Ready,
    /// Task is the currently executing task.
    Running,
    /// Task is blocked (see [`BlockReason`] for why).
    Blocked,
    /// Task has been explicitly suspended (can be resumed).
    Suspended,
    /// Task has exited; exit status retained until collected.
    Zombie,
}

/// Why a task is blocked — stored alongside `TaskState::Blocked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// Not blocked (default / task is Free/Ready/Running).
    None,
    /// Sleeping until `wakeup_tick`.
    Sleep,
    /// Waiting for an IPC message.
    IpcRecv,
    /// Waiting for a child task to exit (`join_target` task ID).
    Join,
    /// Blocked on a futex (waiting for a `SYS_FUTEX_WAKE`).
    Futex,
    /// Blocked trying to send on a full channel.
    ChanSend(usize),
    /// Blocked trying to receive from an empty channel.
    ChanRecv(usize),
    /// Blocked waiting for a poll event set to fire (`SYS_POLL_WAIT`).
    PollWait,
    /// Blocked on `accept()` waiting for a connection.
    SockAccept(usize),
    /// Blocked on `recv()` waiting for data on a socket.
    SockRecv(usize),
    /// Blocked on `send()` waiting for space in peer's buffer.
    SockSend(usize),
    /// Blocked waiting for an IRQ to fire (userspace driver).
    /// The usize is the IRQ line the driver is waiting on.
    IrqWait(usize),
}

/// Task Control Block — one per thread slot.
///
/// Threads belong to a [`Process`](crate::process::Process) identified
/// by `process_id`.  Multiple threads can share the same process (and
/// therefore the same address-space / memory regions).
#[derive(Debug, Clone, Copy)]
pub struct Tcb {
    pub state: TaskState,
    pub priority: u8,
    /// Original priority before any inheritance boost.
    pub base_priority: u8,
    pub name: &'static str,
    pub context: TaskContext,
    /// Bottom of stack allocation (pointer kept for bookkeeping).
    pub stack_bottom: usize,
    /// Stack size in bytes.
    pub stack_size: usize,
    /// Why this task is blocked (meaningful only when `state == Blocked`).
    pub block_reason: BlockReason,
    /// Tick at which a sleeping task should wake (0 = not sleeping).
    pub wakeup_tick: u64,
    /// Parent task index (who spawned this task), or `usize::MAX` for root.
    pub parent: usize,
    /// Task index we are joining on, or `usize::MAX` if not joining.
    pub join_target: usize,
    /// Exit code set by `SYS_EXIT`, readable via `SYS_JOIN`.
    pub exit_code: usize,
    /// Per-thread memory regions (stack + guard).
    pub regions: TaskRegions,
    /// Number of valid entries in `regions`.
    pub region_count: usize,
    /// Index into the [`ProcessTable`](crate::process::ProcessTable).
    /// `usize::MAX` means "no process" (legacy / kernel-internal).
    pub process_id: usize,
    /// Thread-local storage base address (written to `tp`/x4 on RISC-V
    /// context switch).  0 means TLS is not configured.
    pub tls_base: usize,
}

impl Tcb {
    pub const fn empty() -> Self {
        Self {
            state: TaskState::Free,
            priority: 0,
            base_priority: 0,
            name: "",
            context: TaskContext::zero(),
            stack_bottom: 0,
            stack_size: 0,
            block_reason: BlockReason::None,
            wakeup_tick: 0,
            parent: usize::MAX,
            join_target: usize::MAX,
            exit_code: 0,
            regions: [TaskMemRegion::empty(); MAX_TASK_REGIONS],
            region_count: 0,
            process_id: usize::MAX,
            tls_base: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// Simple round-robin / priority scheduler operating over a fixed task table.
pub struct Scheduler {
    /// Task table (static, no heap).
    pub tasks: [Tcb; MAX_TASKS],
    /// Index of the currently running task (or `usize::MAX` if none).
    pub current: usize,
    /// Monotonically increasing tick counter.
    pub ticks: u64,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            tasks: [Tcb::empty(); MAX_TASKS],
            current: usize::MAX,
            ticks: 0,
        }
    }

    /// Register a new task (thread). Returns the task index, or `None` if the table is full.
    ///
    /// `entry` is the function pointer the task begins executing at.
    /// `stack_top` is the highest usable address of the task's stack.
    /// `process_id` links the thread to a process (`usize::MAX` for legacy/kernel tasks).
    ///
    /// Automatically grants per-thread stack RW region and stack guard.
    pub fn create_task(
        &mut self,
        name: &'static str,
        entry: usize,
        stack_top: usize,
        stack_bottom: usize,
        priority: u8,
        process_id: usize,
    ) -> Option<usize> {
        for (i, slot) in self.tasks.iter_mut().enumerate() {
            if slot.state == TaskState::Free {
                let mut ctx = TaskContext::zero();
                ctx.set_pc(entry);
                ctx.set_sp(stack_top);

                // Grant stack region (RW, no execute).
                let mut regions = [TaskMemRegion::empty(); MAX_TASK_REGIONS];
                regions[0] = TaskMemRegion {
                    base: stack_bottom,
                    size: stack_top - stack_bottom,
                    perms: MemPerms::RW,
                };
                // Stack guard: no-access region below the stack.
                // When PMP is enforced (U-mode), overflow into this
                // region triggers a trap instead of silent corruption.
                if stack_bottom >= STACK_GUARD_SIZE {
                    regions[1] = TaskMemRegion {
                        base: stack_bottom - STACK_GUARD_SIZE,
                        size: STACK_GUARD_SIZE,
                        perms: MemPerms::NONE,
                    };
                }

                *slot = Tcb {
                    state: TaskState::Ready,
                    priority,
                    base_priority: priority,
                    name,
                    context: ctx,
                    stack_bottom,
                    stack_size: stack_top - stack_bottom,
                    block_reason: BlockReason::None,
                    wakeup_tick: 0,
                    parent: usize::MAX,
                    join_target: usize::MAX,
                    exit_code: 0,
                    regions,
                    region_count: if stack_bottom >= STACK_GUARD_SIZE {
                        2
                    } else {
                        1
                    },
                    process_id,
                    tls_base: 0,
                };
                return Some(i);
            }
        }
        None
    }

    /// Grant an additional memory region to a task.
    /// Returns `true` on success, `false` if the region table is full.
    pub fn grant_region(&mut self, task_id: usize, region: TaskMemRegion) -> bool {
        if task_id >= MAX_TASKS {
            return false;
        }
        let tcb = &mut self.tasks[task_id];
        if tcb.region_count >= MAX_TASK_REGIONS {
            return false;
        }
        tcb.regions[tcb.region_count] = region;
        tcb.region_count += 1;
        true
    }

    /// Pick the first ready task, mark it Running, and return a pointer
    /// to its saved context.  The caller (arch-specific asm) loads this
    /// context and `mret`s into the task.
    pub fn start(&mut self) -> Option<*const TaskContext> {
        if let Some(idx) = self.pick_next() {
            self.current = idx;
            self.tasks[idx].state = TaskState::Running;
            Some(&self.tasks[idx].context as *const TaskContext)
        } else {
            None
        }
    }

    /// Pick the next task to run (round-robin among `Ready` tasks).
    /// Returns the index, or `None` if nothing is runnable.
    pub fn pick_next(&self) -> Option<usize> {
        let start = if self.current == usize::MAX {
            0
        } else {
            (self.current + 1) % MAX_TASKS
        };

        // --- priority scan (used when dist-rt is active) --------------------
        #[cfg(feature = "dist-rt")]
        {
            let mut best_idx: Option<usize> = None;
            let mut best_pri: Option<u8> = None;
            for offset in 0..MAX_TASKS {
                let idx = (start + offset) % MAX_TASKS;
                let t = &self.tasks[idx];
                if t.state == TaskState::Ready {
                    match best_pri {
                        None => {
                            best_pri = Some(t.priority);
                            best_idx = Some(idx);
                        }
                        Some(bp) if t.priority > bp => {
                            best_pri = Some(t.priority);
                            best_idx = Some(idx);
                        }
                        _ => {}
                    }
                }
            }
            return best_idx;
        }

        // --- simple round-robin (minimal / app) ----------------------------
        #[cfg(not(feature = "dist-rt"))]
        {
            for offset in 0..MAX_TASKS {
                let idx = (start + offset) % MAX_TASKS;
                if self.tasks[idx].state == TaskState::Ready {
                    return Some(idx);
                }
            }
            None
        }
    }

    /// Called from the timer ISR on every tick. Advances the tick counter and
    /// returns `true` when a context switch is needed.
    pub fn tick(&mut self) -> bool {
        self.ticks += 1;

        // Mark current task as Ready (yield its timeslice).
        if self.current < MAX_TASKS {
            if self.tasks[self.current].state == TaskState::Running {
                self.tasks[self.current].state = TaskState::Ready;
            }
        }

        // Try to pick a new task.
        if let Some(next) = self.pick_next() {
            if next != self.current {
                self.current = next;
                self.tasks[next].state = TaskState::Running;
                return true; // context switch needed
            }
            // Same task continues — mark it Running again.
            self.tasks[next].state = TaskState::Running;
        }

        false
    }

    /// Save the CPU context for the current task.
    pub fn save_current_context(&mut self, ctx: &TaskContext) {
        if self.current < MAX_TASKS {
            self.tasks[self.current].context = *ctx;
        }
    }

    /// Get an immutable reference to the current task's saved context
    /// (used to restore registers after a switch).
    pub fn current_context(&self) -> Option<&TaskContext> {
        if self.current < MAX_TASKS {
            Some(&self.tasks[self.current].context)
        } else {
            None
        }
    }

    /// Get a mutable reference to the current task's saved context
    /// (the trap handler writes the restore frame here).
    pub fn current_context_mut(&mut self) -> Option<&mut TaskContext> {
        if self.current < MAX_TASKS {
            Some(&mut self.tasks[self.current].context)
        } else {
            None
        }
    }
}

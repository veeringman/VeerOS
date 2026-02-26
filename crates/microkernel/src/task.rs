//! Minimal cooperative/preemptive task scheduler.
//!
//! This module provides:
//! - A fixed-size task table (TCBs) held in static storage.
//! - Round-robin scheduling for the `Minimal` distribution.
//! - Priority-aware round-robin for the `RealTime` distribution.
//! - A `tick()` entry point the timer ISR calls each period.

use arch::TaskContext;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum number of concurrent tasks.
pub const MAX_TASKS: usize = 8;

/// Default stack size per task (bytes). Boards can override at link time.
pub const DEFAULT_STACK_SIZE: usize = 2048;

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
    /// Task is blocked (waiting for IPC, timer, etc.).
    Blocked,
}

/// Task Control Block — one per task slot.
#[derive(Debug, Clone, Copy)]
pub struct Tcb {
    pub state: TaskState,
    pub priority: u8,
    pub name: &'static str,
    pub context: TaskContext,
    /// Bottom of stack allocation (pointer kept for bookkeeping).
    pub stack_bottom: usize,
    /// Stack size in bytes.
    pub stack_size: usize,
}

impl Tcb {
    pub const fn empty() -> Self {
        Self {
            state: TaskState::Free,
            priority: 0,
            name: "",
            context: TaskContext::zero(),
            stack_bottom: 0,
            stack_size: 0,
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

    /// Register a new task. Returns the task index, or `None` if the table is full.
    ///
    /// `entry` is the function pointer the task begins executing at.
    /// `stack_top` is the highest usable address of the task's stack.
    pub fn create_task(
        &mut self,
        name: &'static str,
        entry: usize,
        stack_top: usize,
        stack_bottom: usize,
        priority: u8,
    ) -> Option<usize> {
        for (i, slot) in self.tasks.iter_mut().enumerate() {
            if slot.state == TaskState::Free {
                let mut ctx = TaskContext::zero();
                ctx.pc = entry;
                // RISC-V: x2 = sp
                ctx.gpr[2] = stack_top;

                *slot = Tcb {
                    state: TaskState::Ready,
                    priority,
                    name,
                    context: ctx,
                    stack_bottom,
                    stack_size: stack_top - stack_bottom,
                };
                return Some(i);
            }
        }
        None
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
            let mut best_pri: u8 = 0;
            for offset in 0..MAX_TASKS {
                let idx = (start + offset) % MAX_TASKS;
                let t = &self.tasks[idx];
                if t.state == TaskState::Ready && t.priority > best_pri {
                    best_pri = t.priority;
                    best_idx = Some(idx);
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

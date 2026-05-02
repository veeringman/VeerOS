//! Poll subsystem — multiplexed event waiting for async support.
//!
//! Each task can register interest in a set of events via `SYS_POLL_SET`
//! and block until any fires via `SYS_POLL_WAIT`.  This is the kernel
//! building block for a userspace async executor.
//!
//! Events supported:
//! - `TIMER`: wakeup after N ticks
//! - `IPC`: IPC message pending for this task
//! - `CHAN_READABLE`: a specific channel has data
//! - `CHAN_WRITABLE`: a specific channel has space
//! - `TASK_EXIT`: a specific task has exited

use crate::channel::Channels;
use crate::ipc::Ipc;
use crate::syscall::*;
use crate::task::{BlockReason, Scheduler, TaskState, MAX_TASKS};
use arch::SavedContext;

/// Per-task poll registration.
#[derive(Debug, Clone, Copy)]
pub struct PollEntry {
    /// Bitmask of events this task is interested in.
    pub mask: usize,
    /// Parameter for parameterised events (chan_id, task_id, tick count).
    pub param: usize,
    /// Absolute tick at which the TIMER event fires (0 = not set).
    pub timer_deadline: u64,
}

impl PollEntry {
    pub const fn empty() -> Self {
        Self {
            mask: 0,
            param: 0,
            timer_deadline: 0,
        }
    }
}

/// Poll table — one entry per task slot.
pub struct PollTable {
    pub entries: [PollEntry; MAX_TASKS],
}

impl PollTable {
    pub const fn new() -> Self {
        Self {
            entries: [PollEntry::empty(); MAX_TASKS],
        }
    }

    /// Register interest for a task. Clears any previous registration.
    pub fn set(&mut self, task_id: usize, mask: usize, param: usize, now: u64) {
        if task_id >= MAX_TASKS {
            return;
        }
        let e = &mut self.entries[task_id];
        e.mask = mask;
        e.param = param;
        e.timer_deadline = if mask & POLL_TIMER != 0 {
            now.saturating_add(param as u64)
        } else {
            0
        };
    }

    /// Check which events have fired for a given task.
    ///
    /// Returns a bitmask of fired events (0 = none).
    pub fn check(
        &self,
        task_id: usize,
        sched: &Scheduler,
        ipc: &Ipc,
        channels: &Channels,
    ) -> usize {
        if task_id >= MAX_TASKS {
            return 0;
        }
        let e = &self.entries[task_id];
        if e.mask == 0 {
            return 0;
        }
        let mut fired = 0usize;

        // Timer
        if e.mask & POLL_TIMER != 0 && e.timer_deadline > 0 && sched.ticks >= e.timer_deadline {
            fired |= POLL_TIMER;
        }

        // IPC message pending
        if e.mask & POLL_IPC != 0 && ipc.has_message(task_id as u8) {
            fired |= POLL_IPC;
        }

        // Channel readable (has data)
        if e.mask & POLL_CHAN_READABLE != 0 {
            if channels.has_data(e.param) {
                fired |= POLL_CHAN_READABLE;
            }
        }

        // Channel writable (has space)
        if e.mask & POLL_CHAN_WRITABLE != 0 {
            if channels.has_space(e.param) {
                fired |= POLL_CHAN_WRITABLE;
            }
        }

        // Task exited
        if e.mask & POLL_TASK_EXIT != 0 {
            let target = e.param;
            if target < sched.tasks.len() && sched.tasks[target].state == TaskState::Free {
                fired |= POLL_TASK_EXIT;
            }
        }

        fired
    }

    /// Clear a task's poll registration.
    pub fn clear(&mut self, task_id: usize) {
        if task_id < MAX_TASKS {
            self.entries[task_id] = PollEntry::empty();
        }
    }
}

/// Wake any tasks that are poll-blocked and have events ready or timed out.
///
/// Called from the timer tick handler alongside `wake_sleepers`.
pub fn wake_poll_waiters(
    poll: &mut PollTable,
    sched: &mut Scheduler,
    ipc: &Ipc,
    channels: &Channels,
) {
    let now = sched.ticks;
    for i in 0..MAX_TASKS {
        if sched.tasks[i].state == TaskState::Blocked
            && sched.tasks[i].block_reason == BlockReason::PollWait
        {
            let fired = poll.check(i, sched, ipc, channels);
            let timed_out = sched.tasks[i].wakeup_tick > 0 && now >= sched.tasks[i].wakeup_tick;
            if fired != 0 || timed_out {
                sched.tasks[i].state = TaskState::Ready;
                sched.tasks[i].block_reason = BlockReason::None;
                sched.tasks[i].wakeup_tick = 0;
                // Store the fired events mask so the re-executed
                // SYS_POLL_WAIT sees them and returns immediately.
                // (The fired mask is returned to a0.)
                sched.tasks[i].context.set_ret(0, fired);
                poll.clear(i);
            }
        }
    }
}

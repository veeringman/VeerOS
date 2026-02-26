//! Minimal synchronous IPC (inter-process communication) for VeerOS.
//!
//! This is a lightweight, fixed-size message-passing primitive designed for
//! microkernel-style service calls between tasks.
//!
//! Design:
//! - Each task owns one **mailbox** (single-slot, no heap).
//! - `send()` copies a message into the destination's mailbox.
//! - `recv()` blocks the calling task until a message arrives.
//! - Messages are small fixed-size structs (fits in registers on context switch).

use crate::task::{Scheduler, TaskState, MAX_TASKS};

// ---------------------------------------------------------------------------
// Message type
// ---------------------------------------------------------------------------

/// Fixed-size IPC message — 4 machine words.
///
/// Kept small so it can travel in registers and be copied cheaply.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Message {
    /// Sender task ID (filled in by the kernel, not the caller).
    pub sender: u8,
    /// Opcode / service number — meaning is defined by the receiving service.
    pub opcode: u8,
    /// Payload words — general purpose.
    pub arg0: usize,
    pub arg1: usize,
}

impl Message {
    pub const fn empty() -> Self {
        Self {
            sender: 0,
            opcode: 0,
            arg0: 0,
            arg1: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Mailbox (one per task)
// ---------------------------------------------------------------------------

/// Single-slot mailbox. `full` indicates whether `msg` contains a pending
/// message that has not yet been consumed by the owning task.
#[derive(Debug, Clone, Copy)]
pub struct Mailbox {
    pub msg: Message,
    pub full: bool,
}

impl Mailbox {
    pub const fn empty() -> Self {
        Self {
            msg: Message::empty(),
            full: false,
        }
    }
}

// ---------------------------------------------------------------------------
// IPC subsystem (lives next to the scheduler)
// ---------------------------------------------------------------------------

/// Fixed array of mailboxes — one per task slot.
pub struct Ipc {
    pub mailboxes: [Mailbox; MAX_TASKS],
}

impl Ipc {
    pub const fn new() -> Self {
        Self {
            mailboxes: [Mailbox::empty(); MAX_TASKS],
        }
    }

    /// Send a message to task `dest`.
    ///
    /// - Copies `msg` into the destination's mailbox (overwrites if already
    ///   full — fire-and-forget semantics in v1).
    /// - If the destination task was `Blocked` waiting for a message, it is
    ///   moved back to `Ready`.
    ///
    /// Returns `true` on success, `false` if `dest` is out of range or the
    /// destination slot is `Free`.
    pub fn send(&mut self, sched: &mut Scheduler, from: u8, dest: u8, mut msg: Message) -> bool {
        let d = dest as usize;
        if d >= MAX_TASKS {
            return false;
        }
        if sched.tasks[d].state == TaskState::Free {
            return false;
        }

        msg.sender = from;
        self.mailboxes[d].msg = msg;
        self.mailboxes[d].full = true;

        // Wake the destination if it was blocked on recv.
        if sched.tasks[d].state == TaskState::Blocked {
            sched.tasks[d].state = TaskState::Ready;
        }

        true
    }

    /// Try to receive a message for task `tid`.
    ///
    /// - If the mailbox is full, consumes the message and returns `Some(msg)`.
    /// - If the mailbox is empty, marks the task as `Blocked` and returns
    ///   `None` — the scheduler should then switch away.
    pub fn recv(&mut self, sched: &mut Scheduler, tid: u8) -> Option<Message> {
        let t = tid as usize;
        if t >= MAX_TASKS {
            return None;
        }

        if self.mailboxes[t].full {
            let msg = self.mailboxes[t].msg;
            self.mailboxes[t].full = false;
            Some(msg)
        } else {
            // Nothing pending — block until a send() wakes us.
            if sched.tasks[t].state == TaskState::Running {
                sched.tasks[t].state = TaskState::Blocked;
            }
            None
        }
    }

    /// Non-blocking peek: returns `true` if the mailbox has a pending message.
    pub fn has_message(&self, tid: u8) -> bool {
        let t = tid as usize;
        if t >= MAX_TASKS {
            return false;
        }
        self.mailboxes[t].full
    }
}

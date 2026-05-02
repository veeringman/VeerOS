//! Intent submission and lifecycle — declarative goals that the kernel
//! decomposes into agent DAGs.
//!
//! # Example
//!
//! ```rust,no_run
//! use userlib::intent;
//!
//! let id = intent::submit(
//!     "deploy web service to edge nodes",
//!     intent::Class::Deploy,
//!     intent::Priority::Elevated,
//! ).unwrap();
//!
//! loop {
//!     match intent::status(id) {
//!         Some(intent::Status::Fulfilled) => break,
//!         Some(intent::Status::Failed) => panic!("intent failed"),
//!         _ => userlib::task::yield_now(),
//!     }
//! }
//! ```

use crate::sys;

const SYS_INTENT_SUBMIT: usize = 0xF5;
const SYS_INTENT_STATUS: usize = 0xF6;
const SYS_INTENT_CANCEL: usize = 0xF7;
const SYS_INTENT_SCHED_STATS: usize = 0xFB;

/// Intent classification — tells the kernel what kind of goal this is.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Compute = 0,
    Deploy = 1,
    Monitor = 2,
    Communicate = 3,
    Data = 4,
    Admin = 5,
    Pipeline = 6,
    Custom = 255,
}

/// Intent lifecycle status.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Free = 0,
    Pending = 1,
    Planning = 2,
    Active = 3,
    Fulfilled = 4,
    Failed = 5,
    Cancelled = 6,
}

impl Status {
    pub fn from_usize(v: usize) -> Self {
        match v {
            0 => Self::Free,
            1 => Self::Pending,
            2 => Self::Planning,
            3 => Self::Active,
            4 => Self::Fulfilled,
            5 => Self::Failed,
            6 => Self::Cancelled,
            _ => Self::Free,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Pending => "pending",
            Self::Planning => "planning",
            Self::Active => "active",
            Self::Fulfilled => "fulfilled",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Goal priority (same as agent priority — shared enum).
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Background = 0,
    Normal = 1,
    Elevated = 2,
    Critical = 3,
    Realtime = 4,
}

/// Submit an intent — a declarative goal for the kernel to fulfil.
///
/// - `description`: human-readable goal description (max 64 bytes).
/// - `class`: what kind of work this intent represents.
/// - `priority`: urgency level.
///
/// Returns the intent ID on success, `None` if the intent table is full.
#[inline]
pub fn submit(description: &str, class: Class, priority: Priority) -> Option<usize> {
    let ret = sys::syscall4(
        SYS_INTENT_SUBMIT,
        description.as_ptr() as usize,
        description.len(),
        class as usize,
        priority as usize,
    );
    if ret == usize::MAX {
        None
    } else {
        Some(ret)
    }
}

/// Query the lifecycle status of an intent.
#[inline]
pub fn status(intent_id: usize) -> Option<Status> {
    let ret = sys::syscall1(SYS_INTENT_STATUS, intent_id);
    if ret == usize::MAX {
        None
    } else {
        Some(Status::from_usize(ret))
    }
}

/// Cancel an in-flight intent. Returns `true` on success.
#[inline]
pub fn cancel(intent_id: usize) -> bool {
    sys::syscall1(SYS_INTENT_CANCEL, intent_id) == 0
}

/// Query intent scheduler statistics.
///
/// Returns `(intents_fulfilled, agents_spawned)`.
#[inline]
pub fn sched_stats() -> (usize, usize) {
    sys::syscall2(SYS_INTENT_SCHED_STATS, 0, 0)
}

//! Agent lifecycle — spawn autonomous agents with goals, query and
//! manage their state, and read/write per-agent context memory.
//!
//! # Example
//!
//! ```rust,no_run
//! use userlib::agent;
//!
//! // Spawn an agent with a goal
//! extern "C" fn worker() { /* ... */ agent::complete(); }
//! let id = agent::spawn("deploy webapp", agent::Priority::Normal,
//!                        worker as usize, stack_top).unwrap();
//!
//! // Check status
//! let (state, ticks) = agent::status(id).unwrap();
//!
//! // Read/write context memory within an agent thread
//! agent::ctx_set(b"stage", b"provisioning");
//! let mut buf = [0u8; 64];
//! let len = agent::ctx_get(b"stage", &mut buf);
//! ```

use crate::sys;

const SYS_AGENT_SPAWN: usize = 0xF0;
const SYS_AGENT_STATUS: usize = 0xF1;
const SYS_AGENT_COMPLETE: usize = 0xF2;
const SYS_AGENT_CTX_SET: usize = 0xF3;
const SYS_AGENT_CTX_GET: usize = 0xF4;
const SYS_AGENT_COUNT: usize = 0xFC;

/// Agent goal priority.
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Background = 0,
    Normal = 1,
    Elevated = 2,
    Critical = 3,
    Realtime = 4,
}

/// Agent lifecycle state (returned by `status()`).
#[repr(usize)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Free = 0,
    Spawned = 1,
    Planning = 2,
    Executing = 3,
    Blocked = 4,
    Completed = 5,
    Failed = 6,
}

impl State {
    pub fn from_usize(v: usize) -> Self {
        match v {
            0 => Self::Free,
            1 => Self::Spawned,
            2 => Self::Planning,
            3 => Self::Executing,
            4 => Self::Blocked,
            5 => Self::Completed,
            6 => Self::Failed,
            _ => Self::Free,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Spawned => "spawned",
            Self::Planning => "planning",
            Self::Executing => "executing",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Spawn a new autonomous agent.
///
/// - `goal`: UTF-8 description of what the agent should accomplish (max 64 bytes).
/// - `priority`: scheduling priority for the agent's goal.
/// - `entry`: function pointer for the agent's kernel thread.
/// - `stack_top`: top of the agent's stack.
///
/// Returns the agent ID on success, `None` if the agent table is full.
#[inline]
pub fn spawn(goal: &str, priority: Priority, entry: usize, stack_top: usize) -> Option<usize> {
    let ret = sys::syscall5(
        SYS_AGENT_SPAWN,
        goal.as_ptr() as usize,
        goal.len(),
        priority as usize,
        entry,
        stack_top,
    );
    if ret == usize::MAX { None } else { Some(ret) }
}

/// Query agent status.
///
/// Returns `(state, ticks_used)` on success, `None` for invalid agent ID.
#[inline]
pub fn status(agent_id: usize) -> Option<(State, usize)> {
    let (s, t) = sys::syscall2(SYS_AGENT_STATUS, agent_id, 0);
    if s == usize::MAX { None } else { Some((State::from_usize(s), t)) }
}

/// Mark the calling agent as completed.
#[inline]
pub fn complete() -> bool {
    // Agent marks itself completed — need agent_id. Since the kernel
    // resolves the calling task's agent, we pass the state directly.
    // The actual agent_id must be known by the caller or obtained via
    // a TLS-like mechanism. For now, a convenience wrapper exists below.
    false
}

/// Mark a specific agent as completed.
#[inline]
pub fn complete_agent(agent_id: usize) -> bool {
    sys::syscall3(SYS_AGENT_COMPLETE, agent_id, 5, 0) == 0
}

/// Mark a specific agent as failed.
#[inline]
pub fn fail_agent(agent_id: usize) -> bool {
    sys::syscall3(SYS_AGENT_COMPLETE, agent_id, 6, 0) == 0
}

/// Set a key-value pair in the calling agent's context memory.
///
/// Keys are up to 32 bytes, values up to 64 bytes.
#[inline]
pub fn ctx_set(key: &[u8], value: &[u8]) -> bool {
    sys::syscall4(
        SYS_AGENT_CTX_SET,
        key.as_ptr() as usize,
        key.len(),
        value.as_ptr() as usize,
        value.len(),
    ) == 0
}

/// Get a value from the calling agent's context memory.
///
/// Returns the value length on success (may exceed `buf.len()` if
/// buffer is too small — only `min(val_len, buf.len())` bytes written).
/// Returns `None` if the key is not found.
#[inline]
pub fn ctx_get(key: &[u8], buf: &mut [u8]) -> Option<usize> {
    let ret = sys::syscall4(
        SYS_AGENT_CTX_GET,
        key.as_ptr() as usize,
        key.len(),
        buf.as_mut_ptr() as usize,
        buf.len(),
    );
    if ret == usize::MAX { None } else { Some(ret) }
}

/// Get the number of active (non-free) agents.
#[inline]
pub fn count() -> usize {
    sys::syscall0(SYS_AGENT_COUNT)
}

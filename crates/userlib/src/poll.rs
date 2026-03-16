//! Poll / async event multiplexing wrappers.
//!
//! Provides access to the kernel's poll subsystem which lets a task
//! register interest in multiple event sources and wait for any of
//! them to fire.

use crate::sys;

// Syscall numbers — must match `microkernel::syscall`.
const SYS_POLL_SET: usize = 0x60;
const SYS_POLL_WAIT: usize = 0x61;

// Event flags — must match `microkernel::syscall::POLL_*`.
pub const POLL_TIMER: usize = 1 << 0;
pub const POLL_IPC: usize = 1 << 1;
pub const POLL_CHAN_READABLE: usize = 1 << 2;
pub const POLL_CHAN_WRITABLE: usize = 1 << 3;
pub const POLL_TASK_EXIT: usize = 1 << 4;

/// Register interest in a set of events.
///
/// - `mask`: OR of `POLL_*` flags selecting event types.
/// - `param`: event-specific parameter (channel id, task id, or tick
///   count for `POLL_TIMER`).
///
/// This replaces any previous registration for the calling task.
pub fn poll_set(mask: usize, param: usize) {
    sys::syscall2(SYS_POLL_SET, mask, param);
}

/// Wait for registered events to fire.
///
/// - `timeout`: maximum ticks to wait. `0` polls without blocking;
///   `usize::MAX` waits indefinitely.
///
/// Returns a bitmask of the events that actually fired (may be 0 if
/// the timeout expired without any events).
pub fn poll_wait(timeout: usize) -> usize {
    sys::syscall1(SYS_POLL_WAIT, timeout)
}

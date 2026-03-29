//! Capability management syscalls.
//!
//! These wrap the kernel's per-process capability syscalls into a safe API.
//! Capabilities are bit-flags that gate access to syscall groups; a process
//! can only *drop* its own caps (never gain new ones).

use crate::sys;

// Syscall numbers (must match microkernel::syscall)
const SYS_CAP_GET: usize       = 0xD0;
const SYS_CAP_DROP: usize      = 0xD1;
const SYS_CAP_SET_CHILD: usize = 0xD2;

/// Get the current process's capability bitmask.
#[inline]
pub fn get() -> u32 {
    sys::syscall0(SYS_CAP_GET) as u32
}

/// Irrevocably drop capabilities from the current process.
///
/// `bits` is a bitmask of capabilities to remove.  Returns `true` on
/// success.  Dropping already-absent caps is a no-op success.
#[inline]
pub fn drop_caps(bits: u32) -> bool {
    sys::syscall1(SYS_CAP_DROP, bits as usize) == 0
}

/// Set the capability mask of a child process.
///
/// The caller must own `CAP_ADMIN`, the target must be the caller's direct
/// child, and `bits` must be a subset of the caller's own caps.
/// Returns `true` on success.
#[inline]
pub fn set_child(child_pid: usize, bits: u32) -> bool {
    sys::syscall2(SYS_CAP_SET_CHILD, child_pid, bits as usize).0 == 0
}

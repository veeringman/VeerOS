//! User identity syscall wrappers.
//!
//! Provides safe wrappers for `SYS_GETUID`, `SYS_GETGID`, `SYS_SETUID`,
//! `SYS_LOGIN`, and `SYS_LOGOUT`.

use crate::sys::{syscall0, syscall1, syscall4};

const SYS_GETUID: usize = 0x90;
const SYS_GETGID: usize = 0x91;
const SYS_SETUID: usize = 0x92;
const SYS_LOGIN: usize = 0x93;
const SYS_LOGOUT: usize = 0x94;

/// Get the calling process's user ID.
pub fn getuid() -> u16 {
    syscall0(SYS_GETUID) as u16
}

/// Get the calling process's group ID.
pub fn getgid() -> u16 {
    syscall0(SYS_GETGID) as u16
}

/// Set the calling process's UID (root only).
/// Returns `true` on success, `false` on permission denied.
pub fn setuid(uid: u16) -> bool {
    syscall1(SYS_SETUID, uid as usize) == 0
}

/// Authenticate and create a session.
/// Returns a session token (nonzero) on success, or 0 on failure.
pub fn login(username: &str, password: &[u8]) -> u32 {
    let ret = syscall4(
        SYS_LOGIN,
        username.as_ptr() as usize,
        username.len(),
        password.as_ptr() as usize,
        password.len(),
    );
    ret as u32
}

/// Invalidate the current session and reset to root.
/// Returns `true` if a session was found and invalidated.
pub fn logout() -> bool {
    syscall0(SYS_LOGOUT) != 0
}

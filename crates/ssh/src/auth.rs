//! SSH-2 User Authentication (RFC 4252).
//!
//! Supports:
//! - `none` — always rejected, reports available methods
//! - `password` — FNV-1a hash comparison (same as existing VeerOS auth)
//! - `publickey` — Ed25519 public key verification (future)

use crate::{get_string, get_u32, put_string, put_u32, MAX_PAYLOAD};
use arch::Serial;

/// Authentication result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthResult {
    /// Authentication succeeded.
    Success,
    /// Authentication failed — send USERAUTH_FAILURE with available methods.
    Failure,
    /// Partial success (multi-factor — not used yet).
    Partial,
}

/// Check a USERAUTH_REQUEST payload.
///
/// Payload format (RFC 4252 §5):
///   byte      SSH_MSG_USERAUTH_REQUEST (50)
///   string    user name
///   string    service name ("ssh-connection")
///   string    method name ("password" | "none" | "publickey")
///   ... method-specific fields
///
/// `password_verify` is called with `(username_bytes, password_bytes)` and
/// must return `true` to accept.
///
/// Returns `(AuthResult, username_len)`. The username is copied into `user_out`.
pub fn check_userauth(
    payload: &[u8],
    password_verify: fn(&[u8], &[u8]) -> bool,
    user_out: &mut [u8; 64],
) -> (AuthResult, usize) {
    if payload.is_empty() || payload[0] != 50 {
        return (AuthResult::Failure, 0);
    }

    let mut off = 1;

    // user name
    if off + 4 > payload.len() {
        return (AuthResult::Failure, 0);
    }
    let (user, consumed) = get_string(&payload[off..]);
    off += consumed;
    let user_len = user.len().min(64);
    user_out[..user_len].copy_from_slice(&user[..user_len]);

    // service name (expect "ssh-connection")
    if off + 4 > payload.len() {
        return (AuthResult::Failure, 0);
    }
    let (_service, consumed) = get_string(&payload[off..]);
    off += consumed;

    // method name
    if off + 4 > payload.len() {
        return (AuthResult::Failure, 0);
    }
    let (method, consumed) = get_string(&payload[off..]);
    off += consumed;

    if method == b"none" {
        return (AuthResult::Failure, user_len);
    }

    if method == b"password" {
        // boolean    FALSE (no old password)
        if off >= payload.len() {
            return (AuthResult::Failure, user_len);
        }
        let _change_password = payload[off];
        off += 1;

        // string    password
        if off + 4 > payload.len() {
            return (AuthResult::Failure, user_len);
        }
        let (password, _consumed) = get_string(&payload[off..]);

        // Verify via caller-supplied callback
        if password_verify(&user_out[..user_len], password) {
            return (AuthResult::Success, user_len);
        } else {
            return (AuthResult::Failure, user_len);
        }
    }

    // Unknown method
    (AuthResult::Failure, user_len)
}

/// Build a USERAUTH_FAILURE message.
///
/// Returns payload bytes written.
pub fn build_userauth_failure(buf: &mut [u8]) -> usize {
    let mut off = 0;
    buf[off] = 51; // SSH_MSG_USERAUTH_FAILURE
    off += 1;
    // name-list of methods that can continue
    off += put_string(&mut buf[off..], b"password");
    // partial success
    buf[off] = 0; // false
    off += 1;
    off
}

/// Build a USERAUTH_SUCCESS message.
pub fn build_userauth_success(buf: &mut [u8]) -> usize {
    buf[0] = 52; // SSH_MSG_USERAUTH_SUCCESS
    1
}

/// FNV-1a 32-bit hash — same as net::auth::fnv1a for compatibility.
pub const fn fnv1a(bytes: &[u8]) -> u32 {
    const FNV_OFFSET: u32 = 2_166_136_261;
    const FNV_PRIME: u32 = 16_777_619;
    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

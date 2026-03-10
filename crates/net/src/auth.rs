//! Simple authentication gate for VeerOS remote shell.
//!
//! Provides a password challenge before granting shell access over the
//! network.  The password is stored as a compile-time FNV-1a hash —
//! no plaintext appears in the binary.
//!
//! This is *not* SSH-grade security (no encryption, no key exchange),
//! but it prevents casual unauthorised access until a proper encrypted
//! protocol is added.

use arch::{Console, Serial};
use core::fmt::Write;

// ---------------------------------------------------------------------------
// FNV-1a hash (32-bit) — simple, no_std, no dependencies
// ---------------------------------------------------------------------------

const FNV_OFFSET: u32 = 2_166_136_261;
const FNV_PRIME: u32 = 16_777_619;

/// Compute the FNV-1a 32-bit hash of a byte slice.
pub const fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}

// ---------------------------------------------------------------------------
// Authentication gate
// ---------------------------------------------------------------------------

/// Maximum password length (bytes) accepted from the user.
const MAX_PW_LEN: usize = 64;

/// Maximum number of login attempts before the connection is dropped.
const MAX_ATTEMPTS: u8 = 3;

/// Run the login prompt.  Returns `true` if authentication succeeded.
///
/// `password_hash` is the pre-computed FNV-1a hash of the expected
/// password (use `auth::fnv1a(b"your-password")` at compile time).
pub fn login_prompt<S: Serial>(
    con: &mut Console<S>,
    password_hash: u32,
) -> bool {
    let _ = writeln!(con, "");
    let _ = writeln!(con, "VeerOS remote shell");
    let _ = writeln!(con, "");

    for attempt in 0..MAX_ATTEMPTS {
        let remaining = MAX_ATTEMPTS - attempt;
        if remaining < MAX_ATTEMPTS {
            let _ = writeln!(con, "  ({} attempt{} remaining)", remaining,
                if remaining == 1 { "" } else { "s" });
        }
        con.write_str_raw("password: ");

        let mut buf = [0u8; MAX_PW_LEN];
        let len = read_password(con, &mut buf);

        if len == 0 {
            // EOF / disconnect
            return false;
        }

        let entered_hash = fnv1a(&buf[..len]);
        if entered_hash == password_hash {
            let _ = writeln!(con, "");
            return true;
        }

        let _ = writeln!(con, "");
        let _ = writeln!(con, "  access denied");
    }

    let _ = writeln!(con, "  too many failed attempts — disconnecting");
    false
}

/// Read a password line (no echo).  Returns the number of bytes read.
///
/// Stops on CR, LF, or Ctrl-D (EOF, returns 0).
fn read_password<S: Serial>(con: &mut Console<S>, buf: &mut [u8]) -> usize {
    let mut pos = 0;
    loop {
        let b = con.read_byte();
        match b {
            // Enter → done
            0x0D | 0x0A => return pos,
            // Ctrl-D → EOF
            0x04 => return 0,
            // Ctrl-C → abort
            0x03 => return 0,
            // Backspace / DEL
            0x08 | 0x7F => {
                if pos > 0 {
                    pos -= 1;
                }
            }
            // Printable
            0x20..=0x7E => {
                if pos < buf.len() {
                    buf[pos] = b;
                    pos += 1;
                    // Print a dot for visual feedback
                    con.write_str_raw("*");
                }
            }
            _ => {}
        }
    }
}

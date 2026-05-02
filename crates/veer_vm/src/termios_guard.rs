//! Host terminal raw-mode guard.
//!
//! Puts stdin into raw (non-canonical, no-echo, no-isig) mode so individual
//! keystrokes — including Ctrl-C, arrow keys, etc. — are forwarded into the
//! guest instead of being intercepted by the host line discipline. The
//! original termios is restored on drop.

use anyhow::{Context, Result};
use nix::sys::termios::{self, SetArg, Termios};
use std::os::fd::{AsFd, AsRawFd};

pub struct RawMode {
    original: Termios,
    /// True if stdin was actually a TTY; otherwise drop is a no-op.
    active: bool,
}

impl RawMode {
    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn enter() -> Result<Self> {
        let stdin = std::io::stdin();
        let fd = stdin.as_fd();
        if !nix::unistd::isatty(fd.as_raw_fd()).unwrap_or(false) {
            // Not a TTY — keep stdin as-is (pipes, redirected files).
            // SAFETY: zero-initialised termios acts as an unused placeholder.
            let dummy: Termios = unsafe { std::mem::zeroed() };
            return Ok(Self {
                original: dummy,
                active: false,
            });
        }

        let original = loop {
            match termios::tcgetattr(fd) {
                Ok(t) => break t,
                Err(nix::errno::Errno::EINTR) => continue,
                Err(e) => return Err(e).context("tcgetattr(stdin)"),
            }
        };
        let mut raw = original.clone();
        termios::cfmakeraw(&mut raw);
        loop {
            match termios::tcsetattr(fd, SetArg::TCSANOW, &raw) {
                Ok(()) => break,
                Err(nix::errno::Errno::EINTR) => continue,
                // EIO/ENOTTY/TTOU — stdin belongs to a different process
                // group (e.g. we were launched in a background shell job);
                // fall back to leaving stdin alone rather than aborting.
                Err(nix::errno::Errno::EIO) | Err(nix::errno::Errno::ENOTTY) => {
                    return Ok(Self {
                        original,
                        active: false,
                    });
                }
                Err(e) => return Err(e).context("tcsetattr(stdin, raw)"),
            }
        }
        Ok(Self {
            original,
            active: true,
        })
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let stdin = std::io::stdin();
        let _ = termios::tcsetattr(stdin.as_fd(), SetArg::TCSANOW, &self.original);
    }
}

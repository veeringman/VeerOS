//! Kernel log ring buffer.
//!
//! A fixed-size ring buffer for kernel diagnostic messages (equivalent
//! to Linux's `dmesg`). Boot messages, driver init, errors, and other
//! kernel events are recorded here and displayed by `dmesg`.

use core::fmt::{self, Write};

/// Size of the log buffer in bytes.
const LOG_SIZE: usize = 4096;

/// Kernel log ring buffer.
pub struct KernelLog {
    buf: [u8; LOG_SIZE],
    /// Write position (wraps around).
    head: usize,
    /// Total bytes ever written (for detecting wrap).
    total: usize,
}

impl KernelLog {
    pub const fn new() -> Self {
        Self {
            buf: [0u8; LOG_SIZE],
            head: 0,
            total: 0,
        }
    }

    /// Append a byte to the log.
    fn push(&mut self, b: u8) {
        self.buf[self.head] = b;
        self.head = (self.head + 1) % LOG_SIZE;
        self.total += 1;
    }

    /// Append a string slice to the log.
    pub fn push_str(&mut self, s: &str) {
        for &b in s.as_bytes() {
            self.push(b);
        }
    }

    /// Write the log contents (oldest → newest) to a writer.
    pub fn dump(&self, w: &mut dyn fmt::Write) {
        if self.total <= LOG_SIZE {
            // No wrap — data is buf[0..head]
            if let Ok(s) = core::str::from_utf8(&self.buf[..self.head]) {
                let _ = w.write_str(s);
            }
        } else {
            // Wrapped — data is buf[head..] then buf[..head]
            if let Ok(s) = core::str::from_utf8(&self.buf[self.head..]) {
                let _ = w.write_str(s);
            }
            if let Ok(s) = core::str::from_utf8(&self.buf[..self.head]) {
                let _ = w.write_str(s);
            }
        }
    }
}

/// Implement `core::fmt::Write` so we can use `write!()` / `writeln!()`.
impl Write for KernelLog {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

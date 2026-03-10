//! Console I/O syscall wrappers and formatting helpers.
//!
//! These functions always operate on the platform's primary console
//! (UART0 on hardware, stdout on QEMU).

use crate::sys;
use core::fmt;

// Syscall numbers (must match microkernel::syscall)
const SYS_WRITE_BYTE: usize = 0x20;
const SYS_WRITE_BUF: usize = 0x21;
const SYS_READ_BYTE: usize = 0x22;

/// Write a single byte to the console.
#[inline]
pub fn write_byte(b: u8) {
    sys::syscall1(SYS_WRITE_BYTE, b as usize);
}

/// Write a byte-slice to the console.
///
/// The kernel copies from the caller's buffer in a loop, so this is
/// safe even without an MMU — both kernel and task share the same
/// address space today.
#[inline]
pub fn write_buf(buf: &[u8]) {
    sys::syscall2(SYS_WRITE_BUF, buf.as_ptr() as usize, buf.len());
}

/// Read one byte from the console (blocking).
///
/// Returns `None` if the kernel's console driver has no data and does
/// not support blocking reads (platform-dependent).
#[inline]
pub fn read_byte() -> Option<u8> {
    let r = sys::syscall0(SYS_READ_BYTE);
    if r == usize::MAX {
        None
    } else {
        Some(r as u8)
    }
}

/// A zero-sized writer that pushes bytes through the write syscall.
pub struct Console;

impl fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        write_buf(s.as_bytes());
        Ok(())
    }
}

/// Print formatted text to the console (no newline).
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::io::Console, $($arg)*);
    }};
}

/// Print formatted text to the console, followed by a newline.
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = writeln!($crate::io::Console, $($arg)*);
    }};
}

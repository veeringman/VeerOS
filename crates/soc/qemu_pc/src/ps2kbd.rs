//! PS/2 keyboard driver for x86 PCs.
//!
//! Reads scan codes from I/O port 0x60, translates scan-code set 1 to ASCII,
//! and feeds events into the microkernel `InputSubsystem`.  IRQ 1 (vector 33)
//! triggers the handler; the caller is responsible for sending EOI.

use crate::inb;
use core::sync::atomic::{AtomicBool, Ordering};

/// PS/2 data port.
const PS2_DATA: u16 = 0x60;
/// PS/2 status port.
const PS2_STATUS: u16 = 0x64;

// Status register bits.
const STATUS_OUTPUT_FULL: u8 = 1 << 0;

/// True when a Shift key (left or right) is held.
static SHIFT_HELD: AtomicBool = AtomicBool::new(false);
/// True when Ctrl key is held.
static CTRL_HELD: AtomicBool = AtomicBool::new(false);
/// True when Caps Lock is toggled on.
static CAPS_LOCK: AtomicBool = AtomicBool::new(false);

/// Returns `true` if a scan code is available.
pub fn has_data() -> bool {
    unsafe { (inb(PS2_STATUS) & STATUS_OUTPUT_FULL) != 0 }
}

/// Read one raw scan code from the keyboard controller.
pub fn read_scancode() -> u8 {
    unsafe { inb(PS2_DATA) }
}

/// Translate a scan-code set 1 press event to an ASCII byte.
///
/// Returns `Some(ascii)` for printable / control characters, `None` for
/// non-printable keys (shift, ctrl, alt, function keys, etc.).
pub fn scancode_to_ascii(scancode: u8) -> Option<u8> {
    // Break codes (key release) have bit 7 set.
    if scancode & 0x80 != 0 {
        let released = scancode & 0x7F;
        match released {
            0x2A | 0x36 => SHIFT_HELD.store(false, Ordering::Relaxed), // L/R Shift
            0x1D        => CTRL_HELD.store(false, Ordering::Relaxed),  // Ctrl
            _ => {}
        }
        return None;
    }

    // Press codes.
    match scancode {
        0x2A | 0x36 => { SHIFT_HELD.store(true, Ordering::Relaxed); return None; }
        0x1D        => { CTRL_HELD.store(true, Ordering::Relaxed);  return None; }
        0x3A        => {
            // Caps Lock toggle.
            let prev = CAPS_LOCK.load(Ordering::Relaxed);
            CAPS_LOCK.store(!prev, Ordering::Relaxed);
            return None;
        }
        _ => {}
    }

    let shift = SHIFT_HELD.load(Ordering::Relaxed);
    let ctrl  = CTRL_HELD.load(Ordering::Relaxed);
    let caps  = CAPS_LOCK.load(Ordering::Relaxed);

    // Lookup un-shifted ASCII.
    let base = match scancode {
        0x01 => 0x1B, // Esc
        0x02 => b'1',
        0x03 => b'2',
        0x04 => b'3',
        0x05 => b'4',
        0x06 => b'5',
        0x07 => b'6',
        0x08 => b'7',
        0x09 => b'8',
        0x0A => b'9',
        0x0B => b'0',
        0x0C => b'-',
        0x0D => b'=',
        0x0E => 0x08, // Backspace
        0x0F => b'\t',
        0x10 => b'q',
        0x11 => b'w',
        0x12 => b'e',
        0x13 => b'r',
        0x14 => b't',
        0x15 => b'y',
        0x16 => b'u',
        0x17 => b'i',
        0x18 => b'o',
        0x19 => b'p',
        0x1A => b'[',
        0x1B => b']',
        0x1C => b'\r', // Enter
        0x1E => b'a',
        0x1F => b's',
        0x20 => b'd',
        0x21 => b'f',
        0x22 => b'g',
        0x23 => b'h',
        0x24 => b'j',
        0x25 => b'k',
        0x26 => b'l',
        0x27 => b';',
        0x28 => b'\'',
        0x29 => b'`',
        0x2B => b'\\',
        0x2C => b'z',
        0x2D => b'x',
        0x2E => b'c',
        0x2F => b'v',
        0x30 => b'b',
        0x31 => b'n',
        0x32 => b'm',
        0x33 => b',',
        0x34 => b'.',
        0x35 => b'/',
        0x39 => b' ',
        _ => return None,
    };

    if ctrl {
        // Ctrl+A..Z → 0x01..0x1A
        if base >= b'a' && base <= b'z' {
            return Some(base - b'a' + 1);
        }
        return Some(base);
    }

    let effective_shift = shift ^ caps;

    if effective_shift {
        let shifted = match base {
            b'1' => b'!',
            b'2' => b'@',
            b'3' => b'#',
            b'4' => b'$',
            b'5' => b'%',
            b'6' => b'^',
            b'7' => b'&',
            b'8' => b'*',
            b'9' => b'(',
            b'0' => b')',
            b'-' => b'_',
            b'=' => b'+',
            b'[' => b'{',
            b']' => b'}',
            b';' => b':',
            b'\'' => b'"',
            b'`' => b'~',
            b'\\' => b'|',
            b',' => b'<',
            b'.' => b'>',
            b'/' => b'?',
            c @ b'a'..=b'z' => c - 32, // to uppercase
            other => other,
        };
        Some(shifted)
    } else {
        Some(base)
    }
}

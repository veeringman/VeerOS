//! UART keyboard: the PL011 serial line as a command-palette input source.
//!
//! QEMU `virt` has no PS/2 hardware, and `veer-vm`/HVF is serial-only, so
//! the primary VeeroS key path is the UART itself — exactly the
//! command-palette-first interaction the VeerFlow vision calls for
//! (invoke-first, not location-first). Bytes arriving on UART0 are decoded
//! into [`arch::InputEvent`]s and fed to the kernel [`InputSubsystem`].
//!
//! Special keys map above ASCII so UI layers can match them without
//! ambiguity:
//!
//! | Key       | Code |
//! |-----------|------|
//! | Enter     | `0x0D` (CR) |
//! | Backspace | `0x7F` (DEL; `0x08` is normalized to it) |
//! | Escape    | `0x1B` |
//! | Up        | `0x81` |
//! | Down      | `0x82` |
//! | Right     | `0x83` |
//! | Left      | `0x84` |
//!
//! VT100/ANSI arrow sequences (`ESC [ A/B/C/D`) are consumed by a small
//! state machine; anything else passes through as plain bytes.

use core::cell::Cell;

use arch::{InputDevice, InputEvent, Serial};

use crate::mem;
use crate::uart::Pl011;

pub const KEY_UP: u8 = 0x81;
pub const KEY_DOWN: u8 = 0x82;
pub const KEY_RIGHT: u8 = 0x83;
pub const KEY_LEFT: u8 = 0x84;

/// UART-backed keyboard.
pub struct UartKeyboard {
    uart: Pl011,
    /// 0 = idle, 1 = saw ESC, 2 = saw ESC `[`. `Cell` so the
    /// `&self`-receiver [`InputDevice`] impl can decode.
    esc_state: Cell<u8>,
}

impl UartKeyboard {
    pub const fn new() -> Self {
        Self {
            uart: Pl011::new(),
            esc_state: Cell::new(0),
        }
    }

    /// Decode one raw byte: a complete key, or `None` while an escape
    /// sequence is still in flight.
    fn decode(&self, b: u8) -> Option<InputEvent> {
        match self.esc_state.get() {
            1 => {
                if b == b'[' {
                    self.esc_state.set(2);
                    return None;
                }
                // Bare ESC followed by another key: report Escape; the
                // follow-up byte is consumed (v1 simplification).
                self.esc_state.set(0);
                return Some(InputEvent::KeyPress(0x1B));
            }
            2 => {
                self.esc_state.set(0);
                return match b {
                    b'A' => Some(InputEvent::KeyPress(KEY_UP)),
                    b'B' => Some(InputEvent::KeyPress(KEY_DOWN)),
                    b'C' => Some(InputEvent::KeyPress(KEY_RIGHT)),
                    b'D' => Some(InputEvent::KeyPress(KEY_LEFT)),
                    _ => None,
                };
            }
            _ => {}
        }
        match b {
            0x1B => {
                self.esc_state.set(1);
                None
            }
            0x08 => Some(InputEvent::KeyPress(0x7F)),
            _ => Some(InputEvent::KeyPress(b)),
        }
    }
}

impl InputDevice for UartKeyboard {
    fn poll_event(&self) -> InputEvent {
        loop {
            if !self.uart.has_data() {
                return InputEvent::None;
            }
            // Data is present: read DR directly (the driver's `read_byte`
            // would block, which we just ruled out).
            let b = unsafe { core::ptr::read_volatile(mem::UART0_BASE as *const u32) as u8 };
            if let Some(ev) = self.decode(b) {
                return ev;
            }
        }
    }

    fn has_event(&self) -> bool {
        self.uart.has_data()
    }
}

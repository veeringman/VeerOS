//! Input subsystem — multiplexes keyboard and mouse events from USB HID,
//! BLE HOGP, and future input sources into a unified queue readable via
//! `/dev/keyboard` and `/dev/mouse` device nodes.
//!
//! Also provides `InputSerial`, a bridge that implements the `Serial` trait
//! by pulling keystrokes from the keyboard queue. This lets the existing
//! shell work transparently with USB/BLE keyboards.
//!
//! # Device major/minor assignments
//!
//! | Major | Minor | Path            | Description                    |
//! |-------|-------|-----------------|--------------------------------|
//! |   1   |   0   | /dev/keyboard   | Keyboard — reads return ASCII  |
//! |   1   |   1   | /dev/mouse      | Mouse — reads return raw events|
//!
//! # Architecture
//!
//! ```text
//!  USB HID ──┐
//!  BLE HOGP ─┤──▶ KeyboardState ──▶ KBD_QUEUE ──▶ /dev/keyboard
//!  GPIO kbd ─┘                                       │
//!                                                    ▼
//!                                              InputSerial
//!                                              (Serial trait)
//!                                                    │
//!                                                    ▼
//!                                                  Shell
//!
//!  USB HID ──┐
//!  BLE HOGP ─┤──▶ MouseState ──▶ MOUSE_QUEUE ──▶ /dev/mouse
//!  Touchpad ─┘
//! ```

use crate::hid::{InputQueue, KeyboardState, MouseState};
use arch::InputEvent;

// ═══════════════════════════════════════════════════════════════════════════
// Device major/minor for input devices
// ═══════════════════════════════════════════════════════════════════════════

/// Major number for input devices.
pub const INPUT_MAJOR: u8 = 1;
/// Minor 0 = keyboard.
pub const KBD_MINOR: u8 = 0;
/// Minor 1 = mouse.
pub const MOUSE_MINOR: u8 = 1;

// ═══════════════════════════════════════════════════════════════════════════
// Input subsystem
// ═══════════════════════════════════════════════════════════════════════════

/// Unified input subsystem managing keyboard and mouse queues.
pub struct InputSubsystem {
    /// Keyboard event queue (KeyPress / KeyRelease events as ASCII bytes).
    pub kbd_queue: InputQueue,
    /// Mouse event queue (MouseMove / MouseButton events).
    pub mouse_queue: InputQueue,
    /// Boot keyboard report state tracker.
    pub kbd_state: KeyboardState,
    /// Boot mouse report state tracker.
    pub mouse_state: MouseState,
    /// Number of keyboard reports received (stats).
    pub kbd_report_count: u32,
    /// Number of mouse reports received (stats).
    pub mouse_report_count: u32,
    /// Whether the subsystem has been initialised.
    pub active: bool,
}

impl InputSubsystem {
    pub const fn new() -> Self {
        Self {
            kbd_queue: InputQueue::new(),
            mouse_queue: InputQueue::new(),
            kbd_state: KeyboardState::new(),
            mouse_state: MouseState::new(),
            kbd_report_count: 0,
            mouse_report_count: 0,
            active: false,
        }
    }

    /// Initialise the input subsystem.
    pub fn init(&mut self) {
        self.kbd_queue.clear();
        self.mouse_queue.clear();
        self.active = true;
    }

    /// Feed an 8-byte USB HID boot keyboard report into the subsystem.
    pub fn feed_keyboard_report(&mut self, report: &[u8; 8]) {
        self.kbd_state.process_report(report, &mut self.kbd_queue);
        self.kbd_report_count = self.kbd_report_count.wrapping_add(1);
    }

    /// Feed a 3-byte USB HID boot mouse report into the subsystem.
    pub fn feed_mouse_report(&mut self, report: &[u8; 3]) {
        self.mouse_state
            .process_report(report, &mut self.mouse_queue);
        self.mouse_report_count = self.mouse_report_count.wrapping_add(1);
    }

    /// Feed a raw `InputEvent` directly (e.g., from BLE notification).
    pub fn feed_event(&mut self, ev: InputEvent) {
        match ev {
            InputEvent::KeyPress(_) | InputEvent::KeyRelease(_) => {
                self.kbd_queue.push(ev);
            }
            InputEvent::MouseMove { .. } | InputEvent::MouseButton { .. } => {
                self.mouse_queue.push(ev);
            }
            InputEvent::None => {}
        }
    }

    /// Read from the keyboard queue (for /dev/keyboard).
    ///
    /// Returns ASCII bytes from KeyPress events.
    /// Fills `buf` with as many bytes as available, returns count.
    pub fn kbd_read(&mut self, buf: &mut [u8]) -> usize {
        let mut count = 0;
        for b in buf.iter_mut() {
            match self.kbd_queue.pop() {
                InputEvent::KeyPress(ascii) => {
                    *b = ascii;
                    count += 1;
                }
                InputEvent::KeyRelease(_) => {
                    // Skip releases for byte-oriented reads.
                    continue;
                }
                _ => break,
            }
        }
        count
    }

    /// Read from the mouse queue (for /dev/mouse).
    ///
    /// Each mouse event is encoded as 4 bytes:
    /// `[type, button/dx_lo, dy_lo/0, flags]`
    ///
    /// Type 1 = MouseMove:  `[1, dx_lo, dy_lo, (dx_hi<<4 | dy_hi)]`
    /// Type 2 = MouseButton: `[2, button, pressed, 0]`
    ///
    /// Returns the number of bytes written.
    pub fn mouse_read(&mut self, buf: &mut [u8]) -> usize {
        let mut offset = 0;
        while offset + 4 <= buf.len() {
            match self.mouse_queue.pop() {
                InputEvent::MouseMove { dx, dy } => {
                    buf[offset] = 1; // type: move
                    buf[offset + 1] = dx as u8;
                    buf[offset + 2] = dy as u8;
                    buf[offset + 3] = (((dx >> 8) as u8) << 4) | ((dy >> 8) as u8 & 0x0F);
                    offset += 4;
                }
                InputEvent::MouseButton { button, pressed } => {
                    buf[offset] = 2; // type: button
                    buf[offset + 1] = button;
                    buf[offset + 2] = if pressed { 1 } else { 0 };
                    buf[offset + 3] = 0;
                    offset += 4;
                }
                _ => break,
            }
        }
        offset
    }

    /// Returns `true` if the keyboard queue has data.
    pub fn kbd_has_data(&self) -> bool {
        !self.kbd_queue.is_empty()
    }

    /// Returns `true` if the mouse queue has data.
    pub fn mouse_has_data(&self) -> bool {
        !self.mouse_queue.is_empty()
    }

    /// Write input subsystem status.
    pub fn write_status(&self, w: &mut dyn core::fmt::Write) {
        let _ = writeln!(
            w,
            "Input subsystem: {}",
            if self.active { "active" } else { "inactive" }
        );
        let _ = writeln!(
            w,
            "  Keyboard queue: {}/{} events, {} reports processed",
            self.kbd_queue.len(),
            crate::hid::INPUT_QUEUE_SIZE,
            self.kbd_report_count
        );
        let _ = writeln!(
            w,
            "  Mouse queue:    {}/{} events, {} reports processed",
            self.mouse_queue.len(),
            crate::hid::INPUT_QUEUE_SIZE,
            self.mouse_report_count
        );
    }
}

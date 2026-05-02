//! HID (Human Interface Device) core — scancode tables, input event queue,
//! and keyboard state tracking for USB HID and BLE HOGP devices.
//!
//! This module is device-agnostic: USB xHCI, BLE HOGP, and future PS/2 or
//! GPIO matrix keyboards all feed into the same `InputQueue`.
//!
//! # Keyboard protocol
//!
//! USB HID boot keyboard reports are 8 bytes:
//!
//! ```text
//! [0] modifier bitmask (Ctrl/Shift/Alt/GUI L+R)
//! [1] reserved (0x00)
//! [2..8] up to 6 simultaneous key scan codes (Usage IDs)
//! ```
//!
//! # Mouse protocol
//!
//! USB HID boot mouse reports are 3+ bytes:
//!
//! ```text
//! [0] button bitmask (bit 0 = left, bit 1 = right, bit 2 = middle)
//! [1] X displacement (i8, signed)
//! [2] Y displacement (i8, signed)
//! ```

use arch::InputEvent;

// ═══════════════════════════════════════════════════════════════════════════
// USB HID scancode → ASCII mapping (US QWERTY layout)
// ═══════════════════════════════════════════════════════════════════════════

/// USB HID Usage ID → ASCII (unshifted). Index = HID usage code (0x00–0x7F).
/// 0 means "no direct ASCII mapping" (modifier, special key, etc.).
static HID_TO_ASCII: [u8; 128] = {
    let mut t = [0u8; 128];
    // Letters a-z (HID 0x04–0x1D)
    t[0x04] = b'a';
    t[0x05] = b'b';
    t[0x06] = b'c';
    t[0x07] = b'd';
    t[0x08] = b'e';
    t[0x09] = b'f';
    t[0x0A] = b'g';
    t[0x0B] = b'h';
    t[0x0C] = b'i';
    t[0x0D] = b'j';
    t[0x0E] = b'k';
    t[0x0F] = b'l';
    t[0x10] = b'm';
    t[0x11] = b'n';
    t[0x12] = b'o';
    t[0x13] = b'p';
    t[0x14] = b'q';
    t[0x15] = b'r';
    t[0x16] = b's';
    t[0x17] = b't';
    t[0x18] = b'u';
    t[0x19] = b'v';
    t[0x1A] = b'w';
    t[0x1B] = b'x';
    t[0x1C] = b'y';
    t[0x1D] = b'z';
    // Digits 1-9, 0 (HID 0x1E–0x27)
    t[0x1E] = b'1';
    t[0x1F] = b'2';
    t[0x20] = b'3';
    t[0x21] = b'4';
    t[0x22] = b'5';
    t[0x23] = b'6';
    t[0x24] = b'7';
    t[0x25] = b'8';
    t[0x26] = b'9';
    t[0x27] = b'0';
    // Special keys
    t[0x28] = b'\r'; // Enter
    t[0x29] = 0x1B; // Escape
    t[0x2A] = 0x08; // Backspace
    t[0x2B] = b'\t'; // Tab
    t[0x2C] = b' '; // Space
                    // Symbols (US layout, unshifted)
    t[0x2D] = b'-';
    t[0x2E] = b'=';
    t[0x2F] = b'[';
    t[0x30] = b']';
    t[0x31] = b'\\';
    t[0x33] = b';';
    t[0x34] = b'\'';
    t[0x35] = b'`';
    t[0x36] = b',';
    t[0x37] = b'.';
    t[0x38] = b'/';
    // Delete
    t[0x4C] = 0x7F;
    t
};

/// USB HID Usage ID → ASCII (shifted). Only entries that change with Shift.
static HID_TO_ASCII_SHIFT: [u8; 128] = {
    let mut t = [0u8; 128];
    // Shift + letters → uppercase
    t[0x04] = b'A';
    t[0x05] = b'B';
    t[0x06] = b'C';
    t[0x07] = b'D';
    t[0x08] = b'E';
    t[0x09] = b'F';
    t[0x0A] = b'G';
    t[0x0B] = b'H';
    t[0x0C] = b'I';
    t[0x0D] = b'J';
    t[0x0E] = b'K';
    t[0x0F] = b'L';
    t[0x10] = b'M';
    t[0x11] = b'N';
    t[0x12] = b'O';
    t[0x13] = b'P';
    t[0x14] = b'Q';
    t[0x15] = b'R';
    t[0x16] = b'S';
    t[0x17] = b'T';
    t[0x18] = b'U';
    t[0x19] = b'V';
    t[0x1A] = b'W';
    t[0x1B] = b'X';
    t[0x1C] = b'Y';
    t[0x1D] = b'Z';
    // Shift + digits → symbols
    t[0x1E] = b'!';
    t[0x1F] = b'@';
    t[0x20] = b'#';
    t[0x21] = b'$';
    t[0x22] = b'%';
    t[0x23] = b'^';
    t[0x24] = b'&';
    t[0x25] = b'*';
    t[0x26] = b'(';
    t[0x27] = b')';
    // Shift + symbols
    t[0x28] = b'\r'; // Enter (unchanged)
    t[0x2C] = b' '; // Space (unchanged)
    t[0x2D] = b'_';
    t[0x2E] = b'+';
    t[0x2F] = b'{';
    t[0x30] = b'}';
    t[0x31] = b'|';
    t[0x33] = b':';
    t[0x34] = b'"';
    t[0x35] = b'~';
    t[0x36] = b'<';
    t[0x37] = b'>';
    t[0x38] = b'?';
    t
};

// ═══════════════════════════════════════════════════════════════════════════
// HID modifier bitmask (byte 0 of boot keyboard report)
// ═══════════════════════════════════════════════════════════════════════════

pub const MOD_LEFT_CTRL: u8 = 1 << 0;
pub const MOD_LEFT_SHIFT: u8 = 1 << 1;
pub const MOD_LEFT_ALT: u8 = 1 << 2;
pub const MOD_LEFT_GUI: u8 = 1 << 3;
pub const MOD_RIGHT_CTRL: u8 = 1 << 4;
pub const MOD_RIGHT_SHIFT: u8 = 1 << 5;
pub const MOD_RIGHT_ALT: u8 = 1 << 6;
pub const MOD_RIGHT_GUI: u8 = 1 << 7;

/// Returns `true` if any Shift key is held.
#[inline]
pub fn is_shift(modifiers: u8) -> bool {
    modifiers & (MOD_LEFT_SHIFT | MOD_RIGHT_SHIFT) != 0
}

/// Returns `true` if any Ctrl key is held.
#[inline]
pub fn is_ctrl(modifiers: u8) -> bool {
    modifiers & (MOD_LEFT_CTRL | MOD_RIGHT_CTRL) != 0
}

// ═══════════════════════════════════════════════════════════════════════════
// Scancode → InputEvent conversion
// ═══════════════════════════════════════════════════════════════════════════

/// Convert a USB HID usage code + modifiers to an `InputEvent::KeyPress`.
/// Returns `InputEvent::None` for unknown / unmapped keys.
pub fn hid_key_to_event(usage: u8, modifiers: u8) -> InputEvent {
    if usage == 0 || usage as usize >= 128 {
        return InputEvent::None;
    }
    let ascii = if is_shift(modifiers) {
        let s = HID_TO_ASCII_SHIFT[usage as usize];
        if s != 0 {
            s
        } else {
            HID_TO_ASCII[usage as usize]
        }
    } else {
        HID_TO_ASCII[usage as usize]
    };
    // Ctrl+letter → control character (0x01–0x1A)
    if is_ctrl(modifiers) && ascii >= b'a' && ascii <= b'z' {
        return InputEvent::KeyPress(ascii - b'a' + 1);
    }
    if is_ctrl(modifiers) && ascii >= b'A' && ascii <= b'Z' {
        return InputEvent::KeyPress(ascii - b'A' + 1);
    }
    if ascii != 0 {
        InputEvent::KeyPress(ascii)
    } else {
        InputEvent::None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Input event ring buffer
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum number of queued input events.
pub const INPUT_QUEUE_SIZE: usize = 64;

/// Lock-free single-producer / single-consumer ring buffer of `InputEvent`.
pub struct InputQueue {
    buf: [InputEvent; INPUT_QUEUE_SIZE],
    head: usize,
    tail: usize,
}

impl InputQueue {
    pub const fn new() -> Self {
        Self {
            buf: [InputEvent::None; INPUT_QUEUE_SIZE],
            head: 0,
            tail: 0,
        }
    }

    /// Number of queued events.
    pub fn len(&self) -> usize {
        self.tail.wrapping_sub(self.head) % INPUT_QUEUE_SIZE
    }

    pub fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    pub fn is_full(&self) -> bool {
        (self.tail + 1) % INPUT_QUEUE_SIZE == self.head
    }

    /// Push an event. Returns `false` if the queue is full (event dropped).
    pub fn push(&mut self, ev: InputEvent) -> bool {
        if self.is_full() {
            return false;
        }
        self.buf[self.tail] = ev;
        self.tail = (self.tail + 1) % INPUT_QUEUE_SIZE;
        true
    }

    /// Pop the oldest event. Returns `InputEvent::None` if empty.
    pub fn pop(&mut self) -> InputEvent {
        if self.is_empty() {
            return InputEvent::None;
        }
        let ev = self.buf[self.head];
        self.head = (self.head + 1) % INPUT_QUEUE_SIZE;
        ev
    }

    /// Peek at the oldest event without consuming it.
    pub fn peek(&self) -> InputEvent {
        if self.is_empty() {
            InputEvent::None
        } else {
            self.buf[self.head]
        }
    }

    /// Clear all events.
    pub fn clear(&mut self) {
        self.head = 0;
        self.tail = 0;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Boot keyboard report parser
// ═══════════════════════════════════════════════════════════════════════════

/// Tracks keyboard state and converts HID boot reports → InputEvents.
///
/// Maintains the set of previously pressed keys to generate release events
/// and detect new key presses (avoiding auto-repeat at the HID level).
pub struct KeyboardState {
    /// Previously reported key usage codes (up to 6).
    prev_keys: [u8; 6],
    /// Previous modifier byte.
    prev_mods: u8,
}

impl KeyboardState {
    pub const fn new() -> Self {
        Self {
            prev_keys: [0; 6],
            prev_mods: 0,
        }
    }

    /// Process an 8-byte USB HID boot keyboard report and push
    /// resulting InputEvents into `queue`.
    ///
    /// Report format: `[modifiers, 0x00, key0, key1, key2, key3, key4, key5]`
    pub fn process_report(&mut self, report: &[u8; 8], queue: &mut InputQueue) {
        let mods = report[0];
        let keys = &report[2..8];

        // Detect released keys (were in prev_keys but not in keys).
        for &pk in self.prev_keys.iter() {
            if pk == 0 {
                continue;
            }
            let still_held = keys.iter().any(|&k| k == pk);
            if !still_held {
                let ev = hid_key_to_event(pk, self.prev_mods);
                if let InputEvent::KeyPress(ascii) = ev {
                    queue.push(InputEvent::KeyRelease(ascii));
                }
            }
        }

        // Detect newly pressed keys (in keys but not in prev_keys).
        for &k in keys.iter() {
            if k == 0 {
                continue;
            }
            let was_held = self.prev_keys.iter().any(|&pk| pk == k);
            if !was_held {
                let ev = hid_key_to_event(k, mods);
                if ev != InputEvent::None {
                    queue.push(ev);
                }
            }
        }

        // Save state.
        self.prev_mods = mods;
        self.prev_keys.copy_from_slice(keys);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Boot mouse report parser
// ═══════════════════════════════════════════════════════════════════════════

/// Tracks mouse button state and converts HID boot reports → InputEvents.
pub struct MouseState {
    prev_buttons: u8,
}

impl MouseState {
    pub const fn new() -> Self {
        Self { prev_buttons: 0 }
    }

    /// Process a 3-byte USB HID boot mouse report.
    ///
    /// Report format: `[buttons, dx (i8), dy (i8)]`
    pub fn process_report(&mut self, report: &[u8; 3], queue: &mut InputQueue) {
        let buttons = report[0];
        let dx = report[1] as i8 as i16;
        let dy = report[2] as i8 as i16;

        // Movement event.
        if dx != 0 || dy != 0 {
            queue.push(InputEvent::MouseMove { dx, dy });
        }

        // Button changes.
        let changed = buttons ^ self.prev_buttons;
        for bit in 0..3u8 {
            if changed & (1 << bit) != 0 {
                let pressed = buttons & (1 << bit) != 0;
                queue.push(InputEvent::MouseButton {
                    button: bit,
                    pressed,
                });
            }
        }

        self.prev_buttons = buttons;
    }
}

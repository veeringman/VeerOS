//! VGA text-mode console (80×25 @ 0xB8000).
//!
//! Implements the `arch::Serial` trait so it can be used as a `Console`
//! backend alongside (or instead of) COM1.  Also provides direct
//! character/attribute writing for boot diagnostics.

use arch::Serial;

/// VGA text buffer base address.
const VGA_BASE: usize = 0xB8000;

/// Screen dimensions.
const COLS: usize = 80;
const ROWS: usize = 25;

/// Default attribute: light grey on black.
const DEFAULT_ATTR: u8 = 0x07;

/// Simple global cursor state.
/// (Not behind a lock — single-core only.)
static mut CURSOR_ROW: usize = 0;
static mut CURSOR_COL: usize = 0;

/// VGA text-mode console.
pub struct VgaText;

impl VgaText {
    pub const fn new() -> Self {
        Self
    }

    /// Clear the entire screen and reset cursor to (0,0).
    pub fn clear(&self) {
        let buf = VGA_BASE as *mut u16;
        let blank = (DEFAULT_ATTR as u16) << 8 | b' ' as u16;
        for i in 0..(COLS * ROWS) {
            unsafe {
                buf.add(i).write_volatile(blank);
            }
        }
        unsafe {
            CURSOR_ROW = 0;
            CURSOR_COL = 0;
        }
        self.update_hw_cursor();
    }

    /// Write a character with a given attribute at (row, col).
    pub fn write_at(&self, row: usize, col: usize, ch: u8, attr: u8) {
        if row >= ROWS || col >= COLS {
            return;
        }
        let buf = VGA_BASE as *mut u16;
        let offset = row * COLS + col;
        let val = (attr as u16) << 8 | ch as u16;
        unsafe {
            buf.add(offset).write_volatile(val);
        }
    }

    /// Scroll the screen up by one line.
    fn scroll_up(&self) {
        let buf = VGA_BASE as *mut u16;
        // Move lines 1..ROWS up by one.
        for i in 0..((ROWS - 1) * COLS) {
            unsafe {
                let val = buf.add(i + COLS).read_volatile();
                buf.add(i).write_volatile(val);
            }
        }
        // Clear the last line.
        let blank = (DEFAULT_ATTR as u16) << 8 | b' ' as u16;
        for c in 0..COLS {
            unsafe {
                buf.add((ROWS - 1) * COLS + c).write_volatile(blank);
            }
        }
    }

    /// Advance cursor, scrolling if needed.
    fn advance_cursor(&self) {
        unsafe {
            CURSOR_COL += 1;
            if CURSOR_COL >= COLS {
                CURSOR_COL = 0;
                CURSOR_ROW += 1;
            }
            if CURSOR_ROW >= ROWS {
                self.scroll_up();
                CURSOR_ROW = ROWS - 1;
            }
        }
    }

    /// Move cursor to the beginning of the next line.
    fn newline(&self) {
        unsafe {
            CURSOR_COL = 0;
            CURSOR_ROW += 1;
            if CURSOR_ROW >= ROWS {
                self.scroll_up();
                CURSOR_ROW = ROWS - 1;
            }
        }
    }

    /// Update the hardware text-mode cursor position via VGA I/O ports.
    fn update_hw_cursor(&self) {
        let pos = unsafe { CURSOR_ROW * COLS + CURSOR_COL } as u16;
        unsafe {
            // VGA CRTC register select (port 0x3D4) + data (port 0x3D5).
            crate::outb(0x3D4, 0x0F); // cursor location low
            crate::outb(0x3D5, (pos & 0xFF) as u8);
            crate::outb(0x3D4, 0x0E); // cursor location high
            crate::outb(0x3D5, ((pos >> 8) & 0xFF) as u8);
        }
    }

    /// Put a single character at the current cursor position and advance.
    fn putchar(&self, ch: u8) {
        match ch {
            b'\n' => self.newline(),
            b'\r' => unsafe {
                CURSOR_COL = 0;
            },
            0x08 => {
                // Backspace: move cursor back and blank the cell.
                unsafe {
                    if CURSOR_COL > 0 {
                        CURSOR_COL -= 1;
                        self.write_at(CURSOR_ROW, CURSOR_COL, b' ', DEFAULT_ATTR);
                    }
                }
            }
            ch => {
                unsafe {
                    self.write_at(CURSOR_ROW, CURSOR_COL, ch, DEFAULT_ATTR);
                }
                self.advance_cursor();
            }
        }
        self.update_hw_cursor();
    }
}

impl Serial for VgaText {
    fn write_byte(&self, byte: u8) {
        self.putchar(byte);
    }

    fn read_byte(&self) -> u8 {
        // VGA has no input — use PS/2 keyboard instead.
        // Block-poll the PS/2 keyboard here so that `Console<VgaText>` works
        // as a full interactive console.
        loop {
            if crate::ps2kbd::has_data() {
                let sc = crate::ps2kbd::read_scancode();
                if let Some(ascii) = crate::ps2kbd::scancode_to_ascii(sc) {
                    return ascii;
                }
            }
            core::hint::spin_loop();
        }
    }

    fn has_data(&self) -> bool {
        crate::ps2kbd::has_data()
    }
}

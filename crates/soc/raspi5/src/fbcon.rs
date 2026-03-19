//! Framebuffer console — implements `Serial` for HDMI output.
//!
//! Renders text on the GPU-allocated framebuffer using an 8×8 bitmap
//! font.  Output is mirrored to the PL011 UART so the serial terminal
//! stays in sync.
//!
//! Input (read_byte / has_data) is always sourced from the UART, since
//! the framebuffer is output-only.

use arch::Serial;
use crate::fb::FbInfo;
use crate::font;
use crate::logo;
use crate::uart::Pl011;

// ─── Console dimensions ──────────────────────────────────────────────

/// Character cell width in pixels.
const CHAR_W: u32 = font::CHAR_W;   // 8
/// Character cell height in pixels (glyph + row spacing).
const CHAR_H: u32 = font::CHAR_H;   // 12

// ─── Cursor state (module-level statics, single-core safe) ──────────

static mut CURSOR_COL: u32 = 0;
static mut CURSOR_ROW: u32 = 0;

// ─── Colors ──────────────────────────────────────────────────────────

/// Foreground: terminal green (0x00RRGGBB).
const FG_COLOR: u32 = 0x0000_FF00;
/// Background: black.
const BG_COLOR: u32 = 0x0000_0000;

// ─── FbConsole ───────────────────────────────────────────────────────

/// Framebuffer console device.
///
/// This is `Copy` so it can be passed by value to `Console<S: Serial>`.
/// Mutable per-instance state (cursor position) lives in module-level
/// statics — safe because VeerOS is single-core with cooperative
/// console access.
#[derive(Clone, Copy)]
pub struct FbConsole {
    /// Pointer to the pixel buffer (null when inactive).
    fb_ptr: *mut u32,
    /// Display width in pixels.
    width: u32,
    /// Display height in pixels.
    height: u32,
    /// Bytes per row (pitch).
    pitch: u32,
    /// Number of text columns.
    cols: u32,
    /// Number of text rows.
    rows: u32,
}

impl FbConsole {
    /// Create an inactive (UART-only) console.
    pub const fn inactive() -> Self {
        Self {
            fb_ptr: core::ptr::null_mut(),
            width: 0,
            height: 0,
            pitch: 0,
            cols: 0,
            rows: 0,
        }
    }

    /// Create an active framebuffer console from allocated FB info.
    pub fn new(info: FbInfo) -> Self {
        Self {
            fb_ptr: info.fb_ptr,
            width: info.width,
            height: info.height,
            pitch: info.pitch,
            cols: info.width / CHAR_W,
            rows: info.height / CHAR_H,
        }
    }

    /// Returns `true` if a framebuffer is available.
    #[inline]
    pub fn is_active(&self) -> bool {
        !self.fb_ptr.is_null()
    }

    /// Blit the boot logo at pixel position (x, y) with alpha blending
    /// against the black background. Returns the width used in pixels.
    pub fn blit_logo(&self, x: u32, y: u32) -> u32 {
        if !self.is_active() {
            return 0;
        }
        let w = logo::LOGO_W;
        let h = logo::LOGO_H;
        for dy in 0..h {
            for dx in 0..w {
                let px = logo::LOGO_DATA[(dy * w + dx) as usize];
                let a = (px >> 24) & 0xFF;
                if a == 0 {
                    continue; // fully transparent — skip
                }
                let r = (px >> 16) & 0xFF;
                let g = (px >> 8) & 0xFF;
                let b = px & 0xFF;
                // Alpha blend against black: out = color * alpha / 255
                let ro = (r * a) / 255;
                let go = (g * a) / 255;
                let bo = (b * a) / 255;
                let color = (ro << 16) | (go << 8) | bo;
                unsafe { self.put_pixel(x + dx, y + dy, color); }
            }
        }
        w
    }

    /// Clear the entire screen to the background color.
    pub fn clear_screen(&self) {
        if !self.is_active() {
            return;
        }
        unsafe {
            let total_bytes = self.pitch as usize * self.height as usize;
            core::ptr::write_bytes(self.fb_ptr as *mut u8, 0, total_bytes);
            CURSOR_COL = 0;
            CURSOR_ROW = 0;
        }
    }

    // ── Internal rendering ───────────────────────────────────────────

    /// Put a single pixel at (x, y).
    #[inline(always)]
    unsafe fn put_pixel(&self, x: u32, y: u32, color: u32) {
        if x < self.width && y < self.height {
            let offset = (y as usize * self.pitch as usize / 4) + x as usize;
            core::ptr::write_volatile(self.fb_ptr.add(offset), color);
        }
    }

    /// Draw a single character glyph at the given text (col, row) position.
    fn draw_char(&self, col: u32, row: u32, ch: u8) {
        let glyph = font::glyph(ch);
        let px = col * CHAR_W;
        let py = row * CHAR_H;

        unsafe {
            for glyph_row in 0..font::GLYPH_H {
                let bits = glyph[glyph_row as usize];
                let y = py + glyph_row;
                for bit in 0..8u32 {
                    let color = if bits & (0x80 >> bit) != 0 {
                        FG_COLOR
                    } else {
                        BG_COLOR
                    };
                    self.put_pixel(px + bit, y, color);
                }
            }
        }
    }

    /// Clear a character cell to the background color.
    fn clear_char(&self, col: u32, row: u32) {
        let px = col * CHAR_W;
        let py = row * CHAR_H;
        unsafe {
            for dy in 0..CHAR_H {
                for dx in 0..CHAR_W {
                    self.put_pixel(px + dx, py + dy, BG_COLOR);
                }
            }
        }
    }

    /// Scroll all text up by one line, clearing the bottom line.
    fn scroll_up(&self) {
        unsafe {
            let fb = self.fb_ptr as *mut u8;
            let row_bytes = self.pitch as usize * CHAR_H as usize;
            let total = (self.rows - 1) as usize * row_bytes;
            // Copy rows 1..N up to 0..N-1 (overlapping — use `copy`).
            core::ptr::copy(fb.add(row_bytes), fb, total);
            // Clear the last row.
            core::ptr::write_bytes(fb.add(total), 0, row_bytes);
        }
    }

    /// Handle a single byte of output on the framebuffer.
    fn render_byte(&self, byte: u8) {
        unsafe {
            match byte {
                b'\r' => {
                    CURSOR_COL = 0;
                }
                b'\n' => {
                    CURSOR_COL = 0;
                    CURSOR_ROW += 1;
                    if CURSOR_ROW >= self.rows {
                        self.scroll_up();
                        CURSOR_ROW = self.rows - 1;
                    }
                }
                0x08 => {
                    // Backspace
                    if CURSOR_COL > 0 {
                        CURSOR_COL -= 1;
                        self.clear_char(CURSOR_COL, CURSOR_ROW);
                    }
                }
                0x09 => {
                    // Tab — advance to next 8-column boundary
                    let target = (CURSOR_COL + 8) & !7;
                    let target = if target >= self.cols { self.cols - 1 } else { target };
                    while CURSOR_COL < target {
                        self.clear_char(CURSOR_COL, CURSOR_ROW);
                        CURSOR_COL += 1;
                    }
                }
                ch if ch >= 0x20 && ch <= 0x7E => {
                    self.draw_char(CURSOR_COL, CURSOR_ROW, ch);
                    CURSOR_COL += 1;
                    if CURSOR_COL >= self.cols {
                        CURSOR_COL = 0;
                        CURSOR_ROW += 1;
                        if CURSOR_ROW >= self.rows {
                            self.scroll_up();
                            CURSOR_ROW = self.rows - 1;
                        }
                    }
                }
                _ => {
                    // Ignore other control characters.
                }
            }
        }
    }
}

// ─── Serial trait implementation ─────────────────────────────────────

impl Serial for FbConsole {
    fn write_byte(&self, byte: u8) {
        // If framebuffer is active, render on screen.
        if self.is_active() {
            self.render_byte(byte);
        }

        // Mirror to UART only when RP1 is confirmed reachable.
        // The RP1 southbridge is behind PCIe — accessing it before
        // the firmware maps the BAR causes a synchronous data abort.
        if unsafe { UART_READY } {
            Pl011::new().write_byte(byte);
        }
    }

    fn read_byte(&self) -> u8 {
        // Check USB keyboard queue first.
        loop {
            if let Some(f) = unsafe { KBD_POLL_FN } {
                if let Some(b) = f() {
                    return b;
                }
            }
            if unsafe { UART_READY } {
                if Pl011::new().has_data() {
                    return Pl011::new().read_byte();
                }
            }
            // No data from either source — yield and retry.
            #[cfg(target_arch = "aarch64")]
            unsafe { core::arch::asm!("wfe", options(nomem, nostack)); }
        }
    }

    fn has_data(&self) -> bool {
        if let Some(f) = unsafe { KBD_HAS_DATA_FN } {
            if f() {
                return true;
            }
        }
        if unsafe { UART_READY } {
            Pl011::new().has_data()
        } else {
            false
        }
    }
}

// ─── UART readiness flag ─────────────────────────────────────────────

/// Set by the kernel after confirming RP1 UART is accessible.
static mut UART_READY: bool = false;

/// Mark the PL011 UART as safe to access (call after RP1 BAR is mapped).
pub fn enable_uart_mirror() {
    unsafe { UART_READY = true; }
}

/// Set the text cursor position (column, row) directly.
///
/// # Safety
/// Caller must ensure col and row are within the console grid.
pub unsafe fn set_cursor(col: u32, row: u32) {
    CURSOR_COL = col;
    CURSOR_ROW = row;
}

// ─── USB keyboard hook ──────────────────────────────────────────────

/// Optional callback to poll USB keyboard for a byte.
/// Returns `Some(ascii)` if a key is available, `None` otherwise.
static mut KBD_POLL_FN: Option<fn() -> Option<u8>> = None;

/// Optional callback to check if the keyboard queue has data (non-consuming).
static mut KBD_HAS_DATA_FN: Option<fn() -> bool> = None;

/// Register keyboard callbacks (called by the kernel after
/// InputSubsystem is initialised).
pub fn set_kbd_poll(poll: fn() -> Option<u8>, has_data: fn() -> bool) {
    unsafe {
        KBD_POLL_FN = Some(poll);
        KBD_HAS_DATA_FN = Some(has_data);
    }
}

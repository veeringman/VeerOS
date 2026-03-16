//! Readline-style line editor for the VeerOS shell.
//!
//! Provides cursor movement (left/right/home/end), word movement (Alt-b/f),
//! history recall (up/down arrows), character delete (Ctrl-D, Ctrl-H),
//! word kill (Ctrl-W, Alt-d), line kill (Ctrl-U/K), transpose (Ctrl-T),
//! and clear screen (Ctrl-L).

use arch::{Console, Serial};

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum bytes in a single line.
const MAX_LINE: usize = 128;

/// Number of history entries kept.
const HIST_SIZE: usize = 32;

// Control codes
const BS: u8 = 0x08;
const DEL: u8 = 0x7F;
const CR: u8 = 0x0D;
const LF: u8 = 0x0A;
const ETX: u8 = 0x03; // Ctrl-C
const EOT: u8 = 0x04; // Ctrl-D
const ESC: u8 = 0x1B;
const TAB: u8 = 0x09;

// ═══════════════════════════════════════════════════════════════════════════
// History ring-buffer
// ═══════════════════════════════════════════════════════════════════════════

/// Fixed-size ring buffer of history entries.
pub struct History {
    entries: [[u8; MAX_LINE]; HIST_SIZE],
    lengths: [usize; HIST_SIZE],
    /// Next slot to write (wraps around).
    head: usize,
    /// Number of valid entries (0..=HIST_SIZE).
    count: usize,
}

impl History {
    pub const fn new() -> Self {
        Self {
            entries: [[0u8; MAX_LINE]; HIST_SIZE],
            lengths: [0; HIST_SIZE],
            head: 0,
            count: 0,
        }
    }

    /// Push a line into history (skip if empty or same as last).
    pub fn push(&mut self, buf: &[u8], len: usize) {
        if len == 0 { return; }

        // Deduplicate: skip if same as most recent entry
        if self.count > 0 {
            let last = if self.head == 0 { HIST_SIZE - 1 } else { self.head - 1 };
            if self.lengths[last] == len && self.entries[last][..len] == buf[..len] {
                return;
            }
        }

        let idx = self.head;
        self.entries[idx][..len].copy_from_slice(&buf[..len]);
        self.lengths[idx] = len;
        self.head = (self.head + 1) % HIST_SIZE;
        if self.count < HIST_SIZE {
            self.count += 1;
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Get the i-th most recent entry (0 = newest).
    /// Returns (data_slice, len).
    pub fn get(&self, age: usize) -> Option<(&[u8], usize)> {
        if age >= self.count { return None; }
        let idx = if self.head > age {
            self.head - 1 - age
        } else {
            HIST_SIZE - 1 - (age - self.head)
        };
        let len = self.lengths[idx];
        Some((&self.entries[idx][..len], len))
    }

    /// Get entry by absolute index (0 = oldest).
    pub fn get_absolute(&self, index: usize) -> Option<(&[u8], usize)> {
        if index >= self.count { return None; }
        // Oldest entry index
        let oldest = if self.count < HIST_SIZE {
            0
        } else {
            self.head % HIST_SIZE
        };
        let idx = (oldest + index) % HIST_SIZE;
        let len = self.lengths[idx];
        Some((&self.entries[idx][..len], len))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// LineEditor result
// ═══════════════════════════════════════════════════════════════════════════

/// What happened when the user finished a line.
pub enum LineResult {
    /// User pressed Enter — line is ready.
    Line,
    /// User pressed Ctrl-C — line abandoned.
    Interrupt,
    /// User pressed Ctrl-D on empty line — EOF / logout.
    Eof,
}

// ═══════════════════════════════════════════════════════════════════════════
// LineEditor
// ═══════════════════════════════════════════════════════════════════════════

/// Readline-style line editor with cursor movement and history.
pub struct LineEditor {
    /// Current edit buffer.
    pub buf: [u8; MAX_LINE],
    /// Number of bytes in the buffer.
    pub len: usize,
    /// Cursor position within the line (0..=len).
    pub cursor: usize,
    /// History ring buffer (shared across invocations).
    pub history: History,
    /// History navigation index (0 = current line, 1 = last cmd, ...).
    hist_idx: usize,
    /// Saved current line when navigating history.
    saved_buf: [u8; MAX_LINE],
    saved_len: usize,
}

impl LineEditor {
    pub const fn new() -> Self {
        Self {
            buf: [0u8; MAX_LINE],
            len: 0,
            cursor: 0,
            history: History::new(),
            hist_idx: 0,
            saved_buf: [0u8; MAX_LINE],
            saved_len: 0,
        }
    }

    /// Read one line from the console.  Prompt is already printed.
    /// Returns the result type.  On `Line`, the buffer `buf[..len]` holds
    /// the entered text.
    pub fn read_line<S: Serial>(&mut self, con: &mut Console<S>) -> LineResult {
        self.len = 0;
        self.cursor = 0;
        self.hist_idx = 0;

        loop {
            let byte = con.read_byte();

            match byte {
                // ── Enter ────────────────────────────────────────
                CR | LF => {
                    // Record in history before returning
                    self.history.push(&self.buf, self.len);
                    return LineResult::Line;
                }

                // ── Ctrl-C ───────────────────────────────────────
                ETX => {
                    con.write_str_raw("^C");
                    return LineResult::Interrupt;
                }

                // ── Ctrl-D ───────────────────────────────────────
                EOT => {
                    if self.len == 0 {
                        return LineResult::Eof;
                    }
                    // Non-empty line: delete char under cursor (like readline)
                    self.delete_at_cursor(con);
                }

                // ── Backspace ────────────────────────────────────
                BS | DEL => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                        self.delete_at_cursor(con);
                    }
                }

                // ── Ctrl-A (Home) ────────────────────────────────
                0x01 => {
                    self.move_cursor_to(con, 0);
                }

                // ── Ctrl-E (End) ─────────────────────────────────
                0x05 => {
                    self.move_cursor_to(con, self.len);
                }

                // ── Ctrl-B (Back) ────────────────────────────────
                0x02 => {
                    if self.cursor > 0 {
                        self.move_cursor_to(con, self.cursor - 1);
                    }
                }

                // ── Ctrl-F (Forward) ─────────────────────────────
                0x06 => {
                    if self.cursor < self.len {
                        self.move_cursor_to(con, self.cursor + 1);
                    }
                }

                // ── Ctrl-U (Kill to beginning) ───────────────────
                0x15 => {
                    if self.cursor > 0 {
                        let del = self.cursor;
                        // Shift remainder left
                        let remain = self.len - self.cursor;
                        for i in 0..remain {
                            self.buf[i] = self.buf[del + i];
                        }
                        self.len -= del;
                        self.cursor = 0;
                        self.redraw_line(con);
                    }
                }

                // ── Ctrl-K (Kill to end) ─────────────────────────
                0x0B => {
                    if self.cursor < self.len {
                        self.len = self.cursor;
                        // Clear from cursor to end of old line
                        con.write_str_raw("\x1B[K");
                    }
                }

                // ── Ctrl-W (Kill word backward) ──────────────────
                0x17 => {
                    self.kill_word_backward(con);
                }

                // ── Ctrl-T (Transpose) ───────────────────────────
                0x14 => {
                    if self.cursor > 0 && self.cursor < self.len {
                        let a = self.cursor - 1;
                        self.buf.swap(a, self.cursor);
                        self.cursor += 1;
                        self.redraw_from(con, a);
                    }
                }

                // ── Ctrl-L (Clear screen) ────────────────────────
                0x0C => {
                    con.write_str_raw("\x1B[2J\x1B[H");
                    return LineResult::Interrupt; // re-prompt
                }

                // ── Tab — placeholder ────────────────────────────
                TAB => { /* TODO: auto-completion */ }

                // ── ESC sequences ────────────────────────────────
                ESC => {
                    self.handle_escape(con);
                }

                // ── Printable ASCII ──────────────────────────────
                0x20..=0x7E => {
                    self.insert_char(con, byte);
                }

                _ => {} // ignore
            }
        }
    }

    // ── Escape sequence handling ─────────────────────────────────────

    fn handle_escape<S: Serial>(&mut self, con: &mut Console<S>) {
        if !con.has_data() { return; }
        let b2 = con.read_byte();

        match b2 {
            b'[' => {
                // CSI sequence: ESC [ ...
                if !con.has_data() { return; }
                let b3 = con.read_byte();
                match b3 {
                    b'A' => self.history_prev(con),      // Up arrow
                    b'B' => self.history_next(con),      // Down arrow
                    b'C' => {                            // Right arrow
                        if self.cursor < self.len {
                            self.move_cursor_to(con, self.cursor + 1);
                        }
                    }
                    b'D' => {                            // Left arrow
                        if self.cursor > 0 {
                            self.move_cursor_to(con, self.cursor - 1);
                        }
                    }
                    b'H' => self.move_cursor_to(con, 0),       // Home
                    b'F' => self.move_cursor_to(con, self.len), // End
                    b'1' | b'7' => {
                        // ESC [ 1 ~ (Home) or ESC [ 7 ~ (Home)
                        if con.has_data() { let _ = con.read_byte(); } // consume '~'
                        self.move_cursor_to(con, 0);
                    }
                    b'4' | b'8' => {
                        // ESC [ 4 ~ (End) or ESC [ 8 ~ (End)
                        if con.has_data() { let _ = con.read_byte(); }
                        self.move_cursor_to(con, self.len);
                    }
                    b'3' => {
                        // ESC [ 3 ~ (Delete key)
                        if con.has_data() { let _ = con.read_byte(); }
                        self.delete_at_cursor(con);
                    }
                    _ => {} // unknown CSI
                }
            }
            b'b' => {
                // Alt-b: word backward
                self.move_cursor_to(con, self.word_back_pos());
            }
            b'f' => {
                // Alt-f: word forward
                self.move_cursor_to(con, self.word_fwd_pos());
            }
            b'd' => {
                // Alt-d: kill word forward
                self.kill_word_forward(con);
            }
            _ => {} // unknown escape
        }
    }

    // ── Edit operations ──────────────────────────────────────────────

    fn insert_char<S: Serial>(&mut self, con: &mut Console<S>, byte: u8) {
        if self.len >= MAX_LINE - 1 { return; }

        if self.cursor == self.len {
            // Append at end — fast path, just echo
            self.buf[self.len] = byte;
            self.len += 1;
            self.cursor += 1;
            let ch = [byte];
            if let Ok(s) = core::str::from_utf8(&ch) {
                con.write_str_raw(s);
            }
        } else {
            // Insert in middle — shift right and redraw from cursor
            let mut i = self.len;
            while i > self.cursor {
                self.buf[i] = self.buf[i - 1];
                i -= 1;
            }
            self.buf[self.cursor] = byte;
            self.len += 1;
            self.cursor += 1;
            self.redraw_from(con, self.cursor - 1);
        }
    }

    fn delete_at_cursor<S: Serial>(&mut self, con: &mut Console<S>) {
        if self.cursor >= self.len { return; }
        let mut i = self.cursor;
        while i + 1 < self.len {
            self.buf[i] = self.buf[i + 1];
            i += 1;
        }
        self.len -= 1;
        self.redraw_from(con, self.cursor);
    }

    fn kill_word_backward<S: Serial>(&mut self, con: &mut Console<S>) {
        if self.cursor == 0 { return; }
        let old = self.cursor;
        // Skip spaces
        while self.cursor > 0 && self.buf[self.cursor - 1] == b' ' {
            self.cursor -= 1;
        }
        // Skip word chars
        while self.cursor > 0 && self.buf[self.cursor - 1] != b' ' {
            self.cursor -= 1;
        }
        let del = old - self.cursor;
        // Shift remainder
        let remain = self.len - old;
        for i in 0..remain {
            self.buf[self.cursor + i] = self.buf[old + i];
        }
        self.len -= del;
        self.redraw_line(con);
    }

    fn kill_word_forward<S: Serial>(&mut self, con: &mut Console<S>) {
        if self.cursor >= self.len { return; }
        let start = self.cursor;
        let mut end = self.cursor;
        // Skip word chars
        while end < self.len && self.buf[end] != b' ' {
            end += 1;
        }
        // Skip spaces
        while end < self.len && self.buf[end] == b' ' {
            end += 1;
        }
        let del = end - start;
        let remain = self.len - end;
        for i in 0..remain {
            self.buf[start + i] = self.buf[end + i];
        }
        self.len -= del;
        self.redraw_from(con, self.cursor);
    }

    // ── History ──────────────────────────────────────────────────────

    fn history_prev<S: Serial>(&mut self, con: &mut Console<S>) {
        if self.hist_idx >= self.history.len() { return; }

        // Save current line if at idx 0
        if self.hist_idx == 0 {
            self.saved_buf[..self.len].copy_from_slice(&self.buf[..self.len]);
            self.saved_len = self.len;
        }

        if let Some((data, hlen)) = self.history.get(self.hist_idx) {
            // Move cursor to start of input
            for _ in 0..self.cursor { con.write_str_raw("\x08"); }
            con.write_str_raw("\x1B[K");
            // Load history entry
            self.buf[..hlen].copy_from_slice(&data[..hlen]);
            self.len = hlen;
            self.cursor = hlen;
            self.hist_idx += 1;
            // Re-emit buffer
            if hlen > 0 {
                let s = unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) };
                con.write_str_raw(s);
            }
        }
    }

    fn history_next<S: Serial>(&mut self, con: &mut Console<S>) {
        if self.hist_idx == 0 { return; }

        self.hist_idx -= 1;

        // Move cursor to start of input
        for _ in 0..self.cursor { con.write_str_raw("\x08"); }
        con.write_str_raw("\x1B[K");

        if self.hist_idx == 0 {
            // Restore saved line
            self.buf[..self.saved_len].copy_from_slice(&self.saved_buf[..self.saved_len]);
            self.len = self.saved_len;
            self.cursor = self.saved_len;
        } else {
            if let Some((data, hlen)) = self.history.get(self.hist_idx - 1) {
                self.buf[..hlen].copy_from_slice(&data[..hlen]);
                self.len = hlen;
                self.cursor = hlen;
            }
        }
        // Re-emit buffer
        if self.len > 0 {
            let s = unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) };
            con.write_str_raw(s);
        }
    }

    // ── Word position helpers ────────────────────────────────────────

    fn word_back_pos(&self) -> usize {
        let mut p = self.cursor;
        // Skip spaces backward
        while p > 0 && self.buf[p - 1] == b' ' { p -= 1; }
        // Skip word chars backward
        while p > 0 && self.buf[p - 1] != b' ' { p -= 1; }
        p
    }

    fn word_fwd_pos(&self) -> usize {
        let mut p = self.cursor;
        // Skip word chars
        while p < self.len && self.buf[p] != b' ' { p += 1; }
        // Skip spaces
        while p < self.len && self.buf[p] == b' ' { p += 1; }
        p
    }

    // ── Terminal drawing helpers ─────────────────────────────────────

    /// Move terminal cursor to a new buffer position.
    fn move_cursor_to<S: Serial>(&mut self, con: &mut Console<S>, new_pos: usize) {
        if new_pos == self.cursor { return; }
        if new_pos < self.cursor {
            // Move left
            let n = self.cursor - new_pos;
            for _ in 0..n {
                con.write_str_raw("\x08"); // BS
            }
        } else {
            // Move right — emit the characters (so they display properly)
            for i in self.cursor..new_pos {
                let ch = [self.buf[i]];
                if let Ok(s) = core::str::from_utf8(&ch) {
                    con.write_str_raw(s);
                }
            }
        }
        self.cursor = new_pos;
    }

    /// Redraw from position `from` to end, then reposition cursor.
    fn redraw_from<S: Serial>(&self, con: &mut Console<S>, from: usize) {
        // Move to `from`
        if from < self.cursor {
            let n = self.cursor - from;
            for _ in 0..n { con.write_str_raw("\x08"); }
        }
        // Print from `from` to end of buffer
        for i in from..self.len {
            let ch = [self.buf[i]];
            if let Ok(s) = core::str::from_utf8(&ch) {
                con.write_str_raw(s);
            }
        }
        // Clear any trailing chars from old content
        con.write_str_raw(" \x08"); // space-back clears ghost char
        con.write_str_raw("\x1B[K"); // clear to end of line
        // Move back to cursor position
        let back = self.len - self.cursor;
        for _ in 0..back { con.write_str_raw("\x08"); }
    }

    /// Redraw entire line from scratch.
    /// Uses the stored prompt_len to know where input starts.
    fn redraw_line<S: Serial>(&self, con: &mut Console<S>) {
        // Move cursor to start of input (back up cursor positions)
        for _ in 0..self.cursor { con.write_str_raw("\x08"); }
        // Clear from here to end of line
        con.write_str_raw("\x1B[K");
        // Re-emit the buffer
        if self.len > 0 {
            let s = unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) };
            con.write_str_raw(s);
        }
        // Move back to cursor position
        let back = self.len - self.cursor;
        for _ in 0..back { con.write_str_raw("\x08"); }
    }

    /// Redraw entire line assuming terminal cursor is at column 0.
    /// Caller provides prompt string so we can reprint it.
    pub fn redraw_line_with_prompt<S: Serial>(&self, con: &mut Console<S>, prompt: &str) {
        // Clear entire line
        con.write_str_raw("\r\x1B[2K");
        // Print prompt
        con.write_str_raw(prompt);
        // Print buffer content
        if self.len > 0 {
            // SAFETY: we only store ASCII printable bytes.
            let s = unsafe { core::str::from_utf8_unchecked(&self.buf[..self.len]) };
            con.write_str_raw(s);
        }
        // Position cursor
        let back = self.len - self.cursor;
        for _ in 0..back { con.write_str_raw("\x08"); }
    }
}

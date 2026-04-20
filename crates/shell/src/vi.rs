//! VeerOS built-in vi editor.
//!
//! A minimal modal text editor inspired by vi/vim, running entirely in
//! `no_std` on a serial console.  Supports Normal, Insert, and Command
//! modes with the most commonly used vi key-bindings.

use core::fmt::Write;
use arch::{Console, Serial};

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum number of lines in the edit buffer.
#[cfg(feature = "small-vi")]
const MAX_LINES: usize = 48;
#[cfg(feature = "large-vi")]
const MAX_LINES: usize = 512;
#[cfg(not(any(feature = "small-vi", feature = "large-vi")))]
const MAX_LINES: usize = 128;

/// Maximum bytes per line (excluding terminator).
#[cfg(feature = "large-vi")]
const MAX_COLS: usize = 120;
#[cfg(not(feature = "large-vi"))]
const MAX_COLS: usize = 80;

/// Terminal height (rows) — conservative default for serial consoles.
const TERM_ROWS: usize = 24;

/// Terminal width (cols).
const TERM_COLS: usize = 80;

/// Visible text rows (minus 1 for status line).
const TEXT_ROWS: usize = TERM_ROWS - 1;

// ASCII / control codes
const ESC: u8 = 0x1B;
const CR: u8 = 0x0D;
const LF: u8 = 0x0A;
const BS: u8 = 0x08;
const DEL: u8 = 0x7F;
const TAB: u8 = 0x09;

// ═══════════════════════════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════════════════════════

/// Editor mode.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Insert,
    Replace,
    Command,
    Search,
}

/// Editor settings (configurable via `:set`).
pub struct ViSettings {
    /// Show line numbers.
    pub number: bool,
    /// Tab-stop width.
    pub tabstop: u8,
    /// Flash matching bracket.
    pub showmatch: bool,
    /// Auto-indent new lines.
    pub autoindent: bool,
    /// Show mode indicator.
    pub showmode: bool,
}

impl ViSettings {
    pub const fn default() -> Self {
        Self {
            number: false,
            tabstop: 4,
            showmatch: false,
            autoindent: false,
            showmode: true,
        }
    }
}

/// Records the last editing action for `.` repeat.
#[derive(Clone, Copy)]
struct LastEdit {
    kind: EditKind,
    ch: u8,
    count: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum EditKind {
    None,
    DeleteChar,    // x
    DeleteLine,    // dd
    ChangeLine,    // cc (delete line + insert)
    ReplaceChar,   // r<ch>
    PutBelow,      // p
    PutAbove,      // P
    JoinLine,      // J
    IndentRight,   // >>
    IndentLeft,    // <<
    ToggleCase,    // ~
    DeleteToEnd,   // D
}

/// A single line in the buffer.
struct Line {
    data: [u8; MAX_COLS],
    len: usize,
}

impl Line {
    const fn new() -> Self {
        Self { data: [0u8; MAX_COLS], len: 0 }
    }

    fn as_str(&self) -> &str {
        // Safety: we only store ASCII printable bytes + spaces.
        unsafe { core::str::from_utf8_unchecked(&self.data[..self.len]) }
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    /// Insert a byte at position `col`, shifting right.
    fn insert(&mut self, col: usize, byte: u8) -> bool {
        if self.len >= MAX_COLS - 1 { return false; }
        let col = if col > self.len { self.len } else { col };
        // shift right
        let mut i = self.len;
        while i > col {
            self.data[i] = self.data[i - 1];
            i -= 1;
        }
        self.data[col] = byte;
        self.len += 1;
        true
    }

    /// Delete the byte at `col`, shifting left.  Returns deleted byte.
    fn delete(&mut self, col: usize) -> Option<u8> {
        if col >= self.len { return None; }
        let ch = self.data[col];
        let mut i = col;
        while i + 1 < self.len {
            self.data[i] = self.data[i + 1];
            i += 1;
        }
        self.len -= 1;
        Some(ch)
    }

    /// Append bytes from a slice.
    fn push_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.len < MAX_COLS {
                self.data[self.len] = b;
                self.len += 1;
            }
        }
    }

    /// Split this line at `col`.  Returns the right half as a new Line.
    fn split_at(&mut self, col: usize) -> Line {
        let mut right = Line::new();
        if col < self.len {
            right.push_bytes(&self.data[col..self.len]);
            self.len = col;
        }
        right
    }

    /// Copy contents from another line.
    fn copy_from(&mut self, other: &Line) {
        self.data[..other.len].copy_from_slice(&other.data[..other.len]);
        self.len = other.len;
    }
}

/// The vi editor state.
pub struct Vi {
    /// Text buffer — array of lines.
    lines: [Line; MAX_LINES],
    /// Number of lines in use (always >= 1).
    num_lines: usize,
    /// Cursor row (0-based, within document).
    row: usize,
    /// Cursor column (0-based).
    col: usize,
    /// First visible line (scroll offset).
    scroll: usize,
    /// Current editor mode.
    mode: Mode,
    /// Command-line buffer (for ':' commands).
    cmd_buf: [u8; 64],
    cmd_len: usize,
    /// Yank (copy) buffer — one line.
    yank: Line,
    /// Whether the buffer has been modified.
    dirty: bool,
    /// Status message to display.
    status: [u8; 64],
    status_len: usize,
    /// Filename (if any).
    filename: [u8; 32],
    filename_len: usize,
    /// Editor settings.
    pub settings: ViSettings,
    /// Search pattern buffer.
    search_buf: [u8; 48],
    search_len: usize,
    /// Search direction: true = forward, false = backward.
    search_fwd: bool,
    /// Pending count prefix (e.g., 3dd).
    count: usize,
    /// Accumulating count digits.
    counting: bool,
    /// Last edit for `.` repeat.
    last_edit: LastEdit,
}

impl Vi {
    /// Create a new, empty editor.
    pub fn new() -> Self {
        const EMPTY: Line = Line::new();
        Self {
            lines: [EMPTY; MAX_LINES],
            num_lines: 1,
            row: 0,
            col: 0,
            scroll: 0,
            mode: Mode::Normal,
            cmd_buf: [0u8; 64],
            cmd_len: 0,
            yank: Line::new(),
            dirty: false,
            status: [0u8; 64],
            status_len: 0,
            filename: [0u8; 32],
            filename_len: 0,
            settings: ViSettings::default(),
            search_buf: [0u8; 48],
            search_len: 0,
            search_fwd: true,
            count: 0,
            counting: false,
            last_edit: LastEdit { kind: EditKind::None, ch: 0, count: 1 },
        }
    }

    /// Load text content into the editor buffer.
    pub fn load(&mut self, text: &str) {
        self.num_lines = 0;
        self.row = 0;
        self.col = 0;
        self.scroll = 0;
        self.dirty = false;

        for line_str in text.split('\n') {
            if self.num_lines >= MAX_LINES { break; }
            self.lines[self.num_lines].clear();
            let bytes = line_str.as_bytes();
            let take = if bytes.len() > MAX_COLS { MAX_COLS } else { bytes.len() };
            // Strip trailing CR if present
            let take = if take > 0 && bytes[take - 1] == CR { take - 1 } else { take };
            self.lines[self.num_lines].push_bytes(&bytes[..take]);
            self.num_lines += 1;
        }

        if self.num_lines == 0 {
            self.num_lines = 1; // always at least one line
        }
    }

    /// Set the filename shown in status.
    pub fn set_filename(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let take = if bytes.len() > 32 { 32 } else { bytes.len() };
        self.filename[..take].copy_from_slice(&bytes[..take]);
        self.filename_len = take;
    }

    /// Retrieve the current buffer contents as a byte count.
    pub fn content_bytes(&self) -> usize {
        let mut total = 0usize;
        for i in 0..self.num_lines {
            total += self.lines[i].len;
            if i + 1 < self.num_lines {
                total += 1; // newline
            }
        }
        total
    }

    /// Write the buffer contents to a `fmt::Write` sink.
    pub fn write_contents(&self, w: &mut dyn Write) {
        for i in 0..self.num_lines {
            let _ = w.write_str(self.lines[i].as_str());
            if i + 1 < self.num_lines {
                let _ = w.write_char('\n');
            }
        }
    }

    // ── Main editor loop ─────────────────────────────────────────────

    /// Run the interactive editor.  Returns `true` if the user saved.
    pub fn run<S: Serial>(&mut self, con: &mut Console<S>) -> bool {
        // Hide cursor, clear screen, initial draw
        self.full_redraw(con);
        let mut saved = false;

        loop {
            let byte = con.read_byte();

            match self.mode {
                Mode::Normal  => {
                    if self.handle_normal(con, byte) {
                        break;
                    }
                }
                Mode::Insert   => self.handle_insert(con, byte),
                Mode::Replace  => self.handle_replace(con, byte),
                Mode::Command  => {
                    match self.handle_command(con, byte) {
                        CmdResult::Continue => {}
                        CmdResult::Quit => break,
                        CmdResult::SaveQuit => { saved = true; break; }
                    }
                }
                Mode::Search   => self.handle_search_input(con, byte),
            }
        }

        // Restore screen
        Self::vt_clear(con);
        Self::vt_cursor(con, 1, 1);
        Self::vt_show_cursor(con);

        saved
    }

    // ── Normal mode ──────────────────────────────────────────────────

    /// Handle a keystroke in Normal mode.
    /// Returns `true` if the editor should quit (ZZ).
    fn handle_normal<S: Serial>(&mut self, con: &mut Console<S>, byte: u8) -> bool {
        // ── Count prefix accumulation ────────────────────────────
        if byte >= b'1' && byte <= b'9' && !self.counting {
            self.count = (byte - b'0') as usize;
            self.counting = true;
            return false;
        }
        if byte >= b'0' && byte <= b'9' && self.counting {
            self.count = self.count.saturating_mul(10).saturating_add((byte - b'0') as usize);
            return false;
        }

        let cnt = if self.counting { self.count } else { 1 };
        self.counting = false;
        self.count = 0;

        match byte {
            // ── Movement ─────────────────────────────────────────
            b'h' => { for _ in 0..cnt { if self.col > 0 { self.col -= 1; } } }
            b'j' => { for _ in 0..cnt { self.move_down(); } }
            b'k' => { for _ in 0..cnt { self.move_up(); } }
            b'l' => { for _ in 0..cnt { self.move_right(); } }
            b'0' => { self.col = 0; }
            b'^' => { self.col = self.first_nonblank(); }
            b'$' => { self.col = self.end_of_line(); }
            b'w' => { for _ in 0..cnt { self.word_forward(); } }
            b'b' => { for _ in 0..cnt { self.word_backward(); } }
            b'e' => { for _ in 0..cnt { self.word_end(); } }

            b'G' => {
                if cnt > 1 || self.counting {
                    // <N>G = go to line N
                    let target = if cnt <= self.num_lines { cnt - 1 } else { self.num_lines - 1 };
                    self.row = target;
                } else {
                    self.row = self.num_lines - 1;
                }
                self.clamp_col();
            }
            b'g' => {
                let next = con.read_byte();
                if next == b'g' {
                    self.row = 0;
                    self.col = 0;
                }
            }

            // ── Screen-relative movement ─────────────────────────
            b'H' => {
                self.row = self.scroll;
                self.col = self.first_nonblank();
            }
            b'M' => {
                let mid = self.scroll + TEXT_ROWS / 2;
                self.row = core::cmp::min(mid, self.num_lines - 1);
                self.col = self.first_nonblank();
            }
            b'L' => {
                let bot = core::cmp::min(self.scroll + TEXT_ROWS - 1, self.num_lines - 1);
                self.row = bot;
                self.col = self.first_nonblank();
            }

            // ── Scrolling ────────────────────────────────────────
            0x06 => { // Ctrl-F
                let step = TEXT_ROWS.saturating_sub(2);
                for _ in 0..cnt {
                    self.row = core::cmp::min(self.row + step, self.num_lines - 1);
                }
                self.clamp_col();
            }
            0x02 => { // Ctrl-B
                let step = TEXT_ROWS.saturating_sub(2);
                for _ in 0..cnt { self.row = self.row.saturating_sub(step); }
                self.clamp_col();
            }
            0x04 => { // Ctrl-D (half page down)
                let step = TEXT_ROWS / 2;
                for _ in 0..cnt {
                    self.row = core::cmp::min(self.row + step, self.num_lines - 1);
                }
                self.clamp_col();
            }
            0x15 => { // Ctrl-U (half page up)
                let step = TEXT_ROWS / 2;
                for _ in 0..cnt { self.row = self.row.saturating_sub(step); }
                self.clamp_col();
            }

            // ── Find char in line ────────────────────────────────
            b'f' => {
                let ch = con.read_byte();
                if ch >= 0x20 {
                    for _ in 0..cnt { self.find_char_fwd(ch); }
                }
            }
            b'F' => {
                let ch = con.read_byte();
                if ch >= 0x20 {
                    for _ in 0..cnt { self.find_char_back(ch); }
                }
            }
            b't' => {
                let ch = con.read_byte();
                if ch >= 0x20 {
                    for _ in 0..cnt { self.find_char_fwd(ch); }
                    if self.col > 0 { self.col -= 1; }
                }
            }
            b'T' => {
                let ch = con.read_byte();
                if ch >= 0x20 {
                    for _ in 0..cnt { self.find_char_back(ch); }
                    if self.col < self.lines[self.row].len.saturating_sub(1) { self.col += 1; }
                }
            }

            // ── Search ───────────────────────────────────────────
            b'/' => {
                self.mode = Mode::Search;
                self.search_fwd = true;
                self.cmd_len = 0;
            }
            b'?' => {
                self.mode = Mode::Search;
                self.search_fwd = false;
                self.cmd_len = 0;
            }
            b'n' => { for _ in 0..cnt { self.search_next(); } }
            b'N' => { for _ in 0..cnt { self.search_prev(); } }
            b'*' => {
                // Search for word under cursor
                self.search_word_under_cursor();
            }

            // ── Insert entry ─────────────────────────────────────
            b'i' => {
                self.mode = Mode::Insert;
                self.set_status("-- INSERT --");
            }
            b'a' => {
                if self.lines[self.row].len > 0 {
                    self.col = core::cmp::min(self.col + 1, self.lines[self.row].len);
                }
                self.mode = Mode::Insert;
                self.set_status("-- INSERT --");
            }
            b'A' => {
                self.col = self.lines[self.row].len;
                self.mode = Mode::Insert;
                self.set_status("-- INSERT --");
            }
            b'I' => {
                self.col = self.first_nonblank();
                self.mode = Mode::Insert;
                self.set_status("-- INSERT --");
            }
            b'o' => {
                if self.insert_line_below() {
                    self.row += 1;
                    self.col = 0;
                    if self.settings.autoindent {
                        let indent = self.get_indent(self.row.saturating_sub(1));
                        self.apply_indent(self.row, indent);
                        self.col = indent;
                    }
                    self.mode = Mode::Insert;
                    self.set_status("-- INSERT --");
                }
            }
            b'O' => {
                if self.insert_line_at(self.row) {
                    self.col = 0;
                    if self.settings.autoindent {
                        let indent = self.get_indent(if self.row + 1 < self.num_lines { self.row + 1 } else { self.row });
                        self.apply_indent(self.row, indent);
                        self.col = indent;
                    }
                    self.mode = Mode::Insert;
                    self.set_status("-- INSERT --");
                }
            }

            // ── Replace mode ─────────────────────────────────────
            b'R' => {
                self.mode = Mode::Replace;
                self.set_status("-- REPLACE --");
            }

            // ── Editing ──────────────────────────────────────────
            b'x' => {
                for _ in 0..cnt {
                    if self.lines[self.row].len > 0 {
                        self.lines[self.row].delete(self.col);
                        self.dirty = true;
                        self.clamp_col();
                    }
                }
                self.last_edit = LastEdit { kind: EditKind::DeleteChar, ch: 0, count: cnt };
            }
            b'r' => {
                let ch = con.read_byte();
                if ch >= 0x20 && ch <= 0x7E && self.col < self.lines[self.row].len {
                    for i in 0..cnt {
                        let pos = self.col + i;
                        if pos < self.lines[self.row].len {
                            self.lines[self.row].data[pos] = ch;
                        }
                    }
                    self.dirty = true;
                    self.last_edit = LastEdit { kind: EditKind::ReplaceChar, ch, count: cnt };
                }
            }
            b'~' => {
                // Toggle case of char under cursor
                for _ in 0..cnt {
                    if self.col < self.lines[self.row].len {
                        let c = self.lines[self.row].data[self.col];
                        self.lines[self.row].data[self.col] = if c >= b'a' && c <= b'z' {
                            c - 32
                        } else if c >= b'A' && c <= b'Z' {
                            c + 32
                        } else {
                            c
                        };
                        self.dirty = true;
                        self.move_right();
                    }
                }
                self.last_edit = LastEdit { kind: EditKind::ToggleCase, ch: 0, count: cnt };
            }

            b'd' => {
                let next = con.read_byte();
                match next {
                    b'd' => {
                        // dd — delete N lines
                        for _ in 0..cnt {
                            self.yank.copy_from(&self.lines[self.row]);
                            self.delete_line(self.row);
                            self.dirty = true;
                            self.clamp_row();
                            self.clamp_col();
                        }
                        self.last_edit = LastEdit { kind: EditKind::DeleteLine, ch: 0, count: cnt };
                    }
                    b'w' => {
                        // dw — delete word(s)
                        for _ in 0..cnt {
                            self.delete_word();
                        }
                    }
                    b'$' => {
                        // d$ — delete to end of line (same as D)
                        self.delete_to_end();
                    }
                    b'0' => {
                        // d0 — delete to beginning of line
                        self.delete_to_start();
                    }
                    _ => {}
                }
            }
            b'D' => {
                // Delete from cursor to end of line
                self.delete_to_end();
                self.last_edit = LastEdit { kind: EditKind::DeleteToEnd, ch: 0, count: 1 };
            }
            b'C' => {
                // Change from cursor to end of line
                self.delete_to_end();
                self.mode = Mode::Insert;
                self.set_status("-- INSERT --");
            }
            b'c' => {
                let next = con.read_byte();
                match next {
                    b'c' => {
                        // cc — change entire line
                        self.lines[self.row].clear();
                        self.col = 0;
                        self.dirty = true;
                        self.mode = Mode::Insert;
                        self.set_status("-- INSERT --");
                        self.last_edit = LastEdit { kind: EditKind::ChangeLine, ch: 0, count: 1 };
                    }
                    b'w' => {
                        // cw — change word
                        self.delete_word();
                        self.mode = Mode::Insert;
                        self.set_status("-- INSERT --");
                    }
                    b'$' => {
                        self.delete_to_end();
                        self.mode = Mode::Insert;
                        self.set_status("-- INSERT --");
                    }
                    _ => {}
                }
            }

            b'y' => {
                let next = con.read_byte();
                if next == b'y' {
                    self.yank.copy_from(&self.lines[self.row]);
                    self.set_status("1 line yanked");
                }
            }
            b'p' => {
                if self.yank.len > 0 {
                    for _ in 0..cnt {
                        if self.insert_line_below() {
                            self.row += 1;
                            self.lines[self.row].copy_from(&self.yank);
                            self.dirty = true;
                        }
                    }
                    self.last_edit = LastEdit { kind: EditKind::PutBelow, ch: 0, count: cnt };
                }
            }
            b'P' => {
                if self.yank.len > 0 {
                    for _ in 0..cnt {
                        if self.insert_line_at(self.row) {
                            self.lines[self.row].copy_from(&self.yank);
                            self.dirty = true;
                        }
                    }
                    self.last_edit = LastEdit { kind: EditKind::PutAbove, ch: 0, count: cnt };
                }
            }
            b'J' => {
                for _ in 0..cnt {
                    self.join_lines();
                }
                self.last_edit = LastEdit { kind: EditKind::JoinLine, ch: 0, count: cnt };
            }

            // ── Indent / Dedent ──────────────────────────────────
            b'>' => {
                let next = con.read_byte();
                if next == b'>' {
                    let ts = self.settings.tabstop as usize;
                    for i in 0..cnt {
                        let r = self.row + i;
                        if r < self.num_lines { self.indent_line(r, ts); }
                    }
                    self.dirty = true;
                    self.last_edit = LastEdit { kind: EditKind::IndentRight, ch: 0, count: cnt };
                }
            }
            b'<' => {
                let next = con.read_byte();
                if next == b'<' {
                    let ts = self.settings.tabstop as usize;
                    for i in 0..cnt {
                        let r = self.row + i;
                        if r < self.num_lines { self.dedent_line(r, ts); }
                    }
                    self.dirty = true;
                    self.last_edit = LastEdit { kind: EditKind::IndentLeft, ch: 0, count: cnt };
                }
            }

            // ── Bracket matching ─────────────────────────────────
            b'%' => {
                self.match_bracket();
            }

            // ── Repeat last edit ─────────────────────────────────
            b'.' => {
                self.repeat_last_edit();
            }

            b'u' => {
                self.set_status("undo not available");
            }

            // ── Command mode entry ───────────────────────────────
            b':' => {
                self.mode = Mode::Command;
                self.cmd_len = 0;
            }

            // ── Quick save/quit ──────────────────────────────────
            b'Z' => {
                let next = con.read_byte();
                if next == b'Z' {
                    return true;
                } else if next == b'Q' {
                    return true;
                }
            }

            // ── Escape sequences from terminal ───────────────────
            ESC => {
                if con.has_data() {
                    let b2 = con.read_byte();
                    if b2 == b'[' && con.has_data() {
                        let arrow = con.read_byte();
                        match arrow {
                            b'A' => { for _ in 0..cnt { self.move_up(); } }
                            b'B' => { for _ in 0..cnt { self.move_down(); } }
                            b'C' => { for _ in 0..cnt { self.move_right(); } }
                            b'D' => { for _ in 0..cnt { if self.col > 0 { self.col -= 1; } } }
                            _ => {}
                        }
                    }
                }
            }

            _ => {}
        }

        self.adjust_scroll();
        self.draw_screen(con);
        false
    }

    // ── Insert mode ──────────────────────────────────────────────────

    fn handle_insert<S: Serial>(&mut self, con: &mut Console<S>, byte: u8) {
        match byte {
            ESC => {
                // Return to normal mode
                self.mode = Mode::Normal;
                if self.col > 0 { self.col -= 1; }
                self.set_status("");
            }

            CR | LF => {
                // Split the current line at cursor
                let right = self.lines[self.row].split_at(self.col);
                if self.insert_line_below() {
                    self.row += 1;
                    self.lines[self.row].copy_from(&right);
                    self.col = 0;
                    self.dirty = true;
                }
            }

            BS | DEL => {
                if self.col > 0 {
                    self.col -= 1;
                    self.lines[self.row].delete(self.col);
                    self.dirty = true;
                } else if self.row > 0 {
                    // Join with previous line
                    let prev = self.row - 1;
                    let new_col = self.lines[prev].len;
                    // Append current line to prev
                    let cur_len = self.lines[self.row].len;
                    let avail = MAX_COLS - self.lines[prev].len;
                    let take = if cur_len < avail { cur_len } else { avail };
                    if take > 0 {
                        let start = self.lines[prev].len;
                        for i in 0..take {
                            self.lines[prev].data[start + i] = self.lines[self.row].data[i];
                        }
                        self.lines[prev].len += take;
                    }
                    self.delete_line(self.row);
                    self.row = prev;
                    self.col = new_col;
                    self.dirty = true;
                }
            }

            TAB => {
                // Insert spaces (no real tab support)
                for _ in 0..4 {
                    if self.lines[self.row].insert(self.col, b' ') {
                        self.col += 1;
                        self.dirty = true;
                    }
                }
            }

            0x20..=0x7E => {
                if self.lines[self.row].insert(self.col, byte) {
                    self.col += 1;
                    self.dirty = true;
                }
            }

            _ => {
                // Arrow keys via ESC sequence in insert mode
                if byte == 0x1B && con.has_data() {
                    let b2 = con.read_byte();
                    if b2 == b'[' && con.has_data() {
                        let arrow = con.read_byte();
                        match arrow {
                            b'A' => { self.move_up(); }
                            b'B' => { self.move_down(); }
                            b'C' => { self.move_right(); }
                            b'D' => { if self.col > 0 { self.col -= 1; } }
                            _ => {}
                        }
                    }
                }
            }
        }

        self.adjust_scroll();
        self.draw_screen(con);
    }

    // ── Command mode ─────────────────────────────────────────────────

    fn handle_command<S: Serial>(&mut self, con: &mut Console<S>, byte: u8) -> CmdResult {
        match byte {
            ESC => {
                self.mode = Mode::Normal;
                self.set_status("");
                self.draw_screen(con);
                return CmdResult::Continue;
            }

            CR | LF => {
                // Execute the command
                let result = self.exec_command(con);
                self.mode = Mode::Normal;
                self.adjust_scroll();
                self.draw_screen(con);
                return result;
            }

            BS | DEL => {
                if self.cmd_len > 0 {
                    self.cmd_len -= 1;
                }
                // Redraw status with partial command
                self.draw_status_line(con);
            }

            0x20..=0x7E => {
                if self.cmd_len < 63 {
                    self.cmd_buf[self.cmd_len] = byte;
                    self.cmd_len += 1;
                }
                self.draw_status_line(con);
            }

            _ => {}
        }
        CmdResult::Continue
    }

    /// Execute a ':' command.
    fn exec_command<S: Serial>(&mut self, _con: &mut Console<S>) -> CmdResult {
        // Copy command buffer out to avoid borrow conflicts with self
        let mut cbuf = [0u8; 64];
        let clen = self.cmd_len;
        cbuf[..clen].copy_from_slice(&self.cmd_buf[..clen]);

        let cmd = match core::str::from_utf8(&cbuf[..clen]) {
            Ok(s) => s.trim(),
            Err(_) => {
                self.set_status("invalid command");
                return CmdResult::Continue;
            }
        };

        // :q  — quit (if not dirty)
        if cmd == "q" {
            if self.dirty {
                self.set_status("No write since last change (use :q! to override)");
                return CmdResult::Continue;
            }
            return CmdResult::Quit;
        }
        // :q!  — force quit
        if cmd == "q!" {
            return CmdResult::Quit;
        }
        // :w  — write (just mark saved, no filesystem in VeerOS yet)
        if cmd == "w" {
            self.dirty = false;
            let bytes = self.content_bytes();
            self.set_status_fmt(bytes, "bytes written");
            return CmdResult::Continue;
        }
        // :wq or :x  — write and quit
        if cmd == "wq" || cmd == "x" {
            self.dirty = false;
            return CmdResult::SaveQuit;
        }
        // :set — view/change settings
        if cmd == "set" {
            // Show current settings
            let mut tmp = [0u8; 64];
            let mut pos = 0;
            let s: &[u8] = if self.settings.number { b"number " } else { b"nonumber " };
            for &b in s {
                if pos < 64 { tmp[pos] = b; pos += 1; }
            }
            for &b in b"ts=" {
                if pos < 64 { tmp[pos] = b; pos += 1; }
            }
            let mut nbuf = [0u8; 8];
            let n = fmt_usize(&mut nbuf, self.settings.tabstop as usize);
            for i in 0..n {
                if pos < 64 { tmp[pos] = nbuf[i]; pos += 1; }
            }
            if pos < 64 { tmp[pos] = b' '; pos += 1; }
            let s: &[u8] = if self.settings.autoindent { b"ai " } else { b"noai " };
            for &b in s {
                if pos < 64 { tmp[pos] = b; pos += 1; }
            }
            let s: &[u8] = if self.settings.showmatch { b"sm" } else { b"nosm" };
            for &b in s {
                if pos < 64 { tmp[pos] = b; pos += 1; }
            }
            self.status[..pos].copy_from_slice(&tmp[..pos]);
            self.status_len = pos;
            return CmdResult::Continue;
        }
        if cmd.len() > 4 && cmd.as_bytes()[..4] == *b"set " {
            let arg = cmd[4..].trim();
            if arg == "number" || arg == "nu" {
                self.settings.number = true;
            } else if arg == "nonumber" || arg == "nonu" {
                self.settings.number = false;
            } else if arg == "autoindent" || arg == "ai" {
                self.settings.autoindent = true;
            } else if arg == "noautoindent" || arg == "noai" {
                self.settings.autoindent = false;
            } else if arg == "showmatch" || arg == "sm" {
                self.settings.showmatch = true;
            } else if arg == "noshowmatch" || arg == "nosm" {
                self.settings.showmatch = false;
            } else if arg == "showmode" {
                self.settings.showmode = true;
            } else if arg == "noshowmode" {
                self.settings.showmode = false;
            } else if arg.as_bytes().iter().any(|&b| b == b'=') {
                // Generic key=value for tabstop
                if let Some(eq) = arg.as_bytes().iter().position(|&b| b == b'=') {
                    let key = &arg[..eq];
                    let val_s = &arg[eq + 1..];
                    if key == "tabstop" || key == "ts" {
                        if let Ok(val) = parse_usize(val_s) {
                            if val >= 1 && val <= 16 {
                                self.settings.tabstop = val as u8;
                            } else {
                                self.set_status("tabstop: 1-16");
                                return CmdResult::Continue;
                            }
                        }
                    } else {
                        self.set_status("unknown option");
                        return CmdResult::Continue;
                    }
                }
            } else {
                self.set_status("unknown option");
                return CmdResult::Continue;
            }
            self.set_status("ok");
            return CmdResult::Continue;
        }
        // :s/pat/rep/ — substitute on current line
        if cmd.len() > 2 && cmd.as_bytes()[0] == b's' && cmd.as_bytes()[1] == b'/' {
            self.exec_substitute(cmd);
            return CmdResult::Continue;
        }
        // :<number>  — go to line
        if let Ok(n) = parse_usize(cmd) {
            if n > 0 && n <= self.num_lines {
                self.row = n - 1;
                self.col = 0;
                self.clamp_col();
            } else {
                self.set_status("line out of range");
            }
            return CmdResult::Continue;
        }

        self.set_status("unknown command");
        CmdResult::Continue
    }

    // ── Movement helpers ─────────────────────────────────────────────

    fn move_up(&mut self) {
        if self.row > 0 {
            self.row -= 1;
            self.clamp_col();
        }
    }

    fn move_down(&mut self) {
        if self.row + 1 < self.num_lines {
            self.row += 1;
            self.clamp_col();
        }
    }

    fn move_right(&mut self) {
        let line_end = self.end_of_line();
        if self.col < line_end {
            self.col += 1;
        }
    }

    fn end_of_line(&self) -> usize {
        let len = self.lines[self.row].len;
        if len == 0 { 0 } else { len - 1 }
    }

    fn first_nonblank(&self) -> usize {
        let line = &self.lines[self.row];
        for i in 0..line.len {
            if line.data[i] != b' ' && line.data[i] != TAB {
                return i;
            }
        }
        0
    }

    fn word_forward(&mut self) {
        let line = &self.lines[self.row];
        let mut c = self.col;

        // Skip current word
        while c < line.len && line.data[c] != b' ' {
            c += 1;
        }
        // Skip spaces
        while c < line.len && line.data[c] == b' ' {
            c += 1;
        }

        if c >= line.len && self.row + 1 < self.num_lines {
            // Move to next line
            self.row += 1;
            self.col = self.first_nonblank();
        } else {
            self.col = if c < line.len { c } else { self.end_of_line() };
        }
    }

    fn word_backward(&mut self) {
        let line = &self.lines[self.row];
        let mut c = self.col;

        if c == 0 {
            if self.row > 0 {
                self.row -= 1;
                self.col = self.end_of_line();
            }
            return;
        }

        // Back up over spaces
        while c > 0 && line.data[c - 1] == b' ' {
            c -= 1;
        }
        // Back up over word chars
        while c > 0 && line.data[c - 1] != b' ' {
            c -= 1;
        }
        self.col = c;
    }

    fn clamp_col(&mut self) {
        let max = if self.lines[self.row].len == 0 {
            0
        } else if self.mode == Mode::Insert {
            self.lines[self.row].len
        } else {
            self.lines[self.row].len - 1
        };
        if self.col > max {
            self.col = max;
        }
    }

    fn clamp_row(&mut self) {
        if self.row >= self.num_lines {
            self.row = if self.num_lines > 0 { self.num_lines - 1 } else { 0 };
        }
    }

    fn adjust_scroll(&mut self) {
        if self.row < self.scroll {
            self.scroll = self.row;
        } else if self.row >= self.scroll + TEXT_ROWS {
            self.scroll = self.row - TEXT_ROWS + 1;
        }
    }

    // ── Line manipulation ────────────────────────────────────────────

    fn insert_line_below(&mut self) -> bool {
        self.insert_line_at(self.row + 1)
    }

    fn insert_line_at(&mut self, at: usize) -> bool {
        if self.num_lines >= MAX_LINES { return false; }
        // Shift lines down
        let mut i = self.num_lines;
        while i > at {
            // Use a temp buffer to avoid borrow overlaps
            let mut tmp = Line::new();
            tmp.data[..self.lines[i - 1].len].copy_from_slice(&self.lines[i - 1].data[..self.lines[i - 1].len]);
            tmp.len = self.lines[i - 1].len;
            self.lines[i].data[..tmp.len].copy_from_slice(&tmp.data[..tmp.len]);
            self.lines[i].len = tmp.len;
            i -= 1;
        }
        self.lines[at].clear();
        self.num_lines += 1;
        true
    }

    fn delete_line(&mut self, at: usize) {
        if self.num_lines <= 1 {
            // Don't delete the last line, just clear it
            self.lines[0].clear();
            return;
        }
        let mut i = at;
        while i + 1 < self.num_lines {
            let mut tmp = Line::new();
            tmp.data[..self.lines[i + 1].len].copy_from_slice(&self.lines[i + 1].data[..self.lines[i + 1].len]);
            tmp.len = self.lines[i + 1].len;
            self.lines[i].data[..tmp.len].copy_from_slice(&tmp.data[..tmp.len]);
            self.lines[i].len = tmp.len;
            i += 1;
        }
        self.num_lines -= 1;
    }

    // ── Screen drawing (VT100) ───────────────────────────────────────

    fn full_redraw<S: Serial>(&mut self, con: &mut Console<S>) {
        Self::vt_hide_cursor(con);
        Self::vt_clear(con);
        self.adjust_scroll();
        self.draw_screen(con);
    }

    fn draw_screen<S: Serial>(&mut self, con: &mut Console<S>) {
        Self::vt_hide_cursor(con);

        let gutter = if self.settings.number { 5 } else { 0 }; // "NNN " = 4 chars + space
        let text_width = TERM_COLS - gutter;

        // Draw text rows
        for screen_row in 0..TEXT_ROWS {
            let doc_row = self.scroll + screen_row;
            Self::vt_cursor(con, screen_row + 1, 1);
            Self::vt_clear_line(con);

            if doc_row < self.num_lines {
                // Line numbers
                if self.settings.number {
                    let mut nbuf = [0u8; 8];
                    let n = fmt_usize(&mut nbuf, doc_row + 1);
                    // Right-align in 4 chars
                    for _ in 0..(4usize.saturating_sub(n)) {
                        con.write_str_raw(" ");
                    }
                    con.write_str_raw(unsafe { core::str::from_utf8_unchecked(&nbuf[..n]) });
                    con.write_str_raw(" ");
                }

                let line = &self.lines[doc_row];
                let show = if line.len > text_width { text_width } else { line.len };
                if show > 0 {
                    con.write_str_raw(
                        unsafe { core::str::from_utf8_unchecked(&line.data[..show]) }
                    );
                }
            } else {
                // Empty line — show tilde like vi
                if self.settings.number {
                    con.write_str_raw("     ");
                }
                con.write_str_raw("~");
            }
        }

        // Draw status line
        self.draw_status_line(con);

        // Position physical cursor
        let screen_row = self.row - self.scroll + 1;
        let screen_col = self.col + 1 + gutter;
        Self::vt_cursor(con, screen_row, screen_col);
        Self::vt_show_cursor(con);
    }

    fn draw_status_line<S: Serial>(&self, con: &mut Console<S>) {
        Self::vt_cursor(con, TERM_ROWS, 1);
        // Reverse video for status bar
        con.write_str_raw("\x1B[7m");
        Self::vt_clear_line(con);

        if self.mode == Mode::Command {
            con.write_str_raw(":");
            if self.cmd_len > 0 {
                let s = unsafe { core::str::from_utf8_unchecked(&self.cmd_buf[..self.cmd_len]) };
                con.write_str_raw(s);
            }
        } else {
            // Show filename or "[No Name]"
            if self.filename_len > 0 {
                let name = unsafe { core::str::from_utf8_unchecked(&self.filename[..self.filename_len]) };
                con.write_str_raw(name);
            } else {
                con.write_str_raw("[No Name]");
            }

            if self.dirty {
                con.write_str_raw(" [+]");
            }

            // Show status message if any
            if self.status_len > 0 {
                con.write_str_raw("  ");
                let s = unsafe { core::str::from_utf8_unchecked(&self.status[..self.status_len]) };
                con.write_str_raw(s);
            }

            // Right-aligned: line/col info
            // Position to right side of status bar
            Self::vt_cursor(con, TERM_ROWS, TERM_COLS - 20);
            // Format: "Line X/Y  Col Z"
            let mut tmp = [0u8; 32];
            let n = fmt_line_info(&mut tmp, self.row + 1, self.num_lines, self.col + 1);
            con.write_str_raw(unsafe { core::str::from_utf8_unchecked(&tmp[..n]) });
        }

        // Reset video attributes
        con.write_str_raw("\x1B[0m");
    }

    // ── Status helpers ───────────────────────────────────────────────

    fn set_status(&mut self, msg: &str) {
        let bytes = msg.as_bytes();
        let take = if bytes.len() > 64 { 64 } else { bytes.len() };
        self.status[..take].copy_from_slice(&bytes[..take]);
        self.status_len = take;
    }

    fn set_status_fmt(&mut self, num: usize, suffix: &str) {
        // Format: "<num> <suffix>"
        let mut tmp = [0u8; 16];
        let n = fmt_usize(&mut tmp, num);
        let mut pos = 0;
        let bytes = &tmp[..n];
        for &b in bytes {
            if pos < 64 { self.status[pos] = b; pos += 1; }
        }
        if pos < 64 { self.status[pos] = b' '; pos += 1; }
        for &b in suffix.as_bytes() {
            if pos < 64 { self.status[pos] = b; pos += 1; }
        }
        self.status_len = pos;
    }

    // ── VT100 escape helpers ─────────────────────────────────────────

    fn vt_clear<S: Serial>(con: &mut Console<S>) {
        con.write_str_raw("\x1B[2J");
    }

    fn vt_cursor<S: Serial>(con: &mut Console<S>, row: usize, col: usize) {
        // ESC [ <row> ; <col> H
        con.write_str_raw("\x1B[");
        let mut buf = [0u8; 8];
        let n = fmt_usize(&mut buf, row);
        con.write_str_raw(unsafe { core::str::from_utf8_unchecked(&buf[..n]) });
        con.write_str_raw(";");
        let n = fmt_usize(&mut buf, col);
        con.write_str_raw(unsafe { core::str::from_utf8_unchecked(&buf[..n]) });
        con.write_str_raw("H");
    }

    fn vt_clear_line<S: Serial>(con: &mut Console<S>) {
        con.write_str_raw("\x1B[2K");
    }

    fn vt_hide_cursor<S: Serial>(con: &mut Console<S>) {
        con.write_str_raw("\x1B[?25l");
    }

    fn vt_show_cursor<S: Serial>(con: &mut Console<S>) {
        con.write_str_raw("\x1B[?25h");
    }

    // ── Replace mode ─────────────────────────────────────────────────

    fn handle_replace<S: Serial>(&mut self, con: &mut Console<S>, byte: u8) {
        match byte {
            ESC => {
                self.mode = Mode::Normal;
                if self.col > 0 { self.col -= 1; }
                self.set_status("");
            }

            CR | LF => {
                // Split like insert
                let right = self.lines[self.row].split_at(self.col);
                if self.insert_line_below() {
                    self.row += 1;
                    self.lines[self.row].copy_from(&right);
                    self.col = 0;
                    self.dirty = true;
                }
            }

            BS | DEL => {
                if self.col > 0 {
                    self.col -= 1;
                }
            }

            0x20..=0x7E => {
                let line = &mut self.lines[self.row];
                if self.col < line.len {
                    // Overwrite existing character
                    line.data[self.col] = byte;
                } else {
                    // At end of line, insert like insert mode
                    let _ = line.insert(self.col, byte);
                }
                self.col += 1;
                self.dirty = true;
            }

            _ => {
                // Arrow keys via ESC sequence in replace mode
                if byte == ESC && con.has_data() {
                    let b2 = con.read_byte();
                    if b2 == b'[' && con.has_data() {
                        let arrow = con.read_byte();
                        match arrow {
                            b'A' => self.move_up(),
                            b'B' => self.move_down(),
                            b'C' => self.move_right(),
                            b'D' => { if self.col > 0 { self.col -= 1; } }
                            _ => {}
                        }
                    }
                }
            }
        }

        self.adjust_scroll();
        self.draw_screen(con);
    }

    // ── Search input mode ────────────────────────────────────────────

    fn handle_search_input<S: Serial>(&mut self, con: &mut Console<S>, byte: u8) {
        match byte {
            ESC => {
                self.mode = Mode::Normal;
                self.set_status("");
            }

            CR | LF => {
                // Copy cmd_buf to search_buf, then execute search
                let len = core::cmp::min(self.cmd_len, 48);
                self.search_buf[..len].copy_from_slice(&self.cmd_buf[..len]);
                self.search_len = len;
                self.mode = Mode::Normal;
                if self.search_fwd {
                    self.search_next();
                } else {
                    self.search_prev();
                }
            }

            BS | DEL => {
                if self.cmd_len > 0 {
                    self.cmd_len -= 1;
                }
            }

            0x20..=0x7E => {
                if self.cmd_len < 63 {
                    self.cmd_buf[self.cmd_len] = byte;
                    self.cmd_len += 1;
                }
            }

            _ => {}
        }

        // Draw search prompt on status line
        Self::vt_cursor(con, TERM_ROWS, 1);
        con.write_str_raw("\x1B[7m");
        Self::vt_clear_line(con);
        if self.search_fwd {
            con.write_str_raw("/");
        } else {
            con.write_str_raw("?");
        }
        if self.cmd_len > 0 {
            let s = unsafe { core::str::from_utf8_unchecked(&self.cmd_buf[..self.cmd_len]) };
            con.write_str_raw(s);
        }
        con.write_str_raw("\x1B[0m");
    }

    // ── Additional movement helpers ──────────────────────────────────

    /// Move to end of current word (like `e` in vi).
    fn word_end(&mut self) {
        let line = &self.lines[self.row];
        let mut c = self.col;

        if c >= line.len.saturating_sub(1) {
            // Move to next line
            if self.row + 1 < self.num_lines {
                self.row += 1;
                self.col = self.first_nonblank();
                // Now find end of word on new line
                let line = &self.lines[self.row];
                let mut c2 = self.col;
                while c2 < line.len && line.data[c2] != b' ' {
                    c2 += 1;
                }
                self.col = if c2 > 0 { c2 - 1 } else { 0 };
            }
            return;
        }

        // Advance past current character
        c += 1;
        // Skip spaces
        while c < line.len && line.data[c] == b' ' {
            c += 1;
        }
        // Move to end of word
        while c < line.len && line.data[c] != b' ' {
            c += 1;
        }
        self.col = if c > 0 { c - 1 } else { 0 };
    }

    /// Find character forward in current line (`f` command).
    fn find_char_fwd(&mut self, ch: u8) {
        let line = &self.lines[self.row];
        let mut c = self.col + 1;
        while c < line.len {
            if line.data[c] == ch {
                self.col = c;
                return;
            }
            c += 1;
        }
    }

    /// Find character backward in current line (`F` command).
    fn find_char_back(&mut self, ch: u8) {
        let line = &self.lines[self.row];
        if self.col == 0 { return; }
        let mut c = self.col - 1;
        loop {
            if line.data[c] == ch {
                self.col = c;
                return;
            }
            if c == 0 { break; }
            c -= 1;
        }
    }

    // ── Search helpers ───────────────────────────────────────────────

    /// Search forward from cursor for the current search pattern.
    fn search_next(&mut self) {
        if self.search_len == 0 {
            self.set_status("no search pattern");
            return;
        }
        let pattern = &self.search_buf[..self.search_len];
        let start_row = self.row;
        let start_col = self.col + 1;

        // Search from cursor position forward
        let mut r = start_row;
        let mut c = start_col;

        loop {
            if let Some(pos) = self.find_in_line(r, c, pattern) {
                self.row = r;
                self.col = pos;
                return;
            }
            r += 1;
            c = 0;
            if r >= self.num_lines { r = 0; }
            if r == start_row {
                // Wrapped around to start — check the start line from col 0
                if let Some(pos) = self.find_in_line(r, 0, pattern) {
                    if pos > self.col || c <= self.col {
                        self.row = r;
                        self.col = pos;
                        self.set_status("search wrapped");
                        return;
                    }
                }
                self.set_status("pattern not found");
                return;
            }
        }
    }

    /// Search backward from cursor for the current search pattern.
    fn search_prev(&mut self) {
        if self.search_len == 0 {
            self.set_status("no search pattern");
            return;
        }
        let pattern = &self.search_buf[..self.search_len];
        let start_row = self.row;

        // Search backward
        let mut r = start_row;
        let mut first = true;

        loop {
            let line = &self.lines[r];
            let end_col = if first && self.col > 0 { self.col - 1 } else if first { 0 } else { line.len };
            first = false;

            // Search backwards through the line
            if line.len >= pattern.len() && end_col > 0 {
                let search_end = core::cmp::min(end_col, line.len - pattern.len() + 1);
                let mut c = search_end;
                loop {
                    if c == 0 { break; }
                    c -= 1;
                    if self.line_matches_at(r, c, pattern) {
                        self.row = r;
                        self.col = c;
                        return;
                    }
                }
            }

            if r == 0 { r = self.num_lines; }
            r -= 1;
            if r == start_row {
                self.set_status("pattern not found");
                return;
            }
        }
    }

    /// Find a pattern in a line starting from `from_col`.  Returns column if found.
    fn find_in_line(&self, row: usize, from_col: usize, pattern: &[u8]) -> Option<usize> {
        let line = &self.lines[row];
        if line.len < pattern.len() { return None; }
        let mut c = from_col;
        let end = line.len - pattern.len() + 1;
        while c < end {
            if self.line_matches_at(row, c, pattern) {
                return Some(c);
            }
            c += 1;
        }
        None
    }

    /// Check if `pattern` matches at (row, col).
    fn line_matches_at(&self, row: usize, col: usize, pattern: &[u8]) -> bool {
        let line = &self.lines[row];
        if col + pattern.len() > line.len { return false; }
        for i in 0..pattern.len() {
            if line.data[col + i] != pattern[i] { return false; }
        }
        true
    }

    /// Search for the word under the cursor (`*` command).
    fn search_word_under_cursor(&mut self) {
        let line = &self.lines[self.row];
        if self.col >= line.len { return; }

        // Find word boundaries
        let mut start = self.col;
        while start > 0 && line.data[start - 1] != b' ' {
            start -= 1;
        }
        let mut end = self.col;
        while end < line.len && line.data[end] != b' ' {
            end += 1;
        }

        let word_len = end - start;
        if word_len == 0 || word_len > 48 { return; }

        self.search_buf[..word_len].copy_from_slice(&line.data[start..end]);
        self.search_len = word_len;
        self.search_fwd = true;
        self.search_next();
    }

    // ── Indent helpers ───────────────────────────────────────────────

    /// Get the amount of leading whitespace on a line.
    fn get_indent(&self, row: usize) -> usize {
        if row >= self.num_lines { return 0; }
        let line = &self.lines[row];
        let mut n = 0;
        while n < line.len && (line.data[n] == b' ' || line.data[n] == TAB) {
            n += 1;
        }
        n
    }

    /// Set the leading indent on a line to exactly `n` spaces.
    fn apply_indent(&mut self, row: usize, n: usize) {
        if row >= self.num_lines { return; }
        let current_indent = self.get_indent(row);
        if n > current_indent {
            // Insert spaces at beginning
            let add = n - current_indent;
            for _ in 0..add {
                self.lines[row].insert(0, b' ');
            }
        } else if n < current_indent {
            // Remove spaces from beginning
            let remove = current_indent - n;
            for _ in 0..remove {
                self.lines[row].delete(0);
            }
        }
    }

    /// Indent a line by `tabstop` spaces (`>>` command).
    fn indent_line(&mut self, row: usize, tabstop: usize) {
        if row >= self.num_lines { return; }
        for _ in 0..tabstop {
            self.lines[row].insert(0, b' ');
        }
    }

    /// Dedent a line by up to `tabstop` spaces (`<<` command).
    fn dedent_line(&mut self, row: usize, tabstop: usize) {
        if row >= self.num_lines { return; }
        let mut removed = 0;
        while removed < tabstop && self.lines[row].len > 0 && self.lines[row].data[0] == b' ' {
            self.lines[row].delete(0);
            removed += 1;
        }
    }

    // ── Edit helpers ─────────────────────────────────────────────────

    /// Delete from cursor to end of line (`D` / `d$` command).
    fn delete_to_end(&mut self) {
        let line = &mut self.lines[self.row];
        if self.col < line.len {
            line.len = self.col;
            self.dirty = true;
            self.clamp_col();
        }
    }

    /// Delete from start of line to cursor (`d0` command).
    fn delete_to_start(&mut self) {
        if self.col == 0 { return; }
        let line = &mut self.lines[self.row];
        // Shift remaining data left
        let remaining = line.len - self.col;
        for i in 0..remaining {
            line.data[i] = line.data[self.col + i];
        }
        line.len = remaining;
        self.col = 0;
        self.dirty = true;
    }

    /// Delete word from cursor (`dw` command).
    fn delete_word(&mut self) {
        let line = &mut self.lines[self.row];
        if self.col >= line.len { return; }

        let mut end = self.col;
        // Skip non-space chars (word)
        while end < line.len && line.data[end] != b' ' {
            end += 1;
        }
        // Also skip trailing spaces
        while end < line.len && line.data[end] == b' ' {
            end += 1;
        }

        let del_count = end - self.col;
        // Shift remaining left
        let remaining = line.len - end;
        for i in 0..remaining {
            line.data[self.col + i] = line.data[end + i];
        }
        line.len -= del_count;
        self.dirty = true;
        self.clamp_col();
    }

    /// Execute `:s/pattern/replacement/` substitute on the current line.
    fn exec_substitute(&mut self, cmd: &str) {
        let bytes = cmd.as_bytes();
        // parse s/pat/rep/[g]
        // Find the three '/' separators (first is at index 1)
        let mut slashes = [0usize; 3];
        let mut si = 0;
        for i in 1..bytes.len() {
            if bytes[i] == b'/' {
                if si < 3 { slashes[si] = i; si += 1; }
            }
        }
        if si < 2 {
            self.set_status("usage: s/pat/rep/");
            return;
        }

        let pat_start = slashes[0] + 1;
        let pat_end = slashes[1];
        let rep_start = slashes[1] + 1;
        let rep_end = if si >= 3 { slashes[2] } else { bytes.len() };
        let global = si >= 3 && rep_end + 1 <= bytes.len() && bytes.get(rep_end + 1 - 1).copied() == Some(b'g');

        let pat_len = pat_end - pat_start;
        let rep_len = rep_end - rep_start;
        if pat_len == 0 {
            self.set_status("empty pattern");
            return;
        }

        let line = &mut self.lines[self.row];
        let mut count = 0usize;
        let mut c = 0usize;

        loop {
            if c + pat_len > line.len { break; }
            let mut matched = true;
            for i in 0..pat_len {
                if line.data[c + i] != bytes[pat_start + i] {
                    matched = false;
                    break;
                }
            }
            if matched {
                // Replace: remove pat_len chars at c, insert rep_len chars
                // First remove
                let tail = line.len - (c + pat_len);
                for i in 0..tail {
                    line.data[c + i] = line.data[c + pat_len + i];
                }
                line.len -= pat_len;
                // Then insert replacement
                if rep_len > 0 && line.len + rep_len <= MAX_COLS {
                    // Shift tail right
                    let tail2 = line.len - c;
                    let mut i = tail2;
                    while i > 0 {
                        i -= 1;
                        line.data[c + rep_len + i] = line.data[c + i];
                    }
                    for i in 0..rep_len {
                        line.data[c + i] = bytes[rep_start + i];
                    }
                    line.len += rep_len;
                }
                count += 1;
                c += rep_len;
                if !global { break; }
            } else {
                c += 1;
            }
        }

        if count > 0 {
            self.dirty = true;
            let mut tmp = [0u8; 16];
            let n = fmt_usize(&mut tmp, count);
            let mut pos = 0;
            for i in 0..n {
                if pos < 64 { self.status[pos] = tmp[i]; pos += 1; }
            }
            for &b in b" substitution(s)" {
                if pos < 64 { self.status[pos] = b; pos += 1; }
            }
            self.status_len = pos;
        } else {
            self.set_status("pattern not found");
        }
    }

    /// Join current line with the next line (`J` command).
    fn join_lines(&mut self) {
        if self.row + 1 >= self.num_lines { return; }
        let cur_len = self.lines[self.row].len;
        let next_len = self.lines[self.row + 1].len;

        // Add a space separator if current line is non-empty and doesn't end with space
        let add_space = cur_len > 0 && self.lines[self.row].data[cur_len - 1] != b' ';
        let need = if add_space { 1 } else { 0 };

        if cur_len + need + next_len > MAX_COLS {
            self.set_status("line too long");
            return;
        }

        if add_space {
            self.lines[self.row].data[cur_len] = b' ';
            self.lines[self.row].len = cur_len + 1;
        }

        // Skip leading whitespace of next line
        let next_row = self.row + 1;
        let mut start = 0;
        while start < self.lines[next_row].len && self.lines[next_row].data[start] == b' ' {
            start += 1;
        }

        let copy_len = self.lines[next_row].len - start;
        let dest_start = self.lines[self.row].len;
        for i in 0..copy_len {
            if dest_start + i < MAX_COLS {
                self.lines[self.row].data[dest_start + i] = self.lines[next_row].data[start + i];
            }
        }
        self.lines[self.row].len = dest_start + copy_len;

        self.delete_line(next_row);
        self.col = cur_len;
        self.dirty = true;
    }

    /// Match bracket under cursor (`%` command).
    fn match_bracket(&mut self) {
        let line = &self.lines[self.row];
        if self.col >= line.len { return; }
        let ch = line.data[self.col];

        let (target, forward) = match ch {
            b'(' => (b')', true),
            b')' => (b'(', false),
            b'[' => (b']', true),
            b']' => (b'[', false),
            b'{' => (b'}', true),
            b'}' => (b'{', false),
            _ => return,
        };

        let mut depth: isize = 1;
        let mut r = self.row;
        let mut c = self.col;

        if forward {
            loop {
                c += 1;
                while c >= self.lines[r].len {
                    r += 1;
                    if r >= self.num_lines { return; }
                    c = 0;
                }
                let b = self.lines[r].data[c];
                if b == ch { depth += 1; }
                if b == target { depth -= 1; }
                if depth == 0 {
                    self.row = r;
                    self.col = c;
                    return;
                }
            }
        } else {
            loop {
                if c == 0 {
                    if r == 0 { return; }
                    r -= 1;
                    c = self.lines[r].len;
                    if c == 0 { continue; }
                }
                c -= 1;
                let b = self.lines[r].data[c];
                if b == ch { depth += 1; }
                if b == target { depth -= 1; }
                if depth == 0 {
                    self.row = r;
                    self.col = c;
                    return;
                }
            }
        }
    }

    /// Repeat the last edit (`.` command).
    fn repeat_last_edit(&mut self) {
        let edit = self.last_edit;
        match edit.kind {
            EditKind::None => {}
            EditKind::DeleteChar => {
                for _ in 0..edit.count {
                    if self.lines[self.row].len > 0 {
                        self.lines[self.row].delete(self.col);
                        self.dirty = true;
                        self.clamp_col();
                    }
                }
            }
            EditKind::DeleteLine => {
                for _ in 0..edit.count {
                    self.yank.copy_from(&self.lines[self.row]);
                    self.delete_line(self.row);
                    self.dirty = true;
                    self.clamp_row();
                    self.clamp_col();
                }
            }
            EditKind::ChangeLine => {
                self.lines[self.row].clear();
                self.col = 0;
                self.dirty = true;
                self.mode = Mode::Insert;
                self.set_status("-- INSERT --");
            }
            EditKind::ReplaceChar => {
                if self.col < self.lines[self.row].len {
                    for i in 0..edit.count {
                        let pos = self.col + i;
                        if pos < self.lines[self.row].len {
                            self.lines[self.row].data[pos] = edit.ch;
                        }
                    }
                    self.dirty = true;
                }
            }
            EditKind::PutBelow => {
                if self.yank.len > 0 {
                    for _ in 0..edit.count {
                        if self.insert_line_below() {
                            self.row += 1;
                            self.lines[self.row].copy_from(&self.yank);
                            self.dirty = true;
                        }
                    }
                }
            }
            EditKind::PutAbove => {
                if self.yank.len > 0 {
                    for _ in 0..edit.count {
                        if self.insert_line_at(self.row) {
                            self.lines[self.row].copy_from(&self.yank);
                            self.dirty = true;
                        }
                    }
                }
            }
            EditKind::JoinLine => {
                for _ in 0..edit.count {
                    self.join_lines();
                }
            }
            EditKind::IndentRight => {
                let ts = self.settings.tabstop as usize;
                for i in 0..edit.count {
                    let r = self.row + i;
                    if r < self.num_lines { self.indent_line(r, ts); }
                }
                self.dirty = true;
            }
            EditKind::IndentLeft => {
                let ts = self.settings.tabstop as usize;
                for i in 0..edit.count {
                    let r = self.row + i;
                    if r < self.num_lines { self.dedent_line(r, ts); }
                }
                self.dirty = true;
            }
            EditKind::ToggleCase => {
                for _ in 0..edit.count {
                    if self.col < self.lines[self.row].len {
                        let c = self.lines[self.row].data[self.col];
                        self.lines[self.row].data[self.col] = if c >= b'a' && c <= b'z' {
                            c - 32
                        } else if c >= b'A' && c <= b'Z' {
                            c + 32
                        } else {
                            c
                        };
                        self.dirty = true;
                        self.move_right();
                    }
                }
            }
            EditKind::DeleteToEnd => {
                self.delete_to_end();
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Command result
// ═══════════════════════════════════════════════════════════════════════════

enum CmdResult {
    Continue,
    Quit,
    SaveQuit,
}

// ═══════════════════════════════════════════════════════════════════════════
// Utility — no_std integer formatting
// ═══════════════════════════════════════════════════════════════════════════

/// Format a `usize` into a byte buffer.  Returns the number of digits written.
fn fmt_usize(buf: &mut [u8], mut val: usize) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 20];
    let mut i = 0;
    while val > 0 {
        tmp[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    // Reverse into buf
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    i
}

/// Parse a `usize` from a string.  Very simple — digits only.
fn parse_usize(s: &str) -> Result<usize, ()> {
    let bytes = s.as_bytes();
    if bytes.is_empty() { return Err(()); }
    let mut val: usize = 0;
    for &b in bytes {
        if b < b'0' || b > b'9' { return Err(()); }
        val = val.checked_mul(10).ok_or(())?;
        val = val.checked_add((b - b'0') as usize).ok_or(())?;
    }
    Ok(val)
}

/// Format "Line X/Y  Col Z" into a buffer.  Returns bytes written.
fn fmt_line_info(buf: &mut [u8], row: usize, total: usize, col: usize) -> usize {
    let mut pos = 0;

    // "Line "
    for &b in b"Line " {
        if pos < buf.len() { buf[pos] = b; pos += 1; }
    }

    // row number
    let mut tmp = [0u8; 8];
    let n = fmt_usize(&mut tmp, row);
    for i in 0..n {
        if pos < buf.len() { buf[pos] = tmp[i]; pos += 1; }
    }

    // "/"
    if pos < buf.len() { buf[pos] = b'/'; pos += 1; }

    // total
    let n = fmt_usize(&mut tmp, total);
    for i in 0..n {
        if pos < buf.len() { buf[pos] = tmp[i]; pos += 1; }
    }

    // "  Col "
    for &b in b"  Col " {
        if pos < buf.len() { buf[pos] = b; pos += 1; }
    }

    // col number
    let n = fmt_usize(&mut tmp, col);
    for i in 0..n {
        if pos < buf.len() { buf[pos] = tmp[i]; pos += 1; }
    }

    pos
}

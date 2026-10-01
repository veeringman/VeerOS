//! First VeeroS view: live system status + command palette.
//!
//! Bound to in-kernel state (display mode, input counts, last key) — the
//! Fabric Client pattern in miniature: views render from fabric/system
//! state, keys invoke. Dark-first per the VeerFlow visual language.
//!
//! The palette keeps an edit line, a small history (Up/Down), and a
//! 4-line result log. Commands are local for now (`help`, `status`,
//! `fabric`, `intent`, `echo`, `clear`); each documents its fabric
//! follow-up so the surface grows into real invocation.

use veer_ui::{theme, Arena, Children, Color, EdgeInsets, Icon, IconSymbol, Text, View, Widget};

/// Edit-line capacity (bytes).
const CMD_MAX: usize = 96;
/// Result log depth.
const LOG_DEPTH: usize = 4;
/// Log line capacity (bytes).
const LOG_WIDTH: usize = 100;
/// History depth.
const HIST_DEPTH: usize = 8;
/// History entry capacity (bytes).
const HIST_WIDTH: usize = 64;

/// Log line severity.
const LOG_INFO: u8 = 0;
const LOG_OK: u8 = 1;
const LOG_ERR: u8 = 2;

fn log_color(kind: u8) -> Color {
    match kind {
        LOG_OK => theme::ACCENT,
        LOG_ERR => theme::DANGER,
        _ => theme::MUTED,
    }
}

/// Live system status view.
pub struct SystemView {
    pub last_key: u8,
    pub has_key: bool,
    pub key_count: u32,
    pub fb_label: &'static str,
    pub virt_kbd: bool,
    pub virt_ptr: bool,
    pub virt_note: &'static str,
    pub vq0: (u16, u16),
    pub vq1: (u16, u16),
    cmd: [u8; CMD_MAX],
    cmd_len: usize,
    backup: [u8; CMD_MAX],
    backup_len: usize,
    hist: [[u8; HIST_WIDTH]; HIST_DEPTH],
    hist_len: [usize; HIST_DEPTH],
    hist_count: usize,
    /// `None` = editing fresh; `Some(i)` = viewing history entry `i`.
    hist_nav: Option<usize>,
    log: [[u8; LOG_WIDTH]; LOG_DEPTH],
    log_len: [usize; LOG_DEPTH],
    log_kind: [u8; LOG_DEPTH],
    log_count: usize,
    intent_seq: u32,
}

impl SystemView {
    pub const fn new() -> Self {
        Self {
            last_key: 0,
            has_key: false,
            key_count: 0,
            fb_label: "no display",
            virt_kbd: false,
            virt_ptr: false,
            virt_note: "absent",
            vq0: (0, 0),
            vq1: (0, 0),
            cmd: [0; CMD_MAX],
            cmd_len: 0,
            backup: [0; CMD_MAX],
            backup_len: 0,
            hist: [[0; HIST_WIDTH]; HIST_DEPTH],
            hist_len: [0; HIST_DEPTH],
            hist_count: 0,
            hist_nav: None,
            log: [[0; LOG_WIDTH]; LOG_DEPTH],
            log_len: [0; LOG_DEPTH],
            log_kind: [0; LOG_DEPTH],
            log_count: 0,
            intent_seq: 0,
        }
    }

    fn cmd_str(&self) -> &str {
        core::str::from_utf8(&self.cmd[..self.cmd_len]).unwrap_or("")
    }

    fn push_log(&mut self, kind: u8, text: &str) {
        let bytes = text.as_bytes();
        let n = bytes.len().min(LOG_WIDTH);
        if self.log_count < LOG_DEPTH {
            let i = self.log_count;
            self.log[i][..n].copy_from_slice(&bytes[..n]);
            self.log_len[i] = n;
            self.log_kind[i] = kind;
            self.log_count += 1;
        } else {
            for i in 0..LOG_DEPTH - 1 {
                self.log[i] = self.log[i + 1];
                self.log_len[i] = self.log_len[i + 1];
                self.log_kind[i] = self.log_kind[i + 1];
            }
            self.log[LOG_DEPTH - 1][..n].copy_from_slice(&bytes[..n]);
            self.log_len[LOG_DEPTH - 1] = n;
            self.log_kind[LOG_DEPTH - 1] = kind;
        }
    }

    fn push_hist(&mut self) {
        if self.cmd_len == 0 {
            return;
        }
        if self.hist_count < HIST_DEPTH {
            let i = self.hist_count;
            let n = self.cmd_len.min(HIST_WIDTH);
            self.hist[i][..n].copy_from_slice(&self.cmd[..n]);
            self.hist_len[i] = n;
            self.hist_count += 1;
        } else {
            for i in 0..HIST_DEPTH - 1 {
                self.hist[i] = self.hist[i + 1];
                self.hist_len[i] = self.hist_len[i + 1];
            }
            let n = self.cmd_len.min(HIST_WIDTH);
            self.hist[HIST_DEPTH - 1][..n].copy_from_slice(&self.cmd[..n]);
            self.hist_len[HIST_DEPTH - 1] = n;
        }
    }

    fn load_hist(&mut self, i: usize) {
        let n = self.hist_len[i].min(CMD_MAX);
        self.cmd[..n].copy_from_slice(&self.hist[i][..n]);
        self.cmd_len = n;
    }

    fn save_backup(&mut self) {
        self.backup[..self.cmd_len].copy_from_slice(&self.cmd[..self.cmd_len]);
        self.backup_len = self.cmd_len;
    }

    fn restore_backup(&mut self) {
        self.cmd[..self.backup_len].copy_from_slice(&self.backup[..self.backup_len]);
        self.cmd_len = self.backup_len;
    }

    /// Execute the edit line. Returns the log severity for tests.
    fn exec(&mut self) -> u8 {
        // Copy out first: `cmd`/`args` must not borrow `self` across
        // the `&mut` log/history calls below.
        let mut linebuf = [0u8; CMD_MAX];
        linebuf[..self.cmd_len].copy_from_slice(&self.cmd[..self.cmd_len]);
        let line = core::str::from_utf8(&linebuf[..self.cmd_len]).unwrap_or("");
        let (cmd, args) = match line.find(' ') {
            Some(i) => (&line[..i], line[i + 1..].trim_start()),
            None => (line, ""),
        };
        let kind = match cmd {
            "" => LOG_INFO,
            "help" => {
                self.push_log(LOG_INFO, "help status fabric intent echo clear");
                LOG_INFO
            }
            "status" => {
                self.push_log(LOG_OK, "display ok - input ok - console live");
                LOG_OK
            }
            "fabric" => {
                self.push_log(LOG_OK, "nodes 1/1: local aarch64-virt (standalone)");
                LOG_OK
            }
            "keys" => LOG_INFO,
            "echo" => {
                self.push_log(LOG_INFO, if args.is_empty() { "(empty)" } else { args });
                LOG_INFO
            }
            "clear" => {
                self.log_count = 0;
                LOG_INFO
            }
            "intent" => {
                self.intent_seq = self.intent_seq.wrapping_add(1);
                // Log via a fixed scratch: format without arena (state!).
                let mut scratch = [0u8; LOG_WIDTH];
                let msg: &[u8] = if args.is_empty() {
                    b"intent: need a goal (usage: intent <goal>)"
                } else {
                    let head = b"intent #";
                    let mut n = 0;
                    scratch[..head.len()].copy_from_slice(head);
                    n += head.len();
                    let mut seq = self.intent_seq;
                    let mut digits = [0u8; 10];
                    let mut nd = 0;
                    if seq == 0 {
                        digits[0] = b'0';
                        nd = 1;
                    } else {
                        while seq > 0 && nd < 10 {
                            digits[nd] = b'0' + (seq % 10) as u8;
                            seq /= 10;
                            nd += 1;
                        }
                    }
                    for j in (0..nd).rev() {
                        if n < LOG_WIDTH {
                            scratch[n] = digits[j];
                            n += 1;
                        }
                    }
                    let tail = b" queued (local stub - fabric next)";
                    let t = tail.len().min(LOG_WIDTH - n);
                    scratch[n..n + t].copy_from_slice(&tail[..t]);
                    n += t;
                    &scratch[..n]
                };
                let text = core::str::from_utf8(msg).unwrap_or("intent queued");
                // `text` borrows `scratch`: copy into the log now.
                self.push_log(LOG_OK, text);
                LOG_OK
            }
            _ => {
                self.push_log(LOG_ERR, "unknown command (try help)");
                LOG_ERR
            }
        };
        if cmd == "keys" {
            // Needs the live count: push after computing (borrow-safe).
            let n = self.key_count;
            let mut scratch = [0u8; 32];
            let head = b"keys pressed: ";
            scratch[..head.len()].copy_from_slice(head);
            let mut m = head.len();
            let mut v = n;
            let mut digits = [0u8; 10];
            let mut nd = 0;
            if v == 0 {
                digits[0] = b'0';
                nd = 1;
            } else {
                while v > 0 && nd < 10 {
                    digits[nd] = b'0' + (v % 10) as u8;
                    v /= 10;
                    nd += 1;
                }
            }
            for j in (0..nd).rev() {
                if m < scratch.len() {
                    scratch[m] = digits[j];
                    m += 1;
                }
            }
            let text = core::str::from_utf8(&scratch[..m]).unwrap_or("keys ?");
            self.push_log(LOG_INFO, text);
        }
        kind
    }
}

impl View for SystemView {
    fn build<'a>(&self, ui: &'a mut Arena) -> Widget<'a> {
        let title = Widget::Text(Text::new("VeeroS", theme::TEXT).scale(2));
        let sub = Widget::Text(Text::new(
            "fabric console - command-palette first - live",
            theme::MUTED,
        ));

        let dot = Widget::Icon(Icon::new(IconSymbol::Dot, 20, theme::ACCENT));
        let disp_line = ui.alloc_fmt(format_args!("display {}", self.fb_label));
        let r_display =
            Widget::Row(Children::new(ui.alloc_slice(&[
                dot,
                Widget::Text(Text::new(disp_line, theme::TEXT)),
            ])));

        let check = Widget::Icon(Icon::new(IconSymbol::Check, 20, theme::ACCENT));
        let input_line = ui.alloc_fmt(format_args!("input uart keys={}", self.key_count));
        let r_input =
            Widget::Row(Children::new(ui.alloc_slice(&[
                check,
                Widget::Text(Text::new(input_line, theme::TEXT)),
            ])));

        let key_line = if self.has_key {
            ui.alloc_fmt(format_args!("last key 0x{:02x}", self.last_key))
        } else {
            "last key --"
        };
        let info = Widget::Icon(Icon::new(IconSymbol::Info, 20, theme::INFO));
        let r_key =
            Widget::Row(Children::new(ui.alloc_slice(&[
                info,
                Widget::Text(Text::new(key_line, theme::TEXT)),
            ])));

        let (vicon, vline) = if self.virt_kbd && self.virt_ptr {
            (
                Widget::Icon(Icon::new(IconSymbol::Check, 20, theme::ACCENT)),
                ui.alloc_fmt(format_args!("vnc keyboard + tablet ready")),
            )
        } else if self.virt_kbd || self.virt_ptr {
            (
                Widget::Icon(Icon::new(IconSymbol::Alert, 20, theme::WARN)),
                ui.alloc_fmt(format_args!("vnc input partial")),
            )
        } else {
            (
                Widget::Icon(Icon::new(IconSymbol::Dot, 20, theme::MUTED)),
                ui.alloc_fmt(format_args!("vnc input {}", self.virt_note)),
            )
        };
        let r_virt = Widget::Row(Children::new(
            ui.alloc_slice(&[vicon, Widget::Text(Text::new(vline, theme::TEXT))]),
        ));

        let qline = ui.alloc_fmt(format_args!(
            "vq0 av={} us={} vq1 av={} us={}",
            self.vq0.0, self.vq0.1, self.vq1.0, self.vq1.1
        ));
        let r_vq = Widget::Row(Children::new(
            ui.alloc_slice(&[Widget::Text(Text::new(qline, theme::MUTED))]),
        ));

        // Icon strip: every v1 symbol, proving the stroke arms on-device.
        let mut strip: [Widget; 12] = [Widget::Empty; 12];
        for (i, s) in IconSymbol::ALL.iter().enumerate() {
            strip[i] = Widget::Icon(Icon::new(*s, 24, theme::TEXT));
        }
        let r_icons = Widget::Row(Children {
            items: ui.alloc_slice(&strip),
            spacing: 12,
            padding: EdgeInsets::zero(),
        });

        // Command palette.
        let prompt = Widget::Text(Text::new("> ", theme::ACCENT));
        let entry = ui.alloc_fmt(format_args!("{}_", self.cmd_str()));
        let r_cmd =
            Widget::Row(Children::new(ui.alloc_slice(&[
                prompt,
                Widget::Text(Text::new(entry, theme::TEXT)),
            ])));

        let hint = Widget::Text(Text::new(
            "type a command - ENTER invokes - try help",
            theme::MUTED,
        ));

        let body = ui.alloc_slice(&[
            title,
            sub,
            Widget::Divider,
            r_display,
            r_input,
            r_key,
            r_virt,
            r_vq,
            Widget::Divider,
            r_icons,
            r_cmd,
        ]);
        // Append log lines after the prompt (arena order irrelevant).
        let mut full: [Widget; 11 + LOG_DEPTH + 1] = [Widget::Empty; 11 + LOG_DEPTH + 1];
        for (i, w) in body.iter().enumerate() {
            full[i] = *w;
        }
        let mut n = body.len();
        for i in 0..self.log_count {
            let text = core::str::from_utf8(&self.log[i][..self.log_len[i]]).unwrap_or("?");
            let placed = ui.alloc_str(text);
            full[n] = Widget::Text(Text::new(placed, log_color(self.log_kind[i])));
            n += 1;
        }
        full[n] = hint;
        n += 1;
        Widget::Column(Children {
            items: ui.alloc_slice(&full[..n]),
            spacing: 10,
            padding: EdgeInsets::all(28),
        })
    }

    fn on_key(&mut self, key: u8) -> bool {
        self.last_key = key;
        self.has_key = true;
        self.key_count = self.key_count.wrapping_add(1);
        match key {
            0x0D => {
                // Enter: invoke.
                self.push_hist();
                self.exec();
                self.cmd_len = 0;
                self.hist_nav = None;
            }
            0x7F => {
                // Backspace.
                self.hist_nav = None;
                self.cmd_len = self.cmd_len.saturating_sub(1);
            }
            0x81 => {
                // Up: older history.
                if self.hist_count == 0 {
                    return true;
                }
                match self.hist_nav {
                    None => {
                        self.save_backup();
                        let i = self.hist_count - 1;
                        self.hist_nav = Some(i);
                        self.load_hist(i);
                    }
                    Some(i) => {
                        let j = i.saturating_sub(1);
                        self.hist_nav = Some(j);
                        self.load_hist(j);
                    }
                }
            }
            0x82 => {
                // Down: newer history / back to edit line.
                match self.hist_nav {
                    None => {}
                    Some(i) => {
                        if i + 1 < self.hist_count {
                            self.hist_nav = Some(i + 1);
                            self.load_hist(i + 1);
                        } else {
                            self.hist_nav = None;
                            self.restore_backup();
                        }
                    }
                }
            }
            b if (0x20..0x7F).contains(&b) => {
                self.hist_nav = None;
                if self.cmd_len < CMD_MAX {
                    self.cmd[self.cmd_len] = b;
                    self.cmd_len += 1;
                }
            }
            _ => {}
        }
        true
    }
}

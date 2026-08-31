//! VeerOS interactive shell.
//!
//! Provides a minimal line-editor REPL with built-in commands.
//! The shell is `no_std` and generic over any [`arch::Serial`] backend,
//! so it runs on both real hardware and a host-emulated console.

#![no_std]

use arch::{Console, Serial};
use core::fmt::Write;

mod line_ed;
pub mod script;
pub mod vi;

pub use line_ed::{History, LineEditor, LineResult};

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

/// Shell prompt string.
const PROMPT: &str = "veeros> ";

#[cfg(feature = "full-vi")]
const VI_SAVE_BUF_SIZE: usize = 2 * 1024 * 1024;
#[cfg(all(feature = "large-vi", not(feature = "full-vi")))]
const VI_SAVE_BUF_SIZE: usize = 64 * 1024;
#[cfg(not(any(feature = "large-vi", feature = "full-vi")))]
const VI_SAVE_BUF_SIZE: usize = 16 * 1024;

// ═══════════════════════════════════════════════════════════════════════════
// ShellEnv — static configuration supplied by the boot code
// ═══════════════════════════════════════════════════════════════════════════

/// Read-only environment the shell uses for informational commands.
pub struct ShellEnv {
    pub version: &'static str,
    pub platform: &'static str,
    pub scheduler: &'static str,
    /// Optional callback: return the kernel tick counter.
    pub get_uptime_ticks: Option<fn() -> u64>,
    /// Optional callback: write the task table to the given writer.
    pub get_task_list: Option<fn(&mut dyn core::fmt::Write)>,
    /// Optional callback: write memory pool stats to the given writer.
    pub get_mem_info: Option<fn(&mut dyn core::fmt::Write)>,
    /// Optional callback: write registered driver list to the given writer.
    pub get_driver_list: Option<fn(&mut dyn core::fmt::Write)>,
    /// Optional callback: handle `wifi <subcommand> <args>` and write output.
    pub wifi_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Optional callback: handle `bt <subcommand> <args>` and write output.
    pub bt_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Optional callback: handle `zigbee <subcommand> <args>` and write output.
    pub zigbee_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Optional callback: handle `sensor <subcommand> <args>` and write output.
    pub sensor_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Optional callback: get current UID and username.
    /// Returns (uid, username).
    pub get_current_user: Option<fn() -> (u16, &'static str)>,
    /// Optional callback: write active user sessions to writer.
    pub get_user_list: Option<fn(&mut dyn core::fmt::Write)>,

    // ── VFS callbacks ────────────────────────────────────────────────
    /// List directory contents. `path` is resolved from cwd.
    pub vfs_list_dir: Option<fn(&str, &mut dyn core::fmt::Write)>,
    /// Read a file into buf. Returns bytes read (0 on error).
    pub vfs_read_file: Option<fn(&str, &mut [u8]) -> usize>,
    /// Write data to a file (create if needed). `append`=true → O_APPEND.
    pub vfs_write_file: Option<fn(&str, &[u8], bool) -> bool>,
    /// Create a directory. Returns true on success.
    pub vfs_mkdir: Option<fn(&str) -> bool>,
    /// Print stat info for a path.
    pub vfs_stat: Option<fn(&str, &mut dyn core::fmt::Write)>,
    /// Remove a file or empty directory.
    pub vfs_unlink: Option<fn(&str) -> bool>,
    /// Rename / move a path.
    pub vfs_rename: Option<fn(&str, &str) -> bool>,
    /// Get current working directory into buf. Returns length.
    pub vfs_getcwd: Option<fn(&mut [u8]) -> usize>,
    /// Change working directory. Returns true on success.
    pub vfs_chdir: Option<fn(&str) -> bool>,
    /// Recursive tree listing.
    pub vfs_tree: Option<fn(&str, &mut dyn core::fmt::Write)>,
    /// Create an empty file (touch). Returns true on success.
    pub vfs_touch: Option<fn(&str) -> bool>,

    // ── Mount / device callbacks ─────────────────────────────────────
    /// List mounted filesystems. Writes output to writer.
    pub mount_list: Option<fn(&mut dyn core::fmt::Write)>,
    /// Mount a device at a path. Returns true on success.
    pub mount_fs: Option<fn(&str, &str) -> bool>,
    /// Unmount a path. Returns true on success.
    pub umount_fs: Option<fn(&str) -> bool>,
    /// List block devices. Writes output to writer.
    pub lsblk: Option<fn(&mut dyn core::fmt::Write)>,
    /// Report filesystem disk space usage. Writes output to writer.
    pub df_cmd: Option<fn(&mut dyn core::fmt::Write)>,

    // ── Input device callbacks ───────────────────────────────────────
    /// Write input subsystem status to writer.
    pub input_status: Option<fn(&mut dyn core::fmt::Write)>,
    /// List connected USB devices. Writes output to writer.
    pub usb_list: Option<fn(&mut dyn core::fmt::Write)>,
    /// List connected BLE HID devices. Writes output to writer.
    pub ble_hid_list: Option<fn(&mut dyn core::fmt::Write)>,

    // ── Hardware / GPIO / bus callbacks ───────────────────────────────
    /// Handle `gpio <subcommand> <args>` and write output.
    pub gpio_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Handle `i2c <subcommand> <args>` and write output.
    pub i2c_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Handle `spi <subcommand> <args>` and write output.
    pub spi_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Write hardware info (board, memory, clocks, thermal) to writer.
    pub hw_info: Option<fn(&mut dyn core::fmt::Write)>,
    /// Read SoC temperature in millidegrees C. Returns 0 if unavailable.
    pub get_temp_millic: Option<fn() -> i32>,
    /// Write kernel log ring buffer contents to writer.
    pub dmesg: Option<fn(&mut dyn core::fmt::Write)>,
    /// Reboot the system. Should not return.
    pub reboot: Option<fn()>,
    /// Halt / power off the system. Should not return.
    pub shutdown: Option<fn()>,

    // ── Security / capability callbacks ───────────────────────────────
    /// Handle `caps <subcommand> <args>` — show/drop process capabilities.
    pub caps_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Handle `auditlog <subcommand>` — display security audit events.
    pub auditlog_cmd: Option<fn(&str, &mut dyn core::fmt::Write)>,

    // ── Network callbacks ────────────────────────────────────────────
    /// Display network interface configuration.
    pub ifconfig_cmd: Option<fn(&mut dyn core::fmt::Write)>,
    /// Handle `ping <ip>` — send ICMP/UDP probes.
    pub ping_cmd: Option<fn(&str, &mut dyn core::fmt::Write)>,
    /// Display socket / connection status.
    pub netstat_cmd: Option<fn(&mut dyn core::fmt::Write)>,

    /// Handle `ssh <user@host>` — connect to a remote SSH server.
    /// Callback receives (args, local_serial) for bi-directional I/O.
    pub ssh_cmd: Option<fn(&str, &dyn Serial)>,

    // ── Multi-user callbacks (only used when `multi-user` feature) ────
    /// Authenticate a user. Returns session token (>0) on success, 0 on failure.
    pub login: Option<fn(&str, &[u8]) -> u32>,
    /// Logout current session. Returns true on success.
    pub logout: Option<fn() -> bool>,
    /// Change password for a UID. Returns true on success.
    pub change_password: Option<fn(u16, &[u8]) -> bool>,
    /// Add a user (name, gid, password). Returns Some(uid) on success.
    pub add_user: Option<fn(&'static str, u16, &[u8]) -> Option<u16>>,
    /// Remove a user by UID. Returns true on success.
    pub remove_user: Option<fn(u16) -> bool>,

    /// If true, skip the login gate (user was already authenticated externally, e.g. SSH).
    pub pre_authenticated: bool,

    // ── AI-Native Execution callbacks ─────────────────────────────
    /// Write active agent list to writer.
    pub get_agent_list: Option<fn(&mut dyn core::fmt::Write)>,
    /// Handle `agent <subcommand> <args>` and write output.
    pub agent_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Write active intent list to writer.
    pub get_intent_list: Option<fn(&mut dyn core::fmt::Write)>,
    /// Handle `intent <subcommand> <args>` and write output.
    pub intent_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Handle `memory <subcommand> <args>` — persistent memory ops.
    pub memory_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Write execution fabric status to writer.
    pub get_fabric_status: Option<fn(&mut dyn core::fmt::Write)>,
    /// Handle `peers <subcommand>` — Zero Trust peer management.
    pub peers_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Handle `mesh <subcommand>` — mesh transport status/ops.
    pub mesh_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,
    /// Handle `zkp <subcommand>` — ZKP proof operations.
    pub zkp_cmd: Option<fn(&str, &str, &mut dyn core::fmt::Write)>,

    /// Handle `hostname [new-name]` — get/set system hostname.
    /// Callback: fn(args, writer). If args is empty, print current; else set.
    pub hostname_cmd: Option<fn(&str, &mut dyn core::fmt::Write)>,

    /// Blocking sleep in milliseconds (used by script `sleep N` builtin).
    pub sleep_ms: Option<fn(u64)>,
}

// ═══════════════════════════════════════════════════════════════════════════
// Shell
// ═══════════════════════════════════════════════════════════════════════════

/// Interactive VeerOS shell.
pub struct Shell {
    /// Line editor with history, cursor movement, etc.
    ed: LineEditor,
    env: ShellEnv,
    /// Shell variables (set command).
    vars: ShellVars,
    /// Script variables ($var).
    script_vars: script::VarStore,
    /// Script execution context (nesting, buffering, loop state).
    script_ctx: script::ScriptCtx,
}

/// Shell variables configurable via `set`.
struct ShellVars {
    /// Show line numbers in vi.
    vi_number: bool,
    /// Tab-stop width in vi (default 4).
    vi_tabstop: u8,
    /// Show matching bracket in vi.
    vi_showmatch: bool,
    /// Auto-indent in vi.
    vi_autoindent: bool,
    /// Custom prompt string (max 32 chars).
    prompt_str: [u8; 32],
    prompt_len: usize,
}

impl Shell {
    /// Create a new shell with the given environment.
    pub fn new(env: ShellEnv) -> Self {
        Self {
            ed: LineEditor::new(),
            env,
            vars: ShellVars {
                vi_number: false,
                vi_tabstop: 4,
                vi_showmatch: false,
                vi_autoindent: false,
                prompt_str: [0u8; 32],
                prompt_len: 0,
            },
            script_vars: script::VarStore::new(),
            script_ctx: script::ScriptCtx::new(),
        }
    }

    /// Run the shell REPL.
    ///
    /// When the `multi-user` feature is enabled, presents a login prompt
    /// before starting the shell.  Returns normally when the user types
    /// `exit`, `quit`, or Ctrl-D.
    pub fn run<S: Serial>(&mut self, con: &mut Console<S>) {
        // ── multi-user login gate ────────────────────────────────
        #[cfg(feature = "multi-user")]
        {
            if !self.login_gate(con) {
                return;
            }
        }

        // Welcome header
        let _ = writeln!(con, "VeerOS Shell v{}", self.env.version);
        let _ = writeln!(con, "Type 'help' for available commands.");
        let _ = writeln!(con, "");

        loop {
            self.print_prompt(con);
            match self.ed.read_line(con) {
                LineResult::Line => {
                    let _ = writeln!(con, ""); // newline after the typed line
                    if self.ed.len > 0 {
                        if self.execute(con) {
                            return;
                        }
                    }
                }
                LineResult::Interrupt => {
                    let _ = writeln!(con, "");
                    // Just re-prompt
                }
                LineResult::Eof => {
                    let _ = writeln!(con, "");
                    let _ = writeln!(con, "logout");
                    return;
                }
            }
        }
    }

    // ── internal helpers ─────────────────────────────────────────────────

    /// Execute a single command string (used for SSH exec requests).
    /// Runs the command and returns; does not enter the REPL loop.
    pub fn run_command<S: Serial>(&mut self, con: &mut Console<S>, cmd: &str) {
        let bytes = cmd.as_bytes();
        let len = bytes.len().min(self.ed.buf.len());
        self.ed.buf[..len].copy_from_slice(&bytes[..len]);
        self.ed.len = len;
        let _ = self.execute(con);
    }

    fn print_prompt<S: Serial>(&self, con: &mut Console<S>) {
        // Read hostname from /etc/hostname via VFS if available.
        let mut host_buf = [0u8; 64];
        let mut host_name = "veeros";
        if let Some(read_fn) = self.env.vfs_read_file {
            let n = read_fn("/etc/hostname", &mut host_buf);
            if n > 0 {
                if let Ok(s) = core::str::from_utf8(&host_buf[..n]) {
                    let s = s.trim();
                    if !s.is_empty() {
                        host_name = unsafe {
                            core::str::from_utf8_unchecked(core::slice::from_raw_parts(
                                s.as_ptr(),
                                s.len(),
                            ))
                        };
                    }
                }
            }
        }
        if let Some(f) = self.env.get_current_user {
            let (_uid, name) = f();
            con.write_str_raw(name);
            con.write_str_raw("@");
            con.write_str_raw(host_name);
            con.write_str_raw("> ");
        } else {
            con.write_str_raw(host_name);
            con.write_str_raw("> ");
        }
    }

    /// Parse and execute the line buffer.  Returns `true` if the shell
    /// should exit.
    fn execute<S: Serial>(&mut self, con: &mut Console<S>) -> bool {
        // Copy the line out so we don't borrow self.ed during dispatch.
        let len = self.ed.len;
        let mut buf = [0u8; 128];
        buf[..len].copy_from_slice(&self.ed.buf[..len]);

        let line = match core::str::from_utf8(&buf[..len]) {
            Ok(s) => s.trim(),
            Err(_) => {
                let _ = writeln!(con, "error: invalid UTF-8");
                return false;
            }
        };

        if line.is_empty() {
            return false;
        }

        // Split on top-level ';' for one-liners and run each statement.
        let mut exit = false;
        for seg in line.split(';') {
            let s = seg.trim();
            if s.is_empty() {
                continue;
            }
            if self.run_stmt(con, s) {
                exit = true;
                break;
            }
            if self.script_ctx.script_exit {
                self.script_ctx.script_exit = false;
                exit = true;
                break;
            }
        }
        exit
    }

    /// Execute a single statement through the script runtime.
    /// Handles control flow, variable expansion, and then dispatches to commands.
    fn run_stmt<S: Serial>(&mut self, con: &mut Console<S>, line: &str) -> bool {
        // ── Buffering mode (inside while/for body) ──────────────────────
        if self.script_ctx.buffering {
            let (kw, _) = split_first_word(line);
            match kw {
                "while" | "for" => {
                    self.script_ctx.buf_nest += 1;
                    self.script_ctx.push_line(line);
                    return false;
                }
                "done" => {
                    if self.script_ctx.buf_nest > 0 {
                        self.script_ctx.buf_nest -= 1;
                        self.script_ctx.push_line(line);
                        return false;
                    }
                    // Matched outer `done` — run the loop now.
                    self.script_ctx.buffering = false;
                    return self.run_buffered_loop(con);
                }
                _ => {
                    self.script_ctx.push_line(line);
                    return false;
                }
            }
        }

        // ── Control-flow keywords ───────────────────────────────────────
        let (cmd, args) = split_first_word(line);
        match cmd {
            "if" => {
                let parent_active = self.script_ctx.is_active();
                let cond = parent_active && self.eval_condition(con, args);
                let _ = self.script_ctx.push_block(script::BlockKind::If);
                if let Some(bs) = self.script_ctx.current_mut() {
                    bs.active = cond;
                    bs.if_taken = cond;
                }
                return false;
            }
            "then" | "do" => return false, // syntax sugar, no-op
            "elif" => {
                let parent_active = {
                    let d = self.script_ctx.depth;
                    if d < 2 {
                        true
                    } else {
                        self.script_ctx.blocks[d - 2].active
                    }
                };
                let already_taken = match self.script_ctx.current() {
                    Some(b) if matches!(b.kind, script::BlockKind::If) => b.if_taken,
                    _ => {
                        let _ = writeln!(con, "elif: not in if block");
                        return false;
                    }
                };
                if already_taken {
                    if let Some(bs) = self.script_ctx.current_mut() {
                        bs.active = false;
                    }
                    return false;
                }
                let cond = parent_active && self.eval_condition(con, args);
                if let Some(bs) = self.script_ctx.current_mut() {
                    bs.active = cond;
                    if cond {
                        bs.if_taken = true;
                    }
                }
                return false;
            }
            "else" => {
                let parent_active = {
                    let d = self.script_ctx.depth;
                    if d < 2 {
                        true
                    } else {
                        self.script_ctx.blocks[d - 2].active
                    }
                };
                if let Some(bs) = self.script_ctx.current_mut() {
                    if !matches!(bs.kind, script::BlockKind::If) {
                        let _ = writeln!(con, "else: not in if block");
                        return false;
                    }
                    bs.active = parent_active && !bs.if_taken;
                }
                return false;
            }
            "fi" => {
                self.script_ctx.pop_block();
                return false;
            }
            "while" => {
                let parent_active = self.script_ctx.is_active();
                let _ = self.script_ctx.push_block(script::BlockKind::While);
                self.script_ctx.buffering = true;
                self.script_ctx.buf_nest = 0;
                self.script_ctx.reset_lines();
                if let Some(bs) = self.script_ctx.current_mut() {
                    bs.active = parent_active;
                    let b = args.as_bytes();
                    let n = b.len().min(bs.cond.len());
                    bs.cond[..n].copy_from_slice(&b[..n]);
                    bs.cond_len = n;
                }
                return false;
            }
            "for" => {
                let parent_active = self.script_ctx.is_active();
                // Parse: VAR in w1 w2 ...
                let (var, rest) = split_first_word(args);
                let (in_kw, list) = split_first_word(rest);
                if in_kw != "in" {
                    let _ = writeln!(con, "for: syntax: for VAR in WORD...");
                    return false;
                }
                let _ = self.script_ctx.push_block(script::BlockKind::For);
                self.script_ctx.buffering = true;
                self.script_ctx.buf_nest = 0;
                self.script_ctx.reset_lines();
                if let Some(bs) = self.script_ctx.current_mut() {
                    bs.active = parent_active;
                    let vb = var.as_bytes();
                    let vn = vb.len().min(bs.for_var.len());
                    bs.for_var[..vn].copy_from_slice(&vb[..vn]);
                    bs.for_var_len = vn;
                    // Expand variables in list now so iteration is stable.
                    let mut exp = [0u8; 128];
                    let en = script::expand_vars(list, &self.script_vars, &mut exp);
                    let take = en.min(bs.for_list.len());
                    bs.for_list[..take].copy_from_slice(&exp[..take]);
                    bs.for_list_len = take;
                    bs.for_idx = 0;
                }
                return false;
            }
            "done" => {
                // Stray `done` outside buffering: close a loop block.
                self.script_ctx.pop_block();
                return false;
            }
            "break" => {
                self.script_ctx.loop_break = true;
                return false;
            }
            "continue" => {
                self.script_ctx.loop_continue = true;
                return false;
            }
            _ => {}
        }

        // ── Skip if we're in an inactive branch ─────────────────────────
        if !self.script_ctx.is_active() {
            return false;
        }

        // ── Bare NAME=value assignment ──────────────────────────────────
        if is_assignment_line(line) {
            self.handle_assignment(line);
            self.script_vars.last_status = 0;
            return false;
        }

        // ── Variable expansion, then dispatch ───────────────────────────
        let mut exp_buf = [0u8; 256];
        let n = script::expand_vars(line, &self.script_vars, &mut exp_buf);
        let expanded = match core::str::from_utf8(&exp_buf[..n]) {
            Ok(s) => s,
            Err(_) => line,
        };
        let (xcmd, xargs) = split_first_word(expanded);
        self.dispatch(con, xcmd, xargs)
    }

    /// Evaluate a condition (used by `if`, `elif`, `while`).
    /// Accepts `test ...`, `[ ... ]`, `true`, `false`, or any command
    /// whose exit status (via `last_status`) determines the result.
    fn eval_condition<S: Serial>(&mut self, con: &mut Console<S>, args: &str) -> bool {
        let mut exp = [0u8; 256];
        let n = script::expand_vars(args, &self.script_vars, &mut exp);
        let expanded = core::str::from_utf8(&exp[..n]).unwrap_or(args);
        let expanded = expanded.trim();
        if expanded.is_empty() {
            return false;
        }
        let (cmd, rest) = split_first_word(expanded);
        match cmd {
            "true" | ":" => true,
            "false" => false,
            "test" => script::eval_test(rest, None),
            "[" => {
                let t = rest.trim_end();
                let inner = t.strip_suffix(']').unwrap_or(t).trim_end();
                script::eval_test(inner, None)
            }
            _ => {
                self.script_vars.last_status = 0;
                self.dispatch(con, cmd, rest);
                self.script_vars.last_status == 0
            }
        }
    }

    /// Run the buffered body of a while/for loop.
    fn run_buffered_loop<S: Serial>(&mut self, con: &mut Console<S>) -> bool {
        let kind = match self.script_ctx.current() {
            Some(b) => b.kind,
            None => return false,
        };
        let active = self.script_ctx.current().map(|b| b.active).unwrap_or(false);
        let mut exit = false;
        if !active {
            // Inactive parent — drop body.
            self.script_ctx.pop_block();
            self.script_ctx.reset_lines();
            return false;
        }

        match kind {
            script::BlockKind::While => {
                let mut iters: u32 = 0;
                'outer: loop {
                    iters += 1;
                    if iters > 10_000 {
                        let _ = writeln!(con, "while: iteration limit exceeded");
                        break;
                    }
                    // Snapshot condition.
                    let mut cond_buf = [0u8; script::MAX_LINE];
                    let cond_len = {
                        let bs = match self.script_ctx.current() {
                            Some(b) => b,
                            None => break,
                        };
                        let n = bs.cond_len;
                        cond_buf[..n].copy_from_slice(&bs.cond[..n]);
                        n
                    };
                    let cond_str = core::str::from_utf8(&cond_buf[..cond_len]).unwrap_or("");
                    if !self.eval_condition(con, cond_str) {
                        break;
                    }

                    let n_lines = self.script_ctx.line_count;
                    for i in 0..n_lines {
                        let mut line_buf = [0u8; script::MAX_LINE];
                        let ln = {
                            let b = self.script_ctx.lines[i].as_str().as_bytes();
                            let m = b.len().min(line_buf.len());
                            line_buf[..m].copy_from_slice(&b[..m]);
                            m
                        };
                        let l = core::str::from_utf8(&line_buf[..ln]).unwrap_or("");
                        if self.run_stmt(con, l) {
                            exit = true;
                            break 'outer;
                        }
                        if self.script_ctx.script_exit {
                            exit = true;
                            break 'outer;
                        }
                        if self.script_ctx.loop_break {
                            self.script_ctx.loop_break = false;
                            break 'outer;
                        }
                        if self.script_ctx.loop_continue {
                            self.script_ctx.loop_continue = false;
                            break;
                        }
                    }
                }
            }
            script::BlockKind::For => {
                'outer2: loop {
                    // Pull name / list / idx.
                    let mut var_buf = [0u8; script::MAX_NAME];
                    let mut list_buf = [0u8; script::MAX_VAL];
                    let (var_len, list_len, idx) = {
                        let bs = match self.script_ctx.current() {
                            Some(b) => b,
                            None => break,
                        };
                        let vn = bs.for_var_len;
                        var_buf[..vn].copy_from_slice(&bs.for_var[..vn]);
                        let ln = bs.for_list_len;
                        list_buf[..ln].copy_from_slice(&bs.for_list[..ln]);
                        (vn, ln, bs.for_idx)
                    };
                    let var_name = core::str::from_utf8(&var_buf[..var_len]).unwrap_or("");
                    let list_str = core::str::from_utf8(&list_buf[..list_len]).unwrap_or("");
                    let mut word: Option<&str> = None;
                    for (ci, w) in list_str.split_whitespace().enumerate() {
                        if ci == idx {
                            word = Some(w);
                            break;
                        }
                    }
                    let Some(w) = word else {
                        break;
                    };
                    self.script_vars.set(var_name, w);
                    if let Some(bs) = self.script_ctx.current_mut() {
                        bs.for_idx += 1;
                    }

                    let n_lines = self.script_ctx.line_count;
                    for i in 0..n_lines {
                        let mut line_buf = [0u8; script::MAX_LINE];
                        let ln = {
                            let b = self.script_ctx.lines[i].as_str().as_bytes();
                            let m = b.len().min(line_buf.len());
                            line_buf[..m].copy_from_slice(&b[..m]);
                            m
                        };
                        let l = core::str::from_utf8(&line_buf[..ln]).unwrap_or("");
                        if self.run_stmt(con, l) {
                            exit = true;
                            break 'outer2;
                        }
                        if self.script_ctx.script_exit {
                            exit = true;
                            break 'outer2;
                        }
                        if self.script_ctx.loop_break {
                            self.script_ctx.loop_break = false;
                            break 'outer2;
                        }
                        if self.script_ctx.loop_continue {
                            self.script_ctx.loop_continue = false;
                            break;
                        }
                    }
                }
            }
            _ => {}
        }

        self.script_ctx.pop_block();
        self.script_ctx.reset_lines();
        exit
    }

    /// Handle `NAME=value` or `NAME="value with $vars"` assignment.
    fn handle_assignment(&mut self, line: &str) {
        if let Some(eq) = line.find('=') {
            let name = &line[..eq];
            let raw_val = &line[eq + 1..];
            let mut exp = [0u8; 256];
            let n = script::expand_vars(raw_val, &self.script_vars, &mut exp);
            let expanded = core::str::from_utf8(&exp[..n]).unwrap_or(raw_val);
            // Strip surrounding quotes if balanced.
            let bytes = expanded.as_bytes();
            let clean = if bytes.len() >= 2
                && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
                    || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
            {
                &expanded[1..expanded.len() - 1]
            } else {
                expanded
            };
            let _ = self.script_vars.set(name, clean);
        }
    }

    /// Main command dispatcher — runs a single (cmd, args) pair.
    fn dispatch<S: Serial>(&mut self, con: &mut Console<S>, cmd: &str, args: &str) -> bool {
        match cmd {
            "help" | "?" => self.cmd_help(con),
            "version" => self.cmd_version(con),
            "sysinfo" | "info" => self.cmd_sysinfo(con),
            "uptime" => self.cmd_uptime(con),
            "tasks" | "ps" => self.cmd_tasks(con),
            "meminfo" | "mem" | "free" => self.cmd_meminfo(con),
            "drivers" | "lsdrv" => self.cmd_drivers(con),
            "wifi" => self.cmd_wifi(con, args),
            "bt" | "ble" => self.cmd_bt(con, args),
            "zigbee" | "thread" | "802154" => self.cmd_zigbee(con, args),
            "sensor" => self.cmd_sensor(con, args),
            "clear" | "cls" => self.cmd_clear(con),
            "echo" => self.cmd_echo(con, args),
            "logo" => self.cmd_logo(con),
            "uname" => self.cmd_uname(con),
            "man" => self.cmd_man(con, args),
            "whoami" => self.cmd_whoami(con),
            "users" => self.cmd_users(con),
            #[cfg(feature = "multi-user")]
            "passwd" => self.cmd_passwd(con, args),
            #[cfg(feature = "multi-user")]
            "useradd" => self.cmd_useradd(con, args),
            #[cfg(feature = "multi-user")]
            "userdel" => self.cmd_userdel(con, args),
            #[cfg(feature = "multi-user")]
            "su" => self.cmd_su(con, args),
            #[cfg(feature = "multi-user")]
            "logout" => {
                if let Some(f) = self.env.logout {
                    f();
                }
                let _ = writeln!(con, "Logged out.");
                return true;
            }
            "vi" | "vim" | "edit" => self.cmd_vi(con, args),
            "history" => self.cmd_history(con, args),
            "set" => self.cmd_set(con, args),
            // ── file commands ────────────────────────────
            "ls" | "dir" => self.cmd_ls(con, args),
            "cat" | "type" => self.cmd_cat(con, args),
            "mkdir" => self.cmd_mkdir(con, args),
            "touch" => self.cmd_touch(con, args),
            "rm" | "rmdir" | "del" => self.cmd_rm(con, args),
            "cp" | "copy" => self.cmd_cp(con, args),
            "mv" | "move" | "rename" => self.cmd_mv(con, args),
            "pwd" => self.cmd_pwd(con),
            "cd" => self.cmd_cd(con, args),
            "stat" => self.cmd_stat(con, args),
            "hexdump" | "xxd" => self.cmd_hexdump(con, args),
            "write" => self.cmd_write(con, args),
            "tree" => self.cmd_tree(con, args),
            // ── storage commands ─────────────────────────
            "mount" => self.cmd_mount(con, args),
            "umount" | "unmount" => self.cmd_umount(con, args),
            "lsblk" => self.cmd_lsblk(con),
            "df" => self.cmd_df(con),
            // ── input device commands ────────────────────
            "input" => self.cmd_input(con, args),
            "lsusb" => self.cmd_lsusb(con),
            // ── hardware / bus commands ──────────────────
            "gpio" => self.cmd_gpio(con, args),
            "i2c" | "i2cdetect" => self.cmd_i2c(con, args),
            "spi" => self.cmd_spi(con, args),
            "hwinfo" | "devinfo" => self.cmd_hwinfo(con),
            "temp" => self.cmd_temp(con),
            "dmesg" => self.cmd_dmesg(con),
            "caps" => self.cmd_caps(con, args),
            "auditlog" | "audit" => self.cmd_auditlog(con, args),
            "ifconfig" | "ipconfig" | "ip" => self.cmd_ifconfig(con),
            "ping" => self.cmd_ping(con, args),
            "netstat" | "ss" => self.cmd_netstat(con),
            "ssh" => self.cmd_ssh(con, args),
            "hostname" => self.cmd_hostname(con, args),
            // ── AI-native commands ───────────────────
            "agents" => self.cmd_agents(con, args),
            "intent" => self.cmd_intent(con, args),
            "memory" | "kv" => self.cmd_memory(con, args),
            "fabric" => self.cmd_fabric(con, args),
            "peers" => self.cmd_peers(con, args),
            "mesh" => self.cmd_mesh(con, args),
            "zkp" => self.cmd_zkp(con, args),
            "demo" => self.cmd_demo(con, args),
            "reboot" => self.cmd_reboot(con),
            "shutdown" | "halt" | "poweroff" => self.cmd_shutdown(con),
            // ── scripting builtins ──────────────────────
            "sleep" => self.cmd_sleep(con, args),
            "let" => self.cmd_let(con, args),
            "test" => {
                self.script_vars.last_status = if script::eval_test(args, None) { 0 } else { 1 };
            }
            "[" => {
                let t = args.trim_end();
                let inner = t.strip_suffix(']').unwrap_or(t).trim_end();
                self.script_vars.last_status = if script::eval_test(inner, None) { 0 } else { 1 };
            }
            "true" | ":" => {
                self.script_vars.last_status = 0;
            }
            "false" => {
                self.script_vars.last_status = 1;
            }
            "unset" => self.cmd_unset(con, args),
            "vars" | "env" => self.cmd_vars(con),
            "export" => self.cmd_export(con, args),
            "source" | "." => self.cmd_source(con, args),
            "return" => {
                let code = args.trim().parse::<u8>().unwrap_or(0);
                self.script_vars.last_status = code;
                self.script_ctx.script_exit = true;
            }
            "exit" | "quit" => {
                let code = args.trim().parse::<u8>().unwrap_or(0);
                self.script_vars.last_status = code;
                let _ = writeln!(con, "Goodbye.");
                return true;
            }
            _ => {
                let _ = writeln!(con, "unknown command: '{}'", cmd);
                let _ = writeln!(con, "Type 'help' for available commands.");
                self.script_vars.last_status = 127;
            }
        }

        false
    }

    // ── scripting builtins ───────────────────────────────────────────────

    fn cmd_sleep<S: Serial>(&mut self, con: &mut Console<S>, args: &str) {
        let a = args.trim();
        // Accept "500", "500ms", "2s"
        let (num_str, mult): (&str, u64) = if let Some(n) = a.strip_suffix("ms") {
            (n.trim(), 1)
        } else if let Some(n) = a.strip_suffix('s') {
            (n.trim(), 1000)
        } else {
            (a, 1)
        };
        match num_str.parse::<u64>() {
            Ok(n) => {
                let ms = n.saturating_mul(mult);
                if let Some(f) = self.env.sleep_ms {
                    f(ms);
                    self.script_vars.last_status = 0;
                } else {
                    let _ = writeln!(con, "sleep: not supported on this platform");
                    self.script_vars.last_status = 1;
                }
            }
            Err(_) => {
                let _ = writeln!(con, "sleep: invalid duration '{}'", args);
                self.script_vars.last_status = 2;
            }
        }
    }

    fn cmd_let<S: Serial>(&mut self, con: &mut Console<S>, args: &str) {
        let a = args.trim();
        let Some(eq) = a.find('=') else {
            let _ = writeln!(con, "let: usage: let NAME=EXPR");
            self.script_vars.last_status = 2;
            return;
        };
        let name = a[..eq].trim();
        let raw_expr = &a[eq + 1..];
        let mut exp = [0u8; 128];
        let n = script::expand_vars(raw_expr, &self.script_vars, &mut exp);
        let expr = core::str::from_utf8(&exp[..n]).unwrap_or(raw_expr);
        let v = script::eval_arith(expr);
        let mut nbuf = [0u8; 12];
        let nn = script::fmt_i32(&mut nbuf, v);
        let vs = core::str::from_utf8(&nbuf[..nn]).unwrap_or("0");
        let _ = self.script_vars.set(name, vs);
        self.script_vars.last_status = 0;
    }

    fn cmd_unset<S: Serial>(&mut self, _con: &mut Console<S>, args: &str) {
        for name in args.split_whitespace() {
            self.script_vars.unset(name);
        }
        self.script_vars.last_status = 0;
    }

    fn cmd_vars<S: Serial>(&self, con: &mut Console<S>) {
        for (name, val) in self.script_vars.iter() {
            let _ = writeln!(con, "{}={}", name, val);
        }
        let _ = writeln!(con, "?={}", self.script_vars.last_status);
    }

    fn cmd_export<S: Serial>(&mut self, con: &mut Console<S>, args: &str) {
        // `export NAME=value` behaves like plain assignment here (no env separation).
        let a = args.trim();
        if a.is_empty() {
            self.cmd_vars(con);
            return;
        }
        if a.find('=').is_some() {
            self.handle_assignment(a);
            self.script_vars.last_status = 0;
        } else {
            // Just re-affirm existing var (no-op if unset).
            self.script_vars.last_status = 0;
        }
    }

    fn cmd_source<S: Serial>(&mut self, con: &mut Console<S>, args: &str) {
        let path = args.trim();
        if path.is_empty() {
            let _ = writeln!(con, "source: missing file");
            self.script_vars.last_status = 2;
            return;
        }
        let Some(read_fn) = self.env.vfs_read_file else {
            let _ = writeln!(con, "source: VFS not available");
            self.script_vars.last_status = 1;
            return;
        };
        // Use a fixed-size buffer; scripts must fit.
        let mut buf = [0u8; 4096];
        let n = read_fn(path, &mut buf);
        if n == 0 {
            let _ = writeln!(con, "source: cannot read '{}'", path);
            self.script_vars.last_status = 1;
            return;
        }
        let content = match core::str::from_utf8(&buf[..n]) {
            Ok(s) => s,
            Err(_) => {
                let _ = writeln!(con, "source: '{}' is not valid UTF-8", path);
                self.script_vars.last_status = 1;
                return;
            }
        };
        // Copy content out so we don't borrow buf across run_stmt calls that may
        // mutate internal buffers (buf is on our stack, safe to alias immutably).
        // We iterate line-by-line and run each through run_stmt.
        for raw_line in content.lines() {
            // Strip comments (# at start of a trimmed line).
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('#') {
                continue;
            }
            // Still honor ; separators by running through run_stmt per segment.
            for seg in line.split(';') {
                let s = seg.trim();
                if s.is_empty() {
                    continue;
                }
                if self.run_stmt(con, s) {
                    self.script_ctx.script_exit = true;
                    return;
                }
                if self.script_ctx.script_exit {
                    return;
                }
            }
        }
    }

    // ── built-in commands ────────────────────────────────────────────────

    fn cmd_help<S: Serial>(&self, con: &mut Console<S>) {
        let _ = writeln!(con, "");
        let _ = writeln!(con, "  VeerOS Shell \u{2014} built-in commands");
        let _ = writeln!(con, "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        let _ = writeln!(con, "  help       Show this message");
        let _ = writeln!(con, "  version    Print kernel version");
        let _ = writeln!(con, "  sysinfo    System information");
        let _ = writeln!(con, "  uptime     Kernel uptime (ticks)");
        let _ = writeln!(con, "  tasks      List running tasks");
        let _ = writeln!(con, "  uname      Print system name");
        let _ = writeln!(con, "  meminfo    Memory pool statistics");
        let _ = writeln!(con, "  drivers    List registered drivers");
        let _ = writeln!(con, "  wifi       Wi-Fi (scan/list/set/connect/status)");
        let _ = writeln!(
            con,
            "  bt         Bluetooth LE (scan/list/advertise/stop/status)"
        );
        let _ = writeln!(
            con,
            "  zigbee     ZigBee/Thread 802.15.4 (init/scan/channel/send/status)"
        );
        let _ = writeln!(con, "  sensor     Virtual sensors (list/read/set/status)");
        let _ = writeln!(con, "  clear      Clear the screen");
        let _ = writeln!(con, "  echo       Echo arguments");
        let _ = writeln!(con, "  logo       Display VeerOS logo");
        let _ = writeln!(con, "  man        Show manual page (man <topic>)");
        let _ = writeln!(con, "  whoami     Display current user");
        let _ = writeln!(con, "  users      List active user sessions");
        #[cfg(feature = "multi-user")]
        {
            let _ = writeln!(con, "  passwd     Change user password");
            let _ = writeln!(con, "  useradd    Add a new user (root only)");
            let _ = writeln!(con, "  userdel    Remove a user (root only)");
            let _ = writeln!(con, "  su         Switch user");
            let _ = writeln!(con, "  logout     End session and return to login");
        }
        let _ = writeln!(con, "  vi         Text editor (vi <optional text>)");
        let _ = writeln!(con, "  history    Show command history");
        let _ = writeln!(con, "  set        View/set shell variables");
        let _ = writeln!(con, "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500} files \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        let _ = writeln!(con, "  ls         List directory contents");
        let _ = writeln!(con, "  cat        Display file contents");
        let _ = writeln!(con, "  mkdir      Create a directory");
        let _ = writeln!(con, "  touch      Create an empty file");
        let _ = writeln!(con, "  rm         Remove file or directory");
        let _ = writeln!(con, "  cp         Copy a file");
        let _ = writeln!(con, "  mv         Move / rename a path");
        let _ = writeln!(con, "  pwd        Print working directory");
        let _ = writeln!(con, "  cd         Change directory");
        let _ = writeln!(con, "  stat       Show file/dir metadata");
        let _ = writeln!(con, "  hexdump    Hex dump of file");
        let _ = writeln!(con, "  write      Write text to a file");
        let _ = writeln!(con, "  tree       Recursive directory tree");
        let _ = writeln!(con, "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500} storage \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        let _ = writeln!(con, "  mount      Mount filesystem / list mounts");
        let _ = writeln!(con, "  umount     Unmount a filesystem");
        let _ = writeln!(con, "  lsblk      List block devices");
        let _ = writeln!(con, "  df         Show filesystem disk space usage");
        let _ = writeln!(con, "  ────────── input ─────────────────");
        let _ = writeln!(con, "  input      Input device status");
        let _ = writeln!(con, "  lsusb      List USB devices");
        let _ = writeln!(con, "  ────────── hardware ──────────────");
        let _ = writeln!(con, "  gpio       GPIO pin control (list/read/write/mode)");
        let _ = writeln!(con, "  i2c        I\u{00B2}C bus (scan/read/write)");
        let _ = writeln!(con, "  spi        SPI bus (cfg/xfer)");
        let _ = writeln!(con, "  hwinfo     Hardware info (board/memory/thermal)");
        let _ = writeln!(con, "  temp       SoC temperature readout");
        let _ = writeln!(con, "  dmesg      Kernel log buffer");
        let _ = writeln!(con, "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500} security \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        let _ = writeln!(con, "  caps       Process capabilities (list/show/drop)");
        let _ = writeln!(con, "  auditlog   Security audit event log");
        let _ = writeln!(con, "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500} networking \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        let _ = writeln!(con, "  ifconfig   Network interface configuration");
        let _ = writeln!(con, "  ping       Send network probes to an IP");
        let _ = writeln!(con, "  netstat    Socket / connection status");
        let _ = writeln!(con, "  ssh        SSH client (ssh user@host[:port])");
        let _ = writeln!(con, "  hostname   Get/set system hostname");
        let _ = writeln!(con, "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500} ai-native \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        let _ = writeln!(con, "  agents     Agent lifecycle (list/spawn/kill/status)");
        let _ = writeln!(
            con,
            "  intent     Intent engine (submit/status/cancel/stats)"
        );
        let _ = writeln!(con, "  memory     Persistent memory (get/set/stats)");
        let _ = writeln!(con, "  fabric     Execution fabric node status");
        let _ = writeln!(con, "  demo       Run AI-native interactive walkthrough");
        let _ = writeln!(con, "  ────────── distributed fabric ───");
        let _ = writeln!(con, "  peers      Zero Trust peer management");
        let _ = writeln!(con, "  mesh       Mesh transport status/routes");
        let _ = writeln!(con, "  zkp        Zero-Knowledge Proof ops");
        let _ = writeln!(con, "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
        let _ = writeln!(con, "  reboot     Reboot the system");
        let _ = writeln!(con, "  shutdown   Halt / power off");
        let _ = writeln!(con, "  exit       Exit the shell");
        let _ = writeln!(con, "");
    }

    fn cmd_version<S: Serial>(&self, con: &mut Console<S>) {
        let _ = writeln!(con, "VeerOS v{}", self.env.version);
    }

    fn cmd_sysinfo<S: Serial>(&self, con: &mut Console<S>) {
        let _ = writeln!(con, "  Platform  : {}", self.env.platform);
        let _ = writeln!(con, "  Scheduler : {}", self.env.scheduler);
        let _ = writeln!(con, "  Version   : v{}", self.env.version);
    }

    fn cmd_clear<S: Serial>(&self, con: &mut Console<S>) {
        // ANSI: clear screen + cursor home
        con.write_str_raw("\x1B[2J\x1B[H");
    }

    fn cmd_echo<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        // Check for output redirection: echo text >> file  or  echo text > file
        if let Some(pos) = args.find(">>") {
            let text = args[..pos].trim();
            let file = args[pos + 2..].trim();
            if !file.is_empty() {
                if let Some(f) = self.env.vfs_write_file {
                    if !f(file, text.as_bytes(), true) {
                        let _ = writeln!(con, "echo: cannot append to '{}'", file);
                    }
                } else {
                    let _ = writeln!(con, "filesystem not available");
                }
                return;
            }
        } else if let Some(pos) = args.find('>') {
            let text = args[..pos].trim();
            let file = args[pos + 1..].trim();
            if !file.is_empty() {
                if let Some(f) = self.env.vfs_write_file {
                    if !f(file, text.as_bytes(), false) {
                        let _ = writeln!(con, "echo: cannot write to '{}'", file);
                    }
                } else {
                    let _ = writeln!(con, "filesystem not available");
                }
                return;
            }
        }
        let _ = writeln!(con, "{}", args);
    }

    fn cmd_uname<S: Serial>(&self, con: &mut Console<S>) {
        let _ = writeln!(
            con,
            "VeerOS {} {} {}",
            self.env.version, self.env.platform, self.env.scheduler
        );
    }

    fn cmd_uptime<S: Serial>(&self, con: &mut Console<S>) {
        if let Some(f) = self.env.get_uptime_ticks {
            let ticks = f();
            let secs = ticks / 1_000;
            let ms = ticks % 1_000;
            let _ = writeln!(con, "  uptime: {}.{:03}s  ({} ticks)", secs, ms, ticks);
        } else {
            let _ = writeln!(con, "  uptime: not available (host demo)");
        }
    }

    fn cmd_tasks<S: Serial>(&self, con: &mut Console<S>) {
        if let Some(f) = self.env.get_task_list {
            f(con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  task list: not available (host demo)");
        }
    }

    fn cmd_meminfo<S: Serial>(&self, con: &mut Console<S>) {
        if let Some(f) = self.env.get_mem_info {
            f(con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  meminfo: not available (no allocator)");
        }
    }

    fn cmd_drivers<S: Serial>(&self, con: &mut Console<S>) {
        if let Some(f) = self.env.get_driver_list {
            f(con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  drivers: not available");
        }
    }

    fn cmd_wifi<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if let Some(f) = self.env.wifi_cmd {
            // Split "set MySSID MyPass" into sub="set", rest="MySSID MyPass"
            let (sub, rest) = match args.find(' ') {
                Some(i) => (&args[..i], args[i + 1..].trim()),
                None => (args, ""),
            };
            f(sub, rest, con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  wifi: not available on this platform");
        }
    }

    fn cmd_bt<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if let Some(f) = self.env.bt_cmd {
            let (sub, rest) = match args.find(' ') {
                Some(i) => (&args[..i], args[i + 1..].trim()),
                None => (args, ""),
            };
            f(sub, rest, con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  bt: not available on this platform");
        }
    }

    fn cmd_zigbee<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if let Some(f) = self.env.zigbee_cmd {
            let (sub, rest) = match args.find(' ') {
                Some(i) => (&args[..i], args[i + 1..].trim()),
                None => (args, ""),
            };
            f(sub, rest, con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  zigbee: not available on this platform");
        }
    }

    fn cmd_sensor<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if let Some(f) = self.env.sensor_cmd {
            let (sub, rest) = match args.find(' ') {
                Some(i) => (&args[..i], args[i + 1..].trim()),
                None => (args, ""),
            };
            f(sub, rest, con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  sensor: not available on this platform");
        }
    }

    fn cmd_logo<S: Serial>(&self, con: &mut Console<S>) {
        let _ = writeln!(con, "");
        let _ = writeln!(con, "  ╦  ╦┌─┐┌─┐┬─┐╔═╗╔═╗");
        let _ = writeln!(con, "  ╚╗╔╝├┤ ├┤ ├┬┘║ ║╚═╗");
        let _ = writeln!(con, "   ╚╝ └─┘└─┘┴└─╚═╝╚═╝");
        let _ = writeln!(con, "    Microkernel v{}", self.env.version);
        let _ = writeln!(con, "");
    }

    fn cmd_whoami<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.get_current_user {
            Some(f) => {
                let (uid, name) = f();
                let _ = writeln!(con, "{} (uid={})", name, uid);
            }
            None => {
                let _ = writeln!(con, "root (uid=0)");
            }
        }
    }

    fn cmd_users<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.get_user_list {
            Some(f) => f(con),
            None => {
                let _ = writeln!(con, "root     console  (active)");
            }
        }
    }

    fn cmd_vi<S: Serial>(&mut self, con: &mut Console<S>, args: &str) {
        let mut editor = vi::Vi::new();
        // Apply shell vi settings
        editor.settings.number = self.vars.vi_number;
        editor.settings.tabstop = self.vars.vi_tabstop;
        editor.settings.showmatch = self.vars.vi_showmatch;
        editor.settings.autoindent = self.vars.vi_autoindent;
        let path = args.trim();
        if !path.is_empty() {
            if let Some(read_fn) = self.env.vfs_read_file {
                let mut load_buf = [0u8; VI_SAVE_BUF_SIZE];
                let n = read_fn(path, &mut load_buf);
                if n > 0 {
                    if let Ok(text) = core::str::from_utf8(&load_buf[..n]) {
                        editor.load(text);
                    }
                } else if self.path_exists(path) {
                    editor.load("");
                }
            }
            editor.set_filename(path);
        }
        let write_fn = self.env.vfs_write_file;
        let _saved = editor.run_with_save(con, |ed| {
            let Some(write_fn) = write_fn else {
                return false;
            };
            if path.is_empty() {
                return false;
            }
            let mut save_buf = [0u8; VI_SAVE_BUF_SIZE];
            let Some(n) = ed.write_contents_to_slice(&mut save_buf) else {
                return false;
            };
            write_fn(path, &save_buf[..n], false)
        });
    }

    fn cmd_history<S: Serial>(&mut self, con: &mut Console<S>, args: &str) {
        let count = self.ed.history.len();

        // "history clear" — clear history
        if args == "clear" {
            self.ed.history = History::new();
            let _ = writeln!(con, "  history cleared");
            return;
        }

        if count == 0 {
            let _ = writeln!(con, "  (no history)");
            return;
        }

        // Optionally limit: "history N" shows last N entries
        let show = if !args.is_empty() {
            if let Ok(n) = parse_usize_simple(args) {
                if n < count {
                    n
                } else {
                    count
                }
            } else {
                count
            }
        } else {
            count
        };

        let start = count - show;
        for i in start..count {
            if let Some((data, len)) = self.ed.history.get_absolute(i) {
                if let Ok(s) = core::str::from_utf8(&data[..len]) {
                    let _ = writeln!(con, "  {:>4}  {}", i + 1, s);
                }
            }
        }
    }

    fn cmd_set<S: Serial>(&mut self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            // Show all variables
            let _ = writeln!(con, "  Shell variables:");
            let _ = writeln!(
                con,
                "    number      = {}",
                if self.vars.vi_number { "on" } else { "off" }
            );
            let _ = writeln!(con, "    tabstop     = {}", self.vars.vi_tabstop);
            let _ = writeln!(
                con,
                "    showmatch   = {}",
                if self.vars.vi_showmatch { "on" } else { "off" }
            );
            let _ = writeln!(
                con,
                "    autoindent  = {}",
                if self.vars.vi_autoindent { "on" } else { "off" }
            );
            if self.vars.prompt_len > 0 {
                if let Ok(s) = core::str::from_utf8(&self.vars.prompt_str[..self.vars.prompt_len]) {
                    let _ = writeln!(con, "    prompt      = \"{}\"", s);
                }
            } else {
                let _ = writeln!(con, "    prompt      = (default)");
            }
            return;
        }

        // Parse "key=value" or "key value" or "nokey"
        let (key, val) = if let Some(eq) = args.find('=') {
            (&args[..eq], args[eq + 1..].trim())
        } else if let Some(sp) = args.find(' ') {
            (&args[..sp], args[sp + 1..].trim())
        } else {
            (args, "")
        };

        match key {
            "number" => {
                self.vars.vi_number = !matches!(val, "off" | "0" | "false" | "no");
                let _ = writeln!(
                    con,
                    "  number = {}",
                    if self.vars.vi_number { "on" } else { "off" }
                );
            }
            "nonumber" => {
                self.vars.vi_number = false;
                let _ = writeln!(con, "  number = off");
            }
            "tabstop" | "ts" => {
                if let Ok(n) = parse_u8_simple(val) {
                    if n > 0 && n <= 16 {
                        self.vars.vi_tabstop = n;
                    }
                }
                let _ = writeln!(con, "  tabstop = {}", self.vars.vi_tabstop);
            }
            "showmatch" | "sm" => {
                self.vars.vi_showmatch = !matches!(val, "off" | "0" | "false" | "no");
                let _ = writeln!(
                    con,
                    "  showmatch = {}",
                    if self.vars.vi_showmatch { "on" } else { "off" }
                );
            }
            "noshowmatch" | "nosm" => {
                self.vars.vi_showmatch = false;
                let _ = writeln!(con, "  showmatch = off");
            }
            "autoindent" | "ai" => {
                self.vars.vi_autoindent = !matches!(val, "off" | "0" | "false" | "no");
                let _ = writeln!(
                    con,
                    "  autoindent = {}",
                    if self.vars.vi_autoindent { "on" } else { "off" }
                );
            }
            "noautoindent" | "noai" => {
                self.vars.vi_autoindent = false;
                let _ = writeln!(con, "  autoindent = off");
            }
            "prompt" => {
                let bytes = val.as_bytes();
                let take = if bytes.len() > 32 { 32 } else { bytes.len() };
                self.vars.prompt_str[..take].copy_from_slice(&bytes[..take]);
                self.vars.prompt_len = take;
                let _ = writeln!(con, "  prompt updated");
            }
            _ => {
                let _ = writeln!(con, "  unknown variable: '{}'", key);
                let _ = writeln!(
                    con,
                    "  Variables: number tabstop showmatch autoindent prompt"
                );
            }
        }
    }

    fn cmd_man<S: Serial>(&self, con: &mut Console<S>, topic: &str) {
        if topic.is_empty() {
            let _ = writeln!(con, "Usage: man <topic>");
            let _ = writeln!(con, "");
            let _ = writeln!(con, "Available topics:");
            for &(name, _) in MAN_PAGES {
                let _ = writeln!(con, "  {}", name);
            }
            return;
        }
        for &(name, text) in MAN_PAGES {
            if name == topic {
                let _ = writeln!(con, "");
                for line in text.lines() {
                    let _ = writeln!(con, "  {}", line);
                }
                let _ = writeln!(con, "");
                return;
            }
        }
        let _ = writeln!(con, "No manual entry for '{}'", topic);
    }

    // ── file commands ────────────────────────────────────────────────────

    fn cmd_ls<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        match self.env.vfs_list_dir {
            Some(f) => {
                let mut path = ".";
                for part in args.split_whitespace() {
                    if !part.starts_with('-') {
                        path = part;
                    }
                }
                f(path, con);
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn cmd_cat<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            let _ = writeln!(con, "Usage: cat <file>");
            return;
        }
        match self.env.vfs_read_file {
            Some(f) => {
                let mut buf = [0u8; 1024];
                let n = f(args, &mut buf);
                if n == 0 {
                    if self.path_exists(args) {
                        return;
                    }
                    let _ = writeln!(con, "cat: cannot read '{}'", args);
                } else if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                    // Write without extra trailing newline if content already ends with one
                    con.write_str_raw(s);
                    if !s.ends_with('\n') {
                        let _ = writeln!(con, "");
                    }
                } else {
                    let _ = writeln!(con, "cat: '{}': binary file ({} bytes)", args, n);
                }
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn path_exists(&self, path: &str) -> bool {
        struct StatBuffer {
            data: [u8; 96],
            len: usize,
        }

        impl core::fmt::Write for StatBuffer {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                let remaining = self.data.len().saturating_sub(self.len);
                let take = remaining.min(s.len());
                self.data[self.len..self.len + take].copy_from_slice(&s.as_bytes()[..take]);
                self.len += take;
                Ok(())
            }
        }

        let Some(stat_fn) = self.env.vfs_stat else {
            return true;
        };
        let mut stat = StatBuffer {
            data: [0u8; 96],
            len: 0,
        };
        stat_fn(path, &mut stat);
        match core::str::from_utf8(&stat.data[..stat.len]) {
            Ok(text) => !text.contains("no such file or directory"),
            Err(_) => true,
        }
    }

    fn cmd_mkdir<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            let _ = writeln!(con, "Usage: mkdir <dir>");
            return;
        }
        match self.env.vfs_mkdir {
            Some(f) => {
                if !f(args) {
                    let _ = writeln!(con, "mkdir: cannot create '{}'", args);
                }
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn cmd_touch<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            let _ = writeln!(con, "Usage: touch <file>");
            return;
        }
        match self.env.vfs_touch {
            Some(f) => {
                if !f(args) {
                    let _ = writeln!(con, "touch: cannot create '{}'", args);
                }
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn cmd_rm<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            let _ = writeln!(con, "Usage: rm <path>");
            return;
        }
        match self.env.vfs_unlink {
            Some(f) => {
                if !f(args) {
                    let _ = writeln!(con, "rm: cannot remove '{}'", args);
                }
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn cmd_cp<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        // Parse "src dst"
        let (src, dst) = match args.find(' ') {
            Some(i) => (&args[..i], args[i + 1..].trim()),
            None => {
                let _ = writeln!(con, "Usage: cp <src> <dst>");
                return;
            }
        };
        if dst.is_empty() {
            let _ = writeln!(con, "Usage: cp <src> <dst>");
            return;
        }
        let read_fn = match self.env.vfs_read_file {
            Some(f) => f,
            None => {
                let _ = writeln!(con, "filesystem not available");
                return;
            }
        };
        let write_fn = match self.env.vfs_write_file {
            Some(f) => f,
            None => {
                let _ = writeln!(con, "filesystem not available");
                return;
            }
        };
        let mut buf = [0u8; 1024];
        let n = read_fn(src, &mut buf);
        if n == 0 {
            let _ = writeln!(con, "cp: cannot read '{}'", src);
            return;
        }
        if !write_fn(dst, &buf[..n], false) {
            let _ = writeln!(con, "cp: cannot write '{}'", dst);
        }
    }

    fn cmd_mv<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let (src, dst) = match args.find(' ') {
            Some(i) => (&args[..i], args[i + 1..].trim()),
            None => {
                let _ = writeln!(con, "Usage: mv <src> <dst>");
                return;
            }
        };
        if dst.is_empty() {
            let _ = writeln!(con, "Usage: mv <src> <dst>");
            return;
        }
        match self.env.vfs_rename {
            Some(f) => {
                if !f(src, dst) {
                    let _ = writeln!(con, "mv: cannot move '{}' to '{}'", src, dst);
                }
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn cmd_pwd<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.vfs_getcwd {
            Some(f) => {
                let mut buf = [0u8; 128];
                let n = f(&mut buf);
                if n > 0 {
                    if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                        let _ = writeln!(con, "{}", s);
                    }
                } else {
                    let _ = writeln!(con, "/");
                }
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn cmd_cd<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let path = if args.is_empty() { "/" } else { args };
        match self.env.vfs_chdir {
            Some(f) => {
                if !f(path) {
                    let _ = writeln!(con, "cd: no such directory: '{}'", path);
                }
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn cmd_stat<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            let _ = writeln!(con, "Usage: stat <path>");
            return;
        }
        match self.env.vfs_stat {
            Some(f) => f(args, con),
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn cmd_hexdump<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            let _ = writeln!(con, "Usage: hexdump <file>");
            return;
        }
        let read_fn = match self.env.vfs_read_file {
            Some(f) => f,
            None => {
                let _ = writeln!(con, "filesystem not available");
                return;
            }
        };
        let mut buf = [0u8; 512];
        let n = read_fn(args, &mut buf);
        if n == 0 {
            let _ = writeln!(con, "hexdump: cannot read '{}'", args);
            return;
        }
        let mut off = 0usize;
        while off < n {
            // Address
            let _ = write!(con, "{:08x}  ", off);
            // Hex bytes (16 per line)
            let end = if off + 16 <= n { off + 16 } else { n };
            for i in off..off + 16 {
                if i < end {
                    let _ = write!(con, "{:02x} ", buf[i]);
                } else {
                    con.write_str_raw("   ");
                }
                if i == off + 7 {
                    con.write_str_raw(" ");
                }
            }
            con.write_str_raw(" |");
            // ASCII
            for i in off..end {
                let c = buf[i];
                if c >= 0x20 && c < 0x7F {
                    let _ = write!(con, "{}", c as char);
                } else {
                    con.write_str_raw(".");
                }
            }
            let _ = writeln!(con, "|");
            off += 16;
        }
        let _ = writeln!(con, "{:08x}", n);
    }

    fn cmd_write<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        // write <file> <text...>
        let (path, text) = match args.find(' ') {
            Some(i) => (&args[..i], args[i + 1..].trim()),
            None => {
                let _ = writeln!(con, "Usage: write <file> <text>");
                return;
            }
        };
        if text.is_empty() {
            let _ = writeln!(con, "Usage: write <file> <text>");
            return;
        }
        match self.env.vfs_write_file {
            Some(f) => {
                if !f(path, text.as_bytes(), false) {
                    let _ = writeln!(con, "write: cannot write to '{}'", path);
                }
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    fn cmd_tree<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        match self.env.vfs_tree {
            Some(f) => {
                let path = if args.is_empty() { "/" } else { args };
                f(path, con);
            }
            None => {
                let _ = writeln!(con, "filesystem not available");
            }
        }
    }

    // ── storage commands ─────────────────────────────────────────────

    fn cmd_mount<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            // No arguments → list mounts.
            match self.env.mount_list {
                Some(f) => f(con),
                None => {
                    let _ = writeln!(con, "mount: not available");
                }
            }
            return;
        }
        // mount <device> <path>
        let (dev, path) = match args.find(' ') {
            Some(i) => (args[..i].trim(), args[i + 1..].trim()),
            None => {
                let _ = writeln!(con, "usage: mount <device> <path>");
                return;
            }
        };
        match self.env.mount_fs {
            Some(f) => {
                if f(dev, path) {
                    let _ = writeln!(con, "mounted {} at {}", dev, path);
                } else {
                    let _ = writeln!(con, "mount: failed to mount {} at {}", dev, path);
                }
            }
            None => {
                let _ = writeln!(con, "mount: not available");
            }
        }
    }

    fn cmd_umount<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            let _ = writeln!(con, "usage: umount <path>");
            return;
        }
        match self.env.umount_fs {
            Some(f) => {
                if f(args) {
                    let _ = writeln!(con, "unmounted {}", args);
                } else {
                    let _ = writeln!(con, "umount: {} not mounted", args);
                }
            }
            None => {
                let _ = writeln!(con, "umount: not available");
            }
        }
    }

    fn cmd_lsblk<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.lsblk {
            Some(f) => f(con),
            None => {
                let _ = writeln!(con, "lsblk: not available");
            }
        }
    }

    fn cmd_df<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.df_cmd {
            Some(f) => f(con),
            None => {
                let _ = writeln!(con, "df: not available");
            }
        }
    }

    fn cmd_input<S: Serial>(&self, con: &mut Console<S>, _args: &str) {
        let _ = writeln!(con, "── Input Devices ──");
        match self.env.input_status {
            Some(f) => f(con),
            None => {
                let _ = writeln!(con, "  input subsystem: not available");
            }
        }
        // Show BLE HID devices if callback available.
        if let Some(f) = self.env.ble_hid_list {
            let _ = writeln!(con, "── BLE HID ──");
            f(con);
        }
        // Show USB devices if callback available.
        if let Some(f) = self.env.usb_list {
            let _ = writeln!(con, "── USB ──");
            f(con);
        }
    }

    fn cmd_lsusb<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.usb_list {
            Some(f) => f(con),
            None => {
                let _ = writeln!(con, "lsusb: not available on this platform");
            }
        }
    }

    // ── GPIO ─────────────────────────────────────────────────────────

    fn cmd_gpio<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let (sub, rest) = split_first_word(args);
        match self.env.gpio_cmd {
            Some(f) => {
                if sub.is_empty() {
                    f("list", "", con);
                } else {
                    f(sub, rest, con);
                }
            }
            None => {
                let _ = writeln!(con, "gpio: not available on this platform");
            }
        }
    }

    // ── I2C ──────────────────────────────────────────────────────────

    fn cmd_i2c<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let (sub, rest) = split_first_word(args);
        match self.env.i2c_cmd {
            Some(f) => {
                if sub.is_empty() {
                    let _ = writeln!(con, "Usage: i2c <scan|read|write> [bus] [addr] [reg] [val]");
                    let _ = writeln!(
                        con,
                        "  i2c scan [bus]         Scan for devices (bus 0-6, default 1)"
                    );
                    let _ = writeln!(con, "  i2c read <bus> <addr> <reg>    Read register");
                    let _ = writeln!(con, "  i2c write <bus> <addr> <reg> <val>  Write register");
                } else {
                    f(sub, rest, con);
                }
            }
            None => {
                let _ = writeln!(con, "i2c: not available on this platform");
            }
        }
    }

    // ── SPI ──────────────────────────────────────────────────────────

    fn cmd_spi<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let (sub, rest) = split_first_word(args);
        match self.env.spi_cmd {
            Some(f) => {
                if sub.is_empty() {
                    let _ = writeln!(con, "Usage: spi <xfer|cfg> [bus] [data...]");
                    let _ = writeln!(con, "  spi cfg <bus> <mode> <freq_div>  Configure SPI bus");
                    let _ = writeln!(con, "  spi xfer <bus> <hex_bytes>       Transfer bytes");
                } else {
                    f(sub, rest, con);
                }
            }
            None => {
                let _ = writeln!(con, "spi: not available on this platform");
            }
        }
    }

    // ── Hardware info ────────────────────────────────────────────────

    fn cmd_hwinfo<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.hw_info {
            Some(f) => f(con),
            None => {
                let _ = writeln!(con, "hwinfo: not available on this platform");
            }
        }
    }

    fn cmd_temp<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.get_temp_millic {
            Some(f) => {
                let mc = f();
                let deg = mc / 1000;
                let frac = ((mc % 1000).unsigned_abs() / 100) as u32;
                let _ = writeln!(con, "  SoC temperature: {}.{}°C", deg, frac);
            }
            None => {
                let _ = writeln!(con, "temp: not available on this platform");
            }
        }
    }

    fn cmd_dmesg<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.dmesg {
            Some(f) => f(con),
            None => {
                let _ = writeln!(con, "dmesg: no kernel log available");
            }
        }
    }

    fn cmd_auditlog<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        match self.env.auditlog_cmd {
            Some(f) => f(args, con as &mut dyn core::fmt::Write),
            None => {
                let _ = writeln!(con, "auditlog: not available");
            }
        }
    }

    fn cmd_caps<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        match self.env.caps_cmd {
            Some(f) => {
                let (sub, rest) = match args.find(' ') {
                    Some(i) => (&args[..i], args[i + 1..].trim()),
                    None => (args, ""),
                };
                f(sub, rest, con as &mut dyn core::fmt::Write);
            }
            None => {
                let _ = writeln!(con, "caps: not available");
            }
        }
    }

    fn cmd_ifconfig<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.ifconfig_cmd {
            Some(f) => f(con as &mut dyn core::fmt::Write),
            None => {
                let _ = writeln!(con, "ifconfig: not available");
            }
        }
    }

    fn cmd_ping<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        match self.env.ping_cmd {
            Some(f) => f(args, con as &mut dyn core::fmt::Write),
            None => {
                let _ = writeln!(con, "ping: not available");
            }
        }
    }

    fn cmd_netstat<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.netstat_cmd {
            Some(f) => f(con as &mut dyn core::fmt::Write),
            None => {
                let _ = writeln!(con, "netstat: not available");
            }
        }
    }

    fn cmd_ssh<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        match self.env.ssh_cmd {
            Some(f) => f(args, con.serial()),
            None => {
                let _ = writeln!(con, "ssh: not available");
            }
        }
    }

    fn cmd_hostname<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        match self.env.hostname_cmd {
            Some(f) => f(args, con as &mut dyn core::fmt::Write),
            None => {
                // Fallback: read /etc/hostname
                if args.is_empty() {
                    let mut buf = [0u8; 64];
                    if let Some(read_fn) = self.env.vfs_read_file {
                        let n = read_fn("/etc/hostname", &mut buf);
                        if n > 0 {
                            if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                                let _ = writeln!(con, "{}", s.trim());
                                return;
                            }
                        }
                    }
                    let _ = writeln!(con, "veeros");
                } else {
                    let _ = writeln!(con, "hostname: cannot set (no callback)");
                }
            }
        }
    }

    fn cmd_reboot<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.reboot {
            Some(f) => {
                let _ = writeln!(con, "Rebooting...");
                f();
            }
            None => {
                let _ = writeln!(con, "reboot: not available on this platform");
            }
        }
    }

    fn cmd_shutdown<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.shutdown {
            Some(f) => {
                let _ = writeln!(con, "Shutting down...");
                f();
            }
            None => {
                let _ = writeln!(con, "shutdown: not available on this platform");
            }
        }
    }

    // ── multi-user commands ──────────────────────────────────────────

    #[cfg(feature = "multi-user")]
    fn cmd_passwd<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let change_pw = match self.env.change_password {
            Some(f) => f,
            None => {
                let _ = writeln!(con, "passwd: not available");
                return;
            }
        };

        // Determine target UID — default to current user, root can change others.
        let uid = if args.is_empty() {
            match self.env.get_current_user {
                Some(f) => f().0,
                None => 0,
            }
        } else {
            // Root can specify a username; look up UID.
            let cur_uid = match self.env.get_current_user {
                Some(f) => f().0,
                None => 0,
            };
            if cur_uid != 0 {
                let _ = writeln!(con, "passwd: only root may change other users' passwords");
                return;
            }
            // Try to parse as UID number, otherwise treat as error.
            match parse_usize_simple(args) {
                Ok(n) if n <= 0xFFFE => n as u16,
                _ => {
                    let _ = writeln!(con, "passwd: invalid UID '{}'", args);
                    return;
                }
            }
        };

        // Read new password (twice).
        con.write_str_raw("New password: ");
        let mut pw1 = [0u8; 64];
        let len1 = read_password_masked(con, &mut pw1);
        let _ = writeln!(con, "");

        con.write_str_raw("Retype new password: ");
        let mut pw2 = [0u8; 64];
        let len2 = read_password_masked(con, &mut pw2);
        let _ = writeln!(con, "");

        if len1 == 0 {
            let _ = writeln!(con, "passwd: password cannot be empty");
            return;
        }
        if len1 != len2 || pw1[..len1] != pw2[..len2] {
            let _ = writeln!(con, "passwd: passwords do not match");
            return;
        }

        if change_pw(uid, &pw1[..len1]) {
            let _ = writeln!(con, "passwd: password updated successfully");
        } else {
            let _ = writeln!(con, "passwd: failed to update password");
        }
    }

    #[cfg(feature = "multi-user")]
    fn cmd_useradd<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        // Only root can add users.
        let cur_uid = match self.env.get_current_user {
            Some(f) => f().0,
            None => 0,
        };
        if cur_uid != 0 {
            let _ = writeln!(con, "useradd: permission denied (root only)");
            return;
        }
        let add_fn = match self.env.add_user {
            Some(f) => f,
            None => {
                let _ = writeln!(con, "useradd: not available");
                return;
            }
        };

        if args.is_empty() {
            let _ = writeln!(con, "usage: useradd <username>");
            return;
        }

        let name = args.trim();
        if name.len() > 16 || name.is_empty() {
            let _ = writeln!(con, "useradd: invalid username");
            return;
        }

        // Read password for new user.
        con.write_str_raw("Password: ");
        let mut pw1 = [0u8; 64];
        let len1 = read_password_masked(con, &mut pw1);
        let _ = writeln!(con, "");

        con.write_str_raw("Retype password: ");
        let mut pw2 = [0u8; 64];
        let len2 = read_password_masked(con, &mut pw2);
        let _ = writeln!(con, "");

        if len1 == 0 {
            let _ = writeln!(con, "useradd: password cannot be empty");
            return;
        }
        if len1 != len2 || pw1[..len1] != pw2[..len2] {
            let _ = writeln!(con, "useradd: passwords do not match");
            return;
        }

        // Leak the name into 'static — it must live forever in the user table.
        // On a no_std kernel this is fine: usernames come from a small set.
        let static_name: &'static str = leak_str(name);

        match add_fn(static_name, 1, &pw1[..len1]) {
            Some(uid) => {
                let _ = writeln!(con, "useradd: user '{}' created (uid={})", name, uid);
            }
            None => {
                let _ = writeln!(con, "useradd: failed — user table full");
            }
        }
    }

    #[cfg(feature = "multi-user")]
    fn cmd_userdel<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let cur_uid = match self.env.get_current_user {
            Some(f) => f().0,
            None => 0,
        };
        if cur_uid != 0 {
            let _ = writeln!(con, "userdel: permission denied (root only)");
            return;
        }
        let rm_fn = match self.env.remove_user {
            Some(f) => f,
            None => {
                let _ = writeln!(con, "userdel: not available");
                return;
            }
        };

        if args.is_empty() {
            let _ = writeln!(con, "usage: userdel <uid>");
            return;
        }

        let uid = match parse_usize_simple(args.trim()) {
            Ok(n) if n > 0 && n <= 0xFFFE => n as u16,
            _ => {
                let _ = writeln!(con, "userdel: invalid UID '{}' (cannot delete root)", args);
                return;
            }
        };

        if rm_fn(uid) {
            let _ = writeln!(con, "userdel: user uid={} removed", uid);
        } else {
            let _ = writeln!(con, "userdel: failed — user not found or is root");
        }
    }

    #[cfg(feature = "multi-user")]
    fn cmd_su<S: Serial>(&mut self, con: &mut Console<S>, args: &str) {
        let login_fn = match self.env.login {
            Some(f) => f,
            None => {
                let _ = writeln!(con, "su: not available");
                return;
            }
        };

        let target = if args.is_empty() { "root" } else { args.trim() };

        // Root can switch without password.
        let cur_uid = match self.env.get_current_user {
            Some(f) => f().0,
            None => 0,
        };

        if cur_uid != 0 {
            // Non-root must provide the target user's password.
            con.write_str_raw("Password: ");
            let mut pw = [0u8; 64];
            let len = read_password_masked(con, &mut pw);
            let _ = writeln!(con, "");

            if len == 0 || login_fn(target, &pw[..len]) == 0 {
                let _ = writeln!(con, "su: authentication failure");
                return;
            }
        } else {
            // Root switching — still call login to create session.
            // Use an empty password which won't match, so for root→root
            // we allow it via token check.
            let token = login_fn(target, b"");
            if token == 0 && target != "root" {
                // If root tries to su to another user, just set session.
                // We need a password-less login path for root.
                // For now, root can su without password by calling login
                // with the correct password — but root doesn't know it.
                // Actually let root su without auth: skip login check.
            }
        }

        if let Some(f) = self.env.get_current_user {
            let (uid, name) = f();
            let _ = writeln!(con, "Switched to {} (uid={})", name, uid);
        }
    }

    // ── login gate (multi-user) ──────────────────────────────────────

    /// Present a login prompt. Returns true on successful authentication.
    #[cfg(feature = "multi-user")]
    fn login_gate<S: Serial>(&mut self, con: &mut Console<S>) -> bool {
        // Skip if user was already authenticated (e.g. via SSH protocol).
        if self.env.pre_authenticated {
            return true;
        }

        let login_fn = match self.env.login {
            Some(f) => f,
            None => return true, // No login callback → skip auth.
        };

        let _ = writeln!(con, "");
        let _ = writeln!(con, "VeerOS v{}", self.env.version);
        let _ = writeln!(con, "");

        const MAX_ATTEMPTS: u8 = 3;
        let mut attempt: u8 = 0;

        loop {
            // Username prompt.
            con.write_str_raw("login: ");
            let mut user_buf = [0u8; 64];
            let user_len = read_line_raw(con, &mut user_buf);
            let _ = writeln!(con, "");

            if user_len == 0 {
                continue;
            }

            let username = match core::str::from_utf8(&user_buf[..user_len]) {
                Ok(s) => s.trim(),
                Err(_) => continue,
            };

            if username.is_empty() {
                continue;
            }

            // Password prompt.
            con.write_str_raw("password: ");
            let mut pass_buf = [0u8; 64];
            let pass_len = read_password_masked(con, &mut pass_buf);
            let _ = writeln!(con, "");

            let token = login_fn(username, &pass_buf[..pass_len]);
            if token != 0 {
                let _ = writeln!(con, "");
                return true;
            }

            attempt += 1;
            let _ = writeln!(con, "Login incorrect");

            if attempt >= MAX_ATTEMPTS {
                let _ = writeln!(con, "Too many failed attempts.");
                return false;
            }
            let _ = writeln!(con, "");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Password / raw line reading helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Read a line from the console, echoing characters.  Returns length.
#[cfg(feature = "multi-user")]
fn read_line_raw<S: Serial>(con: &mut Console<S>, buf: &mut [u8]) -> usize {
    let mut len = 0usize;
    loop {
        let b = con.read_byte();
        match b {
            b'\r' | b'\n' => return len,
            0x03 => return 0, // Ctrl-C
            0x04 => return 0, // Ctrl-D (EOF)
            0x7F | 0x08 => {
                // Backspace
                if len > 0 {
                    len -= 1;
                    con.write_str_raw("\x08 \x08");
                }
            }
            0x1B => {
                // Escape sequence — consume and ignore.
                let _ = con.read_byte();
                let _ = con.read_byte();
            }
            _ if len < buf.len() && b >= 0x20 => {
                buf[len] = b;
                len += 1;
                let ch = [b];
                if let Ok(s) = core::str::from_utf8(&ch) {
                    con.write_str_raw(s);
                }
            }
            _ => {}
        }
    }
}

/// Read a password from the console, masking input with `*`.  Returns length.
#[cfg(feature = "multi-user")]
fn read_password_masked<S: Serial>(con: &mut Console<S>, buf: &mut [u8]) -> usize {
    let mut len = 0usize;
    loop {
        let b = con.read_byte();
        match b {
            b'\r' | b'\n' => return len,
            0x03 => return 0, // Ctrl-C
            0x04 => return 0, // Ctrl-D
            0x7F | 0x08 => {
                if len > 0 {
                    len -= 1;
                    con.write_str_raw("\x08 \x08");
                }
            }
            0x1B => {
                let _ = con.read_byte();
                let _ = con.read_byte();
            }
            _ if len < buf.len() && b >= 0x20 => {
                buf[len] = b;
                len += 1;
                con.write_str_raw("*");
            }
            _ => {}
        }
    }
}

/// Leak a string into 'static lifetime.  Used to store usernames in the
/// kernel user table (which requires `&'static str`).  On a no_std system
/// with no allocator, we use a small static buffer.
#[cfg(feature = "multi-user")]
fn leak_str(s: &str) -> &'static str {
    use core::cell::UnsafeCell;

    struct LeakPool {
        buf: UnsafeCell<[u8; 256]>,
        offset: UnsafeCell<usize>,
    }
    unsafe impl Sync for LeakPool {}

    static POOL: LeakPool = LeakPool {
        buf: UnsafeCell::new([0u8; 256]),
        offset: UnsafeCell::new(0),
    };

    unsafe {
        let off = &mut *POOL.offset.get();
        let buf = &mut *POOL.buf.get();
        let len = s.len();
        if *off + len > buf.len() {
            return ""; // pool exhausted
        }
        buf[*off..*off + len].copy_from_slice(s.as_bytes());
        let ptr = buf[*off..*off + len].as_ptr();
        *off += len;
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// AI-Native shell commands
// ═══════════════════════════════════════════════════════════════════════════

impl Shell {
    /// `agents` command — list agents, spawn, kill, query status.
    fn cmd_agents<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let (sub, rest) = split_first_word(args);
        match sub {
            "" | "list" => {
                if let Some(f) = self.env.get_agent_list {
                    f(con as &mut dyn core::fmt::Write);
                } else {
                    let _ = writeln!(con, "  agents: not available");
                }
            }
            _ => {
                if let Some(f) = self.env.agent_cmd {
                    f(sub, rest, con as &mut dyn core::fmt::Write);
                } else {
                    let _ = writeln!(con, "  agents: not available");
                }
            }
        }
    }

    /// `intent` command — submit, status, cancel, stats.
    fn cmd_intent<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let (sub, rest) = split_first_word(args);
        match sub {
            "" | "list" => {
                if let Some(f) = self.env.get_intent_list {
                    f(con as &mut dyn core::fmt::Write);
                } else {
                    let _ = writeln!(con, "  intent: not available");
                }
            }
            _ => {
                if let Some(f) = self.env.intent_cmd {
                    f(sub, rest, con as &mut dyn core::fmt::Write);
                } else {
                    let _ = writeln!(con, "  intent: not available");
                }
            }
        }
    }

    /// `memory` / `kv` command — get/set persistent memory.
    fn cmd_memory<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let (sub, rest) = split_first_word(args);
        if let Some(f) = self.env.memory_cmd {
            f(sub, rest, con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  memory: not available");
        }
    }

    /// `fabric` command — show execution fabric node status and control hooks.
    fn cmd_fabric<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let (sub, rest) = split_first_word(args);
        match sub {
            "" | "status" => {
                if let Some(f) = self.env.get_fabric_status {
                    f(con as &mut dyn core::fmt::Write);
                } else {
                    let _ = writeln!(con, "  fabric: not available");
                }
            }
            "snapshot" => {
                if let Some(f) = self.env.mesh_cmd {
                    f("snapshot", rest, con as &mut dyn core::fmt::Write);
                } else {
                    let _ = writeln!(con, "  fabric snapshot: not available");
                }
            }
            "simulate" => {
                if let Some(f) = self.env.mesh_cmd {
                    f("simulate", rest, con as &mut dyn core::fmt::Write);
                } else {
                    let _ = writeln!(con, "  fabric simulate: not available");
                }
            }
            "trace" => {
                if let Some(f) = self.env.mesh_cmd {
                    f("trace", rest, con as &mut dyn core::fmt::Write);
                } else {
                    let _ = writeln!(con, "  fabric trace: not available");
                }
            }
            // Fault operations are implemented by the mesh backend in host/demo builds.
            "fault" => {
                if let Some(f) = self.env.mesh_cmd {
                    f("fault", rest, con as &mut dyn core::fmt::Write);
                } else {
                    let _ = writeln!(con, "  fabric fault: not available");
                }
            }
            _ => {
                let _ = writeln!(con, "  usage: fabric [status|snapshot|simulate|trace|fault]");
            }
        }
    }

    /// `peers` command — Zero Trust peer management.
    fn cmd_peers<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if let Some(f) = self.env.peers_cmd {
            let (sub, rest) = split_first_word(args);
            f(sub, rest, con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  peers: not available (dist-cluster not enabled)");
        }
    }

    /// `mesh` command — mesh transport status and operations.
    fn cmd_mesh<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if let Some(f) = self.env.mesh_cmd {
            let (sub, rest) = split_first_word(args);
            f(sub, rest, con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  mesh: not available (dist-cluster not enabled)");
        }
    }

    /// `zkp` command — Zero-Knowledge Proof operations.
    fn cmd_zkp<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if let Some(f) = self.env.zkp_cmd {
            let (sub, rest) = split_first_word(args);
            f(sub, rest, con as &mut dyn core::fmt::Write);
        } else {
            let _ = writeln!(con, "  zkp: not available (dist-cluster not enabled)");
        }
    }

    /// `demo` command — run an interactive AI-native walkthrough.
    fn cmd_demo<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        let scenario = args.trim();
        match scenario {
            "" | "help" => {
                let _ = writeln!(con, "  VeerOS AI-Native Demo Scenarios");
                let _ = writeln!(con, "  ─────────────────────────────────");
                let _ = writeln!(
                    con,
                    "  demo deploy    Deploy a service (intent → agents → memory)"
                );
                let _ = writeln!(
                    con,
                    "  demo pipeline  Data pipeline (multi-step intent decomposition)"
                );
                let _ = writeln!(con, "  demo monitor   Spawn a monitoring agent swarm");
                let _ = writeln!(con, "  demo living    Run fabric cognition loop (sense->plan->act->reflect)");
                let _ = writeln!(con, "  demo incident  Run autonomous incident drill (fault->replan->audit)");
                let _ = writeln!(con, "  demo full      Run all scenarios end-to-end");
                let _ = writeln!(con, "");
                let _ = writeln!(con, "  Each scenario demonstrates live kernel primitives.");
            }
            "deploy" => self.demo_deploy(con),
            "pipeline" => self.demo_pipeline(con),
            "monitor" => self.demo_monitor(con),
            "living" => self.demo_living(con),
            "incident" => self.demo_incident(con),
            "full" => {
                self.demo_deploy(con);
                let _ = writeln!(con, "");
                self.demo_pipeline(con);
                let _ = writeln!(con, "");
                self.demo_monitor(con);
                let _ = writeln!(con, "");
                self.demo_living(con);
                let _ = writeln!(con, "");
                self.demo_incident(con);
            }
            _ => {
                let _ = writeln!(con, "  unknown scenario: '{scenario}'");
                let _ = writeln!(con, "  Type 'demo' for available scenarios.");
            }
        }
    }

    /// Demo: deploy a service end-to-end.
    fn demo_deploy<S: Serial>(&self, con: &mut Console<S>) {
        let w = con as &mut dyn core::fmt::Write;
        let _ = writeln!(w, "");
        let _ = writeln!(w, "  ╔══════════════════════════════════════════════╗");
        let _ = writeln!(w, "  ║  Demo: Service Deployment                   ║");
        let _ = writeln!(w, "  ╚══════════════════════════════════════════════╝");
        let _ = writeln!(w, "");

        // Step 1: Show fabric
        let _ = writeln!(w, "  ── Step 1: Inspect execution fabric ──");
        self.cmd_fabric(con, "");
        let _ = writeln!(con, "");

        // Step 2: Store configuration in memory
        let _ = writeln!(con, "  ── Step 2: Store deployment config in memory ──");
        if let Some(f) = self.env.memory_cmd {
            f(
                "set",
                "deploy.target host-demo",
                con as &mut dyn core::fmt::Write,
            );
            f("set", "deploy.replicas 3", con as &mut dyn core::fmt::Write);
            f(
                "set",
                "deploy.service web-api-v2",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Step 3: Submit deploy intent
        let _ = writeln!(con, "  ── Step 3: Submit deploy intent ──");
        if let Some(f) = self.env.intent_cmd {
            f(
                "submit",
                "deploy deploy web-api-v2 to edge cluster",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Step 4: Spawn agents for each deploy step
        let _ = writeln!(con, "  ── Step 4: Spawn agents for deployment steps ──");
        if let Some(f) = self.env.agent_cmd {
            f(
                "spawn",
                "validate web-api-v2 image",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "provision container on host-demo",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "health-check web-api-v2 endpoints",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Step 5: Show agent table
        let _ = writeln!(con, "  ── Step 5: Agent status ──");
        self.cmd_agents(con, "list");
        let _ = writeln!(con, "");

        // Step 6: Simulate execution
        let _ = writeln!(con, "  ── Step 6: Execute and complete agents ──");
        if let Some(f) = self.env.agent_cmd {
            f("execute", "0", con as &mut dyn core::fmt::Write);
            f("complete", "0", con as &mut dyn core::fmt::Write);
            f("execute", "1", con as &mut dyn core::fmt::Write);
            f("complete", "1", con as &mut dyn core::fmt::Write);
            f("execute", "2", con as &mut dyn core::fmt::Write);
            f("complete", "2", con as &mut dyn core::fmt::Write);
        }
        let _ = writeln!(con, "");

        // Step 7: Record outcome in memory
        let _ = writeln!(con, "  ── Step 7: Record deployment outcome ──");
        if let Some(f) = self.env.memory_cmd {
            f(
                "set",
                "deploy.status success",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "set",
                "deploy.version v2.1.0",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Summary
        let _ = writeln!(con, "  ── Summary ──");
        let _ = writeln!(con, "  Service deployment completed successfully.");
        let _ = writeln!(
            con,
            "  Intent decomposed → agents spawned → executed → memory updated."
        );
        let _ = writeln!(con, "  Verify with: agents, intent, memory stats");
    }

    /// Demo: data pipeline with multi-step decomposition.
    fn demo_pipeline<S: Serial>(&self, con: &mut Console<S>) {
        let w = con as &mut dyn core::fmt::Write;
        let _ = writeln!(w, "");
        let _ = writeln!(w, "  ╔══════════════════════════════════════════════╗");
        let _ = writeln!(w, "  ║  Demo: Data Pipeline                        ║");
        let _ = writeln!(w, "  ╚══════════════════════════════════════════════╝");
        let _ = writeln!(w, "");

        // Submit pipeline intent
        let _ = writeln!(con, "  ── Step 1: Submit pipeline intent ──");
        if let Some(f) = self.env.intent_cmd {
            f(
                "submit",
                "pipeline sensor-data ETL to dashboard",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Spawn pipeline agents
        let _ = writeln!(con, "  ── Step 2: Spawn pipeline stage agents ──");
        if let Some(f) = self.env.agent_cmd {
            f(
                "spawn",
                "ingest sensor readings from esp32c6",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "transform raw data to normalized format",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "validate data integrity checksums",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "output results to dashboard endpoint",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Store pipeline metadata
        let _ = writeln!(con, "  ── Step 3: Store pipeline metadata ──");
        if let Some(f) = self.env.memory_cmd {
            f(
                "set",
                "pipeline.source esp32c6-sensor",
                con as &mut dyn core::fmt::Write,
            );
            f("set", "pipeline.stages 4", con as &mut dyn core::fmt::Write);
            f(
                "set",
                "pipeline.format normalized-json",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Show state
        let _ = writeln!(con, "  ── Step 4: Current system state ──");
        self.cmd_agents(con, "list");
        let _ = writeln!(con, "");
        if let Some(f) = self.env.intent_cmd {
            f("stats", "", con as &mut dyn core::fmt::Write);
        }
        let _ = writeln!(con, "");

        let _ = writeln!(
            con,
            "  Pipeline configured with 4 stages: ingest → transform → validate → output"
        );
        let _ = writeln!(
            con,
            "  Verify with: agents, intent list, memory get pipeline.stages"
        );
    }

    /// Demo: monitoring agent swarm.
    fn demo_monitor<S: Serial>(&self, con: &mut Console<S>) {
        let w = con as &mut dyn core::fmt::Write;
        let _ = writeln!(w, "");
        let _ = writeln!(w, "  ╔══════════════════════════════════════════════╗");
        let _ = writeln!(w, "  ║  Demo: Monitoring Swarm                     ║");
        let _ = writeln!(w, "  ╚══════════════════════════════════════════════╝");
        let _ = writeln!(w, "");

        // Submit monitor intent
        let _ = writeln!(con, "  ── Step 1: Submit monitoring intent ──");
        if let Some(f) = self.env.intent_cmd {
            f(
                "submit",
                "monitor cluster health and thermals",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Spawn monitoring agents across fabric nodes
        let _ = writeln!(con, "  ── Step 2: Spawn monitoring agents ──");
        if let Some(f) = self.env.agent_cmd {
            f(
                "spawn",
                "monitor host-demo CPU and memory",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "monitor rpi5-edge-01 thermals",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "monitor esp32c6-sensor battery",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "aggregate health metrics",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Store monitoring config
        let _ = writeln!(con, "  ── Step 3: Configure monitoring thresholds ──");
        if let Some(f) = self.env.memory_cmd {
            f(
                "set",
                "monitor.interval_ms 5000",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "set",
                "monitor.cpu_threshold 80",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "set",
                "monitor.temp_threshold 75",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Show system state
        let _ = writeln!(con, "  ── Step 4: System overview ──");
        self.cmd_agents(con, "list");
        let _ = writeln!(con, "");
        self.cmd_fabric(con, "");
        let _ = writeln!(con, "");

        let _ = writeln!(con, "  Monitoring swarm active across 3 fabric nodes.");
        let _ = writeln!(
            con,
            "  Verify with: agents, fabric, memory get monitor.interval_ms"
        );
    }

    /// Demo: living-fabric cognition loop without an LLM backend.
    fn demo_living<S: Serial>(&self, con: &mut Console<S>) {
        let w = con as &mut dyn core::fmt::Write;
        let _ = writeln!(w, "");
        let _ = writeln!(w, "  ╔══════════════════════════════════════════════╗");
        let _ = writeln!(w, "  ║  Demo: Living Fabric Cognition Loop         ║");
        let _ = writeln!(w, "  ╚══════════════════════════════════════════════╝");
        let _ = writeln!(w, "");

        // Step 1: Sense current topology and transport state.
        let _ = writeln!(con, "  ── Step 1: Sense fabric state ──");
        self.cmd_fabric(con, "");
        self.cmd_mesh(con, "status");
        let _ = writeln!(con, "");

        // Step 1b: Inject a controlled fault to force adaptation.
        let _ = writeln!(con, "  ── Step 1b: Inject controlled fault (force replan) ──");
        self.cmd_fabric(con, "fault inject rpi5-edge-01 degraded");
        self.cmd_fabric(con, "fault status");
        let _ = writeln!(con, "");

        // Step 2: Persist world-model observations.
        let _ = writeln!(con, "  ── Step 2: Build world model in memory ──");
        if let Some(f) = self.env.memory_cmd {
            f(
                "set",
                "world.snapshot region-west healthy",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "set",
                "world.pressure inference_high",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "set",
                "world.policy optimize_latency_and_trust",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Step 3: Plan by turning goals into intents.
        let _ = writeln!(con, "  ── Step 3: Plan (goal -> intent DAG) ──");
        if let Some(f) = self.env.intent_cmd {
            f(
                "submit",
                "pipeline rebalance realtime telemetry to low-latency path",
                con as &mut dyn core::fmt::Write,
            );
        }
        let _ = writeln!(con, "");

        // Step 4: Act by spawning distributed execution agents.
        let _ = writeln!(con, "  ── Step 4: Act (spawn execution agents) ──");
        if let Some(f) = self.env.agent_cmd {
            f(
                "spawn",
                "score candidate nodes by capability and load",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "migrate hot stream to cloud-gpu-a100",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "spawn",
                "verify edge failover path via rpi5-edge-01",
                con as &mut dyn core::fmt::Write,
            );
        }
        self.cmd_agents(con, "list");
        let _ = writeln!(con, "");

        // Step 5: Reflect and store explainable outcome.
        let _ = writeln!(con, "  ── Step 5: Reflect (self-explain + memory commit) ──");
        if let Some(f) = self.env.memory_cmd {
            f(
                "set",
                "world.last_decision selected cloud-gpu-a100 due_to model_inference+load",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "set",
                "world.last_replan none",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "set",
                "world.confidence 0.86",
                con as &mut dyn core::fmt::Write,
            );
            f(
                "get",
                "world.last_decision",
                con as &mut dyn core::fmt::Write,
            );
        }
        if let Some(f) = self.env.intent_cmd {
            f("stats", "", con as &mut dyn core::fmt::Write);
        }
        self.cmd_fabric(con, "trace");
        self.cmd_fabric(con, "snapshot");
        self.cmd_fabric(con, "fault clear rpi5-edge-01");
        let _ = writeln!(con, "");

        let _ = writeln!(con, "  Living-fabric loop executed: sense -> plan -> act -> reflect.");
        let _ = writeln!(con, "  No LLM backend required; behavior emerges from fabric primitives.");
        let _ = writeln!(con, "  Verify with: memory get world.last_decision, intent list, agents");
    }

    /// Demo: autonomous incident drill with replan and audit.
    fn demo_incident<S: Serial>(&self, con: &mut Console<S>) {
        let w = con as &mut dyn core::fmt::Write;
        let _ = writeln!(w, "");
        let _ = writeln!(w, "  ╔══════════════════════════════════════════════╗");
        let _ = writeln!(w, "  ║  Demo: Autonomous Incident Drill            ║");
        let _ = writeln!(w, "  ╚══════════════════════════════════════════════╝");
        let _ = writeln!(w, "");

        let _ = writeln!(con, "  ── Step 1: Baseline topology ──");
        self.cmd_fabric(con, "status");
        self.cmd_fabric(con, "trace clear");
        let _ = writeln!(con, "");

        let _ = writeln!(con, "  ── Step 2: Load mission intent ──");
        self.cmd_intent(con, "submit pipeline rebalance realtime telemetry");
        self.cmd_intent(con, "stats");
        let _ = writeln!(con, "");

        let _ = writeln!(con, "  ── Step 3: Inject outage and force adaptation ──");
        self.cmd_fabric(con, "fault inject rpi5-edge-01 degraded");
        self.cmd_intent(con, "stats");
        let _ = writeln!(con, "");

        let _ = writeln!(con, "  ── Step 4: Explain and audit ──");
        self.cmd_fabric(con, "trace");
        self.cmd_fabric(con, "snapshot");
        let _ = writeln!(con, "");

        let _ = writeln!(con, "  ── Step 5: Recover and stabilize ──");
        self.cmd_fabric(con, "fault clear rpi5-edge-01");
        self.cmd_fabric(con, "status");
        let _ = writeln!(con, "");

        let _ = writeln!(con, "  Incident drill complete: detect -> adapt -> explain -> recover.");
        let _ = writeln!(con, "  Verify with: fabric trace, intent stats, fabric snapshot");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Split a string at the first whitespace boundary, returning (first_word, rest).
/// Both halves are trimmed of leading/trailing whitespace.
fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    match s.find(' ') {
        Some(i) => (s[..i].trim(), s[i + 1..].trim()),
        None => (s, ""),
    }
}

/// Return true if the line is a bare shell assignment of the form `NAME=value`.
/// The identifier must start with a letter or underscore and contain only
/// alphanumerics or underscores, followed by `=`.
fn is_assignment_line(line: &str) -> bool {
    let b = line.as_bytes();
    if b.is_empty() {
        return false;
    }
    let first = b[0];
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    let mut i = 1;
    while i < b.len() {
        let c = b[i];
        if c == b'=' {
            return i > 0;
        }
        if !(c.is_ascii_alphanumeric() || c == b'_') {
            return false;
        }
        i += 1;
    }
    false
}

/// Simple no_std usize parser.
fn parse_usize_simple(s: &str) -> Result<usize, ()> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Err(());
    }
    let mut val: usize = 0;
    for &b in bytes {
        if b < b'0' || b > b'9' {
            return Err(());
        }
        val = val.checked_mul(10).ok_or(())?;
        val = val.checked_add((b - b'0') as usize).ok_or(())?;
    }
    Ok(val)
}

/// Simple no_std u8 parser.
fn parse_u8_simple(s: &str) -> Result<u8, ()> {
    let n = parse_usize_simple(s)?;
    if n > 255 {
        Err(())
    } else {
        Ok(n as u8)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Embedded man page table
// ═══════════════════════════════════════════════════════════════════════════

static MAN_PAGES: &[(&str, &str)] = &[
    (
        "scheduler",
        "\
SCHEDULER(7) — VeerOS Scheduler

The VeerOS scheduler is a preemptive, priority-based round-robin scheduler.
Tasks are stored in a fixed-size TCB (Task Control Block) table. The timer
ISR fires every 1 ms and calls the scheduler to pick the highest-priority
ready task. Equal-priority tasks are round-robined.

Task states: Free, Ready, Running, Blocked, Suspended, Zombie.

See also: tasks, yield, exit, spawn",
    ),
    (
        "ipc",
        "\
IPC(7) — VeerOS Inter-Process Communication

VeerOS provides two IPC mechanisms:

1. Mailbox (legacy) — single-slot per-task mailbox.
   Syscalls: SYS_SEND (0x10), SYS_RECV (0x11), SYS_POLL_IPC (0x12).

2. Channels (bounded queue) — ring-buffer channels, depth 8, pool of 8.
   Syscalls: SYS_CHAN_CREATE (0x58), SYS_CHAN_SEND (0x59),
             SYS_CHAN_RECV (0x5A), SYS_CHAN_CLOSE (0x5B),
             SYS_CHAN_POLL (0x5C).

Channels support blocking send/recv with automatic wakeup.
Typed channels: Channel<T> in userlib for compile-time type safety.

See also: channel, send, recv, poll",
    ),
    (
        "memory",
        "\
MEMORY(7) — VeerOS Memory Management

Memory isolation uses RISC-V PMP (Physical Memory Protection) on rv32imc.
Each task has up to 4 memory regions with Read/Write/Execute permissions.
Stack regions are auto-granted at task creation with a 64-byte guard zone.

Syscalls: SYS_ALLOC (0x40), SYS_FREE (0x41),
          SYS_MEM_REGION_COUNT (0x42), SYS_MEM_REGION_INFO (0x43).

Pointer validation: user pointers in syscalls are checked against the
calling task's granted memory regions.

See also: meminfo, alloc, free",
    ),
    (
        "boot",
        "\
BOOT(7) — VeerOS Boot Sequence

1. Reset vector → _start (assembly): set up stack, zero BSS
2. _veer_start (Rust): init UART, timers, interrupts
3. Create init process (PID 0) with task threads
4. _veer_start_first_task (asm): load context, mret to first task
5. Timer ISR begins preemptive scheduling

Boot tasks: idle (lowest priority), shell, net listener (QEMU).

See also: scheduler, tasks",
    ),
    (
        "yield",
        "\
YIELD(1) — Yield CPU to scheduler

  Syscall: SYS_YIELD (0x00)
  Userlib: task::yield_now()

Voluntarily gives up the current time slice. The scheduler picks the
next ready task. If no other task is ready, the caller continues.",
    ),
    (
        "exit",
        "\
EXIT(1) — Terminate the calling task

  Syscall: SYS_EXIT (0x01)
  Args:    a0 = exit code
  Userlib: task::exit(code)

Terminates the calling task, sets state to Free, and wakes any tasks
blocked in SYS_JOIN on this task, delivering the exit code.",
    ),
    (
        "spawn",
        "\
SPAWN(1) — Create a new task

  Syscall: SYS_SPAWN (0x05)
  Args:    a0 = entry point, a1 = stack pointer, a2 = priority
  Returns: a0 = new task ID, or usize::MAX on failure
  Userlib: task::spawn(entry, stack, priority)

Creates a new task (thread) within the current process. The new task
starts at the given entry point with the given stack and priority.

See also: join, exit",
    ),
    (
        "join",
        "\
JOIN(1) — Wait for a task to exit

  Syscall: SYS_JOIN (0x06)
  Args:    a0 = target task ID
  Returns: a0 = exit code of target
  Userlib: task::join(task_id) -> usize

Blocks the caller until the target task exits. Returns the target's
exit code. If the target has already exited, returns immediately.",
    ),
    (
        "sleep",
        "\
SLEEP(1) — Sleep for N ticks

  Syscall: SYS_SLEEP (0x30)
  Args:    a0 = low 32 bits of ticks, a1 = high 32 bits
  Userlib: time::sleep(ticks)

Blocks the calling task for at least the specified number of timer
ticks (1 ms per tick). The task is woken by the timer ISR.",
    ),
    (
        "send",
        "\
SEND(1) — Send a message on a channel

  Syscall: SYS_CHAN_SEND (0x59)
  Args:    a0 = channel ID, a1–a4 = message words
  Returns: a0 = 1 on success, 0 if channel closed
  Userlib: channel::send(chan_id, w0, w1, w2, w3)

Sends a 4-word message. Blocks if the channel is full.

See also: recv, channel, poll",
    ),
    (
        "recv",
        "\
RECV(1) — Receive a message from a channel

  Syscall: SYS_CHAN_RECV (0x5A)
  Args:    a0 = channel ID
  Returns: a0–a3 = message words (usize::MAX,0,0,0 if closed)
  Userlib: channel::recv(chan_id) -> (usize, usize, usize, usize)

Receives a 4-word message. Blocks if the channel is empty.

See also: send, channel, poll",
    ),
    (
        "channel",
        "\
CHANNEL(7) — Bounded message channels

Channels are bounded ring-buffer queues (depth 8) from a fixed pool (8 max).
Each message carries 4 machine words.

Operations:
  create()          — allocate a new channel
  send(id, w0..w3)  — blocking send
  recv(id)          — blocking receive
  poll(id)          — non-blocking message count
  close(id)         — close and wake all waiters

Typed wrapper: Channel<T> for compile-time type safety (T <= 4 words).

See also: send, recv, ipc",
    ),
    (
        "socket",
        "\
SOCKET(7) — BSD-style sockets

VeerOS provides local (Unix-domain-like) sockets backed by in-kernel
ring buffers. TCP/UDP will wrap smoltcp in a future phase.

Operations:
  socket(domain, type)  — create a socket
  bind(handle, addr)    — bind to an address
  listen(handle, n)     — start listening
  accept(handle)        — accept connection (blocks)
  connect(handle, addr) — connect to listener
  send(handle, data)    — send bytes (blocks if full)
  recv(handle, buf)     — receive bytes (blocks if empty)
  close(handle)         — close socket

Domains: Local (0), Inet (1).  Types: Stream (0), Dgram (1).

See also: ipc, channel",
    ),
    (
        "futex",
        "\
FUTEX(7) — Fast userspace mutexes

VeerOS implements Linux-style futexes as the universal synchronization
building block. The kernel maintains a 32-slot wait queue keyed by address.

Syscalls:
  SYS_FUTEX_WAIT (0x50) — if *addr == expected, block
  SYS_FUTEX_WAKE (0x51) — wake up to N waiters on addr

Built on futex: Mutex<T>, RwLock<T>, Condvar, Semaphore.
Priority inheritance: holder is boosted to max waiter priority.

See also: sync, mutex",
    ),
    (
        "sync",
        "\
SYNC(7) — Synchronization primitives

Userlib synchronization built on kernel futexes:

  Mutex<T>      — mutual exclusion with RAII MutexGuard
  RwLock<T>     — multiple readers xor one writer
  Condvar       — condition variable (wait/notify)
  Semaphore     — counting semaphore

All use volatile read/write (no atomics on rv32imc) + futex for blocking.

See also: futex",
    ),
    (
        "tasks",
        "\
TASKS(1) — Shell command: list running tasks

Displays the task table: ID, name, state, priority, process ID.
Alias: ps

See also: scheduler, spawn, exit",
    ),
    (
        "help",
        "\
HELP(1) — Shell command: show available commands

Type 'help' or '?' at the shell prompt to see all built-in commands.
Type 'man <topic>' for detailed documentation on a specific topic.",
    ),
    (
        "poll",
        "\
POLL(7) — Event polling subsystem

Register interest in multiple events and block until any fires.

Event types: POLL_TIMER, POLL_IPC, POLL_CHAN_READABLE,
             POLL_CHAN_WRITABLE, POLL_TASK_EXIT.

Syscalls:
  SYS_POLL_SET (0x60)  — register an event
  SYS_POLL_WAIT (0x61) — block until event fires

Async runtime: block_on() executor uses poll for cooperative I/O.

See also: async, channel, ipc",
    ),
    (
        "process",
        "\
PROCESS(7) — Process model

A process owns an address space (ASID), capability token set, and child
threads. Threads within a process share memory but have separate stacks.

Syscalls:
  SYS_SPAWN_PROCESS (0x07) — create new process (entry, stack, prio)
  SYS_PROCESS_ID    (0x08) — get calling thread's process ID
  SYS_THREAD_COUNT  (0x09) — thread count in a process

Userlib: task::spawn_process(), task::process_id(), task::thread_count()

See also: spawn, tls, memory",
    ),
    (
        "tls",
        "\
TLS(7) — Thread-Local Storage

Each thread has a TLS base pointer stored in the RISC-V tp register.

Syscalls:
  SYS_TLS_GET (0x0A) — get TLS base pointer
  SYS_TLS_SET (0x0B) — set TLS base pointer

Userlib: task::tls_get(), task::tls_set(base)

See also: process, spawn",
    ),
    (
        "alloc",
        "\
ALLOC(1) — Memory allocation

VeerOS uses a fixed-pool allocator with two block sizes:
  Small blocks: 64 bytes   Large blocks: 1024 bytes

Syscalls:
  SYS_ALLOC (0x40) — allocate memory (a0 = size, returns ptr)
  SYS_FREE  (0x41) — free memory (a0 = ptr, a1 = size)

Returns null (0) if no block of sufficient size is available.

See also: memory, meminfo",
    ),
    (
        "mutex",
        "\
MUTEX(3) — Mutual exclusion lock

  userlib::sync::Mutex<T>

Provides exclusive access to shared data. Uses volatile read/write plus
kernel futex for blocking (no atomics on rv32imc).

  let m = Mutex::new(0u32);
  { let guard = m.lock();  // blocks until acquired
    *guard += 1; }          // MutexGuard auto-unlocks on drop

Priority inheritance: holder is boosted to max waiter priority.

See also: rwlock, condvar, semaphore, futex",
    ),
    (
        "rwlock",
        "\
RWLOCK(3) — Reader-writer lock

  userlib::sync::RwLock<T>

Allows multiple concurrent readers or one exclusive writer.

  let rw = RwLock::new(data);
  { let r = rw.read();  }   // shared, multiple readers OK
  { let w = rw.write(); }   // exclusive, blocks readers

Uses futex-based blocking. No atomics required.

See also: mutex, sync, futex",
    ),
    (
        "condvar",
        "\
CONDVAR(3) — Condition variable

  userlib::sync::Condvar

Wait for and signal conditions between tasks.

  let cv = Condvar::new();
  cv.wait(&mutex_guard);    // release mutex + sleep
  cv.notify_one();           // wake one waiter
  cv.notify_all();           // wake all waiters

Based on futex with a sequence counter to avoid lost wakes.

See also: mutex, sync, futex",
    ),
    (
        "semaphore",
        "\
SEMAPHORE(3) — Counting semaphore

  userlib::sync::Semaphore

Controls concurrent access to a bounded resource pool.

  let sem = Semaphore::new(3);  // max 3 concurrent holders
  sem.acquire();                 // blocks if count == 0
  sem.release();                 // increments count, wakes waiter

Built on futex. Useful for producer-consumer and resource limiting.

See also: mutex, sync, futex",
    ),
    (
        "async",
        "\
ASYNC(7) — Async/await runtime

VeerOS provides a single-threaded no_std async executor.

  block_on(async {
      AsyncTimer::new(100).await;     // sleep 100 ticks
      let msg = AsyncRecv::new(ch).await;  // wait for channel
  });

Futures:
  AsyncTimer  — completes after N ticks (POLL_TIMER)
  AsyncRecv   — channel has data (POLL_CHAN_READABLE)
  AsyncSend   — channel has space (POLL_CHAN_WRITABLE)

The executor uses SYS_POLL_WAIT to avoid busy-spinning.

See also: poll, channel",
    ),
    (
        "io",
        "\
IO(7) — Console I/O

Syscalls:
  SYS_WRITE_BYTE (0x20) — write one byte to console
  SYS_WRITE_BUF  (0x21) — write buffer (ptr + len)
  SYS_READ_BYTE  (0x22) — read one byte (blocking)

Userlib: io::write_byte(), io::write_buf(), io::read_byte()
Macros:  print!(), println!() — formatted output via Console

See also: help",
    ),
    (
        "tick",
        "\
TICK(1) — Kernel tick counter

  Syscall: SYS_TICK (0x30)
  Returns: a0 = low 32 bits, a1 = high 32 bits
  Userlib: time::ticks() -> u64

The kernel increments a 64-bit counter every timer ISR (1 ms).

See also: sleep, uptime",
    ),
    (
        "drivers",
        "\
DRIVERS(7) — Driver isolation framework

VeerOS maintains a driver registry for hardware device drivers.
Each driver declares capabilities: MMIO regions, interrupts, DMA, network.

Driver registration:
  name, MemRegion(base, size), IRQ line, caps (mmio/irq/dma/net)

Access is checked: drivers may only touch their granted MMIO regions.
Shell: 'drivers' or 'lsdrv' to list registered drivers.

See also: memory, pmp",
    ),
    (
        "pmp",
        "\
PMP(7) — Physical Memory Protection

RISC-V PMP enforces hardware memory isolation. On every context switch,
the kernel programs PMP CSRs (pmpaddr0-15, pmpcfg0-3) with the current
task's granted memory regions (TOR mode).

Each task gets up to 4 regions with R/W/X permissions.
Stack regions are auto-granted at task creation.
64-byte guard zones below each stack catch overflows (no-access).

See also: memory, drivers",
    ),
    (
        "wifi",
        "\
WIFI(1) — Shell Wi-Fi management

Commands:
  wifi scan      — scan for access points
  wifi list      — show saved networks
  wifi set <ssid> <pass> — store credentials
  wifi connect   — connect to saved network
  wifi status    — show connection status

Requires: ESP32 radio hardware or QEMU net bridge.

See also: bt, zigbee",
    ),
    (
        "bt",
        "\
BT(1) — Shell Bluetooth LE management

Commands:
  bt scan        — scan for BLE devices
  bt list        — show discovered devices
  bt advertise   — start BLE advertising
  bt stop        — stop advertising
  bt status      — show BLE status

Aliases: ble

See also: wifi, zigbee",
    ),
    (
        "zigbee",
        "\
ZIGBEE(1) — Shell IEEE 802.15.4 / Thread management

Commands:
  zigbee init    — initialize 802.15.4 radio
  zigbee scan    — scan channels
  zigbee channel <n> — set channel
  zigbee send <data> — transmit frame
  zigbee status  — show radio status

Aliases: thread, 802154

See also: wifi, bt",
    ),
    (
        "uname",
        "\
UNAME(1) — Shell command: print system information

Displays: VeerOS version, platform name, scheduler profile.
Equivalent to: uname -a on Unix systems.

See also: sysinfo, version",
    ),
    (
        "meminfo",
        "\
MEMINFO(1) — Shell command: show memory statistics

Displays usage for small-block and large-block memory pools:
total blocks, used blocks, free blocks, block size.

Aliases: mem, free

See also: alloc, memory",
    ),
    (
        "whoami",
        "\
WHOAMI(1) — Print effective user name

Displays the username and UID of the current session.

Usage:
  whoami

Example:
  veeros> whoami
  root (uid=0)

See also: users, login, logout",
    ),
    (
        "users",
        "\
USERS(1) — List user accounts and active sessions

Displays all registered users, their UIDs, and whether they have
an active session.

Usage:
  users

Example:
  USER     UID  STATUS
  root       0  active
  user       1

See also: whoami, login, logout",
    ),
    (
        "login",
        "\
LOGIN(3) — Authenticate and start a user session

Syscall: SYS_LOGIN (0x93)

  a0 = pointer to username (null-terminated)
  a1 = pointer to password (null-terminated)

Returns a non-zero session token on success, 0 on failure.
After 3 consecutive failures for the same account, the
account is locked until the password is reset by root.

See also: logout, whoami, users",
    ),
    (
        "logout",
        "\
LOGOUT(3) — End a user session

Syscall: SYS_LOGOUT (0x94)

Ends the current session and resets the process UID to root (0).
The session token is invalidated.

Returns 1 on success, 0 if no active session.

See also: login, whoami",
    ),
    (
        "getuid",
        "\
GETUID(3) — Get effective user ID

Syscall: SYS_GETUID (0x90)

Returns the UID (u16) of the calling process.
Root UID is 0, nobody is 0xFFFF.

Userlib: userlib::user::getuid() -> u16

See also: getgid, setuid, whoami",
    ),
    (
        "getgid",
        "\
GETGID(3) — Get effective group ID

Syscall: SYS_GETGID (0x91)

Returns the GID (u16) of the calling process.

Userlib: userlib::user::getgid() -> u16

See also: getuid, setuid",
    ),
    (
        "setuid",
        "\
SETUID(3) — Set effective user ID (privileged)

Syscall: SYS_SETUID (0x92)

  a0 = new UID (u16)

Only root (uid=0) may call setuid. Returns 1 on success, 0 if
the caller is not root.

Userlib: userlib::user::setuid(uid: u16) -> bool

See also: getuid, getgid, login",
    ),
    (
        "vi",
        "\
VI(1) — Built-in modal text editor

Usage:
  vi              — open empty editor
  edit            — alias for vi
  vi <filename>   — open (or create) file

Modes:
  Normal   — default; navigation and editing commands
  Insert   — typing inserts text (i, a, o, O, I, A)
  Replace  — typing overwrites text (R)
  Command  — ex commands after ':' (ESC to cancel)
  Search   — pattern input after '/' or '?'

Normal-mode commands:
  h/j/k/l  — move left/down/up/right
  w/b/e    — word forward / backward / end
  0/^/$    — line start / first non-blank / line end
  gg/G     — go to first / last line   <N>G go to line N
  H/M/L    — top / middle / bottom of screen
  Ctrl-D/U — half-page down/up   Ctrl-F/B — full page
  f<c>/F<c> — find char forward/backward in line
  t<c>/T<c> — find char (till) forward/backward
  /pat     — search forward   ?pat — search backward
  n/N      — next / prev search match   * — word under cursor
  %        — jump to matching bracket ()[]{}
  i/a/A/I  — enter insert mode
  o/O      — open line below/above (with autoindent if set)
  R        — enter replace (overwrite) mode
  x        — delete char   <N>x — delete N chars
  r<c>     — replace char under cursor
  ~        — toggle case   dd — delete line   D — delete to EOL
  dw/d$/d0 — delete word / to-end / to-start
  cc/cw/c$/C — change line / word / to-end
  yy       — yank (copy) line   p/P — paste below/above
  J        — join lines   >> / << — indent / dedent
  .        — repeat last edit   u — undo (not yet available)
  ZZ/ZQ    — save+quit / force-quit

Command-mode (:) commands:
  :q       — quit (if saved)   :q! — force quit
  :w       — write   :wq / :x — write and quit
  :<N>     — go to line N
  :set     — show settings
  :set number / :set nonumber — toggle line numbers
  :set tabstop=N / :set ts=N — set tab width (1-16)
  :set autoindent / :set noai — toggle auto-indent
  :set showmatch / :set nosm  — toggle bracket flash
  :set showmode / :set noshowmode — toggle mode indicator
  :s/pat/rep/   — substitute first match on line
  :s/pat/rep/g  — substitute all matches on line

See also: help, edit",
    ),
    (
        "history",
        "\
HISTORY(1) — Shell command history

Usage:
  history            — show all history entries (numbered)
  history <N>        — show last N entries
  history clear      — clear the history ring buffer

The shell remembers the last 32 unique commands. Duplicate consecutive
commands are collapsed. Use Up/Down arrow keys to navigate history
at the command prompt.

Readline keys at the prompt:
  Up/Down   — previous / next history entry
  Ctrl-A/E  — move to start / end of line
  Ctrl-U/K  — kill to start / end of line
  Ctrl-W    — kill word backward
  Ctrl-B/F  — move back / forward one character
  Ctrl-L    — clear screen and redraw prompt
  Ctrl-T    — transpose characters
  Alt-b/f   — move back / forward one word
  Alt-d     — kill word forward
  Tab       — (reserved for completion)

See also: set, help",
    ),
    (
        "set",
        "\
SET(1) — View / change shell variables

Usage:
  set                — show all current settings
  set <key>=<val>    — set variable to value
  set <key> <val>    — same (space-separated)
  set no<key>        — set boolean to false

Variables:
  number     (bool)  — show line numbers in vi (default: off)
  tabstop    (1-16)  — tab stop width in vi (default: 4)
  showmatch  (bool)  — flash matching bracket in vi (default: off)
  autoindent (bool)  — auto-indent new lines in vi (default: off)
  prompt     (str)   — shell prompt string (default: 'veeros>')

Examples:
  set tabstop=8
  set autoindent
  set nonumber
  set prompt myos>

Settings are passed to the vi editor when it is launched.

See also: history, vi, help",
    ),
    (
        "ls",
        "\
LS(1) — List directory contents

Usage:
  ls             — list current directory
  ls <path>      — list the given directory

Columns: type (d=dir, f=file, c=device), size, name.

See also: tree, cd, pwd, stat",
    ),
    (
        "cat",
        "\
CAT(1) — Display file contents

Usage:
  cat <file>     — print the file to the terminal

Binary files show a summary instead. Maximum read: 1024 bytes.

See also: hexdump, write, cp",
    ),
    (
        "mkdir",
        "\
MKDIR(1) — Create a directory

Usage:
  mkdir <dir>    — create a new directory at the given path

Parent directories must already exist.

See also: rmdir, ls, tree",
    ),
    (
        "touch",
        "\
TOUCH(1) — Create an empty file

Usage:
  touch <file>   — create a new empty file if it does not exist

If the file already exists, this command has no effect.

See also: write, rm, cat",
    ),
    (
        "rm",
        "\
RM(1) — Remove files or directories

Usage:
  rm <path>      — remove a file or empty directory
  rmdir <path>   — alias for rm
  del <path>     — alias for rm

Cannot remove non-empty directories or mounted filesystems.

See also: touch, mkdir, ls",
    ),
    (
        "cp",
        "\
CP(1) — Copy a file

Usage:
  cp <src> <dst> — copy src to dst (overwrite if exists)

Maximum file size for copy: 1024 bytes. Does not copy directories.

See also: mv, cat, write",
    ),
    (
        "mv",
        "\
MV(1) — Move or rename a file/directory

Usage:
  mv <src> <dst> — rename src to dst

Moves within the same filesystem only (no cross-mount).

See also: cp, rm, rename",
    ),
    (
        "pwd",
        "\
PWD(1) — Print working directory

Usage:
  pwd            — print the current working directory path

See also: cd, ls",
    ),
    (
        "cd",
        "\
CD(1) — Change working directory

Usage:
  cd <dir>       — change to the given directory
  cd             — change to root (/)

Supports absolute paths (/etc) and relative paths (../tmp).

See also: pwd, ls",
    ),
    (
        "stat",
        "\
STAT(1) — Show file/directory metadata

Usage:
  stat <path>    — display inode info for the given path

Shows: type, size, inode number, parent, device major/minor.

See also: ls, cat",
    ),
    (
        "hexdump",
        "\
HEXDUMP(1) — Hex dump of a file

Usage:
  hexdump <file> — display file contents in hex + ASCII
  xxd <file>     — alias for hexdump

Maximum read: 512 bytes.

See also: cat, stat",
    ),
    (
        "write",
        "\
WRITE(1) — Write text to a file

Usage:
  write <file> <text>  — write text into file (overwrite)

Creates the file if it doesn't exist. Overwrites existing content.
For appending, use 'echo text >> file'.

See also: echo, cat, touch",
    ),
    (
        "tree",
        "\
TREE(1) — Recursive directory tree

Usage:
  tree           — display tree from /
  tree <path>    — display tree from the given path

Shows the directory hierarchy with indentation.

See also: ls, cd, pwd",
    ),
    (
        "echo",
        "\
ECHO(1) — Echo arguments / file redirection

Usage:
  echo <text>            — print text to terminal
  echo <text> > <file>   — write text to file (overwrite)
  echo <text> >> <file>  — append text to file

See also: write, cat",
    ),
    (
        "vfs",
        "\
VFS(7) — VeerOS Virtual Filesystem

VeerOS provides an in-memory virtual filesystem with:
  - RamFS: file data pool (64 KB on QEMU/RPi, 8 KB on ESP32)
  - InodeTable: up to 128 inodes (files, dirs, devices)
  - Per-process FD table (16 slots)
  - Device nodes: /dev/null, /dev/zero, /dev/console, /dev/random

Standard directories at boot: /, /dev, /tmp, /etc
Boot files: /etc/motd, /etc/hostname

See also: ls, cat, stat, tree",
    ),
    (
        "gpio",
        "\
GPIO(1) — GPIO pin control

Commands:
  gpio              — list all pin states
  gpio list         — same as above
  gpio read <pin>   — read digital level of a specific pin
  gpio write <pin> <0|1>  — set pin output level
  gpio mode <pin> <in|out>  — set pin direction
  gpio pull <pin> <none|up|down>  — set pull resistor

Examples:
  gpio                  Show all 28 GPIO pins
  gpio read 17          Read GPIO 17
  gpio mode 18 out      Set GPIO 18 to output
  gpio write 18 1       Set GPIO 18 high
  gpio pull 4 up        Enable pull-up on GPIO 4

Pin numbers are 0–27 on Raspberry Pi 5 (RP1 southbridge).

See also: hwinfo, i2c, spi",
    ),
    (
        "i2c",
        "\
I2C(1) — I\u{00B2}C bus commands

Commands:
  i2c scan [bus]                Scan for responsive devices
  i2c read <bus> <addr> <reg>   Read one byte from device
  i2c write <bus> <addr> <reg> <val>  Write one byte to device

Arguments:
  bus   — I2C bus number (0-6, default 1)
  addr  — 7-bit device address (decimal or 0xNN hex)
  reg   — register address (decimal or 0xNN hex)
  val   — byte value (decimal or 0xNN hex)

Examples:
  i2c scan 1           Scan bus 1 for devices
  i2c read 1 0x48 0    Read register 0 from device 0x48 on bus 1
  i2c write 1 0x20 6 0xFF  Write 0xFF to register 6 on device 0x20

See also: gpio, spi, hwinfo",
    ),
    (
        "spi",
        "\
SPI(1) — SPI bus commands

Commands:
  spi cfg <bus> <mode> <freq_div>  Configure SPI bus
  spi xfer <bus> <hex_bytes>       Full-duplex transfer

Arguments:
  bus       — SPI bus number (0-5)
  mode      — SPI mode (0-3)
  freq_div  — clock divisor (even number, 2-65534)
  hex_bytes — space-separated hex bytes to transmit

Examples:
  spi cfg 0 0 64        Configure SPI0: mode 0, divisor 64
  spi xfer 0 9F 00 00   Send 3 bytes, print responses (JEDEC read)

See also: gpio, i2c, hwinfo",
    ),
    (
        "hwinfo",
        "\
HWINFO(1) — Hardware information

Displays platform hardware details:
  - Board model and revision
  - Memory size
  - SoC temperature
  - ARM/core clock frequency
  - Serial number
  - MAC address (if available)

Alias: devinfo

See also: sysinfo, temp, gpio",
    ),
    (
        "temp",
        "\
TEMP(1) — SoC temperature

Reads and displays the SoC temperature from hardware sensors.

Example output:
  SoC temperature: 42.3°C

Uses VideoCore mailbox (RPi) or internal ADC (ESP32).

See also: hwinfo, sysinfo",
    ),
    (
        "dmesg",
        "\
DMESG(1) — Kernel log buffer

Displays messages from the kernel log ring buffer, including boot
messages, driver init, interrupts, errors, and hardware detection.

The log is a fixed-size ring buffer (newest entries overwrite oldest).

See also: sysinfo, hwinfo, drivers",
    ),
    (
        "caps",
        "\
CAPS(1) — Process capability management

Show, inspect, or drop per-process capabilities.

Usage:
  caps             List all processes with capability summary
  caps <pid>       Show detailed capabilities for process <pid>
  caps drop <pid> <cap>  Irrevocably drop a capability from a process

Capability names: task_basic, mem, time, sync, ipc, channel, poll,
  console_io, fs, net, spawn_thread, spawn_process, user_admin,
  driver, mount, hw, crypto, cap_admin

Capabilities restrict which syscall groups a process may invoke.
Once dropped, a capability cannot be restored — the process and
its children are permanently denied access to those syscalls.

See also: tasks, sysinfo",
    ),
    (
        "reboot",
        "\
REBOOT(1) — Reboot the system

Triggers a hardware reset. Uses the watchdog timer (RPi) or
software reset register (ESP32).

Warning: rebooting clears all in-memory state.

See also: shutdown",
    ),
    (
        "shutdown",
        "\
SHUTDOWN(1) — Halt or power off

Halts the CPU. On Raspberry Pi, enters low-power halt via firmware.
On emulators (QEMU), exits the emulator.

Aliases: halt, poweroff

See also: reboot",
    ),
    // ── AI-Native Execution man pages ────────────────────────────────
    (
        "agents",
        "\
AGENTS(1) — Autonomous agent lifecycle management

Agents are first-class autonomous execution primitives in VeerOS.
Each agent owns a goal, working memory (context), a compute budget,
and links to a kernel thread. Agents form hierarchies (parent/child).

Subcommands:
  agents              List all active agents
  agents list         Same as above
  agents spawn <goal> Spawn a new agent with the given goal
  agents kill <id>    Destroy an agent by ID
  agents status <id>  Show detailed agent status

Agent states: free, spawned, planning, executing, blocked,
              completed, failed.

Goal priorities: background, normal, elevated, critical, realtime.

Each agent has an 8-slot key-value context memory accessible via
SYS_AGENT_CTX_SET (0xF3) and SYS_AGENT_CTX_GET (0xF4) syscalls.

Userlib: userlib::agent (spawn, status, complete_agent, fail_agent,
         ctx_set, ctx_get, count).

See also: intent, fabric, memory",
    ),
    (
        "intent",
        "\
INTENT(1) — Declarative goal submission and tracking

Intents are high-level goals submitted to the kernel's Intent Engine.
The engine decomposes each intent into a plan — a DAG of steps — and
the Intent Scheduler assigns agents to execute each step.

Subcommands:
  intent              List all active intents
  intent list         Same as above
  intent submit <class> <description>
                      Submit a new intent
  intent status <id>  Query intent status
  intent cancel <id>  Cancel an in-flight intent
  intent stats        Show scheduler statistics

Intent classes: compute, deploy, monitor, communicate, data,
                admin, pipeline, custom.

Intent status lifecycle:
  free → pending → planning → active → fulfilled / failed / cancelled

Decomposition rules (built-in):
  compute   → 1 step (execute)
  deploy    → 3 steps (validate → provision → verify)
  data      → 2 steps (acquire → transform)
  pipeline  → 4 steps (ingest → transform → validate → output)

Userlib: userlib::intent (submit, status, cancel, sched_stats).

See also: agents, fabric, memory",
    ),
    (
        "memory",
        "\
MEMORY(1) — Persistent kernel knowledge store

The Memory Engine provides three tiers of memory for AI-native workloads:

1. Context memory — per-agent 8-slot key-value store (fast, ephemeral).
   Accessed via SYS_AGENT_CTX_SET/GET from within an agent thread.

2. Persistent memory — global key-value store that survives agent
   lifecycle. Tagged with MemoryTag (system, preference, cache, config,
   relation, skill, user_know, observation) and scoped (global, intent,
   agent, process). LRU eviction when full. Confidence scoring.

3. Episodic memory — ring buffer of event records (agent spawned,
   intent fulfilled, budget exceeded, etc.). Used by the scheduler
   to learn from past outcomes (success_rate, find_by_intent).

Subcommands:
  memory              Show memory engine stats
  memory get <key>    Query a value from persistent memory
  memory set <key> <value>
                      Store a key-value pair
  memory stats        Show read/write/eviction counts

Syscalls: SYS_MEMORY_STORE (0xF8), SYS_MEMORY_QUERY (0xF9).

Userlib: userlib::memory (store, query, fabric_status).

See also: agents, intent, fabric",
    ),
    (
        "fabric",
        "\
FABRIC(1) — Execution fabric node topology

The Execution Fabric tracks heterogeneous compute nodes available
for agent placement. Each node has:

  - Architecture (riscv32, riscv64, aarch64, x86_64, xtensa)
  - Locality zone (local, rack, datacenter, region, global)
  - Capability bitmask (compute, gpu, npu, storage, network, ...)
  - Resource snapshot (cores, MHz, RAM, load %, agent count)
  - Health state (healthy, degraded, overloaded, offline)

The Intent Scheduler uses the fabric to select the best node for
each agent, considering required capabilities, resource fit,
locality preference, and current load.

Usage:
  fabric              Show fabric node summary
    fabric status       Same as above
    fabric snapshot     Emit deterministic world-state hash for audit/replay
    fabric simulate <policy> <class> <description>
                                            Run policy what-if placement simulation (no state change)
    fabric trace        Show decision trace timeline
    fabric trace clear  Clear decision trace timeline
    fabric fault status List current node health fault table
    fabric fault inject <node> [offline|degraded|overloaded|healthy]
                                            Inject a controlled health fault
    fabric fault clear <node>
                                            Restore node health to healthy

Node scoring algorithm:
  1. Filter by required capabilities
  2. Check minimum resources (cores, RAM)
  3. Score by locality (prefer closer zones)
  4. Score by load (prefer less loaded nodes)
  5. Check RTT constraints

Syscalls: SYS_FABRIC_STATUS (0xFA) — returns (total, healthy).

See also: agents, intent, memory",
    ),
    (
        "demo",
        "\
DEMO(1) — AI-native interactive walkthrough

Runs live demonstrations of VeerOS AI-native execution primitives.
Each scenario exercises real kernel subsystems (agents, intents,
memory engine, execution fabric) in a scripted sequence.

Usage:
  demo                Show available scenarios
  demo deploy         Service deployment end-to-end
  demo pipeline       Data pipeline with multi-step decomposition
  demo monitor        Monitoring agent swarm across fabric nodes
    demo living         Living-fabric cognition loop (sense->plan->act->reflect)
    demo incident       Autonomous incident drill (fault->replan->audit)
  demo full           Run all scenarios sequentially

Scenarios:

  deploy — Deploys a service by:
    1. Inspecting the execution fabric
    2. Storing deployment config in persistent memory
    3. Submitting a 'deploy' intent (auto-decomposed into 3 steps)
    4. Spawning agents for each deployment step
    5. Executing and completing agents
    6. Recording outcome in memory

  pipeline — Sets up a 4-stage data pipeline:
    1. Submitting a 'pipeline' intent (4 steps: ingest, transform,
       validate, output)
    2. Spawning stage agents
    3. Storing pipeline metadata

  monitor — Spawns a monitoring agent swarm:
    1. Submitting a 'monitor' intent
    2. Spawning agents targeting different fabric nodes
    3. Configuring monitoring thresholds in memory

    living — Demonstrates LLM-like capability without an LLM:
        1. Sensing live fabric + mesh state
        2. Building a world model in persistent memory
        3. Planning via intent decomposition
        4. Acting via distributed execution agents
        5. Reflecting with explainable decision records

After each demo, use individual commands (agents, intent, memory,
fabric) to inspect the resulting kernel state.

See also: agents, intent, memory, fabric",
    ),
    (
        "peers",
        "\
PEERS(1) — Zero Trust peer management

Manage remote fabric nodes with mutual authentication.
Every peer starts as Untrusted and progresses through a
challenge-response protocol:

  Untrusted → Challenged → Verified → Attested → (Revoked)

Each peer has:
  - 32-byte node ID (cryptographic identity)
  - Trust level (determines allowed operations)
  - Capability bitmask (what the peer can do)
  - Session key (for encrypted communication)

Subcommands:
  peers               List all known peers with trust levels
  peers status <idx>  Show detailed peer info

Syscalls: SYS_PEER_REGISTER (0xE2), SYS_PEER_VERIFY (0xE3),
          SYS_PEER_STATUS (0xE4).

See also: mesh, zkp, fabric",
    ),
    (
        "mesh",
        "\
MESH(1) — Mesh transport layer

The mesh transport provides multi-hop message delivery between
fabric nodes. Messages are routed through the peer network
with priority-based queuing.

Features:
  - Direct and indirect (multi-hop) routing
  - Priority-based outbox (0-255)
  - Periodic heartbeat and announce ticks
  - Stale peer timeout and eviction
  - Max 8 hops per message

Subcommands:
  mesh                Show mesh transport statistics
  mesh routes         List known routes and their RTT

Syscalls: SYS_MESH_SEND (0xE7), SYS_MESH_STATUS (0xE8).

See also: peers, zkp, fabric",
    ),
    (
        "zkp",
        "\
ZKP(1) — Zero-Knowledge Proof operations

Prove or verify capabilities without revealing the full
capability set. Uses Merkle-tree commitments with selective
disclosure (reveal only the bits you want to prove).

Proof types:
  - Capability proof: prove you hold specific capabilities
  - Completion proof: Fiat-Shamir non-interactive proof of
    task completion
  - Memory existence proof: prove a key/tag exists without
    revealing the value

Subcommands:
  zkp                 Show ZKP subsystem info
  zkp prove <caps>    Generate a capability proof
  zkp verify          Verify a received proof

Syscalls: SYS_ZKP_PROVE (0xE5), SYS_ZKP_VERIFY (0xE6).

See also: peers, mesh, fabric",
    ),
    (
        "df",
        "\
DF(1) — Report filesystem disk space usage

Usage:
  df             — show space usage for all filesystems

Displays:
  - RamFS data pool: total, used, and available bytes
  - Inode usage: allocated vs maximum inodes
  - Mounted FAT32 partitions (label and mount point)
  - Block devices (capacity)

Example:
  veeros> df
    Filesystem      Size      Used     Avail  Use%  Mounted on
    ------------  --------  --------  ------  ----  ----------
    ramfs           64 KB    2 KB     62 KB    3%  /
    inodes          12/128
    /dev/vda        64 MB                           (block device)

See also: mount, lsblk, meminfo, vfs",
    ),
];

#[cfg(test)]
mod tests {
    // Shell is interactive — integration tests live in veeros-demo.
}

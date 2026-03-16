//! VeerOS interactive shell.
//!
//! Provides a minimal line-editor REPL with built-in commands.
//! The shell is `no_std` and generic over any [`arch::Serial`] backend,
//! so it runs on both real hardware and a host-emulated console.

#![no_std]

use core::fmt::Write;
use arch::{Console, Serial};

pub mod vi;
mod line_ed;

pub use line_ed::{History, LineEditor, LineResult};

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

/// Shell prompt string.
const PROMPT: &str = "veeros> ";

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

    // ── Input device callbacks ───────────────────────────────────────
    /// Write input subsystem status to writer.
    pub input_status: Option<fn(&mut dyn core::fmt::Write)>,
    /// List connected USB devices. Writes output to writer.
    pub usb_list: Option<fn(&mut dyn core::fmt::Write)>,
    /// List connected BLE HID devices. Writes output to writer.
    pub ble_hid_list: Option<fn(&mut dyn core::fmt::Write)>,
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
        }
    }

    /// Run the shell REPL.
    ///
    /// Returns normally when the user types `exit`, `quit`, or Ctrl-D.
    pub fn run<S: Serial>(&mut self, con: &mut Console<S>) {
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

    fn print_prompt<S: Serial>(&self, con: &mut Console<S>) {
        if let Some(f) = self.env.get_current_user {
            let (_uid, name) = f();
            con.write_str_raw(name);
            con.write_str_raw("@veeros> ");
        } else {
            con.write_str_raw(PROMPT);
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

        // Split into command + rest-of-line arguments.
        let (cmd, args) = match line.find(' ') {
            Some(i) => (&line[..i], line[i + 1..].trim()),
            None => (line, ""),
        };

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
            "clear" | "cls" => self.cmd_clear(con),
            "echo" => self.cmd_echo(con, args),
            "logo" => self.cmd_logo(con),
            "uname" => self.cmd_uname(con),
            "man" => self.cmd_man(con, args),
            "whoami" => self.cmd_whoami(con),
            "users" => self.cmd_users(con),
            "vi" | "edit" => self.cmd_vi(con, args),
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
            // ── input device commands ────────────────────
            "input" => self.cmd_input(con, args),
            "lsusb" => self.cmd_lsusb(con),
            "exit" | "quit" => {
                let _ = writeln!(con, "Goodbye.");
                return true;
            }
            _ => {
                let _ = writeln!(con, "unknown command: '{}'", cmd);
                let _ = writeln!(con, "Type 'help' for available commands.");
            }
        }

        false
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
        let _ = writeln!(con, "  bt         Bluetooth LE (scan/list/advertise/stop/status)");
        let _ = writeln!(con, "  zigbee     ZigBee/Thread 802.15.4 (init/scan/channel/send/status)");
        let _ = writeln!(con, "  clear      Clear the screen");
        let _ = writeln!(con, "  echo       Echo arguments");
        let _ = writeln!(con, "  logo       Display VeerOS logo");
        let _ = writeln!(con, "  man        Show manual page (man <topic>)");
        let _ = writeln!(con, "  whoami     Display current user");
        let _ = writeln!(con, "  users      List active user sessions");
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
        let _ = writeln!(con, "  ────────── input ─────────────────");
        let _ = writeln!(con, "  input      Input device status");
        let _ = writeln!(con, "  lsusb      List USB devices");
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
        if !args.is_empty() {
            editor.load(args);
            editor.set_filename("scratch");
        }
        let _saved = editor.run(con);
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
                if n < count { n } else { count }
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
            let _ = writeln!(con, "    number      = {}", if self.vars.vi_number { "on" } else { "off" });
            let _ = writeln!(con, "    tabstop     = {}", self.vars.vi_tabstop);
            let _ = writeln!(con, "    showmatch   = {}", if self.vars.vi_showmatch { "on" } else { "off" });
            let _ = writeln!(con, "    autoindent  = {}", if self.vars.vi_autoindent { "on" } else { "off" });
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
                let _ = writeln!(con, "  number = {}", if self.vars.vi_number { "on" } else { "off" });
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
                let _ = writeln!(con, "  showmatch = {}", if self.vars.vi_showmatch { "on" } else { "off" });
            }
            "noshowmatch" | "nosm" => {
                self.vars.vi_showmatch = false;
                let _ = writeln!(con, "  showmatch = off");
            }
            "autoindent" | "ai" => {
                self.vars.vi_autoindent = !matches!(val, "off" | "0" | "false" | "no");
                let _ = writeln!(con, "  autoindent = {}", if self.vars.vi_autoindent { "on" } else { "off" });
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
                let _ = writeln!(con, "  Variables: number tabstop showmatch autoindent prompt");
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
                let path = if args.is_empty() { "." } else { args };
                f(path, con);
            }
            None => { let _ = writeln!(con, "filesystem not available"); }
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
            None => { let _ = writeln!(con, "filesystem not available"); }
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
            None => { let _ = writeln!(con, "filesystem not available"); }
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
            None => { let _ = writeln!(con, "filesystem not available"); }
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
            None => { let _ = writeln!(con, "filesystem not available"); }
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
            None => { let _ = writeln!(con, "filesystem not available"); return; }
        };
        let write_fn = match self.env.vfs_write_file {
            Some(f) => f,
            None => { let _ = writeln!(con, "filesystem not available"); return; }
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
            None => { let _ = writeln!(con, "filesystem not available"); }
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
            None => { let _ = writeln!(con, "filesystem not available"); }
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
            None => { let _ = writeln!(con, "filesystem not available"); }
        }
    }

    fn cmd_stat<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            let _ = writeln!(con, "Usage: stat <path>");
            return;
        }
        match self.env.vfs_stat {
            Some(f) => f(args, con),
            None => { let _ = writeln!(con, "filesystem not available"); }
        }
    }

    fn cmd_hexdump<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            let _ = writeln!(con, "Usage: hexdump <file>");
            return;
        }
        let read_fn = match self.env.vfs_read_file {
            Some(f) => f,
            None => { let _ = writeln!(con, "filesystem not available"); return; }
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
            None => { let _ = writeln!(con, "filesystem not available"); }
        }
    }

    fn cmd_tree<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        match self.env.vfs_tree {
            Some(f) => {
                let path = if args.is_empty() { "/" } else { args };
                f(path, con);
            }
            None => { let _ = writeln!(con, "filesystem not available"); }
        }
    }

    // ── storage commands ─────────────────────────────────────────────

    fn cmd_mount<S: Serial>(&self, con: &mut Console<S>, args: &str) {
        if args.is_empty() {
            // No arguments → list mounts.
            match self.env.mount_list {
                Some(f) => f(con),
                None => { let _ = writeln!(con, "mount: not available"); }
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
            None => { let _ = writeln!(con, "mount: not available"); }
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
            None => { let _ = writeln!(con, "umount: not available"); }
        }
    }

    fn cmd_lsblk<S: Serial>(&self, con: &mut Console<S>) {
        match self.env.lsblk {
            Some(f) => f(con),
            None => { let _ = writeln!(con, "lsblk: not available"); }
        }
    }

    fn cmd_input<S: Serial>(&self, con: &mut Console<S>, _args: &str) {
        let _ = writeln!(con, "── Input Devices ──");
        match self.env.input_status {
            Some(f) => f(con),
            None => { let _ = writeln!(con, "  input subsystem: not available"); }
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
            None => { let _ = writeln!(con, "lsusb: not available on this platform"); }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Simple no_std usize parser.
fn parse_usize_simple(s: &str) -> Result<usize, ()> {
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

/// Simple no_std u8 parser.
fn parse_u8_simple(s: &str) -> Result<u8, ()> {
    let n = parse_usize_simple(s)?;
    if n > 255 { Err(()) } else { Ok(n as u8) }
}

// ═══════════════════════════════════════════════════════════════════════════
// Embedded man page table
// ═══════════════════════════════════════════════════════════════════════════

static MAN_PAGES: &[(&str, &str)] = &[
    ("scheduler", "\
SCHEDULER(7) — VeerOS Scheduler

The VeerOS scheduler is a preemptive, priority-based round-robin scheduler.
Tasks are stored in a fixed-size TCB (Task Control Block) table. The timer
ISR fires every 1 ms and calls the scheduler to pick the highest-priority
ready task. Equal-priority tasks are round-robined.

Task states: Free, Ready, Running, Blocked, Suspended, Zombie.

See also: tasks, yield, exit, spawn"),

    ("ipc", "\
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

See also: channel, send, recv, poll"),

    ("memory", "\
MEMORY(7) — VeerOS Memory Management

Memory isolation uses RISC-V PMP (Physical Memory Protection) on rv32imc.
Each task has up to 4 memory regions with Read/Write/Execute permissions.
Stack regions are auto-granted at task creation with a 64-byte guard zone.

Syscalls: SYS_ALLOC (0x40), SYS_FREE (0x41),
          SYS_MEM_REGION_COUNT (0x42), SYS_MEM_REGION_INFO (0x43).

Pointer validation: user pointers in syscalls are checked against the
calling task's granted memory regions.

See also: meminfo, alloc, free"),

    ("boot", "\
BOOT(7) — VeerOS Boot Sequence

1. Reset vector → _start (assembly): set up stack, zero BSS
2. _veer_start (Rust): init UART, timers, interrupts
3. Create init process (PID 0) with task threads
4. _veer_start_first_task (asm): load context, mret to first task
5. Timer ISR begins preemptive scheduling

Boot tasks: idle (lowest priority), shell, net listener (QEMU).

See also: scheduler, tasks"),

    ("yield", "\
YIELD(1) — Yield CPU to scheduler

  Syscall: SYS_YIELD (0x00)
  Userlib: task::yield_now()

Voluntarily gives up the current time slice. The scheduler picks the
next ready task. If no other task is ready, the caller continues."),

    ("exit", "\
EXIT(1) — Terminate the calling task

  Syscall: SYS_EXIT (0x01)
  Args:    a0 = exit code
  Userlib: task::exit(code)

Terminates the calling task, sets state to Free, and wakes any tasks
blocked in SYS_JOIN on this task, delivering the exit code."),

    ("spawn", "\
SPAWN(1) — Create a new task

  Syscall: SYS_SPAWN (0x05)
  Args:    a0 = entry point, a1 = stack pointer, a2 = priority
  Returns: a0 = new task ID, or usize::MAX on failure
  Userlib: task::spawn(entry, stack, priority)

Creates a new task (thread) within the current process. The new task
starts at the given entry point with the given stack and priority.

See also: join, exit"),

    ("join", "\
JOIN(1) — Wait for a task to exit

  Syscall: SYS_JOIN (0x06)
  Args:    a0 = target task ID
  Returns: a0 = exit code of target
  Userlib: task::join(task_id) -> usize

Blocks the caller until the target task exits. Returns the target's
exit code. If the target has already exited, returns immediately."),

    ("sleep", "\
SLEEP(1) — Sleep for N ticks

  Syscall: SYS_SLEEP (0x30)
  Args:    a0 = low 32 bits of ticks, a1 = high 32 bits
  Userlib: time::sleep(ticks)

Blocks the calling task for at least the specified number of timer
ticks (1 ms per tick). The task is woken by the timer ISR."),

    ("send", "\
SEND(1) — Send a message on a channel

  Syscall: SYS_CHAN_SEND (0x59)
  Args:    a0 = channel ID, a1–a4 = message words
  Returns: a0 = 1 on success, 0 if channel closed
  Userlib: channel::send(chan_id, w0, w1, w2, w3)

Sends a 4-word message. Blocks if the channel is full.

See also: recv, channel, poll"),

    ("recv", "\
RECV(1) — Receive a message from a channel

  Syscall: SYS_CHAN_RECV (0x5A)
  Args:    a0 = channel ID
  Returns: a0–a3 = message words (usize::MAX,0,0,0 if closed)
  Userlib: channel::recv(chan_id) -> (usize, usize, usize, usize)

Receives a 4-word message. Blocks if the channel is empty.

See also: send, channel, poll"),

    ("channel", "\
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

See also: send, recv, ipc"),

    ("socket", "\
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

See also: ipc, channel"),

    ("futex", "\
FUTEX(7) — Fast userspace mutexes

VeerOS implements Linux-style futexes as the universal synchronization
building block. The kernel maintains a 32-slot wait queue keyed by address.

Syscalls:
  SYS_FUTEX_WAIT (0x50) — if *addr == expected, block
  SYS_FUTEX_WAKE (0x51) — wake up to N waiters on addr

Built on futex: Mutex<T>, RwLock<T>, Condvar, Semaphore.
Priority inheritance: holder is boosted to max waiter priority.

See also: sync, mutex"),

    ("sync", "\
SYNC(7) — Synchronization primitives

Userlib synchronization built on kernel futexes:

  Mutex<T>      — mutual exclusion with RAII MutexGuard
  RwLock<T>     — multiple readers xor one writer
  Condvar       — condition variable (wait/notify)
  Semaphore     — counting semaphore

All use volatile read/write (no atomics on rv32imc) + futex for blocking.

See also: futex"),

    ("tasks", "\
TASKS(1) — Shell command: list running tasks

Displays the task table: ID, name, state, priority, process ID.
Alias: ps

See also: scheduler, spawn, exit"),

    ("help", "\
HELP(1) — Shell command: show available commands

Type 'help' or '?' at the shell prompt to see all built-in commands.
Type 'man <topic>' for detailed documentation on a specific topic."),

    ("poll", "\
POLL(7) — Event polling subsystem

Register interest in multiple events and block until any fires.

Event types: POLL_TIMER, POLL_IPC, POLL_CHAN_READABLE,
             POLL_CHAN_WRITABLE, POLL_TASK_EXIT.

Syscalls:
  SYS_POLL_SET (0x60)  — register an event
  SYS_POLL_WAIT (0x61) — block until event fires

Async runtime: block_on() executor uses poll for cooperative I/O.

See also: async, channel, ipc"),

    ("process", "\
PROCESS(7) — Process model

A process owns an address space (ASID), capability token set, and child
threads. Threads within a process share memory but have separate stacks.

Syscalls:
  SYS_SPAWN_PROCESS (0x07) — create new process (entry, stack, prio)
  SYS_PROCESS_ID    (0x08) — get calling thread's process ID
  SYS_THREAD_COUNT  (0x09) — thread count in a process

Userlib: task::spawn_process(), task::process_id(), task::thread_count()

See also: spawn, tls, memory"),

    ("tls", "\
TLS(7) — Thread-Local Storage

Each thread has a TLS base pointer stored in the RISC-V tp register.

Syscalls:
  SYS_TLS_GET (0x0A) — get TLS base pointer
  SYS_TLS_SET (0x0B) — set TLS base pointer

Userlib: task::tls_get(), task::tls_set(base)

See also: process, spawn"),

    ("alloc", "\
ALLOC(1) — Memory allocation

VeerOS uses a fixed-pool allocator with two block sizes:
  Small blocks: 64 bytes   Large blocks: 1024 bytes

Syscalls:
  SYS_ALLOC (0x40) — allocate memory (a0 = size, returns ptr)
  SYS_FREE  (0x41) — free memory (a0 = ptr, a1 = size)

Returns null (0) if no block of sufficient size is available.

See also: memory, meminfo"),

    ("mutex", "\
MUTEX(3) — Mutual exclusion lock

  userlib::sync::Mutex<T>

Provides exclusive access to shared data. Uses volatile read/write plus
kernel futex for blocking (no atomics on rv32imc).

  let m = Mutex::new(0u32);
  { let guard = m.lock();  // blocks until acquired
    *guard += 1; }          // MutexGuard auto-unlocks on drop

Priority inheritance: holder is boosted to max waiter priority.

See also: rwlock, condvar, semaphore, futex"),

    ("rwlock", "\
RWLOCK(3) — Reader-writer lock

  userlib::sync::RwLock<T>

Allows multiple concurrent readers or one exclusive writer.

  let rw = RwLock::new(data);
  { let r = rw.read();  }   // shared, multiple readers OK
  { let w = rw.write(); }   // exclusive, blocks readers

Uses futex-based blocking. No atomics required.

See also: mutex, sync, futex"),

    ("condvar", "\
CONDVAR(3) — Condition variable

  userlib::sync::Condvar

Wait for and signal conditions between tasks.

  let cv = Condvar::new();
  cv.wait(&mutex_guard);    // release mutex + sleep
  cv.notify_one();           // wake one waiter
  cv.notify_all();           // wake all waiters

Based on futex with a sequence counter to avoid lost wakes.

See also: mutex, sync, futex"),

    ("semaphore", "\
SEMAPHORE(3) — Counting semaphore

  userlib::sync::Semaphore

Controls concurrent access to a bounded resource pool.

  let sem = Semaphore::new(3);  // max 3 concurrent holders
  sem.acquire();                 // blocks if count == 0
  sem.release();                 // increments count, wakes waiter

Built on futex. Useful for producer-consumer and resource limiting.

See also: mutex, sync, futex"),

    ("async", "\
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

See also: poll, channel"),

    ("io", "\
IO(7) — Console I/O

Syscalls:
  SYS_WRITE_BYTE (0x20) — write one byte to console
  SYS_WRITE_BUF  (0x21) — write buffer (ptr + len)
  SYS_READ_BYTE  (0x22) — read one byte (blocking)

Userlib: io::write_byte(), io::write_buf(), io::read_byte()
Macros:  print!(), println!() — formatted output via Console

See also: help"),

    ("tick", "\
TICK(1) — Kernel tick counter

  Syscall: SYS_TICK (0x30)
  Returns: a0 = low 32 bits, a1 = high 32 bits
  Userlib: time::ticks() -> u64

The kernel increments a 64-bit counter every timer ISR (1 ms).

See also: sleep, uptime"),

    ("drivers", "\
DRIVERS(7) — Driver isolation framework

VeerOS maintains a driver registry for hardware device drivers.
Each driver declares capabilities: MMIO regions, interrupts, DMA, network.

Driver registration:
  name, MemRegion(base, size), IRQ line, caps (mmio/irq/dma/net)

Access is checked: drivers may only touch their granted MMIO regions.
Shell: 'drivers' or 'lsdrv' to list registered drivers.

See also: memory, pmp"),

    ("pmp", "\
PMP(7) — Physical Memory Protection

RISC-V PMP enforces hardware memory isolation. On every context switch,
the kernel programs PMP CSRs (pmpaddr0-15, pmpcfg0-3) with the current
task's granted memory regions (TOR mode).

Each task gets up to 4 regions with R/W/X permissions.
Stack regions are auto-granted at task creation.
64-byte guard zones below each stack catch overflows (no-access).

See also: memory, drivers"),

    ("wifi", "\
WIFI(1) — Shell Wi-Fi management

Commands:
  wifi scan      — scan for access points
  wifi list      — show saved networks
  wifi set <ssid> <pass> — store credentials
  wifi connect   — connect to saved network
  wifi status    — show connection status

Requires: ESP32 radio hardware or QEMU net bridge.

See also: bt, zigbee"),

    ("bt", "\
BT(1) — Shell Bluetooth LE management

Commands:
  bt scan        — scan for BLE devices
  bt list        — show discovered devices
  bt advertise   — start BLE advertising
  bt stop        — stop advertising
  bt status      — show BLE status

Aliases: ble

See also: wifi, zigbee"),

    ("zigbee", "\
ZIGBEE(1) — Shell IEEE 802.15.4 / Thread management

Commands:
  zigbee init    — initialize 802.15.4 radio
  zigbee scan    — scan channels
  zigbee channel <n> — set channel
  zigbee send <data> — transmit frame
  zigbee status  — show radio status

Aliases: thread, 802154

See also: wifi, bt"),

    ("uname", "\
UNAME(1) — Shell command: print system information

Displays: VeerOS version, platform name, scheduler profile.
Equivalent to: uname -a on Unix systems.

See also: sysinfo, version"),

    ("meminfo", "\
MEMINFO(1) — Shell command: show memory statistics

Displays usage for small-block and large-block memory pools:
total blocks, used blocks, free blocks, block size.

Aliases: mem, free

See also: alloc, memory"),

    ("whoami", "\
WHOAMI(1) — Print effective user name

Displays the username and UID of the current session.

Usage:
  whoami

Example:
  veeros> whoami
  root (uid=0)

See also: users, login, logout"),

    ("users", "\
USERS(1) — List user accounts and active sessions

Displays all registered users, their UIDs, and whether they have
an active session.

Usage:
  users

Example:
  USER     UID  STATUS
  root       0  active
  user       1

See also: whoami, login, logout"),

    ("login", "\
LOGIN(3) — Authenticate and start a user session

Syscall: SYS_LOGIN (0x93)

  a0 = pointer to username (null-terminated)
  a1 = pointer to password (null-terminated)

Returns a non-zero session token on success, 0 on failure.
After 3 consecutive failures for the same account, the
account is locked until the password is reset by root.

See also: logout, whoami, users"),

    ("logout", "\
LOGOUT(3) — End a user session

Syscall: SYS_LOGOUT (0x94)

Ends the current session and resets the process UID to root (0).
The session token is invalidated.

Returns 1 on success, 0 if no active session.

See also: login, whoami"),

    ("getuid", "\
GETUID(3) — Get effective user ID

Syscall: SYS_GETUID (0x90)

Returns the UID (u16) of the calling process.
Root UID is 0, nobody is 0xFFFF.

Userlib: userlib::user::getuid() -> u16

See also: getgid, setuid, whoami"),

    ("getgid", "\
GETGID(3) — Get effective group ID

Syscall: SYS_GETGID (0x91)

Returns the GID (u16) of the calling process.

Userlib: userlib::user::getgid() -> u16

See also: getuid, setuid"),

    ("setuid", "\
SETUID(3) — Set effective user ID (privileged)

Syscall: SYS_SETUID (0x92)

  a0 = new UID (u16)

Only root (uid=0) may call setuid. Returns 1 on success, 0 if
the caller is not root.

Userlib: userlib::user::setuid(uid: u16) -> bool

See also: getuid, getgid, login"),

    ("vi", "\
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

See also: help, edit"),

    ("history", "\
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

See also: set, help"),

    ("set", "\
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

See also: history, vi, help"),

    ("ls", "\
LS(1) — List directory contents

Usage:
  ls             — list current directory
  ls <path>      — list the given directory

Columns: type (d=dir, f=file, c=device), size, name.

See also: tree, cd, pwd, stat"),

    ("cat", "\
CAT(1) — Display file contents

Usage:
  cat <file>     — print the file to the terminal

Binary files show a summary instead. Maximum read: 1024 bytes.

See also: hexdump, write, cp"),

    ("mkdir", "\
MKDIR(1) — Create a directory

Usage:
  mkdir <dir>    — create a new directory at the given path

Parent directories must already exist.

See also: rmdir, ls, tree"),

    ("touch", "\
TOUCH(1) — Create an empty file

Usage:
  touch <file>   — create a new empty file if it does not exist

If the file already exists, this command has no effect.

See also: write, rm, cat"),

    ("rm", "\
RM(1) — Remove files or directories

Usage:
  rm <path>      — remove a file or empty directory
  rmdir <path>   — alias for rm
  del <path>     — alias for rm

Cannot remove non-empty directories or mounted filesystems.

See also: touch, mkdir, ls"),

    ("cp", "\
CP(1) — Copy a file

Usage:
  cp <src> <dst> — copy src to dst (overwrite if exists)

Maximum file size for copy: 1024 bytes. Does not copy directories.

See also: mv, cat, write"),

    ("mv", "\
MV(1) — Move or rename a file/directory

Usage:
  mv <src> <dst> — rename src to dst

Moves within the same filesystem only (no cross-mount).

See also: cp, rm, rename"),

    ("pwd", "\
PWD(1) — Print working directory

Usage:
  pwd            — print the current working directory path

See also: cd, ls"),

    ("cd", "\
CD(1) — Change working directory

Usage:
  cd <dir>       — change to the given directory
  cd             — change to root (/)

Supports absolute paths (/etc) and relative paths (../tmp).

See also: pwd, ls"),

    ("stat", "\
STAT(1) — Show file/directory metadata

Usage:
  stat <path>    — display inode info for the given path

Shows: type, size, inode number, parent, device major/minor.

See also: ls, cat"),

    ("hexdump", "\
HEXDUMP(1) — Hex dump of a file

Usage:
  hexdump <file> — display file contents in hex + ASCII
  xxd <file>     — alias for hexdump

Maximum read: 512 bytes.

See also: cat, stat"),

    ("write", "\
WRITE(1) — Write text to a file

Usage:
  write <file> <text>  — write text into file (overwrite)

Creates the file if it doesn't exist. Overwrites existing content.
For appending, use 'echo text >> file'.

See also: echo, cat, touch"),

    ("tree", "\
TREE(1) — Recursive directory tree

Usage:
  tree           — display tree from /
  tree <path>    — display tree from the given path

Shows the directory hierarchy with indentation.

See also: ls, cd, pwd"),

    ("echo", "\
ECHO(1) — Echo arguments / file redirection

Usage:
  echo <text>            — print text to terminal
  echo <text> > <file>   — write text to file (overwrite)
  echo <text> >> <file>  — append text to file

See also: write, cat"),

    ("vfs", "\
VFS(7) — VeerOS Virtual Filesystem

VeerOS provides an in-memory virtual filesystem with:
  - RamFS: file data pool (64 KB on QEMU/RPi, 8 KB on ESP32)
  - InodeTable: up to 128 inodes (files, dirs, devices)
  - Per-process FD table (16 slots)
  - Device nodes: /dev/null, /dev/zero, /dev/console, /dev/random

Standard directories at boot: /, /dev, /tmp, /etc
Boot files: /etc/motd, /etc/hostname

See also: ls, cat, stat, tree"),
];

#[cfg(test)]
mod tests {
    // Shell is interactive — integration tests live in veeros-demo.
}

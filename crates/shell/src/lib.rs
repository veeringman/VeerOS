//! VeerOS interactive shell.
//!
//! Provides a minimal line-editor REPL with built-in commands.
//! The shell is `no_std` and generic over any [`arch::Serial`] backend,
//! so it runs on both real hardware and a host-emulated console.

#![no_std]

use core::fmt::Write;
use arch::{Console, Serial};

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum line length (bytes).
const MAX_LINE: usize = 128;

/// Shell prompt string.
const PROMPT: &str = "veeros> ";

// ASCII control codes
const BS: u8 = 0x08;
const DEL: u8 = 0x7F;
const CR: u8 = 0x0D;
const LF: u8 = 0x0A;
const ETX: u8 = 0x03; // Ctrl-C
const EOT: u8 = 0x04; // Ctrl-D
const ESC: u8 = 0x1B;
const TAB: u8 = 0x09;

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
}

// ═══════════════════════════════════════════════════════════════════════════
// Shell
// ═══════════════════════════════════════════════════════════════════════════

/// Interactive VeerOS shell.
pub struct Shell {
    buf: [u8; MAX_LINE],
    pos: usize,
    env: ShellEnv,
}

impl Shell {
    /// Create a new shell with the given environment.
    pub fn new(env: ShellEnv) -> Self {
        Self {
            buf: [0u8; MAX_LINE],
            pos: 0,
            env,
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

        self.print_prompt(con);

        loop {
            let byte = con.read_byte();

            match byte {
                // ── Enter ───────────────────────────────────────────
                CR | LF => {
                    let _ = writeln!(con, ""); // newline after the typed line
                    if self.pos > 0 {
                        let should_exit = self.execute(con);
                        self.pos = 0;
                        if should_exit {
                            return;
                        }
                    }
                    self.print_prompt(con);
                }

                // ── Backspace / Delete ──────────────────────────────
                BS | DEL => {
                    if self.pos > 0 {
                        self.pos -= 1;
                        // Erase on terminal: back, space, back
                        con.write_str_raw("\x08 \x08");
                    }
                }

                // ── Ctrl-C ──────────────────────────────────────────
                ETX => {
                    con.write_str_raw("^C");
                    let _ = writeln!(con, "");
                    self.pos = 0;
                    self.print_prompt(con);
                }

                // ── Ctrl-D ──────────────────────────────────────────
                EOT => {
                    let _ = writeln!(con, "");
                    let _ = writeln!(con, "logout");
                    return;
                }

                // ── Escape sequences (arrow keys, etc.) — discard ──
                ESC => {
                    // Terminal escape sequences are typically ESC [ <code>.
                    // Greedily consume if bytes are already buffered.
                    if con.has_data() {
                        let _ = con.read_byte(); // usually '['
                        if con.has_data() {
                            let _ = con.read_byte(); // direction letter
                        }
                    }
                }

                // ── Tab — autocomplete placeholder ──────────────────
                TAB => {
                    // TODO: command auto-completion in a future release.
                }

                // ── Printable ASCII ────────────────────────────────
                0x20..=0x7E => {
                    if self.pos < MAX_LINE - 1 {
                        self.buf[self.pos] = byte;
                        self.pos += 1;
                        // Echo the character
                        let ch = [byte];
                        if let Ok(s) = core::str::from_utf8(&ch) {
                            con.write_str_raw(s);
                        }
                    }
                }

                _ => {} // Ignore other control bytes
            }
        }
    }

    // ── internal helpers ─────────────────────────────────────────────────

    fn print_prompt<S: Serial>(&self, con: &mut Console<S>) {
        con.write_str_raw(PROMPT);
    }

    /// Parse and execute the line buffer.  Returns `true` if the shell
    /// should exit.
    fn execute<S: Serial>(&self, con: &mut Console<S>) -> bool {
        let line = match core::str::from_utf8(&self.buf[..self.pos]) {
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
            "clear" | "cls" => self.cmd_clear(con),
            "echo" => self.cmd_echo(con, args),
            "logo" => self.cmd_logo(con),
            "uname" => self.cmd_uname(con),
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
        let _ = writeln!(con, "  clear      Clear the screen");
        let _ = writeln!(con, "  echo       Echo arguments");
        let _ = writeln!(con, "  logo       Display VeerOS logo");
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

    fn cmd_logo<S: Serial>(&self, con: &mut Console<S>) {
        let _ = writeln!(con, "");
        let _ = writeln!(con, "  ╦  ╦┌─┐┌─┐┬─┐╔═╗╔═╗");
        let _ = writeln!(con, "  ╚╗╔╝├┤ ├┤ ├┬┘║ ║╚═╗");
        let _ = writeln!(con, "   ╚╝ └─┘└─┘┴└─╚═╝╚═╝");
        let _ = writeln!(con, "    Microkernel v{}", self.env.version);
        let _ = writeln!(con, "");
    }
}

#[cfg(test)]
mod tests {
    // Shell is interactive — integration tests live in veeros-demo.
}

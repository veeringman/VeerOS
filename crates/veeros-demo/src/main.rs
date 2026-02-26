//! VeerOS host-runnable demo.
//!
//! This binary boots VeerOS on the host machine by emulating the serial
//! console over stdin / stdout.  It runs the full boot banner sequence
//! followed by the interactive VeerOS shell.
//!
//! ```
//! cargo run -p veeros-demo
//! ```

use std::io::{self, Read, Write as IoWrite};
use std::os::unix::io::AsRawFd;

use arch::{Console, Serial};
use shell::{Shell, ShellEnv};

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ═══════════════════════════════════════════════════════════════════════════
// Terminal raw mode (POSIX)
// ═══════════════════════════════════════════════════════════════════════════

/// Saved original terminal settings so we can restore on exit.
static mut ORIG_TERMIOS: Option<libc::termios> = None;

/// Put stdin into raw mode and save the original settings.
fn enable_raw_mode() {
    unsafe {
        let mut orig: libc::termios = std::mem::zeroed();
        libc::tcgetattr(libc::STDIN_FILENO, &mut orig);
        ORIG_TERMIOS = Some(orig);

        let mut raw = orig;
        libc::cfmakeraw(&mut raw);
        // Keep ISIG so Ctrl-C can still generate SIGINT if needed,
        // but we handle Ctrl-C in the shell itself.
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VMIN] = 1;  // read returns after 1 byte
        raw.c_cc[libc::VTIME] = 0; // no timeout
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
    }
}

/// Restore the original terminal settings.
fn disable_raw_mode() {
    unsafe {
        if let Some(ref orig) = ORIG_TERMIOS {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, orig);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Serial backend over host stdin / stdout
// ═══════════════════════════════════════════════════════════════════════════

/// Host serial implementation — stdin for RX, stdout for TX.
struct StdSerial;

impl Serial for StdSerial {
    fn write_byte(&self, byte: u8) {
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        if byte == b'\n' {
            // In raw mode bare LF doesn't return the cursor.
            let _ = lock.write_all(b"\r\n");
        } else {
            let _ = lock.write_all(&[byte]);
        }
        let _ = lock.flush();
    }

    fn read_byte(&self) -> u8 {
        let mut buf = [0u8; 1];
        let _ = io::stdin().lock().read_exact(&mut buf);
        buf[0]
    }

    fn has_data(&self) -> bool {
        let fd = io::stdin().as_raw_fd();
        let mut count: libc::c_int = 0;
        unsafe {
            libc::ioctl(fd, libc::FIONREAD, &mut count);
        }
        count > 0
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// main — simulated boot sequence → shell
// ═══════════════════════════════════════════════════════════════════════════

fn main() {
    // Raw terminal so the shell gets byte-at-a-time input.
    enable_raw_mode();

    // Make sure we restore the terminal no matter how we exit.
    // (Ctrl-D / `exit` / panic)
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            disable_raw_mode();
        }
    }
    let _guard = Guard;

    // Build the console over our host serial backend.
    let serial = StdSerial;
    let mut con = Console::new(serial);

    // ── Boot banner (mirrors the real kernel-esp32 boot path) ────────
    use core::fmt::Write;
    let _ = writeln!(con, "");
    let _ = writeln!(con, "========================================");
    let _ = writeln!(con, "  VeerOS v{VERSION}");
    let _ = writeln!(con, "  Platform : Host (demo)");
    let _ = writeln!(con, "  Scheduler: minimal");
    let _ = writeln!(con, "========================================");
    let _ = writeln!(con, "");
    let _ = writeln!(con, "[boot] trap vector skipped (host build)");
    let _ = writeln!(con, "[boot] interrupt controller  — n/a (host)");
    let _ = writeln!(con, "[boot] systimer tick          — n/a (host)");
    let _ = writeln!(con, "[boot] idle task registered");
    let _ = writeln!(con, "[boot] shell starting...");
    let _ = writeln!(con, "");

    // ── Launch the shell ─────────────────────────────────────────────
    let env = ShellEnv {
        version: VERSION,
        platform: "Host (demo)",
        scheduler: "minimal",
        get_uptime_ticks: None,
        get_task_list: None,
    };
    let mut sh = Shell::new(env);
    sh.run(&mut con);

    let _ = writeln!(con, "VeerOS halted.");
}

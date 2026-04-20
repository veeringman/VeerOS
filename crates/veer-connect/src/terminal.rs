//! Interactive terminal mode over VSC encrypted channel.
//!
//! Sets the local terminal to raw mode and shuttles bytes between
//! stdin/stdout and the encrypted TCP connection.  Ctrl-] disconnects.

use crate::vsc::{self, SecureChannel, MAX_FRAME_PT};

use std::io::{self, Write};
use std::net::TcpStream;
use std::os::unix::io::AsRawFd;

/// Run the interactive terminal loop.
pub fn run(stream: &TcpStream, ch: &mut SecureChannel) {
    // Save original terminal settings.
    let stdin_fd = io::stdin().as_raw_fd();
    let old_termios = match get_termios(stdin_fd) {
        Some(t) => t,
        None => {
            eprintln!("[vsc] warning: could not get terminal settings");
            return;
        }
    };

    // Set raw mode.
    set_raw_mode(stdin_fd);

    // Make socket non-blocking for the select loop.
    stream
        .set_nonblocking(true)
        .expect("failed to set non-blocking");

    let result = terminal_loop(stream, ch);

    // Restore terminal.
    set_termios(stdin_fd, &old_termios);
    stream.set_nonblocking(false).ok();

    if let Err(e) = result {
        eprintln!("\r\n[vsc] {}\r", e);
    }
}

fn terminal_loop(stream: &TcpStream, ch: &mut SecureChannel) -> Result<(), String> {
    let sock_fd = stream.as_raw_fd();
    let stdin_fd = io::stdin().as_raw_fd();

    // Print session banner.
    let banner = "\r\n\x1b[32m[vsc] encrypted session established\x1b[0m\r\n";
    io::stdout().write_all(banner.as_bytes()).ok();
    io::stdout().flush().ok();

    loop {
        // Use poll(2) to wait on stdin and socket.
        let mut fds = [
            libc_pollfd(stdin_fd, POLLIN),
            libc_pollfd(sock_fd, POLLIN),
        ];

        let ret = unsafe { libc_poll(fds.as_mut_ptr(), 2, 100) };
        if ret < 0 {
            return Err("poll error".into());
        }

        // Check stdin.
        if fds[0].revents & POLLIN != 0 {
            let mut buf = [0u8; MAX_FRAME_PT];
            let n = unsafe { libc_read(stdin_fd, buf.as_mut_ptr(), buf.len()) };
            if n <= 0 {
                return Ok(());
            }
            let data = &buf[..n as usize];

            // Ctrl-] (0x1d) to disconnect.
            if data.contains(&0x1d) {
                let msg = "\r\n\x1b[33m[vsc] disconnected\x1b[0m\r\n";
                io::stdout().write_all(msg.as_bytes()).ok();
                io::stdout().flush().ok();
                return Ok(());
            }

            vsc::send_frame(stream, ch, data);
        }

        // Check socket.
        if fds[1].revents & POLLIN != 0 {
            let mut buf = [0u8; MAX_FRAME_PT];
            match vsc::recv_frame(stream, ch, &mut buf) {
                Some(n) => {
                    // Convert \n → \r\n for the terminal.
                    let data = &buf[..n];
                    let mut out = Vec::with_capacity(n * 2);
                    for &b in data {
                        if b == b'\n' {
                            out.push(b'\r');
                        }
                        out.push(b);
                    }
                    io::stdout().write_all(&out).ok();
                    io::stdout().flush().ok();
                }
                None => {
                    let msg = "\r\n\x1b[31m[vsc] connection closed\x1b[0m\r\n";
                    io::stdout().write_all(msg.as_bytes()).ok();
                    io::stdout().flush().ok();
                    return Ok(());
                }
            }
        }

        // Check for socket hangup.
        if fds[1].revents & (POLLHUP | POLLERR) != 0 {
            let msg = "\r\n\x1b[31m[vsc] connection lost\x1b[0m\r\n";
            io::stdout().write_all(msg.as_bytes()).ok();
            io::stdout().flush().ok();
            return Ok(());
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Minimal libc bindings for terminal control (Unix)
// ═══════════════════════════════════════════════════════════════════════════
//
// We use raw syscalls instead of pulling in the `libc` or `nix` crate to
// keep the dependency tree minimal.  This is Linux-specific for now;
// the constants and struct layouts are the same on macOS.

const POLLIN: i16 = 0x0001;
const POLLHUP: i16 = 0x0010;
const POLLERR: i16 = 0x0008;

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

fn libc_pollfd(fd: i32, events: i16) -> PollFd {
    PollFd { fd, events, revents: 0 }
}

extern "C" {
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
    fn read(fd: i32, buf: *mut u8, count: usize) -> isize;
    fn tcgetattr(fd: i32, termios: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, action: i32, termios: *const Termios) -> i32;
    fn cfmakeraw(termios: *mut Termios);
}

unsafe fn libc_poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32 {
    unsafe { poll(fds, nfds, timeout) }
}

unsafe fn libc_read(fd: i32, buf: *mut u8, count: usize) -> isize {
    unsafe { read(fd, buf, count) }
}

// Termios struct — Linux x86_64 layout.
// The exact layout varies by arch but the fields we touch (c_lflag, c_iflag,
// c_oflag, c_cflag) are at the same offsets on all Linux targets.
#[repr(C)]
#[derive(Clone, Copy)]
struct Termios {
    c_iflag: u32,
    c_oflag: u32,
    c_cflag: u32,
    c_lflag: u32,
    c_line: u8,
    c_cc: [u8; 32],
    c_ispeed: u32,
    c_ospeed: u32,
}

const TCSADRAIN: i32 = 1;

fn get_termios(fd: i32) -> Option<Termios> {
    let mut t = Termios {
        c_iflag: 0, c_oflag: 0, c_cflag: 0, c_lflag: 0,
        c_line: 0, c_cc: [0; 32], c_ispeed: 0, c_ospeed: 0,
    };
    let ret = unsafe { tcgetattr(fd, &mut t as *mut Termios) };
    if ret == 0 { Some(t) } else { None }
}

fn set_termios(fd: i32, t: &Termios) {
    unsafe { tcsetattr(fd, TCSADRAIN, t as *const Termios); }
}

fn set_raw_mode(fd: i32) {
    if let Some(mut t) = get_termios(fd) {
        unsafe { cfmakeraw(&mut t as *mut Termios); }
        set_termios(fd, &t);
    }
}

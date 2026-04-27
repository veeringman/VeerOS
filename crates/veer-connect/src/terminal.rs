//! Interactive terminal mode over VSC encrypted channel.
//!
//! Sets the local terminal to raw mode and shuttles bytes between
//! stdin/stdout and the encrypted TCP connection.  Ctrl-] disconnects.

use crate::vsc::{self, SecureChannel, MAX_FRAME_PT};
use libc::{self, termios};

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
            // Non-TTY stdin (pipe, redirect). Run without raw mode.
            stream
                .set_nonblocking(true)
                .expect("failed to set non-blocking");
            let _ = terminal_loop(stream, ch);
            stream.set_nonblocking(false).ok();
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
            libc::pollfd { fd: stdin_fd, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: sock_fd, events: libc::POLLIN, revents: 0 },
        ];

        let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 100) };
        if ret < 0 {
            return Err("poll error".into());
        }

        // Check stdin.
        if fds[0].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; MAX_FRAME_PT];
            let n = unsafe { libc::read(stdin_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
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
        if fds[1].revents & libc::POLLIN != 0 {
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
        if fds[1].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            let msg = "\r\n\x1b[31m[vsc] connection lost\x1b[0m\r\n";
            io::stdout().write_all(msg.as_bytes()).ok();
            io::stdout().flush().ok();
            return Ok(());
        }
    }
}
fn get_termios(fd: i32) -> Option<termios> {
    let mut t: termios = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::tcgetattr(fd, &mut t as *mut termios) };
    if ret == 0 { Some(t) } else { None }
}

fn set_termios(fd: i32, t: &termios) {
    unsafe { libc::tcsetattr(fd, libc::TCSADRAIN, t as *const termios); }
}

fn set_raw_mode(fd: i32) {
    if let Some(mut t) = get_termios(fd) {
        unsafe { libc::cfmakeraw(&mut t as *mut termios); }
        set_termios(fd, &t);
    }
}

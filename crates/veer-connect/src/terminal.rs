//! Interactive terminal mode over VSC encrypted channel.
//!
//! Sets the local terminal to raw mode and shuttles bytes between
//! stdin/stdout and the encrypted TCP connection.  Ctrl-] disconnects.

use crate::vsc::{self, SecureChannel, MAX_FRAME_PT};
#[cfg(unix)]
use libc::{self, termios};

use std::io::{self, Write};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

/// Run the interactive terminal loop.
#[cfg(unix)]
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

#[cfg(not(unix))]
pub fn run(stream: &TcpStream, ch: &mut SecureChannel) {
    eprintln!("[vsc] interactive shell on this host uses line mode (Ctrl+C to exit)");

    let stdin = io::stdin();
    let mut buf = [0u8; MAX_FRAME_PT];

    // Show the initial guest output (login banner + prompt) before blocking
    // on stdin.  Without this, the Windows line-mode loop would block on
    // stdin.read_line() and never display the login: / password: prompts.
    if !drain_guest_output(stream, ch, &mut buf) {
        return;
    }

    loop {
        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => {
                // stdin EOF — drain any final response from the guest then exit.
                drain_guest_output(stream, ch, &mut buf);
                break;
            }
            Ok(_) => {
                vsc::send_frame(stream, ch, line.as_bytes());
                // Drain all frames the guest sends in response (may be several
                // for multi-line output like `help`).
                if !drain_guest_output(stream, ch, &mut buf) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// Drain all frames the guest has buffered, writing them to stdout.
///
/// Uses `TcpStream::peek` with a short read timeout to detect when no more
/// data is arriving without permanently blocking.  Returns `false` if the
/// connection was closed by the peer.
#[cfg(not(unix))]
fn drain_guest_output(stream: &TcpStream, ch: &mut SecureChannel, buf: &mut [u8]) -> bool {
    use std::io::ErrorKind;
    use std::time::Duration;

    // Idle window: if no new byte arrives within this period, assume the
    // guest has finished sending its current response.
    const IDLE_MS: u64 = 800;

    stream
        .set_read_timeout(Some(Duration::from_millis(IDLE_MS)))
        .ok();

    loop {
        let mut peek = [0u8; 1];
        match stream.peek(&mut peek) {
            Ok(0) => {
                // Peer closed the connection.
                stream.set_read_timeout(None).ok();
                return false;
            }
            Ok(_) => {
                // Data is available — read a full frame without a timeout so
                // read_exact doesn't bail mid-frame.
                stream.set_read_timeout(None).ok();
                match vsc::recv_frame(stream, ch, buf) {
                    Some(n) => {
                        let _ = io::stdout().write_all(&buf[..n]);
                        let _ = io::stdout().flush();
                    }
                    None => return false,
                }
                // Re-arm the idle window for the next frame.
                stream
                    .set_read_timeout(Some(Duration::from_millis(IDLE_MS)))
                    .ok();
            }
            Err(ref e)
                if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
            {
                // Idle window elapsed — no more data for now.
                stream.set_read_timeout(None).ok();
                return true;
            }
            Err(_) => {
                stream.set_read_timeout(None).ok();
                return false;
            }
        }
    }
}

#[cfg(unix)]
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
            libc::pollfd {
                fd: stdin_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: sock_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 100) };
        if ret < 0 {
            return Err("poll error".into());
        }

        // Check stdin.
        if fds[0].revents & libc::POLLIN != 0 {
            let mut buf = [0u8; MAX_FRAME_PT];
            let n =
                unsafe { libc::read(stdin_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
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
#[cfg(unix)]
fn get_termios(fd: i32) -> Option<termios> {
    let mut t: termios = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::tcgetattr(fd, &mut t as *mut termios) };
    if ret == 0 {
        Some(t)
    } else {
        None
    }
}

#[cfg(unix)]
fn set_termios(fd: i32, t: &termios) {
    unsafe {
        libc::tcsetattr(fd, libc::TCSADRAIN, t as *const termios);
    }
}

#[cfg(unix)]
fn set_raw_mode(fd: i32) {
    if let Some(mut t) = get_termios(fd) {
        unsafe {
            libc::cfmakeraw(&mut t as *mut termios);
        }
        set_termios(fd, &t);
    }
}

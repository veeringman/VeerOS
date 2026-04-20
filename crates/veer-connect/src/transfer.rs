//! Secure file transfer over VSC encrypted channel.
//!
//! ## Protocol (after VSC handshake):
//!
//! ### Push (upload)
//! 1. Client sends mode frame: `[0x01]`
//! 2. Server authenticates (password prompt over encrypted channel)
//! 3. Client sends transfer header:
//!    `[path_len:u16 LE][path bytes][file_size:u32 LE]`
//! 4. Client sends file data in encrypted frames (≤ 256 bytes each)
//! 5. Server responds with status: `[0x00]` = OK, `[0x01]` = error
//!
//! ### Pull (download)
//! 1. Client sends mode frame: `[0x02]`
//! 2. Server authenticates (password prompt over encrypted channel)
//! 3. Client sends request: `[path_len:u16 LE][path bytes]`
//! 4. Server responds: `[status:u8][file_size:u32 LE]`
//!    status 0x00 = OK (file data follows), 0x01 = not found
//! 5. Server sends file data in encrypted frames

use crate::vsc::{self, SecureChannel, MODE_PUSH, MODE_PULL, MAX_FRAME_PT};

use std::fs;
use std::io::{self, Write};
use std::net::TcpStream;
use std::process;

/// Transfer status codes.
const STATUS_OK: u8 = 0x00;

/// Authenticate interactively: read password prompt from server, send password.
fn authenticate(stream: &TcpStream, ch: &mut SecureChannel) -> bool {
    // The server sends the login prompt over the encrypted channel.
    // We need to read and display it, then send the password.
    let mut buf = [0u8; MAX_FRAME_PT];

    // Read and display server prompts until we see "Password:" or similar.
    // Then read password from local terminal and send it.
    loop {
        // Set a read timeout so we don't block forever.
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();

        match vsc::recv_frame(stream, ch, &mut buf) {
            Some(n) => {
                let text = &buf[..n];
                // Display the prompt.
                io::stderr().write_all(text).ok();
                io::stderr().flush().ok();

                // Check if this is a password prompt.
                let s = core::str::from_utf8(text).unwrap_or("");
                if s.contains("Password:") || s.contains("password:") {
                    // Read password from terminal (no echo).
                    let password = read_password();
                    // Send password + CR.
                    let mut pw_data = password.into_bytes();
                    pw_data.push(b'\r');
                    vsc::send_frame(stream, ch, &pw_data);
                } else if s.contains("VeerOS#") || s.contains("$") || s.contains("#") {
                    // We got a shell prompt — auth succeeded.
                    stream.set_read_timeout(None).ok();
                    return true;
                } else if s.contains("denied") || s.contains("failed") || s.contains("closed") {
                    eprintln!();
                    stream.set_read_timeout(None).ok();
                    return false;
                }
            }
            None => {
                stream.set_read_timeout(None).ok();
                return false;
            }
        }
    }
}

/// Read a password from the terminal without echo.
fn read_password() -> String {
    // Save terminal state, disable echo.
    let stdin_fd = 0i32;
    let old = unsafe {
        let mut t = std::mem::zeroed::<Termios>();
        tcgetattr(stdin_fd, &mut t);
        let old = t;
        // Disable echo.
        t.c_lflag &= !(ECHO);
        tcsetattr(stdin_fd, 0, &t);
        old
    };

    let mut password = String::new();
    io::stdin().read_line(&mut password).ok();

    // Restore terminal.
    unsafe { tcsetattr(stdin_fd, 0, &old); }
    eprintln!(); // newline after hidden input

    password.trim_end().to_string()
}

// Minimal termios for password reading.
const ECHO: u32 = 0x0008;

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

extern "C" {
    fn tcgetattr(fd: i32, termios: *mut Termios) -> i32;
    fn tcsetattr(fd: i32, action: i32, termios: *const Termios) -> i32;
}

/// Upload a local file to the VeerOS machine.
pub fn push(stream: &TcpStream, ch: &mut SecureChannel, local_path: &str, remote_path: &str) {
    // Read local file.
    let data = match fs::read(local_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[vsc] error reading {}: {}", local_path, e);
            process::exit(1);
        }
    };

    let file_size = data.len();
    eprintln!("[vsc] uploading {} ({} bytes) → {}", local_path, file_size, remote_path);

    // Send mode byte.
    vsc::send_frame(stream, ch, &[MODE_PUSH]);

    // Authenticate.
    if !authenticate(stream, ch) {
        eprintln!("[vsc] authentication failed");
        process::exit(1);
    }

    // Send transfer header: [path_len:u16][path][file_size:u32].
    let path_bytes = remote_path.as_bytes();
    let path_len = path_bytes.len() as u16;
    let mut header = Vec::with_capacity(2 + path_bytes.len() + 4);
    header.extend_from_slice(&path_len.to_le_bytes());
    header.extend_from_slice(path_bytes);
    header.extend_from_slice(&(file_size as u32).to_le_bytes());
    vsc::send_frame(stream, ch, &header);

    // Send file data in chunks.
    let mut sent = 0usize;
    while sent < file_size {
        let end = std::cmp::min(sent + MAX_FRAME_PT, file_size);
        vsc::send_frame(stream, ch, &data[sent..end]);
        sent = end;

        // Progress indicator.
        let pct = (sent * 100) / file_size.max(1);
        eprint!("\r[vsc] progress: {}%  ({}/{})", pct, sent, file_size);
    }
    eprintln!();

    // Read server status.
    let mut status_buf = [0u8; MAX_FRAME_PT];
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();
    match vsc::recv_frame(stream, ch, &mut status_buf) {
        Some(n) if n >= 1 && status_buf[0] == STATUS_OK => {
            eprintln!("[vsc] upload complete: {} → {}", local_path, remote_path);
        }
        Some(n) if n >= 1 => {
            let msg = if n > 1 {
                core::str::from_utf8(&status_buf[1..n]).unwrap_or("unknown error")
            } else {
                "server reported error"
            };
            eprintln!("[vsc] upload failed: {}", msg);
            process::exit(1);
        }
        _ => {
            eprintln!("[vsc] upload failed: no response from server");
            process::exit(1);
        }
    }
}

/// Download a file from the VeerOS machine.
pub fn pull(stream: &TcpStream, ch: &mut SecureChannel, remote_path: &str, local_path: &str) {
    eprintln!("[vsc] downloading {} → {}", remote_path, local_path);

    // Send mode byte.
    vsc::send_frame(stream, ch, &[MODE_PULL]);

    // Authenticate.
    if !authenticate(stream, ch) {
        eprintln!("[vsc] authentication failed");
        process::exit(1);
    }

    // Send download request: [path_len:u16][path].
    let path_bytes = remote_path.as_bytes();
    let path_len = path_bytes.len() as u16;
    let mut req = Vec::with_capacity(2 + path_bytes.len());
    req.extend_from_slice(&path_len.to_le_bytes());
    req.extend_from_slice(path_bytes);
    vsc::send_frame(stream, ch, &req);

    // Read server response: [status:u8][file_size:u32].
    let mut resp = [0u8; MAX_FRAME_PT];
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();
    match vsc::recv_frame(stream, ch, &mut resp) {
        Some(n) if n >= 5 => n,
        Some(n) if n >= 1 && resp[0] != STATUS_OK => {
            let msg = if n > 1 {
                core::str::from_utf8(&resp[1..n]).unwrap_or("unknown error")
            } else {
                "file not found"
            };
            eprintln!("[vsc] download failed: {}", msg);
            process::exit(1);
        }
        _ => {
            eprintln!("[vsc] download failed: no response from server");
            process::exit(1);
        }
    };

    if resp[0] != STATUS_OK {
        eprintln!("[vsc] download failed: server error");
        process::exit(1);
    }

    let file_size = u32::from_le_bytes([resp[1], resp[2], resp[3], resp[4]]) as usize;
    eprintln!("[vsc] file size: {} bytes", file_size);

    // Receive file data.
    let mut file_data = Vec::with_capacity(file_size);
    stream.set_read_timeout(Some(std::time::Duration::from_secs(30))).ok();

    while file_data.len() < file_size {
        let mut buf = [0u8; MAX_FRAME_PT];
        match vsc::recv_frame(stream, ch, &mut buf) {
            Some(n) => {
                let take = std::cmp::min(n, file_size - file_data.len());
                file_data.extend_from_slice(&buf[..take]);

                let pct = (file_data.len() * 100) / file_size.max(1);
                eprint!("\r[vsc] progress: {}%  ({}/{})", pct, file_data.len(), file_size);
            }
            None => {
                eprintln!("\n[vsc] download failed: connection lost");
                process::exit(1);
            }
        }
    }
    eprintln!();

    // Write to local file.
    match fs::write(local_path, &file_data) {
        Ok(()) => {
            eprintln!("[vsc] download complete: {} → {}", remote_path, local_path);
        }
        Err(e) => {
            eprintln!("[vsc] error writing {}: {}", local_path, e);
            process::exit(1);
        }
    }
}

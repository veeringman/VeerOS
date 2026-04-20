//! VeerOS Secure Connect (VSC) — encrypted shell and file transfer.
//!
//! # Usage
//!
//! ```text
//! veer-connect shell <host> <port>              Interactive encrypted shell
//! veer-connect push  <host> <port> <local> <remote>   Upload file
//! veer-connect pull  <host> <port> <remote> <local>   Download file
//! ```
//!
//! For SSH-enabled targets (x86_64 QEMU PC), `veeros-vm` delegates to
//! `ssh` / `scp` instead.  This tool handles the VSC protocol used by
//! all other targets (ESP32-C6, Raspi5 IoT, etc.).

mod vsc;
mod terminal;
mod transfer;

use std::env;
use std::process;

fn usage() {
    eprintln!("VeerOS Secure Connect v0.1");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  veer-connect shell <host> <port>");
    eprintln!("  veer-connect push  <host> <port> <local-path> <remote-path>");
    eprintln!("  veer-connect pull  <host> <port> <remote-path> <local-path>");
    eprintln!();
    eprintln!("Modes:");
    eprintln!("  shell  — Interactive encrypted terminal (Ctrl-] to disconnect)");
    eprintln!("  push   — Upload a local file to the VeerOS machine");
    eprintln!("  pull   — Download a file from the VeerOS machine");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
        process::exit(1);
    }

    match args[1].as_str() {
        "shell" => {
            if args.len() != 4 {
                eprintln!("Usage: veer-connect shell <host> <port>");
                process::exit(1);
            }
            let host = &args[2];
            let port: u16 = args[3].parse().unwrap_or_else(|_| {
                eprintln!("error: invalid port: {}", args[3]);
                process::exit(1);
            });
            cmd_shell(host, port);
        }
        "push" => {
            if args.len() != 6 {
                eprintln!("Usage: veer-connect push <host> <port> <local-path> <remote-path>");
                process::exit(1);
            }
            let host = &args[2];
            let port: u16 = args[3].parse().unwrap_or_else(|_| {
                eprintln!("error: invalid port: {}", args[3]);
                process::exit(1);
            });
            cmd_push(host, port, &args[4], &args[5]);
        }
        "pull" => {
            if args.len() != 6 {
                eprintln!("Usage: veer-connect pull <host> <port> <remote-path> <local-path>");
                process::exit(1);
            }
            let host = &args[2];
            let port: u16 = args[3].parse().unwrap_or_else(|_| {
                eprintln!("error: invalid port: {}", args[3]);
                process::exit(1);
            });
            cmd_pull(host, port, &args[4], &args[5]);
        }
        "help" | "--help" | "-h" => {
            usage();
        }
        other => {
            eprintln!("error: unknown command: {}", other);
            usage();
            process::exit(1);
        }
    }
}

fn cmd_shell(host: &str, port: u16) {
    eprintln!("[vsc] connecting to {}:{}...", host, port);
    let stream = match std::net::TcpStream::connect((host, port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[vsc] connection failed: {}", e);
            process::exit(1);
        }
    };

    eprintln!("[vsc] performing X25519 key exchange...");
    let mut ch = match vsc::client_handshake(&stream) {
        Some(c) => c,
        None => {
            eprintln!("[vsc] handshake failed");
            process::exit(1);
        }
    };

    // Send mode byte: 0x00 = shell.
    vsc::send_frame(&stream, &mut ch, &[vsc::MODE_SHELL]);

    eprintln!("[vsc] encrypted session (ChaCha20-Poly1305)");
    eprintln!("[vsc] press Ctrl-] to disconnect");

    terminal::run(&stream, &mut ch);
}

fn cmd_push(host: &str, port: u16, local_path: &str, remote_path: &str) {
    eprintln!("[vsc] connecting to {}:{}...", host, port);
    let stream = match std::net::TcpStream::connect((host, port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[vsc] connection failed: {}", e);
            process::exit(1);
        }
    };

    eprintln!("[vsc] performing X25519 key exchange...");
    let mut ch = match vsc::client_handshake(&stream) {
        Some(c) => c,
        None => {
            eprintln!("[vsc] handshake failed");
            process::exit(1);
        }
    };

    transfer::push(&stream, &mut ch, local_path, remote_path);
}

fn cmd_pull(host: &str, port: u16, remote_path: &str, local_path: &str) {
    eprintln!("[vsc] connecting to {}:{}...", host, port);
    let stream = match std::net::TcpStream::connect((host, port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[vsc] connection failed: {}", e);
            process::exit(1);
        }
    };

    eprintln!("[vsc] performing X25519 key exchange...");
    let mut ch = match vsc::client_handshake(&stream) {
        Some(c) => c,
        None => {
            eprintln!("[vsc] handshake failed");
            process::exit(1);
        }
    };

    transfer::pull(&stream, &mut ch, remote_path, local_path);
}

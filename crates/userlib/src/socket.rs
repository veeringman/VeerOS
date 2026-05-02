//! BSD-style socket API for VeerOS user tasks.
//!
//! Wraps the kernel socket syscalls into a safe Rust API.
//! Supports local (Unix-domain-like) sockets; TCP/UDP stubs for future use.
//!
//! # Example
//!
//! ```rust,ignore
//! use userlib::socket::{self, Domain, SockType};
//!
//! // Server
//! let srv = socket::socket(Domain::Local, SockType::Stream).unwrap();
//! socket::bind(srv, 100);
//! socket::listen(srv, 4);
//! let conn = socket::accept(srv).unwrap();
//! let mut buf = [0u8; 64];
//! let n = socket::recv(conn, &mut buf);
//!
//! // Client
//! let cli = socket::socket(Domain::Local, SockType::Stream).unwrap();
//! socket::connect(cli, 100);
//! socket::send(cli, b"hello");
//! socket::close(cli);
//! ```

use crate::sys;

const SYS_SOCKET: usize = 0x70;
const SYS_BIND: usize = 0x71;
const SYS_LISTEN: usize = 0x72;
const SYS_ACCEPT: usize = 0x73;
const SYS_CONNECT: usize = 0x74;
const SYS_SOCK_SEND: usize = 0x75;
const SYS_SOCK_RECV: usize = 0x76;
const SYS_SOCK_CLOSE: usize = 0x77;

/// Socket address family.
#[derive(Debug, Clone, Copy)]
pub enum Domain {
    /// Local (Unix-domain-like) — in-kernel ring buffer.
    Local = 0,
    /// Internet (IPv4) — will wrap smoltcp.
    Inet = 1,
}

/// Socket type.
#[derive(Debug, Clone, Copy)]
pub enum SockType {
    /// Byte stream (connection-oriented).
    Stream = 0,
    /// Datagram (connectionless).
    Dgram = 1,
}

/// Create a socket. Returns the handle, or `None` on failure.
#[inline]
pub fn socket(domain: Domain, sock_type: SockType) -> Option<usize> {
    let h = sys::syscall2(SYS_SOCKET, domain as usize, sock_type as usize).0;
    if h == usize::MAX {
        None
    } else {
        Some(h)
    }
}

/// Bind a socket to an address (port number for local sockets).
/// Returns `true` on success.
#[inline]
pub fn bind(handle: usize, addr: usize) -> bool {
    sys::syscall2(SYS_BIND, handle, addr).0 == 0
}

/// Mark a socket as listening. Returns `true` on success.
#[inline]
pub fn listen(handle: usize, backlog: usize) -> bool {
    sys::syscall2(SYS_LISTEN, handle, backlog).0 == 0
}

/// Accept a connection. Blocks until a client connects.
/// Returns the new connected socket handle, or `None` on error.
#[inline]
pub fn accept(handle: usize) -> Option<usize> {
    let h = sys::syscall1(SYS_ACCEPT, handle);
    if h == usize::MAX {
        None
    } else {
        Some(h)
    }
}

/// Connect to a listening socket at the given address.
/// Returns `true` on success.
#[inline]
pub fn connect(handle: usize, addr: usize) -> bool {
    sys::syscall2(SYS_CONNECT, handle, addr).0 == 0
}

/// Send bytes on a connected socket. Blocks if peer buffer is full.
/// Returns the number of bytes sent, or `usize::MAX` on error.
#[inline]
pub fn send(handle: usize, data: &[u8]) -> usize {
    sys::syscall3(SYS_SOCK_SEND, handle, data.as_ptr() as usize, data.len())
}

/// Receive bytes from a connected socket. Blocks if no data available.
/// Returns the number of bytes read, 0 on EOF, or `usize::MAX` on error.
#[inline]
pub fn recv(handle: usize, buf: &mut [u8]) -> usize {
    sys::syscall3(SYS_SOCK_RECV, handle, buf.as_mut_ptr() as usize, buf.len())
}

/// Close a socket. Returns `true` on success.
#[inline]
pub fn close(handle: usize) -> bool {
    sys::syscall1(SYS_SOCK_CLOSE, handle) == 0
}

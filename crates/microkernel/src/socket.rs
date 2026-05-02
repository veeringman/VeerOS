//! BSD-style socket subsystem for VeerOS.
//!
//! Supports local (Unix-domain-like) sockets backed by in-kernel ring
//! buffers.  TCP/UDP sockets are stubs that will wrap smoltcp later.

use crate::task::{BlockReason, Scheduler, TaskState};

/// Maximum number of simultaneously open sockets.
pub const MAX_SOCKETS: usize = 16;

/// Ring buffer depth per local socket direction (in bytes).
const LOCAL_BUF_SIZE: usize = 256;

// ── Socket domain / type ────────────────────────────────────────────────

/// Socket address family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Local = 0,
    Inet = 1,
}

/// Socket type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SockType {
    Stream = 0,
    Dgram = 1,
}

/// Socket state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SockState {
    Free,
    Created,
    Bound,
    Listening,
    Connected,
    Closed,
}

// ── Ring buffer (byte-granularity) ──────────────────────────────────────

/// A small fixed-size byte ring buffer for local-socket data transfer.
#[derive(Debug)]
struct RingBuf {
    buf: [u8; LOCAL_BUF_SIZE],
    head: usize,
    tail: usize,
    count: usize,
}

impl RingBuf {
    const fn new() -> Self {
        Self {
            buf: [0; LOCAL_BUF_SIZE],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn is_full(&self) -> bool {
        self.count >= LOCAL_BUF_SIZE
    }

    fn available(&self) -> usize {
        self.count
    }

    fn space(&self) -> usize {
        LOCAL_BUF_SIZE - self.count
    }

    /// Write bytes into the ring. Returns number of bytes written.
    fn write(&mut self, data: &[u8]) -> usize {
        let n = data.len().min(self.space());
        for &b in &data[..n] {
            self.buf[self.tail] = b;
            self.tail = (self.tail + 1) % LOCAL_BUF_SIZE;
            self.count += 1;
        }
        n
    }

    /// Read bytes from the ring. Returns number of bytes read.
    fn read(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.available());
        for slot in out[..n].iter_mut() {
            *slot = self.buf[self.head];
            self.head = (self.head + 1) % LOCAL_BUF_SIZE;
            self.count -= 1;
        }
        n
    }
}

// ── Socket struct ───────────────────────────────────────────────────────

/// A single socket slot.
pub struct Socket {
    pub state: SockState,
    pub domain: Domain,
    pub sock_type: SockType,
    /// Address this socket is bound to (for local sockets: a small integer "port").
    pub bound_addr: usize,
    /// For a connected local socket: handle of the peer socket.
    pub peer: usize,
    /// Ring buffer: data flowing *into* this socket (readable by owner).
    rx: RingBuf,
    /// For listening sockets: pending connection queue (socket handle of connector).
    pending_conn: [usize; 4],
    pending_conn_count: usize,
}

impl Socket {
    const fn empty() -> Self {
        Self {
            state: SockState::Free,
            domain: Domain::Local,
            sock_type: SockType::Stream,
            bound_addr: usize::MAX,
            peer: usize::MAX,
            rx: RingBuf::new(),
            pending_conn: [usize::MAX; 4],
            pending_conn_count: 0,
        }
    }
}

// ── Socket table ────────────────────────────────────────────────────────

/// Global socket subsystem.
pub struct SocketTable {
    socks: [Socket; MAX_SOCKETS],
}

impl SocketTable {
    pub const fn new() -> Self {
        Self {
            socks: [
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
                Socket::empty(),
            ],
        }
    }

    /// Read-only view of all socket slots.
    pub fn socks_ref(&self) -> &[Socket; MAX_SOCKETS] {
        &self.socks
    }

    // ── SYS_SOCKET ──────────────────────────────────────────────────

    /// Create a socket. Returns handle or `usize::MAX`.
    pub fn create(&mut self, domain: usize, sock_type: usize) -> usize {
        let dom = match domain {
            0 => Domain::Local,
            1 => Domain::Inet,
            _ => return usize::MAX,
        };
        let st = match sock_type {
            0 => SockType::Stream,
            1 => SockType::Dgram,
            _ => return usize::MAX,
        };
        for (i, s) in self.socks.iter_mut().enumerate() {
            if s.state == SockState::Free {
                *s = Socket::empty();
                s.state = SockState::Created;
                s.domain = dom;
                s.sock_type = st;
                return i;
            }
        }
        usize::MAX // no free slots
    }

    // ── SYS_BIND ────────────────────────────────────────────────────

    /// Bind socket to an address. Returns 0 on success.
    pub fn bind(&mut self, handle: usize, addr: usize) -> usize {
        if handle >= MAX_SOCKETS {
            return usize::MAX;
        }
        if self.socks[handle].state != SockState::Created {
            return usize::MAX;
        }
        let domain = self.socks[handle].domain;
        // Check no other socket already bound to this address+domain.
        for (i, other) in self.socks.iter().enumerate() {
            if i != handle
                && other.domain == domain
                && other.bound_addr == addr
                && other.state != SockState::Free
                && other.state != SockState::Closed
            {
                return usize::MAX; // address in use
            }
        }
        self.socks[handle].bound_addr = addr;
        self.socks[handle].state = SockState::Bound;
        0
    }

    // ── SYS_LISTEN ──────────────────────────────────────────────────

    /// Listen for connections. Returns 0 on success.
    pub fn listen(&mut self, handle: usize, _backlog: usize) -> usize {
        if handle >= MAX_SOCKETS {
            return usize::MAX;
        }
        let s = &mut self.socks[handle];
        if s.state != SockState::Bound || s.sock_type != SockType::Stream {
            return usize::MAX;
        }
        s.state = SockState::Listening;
        0
    }

    // ── SYS_ACCEPT ──────────────────────────────────────────────────

    /// Accept a connection. Returns new socket handle or `None` if no
    /// pending connections (caller should block).
    pub fn accept(&mut self, handle: usize) -> Option<usize> {
        if handle >= MAX_SOCKETS || self.socks[handle].state != SockState::Listening {
            return None;
        }
        if self.socks[handle].pending_conn_count == 0 {
            return None; // no pending — caller must block
        }
        // Dequeue first pending connection.
        let client_handle = self.socks[handle].pending_conn[0];
        let count = self.socks[handle].pending_conn_count;
        // Shift queue.
        for i in 0..count - 1 {
            self.socks[handle].pending_conn[i] = self.socks[handle].pending_conn[i + 1];
        }
        self.socks[handle].pending_conn[count - 1] = usize::MAX;
        self.socks[handle].pending_conn_count -= 1;

        // Allocate a server-side connected socket.
        let listener_domain = self.socks[handle].domain;
        let mut server_handle = usize::MAX;
        for (i, s) in self.socks.iter_mut().enumerate() {
            if s.state == SockState::Free {
                *s = Socket::empty();
                s.state = SockState::Connected;
                s.domain = listener_domain;
                s.sock_type = SockType::Stream;
                s.peer = client_handle;
                server_handle = i;
                break;
            }
        }

        if server_handle == usize::MAX {
            return None; // no free socket slots
        }

        // Complete client connection.
        if client_handle < MAX_SOCKETS {
            self.socks[client_handle].peer = server_handle;
            self.socks[client_handle].state = SockState::Connected;
        }

        Some(server_handle)
    }

    // ── SYS_CONNECT ─────────────────────────────────────────────────

    /// Connect to a listening socket. For local sockets, addr is the port
    /// number the server bound to. Returns 0 on success (queued), `usize::MAX` on error.
    pub fn connect(&mut self, sched: &mut Scheduler, handle: usize, addr: usize) -> usize {
        if handle >= MAX_SOCKETS || self.socks[handle].state != SockState::Created {
            return usize::MAX;
        }
        let domain = self.socks[handle].domain;

        // Find a listening socket on this address+domain.
        let mut listener = usize::MAX;
        for (i, s) in self.socks.iter().enumerate() {
            if s.state == SockState::Listening && s.domain == domain && s.bound_addr == addr {
                listener = i;
                break;
            }
        }
        if listener == usize::MAX {
            return usize::MAX; // no listener
        }

        // Enqueue this socket in the listener's pending queue.
        let ls = &mut self.socks[listener];
        if ls.pending_conn_count >= ls.pending_conn.len() {
            return usize::MAX; // backlog full
        }
        ls.pending_conn[ls.pending_conn_count] = handle;
        ls.pending_conn_count += 1;

        // Wake any task blocked on accept for this listener.
        for task in sched.tasks.iter_mut() {
            if task.state == TaskState::Blocked
                && task.block_reason == BlockReason::SockAccept(listener)
            {
                task.state = TaskState::Ready;
                task.block_reason = BlockReason::None;
                break;
            }
        }

        0
    }

    // ── SYS_SOCK_SEND ───────────────────────────────────────────────

    /// Write bytes into the peer's RX buffer. Returns number of bytes
    /// written, or `None` if peer has no space (caller should block).
    pub fn send_bytes(
        &mut self,
        sched: &mut Scheduler,
        handle: usize,
        data: &[u8],
    ) -> Option<usize> {
        if handle >= MAX_SOCKETS || self.socks[handle].state != SockState::Connected {
            return Some(usize::MAX); // error
        }
        let peer = self.socks[handle].peer;
        if peer >= MAX_SOCKETS || self.socks[peer].state != SockState::Connected {
            return Some(usize::MAX); // peer closed
        }
        if self.socks[peer].rx.is_full() {
            return None; // no space — caller should block
        }
        let n = self.socks[peer].rx.write(data);

        // Wake any task blocked on recv from peer.
        for task in sched.tasks.iter_mut() {
            if task.state == TaskState::Blocked && task.block_reason == BlockReason::SockRecv(peer)
            {
                task.state = TaskState::Ready;
                task.block_reason = BlockReason::None;
                break;
            }
        }
        Some(n)
    }

    // ── SYS_SOCK_RECV ───────────────────────────────────────────────

    /// Read bytes from this socket's RX buffer. Returns number of bytes
    /// read, or `None` if empty (caller should block).
    pub fn recv_bytes(&mut self, handle: usize, out: &mut [u8]) -> Option<usize> {
        if handle >= MAX_SOCKETS || self.socks[handle].state != SockState::Connected {
            return Some(usize::MAX);
        }
        if self.socks[handle].rx.is_empty() {
            // Check if peer is closed → EOF.
            let peer = self.socks[handle].peer;
            if peer >= MAX_SOCKETS
                || self.socks[peer].state == SockState::Closed
                || self.socks[peer].state == SockState::Free
            {
                return Some(0); // EOF
            }
            return None; // empty — caller should block
        }
        Some(self.socks[handle].rx.read(out))
    }

    // ── SYS_SOCK_CLOSE ─────────────────────────────────────────────

    /// Close a socket, waking any blocked peer.
    pub fn close(&mut self, sched: &mut Scheduler, handle: usize) -> usize {
        if handle >= MAX_SOCKETS || self.socks[handle].state == SockState::Free {
            return usize::MAX;
        }
        let peer = self.socks[handle].peer;
        self.socks[handle].state = SockState::Closed;

        // Wake tasks blocked on the peer (recv will see EOF).
        if peer < MAX_SOCKETS {
            for task in sched.tasks.iter_mut() {
                if task.state == TaskState::Blocked {
                    match task.block_reason {
                        BlockReason::SockRecv(h) if h == peer => {
                            task.state = TaskState::Ready;
                            task.block_reason = BlockReason::None;
                        }
                        BlockReason::SockSend(h) if h == handle => {
                            task.state = TaskState::Ready;
                            task.block_reason = BlockReason::None;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Wake tasks blocked on accept if this was a listener.
        for task in sched.tasks.iter_mut() {
            if task.state == TaskState::Blocked
                && task.block_reason == BlockReason::SockAccept(handle)
            {
                task.state = TaskState::Ready;
                task.block_reason = BlockReason::None;
            }
        }

        // Free the slot for reuse.
        self.socks[handle] = Socket::empty();
        0
    }
}

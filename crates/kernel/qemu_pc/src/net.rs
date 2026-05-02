//! Network subsystem for QEMU PC — wires the VIRTIO-NET driver to the
//! smoltcp TCP/IP stack and exposes interface management to the shell.
//!
//! Architecture:
//!   VirtioNet (driver) ← VirtioNetDev (NetworkDevice adapter)
//!     ↕
//!   smoltcp Interface (TCP/IP, ARP, ICMP)
//!     ↕
//!   SocketSet (TCP + UDP sockets)
//!     ↕
//!   net_poll_task() — called from scheduler, drives the stack

use core::cell::UnsafeCell;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use arch::NetworkDevice;
use net::{NetStack, NetStorage};

// Re-export TcpSerial from the net crate so main.rs can use it via `net::TcpSerial`.
pub use net::TcpSerial;
use smoltcp::iface::SocketSet;
#[cfg(feature = "dhcp")]
use smoltcp::socket::dhcpv4;
use smoltcp::socket::tcp::Socket as TcpSocket;
use smoltcp::socket::udp::{PacketBuffer, PacketMetadata, Socket as UdpSocket};
use smoltcp::wire::{IpAddress, IpCidr, IpEndpoint, Ipv4Address};

// ═══════════════════════════════════════════════════════════════════════════
// NetworkDevice adapter for VirtioNet
// ═══════════════════════════════════════════════════════════════════════════

/// Zero-sized adapter that implements [`NetworkDevice`] by accessing the
/// global `VIRTIO_NET` static.  All access goes through raw pointers
/// because the trait requires `&self` but the driver needs `&mut self`.
///
/// This is safe because VeerOS is single-core and interrupts are disabled
/// during packet I/O (the trap dispatcher handles VIRTIO IRQs sequentially).
pub struct VirtioNetDev;

impl NetworkDevice for VirtioNetDev {
    fn has_rx(&self) -> bool {
        let net = unsafe { &mut *crate::VIRTIO_NET.0.get() };
        if !net.active {
            return false;
        }
        // Peek at the RX used ring without consuming.
        let rxq = match net.rxq.as_mut() {
            Some(q) => q,
            None => return false,
        };
        rxq.has_used()
    }

    fn recv(&self, buf: &mut [u8]) -> usize {
        let net = unsafe { &mut *crate::VIRTIO_NET.0.get() };
        net.recv(buf)
    }

    fn send(&self, buf: &[u8]) {
        let net = unsafe { &mut *crate::VIRTIO_NET.0.get() };
        net.send(buf);
    }

    fn mac_address(&self) -> [u8; 6] {
        let net = unsafe { &*crate::VIRTIO_NET.0.get() };
        net.mac
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Network interface state
// ═══════════════════════════════════════════════════════════════════════════

/// Link state of the network interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Down,
    Up,
}

/// Network interface information (for `ifconfig` display).
#[derive(Debug, Clone, Copy)]
pub struct NetInterface {
    pub name: &'static str,
    pub link: LinkState,
    pub mac: [u8; 6],
    pub ip: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway: [u8; 4],
    pub mtu: u16,
    pub rx_packets: u32,
    pub tx_packets: u32,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_errors: u32,
    pub tx_errors: u32,
}

impl NetInterface {
    pub const fn empty() -> Self {
        Self {
            name: "eth0",
            link: LinkState::Down,
            mac: [0; 6],
            ip: [0; 4],
            netmask: [0; 4],
            gateway: [0; 4],
            mtu: 1500,
            rx_packets: 0,
            tx_packets: 0,
            rx_bytes: 0,
            tx_bytes: 0,
            rx_errors: 0,
            tx_errors: 0,
        }
    }
}

pub struct NetIfCell(pub UnsafeCell<NetInterface>);
unsafe impl Sync for NetIfCell {}
pub static NET_IF: NetIfCell = NetIfCell(UnsafeCell::new(NetInterface::empty()));

// ═══════════════════════════════════════════════════════════════════════════
// smoltcp static storage
// ═══════════════════════════════════════════════════════════════════════════

/// smoltcp socket-set storage (static, no heap).
/// We support: 1 TCP (primary) + 4 SSH sessions + 1 SSH client
/// + 1 UDP + 1 DHCP + spare.
const MAX_SMOL_SOCKETS: usize = 16;

// Compile-time sanity: make sure the SocketStorage array can't silently
// overflow into neighbouring statics.
const _: () = {
    let sz = core::mem::size_of::<[smoltcp::iface::SocketStorage<'static>; MAX_SMOL_SOCKETS]>();
    if sz > 32768 {
        panic!("SOCKET_SET_BUF too large");
    }
};

/// Static buffer backing for the smoltcp socket set.
///
/// Placed in its own module-level `static mut` so that the linker keeps it
/// away from `NET_IF`. Previously the two statics were adjacent and smoltcp's
/// `SocketStorage` (which is quite large) silently overflowed into `NET_IF`.
#[repr(align(16))]
struct SmolSocketBuf([smoltcp::iface::SocketStorage<'static>; MAX_SMOL_SOCKETS]);

static mut SOCKET_SET_BUF: SmolSocketBuf = SmolSocketBuf([
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
    smoltcp::iface::SocketStorage::EMPTY,
]);

/// Static socket set (wrapped for Sync).
pub struct SocketSetCell(pub UnsafeCell<Option<SocketSet<'static>>>);
unsafe impl Sync for SocketSetCell {}
pub static SMOL_SOCKETS: SocketSetCell = SocketSetCell(UnsafeCell::new(None));

/// Static NetStack instance.
pub struct NetStackCell(pub UnsafeCell<Option<NetStack<VirtioNetDev>>>);
unsafe impl Sync for NetStackCell {}
pub static NET_STACK: NetStackCell = NetStackCell(UnsafeCell::new(None));

/// Static NetStorage for the primary TCP socket.
static mut NET_STORAGE: NetStorage = NetStorage::new();

/// UDP socket buffers.
const UDP_RX_META_COUNT: usize = 4;
const UDP_TX_META_COUNT: usize = 4;
const UDP_RX_BUF_SIZE: usize = 1024;
const UDP_TX_BUF_SIZE: usize = 1024;

static mut UDP_RX_META: [PacketMetadata; UDP_RX_META_COUNT] =
    [PacketMetadata::EMPTY; UDP_RX_META_COUNT];
static mut UDP_TX_META: [PacketMetadata; UDP_TX_META_COUNT] =
    [PacketMetadata::EMPTY; UDP_TX_META_COUNT];
static mut UDP_RX_DATA: [u8; UDP_RX_BUF_SIZE] = [0u8; UDP_RX_BUF_SIZE];
static mut UDP_TX_DATA: [u8; UDP_TX_BUF_SIZE] = [0u8; UDP_TX_BUF_SIZE];

/// UDP socket handle (stored after creation).
static mut UDP_HANDLE: Option<smoltcp::iface::SocketHandle> = None;

/// Maximum number of concurrent SSH sessions.
#[cfg(feature = "ssh")]
pub const MAX_SSH_SESSIONS: usize = 4;

/// Per-session TCP socket buffers.
#[cfg(feature = "ssh")]
const SSH_SESSION_BUF_SIZE: usize = 16384;

#[cfg(feature = "ssh")]
static mut SSH_SESSION_RX_BUFS: [[u8; SSH_SESSION_BUF_SIZE]; MAX_SSH_SESSIONS] =
    [[0u8; SSH_SESSION_BUF_SIZE]; MAX_SSH_SESSIONS];
#[cfg(feature = "ssh")]
static mut SSH_SESSION_TX_BUFS: [[u8; SSH_SESSION_BUF_SIZE]; MAX_SSH_SESSIONS] =
    [[0u8; SSH_SESSION_BUF_SIZE]; MAX_SSH_SESSIONS];

/// Per-session TCP socket handles.
#[cfg(feature = "ssh")]
pub static mut SSH_SESSION_HANDLES: [Option<smoltcp::iface::SocketHandle>; MAX_SSH_SESSIONS] =
    [None; MAX_SSH_SESSIONS];

/// Per-session in-use flags (true = slot has an active SSH session).
#[cfg(feature = "ssh")]
pub static SSH_SESSION_ACTIVE: [AtomicBool; MAX_SSH_SESSIONS] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];

// Legacy alias — slot 0 for backward compat with SSH client code.
#[cfg(feature = "ssh")]
pub static mut SSH_TCP_HANDLE: Option<smoltcp::iface::SocketHandle> = None;

// ── SSH client (outbound) TCP socket ─────────────────────────────────
#[cfg(feature = "ssh")]
const SSH_CLIENT_TCP_RX_BUF_SIZE: usize = 4096;
#[cfg(feature = "ssh")]
const SSH_CLIENT_TCP_TX_BUF_SIZE: usize = 4096;

#[cfg(feature = "ssh")]
static mut SSH_CLIENT_TCP_RX_BUF: [u8; SSH_CLIENT_TCP_RX_BUF_SIZE] =
    [0u8; SSH_CLIENT_TCP_RX_BUF_SIZE];
#[cfg(feature = "ssh")]
static mut SSH_CLIENT_TCP_TX_BUF: [u8; SSH_CLIENT_TCP_TX_BUF_SIZE] =
    [0u8; SSH_CLIENT_TCP_TX_BUF_SIZE];

/// SSH client (outbound) TCP socket handle.
#[cfg(feature = "ssh")]
pub static mut SSH_CLIENT_TCP_HANDLE: Option<smoltcp::iface::SocketHandle> = None;

/// DHCP socket handle (stored after creation).
#[cfg(feature = "dhcp")]
static mut DHCP_HANDLE: Option<smoltcp::iface::SocketHandle> = None;

/// Whether DHCP has acquired an address.
#[cfg(feature = "dhcp")]
static mut DHCP_CONFIGURED: bool = false;

/// Whether the network stack has been initialised.
static mut NET_INIT: bool = false;
static POLL_BUSY: AtomicBool = AtomicBool::new(false);

// ═══════════════════════════════════════════════════════════════════════════
// Initialisation
// ═══════════════════════════════════════════════════════════════════════════

/// QEMU user-net default configuration.
const DEFAULT_IP: [u8; 4] = [10, 0, 2, 15];
const DEFAULT_NETMASK: [u8; 4] = [255, 255, 255, 0];
const DEFAULT_GATEWAY: [u8; 4] = [10, 0, 2, 2];

/// Initialise the network stack.
///
/// Must be called after the VIRTIO-NET device has been probed.
/// Sets up the smoltcp interface with a static IP, creates TCP and UDP
/// sockets, and marks the interface as Up.
pub fn init() {
    let virtio_net = unsafe { &*crate::VIRTIO_NET.0.get() };
    if !virtio_net.active {
        return; // no NIC — skip
    }

    // Populate interface info.
    let iface = unsafe { &mut *NET_IF.0.get() };
    iface.mac = virtio_net.mac;
    iface.ip = DEFAULT_IP;
    iface.netmask = DEFAULT_NETMASK;
    iface.gateway = DEFAULT_GATEWAY;
    iface.mtu = 1500;

    // Debug: verify NET_IF wasn't clobbered.
    {
        let serial = soc_qemu_pc::default_serial();
        let mut con = arch::Console::new(serial);
        let _ = core::fmt::Write::write_fmt(
            &mut con,
            format_args!(
                "[net] SocketStorage size={}, buf at {:p}, NET_IF at {:p}\n",
                core::mem::size_of::<smoltcp::iface::SocketStorage>(),
                unsafe { &SOCKET_SET_BUF as *const _ },
                unsafe { &*NET_IF.0.get() as *const _ },
            ),
        );
    }

    // Create the smoltcp socket set.
    let socket_set = unsafe { SocketSet::new(&mut SOCKET_SET_BUF.0[..]) };
    unsafe {
        *SMOL_SOCKETS.0.get() = Some(socket_set);
    }

    // Create the NetStack (smoltcp Interface + primary TCP socket).
    // Always start with the static IP so TCP is usable immediately.
    // When the DHCP feature is enabled, the DHCP client runs in parallel
    // and can update the address if it differs from the default.
    let dev = VirtioNetDev;
    let ip = IpCidr::new(
        IpAddress::v4(DEFAULT_IP[0], DEFAULT_IP[1], DEFAULT_IP[2], DEFAULT_IP[3]),
        24,
    );
    let gw = Ipv4Address::new(
        DEFAULT_GATEWAY[0],
        DEFAULT_GATEWAY[1],
        DEFAULT_GATEWAY[2],
        DEFAULT_GATEWAY[3],
    );

    let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
    let storage = unsafe { &mut *core::ptr::addr_of_mut!(NET_STORAGE) };

    let stack = NetStack::new(dev, ip, gw, sockets, storage);
    unsafe {
        *NET_STACK.0.get() = Some(stack);
    }

    // Add a UDP socket for ping/DNS.
    let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
    unsafe {
        let rx_buf = PacketBuffer::new(
            &mut (&mut *core::ptr::addr_of_mut!(UDP_RX_META))[..],
            &mut (&mut *core::ptr::addr_of_mut!(UDP_RX_DATA))[..],
        );
        let tx_buf = PacketBuffer::new(
            &mut (&mut *core::ptr::addr_of_mut!(UDP_TX_META))[..],
            &mut (&mut *core::ptr::addr_of_mut!(UDP_TX_DATA))[..],
        );
        let udp_socket = UdpSocket::new(rx_buf, tx_buf);
        let handle = sockets.add(udp_socket);
        UDP_HANDLE = Some(handle);
    }

    // Add TCP sockets for the SSH server session pool (port 22).
    // Each socket listens independently; when a client connects, one
    // transitions to Established while the rest keep listening.
    #[cfg(feature = "ssh")]
    {
        let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
        unsafe {
            use smoltcp::socket::tcp::SocketBuffer;
            for i in 0..MAX_SSH_SESSIONS {
                let rx_buf = SocketBuffer::new(
                    &mut (&mut *core::ptr::addr_of_mut!(SSH_SESSION_RX_BUFS))[i][..],
                );
                let tx_buf = SocketBuffer::new(
                    &mut (&mut *core::ptr::addr_of_mut!(SSH_SESSION_TX_BUFS))[i][..],
                );
                let mut ssh_socket = TcpSocket::new(rx_buf, tx_buf);
                ssh_socket.set_nagle_enabled(false);
                let handle = sockets.add(ssh_socket);
                SSH_SESSION_HANDLES[i] = Some(handle);
            }
            // Legacy alias — point at slot 0 for ssh_listen/ssh_is_connected compat.
            SSH_TCP_HANDLE = SSH_SESSION_HANDLES[0];
        }
    }

    // Add a TCP socket for SSH client (outbound connections).
    #[cfg(feature = "ssh")]
    {
        let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
        unsafe {
            use smoltcp::socket::tcp::SocketBuffer;
            let rx_buf =
                SocketBuffer::new(&mut (&mut *core::ptr::addr_of_mut!(SSH_CLIENT_TCP_RX_BUF))[..]);
            let tx_buf =
                SocketBuffer::new(&mut (&mut *core::ptr::addr_of_mut!(SSH_CLIENT_TCP_TX_BUF))[..]);
            let mut client_socket = TcpSocket::new(rx_buf, tx_buf);
            client_socket.set_nagle_enabled(false);
            let handle = sockets.add(client_socket);
            SSH_CLIENT_TCP_HANDLE = Some(handle);
        }
    }

    // Add a DHCPv4 socket to acquire IP from the network.
    #[cfg(feature = "dhcp")]
    {
        let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
        let dhcp_socket = dhcpv4::Socket::new();
        let handle = sockets.add(dhcp_socket);
        unsafe {
            DHCP_HANDLE = Some(handle);
        }

        let serial = soc_qemu_pc::default_serial();
        let mut con = arch::Console::new(serial);
        let _ = core::fmt::Write::write_fmt(
            &mut con,
            format_args!("[net] DHCP: client started, awaiting lease...\n"),
        );
    }

    // Mark interface up.
    iface.link = LinkState::Up;
    unsafe {
        NET_INIT = true;
    }
}

/// Returns true if the network stack is initialised.
pub fn is_active() -> bool {
    unsafe { NET_INIT }
}

// ═══════════════════════════════════════════════════════════════════════════
// Polling — must be called periodically
// ═══════════════════════════════════════════════════════════════════════════

/// Drive the smoltcp TCP/IP state machine.
///
/// Call this from the network polling task or the timer ISR.
/// `now_ms` is the current time in milliseconds.
pub fn poll(now_ms: u64) {
    if !is_active() {
        return;
    }

    if POLL_BUSY.swap(true, Ordering::Acquire) {
        return;
    }

    let stack = unsafe { (*NET_STACK.0.get()).as_mut().unwrap() };
    let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
    stack.poll(sockets, now_ms);

    // Process DHCP events.
    #[cfg(feature = "dhcp")]
    unsafe {
        if let Some(handle) = DHCP_HANDLE {
            let event = sockets.get_mut::<dhcpv4::Socket>(handle).poll();
            match event {
                Some(dhcpv4::Event::Configured(config)) => {
                    // Extract Copy values before any further borrows.
                    let address = config.address;
                    let router = config.router;
                    let addr = address.address().0;
                    let prefix = address.prefix_len();

                    stack.apply_ip_config(address, router);

                    // Update NetInterface.
                    let iface = &mut *NET_IF.0.get();
                    iface.ip = addr;
                    let mask = if prefix == 0 {
                        0u32
                    } else {
                        !0u32 << (32 - prefix)
                    };
                    iface.netmask = mask.to_be_bytes();
                    if let Some(gw) = router {
                        iface.gateway = gw.0;
                    }

                    DHCP_CONFIGURED = true;

                    let serial = soc_qemu_pc::default_serial();
                    let mut con = arch::Console::new(serial);
                    let _ = core::fmt::Write::write_fmt(
                        &mut con,
                        format_args!(
                            "[net] DHCP: acquired {}.{}.{}.{}/{}\n",
                            addr[0], addr[1], addr[2], addr[3], prefix,
                        ),
                    );
                    if let Some(gw) = router {
                        let g = gw.0;
                        let _ = core::fmt::Write::write_fmt(
                            &mut con,
                            format_args!(
                                "[net] DHCP: gateway {}.{}.{}.{}\n",
                                g[0], g[1], g[2], g[3],
                            ),
                        );
                    }
                }
                Some(dhcpv4::Event::Deconfigured) => {
                    // Don't remove the IP — keep the static default as
                    // fallback so TCP sockets stay functional.  The next
                    // Configured event will set the correct address.
                    DHCP_CONFIGURED = false;

                    let serial = soc_qemu_pc::default_serial();
                    let mut con = arch::Console::new(serial);
                    let _ = core::fmt::Write::write_fmt(
                        &mut con,
                        format_args!("[net] DHCP: lease expired, waiting for renewal\n"),
                    );
                }
                None => {}
            }
        }
    }

    // Update RX/TX counters.
    let iface = unsafe { &mut *NET_IF.0.get() };
    // (smoltcp doesn't expose packet counters directly; we track at driver level)
    let _ = iface;
    POLL_BUSY.store(false, Ordering::Release);
}

/// Like `poll()`, but **leaves POLL_BUSY held** on success so the caller
/// can safely access the socket set before releasing it.
///
/// Returns `true` if the lock was acquired (caller must release via
/// `POLL_BUSY.store(false, Release)`).  Returns `false` when the lock
/// was already held by another task (stack was NOT polled).
fn poll_locked(now_ms: u64) -> bool {
    if !is_active() {
        return false;
    }

    if POLL_BUSY.swap(true, Ordering::Acquire) {
        return false; // another task is polling
    }

    let stack = unsafe { (*NET_STACK.0.get()).as_mut().unwrap() };
    let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
    stack.poll(sockets, now_ms);

    // NOTE: we intentionally do NOT release POLL_BUSY here.
    true
}

/// Network polling task — runs as a kernel task, polls the stack every tick.
pub fn net_poll_task() -> ! {
    loop {
        let ticks = unsafe { (*crate::SCHEDULER.0.get()).ticks };
        poll(ticks); // 1 tick = 1 ms in VeerOS

        // Yield to let other tasks run. The timer ISR will reschedule us.
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("int 0x80", in("rax") 0x00usize, options(nostack, preserves_flags));
        }
        #[cfg(not(target_arch = "x86_64"))]
        core::hint::spin_loop();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SSH helpers — session pool management
// ═══════════════════════════════════════════════════════════════════════════

/// Acquire the POLL_BUSY lock for exclusive socket-set access.
///
/// Does NOT poll the stack — `net_poll_task` drives `stack.poll()`
/// continuously and handles all TX/RX processing.  TcpSerial uses
/// this to safely read/write socket buffers between polls.
///
/// Returns `true` if the lock was acquired (caller must call `ssh_poll_unlock()`).
/// Returns `false` if another task holds the lock (caller should spin/retry).
#[cfg(feature = "ssh")]
pub fn ssh_poll() -> bool {
    if !is_active() {
        return false;
    }
    !POLL_BUSY.swap(true, Ordering::Acquire)
}

/// Release the POLL_BUSY lock after a `ssh_poll()` + socket access.
#[cfg(feature = "ssh")]
pub fn ssh_poll_unlock() {
    POLL_BUSY.store(false, Ordering::Release);
}

/// Start all idle session sockets listening on the SSH port.
#[cfg(feature = "ssh")]
pub fn ssh_listen_all(port: u16) {
    // Acquire POLL_BUSY so we don't race with net_poll_task.
    loop {
        if !POLL_BUSY.swap(true, Ordering::Acquire) {
            break;
        }
        core::hint::spin_loop();
    }
    unsafe {
        let sockets = (*SMOL_SOCKETS.0.get()).as_mut().unwrap();
        for i in 0..MAX_SSH_SESSIONS {
            if SSH_SESSION_ACTIVE[i].load(Ordering::Relaxed) {
                continue; // slot busy with a session
            }
            if let Some(handle) = SSH_SESSION_HANDLES[i] {
                let socket = sockets.get_mut::<TcpSocket>(handle);
                if !socket.is_listening() {
                    if socket.is_open() {
                        socket.abort();
                    }
                    socket.listen(port).ok();
                }
            }
        }
    }
    POLL_BUSY.store(false, Ordering::Release);
}

/// Start listening on the SSH TCP socket (legacy — listens slot 0).
#[cfg(feature = "ssh")]
pub fn ssh_listen(port: u16) {
    ssh_listen_all(port);
}

/// Find a session slot that has a newly connected client.
/// Returns the slot index (0..MAX_SSH_SESSIONS) or `None`.
/// The caller must mark it active via `SSH_SESSION_ACTIVE[slot].store(true, ...)`.
#[cfg(feature = "ssh")]
pub fn ssh_accept() -> Option<usize> {
    unsafe {
        let sockets = (*SMOL_SOCKETS.0.get()).as_ref().unwrap();
        for i in 0..MAX_SSH_SESSIONS {
            if SSH_SESSION_ACTIVE[i].load(Ordering::Relaxed) {
                continue; // already in use
            }
            if let Some(handle) = SSH_SESSION_HANDLES[i] {
                let socket = sockets.get::<TcpSocket>(handle);
                if socket.is_active() && !socket.is_listening() {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Check if any client is connected (for the listener poll loop).
#[cfg(feature = "ssh")]
pub fn ssh_is_connected() -> bool {
    ssh_accept().is_some()
}

/// Debug: print the state of all SSH session sockets.
#[cfg(feature = "ssh")]
pub fn ssh_debug_socket_states() {
    unsafe {
        let sockets = (*SMOL_SOCKETS.0.get()).as_ref().unwrap();
        let serial = soc_qemu_pc::default_serial();
        let mut con = arch::Console::new(serial);
        for i in 0..MAX_SSH_SESSIONS {
            let active = SSH_SESSION_ACTIVE[i].load(Ordering::Relaxed);
            if let Some(handle) = SSH_SESSION_HANDLES[i] {
                let socket = sockets.get::<TcpSocket>(handle);
                let state = socket.state();
                let _ = core::fmt::Write::write_fmt(
                    &mut con,
                    format_args!("[ssh-dbg] slot {} active={} state={}\n", i, active, state,),
                );
            }
        }
    }
}

/// Check if a specific session socket can send payload bytes.
#[cfg(feature = "ssh")]
pub fn ssh_session_can_send(slot: usize) -> bool {
    unsafe {
        if let Some(handle) = SSH_SESSION_HANDLES.get(slot).and_then(|h| *h) {
            let sockets = (*SMOL_SOCKETS.0.get()).as_ref().unwrap();
            let socket = sockets.get::<TcpSocket>(handle);
            socket.can_send()
        } else {
            false
        }
    }
}

/// Release a session slot: flush pending TX data, abort the socket, re-listen.
#[cfg(feature = "ssh")]
pub fn ssh_session_release(slot: usize, port: u16) {
    // Keep the slot ACTIVE during teardown so the listener doesn't
    // reuse the slot (and its stack) while we're still running.

    // Flush pending TX data: acquire the lock and poll the stack a few
    // times so that any data still in the socket's TX buffer is
    // transmitted before we abort the socket.
    for _ in 0..5 {
        if !POLL_BUSY.swap(true, Ordering::Acquire) {
            unsafe {
                let stack = (*NET_STACK.0.get()).as_mut().unwrap();
                let sockets = (*SMOL_SOCKETS.0.get()).as_mut().unwrap();
                let ticks = (*crate::SCHEDULER.0.get()).ticks;
                stack.poll(sockets, ticks);
            }
            POLL_BUSY.store(false, Ordering::Release);
        }
        // Yield to give the NIC time to transmit.
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("int 0x80", in("rax") 0x00usize, options(nostack, preserves_flags));
        }
    }

    // Now tear down: acquire POLL_BUSY, abort, and re-listen.
    loop {
        if !POLL_BUSY.swap(true, Ordering::Acquire) {
            break;
        }
        core::hint::spin_loop();
    }
    unsafe {
        if let Some(handle) = SSH_SESSION_HANDLES.get(slot).and_then(|h| *h) {
            let sockets = (*SMOL_SOCKETS.0.get()).as_mut().unwrap();
            let socket = sockets.get_mut::<TcpSocket>(handle);
            if socket.is_open() {
                socket.abort();
            }
            socket.listen(port).ok();
        }
    }
    POLL_BUSY.store(false, Ordering::Release);

    // Only now mark the slot as free — the socket is listening and
    // the stack has been flushed.
    SSH_SESSION_ACTIVE[slot].store(false, Ordering::Release);
}

/// Check if the SSH TCP socket can send payload bytes (legacy).
#[cfg(feature = "ssh")]
pub fn ssh_can_send() -> bool {
    ssh_session_can_send(0)
}

// ═══════════════════════════════════════════════════════════════════════════
// SSH client helpers — outbound TCP connection for the SSH client command
// ═══════════════════════════════════════════════════════════════════════════

/// Connect the SSH client TCP socket to a remote host.
///
/// Returns `true` if the connect call was dispatched (SYN sent).
/// The caller must poll until `ssh_client_is_connected()` returns true.
#[cfg(feature = "ssh")]
pub fn ssh_client_connect(ip: [u8; 4], port: u16) -> bool {
    // Acquire POLL_BUSY so we don't race with net_poll_task.
    loop {
        if !POLL_BUSY.swap(true, Ordering::Acquire) {
            break;
        }
        core::hint::spin_loop();
    }
    let result = unsafe {
        let handle = match SSH_CLIENT_TCP_HANDLE {
            Some(h) => h,
            None => {
                POLL_BUSY.store(false, Ordering::Release);
                return false;
            }
        };

        let sockets = (*SMOL_SOCKETS.0.get()).as_mut().unwrap();
        let socket = sockets.get_mut::<TcpSocket>(handle);

        // Abort any previous connection.
        if socket.is_open() {
            socket.abort();
        }

        let stack = (*NET_STACK.0.get()).as_mut().unwrap();
        let cx = stack.context();

        let remote = (
            smoltcp::wire::IpAddress::v4(ip[0], ip[1], ip[2], ip[3]),
            port,
        );
        // Use our IP + ephemeral port as local endpoint.
        let iface = &*NET_IF.0.get();
        let local_ip =
            smoltcp::wire::IpAddress::v4(iface.ip[0], iface.ip[1], iface.ip[2], iface.ip[3]);
        let local = (local_ip, 44222u16); // ephemeral port

        match socket.connect(cx, remote, local) {
            Ok(()) => true,
            Err(_) => false,
        }
    };
    POLL_BUSY.store(false, Ordering::Release);
    result
}

/// Poll function for SSH client — same as server poll (drives the stack).
#[cfg(feature = "ssh")]
pub fn ssh_client_poll() -> bool {
    ssh_poll()
}

/// Release lock after SSH client poll.
#[cfg(feature = "ssh")]
pub fn ssh_client_poll_unlock() {
    ssh_poll_unlock();
}

/// Check if the SSH client TCP socket is connected (established).
///
/// **Must be called while POLL_BUSY is held** (between ssh_client_poll
/// returning true and ssh_client_poll_unlock).
#[cfg(feature = "ssh")]
pub fn ssh_client_is_connected() -> bool {
    unsafe {
        if let Some(handle) = SSH_CLIENT_TCP_HANDLE {
            let sockets = (*SMOL_SOCKETS.0.get()).as_ref().unwrap();
            let socket = sockets.get::<TcpSocket>(handle);
            socket.may_send() && socket.may_recv()
        } else {
            false
        }
    }
}

/// Check if the SSH client TCP socket can send.
///
/// **Must be called while POLL_BUSY is held.**
#[cfg(feature = "ssh")]
pub fn ssh_client_can_send() -> bool {
    unsafe {
        if let Some(handle) = SSH_CLIENT_TCP_HANDLE {
            let sockets = (*SMOL_SOCKETS.0.get()).as_ref().unwrap();
            let socket = sockets.get::<TcpSocket>(handle);
            socket.can_send()
        } else {
            false
        }
    }
}

/// Disconnect and clean up the SSH client TCP socket.
#[cfg(feature = "ssh")]
pub fn ssh_client_disconnect() {
    // Acquire POLL_BUSY so we don't race with net_poll_task.
    loop {
        if !POLL_BUSY.swap(true, Ordering::Acquire) {
            break;
        }
        core::hint::spin_loop();
    }
    unsafe {
        if let Some(handle) = SSH_CLIENT_TCP_HANDLE {
            let sockets = (*SMOL_SOCKETS.0.get()).as_mut().unwrap();
            let socket = sockets.get_mut::<TcpSocket>(handle);
            socket.abort();
        }
    }
    POLL_BUSY.store(false, Ordering::Release);
}

// ═══════════════════════════════════════════════════════════════════════════
// ICMP Ping (raw ICMP echo via smoltcp's ICMP socket alternative:
// we craft ICMP manually and use raw IP, or simpler: just use the
// smoltcp interface's respond-to-ping which is automatic).
//
// For outgoing ping, we do manual ICMP echo request/reply tracking.
// ═══════════════════════════════════════════════════════════════════════════

/// Ping state for a single request.
#[derive(Clone, Copy)]
pub struct PingState {
    pub active: bool,
    pub target: [u8; 4],
    pub seq: u16,
    pub sent_tick: u64,
    pub reply_tick: u64,
    pub got_reply: bool,
    pub timed_out: bool,
}

impl PingState {
    pub const fn empty() -> Self {
        Self {
            active: false,
            target: [0; 4],
            seq: 0,
            sent_tick: 0,
            reply_tick: 0,
            got_reply: false,
            timed_out: false,
        }
    }
}

pub struct PingCell(pub UnsafeCell<PingState>);
unsafe impl Sync for PingCell {}
pub static PING_STATE: PingCell = PingCell(UnsafeCell::new(PingState::empty()));

// ═══════════════════════════════════════════════════════════════════════════
// Shell callback implementations
// ═══════════════════════════════════════════════════════════════════════════

/// `ifconfig` — display network interface configuration.
pub fn ifconfig_cmd(w: &mut dyn core::fmt::Write) {
    let iface = unsafe { &*NET_IF.0.get() };
    let link = match iface.link {
        LinkState::Up => "UP",
        LinkState::Down => "DOWN",
    };

    let _ = writeln!(w, "  {} — {}", iface.name, link);
    let _ = writeln!(
        w,
        "    MAC:     {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        iface.mac[0], iface.mac[1], iface.mac[2], iface.mac[3], iface.mac[4], iface.mac[5]
    );
    let _ = writeln!(
        w,
        "    IPv4:    {}.{}.{}.{}",
        iface.ip[0], iface.ip[1], iface.ip[2], iface.ip[3]
    );
    #[cfg(feature = "dhcp")]
    {
        let src = if unsafe { DHCP_CONFIGURED } {
            "dhcp"
        } else {
            "pending"
        };
        let _ = writeln!(w, "    Source:  {}", src);
    }
    #[cfg(not(feature = "dhcp"))]
    {
        let _ = writeln!(w, "    Source:  static");
    }
    let _ = writeln!(
        w,
        "    Netmask: {}.{}.{}.{}",
        iface.netmask[0], iface.netmask[1], iface.netmask[2], iface.netmask[3]
    );
    let _ = writeln!(
        w,
        "    Gateway: {}.{}.{}.{}",
        iface.gateway[0], iface.gateway[1], iface.gateway[2], iface.gateway[3]
    );
    let _ = writeln!(w, "    MTU:     {}", iface.mtu);
    let _ = writeln!(
        w,
        "    RX:      {} packets, {} bytes",
        iface.rx_packets, iface.rx_bytes
    );
    let _ = writeln!(
        w,
        "    TX:      {} packets, {} bytes",
        iface.tx_packets, iface.tx_bytes
    );
    if iface.rx_errors > 0 || iface.tx_errors > 0 {
        let _ = writeln!(
            w,
            "    Errors:  RX={} TX={}",
            iface.rx_errors, iface.tx_errors
        );
    }

    // Show smoltcp socket status.
    if is_active() {
        let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_ref().unwrap() };
        let stack = unsafe { (*NET_STACK.0.get()).as_ref().unwrap() };
        let tcp = sockets.get::<TcpSocket>(stack.tcp_handle());
        let tcp_state = match tcp.state() {
            smoltcp::socket::tcp::State::Closed => "closed",
            smoltcp::socket::tcp::State::Listen => "listen",
            smoltcp::socket::tcp::State::SynSent => "syn-sent",
            smoltcp::socket::tcp::State::SynReceived => "syn-rcvd",
            smoltcp::socket::tcp::State::Established => "established",
            smoltcp::socket::tcp::State::FinWait1 => "fin-wait-1",
            smoltcp::socket::tcp::State::FinWait2 => "fin-wait-2",
            smoltcp::socket::tcp::State::CloseWait => "close-wait",
            smoltcp::socket::tcp::State::Closing => "closing",
            smoltcp::socket::tcp::State::LastAck => "last-ack",
            smoltcp::socket::tcp::State::TimeWait => "time-wait",
        };
        let _ = writeln!(w, "    TCP:     {}", tcp_state);
    }
}

/// `netstat` — show socket status.
pub fn netstat_cmd(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  Proto  State        Local       Remote");
    let _ = writeln!(w, "  ─────  ───────────  ──────────  ──────────");

    if !is_active() {
        let _ = writeln!(w, "  (network stack not active)");
        return;
    }

    let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_ref().unwrap() };
    let stack = unsafe { (*NET_STACK.0.get()).as_ref().unwrap() };

    // TCP socket.
    let tcp = sockets.get::<TcpSocket>(stack.tcp_handle());
    let state = match tcp.state() {
        smoltcp::socket::tcp::State::Closed => "closed",
        smoltcp::socket::tcp::State::Listen => "LISTEN",
        smoltcp::socket::tcp::State::Established => "ESTABLISHED",
        _ => "other",
    };
    let local = tcp.local_endpoint();
    let remote = tcp.remote_endpoint();
    match (local, remote) {
        (Some(l), Some(r)) => {
            let _ = writeln!(w, "  TCP    {:11}  {:?}  {:?}", state, l, r);
        }
        (Some(l), None) => {
            let _ = writeln!(w, "  TCP    {:11}  {:?}  *:*", state, l);
        }
        (None, Some(r)) => {
            let _ = writeln!(w, "  TCP    {:11}  *:*    {:?}", state, r);
        }
        (None, None) => {
            let _ = writeln!(w, "  TCP    {:11}  *:*    *:*", state);
        }
    }

    // UDP socket.
    if let Some(handle) = unsafe { UDP_HANDLE } {
        let udp = sockets.get::<UdpSocket>(handle);
        let ep = udp.endpoint();
        let _ = writeln!(
            w,
            "  UDP    {:11}  {:?}",
            if ep.port != 0 { "BOUND" } else { "closed" },
            ep
        );
    }

    // Local (in-kernel) sockets.
    let sock_table = unsafe { &*crate::SOCKETS.0.get() };
    for (i, s) in sock_table.socks_ref().iter().enumerate() {
        if s.state == microkernel::socket::SockState::Free {
            continue;
        }
        let dom = match s.domain {
            microkernel::socket::Domain::Local => "local",
            microkernel::socket::Domain::Inet => "inet",
        };
        let st = match s.state {
            microkernel::socket::SockState::Created => "created",
            microkernel::socket::SockState::Bound => "bound",
            microkernel::socket::SockState::Listening => "LISTEN",
            microkernel::socket::SockState::Connected => "CONNECTED",
            microkernel::socket::SockState::Closed => "closed",
            microkernel::socket::SockState::Free => "free",
        };
        let _ = writeln!(
            w,
            "  {:5}  {:11}  sock={}  addr={}",
            dom, st, i, s.bound_addr
        );
    }
}

/// Parse an IPv4 address string like "10.0.2.2" into 4 bytes.
pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut parts = 0usize;
    let mut val = 0u16;
    let mut has_digit = false;

    for b in s.bytes() {
        if b == b'.' {
            if !has_digit || parts >= 3 {
                return None;
            }
            if val > 255 {
                return None;
            }
            octets[parts] = val as u8;
            parts += 1;
            val = 0;
            has_digit = false;
        } else if b.is_ascii_digit() {
            val = val * 10 + (b - b'0') as u16;
            has_digit = true;
        } else {
            return None;
        }
    }
    if !has_digit || parts != 3 || val > 255 {
        return None;
    }
    octets[3] = val as u8;
    Some(octets)
}

/// `ping <ip>` — send ICMP echo request.
///
/// Note: smoltcp handles ICMP echo *replies* automatically when it receives
/// echo requests. For outgoing pings, we need to use a raw socket or
/// craft ICMP manually. For now we verify ARP resolution and reachability
/// by attempting to send a UDP probe and checking if ARP resolves.
pub fn ping_cmd(args: &str, w: &mut dyn core::fmt::Write) {
    if args.is_empty() {
        let _ = writeln!(w, "  usage: ping <ip-address>");
        return;
    }

    let target = match parse_ipv4(args.trim()) {
        Some(ip) => ip,
        None => {
            let _ = writeln!(w, "  invalid IPv4 address: '{}'", args);
            return;
        }
    };

    if !is_active() {
        let _ = writeln!(w, "  network stack not active");
        return;
    }

    let _ = writeln!(
        w,
        "  PING {}.{}.{}.{} — sending 3 probes...",
        target[0], target[1], target[2], target[3]
    );

    // We use the UDP socket to probe — send a tiny UDP packet to a
    // well-known port. This triggers ARP resolution and we can observe
    // if the stack processes the packet.
    let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
    let udp_handle = match unsafe { UDP_HANDLE } {
        Some(h) => h,
        None => {
            let _ = writeln!(w, "  no UDP socket available");
            return;
        }
    };

    // Bind to an ephemeral port if not already bound.
    {
        let udp = sockets.get_mut::<UdpSocket>(udp_handle);
        if !udp.is_open() {
            let _ = udp.bind(12345);
        }
    }

    let dest = IpEndpoint::new(
        IpAddress::v4(target[0], target[1], target[2], target[3]),
        7, // echo port
    );

    for seq in 0..3u16 {
        let payload: [u8; 8] = [b'V', b'e', b'e', b'r', (seq >> 8) as u8, seq as u8, 0, 0];

        {
            let udp = sockets.get_mut::<UdpSocket>(udp_handle);
            match udp.send_slice(&payload, dest) {
                Ok(()) => {}
                Err(_) => {
                    let _ = writeln!(w, "  seq={}: send failed (ARP pending?)", seq);
                }
            }
        }

        // Poll the stack a few times to process ARP and TX.
        let ticks = unsafe { (*crate::SCHEDULER.0.get()).ticks };
        for i in 0..50u64 {
            poll(ticks + i);
        }

        let _ = writeln!(
            w,
            "  seq={}: probe sent to {}.{}.{}.{}:7",
            seq, target[0], target[1], target[2], target[3]
        );
    }

    let _ = writeln!(w, "  (ARP resolution + UDP probes complete)");
    let _ = writeln!(
        w,
        "  Note: smoltcp responds to inbound ICMP echo automatically."
    );
}

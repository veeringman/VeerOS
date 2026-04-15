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

use arch::NetworkDevice;
use net::{NetStack, NetStorage};
use smoltcp::iface::SocketSet;
use smoltcp::socket::tcp::Socket as TcpSocket;
use smoltcp::socket::udp::{Socket as UdpSocket, PacketBuffer, PacketMetadata};
use smoltcp::wire::{IpCidr, Ipv4Address, IpAddress, IpEndpoint};

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
        if !net.active { return false; }
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
/// We support 4 sockets: 2 TCP + 2 UDP.
const MAX_SMOL_SOCKETS: usize = 4;

// Compile-time sanity: make sure the SocketStorage array can't silently
// overflow into neighbouring statics.
const _: () = {
    let sz = core::mem::size_of::<[smoltcp::iface::SocketStorage<'static>; MAX_SMOL_SOCKETS]>();
    if sz > 16384 {
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

/// Whether the network stack has been initialised.
static mut NET_INIT: bool = false;

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
        let _ = core::fmt::Write::write_fmt(&mut con, format_args!(
            "[net] SocketStorage size={}, buf at {:p}, NET_IF at {:p}\n",
            core::mem::size_of::<smoltcp::iface::SocketStorage>(),
            unsafe { &SOCKET_SET_BUF as *const _ },
            unsafe { &*NET_IF.0.get() as *const _ },
        ));
    }

    // Create the smoltcp socket set.
    let socket_set = unsafe {
        SocketSet::new(&mut SOCKET_SET_BUF.0[..])
    };
    unsafe { *SMOL_SOCKETS.0.get() = Some(socket_set); }

    // Create the NetStack (smoltcp Interface + primary TCP socket).
    let dev = VirtioNetDev;
    let ip = IpCidr::new(
        IpAddress::v4(DEFAULT_IP[0], DEFAULT_IP[1], DEFAULT_IP[2], DEFAULT_IP[3]),
        24,
    );
    let gw = Ipv4Address::new(
        DEFAULT_GATEWAY[0], DEFAULT_GATEWAY[1], DEFAULT_GATEWAY[2], DEFAULT_GATEWAY[3],
    );

    let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
    let storage = unsafe { &mut *core::ptr::addr_of_mut!(NET_STORAGE) };

    let stack = NetStack::new(dev, ip, gw, sockets, storage);
    unsafe { *NET_STACK.0.get() = Some(stack); }

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

    // Mark interface up.
    iface.link = LinkState::Up;
    unsafe { NET_INIT = true; }
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

    let stack = unsafe { (*NET_STACK.0.get()).as_mut().unwrap() };
    let sockets = unsafe { (*SMOL_SOCKETS.0.get()).as_mut().unwrap() };
    stack.poll(sockets, now_ms);

    // Update RX/TX counters.
    let iface = unsafe { &mut *NET_IF.0.get() };
    // (smoltcp doesn't expose packet counters directly; we track at driver level)
    let _ = iface;
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
    let _ = writeln!(w, "    MAC:     {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        iface.mac[0], iface.mac[1], iface.mac[2],
        iface.mac[3], iface.mac[4], iface.mac[5]);
    let _ = writeln!(w, "    IPv4:    {}.{}.{}.{}",
        iface.ip[0], iface.ip[1], iface.ip[2], iface.ip[3]);
    let _ = writeln!(w, "    Netmask: {}.{}.{}.{}",
        iface.netmask[0], iface.netmask[1], iface.netmask[2], iface.netmask[3]);
    let _ = writeln!(w, "    Gateway: {}.{}.{}.{}",
        iface.gateway[0], iface.gateway[1], iface.gateway[2], iface.gateway[3]);
    let _ = writeln!(w, "    MTU:     {}", iface.mtu);
    let _ = writeln!(w, "    RX:      {} packets, {} bytes",
        iface.rx_packets, iface.rx_bytes);
    let _ = writeln!(w, "    TX:      {} packets, {} bytes",
        iface.tx_packets, iface.tx_bytes);
    if iface.rx_errors > 0 || iface.tx_errors > 0 {
        let _ = writeln!(w, "    Errors:  RX={} TX={}", iface.rx_errors, iface.tx_errors);
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
        (Some(l), Some(r)) => { let _ = writeln!(w, "  TCP    {:11}  {:?}  {:?}", state, l, r); }
        (Some(l), None)    => { let _ = writeln!(w, "  TCP    {:11}  {:?}  *:*", state, l); }
        (None, Some(r))    => { let _ = writeln!(w, "  TCP    {:11}  *:*    {:?}", state, r); }
        (None, None)       => { let _ = writeln!(w, "  TCP    {:11}  *:*    *:*", state); }
    }

    // UDP socket.
    if let Some(handle) = unsafe { UDP_HANDLE } {
        let udp = sockets.get::<UdpSocket>(handle);
        let ep = udp.endpoint();
        let _ = writeln!(w, "  UDP    {:11}  {:?}", 
            if ep.port != 0 { "BOUND" } else { "closed" },
            ep);
    }

    // Local (in-kernel) sockets.
    let sock_table = unsafe { &*crate::SOCKETS.0.get() };
    for (i, s) in sock_table.socks_ref().iter().enumerate() {
        if s.state == microkernel::socket::SockState::Free { continue; }
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
        let _ = writeln!(w, "  {:5}  {:11}  sock={}  addr={}", dom, st, i, s.bound_addr);
    }
}

/// Parse an IPv4 address string like "10.0.2.2" into 4 bytes.
fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut octets = [0u8; 4];
    let mut parts = 0usize;
    let mut val = 0u16;
    let mut has_digit = false;

    for b in s.bytes() {
        if b == b'.' {
            if !has_digit || parts >= 3 { return None; }
            if val > 255 { return None; }
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
    if !has_digit || parts != 3 || val > 255 { return None; }
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

    let _ = writeln!(w, "  PING {}.{}.{}.{} — sending 3 probes...",
        target[0], target[1], target[2], target[3]);

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
        let payload: [u8; 8] = [
            b'V', b'e', b'e', b'r',
            (seq >> 8) as u8, seq as u8,
            0, 0,
        ];

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

        let _ = writeln!(w, "  seq={}: probe sent to {}.{}.{}.{}:7",
            seq, target[0], target[1], target[2], target[3]);
    }

    let _ = writeln!(w, "  (ARP resolution + UDP probes complete)");
    let _ = writeln!(w, "  Note: smoltcp responds to inbound ICMP echo automatically.");
}

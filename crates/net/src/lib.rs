//! VeerOS network stack — smoltcp integration and TCP-to-Serial bridge.
//!
//! This crate provides:
//! - [`NetStack`]: wraps a [`NetworkDevice`] with a smoltcp TCP/IP interface.
//! - [`TcpSerial`]: implements [`arch::Serial`] over a TCP connection so the
//!   VeerOS shell (or any serial consumer) can run over the network.
//!
//! The design is intentionally **cooperative / polling-based** — the kernel
//! must call [`NetStack::poll()`] periodically (e.g. from a dedicated task
//! or from the timer ISR) to drive the TCP/IP state machine.

#![no_std]

pub mod auth;
#[cfg(feature = "secure-connect")]
pub mod secure;

use arch::{NetMedium, NetworkDevice, Serial};
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, Ieee802154Address, IpCidr, Ipv4Address, Ipv4Cidr};

// ═══════════════════════════════════════════════════════════════════════════
// smoltcp phy adapter — bridges our `NetworkDevice` trait to smoltcp's
// `Device` trait.
// ═══════════════════════════════════════════════════════════════════════════

/// Adapter that lets smoltcp drive any VeerOS [`NetworkDevice`].
pub struct DeviceAdapter<'a, D: NetworkDevice> {
    inner: &'a D,
}

impl<'a, D: NetworkDevice> DeviceAdapter<'a, D> {
    pub fn new(dev: &'a D) -> Self {
        Self { inner: dev }
    }
}

impl<D: NetworkDevice> Device for DeviceAdapter<'_, D> {
    type RxToken<'a> = VeerRxToken<'a, D> where Self: 'a;
    type TxToken<'a> = VeerTxToken<'a, D> where Self: 'a;

    fn receive(
        &mut self,
        _timestamp: Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if self.inner.has_rx() {
            Some((
                VeerRxToken { dev: self.inner },
                VeerTxToken { dev: self.inner },
            ))
        } else {
            None
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(VeerTxToken { dev: self.inner })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = match self.inner.medium() {
            NetMedium::Ethernet => Medium::Ethernet,
            NetMedium::Ieee802154 => Medium::Ieee802154,
        };
        caps.max_transmission_unit = self.inner.mtu();
        caps
    }
}

pub struct VeerRxToken<'a, D: NetworkDevice> {
    dev: &'a D,
}

impl<D: NetworkDevice> RxToken for VeerRxToken<'_, D> {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = [0u8; 1514];
        let len = self.dev.recv(&mut buf);
        f(&mut buf[..len])
    }
}

pub struct VeerTxToken<'a, D: NetworkDevice> {
    dev: &'a D,
}

impl<D: NetworkDevice> TxToken for VeerTxToken<'_, D> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = [0u8; 1514];
        let result = f(&mut buf[..len]);
        self.dev.send(&buf[..len]);
        result
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// NetStack — owns the smoltcp Interface + socket set
// ═══════════════════════════════════════════════════════════════════════════

/// Fixed-size buffer sizes for TCP sockets (no heap).
const TCP_RX_BUF_SIZE: usize = 2048;
const TCP_TX_BUF_SIZE: usize = 2048;

/// Static socket-set storage for smoltcp.
pub struct NetStorage {
    pub tcp_rx_buf: [u8; TCP_RX_BUF_SIZE],
    pub tcp_tx_buf: [u8; TCP_TX_BUF_SIZE],
}

impl NetStorage {
    pub const fn new() -> Self {
        Self {
            tcp_rx_buf: [0u8; TCP_RX_BUF_SIZE],
            tcp_tx_buf: [0u8; TCP_TX_BUF_SIZE],
        }
    }
}

/// High-level network stack wrapping smoltcp.
///
/// The caller is responsible for calling [`poll()`] frequently.
pub struct NetStack<D: NetworkDevice> {
    dev: D,
    iface: Interface,
    tcp_handle: SocketHandle,
}

impl<D: NetworkDevice> NetStack<D> {
    /// Create a new network stack.
    ///
    /// `ip` — the IPv4 address for this device (e.g. `10.0.2.15/24`).
    /// `gateway` — default gateway (e.g. `10.0.2.2` for QEMU user-net).
    /// `sockets` — mutable reference to a `SocketSet` managed by the caller.
    /// `storage` — mutable reference to static buffer storage.
    pub fn new(
        dev: D,
        ip: IpCidr,
        gateway: Ipv4Address,
        sockets: &mut SocketSet<'_>,
        storage: &'static mut NetStorage,
    ) -> Self {
        let hw_addr = match dev.medium() {
            NetMedium::Ethernet => {
                let mac = dev.mac_address();
                HardwareAddress::Ethernet(EthernetAddress(mac))
            }
            NetMedium::Ieee802154 => {
                let ext = dev.mac_address_ext();
                HardwareAddress::Ieee802154(Ieee802154Address::Extended(ext))
            }
        };

        let config = Config::new(hw_addr);
        let mut adapter = DeviceAdapter::new(&dev);
        let mut iface = Interface::new(config, &mut adapter, Instant::ZERO);

        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(ip);
        });
        iface
            .routes_mut()
            .add_default_ipv4_route(gateway)
            .ok();

        let rx_buf = SocketBuffer::new(&mut storage.tcp_rx_buf[..]);
        let tx_buf = SocketBuffer::new(&mut storage.tcp_tx_buf[..]);
        let tcp_socket = TcpSocket::new(rx_buf, tx_buf);
        let tcp_handle = sockets.add(tcp_socket);

        Self {
            dev,
            iface,
            tcp_handle,
        }
    }

    /// Create a new network stack without assigning an IP address.
    ///
    /// Use this when the IP will be acquired via DHCP. Call
    /// [`apply_ip_config()`] once the DHCP lease is obtained.
    pub fn new_dhcp(
        dev: D,
        sockets: &mut SocketSet<'_>,
        storage: &'static mut NetStorage,
    ) -> Self {
        let hw_addr = match dev.medium() {
            NetMedium::Ethernet => {
                let mac = dev.mac_address();
                HardwareAddress::Ethernet(EthernetAddress(mac))
            }
            NetMedium::Ieee802154 => {
                let ext = dev.mac_address_ext();
                HardwareAddress::Ieee802154(Ieee802154Address::Extended(ext))
            }
        };

        let config = Config::new(hw_addr);
        let mut adapter = DeviceAdapter::new(&dev);
        let iface = Interface::new(config, &mut adapter, Instant::ZERO);

        let rx_buf = SocketBuffer::new(&mut storage.tcp_rx_buf[..]);
        let tx_buf = SocketBuffer::new(&mut storage.tcp_tx_buf[..]);
        let tcp_socket = TcpSocket::new(rx_buf, tx_buf);
        let tcp_handle = sockets.add(tcp_socket);

        Self {
            dev,
            iface,
            tcp_handle,
        }
    }

    /// Apply an IP configuration (e.g. from DHCP).
    pub fn apply_ip_config(&mut self, address: Ipv4Cidr, gateway: Option<Ipv4Address>) {
        self.iface.update_ip_addrs(|addrs| {
            addrs.clear();
            let _ = addrs.push(IpCidr::Ipv4(address));
        });
        if let Some(gw) = gateway {
            self.iface.routes_mut().add_default_ipv4_route(gw).ok();
        }
    }

    /// Remove IP configuration (e.g. on DHCP deconfigure).
    pub fn remove_ip_config(&mut self) {
        self.iface.update_ip_addrs(|addrs| {
            addrs.clear();
        });
        // smoltcp 0.11 doesn't expose a direct remove-route API, but
        // clearing addresses is sufficient to stop answering.
    }

    /// Start listening for incoming TCP connections on the given port.
    ///
    /// If the socket is still lingering from a previous session it will
    /// be aborted first so that `listen()` can succeed immediately.
    pub fn listen(&mut self, sockets: &mut SocketSet<'_>, port: u16) {
        let socket = sockets.get_mut::<TcpSocket>(self.tcp_handle);
        if socket.is_open() {
            socket.abort();
        }
        socket.listen(port).ok();
    }

    /// Drive the TCP/IP state machine. Call this frequently.
    ///
    /// `now_ms` — current time in milliseconds (from the tick counter).
    pub fn poll(&mut self, sockets: &mut SocketSet<'_>, now_ms: u64) {
        let timestamp = Instant::from_millis(now_ms as i64);
        let mut adapter = DeviceAdapter::new(&self.dev);
        self.iface.poll(timestamp, &mut adapter, sockets);
    }

    /// Returns the TCP socket handle (for direct access).
    pub fn tcp_handle(&self) -> SocketHandle {
        self.tcp_handle
    }

    /// Convenience: check if a TCP client is currently connected.
    pub fn is_connected(&self, sockets: &SocketSet<'_>) -> bool {
        let socket = sockets.get::<TcpSocket>(self.tcp_handle);
        socket.is_active()
    }

    /// Add a new TCP socket to the socket set using the provided storage.
    /// Returns the socket handle for the new socket.
    pub fn add_tcp_socket(
        &self,
        sockets: &mut SocketSet<'_>,
        storage: &'static mut NetStorage,
    ) -> SocketHandle {
        let rx_buf = SocketBuffer::new(&mut storage.tcp_rx_buf[..]);
        let tx_buf = SocketBuffer::new(&mut storage.tcp_tx_buf[..]);
        let tcp_socket = TcpSocket::new(rx_buf, tx_buf);
        sockets.add(tcp_socket)
    }

    /// Start listening on a specific socket handle.
    pub fn listen_handle(&mut self, sockets: &mut SocketSet<'_>, handle: SocketHandle, port: u16) {
        let socket = sockets.get_mut::<TcpSocket>(handle);
        if socket.is_open() {
            socket.abort();
        }
        socket.listen(port).ok();
    }

    /// Check if a specific socket handle has an active connection.
    pub fn is_connected_handle(&self, sockets: &SocketSet<'_>, handle: SocketHandle) -> bool {
        let socket = sockets.get::<TcpSocket>(handle);
        socket.is_active()
    }

    /// Return a mutable reference to the smoltcp interface context.
    ///
    /// Needed for TCP `connect()` calls (outbound connections).
    pub fn context(&mut self) -> &mut smoltcp::iface::Context {
        self.iface.context()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TcpSerial — implements `arch::Serial` over a TCP socket
// ═══════════════════════════════════════════════════════════════════════════

/// Bridges a smoltcp TCP socket to the VeerOS `Serial` trait.
///
/// This lets the shell (or any other `Serial`-consumer) run transparently
/// over a TCP connection instead of a UART.
///
/// **Usage pattern (inside a task):**
/// ```ignore
/// // After NetStack::poll() detects a connection on port 2323:
/// let tcp_serial = TcpSerial::new(tcp_handle, &SOCKETS, poll_fn, unlock_fn);
/// let mut con = Console::new(tcp_serial);
/// shell.run(&mut con);
/// ```
///
/// Because `Serial` is synchronous (blocking), `TcpSerial` spins and
/// calls the provided `poll_fn` while waiting for data. This keeps the
/// network stack alive during blocking reads.
///
/// The `poll_fn` must acquire a global lock (e.g. POLL_BUSY) and
/// **leave the lock held** on return.  A separate background task must
/// call `NetStack::poll()` frequently to drive the TCP/IP stack.
/// After TcpSerial finishes accessing the socket set, it calls
/// `unlock_fn` to release the lock.  This prevents races with
/// the background net-poll task that also mutates the socket set.
pub struct TcpSerial {
    handle: SocketHandle,
    /// Pointer to the global SocketSet (via UnsafeCell in the kernel).
    sockets: *mut SocketSet<'static>,
    /// Lock, poll the network stack, and keep lock held.
    /// Returns `true` if the lock was acquired (caller must unlock).
    poll_fn: fn() -> bool,
    /// Release the lock acquired by `poll_fn`.
    unlock_fn: fn(),
}

impl TcpSerial {
    /// # Safety
    /// The `sockets` pointer must remain valid for the lifetime of TcpSerial.
    pub unsafe fn new(
        handle: SocketHandle,
        sockets: *mut SocketSet<'static>,
        poll_fn: fn() -> bool,
        unlock_fn: fn(),
    ) -> Self {
        Self {
            handle,
            sockets,
            poll_fn,
            unlock_fn,
        }
    }
}

impl Serial for TcpSerial {
    fn write_byte(&self, byte: u8) {
        self.write_bytes(&[byte]);
    }

    fn write_bytes(&self, bytes: &[u8]) {
        let mut offset = 0;
        while offset < bytes.len() {
            let locked = (self.poll_fn)();
            if !locked {
                core::hint::spin_loop();
                continue;
            }
            let sockets = unsafe { &mut *self.sockets };
            let socket = sockets.get_mut::<TcpSocket>(self.handle);
            if !socket.may_send() {
                (self.unlock_fn)();
                return; // connection closed or closing
            }
            if socket.can_send() {
                match socket.send_slice(&bytes[offset..]) {
                    Ok(sent) if sent > 0 => {
                        offset += sent;
                        (self.unlock_fn)();
                        continue;
                    }
                    Ok(_) | Err(_) => {}
                }
            }
            (self.unlock_fn)();
            core::hint::spin_loop();
        }

        // Drive the stack once more so buffered data gets pushed promptly.
        let locked = (self.poll_fn)();
        if locked { (self.unlock_fn)(); }
    }

    fn read_byte(&self) -> u8 {
        loop {
            let locked = (self.poll_fn)();
            if !locked {
                core::hint::spin_loop();
                continue;
            }
            let sockets = unsafe { &mut *self.sockets };
            let socket = sockets.get_mut::<TcpSocket>(self.handle);
            if !socket.may_recv() {
                (self.unlock_fn)();
                return 0x04; // EOF → Ctrl-D → shell exit
            }
            if socket.can_recv() {
                let mut buf = [0u8; 1];
                if let Ok(n) = socket.recv_slice(&mut buf) {
                    if n > 0 {
                        (self.unlock_fn)();
                        return buf[0];
                    }
                }
            }
            (self.unlock_fn)();
            core::hint::spin_loop();
        }
    }

    fn has_data(&self) -> bool {
        loop {
            let locked = (self.poll_fn)();
            if locked {
                let sockets = unsafe { &mut *self.sockets };
                let socket = sockets.get_mut::<TcpSocket>(self.handle);
                let result = socket.can_recv();
                (self.unlock_fn)();
                return result;
            }
            core::hint::spin_loop();
        }
    }

    fn flush(&self) {
        let locked = (self.poll_fn)();
        if locked { (self.unlock_fn)(); }
    }
}

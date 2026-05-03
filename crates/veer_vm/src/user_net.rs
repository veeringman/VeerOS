use anyhow::{Context, Result};
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::tcp::{self, State as TcpState};
use smoltcp::time::{Duration as SmolDuration, Instant as SmolInstant};
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::vm::SHUTDOWN;

const HOST_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x02];
const HOST_IP: [u8; 4] = [10, 0, 2, 2];
const DNS_IP: [u8; 4] = [10, 0, 2, 3];
const GUEST_IP: [u8; 4] = [10, 0, 2, 15];
const NETMASK: [u8; 4] = [255, 255, 255, 0];

pub struct UserNetHandle {
    to_stack: Sender<Vec<u8>>,
    from_stack: Mutex<Receiver<Vec<u8>>>,
}

impl UserNetHandle {
    pub fn start(guest_mac: [u8; 6], host_port: u16, guest_port: u16) -> Result<Arc<Self>> {
        let (to_stack_tx, to_stack_rx) = mpsc::channel::<Vec<u8>>();
        let (from_stack_tx, from_stack_rx) = mpsc::channel::<Vec<u8>>();
        let listener = TcpListener::bind(("127.0.0.1", host_port))
            .with_context(|| format!("binding host forward 127.0.0.1:{host_port}"))?;
        listener
            .set_nonblocking(true)
            .context("setting host forward listener nonblocking")?;

        thread::Builder::new()
            .name("veer-vm-user-net".into())
            .spawn(move || user_net_thread(guest_mac, guest_port, listener, to_stack_rx, from_stack_tx))
            .context("spawn user-mode networking thread")?;

        Ok(Arc::new(Self {
            to_stack: to_stack_tx,
            from_stack: Mutex::new(from_stack_rx),
        }))
    }

    pub fn send_frame(&self, frame: &[u8]) {
        pace_tcp_frame(frame);
        let _ = self.to_stack.send(frame.to_vec());
    }

    pub fn recv_frame(&self, timeout: Duration) -> Option<Vec<u8>> {
        self.from_stack.lock().unwrap().recv_timeout(timeout).ok()
    }
}

struct QueueDevice {
    rx: VecDeque<Vec<u8>>,
    tx: Sender<Vec<u8>>,
}

struct QueueRxToken(Vec<u8>);
struct QueueTxToken(Sender<Vec<u8>>);

impl Device for QueueDevice {
    type RxToken<'a> = QueueRxToken where Self: 'a;
    type TxToken<'a> = QueueTxToken where Self: 'a;

    fn receive(&mut self, _timestamp: SmolInstant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.rx
            .pop_front()
            .map(|frame| (QueueRxToken(frame), QueueTxToken(self.tx.clone())))
    }

    fn transmit(&mut self, _timestamp: SmolInstant) -> Option<Self::TxToken<'_>> {
        Some(QueueTxToken(self.tx.clone()))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_transmission_unit = 1514;
        caps.medium = Medium::Ethernet;
        caps
    }
}

impl RxToken for QueueRxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut self.0)
    }
}

impl TxToken for QueueTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut frame = vec![0u8; len];
        let ret = f(&mut frame);
        pace_tcp_frame(&frame);
        let _ = self.0.send(frame);
        ret
    }
}

struct ForwardState {
    stream: TcpStream,
    handle: SocketHandle,
    host_closed: bool,
    was_active: bool,
    guest_bytes: bool,
    retry_count: u8,
}

fn user_net_thread(
    guest_mac: [u8; 6],
    guest_port: u16,
    listener: TcpListener,
    from_guest: Receiver<Vec<u8>>,
    to_guest: Sender<Vec<u8>>,
) {
    let mut device = QueueDevice {
        rx: VecDeque::new(),
        tx: to_guest.clone(),
    };
    let mut config = Config::new(EthernetAddress(HOST_MAC).into());
    config.random_seed = 0x5665_6572;
    let started = Instant::now();
    let mut iface = Interface::new(config, &mut device, now(started));
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::new(IpAddress::v4(HOST_IP[0], HOST_IP[1], HOST_IP[2], HOST_IP[3]), 24));
    });

    let mut sockets = SocketSet::new(Vec::new());
    let mut forward: Option<ForwardState> = None;
    let mut next_local_port = 49152u16;

    while !SHUTDOWN.load(Ordering::SeqCst) {
        while let Ok(frame) = from_guest.try_recv() {
            if let Some(reply) = dhcp_reply(&frame, guest_mac) {
                let _ = to_guest.send(reply);
            } else {
                device.rx.push_back(frame);
            }
        }

        let timestamp = now(started);
        let _ = iface.poll(timestamp, &mut device, &mut sockets);

        if forward.is_none() {
            match listener.accept() {
                Ok((stream, _)) => {
                    if stream.set_nonblocking(true).is_ok() {
                        // Give the rv32-soft scheduler a few cycles so the guest's
                        // TCP listener task is running before we attempt to connect.
                        thread::sleep(Duration::from_millis(200));
                        let rx = tcp::SocketBuffer::new(vec![0; 8192]);
                        let tx = tcp::SocketBuffer::new(vec![0; 8192]);
                        let mut socket = tcp::Socket::new(rx, tx);
                        let local_port = next_local_port;
                        next_local_port = next_local_port.wrapping_add(1).max(49152);
                        if socket
                            .connect(
                                iface.context(),
                                (IpAddress::v4(GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3]), guest_port),
                                local_port,
                            )
                            .is_ok()
                        {
                            let handle = sockets.add(socket);
                            eprintln!(
                                "[veer-vm] user-net: hostfwd connected 127.0.0.1 -> {}.{}.{}.{}:{}",
                                GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3], guest_port
                            );
                            forward = Some(ForwardState {
                                stream,
                                handle,
                                host_closed: false,
                                was_active: false,
                                guest_bytes: false,
                                retry_count: 0,
                            });
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => eprintln!("[veer-vm] user-net: accept failed: {e}"),
            }
        }

        if let Some(state) = forward.as_mut() {
            let socket = sockets.get_mut::<tcp::Socket>(state.handle);

            if socket.state() == TcpState::Established && !state.was_active {
                state.was_active = true;
                eprintln!("[veer-vm] user-net: guest tcp established");
            }

            if socket.can_send() && !state.host_closed {
                let mut buf = [0u8; 2048];
                loop {
                    match state.stream.read(&mut buf) {
                        Ok(0) => {
                            state.host_closed = true;
                            socket.close();
                            break;
                        }
                        Ok(n) => {
                            if socket.send_slice(&buf[..n]).is_err() {
                                break;
                            }
                            if n < buf.len() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => {
                            state.host_closed = true;
                            socket.abort();
                            break;
                        }
                    }
                }
            }

            if socket.can_recv() {
                let data = socket.recv(|data| (data.len(), data.to_vec())).unwrap_or_default();
                if !data.is_empty() {
                    state.guest_bytes = true;
                    match state.stream.write(&data) {
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                        Err(_) => {
                            state.host_closed = true;
                            socket.abort();
                        }
                    }
                }
            }

            if !socket.is_open() {
                let handle = state.handle;
                let was_active = state.was_active;
                let tcp_state = socket.state();
                sockets.remove(handle);
                if !state.guest_bytes && !state.host_closed && state.retry_count < 3 {
                    let rx = tcp::SocketBuffer::new(vec![0; 8192]);
                    let tx = tcp::SocketBuffer::new(vec![0; 8192]);
                    let mut retry_socket = tcp::Socket::new(rx, tx);
                    let local_port = next_local_port;
                    next_local_port = next_local_port.wrapping_add(1).max(49152);
                    if retry_socket
                        .connect(
                            iface.context(),
                            (IpAddress::v4(GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3]), guest_port),
                            local_port,
                        )
                        .is_ok()
                    {
                        state.handle = sockets.add(retry_socket);
                        state.was_active = false;
                        state.retry_count += 1;
                        eprintln!(
                            "[veer-vm] user-net: guest reset before data; retrying hostfwd ({}/3)",
                            state.retry_count
                        );
                    } else {
                        forward = None;
                        eprintln!(
                            "[veer-vm] user-net: hostfwd disconnected (was_active={was_active} state={tcp_state:?})"
                        );
                    }
                } else {
                    forward = None;
                    eprintln!(
                        "[veer-vm] user-net: hostfwd disconnected (was_active={was_active} state={tcp_state:?})"
                    );
                }
            }
        }

        let delay = iface
            .poll_delay(timestamp, &sockets)
            .unwrap_or_else(|| SmolDuration::from_millis(5));
        let sleep_ms = delay.total_millis().clamp(1, 5) as u64;
        thread::sleep(Duration::from_millis(sleep_ms));
    }
}

fn now(started: Instant) -> SmolInstant {
    SmolInstant::from_millis(started.elapsed().as_millis() as i64)
}

fn dhcp_reply(frame: &[u8], guest_mac: [u8; 6]) -> Option<Vec<u8>> {
    if frame.len() < 14 + 20 + 8 + 240 || frame[12..14] != [0x08, 0x00] {
        return None;
    }
    let ihl = ((frame[14] & 0x0f) as usize) * 4;
    if frame.len() < 14 + ihl + 8 + 240 || frame[23] != 17 {
        return None;
    }
    let udp = 14 + ihl;
    if u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]) != 67 {
        return None;
    }
    let bootp = udp + 8;
    if frame[bootp + 236..bootp + 240] != [99, 130, 83, 99] {
        return None;
    }
    let msg_type = dhcp_message_type(&frame[bootp + 240..])?;
    let reply_type = match msg_type {
        1 => 2,
        3 => 5,
        _ => return None,
    };

    let mut bootp_payload = vec![0u8; 240];
    bootp_payload[0] = 2;
    bootp_payload[1] = 1;
    bootp_payload[2] = 6;
    bootp_payload[4..8].copy_from_slice(&frame[bootp + 4..bootp + 8]);
    bootp_payload[10..12].copy_from_slice(&frame[bootp + 10..bootp + 12]);
    bootp_payload[16..20].copy_from_slice(&GUEST_IP);
    bootp_payload[20..24].copy_from_slice(&HOST_IP);
    bootp_payload[28..34].copy_from_slice(&guest_mac);
    bootp_payload[236..240].copy_from_slice(&[99, 130, 83, 99]);
    bootp_payload.extend_from_slice(&[
        53, 1, reply_type,
        54, 4, HOST_IP[0], HOST_IP[1], HOST_IP[2], HOST_IP[3],
        1, 4, NETMASK[0], NETMASK[1], NETMASK[2], NETMASK[3],
        3, 4, HOST_IP[0], HOST_IP[1], HOST_IP[2], HOST_IP[3],
        6, 4, DNS_IP[0], DNS_IP[1], DNS_IP[2], DNS_IP[3],
        51, 4, 0, 0, 0x0e, 0x10,
        255,
    ]);

    let udp_len = (8 + bootp_payload.len()) as u16;
    let ip_len = (20 + udp_len as usize) as u16;
    let mut out = Vec::with_capacity(14 + ip_len as usize);
    out.extend_from_slice(&guest_mac);
    out.extend_from_slice(&HOST_MAC);
    out.extend_from_slice(&[0x08, 0x00]);
    out.extend_from_slice(&[
        0x45, 0, (ip_len >> 8) as u8, ip_len as u8, 0, 0, 0, 0, 64, 17, 0, 0,
        HOST_IP[0], HOST_IP[1], HOST_IP[2], HOST_IP[3],
        255, 255, 255, 255,
    ]);
    let sum = ipv4_checksum(&out[14..34]);
    out[24] = (sum >> 8) as u8;
    out[25] = sum as u8;
    out.extend_from_slice(&[0, 67, 0, 68, (udp_len >> 8) as u8, udp_len as u8, 0, 0]);
    out.extend_from_slice(&bootp_payload);
    Some(out)
}

fn dhcp_message_type(options: &[u8]) -> Option<u8> {
    let mut i = 0usize;
    while i < options.len() {
        match options[i] {
            0 => i += 1,
            255 => return None,
            code => {
                if i + 1 >= options.len() {
                    return None;
                }
                let len = options[i + 1] as usize;
                if i + 2 + len > options.len() {
                    return None;
                }
                if code == 53 && len == 1 {
                    return Some(options[i + 2]);
                }
                i += 2 + len;
            }
        }
    }
    None
}

fn ipv4_checksum(header: &[u8]) -> u16 {
    let mut sum = 0u32;
    for chunk in header.chunks(2) {
        let word = if chunk.len() == 2 {
            u16::from_be_bytes([chunk[0], chunk[1]]) as u32
        } else {
            (chunk[0] as u32) << 8
        };
        sum = sum.wrapping_add(word);
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn pace_tcp_frame(frame: &[u8]) {
    if frame.len() < 14 + 20 || frame[12..14] != [0x08, 0x00] {
        return;
    }
    let ihl = ((frame[14] & 0x0f) as usize) * 4;
    if frame.len() >= 14 + ihl + 20 && frame[23] == 6 {
        thread::sleep(Duration::from_millis(5));
    }
}
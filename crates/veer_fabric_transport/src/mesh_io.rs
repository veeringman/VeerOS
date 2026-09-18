//! Host UDP/TCP adapters for the microkernel [`MeshTransport`] trait.
//!
//! Wire envelope (host only; the kernel mesh frame is unchanged):
//!
//! ```text
//! UDP datagram:  [src_node_id: 32][mesh_frame...]
//! TCP stream:    [u16 LE length][src_node_id: 32][mesh_frame...]
//! ```
//!
//! `length` is `32 + mesh_frame.len()`.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use microkernel::fabric_proto::{NodeId, ProtoError, NODE_ID_LEN};
use microkernel::mesh::{MeshTransport, TransportMsg, MAX_MESH_MSG_LEN};

const SRC_LEN: usize = NODE_ID_LEN;
const UDP_MAX: usize = SRC_LEN + MAX_MESH_MSG_LEN;
const TCP_MAX: usize = 2 + UDP_MAX;

fn encode_envelope(local_id: &NodeId, msg: &TransportMsg, out: &mut [u8]) -> Result<usize, ProtoError> {
    let total = SRC_LEN + msg.len;
    if out.len() < total {
        return Err(ProtoError::BufferTooSmall);
    }
    out[..SRC_LEN].copy_from_slice(local_id);
    out[SRC_LEN..total].copy_from_slice(msg.as_slice());
    Ok(total)
}

fn decode_envelope(buf: &[u8]) -> Result<(NodeId, TransportMsg), ProtoError> {
    if buf.len() < SRC_LEN {
        return Err(ProtoError::BufferTooSmall);
    }
    let mut from = [0u8; NODE_ID_LEN];
    from.copy_from_slice(&buf[..SRC_LEN]);
    let msg = TransportMsg::from_slice(&buf[SRC_LEN..])?;
    Ok((from, msg))
}

fn write_timeout(stream: &mut TcpStream, data: &[u8], timeout: Duration) -> Result<(), ProtoError> {
    let start = Instant::now();
    let mut off = 0;
    while off < data.len() {
        match stream.write(&data[off..]) {
            Ok(0) => return Err(ProtoError::Transport),
            Ok(n) => off += n,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if start.elapsed() > timeout {
                    return Err(ProtoError::Transport);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => return Err(ProtoError::Transport),
        }
    }
    Ok(())
}

/// UDP datagram mesh transport.
pub struct UdpMeshTransport {
    socket: UdpSocket,
    local_id: NodeId,
    peers: Vec<(NodeId, SocketAddr)>,
    inbox: VecDeque<(NodeId, TransportMsg)>,
}

impl UdpMeshTransport {
    pub fn bind(addr: SocketAddr, local_id: NodeId) -> Result<Self, ProtoError> {
        let socket = UdpSocket::bind(addr).map_err(|_| ProtoError::Transport)?;
        socket
            .set_nonblocking(true)
            .map_err(|_| ProtoError::Transport)?;
        Ok(Self {
            socket,
            local_id,
            peers: Vec::new(),
            inbox: VecDeque::new(),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, ProtoError> {
        self.socket.local_addr().map_err(|_| ProtoError::Transport)
    }

    pub fn local_id(&self) -> NodeId {
        self.local_id
    }

    /// Remember how to reach `node_id`.
    pub fn map_peer(&mut self, node_id: NodeId, addr: SocketAddr) {
        if let Some(existing) = self.peers.iter_mut().find(|(id, _)| *id == node_id) {
            existing.1 = addr;
            return;
        }
        self.peers.push((node_id, addr));
    }

    pub fn addr_for(&self, node_id: &NodeId) -> Option<SocketAddr> {
        self.peers
            .iter()
            .find(|(id, _)| id == node_id)
            .map(|(_, addr)| *addr)
    }

    /// Send a mesh frame to a known socket before the peer's node ID is learned.
    pub fn send_to_addr(&mut self, addr: SocketAddr, msg: &TransportMsg) -> Result<(), ProtoError> {
        let mut buf = [0u8; UDP_MAX];
        let len = encode_envelope(&self.local_id, msg, &mut buf)?;
        self.socket
            .send_to(&buf[..len], addr)
            .map_err(|_| ProtoError::Transport)?;
        Ok(())
    }

    fn drain(&mut self) {
        let mut buf = [0u8; UDP_MAX];
        loop {
            match self.socket.recv_from(&mut buf) {
                Ok((n, addr)) => {
                    if let Ok((from, msg)) = decode_envelope(&buf[..n]) {
                        self.map_peer(from, addr);
                        self.inbox.push_back((from, msg));
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }
}

impl MeshTransport for UdpMeshTransport {
    fn send(&mut self, node_id: &NodeId, msg: &TransportMsg) -> Result<(), ProtoError> {
        let addr = self.addr_for(node_id).ok_or(ProtoError::Transport)?;
        self.send_to_addr(addr, msg)
    }

    fn recv(&mut self) -> Option<(NodeId, TransportMsg)> {
        self.drain();
        self.inbox.pop_front()
    }
}

struct TcpLink {
    stream: TcpStream,
    node_id: Option<NodeId>,
    buf: Vec<u8>,
}

impl TcpLink {
    fn new(stream: TcpStream) -> Result<Self, ProtoError> {
        stream
            .set_nonblocking(true)
            .map_err(|_| ProtoError::Transport)?;
        let _ = stream.set_nodelay(true);
        Ok(Self {
            stream,
            node_id: None,
            buf: Vec::new(),
        })
    }

    fn pull_frames(&mut self, inbox: &mut VecDeque<(NodeId, TransportMsg)>) {
        let mut tmp = [0u8; TCP_MAX];
        loop {
            match self.stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }

        loop {
            if self.buf.len() < 2 {
                break;
            }
            let frame_len = u16::from_le_bytes([self.buf[0], self.buf[1]]) as usize;
            if frame_len == 0 || frame_len > UDP_MAX {
                self.buf.clear();
                break;
            }
            let total = 2 + frame_len;
            if self.buf.len() < total {
                break;
            }
            if let Ok((from, msg)) = decode_envelope(&self.buf[2..total]) {
                self.node_id = Some(from);
                inbox.push_back((from, msg));
            }
            self.buf.drain(..total);
        }
    }
}

/// Length-prefixed TCP mesh transport.
pub struct TcpMeshTransport {
    listener: TcpListener,
    local_id: NodeId,
    addrs: Vec<(NodeId, SocketAddr)>,
    links: Vec<TcpLink>,
    inbox: VecDeque<(NodeId, TransportMsg)>,
}

impl TcpMeshTransport {
    pub fn bind(addr: SocketAddr, local_id: NodeId) -> Result<Self, ProtoError> {
        let listener = TcpListener::bind(addr).map_err(|_| ProtoError::Transport)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| ProtoError::Transport)?;
        Ok(Self {
            listener,
            local_id,
            addrs: Vec::new(),
            links: Vec::new(),
            inbox: VecDeque::new(),
        })
    }

    pub fn local_addr(&self) -> Result<SocketAddr, ProtoError> {
        self.listener.local_addr().map_err(|_| ProtoError::Transport)
    }

    pub fn map_peer(&mut self, node_id: NodeId, addr: SocketAddr) {
        if let Some(existing) = self.addrs.iter_mut().find(|(id, _)| *id == node_id) {
            existing.1 = addr;
            return;
        }
        self.addrs.push((node_id, addr));
    }

    fn accept_pending(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Ok(link) = TcpLink::new(stream) {
                        self.links.push(link);
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }

    fn ensure_link(&mut self, node_id: &NodeId) -> Result<usize, ProtoError> {
        if let Some(idx) = self
            .links
            .iter()
            .position(|link| link.node_id.as_ref() == Some(node_id))
        {
            return Ok(idx);
        }
        let addr = self
            .addrs
            .iter()
            .find(|(id, _)| id == node_id)
            .map(|(_, addr)| *addr)
            .ok_or(ProtoError::Transport)?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(200))
            .map_err(|_| ProtoError::Transport)?;
        self.links.push(TcpLink::new(stream)?);
        let idx = self.links.len() - 1;
        self.links[idx].node_id = Some(*node_id);
        Ok(idx)
    }

    fn drain(&mut self) {
        self.accept_pending();
        let mut inbox = VecDeque::new();
        for link in self.links.iter_mut() {
            link.pull_frames(&mut inbox);
        }
        self.inbox.append(&mut inbox);
    }
}

impl MeshTransport for TcpMeshTransport {
    fn send(&mut self, node_id: &NodeId, msg: &TransportMsg) -> Result<(), ProtoError> {
        self.accept_pending();
        let idx = self.ensure_link(node_id)?;
        let mut packet = [0u8; TCP_MAX];
        let body_len = encode_envelope(&self.local_id, msg, &mut packet[2..])?;
        let total = 2 + body_len;
        packet[0..2].copy_from_slice(&(body_len as u16).to_le_bytes());
        write_timeout(
            &mut self.links[idx].stream,
            &packet[..total],
            Duration::from_millis(200),
        )
    }

    fn recv(&mut self) -> Option<(NodeId, TransportMsg)> {
        self.drain();
        self.inbox.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use microkernel::fabric_proto::{MsgType, WireMsg};
    use microkernel::mesh::{MeshRouter, MAX_HOPS};
    use microkernel::node_identity::TrustLevel;

    fn node(byte: u8) -> NodeId {
        [byte; NODE_ID_LEN]
    }

    fn ping_pair<T: MeshTransport>(left: &mut T, right: &mut T, left_id: NodeId, right_id: NodeId) {
        let mut sender = MeshRouter::new();
        let mut receiver = MeshRouter::new();
        sender.init(&left_id);
        receiver.init(&right_id);
        sender.add_direct_peer(&right_id, 10, TrustLevel::Verified, 1);
        receiver.add_direct_peer(&left_id, 10, TrustLevel::Verified, 1);

        let wire = WireMsg::build(MsgType::Ack, b"ping").unwrap();
        assert!(sender.enqueue(&right_id, &wire, 1, 2));
        assert!(sender.flush_next(left).unwrap());

        let deadline = Instant::now() + Duration::from_millis(500);
        let delivered = loop {
            if let Some((from, payload)) = receiver.recv_from(right, 3) {
                assert_eq!(from, left_id);
                break payload;
            }
            if Instant::now() > deadline {
                panic!("timed out waiting for mesh ping");
            }
            std::thread::sleep(Duration::from_millis(5));
        };

        let parsed = WireMsg::from_bytes(delivered.as_slice()).unwrap();
        assert_eq!(parsed.header().unwrap().msg_type, MsgType::Ack);
        assert_eq!(parsed.payload(), b"ping");

        let _ = MAX_HOPS;
    }

    #[test]
    fn udp_mesh_ping_two_sockets() {
        let left_id = node(0xA1);
        let right_id = node(0xB2);
        let mut left = UdpMeshTransport::bind("127.0.0.1:0".parse().unwrap(), left_id).unwrap();
        let mut right = UdpMeshTransport::bind("127.0.0.1:0".parse().unwrap(), right_id).unwrap();
        let left_addr = left.local_addr().unwrap();
        let right_addr = right.local_addr().unwrap();
        left.map_peer(right_id, right_addr);
        right.map_peer(left_id, left_addr);
        ping_pair(&mut left, &mut right, left_id, right_id);
    }

    #[test]
    fn tcp_mesh_ping_two_sockets() {
        let left_id = node(0xC3);
        let right_id = node(0xD4);
        let mut right = TcpMeshTransport::bind("127.0.0.1:0".parse().unwrap(), right_id).unwrap();
        let right_addr = right.local_addr().unwrap();
        let mut left = TcpMeshTransport::bind("127.0.0.1:0".parse().unwrap(), left_id).unwrap();
        left.map_peer(right_id, right_addr);
        ping_pair(&mut left, &mut right, left_id, right_id);
    }
}

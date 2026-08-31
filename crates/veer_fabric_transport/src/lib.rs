//! VeerOS Fabric transport runtime.
//!
//! Phase B transport substrate with:
//! - encrypted packets (ChaCha20-Poly1305)
//! - multiplexed logical streams
//! - resumable session snapshot/restore

use std::collections::{HashMap, VecDeque};

use crypto::chacha20::ChaCha20Poly1305;
use crypto::Aead;
use serde::{Deserialize, Serialize};

const MAGIC: [u8; 4] = *b"VFT1";
const HEADER_LEN: usize = 4 + 4 + 8 + 1 + 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum FrameFlags {
    Data = 0,
    Open = 1,
    Close = 2,
}

impl FrameFlags {
    fn from_u8(v: u8) -> Result<Self, TransportError> {
        match v {
            0 => Ok(FrameFlags::Data),
            1 => Ok(FrameFlags::Open),
            2 => Ok(FrameFlags::Close),
            _ => Err(TransportError::InvalidFrame),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub stream_id: u32,
    pub seq: u64,
    pub flags: FrameFlags,
    pub payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub next_stream_id: u32,
    pub tx_seq: u64,
    pub rx_seq: u64,
    pub streams: Vec<StreamSnapshot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamSnapshot {
    pub stream_id: u32,
    pub closed: bool,
}

#[derive(Debug)]
struct StreamState {
    recv_queue: VecDeque<Frame>,
    closed: bool,
}

pub struct FabricSession {
    aead: ChaCha20Poly1305,
    next_stream_id: u32,
    tx_seq: u64,
    rx_seq: u64,
    streams: HashMap<u32, StreamState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    InvalidKeyLength,
    Crypto,
    InvalidFrame,
    StreamNotFound,
    StreamClosed,
    SequenceMismatch,
    PayloadTooLarge,
    Io(String),
}

impl FabricSession {
    pub fn new(key: &[u8]) -> Result<Self, TransportError> {
        if key.len() != ChaCha20Poly1305::KEY_LEN {
            return Err(TransportError::InvalidKeyLength);
        }

        Ok(Self {
            aead: ChaCha20Poly1305::new(key),
            next_stream_id: 1,
            tx_seq: 0,
            rx_seq: 0,
            streams: HashMap::new(),
        })
    }

    pub fn from_snapshot(key: &[u8], snapshot: SessionSnapshot) -> Result<Self, TransportError> {
        let mut s = Self::new(key)?;
        s.next_stream_id = snapshot.next_stream_id;
        s.tx_seq = snapshot.tx_seq;
        s.rx_seq = snapshot.rx_seq;
        s.streams.extend(snapshot.streams.into_iter().map(|stream| {
            (
                stream.stream_id,
                StreamState {
                    recv_queue: VecDeque::new(),
                    closed: stream.closed,
                },
            )
        }));
        Ok(s)
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            next_stream_id: self.next_stream_id,
            tx_seq: self.tx_seq,
            rx_seq: self.rx_seq,
            streams: self
                .streams
                .iter()
                .map(|(&stream_id, state)| StreamSnapshot {
                    stream_id,
                    closed: state.closed,
                })
                .collect(),
        }
    }

    pub fn open_stream(&mut self) -> u32 {
        let stream_id = self.next_stream_id;
        self.next_stream_id = self.next_stream_id.saturating_add(1);
        self.streams.entry(stream_id).or_insert(StreamState {
            recv_queue: VecDeque::new(),
            closed: false,
        });
        stream_id
    }

    pub fn close_stream(&mut self, stream_id: u32) -> Result<Vec<u8>, TransportError> {
        self.ensure_stream_open(stream_id)?;
        self.streams.get_mut(&stream_id).expect("checked").closed = true;
        self.encode_and_encrypt(stream_id, FrameFlags::Close, &[])
    }

    pub fn send(&mut self, stream_id: u32, payload: &[u8]) -> Result<Vec<u8>, TransportError> {
        self.ensure_stream_open(stream_id)?;
        self.encode_and_encrypt(stream_id, FrameFlags::Data, payload)
    }

    pub fn receive(&mut self, packet: &[u8]) -> Result<Frame, TransportError> {
        let frame = self.decrypt_and_decode(packet)?;

        if frame.seq != self.rx_seq {
            return Err(TransportError::SequenceMismatch);
        }
        self.rx_seq = self.rx_seq.saturating_add(1);

        let state = self.streams.entry(frame.stream_id).or_insert(StreamState {
            recv_queue: VecDeque::new(),
            closed: false,
        });
        if state.closed {
            return Err(TransportError::StreamClosed);
        }

        if frame.flags == FrameFlags::Close {
            state.closed = true;
        }
        state.recv_queue.push_back(frame.clone());
        Ok(frame)
    }

    pub fn pop_stream_frame(&mut self, stream_id: u32) -> Result<Option<Frame>, TransportError> {
        let state = self
            .streams
            .get_mut(&stream_id)
            .ok_or(TransportError::StreamNotFound)?;
        Ok(state.recv_queue.pop_front())
    }

    pub fn is_stream_closed(&self, stream_id: u32) -> Result<bool, TransportError> {
        let state = self
            .streams
            .get(&stream_id)
            .ok_or(TransportError::StreamNotFound)?;
        Ok(state.closed)
    }

    fn ensure_stream_open(&self, stream_id: u32) -> Result<(), TransportError> {
        let state = self
            .streams
            .get(&stream_id)
            .ok_or(TransportError::StreamNotFound)?;
        if state.closed {
            return Err(TransportError::StreamClosed);
        }
        Ok(())
    }

    fn encode_and_encrypt(
        &mut self,
        stream_id: u32,
        flags: FrameFlags,
        payload: &[u8],
    ) -> Result<Vec<u8>, TransportError> {
        if payload.len() > u16::MAX as usize {
            return Err(TransportError::PayloadTooLarge);
        }

        let seq = self.tx_seq;
        self.tx_seq = self.tx_seq.saturating_add(1);

        let header = encode_header(stream_id, seq, flags, payload.len() as u16);
        let nonce = nonce_for_seq(seq, 0);

        let mut sealed = vec![0u8; payload.len() + ChaCha20Poly1305::TAG_LEN];
        sealed[..payload.len()].copy_from_slice(payload);
        let out_len = self
            .aead
            .seal_in_place(&nonce, &header, &mut sealed, payload.len())
            .map_err(|_| TransportError::Crypto)?;
        sealed.truncate(out_len);

        let mut out = Vec::with_capacity(header.len() + sealed.len());
        out.extend_from_slice(&header);
        out.extend_from_slice(&sealed);
        Ok(out)
    }

    fn decrypt_and_decode(&self, packet: &[u8]) -> Result<Frame, TransportError> {
        if packet.len() < HEADER_LEN + ChaCha20Poly1305::TAG_LEN {
            return Err(TransportError::InvalidFrame);
        }

        let (stream_id, seq, flags, payload_len, header) = decode_header(packet)?;

        let nonce = nonce_for_seq(seq, 0);
        let mut body = packet[HEADER_LEN..].to_vec();
        let body_len = body.len();
        let pt_len = self
            .aead
            .open_in_place(&nonce, &header, &mut body, body_len)
            .map_err(|_| TransportError::Crypto)?;
        body.truncate(pt_len);

        if body.len() != payload_len as usize {
            return Err(TransportError::InvalidFrame);
        }

        Ok(Frame {
            stream_id,
            seq,
            flags,
            payload: body,
        })
    }
}

#[cfg(feature = "quic-backend")]
pub mod quic_backend {
    use std::future::Future;
    use std::io::ErrorKind;
    use std::pin::Pin;

    use quinn::{Connection, RecvStream, SendStream};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

    use super::{FabricSession, Frame, SessionSnapshot, TransportError};

    pub const MESH_NODE_ID_LEN: usize = 32;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct MeshEnvelope {
        pub node_id: [u8; MESH_NODE_ID_LEN],
        pub payload: Vec<u8>,
    }

    /// Backend-agnostic async transport boundary for host-side mesh bridges.
    pub trait AsyncMeshBridgeTransport {
        fn send_to<'a>(
            &'a mut self,
            node_id: &'a [u8; MESH_NODE_ID_LEN],
            payload: &'a [u8],
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>>;

        fn recv_from<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = Result<Option<MeshEnvelope>, TransportError>> + Send + 'a>>;

        fn finish_send<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>>;
    }

    /// Generic host-side adapter that presents mesh-like operations over any
    /// async bridge transport backend.
    pub struct HostMeshAdapter<T: AsyncMeshBridgeTransport> {
        transport: T,
        local_node_id: [u8; MESH_NODE_ID_LEN],
    }

    impl<T: AsyncMeshBridgeTransport> HostMeshAdapter<T> {
        pub fn new(transport: T, local_node_id: [u8; MESH_NODE_ID_LEN]) -> Self {
            Self {
                transport,
                local_node_id,
            }
        }

        pub fn local_node_id(&self) -> [u8; MESH_NODE_ID_LEN] {
            self.local_node_id
        }

        pub fn transport(&self) -> &T {
            &self.transport
        }

        pub fn transport_mut(&mut self) -> &mut T {
            &mut self.transport
        }

        pub async fn send(
            &mut self,
            node_id: &[u8; MESH_NODE_ID_LEN],
            payload: &[u8],
        ) -> Result<(), TransportError> {
            self.transport.send_to(node_id, payload).await
        }

        pub async fn recv_any(&mut self) -> Result<Option<MeshEnvelope>, TransportError> {
            self.transport.recv_from().await
        }

        /// Receive the next envelope addressed to this adapter's local node ID.
        /// Envelopes for other nodes are skipped.
        pub async fn recv_for_local(&mut self) -> Result<Option<MeshEnvelope>, TransportError> {
            loop {
                match self.transport.recv_from().await? {
                    Some(envelope) if envelope.node_id == self.local_node_id => {
                        return Ok(Some(envelope));
                    }
                    Some(_) => continue,
                    None => return Ok(None),
                }
            }
        }

        pub async fn finish(&mut self) -> Result<(), TransportError> {
            self.transport.finish_send().await
        }
    }

    /// QUIC-backed runtime wrapper around `FabricSession`.
    ///
    /// QUIC handles transport security/reliability/multiplexing on the wire.
    /// `FabricSession` provides Veer-specific framing, sequencing, and
    /// resumable state semantics.
    pub struct QuicFabricRuntime {
        connection: Connection,
        session: FabricSession,
    }

    /// Host-side bridge that pins one QUIC bi-stream and exposes
    /// node-addressed send/recv operations for mesh integration.
    pub struct QuicMeshBridge {
        runtime: QuicFabricRuntime,
        stream_id: u32,
        send: SendStream,
        recv: RecvStream,
        max_len: usize,
    }

    impl QuicFabricRuntime {
        pub fn new(connection: Connection, key: &[u8]) -> Result<Self, TransportError> {
            let session = FabricSession::new(key)?;
            Ok(Self {
                connection,
                session,
            })
        }

        pub fn from_snapshot(
            connection: Connection,
            key: &[u8],
            snapshot: SessionSnapshot,
        ) -> Result<Self, TransportError> {
            let session = FabricSession::from_snapshot(key, snapshot)?;
            Ok(Self {
                connection,
                session,
            })
        }

        pub fn snapshot(&self) -> SessionSnapshot {
            self.session.snapshot()
        }

        pub fn open_logical_stream(&mut self) -> u32 {
            self.session.open_stream()
        }

        pub async fn open_quic_bi(&self) -> Result<(SendStream, RecvStream), TransportError> {
            self.connection
                .open_bi()
                .await
                .map_err(|e| TransportError::Io(format!("open_bi failed: {e}")))
        }

        pub async fn accept_quic_bi(&self) -> Result<(SendStream, RecvStream), TransportError> {
            self.connection
                .accept_bi()
                .await
                .map_err(|e| TransportError::Io(format!("accept_bi failed: {e}")))
        }

        pub async fn send_frame(
            &mut self,
            stream_id: u32,
            payload: &[u8],
            send: &mut SendStream,
        ) -> Result<(), TransportError> {
            let packet = self.session.send(stream_id, payload)?;
            self.send_packet(&packet, send).await
        }

        /// Send one node-addressed mesh envelope over a QUIC stream.
        ///
        /// Envelope format: 32-byte destination node id + payload bytes.
        pub async fn send_mesh_envelope(
            &mut self,
            stream_id: u32,
            node_id: &[u8; MESH_NODE_ID_LEN],
            payload: &[u8],
            send: &mut SendStream,
        ) -> Result<(), TransportError> {
            if payload.len() > (u16::MAX as usize).saturating_sub(MESH_NODE_ID_LEN) {
                return Err(TransportError::PayloadTooLarge);
            }

            let mut envelope = Vec::with_capacity(MESH_NODE_ID_LEN + payload.len());
            envelope.extend_from_slice(node_id);
            envelope.extend_from_slice(payload);
            self.send_frame(stream_id, &envelope, send).await
        }

        /// Send one raw Fabric packet over a QUIC stream using length-prefix framing.
        ///
        /// Prefix format: 4-byte little-endian payload length.
        pub async fn send_packet(
            &self,
            packet: &[u8],
            send: &mut SendStream,
        ) -> Result<(), TransportError> {
            Self::write_len_prefixed(packet, send).await
        }

        async fn write_len_prefixed<W: AsyncWrite + Unpin>(
            packet: &[u8],
            writer: &mut W,
        ) -> Result<(), TransportError> {
            if packet.len() > u32::MAX as usize {
                return Err(TransportError::PayloadTooLarge);
            }

            let len = packet.len() as u32;
            writer
                .write_all(&len.to_le_bytes())
                .await
                .map_err(|e| TransportError::Io(format!("send length failed: {e}")))?;
            writer
                .write_all(packet)
                .await
                .map_err(|e| TransportError::Io(format!("send write_all failed: {e}")))?;
            writer
                .flush()
                .await
                .map_err(|e| TransportError::Io(format!("send flush failed: {e}")))?;
            Ok(())
        }

        pub async fn recv_one_frame(
            &mut self,
            recv: &mut RecvStream,
            max_len: usize,
        ) -> Result<Frame, TransportError> {
            match self.recv_next_frame(recv, max_len).await? {
                Some(frame) => Ok(frame),
                None => Err(TransportError::Io("recv stream closed".into())),
            }
        }

        /// Receive the next Fabric frame from a QUIC stream.
        ///
        /// Returns `Ok(None)` when the stream reaches EOF before a new packet header.
        pub async fn recv_next_frame(
            &mut self,
            recv: &mut RecvStream,
            max_len: usize,
        ) -> Result<Option<Frame>, TransportError> {
            match self.recv_next_packet(recv, max_len).await? {
                Some(packet) => self.session.receive(&packet).map(Some),
                None => Ok(None),
            }
        }

        /// Receive the next node-addressed mesh envelope from a QUIC stream.
        pub async fn recv_next_mesh_envelope(
            &mut self,
            recv: &mut RecvStream,
            max_len: usize,
        ) -> Result<Option<MeshEnvelope>, TransportError> {
            let frame = match self.recv_next_frame(recv, max_len).await? {
                Some(frame) => frame,
                None => return Ok(None),
            };

            if frame.payload.len() < MESH_NODE_ID_LEN {
                return Err(TransportError::InvalidFrame);
            }

            let mut node_id = [0u8; MESH_NODE_ID_LEN];
            node_id.copy_from_slice(&frame.payload[..MESH_NODE_ID_LEN]);
            Ok(Some(MeshEnvelope {
                node_id,
                payload: frame.payload[MESH_NODE_ID_LEN..].to_vec(),
            }))
        }

        /// Receive one raw Fabric packet from a QUIC stream using length-prefix framing.
        pub async fn recv_packet(
            &self,
            recv: &mut RecvStream,
            max_len: usize,
        ) -> Result<Vec<u8>, TransportError> {
            match self.recv_next_packet(recv, max_len).await? {
                Some(packet) => Ok(packet),
                None => Err(TransportError::Io("recv stream closed".into())),
            }
        }

        /// Receive one raw Fabric packet from a QUIC stream using length-prefix framing.
        ///
        /// Returns `Ok(None)` when the stream reaches EOF before a new packet header.
        pub async fn recv_next_packet(
            &self,
            recv: &mut RecvStream,
            max_len: usize,
        ) -> Result<Option<Vec<u8>>, TransportError> {
            Self::try_read_len_prefixed(recv, max_len).await
        }

        fn io_error(msg: &str, err: impl core::fmt::Display) -> TransportError {
            TransportError::Io(format!("{msg}: {err}"))
        }

        async fn try_read_len_prefixed<R: AsyncRead + Unpin>(
            reader: &mut R,
            max_len: usize,
        ) -> Result<Option<Vec<u8>>, TransportError> {
            if max_len > u32::MAX as usize {
                return Err(TransportError::PayloadTooLarge);
            }

            let len = match reader.read_u32_le().await {
                Ok(len) => len as usize,
                Err(e) if e.kind() == ErrorKind::UnexpectedEof => return Ok(None),
                Err(e) => return Err(Self::io_error("recv read length failed", e)),
            };

            if len > max_len {
                return Err(TransportError::PayloadTooLarge);
            }

            let mut packet = vec![0u8; len];
            reader
                .read_exact(&mut packet)
                .await
                .map_err(|e| Self::io_error("recv read packet failed", e))?;
            Ok(Some(packet))
        }

        #[cfg(test)]
        async fn read_len_prefixed<R: AsyncRead + Unpin>(
            reader: &mut R,
            max_len: usize,
        ) -> Result<Vec<u8>, TransportError> {
            match Self::try_read_len_prefixed(reader, max_len).await? {
                Some(packet) => Ok(packet),
                None => Err(TransportError::Io("recv stream closed".into())),
            }
        }

        pub fn close_logical_stream(&mut self, stream_id: u32) -> Result<Vec<u8>, TransportError> {
            self.session.close_stream(stream_id)
        }

        pub fn pop_stream_frame(
            &mut self,
            stream_id: u32,
        ) -> Result<Option<Frame>, TransportError> {
            self.session.pop_stream_frame(stream_id)
        }

        pub fn is_stream_closed(&self, stream_id: u32) -> Result<bool, TransportError> {
            self.session.is_stream_closed(stream_id)
        }

        pub fn into_bridge(
            mut self,
            stream: (SendStream, RecvStream),
            max_len: usize,
        ) -> QuicMeshBridge {
            let stream_id = self.open_logical_stream();
            let (send, recv) = stream;
            QuicMeshBridge {
                runtime: self,
                stream_id,
                send,
                recv,
                max_len,
            }
        }
    }

    impl QuicMeshBridge {
        pub async fn from_connection(
            connection: Connection,
            key: &[u8],
            max_len: usize,
        ) -> Result<Self, TransportError> {
            let runtime = QuicFabricRuntime::new(connection, key)?;
            let stream = runtime.open_quic_bi().await?;
            Ok(runtime.into_bridge(stream, max_len))
        }

        pub fn runtime(&self) -> &QuicFabricRuntime {
            &self.runtime
        }

        pub fn runtime_mut(&mut self) -> &mut QuicFabricRuntime {
            &mut self.runtime
        }

        pub fn stream_id(&self) -> u32 {
            self.stream_id
        }

        pub async fn send_to(
            &mut self,
            node_id: &[u8; MESH_NODE_ID_LEN],
            payload: &[u8],
        ) -> Result<(), TransportError> {
            self.runtime
                .send_mesh_envelope(self.stream_id, node_id, payload, &mut self.send)
                .await
        }

        pub async fn recv_from(&mut self) -> Result<Option<MeshEnvelope>, TransportError> {
            self.runtime
                .recv_next_mesh_envelope(&mut self.recv, self.max_len)
                .await
        }

        pub fn max_len(&self) -> usize {
            self.max_len
        }

        pub fn set_max_len(&mut self, max_len: usize) {
            self.max_len = max_len;
        }

        pub async fn finish_send(&mut self) -> Result<(), TransportError> {
            self.send
                .finish()
                .map_err(|e| TransportError::Io(format!("finish send failed: {e}")))
        }

        pub async fn close_logical_stream(&mut self) -> Result<(), TransportError> {
            let pkt = self.runtime.close_logical_stream(self.stream_id)?;
            self.runtime.send_packet(&pkt, &mut self.send).await
        }

        pub async fn flush_next_frame(&mut self) -> Result<Option<Frame>, TransportError> {
            self.runtime
                .recv_next_frame(&mut self.recv, self.max_len)
                .await
        }
    }

    impl AsyncMeshBridgeTransport for QuicMeshBridge {
        fn send_to<'a>(
            &'a mut self,
            node_id: &'a [u8; MESH_NODE_ID_LEN],
            payload: &'a [u8],
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>> {
            Box::pin(async move { QuicMeshBridge::send_to(self, node_id, payload).await })
        }

        fn recv_from<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = Result<Option<MeshEnvelope>, TransportError>> + Send + 'a>> {
            Box::pin(async move { QuicMeshBridge::recv_from(self).await })
        }

        fn finish_send<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>> {
            Box::pin(async move { QuicMeshBridge::finish_send(self).await })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
        use rcgen::generate_simple_self_signed;
        use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
        use std::collections::VecDeque;
        use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
        use std::sync::Arc;
        use tokio::io::duplex;

        struct MockBridge {
            sent: Vec<( [u8; MESH_NODE_ID_LEN], Vec<u8> )>,
            inbox: VecDeque<MeshEnvelope>,
            finished: bool,
        }

        impl MockBridge {
            fn new(inbox: VecDeque<MeshEnvelope>) -> Self {
                Self {
                    sent: Vec::new(),
                    inbox,
                    finished: false,
                }
            }
        }

        impl AsyncMeshBridgeTransport for MockBridge {
            fn send_to<'a>(
                &'a mut self,
                node_id: &'a [u8; MESH_NODE_ID_LEN],
                payload: &'a [u8],
            ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>> {
                Box::pin(async move {
                    self.sent.push((*node_id, payload.to_vec()));
                    Ok(())
                })
            }

            fn recv_from<'a>(
                &'a mut self,
            ) -> Pin<Box<dyn Future<Output = Result<Option<MeshEnvelope>, TransportError>> + Send + 'a>> {
                Box::pin(async move { Ok(self.inbox.pop_front()) })
            }

            fn finish_send<'a>(
                &'a mut self,
            ) -> Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>> {
                Box::pin(async move {
                    self.finished = true;
                    Ok(())
                })
            }
        }

        #[tokio::test]
        async fn len_prefixed_roundtrip_supports_multiple_packets_on_one_stream() {
            let (mut tx, mut rx) = duplex(1024);
            let p1 = b"hello";
            let p2 = b"world";

            QuicFabricRuntime::write_len_prefixed(p1, &mut tx)
                .await
                .unwrap();
            QuicFabricRuntime::write_len_prefixed(p2, &mut tx)
                .await
                .unwrap();

            let r1 = QuicFabricRuntime::read_len_prefixed(&mut rx, 64).await.unwrap();
            let r2 = QuicFabricRuntime::read_len_prefixed(&mut rx, 64).await.unwrap();
            assert_eq!(r1, p1);
            assert_eq!(r2, p2);
        }

        #[tokio::test]
        async fn len_prefixed_read_rejects_packet_above_limit() {
            let (mut tx, mut rx) = duplex(1024);
            let payload = vec![0xAA; 128];

            QuicFabricRuntime::write_len_prefixed(&payload, &mut tx)
                .await
                .unwrap();

            let err = QuicFabricRuntime::read_len_prefixed(&mut rx, 64)
                .await
                .unwrap_err();
            assert_eq!(err, TransportError::PayloadTooLarge);
        }

        #[tokio::test]
        async fn try_read_len_prefixed_returns_none_on_clean_eof() {
            let (tx, mut rx) = duplex(64);
            drop(tx);
            let packet = QuicFabricRuntime::try_read_len_prefixed(&mut rx, 64)
                .await
                .unwrap();
            assert!(packet.is_none());
        }

        #[tokio::test]
        async fn read_len_prefixed_errors_on_clean_eof() {
            let (tx, mut rx) = duplex(64);
            drop(tx);
            let err = QuicFabricRuntime::read_len_prefixed(&mut rx, 64)
                .await
                .unwrap_err();
            assert_eq!(err, TransportError::Io("recv stream closed".into()));
        }

        #[tokio::test]
        async fn quic_runtime_roundtrip_over_local_quinn_connection() {
            let key = [0x11u8; 32];

            let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let cert_der: CertificateDer<'static> = CertificateDer::from(cert.serialize_der().unwrap());
            let key_der = PrivatePkcs8KeyDer::from(cert.serialize_private_key_der());

            let mut server_config = ServerConfig::with_single_cert(
                vec![cert_der.clone()],
                key_der.into(),
            )
            .unwrap();
            server_config.transport_config(Arc::new(TransportConfig::default()));

            let server_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
            let server_endpoint = Endpoint::server(server_config, server_addr).unwrap();
            let bound_addr = server_endpoint.local_addr().unwrap();

            let mut roots = rustls::RootCertStore::empty();
            roots.add(cert_der).unwrap();
            let client_crypto = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();

            let mut client_endpoint = Endpoint::client(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                0,
            )))
            .unwrap();
            let client_config = ClientConfig::new(Arc::new(
                quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto).unwrap(),
            ));
            client_endpoint.set_default_client_config(client_config);

            let server_task = tokio::spawn(async move {
                let incoming = server_endpoint.accept().await.unwrap();
                let connection = incoming.await.unwrap();
                let mut runtime = QuicFabricRuntime::new(connection, &key).unwrap();
                let (_send, mut recv) = runtime.connection.accept_bi().await.unwrap();
                runtime.recv_one_frame(&mut recv, 64 * 1024).await.unwrap()
            });

            let client_conn = client_endpoint
                .connect(bound_addr, "localhost")
                .unwrap()
                .await
                .unwrap();
            let mut client_runtime = QuicFabricRuntime::new(client_conn, &key).unwrap();
            let sid = client_runtime.open_logical_stream();
            let (mut send, _recv) = client_runtime.open_quic_bi().await.unwrap();
            client_runtime
                .send_frame(sid, b"fabric-over-quic", &mut send)
                .await
                .unwrap();

            let frame = server_task.await.unwrap();
            assert_eq!(frame.stream_id, sid);
            assert_eq!(frame.payload, b"fabric-over-quic");
        }

        #[tokio::test]
        async fn quic_runtime_recv_next_frame_reads_multiple_packets_then_eof() {
            let key = [0x22u8; 32];

            let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let cert_der: CertificateDer<'static> = CertificateDer::from(cert.serialize_der().unwrap());
            let key_der = PrivatePkcs8KeyDer::from(cert.serialize_private_key_der());

            let mut server_config = ServerConfig::with_single_cert(
                vec![cert_der.clone()],
                key_der.into(),
            )
            .unwrap();
            server_config.transport_config(Arc::new(TransportConfig::default()));

            let server_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
            let server_endpoint = Endpoint::server(server_config, server_addr).unwrap();
            let bound_addr = server_endpoint.local_addr().unwrap();

            let mut roots = rustls::RootCertStore::empty();
            roots.add(cert_der).unwrap();
            let client_crypto = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();

            let mut client_endpoint = Endpoint::client(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                0,
            )))
            .unwrap();
            let client_config = ClientConfig::new(Arc::new(
                quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto).unwrap(),
            ));
            client_endpoint.set_default_client_config(client_config);

            let server_task = tokio::spawn(async move {
                let incoming = server_endpoint.accept().await.unwrap();
                let connection = incoming.await.unwrap();
                let mut runtime = QuicFabricRuntime::new(connection, &key).unwrap();
                let (_send, mut recv) = runtime.connection.accept_bi().await.unwrap();

                let f1 = runtime.recv_next_frame(&mut recv, 64 * 1024).await.unwrap();
                let f2 = runtime.recv_next_frame(&mut recv, 64 * 1024).await.unwrap();
                let eof = runtime.recv_next_frame(&mut recv, 64 * 1024).await.unwrap();
                (f1, f2, eof)
            });

            let client_conn = client_endpoint
                .connect(bound_addr, "localhost")
                .unwrap()
                .await
                .unwrap();
            let mut client_runtime = QuicFabricRuntime::new(client_conn, &key).unwrap();
            let sid = client_runtime.open_logical_stream();
            let (mut send, _recv) = client_runtime.open_quic_bi().await.unwrap();
            client_runtime.send_frame(sid, b"one", &mut send).await.unwrap();
            client_runtime.send_frame(sid, b"two", &mut send).await.unwrap();
            send.finish().unwrap();

            let (f1, f2, eof) = server_task.await.unwrap();
            let f1 = f1.unwrap();
            let f2 = f2.unwrap();
            assert_eq!(f1.stream_id, sid);
            assert_eq!(f1.payload, b"one");
            assert_eq!(f2.stream_id, sid);
            assert_eq!(f2.payload, b"two");
            assert!(eof.is_none());
        }

        #[tokio::test]
        async fn quic_runtime_mesh_envelope_roundtrip_and_eof() {
            let key = [0x33u8; 32];
            let node_a = [0xA1u8; MESH_NODE_ID_LEN];
            let node_b = [0xB2u8; MESH_NODE_ID_LEN];

            let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let cert_der: CertificateDer<'static> = CertificateDer::from(cert.serialize_der().unwrap());
            let key_der = PrivatePkcs8KeyDer::from(cert.serialize_private_key_der());

            let mut server_config = ServerConfig::with_single_cert(
                vec![cert_der.clone()],
                key_der.into(),
            )
            .unwrap();
            server_config.transport_config(Arc::new(TransportConfig::default()));

            let server_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
            let server_endpoint = Endpoint::server(server_config, server_addr).unwrap();
            let bound_addr = server_endpoint.local_addr().unwrap();

            let mut roots = rustls::RootCertStore::empty();
            roots.add(cert_der).unwrap();
            let client_crypto = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();

            let mut client_endpoint = Endpoint::client(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                0,
            )))
            .unwrap();
            let client_config = ClientConfig::new(Arc::new(
                quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto).unwrap(),
            ));
            client_endpoint.set_default_client_config(client_config);

            let server_task = tokio::spawn(async move {
                let incoming = server_endpoint.accept().await.unwrap();
                let connection = incoming.await.unwrap();
                let mut runtime = QuicFabricRuntime::new(connection, &key).unwrap();
                let (_send, mut recv) = runtime.connection.accept_bi().await.unwrap();

                let e1 = runtime.recv_next_mesh_envelope(&mut recv, 64 * 1024).await.unwrap();
                let e2 = runtime.recv_next_mesh_envelope(&mut recv, 64 * 1024).await.unwrap();
                let eof = runtime.recv_next_mesh_envelope(&mut recv, 64 * 1024).await.unwrap();
                (e1, e2, eof)
            });

            let client_conn = client_endpoint
                .connect(bound_addr, "localhost")
                .unwrap()
                .await
                .unwrap();
            let mut client_runtime = QuicFabricRuntime::new(client_conn, &key).unwrap();
            let sid = client_runtime.open_logical_stream();
            let (mut send, _recv) = client_runtime.open_quic_bi().await.unwrap();
            client_runtime
                .send_mesh_envelope(sid, &node_a, b"intent-1", &mut send)
                .await
                .unwrap();
            client_runtime
                .send_mesh_envelope(sid, &node_b, b"intent-2", &mut send)
                .await
                .unwrap();
            send.finish().unwrap();

            let (e1, e2, eof) = server_task.await.unwrap();
            let e1 = e1.unwrap();
            let e2 = e2.unwrap();
            assert_eq!(e1.node_id, node_a);
            assert_eq!(e1.payload, b"intent-1");
            assert_eq!(e2.node_id, node_b);
            assert_eq!(e2.payload, b"intent-2");
            assert!(eof.is_none());
        }

        #[tokio::test]
        async fn quic_mesh_bridge_send_to_recv_from_roundtrip() {
            let key = [0x44u8; 32];
            let node_a = [0xCAu8; MESH_NODE_ID_LEN];
            let node_b = [0xDBu8; MESH_NODE_ID_LEN];

            let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let cert_der: CertificateDer<'static> = CertificateDer::from(cert.serialize_der().unwrap());
            let key_der = PrivatePkcs8KeyDer::from(cert.serialize_private_key_der());

            let mut server_config = ServerConfig::with_single_cert(
                vec![cert_der.clone()],
                key_der.into(),
            )
            .unwrap();
            server_config.transport_config(Arc::new(TransportConfig::default()));

            let server_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
            let server_endpoint = Endpoint::server(server_config, server_addr).unwrap();
            let bound_addr = server_endpoint.local_addr().unwrap();

            let mut roots = rustls::RootCertStore::empty();
            roots.add(cert_der).unwrap();
            let client_crypto = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();

            let mut client_endpoint = Endpoint::client(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                0,
            )))
            .unwrap();
            let client_config = ClientConfig::new(Arc::new(
                quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto).unwrap(),
            ));
            client_endpoint.set_default_client_config(client_config);

            let server_task = tokio::spawn(async move {
                let incoming = server_endpoint.accept().await.unwrap();
                let connection = incoming.await.unwrap();
                let runtime = QuicFabricRuntime::new(connection, &key).unwrap();
                let stream = runtime.connection.accept_bi().await.unwrap();
                let mut bridge = runtime.into_bridge(stream, 64 * 1024);

                let e1 = bridge.recv_from().await.unwrap();
                let e2 = bridge.recv_from().await.unwrap();
                let eof = bridge.recv_from().await.unwrap();
                (e1, e2, eof)
            });

            let client_conn = client_endpoint
                .connect(bound_addr, "localhost")
                .unwrap()
                .await
                .unwrap();
            let mut bridge = QuicMeshBridge::from_connection(client_conn, &key, 64 * 1024)
                .await
                .unwrap();
            assert!(bridge.stream_id() > 0);
            AsyncMeshBridgeTransport::send_to(&mut bridge, &node_a, b"hello-a")
                .await
                .unwrap();
            AsyncMeshBridgeTransport::send_to(&mut bridge, &node_b, b"hello-b")
                .await
                .unwrap();
            AsyncMeshBridgeTransport::finish_send(&mut bridge)
                .await
                .unwrap();

            let (e1, e2, eof) = server_task.await.unwrap();
            let e1 = e1.unwrap();
            let e2 = e2.unwrap();
            assert_eq!(e1.node_id, node_a);
            assert_eq!(e1.payload, b"hello-a");
            assert_eq!(e2.node_id, node_b);
            assert_eq!(e2.payload, b"hello-b");
            assert!(eof.is_none());
        }

        #[tokio::test]
        async fn host_mesh_adapter_filters_non_local_envelopes() {
            let local = [0xA0u8; MESH_NODE_ID_LEN];
            let other = [0xEEu8; MESH_NODE_ID_LEN];
            let mut inbox = VecDeque::new();
            inbox.push_back(MeshEnvelope {
                node_id: other,
                payload: b"skip-me".to_vec(),
            });
            inbox.push_back(MeshEnvelope {
                node_id: local,
                payload: b"deliver-me".to_vec(),
            });

            let bridge = MockBridge::new(inbox);
            let mut adapter = HostMeshAdapter::new(bridge, local);

            let got = adapter.recv_for_local().await.unwrap().unwrap();
            assert_eq!(got.node_id, local);
            assert_eq!(got.payload, b"deliver-me");

            adapter.send(&other, b"outbound").await.unwrap();
            adapter.finish().await.unwrap();
        }

        #[tokio::test]
        async fn host_mesh_adapter_roundtrip_over_quic_bridge() {
            let key = [0x55u8; 32];
            let server_node = [0xB1u8; MESH_NODE_ID_LEN];
            let other_node = [0x12u8; MESH_NODE_ID_LEN];

            let cert = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let cert_der: CertificateDer<'static> = CertificateDer::from(cert.serialize_der().unwrap());
            let key_der = PrivatePkcs8KeyDer::from(cert.serialize_private_key_der());

            let mut server_config = ServerConfig::with_single_cert(
                vec![cert_der.clone()],
                key_der.into(),
            )
            .unwrap();
            server_config.transport_config(Arc::new(TransportConfig::default()));

            let server_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
            let server_endpoint = Endpoint::server(server_config, server_addr).unwrap();
            let bound_addr = server_endpoint.local_addr().unwrap();

            let mut roots = rustls::RootCertStore::empty();
            roots.add(cert_der).unwrap();
            let client_crypto = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();

            let mut client_endpoint = Endpoint::client(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                0,
            )))
            .unwrap();
            let client_config = ClientConfig::new(Arc::new(
                quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto).unwrap(),
            ));
            client_endpoint.set_default_client_config(client_config);

            let server_task = tokio::spawn(async move {
                let incoming = server_endpoint.accept().await.unwrap();
                let connection = incoming.await.unwrap();
                let runtime = QuicFabricRuntime::new(connection, &key).unwrap();
                let stream = runtime.connection.accept_bi().await.unwrap();
                let bridge = runtime.into_bridge(stream, 64 * 1024);
                let mut adapter = HostMeshAdapter::new(bridge, server_node);
                let env = adapter.recv_for_local().await.unwrap();
                let eof = adapter.recv_for_local().await.unwrap();
                (env, eof)
            });

            let client_conn = client_endpoint
                .connect(bound_addr, "localhost")
                .unwrap()
                .await
                .unwrap();
            let bridge = QuicMeshBridge::from_connection(client_conn, &key, 64 * 1024)
                .await
                .unwrap();
            let mut adapter = HostMeshAdapter::new(bridge, other_node);
            adapter.send(&server_node, b"mesh-adapter-msg").await.unwrap();
            adapter.finish().await.unwrap();

            let (env, eof) = server_task.await.unwrap();
            let env = env.unwrap();
            assert_eq!(env.node_id, server_node);
            assert_eq!(env.payload, b"mesh-adapter-msg");
            assert!(eof.is_none());
        }
    }
}

fn encode_header(
    stream_id: u32,
    seq: u64,
    flags: FrameFlags,
    payload_len: u16,
) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[0..4].copy_from_slice(&MAGIC);
    h[4..8].copy_from_slice(&stream_id.to_le_bytes());
    h[8..16].copy_from_slice(&seq.to_le_bytes());
    h[16] = flags as u8;
    h[17..19].copy_from_slice(&payload_len.to_le_bytes());
    h
}

fn decode_header(
    packet: &[u8],
) -> Result<(u32, u64, FrameFlags, u16, [u8; HEADER_LEN]), TransportError> {
    let mut header = [0u8; HEADER_LEN];
    header.copy_from_slice(&packet[..HEADER_LEN]);

    if header[0..4] != MAGIC {
        return Err(TransportError::InvalidFrame);
    }

    let stream_id = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let seq = u64::from_le_bytes([
        header[8], header[9], header[10], header[11], header[12], header[13], header[14],
        header[15],
    ]);
    let flags = FrameFlags::from_u8(header[16])?;
    let payload_len = u16::from_le_bytes([header[17], header[18]]);

    Ok((stream_id, seq, flags, payload_len, header))
}

fn nonce_for_seq(seq: u64, lane: u32) -> [u8; ChaCha20Poly1305::NONCE_LEN] {
    // 96-bit nonce = 32-bit lane + 64-bit sequence.
    let mut out = [0u8; ChaCha20Poly1305::NONCE_LEN];
    out[0..4].copy_from_slice(&lane.to_le_bytes());
    out[4..12].copy_from_slice(&seq.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        [7u8; 32]
    }

    #[test]
    fn multiplexed_stream_roundtrip() {
        let mut tx = FabricSession::new(&key()).unwrap();
        let mut rx = FabricSession::new(&key()).unwrap();

        let s1 = tx.open_stream();
        let s2 = tx.open_stream();

        let p1 = tx.send(s1, b"hello").unwrap();
        let p2 = tx.send(s2, b"world").unwrap();

        let f1 = rx.receive(&p1).unwrap();
        let f2 = rx.receive(&p2).unwrap();

        assert_eq!(f1.stream_id, s1);
        assert_eq!(f1.payload, b"hello");
        assert_eq!(f2.stream_id, s2);
        assert_eq!(f2.payload, b"world");

        let q1 = rx.pop_stream_frame(s1).unwrap().unwrap();
        let q2 = rx.pop_stream_frame(s2).unwrap().unwrap();
        assert_eq!(q1.payload, b"hello");
        assert_eq!(q2.payload, b"world");
    }

    #[test]
    fn wrong_key_fails_authentication() {
        let mut tx = FabricSession::new(&key()).unwrap();
        let mut bad_key = key();
        bad_key[0] ^= 0x55;
        let mut rx = FabricSession::new(&bad_key).unwrap();

        let s = tx.open_stream();
        let pkt = tx.send(s, b"secret").unwrap();

        let err = rx.receive(&pkt).unwrap_err();
        assert_eq!(err, TransportError::Crypto);
    }

    #[test]
    fn snapshot_restore_preserves_sequence() {
        let mut tx = FabricSession::new(&key()).unwrap();
        let s = tx.open_stream();
        let _ = tx.send(s, b"a").unwrap();
        let _ = tx.send(s, b"b").unwrap();

        let snap = tx.snapshot();
        let mut tx2 = FabricSession::from_snapshot(&key(), snap).unwrap();
        let pkt = tx2.send(s, b"c").unwrap();

        let (_, seq, _, _, _) = decode_header(&pkt).unwrap();
        assert_eq!(seq, 2);
    }

    #[test]
    fn snapshot_restore_preserves_stream_open_state() {
        let mut tx = FabricSession::new(&key()).unwrap();
        let s1 = tx.open_stream();
        let s2 = tx.open_stream();
        let _ = tx.send(s1, b"a").unwrap();

        let snap = tx.snapshot();
        let mut restored = FabricSession::from_snapshot(&key(), snap).unwrap();

        let pkt = restored.send(s2, b"b").unwrap();
        let (stream_id, seq, _, _, _) = decode_header(&pkt).unwrap();
        assert_eq!(stream_id, s2);
        assert_eq!(seq, 1);
    }

    #[test]
    fn snapshot_restore_preserves_closed_stream_state() {
        let mut tx = FabricSession::new(&key()).unwrap();
        let s = tx.open_stream();
        let _ = tx.close_stream(s).unwrap();

        let snap = tx.snapshot();
        let mut restored = FabricSession::from_snapshot(&key(), snap).unwrap();

        let err = restored.send(s, b"after-close").unwrap_err();
        assert_eq!(err, TransportError::StreamClosed);
    }

    #[test]
    fn close_stream_emits_close_flag() {
        let mut tx = FabricSession::new(&key()).unwrap();
        let mut rx = FabricSession::new(&key()).unwrap();

        let s = tx.open_stream();
        let pkt = tx.close_stream(s).unwrap();
        let frame = rx.receive(&pkt).unwrap();
        assert_eq!(frame.flags, FrameFlags::Close);
        assert!(rx.is_stream_closed(s).unwrap());
    }
}

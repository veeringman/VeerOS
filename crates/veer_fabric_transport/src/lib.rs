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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub next_stream_id: u32,
    pub tx_seq: u64,
    pub rx_seq: u64,
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
        Ok(s)
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            next_stream_id: self.next_stream_id,
            tx_seq: self.tx_seq,
            rx_seq: self.rx_seq,
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
    use quinn::{Connection, RecvStream, SendStream};
    use tokio::io::AsyncWriteExt;

    use super::{FabricSession, Frame, SessionSnapshot, TransportError};

    /// QUIC-backed runtime wrapper around `FabricSession`.
    ///
    /// QUIC handles transport security/reliability/multiplexing on the wire.
    /// `FabricSession` provides Veer-specific framing, sequencing, and
    /// resumable state semantics.
    pub struct QuicFabricRuntime {
        connection: Connection,
        session: FabricSession,
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

        pub async fn send_frame(
            &mut self,
            stream_id: u32,
            payload: &[u8],
            send: &mut SendStream,
        ) -> Result<(), TransportError> {
            let packet = self.session.send(stream_id, payload)?;
            send.write_all(&packet)
                .await
                .map_err(|e| TransportError::Io(format!("send write_all failed: {e}")))?;
            send.flush()
                .await
                .map_err(|e| TransportError::Io(format!("send flush failed: {e}")))?;
            Ok(())
        }

        pub async fn recv_one_frame(
            &mut self,
            recv: &mut RecvStream,
            max_len: usize,
        ) -> Result<Frame, TransportError> {
            let packet = recv
                .read_to_end(max_len)
                .await
                .map_err(|e| TransportError::Io(format!("recv read_to_end failed: {e}")))?;
            self.session.receive(&packet)
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
        tx2.streams.insert(
            s,
            StreamState {
                recv_queue: VecDeque::new(),
                closed: false,
            },
        );
        let pkt = tx2.send(s, b"c").unwrap();

        let (_, seq, _, _, _) = decode_header(&pkt).unwrap();
        assert_eq!(seq, 2);
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

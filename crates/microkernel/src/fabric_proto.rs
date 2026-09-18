//! Fabric Wire Protocol — binary message format for distributed fabric.
//!
//! Every message that crosses a node boundary uses this format.
//! Fixed-size, `no_alloc`, parseable on a 32-bit MCU in bounded time.
//!
//! # Framing
//!
//! ```text
//! ┌──────┬─────┬──────┬────────┬─────────────┬──────────┐
//! │ 0xVE │ ver │ type │ paylen │  payload     │ HMAC-tag │
//! │ 1B   │ 1B  │ 1B   │ 2B     │  0–480 B    │ 32B      │
//! └──────┴─────┴──────┴────────┴─────────────┴──────────┘
//! ```
//!
//! Total max: 5 (header) + 475 (payload) + 32 (HMAC) = 512 bytes.
//! Fits in a single UDP datagram or BLE characteristic.
//!
//! # Security
//!
//! - Every message carries a 32-byte HMAC-SHA256 integrity tag.
//! - Payload is encrypted with ChaCha20-Poly1305 after session establishment.
//! - Sequence numbers provide replay protection.
//! - No implicit trust — every peer must complete mutual auth first.

// ─── Constants ──────────────────────────────────────────────────────────

/// Magic byte identifying a VeerOS fabric message.
pub const FABRIC_MAGIC: u8 = 0xEE;

/// Current protocol version.
pub const FABRIC_PROTO_VERSION: u8 = 1;

/// Maximum payload size (512 - 5 header - 32 HMAC).
pub const MAX_PAYLOAD_LEN: usize = 475;

/// Header size in bytes.
pub const HEADER_LEN: usize = 5;

/// HMAC tag size in bytes (SHA-256).
pub const TAG_LEN: usize = 32;

/// Maximum total message size.
pub const MAX_MSG_LEN: usize = HEADER_LEN + MAX_PAYLOAD_LEN + TAG_LEN;

/// Node ID size (SHA-256 hash of public key).
pub const NODE_ID_LEN: usize = 32;

// ─── Message types ──────────────────────────────────────────────────────

/// Fabric message type tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    /// Node announces its presence and capabilities.
    NodeAnnounce = 0x01,
    /// Periodic heartbeat with resource snapshot.
    NodeHeartbeat = 0x02,
    /// Forward an intent to a remote node for execution.
    IntentForward = 0x03,
    /// Migrate an agent to a remote node.
    AgentMigrate = 0x04,
    /// Synchronize a persistent memory entry.
    MemorySync = 0x05,
    /// Query a remote node's persistent memory.
    MemoryQuery = 0x06,
    /// Zero-knowledge capability proof.
    CapabilityProof = 0x07,
    /// Authentication challenge (nonce).
    Challenge = 0x10,
    /// Authentication challenge response (signed nonce).
    ChallengeResponse = 0x11,
    /// Session key exchange (KEM ciphertext).
    KeyExchange = 0x12,
    /// Positive acknowledgement.
    Ack = 0xF0,
    /// Negative acknowledgement / error.
    Nack = 0xF1,
}

impl MsgType {
    /// Parse from raw byte, returning None for unknown types.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::NodeAnnounce),
            0x02 => Some(Self::NodeHeartbeat),
            0x03 => Some(Self::IntentForward),
            0x04 => Some(Self::AgentMigrate),
            0x05 => Some(Self::MemorySync),
            0x06 => Some(Self::MemoryQuery),
            0x07 => Some(Self::CapabilityProof),
            0x10 => Some(Self::Challenge),
            0x11 => Some(Self::ChallengeResponse),
            0x12 => Some(Self::KeyExchange),
            0xF0 => Some(Self::Ack),
            0xF1 => Some(Self::Nack),
            _ => None,
        }
    }
}

/// Zero-allocation typed view over a fabric payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FabricMsg<'a> {
    NodeAnnounce(&'a [u8]),
    NodeHeartbeat(&'a [u8]),
    IntentForward(&'a [u8]),
    AgentMigrate(&'a [u8]),
    MemorySync(&'a [u8]),
    MemoryQuery(&'a [u8]),
    CapabilityProof(&'a [u8]),
    Challenge(&'a [u8]),
    ChallengeResponse(&'a [u8]),
    KeyExchange(&'a [u8]),
    Ack(&'a [u8]),
    Nack(&'a [u8]),
}

impl<'a> FabricMsg<'a> {
    /// Return the on-wire message type tag for this payload.
    pub const fn msg_type(&self) -> MsgType {
        match self {
            Self::NodeAnnounce(_) => MsgType::NodeAnnounce,
            Self::NodeHeartbeat(_) => MsgType::NodeHeartbeat,
            Self::IntentForward(_) => MsgType::IntentForward,
            Self::AgentMigrate(_) => MsgType::AgentMigrate,
            Self::MemorySync(_) => MsgType::MemorySync,
            Self::MemoryQuery(_) => MsgType::MemoryQuery,
            Self::CapabilityProof(_) => MsgType::CapabilityProof,
            Self::Challenge(_) => MsgType::Challenge,
            Self::ChallengeResponse(_) => MsgType::ChallengeResponse,
            Self::KeyExchange(_) => MsgType::KeyExchange,
            Self::Ack(_) => MsgType::Ack,
            Self::Nack(_) => MsgType::Nack,
        }
    }

    /// Borrow the raw payload bytes for this message.
    pub const fn payload(&self) -> &'a [u8] {
        match self {
            Self::NodeAnnounce(payload)
            | Self::NodeHeartbeat(payload)
            | Self::IntentForward(payload)
            | Self::AgentMigrate(payload)
            | Self::MemorySync(payload)
            | Self::MemoryQuery(payload)
            | Self::CapabilityProof(payload)
            | Self::Challenge(payload)
            | Self::ChallengeResponse(payload)
            | Self::KeyExchange(payload)
            | Self::Ack(payload)
            | Self::Nack(payload) => payload,
        }
    }

    /// Encode this typed message into a complete wire frame.
    pub fn encode(&self) -> Result<WireMsg, ProtoError> {
        WireMsg::build(self.msg_type(), self.payload())
    }

    /// Decode a typed message view from a parsed wire frame.
    pub fn decode(wire: &'a WireMsg) -> Result<Self, ProtoError> {
        let header = wire.header()?;
        Self::from_parts(header.msg_type, wire.payload())
    }

    /// Decode a typed message view directly from bytes.
    pub fn decode_bytes(buf: &'a [u8]) -> Result<Self, ProtoError> {
        if buf.len() < HEADER_LEN + TAG_LEN {
            return Err(ProtoError::BufferTooSmall);
        }

        let header = MsgHeader::decode(buf)?;
        let total_len = HEADER_LEN + header.payload_len as usize + TAG_LEN;
        if buf.len() < total_len {
            return Err(ProtoError::BufferTooSmall);
        }

        let payload = &buf[HEADER_LEN..HEADER_LEN + header.payload_len as usize];
        Self::from_parts(header.msg_type, payload)
    }

    fn from_parts(msg_type: MsgType, payload: &'a [u8]) -> Result<Self, ProtoError> {
        Ok(match msg_type {
            MsgType::NodeAnnounce => Self::NodeAnnounce(payload),
            MsgType::NodeHeartbeat => Self::NodeHeartbeat(payload),
            MsgType::IntentForward => Self::IntentForward(payload),
            MsgType::AgentMigrate => Self::AgentMigrate(payload),
            MsgType::MemorySync => Self::MemorySync(payload),
            MsgType::MemoryQuery => Self::MemoryQuery(payload),
            MsgType::CapabilityProof => Self::CapabilityProof(payload),
            MsgType::Challenge => Self::Challenge(payload),
            MsgType::ChallengeResponse => Self::ChallengeResponse(payload),
            MsgType::KeyExchange => Self::KeyExchange(payload),
            MsgType::Ack => Self::Ack(payload),
            MsgType::Nack => Self::Nack(payload),
        })
    }
}

// ─── Message header ─────────────────────────────────────────────────────

/// Parsed message header.
#[derive(Debug, Clone, Copy)]
pub struct MsgHeader {
    /// Protocol version.
    pub version: u8,
    /// Message type.
    pub msg_type: MsgType,
    /// Payload length in bytes.
    pub payload_len: u16,
}

impl MsgHeader {
    /// Encode header into a 5-byte buffer.
    pub fn encode(&self, buf: &mut [u8; HEADER_LEN]) {
        buf[0] = FABRIC_MAGIC;
        buf[1] = self.version;
        buf[2] = self.msg_type as u8;
        buf[3] = (self.payload_len >> 8) as u8;
        buf[4] = (self.payload_len & 0xFF) as u8;
    }

    /// Decode header from a 5-byte buffer.
    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        if buf.len() < HEADER_LEN {
            return Err(ProtoError::BufferTooSmall);
        }
        if buf[0] != FABRIC_MAGIC {
            return Err(ProtoError::BadMagic);
        }
        if buf[1] != FABRIC_PROTO_VERSION {
            return Err(ProtoError::UnsupportedVersion);
        }
        let msg_type = MsgType::from_u8(buf[2]).ok_or(ProtoError::UnknownMsgType)?;
        let payload_len = ((buf[3] as u16) << 8) | (buf[4] as u16);
        if payload_len as usize > MAX_PAYLOAD_LEN {
            return Err(ProtoError::PayloadTooLarge);
        }
        Ok(Self {
            version: buf[1],
            msg_type,
            payload_len,
        })
    }
}

// ─── Wire message (complete frame) ─────────────────────────────────────

/// A complete fabric message in wire format.
///
/// This is a fixed-size buffer suitable for static allocation.
/// The actual content occupies `HEADER_LEN + payload_len + TAG_LEN` bytes.
pub struct WireMsg {
    /// Raw bytes: header + payload + HMAC tag.
    pub buf: [u8; MAX_MSG_LEN],
    /// Total valid bytes in buf.
    pub len: usize,
}

/// Per-peer sequence state used for replay-protected Fabric messaging.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceState {
    next_tx_seq: u32,
    next_rx_seq: u32,
}

impl SequenceState {
    pub const fn new() -> Self {
        Self {
            next_tx_seq: 0,
            next_rx_seq: 0,
        }
    }

    /// Sequence number that will be assigned to the next outbound message.
    pub const fn next_tx_seq(&self) -> u32 {
        self.next_tx_seq
    }

    /// Sequence number expected on the next inbound message.
    pub const fn next_rx_seq(&self) -> u32 {
        self.next_rx_seq
    }

    /// Sign a message using the current outbound sequence number and advance it.
    pub fn sign_outgoing(&mut self, msg: &mut WireMsg, key: &[u8]) -> u32 {
        let seq = self.next_tx_seq;
        msg.sign(key, seq);
        self.next_tx_seq = self.next_tx_seq.wrapping_add(1);
        seq
    }

    /// Verify a message against the expected inbound sequence number.
    pub fn verify_incoming(&mut self, msg: &WireMsg, key: &[u8], seq: u32) -> Result<(), ProtoError> {
        if seq != self.next_rx_seq {
            return Err(ProtoError::ReplayDetected);
        }
        if !msg.verify(key, seq) {
            return Err(ProtoError::IntegrityFailed);
        }
        self.next_rx_seq = self.next_rx_seq.wrapping_add(1);
        Ok(())
    }
}

impl WireMsg {
    pub const fn empty() -> Self {
        Self {
            buf: [0u8; MAX_MSG_LEN],
            len: 0,
        }
    }

    /// Build a wire message from header + payload.
    ///
    /// The HMAC tag is NOT set here — call `sign()` after building.
    pub fn build(msg_type: MsgType, payload: &[u8]) -> Result<Self, ProtoError> {
        if payload.len() > MAX_PAYLOAD_LEN {
            return Err(ProtoError::PayloadTooLarge);
        }
        let mut msg = Self::empty();
        let header = MsgHeader {
            version: FABRIC_PROTO_VERSION,
            msg_type,
            payload_len: payload.len() as u16,
        };
        let mut hdr_buf = [0u8; HEADER_LEN];
        header.encode(&mut hdr_buf);
        msg.buf[..HEADER_LEN].copy_from_slice(&hdr_buf);
        msg.buf[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
        msg.len = HEADER_LEN + payload.len() + TAG_LEN;
        Ok(msg)
    }

    /// Parse a complete wire message from a byte slice.
    pub fn from_bytes(data: &[u8]) -> Result<Self, ProtoError> {
        if data.len() < HEADER_LEN + TAG_LEN || data.len() > MAX_MSG_LEN {
            return Err(ProtoError::BufferTooSmall);
        }
        let header = MsgHeader::decode(data)?;
        let expected = HEADER_LEN + header.payload_len as usize + TAG_LEN;
        if data.len() < expected {
            return Err(ProtoError::BufferTooSmall);
        }
        let mut msg = Self::empty();
        msg.buf[..expected].copy_from_slice(&data[..expected]);
        msg.len = expected;
        Ok(msg)
    }

    /// Get the header.
    pub fn header(&self) -> Result<MsgHeader, ProtoError> {
        MsgHeader::decode(&self.buf)
    }

    /// Get the payload slice.
    pub fn payload(&self) -> &[u8] {
        let plen = self.len.saturating_sub(HEADER_LEN + TAG_LEN);
        &self.buf[HEADER_LEN..HEADER_LEN + plen]
    }

    /// Get the HMAC tag slice.
    pub fn tag(&self) -> &[u8] {
        let tag_start = self.len.saturating_sub(TAG_LEN);
        &self.buf[tag_start..self.len]
    }

    /// Sign the message: compute HMAC-SHA256 over (header + payload + seq)
    /// and write the tag into the trailing 32 bytes.
    pub fn sign(&mut self, key: &[u8], seq: u32) {
        let data_len = self.len.saturating_sub(TAG_LEN);
        // Build HMAC input: header + payload + 4-byte sequence number
        let mut hmac_input = [0u8; MAX_PAYLOAD_LEN + HEADER_LEN + 4];
        hmac_input[..data_len].copy_from_slice(&self.buf[..data_len]);
        hmac_input[data_len..data_len + 4].copy_from_slice(&seq.to_le_bytes());
        let tag = crypto::hmac_sha256(key, &hmac_input[..data_len + 4]);
        let tag_start = self.len.saturating_sub(TAG_LEN);
        self.buf[tag_start..self.len].copy_from_slice(&tag.bytes[..TAG_LEN]);
    }

    /// Verify the HMAC tag.
    pub fn verify(&self, key: &[u8], seq: u32) -> bool {
        let data_len = self.len.saturating_sub(TAG_LEN);
        let mut hmac_input = [0u8; MAX_PAYLOAD_LEN + HEADER_LEN + 4];
        hmac_input[..data_len].copy_from_slice(&self.buf[..data_len]);
        hmac_input[data_len..data_len + 4].copy_from_slice(&seq.to_le_bytes());
        let expected = crypto::hmac_sha256(key, &hmac_input[..data_len + 4]);
        let tag = self.tag();
        // Constant-time comparison
        let mut diff = 0u8;
        for i in 0..TAG_LEN {
            diff |= tag[i] ^ expected.bytes[i];
        }
        diff == 0
    }

    /// Total bytes to transmit.
    pub fn wire_len(&self) -> usize {
        self.len
    }

    /// Raw bytes to transmit.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

// ─── Payload builders ───────────────────────────────────────────────────

/// Node identity (32 bytes) — SHA-256(public_key).
pub type NodeId = [u8; NODE_ID_LEN];

/// Node announce payload.
///
/// ```text
/// [0..32]   sender node_id
/// [32]      arch (NodeArch as u8)
/// [33]      zone (LocalityZone as u8)
/// [34..38]  capabilities bitmask (u32 LE)
/// [38]      cpu_cores
/// [39..41]  cpu_mhz (u16 LE)
/// [41..45]  ram_kib (u32 LE)
/// [45]      name_len
/// [46..46+name_len] name bytes
/// ```
pub struct AnnouncePayload;

impl AnnouncePayload {
    /// Minimum size of an announce payload (without name).
    pub const MIN_LEN: usize = 46;

    /// Encode an announce payload.
    pub fn encode(
        node_id: &NodeId,
        arch: u8,
        zone: u8,
        capabilities: u32,
        cpu_cores: u8,
        cpu_mhz: u16,
        ram_kib: u32,
        name: &[u8],
        buf: &mut [u8],
    ) -> usize {
        let name_len = name.len().min(24);
        let total = Self::MIN_LEN + name_len;
        if buf.len() < total {
            return 0;
        }
        buf[..32].copy_from_slice(node_id);
        buf[32] = arch;
        buf[33] = zone;
        buf[34..38].copy_from_slice(&capabilities.to_le_bytes());
        buf[38] = cpu_cores;
        buf[39..41].copy_from_slice(&cpu_mhz.to_le_bytes());
        buf[41..45].copy_from_slice(&ram_kib.to_le_bytes());
        buf[45] = name_len as u8;
        buf[46..46 + name_len].copy_from_slice(&name[..name_len]);
        total
    }

    /// Encode an announce payload that also carries the sender Ed25519 public key.
    pub fn encode_with_pubkey(
        node_id: &NodeId,
        arch: u8,
        zone: u8,
        capabilities: u32,
        cpu_cores: u8,
        cpu_mhz: u16,
        ram_kib: u32,
        name: &[u8],
        public_key: &[u8; 32],
        buf: &mut [u8],
    ) -> usize {
        let name_len = name.len().min(24);
        let total = Self::MIN_LEN + name_len + 32;
        if buf.len() < total {
            return 0;
        }
        let written = Self::encode(
            node_id,
            arch,
            zone,
            capabilities,
            cpu_cores,
            cpu_mhz,
            ram_kib,
            name,
            buf,
        );
        if written == 0 {
            return 0;
        }
        buf[written..written + 32].copy_from_slice(public_key);
        total
    }

    /// Decode the optional Ed25519 public key appended after the name field.
    pub fn public_key(payload: &[u8]) -> Option<[u8; 32]> {
        if payload.len() < Self::MIN_LEN {
            return None;
        }
        let name_len = payload[45] as usize;
        let offset = Self::MIN_LEN + name_len;
        if payload.len() < offset + 32 {
            return None;
        }
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&payload[offset..offset + 32]);
        Some(pk)
    }

    /// Decode sender node_id from an announce payload.
    pub fn sender_id(payload: &[u8]) -> Option<&[u8]> {
        if payload.len() < Self::MIN_LEN {
            return None;
        }
        Some(&payload[..32])
    }

    /// Decode arch from announce payload.
    pub fn arch(payload: &[u8]) -> Option<u8> {
        if payload.len() < Self::MIN_LEN {
            return None;
        }
        Some(payload[32])
    }

    /// Decode capabilities bitmask from announce payload.
    pub fn capabilities(payload: &[u8]) -> Option<u32> {
        if payload.len() < Self::MIN_LEN {
            return None;
        }
        Some(u32::from_le_bytes([
            payload[34],
            payload[35],
            payload[36],
            payload[37],
        ]))
    }
}

/// Heartbeat payload.
///
/// ```text
/// [0..32]   sender node_id
/// [32..36]  sequence number (u32 LE)
/// [36]      cpu_load (0–255)
/// [37..39]  active_agents (u16 LE)
/// [39..43]  ram_free_kib (u32 LE)
/// [43]      health (NodeHealth as u8)
/// ```
pub struct HeartbeatPayload;

impl HeartbeatPayload {
    pub const LEN: usize = 44;

    pub fn encode(
        node_id: &NodeId,
        seq: u32,
        cpu_load: u8,
        active_agents: u16,
        ram_free_kib: u32,
        health: u8,
        buf: &mut [u8],
    ) -> usize {
        if buf.len() < Self::LEN {
            return 0;
        }
        buf[..32].copy_from_slice(node_id);
        buf[32..36].copy_from_slice(&seq.to_le_bytes());
        buf[36] = cpu_load;
        buf[37..39].copy_from_slice(&active_agents.to_le_bytes());
        buf[39..43].copy_from_slice(&ram_free_kib.to_le_bytes());
        buf[43] = health;
        Self::LEN
    }

    pub fn sender_id(payload: &[u8]) -> Option<&[u8]> {
        if payload.len() < Self::LEN {
            return None;
        }
        Some(&payload[..32])
    }

    pub fn seq(payload: &[u8]) -> Option<u32> {
        if payload.len() < Self::LEN {
            return None;
        }
        Some(u32::from_le_bytes([
            payload[32],
            payload[33],
            payload[34],
            payload[35],
        ]))
    }

    pub fn cpu_load(payload: &[u8]) -> Option<u8> {
        if payload.len() < Self::LEN {
            return None;
        }
        Some(payload[36])
    }
}

/// Challenge payload (for mutual authentication).
///
/// ```text
/// [0..32]   challenger node_id
/// [32..64]  nonce (32 random bytes)
/// ```
pub struct ChallengePayload;

impl ChallengePayload {
    pub const LEN: usize = 64;

    pub fn encode(node_id: &NodeId, nonce: &[u8; 32], buf: &mut [u8]) -> usize {
        if buf.len() < Self::LEN {
            return 0;
        }
        buf[..32].copy_from_slice(node_id);
        buf[32..64].copy_from_slice(nonce);
        Self::LEN
    }

    pub fn nonce(payload: &[u8]) -> Option<&[u8]> {
        if payload.len() < Self::LEN {
            return None;
        }
        Some(&payload[32..64])
    }
}

/// Challenge response payload.
///
/// ```text
/// [0..32]   responder node_id
/// [32..64]  original nonce (echo back)
/// [64..128] signature over (nonce || challenger_node_id)
/// [128]     signature length
/// ```
pub struct ChallengeResponsePayload;

impl ChallengeResponsePayload {
    pub const MIN_LEN: usize = 129;

    pub fn encode(node_id: &NodeId, nonce: &[u8; 32], signature: &[u8], buf: &mut [u8]) -> usize {
        let sig_len = signature.len().min(64);
        let total = Self::MIN_LEN + sig_len - 1; // -1 because MIN_LEN includes 1 byte for sig_len field, not the sig itself
                                                 // Actually let's keep it simple: fixed layout
        let total = 32 + 32 + sig_len + 1; // node_id + nonce + sig + sig_len_byte
        if buf.len() < total {
            return 0;
        }
        buf[..32].copy_from_slice(node_id);
        buf[32..64].copy_from_slice(nonce);
        buf[64..64 + sig_len].copy_from_slice(&signature[..sig_len]);
        buf[64 + sig_len] = sig_len as u8;
        total
    }
}

// ─── Agent snapshot (for migration) ─────────────────────────────────────

/// Maximum serialized agent snapshot size.
pub const MAX_AGENT_SNAPSHOT_LEN: usize = 384;

/// Serialized agent state for cross-node migration.
///
/// ```text
/// [0]       state (AgentState as u8)
/// [1]       block_reason (u8)
/// [2..66]   goal.description (64 bytes)
/// [66]      goal.desc_len
/// [67]      goal.priority (u8)
/// [68..76]  goal.deadline_tick (u64 LE)
/// [76..84]  goal.budget_ticks (u64 LE)
/// [84..86]  goal.intent_id (u16 LE)
/// [86..94]  ticks_used (u64 LE)
/// [94..96]  replan_count (u16 LE)
/// [96..98]  max_replans (u16 LE)
/// [98]      context_count
/// [99..]    context slots: for each slot:
///             key_len(1) + key(32) + value_len(1) + value(32) = 66 bytes per slot
/// after context:
///           integrity_hash (32 bytes SHA-256 over everything above)
/// ```
pub struct AgentSnapshot;

impl AgentSnapshot {
    /// Fixed overhead before context slots.
    pub const FIXED_LEN: usize = 99;
    /// Per-slot size.
    pub const SLOT_LEN: usize = 66;
    /// Integrity hash size.
    pub const HASH_LEN: usize = 32;

    /// Calculate total size for N context slots.
    pub fn total_len(slot_count: usize) -> usize {
        Self::FIXED_LEN + slot_count * Self::SLOT_LEN + Self::HASH_LEN
    }
}

// ─── Memory sync payload ────────────────────────────────────────────────

/// Memory sync payload for distributed persistent memory.
///
/// ```text
/// [0..32]   origin node_id
/// [32..36]  vector_clock (u32 LE) — logical timestamp for conflict resolution
/// [36]      tag (MemoryTag as u8)
/// [37]      scope (MemoryScope as u8)
/// [38..40]  scope_id (u16 LE)
/// [40]      key_len
/// [41..73]  key (32 bytes, zero-padded)
/// [73]      value_len
/// [74..138] value (64 bytes, zero-padded)
/// [138]     confidence
/// [139]     operation: 0=store, 1=delete
/// ```
pub struct MemorySyncPayload;

impl MemorySyncPayload {
    pub const LEN: usize = 140;

    pub fn encode_store(
        origin: &NodeId,
        vclock: u32,
        tag: u8,
        scope: u8,
        scope_id: u16,
        key: &[u8],
        value: &[u8],
        confidence: u8,
        buf: &mut [u8],
    ) -> usize {
        if buf.len() < Self::LEN {
            return 0;
        }
        buf[..32].copy_from_slice(origin);
        buf[32..36].copy_from_slice(&vclock.to_le_bytes());
        buf[36] = tag;
        buf[37] = scope;
        buf[38..40].copy_from_slice(&scope_id.to_le_bytes());

        let klen = key.len().min(32);
        buf[40] = klen as u8;
        buf[41..41 + klen].copy_from_slice(&key[..klen]);
        // Zero-pad rest of key field
        for b in &mut buf[41 + klen..73] {
            *b = 0;
        }

        let vlen = value.len().min(64);
        buf[73] = vlen as u8;
        buf[74..74 + vlen].copy_from_slice(&value[..vlen]);
        // Zero-pad rest of value field
        for b in &mut buf[74 + vlen..138] {
            *b = 0;
        }

        buf[138] = confidence;
        buf[139] = 0; // store
        Self::LEN
    }
}

// ─── Errors ─────────────────────────────────────────────────────────────

/// Protocol error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtoError {
    /// Buffer too small for operation.
    BufferTooSmall,
    /// Invalid magic byte.
    BadMagic,
    /// Unsupported protocol version.
    UnsupportedVersion,
    /// Unknown message type.
    UnknownMsgType,
    /// Payload exceeds maximum.
    PayloadTooLarge,
    /// HMAC verification failed.
    IntegrityFailed,
    /// Sequence number replay detected.
    ReplayDetected,
    /// Message from untrusted node.
    Untrusted,
    /// Physical transport failed, or the peer has no mapped address.
    Transport,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fabric_msg_roundtrip_preserves_type_and_payload() {
        let payload = [1u8, 2, 3, 4, 5, 6];
        let msg = FabricMsg::IntentForward(&payload);

        let wire = msg.encode().unwrap();
        let decoded = FabricMsg::decode(&wire).unwrap();

        assert_eq!(decoded, FabricMsg::IntentForward(&payload));
        assert_eq!(decoded.msg_type(), MsgType::IntentForward);
        assert_eq!(decoded.payload(), payload);
    }

    #[test]
    fn fabric_msg_decode_bytes_uses_header_payload_length() {
        let payload = [9u8, 8, 7, 6];
        let mut wire = FabricMsg::Challenge(&payload).encode().unwrap();
        wire.sign(b"fabric-test-key", 7);

        let decoded = FabricMsg::decode_bytes(wire.as_bytes()).unwrap();
        assert_eq!(decoded, FabricMsg::Challenge(&payload));
    }

    #[test]
    fn fabric_msg_decode_bytes_rejects_truncated_frame() {
        let payload = [0xAAu8; 3];
        let wire = FabricMsg::Ack(&payload).encode().unwrap();

        let err = FabricMsg::decode_bytes(&wire.as_bytes()[..HEADER_LEN + 1]).unwrap_err();
        assert_eq!(err, ProtoError::BufferTooSmall);
    }

    #[test]
    fn sequence_state_signs_and_accepts_monotonic_messages() {
        let key = b"fabric-test-key";
        let payload = [0x11u8, 0x22, 0x33];
        let mut tx = SequenceState::new();
        let mut rx = SequenceState::new();
        let mut wire = FabricMsg::NodeHeartbeat(&payload).encode().unwrap();

        let seq = tx.sign_outgoing(&mut wire, key);
        assert_eq!(seq, 0);
        assert_eq!(tx.next_tx_seq(), 1);

        rx.verify_incoming(&wire, key, seq).unwrap();
        assert_eq!(rx.next_rx_seq(), 1);
    }

    #[test]
    fn sequence_state_rejects_replayed_messages() {
        let key = b"fabric-test-key";
        let payload = [0x44u8, 0x55];
        let mut tx = SequenceState::new();
        let mut rx = SequenceState::new();
        let mut wire = FabricMsg::Ack(&payload).encode().unwrap();

        let seq = tx.sign_outgoing(&mut wire, key);
        rx.verify_incoming(&wire, key, seq).unwrap();

        let err = rx.verify_incoming(&wire, key, seq).unwrap_err();
        assert_eq!(err, ProtoError::ReplayDetected);
    }

    #[test]
    fn sequence_state_does_not_advance_on_bad_integrity() {
        let key = b"fabric-test-key";
        let payload = [0x77u8, 0x88, 0x99];
        let mut tx = SequenceState::new();
        let mut rx = SequenceState::new();
        let mut wire = FabricMsg::Nack(&payload).encode().unwrap();

        let seq = tx.sign_outgoing(&mut wire, key);
        wire.buf[HEADER_LEN] ^= 0xFF;

        let err = rx.verify_incoming(&wire, key, seq).unwrap_err();
        assert_eq!(err, ProtoError::IntegrityFailed);
        assert_eq!(rx.next_rx_seq(), 0);
    }
}

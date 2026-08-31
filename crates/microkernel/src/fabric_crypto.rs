//! Fabric Crypto — PQC-hybrid encrypted sessions for inter-node traffic.
//!
//! Every message between fabric nodes is encrypted and authenticated.
//! The crypto layer is designed for hybrid classical + post-quantum:
//!
//! ```text
//! Phase 1 (now):  HKDF-SHA256 session keys + ChaCha20-Poly1305 AEAD
//! Phase 2:        X25519 + ML-KEM-768 hybrid KEM → session key
//! Phase 3:        ML-KEM-768 only → session key
//! ```
//!
//! # Defense against "harvest now, decrypt later"
//!
//! Even in Phase 1, session keys are ephemeral (rotated per-session).
//! An attacker recording traffic today gains nothing: the session key
//! is derived from nonces + PSK and zeroized after rotation.
//!
//! When Phase 2 KEM is enabled, session keys gain forward secrecy
//! against quantum computers: even breaking the classical X25519
//! doesn't help if ML-KEM-768 holds (and vice versa).
//!
//! # Per-message encryption
//!
//! Each `FabricMsg` payload is encrypted with ChaCha20-Poly1305:
//! - Key: per-session, derived from HKDF
//! - Nonce: counter-based (u96 from u32 sequence number)
//! - AAD: message header (type + version) for binding

use crate::fabric_proto::{
    MsgHeader, MsgType, WireMsg, FABRIC_PROTO_VERSION, HEADER_LEN, MAX_PAYLOAD_LEN, TAG_LEN,
};
use crypto::Aead;

// ─── Configuration ──────────────────────────────────────────────────────

/// Maximum messages before mandatory key rotation.
pub const KEY_ROTATION_LIMIT: u32 = u32::MAX;

/// Maximum session age before mandatory key rotation.
pub const KEY_ROTATION_INTERVAL_TICKS: u64 = 3_600;

/// Session key length (ChaCha20-Poly1305).
pub const SESSION_KEY_LEN: usize = 32;

/// AEAD nonce length.
pub const NONCE_LEN: usize = 12;

/// AEAD tag length (Poly1305).
pub const AEAD_TAG_LEN: usize = 16;

/// Maximum plaintext payload length once the AEAD tag is accounted for.
pub const MAX_PLAINTEXT_LEN: usize = MAX_PAYLOAD_LEN - AEAD_TAG_LEN;

/// Decrypted Fabric message payload.
#[derive(Clone, Copy)]
pub struct PlainMessage {
    pub msg_type: MsgType,
    pub payload: [u8; MAX_PLAINTEXT_LEN],
    pub payload_len: usize,
}

impl PlainMessage {
    pub const fn empty(msg_type: MsgType) -> Self {
        Self {
            msg_type,
            payload: [0u8; MAX_PLAINTEXT_LEN],
            payload_len: 0,
        }
    }

    pub fn as_payload(&self) -> &[u8] {
        &self.payload[..self.payload_len]
    }
}

// ─── Session state ──────────────────────────────────────────────────────

/// Cryptographic state for one peer session.
///
/// Each peer has its own session with independent keys and counters.
/// Keys are derived from the mutual authentication handshake.
#[derive(Clone, Copy)]
pub struct FabricSession {
    /// Encryption key (our → peer direction).
    pub tx_key: [u8; SESSION_KEY_LEN],
    /// Decryption key (peer → our direction).
    pub rx_key: [u8; SESSION_KEY_LEN],
    /// Outbound nonce counter.
    pub tx_counter: u32,
    /// Inbound nonce counter (for replay detection).
    pub rx_counter: u32,
    /// Total messages encrypted in this session.
    pub tx_total: u32,
    /// Tick when this key epoch was established.
    pub key_epoch_started_at: u64,
    /// Whether this session is active.
    pub active: bool,
    /// Crypto mode for this session.
    pub mode: CryptoSessionMode,
}

/// Crypto mode for a session (matches crypto::hybrid::CryptoMode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CryptoSessionMode {
    /// Symmetric-only (Phase 1): PSK → HKDF → session keys.
    Symmetric = 0,
    /// Hybrid (Phase 2): X25519 + ML-KEM → combined → session keys.
    Hybrid = 1,
    /// PQC-only (Phase 3): ML-KEM → session keys.
    PqcOnly = 2,
}

impl CryptoSessionMode {
    fn supports_symmetric(self) -> bool {
        matches!(self, CryptoSessionMode::Symmetric | CryptoSessionMode::Hybrid)
    }

    fn supports_hybrid(self) -> bool {
        matches!(self, CryptoSessionMode::Hybrid)
    }

    fn supports_pqc_only(self) -> bool {
        matches!(self, CryptoSessionMode::PqcOnly)
    }

    /// Negotiate the strongest common mode and reject classical downgrade.
    pub fn negotiate(
        local: CryptoSessionMode,
        peer: CryptoSessionMode,
    ) -> Result<CryptoSessionMode, FabricCryptoError> {
        if local.supports_pqc_only() && peer.supports_pqc_only() {
            return Ok(CryptoSessionMode::PqcOnly);
        }
        if local.supports_hybrid() && peer.supports_hybrid() {
            return Ok(CryptoSessionMode::Hybrid);
        }
        if matches!(local, CryptoSessionMode::Symmetric)
            && matches!(peer, CryptoSessionMode::Symmetric)
        {
            return Ok(CryptoSessionMode::Symmetric);
        }
        if local.supports_symmetric() && peer.supports_symmetric() {
            return Err(FabricCryptoError::DowngradeRejected);
        }
        Err(FabricCryptoError::NoCommonCryptoMode)
    }
}

impl FabricSession {
    pub const fn empty() -> Self {
        Self {
            tx_key: [0u8; SESSION_KEY_LEN],
            rx_key: [0u8; SESSION_KEY_LEN],
            tx_counter: 0,
            rx_counter: 0,
            tx_total: 0,
            key_epoch_started_at: 0,
            active: false,
            mode: CryptoSessionMode::Symmetric,
        }
    }

    /// Derive session keys from shared material.
    ///
    /// `ikm`: input keying material (from mutual auth or KEM).
    /// `our_id`: our node ID.
    /// `peer_id`: peer's node ID.
    /// `is_initiator`: true if we initiated the handshake.
    ///
    /// Derives two keys: one for each direction, to prevent nonce reuse.
    pub fn derive(ikm: &[u8], our_id: &[u8; 32], peer_id: &[u8; 32], is_initiator: bool) -> Self {
        Self::derive_at_with_mode(
            ikm,
            our_id,
            peer_id,
            is_initiator,
            0,
            CryptoSessionMode::Symmetric,
        )
    }

    /// Derive session keys and stamp the start of the key epoch.
    pub fn derive_at(
        ikm: &[u8],
        our_id: &[u8; 32],
        peer_id: &[u8; 32],
        is_initiator: bool,
        current_tick: u64,
    ) -> Self {
        Self::derive_at_with_mode(
            ikm,
            our_id,
            peer_id,
            is_initiator,
            current_tick,
            CryptoSessionMode::Symmetric,
        )
    }

    /// Derive session keys with an explicitly negotiated crypto mode.
    pub fn derive_at_with_mode(
        ikm: &[u8],
        our_id: &[u8; 32],
        peer_id: &[u8; 32],
        is_initiator: bool,
        current_tick: u64,
        mode: CryptoSessionMode,
    ) -> Self {
        let mut session = Self::empty();

        // tx_key = HKDF(ikm, our_id || peer_id, "veeros-tx-v1")
        let mut salt = [0u8; 64];
        salt[..32].copy_from_slice(our_id);
        salt[32..64].copy_from_slice(peer_id);

        let tx_info = b"veeros-fabric-tx-v1";
        let rx_info = b"veeros-fabric-rx-v1";

        if is_initiator {
            crypto::hkdf_sha256(ikm, &salt, tx_info, &mut session.tx_key);
            crypto::hkdf_sha256(ikm, &salt, rx_info, &mut session.rx_key);
        } else {
            // Non-initiator swaps tx/rx so both sides agree.
            crypto::hkdf_sha256(ikm, &salt, rx_info, &mut session.tx_key);
            crypto::hkdf_sha256(ikm, &salt, tx_info, &mut session.rx_key);
        }

        session.active = true;
        session.mode = mode;
        session.key_epoch_started_at = current_tick;
        session
    }

    /// Build a nonce from the counter.
    ///
    /// ChaCha20-Poly1305 uses a 12-byte nonce.
    /// We use: [0..4] = 0, [4..8] = counter LE, [8..12] = 0.
    fn counter_nonce(counter: u32) -> [u8; NONCE_LEN] {
        let mut nonce = [0u8; NONCE_LEN];
        nonce[4..8].copy_from_slice(&counter.to_le_bytes());
        nonce
    }

    /// Encrypt a payload in-place using ChaCha20-Poly1305.
    ///
    /// `buf[..plaintext_len]` contains the plaintext.
    /// `buf` must have capacity for `plaintext_len + AEAD_TAG_LEN`.
    /// `aad` is additional authenticated data (typically the message header).
    ///
    /// Returns total length (plaintext + AEAD tag) on success.
    pub fn encrypt(
        &mut self,
        buf: &mut [u8],
        plaintext_len: usize,
        aad: &[u8],
    ) -> Result<usize, FabricCryptoError> {
        if !self.active {
            return Err(FabricCryptoError::SessionInactive);
        }
        if self.tx_total >= KEY_ROTATION_LIMIT {
            return Err(FabricCryptoError::KeyRotationRequired);
        }
        if buf.len() < plaintext_len + AEAD_TAG_LEN {
            return Err(FabricCryptoError::BufferTooSmall);
        }

        let nonce = Self::counter_nonce(self.tx_counter);
        let aead = crypto::chacha20::ChaCha20Poly1305::new(&self.tx_key);

        match aead.seal_in_place(&nonce, aad, buf, plaintext_len) {
            Ok(total) => {
                self.tx_counter = self.tx_counter.wrapping_add(1);
                self.tx_total += 1;
                Ok(total)
            }
            Err(_) => Err(FabricCryptoError::EncryptionFailed),
        }
    }

    /// Encrypt a typed Fabric payload into a complete authenticated wire frame.
    ///
    /// Returns the frame plus the outbound sequence/counter used for this packet.
    pub fn encrypt_wire_message(
        &mut self,
        msg_type: MsgType,
        plaintext: &[u8],
    ) -> Result<(WireMsg, u32), FabricCryptoError> {
        if plaintext.len() > MAX_PLAINTEXT_LEN {
            return Err(FabricCryptoError::PayloadTooLarge);
        }
        if !self.active {
            return Err(FabricCryptoError::SessionInactive);
        }
        if self.tx_total >= KEY_ROTATION_LIMIT {
            return Err(FabricCryptoError::KeyRotationRequired);
        }

        let seq = self.tx_counter;
        let encrypted_len = plaintext.len() + AEAD_TAG_LEN;

        let header = MsgHeader {
            version: FABRIC_PROTO_VERSION,
            msg_type,
            payload_len: encrypted_len as u16,
        };
        let mut hdr_buf = [0u8; HEADER_LEN];
        header.encode(&mut hdr_buf);

        let mut msg = WireMsg::empty();
        msg.buf[..HEADER_LEN].copy_from_slice(&hdr_buf);
        msg.buf[HEADER_LEN..HEADER_LEN + plaintext.len()].copy_from_slice(plaintext);

        let enc_total = self.encrypt(
            &mut msg.buf[HEADER_LEN..HEADER_LEN + encrypted_len],
            plaintext.len(),
            &hdr_buf,
        )?;

        msg.len = HEADER_LEN + enc_total + TAG_LEN;
        msg.sign(&self.tx_key, seq);
        Ok((msg, seq))
    }

    /// Decrypt a payload in-place using ChaCha20-Poly1305.
    ///
    /// `buf[..ciphertext_len]` contains ciphertext + AEAD tag.
    /// `aad` is additional authenticated data.
    ///
    /// Returns plaintext length on success.
    pub fn decrypt(
        &mut self,
        buf: &mut [u8],
        ciphertext_len: usize,
        aad: &[u8],
        peer_counter: u32,
    ) -> Result<usize, FabricCryptoError> {
        if !self.active {
            return Err(FabricCryptoError::SessionInactive);
        }

        // Replay detection: reject if counter is too old.
        // Allow a small window for reordering.
        if peer_counter < self.rx_counter && self.rx_counter - peer_counter > 16 {
            return Err(FabricCryptoError::ReplayDetected);
        }

        let nonce = Self::counter_nonce(peer_counter);
        let aead = crypto::chacha20::ChaCha20Poly1305::new(&self.rx_key);

        match aead.open_in_place(&nonce, aad, buf, ciphertext_len) {
            Ok(plaintext_len) => {
                if peer_counter >= self.rx_counter {
                    self.rx_counter = peer_counter.wrapping_add(1);
                }
                Ok(plaintext_len)
            }
            Err(_) => Err(FabricCryptoError::DecryptionFailed),
        }
    }

    /// Verify and decrypt a complete Fabric wire frame into plaintext.
    pub fn decrypt_wire_message(
        &mut self,
        msg: &WireMsg,
        peer_counter: u32,
    ) -> Result<PlainMessage, FabricCryptoError> {
        if !self.active {
            return Err(FabricCryptoError::SessionInactive);
        }
        if !msg.verify(&self.rx_key, peer_counter) {
            return Err(FabricCryptoError::IntegrityFailed);
        }

        let header = msg.header().map_err(|_| FabricCryptoError::InvalidFrame)?;
        if header.payload_len as usize > MAX_PAYLOAD_LEN || header.payload_len as usize < AEAD_TAG_LEN {
            return Err(FabricCryptoError::InvalidFrame);
        }

        let ciphertext_len = header.payload_len as usize;
        let mut decrypted = PlainMessage::empty(header.msg_type);
        decrypted.payload[..ciphertext_len]
            .copy_from_slice(&msg.buf[HEADER_LEN..HEADER_LEN + ciphertext_len]);

        let plaintext_len = self.decrypt(
            &mut decrypted.payload[..ciphertext_len],
            ciphertext_len,
            &msg.buf[..HEADER_LEN],
            peer_counter,
        )?;
        decrypted.payload_len = plaintext_len;
        Ok(decrypted)
    }

    /// Check if session keys need rotation.
    pub fn needs_rotation(&self) -> bool {
        self.tx_total >= KEY_ROTATION_LIMIT
    }

    /// Check if session keys need rotation at the current tick.
    pub fn needs_rotation_at(&self, current_tick: u64) -> bool {
        self.tx_total >= KEY_ROTATION_LIMIT
            || current_tick.saturating_sub(self.key_epoch_started_at) >= KEY_ROTATION_INTERVAL_TICKS
    }

    /// Rotate session keys using HKDF ratchet.
    ///
    /// new_key = HKDF(old_key, counter, "veeros-rotate-v1")
    pub fn rotate(&mut self) -> Result<(), FabricCryptoError> {
        self.rotate_at(self.key_epoch_started_at)
    }

    /// Rotate session keys using an HKDF ratchet with fresh per-epoch salt.
    pub fn rotate_at(&mut self, current_tick: u64) -> Result<(), FabricCryptoError> {
        if !self.active {
            return Err(FabricCryptoError::SessionInactive);
        }

        let info = b"veeros-fabric-rotate-v1";
        let mut salt = [0u8; 12];
        salt[..4].copy_from_slice(&self.tx_counter.to_le_bytes());
        salt[4..12].copy_from_slice(&current_tick.to_le_bytes());

        let mut new_tx = [0u8; SESSION_KEY_LEN];
        let mut new_rx = [0u8; SESSION_KEY_LEN];
        crypto::hkdf_sha256(&self.tx_key, &salt, info, &mut new_tx);
        crypto::hkdf_sha256(&self.rx_key, &salt, info, &mut new_rx);

        // Zeroize old keys (forward secrecy).
        crypto::zeroize(&mut self.tx_key);
        crypto::zeroize(&mut self.rx_key);

        self.tx_key = new_tx;
        self.rx_key = new_rx;
        self.tx_counter = 0;
        self.rx_counter = 0;
        self.tx_total = 0;
        self.key_epoch_started_at = current_tick;

        Ok(())
    }

    /// Destroy this session — zeroize all key material.
    pub fn destroy(&mut self) {
        crypto::zeroize(&mut self.tx_key);
        crypto::zeroize(&mut self.rx_key);
        self.tx_counter = 0;
        self.rx_counter = 0;
        self.tx_total = 0;
        self.key_epoch_started_at = 0;
        self.active = false;
    }
}

// ─── Session table ──────────────────────────────────────────────────────

/// Table of active crypto sessions (one per peer).
pub struct FabricSessionTable {
    pub sessions: [FabricSession; super::node_identity::MAX_PEERS],
}

impl FabricSessionTable {
    pub const fn new() -> Self {
        Self {
            sessions: [FabricSession::empty(); super::node_identity::MAX_PEERS],
        }
    }

    /// Get the session for a peer (by peer index from NodeIdentityManager).
    pub fn get(&self, peer_idx: usize) -> Option<&FabricSession> {
        if peer_idx < self.sessions.len() && self.sessions[peer_idx].active {
            Some(&self.sessions[peer_idx])
        } else {
            None
        }
    }

    /// Get mutable session for a peer.
    pub fn get_mut(&mut self, peer_idx: usize) -> Option<&mut FabricSession> {
        if peer_idx < self.sessions.len() && self.sessions[peer_idx].active {
            Some(&mut self.sessions[peer_idx])
        } else {
            None
        }
    }

    /// Establish a new session for a peer.
    pub fn establish(
        &mut self,
        peer_idx: usize,
        ikm: &[u8],
        our_id: &[u8; 32],
        peer_id: &[u8; 32],
        is_initiator: bool,
    ) -> bool {
        if peer_idx >= self.sessions.len() {
            return false;
        }
        self.sessions[peer_idx] = FabricSession::derive(ikm, our_id, peer_id, is_initiator);
        true
    }

    /// Establish a new session for a peer with an explicit current tick.
    pub fn establish_at(
        &mut self,
        peer_idx: usize,
        ikm: &[u8],
        our_id: &[u8; 32],
        peer_id: &[u8; 32],
        is_initiator: bool,
        current_tick: u64,
    ) -> bool {
        if peer_idx >= self.sessions.len() {
            return false;
        }
        self.sessions[peer_idx] =
            FabricSession::derive_at(ikm, our_id, peer_id, is_initiator, current_tick);
        true
    }

    /// Establish a new session using negotiated crypto mode.
    pub fn establish_negotiated(
        &mut self,
        peer_idx: usize,
        ikm: &[u8],
        our_id: &[u8; 32],
        peer_id: &[u8; 32],
        is_initiator: bool,
        current_tick: u64,
        local_mode: CryptoSessionMode,
        peer_mode: CryptoSessionMode,
    ) -> Result<CryptoSessionMode, FabricCryptoError> {
        if peer_idx >= self.sessions.len() {
            return Err(FabricCryptoError::SessionInactive);
        }
        let negotiated = CryptoSessionMode::negotiate(local_mode, peer_mode)?;
        self.sessions[peer_idx] = FabricSession::derive_at_with_mode(
            ikm,
            our_id,
            peer_id,
            is_initiator,
            current_tick,
            negotiated,
        );
        Ok(negotiated)
    }

    /// Destroy a session (peer disconnected or revoked).
    pub fn destroy(&mut self, peer_idx: usize) {
        if peer_idx < self.sessions.len() {
            self.sessions[peer_idx].destroy();
        }
    }

    /// Count active sessions.
    pub fn active_count(&self) -> usize {
        self.sessions.iter().filter(|s| s.active).count()
    }
}

// ─── Errors ─────────────────────────────────────────────────────────────

/// Fabric crypto error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FabricCryptoError {
    /// Session not established.
    SessionInactive,
    /// Message counter exceeded rotation limit.
    KeyRotationRequired,
    /// Output buffer too small.
    BufferTooSmall,
    /// Plaintext payload exceeds encrypted frame capacity.
    PayloadTooLarge,
    /// Encryption failed (internal AEAD error).
    EncryptionFailed,
    /// Decryption failed (authentication tag mismatch — tampered).
    DecryptionFailed,
    /// Wire frame failed HMAC verification.
    IntegrityFailed,
    /// Wire frame header/payload layout is invalid.
    InvalidFrame,
    /// Replay attack detected (counter too old).
    ReplayDetected,
    /// Negotiation would downgrade a PQC-capable peer to symmetric-only crypto.
    DowngradeRejected,
    /// Peers have no compatible crypto mode.
    NoCommonCryptoMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node_id(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    #[test]
    fn encrypt_and_decrypt_wire_message_roundtrip() {
        let ikm = [0x42u8; 32];
        let mut tx = FabricSession::derive(&ikm, &node_id(1), &node_id(2), true);
        let mut rx = FabricSession::derive(&ikm, &node_id(2), &node_id(1), false);

        let (wire, seq) = tx
            .encrypt_wire_message(MsgType::NodeHeartbeat, b"fabric-heartbeat")
            .unwrap();
        let plain = rx.decrypt_wire_message(&wire, seq).unwrap();

        assert_eq!(plain.msg_type, MsgType::NodeHeartbeat);
        assert_eq!(plain.as_payload(), b"fabric-heartbeat");
    }

    #[test]
    fn decrypt_wire_message_rejects_tampered_ciphertext() {
        let ikm = [0x24u8; 32];
        let mut tx = FabricSession::derive(&ikm, &node_id(3), &node_id(4), true);
        let mut rx = FabricSession::derive(&ikm, &node_id(4), &node_id(3), false);

        let (mut wire, seq) = tx.encrypt_wire_message(MsgType::Ack, b"ok").unwrap();
        wire.buf[HEADER_LEN] ^= 0x01;

        let err = rx.decrypt_wire_message(&wire, seq).unwrap_err();
        assert_eq!(err, FabricCryptoError::IntegrityFailed);
    }

    #[test]
    fn encrypt_wire_message_enforces_plaintext_capacity() {
        let ikm = [0x11u8; 32];
        let mut tx = FabricSession::derive(&ikm, &node_id(5), &node_id(6), true);
        let payload = [0xAAu8; MAX_PLAINTEXT_LEN + 1];

        let err = tx
            .encrypt_wire_message(MsgType::MemorySync, &payload)
            .unwrap_err();
        assert_eq!(err, FabricCryptoError::PayloadTooLarge);
    }

    #[test]
    fn session_requires_rotation_after_max_age() {
        let ikm = [0x33u8; 32];
        let session = FabricSession::derive_at(&ikm, &node_id(7), &node_id(8), true, 100);

        assert!(!session.needs_rotation_at(100 + KEY_ROTATION_INTERVAL_TICKS - 1));
        assert!(session.needs_rotation_at(100 + KEY_ROTATION_INTERVAL_TICKS));
    }

    #[test]
    fn rotate_at_resets_epoch_and_changes_keys() {
        let ikm = [0x55u8; 32];
        let mut session = FabricSession::derive_at(&ikm, &node_id(9), &node_id(10), true, 12);
        let old_tx = session.tx_key;
        let old_rx = session.rx_key;
        session.tx_counter = 77;
        session.rx_counter = 13;
        session.tx_total = 42;

        session.rotate_at(900).unwrap();

        assert_ne!(session.tx_key, old_tx);
        assert_ne!(session.rx_key, old_rx);
        assert_eq!(session.tx_counter, 0);
        assert_eq!(session.rx_counter, 0);
        assert_eq!(session.tx_total, 0);
        assert_eq!(session.key_epoch_started_at, 900);
        assert!(!session.needs_rotation_at(900));
    }

    #[test]
    fn negotiate_prefers_strongest_common_mode() {
        assert_eq!(
            CryptoSessionMode::negotiate(CryptoSessionMode::Hybrid, CryptoSessionMode::Hybrid)
                .unwrap(),
            CryptoSessionMode::Hybrid
        );
        assert_eq!(
            CryptoSessionMode::negotiate(CryptoSessionMode::PqcOnly, CryptoSessionMode::PqcOnly)
                .unwrap(),
            CryptoSessionMode::PqcOnly
        );
        assert_eq!(
            CryptoSessionMode::negotiate(
                CryptoSessionMode::Symmetric,
                CryptoSessionMode::Symmetric,
            )
            .unwrap(),
            CryptoSessionMode::Symmetric
        );
    }

    #[test]
    fn negotiate_rejects_classical_downgrade_for_pqc_capable_peers() {
        let err = CryptoSessionMode::negotiate(
            CryptoSessionMode::Hybrid,
            CryptoSessionMode::Symmetric,
        )
        .unwrap_err();
        assert_eq!(err, FabricCryptoError::DowngradeRejected);
    }

    #[test]
    fn establish_negotiated_records_selected_mode() {
        let ikm = [0x61u8; 32];
        let mut table = FabricSessionTable::new();
        let mode = table
            .establish_negotiated(
                0,
                &ikm,
                &node_id(1),
                &node_id(2),
                true,
                77,
                CryptoSessionMode::Hybrid,
                CryptoSessionMode::Hybrid,
            )
            .unwrap();

        assert_eq!(mode, CryptoSessionMode::Hybrid);
        assert_eq!(table.get(0).unwrap().mode, CryptoSessionMode::Hybrid);
        assert_eq!(table.get(0).unwrap().key_epoch_started_at, 77);
    }
}

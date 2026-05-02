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

use crate::fabric_proto::{HEADER_LEN, MAX_PAYLOAD_LEN, TAG_LEN};
use crypto::Aead;

// ─── Configuration ──────────────────────────────────────────────────────

/// Maximum messages before mandatory key rotation.
pub const KEY_ROTATION_LIMIT: u32 = 1 << 20; // ~1 million messages

/// Session key length (ChaCha20-Poly1305).
pub const SESSION_KEY_LEN: usize = 32;

/// AEAD nonce length.
pub const NONCE_LEN: usize = 12;

/// AEAD tag length (Poly1305).
pub const AEAD_TAG_LEN: usize = 16;

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

impl FabricSession {
    pub const fn empty() -> Self {
        Self {
            tx_key: [0u8; SESSION_KEY_LEN],
            rx_key: [0u8; SESSION_KEY_LEN],
            tx_counter: 0,
            rx_counter: 0,
            tx_total: 0,
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

    /// Check if session keys need rotation.
    pub fn needs_rotation(&self) -> bool {
        self.tx_total >= KEY_ROTATION_LIMIT
    }

    /// Rotate session keys using HKDF ratchet.
    ///
    /// new_key = HKDF(old_key, counter, "veeros-rotate-v1")
    pub fn rotate(&mut self) -> Result<(), FabricCryptoError> {
        if !self.active {
            return Err(FabricCryptoError::SessionInactive);
        }

        let info = b"veeros-fabric-rotate-v1";
        let salt = self.tx_counter.to_le_bytes();

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

        Ok(())
    }

    /// Destroy this session — zeroize all key material.
    pub fn destroy(&mut self) {
        crypto::zeroize(&mut self.tx_key);
        crypto::zeroize(&mut self.rx_key);
        self.tx_counter = 0;
        self.rx_counter = 0;
        self.tx_total = 0;
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
    /// Encryption failed (internal AEAD error).
    EncryptionFailed,
    /// Decryption failed (authentication tag mismatch — tampered).
    DecryptionFailed,
    /// Replay attack detected (counter too old).
    ReplayDetected,
}

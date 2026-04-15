//! Zero Trust Node Identity — cryptographic identity for every fabric node.
//!
//! **Zero Trust principle**: no node is trusted based on network location,
//! physical proximity, or prior relationship.  Every interaction requires
//! cryptographic proof of identity and authorization.
//!
//! # Identity model
//!
//! ```text
//!  ┌─────────────────────────────────────────────────────────────────┐
//!  │  Node Identity                                                  │
//!  │                                                                 │
//!  │  keypair ──► public_key ──► SHA-256 ──► node_id (32 bytes)     │
//!  │                                                                 │
//!  │  attestation_cert = {                                           │
//!  │    node_id, arch, capabilities, zone, timestamp,                │
//!  │    signature(keypair, H(fields))                                │
//!  │  }                                                              │
//!  └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Trust lifecycle
//!
//! ```text
//!  Unknown ──► Untrusted ──► Challenged ──► Verified ──► Attested
//!                                                │           │
//!                                                └───► Revoked
//! ```
//!
//! # PQC-Hybrid support
//!
//! The identity system is designed for hybrid classical + post-quantum:
//! - Phase 1 (now): HMAC-SHA256 symmetric authentication + SHA-256 commitments
//! - Phase 2: Ed25519 + ML-DSA-65 dual signatures on attestation certs
//! - Phase 3: ML-DSA-65 only

use crate::fabric_proto::{NodeId, NODE_ID_LEN};

// ─── Configuration ──────────────────────────────────────────────────────

/// Maximum peers we track identity state for.
#[cfg(feature = "dist-minimal")]
pub const MAX_PEERS: usize = 1;

#[cfg(not(any(feature = "dist-minimal", feature = "dist-full")))]
pub const MAX_PEERS: usize = 8;

#[cfg(feature = "dist-full")]
pub const MAX_PEERS: usize = 64;

/// Maximum length of a pre-shared key (for Phase 1 symmetric auth).
pub const MAX_PSK_LEN: usize = 32;

/// Challenge nonce size.
pub const NONCE_LEN: usize = 32;

/// Attestation certificate max size.
pub const MAX_CERT_LEN: usize = 128;

// ─── Trust levels ───────────────────────────────────────────────────────

/// Trust level of a peer node.
///
/// Trust is earned through cryptographic verification, never assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum TrustLevel {
    /// Node has not completed any authentication.
    Untrusted   = 0,
    /// Challenge has been sent, awaiting response.
    Challenged  = 1,
    /// Mutual authentication completed (challenge-response verified).
    Verified    = 2,
    /// ZKP capability proof verified — highest trust.
    Attested    = 3,
    /// Identity has been revoked — reject all messages.
    Revoked     = 255,
}

// ─── Peer identity record ───────────────────────────────────────────────

/// Per-peer cryptographic identity and session state.
#[derive(Clone, Copy)]
pub struct PeerIdentity {
    /// Peer's node ID (SHA-256 of their public key).
    pub node_id: NodeId,
    /// Whether this slot is in use.
    pub active: bool,
    /// Current trust level.
    pub trust: TrustLevel,
    /// Outbound sequence number (for replay protection).
    pub tx_seq: u32,
    /// Expected inbound sequence number.
    pub rx_seq: u32,
    /// Session key for this peer (ChaCha20-Poly1305).
    /// Derived from HKDF after mutual authentication.
    pub session_key: [u8; 32],
    /// Whether session key is established.
    pub session_active: bool,
    /// Our challenge nonce (for verifying their response).
    pub our_nonce: [u8; NONCE_LEN],
    /// Tick at which this peer was last seen.
    pub last_seen: u64,
    /// Tick at which trust was established.
    pub trust_since: u64,
    /// Number of failed auth attempts from this peer.
    pub failed_auths: u8,
    /// Peer's reported architecture.
    pub arch: u8,
    /// Peer's reported capabilities (verified via ZKP at Attested level).
    pub capabilities: u32,
}

impl PeerIdentity {
    pub const fn empty() -> Self {
        Self {
            node_id: [0u8; NODE_ID_LEN],
            active: false,
            trust: TrustLevel::Untrusted,
            tx_seq: 0,
            rx_seq: 0,
            session_key: [0u8; 32],
            session_active: false,
            our_nonce: [0u8; NONCE_LEN],
            last_seen: 0,
            trust_since: 0,
            failed_auths: 0,
            arch: 255,
            capabilities: 0,
        }
    }
}

// ─── Local node identity ────────────────────────────────────────────────

/// Attestation certificate — self-signed proof of node identity.
///
/// ```text
/// [0..32]   node_id
/// [32]      arch
/// [33..37]  capabilities (u32 LE)
/// [37]      zone
/// [38..46]  timestamp (u64 LE)
/// [46..78]  HMAC-SHA256 over [0..46] using node secret
/// ```
pub struct AttestationCert {
    pub data: [u8; MAX_CERT_LEN],
    pub len: usize,
}

impl AttestationCert {
    pub const SIGNED_LEN: usize = 78;

    pub const fn empty() -> Self {
        Self {
            data: [0u8; MAX_CERT_LEN],
            len: 0,
        }
    }

    /// Build a self-signed attestation certificate.
    pub fn build(
        node_id: &NodeId,
        arch: u8,
        capabilities: u32,
        zone: u8,
        timestamp: u64,
        secret: &[u8; 32],
    ) -> Self {
        let mut cert = Self::empty();
        cert.data[..32].copy_from_slice(node_id);
        cert.data[32] = arch;
        cert.data[33..37].copy_from_slice(&capabilities.to_le_bytes());
        cert.data[37] = zone;
        cert.data[38..46].copy_from_slice(&timestamp.to_le_bytes());

        // HMAC-SHA256 signature over the certificate fields
        let tag = crypto::hmac_sha256(secret, &cert.data[..46]);
        cert.data[46..78].copy_from_slice(&tag.bytes[..32]);
        cert.len = Self::SIGNED_LEN;
        cert
    }

    /// Verify a certificate's HMAC signature.
    pub fn verify(&self, secret: &[u8; 32]) -> bool {
        if self.len < Self::SIGNED_LEN {
            return false;
        }
        let expected = crypto::hmac_sha256(secret, &self.data[..46]);
        let mut diff = 0u8;
        for i in 0..32 {
            diff |= self.data[46 + i] ^ expected.bytes[i];
        }
        diff == 0
    }

    /// Extract node_id from certificate.
    pub fn node_id(&self) -> &[u8] {
        &self.data[..32]
    }

    /// Extract capabilities from certificate.
    pub fn capabilities(&self) -> u32 {
        u32::from_le_bytes([
            self.data[33], self.data[34], self.data[35], self.data[36],
        ])
    }
}

// ─── Node Identity Manager ─────────────────────────────────────────────

/// The local node's identity + peer identity table.
///
/// This is the Zero Trust enforcement point. All fabric messages
/// pass through identity verification before processing.
pub struct NodeIdentityManager {
    /// Our node ID.
    pub local_id: NodeId,
    /// Our secret key (symmetric, pre-shared for Phase 1).
    pub local_secret: [u8; 32],
    /// Our attestation certificate.
    pub local_cert: AttestationCert,
    /// Peer identity table.
    pub peers: [PeerIdentity; MAX_PEERS],
    /// Number of active peers.
    pub peer_count: usize,
    /// Maximum failed auth attempts before auto-revoke.
    pub max_failed_auths: u8,
}

impl NodeIdentityManager {
    pub const fn new() -> Self {
        Self {
            local_id: [0u8; NODE_ID_LEN],
            local_secret: [0u8; 32],
            local_cert: AttestationCert::empty(),
            peers: [PeerIdentity::empty(); MAX_PEERS],
            peer_count: 0,
            max_failed_auths: 3,
        }
    }

    /// Initialize the local node identity.
    ///
    /// Generates node_id = SHA-256(secret) and builds attestation cert.
    pub fn init_local(
        &mut self,
        secret: [u8; 32],
        arch: u8,
        capabilities: u32,
        zone: u8,
        tick: u64,
    ) {
        use crypto::Hash;

        self.local_secret = secret;

        // node_id = SHA-256(secret) — in production this would be
        // SHA-256(public_key), but Phase 1 uses symmetric keys.
        let digest = crypto::sha256::Sha256::digest(&secret);
        self.local_id.copy_from_slice(&digest.bytes[..32]);

        // Build self-signed attestation certificate.
        self.local_cert = AttestationCert::build(
            &self.local_id,
            arch,
            capabilities,
            zone,
            tick,
            &self.local_secret,
        );
    }

    /// Look up a peer by node_id. Returns index or None.
    pub fn find_peer(&self, node_id: &[u8]) -> Option<usize> {
        for (i, peer) in self.peers.iter().enumerate() {
            if peer.active && peer.node_id[..] == node_id[..NODE_ID_LEN.min(node_id.len())] {
                return Some(i);
            }
        }
        None
    }

    /// Register a new peer (starts as Untrusted).
    pub fn register_peer(&mut self, node_id: &NodeId, tick: u64) -> Option<usize> {
        // Check if already known.
        if let Some(idx) = self.find_peer(node_id) {
            return Some(idx);
        }
        // Find empty slot.
        for (i, peer) in self.peers.iter_mut().enumerate() {
            if !peer.active {
                *peer = PeerIdentity::empty();
                peer.node_id.copy_from_slice(node_id);
                peer.active = true;
                peer.trust = TrustLevel::Untrusted;
                peer.last_seen = tick;
                self.peer_count += 1;
                return Some(i);
            }
        }
        None // table full
    }

    /// Generate a challenge nonce for a peer.
    ///
    /// Stores the nonce so we can verify their response later.
    /// Returns the nonce bytes.
    pub fn generate_challenge(
        &mut self,
        peer_idx: usize,
        entropy: &[u8; 32],
    ) -> Option<[u8; NONCE_LEN]> {
        if peer_idx >= MAX_PEERS || !self.peers[peer_idx].active {
            return None;
        }
        // Use HMAC(secret, entropy || peer_id) as nonce to avoid
        // relying solely on the entropy source.
        {
            let mut input = [0u8; 64];
            input[..32].copy_from_slice(entropy);
            input[32..64].copy_from_slice(&self.peers[peer_idx].node_id);
            let nonce_digest = crypto::hmac_sha256(&self.local_secret, &input);
            let mut nonce = [0u8; NONCE_LEN];
            nonce.copy_from_slice(&nonce_digest.bytes[..NONCE_LEN]);
            self.peers[peer_idx].our_nonce = nonce;
            self.peers[peer_idx].trust = TrustLevel::Challenged;
            Some(nonce)
        }
    }

    /// Verify a challenge response from a peer.
    ///
    /// Expected response = HMAC-SHA256(peer_secret, nonce || our_node_id).
    /// In Phase 1 (PSK), both sides know the shared secret.
    /// In Phase 2+, this becomes signature verification.
    pub fn verify_challenge_response(
        &mut self,
        peer_idx: usize,
        response: &[u8],
        psk: &[u8; 32],
        tick: u64,
    ) -> bool {
        if peer_idx >= MAX_PEERS || !self.peers[peer_idx].active {
            return false;
        }
        if self.peers[peer_idx].trust == TrustLevel::Revoked {
            return false;
        }

        // Expected: HMAC(psk, nonce || our_local_id)
        let mut input = [0u8; 64];
        input[..32].copy_from_slice(&self.peers[peer_idx].our_nonce);
        input[32..64].copy_from_slice(&self.local_id);
        let expected = crypto::hmac_sha256(psk, &input);

        // Constant-time comparison
        if response.len() < 32 {
            self.peers[peer_idx].failed_auths += 1;
            if self.peers[peer_idx].failed_auths >= self.max_failed_auths {
                self.peers[peer_idx].trust = TrustLevel::Revoked;
            }
            return false;
        }

        let mut diff = 0u8;
        for i in 0..32 {
            diff |= response[i] ^ expected.bytes[i];
        }

        if diff == 0 {
            self.peers[peer_idx].trust = TrustLevel::Verified;
            self.peers[peer_idx].trust_since = tick;
            self.peers[peer_idx].last_seen = tick;

            // Derive session key: HKDF(psk, nonce || peer_id || our_id, "veeros-session")
            let mut ikm = [0u8; 96];
            ikm[..32].copy_from_slice(&self.peers[peer_idx].our_nonce);
            ikm[32..64].copy_from_slice(&self.peers[peer_idx].node_id);
            ikm[64..96].copy_from_slice(&self.local_id);
            let info = b"veeros-fabric-session-v1";
            crypto::hkdf_sha256(
                &ikm,
                psk,
                info,
                &mut self.peers[peer_idx].session_key,
            );
            self.peers[peer_idx].session_active = true;
            true
        } else {
            self.peers[peer_idx].failed_auths += 1;
            if self.peers[peer_idx].failed_auths >= self.max_failed_auths {
                self.peers[peer_idx].trust = TrustLevel::Revoked;
            }
            false
        }
    }

    /// Produce our challenge response to a peer's nonce.
    pub fn create_challenge_response(
        &self,
        their_nonce: &[u8; 32],
        their_node_id: &NodeId,
        psk: &[u8; 32],
    ) -> [u8; 32] {
        let mut input = [0u8; 64];
        input[..32].copy_from_slice(their_nonce);
        input[32..64].copy_from_slice(their_node_id);
        let tag = crypto::hmac_sha256(psk, &input);
        let mut resp = [0u8; 32];
        resp.copy_from_slice(&tag.bytes[..32]);
        resp
    }

    /// Revoke a peer — immediately reject all future messages.
    pub fn revoke_peer(&mut self, peer_idx: usize) {
        if peer_idx < MAX_PEERS && self.peers[peer_idx].active {
            self.peers[peer_idx].trust = TrustLevel::Revoked;
            self.peers[peer_idx].session_active = false;
            // Zeroize session key
            for b in self.peers[peer_idx].session_key.iter_mut() {
                unsafe { core::ptr::write_volatile(b, 0); }
            }
        }
    }

    /// Check if a peer is authorized for a given operation.
    pub fn is_authorized(&self, peer_idx: usize, min_trust: TrustLevel) -> bool {
        if peer_idx >= MAX_PEERS {
            return false;
        }
        let peer = &self.peers[peer_idx];
        peer.active
            && peer.trust != TrustLevel::Revoked
            && peer.trust >= min_trust
    }

    /// Get the next outbound sequence number for a peer (anti-replay).
    pub fn next_tx_seq(&mut self, peer_idx: usize) -> u32 {
        if peer_idx >= MAX_PEERS {
            return 0;
        }
        let seq = self.peers[peer_idx].tx_seq;
        self.peers[peer_idx].tx_seq = seq.wrapping_add(1);
        seq
    }

    /// Validate an inbound sequence number (anti-replay).
    ///
    /// Accepts if seq >= expected. Updates expected to seq + 1.
    /// Small window (16) allows for minor reordering.
    pub fn validate_rx_seq(&mut self, peer_idx: usize, seq: u32) -> bool {
        if peer_idx >= MAX_PEERS {
            return false;
        }
        let expected = self.peers[peer_idx].rx_seq;
        // Allow sequence numbers within a window of 16
        if seq >= expected || expected.wrapping_sub(seq) <= 16 {
            if seq >= expected {
                self.peers[peer_idx].rx_seq = seq.wrapping_add(1);
            }
            true
        } else {
            false // replay or too old
        }
    }

    /// Count peers at a given trust level or higher.
    pub fn count_at_trust(&self, min_trust: TrustLevel) -> usize {
        self.peers.iter()
            .filter(|p| p.active && p.trust >= min_trust && p.trust != TrustLevel::Revoked)
            .count()
    }

    /// Expire peers not seen for `timeout` ticks.
    pub fn expire_stale(&mut self, current_tick: u64, timeout: u64) {
        for peer in self.peers.iter_mut() {
            if peer.active
                && peer.trust != TrustLevel::Revoked
                && current_tick.saturating_sub(peer.last_seen) > timeout
            {
                peer.trust = TrustLevel::Untrusted;
                peer.session_active = false;
                // Zeroize session key on expiry
                for b in peer.session_key.iter_mut() {
                    unsafe { core::ptr::write_volatile(b, 0); }
                }
            }
        }
    }
}

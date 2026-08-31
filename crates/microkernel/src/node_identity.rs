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

use crate::fabric_crypto::CryptoSessionMode;
use crate::fabric_proto::{NodeId, NODE_ID_LEN};
use crate::zkp::{CapabilityProof, HASH_LEN};

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
    Untrusted = 0,
    /// Challenge has been sent, awaiting response.
    Challenged = 1,
    /// Mutual authentication completed (challenge-response verified).
    Verified = 2,
    /// ZKP capability proof verified — highest trust.
    Attested = 3,
    /// Identity has been revoked — reject all messages.
    Revoked = 255,
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
    /// Their challenge nonce (which we answered).
    pub their_nonce: [u8; NONCE_LEN],
    /// Whether we have verified their response to our challenge.
    pub peer_verified: bool,
    /// Whether we have responded to their challenge.
    pub local_response_sent: bool,
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
    /// Commitment root for the peer's selectively disclosed capabilities.
    pub capability_root: [u8; HASH_LEN],
    /// Peer's advertised crypto capability level.
    pub crypto_mode: CryptoSessionMode,
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
            their_nonce: [0u8; NONCE_LEN],
            peer_verified: false,
            local_response_sent: false,
            last_seen: 0,
            trust_since: 0,
            failed_auths: 0,
            arch: 255,
            capabilities: 0,
            capability_root: [0u8; HASH_LEN],
            crypto_mode: CryptoSessionMode::Symmetric,
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
/// [46]      crypto mode (`CryptoSessionMode` as u8)
/// [47..79]  HMAC-SHA256 over [0..47] using node secret
/// ```
pub struct AttestationCert {
    pub data: [u8; MAX_CERT_LEN],
    pub len: usize,
}

impl AttestationCert {
    pub const SIGNED_LEN: usize = 79;

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
        crypto_mode: CryptoSessionMode,
        secret: &[u8; 32],
    ) -> Self {
        let mut cert = Self::empty();
        cert.data[..32].copy_from_slice(node_id);
        cert.data[32] = arch;
        cert.data[33..37].copy_from_slice(&capabilities.to_le_bytes());
        cert.data[37] = zone;
        cert.data[38..46].copy_from_slice(&timestamp.to_le_bytes());
        cert.data[46] = crypto_mode as u8;

        // HMAC-SHA256 signature over the certificate fields
        let tag = crypto::hmac_sha256(secret, &cert.data[..47]);
        cert.data[47..79].copy_from_slice(&tag.bytes[..32]);
        cert.len = Self::SIGNED_LEN;
        cert
    }

    /// Verify a certificate's HMAC signature.
    pub fn verify(&self, secret: &[u8; 32]) -> bool {
        if self.len < Self::SIGNED_LEN {
            return false;
        }
        let expected = crypto::hmac_sha256(secret, &self.data[..47]);
        let mut diff = 0u8;
        for i in 0..32 {
            diff |= self.data[47 + i] ^ expected.bytes[i];
        }
        diff == 0
    }

    /// Extract node_id from certificate.
    pub fn node_id(&self) -> &[u8] {
        &self.data[..32]
    }

    /// Extract capabilities from certificate.
    pub fn capabilities(&self) -> u32 {
        u32::from_le_bytes([self.data[33], self.data[34], self.data[35], self.data[36]])
    }

    /// Extract advertised crypto mode from certificate.
    pub fn crypto_mode(&self) -> CryptoSessionMode {
        match self.data[46] {
            1 => CryptoSessionMode::Hybrid,
            2 => CryptoSessionMode::PqcOnly,
            _ => CryptoSessionMode::Symmetric,
        }
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
    /// Our advertised crypto capability level.
    pub local_crypto_mode: CryptoSessionMode,
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
            local_crypto_mode: CryptoSessionMode::Symmetric,
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
        self.init_local_with_mode(
            secret,
            arch,
            capabilities,
            zone,
            tick,
            CryptoSessionMode::Symmetric,
        );
    }

    /// Initialize the local node identity with an advertised crypto mode.
    pub fn init_local_with_mode(
        &mut self,
        secret: [u8; 32],
        arch: u8,
        capabilities: u32,
        zone: u8,
        tick: u64,
        crypto_mode: CryptoSessionMode,
    ) {
        use crypto::Hash;

        self.local_secret = secret;
        self.local_crypto_mode = crypto_mode;

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
            crypto_mode,
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
                peer.crypto_mode = CryptoSessionMode::Symmetric;
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
            self.peers[peer_idx].peer_verified = false;
            self.peers[peer_idx].session_active = false;
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
            self.peers[peer_idx].last_seen = tick;
            self.peers[peer_idx].peer_verified = true;
            self.try_activate_session(peer_idx, psk, tick);
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
        &mut self,
        peer_idx: usize,
        their_nonce: &[u8; 32],
        their_node_id: &NodeId,
        psk: &[u8; 32],
        tick: u64,
    ) -> Option<[u8; 32]> {
        if peer_idx >= MAX_PEERS || !self.peers[peer_idx].active {
            return None;
        }
        if self.peers[peer_idx].trust == TrustLevel::Revoked {
            return None;
        }

        self.peers[peer_idx].their_nonce.copy_from_slice(their_nonce);
        self.peers[peer_idx].local_response_sent = true;
        self.peers[peer_idx].last_seen = tick;
        if self.peers[peer_idx].trust == TrustLevel::Untrusted {
            self.peers[peer_idx].trust = TrustLevel::Challenged;
        }

        let mut input = [0u8; 64];
        input[..32].copy_from_slice(their_nonce);
        input[32..64].copy_from_slice(their_node_id);
        let tag = crypto::hmac_sha256(psk, &input);
        let mut resp = [0u8; 32];
        resp.copy_from_slice(&tag.bytes[..32]);
        self.try_activate_session(peer_idx, psk, tick);
        Some(resp)
    }

    /// Revoke a peer — immediately reject all future messages.
    pub fn revoke_peer(&mut self, peer_idx: usize) {
        if peer_idx < MAX_PEERS && self.peers[peer_idx].active {
            self.peers[peer_idx].trust = TrustLevel::Revoked;
            self.peers[peer_idx].session_active = false;
            self.peers[peer_idx].capabilities = 0;
            self.peers[peer_idx].capability_root = [0u8; HASH_LEN];
            self.peers[peer_idx].crypto_mode = CryptoSessionMode::Symmetric;
            // Zeroize session key
            for b in self.peers[peer_idx].session_key.iter_mut() {
                unsafe {
                    core::ptr::write_volatile(b, 0);
                }
            }
        }
    }

    /// Check if a peer is authorized for a given operation.
    pub fn is_authorized(&self, peer_idx: usize, min_trust: TrustLevel) -> bool {
        if peer_idx >= MAX_PEERS {
            return false;
        }
        let peer = &self.peers[peer_idx];
        peer.active && peer.trust != TrustLevel::Revoked && peer.trust >= min_trust
    }

    /// Record the commitment root a peer will use for capability proofs.
    pub fn set_peer_capability_commitment(
        &mut self,
        peer_idx: usize,
        root: &[u8; HASH_LEN],
    ) -> bool {
        if peer_idx >= MAX_PEERS || !self.peers[peer_idx].active {
            return false;
        }
        self.peers[peer_idx].capability_root.copy_from_slice(root);
        true
    }

    /// Record the crypto capability level a peer advertised during join.
    pub fn set_peer_crypto_mode(&mut self, peer_idx: usize, mode: CryptoSessionMode) -> bool {
        if peer_idx >= MAX_PEERS || !self.peers[peer_idx].active {
            return false;
        }
        self.peers[peer_idx].crypto_mode = mode;
        true
    }

    /// Verify a selective-disclosure capability proof and upgrade trust.
    pub fn verify_capability_proof(
        &mut self,
        peer_idx: usize,
        proof: &CapabilityProof,
        expected_cap: u8,
        tick: u64,
    ) -> bool {
        if peer_idx >= MAX_PEERS || !self.peers[peer_idx].active {
            return false;
        }

        let peer = &mut self.peers[peer_idx];
        if peer.trust != TrustLevel::Verified || !peer.session_active {
            return false;
        }
        if expected_cap as usize >= crate::zkp::MAX_CAP_BITS {
            return false;
        }
        if proof.cap_index != expected_cap || proof.claimed_value != 1 {
            return false;
        }

        let mut root_diff = 0u8;
        for i in 0..HASH_LEN {
            root_diff |= peer.capability_root[i] ^ proof.root[i];
        }
        if root_diff != 0 || !proof.verify() {
            return false;
        }

        peer.capabilities |= 1u32 << expected_cap;
        peer.trust = TrustLevel::Attested;
        peer.last_seen = tick;
        peer.trust_since = tick;
        true
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
        self.peers
            .iter()
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
                peer.peer_verified = false;
                peer.local_response_sent = false;
                peer.capabilities = 0;
                peer.capability_root = [0u8; HASH_LEN];
                peer.crypto_mode = CryptoSessionMode::Symmetric;
                // Zeroize session key on expiry
                for b in peer.session_key.iter_mut() {
                    unsafe {
                        core::ptr::write_volatile(b, 0);
                    }
                }
            }
        }
    }

    fn try_activate_session(&mut self, peer_idx: usize, psk: &[u8; 32], tick: u64) {
        let peer = &mut self.peers[peer_idx];
        if !peer.peer_verified || !peer.local_response_sent {
            peer.trust = TrustLevel::Challenged;
            peer.session_active = false;
            return;
        }

        let mut shared_nonce = [0u8; NONCE_LEN];
        for (idx, out) in shared_nonce.iter_mut().enumerate() {
            *out = peer.our_nonce[idx] ^ peer.their_nonce[idx];
        }

        let (first_id, second_id) = if self.local_id <= peer.node_id {
            (&self.local_id, &peer.node_id)
        } else {
            (&peer.node_id, &self.local_id)
        };
        let mut node_ids = [0u8; NODE_ID_LEN * 2];
        node_ids[..NODE_ID_LEN].copy_from_slice(first_id);
        node_ids[NODE_ID_LEN..].copy_from_slice(second_id);

        let info = b"veeros-fabric-session-v1";
        crypto::hkdf_sha256(&shared_nonce, &node_ids, info, &mut peer.session_key);
        peer.trust = TrustLevel::Verified;
        peer.trust_since = tick;
        peer.session_active = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zkp::CapabilityCommitment;

    fn init_manager(secret: [u8; 32], tick: u64) -> NodeIdentityManager {
        let mut mgr = NodeIdentityManager::new();
        mgr.init_local(secret, 2, 0x55AA, 1, tick);
        mgr
    }

    #[test]
    fn mutual_auth_requires_both_directions_before_session_activation() {
        let psk = [0xA5u8; 32];
        let mut left = init_manager([1u8; 32], 1);
        let mut right = init_manager([2u8; 32], 1);

        let left_peer = left.register_peer(&right.local_id, 2).unwrap();
        let right_peer = right.register_peer(&left.local_id, 2).unwrap();

        let left_nonce = left.generate_challenge(left_peer, &[3u8; 32]).unwrap();
        let left_resp = right
            .create_challenge_response(right_peer, &left_nonce, &left.local_id, &psk, 3)
            .unwrap();
        assert!(!right.peers[right_peer].session_active);
        assert_eq!(right.peers[right_peer].trust, TrustLevel::Challenged);

        assert!(left.verify_challenge_response(left_peer, &left_resp, &psk, 4));
        assert!(!left.peers[left_peer].session_active);
        assert_eq!(left.peers[left_peer].trust, TrustLevel::Challenged);

        let right_nonce = right.generate_challenge(right_peer, &[4u8; 32]).unwrap();
        let right_resp = left
            .create_challenge_response(left_peer, &right_nonce, &right.local_id, &psk, 5)
            .unwrap();
        assert!(left.peers[left_peer].session_active);
        assert_eq!(left.peers[left_peer].trust, TrustLevel::Verified);

        assert!(right.verify_challenge_response(right_peer, &right_resp, &psk, 6));
        assert!(right.peers[right_peer].session_active);
        assert_eq!(right.peers[right_peer].trust, TrustLevel::Verified);
        assert_eq!(left.peers[left_peer].session_key, right.peers[right_peer].session_key);
    }

    #[test]
    fn invalid_response_does_not_activate_session() {
        let psk = [0x5Au8; 32];
        let mut left = init_manager([9u8; 32], 1);
        let mut right = init_manager([8u8; 32], 1);

        let left_peer = left.register_peer(&right.local_id, 2).unwrap();
        let right_peer = right.register_peer(&left.local_id, 2).unwrap();
        let left_nonce = left.generate_challenge(left_peer, &[7u8; 32]).unwrap();
        let mut response = right
            .create_challenge_response(right_peer, &left_nonce, &left.local_id, &psk, 3)
            .unwrap();
        response[0] ^= 0xFF;

        assert!(!left.verify_challenge_response(left_peer, &response, &psk, 4));
        assert!(!left.peers[left_peer].session_active);
        assert_eq!(left.peers[left_peer].trust, TrustLevel::Challenged);
    }

    #[test]
    fn verified_peer_can_be_promoted_to_attested_with_valid_capability_proof() {
        let psk = [0xA5u8; 32];
        let mut left = init_manager([1u8; 32], 1);
        let mut right = init_manager([2u8; 32], 1);

        let left_peer = left.register_peer(&right.local_id, 2).unwrap();
        let right_peer = right.register_peer(&left.local_id, 2).unwrap();

        let left_nonce = left.generate_challenge(left_peer, &[3u8; 32]).unwrap();
        let left_resp = right
            .create_challenge_response(right_peer, &left_nonce, &left.local_id, &psk, 3)
            .unwrap();
        assert!(left.verify_challenge_response(left_peer, &left_resp, &psk, 4));

        let right_nonce = right.generate_challenge(right_peer, &[4u8; 32]).unwrap();
        let right_resp = left
            .create_challenge_response(left_peer, &right_nonce, &right.local_id, &psk, 5)
            .unwrap();
        assert!(right.verify_challenge_response(right_peer, &right_resp, &psk, 6));
        assert_eq!(left.peers[left_peer].trust, TrustLevel::Verified);

        let commitment = CapabilityCommitment::commit(1u32 << 10, &[9u8; 32]);
        let proof = crate::zkp::CapabilityProof::prove(&commitment, 10).unwrap();
        assert!(left.set_peer_capability_commitment(left_peer, &commitment.root));
        assert!(left.verify_capability_proof(left_peer, &proof, 10, 7));
        assert_eq!(left.peers[left_peer].trust, TrustLevel::Attested);
        assert_ne!(left.peers[left_peer].capabilities & (1u32 << 10), 0);
    }

    #[test]
    fn capability_proof_requires_matching_committed_root() {
        let psk = [0xA5u8; 32];
        let mut left = init_manager([1u8; 32], 1);
        let mut right = init_manager([2u8; 32], 1);

        let left_peer = left.register_peer(&right.local_id, 2).unwrap();
        let right_peer = right.register_peer(&left.local_id, 2).unwrap();

        let left_nonce = left.generate_challenge(left_peer, &[3u8; 32]).unwrap();
        let left_resp = right
            .create_challenge_response(right_peer, &left_nonce, &left.local_id, &psk, 3)
            .unwrap();
        assert!(left.verify_challenge_response(left_peer, &left_resp, &psk, 4));

        let right_nonce = right.generate_challenge(right_peer, &[4u8; 32]).unwrap();
        let right_resp = left
            .create_challenge_response(left_peer, &right_nonce, &right.local_id, &psk, 5)
            .unwrap();
        assert!(right.verify_challenge_response(right_peer, &right_resp, &psk, 6));

        let commitment = CapabilityCommitment::commit(1u32 << 5, &[9u8; 32]);
        let wrong_commitment = CapabilityCommitment::commit(1u32 << 6, &[10u8; 32]);
        let proof = crate::zkp::CapabilityProof::prove(&commitment, 5).unwrap();
        assert!(left.set_peer_capability_commitment(left_peer, &wrong_commitment.root));
        assert!(!left.verify_capability_proof(left_peer, &proof, 5, 7));
        assert_eq!(left.peers[left_peer].trust, TrustLevel::Verified);
    }

    #[test]
    fn attestation_cert_roundtrip_preserves_crypto_mode() {
        let mut mgr = NodeIdentityManager::new();
        mgr.init_local_with_mode([7u8; 32], 2, 0x55AA, 1, 9, CryptoSessionMode::Hybrid);

        assert_eq!(mgr.local_cert.crypto_mode(), CryptoSessionMode::Hybrid);
        assert!(mgr.local_cert.verify(&mgr.local_secret));
    }

    #[test]
    fn peer_crypto_mode_can_be_recorded() {
        let mut mgr = init_manager([4u8; 32], 1);
        let peer_id = [9u8; 32];
        let peer_idx = mgr.register_peer(&peer_id, 2).unwrap();

        assert!(mgr.set_peer_crypto_mode(peer_idx, CryptoSessionMode::PqcOnly));
        assert_eq!(mgr.peers[peer_idx].crypto_mode, CryptoSessionMode::PqcOnly);
    }
}

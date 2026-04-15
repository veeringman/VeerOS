//! Distributed fabric — Zero Trust peer management, ZKP proofs,
//! mesh transport, and encrypted session queries.
//!
//! # Example
//!
//! ```rust,no_run
//! use userlib::fabric_net;
//!
//! // Register a peer and start Zero Trust challenge
//! let peer_idx = fabric_net::peer_register(&peer_id, &entropy).unwrap();
//!
//! // After challenge-response completes, check status
//! let (trust, caps) = fabric_net::peer_status(peer_idx).unwrap();
//!
//! // Send a message across the mesh
//! fabric_net::mesh_send(&dest_id, b"hello", 128).unwrap();
//!
//! // Query mesh stats
//! let (routes, queued) = fabric_net::mesh_status();
//! ```

use crate::sys;

const SYS_PEER_REGISTER: usize = 0xE2;
const SYS_PEER_VERIFY: usize = 0xE3;
const SYS_PEER_STATUS: usize = 0xE4;
const SYS_ZKP_PROVE: usize = 0xE5;
const SYS_ZKP_VERIFY: usize = 0xE6;
const SYS_MESH_SEND: usize = 0xE7;
const SYS_MESH_STATUS: usize = 0xE8;
const SYS_FABRIC_SESSION_COUNT: usize = 0xE9;

// ─── Zero Trust Peer Management ─────────────────────────────────────────

/// Register a remote peer and initiate a Zero Trust challenge.
///
/// `node_id`: 32-byte unique identifier of the peer.
/// `entropy`: 32-byte random data used to derive the challenge nonce.
///
/// Returns the peer index on success, or `None` if the peer table is full.
pub fn peer_register(node_id: &[u8; 32], entropy: &[u8; 32]) -> Option<usize> {
    let (ret, _) = unsafe {
        sys::syscall2(
            SYS_PEER_REGISTER,
            node_id.as_ptr() as usize,
            entropy.as_ptr() as usize,
        )
    };
    if ret == usize::MAX { None } else { Some(ret) }
}

/// Verify a challenge response from a peer, promoting it to Verified.
///
/// `peer_idx`: index returned by `peer_register()`.
/// `response`: 32-byte HMAC response from the peer.
///
/// Returns `true` if the response is valid and the peer is now Verified.
pub fn peer_verify(peer_idx: usize, response: &[u8; 32]) -> bool {
    let (ret, _) = unsafe {
        sys::syscall2(
            SYS_PEER_VERIFY,
            peer_idx,
            response.as_ptr() as usize,
        )
    };
    ret == 0
}

/// Query a peer's trust level and capability bitmask.
///
/// Returns `(trust_level, capabilities)` or `None` if `peer_idx` is invalid.
/// Trust levels: 0=Untrusted, 1=Challenged, 2=Verified, 3=Attested, 4=Revoked.
pub fn peer_status(peer_idx: usize) -> Option<(usize, usize)> {
    let (a, b) = unsafe {
        sys::syscall2(SYS_PEER_STATUS, peer_idx, 0)
    };
    if a == usize::MAX { None } else { Some((a, b)) }
}

// ─── ZKP Capability Proofs ──────────────────────────────────────────────

/// Generate a Zero-Knowledge Proof that we hold certain capabilities.
///
/// `capability_mask`: bitmask of capabilities to prove.
/// `out`: buffer to receive the proof bytes.
///
/// Returns the proof length on success, or `None` on failure.
pub fn zkp_prove(capability_mask: u32, out: &mut [u8]) -> Option<usize> {
    let ret = unsafe {
        sys::syscall3(
            SYS_ZKP_PROVE,
            capability_mask as usize,
            out.as_mut_ptr() as usize,
            out.len(),
        )
    };
    if ret == usize::MAX { None } else { Some(ret) }
}

/// Verify a ZKP capability proof from a remote peer.
///
/// `proof`: the proof data received from the peer.
/// `expected_caps`: the capability bitmask we expect the proof to cover.
///
/// Returns `true` if the proof is valid.
pub fn zkp_verify(proof: &[u8], expected_caps: u32) -> bool {
    let ret = unsafe {
        sys::syscall3(
            SYS_ZKP_VERIFY,
            proof.as_ptr() as usize,
            proof.len(),
            expected_caps as usize,
        )
    };
    ret == 0
}

// ─── Mesh Transport ─────────────────────────────────────────────────────

/// Send a message via the mesh transport layer.
///
/// `dest`: 32-byte destination node ID.
/// `data`: message payload.
/// `priority`: 0–255 (higher = more urgent).
///
/// Returns `true` on success, `false` if the outbox is full.
pub fn mesh_send(dest: &[u8; 32], data: &[u8], priority: u8) -> bool {
    let ret = unsafe {
        sys::syscall4(
            SYS_MESH_SEND,
            dest.as_ptr() as usize,
            data.as_ptr() as usize,
            data.len(),
            priority as usize,
        )
    };
    ret == 0
}

/// Query mesh transport statistics.
///
/// Returns `(route_count, queued_messages)`.
pub fn mesh_status() -> (usize, usize) {
    unsafe { sys::syscall2(SYS_MESH_STATUS, 0, 0) }
}

// ─── Crypto Sessions ────────────────────────────────────────────────────

/// Get the number of active fabric crypto sessions.
pub fn session_count() -> usize {
    unsafe { sys::syscall0(SYS_FABRIC_SESSION_COUNT) }
}

//! SSH-2 Key Exchange — curve25519-sha256 (RFC 8731).
//!
//! Implements the Elliptic Curve Diffie-Hellman key exchange used to
//! establish session keys. After KEX, both sides derive identical
//! encryption keys via SHA-256 KDF.

use crate::transport::TransportKeys;
use crate::{get_u32, put_mpint, put_string, put_u32, MAX_PAYLOAD, VERSION_STRING};
use crypto::ed25519::{ed25519_sign, Ed25519PublicKey, Ed25519Seed};
use crypto::sha256::Sha256;
use crypto::x25519::{x25519_diffie_hellman, x25519_keypair};
use crypto::{CryptoRng, Hash};

/// Our KEXINIT proposal.
pub struct KexConfig {
    /// Server host key seed (Ed25519).
    pub host_seed: Ed25519Seed,
    /// Server host key public key (Ed25519).
    pub host_pubkey: Ed25519PublicKey,
}

/// State for an ongoing key exchange.
pub struct KexState {
    /// Our ephemeral X25519 secret key.
    pub eph_sk: [u8; 32],
    /// Our ephemeral X25519 public key.
    pub eph_pk: [u8; 32],
    /// Client's KEXINIT payload (needed for exchange hash).
    pub client_kexinit: [u8; MAX_PAYLOAD],
    pub client_kexinit_len: usize,
    /// Server's KEXINIT payload (needed for exchange hash).
    pub server_kexinit: [u8; MAX_PAYLOAD],
    pub server_kexinit_len: usize,
    /// Client's version string.
    pub client_version: [u8; 256],
    pub client_version_len: usize,
    /// Session ID (set to exchange hash H on first KEX).
    pub session_id: [u8; 32],
    pub session_id_set: bool,
}

impl KexState {
    pub fn new(rng: &mut dyn CryptoRng) -> Self {
        let (sk, pk) = x25519_keypair(rng);
        Self {
            eph_sk: sk,
            eph_pk: pk,
            client_kexinit: [0u8; MAX_PAYLOAD],
            client_kexinit_len: 0,
            server_kexinit: [0u8; MAX_PAYLOAD],
            server_kexinit_len: 0,
            client_version: [0u8; 256],
            client_version_len: 0,
            session_id: [0u8; 32],
            session_id_set: false,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// KEXINIT message building
// ═══════════════════════════════════════════════════════════════════════════

/// Algorithms we support (comma-separated name-lists).
const KEX_ALGORITHMS: &[u8] = b"curve25519-sha256";
const HOST_KEY_ALGORITHMS: &[u8] = b"ssh-ed25519";
const CIPHER_ALGORITHMS: &[u8] = b"chacha20-poly1305@openssh.com";
const MAC_ALGORITHMS: &[u8] = b""; // implicit in AEAD
const COMPRESSION_ALGORITHMS: &[u8] = b"none";

/// Build a KEXINIT message payload.
///
/// Returns the number of bytes written into `buf`.
pub fn build_kexinit(buf: &mut [u8], cookie: &[u8; 16]) -> usize {
    let mut off = 0;

    // Message type
    buf[off] = 20; // SSH_MSG_KEXINIT
    off += 1;

    // Cookie (16 random bytes)
    buf[off..off + 16].copy_from_slice(cookie);
    off += 16;

    // Name-lists: kex, host_key, cipher_c2s, cipher_s2c, mac_c2s, mac_s2c,
    //             compress_c2s, compress_s2c, languages_c2s, languages_s2c
    off += crate::put_name_list(&mut buf[off..], KEX_ALGORITHMS);
    off += crate::put_name_list(&mut buf[off..], HOST_KEY_ALGORITHMS);
    off += crate::put_name_list(&mut buf[off..], CIPHER_ALGORITHMS); // c2s
    off += crate::put_name_list(&mut buf[off..], CIPHER_ALGORITHMS); // s2c
    off += crate::put_name_list(&mut buf[off..], MAC_ALGORITHMS); // c2s
    off += crate::put_name_list(&mut buf[off..], MAC_ALGORITHMS); // s2c
    off += crate::put_name_list(&mut buf[off..], COMPRESSION_ALGORITHMS); // c2s
    off += crate::put_name_list(&mut buf[off..], COMPRESSION_ALGORITHMS); // s2c
    off += crate::put_name_list(&mut buf[off..], b""); // languages c2s
    off += crate::put_name_list(&mut buf[off..], b""); // languages s2c

    // first_kex_packet_follows = false
    buf[off] = 0;
    off += 1;

    // reserved (uint32)
    put_u32(&mut buf[off..], 0);
    off += 4;

    off
}

// ═══════════════════════════════════════════════════════════════════════════
// ECDH Reply
// ═══════════════════════════════════════════════════════════════════════════

/// Process client's SSH_MSG_KEX_ECDH_INIT and produce SSH_MSG_KEX_ECDH_REPLY.
///
/// Also computes the exchange hash H and derives session keys.
///
/// Returns (reply_payload_len, transport_keys, exchange_hash, shared_secret, server_eph_pk).
pub fn process_ecdh_init(
    client_eph_pub: &[u8; 32],
    kex_state: &mut KexState,
    config: &KexConfig,
) -> Option<(
    [u8; MAX_PAYLOAD],
    usize,
    TransportKeys,
    [u8; 32],
    [u8; 32],
    [u8; 32],
)> {
    // Compute shared secret K = X25519(server_ephemeral_sk, client_ephemeral_pk)
    let shared_secret = match x25519_diffie_hellman(&kex_state.eph_sk, client_eph_pub) {
        Ok(ss) => ss,
        Err(_) => return None,
    };

    // Build the exchange hash H = SHA-256(V_C || V_S || I_C || I_S || K_S || Q_C || Q_S || K)
    let h = compute_exchange_hash(
        &kex_state.client_version[..kex_state.client_version_len],
        VERSION_STRING,
        &kex_state.client_kexinit[..kex_state.client_kexinit_len],
        &kex_state.server_kexinit[..kex_state.server_kexinit_len],
        &config.host_pubkey,
        client_eph_pub,
        &kex_state.eph_pk,
        &shared_secret,
    );

    // First KEX: session_id = H
    if !kex_state.session_id_set {
        kex_state.session_id.copy_from_slice(&h);
        kex_state.session_id_set = true;
    }

    // Sign H with host key
    let sig = ed25519_sign(&h, &config.host_seed, &config.host_pubkey);

    // Build SSH_MSG_KEX_ECDH_REPLY
    let mut reply = [0u8; MAX_PAYLOAD];
    let mut off = 0;

    reply[off] = 31; // SSH_MSG_KEX_ECDH_REPLY
    off += 1;

    // K_S (host key): string "ssh-ed25519" + string pubkey_bytes
    let host_key_blob_len = 4 + 11 + 4 + 32; // "ssh-ed25519" = 11 bytes
    put_u32(&mut reply[off..], host_key_blob_len as u32);
    off += 4;
    off += put_string(&mut reply[off..], b"ssh-ed25519");
    off += put_string(&mut reply[off..], &config.host_pubkey);

    // Q_S (server ephemeral public key)
    off += put_string(&mut reply[off..], &kex_state.eph_pk);

    // Signature of H: string "ssh-ed25519" + string sig_bytes
    let sig_blob_len = 4 + 11 + 4 + 64;
    put_u32(&mut reply[off..], sig_blob_len as u32);
    off += 4;
    off += put_string(&mut reply[off..], b"ssh-ed25519");
    off += put_string(&mut reply[off..], &sig);

    // Derive session keys
    let keys = derive_keys(&shared_secret, &h, &kex_state.session_id);

    Some((reply, off, keys, h, shared_secret, kex_state.eph_pk))
}

/// Compute the exchange hash H.
///
/// H = SHA-256(V_C || V_S || I_C || I_S || K_S || Q_C || Q_S || K)
///
/// Each component is encoded as an SSH string (length-prefixed).
fn compute_exchange_hash(
    v_c: &[u8], // client version string (without CR LF)
    v_s: &[u8], // server version string
    i_c: &[u8], // client KEXINIT payload
    i_s: &[u8], // server KEXINIT payload
    k_s: &[u8], // server host key (raw ed25519 public key → as "ssh-ed25519" blob)
    q_c: &[u8], // client ephemeral public key
    q_s: &[u8], // server ephemeral public key
    k: &[u8],   // shared secret
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let mut tmp = [0u8; MAX_PAYLOAD];

    // V_C
    let n = put_string(&mut tmp, v_c);
    hasher.update(&tmp[..n]);

    // V_S
    let n = put_string(&mut tmp, v_s);
    hasher.update(&tmp[..n]);

    // I_C (the full KEXINIT packet payload including msg type)
    let n = put_string(&mut tmp, i_c);
    hasher.update(&tmp[..n]);

    // I_S
    let n = put_string(&mut tmp, i_s);
    hasher.update(&tmp[..n]);

    // K_S (host key blob: "ssh-ed25519" || pubkey)
    let mut ks_blob = [0u8; 128];
    let mut ks_len = 0;
    ks_len += put_string(&mut ks_blob[ks_len..], b"ssh-ed25519");
    ks_len += put_string(&mut ks_blob[ks_len..], k_s);
    let n = put_string(&mut tmp, &ks_blob[..ks_len]);
    hasher.update(&tmp[..n]);

    // Q_C
    let n = put_string(&mut tmp, q_c);
    hasher.update(&tmp[..n]);

    // Q_S
    let n = put_string(&mut tmp, q_s);
    hasher.update(&tmp[..n]);

    // K (shared secret as mpint)
    let n = put_mpint(&mut tmp, k);
    hasher.update(&tmp[..n]);

    let digest = hasher.finalize();
    let mut h = [0u8; 32];
    h.copy_from_slice(&digest.bytes[..32]);
    h
}

/// Derive session keys from shared secret K, exchange hash H, and session ID.
///
/// Uses the SSH KDF: HASH(K || H || X || session_id)
/// where X is a single letter (A–F) selecting which key to derive.
fn derive_keys(k: &[u8; 32], h: &[u8; 32], session_id: &[u8; 32]) -> TransportKeys {
    // chacha20-poly1305@openssh.com needs 64 bytes per direction:
    // - First 32 bytes = main key (K_1)
    // - Second 32 bytes = header key (K_2)
    // Key labels: C = client-to-server cipher key, D = server-to-client cipher key

    let c2s_key = derive_key_material(k, h, b'C', session_id);
    let s2c_key = derive_key_material(k, h, b'D', session_id);

    TransportKeys {
        rx_key: {
            let mut k = [0u8; 32];
            k.copy_from_slice(&c2s_key[..32]);
            k
        },
        rx_header_key: {
            let mut k = [0u8; 32];
            k.copy_from_slice(&c2s_key[32..64]);
            k
        },
        tx_key: {
            let mut k = [0u8; 32];
            k.copy_from_slice(&s2c_key[..32]);
            k
        },
        tx_header_key: {
            let mut k = [0u8; 32];
            k.copy_from_slice(&s2c_key[32..64]);
            k
        },
    }
}

/// Derive 64 bytes of key material for one direction.
/// K_1 = HASH(K || H || label || session_id)
/// K_2 = HASH(K || H || K_1)
fn derive_key_material(k: &[u8; 32], h: &[u8; 32], label: u8, session_id: &[u8; 32]) -> [u8; 64] {
    let mut result = [0u8; 64];
    let mut tmp = [0u8; 37];

    // First 32 bytes
    let mut hasher = Sha256::new();
    let mpint_len = put_mpint(&mut tmp, k);
    hasher.update(&tmp[..mpint_len]);
    hasher.update(h);
    hasher.update(&[label]);
    hasher.update(session_id);
    let d = hasher.finalize();
    result[..32].copy_from_slice(&d.bytes[..32]);

    // Second 32 bytes
    let mut hasher = Sha256::new();
    let mpint_len = put_mpint(&mut tmp, k);
    hasher.update(&tmp[..mpint_len]);
    hasher.update(h);
    hasher.update(&result[..32]);
    let d = hasher.finalize();
    result[32..64].copy_from_slice(&d.bytes[..32]);

    result
}

#[cfg(test)]
mod tests {
    extern crate std;

    use self::std::time::Instant;
    use super::*;
    use crypto::rng::ChaChaRng;
    use crypto::x25519::x25519_keypair;

    fn test_server_config() -> KexConfig {
        KexConfig {
            host_seed: [
                0x56, 0x65, 0x65, 0x72, 0x4f, 0x53, 0x2d, 0x48, 0x6f, 0x73, 0x74, 0x4b, 0x65, 0x79,
                0x53, 0x65, 0x65, 0x64, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39,
                0x41, 0x42, 0x43, 0x44,
            ],
            host_pubkey: [
                0xe2, 0x91, 0x74, 0x12, 0xbb, 0x3a, 0x6f, 0x7e, 0x80, 0x07, 0x28, 0x7f, 0xc4, 0x27,
                0x01, 0x65, 0xcf, 0x5d, 0x05, 0x61, 0x63, 0xf0, 0x82, 0x4c, 0xda, 0x73, 0xdc, 0x70,
                0x8d, 0x99, 0x94, 0x3b,
            ],
        }
    }

    fn seeded_rng(seed_byte: u8) -> ChaChaRng {
        ChaChaRng::from_seed([seed_byte; 32])
    }

    fn build_test_kex_state() -> (KexState, [u8; 32]) {
        let mut rng = seeded_rng(0x11);
        let mut kex_state = KexState::new(&mut rng);
        let mut cookie = [0u8; 16];
        cookie.copy_from_slice(&[0xA5; 16]);

        let client_version = b"SSH-2.0-OpenSSH_9.6";
        kex_state.client_version[..client_version.len()].copy_from_slice(client_version);
        kex_state.client_version_len = client_version.len();

        let client_kex_len = build_kexinit(&mut kex_state.client_kexinit, &cookie);
        kex_state.client_kexinit_len = client_kex_len;

        let server_kex_len = build_kexinit(&mut kex_state.server_kexinit, &cookie);
        kex_state.server_kexinit_len = server_kex_len;

        let mut client_rng = seeded_rng(0x22);
        let (_client_sk, client_pk) = x25519_keypair(&mut client_rng);

        (kex_state, client_pk)
    }

    #[test]
    fn process_ecdh_init_produces_reply_and_keys() {
        let (mut kex_state, client_pk) = build_test_kex_state();
        let config = test_server_config();

        let result = process_ecdh_init(&client_pk, &mut kex_state, &config)
            .expect("ecdh init should succeed");

        let (reply, reply_len, keys, exchange_hash, _k, _qs) = result;
        assert_eq!(reply[0], 31);
        assert!(reply_len > 100);
        assert!(exchange_hash.iter().any(|&b| b != 0));
        assert!(keys.rx_key.iter().any(|&b| b != 0));
        assert!(keys.tx_key.iter().any(|&b| b != 0));
    }

    #[test]
    fn process_ecdh_init_timing_trace() {
        let (mut kex_state, client_pk) = build_test_kex_state();
        let config = test_server_config();

        let started = Instant::now();
        let result = process_ecdh_init(&client_pk, &mut kex_state, &config);
        let elapsed = started.elapsed();

        assert!(result.is_some());
        std::println!("process_ecdh_init elapsed: {:?}", elapsed);
    }
}

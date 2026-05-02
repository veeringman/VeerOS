//! X25519 — Elliptic Curve Diffie-Hellman (RFC 7748).
//!
//! `no_std`, `no_alloc`, constant-time. Delegates to `curve25519-dalek`.
//! Used for SSH-2 key exchange (`curve25519-sha256`).

use crate::{CryptoError, CryptoRng};
use curve25519_dalek::montgomery::MontgomeryPoint;

/// X25519 public key (32 bytes).
pub type X25519PublicKey = [u8; 32];

/// X25519 secret key (clamped, 32 bytes).
pub type X25519SecretKey = [u8; 32];

/// X25519 shared secret (32 bytes).
pub type X25519SharedSecret = [u8; 32];

/// Clamp a 32-byte scalar for X25519 (RFC 7748 §5).
pub fn clamp(k: &mut [u8; 32]) {
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;
}

/// Generate an X25519 keypair.
pub fn x25519_keypair(rng: &mut dyn CryptoRng) -> (X25519SecretKey, X25519PublicKey) {
    let mut sk = [0u8; 32];
    rng.fill_bytes(&mut sk);
    clamp(&mut sk);
    let pk = x25519_basepoint(&sk);
    (sk, pk)
}

/// Compute the X25519 public key from a secret key using the base point.
pub fn x25519_basepoint(sk: &[u8; 32]) -> X25519PublicKey {
    MontgomeryPoint::mul_base_clamped(*sk).0
}

/// X25519 scalar multiplication: compute `sk * point` (RFC 7748).
pub fn x25519(sk: &[u8; 32], point: &[u8; 32]) -> [u8; 32] {
    MontgomeryPoint(*point).mul_clamped(*sk).0
}

/// Perform X25519 Diffie-Hellman: compute shared secret from our secret
/// key and their public key.
pub fn x25519_diffie_hellman(
    our_sk: &X25519SecretKey,
    their_pk: &X25519PublicKey,
) -> Result<X25519SharedSecret, CryptoError> {
    let ss = x25519(our_sk, their_pk);

    // Check for low-order points (all-zero output = contributory failure).
    let mut acc: u8 = 0;
    for &b in &ss {
        acc |= b;
    }
    if acc == 0 {
        return Err(CryptoError::InternalError);
    }
    Ok(ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7748_test_vector() {
        // RFC 7748 §6.1
        let alice_sk: [u8; 32] =
            hex_to_bytes("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");

        let alice_pub = x25519_basepoint(&alice_sk);
        let expected_alice_pub: [u8; 32] =
            hex_to_bytes("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a");
        assert_eq!(alice_pub, expected_alice_pub, "Alice public key mismatch");

        let bob_sk: [u8; 32] =
            hex_to_bytes("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");

        let bob_pub = x25519_basepoint(&bob_sk);
        let expected_bob_pub: [u8; 32] =
            hex_to_bytes("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f");
        assert_eq!(bob_pub, expected_bob_pub, "Bob public key mismatch");

        let shared_ab = x25519(&alice_sk, &bob_pub);
        let shared_ba = x25519(&bob_sk, &alice_pub);
        let expected_ss: [u8; 32] =
            hex_to_bytes("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        assert_eq!(shared_ab, expected_ss, "Shared secret (A*B) mismatch");
        assert_eq!(shared_ba, expected_ss, "Shared secret (B*A) mismatch");
    }

    fn hex_to_bytes(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn iterated_one() {
        let mut bp = [0u8; 32];
        bp[0] = 9;
        let result = x25519(&bp, &bp);
        let expected =
            hex_to_bytes("422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079");
        assert_eq!(result, expected, "x25519(9,9) mismatch");
    }
}

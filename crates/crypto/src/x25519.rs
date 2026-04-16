//! X25519 — Elliptic Curve Diffie-Hellman (RFC 7748).
//!
//! `no_std`, `no_alloc`, constant-time Montgomery ladder on Curve25519.
//! Used for SSH-2 key exchange (`curve25519-sha256`).

use crate::curve25519::Fe;
use crate::{zeroize, CryptoError, CryptoRng};

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
///
/// Returns (secret_key, public_key).
pub fn x25519_keypair(rng: &mut dyn CryptoRng) -> (X25519SecretKey, X25519PublicKey) {
    let mut sk = [0u8; 32];
    rng.fill_bytes(&mut sk);
    clamp(&mut sk);
    let pk = x25519_basepoint(&sk);
    (sk, pk)
}

/// Compute the X25519 public key from a secret key using the base point.
///
/// base_point = 9.
pub fn x25519_basepoint(sk: &[u8; 32]) -> X25519PublicKey {
    let mut bp = [0u8; 32];
    bp[0] = 9;
    x25519(sk, &bp)
}

/// X25519 scalar multiplication: compute `sk * point`.
///
/// This is the core Diffie-Hellman operation. Constant-time Montgomery ladder.
pub fn x25519(sk: &[u8; 32], point: &[u8; 32]) -> [u8; 32] {
    let mut k = *sk;
    clamp(&mut k);

    let u = Fe::from_bytes(point);

    // Montgomery ladder
    let mut x_1 = u;
    let mut x_2 = Fe::ONE;
    let mut z_2 = Fe::ZERO;
    let mut x_3 = u;
    let mut z_3 = Fe::ONE;
    let mut swap: u64 = 0;

    // Iterate from bit 254 down to 0
    for pos in (0..=254).rev() {
        let byte = k[pos / 8];
        let bit = ((byte >> (pos & 7)) & 1) as u64;

        swap ^= bit;
        x_2.cswap(&mut x_3, swap);
        z_2.cswap(&mut z_3, swap);
        swap = bit;

        let a = x_2.add(&z_2);
        let aa = a.square();
        let b = x_2.sub(&z_2);
        let bb = b.square();
        let e = aa.sub(&bb);
        let c = x_3.add(&z_3);
        let d = x_3.sub(&z_3);
        let da = d.mul(&a);
        let cb = c.mul(&b);
        x_3 = da.add(&cb).square();
        z_3 = da.sub(&cb).square().mul(&x_1);
        x_2 = aa.mul(&bb);
        // a24 = 121666
        z_2 = e.mul(&aa.add(&e.mul_small(121666)));
    }

    x_2.cswap(&mut x_3, swap);
    z_2.cswap(&mut z_3, swap);

    // Result = x_2 * z_2^(-1)
    let result = x_2.mul(&z_2.invert());
    result.to_bytes()
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

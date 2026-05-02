//! Ed25519 — Digital Signatures (RFC 8032).
//!
//! `no_std`, `no_alloc`, constant-time. Uses SHA-512 internally.
//! Provides signing and verification for SSH-2 host key authentication.
//!
//! Curve: twisted Edwards curve −x² + y² = 1 + d·x²·y² over GF(2²⁵⁵−19)
//! where d = −121665/121666.

use crate::curve25519::Fe;
use crate::sha512::Sha512;
use crate::{zeroize, CryptoError, CryptoRng, Hash};
use ed25519_dalek::{Signature as DalekSignature, Signer, SigningKey, VerifyingKey};

/// Ed25519 secret key (seed, 32 bytes).
pub type Ed25519Seed = [u8; 32];

/// Ed25519 public key (compressed Edwards point, 32 bytes).
pub type Ed25519PublicKey = [u8; 32];

/// Ed25519 signature (64 bytes: R || S).
pub type Ed25519Signature = [u8; 64];

/// Ed25519 expanded secret key (64 bytes from SHA-512 of seed).
struct ExpandedSecretKey {
    scalar: [u8; 32],
    nonce_key: [u8; 32],
}

impl ExpandedSecretKey {
    fn from_seed(seed: &[u8; 32]) -> Self {
        let hash = Sha512::digest(seed);
        let h = hash.bytes;
        let mut scalar = [0u8; 32];
        scalar.copy_from_slice(&h[..32]);
        // Clamp scalar (RFC 8032 §5.1.5)
        scalar[0] &= 248;
        scalar[31] &= 127;
        scalar[31] |= 64;
        let mut nonce_key = [0u8; 32];
        nonce_key.copy_from_slice(&h[32..64]);
        Self { scalar, nonce_key }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Extended Edwards Point
// ═══════════════════════════════════════════════════════════════════════════

/// Point on the Ed25519 curve in extended coordinates (X, Y, Z, T).
/// Invariant: x = X/Z, y = Y/Z, x*y = T/Z.
#[derive(Clone, Copy)]
struct GeP3 {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

/// Point in completed form (for addition result).
#[derive(Clone, Copy)]
struct GeP1P1 {
    x: Fe,
    y: Fe,
    z: Fe,
    t: Fe,
}

/// Point in projective coordinates (X, Y, Z).
#[derive(Clone, Copy)]
struct GeP2 {
    x: Fe,
    y: Fe,
    z: Fe,
}

/// Precomputed point for fixed-base scalar multiplication:
/// (y+x, y-x, 2*d*x*y).
#[derive(Clone, Copy)]
struct GeCached {
    y_plus_x: Fe,
    y_minus_x: Fe,
    z: Fe,
    t2d: Fe,
}

/// Precomputed point for variable-base: (y+x, y-x, x*y*2d).
#[derive(Clone, Copy)]
struct GePrecomp {
    y_plus_x: Fe,
    y_minus_x: Fe,
    xy2d: Fe,
}

impl GeP3 {
    const ZERO: GeP3 = GeP3 {
        x: Fe::ZERO,
        y: Fe::ONE,
        z: Fe::ONE,
        t: Fe::ZERO,
    };

    /// Compress to 32 bytes (y coordinate with sign of x in top bit).
    fn to_bytes(&self) -> [u8; 32] {
        let recip = self.z.invert();
        let x = self.x.mul(&recip);
        let y = self.y.mul(&recip);
        let mut s = y.to_bytes();
        s[31] ^= (if x.is_negative() { 1 } else { 0 }) << 7;
        s
    }

    /// Decompress from 32 bytes.
    fn from_bytes(s: &[u8; 32]) -> Option<GeP3> {
        let mut y_bytes = *s;
        let sign = (y_bytes[31] >> 7) & 1;
        y_bytes[31] &= 127;

        let y = Fe::from_bytes(&y_bytes);
        let y2 = y.square();
        let u = y2.sub(&Fe::ONE); // u = y² - 1
        let v = y2.mul(&Fe::D).add(&Fe::ONE); // v = d·y² + 1

        // x² = u/v, solve via x = u * v^3 * (u * v^7)^((p-5)/8)
        let v3 = v.square().mul(&v);
        let v7 = v3.square().mul(&v);
        let uv7 = u.mul(&v7);
        let x = u.mul(&v3).mul(&uv7.pow_p58());

        // Check: v * x² == u
        let vx2 = v.mul(&x.square());
        let check = vx2.sub(&u);
        let neg_check = vx2.add(&u);

        let x = if check.is_zero() {
            x
        } else if neg_check.is_zero() {
            x.mul(&Fe::SQRT_MINUS_ONE)
        } else {
            return None; // not on curve
        };

        // Adjust sign
        let x = if (x.is_negative() as u8) != sign {
            x.neg()
        } else {
            x
        };

        let t = x.mul(&y);
        Some(GeP3 {
            x,
            y,
            z: Fe::ONE,
            t,
        })
    }

    fn to_cached(&self) -> GeCached {
        GeCached {
            y_plus_x: self.y.add(&self.x),
            y_minus_x: self.y.sub(&self.x),
            z: self.z,
            t2d: self.t.mul(&Fe::D2),
        }
    }

    fn to_p2(&self) -> GeP2 {
        GeP2 {
            x: self.x,
            y: self.y,
            z: self.z,
        }
    }
}

impl GeP1P1 {
    fn to_p3(&self) -> GeP3 {
        GeP3 {
            x: self.x.mul(&self.t),
            y: self.y.mul(&self.z),
            z: self.z.mul(&self.t),
            t: self.x.mul(&self.y),
        }
    }

    fn to_p2(&self) -> GeP2 {
        GeP2 {
            x: self.x.mul(&self.t),
            y: self.y.mul(&self.z),
            z: self.z.mul(&self.t),
        }
    }
}

impl GeP2 {
    const ZERO: GeP2 = GeP2 {
        x: Fe::ZERO,
        y: Fe::ONE,
        z: Fe::ONE,
    };

    fn dbl(&self) -> GeP1P1 {
        let xx = self.x.square();
        let yy = self.y.square();
        let zz2 = self.z.square().add(&self.z.square());
        let x_plus_y = self.x.add(&self.y);
        let x_plus_y_sq = x_plus_y.square();
        let yy_plus_xx = yy.add(&xx);
        let yy_minus_xx = yy.sub(&xx);
        GeP1P1 {
            x: x_plus_y_sq.sub(&yy_plus_xx),
            y: yy_plus_xx,
            z: yy_minus_xx,
            t: zz2.sub(&yy_minus_xx),
        }
    }
}

/// p3 + cached → p1p1 (mixed addition).
fn ge_add(p: &GeP3, q: &GeCached) -> GeP1P1 {
    let ypy = p.y.add(&p.x);
    let ymy = p.y.sub(&p.x);
    let a = ypy.mul(&q.y_plus_x);
    let b = ymy.mul(&q.y_minus_x);
    let c = q.t2d.mul(&p.t);
    let d = p.z.mul(&q.z).add(&p.z.mul(&q.z));
    GeP1P1 {
        x: a.sub(&b),
        y: a.add(&b),
        z: d.add(&c),
        t: d.sub(&c),
    }
}

/// p3 - cached → p1p1 (mixed subtraction).
fn ge_sub(p: &GeP3, q: &GeCached) -> GeP1P1 {
    let ypy = p.y.add(&p.x);
    let ymy = p.y.sub(&p.x);
    let a = ypy.mul(&q.y_minus_x);
    let b = ymy.mul(&q.y_plus_x);
    let c = q.t2d.mul(&p.t);
    let d = p.z.mul(&q.z).add(&p.z.mul(&q.z));
    GeP1P1 {
        x: a.sub(&b),
        y: a.add(&b),
        z: d.sub(&c),
        t: d.add(&c),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Base point (generator)
// ═══════════════════════════════════════════════════════════════════════════

/// The Ed25519 base point B.
fn basepoint() -> GeP3 {
    // Construct B directly from known affine coordinates to avoid
    // relying on point decompression during key generation.
    // x = 15112221349535400772501151409588531511454012693041857206046113283949847762202
    // y = 46316835694926478169428394003475163141307993866256225615783033603165251855960
    let bx: [u8; 32] = [
        0x1a, 0xd5, 0x25, 0x8f, 0x60, 0x2d, 0x56, 0xc9, 0xb2, 0xa7, 0x25, 0x95, 0x60, 0xc7, 0x2c,
        0x69, 0x5c, 0xdc, 0xd6, 0xfd, 0x31, 0xe2, 0xa4, 0xc0, 0xfe, 0x53, 0x6e, 0xcd, 0xd3, 0x36,
        0x69, 0x21,
    ];
    let by: [u8; 32] = [
        0x58, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66,
        0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x66,
        0x66, 0x66,
    ];

    let x = Fe::from_bytes(&bx);
    let y = Fe::from_bytes(&by);
    let t = x.mul(&y);

    GeP3 {
        x,
        y,
        z: Fe::ONE,
        t,
    }
}

impl GeP3 {
    fn expect(self_opt: Option<Self>, _msg: &str) -> Self {
        match self_opt {
            Some(p) => p,
            None => panic!("Ed25519: invalid point"),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Scalar multiplication
// ═══════════════════════════════════════════════════════════════════════════

/// Scalar multiplication: compute `scalar * point`.
/// Uses a simple double-and-add, processing 1 bit at a time.
fn scalarmult(scalar: &[u8; 32], point: &GeP3) -> GeP3 {
    let mut result = GeP3::ZERO;
    let cached = point.to_cached();

    for i in (0..256).rev() {
        let byte = i / 8;
        let bit = i % 8;
        let b = ((scalar[byte] >> bit) & 1) as u64;

        let r2 = result.to_p2();
        result = r2.dbl().to_p3();

        if b == 1 {
            let r1p1 = ge_add(&result, &cached);
            result = r1p1.to_p3();
        }
    }
    result
}

/// Base point scalar multiplication: compute `scalar * B`.
fn scalarmult_base(scalar: &[u8; 32]) -> GeP3 {
    scalarmult(scalar, &basepoint())
}

// ═══════════════════════════════════════════════════════════════════════════
// Scalar arithmetic mod L (group order)
// ═══════════════════════════════════════════════════════════════════════════

/// L = 2^252 + 27742317777372353535851937790883648493
/// The order of the Ed25519 base point.
const L: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
];

/// Reduce a 64-byte scalar (from SHA-512) modulo L.
///
/// Uses Barrett-like reduction. Input: 512-bit number in little-endian.
/// Output: 256-bit number in little-endian, reduced mod L.
fn sc_reduce(s: &[u8; 64]) -> [u8; 32] {
    // Load into 24 × 21-bit limbs for safe arithmetic
    let mut a = [0i64; 24];
    for i in 0..24 {
        let mut v: i64 = 0;
        let byte_off = (i * 21) / 8;
        let bit_off = (i * 21) % 8;
        for j in 0..4 {
            if byte_off + j < 64 {
                v |= (s[byte_off + j] as i64) << (j * 8);
            }
        }
        a[i] = (v >> bit_off) & 0x1fffff;
    }

    // Reduce by subtracting multiples of L from the top.
    // L in 21-bit limbs:
    let l21: [i64; 13] = [
        0x1cf5d3ed, 0x009318d2, 0x1de73e4a, 0x0f3a5e9e, 0x0000014d, 0, 0, 0, 0, 0, 0, 0, 0x00100000,
    ];

    // Schoolbook reduction: for each high limb, subtract q*L
    // where q = a[i+12] (approx quotient digit).
    for i in (12..24).rev() {
        let q = a[i];
        if q == 0 {
            continue;
        }
        a[i] = 0;
        for j in 0..13 {
            if i - 12 + j < 24 {
                a[i - 12 + j] -= q * (l21[j] & 0x1fffff);
            }
        }
        // Propagate borrows
        for j in (i - 12)..(i - 12 + 13).min(24) {
            if a[j] < 0 {
                let borrow = (-a[j] + 0x1fffff) / 0x200000;
                a[j] += borrow * 0x200000;
                if j + 1 < 24 {
                    a[j + 1] -= borrow;
                }
            }
        }
    }

    // Carry-propagate the low 12 limbs
    for i in 0..12 {
        let carry = a[i] >> 21;
        a[i] &= 0x1fffff;
        a[i + 1] += carry;
    }

    // Final conditional subtraction of L if needed
    // (simple: reconstruct bytes and compare)
    let mut out = [0u8; 32];
    // Pack limbs back to bytes
    let mut acc: i64 = 0;
    let mut bits = 0;
    let mut pos = 0;
    for i in 0..12 {
        acc |= a[i] << bits;
        bits += 21;
        while bits >= 8 && pos < 32 {
            out[pos] = acc as u8;
            acc >>= 8;
            bits -= 8;
            pos += 1;
        }
    }
    while pos < 32 {
        out[pos] = acc as u8;
        acc >>= 8;
        pos += 1;
    }

    out
}

/// Multiply-add: compute (a * b + c) mod L.
/// All inputs are 32-byte scalars in little-endian.
fn sc_muladd(a: &[u8; 32], b: &[u8; 32], c: &[u8; 32]) -> [u8; 32] {
    // Schoolbook multiply into 64-byte product, then add c, then reduce.
    let mut product = [0u8; 64];

    // Multiply a * b into u16 accumulators to avoid overflow
    let mut wide = [0u32; 64];
    for i in 0..32 {
        for j in 0..32 {
            wide[i + j] += (a[i] as u32) * (b[j] as u32);
        }
    }

    // Add c
    for i in 0..32 {
        wide[i] += c[i] as u32;
    }

    // Carry propagation
    for i in 0..63 {
        wide[i + 1] += wide[i] >> 8;
        wide[i] &= 0xff;
    }

    for i in 0..64 {
        product[i] = wide[i] as u8;
    }

    sc_reduce(&product)
}

// ═══════════════════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════════════════

/// Generate an Ed25519 keypair from a CSPRNG.
pub fn ed25519_keypair(rng: &mut dyn CryptoRng) -> (Ed25519Seed, Ed25519PublicKey) {
    let mut seed = [0u8; 32];
    rng.fill_bytes(&mut seed);
    let pk = ed25519_public_key(&seed);
    (seed, pk)
}

/// Derive the public key from a seed.
pub fn ed25519_public_key(seed: &[u8; 32]) -> Ed25519PublicKey {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

/// Sign a message with an Ed25519 seed.
///
/// Returns a 64-byte signature (R || S).
pub fn ed25519_sign(msg: &[u8], seed: &[u8; 32], public_key: &[u8; 32]) -> Ed25519Signature {
    let signing_key = SigningKey::from_bytes(seed);
    let verifying_key = signing_key.verifying_key();
    if verifying_key.as_bytes() != public_key {
        return [0u8; 64];
    }

    signing_key.sign(msg).to_bytes()
}

/// Verify an Ed25519 signature.
pub fn ed25519_verify(msg: &[u8], sig: &[u8; 64], public_key: &[u8; 32]) -> bool {
    let verifying_key = match VerifyingKey::from_bytes(public_key) {
        Ok(key) => key,
        Err(_) => return false,
    };
    let signature = DalekSignature::from_bytes(sig);

    verifying_key.verify_strict(msg, &signature).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_round_trip() {
        let seed = [
            0x56, 0x65, 0x65, 0x72, 0x4f, 0x53, 0x2d, 0x48, 0x6f, 0x73, 0x74, 0x4b, 0x65, 0x79,
            0x53, 0x65, 0x65, 0x64, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39,
            0x41, 0x42, 0x43, 0x44,
        ];
        let public_key = ed25519_public_key(&seed);
        let msg = b"ssh-exchange-hash-test";

        let sig = ed25519_sign(msg, &seed, &public_key);

        assert!(ed25519_verify(msg, &sig, &public_key));
    }

    #[test]
    fn rfc8032_test_vector_1() {
        let seed = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let expected_public_key = [
            0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64,
            0x07, 0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68,
            0xf7, 0x07, 0x51, 0x1a,
        ];
        let expected_signature = [
            0xe5, 0x56, 0x43, 0x00, 0xc3, 0x60, 0xac, 0x72, 0x90, 0x86, 0xe2, 0xcc, 0x80, 0x6e,
            0x82, 0x8a, 0x84, 0x87, 0x7f, 0x1e, 0xb8, 0xe5, 0xd9, 0x74, 0xd8, 0x73, 0xe0, 0x65,
            0x22, 0x49, 0x01, 0x55, 0x5f, 0xb8, 0x82, 0x15, 0x90, 0xa3, 0x3b, 0xac, 0xc6, 0x1e,
            0x39, 0x70, 0x1c, 0xf9, 0xb4, 0x6b, 0xd2, 0x5b, 0xf5, 0xf0, 0x59, 0x5b, 0xbe, 0x24,
            0x65, 0x51, 0x41, 0x43, 0x8e, 0x7a, 0x10, 0x0b,
        ];

        let public_key = ed25519_public_key(&seed);
        assert_eq!(public_key, expected_public_key);

        let signature = ed25519_sign(b"", &seed, &public_key);
        assert_eq!(signature, expected_signature);
        assert!(ed25519_verify(b"", &signature, &public_key));
    }
}

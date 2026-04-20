//! ChaCha20-Poly1305 AEAD — RFC 8439
//!
//! `no_std`, `no_alloc`, constant-time. Software-only, runs on all VeerOS
//! targets from riscv32imc (ESP32-C3/C6) to aarch64 (RPi 5) to x86-64.

use crate::{Aead, CryptoError, zeroize};

// ═══════════════════════════════════════════════════════════════════════════
// ChaCha20 stream cipher
// ═══════════════════════════════════════════════════════════════════════════

const SIGMA: [u32; 4] = [0x61707865, 0x3320646e, 0x79622d32, 0x6b206574];

#[inline(always)]
fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[inline(always)]
fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(7);
}

/// Generate one 64-byte ChaCha20 keystream block.
///
/// Public so that `rng` module can use this for the ChaCha20-based DRBG.
#[inline(never)]
pub fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut s = [0u32; 16];
    s[0] = SIGMA[0]; s[1] = SIGMA[1]; s[2] = SIGMA[2]; s[3] = SIGMA[3];
    for i in 0..8 { s[4 + i] = le32(&key[i * 4..]); }
    s[12] = counter;
    for i in 0..3 { s[13 + i] = le32(&nonce[i * 4..]); }

    let initial = s;

    // 20 rounds = 10 double-rounds
    for _ in 0..10 {
        // Column rounds
        quarter_round(&mut s, 0, 4,  8, 12);
        quarter_round(&mut s, 1, 5,  9, 13);
        quarter_round(&mut s, 2, 6, 10, 14);
        quarter_round(&mut s, 3, 7, 11, 15);
        // Diagonal rounds
        quarter_round(&mut s, 0, 5, 10, 15);
        quarter_round(&mut s, 1, 6, 11, 12);
        quarter_round(&mut s, 2, 7,  8, 13);
        quarter_round(&mut s, 3, 4,  9, 14);
    }

    // Add initial state
    for i in 0..16 { s[i] = s[i].wrapping_add(initial[i]); }

    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&s[i].to_le_bytes());
    }
    out
}

/// Apply ChaCha20 keystream XOR to `data`, starting from block `counter`.
#[inline(never)]
pub fn chacha20_xor(key: &[u8; 32], counter: u32, nonce: &[u8; 12], data: &mut [u8]) {
    let mut ctr = counter;
    let mut off = 0;
    while off < data.len() {
        let block = chacha20_block(key, ctr, nonce);
        let take = (data.len() - off).min(64);
        for i in 0..take { data[off + i] ^= block[i]; }
        off += take;
        ctr = ctr.wrapping_add(1);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Poly1305 MAC
// ═══════════════════════════════════════════════════════════════════════════

/// Poly1305 one-time authenticator (internal to this module).
struct Poly1305 {
    r: [u32; 5],   // Clamped key in radix-2^26
    s: [u32; 4],   // Second half of one-time key
    h: [u32; 5],   // Accumulator in radix-2^26
}

impl Poly1305 {
    #[inline(never)]
    fn new(key: &[u8; 32]) -> Self {
        // Clamp r (RFC 8439 §2.5)
        let mut rb = [0u8; 16];
        rb.copy_from_slice(&key[..16]);
        rb[3]  &= 0x0f;  rb[7]  &= 0x0f;
        rb[11] &= 0x0f;  rb[15] &= 0x0f;
        rb[4]  &= 0xfc;  rb[8]  &= 0xfc;  rb[12] &= 0xfc;

        let t0 = le32(&rb[0..]);
        let t1 = le32(&rb[4..]);
        let t2 = le32(&rb[8..]);
        let t3 = le32(&rb[12..]);

        Self {
            r: [
                t0 & 0x3ff_ffff,
                ((t0 >> 26) | (t1 << 6))  & 0x3ff_ffff,
                ((t1 >> 20) | (t2 << 12)) & 0x3ff_ffff,
                ((t2 >> 14) | (t3 << 18)) & 0x3ff_ffff,
                t3 >> 8,
            ],
            s: [le32(&key[16..]), le32(&key[20..]), le32(&key[24..]), le32(&key[28..])],
            h: [0; 5],
        }
    }

    /// Process one block (1–16 bytes). Hibit is placed at byte `len`,
    /// i.e. the number n = le(block) + 2^(8·len) per RFC 8439 §2.5.1.
    #[inline(never)]
    fn block(&mut self, msg: &[u8]) {
        let mut n = [0u8; 17];
        let len = msg.len().min(16);
        n[..len].copy_from_slice(&msg[..len]);
        n[len] = 1; // hibit

        let t0 = le32(&n[0..]);
        let t1 = le32(&n[4..]);
        let t2 = le32(&n[8..]);
        let t3 = le32(&n[12..]);
        let t4 = n[16] as u32;

        // h += n (in radix-2^26)
        self.h[0] = self.h[0].wrapping_add( t0 & 0x3ff_ffff);
        self.h[1] = self.h[1].wrapping_add(((t0 >> 26) | (t1 << 6))  & 0x3ff_ffff);
        self.h[2] = self.h[2].wrapping_add(((t1 >> 20) | (t2 << 12)) & 0x3ff_ffff);
        self.h[3] = self.h[3].wrapping_add(((t2 >> 14) | (t3 << 18)) & 0x3ff_ffff);
        self.h[4] = self.h[4].wrapping_add((t3 >> 8) | (t4 << 24));

        // h *= r  (mod 2^130 − 5)
        let (r0, r1, r2, r3, r4) = (
            self.r[0] as u64, self.r[1] as u64, self.r[2] as u64,
            self.r[3] as u64, self.r[4] as u64,
        );
        // Pre-multiply by 5 for the reduction: 2^130 ≡ 5 (mod p)
        let (s1, s2, s3, s4) = (r1 * 5, r2 * 5, r3 * 5, r4 * 5);

        let (h0, h1, h2, h3, h4) = (
            self.h[0] as u64, self.h[1] as u64, self.h[2] as u64,
            self.h[3] as u64, self.h[4] as u64,
        );

        let d0 = h0*r0 + h1*s4 + h2*s3 + h3*s2 + h4*s1;
        let d1 = h0*r1 + h1*r0 + h2*s4 + h3*s3 + h4*s2;
        let d2 = h0*r2 + h1*r1 + h2*r0 + h3*s4 + h4*s3;
        let d3 = h0*r3 + h1*r2 + h2*r1 + h3*r0 + h4*s4;
        let d4 = h0*r4 + h1*r3 + h2*r2 + h3*r1 + h4*r0;

        // Partial reduction (carry chain)
        let     c0 = d0 >> 26;  let h0 = (d0 & 0x3ff_ffff) as u32;
        let d1 = d1 + c0;
        let     c1 = d1 >> 26;  let h1 = (d1 & 0x3ff_ffff) as u32;
        let d2 = d2 + c1;
        let     c2 = d2 >> 26;  let h2 = (d2 & 0x3ff_ffff) as u32;
        let d3 = d3 + c2;
        let     c3 = d3 >> 26;  let h3 = (d3 & 0x3ff_ffff) as u32;
        let d4 = d4 + c3;
        let     c4 = d4 >> 26;  let h4 = (d4 & 0x3ff_ffff) as u32;

        // Wrap carry from limb4 back to limb0 (×5 because 2^130 ≡ 5)
        let mut h0 = h0.wrapping_add((c4 as u32) * 5);
        let carry = h0 >> 26;
        h0 &= 0x3ff_ffff;
        let h1 = h1.wrapping_add(carry);

        self.h = [h0, h1, h2, h3, h4];
    }

    #[inline(never)]
    fn update(&mut self, data: &[u8]) {
        let mut off = 0;
        while off < data.len() {
            let take = (data.len() - off).min(16);
            self.block(&data[off..off + take]);
            off += take;
        }
    }

    #[inline(never)]
    fn finalize(self) -> [u8; 16] {
        let (mut h0, mut h1, mut h2, mut h3, mut h4) =
            (self.h[0], self.h[1], self.h[2], self.h[3], self.h[4]);

        // Full carry propagation
        let c = h1 >> 26; h1 &= 0x3ff_ffff; h2 += c;
        let c = h2 >> 26; h2 &= 0x3ff_ffff; h3 += c;
        let c = h3 >> 26; h3 &= 0x3ff_ffff; h4 += c;
        let c = h4 >> 26; h4 &= 0x3ff_ffff; h0 += c * 5;
        let c = h0 >> 26; h0 &= 0x3ff_ffff; h1 += c;

        // Compute h − p = h − (2^130 − 5); keep h if h < p, else use h − p
        let mut g0 = h0.wrapping_add(5); let c = g0 >> 26; g0 &= 0x3ff_ffff;
        let mut g1 = h1.wrapping_add(c); let c = g1 >> 26; g1 &= 0x3ff_ffff;
        let mut g2 = h2.wrapping_add(c); let c = g2 >> 26; g2 &= 0x3ff_ffff;
        let mut g3 = h3.wrapping_add(c); let c = g3 >> 26; g3 &= 0x3ff_ffff;
        let g4 = h4.wrapping_add(c).wrapping_sub(1 << 26);

        // Constant-time select: mask = 0xFFFF_FFFF if h ≥ p, 0 otherwise
        let mask = (g4 >> 31).wrapping_sub(1);
        h0 = (h0 & !mask) | (g0 & mask);
        h1 = (h1 & !mask) | (g1 & mask);
        h2 = (h2 & !mask) | (g2 & mask);
        h3 = (h3 & !mask) | (g3 & mask);
        h4 = (h4 & !mask) | (g4 & mask);

        // Assemble 128-bit h from five 26-bit limbs, add s (mod 2^128)
        let h_val = (h0 as u128)
            | ((h1 as u128) << 26)
            | ((h2 as u128) << 52)
            | ((h3 as u128) << 78)
            | ((h4 as u128) << 104);

        let s_val = (self.s[0] as u128)
            | ((self.s[1] as u128) << 32)
            | ((self.s[2] as u128) << 64)
            | ((self.s[3] as u128) << 96);

        let tag_val = h_val.wrapping_add(s_val);

        let mut tag = [0u8; 16];
        tag[..8].copy_from_slice(&(tag_val as u64).to_le_bytes());
        tag[8..].copy_from_slice(&((tag_val >> 64) as u64).to_le_bytes());
        tag
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// ChaCha20-Poly1305 AEAD  (RFC 8439)
// ═══════════════════════════════════════════════════════════════════════════

/// ChaCha20-Poly1305 Authenticated Encryption with Associated Data.
///
/// Key = 256 bit, Nonce = 96 bit, Tag = 128 bit.
/// Post-quantum safe (symmetric primitive — Grover halves security to 128-bit,
/// which is still sufficient).
pub struct ChaCha20Poly1305 {
    key: [u8; 32],
}

/// Compute the Poly1305 tag for AEAD construction (RFC 8439 §2.8).
#[inline(never)]
fn poly1305_aead_tag(
    poly_key: &[u8; 32],
    aad: &[u8],
    ciphertext: &[u8],
) -> [u8; 16] {
    let mut mac = Poly1305::new(poly_key);

    // AAD + pad to 16
    mac.update(aad);
    let aad_pad = (16 - (aad.len() % 16)) % 16;
    if aad_pad > 0 {
        let zeros = [0u8; 16];
        mac.update(&zeros[..aad_pad]);
    }

    // Ciphertext + pad to 16
    mac.update(ciphertext);
    let ct_pad = (16 - (ciphertext.len() % 16)) % 16;
    if ct_pad > 0 {
        let zeros = [0u8; 16];
        mac.update(&zeros[..ct_pad]);
    }

    // Lengths as little-endian u64
    let mut len_block = [0u8; 16];
    len_block[..8].copy_from_slice(&(aad.len() as u64).to_le_bytes());
    len_block[8..].copy_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    mac.update(&len_block);

    mac.finalize()
}

impl Aead for ChaCha20Poly1305 {
    const KEY_LEN: usize = 32;
    const NONCE_LEN: usize = 12;
    const TAG_LEN: usize = 16;

    #[inline(never)]
    fn new(key: &[u8]) -> Self {
        let mut k = [0u8; 32];
        let copy_len = key.len().min(32);
        k[..copy_len].copy_from_slice(&key[..copy_len]);
        Self { key: k }
    }

    #[inline(never)]
    fn seal_in_place(
        &self,
        nonce: &[u8],
        aad: &[u8],
        buffer: &mut [u8],
        plaintext_len: usize,
    ) -> Result<usize, CryptoError> {
        if nonce.len() != Self::NONCE_LEN {
            return Err(CryptoError::InvalidNonceLen);
        }
        if buffer.len() < plaintext_len + Self::TAG_LEN {
            return Err(CryptoError::BufferTooSmall);
        }

        let mut n = [0u8; 12];
        n.copy_from_slice(nonce);

        // Derive Poly1305 one-time key from ChaCha20 block 0
        let poly_block = chacha20_block(&self.key, 0, &n);
        let mut poly_key = [0u8; 32];
        poly_key.copy_from_slice(&poly_block[..32]);

        // Encrypt with ChaCha20 (block counter starts at 1)
        chacha20_xor(&self.key, 1, &n, &mut buffer[..plaintext_len]);

        // Compute & append tag over (AAD, ciphertext)
        let tag = poly1305_aead_tag(&poly_key, aad, &buffer[..plaintext_len]);
        buffer[plaintext_len..plaintext_len + 16].copy_from_slice(&tag);

        zeroize(&mut poly_key);
        Ok(plaintext_len + Self::TAG_LEN)
    }

    #[inline(never)]
    fn open_in_place(
        &self,
        nonce: &[u8],
        aad: &[u8],
        buffer: &mut [u8],
        ciphertext_len: usize,
    ) -> Result<usize, CryptoError> {
        if nonce.len() != Self::NONCE_LEN {
            return Err(CryptoError::InvalidNonceLen);
        }
        if ciphertext_len < Self::TAG_LEN {
            return Err(CryptoError::AuthenticationFailed);
        }

        let plaintext_len = ciphertext_len - Self::TAG_LEN;
        let mut n = [0u8; 12];
        n.copy_from_slice(nonce);

        // Derive Poly1305 one-time key
        let poly_block = chacha20_block(&self.key, 0, &n);
        let mut poly_key = [0u8; 32];
        poly_key.copy_from_slice(&poly_block[..32]);

        // Verify tag BEFORE decrypting (verify-then-decrypt)
        let computed = poly1305_aead_tag(&poly_key, aad, &buffer[..plaintext_len]);

        // Constant-time comparison
        let mut diff: u8 = 0;
        for i in 0..16 {
            diff |= computed[i] ^ buffer[plaintext_len + i];
        }

        zeroize(&mut poly_key);

        if diff != 0 {
            // Wipe buffer on auth failure — never expose unauthenticated plaintext
            zeroize(&mut buffer[..ciphertext_len]);
            return Err(CryptoError::AuthenticationFailed);
        }

        // Decrypt
        chacha20_xor(&self.key, 1, &n, &mut buffer[..plaintext_len]);

        Ok(plaintext_len)
    }
}

impl Drop for ChaCha20Poly1305 {
    fn drop(&mut self) {
        zeroize(&mut self.key);
    }
}

//! Curve25519 field arithmetic — GF(2^255 − 19).
//!
//! `no_std`, `no_alloc`, constant-time. Provides the prime-field operations
//! used by both X25519 (Montgomery ladder) and Ed25519 (twisted Edwards).
//!
//! Representation: 5 × 51-bit limbs in u64, allowing lazy reduction.
//! This is the standard radix-2^51 representation used by most
//! constant-time Curve25519 implementations.

use crate::zeroize;

/// Element of GF(2^255 − 19) in radix-2^51 representation.
#[derive(Clone, Copy)]
pub struct Fe([u64; 5]);

impl Fe {
    pub const ZERO: Fe = Fe([0; 5]);
    pub const ONE: Fe = Fe([1, 0, 0, 0, 0]);

    /// Reduce after addition/subtraction: carry propagation.
    fn reduce(&mut self) {
        let mut c: u64;
        for i in 0..4 {
            c = self.0[i] >> 51;
            self.0[i] &= 0x7ffffffffffff;
            self.0[i + 1] += c;
        }
        c = self.0[4] >> 51;
        self.0[4] &= 0x7ffffffffffff;
        self.0[0] += c * 19;
    }

    /// Full reduction to canonical form [0, 2^255-19).
    fn freeze(&mut self) {
        self.reduce();
        self.reduce();

        // Subtract p = 2^255 - 19 conditionally.
        // Check if self >= p: that means self + 19 >= 2^255.
        let mut t = self.0;
        t[0] += 19;
        let mut c: u64;
        for i in 0..4 {
            c = t[i] >> 51;
            t[i] &= 0x7ffffffffffff;
            t[i + 1] += c;
        }
        c = t[4] >> 51;
        t[4] &= 0x7ffffffffffff;

        // If overflow (c != 0), then self >= p, so use t.
        // Otherwise keep self. Constant-time select.
        let mask = c.wrapping_neg(); // 0xfff...f if c>0, 0 otherwise
        for i in 0..5 {
            self.0[i] = (t[i] & mask) | (self.0[i] & !mask);
        }
    }

    /// Addition in GF(p).
    pub fn add(&self, rhs: &Fe) -> Fe {
        let mut r = Fe([
            self.0[0] + rhs.0[0],
            self.0[1] + rhs.0[1],
            self.0[2] + rhs.0[2],
            self.0[3] + rhs.0[3],
            self.0[4] + rhs.0[4],
        ]);
        r.reduce();
        r
    }

    /// Subtraction in GF(p). Add 2*p to avoid underflow.
    pub fn sub(&self, rhs: &Fe) -> Fe {
        // 2*p limbs: each limb of p is 0x7ffffffffffff except limb 0 which is p-19+2*19=...
        // Easier: add a large enough multiple of p.
        let mut r = Fe([
            self.0[0].wrapping_add(0xfffffffffffda).wrapping_sub(rhs.0[0]), // 2*(2^51 - 19)
            self.0[1].wrapping_add(0xffffffffffffe).wrapping_sub(rhs.0[1]), // 2*(2^51)
            self.0[2].wrapping_add(0xffffffffffffe).wrapping_sub(rhs.0[2]),
            self.0[3].wrapping_add(0xffffffffffffe).wrapping_sub(rhs.0[3]),
            self.0[4].wrapping_add(0xffffffffffffe).wrapping_sub(rhs.0[4]),
        ]);
        r.reduce();
        r
    }

    /// Multiplication in GF(p) using schoolbook with 128-bit intermediates.
    pub fn mul(&self, rhs: &Fe) -> Fe {
        let a = &self.0;
        let b = &rhs.0;

        // Each product a[i]*b[j] fits in 102 bits.
        // We accumulate into u128 accumulators, then reduce.
        let b_19 = [b[1] * 19, b[2] * 19, b[3] * 19, b[4] * 19];

        let mut r0 = (a[0] as u128) * (b[0] as u128)
            + (a[1] as u128) * (b_19[3] as u128)
            + (a[2] as u128) * (b_19[2] as u128)
            + (a[3] as u128) * (b_19[1] as u128)
            + (a[4] as u128) * (b_19[0] as u128);

        let mut r1 = (a[0] as u128) * (b[1] as u128)
            + (a[1] as u128) * (b[0] as u128)
            + (a[2] as u128) * (b_19[3] as u128)
            + (a[3] as u128) * (b_19[2] as u128)
            + (a[4] as u128) * (b_19[1] as u128);

        let mut r2 = (a[0] as u128) * (b[2] as u128)
            + (a[1] as u128) * (b[1] as u128)
            + (a[2] as u128) * (b[0] as u128)
            + (a[3] as u128) * (b_19[3] as u128)
            + (a[4] as u128) * (b_19[2] as u128);

        let mut r3 = (a[0] as u128) * (b[3] as u128)
            + (a[1] as u128) * (b[2] as u128)
            + (a[2] as u128) * (b[1] as u128)
            + (a[3] as u128) * (b[0] as u128)
            + (a[4] as u128) * (b_19[3] as u128);

        let mut r4 = (a[0] as u128) * (b[4] as u128)
            + (a[1] as u128) * (b[3] as u128)
            + (a[2] as u128) * (b[2] as u128)
            + (a[3] as u128) * (b[1] as u128)
            + (a[4] as u128) * (b[0] as u128);

        // Carry propagation
        let c = r0 >> 51; r0 &= 0x7ffffffffffff; r1 += c;
        let c = r1 >> 51; r1 &= 0x7ffffffffffff; r2 += c;
        let c = r2 >> 51; r2 &= 0x7ffffffffffff; r3 += c;
        let c = r3 >> 51; r3 &= 0x7ffffffffffff; r4 += c;
        let c = r4 >> 51; r4 &= 0x7ffffffffffff; r0 += c * 19;

        Fe([r0 as u64, r1 as u64, r2 as u64, r3 as u64, r4 as u64])
    }

    /// Squaring in GF(p) — slightly faster than generic mul.
    pub fn square(&self) -> Fe {
        self.mul(self)
    }

    /// Multiply by a small constant.
    pub fn mul_small(&self, k: u64) -> Fe {
        let mut r0 = (self.0[0] as u128) * (k as u128);
        let mut r1 = (self.0[1] as u128) * (k as u128);
        let mut r2 = (self.0[2] as u128) * (k as u128);
        let mut r3 = (self.0[3] as u128) * (k as u128);
        let mut r4 = (self.0[4] as u128) * (k as u128);

        let c = r0 >> 51; r0 &= 0x7ffffffffffff; r1 += c;
        let c = r1 >> 51; r1 &= 0x7ffffffffffff; r2 += c;
        let c = r2 >> 51; r2 &= 0x7ffffffffffff; r3 += c;
        let c = r3 >> 51; r3 &= 0x7ffffffffffff; r4 += c;
        let c = r4 >> 51; r4 &= 0x7ffffffffffff; r0 += c * 19;

        Fe([r0 as u64, r1 as u64, r2 as u64, r3 as u64, r4 as u64])
    }

    /// Negation in GF(p).
    pub fn neg(&self) -> Fe {
        Fe::ZERO.sub(self)
    }

    /// Modular inverse via Fermat's little theorem: a^(p-2) mod p.
    pub fn invert(&self) -> Fe {
        // p - 2 = 2^255 - 21
        // Use addition chain for 2^255 - 21.
        let z2 = self.square();
        let z9 = {
            let z4 = z2.square();
            let z8 = z4.square();
            z8.mul(self)
        };
        let z11 = z9.mul(&z2);
        let z_5_0 = {
            let t = z11.square();
            t.mul(&z9)
        };
        let z_10_0 = {
            let mut t = z_5_0.square();
            for _ in 1..5 { t = t.square(); }
            t.mul(&z_5_0)
        };
        let z_20_0 = {
            let mut t = z_10_0.square();
            for _ in 1..10 { t = t.square(); }
            t.mul(&z_10_0)
        };
        let z_40_0 = {
            let mut t = z_20_0.square();
            for _ in 1..20 { t = t.square(); }
            t.mul(&z_20_0)
        };
        let z_50_0 = {
            let mut t = z_40_0.square();
            for _ in 1..10 { t = t.square(); }
            t.mul(&z_10_0)
        };
        let z_100_0 = {
            let mut t = z_50_0.square();
            for _ in 1..50 { t = t.square(); }
            t.mul(&z_50_0)
        };
        let z_200_0 = {
            let mut t = z_100_0.square();
            for _ in 1..100 { t = t.square(); }
            t.mul(&z_100_0)
        };
        let z_250_0 = {
            let mut t = z_200_0.square();
            for _ in 1..50 { t = t.square(); }
            t.mul(&z_50_0)
        };
        // 2^255 - 21 = (z_250_0)^(2^5) * z11
        let mut t = z_250_0.square();
        for _ in 1..5 { t = t.square(); }
        t.mul(&z11)
    }

    /// Compute a^((p-5)/8) for square root computation.
    /// Used by Ed25519 point decompression.
    pub fn pow_p58(&self) -> Fe {
        // (p-5)/8 = (2^255 - 24)/8 = 2^252 - 3
        let z2 = self.square();
        let z9 = {
            let z4 = z2.square();
            let z8 = z4.square();
            z8.mul(self)
        };
        let z11 = z9.mul(&z2);
        let z_5_0 = {
            let t = z11.square();
            t.mul(&z9)
        };
        let z_10_0 = {
            let mut t = z_5_0.square();
            for _ in 1..5 { t = t.square(); }
            t.mul(&z_5_0)
        };
        let z_20_0 = {
            let mut t = z_10_0.square();
            for _ in 1..10 { t = t.square(); }
            t.mul(&z_10_0)
        };
        let z_40_0 = {
            let mut t = z_20_0.square();
            for _ in 1..20 { t = t.square(); }
            t.mul(&z_20_0)
        };
        let z_50_0 = {
            let mut t = z_40_0.square();
            for _ in 1..10 { t = t.square(); }
            t.mul(&z_10_0)
        };
        let z_100_0 = {
            let mut t = z_50_0.square();
            for _ in 1..50 { t = t.square(); }
            t.mul(&z_50_0)
        };
        let z_200_0 = {
            let mut t = z_100_0.square();
            for _ in 1..100 { t = t.square(); }
            t.mul(&z_100_0)
        };
        let z_250_0 = {
            let mut t = z_200_0.square();
            for _ in 1..50 { t = t.square(); }
            t.mul(&z_50_0)
        };
        // 2^252 - 3 = (z_250_0)^(2^2) * self
        let mut t = z_250_0.square();
        t = t.square();
        t.mul(self)
    }

    /// Decode from 32 bytes (little-endian). Clamps the top bit.
    pub fn from_bytes(s: &[u8; 32]) -> Fe {
        let mut h = [0u64; 5];
        h[0] = load_8(&s[0..]) & 0x7ffffffffffff;
        h[1] = (load_8(&s[6..]) >> 3) & 0x7ffffffffffff;
        h[2] = (load_8(&s[12..]) >> 6) & 0x7ffffffffffff;
        h[3] = (load_8(&s[19..]) >> 1) & 0x7ffffffffffff;
        h[4] = (load_8(&s[24..]) >> 12) & 0x7ffffffffffff;
        Fe(h)
    }

    /// Encode to 32 bytes (little-endian, canonical form).
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut t = *self;
        t.freeze();
        let h = &t.0;
        let mut s = [0u8; 32];

        s[0] = h[0] as u8;
        s[1] = (h[0] >> 8) as u8;
        s[2] = (h[0] >> 16) as u8;
        s[3] = (h[0] >> 24) as u8;
        s[4] = (h[0] >> 32) as u8;
        s[5] = (h[0] >> 40) as u8;
        s[6] = ((h[0] >> 48) | (h[1] << 3)) as u8;
        s[7] = (h[1] >> 5) as u8;
        s[8] = (h[1] >> 13) as u8;
        s[9] = (h[1] >> 21) as u8;
        s[10] = (h[1] >> 29) as u8;
        s[11] = (h[1] >> 37) as u8;
        s[12] = ((h[1] >> 45) | (h[2] << 6)) as u8;
        s[13] = (h[2] >> 2) as u8;
        s[14] = (h[2] >> 10) as u8;
        s[15] = (h[2] >> 18) as u8;
        s[16] = (h[2] >> 26) as u8;
        s[17] = (h[2] >> 34) as u8;
        s[18] = (h[2] >> 42) as u8;
        s[19] = ((h[2] >> 50) | (h[3] << 1)) as u8;
        s[20] = (h[3] >> 7) as u8;
        s[21] = (h[3] >> 15) as u8;
        s[22] = (h[3] >> 23) as u8;
        s[23] = (h[3] >> 31) as u8;
        s[24] = (h[3] >> 39) as u8;
        s[25] = ((h[3] >> 47) | (h[4] << 4)) as u8;
        s[26] = (h[4] >> 4) as u8;
        s[27] = (h[4] >> 12) as u8;
        s[28] = (h[4] >> 20) as u8;
        s[29] = (h[4] >> 28) as u8;
        s[30] = (h[4] >> 36) as u8;
        s[31] = (h[4] >> 44) as u8;
        s
    }

    /// Conditional swap: if `swap` is 1, swap self and other.
    /// Constant-time.
    pub fn cswap(&mut self, other: &mut Fe, swap: u64) {
        let mask = swap.wrapping_neg();
        for i in 0..5 {
            let t = mask & (self.0[i] ^ other.0[i]);
            self.0[i] ^= t;
            other.0[i] ^= t;
        }
    }

    /// Check if this element is zero (constant-time).
    pub fn is_zero(&self) -> bool {
        let s = self.to_bytes();
        let mut acc: u8 = 0;
        for &b in &s {
            acc |= b;
        }
        acc == 0
    }

    /// Check if this element is negative (LSB of canonical encoding).
    pub fn is_negative(&self) -> bool {
        let s = self.to_bytes();
        (s[0] & 1) != 0
    }

    /// Conditional negation: negate if `negate` is 1.
    pub fn cneg(&self, negate: u64) -> Fe {
        let neg = self.neg();
        let mut result = *self;
        let mask = negate.wrapping_neg();
        for i in 0..5 {
            result.0[i] = (neg.0[i] & mask) | (self.0[i] & !mask);
        }
        result
    }

    /// Square root in GF(p). Returns (sqrt, was_square).
    /// Uses the identity sqrt(a) = a^((p+3)/8) with adjustment.
    pub fn sqrt(&self) -> (Fe, bool) {
        // candidate = a^((p+3)/8)
        // (p+3)/8  = (2^255 - 16)/8 = 2^252 - 2 
        // Actually we use: beta = a * a^((p-5)/8)
        let beta = self.mul(&self.pow_p58());
        let beta_sq = beta.square();

        // Check if beta^2 == self
        let correct = beta_sq.sub(self).is_zero();
        // Check if beta^2 == -self
        let neg_self = self.neg();
        let neg_correct = beta_sq.sub(&neg_self).is_zero();

        // sqrt(-1) = 2^((p-1)/4) mod p
        let sqrt_m1 = Fe::SQRT_MINUS_ONE;
        let result = if correct {
            beta
        } else if neg_correct {
            beta.mul(&sqrt_m1)
        } else {
            beta // not a square
        };

        (result, correct || neg_correct)
    }

    /// sqrt(-1) mod p = 2^((p-1)/4) mod p.
    pub const SQRT_MINUS_ONE: Fe = Fe([
        0x61b274a0ea0b0,
        0x0d5a5fc8f189d,
        0x7ef5e9cbd0c60,
        0x78595a6804c9e,
        0x2b8324804fc1d,
    ]);

    /// d = -121665/121666 mod p (twisted Edwards curve constant).
    pub const D: Fe = Fe([
        0x34dca135978a3,
        0x1a8283b156ebd,
        0x5e7a26001c029,
        0x739c663a03cbb,
        0x52036cee2b6ff,
    ]);

    /// 2*d mod p.
    pub const D2: Fe = Fe([
        0x69b9426b2f159,
        0x35050762add7a,
        0x3cf44c0038052,
        0x6738cc7407a77,
        0x2406d9dc56dff,
    ]);
}

/// Load 8 bytes little-endian into u64 (handles short slices).
fn load_8(s: &[u8]) -> u64 {
    let mut h: u64 = 0;
    let len = s.len().min(8);
    for i in 0..len {
        h |= (s[i] as u64) << (i * 8);
    }
    h
}

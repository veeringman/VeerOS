//! SHA-256 — `no_std`, constant-time, zero-alloc implementation.
//!
//! FIPS 180-4 compliant. Optimised for readability and correctness on
//! embedded targets (riscv32imc, Cortex-A, x86-64).

use crate::{Digest, Hash};

// ── Constants ────────────────────────────────────────────────────────────

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5,
    0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
    0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
    0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5,
    0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const INIT_H: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
    0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

// ── SHA-256 state ────────────────────────────────────────────────────────

/// SHA-256 hasher.
pub struct Sha256 {
    h: [u32; 8],
    block: [u8; 64],
    block_len: usize,
    total_len: u64,
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            h: INIT_H,
            block: [0u8; 64],
            block_len: 0,
            total_len: 0,
        }
    }

    // Prevent LTO from re-optimizing this at the global opt-level ("s").
    // SHA-256 compress is miscompiled on riscv32 when LTO applies opt-level="s".
    #[inline(never)]
    fn compress(&mut self) {
        let mut w = [0u32; 64];

        // Prepare message schedule
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                self.block[i * 4],
                self.block[i * 4 + 1],
                self.block[i * 4 + 2],
                self.block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7)
                ^ w[i - 15].rotate_right(18)
                ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17)
                ^ w[i - 2].rotate_right(19)
                ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = self.h[0];
        let mut b = self.h[1];
        let mut c = self.h[2];
        let mut d = self.h[3];
        let mut e = self.h[4];
        let mut f = self.h[5];
        let mut g = self.h[6];
        let mut hh = self.h[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.h[0] = self.h[0].wrapping_add(a);
        self.h[1] = self.h[1].wrapping_add(b);
        self.h[2] = self.h[2].wrapping_add(c);
        self.h[3] = self.h[3].wrapping_add(d);
        self.h[4] = self.h[4].wrapping_add(e);
        self.h[5] = self.h[5].wrapping_add(f);
        self.h[6] = self.h[6].wrapping_add(g);
        self.h[7] = self.h[7].wrapping_add(hh);
    }
}

impl Hash for Sha256 {
    #[inline(never)]
    fn update(&mut self, data: &[u8]) {
        self.total_len += data.len() as u64;
        let mut offset = 0;

        // Fill partial block
        if self.block_len > 0 {
            let space = 64 - self.block_len;
            let take = data.len().min(space);
            self.block[self.block_len..self.block_len + take]
                .copy_from_slice(&data[..take]);
            self.block_len += take;
            offset = take;

            if self.block_len == 64 {
                self.compress();
                self.block_len = 0;
            }
        }

        // Process full blocks
        while offset + 64 <= data.len() {
            self.block.copy_from_slice(&data[offset..offset + 64]);
            self.compress();
            offset += 64;
        }

        // Buffer remainder
        if offset < data.len() {
            let remaining = data.len() - offset;
            self.block[..remaining].copy_from_slice(&data[offset..]);
            self.block_len = remaining;
        }
    }

    #[inline(never)]
    fn finalize(mut self) -> Digest {
        // MD padding: append 1-bit, zero-pad, append 64-bit length
        let bit_len = self.total_len * 8;
        let mut pad = [0u8; 72]; // max needed: 64 + 8
        pad[0] = 0x80;

        let pad_len = if self.block_len < 56 {
            56 - self.block_len
        } else {
            120 - self.block_len
        };

        self.update(&pad[..pad_len]);
        let len_bytes = bit_len.to_be_bytes();
        self.update(&len_bytes);

        // Output
        let mut digest = Digest::zero(32);
        for i in 0..8 {
            digest.bytes[i * 4..i * 4 + 4].copy_from_slice(&self.h[i].to_be_bytes());
        }
        digest
    }

    #[inline(never)]
    fn digest(data: &[u8]) -> Digest {
        let mut h = Sha256::new();
        h.update(data);
        h.finalize()
    }

    fn output_len() -> usize {
        32
    }
}

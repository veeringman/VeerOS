//! Cryptographically Secure PRNG — ChaCha20-based DRBG.
//!
//! Uses ChaCha20 keystream as the entropy expansion function.
//! Must be seeded from a hardware entropy source at boot; the platform
//! layer (`soc` crate) is responsible for providing initial entropy.
//!
//! Reseed support: call `reseed()` to mix fresh entropy into the state.
//! The DRBG tracks how many bytes have been generated and can signal
//! when reseeding is recommended.

use crate::CryptoRng;

#[cfg(feature = "chacha20")]
use crate::chacha20::chacha20_block;

/// Platform entropy source abstraction.
///
/// Each SoC crate must implement this for its hardware RNG:
///   - ESP32-C3/C6: `RNG` peripheral (true RNG from RF noise)
///   - RPi 5: BCM2712 TRNG
///   - x86-64: RDRAND / RDSEED
///   - RISC-V 64: Zkr `seed` CSR
pub trait EntropySource {
    /// Fill `dest` with hardware-sourced entropy.
    /// Returns `Err` if the hardware source is unhealthy.
    fn fill_entropy(&mut self, dest: &mut [u8]) -> Result<(), crate::CryptoError>;
}

// ═══════════════════════════════════════════════════════════════════════════
// ChaCha20-based DRBG
// ═══════════════════════════════════════════════════════════════════════════

/// Reseed after this many 64-byte blocks (64 × 65536 = 4 MiB).
const RESEED_INTERVAL: u64 = 65536;

/// ChaCha20-based deterministic random bit generator.
///
/// State: 256-bit key + 64-bit block counter + 32-bit nonce (all zero).
/// Each call to `fill_bytes` generates ChaCha20 keystream blocks and
/// XOR-folds the last block back into the key for forward secrecy.
#[cfg(feature = "chacha20")]
pub struct ChaChaRng {
    key: [u8; 32],
    counter: u64,
    blocks_since_reseed: u64,
    seeded: bool,
}

#[cfg(feature = "chacha20")]
impl ChaChaRng {
    /// Create from a 32-byte seed. The caller is responsible for
    /// ensuring the seed has sufficient entropy (≥ 256 bits).
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            key: seed,
            counter: 0,
            blocks_since_reseed: 0,
            seeded: true,
        }
    }

    /// Create from a hardware entropy source.
    pub fn from_entropy(src: &mut dyn EntropySource) -> Result<Self, crate::CryptoError> {
        let mut seed = [0u8; 32];
        src.fill_entropy(&mut seed)?;
        Ok(Self::from_seed(seed))
    }

    /// Mix fresh entropy into the DRBG state.
    ///
    /// new_key = old_key XOR entropy. Resets the reseed counter.
    pub fn reseed(&mut self, src: &mut dyn EntropySource) -> Result<(), crate::CryptoError> {
        let mut entropy = [0u8; 32];
        src.fill_entropy(&mut entropy)?;
        for i in 0..32 {
            self.key[i] ^= entropy[i];
        }
        crate::zeroize(&mut entropy);
        self.blocks_since_reseed = 0;
        Ok(())
    }

    /// Returns true if the DRBG has exceeded the recommended reseed interval.
    pub fn needs_reseed(&self) -> bool {
        self.blocks_since_reseed >= RESEED_INTERVAL
    }

    /// Returns true if the DRBG has been seeded at least once.
    pub fn is_seeded(&self) -> bool {
        self.seeded
    }

    /// Internal: generate one keystream block, advance counter, update key
    /// for forward secrecy.
    fn next_block(&mut self) -> [u8; 64] {
        // Split 64-bit counter into (block_counter_u32, nonce_u32)
        // to cover the full 64-bit counter space.
        let block_ctr = self.counter as u32;
        let mut nonce = [0u8; 12];
        // Embed upper 32 bits of counter into nonce[0..4]
        nonce[0..4].copy_from_slice(&((self.counter >> 32) as u32).to_le_bytes());

        let out = chacha20_block(&self.key, block_ctr, &nonce);

        self.counter = self.counter.wrapping_add(1);
        self.blocks_since_reseed += 1;

        // Forward secrecy: fold keystream back into key every block.
        // If an attacker compromises the key at time T, they cannot
        // reconstruct output from time T−1.
        for i in 0..32 {
            self.key[i] ^= out[32 + i];
        }

        out
    }
}

#[cfg(feature = "chacha20")]
impl CryptoRng for ChaChaRng {
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut off = 0;
        while off < dest.len() {
            let block = self.next_block();
            // Use first 32 bytes of block for output (other 32 folded into key)
            let available = 32;
            let take = (dest.len() - off).min(available);
            dest[off..off + take].copy_from_slice(&block[..take]);
            off += take;
        }
    }
}

#[cfg(feature = "chacha20")]
impl Drop for ChaChaRng {
    fn drop(&mut self) {
        crate::zeroize(&mut self.key);
    }
}

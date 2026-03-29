//! VeerOS Cryptographic Primitives
//!
//! `no_std`, `no_alloc` cryptographic building blocks for VeerOS.
//! All implementations are constant-time where security-relevant.
//!
//! # Architecture: Hybrid Classical + PQC
//!
//! VeerOS uses a **hybrid approach** to post-quantum cryptography.
//! The migration path is controlled by [`hybrid::CryptoMode`]:
//!
//! | Phase | Mode       | KEM              | Signatures          |
//! |-------|------------|------------------|---------------------|
//! | 1     | Classical  | X25519           | Ed25519             |
//! | 2     | Hybrid     | X25519 + ML-KEM  | Ed25519 + ML-DSA    |
//! | 3     | PqcOnly    | ML-KEM           | ML-DSA              |
//!
//! Symmetric primitives (SHA-256, ChaCha20-Poly1305, HKDF) are already
//! post-quantum safe and require no hybridization.
//!
//! # Feature Flags
//!
//! ## Symmetric (quantum-safe)
//! - `sha256`   — SHA-256 hash (default)
//! - `chacha20` — ChaCha20-Poly1305 AEAD + ChaCha20-DRBG RNG (default)
//! - `blake3`   — BLAKE3 hash (future)
//!
//! ## Classical asymmetric
//! - `x25519`  — X25519 ECDH key exchange (future)
//! - `ed25519` — Ed25519 signatures (future)
//!
//! ## Post-quantum asymmetric (NIST FIPS 203/204)
//! - `ml-kem` — ML-KEM-768 (Kyber) key encapsulation (future)
//! - `ml-dsa` — ML-DSA-65 (Dilithium) digital signatures (future)
//!
//! ## Hybrid combiners
//! - `hybrid`   — Classical + PQC combiner logic (requires `sha256`)
//! - `pqc-only` — Drop classical, pure PQC (future migration endpoint)

#![no_std]

// ── Modules ──────────────────────────────────────────────────────────────

#[cfg(feature = "sha256")]
pub mod sha256;
#[cfg(feature = "chacha20")]
pub mod chacha20;
#[cfg(feature = "chacha20")]
pub mod rng;
#[cfg(feature = "hybrid")]
pub mod hybrid;

// ═══════════════════════════════════════════════════════════════════════════
// Core Crypto Traits
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum digest size in bytes (SHA-512 = 64, most others ≤ 32).
pub const MAX_DIGEST_LEN: usize = 64;

/// A fixed-size digest / hash output.
#[derive(Clone, Copy)]
pub struct Digest {
    pub bytes: [u8; MAX_DIGEST_LEN],
    pub len: usize,
}

impl Digest {
    pub const fn zero(len: usize) -> Self {
        Self {
            bytes: [0u8; MAX_DIGEST_LEN],
            len,
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    /// Constant-time equality comparison.
    pub fn ct_eq(&self, other: &Digest) -> bool {
        if self.len != other.len {
            return false;
        }
        let mut diff: u8 = 0;
        for i in 0..self.len {
            diff |= self.bytes[i] ^ other.bytes[i];
        }
        diff == 0
    }
}

impl core::fmt::Debug for Digest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for &b in &self.bytes[..self.len] {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

// ── Hash trait ───────────────────────────────────────────────────────────

/// Streaming hash function interface.
pub trait Hash {
    /// Feed data into the hash.
    fn update(&mut self, data: &[u8]);
    /// Finalize and return the digest. Consumes the hasher state.
    fn finalize(self) -> Digest;
    /// Convenience: hash a single buffer in one call.
    fn digest(data: &[u8]) -> Digest
    where
        Self: Sized;
    /// Output length in bytes.
    fn output_len() -> usize
    where
        Self: Sized;
}

// ── MAC trait ────────────────────────────────────────────────────────────

/// Message Authentication Code (HMAC, Poly1305, etc.)
pub trait Mac {
    /// Feed data into the MAC.
    fn update(&mut self, data: &[u8]);
    /// Finalize and return the authentication tag.
    fn finalize(self) -> Digest;
    /// Verify a tag in constant time.
    fn verify(self, expected: &[u8]) -> bool;
}

// ── AEAD trait ───────────────────────────────────────────────────────────

/// Authenticated Encryption with Associated Data.
pub trait Aead {
    /// Key size in bytes.
    const KEY_LEN: usize;
    /// Nonce size in bytes.
    const NONCE_LEN: usize;
    /// Authentication tag size in bytes.
    const TAG_LEN: usize;

    /// Create an AEAD instance from a key.
    fn new(key: &[u8]) -> Self;

    /// Encrypt in place: appends TAG_LEN bytes of tag to `buffer[..plaintext_len]`.
    /// `buffer` must have capacity for `plaintext_len + TAG_LEN`.
    /// Returns total length (plaintext_len + TAG_LEN) on success.
    fn seal_in_place(
        &self,
        nonce: &[u8],
        aad: &[u8],
        buffer: &mut [u8],
        plaintext_len: usize,
    ) -> Result<usize, CryptoError>;

    /// Decrypt in place: verifies tag and removes it.
    /// `buffer[..ciphertext_len]` includes the TAG_LEN-byte tag at the end.
    /// Returns plaintext length on success.
    fn open_in_place(
        &self,
        nonce: &[u8],
        aad: &[u8],
        buffer: &mut [u8],
        ciphertext_len: usize,
    ) -> Result<usize, CryptoError>;
}

// ── KDF trait ────────────────────────────────────────────────────────────

/// Key Derivation Function.
pub trait Kdf {
    /// Derive key material.
    /// `ikm` = input keying material, `salt` = optional salt,
    /// `info` = context/application info, output written to `okm`.
    fn derive(ikm: &[u8], salt: &[u8], info: &[u8], okm: &mut [u8]);
}

// ── Sign / Verify traits ─────────────────────────────────────────────────

/// Maximum signature size in bytes.
pub const MAX_SIG_LEN: usize = 128;

/// A fixed-size signature.
#[derive(Clone, Copy)]
pub struct Signature {
    pub bytes: [u8; MAX_SIG_LEN],
    pub len: usize,
}

impl Signature {
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Digital signature — sign with private key.
pub trait Signer {
    fn sign(&self, msg: &[u8]) -> Result<Signature, CryptoError>;
}

/// Signature verification — verify with public key.
pub trait Verifier {
    fn verify(&self, msg: &[u8], sig: &Signature) -> Result<bool, CryptoError>;
}

// ── KEM trait ────────────────────────────────────────────────────────────

/// Maximum shared secret size.
pub const MAX_SS_LEN: usize = 64;
/// Maximum KEM ciphertext size (ML-KEM-768 = 1088 bytes — will need larger for PQC).
pub const MAX_KEM_CT_LEN: usize = 1088;

/// Key Encapsulation Mechanism.
pub trait Kem {
    /// Shared secret length in bytes.
    const SS_LEN: usize;
    /// Ciphertext length in bytes.
    const CT_LEN: usize;

    /// Encapsulate: given a public key, produce (shared_secret, ciphertext).
    fn encapsulate(
        pk: &[u8],
        ss_out: &mut [u8],
        ct_out: &mut [u8],
        rng: &mut dyn CryptoRng,
    ) -> Result<(), CryptoError>;

    /// Decapsulate: given secret key + ciphertext, recover shared_secret.
    fn decapsulate(
        sk: &[u8],
        ct: &[u8],
        ss_out: &mut [u8],
    ) -> Result<(), CryptoError>;
}

// ── RNG trait ────────────────────────────────────────────────────────────

/// Cryptographically secure random number generator.
pub trait CryptoRng {
    /// Fill `dest` with cryptographically secure random bytes.
    fn fill_bytes(&mut self, dest: &mut [u8]);

    /// Return a random u32.
    fn next_u32(&mut self) -> u32 {
        let mut buf = [0u8; 4];
        self.fill_bytes(&mut buf);
        u32::from_le_bytes(buf)
    }

    /// Return a random u64.
    fn next_u64(&mut self) -> u64 {
        let mut buf = [0u8; 8];
        self.fill_bytes(&mut buf);
        u64::from_le_bytes(buf)
    }
}

// ── Errors ───────────────────────────────────────────────────────────────

/// Cryptographic operation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// Invalid key length.
    InvalidKeyLen,
    /// Invalid nonce length.
    InvalidNonceLen,
    /// Authentication tag verification failed.
    AuthenticationFailed,
    /// Buffer too small for output.
    BufferTooSmall,
    /// RNG failure (entropy source unhealthy).
    RngFailure,
    /// Algorithm not available on this target.
    Unavailable,
    /// Invalid signature format.
    InvalidSignature,
    /// Generic internal error.
    InternalError,
}

// ── Zeroize ──────────────────────────────────────────────────────────────

/// Securely zero memory. Uses a volatile write to prevent compiler elision.
#[inline(always)]
pub fn zeroize(buf: &mut [u8]) {
    for byte in buf.iter_mut() {
        unsafe {
            core::ptr::write_volatile(byte, 0);
        }
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

// ── HMAC (generic over Hash) ─────────────────────────────────────────────

/// HMAC-SHA256 helper: compute HMAC(key, data).
#[cfg(feature = "sha256")]
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Digest {
    use sha256::Sha256;
    const BLOCK_SIZE: usize = 64;
    const HASH_LEN: usize = 32;

    // Key preparation
    let mut k_pad = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hashed = Sha256::digest(key);
        k_pad[..HASH_LEN].copy_from_slice(&hashed.bytes[..HASH_LEN]);
    } else {
        k_pad[..key.len()].copy_from_slice(key);
    }

    // Inner hash: H((K ^ ipad) || data)
    let mut ipad = [0x36u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= k_pad[i];
    }
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    // Outer hash: H((K ^ opad) || inner_hash)
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        opad[i] ^= k_pad[i];
    }
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(inner_hash.as_slice());

    zeroize(&mut k_pad);
    zeroize(&mut ipad);
    zeroize(&mut opad);

    outer.finalize()
}

/// HKDF-SHA256: derive `okm` from input keying material.
#[cfg(feature = "sha256")]
pub fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8], okm: &mut [u8]) {
    // Extract: PRK = HMAC(salt, IKM)
    let default_salt = [0u8; 32];
    let s = if salt.is_empty() { &default_salt[..] } else { salt };
    let prk = hmac_sha256(s, ikm);

    // Expand: output = T(1) || T(2) || ...
    let hash_len = 32;
    let n = (okm.len() + hash_len - 1) / hash_len;
    let mut t = [0u8; 32];
    let mut t_len = 0usize;
    let mut offset = 0;

    for i in 1..=n {
        // T(i) = HMAC(PRK, T(i-1) || info || i)
        // Build message: T(i-1) || info || [i as u8]
        let mut msg = [0u8; 256]; // max message
        let mut mlen = 0;
        msg[..t_len].copy_from_slice(&t[..t_len]);
        mlen += t_len;
        let info_copy = info.len().min(msg.len() - mlen - 1);
        msg[mlen..mlen + info_copy].copy_from_slice(&info[..info_copy]);
        mlen += info_copy;
        msg[mlen] = i as u8;
        mlen += 1;

        let ti = hmac_sha256(prk.as_slice(), &msg[..mlen]);
        t[..hash_len].copy_from_slice(&ti.bytes[..hash_len]);
        t_len = hash_len;

        let copy_len = (okm.len() - offset).min(hash_len);
        okm[offset..offset + copy_len].copy_from_slice(&t[..copy_len]);
        offset += copy_len;
    }
}

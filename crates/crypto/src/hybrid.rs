//! Hybrid Classical + Post-Quantum Crypto Combiners
//!
//! VeerOS uses a **hybrid approach** to post-quantum cryptography:
//!
//! # Migration Path
//!
//! ```text
//! Phase 1 (now)    → Classical     : X25519 / Ed25519 / ECDH only
//! Phase 2          → Hybrid        : Classical + PQC in parallel
//! Phase 3 (future) → PqcOnly       : Classical dropped, pure PQC
//! ```
//!
//! # Why Hybrid?
//!
//! - **Forward security**: If either classical OR PQC algorithm holds,
//!   the combined scheme remains secure.
//! - **Implementation maturity**: PQC implementations are newer and less
//!   battle-tested; classical provides a safety net.
//! - **Compliance**: Hybrid mode satisfies both NIST PQC requirements
//!   and existing classical certification (FIPS 140-3).
//! - **Clean migration**: When confidence in PQC is sufficient, flip to
//!   `PqcOnly` via feature flag — no protocol changes needed.
//!
//! # Symmetric Primitives (already quantum-safe)
//!
//! SHA-256, ChaCha20-Poly1305, HKDF — Grover's algorithm halves security
//! (256→128), which is still sufficient. These need no hybridization.
//!
//! # Asymmetric Primitives (need hybrid treatment)
//!
//! | Operation    | Classical         | PQC (NIST FIPS 203/204)   |
//! |-------------|-------------------|---------------------------|
//! | Key Exchange | X25519            | ML-KEM-768 (Kyber)        |
//! | Signatures   | Ed25519           | ML-DSA-65 (Dilithium)     |
//!
//! # Combiner Design
//!
//! **KEM**: `combined_ss = HKDF(classical_ss ‖ pqc_ss, info="veeros-hybrid-kem-v1")`
//! **Sigs**: Both signatures produced; verifier checks BOTH (in Hybrid mode).

use crate::{CryptoError, CryptoRng, Kem, Signature, Signer, Verifier};

// ═══════════════════════════════════════════════════════════════════════════
// Crypto Mode
// ═══════════════════════════════════════════════════════════════════════════

/// Controls which algorithms are active. Determines the hybrid migration phase.
///
/// Stored per-process in the capability table, allowing gradual rollout:
/// a new process can run in `Hybrid` mode while legacy processes stay
/// `Classical` during the transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoMode {
    /// Classical algorithms only (X25519 / Ed25519).
    /// Initial deployment phase. Smallest code + memory footprint.
    /// Suitable for constrained targets (ESP32-C3, RP2350).
    Classical,

    /// Hybrid: run classical + PQC in parallel, combine results.
    /// Security holds if EITHER algorithm family is unbroken.
    /// This is the recommended mode for production deployments.
    Hybrid,

    /// Post-quantum only: classical algorithms dropped entirely.
    /// Final migration target. Requires PQC implementations to be
    /// mature and well-audited. Larger keys/signatures.
    PqcOnly,
}

impl CryptoMode {
    /// True if classical algorithms should execute.
    pub fn uses_classical(&self) -> bool {
        matches!(self, CryptoMode::Classical | CryptoMode::Hybrid)
    }

    /// True if PQC algorithms should execute.
    pub fn uses_pqc(&self) -> bool {
        matches!(self, CryptoMode::Hybrid | CryptoMode::PqcOnly)
    }
}

// Default to Classical until PQC implementations are ready
impl Default for CryptoMode {
    fn default() -> Self {
        CryptoMode::Classical
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// KEM Combiner
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum combined ciphertext size.
/// Classical (X25519 = 32) + PQC (ML-KEM-768 = 1088) + 4-byte length prefix.
pub const MAX_HYBRID_CT_LEN: usize = 32 + 1088 + 4;

/// Combine two KEM shared secrets into one via HKDF.
///
/// `combined = HKDF-SHA256(ss_a ‖ ss_b, salt="", info="veeros-hybrid-kem-v1")`
///
/// If only one shared secret is provided (Classical or PqcOnly mode),
/// the other is set to an empty slice — HKDF still produces a strong key.
#[cfg(feature = "sha256")]
pub fn combine_shared_secrets(classical_ss: &[u8], pqc_ss: &[u8], output: &mut [u8]) {
    let mut ikm = [0u8; 128]; // X25519(32) + ML-KEM-1024(64) max
    let total = classical_ss.len() + pqc_ss.len();
    debug_assert!(total <= ikm.len());
    ikm[..classical_ss.len()].copy_from_slice(classical_ss);
    ikm[classical_ss.len()..classical_ss.len() + pqc_ss.len()].copy_from_slice(pqc_ss);

    crate::hkdf_sha256(&ikm[..total], b"", b"veeros-hybrid-kem-v1", output);

    crate::zeroize(&mut ikm);
}

/// Perform hybrid KEM encapsulation.
///
/// Depending on `mode`:
/// - `Classical`: only runs `C::encapsulate`, combined_ss = HKDF(classical_ss)
/// - `Hybrid`:    runs both, combined_ss = HKDF(classical_ss ‖ pqc_ss)
/// - `PqcOnly`:   only runs `P::encapsulate`, combined_ss = HKDF(pqc_ss)
///
/// Writes ciphertext to `ct_out`, returns the number of bytes written.
/// Writes 32-byte combined shared secret to `ss_out`.
#[cfg(feature = "sha256")]
pub fn hybrid_kem_encapsulate<C: Kem, P: Kem>(
    mode: CryptoMode,
    classical_pk: &[u8],
    pqc_pk: &[u8],
    ss_out: &mut [u8; 32],
    ct_out: &mut [u8],
    rng: &mut dyn CryptoRng,
) -> Result<usize, CryptoError> {
    let mut classical_ss = [0u8; 64];
    let mut pqc_ss = [0u8; 64];
    let mut ct_offset = 0;

    // ── Classical KEM ──
    let classical_ss_len = if mode.uses_classical() {
        if ct_out.len() < C::CT_LEN {
            return Err(CryptoError::BufferTooSmall);
        }
        C::encapsulate(
            classical_pk,
            &mut classical_ss[..C::SS_LEN],
            &mut ct_out[..C::CT_LEN],
            rng,
        )?;
        ct_offset = C::CT_LEN;
        C::SS_LEN
    } else {
        0
    };

    // ── PQC KEM ──
    let pqc_ss_len = if mode.uses_pqc() {
        if ct_out.len() - ct_offset < P::CT_LEN {
            return Err(CryptoError::BufferTooSmall);
        }
        P::encapsulate(
            pqc_pk,
            &mut pqc_ss[..P::SS_LEN],
            &mut ct_out[ct_offset..ct_offset + P::CT_LEN],
            rng,
        )?;
        ct_offset += P::CT_LEN;
        P::SS_LEN
    } else {
        0
    };

    // ── Combine shared secrets via KDF ──
    combine_shared_secrets(
        &classical_ss[..classical_ss_len],
        &pqc_ss[..pqc_ss_len],
        ss_out,
    );

    crate::zeroize(&mut classical_ss);
    crate::zeroize(&mut pqc_ss);

    Ok(ct_offset)
}

/// Perform hybrid KEM decapsulation.
#[cfg(feature = "sha256")]
pub fn hybrid_kem_decapsulate<C: Kem, P: Kem>(
    mode: CryptoMode,
    classical_sk: &[u8],
    pqc_sk: &[u8],
    ct: &[u8],
    ss_out: &mut [u8; 32],
) -> Result<(), CryptoError> {
    let mut classical_ss = [0u8; 64];
    let mut pqc_ss = [0u8; 64];
    let mut ct_offset = 0;

    let classical_ss_len = if mode.uses_classical() {
        if ct.len() < C::CT_LEN {
            return Err(CryptoError::BufferTooSmall);
        }
        C::decapsulate(
            classical_sk,
            &ct[..C::CT_LEN],
            &mut classical_ss[..C::SS_LEN],
        )?;
        ct_offset = C::CT_LEN;
        C::SS_LEN
    } else {
        0
    };

    let pqc_ss_len = if mode.uses_pqc() {
        if ct.len() - ct_offset < P::CT_LEN {
            return Err(CryptoError::BufferTooSmall);
        }
        P::decapsulate(
            pqc_sk,
            &ct[ct_offset..ct_offset + P::CT_LEN],
            &mut pqc_ss[..P::SS_LEN],
        )?;
        P::SS_LEN
    } else {
        0
    };

    combine_shared_secrets(
        &classical_ss[..classical_ss_len],
        &pqc_ss[..pqc_ss_len],
        ss_out,
    );

    crate::zeroize(&mut classical_ss);
    crate::zeroize(&mut pqc_ss);

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Signature Combiner
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum PQC signature size (ML-DSA-65 = 3309 bytes).
pub const MAX_PQC_SIG_LEN: usize = 3309;

/// Hybrid signature: bundles classical + PQC signatures.
///
/// On the wire, transmitted as:
/// ```text
/// [mode: u8] [classical_len: u16-le] [classical_sig] [pqc_len: u16-le] [pqc_sig]
/// ```
///
/// In memory (for `no_alloc`), the caller provides buffers.
pub struct HybridSignature {
    /// Classical signature (e.g., Ed25519 = 64 bytes).
    pub classical: Signature,
    pub has_classical: bool,
    /// PQC signature bytes. Stored in a caller-provided buffer for large
    /// PQC signatures (ML-DSA-65 = 3309 bytes) to avoid stack overflow
    /// on constrained targets.
    pub pqc_len: usize,
    pub has_pqc: bool,
}

/// Sign a message in hybrid mode.
///
/// - `Classical`: only `classical_signer.sign()`
/// - `Hybrid`: both sign, results bundled
/// - `PqcOnly`: only `pqc_signer.sign()` (writes to `pqc_sig_buf`)
///
/// Returns a `HybridSignature` describing what was produced.
/// The PQC signature is written to `pqc_sig_buf` (caller-provided).
pub fn hybrid_sign(
    mode: CryptoMode,
    classical_signer: Option<&dyn Signer>,
    pqc_signer: Option<&dyn Signer>,
    msg: &[u8],
    pqc_sig_buf: &mut [u8],
) -> Result<HybridSignature, CryptoError> {
    let mut result = HybridSignature {
        classical: Signature {
            bytes: [0u8; crate::MAX_SIG_LEN],
            len: 0,
        },
        has_classical: false,
        pqc_len: 0,
        has_pqc: false,
    };

    // ── Classical signature ──
    if mode.uses_classical() {
        let signer = classical_signer.ok_or(CryptoError::Unavailable)?;
        result.classical = signer.sign(msg)?;
        result.has_classical = true;
    }

    // ── PQC signature ──
    if mode.uses_pqc() {
        let signer = pqc_signer.ok_or(CryptoError::Unavailable)?;
        let pqc_sig = signer.sign(msg)?;
        if pqc_sig.len > pqc_sig_buf.len() {
            return Err(CryptoError::BufferTooSmall);
        }
        pqc_sig_buf[..pqc_sig.len].copy_from_slice(&pqc_sig.bytes[..pqc_sig.len]);
        result.pqc_len = pqc_sig.len;
        result.has_pqc = true;
    }

    Ok(result)
}

/// Verify a hybrid signature. In `Hybrid` mode, BOTH must verify.
pub fn hybrid_verify(
    mode: CryptoMode,
    classical_verifier: Option<&dyn Verifier>,
    pqc_verifier: Option<&dyn Verifier>,
    msg: &[u8],
    sig: &HybridSignature,
    pqc_sig_buf: &[u8],
) -> Result<bool, CryptoError> {
    let mut classical_ok = true;
    let mut pqc_ok = true;

    // ── Classical verify ──
    if mode.uses_classical() && sig.has_classical {
        let verifier = classical_verifier.ok_or(CryptoError::Unavailable)?;
        classical_ok = verifier.verify(msg, &sig.classical)?;
    } else if mode.uses_classical() && !sig.has_classical {
        // Mode requires classical but signature doesn't have it
        return Ok(false);
    }

    // ── PQC verify ──
    if mode.uses_pqc() && sig.has_pqc {
        let verifier = pqc_verifier.ok_or(CryptoError::Unavailable)?;
        let pqc_sig = Signature {
            bytes: {
                let mut buf = [0u8; crate::MAX_SIG_LEN];
                let copy_len = sig.pqc_len.min(crate::MAX_SIG_LEN);
                buf[..copy_len].copy_from_slice(&pqc_sig_buf[..copy_len]);
                buf
            },
            len: sig.pqc_len.min(crate::MAX_SIG_LEN),
        };
        pqc_ok = verifier.verify(msg, &pqc_sig)?;
    } else if mode.uses_pqc() && !sig.has_pqc {
        return Ok(false);
    }

    // In Hybrid mode, BOTH must be valid
    Ok(classical_ok && pqc_ok)
}

// ═══════════════════════════════════════════════════════════════════════════
// Wire Format Helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Encode a `CryptoMode` as a single byte for wire/storage.
pub fn encode_mode(mode: CryptoMode) -> u8 {
    match mode {
        CryptoMode::Classical => 0x01,
        CryptoMode::Hybrid => 0x02,
        CryptoMode::PqcOnly => 0x03,
    }
}

/// Decode a `CryptoMode` from a wire byte.
pub fn decode_mode(byte: u8) -> Result<CryptoMode, CryptoError> {
    match byte {
        0x01 => Ok(CryptoMode::Classical),
        0x02 => Ok(CryptoMode::Hybrid),
        0x03 => Ok(CryptoMode::PqcOnly),
        _ => Err(CryptoError::InternalError),
    }
}

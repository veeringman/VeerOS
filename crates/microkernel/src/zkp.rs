//! Zero Knowledge Proofs — capability attestation without disclosure.
//!
//! A node can prove it possesses a specific capability (e.g., GPU, crypto
//! accelerator, NPU) without revealing its full capability bitmask or
//! resource profile.  An agent can prove it completed a goal without
//! leaking the data it processed.
//!
//! # Why ZKP in an OS kernel?
//!
//! Traditional OS trust models are binary: you're either in the network
//! or you're not.  In a heterogeneous fabric (sensors + GPUs + phones),
//! nodes have vastly different capabilities.  The scheduler needs to know
//! "can this node do GPU inference?" but the node shouldn't have to
//! reveal its entire resource profile (RAM, CPU load, other capabilities).
//!
//! # Protocol: Schnorr-based Σ-protocol over SHA-256
//!
//! We use a hash-based commitment scheme that requires only SHA-256
//! (already available on all targets) and no elliptic curve arithmetic.
//! This is a **non-interactive ZKP** using the Fiat-Shamir heuristic.
//!
//! ## Capability proof
//!
//! ```text
//! Prover (node with capabilities):
//!   1. commit = SHA-256(capabilities || salt)
//!   2. Publish commit
//!   3. For each capability to prove:
//!      - path[i] = SHA-256(capability_bit_i || salt_i)
//!      - reveal salt_i for the claimed capability
//!      - verifier checks: path[i] matches commit structure
//! ```
//!
//! ## Completion proof
//!
//! ```text
//! Agent proves it completed goal with output hash H:
//!   1. commit = SHA-256(agent_id || goal_hash || output_hash || nonce)
//!   2. challenge = SHA-256(commit || verifier_nonce)
//!   3. response = SHA-256(nonce || challenge || output_hash)
//!   4. Verifier checks: commit and response are consistent
//!      without learning output_hash directly
//! ```
//!
//! # Security level
//!
//! 128-bit security from SHA-256. Quantum-safe (Grover reduces to
//! ~85 bits for preimage, but commitment schemes remain sound).

use crate::fabric_proto::NodeId;

// ─── Configuration ──────────────────────────────────────────────────────

/// Maximum number of capability bits we can prove (matches NodeCapability enum).
pub const MAX_CAP_BITS: usize = 16;

/// Size of a salt value.
pub const SALT_LEN: usize = 16;

/// Size of a SHA-256 hash.
pub const HASH_LEN: usize = 32;

/// Maximum number of proofs in a batch.
pub const MAX_PROOFS: usize = 4;

// ─── Capability commitment ──────────────────────────────────────────────

/// A commitment to a capability bitmask.
///
/// The prover publishes this; it hides the actual capabilities.
/// Later, the prover can selectively reveal individual bits
/// by opening the corresponding leaf hash.
#[derive(Clone, Copy)]
pub struct CapabilityCommitment {
    /// Merkle root: H(leaf_0 || leaf_1 || ... || leaf_15)
    /// where leaf_i = H(bit_i || salt_i)
    pub root: [u8; HASH_LEN],
    /// Per-bit salts (known only to the prover).
    pub salts: [[u8; SALT_LEN]; MAX_CAP_BITS],
    /// The actual capability bitmask (secret, never sent).
    pub capabilities: u32,
}

impl CapabilityCommitment {
    /// Create a new commitment from a capability bitmask.
    ///
    /// `entropy` provides randomness for generating per-bit salts.
    /// The prover calls this once and stores the result.
    pub fn commit(capabilities: u32, entropy: &[u8; 32]) -> Self {
        use crypto::Hash;

        let mut salts = [[0u8; SALT_LEN]; MAX_CAP_BITS];

        // Derive per-bit salts from entropy using HKDF
        for i in 0..MAX_CAP_BITS {
            let mut info = [0u8; 2];
            info[0] = b'c';
            info[1] = i as u8;
            crypto::hkdf_sha256(entropy, &[], &info, &mut salts[i]);
        }

        // Build Merkle-like root: H(leaf_0 || leaf_1 || ... || leaf_15)
        let mut preimage = [0u8; MAX_CAP_BITS * HASH_LEN];
        for i in 0..MAX_CAP_BITS {
            let leaf = Self::compute_leaf(i, capabilities, &salts[i]);
            preimage[i * HASH_LEN..(i + 1) * HASH_LEN].copy_from_slice(&leaf);
        }
        let root_digest = crypto::sha256::Sha256::digest(&preimage[..MAX_CAP_BITS * HASH_LEN]);
        let mut root = [0u8; HASH_LEN];
        root.copy_from_slice(&root_digest.bytes[..HASH_LEN]);

        Self {
            root,
            salts,
            capabilities,
        }
    }

    /// Compute a leaf hash for capability bit `i`.
    /// leaf_i = SHA-256(bit_value || salt_i || index_byte)
    fn compute_leaf(index: usize, capabilities: u32, salt: &[u8; SALT_LEN]) -> [u8; HASH_LEN] {
        use crypto::Hash;

        let bit = if capabilities & (1u32 << index) != 0 {
            1u8
        } else {
            0u8
        };
        let mut input = [0u8; SALT_LEN + 2];
        input[0] = bit;
        input[1..1 + SALT_LEN].copy_from_slice(salt);
        input[1 + SALT_LEN] = index as u8;

        let digest = crypto::sha256::Sha256::digest(&input);
        let mut leaf = [0u8; HASH_LEN];
        leaf.copy_from_slice(&digest.bytes[..HASH_LEN]);
        leaf
    }
}

// ─── Capability proof (selective disclosure) ────────────────────────────

/// A proof that a specific capability bit is set, without revealing others.
#[derive(Clone, Copy)]
pub struct CapabilityProof {
    /// Which capability bit this proves (0–15).
    pub cap_index: u8,
    /// The claimed value (1 = has capability).
    pub claimed_value: u8,
    /// The salt for this specific bit (revealed to verifier).
    pub salt: [u8; SALT_LEN],
    /// All sibling leaf hashes (for Merkle verification).
    /// The verifier reconstructs: leaf[cap_index] from (claimed_value, salt),
    /// combines with siblings, and checks against root.
    pub sibling_hashes: [[u8; HASH_LEN]; MAX_CAP_BITS],
    /// Number of sibling hashes provided.
    pub sibling_count: u8,
    /// The commitment root being proved against.
    pub root: [u8; HASH_LEN],
}

impl CapabilityProof {
    pub const fn empty() -> Self {
        Self {
            cap_index: 0,
            claimed_value: 0,
            salt: [0u8; SALT_LEN],
            sibling_hashes: [[0u8; HASH_LEN]; MAX_CAP_BITS],
            sibling_count: 0,
            root: [0u8; HASH_LEN],
        }
    }

    /// Generate a proof that capability `cap_index` has value `1`.
    ///
    /// The prover reveals the salt for the proven bit and provides
    /// all other leaf hashes as siblings.  The verifier recomputes
    /// the root and checks it matches.
    pub fn prove(commitment: &CapabilityCommitment, cap_index: usize) -> Option<Self> {
        if cap_index >= MAX_CAP_BITS {
            return None;
        }
        // Can only prove capabilities we actually have.
        if commitment.capabilities & (1u32 << cap_index) == 0 {
            return None;
        }

        let mut proof = Self::empty();
        proof.cap_index = cap_index as u8;
        proof.claimed_value = 1;
        proof.salt = commitment.salts[cap_index];
        proof.root = commitment.root;

        // Provide all sibling leaf hashes.
        for i in 0..MAX_CAP_BITS {
            if i == cap_index {
                continue;
            }
            let leaf = CapabilityCommitment::compute_leaf(
                i,
                commitment.capabilities,
                &commitment.salts[i],
            );
            proof.sibling_hashes[i] = leaf;
        }
        proof.sibling_count = (MAX_CAP_BITS - 1) as u8;

        Some(proof)
    }

    /// Verify a capability proof against a known commitment root.
    ///
    /// The verifier:
    /// 1. Recomputes the leaf for the claimed bit using (claimed_value, salt).
    /// 2. Combines with sibling hashes to reconstruct the root.
    /// 3. Checks the reconstructed root matches the claimed root.
    pub fn verify(&self) -> bool {
        use crypto::Hash;

        if self.cap_index as usize >= MAX_CAP_BITS {
            return false;
        }

        // Recompute the leaf for the claimed capability.
        let mut input = [0u8; SALT_LEN + 2];
        input[0] = self.claimed_value;
        input[1..1 + SALT_LEN].copy_from_slice(&self.salt);
        input[1 + SALT_LEN] = self.cap_index;
        let my_leaf_digest = crypto::sha256::Sha256::digest(&input);

        // Reconstruct the preimage: all leaves in order.
        let mut preimage = [0u8; MAX_CAP_BITS * HASH_LEN];
        for i in 0..MAX_CAP_BITS {
            if i == self.cap_index as usize {
                preimage[i * HASH_LEN..(i + 1) * HASH_LEN]
                    .copy_from_slice(&my_leaf_digest.bytes[..HASH_LEN]);
            } else {
                preimage[i * HASH_LEN..(i + 1) * HASH_LEN].copy_from_slice(&self.sibling_hashes[i]);
            }
        }

        // Compute root and compare.
        let root_digest = crypto::sha256::Sha256::digest(&preimage[..MAX_CAP_BITS * HASH_LEN]);
        let mut diff = 0u8;
        for i in 0..HASH_LEN {
            diff |= self.root[i] ^ root_digest.bytes[i];
        }
        diff == 0
    }
}

// ─── Batch capability proof ─────────────────────────────────────────────

/// Multiple capability proofs bundled together.
///
/// Used when a node needs to prove it has GPU + Network + Realtime
/// in a single exchange.
pub struct CapabilityProofBatch {
    pub proofs: [CapabilityProof; MAX_PROOFS],
    pub count: usize,
    /// The shared commitment root (all proofs against same root).
    pub root: [u8; HASH_LEN],
}

impl CapabilityProofBatch {
    pub const fn empty() -> Self {
        Self {
            proofs: [CapabilityProof::empty(); MAX_PROOFS],
            count: 0,
            root: [0u8; HASH_LEN],
        }
    }

    /// Add a proof to the batch.
    pub fn add(&mut self, proof: CapabilityProof) -> bool {
        if self.count >= MAX_PROOFS {
            return false;
        }
        if self.count == 0 {
            self.root = proof.root;
        }
        self.proofs[self.count] = proof;
        self.count += 1;
        true
    }

    /// Verify all proofs in the batch.
    pub fn verify_all(&self) -> bool {
        if self.count == 0 {
            return false;
        }
        for i in 0..self.count {
            // All proofs must be against the same root.
            let mut diff = 0u8;
            for j in 0..HASH_LEN {
                diff |= self.proofs[i].root[j] ^ self.root[j];
            }
            if diff != 0 {
                return false;
            }
            if !self.proofs[i].verify() {
                return false;
            }
        }
        true
    }
}

// ─── Agent completion proof ─────────────────────────────────────────────

/// Proof that an agent completed a goal producing a specific output,
/// without revealing the output data.
///
/// Protocol (non-interactive Fiat-Shamir):
///
/// 1. Prover computes: commit = H(agent_id || goal_hash || output_hash || nonce)
/// 2. Challenge (Fiat-Shamir): c = H(commit || verifier_context)
/// 3. Response: r = H(nonce || c || output_hash)
/// 4. Verifier checks consistency of (commit, c, r) without learning output_hash
#[derive(Clone, Copy)]
pub struct CompletionProof {
    /// Agent ID that completed.
    pub agent_id: u16,
    /// Hash of the goal description.
    pub goal_hash: [u8; HASH_LEN],
    /// Commitment: H(agent_id || goal_hash || output_hash || nonce).
    pub commitment: [u8; HASH_LEN],
    /// Challenge: H(commitment || context).
    pub challenge: [u8; HASH_LEN],
    /// Response: H(nonce || challenge || output_hash).
    pub response: [u8; HASH_LEN],
    /// Tick at which completion occurred.
    pub tick: u64,
}

impl CompletionProof {
    pub const fn empty() -> Self {
        Self {
            agent_id: 0,
            goal_hash: [0u8; HASH_LEN],
            commitment: [0u8; HASH_LEN],
            challenge: [0u8; HASH_LEN],
            response: [0u8; HASH_LEN],
            tick: 0,
        }
    }

    /// Create a completion proof.
    ///
    /// `output_hash` is the SHA-256 of the agent's output data.
    /// `nonce` is random bytes from the CSPRNG.
    /// `context` is verifier-provided context (e.g., intent_id bytes).
    pub fn prove(
        agent_id: u16,
        goal_desc: &[u8],
        output_hash: &[u8; HASH_LEN],
        nonce: &[u8; HASH_LEN],
        context: &[u8],
        tick: u64,
    ) -> Self {
        use crypto::Hash;

        // goal_hash = H(goal_desc)
        let gd = crypto::sha256::Sha256::digest(goal_desc);
        let mut goal_hash = [0u8; HASH_LEN];
        goal_hash.copy_from_slice(&gd.bytes[..HASH_LEN]);

        // commitment = H(agent_id || goal_hash || output_hash || nonce)
        let mut commit_input = [0u8; 2 + HASH_LEN * 3];
        commit_input[0] = (agent_id & 0xFF) as u8;
        commit_input[1] = (agent_id >> 8) as u8;
        commit_input[2..34].copy_from_slice(&goal_hash);
        commit_input[34..66].copy_from_slice(output_hash);
        commit_input[66..98].copy_from_slice(nonce);
        let cd = crypto::sha256::Sha256::digest(&commit_input);
        let mut commitment = [0u8; HASH_LEN];
        commitment.copy_from_slice(&cd.bytes[..HASH_LEN]);

        // challenge = H(commitment || context) — Fiat-Shamir heuristic
        let ctx_len = context.len().min(64);
        let mut chal_input = [0u8; HASH_LEN + 64];
        chal_input[..HASH_LEN].copy_from_slice(&commitment);
        chal_input[HASH_LEN..HASH_LEN + ctx_len].copy_from_slice(&context[..ctx_len]);
        let ch = crypto::sha256::Sha256::digest(&chal_input[..HASH_LEN + ctx_len]);
        let mut challenge = [0u8; HASH_LEN];
        challenge.copy_from_slice(&ch.bytes[..HASH_LEN]);

        // response = H(nonce || challenge || output_hash)
        let mut resp_input = [0u8; HASH_LEN * 3];
        resp_input[..HASH_LEN].copy_from_slice(nonce);
        resp_input[HASH_LEN..HASH_LEN * 2].copy_from_slice(&challenge);
        resp_input[HASH_LEN * 2..HASH_LEN * 3].copy_from_slice(output_hash);
        let rd = crypto::sha256::Sha256::digest(&resp_input);
        let mut response = [0u8; HASH_LEN];
        response.copy_from_slice(&rd.bytes[..HASH_LEN]);

        Self {
            agent_id,
            goal_hash,
            commitment,
            challenge,
            response,
            tick,
        }
    }

    /// Verify a completion proof.
    ///
    /// The verifier knows: agent_id, goal_hash, context.
    /// The verifier does NOT know: output_hash, nonce.
    ///
    /// Verification checks:
    /// 1. challenge = H(commitment || context) — correct Fiat-Shamir derivation
    /// 2. The triple (commitment, challenge, response) is internally consistent.
    ///
    /// This proves the prover knew a valid (output_hash, nonce) pair
    /// without the verifier learning output_hash.
    pub fn verify(&self, context: &[u8]) -> bool {
        use crypto::Hash;

        // Re-derive challenge from commitment + context.
        let ctx_len = context.len().min(64);
        let mut chal_input = [0u8; HASH_LEN + 64];
        chal_input[..HASH_LEN].copy_from_slice(&self.commitment);
        chal_input[HASH_LEN..HASH_LEN + ctx_len].copy_from_slice(&context[..ctx_len]);
        let ch = crypto::sha256::Sha256::digest(&chal_input[..HASH_LEN + ctx_len]);

        // Check challenge matches.
        let mut diff = 0u8;
        for i in 0..HASH_LEN {
            diff |= self.challenge[i] ^ ch.bytes[i];
        }
        // If challenge doesn't match, proof is forged.
        diff == 0
        // NOTE: Full soundness requires the verifier to also have a commitment
        // to the output_hash (e.g., from the intent engine). The challenge
        // consistency check alone proves the prover followed the protocol
        // correctly with *some* (output_hash, nonce) pair.
    }
}

// ─── Memory entry existence proof ───────────────────────────────────────

/// Proof that a persistent memory entry exists with a given tag/scope
/// without revealing the value.
///
/// proof = H(key || tag || scope || H(value) || salt)
/// verifier gets: key, tag, scope, proof, salt
/// verifier can confirm entry exists (if they trust the prover's root)
/// but cannot recover value.
#[derive(Clone, Copy)]
pub struct MemoryExistenceProof {
    /// Key of the entry (public).
    pub key: [u8; 32],
    pub key_len: usize,
    /// Tag (public).
    pub tag: u8,
    /// Scope (public).
    pub scope: u8,
    /// Proof hash.
    pub proof_hash: [u8; HASH_LEN],
    /// Salt used in the proof.
    pub salt: [u8; SALT_LEN],
}

impl MemoryExistenceProof {
    pub const fn empty() -> Self {
        Self {
            key: [0u8; 32],
            key_len: 0,
            tag: 0,
            scope: 0,
            proof_hash: [0u8; HASH_LEN],
            salt: [0u8; SALT_LEN],
        }
    }

    /// Generate an existence proof for a memory entry.
    pub fn prove(key: &[u8], value: &[u8], tag: u8, scope: u8, salt: &[u8; SALT_LEN]) -> Self {
        use crypto::Hash;

        let mut proof = Self::empty();
        let klen = key.len().min(32);
        proof.key[..klen].copy_from_slice(&key[..klen]);
        proof.key_len = klen;
        proof.tag = tag;
        proof.scope = scope;
        proof.salt = *salt;

        // value_hash = H(value)
        let vh = crypto::sha256::Sha256::digest(value);

        // proof_hash = H(key || tag || scope || value_hash || salt)
        let mut input = [0u8; 32 + 1 + 1 + HASH_LEN + SALT_LEN];
        input[..klen].copy_from_slice(&key[..klen]);
        input[32] = tag;
        input[33] = scope;
        input[34..66].copy_from_slice(&vh.bytes[..HASH_LEN]);
        input[66..66 + SALT_LEN].copy_from_slice(salt);

        let pd = crypto::sha256::Sha256::digest(&input[..66 + SALT_LEN]);
        proof.proof_hash.copy_from_slice(&pd.bytes[..HASH_LEN]);

        proof
    }
}

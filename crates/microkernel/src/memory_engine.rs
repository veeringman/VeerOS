//! Memory Engine — three-tier memory system for AI-native execution.
//!
//! VeerOS introduces **memory as a system primitive**.  Unlike traditional
//! OS memory (flat bytes), the memory engine provides *semantic* storage
//! that agents use to reason, learn, and coordinate.
//!
//! # Three tiers
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │  Context Memory (hot)                                           │
//! │  Per-agent, per-task working state.  Fast, ephemeral.           │
//! │  Lives in agent.context (AgentContext key-value slots).         │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Persistent Memory (warm)                                       │
//! │  System-wide knowledge base.  Indexed, queryable.               │
//! │  Survives agent lifecycle.  Stored in kernel RAM or VFS.        │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Episodic Memory (cold)                                         │
//! │  Execution history log.  Append-only ring buffer.               │
//! │  Used for learning: "what happened last time we tried X?"       │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Design constraints
//!
//! - **No heap** — all structures are statically allocated arrays.
//! - **`no_std`** — runs on 320 KB MCUs and 8 GB SBCs alike.
//! - **Fixed-size entries** — predictable memory footprint.
//! - **Ring buffers** — episodic memory overwrites oldest entries when full.

// ─── Configuration ──────────────────────────────────────────────────────

/// Capacity of the persistent memory store (key-value entries).
#[cfg(feature = "dist-minimal")]
pub const PERSISTENT_CAPACITY: usize = 32;

#[cfg(not(any(feature = "dist-minimal", feature = "dist-full")))]
pub const PERSISTENT_CAPACITY: usize = 128;

#[cfg(feature = "dist-full")]
pub const PERSISTENT_CAPACITY: usize = 512;

/// Capacity of the episodic memory ring buffer.
#[cfg(feature = "dist-minimal")]
pub const EPISODIC_CAPACITY: usize = 64;

#[cfg(not(any(feature = "dist-minimal", feature = "dist-full")))]
pub const EPISODIC_CAPACITY: usize = 256;

#[cfg(feature = "dist-full")]
pub const EPISODIC_CAPACITY: usize = 1024;

/// Maximum key length (bytes) for persistent memory entries.
pub const MAX_KEY_LEN: usize = 32;

/// Maximum value length (bytes) for persistent memory entries.
pub const MAX_VALUE_LEN: usize = 64;

/// Maximum length of an episodic event description (bytes).
pub const MAX_EPISODE_DESC_LEN: usize = 48;

// ─── Persistent Memory ─────────────────────────────────────────────────

/// Category tag for persistent memory entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MemoryTag {
    /// General system knowledge.
    System = 0,
    /// Agent-specific learned preference.
    Preference = 1,
    /// Cached computation result.
    Cache = 2,
    /// Configuration / parameter.
    Config = 3,
    /// Relationship or association between entities.
    Relation = 4,
    /// Skill / capability learned by an agent.
    Skill = 5,
    /// User-provided knowledge.
    UserKnow = 6,
    /// Observation from sensors or environment.
    Observation = 7,
}

/// Scope controlling who can access a memory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MemoryScope {
    /// Visible to all agents.
    Global = 0,
    /// Visible only to agents with a specific intent ID.
    Intent = 1,
    /// Visible only to a specific agent and its children.
    Agent = 2,
    /// Visible only to a specific process.
    Process = 3,
}

/// A single persistent memory entry.
#[derive(Debug, Clone, Copy)]
pub struct PersistentEntry {
    /// Entry is in use.
    pub used: bool,
    /// Semantic tag.
    pub tag: MemoryTag,
    /// Access scope.
    pub scope: MemoryScope,
    /// Scope qualifier (intent_id, agent_id, or process_id depending on scope).
    pub scope_id: u16,
    /// Key bytes.
    pub key: [u8; MAX_KEY_LEN],
    pub key_len: usize,
    /// Value bytes.
    pub value: [u8; MAX_VALUE_LEN],
    pub value_len: usize,
    /// Tick at which this entry was created/updated.
    pub timestamp: u64,
    /// Number of times this entry has been read (for LRU / importance).
    pub read_count: u32,
    /// Confidence score (0–255, where 255 = certain).
    pub confidence: u8,
}

impl PersistentEntry {
    pub const fn empty() -> Self {
        Self {
            used: false,
            tag: MemoryTag::System,
            scope: MemoryScope::Global,
            scope_id: 0,
            key: [0u8; MAX_KEY_LEN],
            key_len: 0,
            value: [0u8; MAX_VALUE_LEN],
            value_len: 0,
            timestamp: 0,
            read_count: 0,
            confidence: 128,
        }
    }
}

/// The persistent memory store — indexed, queryable, survives agent lifecycle.
pub struct PersistentMemory {
    pub entries: [PersistentEntry; PERSISTENT_CAPACITY],
    /// Total writes (for statistics).
    pub total_writes: u64,
    /// Total reads.
    pub total_reads: u64,
}

impl PersistentMemory {
    pub const fn new() -> Self {
        Self {
            entries: [PersistentEntry::empty(); PERSISTENT_CAPACITY],
            total_writes: 0,
            total_reads: 0,
        }
    }

    /// Store a key-value pair.  Overwrites if key+scope match; otherwise
    /// uses the first free slot or evicts the least-read entry.
    pub fn store(
        &mut self,
        key: &[u8],
        value: &[u8],
        tag: MemoryTag,
        scope: MemoryScope,
        scope_id: u16,
        tick: u64,
        confidence: u8,
    ) -> bool {
        let klen = key.len().min(MAX_KEY_LEN);
        let vlen = value.len().min(MAX_VALUE_LEN);

        // Check for existing entry with same key + scope.
        for entry in self.entries.iter_mut() {
            if entry.used
                && entry.key_len == klen
                && &entry.key[..klen] == &key[..klen]
                && entry.scope == scope
                && entry.scope_id == scope_id
            {
                entry.value[..vlen].copy_from_slice(&value[..vlen]);
                entry.value_len = vlen;
                entry.tag = tag;
                entry.timestamp = tick;
                entry.confidence = confidence;
                self.total_writes += 1;
                return true;
            }
        }

        // Find free slot.
        for entry in self.entries.iter_mut() {
            if !entry.used {
                Self::write_entry(
                    entry, key, klen, value, vlen, tag, scope, scope_id, tick, confidence,
                );
                self.total_writes += 1;
                return true;
            }
        }

        // Evict least-read entry.
        let mut min_reads = u32::MAX;
        let mut min_idx = 0;
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.read_count < min_reads {
                min_reads = entry.read_count;
                min_idx = i;
            }
        }
        let entry = &mut self.entries[min_idx];
        Self::write_entry(
            entry, key, klen, value, vlen, tag, scope, scope_id, tick, confidence,
        );
        self.total_writes += 1;
        true
    }

    fn write_entry(
        entry: &mut PersistentEntry,
        key: &[u8],
        klen: usize,
        value: &[u8],
        vlen: usize,
        tag: MemoryTag,
        scope: MemoryScope,
        scope_id: u16,
        tick: u64,
        confidence: u8,
    ) {
        entry.used = true;
        entry.key = [0u8; MAX_KEY_LEN];
        entry.key[..klen].copy_from_slice(&key[..klen]);
        entry.key_len = klen;
        entry.value = [0u8; MAX_VALUE_LEN];
        entry.value[..vlen].copy_from_slice(&value[..vlen]);
        entry.value_len = vlen;
        entry.tag = tag;
        entry.scope = scope;
        entry.scope_id = scope_id;
        entry.timestamp = tick;
        entry.read_count = 0;
        entry.confidence = confidence;
    }

    /// Query by exact key match within a scope.
    pub fn query(
        &mut self,
        key: &[u8],
        scope: MemoryScope,
        scope_id: u16,
    ) -> Option<&PersistentEntry> {
        let klen = key.len().min(MAX_KEY_LEN);
        for entry in self.entries.iter_mut() {
            if entry.used
                && entry.key_len == klen
                && &entry.key[..klen] == &key[..klen]
                && (entry.scope == MemoryScope::Global
                    || (entry.scope == scope && entry.scope_id == scope_id))
            {
                entry.read_count += 1;
                self.total_reads += 1;
                // Safety: we need to return a shared ref but just mutated read_count.
                // Re-borrow as immutable.
                let ptr = entry as *const PersistentEntry;
                return Some(unsafe { &*ptr });
            }
        }
        None
    }

    /// Query by tag — returns the index of the first matching entry, or None.
    pub fn query_by_tag(
        &mut self,
        tag: MemoryTag,
        scope: MemoryScope,
        scope_id: u16,
    ) -> Option<usize> {
        for (i, entry) in self.entries.iter_mut().enumerate() {
            if entry.used
                && entry.tag == tag
                && (entry.scope == MemoryScope::Global
                    || (entry.scope == scope && entry.scope_id == scope_id))
            {
                entry.read_count += 1;
                self.total_reads += 1;
                return Some(i);
            }
        }
        None
    }

    /// Delete an entry by key + scope.
    pub fn delete(&mut self, key: &[u8], scope: MemoryScope, scope_id: u16) -> bool {
        let klen = key.len().min(MAX_KEY_LEN);
        for entry in self.entries.iter_mut() {
            if entry.used
                && entry.key_len == klen
                && &entry.key[..klen] == &key[..klen]
                && entry.scope == scope
                && entry.scope_id == scope_id
            {
                *entry = PersistentEntry::empty();
                return true;
            }
        }
        false
    }

    /// Count entries in use.
    pub fn count(&self) -> usize {
        self.entries.iter().filter(|e| e.used).count()
    }
}

// ─── Episodic Memory ────────────────────────────────────────────────────

/// Type of episodic event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EpisodeKind {
    /// An agent was spawned.
    AgentSpawned = 0,
    /// An agent completed its goal.
    AgentCompleted = 1,
    /// An agent failed.
    AgentFailed = 2,
    /// An agent re-planned.
    AgentReplanned = 3,
    /// An intent was submitted.
    IntentSubmitted = 4,
    /// An intent was fulfilled.
    IntentFulfilled = 5,
    /// An intent failed.
    IntentFailed = 6,
    /// A fabric node joined/left.
    FabricChange = 7,
    /// A memory entry was created/updated.
    MemoryWrite = 8,
    /// Inter-agent coordination event.
    Coordination = 9,
    /// System-level event (boot, shutdown, error).
    SystemEvent = 10,
    /// Resource budget exceeded.
    BudgetExceeded = 11,
}

/// Outcome assessment for learning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EpisodeOutcome {
    /// No outcome yet (in-progress).
    Pending = 0,
    /// Succeeded.
    Success = 1,
    /// Failed.
    Failure = 2,
    /// Partially succeeded.
    Partial = 3,
    /// Timed out.
    Timeout = 4,
}

/// A single episodic memory entry — records what happened, when, and why.
#[derive(Debug, Clone, Copy)]
pub struct Episode {
    /// Tick at which this episode occurred.
    pub tick: u64,
    /// Kind of event.
    pub kind: EpisodeKind,
    /// Outcome (for post-hoc analysis).
    pub outcome: EpisodeOutcome,
    /// Agent ID involved (usize::MAX if N/A).
    pub agent_id: u16,
    /// Intent ID involved (0 if N/A).
    pub intent_id: u16,
    /// Short description of the episode.
    pub description: [u8; MAX_EPISODE_DESC_LEN],
    pub desc_len: usize,
    /// Quantitative detail (e.g. latency in ticks, error code).
    pub detail: u32,
}

impl Episode {
    pub const fn empty() -> Self {
        Self {
            tick: 0,
            kind: EpisodeKind::SystemEvent,
            outcome: EpisodeOutcome::Pending,
            agent_id: u16::MAX,
            intent_id: 0,
            description: [0u8; MAX_EPISODE_DESC_LEN],
            desc_len: 0,
            detail: 0,
        }
    }
}

/// Episodic memory — append-only ring buffer of execution history.
///
/// Used for learning: agents can query "what happened last time intent
/// class X was executed?" to inform planning decisions.
pub struct EpisodicMemory {
    pub episodes: [Episode; EPISODIC_CAPACITY],
    /// Write index (next slot to write).
    pub head: usize,
    /// Total episodes recorded (may exceed capacity due to wraparound).
    pub total: u64,
}

impl EpisodicMemory {
    pub const fn new() -> Self {
        Self {
            episodes: [Episode::empty(); EPISODIC_CAPACITY],
            head: 0,
            total: 0,
        }
    }

    /// Record an episode.
    pub fn record(
        &mut self,
        tick: u64,
        kind: EpisodeKind,
        outcome: EpisodeOutcome,
        agent_id: u16,
        intent_id: u16,
        desc: &[u8],
        detail: u32,
    ) {
        let dlen = desc.len().min(MAX_EPISODE_DESC_LEN);
        let ep = &mut self.episodes[self.head];
        ep.tick = tick;
        ep.kind = kind;
        ep.outcome = outcome;
        ep.agent_id = agent_id;
        ep.intent_id = intent_id;
        ep.description = [0u8; MAX_EPISODE_DESC_LEN];
        ep.description[..dlen].copy_from_slice(&desc[..dlen]);
        ep.desc_len = dlen;
        ep.detail = detail;

        self.head = (self.head + 1) % EPISODIC_CAPACITY;
        self.total += 1;
    }

    /// Query: count episodes matching a kind within the last `window` ticks.
    pub fn count_recent(&self, kind: EpisodeKind, current_tick: u64, window: u64) -> usize {
        let cutoff = current_tick.saturating_sub(window);
        self.episodes
            .iter()
            .filter(|e| e.tick >= cutoff && e.kind == kind)
            .count()
    }

    /// Query: find the most recent episode matching a kind.
    pub fn find_latest(&self, kind: EpisodeKind) -> Option<&Episode> {
        // Walk backwards from head.
        let cap = EPISODIC_CAPACITY;
        let n = cap.min(self.total as usize);
        for i in 0..n {
            let idx = (self.head + cap - 1 - i) % cap;
            if self.episodes[idx].kind == kind {
                return Some(&self.episodes[idx]);
            }
        }
        None
    }

    /// Query: find episodes for a specific intent.
    pub fn find_by_intent(&self, intent_id: u16) -> [usize; 8] {
        let mut results = [usize::MAX; 8];
        let mut ri = 0;
        let cap = EPISODIC_CAPACITY;
        let n = cap.min(self.total as usize);
        for i in 0..n {
            let idx = (self.head + cap - 1 - i) % cap;
            if self.episodes[idx].intent_id == intent_id {
                if ri < 8 {
                    results[ri] = idx;
                    ri += 1;
                }
            }
        }
        results
    }

    /// Get the success rate for a given episode kind over the last `window` ticks.
    /// Returns (successes, total) — caller divides.
    pub fn success_rate(
        &self,
        kind: EpisodeKind,
        current_tick: u64,
        window: u64,
    ) -> (usize, usize) {
        let cutoff = current_tick.saturating_sub(window);
        let mut total = 0usize;
        let mut success = 0usize;
        for ep in self.episodes.iter() {
            if ep.tick >= cutoff && ep.kind == kind {
                total += 1;
                if ep.outcome == EpisodeOutcome::Success {
                    success += 1;
                }
            }
        }
        (success, total)
    }
}

// ─── Unified Memory Engine ─────────────────────────────────────────────

/// The Memory Engine — unified access point for all three memory tiers.
///
/// Context memory lives inside each `AgentCb.context` (see `agent.rs`).
/// This struct owns the persistent and episodic tiers.
pub struct MemoryEngine {
    pub persistent: PersistentMemory,
    pub episodic: EpisodicMemory,
}

impl MemoryEngine {
    pub const fn new() -> Self {
        Self {
            persistent: PersistentMemory::new(),
            episodic: EpisodicMemory::new(),
        }
    }
}

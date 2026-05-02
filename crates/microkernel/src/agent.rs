//! Agent — first-class autonomous execution primitive.
//!
//! An **Agent** replaces the traditional process as the primary execution
//! unit in VeerOS's AI-native model.  Where a process blindly executes
//! instructions, an agent pursues *goals*: it maintains context, reasons
//! about its objective, coordinates with peer agents, and adapts its
//! strategy based on feedback from the memory engine.
//!
//! # Lifecycle
//!
//! ```text
//!  Spawned ──► Planning ──► Executing ──► Completed
//!                │   ▲         │                │
//!                │   └─────────┘ (re-plan)      │
//!                ▼                              ▼
//!            Blocked ──────────────────► Failed
//! ```
//!
//! # Hierarchy
//!
//! Agents can **delegate** sub-goals to child agents, forming a
//! goal-decomposition tree.  The root agent owns the user-facing intent;
//! leaf agents perform concrete work (I/O, inference, IPC).  A parent
//! monitors its children and re-plans if any child fails.
//!
//! # Kernel integration
//!
//! Each agent maps to exactly one kernel **thread** (TCB slot).  The
//! scheduler is aware of agent metadata (goal priority, deadline,
//! resource budget) and uses it for placement decisions.  Agents
//! communicate via the existing channel/IPC subsystem but add a
//! semantic envelope (`AgentMessage`) that carries goal context.

use crate::task::{Scheduler, TaskState, MAX_TASKS};

// ─── Configuration ──────────────────────────────────────────────────────

/// Maximum concurrent agents system-wide.
pub const MAX_AGENTS: usize = 32;

/// Maximum depth of the goal-decomposition tree.
pub const MAX_GOAL_DEPTH: usize = 8;

/// Maximum sub-agents a single parent may spawn.
pub const MAX_CHILDREN: usize = 8;

/// Maximum length of a goal description string (bytes).
pub const MAX_GOAL_LEN: usize = 64;

/// Maximum number of key-value pairs in agent context memory.
pub const MAX_CONTEXT_SLOTS: usize = 8;

/// Maximum length of a context key or value (bytes).
pub const MAX_CONTEXT_VALUE_LEN: usize = 32;

// ─── Agent state machine ────────────────────────────────────────────────

/// Lifecycle state of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AgentState {
    /// Slot is unused.
    Free = 0,
    /// Agent has been created but has not started planning.
    Spawned = 1,
    /// Agent is decomposing its goal into sub-tasks / sub-agents.
    Planning = 2,
    /// Agent is actively executing towards its goal.
    Executing = 3,
    /// Agent is waiting on an external event (child completion, I/O, etc.).
    Blocked = 4,
    /// Agent successfully achieved its goal.
    Completed = 5,
    /// Agent failed to achieve its goal after exhausting retries.
    Failed = 6,
}

/// Why an agent is blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AgentBlockReason {
    /// Not blocked.
    None = 0,
    /// Waiting for a child agent to complete.
    ChildWait = 1,
    /// Waiting for an intent resolution.
    IntentWait = 2,
    /// Waiting for a memory query result.
    MemoryQuery = 3,
    /// Waiting for a fabric node assignment.
    FabricWait = 4,
    /// Waiting for inter-agent message.
    MessageWait = 5,
}

// ─── Goal descriptor ────────────────────────────────────────────────────

/// Priority class for goal scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum GoalPriority {
    /// Background — best-effort, may be preempted freely.
    Background = 0,
    /// Normal — standard interactive priority.
    Normal = 1,
    /// Elevated — time-sensitive but not critical.
    Elevated = 2,
    /// Critical — hard deadline, preempts everything below.
    Critical = 3,
    /// Realtime — zero-miss, reserved for safety-critical goals.
    Realtime = 4,
}

/// A goal that an agent pursues.
#[derive(Debug, Clone, Copy)]
pub struct Goal {
    /// Human-readable goal description (e.g. "deploy app globally").
    pub description: [u8; MAX_GOAL_LEN],
    /// Actual length of description.
    pub desc_len: usize,
    /// Scheduling priority.
    pub priority: GoalPriority,
    /// Deadline in kernel ticks (0 = no deadline).
    pub deadline_tick: u64,
    /// Maximum wall-clock ticks allowed for this goal (0 = unlimited).
    pub budget_ticks: u64,
    /// Parent intent ID that spawned this goal (0 = top-level).
    pub intent_id: u16,
}

impl Goal {
    pub const fn empty() -> Self {
        Self {
            description: [0u8; MAX_GOAL_LEN],
            desc_len: 0,
            priority: GoalPriority::Normal,
            deadline_tick: 0,
            budget_ticks: 0,
            intent_id: 0,
        }
    }

    /// Create a goal from a byte slice description.
    pub fn from_bytes(desc: &[u8], priority: GoalPriority) -> Self {
        let mut g = Self::empty();
        let copy_len = desc.len().min(MAX_GOAL_LEN);
        g.description[..copy_len].copy_from_slice(&desc[..copy_len]);
        g.desc_len = copy_len;
        g.priority = priority;
        g
    }

    /// Get the goal description as a byte slice.
    pub fn desc_bytes(&self) -> &[u8] {
        &self.description[..self.desc_len]
    }
}

// ─── Agent context (working memory) ─────────────────────────────────────

/// A single key-value pair in agent working memory.
#[derive(Debug, Clone, Copy)]
pub struct ContextSlot {
    pub key: [u8; MAX_CONTEXT_VALUE_LEN],
    pub key_len: usize,
    pub value: [u8; MAX_CONTEXT_VALUE_LEN],
    pub value_len: usize,
}

impl ContextSlot {
    pub const fn empty() -> Self {
        Self {
            key: [0u8; MAX_CONTEXT_VALUE_LEN],
            key_len: 0,
            value: [0u8; MAX_CONTEXT_VALUE_LEN],
            value_len: 0,
        }
    }
}

/// Per-agent context memory — fast key-value store for task-local state.
#[derive(Debug, Clone, Copy)]
pub struct AgentContext {
    pub slots: [ContextSlot; MAX_CONTEXT_SLOTS],
    pub count: usize,
}

impl AgentContext {
    pub const fn empty() -> Self {
        Self {
            slots: [ContextSlot::empty(); MAX_CONTEXT_SLOTS],
            count: 0,
        }
    }

    /// Set a key-value pair. Overwrites if key exists, appends otherwise.
    pub fn set(&mut self, key: &[u8], value: &[u8]) -> bool {
        // Check if key already exists.
        for i in 0..self.count {
            if self.slots[i].key_len == key.len() && &self.slots[i].key[..key.len()] == key {
                let vlen = value.len().min(MAX_CONTEXT_VALUE_LEN);
                self.slots[i].value[..vlen].copy_from_slice(&value[..vlen]);
                self.slots[i].value_len = vlen;
                return true;
            }
        }
        // Append new slot.
        if self.count >= MAX_CONTEXT_SLOTS {
            return false;
        }
        let s = &mut self.slots[self.count];
        let klen = key.len().min(MAX_CONTEXT_VALUE_LEN);
        s.key[..klen].copy_from_slice(&key[..klen]);
        s.key_len = klen;
        let vlen = value.len().min(MAX_CONTEXT_VALUE_LEN);
        s.value[..vlen].copy_from_slice(&value[..vlen]);
        s.value_len = vlen;
        self.count += 1;
        true
    }

    /// Get a value by key.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        for i in 0..self.count {
            if self.slots[i].key_len == key.len() && &self.slots[i].key[..key.len()] == key {
                return Some(&self.slots[i].value[..self.slots[i].value_len]);
            }
        }
        None
    }
}

// ─── Agent Control Block ────────────────────────────────────────────────

/// Agent Control Block — the kernel-level descriptor for one agent.
///
/// Every agent maps 1:1 to a kernel thread (TCB).  The `task_id` field
/// links the agent to its backing thread in the scheduler.
#[derive(Debug, Clone, Copy)]
pub struct AgentCb {
    /// Lifecycle state.
    pub state: AgentState,
    /// Why this agent is blocked (only meaningful when `state == Blocked`).
    pub block_reason: AgentBlockReason,
    /// The goal this agent is pursuing.
    pub goal: Goal,
    /// Index of the backing kernel thread in the scheduler task table.
    pub task_id: usize,
    /// Parent agent ID (`usize::MAX` for root / top-level agents).
    pub parent: usize,
    /// Number of active child agents.
    pub child_count: usize,
    /// Child agent IDs.
    pub children: [usize; MAX_CHILDREN],
    /// Ticks consumed so far (for budget enforcement).
    pub ticks_used: u64,
    /// Tick at which this agent was spawned.
    pub spawn_tick: u64,
    /// Number of times this agent has re-planned.
    pub replan_count: u16,
    /// Maximum replans before failure (0 = unlimited).
    pub max_replans: u16,
    /// Working memory for this agent.
    pub context: AgentContext,
    /// Channel ID for inter-agent communication (-1 = none).
    pub comm_channel: usize,
}

impl AgentCb {
    pub const fn empty() -> Self {
        Self {
            state: AgentState::Free,
            block_reason: AgentBlockReason::None,
            goal: Goal::empty(),
            task_id: usize::MAX,
            parent: usize::MAX,
            child_count: 0,
            children: [usize::MAX; MAX_CHILDREN],
            ticks_used: 0,
            spawn_tick: 0,
            replan_count: 0,
            max_replans: 3,
            context: AgentContext::empty(),
            comm_channel: usize::MAX,
        }
    }
}

// ─── Agent Table ────────────────────────────────────────────────────────

/// System-wide agent table.
pub struct AgentTable {
    pub agents: [AgentCb; MAX_AGENTS],
    /// Monotonically increasing generation counter for agent IDs.
    pub generation: u32,
}

impl AgentTable {
    pub const fn new() -> Self {
        Self {
            agents: [AgentCb::empty(); MAX_AGENTS],
            generation: 0,
        }
    }

    /// Spawn a new agent bound to the given task (thread) ID.
    ///
    /// Returns the agent ID on success, or `None` if the table is full.
    pub fn spawn(&mut self, goal: Goal, task_id: usize, parent: usize, tick: u64) -> Option<usize> {
        for (i, slot) in self.agents.iter_mut().enumerate() {
            if slot.state == AgentState::Free {
                *slot = AgentCb {
                    state: AgentState::Spawned,
                    block_reason: AgentBlockReason::None,
                    goal,
                    task_id,
                    parent,
                    child_count: 0,
                    children: [usize::MAX; MAX_CHILDREN],
                    ticks_used: 0,
                    spawn_tick: tick,
                    replan_count: 0,
                    max_replans: 3,
                    context: AgentContext::empty(),
                    comm_channel: usize::MAX,
                };
                // Register as child of parent.
                if parent < MAX_AGENTS && self.agents[parent].state != AgentState::Free {
                    let p = &mut self.agents[parent];
                    if p.child_count < MAX_CHILDREN {
                        p.children[p.child_count] = i;
                        p.child_count += 1;
                    }
                }
                self.generation += 1;
                return Some(i);
            }
        }
        None
    }

    /// Transition an agent to a new state.
    pub fn transition(&mut self, agent_id: usize, new_state: AgentState) -> bool {
        if agent_id >= MAX_AGENTS {
            return false;
        }
        let agent = &mut self.agents[agent_id];
        if agent.state == AgentState::Free {
            return false;
        }
        agent.state = new_state;
        true
    }

    /// Mark an agent as completed and propagate to parent.
    ///
    /// If all siblings are done, the parent is unblocked.
    pub fn complete(&mut self, agent_id: usize) -> bool {
        if agent_id >= MAX_AGENTS {
            return false;
        }
        self.agents[agent_id].state = AgentState::Completed;
        let parent = self.agents[agent_id].parent;
        self.try_unblock_parent(parent);
        true
    }

    /// Mark an agent as failed.  If it has retries left, re-enter Planning.
    ///
    /// Returns `true` if the agent will re-plan, `false` if permanently failed.
    pub fn fail(&mut self, agent_id: usize) -> bool {
        if agent_id >= MAX_AGENTS {
            return false;
        }
        let agent = &mut self.agents[agent_id];
        if agent.max_replans == 0 || agent.replan_count < agent.max_replans {
            agent.replan_count += 1;
            agent.state = AgentState::Planning;
            return true; // will re-plan
        }
        agent.state = AgentState::Failed;
        let parent = agent.parent;
        // Propagate failure upward.
        if parent < MAX_AGENTS {
            self.fail(parent);
        }
        false
    }

    /// Check if all children of `parent` are done (completed or failed).
    /// If so, unblock the parent.
    fn try_unblock_parent(&mut self, parent: usize) {
        if parent >= MAX_AGENTS {
            return;
        }
        let p = &self.agents[parent];
        if p.state != AgentState::Blocked {
            return;
        }
        let all_done = (0..p.child_count).all(|ci| {
            let child_id = p.children[ci];
            child_id >= MAX_AGENTS
                || matches!(
                    self.agents[child_id].state,
                    AgentState::Completed | AgentState::Failed | AgentState::Free
                )
        });
        if all_done {
            self.agents[parent].state = AgentState::Executing;
            self.agents[parent].block_reason = AgentBlockReason::None;
        }
    }

    /// Free an agent slot and detach from parent.
    pub fn destroy(&mut self, agent_id: usize) {
        if agent_id >= MAX_AGENTS {
            return;
        }
        let parent = self.agents[agent_id].parent;
        // Remove from parent's child list.
        if parent < MAX_AGENTS {
            let p = &mut self.agents[parent];
            for ci in 0..p.child_count {
                if p.children[ci] == agent_id {
                    // Shift remaining children down.
                    let mut j = ci;
                    while j + 1 < p.child_count {
                        p.children[j] = p.children[j + 1];
                        j += 1;
                    }
                    p.children[p.child_count - 1] = usize::MAX;
                    p.child_count -= 1;
                    break;
                }
            }
        }
        // Recursively destroy children.
        let cc = self.agents[agent_id].child_count;
        for ci in 0..cc {
            let child = self.agents[agent_id].children[ci];
            if child < MAX_AGENTS {
                self.destroy(child);
            }
        }
        self.agents[agent_id] = AgentCb::empty();
    }

    /// Get the agent ID associated with a kernel task (thread) ID.
    pub fn find_by_task(&self, task_id: usize) -> Option<usize> {
        for (i, a) in self.agents.iter().enumerate() {
            if a.state != AgentState::Free && a.task_id == task_id {
                return Some(i);
            }
        }
        None
    }

    /// Count active (non-Free) agents.
    pub fn active_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| a.state != AgentState::Free)
            .count()
    }

    /// Tick all executing agents — increment ticks_used and enforce budgets.
    ///
    /// Returns a list of agent IDs that exceeded their budget (up to 8).
    pub fn tick(&mut self, current_tick: u64) -> [usize; 8] {
        let mut expired = [usize::MAX; 8];
        let mut ei = 0;
        for (i, a) in self.agents.iter_mut().enumerate() {
            if a.state == AgentState::Executing {
                a.ticks_used += 1;
                // Check deadline.
                if a.goal.deadline_tick > 0 && current_tick >= a.goal.deadline_tick {
                    if ei < 8 {
                        expired[ei] = i;
                        ei += 1;
                    }
                }
                // Check budget.
                if a.goal.budget_ticks > 0 && a.ticks_used >= a.goal.budget_ticks {
                    if ei < 8 {
                        expired[ei] = i;
                        ei += 1;
                    }
                }
            }
        }
        expired
    }
}

// ─── Inter-agent message envelope ───────────────────────────────────────

/// Semantic message type for agent-to-agent communication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AgentMsgKind {
    /// Request: "please do X" (parent → child or peer → peer).
    Request = 0,
    /// Response: "here is the result of X".
    Response = 1,
    /// Status update: "I am N% done".
    Progress = 2,
    /// Error: "I failed at X because Y".
    Error = 3,
    /// Coordination: "I need resource R" / "I release resource R".
    Coordinate = 4,
    /// Observation: "I noticed X" (for episodic memory logging).
    Observation = 5,
}

/// Envelope for agent messages sent over kernel channels.
///
/// Fits in a `ChanMsg` (4 × usize words):
///   word0: sender_agent_id | (kind << 16) | (seq << 24)
///   word1: payload0
///   word2: payload1
///   word3: goal_context (intent_id | priority << 16)
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct AgentMessage {
    pub sender: u16,
    pub kind: AgentMsgKind,
    pub seq: u8,
    pub payload: [usize; 2],
    pub intent_id: u16,
    pub priority: GoalPriority,
}

impl AgentMessage {
    /// Pack into 4 usize words for channel transport.
    pub fn pack(&self) -> [usize; 4] {
        let w0 =
            (self.sender as usize) | ((self.kind as usize) << 16) | ((self.seq as usize) << 24);
        let w3 = (self.intent_id as usize) | ((self.priority as usize) << 16);
        [w0, self.payload[0], self.payload[1], w3]
    }

    /// Unpack from 4 usize words.
    pub fn unpack(words: [usize; 4]) -> Self {
        Self {
            sender: (words[0] & 0xFFFF) as u16,
            kind: match (words[0] >> 16) & 0xFF {
                0 => AgentMsgKind::Request,
                1 => AgentMsgKind::Response,
                2 => AgentMsgKind::Progress,
                3 => AgentMsgKind::Error,
                4 => AgentMsgKind::Coordinate,
                _ => AgentMsgKind::Observation,
            },
            seq: ((words[0] >> 24) & 0xFF) as u8,
            payload: [words[1], words[2]],
            intent_id: (words[3] & 0xFFFF) as u16,
            priority: match (words[3] >> 16) & 0xFF {
                0 => GoalPriority::Background,
                1 => GoalPriority::Normal,
                2 => GoalPriority::Elevated,
                3 => GoalPriority::Critical,
                _ => GoalPriority::Realtime,
            },
        }
    }
}

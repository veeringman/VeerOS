//! Intent Engine — goal decomposition and planning.
//!
//! The Intent Engine is the bridge between human-expressed goals and
//! machine-executable agent trees.  When a user (or another agent)
//! submits an **Intent**, the engine:
//!
//!  1. **Parses** the intent into a structured `IntentDescriptor`.
//!  2. **Decomposes** the intent into a DAG of sub-goals via `Plan`.
//!  3. **Assigns** each sub-goal to an agent (new or existing).
//!  4. **Monitors** execution and triggers re-planning on failure.
//!
//! # Design philosophy
//!
//! The intent engine is *not* an LLM in the kernel.  It is a
//! deterministic, rule-based planner that maps structured intents to
//! agent graphs.  The "intelligence" comes from:
//!
//! - **Intent taxonomy** — a finite, well-typed set of intent classes.
//! - **Capability matching** — agents declare what they can do; the
//!   planner assigns goals to agents whose capabilities intersect.
//! - **Feedback loops** — episodic memory from past executions refines
//!   future plans (e.g. "last time step 3 failed, try alternative").
//!
//! Higher-level NL parsing (LLM-assisted) lives in userspace or the
//! shell layer and translates free-form text into `IntentDescriptor`
//! before calling `SYS_INTENT_SUBMIT`.

use crate::agent::{AgentState, AgentTable, Goal, GoalPriority, MAX_AGENTS};

// ─── Configuration ──────────────────────────────────────────────────────

/// Maximum intents that can be in-flight simultaneously.
pub const MAX_INTENTS: usize = 16;

/// Maximum steps in a single execution plan.
pub const MAX_PLAN_STEPS: usize = 16;

/// Maximum length of an intent description (bytes).
pub const MAX_INTENT_DESC_LEN: usize = 128;

/// Maximum number of constraints on an intent.
pub const MAX_INTENT_CONSTRAINTS: usize = 4;

// ─── Intent taxonomy ────────────────────────────────────────────────────

/// Top-level intent classification.
///
/// Each class maps to a different decomposition strategy.  The taxonomy
/// is intentionally coarse — refinement happens via constraints and
/// parameters rather than an explosion of enum variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IntentClass {
    /// Execute a computation (transform data, run inference, etc.).
    Compute = 0,
    /// Deploy or provision a service / workload.
    Deploy = 1,
    /// Monitor a system, metric, or condition.
    Monitor = 2,
    /// Communicate with an external system or user.
    Communicate = 3,
    /// Store, retrieve, or transform data.
    Data = 4,
    /// Manage system configuration or resources.
    Admin = 5,
    /// Composite: a pipeline of multiple intent classes.
    Pipeline = 6,
    /// User-defined / extensible intent class.
    Custom = 255,
}

/// Constraint on intent execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ConstraintKind {
    /// Maximum latency in milliseconds.
    MaxLatencyMs = 0,
    /// Must execute on a specific node class.
    NodeAffinity = 1,
    /// Minimum reliability (0–100 percent).
    MinReliability = 2,
    /// Maximum resource cost (abstract units).
    MaxCost = 3,
    /// Geographic locality requirement.
    Locality = 4,
    /// Security clearance level required.
    SecurityLevel = 5,
}

/// A single constraint attached to an intent.
#[derive(Debug, Clone, Copy)]
pub struct IntentConstraint {
    pub kind: ConstraintKind,
    pub value: usize,
}

impl IntentConstraint {
    pub const fn empty() -> Self {
        Self {
            kind: ConstraintKind::MaxLatencyMs,
            value: 0,
        }
    }
}

// ─── Intent descriptor ──────────────────────────────────────────────────

/// Status of an intent through its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum IntentStatus {
    /// Slot is unused.
    Free = 0,
    /// Intent received, awaiting planning.
    Pending = 1,
    /// Plan generated, agents being spawned.
    Planning = 2,
    /// Agents executing the plan.
    Active = 3,
    /// All plan steps completed successfully.
    Fulfilled = 4,
    /// Intent failed after exhausting retries.
    Failed = 5,
    /// Intent was cancelled by the submitter.
    Cancelled = 6,
}

/// Structured intent descriptor — the kernel-level representation of
/// a user's goal.
#[derive(Debug, Clone, Copy)]
pub struct IntentDescriptor {
    /// Unique intent ID (assigned by the engine).
    pub id: u16,
    /// Classification.
    pub class: IntentClass,
    /// Lifecycle status.
    pub status: IntentStatus,
    /// Human-readable description.
    pub description: [u8; MAX_INTENT_DESC_LEN],
    pub desc_len: usize,
    /// Priority for scheduling.
    pub priority: GoalPriority,
    /// Constraints.
    pub constraints: [IntentConstraint; MAX_INTENT_CONSTRAINTS],
    pub constraint_count: usize,
    /// The root agent assigned to this intent.
    pub root_agent: usize,
    /// Task ID of the submitter (for result delivery).
    pub submitter_task: usize,
    /// Tick at which the intent was submitted.
    pub submit_tick: u64,
    /// Tick at which the intent was fulfilled/failed.
    pub resolve_tick: u64,
}

impl IntentDescriptor {
    pub const fn empty() -> Self {
        Self {
            id: 0,
            class: IntentClass::Compute,
            status: IntentStatus::Free,
            description: [0u8; MAX_INTENT_DESC_LEN],
            desc_len: 0,
            priority: GoalPriority::Normal,
            constraints: [IntentConstraint::empty(); MAX_INTENT_CONSTRAINTS],
            constraint_count: 0,
            root_agent: usize::MAX,
            submitter_task: usize::MAX,
            submit_tick: 0,
            resolve_tick: 0,
        }
    }
}

// ─── Execution plan ─────────────────────────────────────────────────────

/// Dependency relationship between plan steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StepRelation {
    /// No dependency (can execute immediately).
    Independent = 0,
    /// Must wait for the referenced step to complete.
    DependsOn = 1,
    /// Must execute concurrently with the referenced step.
    Parallel = 2,
}

/// One step in an execution plan.
#[derive(Debug, Clone, Copy)]
pub struct PlanStep {
    /// Goal to be pursued by the agent assigned to this step.
    pub goal: Goal,
    /// Relation to another step (index into plan steps array).
    pub relation: StepRelation,
    /// Index of the step this depends on (meaningful only if `relation != Independent`).
    pub depends_on: usize,
    /// Agent assigned to this step (filled during assignment phase).
    pub agent_id: usize,
    /// Whether this step has completed.
    pub completed: bool,
    /// Whether this step has failed.
    pub failed: bool,
}

impl PlanStep {
    pub const fn empty() -> Self {
        Self {
            goal: Goal::empty(),
            relation: StepRelation::Independent,
            depends_on: 0,
            agent_id: usize::MAX,
            completed: false,
            failed: false,
        }
    }
}

/// An execution plan — a DAG of steps produced by intent decomposition.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    pub steps: [PlanStep; MAX_PLAN_STEPS],
    pub step_count: usize,
    /// Whether the plan allows partial success (best-effort).
    pub allow_partial: bool,
}

impl Plan {
    pub const fn empty() -> Self {
        Self {
            steps: [PlanStep::empty(); MAX_PLAN_STEPS],
            step_count: 0,
            allow_partial: false,
        }
    }

    /// Add a step to the plan.
    pub fn add_step(
        &mut self,
        goal: Goal,
        relation: StepRelation,
        depends_on: usize,
    ) -> Option<usize> {
        if self.step_count >= MAX_PLAN_STEPS {
            return None;
        }
        let idx = self.step_count;
        self.steps[idx] = PlanStep {
            goal,
            relation,
            depends_on,
            agent_id: usize::MAX,
            completed: false,
            failed: false,
        };
        self.step_count += 1;
        Some(idx)
    }

    /// Check if all prerequisite steps for `step_idx` are complete.
    pub fn step_ready(&self, step_idx: usize) -> bool {
        if step_idx >= self.step_count {
            return false;
        }
        let step = &self.steps[step_idx];
        match step.relation {
            StepRelation::Independent | StepRelation::Parallel => true,
            StepRelation::DependsOn => {
                let dep = step.depends_on;
                dep < self.step_count && self.steps[dep].completed
            }
        }
    }

    /// Check if the plan is fully resolved (all steps completed or failed).
    pub fn is_resolved(&self) -> bool {
        (0..self.step_count).all(|i| self.steps[i].completed || self.steps[i].failed)
    }

    /// Check if the plan succeeded (all required steps completed).
    pub fn is_success(&self) -> bool {
        if self.allow_partial {
            // At least one step succeeded.
            (0..self.step_count).any(|i| self.steps[i].completed)
        } else {
            (0..self.step_count).all(|i| self.steps[i].completed)
        }
    }
}

// ─── Intent Engine ──────────────────────────────────────────────────────

/// The Intent Engine — manages the lifecycle of all in-flight intents.
pub struct IntentEngine {
    pub intents: [IntentDescriptor; MAX_INTENTS],
    /// Plans corresponding to each intent slot.
    pub plans: [Plan; MAX_INTENTS],
    /// Next intent ID to assign.
    next_id: u16,
}

impl IntentEngine {
    pub const fn new() -> Self {
        Self {
            intents: [IntentDescriptor::empty(); MAX_INTENTS],
            plans: [Plan::empty(); MAX_INTENTS],
            next_id: 1,
        }
    }

    /// Submit a new intent.
    ///
    /// Returns the intent ID on success, or `None` if the table is full.
    pub fn submit(
        &mut self,
        class: IntentClass,
        desc: &[u8],
        priority: GoalPriority,
        submitter_task: usize,
        tick: u64,
    ) -> Option<u16> {
        for (i, slot) in self.intents.iter_mut().enumerate() {
            if slot.status == IntentStatus::Free {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1);
                if self.next_id == 0 {
                    self.next_id = 1;
                }

                let copy_len = desc.len().min(MAX_INTENT_DESC_LEN);
                slot.id = id;
                slot.class = class;
                slot.status = IntentStatus::Pending;
                slot.description[..copy_len].copy_from_slice(&desc[..copy_len]);
                slot.desc_len = copy_len;
                slot.priority = priority;
                slot.constraint_count = 0;
                slot.root_agent = usize::MAX;
                slot.submitter_task = submitter_task;
                slot.submit_tick = tick;
                slot.resolve_tick = 0;
                self.plans[i] = Plan::empty();
                return Some(id);
            }
        }
        None
    }

    /// Add a constraint to a pending intent.
    pub fn add_constraint(&mut self, intent_id: u16, kind: ConstraintKind, value: usize) -> bool {
        if let Some(slot) = self.find_mut(intent_id) {
            if slot.constraint_count >= MAX_INTENT_CONSTRAINTS {
                return false;
            }
            slot.constraints[slot.constraint_count] = IntentConstraint { kind, value };
            slot.constraint_count += 1;
            true
        } else {
            false
        }
    }

    /// Decompose a pending intent into an execution plan.
    ///
    /// This is the core planning function.  The current implementation
    /// uses a simple rule-based decomposition keyed on `IntentClass`.
    /// Future versions will consult episodic memory for refinement.
    pub fn decompose(&mut self, intent_id: u16) -> bool {
        let idx = match self.find_index(intent_id) {
            Some(i) => i,
            None => return false,
        };
        if self.intents[idx].status != IntentStatus::Pending {
            return false;
        }

        let intent = &self.intents[idx];
        let mut plan = Plan::empty();

        match intent.class {
            IntentClass::Compute => {
                // Single-step: execute the computation.
                plan.add_step(
                    Goal::from_bytes(intent.desc_bytes(), intent.priority),
                    StepRelation::Independent,
                    0,
                );
            }
            IntentClass::Deploy => {
                // Three-phase: validate → provision → verify.
                let s0 = plan
                    .add_step(
                        Goal::from_bytes(b"validate deployment config", GoalPriority::Elevated),
                        StepRelation::Independent,
                        0,
                    )
                    .unwrap_or(0);
                let s1 = plan
                    .add_step(
                        Goal::from_bytes(b"provision resources", intent.priority),
                        StepRelation::DependsOn,
                        s0,
                    )
                    .unwrap_or(0);
                plan.add_step(
                    Goal::from_bytes(b"verify deployment health", GoalPriority::Critical),
                    StepRelation::DependsOn,
                    s1,
                );
            }
            IntentClass::Monitor => {
                // Single long-running monitor agent.
                plan.add_step(
                    Goal::from_bytes(intent.desc_bytes(), intent.priority),
                    StepRelation::Independent,
                    0,
                );
            }
            IntentClass::Pipeline => {
                // The description encodes a pipe — decompose linearly.
                // For now, create a single "execute pipeline" step.
                plan.add_step(
                    Goal::from_bytes(intent.desc_bytes(), intent.priority),
                    StepRelation::Independent,
                    0,
                );
            }
            IntentClass::Data => {
                // Two-phase: acquire → transform.
                let s0 = plan
                    .add_step(
                        Goal::from_bytes(b"acquire data", intent.priority),
                        StepRelation::Independent,
                        0,
                    )
                    .unwrap_or(0);
                plan.add_step(
                    Goal::from_bytes(b"transform data", intent.priority),
                    StepRelation::DependsOn,
                    s0,
                );
            }
            IntentClass::Communicate | IntentClass::Admin | IntentClass::Custom => {
                // Default: single step.
                plan.add_step(
                    Goal::from_bytes(intent.desc_bytes(), intent.priority),
                    StepRelation::Independent,
                    0,
                );
            }
        }

        self.plans[idx] = plan;
        self.intents[idx].status = IntentStatus::Planning;
        true
    }

    /// Advance the intent — check plan progress, update status.
    ///
    /// Called periodically (e.g. from scheduler tick or explicit syscall).
    pub fn tick(&mut self, agents: &AgentTable, current_tick: u64) {
        for i in 0..MAX_INTENTS {
            match self.intents[i].status {
                IntentStatus::Active => {
                    let plan = &mut self.plans[i];
                    // Sync step completion from agent states.
                    for s in 0..plan.step_count {
                        let aid = plan.steps[s].agent_id;
                        if aid < MAX_AGENTS {
                            match agents.agents[aid].state {
                                AgentState::Completed => plan.steps[s].completed = true,
                                AgentState::Failed => plan.steps[s].failed = true,
                                _ => {}
                            }
                        }
                    }
                    if plan.is_resolved() {
                        if plan.is_success() {
                            self.intents[i].status = IntentStatus::Fulfilled;
                        } else {
                            self.intents[i].status = IntentStatus::Failed;
                        }
                        self.intents[i].resolve_tick = current_tick;
                    }
                }
                _ => {}
            }
        }
    }

    /// Cancel an intent.  Marks all associated agents for teardown.
    pub fn cancel(&mut self, intent_id: u16) -> bool {
        if let Some(idx) = self.find_index(intent_id) {
            if self.intents[idx].status == IntentStatus::Free {
                return false;
            }
            self.intents[idx].status = IntentStatus::Cancelled;
            true
        } else {
            false
        }
    }

    /// Query the status of an intent.
    pub fn status(&self, intent_id: u16) -> Option<IntentStatus> {
        self.find(intent_id).map(|d| d.status)
    }

    /// Count active (non-Free) intents.
    pub fn active_count(&self) -> usize {
        self.intents
            .iter()
            .filter(|d| d.status != IntentStatus::Free)
            .count()
    }

    // ── Internal helpers ─────────────────────────────────────────────

    fn find(&self, intent_id: u16) -> Option<&IntentDescriptor> {
        self.intents
            .iter()
            .find(|d| d.id == intent_id && d.status != IntentStatus::Free)
    }

    fn find_mut(&mut self, intent_id: u16) -> Option<&mut IntentDescriptor> {
        self.intents
            .iter_mut()
            .find(|d| d.id == intent_id && d.status != IntentStatus::Free)
    }

    fn find_index(&self, intent_id: u16) -> Option<usize> {
        self.intents
            .iter()
            .position(|d| d.id == intent_id && d.status != IntentStatus::Free)
    }

    /// Get the description bytes for an intent (helper for decompose).
    fn desc_of(&self, idx: usize) -> &[u8] {
        &self.intents[idx].description[..self.intents[idx].desc_len]
    }
}

impl IntentDescriptor {
    /// Get description as byte slice.
    pub fn desc_bytes(&self) -> &[u8] {
        &self.description[..self.desc_len]
    }
}

//! Intent Scheduler — goal-priority scheduling for AI-native execution.
//!
//! The Intent Scheduler is a *meta-scheduler* that sits above the
//! existing task scheduler.  Where the task scheduler picks which
//! **thread** runs next based on priority and state, the intent
//! scheduler decides which **goals** should be pursued, which agents
//! to spawn, and how to allocate the compute fabric.
//!
//! # Scheduling loop
//!
//! ```text
//!  ┌──────────────┐
//!  │  Pending      │◄─── SYS_INTENT_SUBMIT
//!  │  Intents      │
//!  └──────┬───────┘
//!         │ decompose()
//!         ▼
//!  ┌──────────────┐
//!  │  Plans        │
//!  │  (step DAGs)  │
//!  └──────┬───────┘
//!         │ assign_agents()
//!         ▼
//!  ┌──────────────┐
//!  │  Agents       │──── per-step agent, bound to thread
//!  │  (executing)  │
//!  └──────┬───────┘
//!         │ tick() — monitor progress, enforce budgets
//!         ▼
//!  ┌──────────────┐
//!  │  Memory       │──── record episodes, update knowledge
//!  │  Engine       │
//!  └──────────────┘
//! ```
//!
//! # Integration points
//!
//! - Called from the timer ISR (via `tick()`) for periodic monitoring.
//! - Called from `SYS_INTENT_SUBMIT` for new intent processing.
//! - Reads/writes the agent table, intent engine, memory engine, fabric.

use crate::agent::{AgentBlockReason, AgentState, AgentTable, Goal, GoalPriority, MAX_AGENTS};
use crate::fabric::{ExecutionFabric, NodeCapability, PlacementConstraint};
use crate::intent::{IntentClass, IntentEngine, IntentStatus, MAX_INTENTS};
use crate::memory_engine::{EpisodeKind, EpisodeOutcome, MemoryEngine, MemoryScope, MemoryTag};
use crate::task::{Scheduler, TaskState};

// ─── Configuration ──────────────────────────────────────────────────────

/// How often (in ticks) the intent scheduler runs its full cycle.
pub const INTENT_SCHED_INTERVAL: u64 = 10;

/// Health check interval for fabric nodes (in ticks).
pub const FABRIC_HEALTH_INTERVAL: u64 = 100;

/// Heartbeat timeout before a node is marked offline (in ticks).
pub const FABRIC_HEARTBEAT_TIMEOUT: u64 = 500;

// ─── Scheduling statistics ──────────────────────────────────────────────

/// Runtime statistics for the intent scheduler.
#[derive(Debug, Clone, Copy)]
pub struct IntentSchedStats {
    /// Total intents submitted since boot.
    pub intents_submitted: u64,
    /// Total intents successfully fulfilled.
    pub intents_fulfilled: u64,
    /// Total intents that failed.
    pub intents_failed: u64,
    /// Total agents spawned.
    pub agents_spawned: u64,
    /// Total agents completed.
    pub agents_completed: u64,
    /// Total agents failed.
    pub agents_failed: u64,
    /// Total re-plans triggered.
    pub replans: u64,
    /// Last tick the scheduler ran.
    pub last_tick: u64,
}

impl IntentSchedStats {
    pub const fn new() -> Self {
        Self {
            intents_submitted: 0,
            intents_fulfilled: 0,
            intents_failed: 0,
            agents_spawned: 0,
            agents_completed: 0,
            agents_failed: 0,
            replans: 0,
            last_tick: 0,
        }
    }
}

// ─── Intent Scheduler ───────────────────────────────────────────────────

/// The Intent Scheduler — orchestrates the AI-native execution loop.
pub struct IntentScheduler {
    pub stats: IntentSchedStats,
}

impl IntentScheduler {
    pub const fn new() -> Self {
        Self {
            stats: IntentSchedStats::new(),
        }
    }

    /// Main scheduling tick — called periodically from the timer ISR.
    ///
    /// 1. Process pending intents (decompose → plan).
    /// 2. Assign agents to ready plan steps.
    /// 3. Monitor executing agents (budget, deadline).
    /// 4. Update memory engine with episodes.
    /// 5. Check fabric health.
    pub fn tick(
        &mut self,
        current_tick: u64,
        intents: &mut IntentEngine,
        agents: &mut AgentTable,
        memory: &mut MemoryEngine,
        fabric: &mut ExecutionFabric,
        sched: &mut Scheduler,
    ) {
        // Rate limit: only run full cycle every INTENT_SCHED_INTERVAL ticks.
        if current_tick.saturating_sub(self.stats.last_tick) < INTENT_SCHED_INTERVAL {
            return;
        }
        self.stats.last_tick = current_tick;

        // Phase 1: Decompose pending intents into plans.
        self.process_pending_intents(intents, current_tick);

        // Phase 2: Assign agents to ready plan steps.
        self.assign_agents(intents, agents, fabric, sched, memory, current_tick);

        // Phase 3: Monitor executing agents.
        self.monitor_agents(agents, intents, memory, sched, current_tick);

        // Phase 4: Sync intent status from agent completion.
        intents.tick(agents, current_tick);

        // Phase 5: Record fulfilled/failed intents in episodic memory.
        self.record_intent_outcomes(intents, memory, current_tick);

        // Phase 6: Fabric health check (less frequent).
        if current_tick % FABRIC_HEALTH_INTERVAL == 0 {
            fabric.check_health(current_tick, FABRIC_HEARTBEAT_TIMEOUT);
        }
    }

    // ── Phase 1: Intent decomposition ────────────────────────────────

    fn process_pending_intents(&mut self, intents: &mut IntentEngine, tick: u64) {
        for i in 0..MAX_INTENTS {
            if intents.intents[i].status == IntentStatus::Pending {
                let id = intents.intents[i].id;
                if intents.decompose(id) {
                    self.stats.intents_submitted += 1;
                }
            }
        }
    }

    // ── Phase 2: Agent assignment ────────────────────────────────────

    fn assign_agents(
        &mut self,
        intents: &mut IntentEngine,
        agents: &mut AgentTable,
        fabric: &ExecutionFabric,
        sched: &mut Scheduler,
        memory: &mut MemoryEngine,
        tick: u64,
    ) {
        for i in 0..MAX_INTENTS {
            if intents.intents[i].status != IntentStatus::Planning
                && intents.intents[i].status != IntentStatus::Active
            {
                continue;
            }

            let plan = &mut intents.plans[i];
            let intent_id = intents.intents[i].id;
            let mut any_active = false;

            for s in 0..plan.step_count {
                if plan.steps[s].completed || plan.steps[s].failed {
                    continue;
                }
                if plan.steps[s].agent_id < MAX_AGENTS {
                    any_active = true;
                    continue; // already assigned
                }
                if !plan.step_ready(s) {
                    continue; // dependencies not met
                }

                // Select placement node.
                let constraint =
                    self.build_placement_constraint(&plan.steps[s].goal, &intents.intents[i]);
                let _node = fabric.select_node(&constraint);

                // Spawn agent for this step.
                // For now, agents execute on the local node (remote dispatch
                // is wired in Phase 11 — distributed cluster).
                let mut goal = plan.steps[s].goal;
                goal.intent_id = intent_id;

                // Create a kernel thread for this agent.
                // The agent's entry point and stack must be provided by the
                // intent submitter or a registered agent factory.  For now
                // we just record the agent without a backing thread — the
                // caller (dispatch.rs) wires the thread at spawn time.
                if let Some(agent_id) = agents.spawn(goal, usize::MAX, usize::MAX, tick) {
                    plan.steps[s].agent_id = agent_id;
                    agents.transition(agent_id, AgentState::Executing);
                    self.stats.agents_spawned += 1;
                    any_active = true;

                    // Episodic record.
                    memory.episodic.record(
                        tick,
                        EpisodeKind::AgentSpawned,
                        EpisodeOutcome::Pending,
                        agent_id as u16,
                        intent_id,
                        goal.desc_bytes(),
                        0,
                    );
                }
            }

            if any_active && intents.intents[i].status == IntentStatus::Planning {
                intents.intents[i].status = IntentStatus::Active;
            }
        }
    }

    /// Build placement constraint from goal + intent constraints.
    fn build_placement_constraint(
        &self,
        goal: &Goal,
        intent: &crate::intent::IntentDescriptor,
    ) -> PlacementConstraint {
        let mut pc = PlacementConstraint::any();

        // Map intent constraints to placement constraints.
        for ci in 0..intent.constraint_count {
            let c = &intent.constraints[ci];
            match c.kind {
                crate::intent::ConstraintKind::MaxLatencyMs => {
                    pc.max_rtt_us = (c.value as u32).saturating_mul(1000);
                }
                crate::intent::ConstraintKind::MinReliability => {
                    // High reliability → prefer local or rack-local nodes.
                    if c.value > 90 {
                        pc.preferred_zone = crate::fabric::LocalityZone::Rack;
                    }
                }
                crate::intent::ConstraintKind::Locality => {
                    pc.preferred_zone = match c.value {
                        0 => crate::fabric::LocalityZone::Local,
                        1 => crate::fabric::LocalityZone::Rack,
                        2 => crate::fabric::LocalityZone::DataCenter,
                        3 => crate::fabric::LocalityZone::Region,
                        _ => crate::fabric::LocalityZone::Global,
                    };
                }
                _ => {}
            }
        }

        // Goal priority → prefer low-load nodes for critical goals.
        if goal.priority >= GoalPriority::Critical {
            pc.prefer_low_load = true;
        }

        pc
    }

    // ── Phase 3: Agent monitoring ────────────────────────────────────

    fn monitor_agents(
        &mut self,
        agents: &mut AgentTable,
        intents: &mut IntentEngine,
        memory: &mut MemoryEngine,
        _sched: &mut Scheduler,
        tick: u64,
    ) {
        let expired = agents.tick(tick);

        for &agent_id in expired.iter() {
            if agent_id >= MAX_AGENTS {
                continue;
            }

            // Budget or deadline exceeded — attempt re-plan or fail.
            let intent_id = agents.agents[agent_id].goal.intent_id;
            let replanned = agents.fail(agent_id);

            memory.episodic.record(
                tick,
                EpisodeKind::BudgetExceeded,
                if replanned {
                    EpisodeOutcome::Partial
                } else {
                    EpisodeOutcome::Failure
                },
                agent_id as u16,
                intent_id,
                b"budget/deadline exceeded",
                agents.agents[agent_id].ticks_used as u32,
            );

            if replanned {
                self.stats.replans += 1;
            } else {
                self.stats.agents_failed += 1;
            }
        }

        // Check for newly completed agents.
        for i in 0..MAX_AGENTS {
            let a = &agents.agents[i];
            if a.state == AgentState::Completed {
                // Check if we already recorded this (avoid double-count).
                let intent_id = a.goal.intent_id;
                self.stats.agents_completed += 1;

                memory.episodic.record(
                    tick,
                    EpisodeKind::AgentCompleted,
                    EpisodeOutcome::Success,
                    i as u16,
                    intent_id,
                    a.goal.desc_bytes(),
                    a.ticks_used as u32,
                );

                // Store successful strategy in persistent memory.
                memory.persistent.store(
                    a.goal.desc_bytes(),
                    b"success",
                    MemoryTag::Skill,
                    MemoryScope::Global,
                    0,
                    tick,
                    200, // high confidence
                );

                // Free the agent slot.
                agents.destroy(i);
            } else if a.state == AgentState::Failed {
                let intent_id = a.goal.intent_id;
                self.stats.agents_failed += 1;

                memory.episodic.record(
                    tick,
                    EpisodeKind::AgentFailed,
                    EpisodeOutcome::Failure,
                    i as u16,
                    intent_id,
                    a.goal.desc_bytes(),
                    a.ticks_used as u32,
                );

                agents.destroy(i);
            }
        }
    }

    // ── Phase 5: Intent outcome recording ────────────────────────────

    fn record_intent_outcomes(
        &mut self,
        intents: &mut IntentEngine,
        memory: &mut MemoryEngine,
        tick: u64,
    ) {
        for i in 0..MAX_INTENTS {
            match intents.intents[i].status {
                IntentStatus::Fulfilled => {
                    self.stats.intents_fulfilled += 1;
                    let id = intents.intents[i].id;
                    let desc = &intents.intents[i].description[..intents.intents[i].desc_len];
                    let duration = tick.saturating_sub(intents.intents[i].submit_tick);

                    memory.episodic.record(
                        tick,
                        EpisodeKind::IntentFulfilled,
                        EpisodeOutcome::Success,
                        u16::MAX,
                        id,
                        desc,
                        duration as u32,
                    );

                    // Store in persistent memory for future planning.
                    memory.persistent.store(
                        desc,
                        &(duration as u32).to_le_bytes(),
                        MemoryTag::Cache,
                        MemoryScope::Global,
                        0,
                        tick,
                        220,
                    );

                    // Free the intent slot.
                    intents.intents[i] = crate::intent::IntentDescriptor::empty();
                }
                IntentStatus::Failed => {
                    self.stats.intents_failed += 1;
                    let id = intents.intents[i].id;
                    let desc = &intents.intents[i].description[..intents.intents[i].desc_len];

                    memory.episodic.record(
                        tick,
                        EpisodeKind::IntentFailed,
                        EpisodeOutcome::Failure,
                        u16::MAX,
                        id,
                        desc,
                        0,
                    );

                    intents.intents[i] = crate::intent::IntentDescriptor::empty();
                }
                IntentStatus::Cancelled => {
                    intents.intents[i] = crate::intent::IntentDescriptor::empty();
                }
                _ => {}
            }
        }
    }
}

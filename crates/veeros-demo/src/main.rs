//! VeerOS host-runnable demo.
//!
//! This binary boots VeerOS on the host machine by emulating the serial
//! console over stdin / stdout.  It runs the full boot banner sequence
//! followed by the interactive VeerOS shell with **live AI-native
//! subsystems** (agents, intents, memory, fabric).
//!
//! ```
//! cargo run -p veeros-demo
//! ```

use std::io::{self, Read, Write as IoWrite};
use std::os::unix::io::AsRawFd;

use arch::{Console, Serial};
use core::fmt::Write;
use shell::{Shell, ShellEnv};

use microkernel::agent::{AgentState, AgentTable, Goal, GoalPriority};
use microkernel::fabric::{
    ExecutionFabric, LocalityZone, NodeArch, NodeCapability, NodeHealth, NodeResources,
    PlacementConstraint,
};
use microkernel::intent::{IntentClass, IntentEngine, IntentStatus};
use microkernel::intent_sched::IntentScheduler;
use microkernel::memory_engine::{MemoryEngine, MemoryScope, MemoryTag};

#[cfg(feature = "quic-demo")]
use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
#[cfg(feature = "quic-demo")]
use rcgen::generate_simple_self_signed;
#[cfg(feature = "quic-demo")]
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
#[cfg(feature = "quic-demo")]
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
#[cfg(feature = "quic-demo")]
use std::sync::Arc;
#[cfg(feature = "quic-demo")]
use veer_fabric_transport::quic_backend::{HostMeshAdapter, QuicFabricRuntime, QuicMeshBridge, MESH_NODE_ID_LEN};

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ═══════════════════════════════════════════════════════════════════════════
// Terminal raw mode (POSIX)
// ═══════════════════════════════════════════════════════════════════════════

/// Saved original terminal settings so we can restore on exit.
static mut ORIG_TERMIOS: Option<libc::termios> = None;

/// Put stdin into raw mode and save the original settings.
fn enable_raw_mode() {
    unsafe {
        let mut orig: libc::termios = std::mem::zeroed();
        libc::tcgetattr(libc::STDIN_FILENO, &mut orig);
        ORIG_TERMIOS = Some(orig);

        let mut raw = orig;
        libc::cfmakeraw(&mut raw);
        // Keep ISIG so Ctrl-C can still generate SIGINT if needed,
        // but we handle Ctrl-C in the shell itself.
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::IEXTEN);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VMIN] = 1; // read returns after 1 byte
        raw.c_cc[libc::VTIME] = 0; // no timeout
        libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
    }
}

/// Restore the original terminal settings.
fn disable_raw_mode() {
    unsafe {
        if let Some(ref orig) = ORIG_TERMIOS {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, orig);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// AI-native kernel subsystem statics
// ═══════════════════════════════════════════════════════════════════════════

static mut AGENTS: AgentTable = AgentTable::new();
static mut INTENTS: IntentEngine = IntentEngine::new();
static mut MEMORY: MemoryEngine = MemoryEngine::new();
static mut FABRIC: ExecutionFabric = ExecutionFabric::new();
static mut SCHED: IntentScheduler = IntentScheduler::new();
/// Monotonic tick counter (incremented on each intent/agent operation).
static mut TICK: u64 = 1;

const TRACE_CAPACITY: usize = 32;
const TRACE_ACTION_LEN: usize = 16;
const TRACE_REASON_LEN: usize = 80;

#[derive(Clone, Copy)]
struct DecisionTraceEvent {
    used: bool,
    tick: u64,
    intent_id: u16,
    selected_node: usize,
    required_cap: Option<NodeCapability>,
    action: [u8; TRACE_ACTION_LEN],
    action_len: usize,
    reason: [u8; TRACE_REASON_LEN],
    reason_len: usize,
}

impl DecisionTraceEvent {
    const fn empty() -> Self {
        Self {
            used: false,
            tick: 0,
            intent_id: u16::MAX,
            selected_node: usize::MAX,
            required_cap: None,
            action: [0; TRACE_ACTION_LEN],
            action_len: 0,
            reason: [0; TRACE_REASON_LEN],
            reason_len: 0,
        }
    }
}

static mut DECISION_TRACE: [DecisionTraceEvent; TRACE_CAPACITY] =
    [DecisionTraceEvent::empty(); TRACE_CAPACITY];
static mut DECISION_TRACE_NEXT: usize = 0;
static mut DECISION_TRACE_COUNT: usize = 0;

fn next_tick() -> u64 {
    unsafe {
        TICK += 1;
        TICK
    }
}

fn cap_label(cap: Option<NodeCapability>) -> &'static str {
    match cap {
        Some(NodeCapability::Compute) => "compute",
        Some(NodeCapability::GpuCompute) => "gpu_compute",
        Some(NodeCapability::NpuInference) => "npu_inference",
        Some(NodeCapability::FpgaLogic) => "fpga_logic",
        Some(NodeCapability::BlockStorage) => "block_storage",
        Some(NodeCapability::Network) => "network",
        Some(NodeCapability::Radio) => "radio",
        Some(NodeCapability::Sensors) => "sensors",
        Some(NodeCapability::Display) => "display",
        Some(NodeCapability::UsbHost) => "usb_host",
        Some(NodeCapability::CryptoAccel) => "crypto_accel",
        Some(NodeCapability::Quantum) => "quantum",
        Some(NodeCapability::Realtime) => "realtime",
        Some(NodeCapability::LowPower) => "low_power",
        Some(NodeCapability::CloudApi) => "cloud_api",
        Some(NodeCapability::ModelInference) => "model_inference",
        None => "none",
    }
}

fn copy_limited(dst: &mut [u8], src: &str) -> usize {
    let n = src.len().min(dst.len());
    dst[..n].copy_from_slice(&src.as_bytes()[..n]);
    n
}

fn push_decision_trace(
    action: &str,
    intent_id: u16,
    selected_node: usize,
    required_cap: Option<NodeCapability>,
    reason: &str,
) {
    unsafe {
        let idx = DECISION_TRACE_NEXT;
        DECISION_TRACE[idx] = DecisionTraceEvent::empty();
        DECISION_TRACE[idx].used = true;
        DECISION_TRACE[idx].tick = next_tick();
        DECISION_TRACE[idx].intent_id = intent_id;
        DECISION_TRACE[idx].selected_node = selected_node;
        DECISION_TRACE[idx].required_cap = required_cap;
        DECISION_TRACE[idx].action_len = copy_limited(&mut DECISION_TRACE[idx].action, action);
        DECISION_TRACE[idx].reason_len = copy_limited(&mut DECISION_TRACE[idx].reason, reason);

        DECISION_TRACE_NEXT = (DECISION_TRACE_NEXT + 1) % TRACE_CAPACITY;
        if DECISION_TRACE_COUNT < TRACE_CAPACITY {
            DECISION_TRACE_COUNT += 1;
        }
    }
}

fn print_decision_trace(w: &mut dyn Write) {
    unsafe {
        if DECISION_TRACE_COUNT == 0 {
            let _ = writeln!(w, "  decision trace: empty");
            return;
        }
        let _ = writeln!(w, "  decision trace (oldest -> newest):");
        let start = (DECISION_TRACE_NEXT + TRACE_CAPACITY - DECISION_TRACE_COUNT) % TRACE_CAPACITY;
        for i in 0..DECISION_TRACE_COUNT {
            let idx = (start + i) % TRACE_CAPACITY;
            let ev = DECISION_TRACE[idx];
            if !ev.used {
                continue;
            }

            let action = core::str::from_utf8(&ev.action[..ev.action_len]).unwrap_or("?");
            let reason = core::str::from_utf8(&ev.reason[..ev.reason_len]).unwrap_or("?");
            let cap = cap_label(ev.required_cap);
            if ev.selected_node == usize::MAX {
                let _ = writeln!(
                    w,
                    "    t={} action={} intent=#{} cap={} node=none reason={}",
                    ev.tick, action, ev.intent_id, cap, reason
                );
            } else {
                let name = if ev.selected_node < FABRIC.nodes.len()
                    && FABRIC.nodes[ev.selected_node].health != NodeHealth::Unused
                {
                    core::str::from_utf8(
                        &FABRIC.nodes[ev.selected_node].name[..FABRIC.nodes[ev.selected_node].name_len],
                    )
                    .unwrap_or("?")
                } else {
                    "stale"
                };
                let _ = writeln!(
                    w,
                    "    t={} action={} intent=#{} cap={} node=#{} ({}) reason={}",
                    ev.tick, action, ev.intent_id, cap, ev.selected_node, name, reason
                );
            }
        }
    }
}

fn clear_decision_trace() {
    unsafe {
        DECISION_TRACE = [DecisionTraceEvent::empty(); TRACE_CAPACITY];
        DECISION_TRACE_NEXT = 0;
        DECISION_TRACE_COUNT = 0;
    }
}

#[cfg(feature = "quic-demo")]
async fn quic_smoke_async() -> Result<(), String> {
    let key = [0x77u8; 32];
    let server_node = [0xB1u8; MESH_NODE_ID_LEN];
    let client_node = [0x12u8; MESH_NODE_ID_LEN];

    let cert = generate_simple_self_signed(vec!["localhost".into()])
        .map_err(|e| format!("cert gen failed: {e}"))?;
    let cert_der: CertificateDer<'static> = CertificateDer::from(
        cert.serialize_der()
            .map_err(|e| format!("cert serialize failed: {e}"))?,
    );
    let key_der = PrivatePkcs8KeyDer::from(cert.serialize_private_key_der());

    let mut server_config = ServerConfig::with_single_cert(vec![cert_der.clone()], key_der.into())
        .map_err(|e| format!("server config failed: {e}"))?;
    server_config.transport_config(Arc::new(TransportConfig::default()));

    let server_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
    let server_endpoint =
        Endpoint::server(server_config, server_addr).map_err(|e| format!("server endpoint failed: {e}"))?;
    let bound_addr = server_endpoint
        .local_addr()
        .map_err(|e| format!("server local addr failed: {e}"))?;

    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(cert_der)
        .map_err(|e| format!("root add failed: {e}"))?;
    let client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let mut client_endpoint = Endpoint::client(SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::LOCALHOST,
        0,
    )))
    .map_err(|e| format!("client endpoint failed: {e}"))?;
    let client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)
            .map_err(|e| format!("client quic config failed: {e}"))?,
    ));
    client_endpoint.set_default_client_config(client_config);

    let server_task = tokio::spawn(async move {
        let incoming = server_endpoint
            .accept()
            .await
            .ok_or_else(|| "server accept returned none".to_string())?;
        let connection = incoming
            .await
            .map_err(|e| format!("server handshake failed: {e}"))?;
        let runtime = QuicFabricRuntime::new(connection, &key)
            .map_err(|e| format!("server runtime failed: {e:?}"))?;
        let stream = runtime
            .accept_quic_bi()
            .await
            .map_err(|e| format!("server accept_bi failed: {e:?}"))?;
        let bridge = runtime.into_bridge(stream, 64 * 1024);
        let mut adapter = HostMeshAdapter::new(bridge, server_node);
        let env = adapter
            .recv_for_local()
            .await
            .map_err(|e| format!("server recv failed: {e:?}"))?
            .ok_or_else(|| "server got eof".to_string())?;
        if env.payload != b"veeros-quic-smoke" {
            return Err("unexpected payload".to_string());
        }
        Ok::<(), String>(())
    });

    let client_conn = client_endpoint
        .connect(bound_addr, "localhost")
        .map_err(|e| format!("client connect init failed: {e}"))?
        .await
        .map_err(|e| format!("client connect failed: {e}"))?;
    let bridge = QuicMeshBridge::from_connection(client_conn, &key, 64 * 1024)
        .await
        .map_err(|e| format!("client bridge failed: {e:?}"))?;
    let mut adapter = HostMeshAdapter::new(bridge, client_node);
    adapter
        .send(&server_node, b"veeros-quic-smoke")
        .await
        .map_err(|e| format!("client send failed: {e:?}"))?;
    adapter
        .finish()
        .await
        .map_err(|e| format!("client finish failed: {e:?}"))?;

    server_task
        .await
        .map_err(|e| format!("server task failed: {e}"))??;
    Ok(())
}

#[cfg(feature = "quic-demo")]
fn run_quic_smoke() -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime failed: {e}"))?;
    rt.block_on(quic_smoke_async())
}

// ─── Agent callbacks ────────────────────────────────────────────────────

fn do_get_agent_list(w: &mut dyn Write) {
    unsafe {
        let count = AGENTS.active_count();
        let _ = writeln!(w, "  Active agents: {count}/32");
        let _ = writeln!(w, "  {:>3}  {:>10}  {:>8}  Goal", "ID", "State", "Ticks");
        let _ = writeln!(w, "  ---  ----------  --------  ----");
        for (i, a) in AGENTS.agents.iter().enumerate() {
            if a.state == AgentState::Free {
                continue;
            }
            let state = match a.state {
                AgentState::Free => "free",
                AgentState::Spawned => "spawned",
                AgentState::Planning => "planning",
                AgentState::Executing => "executing",
                AgentState::Blocked => "blocked",
                AgentState::Completed => "completed",
                AgentState::Failed => "failed",
            };
            let desc = a.goal.desc_bytes();
            let goal_str = core::str::from_utf8(desc).unwrap_or("?");
            let _ = writeln!(
                w,
                "  {:>3}  {:>10}  {:>8}  {}",
                i, state, a.ticks_used, goal_str
            );
        }
    }
}

fn do_agent_cmd(sub: &str, rest: &str, w: &mut dyn Write) {
    unsafe {
        match sub {
            "spawn" => {
                if rest.is_empty() {
                    let _ = writeln!(w, "  usage: agents spawn <goal description>");
                    return;
                }
                let goal = Goal::from_bytes(rest.as_bytes(), GoalPriority::Normal);
                let tick = next_tick();
                match AGENTS.spawn(goal, 0, usize::MAX, tick) {
                    Some(id) => {
                        AGENTS.transition(id, AgentState::Planning);
                        let _ = writeln!(w, "  agent #{id} spawned (planning)");
                    }
                    None => {
                        let _ = writeln!(w, "  error: agent table full");
                    }
                }
            }
            "status" => {
                let id = rest.parse::<usize>().unwrap_or(usize::MAX);
                if id >= 32 || AGENTS.agents[id].state == AgentState::Free {
                    let _ = writeln!(w, "  error: invalid agent id");
                    return;
                }
                let a = &AGENTS.agents[id];
                let desc = core::str::from_utf8(a.goal.desc_bytes()).unwrap_or("?");
                let _ = writeln!(w, "  Agent #{id}");
                let _ = writeln!(w, "    State     : {:?}", a.state);
                let _ = writeln!(w, "    Goal      : {desc}");
                let _ = writeln!(w, "    Ticks used: {}", a.ticks_used);
                let _ = writeln!(w, "    Children  : {}", a.child_count);
                let _ = writeln!(w, "    Replans   : {}/{}", a.replan_count, a.max_replans);
            }
            "execute" => {
                let id = rest.parse::<usize>().unwrap_or(usize::MAX);
                if id >= 32 || AGENTS.agents[id].state == AgentState::Free {
                    let _ = writeln!(w, "  error: invalid agent id");
                    return;
                }
                AGENTS.transition(id, AgentState::Executing);
                let _ = writeln!(w, "  agent #{id} → executing");
            }
            "complete" => {
                let id = rest.parse::<usize>().unwrap_or(usize::MAX);
                if id >= 32 {
                    let _ = writeln!(w, "  error: invalid agent id");
                    return;
                }
                if AGENTS.complete(id) {
                    let _ = writeln!(w, "  agent #{id} → completed");
                } else {
                    let _ = writeln!(w, "  error: cannot complete agent #{id}");
                }
            }
            "kill" => {
                let id = rest.parse::<usize>().unwrap_or(usize::MAX);
                if id >= 32 {
                    let _ = writeln!(w, "  error: invalid agent id");
                    return;
                }
                AGENTS.destroy(id);
                let _ = writeln!(w, "  agent #{id} destroyed");
            }
            "count" => {
                let _ = writeln!(w, "  active agents: {}", AGENTS.active_count());
            }
            _ => {
                let _ = writeln!(
                    w,
                    "  usage: agents [list|spawn|status|execute|complete|kill|count] <args>"
                );
            }
        }
    }
}

// ─── Intent callbacks ───────────────────────────────────────────────────

fn do_get_intent_list(w: &mut dyn Write) {
    unsafe {
        let count = INTENTS.active_count();
        let _ = writeln!(w, "  Active intents: {count}/16");
        let _ = writeln!(
            w,
            "  {:>3}  {:>10}  {:>10}  Description",
            "ID", "Class", "Status"
        );
        let _ = writeln!(w, "  ---  ----------  ----------  -----------");
        for slot in INTENTS.intents.iter() {
            if slot.status == IntentStatus::Free {
                continue;
            }
            let class = match slot.class {
                IntentClass::Compute => "compute",
                IntentClass::Deploy => "deploy",
                IntentClass::Monitor => "monitor",
                IntentClass::Communicate => "comm",
                IntentClass::Data => "data",
                IntentClass::Admin => "admin",
                IntentClass::Pipeline => "pipeline",
                IntentClass::Custom => "custom",
            };
            let status = match slot.status {
                IntentStatus::Free => "free",
                IntentStatus::Pending => "pending",
                IntentStatus::Planning => "planning",
                IntentStatus::Active => "active",
                IntentStatus::Fulfilled => "fulfilled",
                IntentStatus::Failed => "failed",
                IntentStatus::Cancelled => "cancelled",
            };
            let desc = core::str::from_utf8(&slot.description[..slot.desc_len]).unwrap_or("?");
            let _ = writeln!(
                w,
                "  {:>3}  {:>10}  {:>10}  {}",
                slot.id, class, status, desc
            );
        }
    }
}

fn build_demo_constraint(class: IntentClass, desc: &str) -> PlacementConstraint {
    let mut constraint = PlacementConstraint::any();
    constraint.required_cap = match class {
        IntentClass::Compute | IntentClass::Deploy | IntentClass::Pipeline | IntentClass::Admin => {
            Some(NodeCapability::Compute)
        }
        IntentClass::Monitor | IntentClass::Communicate => Some(NodeCapability::Network),
        IntentClass::Data => Some(NodeCapability::BlockStorage),
        IntentClass::Custom => None,
    };
    if desc.contains("inference") || desc.contains("model") {
        constraint.required_cap = Some(NodeCapability::ModelInference);
    } else if desc.contains("sensor") {
        constraint.required_cap = Some(NodeCapability::Sensors);
    } else if desc.contains("gpu") {
        constraint.required_cap = Some(NodeCapability::GpuCompute);
    }
    if desc.contains("latency") || desc.contains("realtime") {
        constraint.max_rtt_us = 2_000;
    }
    constraint
}

fn parse_intent_class(s: &str) -> Option<IntentClass> {
    match s {
        "compute" => Some(IntentClass::Compute),
        "deploy" => Some(IntentClass::Deploy),
        "monitor" => Some(IntentClass::Monitor),
        "comm" | "communicate" => Some(IntentClass::Communicate),
        "data" => Some(IntentClass::Data),
        "admin" => Some(IntentClass::Admin),
        "pipeline" => Some(IntentClass::Pipeline),
        "custom" => Some(IntentClass::Custom),
        _ => None,
    }
}

fn policy_weights(policy: &str) -> Option<(u32, u32, u32, u32)> {
    match policy {
        "balanced" => Some((30, 25, 25, 20)),
        "latency-first" => Some((50, 20, 20, 10)),
        "trust-first" => Some((15, 20, 55, 10)),
        "cost-first" => Some((15, 15, 15, 55)),
        _ => None,
    }
}

fn policy_score_for_node(
    node: &microkernel::fabric::FabricNode,
    required_cap: Option<NodeCapability>,
    policy: &str,
) -> Option<u32> {
    if node.health == NodeHealth::Unused || node.health == NodeHealth::Offline {
        return None;
    }
    if let Some(cap) = required_cap {
        if !node.has_capability(cap) {
            return None;
        }
    }

    let (w_lat, w_load, w_trust, w_cost) = policy_weights(policy)?;

    let latency = if node.rtt_us == 0 {
        1000
    } else {
        let capped = node.rtt_us.min(20_000);
        1000u32.saturating_sub((capped * 1000) / 20_000)
    };
    let load = 1000u32.saturating_sub((node.resources.cpu_load as u32 * 1000) / 255);
    let base_trust: u32 = match node.health {
        NodeHealth::Healthy => 1000,
        NodeHealth::Overloaded => 650,
        NodeHealth::Degraded => 400,
        _ => 0,
    };
    let trust = if required_cap.is_some() {
        base_trust.saturating_add(80).min(1000)
    } else {
        base_trust
    };
    let cost = match node.zone {
        LocalityZone::Local => 900,
        LocalityZone::Rack => 700,
        LocalityZone::DataCenter => 500,
        LocalityZone::Region => 300,
        LocalityZone::Global => 150,
    };

    let weighted = latency * w_lat + load * w_load + trust * w_trust + cost * w_cost;
    Some(weighted / 100)
}

fn run_policy_simulation(policy: &str, class: IntentClass, desc: &str, w: &mut dyn Write) {
    let constraint = build_demo_constraint(class, desc);
    let mut ranked: Vec<(usize, u32)> = Vec::new();

    unsafe {
        for (i, node) in FABRIC.nodes.iter().enumerate() {
            if let Some(score) = policy_score_for_node(node, constraint.required_cap, policy) {
                ranked.push((i, score));
            }
        }
    }

    ranked.sort_by(|a, b| b.1.cmp(&a.1));

    let _ = writeln!(w, "  simulate policy={policy} class={:?}", class);
    let _ = writeln!(w, "  required capability: {}", cap_label(constraint.required_cap));
    if ranked.is_empty() {
        let _ = writeln!(w, "  no eligible nodes for this simulation");
        push_decision_trace("simulate", u16::MAX, usize::MAX, constraint.required_cap, "no_eligible_node");
        return;
    }

    let _ = writeln!(w, "  ranked candidates:");
    for (rank, (idx, score)) in ranked.iter().take(3).enumerate() {
        unsafe {
            let node = &FABRIC.nodes[*idx];
            let name = core::str::from_utf8(&node.name[..node.name_len]).unwrap_or("?");
            let _ = writeln!(w, "    {}. #{} ({}) score={}", rank + 1, idx, name, score);
        }
    }

    let winner = ranked[0].0;
    unsafe {
        let name = core::str::from_utf8(&FABRIC.nodes[winner].name[..FABRIC.nodes[winner].name_len]).unwrap_or("?");
        let _ = writeln!(w, "  winner: #{} ({})", winner, name);
    }
    push_decision_trace("simulate", u16::MAX, winner, constraint.required_cap, policy);
}

fn record_scheduler_decision(
    intent_id: u16,
    selected: Option<usize>,
    required_cap: Option<NodeCapability>,
    reason: &'static str,
    action: &'static str,
) {
    unsafe {
        SCHED.stats.last_required_cap = required_cap;
        if let Some(idx) = selected {
            SCHED.stats.last_selected_node = idx;
        } else {
            SCHED.stats.last_selected_node = usize::MAX;
        }
        SCHED.stats.last_decision_reason = reason;
    }
    push_decision_trace(
        action,
        intent_id,
        selected.unwrap_or(usize::MAX),
        required_cap,
        reason,
    );
}

fn trigger_autonomous_replan(w: &mut dyn Write, trigger: &'static str) {
    unsafe {
        let mut replanned = 0usize;
        for slot in INTENTS.intents.iter() {
            if !(slot.status == IntentStatus::Planning || slot.status == IntentStatus::Active) {
                continue;
            }
            let desc = core::str::from_utf8(&slot.description[..slot.desc_len]).unwrap_or("");
            let constraint = build_demo_constraint(slot.class, desc);
            let selected = FABRIC.select_node(&constraint);
            record_scheduler_decision(
                slot.id,
                selected,
                constraint.required_cap,
                "autonomous_replan_after_fault",
                "replan",
            );

            if let Some(idx) = selected {
                replanned += 1;
                let name = core::str::from_utf8(&FABRIC.nodes[idx].name[..FABRIC.nodes[idx].name_len])
                    .unwrap_or("?");
                let _ = writeln!(w, "  replan intent #{} -> node #{} ({})", slot.id, idx, name);
            } else {
                let _ = writeln!(w, "  replan intent #{} -> no eligible node", slot.id);
            }
        }

        if replanned == 0 {
            let _ = writeln!(w, "  autonomous replan: no active intents");
        }

        let _ = MEMORY.persistent.store(
            b"world.last_replan",
            trigger.as_bytes(),
            MemoryTag::Observation,
            MemoryScope::Global,
            0,
            next_tick(),
            220,
        );
    }
}

fn do_intent_cmd(sub: &str, rest: &str, w: &mut dyn Write) {
    unsafe {
        match sub {
            "submit" => {
                // intent submit <class> <description>
                let (class_str, desc) = match rest.find(' ') {
                    Some(i) => (rest[..i].trim(), rest[i + 1..].trim()),
                    None => (rest.trim(), ""),
                };
                let class = match class_str {
                    "compute" => IntentClass::Compute,
                    "deploy" => IntentClass::Deploy,
                    "monitor" => IntentClass::Monitor,
                    "comm" | "communicate" => IntentClass::Communicate,
                    "data" => IntentClass::Data,
                    "admin" => IntentClass::Admin,
                    "pipeline" => IntentClass::Pipeline,
                    "custom" => IntentClass::Custom,
                    _ => {
                        let _ = writeln!(w, "  unknown class: {class_str}");
                        let _ = writeln!(
                            w,
                            "  classes: compute deploy monitor comm data admin pipeline custom"
                        );
                        return;
                    }
                };
                if desc.is_empty() {
                    let _ = writeln!(w, "  usage: intent submit <class> <description>");
                    return;
                }
                let tick = next_tick();
                match INTENTS.submit(class, desc.as_bytes(), GoalPriority::Normal, 0, tick) {
                    Some(id) => {
                        let constraint = build_demo_constraint(class, desc);
                        let selected = FABRIC.select_node(&constraint);
                        record_scheduler_decision(
                            id,
                            selected,
                            constraint.required_cap,
                            if selected.is_some() {
                                "best_score_under_constraints"
                            } else {
                                "no_eligible_node_fallback_local"
                            },
                            "plan",
                        );

                        // auto-decompose
                        INTENTS.decompose(id);
                        let _ = writeln!(w, "  intent #{id} submitted (class={class_str})");
                        // show the plan steps
                        let idx = INTENTS.intents.iter().position(|s| s.id == id);
                        if let Some(idx) = idx {
                            let plan = &INTENTS.plans[idx];
                            let _ =
                                writeln!(w, "  plan decomposed into {} step(s):", plan.step_count);
                            for s in 0..plan.step_count {
                                let step = &plan.steps[s];
                                let g = core::str::from_utf8(step.goal.desc_bytes()).unwrap_or("?");
                                let rel = match step.relation {
                                    microkernel::intent::StepRelation::Independent => "independent",
                                    microkernel::intent::StepRelation::DependsOn => "depends-on",
                                    microkernel::intent::StepRelation::Parallel => "parallel",
                                };
                                let _ = writeln!(w, "    step {s}: {g} ({rel})");
                            }
                        }
                    }
                    None => {
                        let _ = writeln!(w, "  error: intent table full");
                    }
                }
            }
            "status" => {
                let id = rest.parse::<u16>().unwrap_or(0);
                match INTENTS.status(id) {
                    Some(s) => {
                        let _ = writeln!(w, "  intent #{id}: {:?}", s);
                    }
                    None => {
                        let _ = writeln!(w, "  error: no intent with id {id}");
                    }
                }
            }
            "cancel" => {
                let id = rest.parse::<u16>().unwrap_or(0);
                if INTENTS.cancel(id) {
                    let _ = writeln!(w, "  intent #{id} cancelled");
                } else {
                    let _ = writeln!(w, "  error: cannot cancel intent #{id}");
                }
            }
            "stats" => {
                let s = &SCHED.stats;
                let _ = writeln!(w, "  Intent Scheduler Statistics");
                let _ = writeln!(w, "    intents submitted : {}", s.intents_submitted);
                let _ = writeln!(w, "    intents fulfilled : {}", s.intents_fulfilled);
                let _ = writeln!(w, "    intents failed    : {}", s.intents_failed);
                let _ = writeln!(w, "    agents spawned    : {}", s.agents_spawned);
                let _ = writeln!(w, "    agents completed  : {}", s.agents_completed);
                let _ = writeln!(w, "    agents failed     : {}", s.agents_failed);
                let _ = writeln!(w, "    replans           : {}", s.replans);
                let _ = writeln!(w, "    last tick          : {}", s.last_tick);
                let cap = cap_label(s.last_required_cap);
                let _ = writeln!(w, "    required capability: {cap}");
                if s.last_selected_node == usize::MAX {
                    let _ = writeln!(w, "    selected node      : none");
                } else {
                    let idx = s.last_selected_node;
                    if idx < FABRIC.nodes.len() && FABRIC.nodes[idx].health != NodeHealth::Unused {
                        let name = core::str::from_utf8(&FABRIC.nodes[idx].name[..FABRIC.nodes[idx].name_len])
                            .unwrap_or("?");
                        let _ = writeln!(w, "    selected node      : #{idx} ({name})");
                    } else {
                        let _ = writeln!(w, "    selected node      : #{idx} (stale)");
                    }
                }
                let _ = writeln!(w, "    decision reason    : {}", s.last_decision_reason);
            }
            _ => {
                let _ = writeln!(
                    w,
                    "  usage: intent [list|submit|status|cancel|stats] <args>"
                );
            }
        }
    }
}

// ─── Memory callbacks ───────────────────────────────────────────────────

fn do_memory_cmd(sub: &str, rest: &str, w: &mut dyn Write) {
    unsafe {
        match sub {
            "" | "stats" => {
                let pcount = MEMORY.persistent.count();
                let _ = writeln!(w, "  Memory Engine Status");
                let _ = writeln!(w, "    persistent entries: {pcount}");
            }
            "set" | "store" => {
                // memory set <key> <value>
                let (key, val) = match rest.find(' ') {
                    Some(i) => (rest[..i].trim(), rest[i + 1..].trim()),
                    None => {
                        let _ = writeln!(w, "  usage: memory set <key> <value>");
                        return;
                    }
                };
                let tick = next_tick();
                if MEMORY.persistent.store(
                    key.as_bytes(),
                    val.as_bytes(),
                    MemoryTag::UserKnow,
                    MemoryScope::Global,
                    0,
                    tick,
                    200,
                ) {
                    let _ = writeln!(w, "  stored: {key} = {val}");
                } else {
                    let _ = writeln!(w, "  error: persistent store full");
                }
            }
            "get" | "query" => {
                if rest.is_empty() {
                    let _ = writeln!(w, "  usage: memory get <key>");
                    return;
                }
                match MEMORY
                    .persistent
                    .query(rest.as_bytes(), MemoryScope::Global, 0)
                {
                    Some(entry) => {
                        let val =
                            core::str::from_utf8(&entry.value[..entry.value_len]).unwrap_or("?");
                        let _ = writeln!(
                            w,
                            "  {rest} = {val}  (reads={}, confidence={})",
                            entry.read_count, entry.confidence
                        );
                    }
                    None => {
                        let _ = writeln!(w, "  key not found: {rest}");
                    }
                }
            }
            "delete" | "del" => {
                if rest.is_empty() {
                    let _ = writeln!(w, "  usage: memory delete <key>");
                    return;
                }
                if MEMORY
                    .persistent
                    .delete(rest.as_bytes(), MemoryScope::Global, 0)
                {
                    let _ = writeln!(w, "  deleted: {rest}");
                } else {
                    let _ = writeln!(w, "  key not found: {rest}");
                }
            }
            _ => {
                let _ = writeln!(w, "  usage: memory [stats|set|get|delete] <args>");
            }
        }
    }
}

// ─── Fabric callback ────────────────────────────────────────────────────

fn do_get_fabric_status(w: &mut dyn Write) {
    unsafe {
        let (total, healthy) = FABRIC.node_counts();
        let _ = writeln!(w, "  Execution Fabric: {healthy}/{total} nodes healthy");
        let _ = writeln!(
            w,
            "  {:>3}  {:>8}  {:>8}  {:>6}  {:>5}  {:>5}  Name",
            "IDX", "Arch", "Zone", "Cores", "RAM", "Load%"
        );
        let _ = writeln!(w, "  ---  --------  --------  ------  -----  -----  ----");
        for (i, node) in FABRIC.nodes.iter().enumerate() {
            if node.health == microkernel::fabric::NodeHealth::Unused {
                continue;
            }
            let arch = match node.arch {
                NodeArch::Riscv32 => "rv32",
                NodeArch::Riscv64 => "rv64",
                NodeArch::Aarch64 => "arm64",
                NodeArch::X86_64 => "x86_64",
                NodeArch::Xtensa => "xtensa",
                NodeArch::Unknown => "??",
            };
            let zone = match node.zone {
                LocalityZone::Local => "local",
                LocalityZone::Rack => "rack",
                LocalityZone::DataCenter => "dc",
                LocalityZone::Region => "region",
                LocalityZone::Global => "global",
            };
            let name = core::str::from_utf8(&node.name[..node.name_len]).unwrap_or("?");
            let load_pct = (node.resources.cpu_load as u32 * 100) / 255;
            let ram_mb = node.resources.ram_kib / 1024;
            let _ = writeln!(
                w,
                "  {:>3}  {:>8}  {:>8}  {:>6}  {:>4}M  {:>4}%  {}",
                i, arch, zone, node.resources.cpu_cores, ram_mb, load_pct, name
            );
        }
    }
}

fn fnv1a64_update(mut hash: u64, data: &[u8]) -> u64 {
    const FNV_PRIME: u64 = 0x0000_0100_0000_01B3;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn world_snapshot_hash() -> (usize, u64) {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    let mut hash = FNV_OFFSET;
    let mut count = 0usize;

    unsafe {
        for node in FABRIC.nodes.iter() {
            if node.health == NodeHealth::Unused {
                continue;
            }
            count += 1;

            hash = fnv1a64_update(hash, &[node.health as u8]);
            hash = fnv1a64_update(hash, &[node.arch as u8]);
            hash = fnv1a64_update(hash, &[node.zone as u8]);
            hash = fnv1a64_update(hash, &node.capabilities.to_le_bytes());
            hash = fnv1a64_update(hash, &node.resources.cpu_cores.to_le_bytes());
            hash = fnv1a64_update(hash, &node.resources.cpu_mhz.to_le_bytes());
            hash = fnv1a64_update(hash, &node.resources.ram_kib.to_le_bytes());
            hash = fnv1a64_update(hash, &node.resources.ram_free_kib.to_le_bytes());
            hash = fnv1a64_update(hash, &node.resources.accel_count.to_le_bytes());
            hash = fnv1a64_update(hash, &node.resources.cpu_load.to_le_bytes());
            hash = fnv1a64_update(hash, &node.resources.active_agents.to_le_bytes());
            hash = fnv1a64_update(hash, &node.rtt_us.to_le_bytes());
            hash = fnv1a64_update(hash, &(node.name_len as u64).to_le_bytes());
            hash = fnv1a64_update(hash, &node.name[..node.name_len]);
        }
    }

    (count, hash)
}

fn parse_fault_health(s: &str) -> Option<NodeHealth> {
    match s {
        "healthy" => Some(NodeHealth::Healthy),
        "degraded" => Some(NodeHealth::Degraded),
        "overloaded" => Some(NodeHealth::Overloaded),
        "offline" => Some(NodeHealth::Offline),
        _ => None,
    }
}

fn find_fabric_node(selector: &str) -> Option<usize> {
    if let Ok(idx) = selector.parse::<usize>() {
        unsafe {
            if idx < FABRIC.nodes.len() && FABRIC.nodes[idx].health != NodeHealth::Unused {
                return Some(idx);
            }
        }
    }

    unsafe {
        for (i, node) in FABRIC.nodes.iter().enumerate() {
            if node.health == NodeHealth::Unused {
                continue;
            }
            let name = core::str::from_utf8(&node.name[..node.name_len]).unwrap_or("");
            if name == selector {
                return Some(i);
            }
        }
    }
    None
}

fn set_node_health_and_record(idx: usize, health: NodeHealth, w: &mut dyn Write) {
    unsafe {
        if idx >= FABRIC.nodes.len() || FABRIC.nodes[idx].health == NodeHealth::Unused {
            let _ = writeln!(w, "  error: invalid node index {idx}");
            return;
        }

        let old = FABRIC.nodes[idx].health;
        FABRIC.nodes[idx].health = health;

        let name = core::str::from_utf8(&FABRIC.nodes[idx].name[..FABRIC.nodes[idx].name_len]).unwrap_or("?");
        let _ = writeln!(w, "  fault update: node #{idx} ({name}) {:?} -> {:?}", old, health);
        push_decision_trace(
            "fault",
            u16::MAX,
            idx,
            None,
            if matches!(health, NodeHealth::Offline | NodeHealth::Degraded) {
                "fault_injected"
            } else {
                "fault_cleared"
            },
        );

        if matches!(health, NodeHealth::Offline | NodeHealth::Degraded) {
            SCHED.stats.replans = SCHED.stats.replans.saturating_add(1);
            SCHED.stats.last_decision_reason = "fault_injection_replan";
            let _ = writeln!(w, "  replan signal: triggered (scheduler replans = {})", SCHED.stats.replans);
            let _ = MEMORY.persistent.store(
                b"world.last_replan",
                b"fault-injection",
                MemoryTag::Observation,
                MemoryScope::Global,
                0,
                next_tick(),
                220,
            );
            trigger_autonomous_replan(w, "fault-injection");
        }
    }
}

fn do_mesh_cmd(sub: &str, _rest: &str, w: &mut dyn Write) {
    match sub {
        "" | "status" => {
            #[cfg(feature = "quic-demo")]
            {
                let _ = writeln!(w, "  mesh transport: host QUIC bridge smoke available");
                let _ = writeln!(w, "  usage: mesh quic-smoke");
            }
            #[cfg(not(feature = "quic-demo"))]
            {
                let _ = writeln!(
                    w,
                    "  mesh transport: QUIC smoke disabled (build with --features quic-demo)"
                );
            }
        }
        "quic-smoke" => {
            #[cfg(feature = "quic-demo")]
            {
                let _ = writeln!(w, "  running QUIC mesh bridge smoke...");
                match run_quic_smoke() {
                    Ok(()) => {
                        let _ = writeln!(w, "  quic smoke: ok");
                    }
                    Err(e) => {
                        let _ = writeln!(w, "  quic smoke: failed ({e})");
                    }
                }
            }
            #[cfg(not(feature = "quic-demo"))]
            {
                let _ = writeln!(
                    w,
                    "  quic smoke unavailable (rebuild with --features quic-demo)"
                );
            }
        }
        "fault" => {
            let rest = _rest.trim();
            let mut parts = rest.split_whitespace();
            let action = parts.next().unwrap_or("");
            match action {
                "status" | "list" => {
                    let _ = writeln!(w, "  fault table (fabric health):");
                    unsafe {
                        for (i, node) in FABRIC.nodes.iter().enumerate() {
                            if node.health == NodeHealth::Unused {
                                continue;
                            }
                            let name =
                                core::str::from_utf8(&node.name[..node.name_len]).unwrap_or("?");
                            let _ = writeln!(w, "    #{i:<2} {:<18} {:?}", name, node.health);
                        }
                    }
                }
                "inject" => {
                    let selector = parts.next().unwrap_or("");
                    let health_str = parts.next().unwrap_or("offline");
                    if selector.is_empty() {
                        let _ = writeln!(
                            w,
                            "  usage: fabric fault inject <node_idx|node_name> [offline|degraded|overloaded|healthy]"
                        );
                        return;
                    }
                    let Some(idx) = find_fabric_node(selector) else {
                        let _ = writeln!(w, "  error: unknown node '{selector}'");
                        return;
                    };
                    let Some(health) = parse_fault_health(health_str) else {
                        let _ = writeln!(w, "  error: invalid health '{health_str}'");
                        return;
                    };
                    set_node_health_and_record(idx, health, w);
                }
                "clear" => {
                    let selector = parts.next().unwrap_or("");
                    if selector.is_empty() {
                        let _ = writeln!(w, "  usage: fabric fault clear <node_idx|node_name>");
                        return;
                    }
                    let Some(idx) = find_fabric_node(selector) else {
                        let _ = writeln!(w, "  error: unknown node '{selector}'");
                        return;
                    };
                    set_node_health_and_record(idx, NodeHealth::Healthy, w);
                }
                _ => {
                    let _ = writeln!(w, "  usage: fabric fault [status|inject|clear] ...");
                    let _ = writeln!(w, "    inject: fabric fault inject <node_idx|node_name> [offline|degraded|overloaded|healthy]");
                    let _ = writeln!(w, "    clear : fabric fault clear <node_idx|node_name>");
                }
            }
        }
        "snapshot" => {
            let (count, hash) = world_snapshot_hash();
            let _ = writeln!(w, "  world snapshot: nodes={count} hash=0x{hash:016x}");
            push_decision_trace("snapshot", u16::MAX, usize::MAX, None, "world_state_hash");

            let hash_str = format!("0x{hash:016x}");
            unsafe {
                let _ = MEMORY.persistent.store(
                    b"world.snapshot.hash",
                    hash_str.as_bytes(),
                    MemoryTag::Observation,
                    MemoryScope::Global,
                    0,
                    next_tick(),
                    240,
                );
                let _ = MEMORY.persistent.store(
                    b"world.snapshot.node_count",
                    count.to_string().as_bytes(),
                    MemoryTag::Observation,
                    MemoryScope::Global,
                    0,
                    next_tick(),
                    240,
                );
            }
        }
        "simulate" => {
            let rest = _rest.trim();
            let mut parts = rest.split_whitespace();
            let policy = parts.next().unwrap_or("balanced");
            let class_s = parts.next().unwrap_or("");
            let desc = parts.collect::<Vec<&str>>().join(" ");

            if policy_weights(policy).is_none() {
                let _ = writeln!(w, "  error: unknown policy '{policy}'");
                let _ = writeln!(w, "  policies: balanced | latency-first | trust-first | cost-first");
                return;
            }
            let Some(class) = parse_intent_class(class_s) else {
                let _ = writeln!(w, "  usage: fabric simulate <policy> <class> <description>");
                let _ = writeln!(w, "  classes: compute deploy monitor comm data admin pipeline custom");
                return;
            };
            if desc.is_empty() {
                let _ = writeln!(w, "  usage: fabric simulate <policy> <class> <description>");
                return;
            }

            run_policy_simulation(policy, class, &desc, w);
        }
        "trace" => {
            let rest = _rest.trim();
            let mut parts = rest.split_whitespace();
            let action = parts.next().unwrap_or("list");
            match action {
                "" | "list" => print_decision_trace(w),
                "clear" => {
                    clear_decision_trace();
                    let _ = writeln!(w, "  decision trace: cleared");
                }
                _ => {
                    let _ = writeln!(w, "  usage: fabric trace [list|clear]");
                }
            }
        }
        _ => {
            let _ = writeln!(w, "  usage: mesh [status|quic-smoke|snapshot|simulate|fault|trace]");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Serial backend over host stdin / stdout
// ═══════════════════════════════════════════════════════════════════════════

/// Host serial implementation — stdin for RX, stdout for TX.
struct StdSerial;

impl Serial for StdSerial {
    fn write_byte(&self, byte: u8) {
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        if byte == b'\n' {
            // In raw mode bare LF doesn't return the cursor.
            let _ = lock.write_all(b"\r\n");
        } else {
            let _ = lock.write_all(&[byte]);
        }
        let _ = lock.flush();
    }

    fn read_byte(&self) -> u8 {
        let mut buf = [0u8; 1];
        let _ = io::stdin().lock().read_exact(&mut buf);
        buf[0]
    }

    fn has_data(&self) -> bool {
        let fd = io::stdin().as_raw_fd();
        let mut count: libc::c_int = 0;
        unsafe {
            libc::ioctl(fd, libc::FIONREAD, &mut count);
        }
        count > 0
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// main — simulated boot sequence → shell
// ═══════════════════════════════════════════════════════════════════════════

fn main() {
    // Raw terminal so the shell gets byte-at-a-time input.
    enable_raw_mode();

    // Make sure we restore the terminal no matter how we exit.
    // (Ctrl-D / `exit` / panic)
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            disable_raw_mode();
        }
    }
    let _guard = Guard;

    // ── Pre-populate the execution fabric with demo nodes ────────────
    unsafe {
        // Local node — the host machine
        let local = FABRIC.register_local(
            b"host-demo",
            NodeArch::X86_64,
            NodeResources {
                cpu_cores: 4,
                cpu_mhz: 3600,
                ram_kib: 16 * 1024 * 1024, // 16 GiB
                ram_free_kib: 12 * 1024 * 1024,
                accel_count: 0,
                cpu_load: 25,
                active_agents: 0,
            },
        );
        FABRIC.nodes[local].add_capability(NodeCapability::Compute);
        FABRIC.nodes[local].add_capability(NodeCapability::Network);
        FABRIC.nodes[local].add_capability(NodeCapability::CryptoAccel);

        // Simulated remote: a Raspberry Pi 5 edge node
        if let Some(rpi) = FABRIC.register_remote(
            b"rpi5-edge-01",
            NodeArch::Aarch64,
            LocalityZone::Rack,
            (1 << NodeCapability::Compute as u8)
                | (1 << NodeCapability::Sensors as u8)
                | (1 << NodeCapability::LowPower as u8),
            NodeResources {
                cpu_cores: 4,
                cpu_mhz: 2400,
                ram_kib: 8 * 1024 * 1024,
                ram_free_kib: 6 * 1024 * 1024,
                accel_count: 0,
                cpu_load: 10,
                active_agents: 0,
            },
            800, // 0.8ms RTT
            1,
        ) {
            let _ = rpi; // registered
        }

        // Simulated remote: ESP32-C6 sensor node
        if let Some(esp) = FABRIC.register_remote(
            b"esp32c6-sensor",
            NodeArch::Riscv32,
            LocalityZone::Rack,
            (1 << NodeCapability::Sensors as u8)
                | (1 << NodeCapability::Radio as u8)
                | (1 << NodeCapability::LowPower as u8),
            NodeResources {
                cpu_cores: 1,
                cpu_mhz: 160,
                ram_kib: 512,
                ram_free_kib: 384,
                accel_count: 0,
                cpu_load: 5,
                active_agents: 0,
            },
            1200, // 1.2ms RTT
            1,
        ) {
            let _ = esp;
        }

        // Simulated remote: cloud GPU inference node
        if let Some(gpu) = FABRIC.register_remote(
            b"cloud-gpu-a100",
            NodeArch::X86_64,
            LocalityZone::DataCenter,
            (1 << NodeCapability::Compute as u8)
                | (1 << NodeCapability::GpuCompute as u8)
                | (1 << NodeCapability::ModelInference as u8)
                | (1 << NodeCapability::CloudApi as u8),
            NodeResources {
                cpu_cores: 16,
                cpu_mhz: 3200,
                ram_kib: 64 * 1024 * 1024,
                ram_free_kib: 48 * 1024 * 1024,
                accel_count: 8,
                cpu_load: 40,
                active_agents: 0,
            },
            5000, // 5ms RTT
            1,
        ) {
            let _ = gpu;
        }

        // Seed some persistent memory entries
        MEMORY.persistent.store(
            b"os.name",
            b"VeerOS",
            MemoryTag::System,
            MemoryScope::Global,
            0,
            1,
            255,
        );
        MEMORY.persistent.store(
            b"os.arch",
            b"multi-arch",
            MemoryTag::System,
            MemoryScope::Global,
            0,
            1,
            255,
        );
        MEMORY.persistent.store(
            b"demo.mode",
            b"interactive",
            MemoryTag::Config,
            MemoryScope::Global,
            0,
            1,
            200,
        );
    }

    // Build the console over our host serial backend.
    let serial = StdSerial;
    let mut con = Console::new(serial);

    // ── Boot banner ──────────────────────────────────────────────────
    let _ = writeln!(con, "");
    let _ = writeln!(con, "========================================");
    let _ = writeln!(con, "  VeerOS v{VERSION}  \u{2014}  AI-Native OS");
    let _ = writeln!(con, "  Platform : Host (demo)");
    let _ = writeln!(con, "  Scheduler: minimal");
    let _ = writeln!(con, "========================================");
    let _ = writeln!(con, "");
    let _ = writeln!(con, "[boot] trap vector skipped (host build)");
    let _ = writeln!(con, "[boot] interrupt controller  \u{2014} n/a (host)");
    let _ = writeln!(con, "[boot] systimer tick          \u{2014} n/a (host)");
    let _ = writeln!(con, "[boot] idle task registered");
    let _ = writeln!(con, "[ai]   agent table            \u{2014} 32 slots");
    let _ = writeln!(con, "[ai]   intent engine          \u{2014} 16 slots");
    let _ = writeln!(
        con,
        "[ai]   memory engine          \u{2014} persistent + episodic"
    );
    unsafe {
        let (total, healthy) = FABRIC.node_counts();
        let _ = writeln!(
            con,
            "[ai]   execution fabric      \u{2014} {total} nodes ({healthy} healthy)"
        );
    }
    let _ = writeln!(con, "[ai]   intent scheduler       \u{2014} online");
    #[cfg(feature = "quic-demo")]
    {
        match run_quic_smoke() {
            Ok(()) => {
                let _ = writeln!(con, "[net]  quic mesh bridge       \u{2014} smoke ok");
            }
            Err(e) => {
                let _ = writeln!(con, "[net]  quic mesh bridge       \u{2014} smoke failed ({e})");
            }
        }
    }
    #[cfg(not(feature = "quic-demo"))]
    {
        let _ = writeln!(con, "[net]  quic mesh bridge       \u{2014} disabled (build without 'quic-demo')");
    }
    let _ = writeln!(con, "[boot] shell starting...");
    let _ = writeln!(con, "");
    let _ = writeln!(
        con,
        "Type 'help' for commands.  Try: agents, intent, memory, fabric"
    );
    let _ = writeln!(con, "");

    // ── Launch the shell ─────────────────────────────────────────────
    let env = ShellEnv {
        version: VERSION,
        platform: "Host (demo)",
        scheduler: "minimal",
        get_uptime_ticks: None,
        get_task_list: None,
        get_mem_info: None,
        get_driver_list: None,
        wifi_cmd: None,
        bt_cmd: None,
        zigbee_cmd: None,
        sensor_cmd: None,
        get_current_user: None,
        get_user_list: None,
        vfs_list_dir: None,
        vfs_read_file: None,
        vfs_write_file: None,
        vfs_mkdir: None,
        vfs_stat: None,
        vfs_unlink: None,
        vfs_rename: None,
        vfs_getcwd: None,
        vfs_chdir: None,
        vfs_tree: None,
        vfs_touch: None,
        mount_list: None,
        mount_fs: None,
        umount_fs: None,
        lsblk: None,
        input_status: None,
        usb_list: None,
        ble_hid_list: None,
        gpio_cmd: None,
        i2c_cmd: None,
        spi_cmd: None,
        hw_info: None,
        get_temp_millic: None,
        dmesg: None,
        reboot: None,
        shutdown: None,
        caps_cmd: None,
        auditlog_cmd: None,
        ifconfig_cmd: None,
        ping_cmd: None,
        netstat_cmd: None,
        ssh_cmd: None,
        login: None,
        logout: None,
        change_password: None,
        add_user: None,
        remove_user: None,
        pre_authenticated: true,
        // AI-native — fully wired
        get_agent_list: Some(do_get_agent_list),
        agent_cmd: Some(do_agent_cmd),
        get_intent_list: Some(do_get_intent_list),
        intent_cmd: Some(do_intent_cmd),
        memory_cmd: Some(do_memory_cmd),
        get_fabric_status: Some(do_get_fabric_status),
        peers_cmd: None,
        mesh_cmd: Some(do_mesh_cmd),
        zkp_cmd: None,
        hostname_cmd: None,
        df_cmd: None,
        sleep_ms: None,
    };
    let mut sh = Shell::new(env);
    sh.run(&mut con);

    let _ = writeln!(con, "VeerOS halted.");
}

#[cfg(all(test, feature = "quic-demo"))]
mod tests {
    use super::*;

    #[test]
    fn quic_smoke_flow_passes() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async {
            if let Err(e) = quic_smoke_async().await {
                panic!("quic smoke failed: {e}");
            }
        });
    }
}

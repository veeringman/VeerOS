use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use vas::{canonicalize, AddressType, VasAddress};
use veer_governor::{analyze_audit_records, AnalysisConfig, AuditAnalysisReport, AuditRecord};
use veer_graph::{
    Direction, EdgeKind, EdgeWeights, GraphCore, PolicySet, SolverWeights, VertexKind,
};
use veer_resolve::{Endpoint, ResolveError, Resolver, ServiceBinding};

#[derive(Parser, Debug)]
#[command(name = "veer", version, about = "VeerOS unified CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    Aura {
        #[command(subcommand)]
        cmd: AuraCmd,
    },
    Fold {
        #[command(subcommand)]
        cmd: FoldCmd,
    },
    Gateway {
        #[command(subcommand)]
        cmd: GatewayCmd,
    },
    Trace {
        #[command(subcommand)]
        cmd: TraceCmd,
    },
    Connect(ConnectArgs),
}

#[derive(Subcommand, Debug)]
enum AuraCmd {
    Create(AuraCreateArgs),
    Join(AuraJoinArgs),
    Share(AuraShareArgs),
    GrantAgent(AuraGrantAgentArgs),
    Overlap(AuraOverlapArgs),
    Leave(AuraLeaveArgs),
}

#[derive(Subcommand, Debug)]
enum FoldCmd {
    Launch(FoldLaunchArgs),
}

#[derive(Subcommand, Debug)]
enum GatewayCmd {
    /// Classic TCP socket bridge (local listen -> remote target).
    Bridge(BridgeArgs),
}

#[derive(Subcommand, Debug)]
enum TraceCmd {
    /// Explain solver decision path from a graph snapshot.
    Decision(TraceDecisionArgs),
    /// Analyze governor audit stream for anomalies and policy suggestions.
    Governor(TraceGovernorArgs),
}

#[derive(Args, Debug)]
struct AuraCreateArgs {
    aura: String,
    #[arg(long)]
    name: Option<String>,
}

#[derive(Args, Debug)]
struct AuraJoinArgs {
    aura: String,
}

#[derive(Args, Debug)]
struct AuraShareArgs {
    aura: String,
    #[arg(long = "with", required = true)]
    with_members: Vec<String>,
}

#[derive(Args, Debug)]
struct AuraLeaveArgs {
    aura: String,
}

#[derive(Args, Debug)]
struct AuraGrantAgentArgs {
    aura: String,
    #[arg(long)]
    agent: String,
    #[arg(long)]
    expires_unix_ms: u128,
    #[arg(long = "scope", required = true)]
    scopes: Vec<String>,
}

#[derive(Args, Debug)]
struct AuraOverlapArgs {
    /// Optional principal to inspect (usr/dev/fld/agt/svc/nod).
    #[arg(long)]
    member: Option<String>,
    /// Include active temporary agent memberships in overlap results.
    #[arg(long)]
    include_temporary: bool,
    /// Emit JSON output.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct FoldLaunchArgs {
    #[arg(short, long)]
    manifest: PathBuf,
    #[arg(long = "aura")]
    auras: Vec<String>,
}

#[derive(Args, Debug)]
struct ConnectArgs {
    service: String,
    #[arg(long)]
    host: Option<String>,
    #[arg(long)]
    port: Option<u16>,
    /// Optional path to legacy route map TOML.
    ///
    /// If omitted, defaults to
    /// `$XDG_CONFIG_HOME/veeros/legacy-routes.toml` (or
    /// `$HOME/.config/veeros/legacy-routes.toml`).
    #[arg(long)]
    legacy_map: Option<PathBuf>,
    /// Optional path to resolver service registry TOML.
    ///
    /// If omitted, defaults to
    /// `$XDG_CONFIG_HOME/veeros/resolve-map.toml` (or
    /// `$HOME/.config/veeros/resolve-map.toml`).
    #[arg(long)]
    resolve_map: Option<PathBuf>,
    #[arg(long = "aura")]
    auras: Vec<String>,
}

#[derive(Args, Debug)]
struct BridgeArgs {
    /// Local TCP listener bind address, e.g. 127.0.0.1:19000
    #[arg(long)]
    listen: String,
    /// Remote target host (DNS or IP)
    #[arg(long)]
    target_host: String,
    /// Remote target port
    #[arg(long)]
    target_port: u16,
}

#[derive(Args, Debug)]
struct TraceDecisionArgs {
    /// Graph snapshot TOML path.
    #[arg(long)]
    graph: PathBuf,
    /// Source vertex id (VAS).
    #[arg(long)]
    source: String,
    /// Edge kind to evaluate.
    #[arg(long, value_enum)]
    edge_kind: EdgeKindArg,
    /// Optional target vertex kind filter.
    #[arg(long, value_enum)]
    target_kind: Option<VertexKindArg>,
    /// Optional policy TOML path (`PolicySet` schema).
    #[arg(long)]
    policy: Option<PathBuf>,
    /// Emit JSON instead of text table.
    #[arg(long)]
    json: bool,
    #[arg(long, default_value_t = 1.0)]
    alpha: f32,
    #[arg(long, default_value_t = 1.0)]
    beta: f32,
    #[arg(long, default_value_t = 1.0)]
    gamma: f32,
    #[arg(long, default_value_t = 1.0)]
    delta: f32,
}

#[derive(Args, Debug)]
struct TraceGovernorArgs {
    /// Governor JSONL audit file path.
    #[arg(long)]
    audit: PathBuf,
    /// Analyze only the most recent N records.
    #[arg(long, default_value_t = 200)]
    limit: usize,
    /// Minimum event count before deny ratio anomaly is evaluated.
    #[arg(long)]
    min_events: Option<usize>,
    /// Deny ratio threshold for high-severity anomaly.
    #[arg(long)]
    deny_ratio_threshold: Option<f32>,
    /// Threshold for repeated "actor not in admins" denied events.
    #[arg(long)]
    unknown_actor_denied_threshold: Option<usize>,
    /// Threshold for repeated "member type not allowed" denied events.
    #[arg(long)]
    member_type_denied_threshold: Option<usize>,
    /// Threshold for repeated successful policy updates.
    #[arg(long)]
    policy_update_churn_threshold: Option<usize>,
    /// Emit JSON report.
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum EdgeKindArg {
    Trust,
    Membership,
    Reachability,
    Capability,
    Replication,
    Affinity,
}

impl EdgeKindArg {
    fn as_kind(self) -> EdgeKind {
        match self {
            EdgeKindArg::Trust => EdgeKind::Trust,
            EdgeKindArg::Membership => EdgeKind::Membership,
            EdgeKindArg::Reachability => EdgeKind::Reachability,
            EdgeKindArg::Capability => EdgeKind::Capability,
            EdgeKindArg::Replication => EdgeKind::Replication,
            EdgeKindArg::Affinity => EdgeKind::Affinity,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum VertexKindArg {
    User,
    Device,
    Fold,
    Aura,
    Service,
    Vault,
    Agent,
    Zone,
    Node,
    Event,
}

impl VertexKindArg {
    fn as_kind(self) -> VertexKind {
        match self {
            VertexKindArg::User => VertexKind::User,
            VertexKindArg::Device => VertexKind::Device,
            VertexKindArg::Fold => VertexKind::Fold,
            VertexKindArg::Aura => VertexKind::Aura,
            VertexKindArg::Service => VertexKind::Service,
            VertexKindArg::Vault => VertexKind::Vault,
            VertexKindArg::Agent => VertexKind::Agent,
            VertexKindArg::Zone => VertexKind::Zone,
            VertexKindArg::Node => VertexKind::Node,
            VertexKindArg::Event => VertexKind::Event,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AuraState {
    joined: BTreeSet<String>,
    created: BTreeMap<String, CreatedAura>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CreatedAura {
    display_name: Option<String>,
    shared_with: BTreeSet<String>,
    #[serde(default)]
    temporary_agents: BTreeMap<String, TemporaryAgentGrant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TemporaryAgentGrant {
    expires_unix_ms: u128,
    #[serde(default)]
    scopes: BTreeSet<String>,
    granted_at_unix_ms: u128,
}

#[derive(Debug, Serialize)]
struct AuraOverlapEdgeView {
    left_aura: String,
    right_aura: String,
    shared_members: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LegacyRouteMap {
    #[serde(default)]
    services: BTreeMap<String, LegacyRoute>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct LegacyRoute {
    #[serde(default)]
    dns: Vec<String>,
    #[serde(default)]
    ips: Vec<String>,
    port: u16,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ResolveMap {
    #[serde(default)]
    services: BTreeMap<String, Vec<ResolveEndpointSpec>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResolveEndpointSpec {
    node: String,
    transport: String,
    #[serde(default = "default_true")]
    healthy: bool,
    #[serde(default)]
    latency_ms: Option<u32>,
    #[serde(default)]
    required_aura: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct GraphSpec {
    #[serde(default)]
    vertices: Vec<GraphVertexSpec>,
    #[serde(default)]
    edges: Vec<GraphEdgeSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphVertexSpec {
    id: String,
    #[serde(default)]
    attrs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphEdgeSpec {
    from: String,
    to: String,
    kind: EdgeKind,
    #[serde(default)]
    weights: Option<EdgeWeights>,
    #[serde(default)]
    attrs: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
struct DecisionTraceReport {
    source: String,
    edge_kind: String,
    target_kind: Option<String>,
    policy_mode: String,
    selected: Option<DecisionCandidate>,
    candidates: Vec<DecisionCandidate>,
}

#[derive(Debug, Serialize)]
struct DecisionCandidate {
    target: String,
    target_kind: String,
    policy_allowed: bool,
    kind_allowed: bool,
    score: Option<f32>,
    latency: f32,
    trust: f32,
    cost: f32,
    affinity: f32,
    load: f32,
}

fn default_true() -> bool {
    true
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Aura { cmd } => cmd_aura(cmd),
        Cmd::Fold { cmd } => cmd_fold(cmd),
        Cmd::Gateway { cmd } => cmd_gateway(cmd),
        Cmd::Trace { cmd } => cmd_trace(cmd),
        Cmd::Connect(args) => cmd_connect(args),
    }
}

fn cmd_trace(cmd: TraceCmd) -> Result<()> {
    match cmd {
        TraceCmd::Decision(args) => cmd_trace_decision(args),
        TraceCmd::Governor(args) => cmd_trace_governor(args),
    }
}

fn cmd_trace_governor(args: TraceGovernorArgs) -> Result<()> {
    let all = load_audit_records(&args.audit)?;
    let records = if args.limit == 0 || all.len() <= args.limit {
        all
    } else {
        all[all.len() - args.limit..].to_vec()
    };

    let mut cfg = AnalysisConfig::default();
    if let Some(v) = args.min_events {
        cfg.min_events = v;
    }
    if let Some(v) = args.deny_ratio_threshold {
        cfg.deny_ratio_threshold = v;
    }
    if let Some(v) = args.unknown_actor_denied_threshold {
        cfg.unknown_actor_denied_threshold = v;
    }
    if let Some(v) = args.member_type_denied_threshold {
        cfg.member_type_denied_threshold = v;
    }
    if let Some(v) = args.policy_update_churn_threshold {
        cfg.policy_update_churn_threshold = v;
    }

    let report = analyze_audit_records(&records, &cfg);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_governor_report(&report, &cfg, records.len());
    }
    Ok(())
}

fn cmd_trace_decision(args: TraceDecisionArgs) -> Result<()> {
    let graph = load_graph_spec(&args.graph)?;
    let source = canonical_typed(&args.source, AddressType::Service, "source")?;
    let edge_kind = args.edge_kind.as_kind();
    let target_kind = args.target_kind.map(|k| k.as_kind());

    let policy = if let Some(path) = args.policy.as_deref() {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str::<PolicySet>(&text).with_context(|| format!("parsing {}", path.display()))?
    } else {
        PolicySet::default()
    };

    let coeff = SolverWeights {
        alpha_latency: args.alpha,
        beta_cost: args.beta,
        gamma_trust: args.gamma,
        delta_affinity: args.delta,
    };

    let selected = graph
        .solve_best_target_with_policy(&source, edge_kind, target_kind, coeff, &policy)
        .map_err(|e| anyhow::anyhow!("solver error: {:?}", e))?;

    let mut candidates = Vec::new();
    for e in graph
        .neighbors(&source, Direction::Out, Some(edge_kind))
        .map_err(|e| anyhow::anyhow!("graph query error: {:?}", e))?
    {
        let Some(v) = graph.get_vertex(&e.to) else {
            continue;
        };
        let kind_allowed = target_kind.map(|k| k == v.kind).unwrap_or(true);
        let policy_allowed = policy.allows(v, e);
        let score = if kind_allowed && policy_allowed {
            Some(
                coeff.alpha_latency * e.weights.latency
                    + coeff.beta_cost * (e.weights.cost + e.weights.load)
                    - coeff.gamma_trust * e.weights.trust
                    - coeff.delta_affinity * e.weights.affinity,
            )
        } else {
            None
        };

        candidates.push(DecisionCandidate {
            target: e.to.clone(),
            target_kind: format!("{:?}", v.kind).to_ascii_lowercase(),
            policy_allowed,
            kind_allowed,
            score,
            latency: e.weights.latency,
            trust: e.weights.trust,
            cost: e.weights.cost,
            affinity: e.weights.affinity,
            load: e.weights.load,
        });
    }
    candidates.sort_by(|a, b| a.target.cmp(&b.target));

    let report = DecisionTraceReport {
        source,
        edge_kind: format!("{:?}", edge_kind).to_ascii_lowercase(),
        target_kind: target_kind.map(|k| format!("{:?}", k).to_ascii_lowercase()),
        policy_mode: format!("{:?}", policy.mode).to_ascii_lowercase(),
        selected: selected.map(|s| DecisionCandidate {
            target: s.target,
            target_kind: target_kind
                .map(|k| format!("{:?}", k).to_ascii_lowercase())
                .unwrap_or_else(|| "unknown".to_string()),
            policy_allowed: true,
            kind_allowed: true,
            score: Some(s.score),
            latency: s.weights.latency,
            trust: s.weights.trust,
            cost: s.weights.cost,
            affinity: s.weights.affinity,
            load: s.weights.load,
        }),
        candidates,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_decision_report(&report);
    }

    Ok(())
}

fn load_graph_spec(path: &Path) -> Result<GraphCore> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let spec: GraphSpec =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;

    let mut g = GraphCore::new();
    for v in &spec.vertices {
        g.upsert_vertex(&v.id, None, v.attrs.clone())
            .map_err(|e| anyhow::anyhow!("graph vertex {} invalid: {:?}", v.id, e))?;
    }
    for e in &spec.edges {
        g.upsert_edge(
            &e.from,
            &e.to,
            e.kind,
            e.weights.unwrap_or_default(),
            e.attrs.clone(),
        )
        .map_err(|err| anyhow::anyhow!("graph edge {} -> {} invalid: {:?}", e.from, e.to, err))?;
    }
    Ok(g)
}

fn print_decision_report(report: &DecisionTraceReport) {
    println!("source      : {}", report.source);
    println!("edge kind   : {}", report.edge_kind);
    println!(
        "target kind : {}",
        report
            .target_kind
            .clone()
            .unwrap_or_else(|| "any".to_string())
    );
    println!("policy mode : {}", report.policy_mode);

    if let Some(sel) = &report.selected {
        println!(
            "selected    : {} (score {:.4})",
            sel.target,
            sel.score.unwrap_or_default()
        );
    } else {
        println!("selected    : none");
    }

    println!();
    println!(
        "{:<30} {:<8} {:<8} {:>10} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "TARGET", "KIND", "POLICY", "SCORE", "LAT", "TRUST", "COST", "AFF", "LOAD"
    );
    for c in &report.candidates {
        let score = c
            .score
            .map(|s| format!("{s:.4}"))
            .unwrap_or_else(|| "blocked".to_string());
        println!(
            "{:<30} {:<8} {:<8} {:>10} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3}",
            c.target,
            c.target_kind,
            if c.policy_allowed && c.kind_allowed {
                "yes"
            } else {
                "no"
            },
            score,
            c.latency,
            c.trust,
            c.cost,
            c.affinity,
            c.load,
        );
    }
}

fn load_audit_records(path: &Path) -> Result<Vec<AuditRecord>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let rec: AuditRecord = serde_json::from_str(trimmed)
            .with_context(|| format!("parsing {} line {}", path.display(), idx + 1))?;
        out.push(rec);
    }
    Ok(out)
}

fn print_governor_report(report: &AuditAnalysisReport, cfg: &AnalysisConfig, analyzed: usize) {
    println!("governor audit analysis");
    println!("  analyzed events : {}", analyzed);
    println!("  total events    : {}", report.total_events);
    println!("  allowed         : {}", report.allowed_events);
    println!("  denied          : {}", report.denied_events);
    println!("  deny ratio      : {:.1}%", report.deny_ratio * 100.0);
    println!("  unique actors   : {}", report.unique_actors);
    println!();
    println!(
        "thresholds: min_events={} deny_ratio>={:.1}% unknown_actor_denied>={} member_type_denied>={} policy_update_churn>={}",
        cfg.min_events,
        cfg.deny_ratio_threshold * 100.0,
        cfg.unknown_actor_denied_threshold,
        cfg.member_type_denied_threshold,
        cfg.policy_update_churn_threshold,
    );

    println!();
    if report.findings.is_empty() {
        println!("findings: none");
    } else {
        println!("findings:");
        for f in &report.findings {
            println!(
                "  - [{}] {} (count={}): {}",
                f.severity, f.code, f.count, f.detail
            );
        }
    }

    println!();
    if report.suggestions.is_empty() {
        println!("policy suggestions: none");
    } else {
        println!("policy suggestions:");
        for s in &report.suggestions {
            println!("  - [{}] {}: {}", s.priority, s.id, s.summary);
            println!("    rationale: {}", s.rationale);
        }
    }
}

fn cmd_gateway(cmd: GatewayCmd) -> Result<()> {
    match cmd {
        GatewayCmd::Bridge(args) => cmd_gateway_bridge(args),
    }
}

fn cmd_gateway_bridge(args: BridgeArgs) -> Result<()> {
    let target = format!("{}:{}", args.target_host, args.target_port);
    let listener = TcpListener::bind(&args.listen)
        .with_context(|| format!("binding local bridge listener {}", args.listen))?;

    println!("legacy socket bridge active");
    println!("  listen: {}", args.listen);
    println!("  target: {}", target);

    for inbound in listener.incoming() {
        let target = target.clone();
        match inbound {
            Ok(client) => {
                thread::spawn(move || {
                    if let Err(e) = handle_bridge_client(client, &target) {
                        eprintln!("bridge client error: {e:#}");
                    }
                });
            }
            Err(e) => eprintln!("bridge accept error: {e}"),
        }
    }

    Ok(())
}

fn handle_bridge_client(client: TcpStream, target: &str) -> Result<()> {
    let server =
        TcpStream::connect(target).with_context(|| format!("connecting bridge target {target}"))?;

    let mut c_r = client
        .try_clone()
        .context("cloning client stream for upstream")?;
    let mut c_w = client;
    let mut s_r = server
        .try_clone()
        .context("cloning server stream for downstream")?;
    let mut s_w = server;

    let up = thread::spawn(move || {
        let _ = io::copy(&mut c_r, &mut s_w);
        let _ = s_w.shutdown(Shutdown::Write);
    });
    let down = thread::spawn(move || {
        let _ = io::copy(&mut s_r, &mut c_w);
        let _ = c_w.shutdown(Shutdown::Write);
    });

    let _ = up.join();
    let _ = down.join();
    Ok(())
}

fn cmd_aura(cmd: AuraCmd) -> Result<()> {
    let mut state = load_state()?;
    match cmd {
        AuraCmd::Create(args) => {
            let aura = canonical_typed(&args.aura, AddressType::Aura, "aura")?;
            state.joined.insert(aura.clone());
            state
                .created
                .entry(aura.clone())
                .and_modify(|c| {
                    if args.name.is_some() {
                        c.display_name = args.name.clone();
                    }
                })
                .or_insert(CreatedAura {
                    display_name: args.name,
                    shared_with: BTreeSet::new(),
                    temporary_agents: BTreeMap::new(),
                });
            save_state(&state)?;
            println!("aura created: {}", aura);
            Ok(())
        }
        AuraCmd::Join(args) => {
            let aura = canonical_typed(&args.aura, AddressType::Aura, "aura")?;
            let inserted = state.joined.insert(aura.clone());
            save_state(&state)?;
            if inserted {
                println!("joined aura: {}", aura);
            } else {
                println!("already joined aura: {}", aura);
            }
            Ok(())
        }
        AuraCmd::Share(args) => {
            let aura = canonical_typed(&args.aura, AddressType::Aura, "aura")?;
            let created = state.created.get_mut(&aura).with_context(|| {
                format!(
                    "aura not found in local state: {aura}; create it first with veer aura create"
                )
            })?;
            let mut added = 0usize;
            for member in &args.with_members {
                let canonical = canonical_share_target(member)?;
                if created.shared_with.insert(canonical) {
                    added += 1;
                }
            }
            save_state(&state)?;
            println!("shared aura {} with {} new member(s)", aura, added);
            Ok(())
        }
        AuraCmd::GrantAgent(args) => {
            let now = now_unix_ms();
            if args.expires_unix_ms <= now {
                bail!("expires_unix_ms must be in the future");
            }

            let aura = canonical_typed(&args.aura, AddressType::Aura, "aura")?;
            let agent = canonical_typed(&args.agent, AddressType::Agent, "agent")?;
            let created = state.created.get_mut(&aura).with_context(|| {
                format!(
                    "aura not found in local state: {aura}; create it first with veer aura create"
                )
            })?;

            let mut scopes = BTreeSet::new();
            for scope in &args.scopes {
                let s = scope.trim().to_ascii_lowercase();
                if s.is_empty() {
                    bail!("scope values must be non-empty");
                }
                scopes.insert(s);
            }
            if scopes.is_empty() {
                bail!("at least one --scope is required");
            }

            created.temporary_agents.insert(
                agent.clone(),
                TemporaryAgentGrant {
                    expires_unix_ms: args.expires_unix_ms,
                    scopes,
                    granted_at_unix_ms: now,
                },
            );

            save_state(&state)?;
            println!(
                "temporary agent membership granted: {} in {} until {}",
                agent, aura, args.expires_unix_ms
            );
            Ok(())
        }
        AuraCmd::Overlap(args) => {
            let now = now_unix_ms();
            let mut aura_members = BTreeMap::new();
            for (aura, created) in &state.created {
                let members = members_for_created_aura(created, now, args.include_temporary);
                aura_members.insert(aura.clone(), members);
            }

            if let Some(member) = args.member {
                let member = canonical_overlap_target(&member)?;
                let mut in_auras = Vec::new();
                for (aura, members) in &aura_members {
                    if members.contains(&member) {
                        in_auras.push(aura.clone());
                    }
                }
                in_auras.sort();

                if args.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "member": member,
                            "auras": in_auras,
                        }))?
                    );
                } else {
                    println!("member: {}", member);
                    if in_auras.is_empty() {
                        println!("participates in: none");
                    } else {
                        println!("participates in:");
                        for aura in &in_auras {
                            println!("  - {}", aura);
                        }
                    }
                }
                return Ok(());
            }

            let mut aura_names: Vec<String> = aura_members.keys().cloned().collect();
            aura_names.sort();
            let mut edges = Vec::new();
            for i in 0..aura_names.len() {
                for j in (i + 1)..aura_names.len() {
                    let left = &aura_names[i];
                    let right = &aura_names[j];
                    let left_set = aura_members.get(left).cloned().unwrap_or_default();
                    let right_set = aura_members.get(right).cloned().unwrap_or_default();
                    let shared_members: Vec<String> =
                        left_set.intersection(&right_set).cloned().collect();
                    if !shared_members.is_empty() {
                        edges.push(AuraOverlapEdgeView {
                            left_aura: left.clone(),
                            right_aura: right.clone(),
                            shared_members,
                        });
                    }
                }
            }

            if args.json {
                println!("{}", serde_json::to_string_pretty(&edges)?);
            } else if edges.is_empty() {
                println!("no aura overlap edges");
            } else {
                println!("aura overlap graph:");
                for e in &edges {
                    println!(
                        "  - {} <-> {} (shared={}): {}",
                        e.left_aura,
                        e.right_aura,
                        e.shared_members.len(),
                        e.shared_members.join(",")
                    );
                }
            }
            Ok(())
        }
        AuraCmd::Leave(args) => {
            let aura = canonical_typed(&args.aura, AddressType::Aura, "aura")?;
            let removed = state.joined.remove(&aura);
            save_state(&state)?;
            if removed {
                println!("left aura: {}", aura);
            } else {
                println!("not currently joined: {}", aura);
            }
            Ok(())
        }
    }
}

fn now_unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn cmd_fold(cmd: FoldCmd) -> Result<()> {
    match cmd {
        FoldCmd::Launch(args) => {
            let manifest_text = std::fs::read_to_string(&args.manifest)
                .with_context(|| format!("reading {}", args.manifest.display()))?;
            let mut doc: toml::Value = toml::from_str(&manifest_text)
                .with_context(|| format!("parsing {}", args.manifest.display()))?;

            let merged = merged_manifest_auras(&doc, &args.auras)?;
            set_manifest_auras(&mut doc, &merged)?;

            let out = toml::to_string_pretty(&doc).context("serializing launch manifest")?;
            let temp_manifest = write_temp_manifest(&out)?;

            let status = Command::new("fold")
                .args(["spawn", "--manifest"])
                .arg(&temp_manifest)
                .status()
                .context("spawning fold process; ensure fold is on PATH")?;

            let _ = std::fs::remove_file(&temp_manifest);

            if !status.success() {
                bail!("fold spawn failed with status: {}", status);
            }
            Ok(())
        }
    }
}

fn cmd_connect(args: ConnectArgs) -> Result<()> {
    let service = canonical_typed(&args.service, AddressType::Service, "service")?;
    let caller_auras: Vec<String> = args
        .auras
        .iter()
        .map(|a| canonical_typed(a, AddressType::Aura, "aura"))
        .collect::<Result<Vec<_>>>()?;

    let resolved_target =
        resolve_target_from_map(args.resolve_map.as_deref(), &service, &caller_auras)?;

    let legacy_route = load_legacy_route(args.legacy_map.as_deref(), &service)?;
    let resolver_port = resolved_target.as_ref().map(|(_, p)| *p);
    let port = args
        .port
        .or(resolver_port)
        .or_else(|| legacy_route.as_ref().map(|r| r.port))
        .with_context(|| "no port provided; pass --port or define route port in legacy map")?;
    let resolver_host = resolved_target.as_ref().map(|(h, _)| h.as_str());
    let candidate_hosts =
        build_candidate_hosts(args.host.as_deref(), resolver_host, legacy_route.as_ref());
    if candidate_hosts.is_empty() {
        bail!(
            "no target host candidates; pass --host, provide resolve-map, or define dns/ips in legacy route map for {}",
            service
        );
    }
    let (target_host, resolved) = pick_target_host_with_fallback(&candidate_hosts, port)?;

    println!("service: {}", service);
    if !caller_auras.is_empty() {
        println!("caller auras: {}", caller_auras.join(","));
    }
    println!("transport target: {}:{}", target_host, port);
    println!(
        "legacy resolution: {}",
        if resolved {
            "resolved"
        } else {
            "unresolved-used-first-candidate"
        }
    );

    let mut cmd = Command::new("veer-connect");
    cmd.arg("shell").arg(&target_host).arg(port.to_string());
    cmd.env("VEER_SERVICE", &service);
    cmd.env("VEER_CALLER_AURAS", caller_auras.join(","));
    cmd.env("VEER_LEGACY_TARGET", format!("{}:{}", target_host, port));

    let status = cmd
        .status()
        .context("launching veer-connect; ensure veer-connect is on PATH")?;
    if !status.success() {
        bail!("veer-connect shell failed with status: {}", status);
    }
    Ok(())
}

fn resolve_target_from_map(
    path: Option<&Path>,
    service: &str,
    caller_auras: &[String],
) -> Result<Option<(String, u16)>> {
    let map_path = path
        .map(ToOwned::to_owned)
        .unwrap_or(resolve_map_default_path()?);
    if !map_path.exists() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&map_path)
        .with_context(|| format!("reading {}", map_path.display()))?;
    let map: ResolveMap =
        toml::from_str(&text).with_context(|| format!("parsing {}", map_path.display()))?;

    let mut resolver = Resolver::new();
    for (svc_raw, endpoints) in map.services {
        let svc = canonical_typed(&svc_raw, AddressType::Service, "service")?;
        for ep in endpoints {
            let required_aura = if let Some(aura) = ep.required_aura {
                Some(canonical_typed(&aura, AddressType::Aura, "aura")?)
            } else {
                None
            };

            resolver
                .register(
                    &svc,
                    ServiceBinding {
                        endpoint: Endpoint {
                            node: ep.node,
                            transport: ep.transport,
                            latency_ms: ep.latency_ms.unwrap_or(10),
                            healthy: ep.healthy,
                        },
                        required_aura,
                    },
                )
                .map_err(|_| anyhow::anyhow!("invalid resolver entry for {svc}"))?;
        }
    }

    let aura_refs: Vec<&str> = caller_auras.iter().map(|s| s.as_str()).collect();
    let out = match resolver.resolve(service, &aura_refs) {
        Ok(rr) => {
            let (host, port) = endpoint_host_port(&rr.selected).with_context(|| {
                format!(
                    "resolver endpoint for {} does not contain host/port",
                    service
                )
            })?;
            Some((host, port))
        }
        Err(ResolveError::NotFound) => None,
        Err(ResolveError::DeniedByAura) => {
            bail!("resolver denied by aura policy for {}", service);
        }
        Err(ResolveError::NoHealthyEndpoint) => {
            bail!("resolver has no healthy endpoint for {}", service);
        }
        Err(ResolveError::InvalidAddress) => {
            bail!("resolver rejected invalid address for {}", service);
        }
    };

    Ok(out)
}

fn load_legacy_route(path: Option<&Path>, service: &str) -> Result<Option<LegacyRoute>> {
    let map_path = path
        .map(ToOwned::to_owned)
        .unwrap_or(legacy_map_default_path()?);
    if !map_path.exists() {
        return Ok(None);
    }

    let text = std::fs::read_to_string(&map_path)
        .with_context(|| format!("reading {}", map_path.display()))?;
    let mut map: LegacyRouteMap =
        toml::from_str(&text).with_context(|| format!("parsing {}", map_path.display()))?;

    let mut canonicalized = BTreeMap::new();
    for (k, v) in map.services {
        let key = canonical_typed(&k, AddressType::Service, "service")?;
        canonicalized.insert(key, v);
    }
    map.services = canonicalized;

    Ok(map.services.remove(service))
}

fn build_candidate_hosts(
    explicit_host: Option<&str>,
    resolver_host: Option<&str>,
    route: Option<&LegacyRoute>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(h) = explicit_host {
        let h = h.trim().to_string();
        if !h.is_empty() && seen.insert(h.clone()) {
            out.push(h);
        }
    }
    if let Some(h) = resolver_host {
        let h = h.trim().to_string();
        if !h.is_empty() && seen.insert(h.clone()) {
            out.push(h);
        }
    }
    if let Some(route) = route {
        for h in route.dns.iter().chain(route.ips.iter()) {
            let h = h.trim().to_string();
            if !h.is_empty() && seen.insert(h.clone()) {
                out.push(h);
            }
        }
    }

    out
}

fn pick_target_host_with_fallback(candidates: &[String], port: u16) -> Result<(String, bool)> {
    for host in candidates {
        if host.parse::<IpAddr>().is_ok() {
            return Ok((host.clone(), true));
        }
        if host_resolves(host, port) {
            return Ok((host.clone(), true));
        }
    }

    // Last-resort fallback: keep legacy behavior and hand the first candidate
    // to the transport command, which may still be able to connect.
    let first = candidates
        .first()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no host candidates available"))?;
    Ok((first, false))
}

fn host_resolves(host: &str, port: u16) -> bool {
    (host, port)
        .to_socket_addrs()
        .map(|mut addrs| addrs.next().is_some())
        .unwrap_or(false)
}

fn endpoint_host_port(ep: &Endpoint) -> Result<(String, u16)> {
    if let Some((host, port)) = split_host_port(&ep.node) {
        return Ok((host, port));
    }

    if let Some(rest) = ep.transport.split_once("://").map(|(_, r)| r) {
        if let Some((host, port)) = split_host_port(rest) {
            return Ok((host, port));
        }
    }

    if let Some(port_txt) = ep.transport.strip_prefix("tcp:") {
        let port: u16 = port_txt
            .parse()
            .with_context(|| format!("invalid tcp port in transport {}", ep.transport))?;
        if !ep.node.trim().is_empty() {
            return Ok((ep.node.trim().to_string(), port));
        }
    }

    if let Some(port_txt) = ep.transport.strip_prefix("quic:") {
        let port: u16 = port_txt
            .parse()
            .with_context(|| format!("invalid quic port in transport {}", ep.transport))?;
        if !ep.node.trim().is_empty() {
            return Ok((ep.node.trim().to_string(), port));
        }
    }

    bail!(
        "cannot extract host:port from endpoint node={} transport={}",
        ep.node,
        ep.transport
    )
}

fn split_host_port(input: &str) -> Option<(String, u16)> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }

    if let Ok(addr) = s.parse::<std::net::SocketAddr>() {
        return Some((addr.ip().to_string(), addr.port()));
    }

    let (host, port_txt) = s.rsplit_once(':')?;
    let port: u16 = port_txt.parse().ok()?;
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port))
}

fn canonical_typed(input: &str, expected: AddressType, what: &str) -> Result<String> {
    let canonical = canonicalize(input)
        .map_err(|e| anyhow::anyhow!("invalid {} address {}: {}", what, input, e))?;
    let parsed = VasAddress::parse(&canonical)
        .map_err(|e| anyhow::anyhow!("invalid {} address {}: {}", what, input, e))?;
    if parsed.kind != expected {
        bail!(
            "{} address must use {}{{...}} type",
            what,
            address_type_code(expected)
        );
    }
    Ok(canonical)
}

fn canonical_share_target(input: &str) -> Result<String> {
    let canonical = canonicalize(input)
        .map_err(|e| anyhow::anyhow!("invalid share target {}: {}", input, e))?;
    let parsed = VasAddress::parse(&canonical)
        .map_err(|e| anyhow::anyhow!("invalid share target {}: {}", input, e))?;
    match parsed.kind {
        AddressType::User
        | AddressType::Device
        | AddressType::Fold
        | AddressType::Agent
        | AddressType::Service
        | AddressType::Node => Ok(canonical),
        _ => bail!("share target must be one of usr/dev/fld/agt/svc/nod"),
    }
}

fn canonical_overlap_target(input: &str) -> Result<String> {
    let canonical = canonicalize(input)
        .map_err(|e| anyhow::anyhow!("invalid overlap target {}: {}", input, e))?;
    let parsed = VasAddress::parse(&canonical)
        .map_err(|e| anyhow::anyhow!("invalid overlap target {}: {}", input, e))?;
    match parsed.kind {
        AddressType::User
        | AddressType::Device
        | AddressType::Fold
        | AddressType::Agent
        | AddressType::Service
        | AddressType::Node => Ok(canonical),
        _ => bail!("overlap target must be one of usr/dev/fld/agt/svc/nod"),
    }
}

fn members_for_created_aura(
    created: &CreatedAura,
    now_unix_ms: u128,
    include_temporary: bool,
) -> BTreeSet<String> {
    let mut out = created.shared_with.clone();
    if include_temporary {
        for (agent, grant) in &created.temporary_agents {
            if grant.expires_unix_ms > now_unix_ms {
                out.insert(agent.clone());
            }
        }
    }
    out
}

fn merged_manifest_auras(doc: &toml::Value, extra: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(existing) = doc.get("auras") {
        let arr = existing
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("manifest auras must be an array of strings"))?;
        for v in arr {
            let s = v
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("manifest auras must contain only strings"))?;
            let c = canonical_typed(s, AddressType::Aura, "aura")?;
            if seen.insert(c.clone()) {
                out.push(c);
            }
        }
    }

    for aura in extra {
        let c = canonical_typed(aura, AddressType::Aura, "aura")?;
        if seen.insert(c.clone()) {
            out.push(c);
        }
    }

    Ok(out)
}

fn set_manifest_auras(doc: &mut toml::Value, auras: &[String]) -> Result<()> {
    let Some(table) = doc.as_table_mut() else {
        bail!("manifest root must be a TOML table");
    };
    let arr = auras
        .iter()
        .cloned()
        .map(toml::Value::String)
        .collect::<Vec<_>>();
    table.insert("auras".to_string(), toml::Value::Array(arr));
    Ok(())
}

fn write_temp_manifest(content: &str) -> Result<PathBuf> {
    let name = format!(
        "veer-launch-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn state_path() -> Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = std::env::var("HOME").context("HOME is not set")?;
        Path::new(&home).join(".local").join("state")
    };
    Ok(base.join("veeros").join("aura-state.json"))
}

fn legacy_map_default_path() -> Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = std::env::var("HOME").context("HOME is not set")?;
        Path::new(&home).join(".config")
    };
    Ok(base.join("veeros").join("legacy-routes.toml"))
}

fn resolve_map_default_path() -> Result<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else {
        let home = std::env::var("HOME").context("HOME is not set")?;
        Path::new(&home).join(".config")
    };
    Ok(base.join("veeros").join("resolve-map.toml"))
}

fn load_state() -> Result<AuraState> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(AuraState::default());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let state: AuraState =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(state)
}

fn save_state(state: &AuraState) -> Result<()> {
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(state).context("serializing aura state")?;
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn address_type_code(t: AddressType) -> &'static str {
    match t {
        AddressType::User => "usr",
        AddressType::Device => "dev",
        AddressType::Fold => "fld",
        AddressType::Aura => "aur",
        AddressType::Service => "svc",
        AddressType::Vault => "vlt",
        AddressType::Agent => "agt",
        AddressType::Zone => "zon",
        AddressType::Node => "nod",
        AddressType::Event => "evt",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_typed_accepts_expected_type() {
        let out =
            canonical_typed(" SVC{Render,Company,Live}", AddressType::Service, "service").unwrap();
        assert_eq!(out, "svc{render,company,live}");
    }

    #[test]
    fn canonical_typed_rejects_wrong_type() {
        let err = canonical_typed("aur{x,y,z}", AddressType::Service, "service").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("must use svc"));
    }

    #[test]
    fn merge_manifest_auras_dedups_and_canonicalizes() {
        let doc: toml::Value = toml::from_str(
            r#"
name = "x"
cmd = "/bin/true"
auras = ["aur{team,private,open}", " AUR{Team,Private,Open } "]
"#,
        )
        .unwrap();

        let merged = merged_manifest_auras(&doc, &["aur{ops,private,open}".to_string()]).unwrap();
        assert_eq!(
            merged,
            vec![
                "aur{team,private,open}".to_string(),
                "aur{ops,private,open}".to_string()
            ]
        );
    }

    #[test]
    fn candidate_hosts_prefers_explicit_then_dns_then_ips_deduped() {
        let route = LegacyRoute {
            dns: vec!["svc.example.local".into(), "svc.example.local".into()],
            ips: vec!["10.1.1.9".into(), "10.1.1.9".into()],
            port: 2232,
        };

        let out = build_candidate_hosts(
            Some("manual.example"),
            Some("resolved.example"),
            Some(&route),
        );
        assert_eq!(
            out,
            vec![
                "manual.example".to_string(),
                "resolved.example".to_string(),
                "svc.example.local".to_string(),
                "10.1.1.9".to_string(),
            ]
        );
    }

    #[test]
    fn fallback_prefers_literal_ip_candidate() {
        let (host, resolved) = pick_target_host_with_fallback(
            &["unresolvable.invalid".to_string(), "127.0.0.1".to_string()],
            2232,
        )
        .unwrap();
        assert_eq!(host, "127.0.0.1");
        assert!(resolved);
    }

    #[test]
    fn endpoint_host_port_accepts_node_socket_form() {
        let ep = Endpoint {
            node: "10.0.0.4:2232".into(),
            transport: "tcp".into(),
            latency_ms: 10,
            healthy: true,
        };

        let (host, port) = endpoint_host_port(&ep).unwrap();
        assert_eq!(host, "10.0.0.4");
        assert_eq!(port, 2232);
    }

    #[test]
    fn endpoint_host_port_accepts_transport_url_form() {
        let ep = Endpoint {
            node: "ignored".into(),
            transport: "tcp://render.internal:3344".into(),
            latency_ms: 10,
            healthy: true,
        };

        let (host, port) = endpoint_host_port(&ep).unwrap();
        assert_eq!(host, "render.internal");
        assert_eq!(port, 3344);
    }

    #[test]
    fn share_target_accepts_fold_address() {
        let out = canonical_share_target(" FLD{worker,gpu,warm} ").unwrap();
        assert_eq!(out, "fld{worker,gpu,warm}");
    }

    #[test]
    fn overlap_members_include_active_temporary_only() {
        let mut created = CreatedAura {
            display_name: None,
            shared_with: BTreeSet::from(["usr{alice,corp,active}".to_string()]),
            temporary_agents: BTreeMap::from([
                (
                    "agt{opsbot,corp,live}".to_string(),
                    TemporaryAgentGrant {
                        expires_unix_ms: 200,
                        scopes: BTreeSet::from(["observe".to_string()]),
                        granted_at_unix_ms: 100,
                    },
                ),
                (
                    "agt{expired,corp,live}".to_string(),
                    TemporaryAgentGrant {
                        expires_unix_ms: 80,
                        scopes: BTreeSet::from(["observe".to_string()]),
                        granted_at_unix_ms: 10,
                    },
                ),
            ]),
        };

        let without_temp = members_for_created_aura(&created, 100, false);
        assert_eq!(
            without_temp,
            BTreeSet::from(["usr{alice,corp,active}".to_string()])
        );

        let with_temp = members_for_created_aura(&created, 100, true);
        assert_eq!(
            with_temp,
            BTreeSet::from([
                "usr{alice,corp,active}".to_string(),
                "agt{opsbot,corp,live}".to_string(),
            ])
        );

        created.temporary_agents.clear();
        let no_temp = members_for_created_aura(&created, 100, true);
        assert_eq!(
            no_temp,
            BTreeSet::from(["usr{alice,corp,active}".to_string()])
        );
    }
}

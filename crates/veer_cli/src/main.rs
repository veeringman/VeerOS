use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use vas::{canonicalize, AddressType, VasAddress};
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
    Connect(ConnectArgs),
}

#[derive(Subcommand, Debug)]
enum AuraCmd {
    Create(AuraCreateArgs),
    Join(AuraJoinArgs),
    Share(AuraShareArgs),
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

#[derive(Debug, Default, Serialize, Deserialize)]
struct AuraState {
    joined: BTreeSet<String>,
    created: BTreeMap<String, CreatedAura>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CreatedAura {
    display_name: Option<String>,
    shared_with: BTreeSet<String>,
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

fn default_true() -> bool {
    true
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Aura { cmd } => cmd_aura(cmd),
        Cmd::Fold { cmd } => cmd_fold(cmd),
        Cmd::Gateway { cmd } => cmd_gateway(cmd),
        Cmd::Connect(args) => cmd_connect(args),
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
    let server = TcpStream::connect(target)
        .with_context(|| format!("connecting bridge target {target}"))?;

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
            state.created.entry(aura.clone()).and_modify(|c| {
                if args.name.is_some() {
                    c.display_name = args.name.clone();
                }
            }).or_insert(CreatedAura {
                display_name: args.name,
                shared_with: BTreeSet::new(),
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
            let created = state
                .created
                .get_mut(&aura)
                .with_context(|| format!("aura not found in local state: {aura}; create it first with veer aura create"))?;
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

    let resolved_target = resolve_target_from_map(args.resolve_map.as_deref(), &service, &caller_auras)?;

    let legacy_route = load_legacy_route(args.legacy_map.as_deref(), &service)?;
    let resolver_port = resolved_target.as_ref().map(|(_, p)| *p);
    let port = args
        .port
        .or(resolver_port)
        .or_else(|| legacy_route.as_ref().map(|r| r.port))
        .with_context(|| "no port provided; pass --port or define route port in legacy map")?;
    let resolver_host = resolved_target.as_ref().map(|(h, _)| h.as_str());
    let candidate_hosts = build_candidate_hosts(args.host.as_deref(), resolver_host, legacy_route.as_ref());
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
    println!("legacy resolution: {}", if resolved { "resolved" } else { "unresolved-used-first-candidate" });

    let mut cmd = Command::new("veer-connect");
    cmd.arg("shell").arg(&target_host).arg(port.to_string());
    cmd.env("VEER_SERVICE", &service);
    cmd.env("VEER_CALLER_AURAS", caller_auras.join(","));
    cmd.env("VEER_LEGACY_TARGET", format!("{}:{}", target_host, port));

    let status = cmd.status().context("launching veer-connect; ensure veer-connect is on PATH")?;
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
    let map: ResolveMap = toml::from_str(&text)
        .with_context(|| format!("parsing {}", map_path.display()))?;

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
            let (host, port) = endpoint_host_port(&rr.selected)
                .with_context(|| format!("resolver endpoint for {} does not contain host/port", service))?;
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
    let mut map: LegacyRouteMap = toml::from_str(&text)
        .with_context(|| format!("parsing {}", map_path.display()))?;

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

    bail!("cannot extract host:port from endpoint node={} transport={}", ep.node, ep.transport)
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
        bail!("{} address must use {}{{...}} type", what, address_type_code(expected));
    }
    Ok(canonical)
}

fn canonical_share_target(input: &str) -> Result<String> {
    let canonical = canonicalize(input)
        .map_err(|e| anyhow::anyhow!("invalid share target {}: {}", input, e))?;
    let parsed = VasAddress::parse(&canonical)
        .map_err(|e| anyhow::anyhow!("invalid share target {}: {}", input, e))?;
    match parsed.kind {
        AddressType::User | AddressType::Device | AddressType::Agent | AddressType::Service | AddressType::Node => Ok(canonical),
        _ => bail!("share target must be one of usr/dev/agt/svc/nod"),
    }
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
    let arr = auras.iter().cloned().map(toml::Value::String).collect::<Vec<_>>();
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
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let state: AuraState = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(state)
}

fn save_state(state: &AuraState) -> Result<()> {
    let path = state_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
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
        let out = canonical_typed(" SVC{Render,Company,Live}", AddressType::Service, "service").unwrap();
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
        assert_eq!(merged, vec!["aur{team,private,open}".to_string(), "aur{ops,private,open}".to_string()]);
    }

    #[test]
    fn candidate_hosts_prefers_explicit_then_dns_then_ips_deduped() {
        let route = LegacyRoute {
            dns: vec!["svc.example.local".into(), "svc.example.local".into()],
            ips: vec!["10.1.1.9".into(), "10.1.1.9".into()],
            port: 2232,
        };

        let out = build_candidate_hosts(Some("manual.example"), Some("resolved.example"), Some(&route));
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
}

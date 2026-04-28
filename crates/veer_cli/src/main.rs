use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use vas::{canonicalize, AddressType, VasAddress};

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
    host: String,
    #[arg(long)]
    port: u16,
    #[arg(long = "aura")]
    auras: Vec<String>,
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Aura { cmd } => cmd_aura(cmd),
        Cmd::Fold { cmd } => cmd_fold(cmd),
        Cmd::Connect(args) => cmd_connect(args),
    }
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

    println!("service: {}", service);
    if !caller_auras.is_empty() {
        println!("caller auras: {}", caller_auras.join(","));
    }
    println!("transport target: {}:{}", args.host, args.port);

    let mut cmd = Command::new("veer-connect");
    cmd.arg("shell").arg(&args.host).arg(args.port.to_string());
    cmd.env("VEER_SERVICE", &service);
    cmd.env("VEER_CALLER_AURAS", caller_auras.join(","));

    let status = cmd.status().context("launching veer-connect; ensure veer-connect is on PATH")?;
    if !status.success() {
        bail!("veer-connect shell failed with status: {}", status);
    }
    Ok(())
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
}

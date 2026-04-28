//! VeerOS Fold — host engine and CLI entrypoint.

mod cli;
mod engine;
mod manifest;
mod mobility;
mod state;
mod vm;

use anyhow::{Context, Result};
use clap::Parser;

use crate::cli::{Cli, Cmd, MobilityCmd, VmCmd};
use crate::engine::{Engine, PlatformEngine};
use crate::manifest::Manifest;
use crate::state::StateDir;

fn main() -> Result<()> {
    let args = Cli::parse();
    match args.cmd {
        Cmd::Spawn { manifest } => cmd_spawn(&manifest),
        Cmd::List => cmd_list(),
        Cmd::Logs { name, follow } => cmd_logs(&name, follow),
        Cmd::Stop { name } => cmd_stop(&name),
        Cmd::Rm { name, force } => cmd_rm(&name, force),
        Cmd::Vm { cmd } => cmd_vm(cmd),
        Cmd::Mobility { cmd } => cmd_mobility(cmd),
    }
}

fn cmd_spawn(manifest_path: &std::path::Path) -> Result<()> {
    let manifest = Manifest::load(manifest_path)?;
    let engine = PlatformEngine::new();
    let rec = engine.spawn(manifest)?;
    println!("fold {} spawned (pid {})", rec.name, rec.pid);
    println!("  log: {}", rec.log_path.display());
    Ok(())
}

fn cmd_list() -> Result<()> {
    let state = StateDir::open()?;
    let engine = PlatformEngine::new();
    let recs = state.list()?;
    if recs.is_empty() {
        println!("no folds");
        return Ok(());
    }
    println!("{:<20} {:>8} {:<10} {:<25} CMD", "NAME", "PID", "STATUS", "STARTED");
    for rec in recs {
        let status = if engine.is_alive(&rec) { "running" } else { "stopped" };
        let argv = std::iter::once(rec.manifest.cmd.as_str())
            .chain(rec.manifest.args.iter().map(|s| s.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{:<20} {:>8} {:<10} {:<25} {}",
            rec.name,
            rec.pid,
            status,
            rec.started_at.format("%Y-%m-%d %H:%M:%S"),
            argv,
        );
    }
    Ok(())
}

fn cmd_logs(name: &str, follow: bool) -> Result<()> {
    let state = StateDir::open()?;
    let rec = state.load(name).with_context(|| format!("no such fold: {name}"))?;
    let mut file = std::fs::File::open(&rec.log_path)
        .with_context(|| format!("opening {}", rec.log_path.display()))?;
    use std::io::{copy, stdout, Read, Seek, SeekFrom, Write};
    copy(&mut file, &mut stdout())?;
    if !follow {
        return Ok(());
    }
    // Poor-person's tail -f: poll every 250ms.
    let mut pos = file.seek(SeekFrom::End(0))?;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let len = file.metadata()?.len();
        if len > pos {
            file.seek(SeekFrom::Start(pos))?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            stdout().write_all(&buf)?;
            stdout().flush()?;
            pos = len;
        }
    }
}

fn cmd_stop(name: &str) -> Result<()> {
    let state = StateDir::open()?;
    let rec = state.load(name).with_context(|| format!("no such fold: {name}"))?;
    let engine = PlatformEngine::new();
    engine.stop(&rec)?;
    println!("sent SIGTERM to fold {} (pid {})", rec.name, rec.pid);
    Ok(())
}

fn cmd_rm(name: &str, force: bool) -> Result<()> {
    let state = StateDir::open()?;
    let rec = state.load(name).with_context(|| format!("no such fold: {name}"))?;
    let engine = PlatformEngine::new();
    if engine.is_alive(&rec) {
        if !force {
            anyhow::bail!("fold {} is still running — use `fold stop` first, or pass --force", name);
        }
        // Best-effort SIGKILL.
        let _ = engine.stop(&rec);
    }
    let _ = engine.cleanup(&rec);
    state.remove(name)?;
    println!("removed fold {}", name);
    Ok(())
}

fn cmd_vm(cmd: VmCmd) -> Result<()> {
    match cmd {
        VmCmd::Spawn { kernel, memory, arch, tap, name, vmm, user_ns, memory_cap, pids_max } => {
            let manifest = vm::build_manifest(vm::VmSpawnOpts {
                kernel, memory, arch, tap, name, vmm, user_ns, memory_cap, pids_max,
            })?;
            let engine = PlatformEngine::new();
            let rec = engine.spawn(manifest)?;
            println!("microVM {} spawned (pid {})", rec.name, rec.pid);
            println!("  arch   : {}", arch_arg(&rec));
            println!("  kernel : {}", kernel_arg(&rec));
            println!("  memory : {} MiB", memory_arg(&rec));
            if let Some(tap) = tap_arg(&rec) {
                println!("  tap    : {}", tap);
            }
            println!("  log    : {}", rec.log_path.display());
            println!();
            println!("Stream boot output:  fold logs {} --follow", rec.name);
            println!("Shut it down     :  fold stop {}", rec.name);
            Ok(())
        }
    }
}

fn cmd_mobility(cmd: MobilityCmd) -> Result<()> {
    match cmd {
        MobilityCmd::MigratePlan {
            name,
            target_zone,
            target_device,
            strategy,
            json,
        } => {
            let state = StateDir::open()?;
            let rec = state
                .load(&name)
                .with_context(|| format!("no such fold: {name}"))?;
            let plan = mobility::build_migration_plan(
                &rec,
                &target_zone,
                target_device.as_deref(),
                strategy.into(),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                println!("migration plan for fold {}", plan.fold);
                println!("  strategy      : {:?}", plan.strategy);
                println!("  target zone   : {}", plan.target_zone);
                if let Some(dev) = &plan.target_device {
                    println!("  target device : {}", dev);
                }
                println!("  checkpoint    : {}", plan.requires_checkpoint);
                println!("  steps:");
                for step in &plan.steps {
                    println!("    - {}", step);
                }
            }
            Ok(())
        }
        MobilityCmd::ReplicatePlan {
            name,
            targets,
            consistency,
            json,
        } => {
            let state = StateDir::open()?;
            let rec = state
                .load(&name)
                .with_context(|| format!("no such fold: {name}"))?;
            let plan = mobility::build_replication_plan(&rec, &targets, consistency.into())?;
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            } else {
                println!("replication plan for fold {}", plan.fold);
                println!("  consistency : {:?}", plan.consistency);
                println!("  targets:");
                for t in &plan.targets {
                    println!(
                        "    - zone={} device={} replica={}",
                        t.zone,
                        t.device.as_deref().unwrap_or("<auto>"),
                        t.replica_name
                    );
                }
                println!("  steps:");
                for step in &plan.steps {
                    println!("    - {}", step);
                }
            }
            Ok(())
        }
    }
}

fn arch_arg(rec: &state::FoldRecord) -> String {
    let mut it = rec.manifest.args.iter();
    while let Some(a) = it.next() {
        if a == "--arch" {
            if let Some(v) = it.next() { return v.clone(); }
        }
    }
    "x86_64".into()
}

/// Pick the `--kernel` value out of a fold record's argv for pretty-print.
fn kernel_arg(rec: &state::FoldRecord) -> String {
    let mut it = rec.manifest.args.iter();
    while let Some(a) = it.next() {
        if a == "--kernel" {
            if let Some(v) = it.next() { return v.clone(); }
        }
    }
    "<unknown>".into()
}

fn memory_arg(rec: &state::FoldRecord) -> String {
    let mut it = rec.manifest.args.iter();
    while let Some(a) = it.next() {
        if a == "--memory" {
            if let Some(v) = it.next() { return v.clone(); }
        }
    }
    "<unknown>".into()
}

fn tap_arg(rec: &state::FoldRecord) -> Option<String> {
    let mut it = rec.manifest.args.iter();
    while let Some(a) = it.next() {
        if a == "--tap" {
            return it.next().cloned();
        }
    }
    None
}

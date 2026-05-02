use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use super::Engine;
use crate::manifest::Manifest;
use crate::state::{FoldRecord, StateDir};

pub struct MacosEngine;

impl MacosEngine {
    pub fn new() -> Self {
        Self
    }
}

impl Engine for MacosEngine {
    fn spawn(&self, manifest: Manifest) -> Result<FoldRecord> {
        let state = StateDir::open()?;
        anyhow::ensure!(
            !state.record_path(&manifest.name).exists(),
            "fold {:?} already exists; use `fold rm` first",
            manifest.name
        );

        let log_path = state.log_path(&manifest.name);
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
            .with_context(|| format!("opening log {}", log_path.display()))?;
        let log_file_err = log_file
            .try_clone()
            .with_context(|| format!("cloning log {}", log_path.display()))?;

        let mut cmd = build_command(&manifest)?;
        cmd.stdin(Stdio::null())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_file_err));

        if !manifest.env.is_empty() {
            cmd.envs(manifest.env.clone());
        }
        if let Some(workdir) = &manifest.workdir {
            cmd.current_dir(workdir);
        }

        let child = cmd
            .spawn()
            .with_context(|| format!("spawning '{}' for fold {}", manifest.cmd, manifest.name))?;

        let rec = FoldRecord {
            name: manifest.name.clone(),
            pid: child.id() as i32,
            started_at: Utc::now(),
            manifest,
            log_path,
            exit_status: None,
        };
        state.save(&rec)?;
        Ok(rec)
    }

    fn stop(&self, rec: &FoldRecord) -> Result<()> {
        let rc = unsafe { libc::kill(rec.pid, libc::SIGTERM) };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        bail!("kill({}): {}", rec.pid, err);
    }

    fn is_alive(&self, rec: &FoldRecord) -> bool {
        let rc = unsafe { libc::kill(rec.pid, 0) };
        if rc == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        err.raw_os_error() == Some(libc::EPERM)
    }

    fn cleanup(&self, rec: &FoldRecord) -> Result<()> {
        let profile = sandbox_profile_path(&rec.name);
        if profile.exists() {
            let _ = fs::remove_file(profile);
        }
        Ok(())
    }
}

fn build_command(manifest: &Manifest) -> Result<Command> {
    if should_use_sandbox(manifest) {
        let profile_path = write_sandbox_profile(&manifest.name)?;
        let mut cmd = Command::new("sandbox-exec");
        cmd.arg("-f")
            .arg(profile_path)
            .arg(&manifest.cmd)
            .args(&manifest.args);
        return Ok(cmd);
    }

    let mut cmd = Command::new(&manifest.cmd);
    cmd.args(&manifest.args);
    Ok(cmd)
}

fn should_use_sandbox(manifest: &Manifest) -> bool {
    matches!(
        manifest.seccomp.as_ref().map(|s| s.profile.as_str()),
        Some("default")
    )
}

fn sandbox_profile_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("fold-{}.sb", name))
}

fn write_sandbox_profile(name: &str) -> Result<PathBuf> {
    let path = sandbox_profile_path(name);
    // Keep this permissive for compatibility; later phases can tighten policy.
    let profile = "(version 1)\n(allow default)\n";
    fs::write(&path, profile)
        .with_context(|| format!("writing sandbox profile {}", path.display()))?;
    Ok(path)
}

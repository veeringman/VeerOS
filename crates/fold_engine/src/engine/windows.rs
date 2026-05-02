use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::process::{Command, Stdio};

use super::Engine;
use crate::manifest::Manifest;
use crate::state::{FoldRecord, StateDir};

pub struct WindowsEngine;

impl WindowsEngine {
    pub fn new() -> Self {
        Self
    }
}

impl Engine for WindowsEngine {
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

        let mut cmd = Command::new(&manifest.cmd);
        cmd.args(&manifest.args)
            .stdin(Stdio::null())
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
        if !self.is_alive(rec) {
            return Ok(());
        }

        let status = Command::new("taskkill")
            .arg("/PID")
            .arg(rec.pid.to_string())
            .arg("/T")
            .status()
            .context("invoking taskkill")?;

        if status.success() || !self.is_alive(rec) {
            return Ok(());
        }

        bail!("taskkill failed for pid {}", rec.pid)
    }

    fn is_alive(&self, rec: &FoldRecord) -> bool {
        // Prefer pwsh when available; fallback to Windows PowerShell.
        let probe = format!(
            "$p = Get-Process -Id {} -ErrorAction SilentlyContinue; if ($null -ne $p) {{ exit 0 }} else {{ exit 1 }}",
            rec.pid
        );

        let status = Command::new("pwsh")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(&probe)
            .status();
        match status {
            Ok(s) => return s.success(),
            Err(_) => {}
        }

        Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(probe)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

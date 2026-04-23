//! On-disk fold state registry.
//!
//! Stored at `$XDG_STATE_HOME/veeros/fold/` (falling back to
//! `~/.local/state/veeros/fold/`) with one JSON record per fold and one
//! log file per fold.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::manifest::Manifest;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoldRecord {
    pub name: String,
    pub pid: i32,
    pub started_at: DateTime<Utc>,
    pub manifest: Manifest,
    /// Path to the captured stdout+stderr log.
    pub log_path: PathBuf,
    /// Populated by `fold stop`/waitpid when the fold has exited.
    #[serde(default)]
    pub exit_status: Option<i32>,
}

pub struct StateDir {
    root: PathBuf,
}

impl StateDir {
    pub fn open() -> Result<Self> {
        let root = state_root()?;
        fs::create_dir_all(&root)
            .with_context(|| format!("creating state dir {}", root.display()))?;
        Ok(Self { root })
    }

    pub fn record_path(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.json"))
    }

    pub fn log_path(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.log"))
    }

    pub fn save(&self, rec: &FoldRecord) -> Result<()> {
        let path = self.record_path(&rec.name);
        let text = serde_json::to_string_pretty(rec)?;
        fs::write(&path, text)
            .with_context(|| format!("writing record {}", path.display()))?;
        Ok(())
    }

    pub fn load(&self, name: &str) -> Result<FoldRecord> {
        let path = self.record_path(name);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("reading record {}", path.display()))?;
        Ok(serde_json::from_str(&text)?)
    }

    pub fn list(&self) -> Result<Vec<FoldRecord>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(text) = fs::read_to_string(&path) {
                    if let Ok(rec) = serde_json::from_str::<FoldRecord>(&text) {
                        out.push(rec);
                    }
                }
            }
        }
        out.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        Ok(out)
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let rec = self.record_path(name);
        let log = self.log_path(name);
        if rec.exists() { fs::remove_file(&rec)?; }
        if log.exists() { fs::remove_file(&log)?; }
        Ok(())
    }
}

fn state_root() -> Result<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_STATE_HOME") {
        if !xdg.is_empty() {
            return Ok(PathBuf::from(xdg).join("veeros").join("fold"));
        }
    }
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".local/state/veeros/fold"))
}

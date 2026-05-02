//! Fold manifest — declarative description of a fold (secure compute envelope).
//!
//! Format: TOML. Minimal MVP schema; will grow to cover capabilities,
//! resource limits, network identity, signed attestations, etc.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use vas::{canonicalize, AddressType, VasAddress};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Unique fold name (used as handle for list/logs/stop/rm).
    pub name: String,

    /// Command to exec inside the fold.
    pub cmd: String,

    /// Arguments passed to `cmd` (argv[1..]).
    #[serde(default)]
    pub args: Vec<String>,

    /// Aura memberships attached to this fold.
    ///
    /// Each entry must be a valid Aura VAS address (`aur{...}`).
    /// Values are canonicalized on load.
    #[serde(default)]
    pub auras: Vec<String>,

    /// Environment variables. If empty, inherits a minimal set from host.
    #[serde(default)]
    pub env: BTreeMap<String, String>,

    /// Optional rootfs directory. If set on Linux, fold pivot_roots into it.
    /// If None, fold shares host rootfs (still isolated via other namespaces).
    #[serde(default)]
    pub rootfs: Option<PathBuf>,

    /// Hostname inside the fold's UTS namespace.
    #[serde(default)]
    pub hostname: Option<String>,

    /// Working directory for `cmd`. Defaults to "/".
    #[serde(default)]
    pub workdir: Option<PathBuf>,

    /// Linux namespace toggles. Defaults produce a standard isolated fold.
    #[serde(default)]
    pub namespaces: Namespaces,

    /// cgroups v2 resource limits. If unset, fold inherits caller's cgroup.
    #[serde(default)]
    pub limits: Option<Limits>,

    /// seccomp BPF syscall filter. If `None`, no filter is installed.
    #[serde(default)]
    pub seccomp: Option<Seccomp>,
}

/// cgroups v2 limits. Written as the corresponding controller files under the
/// fold's delegated cgroup directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Limits {
    /// `cpu.max` contents. Either `"max"` or `"<quota_us> <period_us>"`.
    /// Example: `"50000 100000"` = 0.5 CPU.
    #[serde(default)]
    pub cpu_max: Option<String>,
    /// `memory.max` in bytes. Example: `268435456` = 256 MiB.
    #[serde(default)]
    pub memory_max: Option<u64>,
    /// `memory.swap.max` in bytes.
    #[serde(default)]
    pub memory_swap_max: Option<u64>,
    /// `pids.max` — maximum number of processes inside the fold.
    #[serde(default)]
    pub pids_max: Option<u64>,
    /// Parent cgroup directory (must be pre-delegated). The fold creates a
    /// child `fold-<name>` underneath. Defaults to auto-detect via
    /// `/proc/self/cgroup`.
    #[serde(default)]
    pub cgroup_parent: Option<PathBuf>,
}

/// seccomp syscall filter profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Seccomp {
    /// Named profile. `"default"` installs a curated denylist of dangerous
    /// syscalls on top of default-allow. `"none"` disables.
    #[serde(default = "default_profile")]
    pub profile: String,
    /// Extra syscall names to deny (by Linux syscall name, e.g. `"mount"`).
    #[serde(default)]
    pub deny: Vec<String>,
}

fn default_profile() -> String {
    "default".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespaces {
    #[serde(default = "t")]
    pub pid: bool,
    #[serde(default = "t")]
    pub mount: bool,
    #[serde(default = "t")]
    pub uts: bool,
    #[serde(default = "t")]
    pub ipc: bool,
    #[serde(default = "t")]
    pub net: bool,
    /// User namespace — enables rootless operation when true. Defaults off
    /// because it requires careful uid_map/gid_map setup (MVP: off).
    #[serde(default)]
    pub user: bool,
}

impl Default for Namespaces {
    fn default() -> Self {
        Self {
            pid: true,
            mount: true,
            uts: true,
            ipc: true,
            net: true,
            user: false,
        }
    }
}

fn t() -> bool {
    true
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading manifest {}", path.display()))?;
        let mut m: Manifest = toml::from_str(&text)
            .with_context(|| format!("parsing manifest {}", path.display()))?;
        m.resolve_relative_paths(path);
        m.normalize_auras()?;
        m.validate()?;
        Ok(m)
    }

    fn normalize_auras(&mut self) -> Result<()> {
        let mut out = Vec::with_capacity(self.auras.len());
        let mut seen = BTreeSet::new();

        for raw in &self.auras {
            let canonical = canonicalize(raw)
                .map_err(|e| anyhow::anyhow!("manifest: invalid aura address '{raw}': {e}"))?;
            let addr = VasAddress::parse(&canonical)
                .map_err(|e| anyhow::anyhow!("manifest: invalid aura address '{raw}': {e}"))?;
            anyhow::ensure!(
                addr.kind == AddressType::Aura,
                "manifest: aura entry must use aur{{...}} type, got '{}': {}",
                raw,
                canonical
            );

            if seen.insert(canonical.clone()) {
                out.push(canonical);
            }
        }

        self.auras = out;
        Ok(())
    }

    fn resolve_relative_paths(&mut self, manifest_path: &Path) {
        let Some(base_dir) = manifest_path.parent() else {
            return;
        };

        if let Some(rootfs) = self.rootfs.as_mut() {
            if rootfs.is_relative() {
                *rootfs = base_dir.join(&*rootfs);
            }
        }

        if let Some(workdir) = self.workdir.as_mut() {
            if workdir.is_relative() {
                *workdir = base_dir.join(&*workdir);
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(!self.name.is_empty(), "manifest: name must not be empty");
        anyhow::ensure!(
            self.name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "manifest: name must match [A-Za-z0-9_-]"
        );
        anyhow::ensure!(!self.cmd.is_empty(), "manifest: cmd must not be empty");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_normalizes_and_dedups_auras() {
        let mut m: Manifest = toml::from_str(
            r#"
name = "a"
cmd = "/bin/true"
auras = [" AUR{Design,PRIVATE,Open} ", "aur{design,private,open}", "aur{company,private,open}"]
"#,
        )
        .unwrap();

        m.normalize_auras().unwrap();
        assert_eq!(
            m.auras,
            vec![
                "aur{design,private,open}".to_string(),
                "aur{company,private,open}".to_string(),
            ]
        );
    }

    #[test]
    fn manifest_rejects_non_aura_entry() {
        let mut m: Manifest = toml::from_str(
            r#"
name = "a"
cmd = "/bin/true"
auras = ["svc{render,company,live}"]
"#,
        )
        .unwrap();

        let err = m.normalize_auras().unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("aura entry must use aur"));
    }
}

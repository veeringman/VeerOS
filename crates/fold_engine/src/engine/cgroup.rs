//! Minimal cgroups v2 driver for fold_engine.
//!
//! Creates a child cgroup under a delegated parent, writes limits, and moves
//! a target PID into it. If the system is not cgroup v2 or the parent is not
//! writable, callers get a clear error — we never silently skip limits.
//!
//! Typical layout:
//!   `/sys/fs/cgroup/fold.slice/fold-<name>/`     (root, via systemd slice)
//!   `/sys/fs/cgroup/user.slice/user-<uid>.slice/user@<uid>.service/app.slice/fold-<name>.slice/`
//!     (rootless, requires delegation — see `systemd.resource-control`(5))

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::manifest::Limits;

pub struct CgroupHandle {
    dir: PathBuf,
    /// If true, `cleanup()` will `rmdir` this cgroup.
    #[allow(dead_code)] // reserved for future `fold stats` / drop-based cleanup
    owns: bool,
}

#[allow(dead_code)]
impl CgroupHandle {
    pub fn dir(&self) -> &Path { &self.dir }

    /// Remove the cgroup directory. No-op if we did not create it or if the
    /// directory still contains processes.
    pub fn cleanup(self) {
        if !self.owns { return; }
        // `rmdir` only succeeds when the cgroup is empty; ignore EBUSY etc.
        let _ = fs::remove_dir(&self.dir);
    }
}

/// Create `<parent>/fold-<name>/`, write limits, and return a handle whose
/// `dir()` is the new cgroup path. The caller must still write the target
/// PID into `<dir>/cgroup.procs`.
pub fn create(name: &str, limits: &Limits) -> Result<CgroupHandle> {
    let parent = resolve_parent(limits.cgroup_parent.as_deref())?;
    ensure_v2(&parent)?;

    let dir = parent.join(format!("fold-{name}"));
    fs::create_dir_all(&dir)
        .with_context(|| format!("creating cgroup {}", dir.display()))?;

    // Enable required controllers in the parent *before* writing our limits.
    // This is a best-effort: if the parent already has them enabled (the
    // common case for systemd-delegated slices) the write is a no-op.
    let _ = enable_controllers(&parent);

    if let Some(cpu) = &limits.cpu_max {
        write_file(&dir.join("cpu.max"), cpu)
            .context("writing cpu.max")?;
    }
    if let Some(mem) = limits.memory_max {
        write_file(&dir.join("memory.max"), &mem.to_string())
            .context("writing memory.max")?;
    }
    if let Some(swap) = limits.memory_swap_max {
        // memory.swap.max is unavailable on kernels without swap accounting;
        // don't fail hard on ENOENT.
        let p = dir.join("memory.swap.max");
        if p.exists() {
            write_file(&p, &swap.to_string())
                .context("writing memory.swap.max")?;
        }
    }
    if let Some(pids) = limits.pids_max {
        write_file(&dir.join("pids.max"), &pids.to_string())
            .context("writing pids.max")?;
    }

    Ok(CgroupHandle { dir, owns: true })
}

/// Move the given PID into the cgroup.
pub fn attach_pid(handle: &CgroupHandle, pid: i32) -> Result<()> {
    let procs = handle.dir.join("cgroup.procs");
    write_file(&procs, &pid.to_string())
        .with_context(|| format!("writing pid {pid} to {}", procs.display()))?;
    Ok(())
}

/// Best-effort removal of a fold's cgroup (used by `fold rm`). Missing
/// directories and non-empty cgroups are silently ignored.
pub fn remove(name: &str, limits: &Limits) -> Result<()> {
    let parent = resolve_parent(limits.cgroup_parent.as_deref())?;
    let dir = parent.join(format!("fold-{name}"));
    let _ = fs::remove_dir(&dir);
    Ok(())
}

// ─── helpers ────────────────────────────────────────────────────────────────

fn write_file(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents)
        .with_context(|| format!("writing {} = {:?}", path.display(), contents))?;
    Ok(())
}

fn ensure_v2(parent: &Path) -> Result<()> {
    // The v2 mount point always exposes `cgroup.controllers`. Its absence
    // means we're either on v1-only or the path is not a cgroup mount.
    let marker = find_v2_root(parent).unwrap_or_else(|| parent.to_path_buf());
    if !marker.join("cgroup.controllers").exists() {
        bail!(
            "cgroup v2 not available at {} (missing cgroup.controllers). \
             Mount cgroup2 or run with --no-limits.",
            marker.display()
        );
    }
    Ok(())
}

fn find_v2_root(start: &Path) -> Option<PathBuf> {
    let mut p = start.to_path_buf();
    loop {
        if p.join("cgroup.controllers").exists() {
            return Some(p);
        }
        match p.parent() {
            Some(parent) if parent != p => p = parent.to_path_buf(),
            _ => return None,
        }
    }
}

fn enable_controllers(parent: &Path) -> Result<()> {
    let sctl = parent.join("cgroup.subtree_control");
    if !sctl.exists() { return Ok(()); }
    // Enable the controllers we care about. If a controller isn't available
    // in this subtree, the kernel will reject the write — ignore the error
    // per controller.
    for c in ["cpu", "memory", "pids"] {
        let _ = fs::write(&sctl, format!("+{c}"));
    }
    Ok(())
}

/// Resolve the parent cgroup directory: either an explicit override from the
/// manifest, or auto-detected from `/proc/self/cgroup` + the v2 mount root.
fn resolve_parent(override_: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = override_ {
        return Ok(p.to_path_buf());
    }
    // Parse our own cgroup v2 membership line: "0::/user.slice/...".
    let text = fs::read_to_string("/proc/self/cgroup")
        .context("reading /proc/self/cgroup")?;
    let rel = text.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .context("no cgroup v2 line (0::) in /proc/self/cgroup — v1 system?")?
        .trim();
    // v2 root. Standard distros mount it at /sys/fs/cgroup.
    let root = Path::new("/sys/fs/cgroup");
    if !root.join("cgroup.controllers").exists() {
        bail!("cgroup v2 root not mounted at /sys/fs/cgroup");
    }
    // Strip leading slash so join() keeps root.
    let rel = rel.trim_start_matches('/');
    Ok(root.join(rel))
}

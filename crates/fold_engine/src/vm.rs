//! `fold vm` — synthesize a Fold manifest that runs the `veer-vm` microVMM
//! inside a fresh namespace/cgroup/seccomp envelope.
//!
//! No external manifest file needed: the caller provides a kernel path and
//! a few knobs; we build a `Manifest` in memory and hand it to the Linux
//! engine exactly like any other fold.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::cli::VmArchArg;
use crate::manifest::{Limits, Manifest, Namespaces, Seccomp};

pub struct VmSpawnOpts {
    pub kernel: PathBuf,
    pub memory: usize,
    pub arch: VmArchArg,
    pub tap: Option<String>,
    pub name: Option<String>,
    pub vmm: Option<PathBuf>,
    pub user_ns: bool,
    pub memory_cap: Option<u64>,
    pub pids_max: Option<u64>,
}

pub fn build_manifest(opts: VmSpawnOpts) -> Result<Manifest> {
    // ── 1. Resolve absolute boot image path ──────────────────
    let kernel = opts.kernel.canonicalize()
        .with_context(|| format!("boot image path not found: {}", opts.kernel.display()))?;

    // ── 2. Locate `veer-vm` binary ───────────────────────────
    let vmm = match opts.vmm {
        Some(p) => p.canonicalize()
            .with_context(|| format!("--vmm path not found: {}", p.display()))?,
        None => locate_veer_vm().context("could not locate `veer-vm` binary")?,
    };
    if !vmm.is_file() {
        bail!("veer-vm binary is not a regular file: {}", vmm.display());
    }

    // ── 3. Fold name ─────────────────────────────────────────
    let name = opts.name.unwrap_or_else(random_vm_name);

    // ── 4. cgroup limits (opt-in) ────────────────────────────
    // cgroups v2 rejects writes to `memory.max` / `pids.max` unless the
    // parent cgroup has the respective controllers in `subtree_control`.
    // On a default systemd-run --user --scope session the memory
    // controller often isn't delegated, so we only create a fold cgroup
    // when the user explicitly asks for at least one limit.
    let limits = if opts.memory_cap.is_some() || opts.pids_max.is_some() {
        Some(Limits {
            cpu_max: None,
            memory_max: opts.memory_cap,
            memory_swap_max: None,
            pids_max: opts.pids_max,
            cgroup_parent: None, // auto-detect
        })
    } else {
        None
    };

    // ── 5. seccomp profile ───────────────────────────────────
    // Default fold denylist is already compatible with KVM (it does not
    // block `ioctl`, `openat`, `mmap`). Use it as-is.
    let seccomp = Seccomp {
        profile: "default".into(),
        deny: Vec::new(),
    };

    // ── 6. Namespaces ────────────────────────────────────────
    let namespaces = Namespaces {
        pid: true, mount: true, uts: true, ipc: true,
        // NET namespace on = guest is unreachable from host network and
        // vice versa (intentional — this is a VeerOS microVM, isolation
        // first). Once we add virtio-net, we'll open a tap/veth into it.
        net: true,
        user: opts.user_ns,
    };

    // ── 7. Environment — minimal passthrough ─────────────────
    let mut env = BTreeMap::new();
    env.insert("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into());
    env.insert("TERM".into(), std::env::var("TERM").unwrap_or_else(|_| "dumb".into()));

    // ── 8. Arguments to veer-vm ──────────────────────────────
    let mut args = vec![
        "--arch".into(), opts.arch.as_cli_value().into(),
        "--kernel".into(), kernel.to_string_lossy().into_owned(),
        "--memory".into(), opts.memory.to_string(),
    ];
    if let Some(tap) = opts.tap {
        args.push("--tap".into());
        args.push(tap);
    }

    Ok(Manifest {
        name,
        cmd: vmm.to_string_lossy().into_owned(),
        args,
        env,
        rootfs: None, // share host rootfs — /dev/kvm must stay visible
        hostname: Some("veeros-vm".into()),
        workdir: Some(PathBuf::from("/")),
        namespaces,
        limits,
        seccomp: Some(seccomp),
    })
}

/// Hunt for the `veer-vm` binary in (in order):
///   1. `$CARGO_WORKSPACE/target/debug/veer-vm`
///   2. `$CARGO_WORKSPACE/target/release/veer-vm`
///   3. `$PATH`
fn locate_veer_vm() -> Result<PathBuf> {
    let host_target = std::env::var("HOST").ok().or_else(detect_rust_host_target);

    // (1) + (2): walk up from CWD looking for a `target/` sibling of a
    // Cargo.toml with `veer_vm` in the workspace.
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            let mut candidates = vec![
                "target/debug/veer-vm".to_string(),
                "target/release/veer-vm".to_string(),
            ];

            if let Some(host) = &host_target {
                candidates.push(format!("target/{host}/debug/veer-vm"));
                candidates.push(format!("target/{host}/release/veer-vm"));
            }

            for sub in candidates {
                let p = ancestor.join(sub);
                if p.is_file() {
                    return p.canonicalize()
                        .with_context(|| format!("canonicalizing {}", p.display()));
                }
            }
        }
    }
    // (3): PATH lookup.
    if let Some(p) = which_in_path("veer-vm") {
        return Ok(p);
    }
    bail!(
        "`veer-vm` not found. Build it first:\n\
         \n    cargo build -p veer_vm\n\
         \n or pass an explicit --vmm <path>."
    );
}

fn detect_rust_host_target() -> Option<String> {
    let out = std::process::Command::new("rustc")
        .args(["-vV"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("host: ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn which_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn random_vm_name() -> String {
    // 6 hex digits from nanosecond time — good enough for a user-facing
    // handle; uniqueness within a single host's fold state dir.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("veeros-vm-{:06x}", nanos & 0xFF_FFFF)
}

#[allow(dead_code)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn mk_test_files() -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!(
            "fold-engine-vm-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&base).unwrap();

        let kernel = base.join("kernel.elf");
        let vmm = base.join("veer-vm");
        fs::write(&kernel, b"k").unwrap();
        fs::write(&vmm, b"v").unwrap();
        (base, kernel, vmm)
    }

    #[test]
    fn build_manifest_includes_tap_when_provided() {
        let (base, kernel, vmm) = mk_test_files();
        let manifest = build_manifest(VmSpawnOpts {
            kernel,
            memory: 128,
            arch: VmArchArg::Riscv32,
            tap: Some("tap0".to_string()),
            name: Some("vm-test".to_string()),
            vmm: Some(vmm),
            user_ns: true,
            memory_cap: None,
            pids_max: None,
        })
        .unwrap();

        assert!(manifest
            .args
            .windows(2)
            .any(|w| w[0] == "--tap" && w[1] == "tap0"));

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn build_manifest_omits_tap_when_not_provided() {
        let (base, kernel, vmm) = mk_test_files();
        let manifest = build_manifest(VmSpawnOpts {
            kernel,
            memory: 128,
            arch: VmArchArg::Riscv32,
            tap: None,
            name: Some("vm-test".to_string()),
            vmm: Some(vmm),
            user_ns: true,
            memory_cap: None,
            pids_max: None,
        })
        .unwrap();

        assert!(!manifest.args.iter().any(|a| a == "--tap"));

        let _ = fs::remove_dir_all(base);
    }
}

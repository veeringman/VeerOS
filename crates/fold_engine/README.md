# VeerOS Fold

Cross-platform secure compute envelope for VeerOS.

A **Fold** is a lightweight, secure, rapidly-deployable runtime unit — not a
container, not a VM. This crate is the *host-side* engine and CLI (`fold`)
used to spawn and manage folds on developer workstations and VeerOS fabric
nodes.

## Status

Phase 8B+. Linux backend now supports:
- PID / mount / UTS / IPC / net namespaces (required), user namespace (optional, rootless)
- rootless spawn via user-ns + uid_map/gid_map (no sudo needed on kernels with `kernel.unprivileged_userns_clone=1`)
- cgroups v2 resource limits (`cpu.max`, `memory.max`, `memory.swap.max`, `pids.max`)
- seccomp BPF syscall filter with a curated default denylist + per-fold extensions
- structured launcher→parent error reporting pipe

macOS and Windows backends are stubs.

## Quick start

```bash
cargo build -p fold_engine --release

# Rootless (no sudo), no resource limits
./target/release/fold spawn --manifest crates/fold_engine/examples/rootless.toml

# Full-isolation + seccomp + cgroup limits (needs root or a delegated cgroup)
sudo ./target/release/fold spawn --manifest crates/fold_engine/examples/hardened.toml

# Declarative veer-vm launch (equivalent to `fold vm spawn`)
./target/debug/fold spawn --manifest crates/fold_engine/examples/veer-vm.toml

./target/release/fold list
./target/release/fold logs rootless
./target/release/fold stop rootless
./target/release/fold rm rootless
```

For rootless cgroup limits, wrap the spawn in a delegated scope:

```bash
systemd-run --user --scope --property=Delegate=yes -- \
  ./target/release/fold spawn --manifest crates/fold_engine/examples/rootless.toml
```

The declarative VM example in `crates/fold_engine/examples/veer-vm.toml`
assumes you are running from the workspace root and have already built both
`veer-vm` and the x86_64 `kernel-qemu-pc` guest image. Adjust `cmd`,
`--kernel`, and `[namespaces].user` to match your host setup. The checked-in
example enables `user = true` so it can launch rootlessly on hosts that allow
unprivileged user namespaces and expose `/dev/kvm` to the current user.
Relative `workdir` and `rootfs` manifest paths are resolved from the manifest
file's own directory.

## Manifest (TOML)

```toml
name = "hello"
cmd  = "/bin/sh"
args = ["-c", "echo hi from fold; sleep 3"]
hostname = "fold-hello"

[env]
PATH = "/usr/bin:/bin"

[namespaces]
pid = true
mount = true
uts = true
ipc = true
net = true
user = false    # set true for rootless

[seccomp]
profile = "default"   # or "none"
deny    = ["personality"]   # extra syscall names to deny

[limits]
cpu_max    = "50000 100000"   # quota/period microseconds — 0.5 CPU
memory_max = 268435456        # bytes
pids_max   = 64
```

## Roadmap

- [x] cgroups v2 resource limits
- [x] seccomp default-deny profile
- [x] Rootless / user-namespace mode with uid_map/gid_map
- [ ] Ed25519-signed manifests (via VeerOS `crypto` crate)
- [ ] veth pair + optional bridge attachment
- [ ] overlayfs rootfs layering
- [ ] GPU / accelerator attach (UAI bridge)
- [ ] Snapshot / migration / refold
- [ ] macOS backend (sandbox_init + launchd)
- [ ] Windows backend (Job Objects + Windows containers)

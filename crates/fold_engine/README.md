
# VeerOS Fold

Cross-platform secure compute envelope for VeerOS.

A **Fold** is a lightweight, secure, rapidly-deployable runtime unit — not a container, not a VM. This crate is the *host-side* engine and CLI (`fold`) used to spawn and manage folds on Linux, macOS, and (planned) Windows hosts, as well as on VeerOS fabric nodes.

**Supported host platforms:**
- **Linux**: Full-featured (namespaces, cgroups, seccomp, rootless, VM integration)
- **macOS**: Stub (planned, basic CLI only)
- **Windows**: Stub (planned)

**macOS remains stubbed; Windows provides process lifecycle backend.**

**Supported guest types:**
- Native processes (Linux)
- VeerOS VMs (via `veer-vm`, supports x86_64 and riscv32 guests)

**Backend selection is automatic based on host OS and manifest.**

## Status — Multi-platform, multi-arch

Phase 8B+. Linux backend now supports:
- PID / mount / UTS / IPC / net namespaces (required), user namespace (optional, rootless)
- rootless spawn via user-ns + uid_map/gid_map (no sudo needed on kernels with `kernel.unprivileged_userns_clone=1`)
- cgroups v2 resource limits (`cpu.max`, `memory.max`, `memory.swap.max`, `pids.max`)
- seccomp BPF syscall filter with a curated default denylist + per-fold extensions
- structured launcher→parent error reporting pipe
- VeerOS VM launch (x86_64, riscv32 via QEMU TCG fallback)

**macOS backend is stubbed; Windows has a functional process backend.**

## Platform backend status

| Host OS   | Backend         | Guest type         | Status         |
|-----------|-----------------|--------------------|----------------|
| Linux     | namespaces/cg   | native process     | Stable         |
| Linux     | veer-vm (KVM)   | x86_64 VM          | Stable         |
| Linux     | veer-vm (KVM)   | riscv32 VM         | Stable         |
| Linux     | veer-vm (QEMU)  | riscv32 VM         | Stable         |
| macOS     | stub            | (planned)          | Not implemented|
| Windows   | process backend | host process / VM wrapper | Functional |

**Note:** For riscv32 VMs on non-riscv64 hosts, QEMU TCG is required (`qemu-system-riscv32`).

## Quick start

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

# Mobility planning (Phase D)
./target/release/fold mobility migrate-plan rootless --target-zone zone-east --target-device dev{edge-a,corp,active}
./target/release/fold mobility replicate-plan rootless --target zone-east:dev{edge-a,corp,active} --target zone-west
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

`fold vm spawn --kernel ...` now accepts either the direct Multiboot ELF build
output or the GRUB ISO at `build/veeros.iso`; both are passed through to
`veer-vm`, which extracts `/boot/kernel.elf` from the ISO when needed.

`fold vm spawn` also supports explicit guest architecture selection:

```bash
# x86_64 path (default)
./target/debug/fold vm spawn --arch x86_64 --kernel ./build/veeros.iso --memory 128

# riscv32 path (ESP32-C6 qemu distribution ELF)
./target/debug/fold vm spawn --arch riscv32 \
  --kernel ./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6 \
  --memory 128
```

Use `scripts/build-veer-vm-esp32c6.sh` to build the riscv32 kernel image plus
the host `veer-vm` and `fold` binaries together.

## Manifest (TOML)

```toml
name = "hello"
cmd  = "/bin/sh"
args = ["-c", "echo hi from fold; sleep 3"]
auras = ["aur{apps,dev,private}", "aur{observability,dev,private}"]
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

`auras` binds the fold to one or more Aura memberships using canonical VAS
Aura addresses (`aur{...}`). Entries are canonicalized and duplicates are
removed when the manifest is loaded.

## Roadmap

- [x] cgroups v2 resource limits
- [x] seccomp default-deny profile
- [x] Rootless / user-namespace mode with uid_map/gid_map
- [ ] Ed25519-signed manifests (via VeerOS `crypto` crate)
- [ ] veth pair + optional bridge attachment
- [ ] overlayfs rootfs layering
- [ ] GPU / accelerator attach (UAI bridge)
- [~] Snapshot / migration / refold
  - [x] Migration/replication plan generation (`fold mobility migrate-plan|replicate-plan`)
  - [ ] Runtime checkpoint transfer + restore execution path
- [ ] macOS backend (sandbox_init + launchd)
- [ ] Windows backend (Job Objects + Windows containers)

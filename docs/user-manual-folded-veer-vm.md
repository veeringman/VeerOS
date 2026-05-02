# User Manual: Working with Folded veer-vm (2026)

## Introduction
This guide explains how to use VeerOS's Fold Engine to securely run and manage `veer-vm` microVMMs. "Folded veer-vm" means running the `veer-vm` KVM-based microVMM inside a Fold, gaining resource isolation, security, and lifecycle management.

---

## 1. Concepts
- **Fold**: A secure, lightweight runtime envelope (not a container, not a VM) managed by the Fold Engine.
- **veer-vm**: A microVMM (like Firecracker) that boots VeerOS kernels using KVM.
- **Folded veer-vm**: `veer-vm` launched as a Fold, combining microVM isolation with Fold's resource controls and auditability.

---

## 2. Prerequisites
- Linux host with KVM support (`/dev/kvm`)
- VeerOS workspace built (`cargo build`)
- Root or delegated cgroup privileges for full isolation (rootless mode supported)

### Windows scope note
- This manual targets Linux Fold Engine workflows.
- On Windows, ESP32C6 guest tests currently use direct `veer-vm.exe --backend custom` execution.
- `scripts/run-veer-vm-windows.ps1` currently validates `-Arch x86_64` only, so it is not the ESP32C6 (`riscv32`) launcher.
- For Windows `veer-connect` test troubleshooting, see [veer-connect-updates-2026.md](./veer-connect-updates-2026.md).

---

## 3. Quick Start
### Build everything
```bash
cargo build -p fold_engine -p veer_vm --release
```

### Launch a folded veer-vm
```bash
./target/release/fold spawn --manifest crates/fold_engine/examples/veer-vm.toml
```
- The manifest describes the VM: kernel path, memory, network, etc.
- Fold will launch `veer-vm` inside a secure envelope.

### List running folds
```bash
./target/release/fold list
```

### View logs
```bash
./target/release/fold logs <fold-name>
```

### Stop and remove
```bash
./target/release/fold stop <fold-name>
./target/release/fold rm <fold-name>
```

---

## 4. Manifest Example (`veer-vm.toml`)
```toml
name = "my-vm"
cmd  = "./target/release/veer-vm"
args = ["--kernel", "path/to/veeros-kernel.elf", "--memory", "256"]
hostname = "folded-vm"

[env]
PATH = "/usr/bin:/bin"

[namespaces]
pid = true
mount = true
uts = true
ipc = true
net = true
user = false

[seccomp]
profile = "default"

[limits]
cpu_max    = "50000 100000"
memory_max = 268435456
pids_max   = 64
```

---

## 5. Features & Benefits
- **Resource limits**: CPU, memory, pids via cgroups v2
- **Syscall filtering**: seccomp default-deny
- **Rootless mode**: User namespaces for non-root operation
- **State registry**: All folds tracked in `$XDG_STATE_HOME/veeros/fold/`
- **Auditability**: Logs and state for every fold
- **Integration**: `fold vm spawn` auto-generates manifests for common cases

---

## 6. Advanced Usage
### Rootless cgroup delegation
```bash
systemd-run --user --scope --property=Delegate=yes -- \
  ./target/release/fold spawn --manifest crates/fold_engine/examples/veer-vm.toml
```

### Customizing the manifest
- Change kernel, memory, network, or add environment variables as needed.
- Add extra seccomp deny rules for further hardening.

### Snapshots & Migration (future)
- Planned: snapshot/restore of folded VMs for live migration and backup.

---

## 7. Troubleshooting
- **`veer-vm` not found**: Build it first with `cargo build -p veer_vm`.
- **KVM errors**: Ensure `/dev/kvm` exists and you have permission.
- **Resource errors**: Check cgroup delegation or run as root.
- **Logs**: Use `fold logs <name>` for detailed output.
- **Windows ESP32C6 connect failure**: Most failures are guest launch/port-forward setup rather than `veer-connect` itself; use the Windows checklist in [veer-connect-updates-2026.md](./veer-connect-updates-2026.md).

---

## 8. References
- [crates/fold_engine/README.md](../crates/fold_engine/README.md)
- [crates/veer_vm/README.md](../crates/veer_vm/README.md)
- [crates/fold_engine/examples/veer-vm.toml](../crates/fold_engine/examples/veer-vm.toml)

---

_Last updated: 2026-05-03_

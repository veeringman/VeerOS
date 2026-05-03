# macOS HVF Development Loop

This guide is for developing `veer-vm` on an Intel macOS host using Hypervisor.framework (HVF).

## Scope

- Host: Intel macOS (`x86_64-apple-darwin`)
- VMM backend: HVF (`crates/veer_vm/src/backend/hvf.rs`)
- Current status: build and bring-up scaffolding is in place; full boot-to-banner may still be blocked by host HVF availability (`hv_vm_create`).

## Prerequisites

1. Xcode Command Line Tools installed on the Mac host.
2. Rust toolchain available on Mac (`rustup`, `cargo`).
3. Workspace synced to the Mac (or use rsync from Linux dev host).

## Build On macOS

Use helper script:

```bash
./scripts/build-mac.sh
```

Equivalent commands:

```bash
cargo build -p veer_vm --target x86_64-apple-darwin
cargo build -p fold_engine --target x86_64-apple-darwin
```

## Run HVF Preflight

Preflight checks whether the host advertises required HVF support:

```bash
cargo run -q -p veer_vm --target x86_64-apple-darwin -- --hvf-preflight
```

Expected signals:

- `kern.hv_support=1`
- `kern.hv_vmm_present=0`
- CPU features include `VMX`

If preflight passes but VM creation still fails, the most likely issue is host-level hypervisor contention/policy.

## Run HVF Probe

Use `--hvf-probe` to perform a minimal `hv_vm_create` + `hv_vm_destroy` cycle:

```bash
cargo run -q -p veer_vm --target x86_64-apple-darwin -- --hvf-probe
```

Use this before full VM launch when diagnosing host-level issues. If this fails with `HV_BUSY`, the issue is host contention, not guest boot plumbing.

## Run veer-vm (macOS target)

Use helper script:

```bash
./scripts/run-mac.sh ./target/x86_64-unknown-none/debug/kernel-qemu-pc
```

Direct command:

```bash
cargo run -p veer_vm --target x86_64-apple-darwin -- \
  --kernel ./target/x86_64-unknown-none/debug/kernel-qemu-pc \
  --memory 128
```

## One-Shot HVF Exit Diagnostics

Use `--hvf-run-once` to execute a bounded bring-up loop and print decoded exits:

```bash
cargo run -q -p veer_vm --target x86_64-apple-darwin -- \
  --kernel ./target/x86_64-unknown-none/debug/kernel-qemu-pc \
  --memory 128 \
  --hvf-run-once
```

This mode currently includes:

- CPUID passthrough handling
- Basic COM1 (16550) PIO behavior
- LAPIC/IOAPIC MMIO region stubs with stateful register surfaces

## Linux -> macOS rsync loop

From Linux dev host:

```bash
sshpass -p '<password>' rsync -az --delete \
  --exclude target/ --exclude .git/ \
  ./ <user>@<mac-host>:~/rnd/VeerOS/
```

Then on macOS host:

```bash
cd ~/rnd/VeerOS
cargo check -p veer_vm --target x86_64-apple-darwin
```

## ESP32-C6 Virtual IoT on macOS

For EdgeFabric virtual IoT workflows on Apple Silicon two runtime modes are available:

### Mode 1: QEMU (always available)

```bash
brew install qemu
veeros-vm create my-esp32 --target esp32c6 --net nat
veeros-vm start my-esp32
veer-connect shell localhost 2230
```

### Mode 2: veer-vm binary (rootless, recommended)

Uses Apple Hypervisor framework — no QEMU, no root:

```bash
cargo build -p veer-vm
veeros-vm create my-esp32 --target esp32c6 --net user
veeros-vm start my-esp32
veer-connect shell localhost 2230
```

### Multiple concurrent VMs

Both modes support running multiple VMs simultaneously. Ports are auto-assigned starting at 2230 and the orphan-cleanup guard prevents stale process accumulation from earlier crashed or unclean stops.

### EdgeFabric agent (macOS, agent-only mode)

To run just the agent and streaming server locally without a control plane:

```bash
EF_RUN_MODE=development \
EF__VEEROS__ROOT_DIR=~/VeerOS \
EF__VEEROS__VM_SCRIPT_PATH=~/VeerOS/scripts/veeros-vm \
EF__VEEROS__VEER_CONNECT_PATH=~/VeerOS/target/debug/veer-connect \
  ./scripts/deploy.sh agent
```

The streaming server on port 7070 starts immediately. Control-plane registration runs in the background and retries automatically when available.

---

## Troubleshooting

1. `hv_vm_create` fails:
   - Run `--hvf-preflight` first.
   - Confirm `kern.hv_support=1` and `kern.hv_vmm_present=0`.
   - Check for competing hypervisor usage on host.

2. `can't find crate for core/std` when checking Darwin target from Linux:
   - This is expected unless the Darwin std target is installed locally.
   - Use remote macOS checks as source of truth.

3. No visible guest output in bring-up:
   - Use `--hvf-run-once` and inspect emitted exit traces.
   - Confirm kernel ELF path is correct and loadable.
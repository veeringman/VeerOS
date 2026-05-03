# VeerOS User Manual

## Table of Contents
1. [Prerequisites](#prerequisites)
2. [Building](#building)
3. [Running on QEMU](#running-on-qemu)
4. [Running the Host Demo](#running-the-host-demo)
5. [Flashing to ESP32](#flashing-to-esp32)
6. [Shell Reference](#shell-reference)
7. [Distribution Feature Flags](#distribution-feature-flags)
8. [EdgeFabric Runtime Integration](#edgefabric-runtime-integration)

---

## Prerequisites

| Tool | Version | Install |
|------|---------|---------|
| Rust (stable) | ≥ 1.75 | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| RISC-V target | riscv32imc-unknown-none-elf | `rustup target add riscv32imc-unknown-none-elf` |
| QEMU (RISC-V) | ≥ 8.0 | `sudo apt-get install qemu-system-misc` (Debian/Ubuntu) |
| espflash (ESP32 only) | ≥ 3.0 | `cargo install espflash` |

## Building

### Check all crates (host + RISC-V)

```sh
cargo check-all          # alias: --workspace
```

### Build the QEMU kernel

```sh
cargo build -p kernel-qemu-virt --target riscv32imc-unknown-none-elf
```

### Build the ESP32-C3 kernel

```sh
cargo build -p kernel-esp32 --target riscv32imc-unknown-none-elf
```

### Build the host demo

```sh
cargo build -p veeros-demo
```

## Running on QEMU

```sh
qemu-system-riscv32 \
    -machine virt \
    -nographic \
    -bios none \
    -kernel target/riscv32imc-unknown-none-elf/debug/kernel-qemu-virt
```

You will see the boot banner and a VeerOS shell prompt:

```
================================================
  VeerOS v0.1.0 — Microkernel for RISC-V
  Platform : QEMU virt (riscv32)
  Scheduler: round-robin preemptive
================================================

VeerOS> 
```

### Exiting QEMU

- Type `exit` or `quit` at the shell prompt — VeerOS writes the SiFive Test
  poweroff word and QEMU terminates cleanly.
- Or press **Ctrl-A** then **X** to kill QEMU immediately.

## Running the Host Demo

The `veeros-demo` binary runs a simplified VeerOS shell on your workstation
using standard I/O (no RISC-V hardware required). Useful for testing shell
features.

```sh
cargo run -p veeros-demo
```

Press **Ctrl-D** or type `exit` to leave.

> **Note:** The host demo does not have preemptive scheduling — `uptime` and
> `tasks` are disabled.

## Flashing to ESP32

> Requires an ESP32-C3 (or C6/H2) connected via USB.

```sh
cargo build -p kernel-esp32 --target riscv32imc-unknown-none-elf --release

espflash flash \
    --chip esp32c3 \
    target/riscv32imc-unknown-none-elf/release/kernel-esp32
```

Connect a serial terminal (e.g., `espflash monitor`) at 115 200 baud to
interact with the shell.

## Shell Reference

| Command | Description |
|---------|-------------|
| `help` | List available commands |
| `version` | Show VeerOS version string |
| `sysinfo` | Platform, scheduler type, distribution |
| `uname` | Unix-style `uname -a` output |
| `uptime` | Elapsed ticks since boot (QEMU only) |
| `tasks` / `ps` | List all tasks with state (QEMU only) |
| `echo <text>` | Print text back to the console |
| `clear` / `cls` | Clear the terminal screen (ANSI) |
| `logo` | Print the VeerOS ASCII art logo |
| `exit` / `quit` | Leave the shell (powers off on QEMU) |

### Line Editing

- **Backspace / Delete** — erase last character.
- **Ctrl-C** — cancel current line.
- **Ctrl-D** — end-of-input (exits).
- **ESC sequences** — silently discarded (arrow keys etc.).

## Distribution Feature Flags

VeerOS supports multiple distribution profiles selected at compile time via
Cargo feature flags on the `distributions` crate.

| Feature | Scheduler | Extras |
|---------|-----------|--------|
| `dist-minimal` | Round-robin | Core IPC, VMem, driver isolation |
| `dist-app` | Round-robin | + Network stack, app runtime |
| `dist-rt` | Priority-based | + Real-time guarantees |
| `dist-full` | Priority-based | Everything |

Example:

```sh
cargo build -p kernel-qemu-virt \
    --target riscv32imc-unknown-none-elf \
    --features distributions/dist-rt
```

> Feature-gated components are still being implemented. The default build uses
> the `dist-minimal` round-robin scheduler.

## veeros-vm CLI

`scripts/veeros-vm` is the primary lifecycle manager for VeerOS VMs used by EdgeFabric.

### ESP32-C6 — two runtime modes

#### `--net nat` (QEMU, default)

Uses `qemu-system-riscv32` with a host-forwarded port. Works on Linux and macOS.

```bash
veeros-vm create my-dev --target esp32c6 --net nat
veeros-vm start my-dev
veer-connect shell localhost 2230
```

#### `--net user` (veer-vm binary, rootless)

Uses the `veer-vm` Rust VMM (HVF on macOS, KVM on Linux). No QEMU needed, no root required. Preferred on macOS.

```bash
cargo build -p veer-vm          # build once
veeros-vm create my-dev --target esp32c6 --net user
veeros-vm start my-dev
veer-connect shell localhost 2230
```

### Multi-VM operation

Multiple VMs can run concurrently. Each receives a unique host-forwarded port starting at 2230. Port allocation is serialized to prevent collisions.

```bash
veeros-vm create vm-a --target esp32c6 --net nat   # gets 2230
veeros-vm create vm-b --target esp32c6 --net nat   # gets 2231
veeros-vm start vm-a && veeros-vm start vm-b
veeros-vm list                                      # human-readable table
veeros-vm list-json                                 # JSON array (used by EdgeFabric agent)
```

### Stale process cleanup

If a previous VM was killed without a clean stop, `veeros-vm start` automatically detects and removes orphaned `qemu-system-riscv32` or `veer-vm` processes tied to that VM before launching a new instance.

### Sensor injection

```bash
veeros-vm sensor inject my-dev temperature 27.5
veeros-vm sensor inject my-dev humidity 55.0
```

---

## EdgeFabric Runtime Integration

For orchestration and lifecycle management of fold + veer-vm based ESP32-C6
virtual IoT runtimes, see:

- [EdgeFabric Integration Guide: VeerOS ESP32-C6 Virtual IoT Runtime](edgefabric-esp32c6-runtime.md)

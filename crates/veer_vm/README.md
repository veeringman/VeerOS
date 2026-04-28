# veer-vm — VeerOS microVMM

A lightweight, cross-platform microVMM for VeerOS kernels. `veer-vm` aims to be a Firecracker-class VMM, providing a fast, minimal alternative to QEMU for development and deployment.

**Supported host platforms:**
- **Linux**: Native KVM backend (production-ready)
- **macOS**: Hypervisor.framework (HVF) backend (experimental, x86_64 only)
- **Windows**: Planned (stub only)

**Supported guest architectures:**
- x86_64 (native)
- riscv32 (ESP32-C6, via QEMU TCG fallback on non-riscv64 hosts)
- aarch64 (planned)

**Backend selection is automatic based on host OS and guest arch.**

## Status — Multi-platform, multi-arch

`veer-vm` can:
- Boot VeerOS kernels as Multiboot v1 ELF or from VeerOS ISO images
- Run on Linux (KVM), macOS (HVF), and (stub) Windows
- Support x86_64 and riscv32 guests (QEMU TCG fallback for RISC-V on x86_64/aarch64 hosts)
- Emulate 16550A UART (guest serial → host stdout, host stdin → guest)
- Clean shutdown (Ctrl-A x, signals)
- Snapshot/restore (RAM, vCPU, IRQ/PIT, UART state)

**Note:** For riscv32 guests on non-riscv64 hosts, QEMU TCG is required (`qemu-system-riscv32`).

## Example: Booting VeerOS on different platforms

### Linux (KVM, x86_64 guest)
```bash
cargo build -p veer_vm
./target/debug/veer-vm \
    --kernel ./target/x86_64-unknown-none/debug/kernel-qemu-pc \
    --memory 128
```

### macOS (HVF, x86_64 guest)
```bash
cargo build -p veer_vm
./target/debug/veer-vm \
    --kernel ./target/x86_64-unknown-none/debug/kernel-qemu-pc \
    --memory 128
```

### RISC-V guest (ESP32-C6, QEMU TCG fallback)
```bash
./target/debug/veer-vm \
  --arch riscv32 \
  --kernel ./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6 \
  --memory 128
# On non-riscv64 hosts, requires qemu-system-riscv32 in PATH
```

### Boot from VeerOS ISO
```bash
./target/debug/veer-vm \
  --kernel ./build/veeros.iso \
  --memory 128
```

### Snapshot/restore
```bash
./target/debug/veer-vm \
  --kernel ./build/veeros.iso \
  --snapshot-save /tmp/veeros.snap
./target/debug/veer-vm \
  --restore /tmp/veeros.snap
```

## Platform backend status

| Host OS   | Backend         | Guest arch    | Status         |
|-----------|-----------------|--------------|----------------|
| Linux     | KVM             | x86_64       | Stable         |
| Linux     | KVM             | riscv32      | Stable         |
| macOS     | HVF             | x86_64       | Experimental   |
| macOS     | QEMU TCG        | riscv32      | Experimental   |
| Windows   | (stub)          | (planned)    | Not implemented|

## Fold integration

`veer-vm` can be launched inside a VeerOS Fold for full namespace, cgroup, and seccomp isolation. See the [fold_engine README](../fold_engine/README.md) for details.

---
The rest of this README documents architecture, boot contract, and roadmap.
# veer-vm — VeerOS microVMM

A lightweight Firecracker-class KVM VMM, built as an alternative to QEMU
for booting VeerOS kernels. Runs on Linux, requires `/dev/kvm` access.

## Status — Phase 2 working

As of this commit, `veer-vm` can:

- Parse a 64-bit ELF kernel and load its `PT_LOAD` segments into guest RAM.
- Set up a vCPU in 32-bit protected mode with flat segments, matching the
  Multiboot v1 entry contract (EAX=magic, EBX=info-ptr, paging off).
- Use KVM's **in-kernel IRQ chip** (PIC + IOAPIC + LAPIC) and **in-kernel
  PIT** (8254) — no userspace MMIO emulation, HLT handled in-kernel, real
  calibrated timer (~2800 LAPIC ticks/ms on a recent Intel host).
- Emulate a 16550A UART at PIO `0x3F8`, bidirectional:
  - TX: guest → host stdout.
  - RX: raw-mode host stdin → dedicated reader thread → shared RX queue;
    IRQ 4 asserted via `KVM_IRQ_LINE` when `IER.ERBFI` is set and data
    is available, deasserted when the queue drains.
- QEMU-style `Ctrl-A x` escape to quit the VMM cleanly; terminal
  restored via an RAII guard.
- Handle SIGTERM / SIGHUP / SIGUSR1 for clean shutdown.

Verified boot of `crates/kernel/qemu_pc`:

## Usage

```bash
```
[veer-vm] loaded kernel …/kernel-qemu-pc: entry=0x100010 end=0x408000 memory=128 MiB
[veer-vm] raw mode engaged — press Ctrl-A x to quit
[veer-vm] vcpu entering guest at 0x100010

========================================
  VeerOS v0.1.0
  Platform : QEMU PC (x86-64)
  Scheduler: application
========================================
[boot] heap initialised (64 KiB — small 16K + large 48K)
[boot] frame allocator: 13280 KiB free (3320 frames, start=0x408000)
[boot] GDT + IDT + SYSCALL/SYSRET configured
[boot] LAPIC + I/O APIC ready (timer 2783 ticks/ms)
[boot] starting scheduler — preemptive mode

VeerOS v0.1.0       ← second greeting printed by a scheduled task
```

## Usage

```bash
cargo build -p veer_vm
./target/debug/veer-vm \
    --kernel ./target/x86_64-unknown-none/debug/kernel-qemu-pc \
    --memory 128

# Or boot straight from the GRUB ISO produced by build-qemu-pc.sh
./target/debug/veer-vm \
  --kernel ./build/veeros.iso \
  --memory 128

# Save a snapshot directory on clean shutdown (Ctrl-A x, SIGTERM)
./target/debug/veer-vm \
  --kernel ./build/veeros.iso \
  --snapshot-save /tmp/veeros.snap

# Restore from that snapshot later
./target/debug/veer-vm \
  --restore /tmp/veeros.snap

# RISC-V 32-bit kernel image loading path (build support)
./target/debug/veer-vm \
  --arch riscv32 \
  --kernel ./target/riscv32imc-unknown-none-elf/debug/kernel-qemu-esp32c6 \
  --memory 128
```

`--memory` is in MiB (default 128). `--kernel` accepts either a Multiboot
ELF or a VeerOS ISO containing `/boot/kernel.elf` for `--arch x86_64`.
`--arch riscv32` supports two execution paths:

- Linux riscv64 host: native RISC-V KVM backend path in `veer-vm`.
- x86_64/AArch64 hosts: QEMU TCG virtualizer path via `qemu-system-riscv32`.

Install `qemu-system-riscv32` on non-riscv64 hosts for the riscv32 path.

Automated smoke check for this path:

```bash
bash ./scripts/veer-vm-snapshot-smoke.sh
```

## Architecture

| Module             | Responsibility                                       |
|--------------------|------------------------------------------------------|
| `memory.rs`        | Single `mmap`-backed guest memory region, registered |
|                    | with KVM via `set_user_memory_region`.               |
| `elf.rs`           | Parse ELF64 and copy `PT_LOAD` segments to `p_paddr`.|
| `multiboot.rs`     | Minimal Multiboot v1 info struct (flags + mem_upper).|
| `serial.rs`        | 16550A UART register model on PIO `0x3F8-0x3FF` with |
|                    | bidirectional data and IRQ 4 assertion.              |
| `termios_guard.rs` | RAII guard putting stdin into raw mode (no echo, no  |
|                    | line buffering, no ISIG) and restoring on drop.      |
| `vm.rs`            | KVM orchestration: irqchip + PIT, vCPU sregs/regs,   |
|                    | run loop, PIO dispatch, stdin reader thread, signal  |
|                    | handlers.                                            |
| `main.rs`          | CLI (clap).                                          |

## Boot contract

VeerOS's `_start` (in `crates/kernel/qemu_pc/src/main.rs`) expects the
Multiboot v1 machine state:

- Real CR0.PE=1, CR0.PG=0 — 32-bit protected mode, paging off.
- CS flat 32-bit code (descriptor 0x00CF9A00_0000FFFF).
- DS/ES/FS/GS/SS flat 32-bit data (descriptor 0x00CF9200_0000FFFF).
- EAX = 0x2BADB002.
- EBX = guest phys address of a Multiboot info struct.
- EIP = ELF entry point.
- A20 on (KVM always satisfies this).

`veer-vm` reproduces this exactly via `KVM_SET_SREGS` / `KVM_SET_REGS`
without needing to stage a GDT in guest memory.

## Roadmap

| Phase | Item                                                              |
|-------|-------------------------------------------------------------------|
| 1 ✅  | Boot + 16550 serial TX.                                           |
| 2 ✅  | In-kernel irqchip + PIT + UART RX + clean shutdown.               |
| 3     | Virtio-mmio transport + virtio-console / virtio-blk / virtio-net. |
| 3 ✅  | Boot image format: load VeerOS ISO directly (instead of ELF).     |
| 4 ~   | Snapshot / restore foundation: save and restore RAM + vCPU + irqchip/PIT + UART + virtio-blk transport state. |
| 4 ✅  | Fold × VMM fusion: `fold vm spawn` wraps `veer-vm` in a fold.     |
| 6 ~   | ESP32-C6 support: riscv32 ELF loading + `--arch riscv32` wiring + cross-host virtualizer path (QEMU on x86_64/AArch64, KVM on riscv64). |
| 5     | aarch64 KVM backend for RPi5 guest kernels.                       |

## Fold integration

As of Phase 4, `fold vm spawn` handles the whole thing:

```bash
cargo build -p veer_vm -p fold_engine

./target/debug/fold vm spawn \
    --kernel ./target/x86_64-unknown-none/debug/kernel-qemu-pc \
    --memory 128 \
    --user-ns \
    --name veeros1

./target/debug/fold list
./target/debug/fold logs veeros1 --follow   # live boot output
./target/debug/fold stop veeros1
./target/debug/fold rm   veeros1 --force
```

The fold runs `veer-vm` as init (PID 1 in a fresh PID namespace), in a
fresh mount/UTS/IPC/net namespace, with the default seccomp denylist
(blocks `mount`, `unshare`, `bpf`, module load, `ptrace`, etc., but
allows `ioctl` — KVM works fine). Guest serial output is captured to
`$XDG_STATE_HOME/veeros/fold/<name>.log`.

## Known gaps

- No signal-safe restore of termios if the process is SIGKILLed — SIGTERM
  path is handled but SIGKILL cannot be caught. Mitigation: user can run
  `reset` in their shell if the terminal is left in raw mode.
- Single vCPU, no SMP.
- No virtio yet; block/net I/O is not available.

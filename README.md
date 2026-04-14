<!-- LOGO PLACEHOLDER -->
![VeerOS Logo](./docs/veeros.logo.png)

**VeerOS** is a from-scratch operating system written in Rust, engineered around the
principle that intelligence, security, and coordination should be intrinsic to the
kernel — not layered on after the fact.

It begins where the hardware begins: bare metal. From a single microcontroller to
a fleet of heterogeneous nodes, VeerOS presents one unified abstraction — a
*coherent computational surface* that adapts its shape to the workload and the
topology beneath it.

---

## Philosophy

> *The boundary between the system and the application should dissolve when it
> no longer serves either.*

VeerOS is not a general-purpose kernel with plugins. It is a **composable
runtime** whose personality is selected at compile time through feature flags and
distribution profiles. A sensor node, a network appliance, a cloud orchestrator,
and an AI inference endpoint can all be the same kernel binary — or completely
different ones — depending on what you choose to include.

---

## Capabilities

- **Multi-Architecture Kernel** — runs on RISC-V (32/64), ARM64, Xtensa, and
  x86-64; the same microkernel, abstracted at the instruction level.

- **Hardware Spectrum** — from 320 KB ESP32 microcontrollers (C3/C6/H2) through
  Raspberry Pi 5 SBCs to KVM-accelerated virtual machines on cloud metal; Xtensa
  (ESP32-S3) and ARM64 bring-up in progress.

- **Wireless Radio Stack** — Wi-Fi (WPA2/WPA3 STA/AP), BLE 5.0 (GAP/GATT/HOGP),
  and IEEE 802.15.4 with Zigbee, Thread, and Matter layers; all three radios
  coexist on a single SoC, mediated by a kernel coexistence layer.

- **Adaptive Distribution Model** — compile-time feature composition produces
  purpose-built images: `dist-minimal` bare MCU, `dist-rt` real-time controller,
  `dist-edge` IoT/AI edge node, `dist-ai` inference platform, `dist-cluster`
  distributed OS participant, `dist-cloud` orchestration host, `dist-firewall`
  network appliance, and `dist-gateway` IoT protocol bridge.

- **Coherent Multi-Node Fabric** — nodes discover each other, elect leaders,
  share state, and present a single-system illusion. Processes and files
  migrate transparently.

- **Native Orchestration** — container scheduling, desired-state reconciliation,
  rolling deployments, health-aware placement — without a separate control plane.

- **Built-In Service Mesh** — load balancing, circuit breaking, mTLS, traffic
  splitting, and observability injected at the kernel socket layer. No sidecars.

- **Packet-Level Network Intelligence** — stateful filtering, NAT, VPN tunnels,
  traffic shaping, and protocol identification as kernel primitives. VeerOS is
  the firewall.

- **AI as a System Primitive** — inference engine in the kernel, NL-aware shell,
  autonomous agents, vision/voice pipelines, on-device training; the OS thinks.

- **Security by Construction** — capability-based access control, isolation
  domains, post-quantum cryptography, measured boot, hardware-enforced
  sandboxing; configurable single-user / multi-user identity model, feature-gated
  per distribution profile.

- **Universal Filesystem** — everything is a file; VFS unifies RamFS, DevFS
  (`/dev/null`, `/dev/random`, `/dev/console`), and persistent FAT32 on SD/eMMC
  — all through the same `open` / `read` / `write` / `seek` syscall surface.

- **Async-Native Userland** — `no_std` cooperative executor, typed async channels,
  and future-based I/O and timers ship inside `userlib`; concurrency scales from
  a single ISR to a thread pool without a separate runtime dependency.

- **Unified Input** — USB keyboards and mice via xHCI host controller; BLE
  keyboards and mice via HOGP; both converge on `/dev/keyboard` and `/dev/mouse`,
  decoupled from the transport layer.

- **Quantum Interface Layer** — abstraction over simulators, coprocessors, and
  cloud QPUs; circuits compile and execute through the same syscall surface.

---

## Repository Structure

```text
crates/
  arch/           → CPU-neutral instruction abstraction
  microkernel/    → Scheduler, IPC, VFS, capabilities, futex, channels
  soc/            → Hardware drivers per chip family
  kernel/         → Per-board binary entry points
  net/            → smoltcp TCP/IP stack + network abstractions
  crypto/         → Post-quantum and symmetric cryptography primitives
  shell/          → Interactive shell — readline, vi editor, man pages
  userlib/        → Userspace library — async runtime, sync, fs, sockets
  distributions/  → Feature-flag composition profiles
docs/             → Architecture, bringup guides, specs
scripts/          → Build, flash, debug helpers
```

## Architecture

VeerOS is built around a layered, multi-architecture design that separates
portable kernel logic from hardware- and CPU-specific code.

➡️ **Detailed design & diagram:** [Architecture Documentation](docs/architecture.md)

---

## Distribution Profiles

VeerOS uses Rust feature flags to compose purpose-built kernel images.
Two orthogonal axes — **profile** (scheduler + capabilities) and **components**
(shell, net, AI, cluster, firewall, …) — combine at compile time. Any mix of
flags is valid.

### Profile Hierarchy

```
dist-minimal                    bare scheduler, IPC, memory isolation
├── dist-app                    + shell, networking, userlib, samples
│   ├── dist-ai                 + inference engine, NL shell, NPU backends
│   ├── dist-cluster            + cluster membership, distributed sched/IPC/VFS
│   │   └── dist-cloud          + orchestration, service mesh, observability
│   └── dist-full               all Tier 1 components + priority scheduler
├── dist-rt                     + priority real-time scheduler
│   └── dist-xrt                + accelerator / GPU / FPGA / QPU (UAI)
├── dist-edge                   + edge AI inference, WiFi/BLE, sensor pipeline
│   └── dist-gateway            + Thread border router, Zigbee, MQTT broker
└── dist-firewall               + packet filter, NAT, VPN, DPI, traffic shaping
```

### Quick Reference

| Profile | Base | Purpose | Target Hardware |
|---------|------|---------|-----------------|
| `dist-minimal` | — | Bare MCU, boot-to-idle | ESP32-C3, RP2350 |
| `dist-app` | minimal | Interactive workstation | QEMU virt, RPi 3+ |
| `dist-rt` | minimal | Hard real-time control | ESP32-C6, industrial |
| `dist-xrt` | rt | RT + hardware accelerators | x86-64/ARM64 + GPU/FPGA |
| `dist-full` | app | Everything included | RPi 5, x86-64 |
| `dist-edge` | minimal | IoT / edge AI node | ESP32-S3, RPi Zero |
| `dist-ai` | app | AI-native inference platform | RPi 4/5, x86-64 ≥ 2 GB |
| `dist-cluster` | app | Distributed OS node | RPi 3+, x86-64, ARM64 |
| `dist-cloud` | cluster | Cloud orchestration host | x86-64 KVM, ARM64 KVM |
| `dist-firewall` | minimal | Router / firewall / VPN gateway | x86-64, ARM64, RPi 4/5 |
| `dist-gateway` | edge | IoT protocol bridge | RPi 3+, ESP32-S3 |

### Build Examples

```sh
# Bare scheduler — nothing but an idle loop
cargo build -p kernel-qemu-virt --no-default-features --features dist-minimal

# Interactive shell + networking (default for QEMU RISC-V)
cargo build -p kernel-qemu-virt --features dist-app

# Full ESP32 with all radios
cargo build -p kernel-xiao-esp32c6 --features dist-full,wifi,ble,ieee802154

# x86-64 network appliance
cargo build -p kernel-qemu-pc --features dist-firewall

# AI workstation with NPU offload
cargo build -p kernel-qemu-pc --features dist-ai

# Cloud cluster node
cargo build -p kernel-qemu-pc --features dist-cloud

# Mix-and-match — any combination is valid
cargo build -p kernel-qemu-pc --features dist-firewall,ai
```

➡️ **Full distribution details:** see Phase 4 in [TODO.md](TODO.md)


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


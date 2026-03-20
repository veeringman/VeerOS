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

- **Hardware Spectrum** — from 320 KB ESP32 microcontrollers through Raspberry Pi
  SBCs to KVM-accelerated virtual machines on cloud metal.

- **Adaptive Distribution Model** — compile-time feature composition produces
  purpose-built images: real-time controller, edge AI node, network security
  appliance, cluster participant, or full cloud platform.

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
  sandboxing.

- **Quantum Interface Layer** — abstraction over simulators, coprocessors, and
  cloud QPUs; circuits compile and execute through the same syscall surface.

---

## Repository Structure

```text
crates/
  arch/           → CPU-neutral instruction abstraction
  microkernel/    → Scheduler, IPC, VFS, capabilities
  soc/            → Hardware drivers per chip family
  kernel/         → Per-board binary entry points
  net/            → TCP/IP stack + network abstractions
  shell/          → Interactive command interpreter
  userlib/        → Userspace library + async runtime
  distributions/  → Feature-flag composition profiles
docs/             → Architecture, bringup guides, specs
scripts/          → Build, flash, debug helpers
```

## Architecture

VeerOS is built around a layered, multi-architecture design that separates
portable kernel logic from hardware- and CPU-specific code.

➡️ **Detailed design & diagram:** [Architecture Documentation](docs/architecture.md)


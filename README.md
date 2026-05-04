<div align="center">

<img src="./veeros-logo.png" alt="VeerOS Logo" width="180" /><p>
<img src="./veeros.png" alt="VeerOS Logo" width="210" />


**An AI-native operating system - a single fabric for a single MCU to a planet scale fleet**

*Goals are kernel primitives. Agents are first-class citizens. The browser is obsolete. The OS understands intent.*

[![Rust](https://img.shields.io/badge/rust-nightly-orange)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue)](#)
[![Architectures](https://img.shields.io/badge/arch-RISC--V%20%7C%20ARM64%20%7C%20x86--64%20%7C%20Xtensa-green)](#)

</div>

---

**VeerOS** is a from-scratch Rust operating system that turns devices — from
microcontrollers to cloud nodes — into one coherent computational surface, with
intelligence, security, and orchestration built into the kernel by design.

## Latest Updates (2026-05-04)

- Updated VeerOS VM network smoke coverage for the current connect flow and host forwarding behavior.
- Added shell terminal compatibility notes and routing analysis docs for VeerOS host access.
- Clarified operator direction for host shell access: prefer `veer-connect` style shell routing for VeerOS hosts over generic SSH assumptions when host metadata advertises VeerOS capabilities.

Where traditional operating systems understand processes, threads, and files,
VeerOS introduces three new kernel primitives: **Intents** (declarative goals),
**Agents** (autonomous execution units), and an **Execution Fabric**
(heterogeneous node topology). Compute, state, security, networking, and
observability collapse into a single fabric — not layered on after the fact.
Together, they replace entire cloud stacks — with few syscalls.

> **Try it now:**
> ```bash
> cargo run -p veeros-demo      # interactive AI-native shell
> ```

---

## Why VeerOS Exists

Every layer in the modern infrastructure stack exists because the OS doesn't
understand goals:

| Problem | Today's Stack | VeerOS |
|---|---|---|
| Orchestration | Kubernetes (2M+ lines) | `intent submit` (1 syscall) |
| Service mesh | Istio + Envoy sidecars | ZeroServices — kernel-native routing + mTLS |
| API gateway | Kong / Nginx / Traefik | `SYS_SVC_EXPOSE` (1 syscall) |
| Service calls | REST endpoints + HTTP | `invoke("name.function", payload)` |
| Databases / caches / queues | Redis + Kafka + etcd | State Fabric (kernel-native KV + streams) |
| Observability | Prometheus + Grafana + OTel | Structured events (typed, not string logs) |
| Config management | etcd / Consul / Vault | Persistent memory (in-kernel KV) |
| Task scheduling | Airflow / Celery / Temporal | Intent decomposition → agent DAGs |
| Cross-arch deployment | Docker multi-arch + QEMU | Native fabric placement |
| Failover / self-healing | Custom health checks + PDBs | Kernel heartbeat + auto-replan |
| Network access control | ClearPass / Aruba / ISE | Kernel-native 802.1X + posture |
| Endpoint security | CrowdStrike / Trellix / Defender | Kernel EDR — syscall-level visibility |
| Zero Trust access | Zscaler / Cloudflare ZTNA | Kernel ZTNA — continuous auth per-request |
| Device management | Intune / JAMF / SCCM | Fabric join = enrollment (0 agents) |
| WAN connectivity | Tailscale / ZeroTier / VPN | WAN-scale fabric with NAT traversal |
| Fleet management | Ansible / Terraform / Salt | Unified console + fleet intents |
| Cross-org integration | API gateways + VPNs + OAuth2 | InterFabric (IFP) — federated invocation |
| User interface | Browser + REST + JS frameworks | Fabric Client (VeerUX) — composable, AI-rendered views |

**VeerOS collapses this entire stack into the kernel.** The OS itself plans,
schedules, places, monitors, secures, renders, and manages — from a single MCU
to a planet-scale fleet.

---

## Real-World Applications

### Edge AI & Industrial IoT
A factory floor with 200 ESP32 sensors, 10 RPi gateways, and 2 cloud GPUs.
Submit `intent submit pipeline collect vibration data, detect anomalies, alert` — the kernel decomposes across the fabric, ingest on sensors, normalize on gateways, inference on GPU. Zero middleware.
**→ Smart manufacturing, precision agriculture, oil & gas monitoring.**

### Autonomous Robotics & Drones
Each drone function (navigation, obstacle avoidance, battery management) is a
kernel agent with a compute budget and deadline. Low battery triggers
`intent submit admin emergency return-to-base` — the kernel re-plans in
microseconds, no ROS2 restarts.
**→ Delivery drones, warehouse robots, surgical robotics, autonomous vehicles.**

### Satellite & Space Systems
A CubeSat's RISC-V compute module, camera, and radio are fabric nodes. Ground
control sends an imaging intent; the kernel budgets compute ticks, schedules
capture when power permits, queues downlink for the next pass — all in-kernel.
**→ Smallsat constellations, deep-space probes, military ISR platforms.**

### Telecom & 5G Network Functions
Network functions (packet forwarding, beamforming, encryption) become agents
with placement constraints. `intent submit compute packet-classification with
latency < 1ms` → placed on FPGA node. Node degrades → auto-migrated to crypto
accelerator node.
**→ 5G edge, NFV, SD-WAN appliances.**

### Medical Devices & Wearables
Continuous glucose monitor + insulin pump + phone gateway. Sensor agent runs on
wearable RISC-V, stores readings in persistent memory, phone agent detects
dangerous trends and spawns a critical-priority dosage agent — guaranteed to
meet deadline by kernel-level budget enforcement.
**→ Implantable devices, patient monitoring, wearable diagnostics.**

### Distributed ML / AI Pipelines
A research lab with GPUs, CPUs, and TPUs. `intent submit pipeline distributed
training across available GPUs` → the fabric maps GPU-capable nodes, the
scheduler spawns data-loader, trainer, and checkpoint agents with placement
constraints. Node overloads → automatic agent migration.
**→ AI research labs, MLOps platforms, inference serving.**

### Defense & Tactical C2
Disconnected edge nodes (radios, drones, vehicles) join the fabric when in
range. `intent submit monitor persistent surveillance of sector 7` — the kernel
decomposes across available assets, re-plans when a drone goes offline, records
all decisions in episodic memory for after-action review.
**→ Tactical edge computing, JADC2, unmanned systems C2.**

### Enterprise Security & Zero Trust
The kernel IS the security appliance. 802.1X authentication, device posture
assessment, syscall-level EDR, and continuous ZTNA — all enforced at the kernel
level. No ClearPass, no CrowdStrike agent, no Zscaler tunnel. Every packet,
every syscall, every authentication event is visible and enforceable at the
source of truth. Threat detected → process sandboxed in microseconds, not
minutes.
**→ Corporate networks, branch offices, regulated industries, government.**

### Enterprise Fleet & Device Management
Fabric join IS device enrollment. Every VeerOS node self-describes its hardware,
firmware version, security posture, and compliance status at join time.
Configuration profiles pushed fleet-wide via a single `intent submit`. OTA
firmware updates, remote wipe, compliance enforcement — no Intune server, no
JAMF cloud, no SCCM infrastructure.
**→ Enterprise IT, managed IoT fleets, retail POS, healthcare devices.**

### Global-Scale WAN Fabric
Nodes behind NAT, across continents, on cellular networks — all part of one
coherent fabric. UDP hole punching, TURN relay fallback, PQC-encrypted tunnels
over public Internet. Latency-aware routing selects the fastest path. WAN
partitions handled gracefully — nodes operate independently and sync on
reconnect.
**→ Multi-site enterprise, global CDN, remote offices, mobile workforce.**

### Cross-Organization Federation
Independent VeerOS fabrics communicate via InterFabric Protocol (IFP) — trust-bound,
identity-driven invocation between organizations. No shared networks, no VPNs,
no API gateways on either side. `invoke("analytics.process", payload, { target:
"fabric://partner.analytics.eu" })` — the kernel handles identity federation,
policy enforcement at both boundaries, and end-to-end encrypted transport.
Compromise one fabric → revoke trust instantly, channels severed in < 1 second.
`fabric://...` identifies the target trust domain (fabric). Object addressing
inside and across fabrics uses VAS tuples (for example `svc{...}`, `aur{...}`).
**→ B2B integration, supply chain, multi-cloud, partner ecosystems, coalition ops.**

### Fabric Client — Beyond the Browser
No user-facing web URLs. No REST. No cookies. No JavaScript frameworks. The Fabric Client
(VeerFlow) renders composable views directly from fabric state — semantic
navigation replaces address bars, identity-native auth replaces login forms,
and AI generates adaptive interfaces that reshape from a serial terminal to a
pixel framebuffer. A developer publishes a function; the fabric renders the UI.
A user navigates by intent, not by URL.
**→ Desktop workstations, kiosks, dashboards, embedded HMI, field terminals.**

---

## AI-Native Architecture

VeerOS is the first OS where AI agent orchestration is a **kernel subsystem**,
not a userspace framework.

```
┌──────────────────────────────────────────────────────────────┐
│  Shell / Application                                         │
│    agents · intent · memory · fabric · demo                  │
├──────────┬───────────┬───────────┬───────────────────────────┤
│  Intent  │  Agent    │  Memory   │  Execution Fabric         │
│  Engine  │  Table    │  Engine   │                           │
│          │           │           │  host-demo    (x86,local) │
│  goals → │  32 slots │  context  │  rpi5-edge    (arm64,rack)│
│  plan DAG│  lifecycle│  persist  │  esp32-sensor (rv32,rack) │
│          │  budget   │  episodic │  cloud-gpu    (x86,dc)    │
├──────────┴───────────┴───────────┴───────────────────────────┤
│               Intent Scheduler (6-phase tick)                │
│  decompose → place → spawn → monitor → record → health      │
├──────────────────────────────────────────────────────────────┤
│  Classic Kernel: Scheduler · IPC · VFS · Sockets · Crypto    │
└──────────────────────────────────────────────────────────────┘
```

**13 new syscalls** (0xF0–0xFF) expose the AI layer to userspace:
`agent_spawn`, `agent_status`, `agent_complete`, `agent_ctx_set/get`,
`intent_submit`, `intent_status`, `intent_cancel`, `memory_store`,
`memory_query`, `fabric_status`, `sched_stats`, `agent_count`.

➡️ **Full design:** [Architecture Documentation](docs/architecture.md)
➡️ **macOS HVF dev loop:** [macOS Development Guide](docs/macos-dev.md) · [macOS VM Manual](docs/macos-vm-manual.txt)

---

## Intelligence Philosophy — The Fabric IS the Model

VeerOS does not bolt an LLM onto an operating system. The distributed fabric
itself is a continuously learning, deterministic intelligence.

### How the Fabric Learns

Every execution cycle feeds back into future decisions:

```
  Intent submitted
    → Decomposed into plan (step DAG)
    → Agents placed on fabric nodes
    → Execution monitored (budget, deadline, outcome)
    → Episode recorded (what worked, what failed, timing, placement)
    → Future decomposition + placement refined by accumulated episodes
    → Integrity signals update behavioral baselines
    → Repair strategies promoted or demoted based on outcomes
    → Knowledge propagated fleet-wide via gossip
```

There is no central model, no training step, no weight matrix. The **"model"**
is the collective state distributed across every node — episodic memories,
placement scores, behavioral baselines, repair strategies, and integrity
history.

### Deterministic by Design

Every learning mechanism in VeerOS is **fully deterministic and auditable**:

| Mechanism | How It Works | Determinism |
|---|---|---|
| Placement scoring | Weighted formula: capability × resources × locality × load | Same inputs → same placement, always |
| Intent decomposition | Rule-based constraint matching (not an LLM) | Same goal + state → same plan |
| Anomaly detection | Statistical z-score against observed baseline | Reproducible given same history |
| Repair strategy selection | Score-ranked registry; promote on success, demote on failure | Deterministic ranking |
| Episodic memory | Append-only log of (action, context, outcome) tuples | Exact replay possible |
| Knowledge distillation | IF/THEN rules extracted from episodes (26E) | Rules are explicit, inspectable |
| Fleet learning | Gossip replication of proven strategies | Same strategy, same evaluation |

**Given the same policy version and the same state snapshot, VeerOS will always make the same decision.** Every
learning step is reproducible, every decision is traceable to specific episodic
evidence, and every adaptation is explainable in causal terms — not statistical
correlation.

### Where LLMs Fit

LLMs are **one inference tool** the fabric uses for specific tasks — never the
learning substrate itself:

| Task | Mechanism | Deterministic? |
|---|---|---|
| NL shell commands | Intent classifier (<50KB) | Yes (argmax, no sampling) |
| Self-explanation text | LLM generates causal summary | No (sampling) — **opt-in, auditable** |
| Autonomous fix proposals | LLM analyzes deviation + spec | No (sampling) — **simulation-validated** |
| Cloud AI escalation | External API (OpenAI, Ollama) | No — **privacy-gated, never default** |

Non-deterministic LLM outputs are always:
- **Opt-in** — switched on explicitly (`ai on`), never default behavior
- **Validated** — repair proposals tested in simulation before deployment
- **Auditable** — every LLM-assisted decision logged with full context
- **Bypassable** — the system works without them; LLMs accelerate, they don't decide

### What This Means

```
Conventional AI:    Train model → Deploy → Inference (static until retrained)
VeerOS Fabric:      Execute → Record → Refine → Execute (continuous, deterministic)

Conventional AI:    Central model server, GPU cluster, training pipeline
VeerOS Fabric:      Every node learns independently + shares via gossip

Conventional AI:    Model = opaque weight matrix
VeerOS Fabric:      "Model" = inspectable episodes + explicit rules + scored strategies

Conventional AI:    Non-deterministic (sampling, floating-point variance)
VeerOS Fabric:      Deterministic (same history → same decision, always)
```

**The fabric doesn't use a model. The fabric IS the model.**

---

## Quick Start

```bash
# Interactive AI-native demo (runs on your host machine)
cargo run -p veeros-demo

# Run a scripted scenario
./scripts/demo.sh --scenario deploy    # service deployment
./scripts/demo.sh --scenario pipeline  # data pipeline
./scripts/demo.sh --scenario monitor   # monitoring swarm
./scripts/demo.sh --scenario full      # all scenarios

# Build for real hardware
./scripts/build-qemu-pc.sh            # x86-64 QEMU
./scripts/run-qemu-pc.sh              # x86-64 QEMU with virtio-net + SSH forwarding
./scripts/build-esp32c6.sh            # RISC-V ESP32-C6
./scripts/build-raspi5.sh             # ARM64 Raspberry Pi 5
```

For QEMU PC networking, VeerOS uses QEMU user-net with guest SSH on port `2222`.
Use `./scripts/run-qemu-pc.sh` to attach the required `virtio-net` device and forward host port `2222`
to the guest SSH service. To allow LAN access from another machine, leave the default
`HOST_BIND_ADDR=0.0.0.0`; to restrict access to the local host only, run with `HOST_BIND_ADDR=127.0.0.1`.

### macOS Apple Silicon — AArch64 HVF VM Kit

VeerOS runs natively on Apple Silicon using Hypervisor.framework — no QEMU, no Docker.
The kit ships with a VM manager, a pre-signed runner, a FAT32 persistent disk, SSH, and
a built-in Veer secure console.

```bash
# Deploy the macOS VM kit (builds kernel, host tools, signs binary, formats disk)
./scripts/deploy-macos-veer-vm.sh

# Create and start a normal VM instance
~/VeerOS-VMs/veeros-vm create --name dev1 --target aarch64-hvf --mode normal
sudo -v && ~/VeerOS-VMs/veeros-vm start --name dev1

# Connect via Veer secure console
~/VeerOS-VMs/veeros-vm connect --name dev1

# Connect via SSH  (guest: root / toor)
ssh -o StrictHostKeyChecking=no root@192.168.2.100

# Create and start a Fold-managed instance
~/VeerOS-VMs/veeros-vm create --name fold1 --target aarch64-hvf --mode folded
sudo -v && ~/VeerOS-VMs/veeros-vm start --name fold1
```

Guest services after boot:

| Service | Address |
|---|---|
| SSH | `root@192.168.2.100:22` (password: `toor`) |
| Veer secure console | `192.168.2.100:2323` |
| Persistent disk | mounted at `/disk` (FAT32) |

Manager commands: `create` · `start` · `stop` · `list` · `status` · `logs [--follow]` · `connect` · `ssh` · `fold`

Full reference: `~/VeerOS-VMs/VeerOS-VM-Manual.txt` or [docs/macos-vm-manual.txt](docs/macos-vm-manual.txt)

### Windows Host Tools — veer-vm, fold, veer-connect

Windows now has a host-tools build path aligned with the macOS workflow.

```powershell
# Build debug host tools for native Windows target
./scripts/build-windows-host-tools.ps1

# Build release host tools
./scripts/build-windows-host-tools.ps1 release

# Optional target override:
#   $env:VEER_VM_WIN_TARGET = "x86_64-pc-windows-msvc"
#   $env:VEER_VM_WIN_TARGET = "all"
```

Artifacts are produced under `target/<target-triple>/<profile>/`:

- `veer-vm.exe`
- `fold.exe`
- `veer-connect.exe`

Notes:

- `veer-vm` on Windows runs VeerOS guests via QEMU + WHPX (x86_64 ISO path).
- `fold` on Windows now provides process lifecycle management for guest launch workflows.
- `veer-connect` builds and supports shell/file operations; interactive shell runs in line mode on non-Unix hosts.

Windows backend options for `veer-vm`:

```powershell
# Built-in QEMU + WHPX backend
./target/x86_64-pc-windows-msvc/debug/veer-vm.exe --backend qemu --kernel build/veeros.iso --arch x86_64 --memory 256

# Custom backend runner (default args)
./target/x86_64-pc-windows-msvc/debug/veer-vm.exe --backend custom --custom-runner C:/tools/my-vmm.exe --kernel build/veeros.iso --arch x86_64 --memory 256

# Custom backend runner (templated args)
./target/x86_64-pc-windows-msvc/debug/veer-vm.exe --backend custom --custom-runner C:/tools/my-vmm.exe `
  --custom-arg "--image" --custom-arg "{kernel}" --custom-arg "--mem" --custom-arg "{memory_mib}" --custom-arg "--cpu" --custom-arg "{cpus}"
```

Custom arg tokens: `{kernel}` `{memory_mib}` `{cpus}` `{arch}` `{disk}` `{disk_ro}` `{mac}`.

Convenience wrapper for Windows guest launch:

```powershell
# qemu backend (default)
./scripts/run-veer-vm-windows.ps1 -Build

# custom backend
./scripts/run-veer-vm-windows.ps1 -Backend custom -CustomRunner C:/tools/my-vmm.exe -Kernel build/veeros.iso -Memory 256

# auto-detect qemu and persist VEER_VM_QEMU for future shells
./scripts/run-veer-vm-windows.ps1 -Backend qemu -PersistQemuPath
```

QEMU detection helper:

```powershell
# print detected qemu path
./scripts/find-qemu-windows.ps1

# set VEER_VM_QEMU for current process and persist in user env
./scripts/find-qemu-windows.ps1 -SetProcessEnv -PersistUserEnv
```

### Demo Session

```
veeros> fabric
  Execution Fabric: 4/4 nodes healthy
    0  x86_64  local   4 cores  16384M   9%  host-demo
    1   arm64   rack   4 cores   8192M   3%  rpi5-edge-01
    2    rv32   rack   1 cores      0M   1%  esp32c6-sensor
    3  x86_64     dc  16 cores  65536M  15%  cloud-gpu-a100

veeros> intent submit deploy upgrade edge firmware
  intent #1 submitted (class=deploy)
  plan decomposed into 3 step(s):
    step 0: validate deployment config (independent)
    step 1: provision resources (depends-on)
    step 2: verify deployment health (depends-on)

veeros> agents spawn validate firmware image
  agent #0 spawned (planning)

veeros> memory set deploy.target rpi5-edge-01
  stored: deploy.target = rpi5-edge-01
```

---

## Capabilities

- **Multi-Architecture Kernel** — runs on RISC-V (32/64), ARM64, Xtensa, and
  x86-64; the same microkernel, abstracted at the instruction level.

- **Hardware Spectrum** — from 320 KB ESP32 microcontrollers (C3/C6/H2) through
  Raspberry Pi 5 SBCs to KVM-accelerated virtual machines on cloud metal; Xtensa
  (ESP32-S3) and ARM64 in-progress. On **Apple Silicon** VeerOS runs directly under
  Hypervisor.framework (`crates/kernel/aarch64_virt`) with virtio-mmio net + block,
  FAT32 persistent disk, SSH, and a built-in Veer secure console — no QEMU required.

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
  migrate transparently. WAN-scale overlay extends the fabric across the
  public Internet with NAT traversal, PQC-encrypted tunnels, and latency-aware
  routing — from LAN to planet-scale.

- **Native Orchestration** — container scheduling, desired-state reconciliation,
  rolling deployments, health-aware placement — without a separate control plane.

- **ZeroServices Architecture** — services are kernel objects, not containers
  with sidecars. The kernel provides service identity, mTLS, load balancing,
  circuit breaking, distributed tracing, and observability natively.
  `SYS_SVC_EXPOSE` replaces the entire API gateway tier — auth, rate limiting,
  TLS termination, and routing in a single syscall. Eliminates ~15 infrastructure
  services (Istio, Envoy, Kong, Consul, cert-manager, etc.) with zero proxies.

- **Packet-Level Network Intelligence** — stateful filtering, NAT, VPN tunnels,
  traffic shaping, and protocol identification as kernel primitives. VeerOS is
  the firewall.

- **Kernel-Native Enterprise Security** — the OS is the security appliance:
  802.1X network access control (replaces ClearPass/ISE), syscall-level endpoint
  detection & response (replaces CrowdStrike/Trellix), and continuous Zero Trust
  network access (replaces Zscaler/Cloudflare ZTNA). Every packet, every syscall,
  every auth event observed and enforced at the kernel — no agents, no sidecars.

- **Enterprise Device Management** — fabric join IS device enrollment. Hardware
  attestation, compliance enforcement, configuration profiles, OTA firmware
  updates, and remote wipe — all kernel-native. Replaces Intune, JAMF, SCCM,
  and Workspace ONE with zero MDM servers and zero agents.

- **Unified Console** — any node's shell can manage any other node. `attach`
  to remote nodes, `broadcast` commands fleet-wide, view distributed logs with
  `dmesg --fabric`, monitor the entire fleet with `top --fabric` — all over
  encrypted fabric channels. No central controller.

- **Structured Event System** — every kernel action emits a typed, structured
  event — not a string log. `KernelEvent` replaces logs, metrics, traces, and
  security signals with one primitive. Multi-sink fanout delivers events to
  serial, VGA, network, and storage simultaneously with priority-aware filtering.
  Replaces ELK, Prometheus, and APM tools.

- **Function Invocation Model** — beyond services: `invoke("auth.login", payload)`.
  No URLs, no endpoints, no long-running servers. Identity-based routing,
  ephemeral execution, versioned functions, short-lived identity tokens. The
  kernel resolves, routes, and load-balances — every invocation authenticated
  by cryptographic identity, not network address.

- **State Fabric** — global, distributed, service-less data plane. Key-value,
  streams, and objects with locality-aware replication, CRDTs for conflict-free
  offline operation, and tunable consistency (strong → eventual). Compute is
  stateless; state lives in the fabric. Replaces Redis, Kafka, and etcd.

- **QUIC-Native Fabric Protocol** — binary, zero-copy wire protocol over QUIC.
  Multiplexed streams, 0-RTT resumption, connection migration across IP changes.
  Every fabric message — invocations, state sync, events, management — uses the
  same compact binary framing. Decentralized peer-coordinated scheduling with
  cost/energy-aware placement.

- **WASM Sandbox** — WebAssembly execution sandbox alongside MicroVMs and
  containers. Portable bytecode, memory-safe, capability-constrained. Write
  functions in any language that compiles to WASM (Rust, C, Go, TypeScript).
  Lighter than containers, more portable than native binaries.

- **InterFabric Protocol (VeerLink)** — cross-fabric federation without shared
  networks, VPNs, or API gateways. Independent fabrics establish cryptographic
  trust contracts, exchange scoped identity tokens (never raw identities), and
  invoke functions across organizational boundaries with policy enforcement at
  both ends. Compromise a peer → revoke trust in < 1 second. An internet of
  fabrics, not an internet of endpoints.

- **Fabric Client (VeerFlow)** — the browser is obsolete. Composable views render
  directly from fabric state — no URLs, no REST, no cookies, no JS frameworks.
  Semantic navigation replaces address bars; identity-native auth replaces login
  forms. AI generates adaptive interfaces that reshape from a serial console to a
  full pixel framebuffer with window compositing. Functions publish their UI
  contract; the client renders it. `dist-desktop` adds a window compositor and
  GPU-accelerated rendering.

- **AI as a System Primitive** — autonomous agents, declarative intents, three-tier
  memory engine, and heterogeneous execution fabric are kernel primitives, not
  userspace libraries. The intent scheduler decomposes goals into agent DAGs,
  places them across the fabric, enforces budgets, and learns from outcomes via
  episodic memory. Inference engine, NL-aware shell, vision/voice pipelines, and
  on-device training extend the AI surface into userspace.

- **Deterministic Intelligence** — the fabric itself is a continuously learning
  system. Every execution records episodes, refines placement scores, updates
  behavioral baselines, and promotes repair strategies — all deterministically.
  Same history → same decision, always. No opaque weight matrices, no stochastic
  sampling, no training pipelines. LLMs are an opt-in inference tool, never the
  learning substrate.

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
  microkernel/    → Scheduler, IPC, VFS, capabilities, futex, channels,
                    agents, intents, memory engine, fabric, intent scheduler
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
│   │   ├── dist-cloud          + orchestration, ZeroServices, observability
│   │   └── dist-fleet          + device management, unified console
│   ├── dist-desktop            + Fabric Client, composable views, window compositor
│   └── dist-full               all components + priority scheduler
├── dist-rt                     + priority real-time scheduler
│   └── dist-xrt                + accelerator / GPU / FPGA / QPU (UAI)
├── dist-edge                   + edge AI inference, WiFi/BLE, sensor pipeline
│   └── dist-gateway            + Thread border router, Zigbee, MQTT broker
├── dist-firewall               + packet filter, NAT, VPN, DPI, traffic shaping
└── dist-security               + NAC + EDR + ZTNA (enterprise security)
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
| `dist-security` | firewall | Enterprise NAC + EDR + ZTNA | x86-64, ARM64 |
| `dist-fleet` | cluster | Fleet / device management + console | x86-64, ARM64, RPi 4/5 |
| `dist-desktop` | app | Fabric Client + window compositor | RPi 4/5 (HDMI), x86-64 |

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

# Enterprise security appliance (NAC + EDR + ZTNA)
cargo build -p kernel-qemu-pc --features dist-security

# Fleet management node
cargo build -p kernel-qemu-pc --features dist-fleet

# Desktop with Fabric Client
cargo build -p kernel-qemu-pc --features dist-desktop

# Mix-and-match — any combination is valid
cargo build -p kernel-qemu-pc --features dist-firewall,ai,sec-ztna
```

➡️ **Full distribution details:** see Phase 4 in [TODO.md](TODO.md)

---

## Vision

VeerOS is building toward a future where the operating system IS the
infrastructure — where there is no distinction between kernel, platform,
security appliance, orchestrator, and user interface.

**Near-term:** A production-quality microkernel running on real hardware across
four architectures, with AI-native primitives and a coherent distributed fabric.

**Mid-term:** Fabric-scale deployment where heterogeneous nodes — from MCUs to
GPU servers — form a single operating system. ZeroServices, State Fabric, and
the function invocation model eliminate the entire middleware tier.

**Long-term:** The Fabric Client replaces the browser. InterFabric Protocol
connects independent organizations without shared infrastructure. AI generates
interfaces, decomposes goals, and manages systems autonomously. The OS adapts,
learns, and operates — from a sensor on a factory floor to a constellation of
satellites — as one coherent whole.

**The endgame:** no containers, no sidecars, no API gateways, no browsers, no
cloud consoles, no MDM agents, no VPNs. Just the kernel, the fabric, and
intent.

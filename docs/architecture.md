# VeerOS Architecture

VeerOS is a **Rust-native, bare-metal operating system** targeting RISC-V
microcontrollers.  It cleanly separates **SoC drivers from kernel logic**
using a three-layer crate hierarchy, making it straightforward to add new
boards without rewriting the OS.

---

## Crate Hierarchy

```
┌─────────────────────────────────────────────────────────────────────┐
│  Layer 0 — arch                                                     │
│  Trait definitions: Platform, Serial, InterruptController,          │
│  TickTimer, NetworkDevice, TaskContext, Console                     │
└──────────────────────────┬──────────────────────────────────────────┘
                           │ implements
┌──────────────────────────▼──────────────────────────────────────────┐
│  Layer 1 — SoC drivers                                              │
│  ┌────────────────────┐  ┌──────────────────────┐                   │
│  │  soc-esp32          │  │  soc-qemu-virt        │                  │
│  │  • uart  (UART0)   │  │  • uart  (NS16550)    │                  │
│  │  • intc  (INTC)    │  │  • clint (mtime)      │                  │
│  │  • systimer        │  │  • virtio_net          │                  │
│  │  • wdt  (C3/C6/H2) │  │                       │                  │
│  │  • wifi             │  │                       │                  │
│  │  • mem  (map const) │  │                       │                  │
│  └────────────────────┘  └──────────────────────┘                   │
└──────────────────────────┬──────────────────────────────────────────┘
                           │ used by
┌──────────────────────────▼──────────────────────────────────────────┐
│  Layer 2 — Kernel binaries (one per board)                          │
│  ┌─────────────────────────┐  ┌─────────────────────────┐          │
│  │ kernel-xiao-esp32c6     │  │ kernel-qemu-virt         │          │
│  │ • _start assembly       │  │ • _start assembly        │          │
│  │ • watchdog disable      │  │ • CLINT timer setup      │          │
│  │ • scheduler + tasks     │  │ • scheduler + tasks      │          │
│  │ • shell (UART)          │  │ • shell (UART + TCP)     │          │
│  │ • linker: xiao-esp32c6.x│  │ • linker: qemu-virt.x   │          │
│  └─────────────────────────┘  └─────────────────────────┘          │
└─────────────────────────────────────────────────────────────────────┘

  Shared OS components (used by all kernels):
    microkernel  — Scheduler, PoolAllocator, Heap, DriverRegistry
    shell        — Interactive command shell (ShellEnv callbacks)
    net          — smoltcp integration, TcpSerial, auth
```

### Adding a new board

1. **Same SoC family** — add a new kernel crate under `crates/kernel/`,
   depend on the existing SoC crate, provide a board-specific linker script.

2. **New SoC** — create a SoC crate under `crates/soc/` implementing the
   `arch` traits, then add the kernel crate.

---

## Workspace layout

```
crates/
  arch/                     # Layer 0: architecture-neutral traits
  soc/
    esp32/                  # Layer 1: ESP32 RISC-V family (C3/C6/H2 via features)
    qemu_virt/              # Layer 1: QEMU virt RISC-V 32-bit machine
  kernel/
    xiao_esp32c6/           # Layer 2: kernel for Seeed XIAO ESP32-C6
    qemu_virt/              # Layer 2: kernel for QEMU virt
  microkernel/              # Shared: scheduler, allocator, driver registry
  shell/                    # Shared: interactive shell
  net/                      # Shared: smoltcp networking, TCP serial, auth
  distributions/            # Build profiles (minimal, app, rt, full)
```

---

## ESP32 variant support

The `soc-esp32` crate uses Cargo features to handle variant differences:

| Feature | SoC       | SRAM   | Address space            | Target triple              |
|---------|-----------|--------|--------------------------|----------------------------|
| `c3`    | ESP32-C3  | 400 KB | Split IRAM/DRAM          | riscv32imc-unknown-none-elf |
| `c6`    | ESP32-C6  | 512 KB | Unified @ 0x4080_0000    | riscv32imc-unknown-none-elf |
| `h2`    | ESP32-H2  | 320 KB | Unified @ 0x4080_0000    | riscv32imc-unknown-none-elf |

Shared peripherals (UART0, SYSTIMER, INTC) use the same base addresses.
Variant-specific code (WDT registers, memory map constants) is `#[cfg]`-gated.

---

## Boot sequence (ESP32-C6)

1. ROM bootloader loads ELF from SPI flash into HP SRAM at `0x4080_0000`
2. `_start` (assembly): disable interrupts, set stack pointer, zero BSS
3. `_rust_start`: disable watchdogs, init console, init heap
4. Register drivers (UART, INTC, SYSTIMER)
5. Install trap vector, configure interrupt controller
6. Start SYSTIMER tick (1 ms)
7. Create tasks: idle (priority 0), shell (priority 1)
8. `_veer_start_first_task` → `mret` into first runnable task

## Boot sequence (QEMU virt)

Same flow, but uses CLINT timer instead of SYSTIMER, and adds a network
listener task for remote TCP shell over VIRTIO-NET.

---

## Key Design Principles

- **Portability** — kernel logic depends only on `arch` traits, never on
  register addresses.
- **Extensibility** — new SoC = new crate implementing traits; new board =
  new kernel crate with linker script.
- **Minimalism** — each layer does one thing; no unnecessary abstractions.

- **Networking-First**  
  `smoltcp` integrated directly into the kernel.

- **Security**  
  Strong memory isolation using PMP or MPU.

- **Future-Proof**  
  Designed to scale from microcontrollers to multicore systems.

---

## Process Model (Current → Planned)

### Current (v0.1): Flat Task Model

```
┌───────────────────────────────────────────────────────┐
│  Single address space (M-mode, no MMU/PMP)            │
│                                                       │
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐         │
│  │ idle   │ │ shell  │ │ net    │ │ user   │ ...×16  │
│  │ task 0 │ │ task 1 │ │ task 2 │ │ task 3 │         │
│  └────────┘ └────────┘ └────────┘ └────────┘         │
│  Fixed TCB table [Tcb; 16], round-robin / priority    │
│  Single-slot IPC mailbox, no locks, no processes      │
└───────────────────────────────────────────────────────┘
```

- Tasks are flat: no parent-child, no address space isolation
- `TaskContext` is RISC-V 32-GPR specific
- Sleep uses `gpr[0]` overload instead of a proper field
- IPC is fire-and-forget single-slot (overwrites on full)

### Planned (v0.2+): Process + Thread + Multi-Arch

```
┌──────────────────────────────────────────────────────────────────┐
│  Kernel (privileged: M-mode / EL1 / Ring 0)                     │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │ Scheduler │ IPC queues │ Futex table │ Socket layer      │    │
│  └──────────────────────────────────────────────────────────┘    │
├──────────────────────────────────────────────────────────────────┤
│  Process A (ASID 1)              │  Process B (ASID 2)          │
│  ┌────────┐ ┌────────┐          │  ┌────────┐                  │
│  │ Thread 0│ │ Thread 1│          │  │ Thread 0│                  │
│  │ (main) │ │ (worker)│          │  │ (main) │                  │
│  └────────┘ └────────┘          │  └────────┘                  │
│  Shared: heap, .text, .rodata   │  Own: heap, .text, .rodata   │
│  PMP/MPU/page-table isolated    │  PMP/MPU/page-table isolated │
└──────────────────────────────────────────────────────────────────┘
```

Key changes:
- **`SavedContext` trait** — per-arch context type (riscv32, riscv64, aarch64, x86-64)
- **Process owns address space** — ASID, memory regions, handle table, child list
- **Thread runs within process** — own stack + context, shares process memory
- **BlockReason enum** — typed blocking: `Sleep`, `IpcRecv`, `MutexWait`, `Join`, `IoWait`
- **Message queues** — bounded ring buffers replace single-slot mailboxes
- **Futex-based synchronization** — Mutex, Condvar, Semaphore, RwLock in userlib
- **Socket API** — unified local + network sockets with handle table
- **Async executor** — `no_std` poll-based runtime in userlib using `SYS_POLL_WAIT`

### Multi-Architecture `SavedContext`

```rust
// In arch crate — trait that each arch implements:
pub trait SavedContext: Copy + Sized {
    fn zero() -> Self;
    fn set_pc(&mut self, pc: usize);
    fn set_sp(&mut self, sp: usize);
    fn set_arg(&mut self, n: usize, val: usize);  // syscall args
    fn get_ret(&self, n: usize) -> usize;          // return values
    fn pc(&self) -> usize;
    fn sp(&self) -> usize;
    fn syscall_nr(&self) -> usize;
}

// Per-arch implementations:
//   riscv32: 32 GPRs (x0-x31) + pc + mstatus      = 136 bytes
//   riscv64: 32 GPRs (x0-x31) + pc + sstatus       = 272 bytes
//   aarch64: 31 GPRs (x0-x30) + SP + PC + PSTATE   = 272 bytes
//   x86-64:  16 GPRs + RIP + RFLAGS + segments      = 176 bytes
```

## Summary

VeerOS provides a **clean, modern OS architecture** that combines:
- Rust safety with zero-cost abstractions
- Strong abstraction boundaries (arch → soc → kernel → userlib)
- First-class networking and wireless radio stacks
- AI-native execution as a kernel primitive, not an afterthought
- Kernel-native enterprise security (NAC + EDR + ZTNA) — the OS is the security appliance
- Enterprise device management — fabric join is enrollment, no MDM servers
- WAN-scale fabric spanning the public Internet with NAT traversal and PQC tunnels
- ZeroServices — services as kernel objects, eliminating API gateways and service meshes
- Function invocation model — `invoke("name.function", payload)` replaces URLs and endpoints
- State Fabric — distributed KV, streams, objects with CRDTs and locality-aware replication
- Structured event system — typed kernel events replace string logs, metrics, and traces
- QUIC-native fabric protocol — binary, zero-copy, multiplexed, with decentralized scheduling
- WASM sandbox — portable WebAssembly execution alongside MicroVMs and containers
- Unified console for fleet-wide management from any node
- InterFabric Protocol (IFP / VeerLink) — cross-fabric federation with scoped identity, bilateral policy, and zero shared networks
- Developer SDK + migration path from existing infrastructure
- Long-term scalability from 320 KB MCUs to cloud GPU clusters

This makes VeerOS suitable for **embedded devices, IoT platforms, autonomous
robotics, satellite systems, telecom infrastructure, medical devices, AI
pipelines, tactical edge computing, enterprise networks, and global-scale
fleet management**.

---

## Core Design Principles

VeerOS is not better Kubernetes, lighter microservices, or improved serverless.
It is a **different abstraction boundary** — where OS, infrastructure, and
platform collapse into one.

| Principle | Traditional | VeerOS |
|-----------|------------|--------|
| **Identity > Network** | Trust based on IP, subnet, VPC | Every action tied to cryptographic identity |
| **Invoke > Endpoint** | URLs, REST, service discovery | Function invocation — `invoke("name.fn", payload)` |
| **Events > Logs** | String logs, separate metrics/traces | Typed, structured kernel events — one primitive |
| **Fabric > Infrastructure** | K8s + mesh + gateway + obs stack | One unified execution + state + policy fabric |
| **State > Services** | Stateful services wrapping DBs | Stateless compute + persistent State Fabric |
| **Federation > Exposure** | API gateways + VPNs + OAuth2 | InterFabric — trust-bound federated invocation |

```
Traditional Stack:
  App → Services → Containers → K8s → Mesh → Gateway → Observability

VeerOS (single fabric):
  Code → Fabric → Execution
               → State
               → Policy
               → Events

VeerOS (multi-fabric):
  Fabric A → InterFabric (IFP) → Fabric B
             (trust + identity + policy at boundary)
```

---

## AI-Native Execution Architecture

VeerOS treats AI workloads as first-class kernel primitives. Instead of bolting
agent frameworks on top of the OS, the kernel itself understands goals, agents,
and plans.

### Conceptual Stack

```
┌─────────────────────────────────────────────────────────────────┐
│  Userspace                                                      │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────────────────┐  │
│  │ AI Shell │  │ Agent    │  │  Application / NL Pipeline   │  │
│  │ (10I)    │  │ Factory  │  │  (submit intents via syscall)│  │
│  └────┬─────┘  └────┬─────┘  └────────────┬─────────────────┘  │
│       │              │                     │                    │
│  ─────┼──────────────┼─────────────────────┼──── syscall ──────│
│       ▼              ▼                     ▼                    │
├─────────────────────────────────────────────────────────────────┤
│  Kernel — AI-Native Layer                                       │
│                                                                 │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │             Intent Scheduler (meta-scheduler)            │   │
│  │   6-phase tick: decompose → assign → monitor →           │   │
│  │                 sync → record → health                   │   │
│  └─────┬──────────┬──────────┬──────────┬───────────────────┘   │
│        │          │          │          │                        │
│  ┌─────▼────┐ ┌──▼───────┐ ┌▼────────┐ ┌▼──────────────────┐   │
│  │  Intent  │ │  Agent   │ │ Memory  │ │ Execution Fabric  │   │
│  │  Engine  │ │  Table   │ │ Engine  │ │ (node placement)  │   │
│  │ (goals→  │ │ (32 slots│ │ 3-tier: │ │ Local + remote    │   │
│  │  plans)  │ │  goal+   │ │ context │ │ node scoring      │   │
│  │ step DAG │ │  context)│ │ persist │ │ capability match  │   │
│  │ decompose│ │ lifecycle│ │ episodic│ │ health monitor    │   │
│  └──────────┘ └──────────┘ └─────────┘ └───────────────────┘   │
│                                                                 │
├─────────────────────────────────────────────────────────────────┤
│  Kernel — Classic Layer                                         │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐          │
│  │Scheduler │ │ IPC/Chan │ │   VFS    │ │ Sockets  │ ...      │
│  │(threads) │ │ (futex)  │ │ (files)  │ │ (net)    │          │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘          │
└─────────────────────────────────────────────────────────────────┘
```

### Key Modules

| Module | File | Purpose |
|--------|------|---------|
| Agent Table | `microkernel/src/agent.rs` | Autonomous execution units with goals, context memory, parent/child hierarchy, budget tracking |
| Intent Engine | `microkernel/src/intent.rs` | Declarative goal decomposition into step DAGs with dependency relations |
| Memory Engine | `microkernel/src/memory_engine.rs` | Three-tier memory: per-agent context, persistent key-value, episodic ring buffer |
| Execution Fabric | `microkernel/src/fabric.rs` | Heterogeneous node registry with capability matching and placement scoring |
| Intent Scheduler | `microkernel/src/intent_sched.rs` | Meta-scheduler: orchestrates intent→agent→fabric→memory feedback loop |

### Syscall Surface (0xF0–0xFF)

| Nr | Name | Capability | Purpose |
|----|------|------------|---------|
| 0xF0 | `SYS_AGENT_SPAWN` | AGENT | Spawn agent with goal, priority, entry point |
| 0xF1 | `SYS_AGENT_STATUS` | AGENT | Query agent state + CPU ticks used |
| 0xF2 | `SYS_AGENT_COMPLETE` | AGENT | Mark agent completed or failed |
| 0xF3 | `SYS_AGENT_CTX_SET` | AGENT | Set key-value in agent context memory |
| 0xF4 | `SYS_AGENT_CTX_GET` | AGENT | Get value from agent context memory |
| 0xF5 | `SYS_INTENT_SUBMIT` | INTENT | Submit declarative goal to intent engine |
| 0xF6 | `SYS_INTENT_STATUS` | INTENT | Query intent lifecycle status |
| 0xF7 | `SYS_INTENT_CANCEL` | INTENT | Cancel in-flight intent |
| 0xF8 | `SYS_MEMORY_STORE` | MEMORY_ENGINE | Store key-value in persistent memory |
| 0xF9 | `SYS_MEMORY_QUERY` | MEMORY_ENGINE | Query persistent memory by key |
| 0xFA | `SYS_FABRIC_STATUS` | TASK_BASIC | Query fabric node counts |
| 0xFB | `SYS_INTENT_SCHED_STATS` | TASK_BASIC | Query scheduler statistics |
| 0xFC | `SYS_AGENT_COUNT` | AGENT | Get active agent count |

### Data Flow

```
User submits intent (SYS_INTENT_SUBMIT)
    → IntentEngine stores IntentDescriptor (Pending)
    → IntentScheduler.tick():
        1. Decompose: intent → Plan (step DAG)
        2. Assign: for each ready step, select fabric node + spawn Agent
        3. Monitor: check budget/deadline, expire over-budget agents
        4. Sync: agent completion → update plan step → check intent resolved
        5. Record: fulfilled/failed intents → EpisodicMemory + PersistentMemory
        6. Health: fabric node heartbeat check
    → Agent executes on kernel thread (task_id link)
    → Agent reads/writes context memory (SYS_AGENT_CTX_SET/GET)
    → Agent completes (SYS_AGENT_COMPLETE)
    → Intent fulfilled → episodic record + persistent knowledge updated
```

---

## The Fabric as a Learning System

VeerOS does not bolt AI onto an operating system. The distributed execution
fabric itself is a continuously learning, deterministic intelligence.

### Learning Loop

The Intent Scheduler's 6-phase tick embeds learning directly into execution:

```
Phase 1: Decompose  — intent → plan DAG (informed by past episodes)
Phase 2: Assign     — select nodes + spawn agents (placement scores from history)
Phase 3: Monitor    — track budget, deadline, resource usage
Phase 4: Sync       — agent completion updates plan state
Phase 5: Record     — outcome written to Episodic Memory    ← THIS IS THE LEARNING STEP
Phase 6: Health     — node heartbeats, integrity signals update baselines
```

Every execution cycle is both **inference** (using accumulated knowledge
to make decisions) and **training** (recording outcomes to refine future
decisions). There is no separate training pipeline.

### Distributed Knowledge

The "model" is not a weight matrix. It is the collective state distributed
across every node in the fabric:

| Knowledge Type | Storage | Scope | Purpose |
|---|---|---|---|
| Episodic Memory | Per-node ring buffer | Local + gossip | What happened, what worked |
| Placement Scores | Execution Fabric node table | Per-node | Which nodes are good at what |
| Behavioral Baselines | Integrity Engine (Phase 26) | Per-component | What is normal behavior |
| Repair Strategies | Strategy Registry | Fleet-wide via gossip | What fixes work for what |
| Distilled Rules | Persistent Memory | Fleet-wide | Compressed episode patterns |

Knowledge propagates fleet-wide via gossip — a repair strategy proven on one
node becomes available to all nodes without a central registry.

### Determinism Guarantee

Every learning mechanism is **fully deterministic**:

- **Placement scoring** — explicit weighted formula:
  `score = w₁×capability_match + w₂×resource_fit + w₃×locality + w₄×load`.
  Same inputs → same placement, always.

- **Intent decomposition** — rule-based constraint matching, not an LLM.
  The code explicitly states: "deterministic, not stochastic."

- **Anomaly detection** — statistical z-score against observed baseline.
  Reproducible given the same observation history.

- **Repair strategy selection** — score-ranked registry. Strategies promoted
  on success, demoted on failure. Given the same repair history, the same
  strategy will always be selected.

- **Knowledge distillation** — episodes compressed into explicit IF/THEN rules
  (Phase 26E). Rules are human-readable and auditable.

**Given the same history, the fabric will always make the same decision.**
Every adaptation is traceable to specific episodic evidence, not statistical
correlation.

### Where LLMs Fit (and Don't)

LLMs (Phase 10) are an inference *tool* the fabric uses — never the learning
substrate:

| Fabric Mechanism | Uses LLM? | Notes |
|---|---|---|
| Intent decomposition | No | Rule-based, deterministic |
| Agent placement | No | Weighted scoring formula |
| Anomaly detection | No | Statistical z-score |
| Repair strategy ranking | No | Episode-based score |
| Knowledge distillation | No | Rule extraction from episodes |
| NL shell commands (10I) | Yes | Intent classifier, argmax (deterministic) |
| Self-explanation text (26C) | Yes | Generates human-readable causal summary |
| Fix proposal generation (26D) | Yes | Proposes repair — always simulation-validated |
| Cloud escalation (10F) | Yes | External API — privacy-gated, opt-in |

All LLM-assisted decisions are opt-in, validated, and auditable. The fabric
operates correctly without them — LLMs accelerate human interaction, they
don't make autonomous decisions.

### Design Consequence

```
Conventional AI:    Train offline → Deploy model → Serve inference
VeerOS Fabric:      Execute → Record → Refine → Execute (continuous loop)

Conventional AI:    Central GPU cluster trains a weight matrix
VeerOS Fabric:      Every node learns independently, shares via gossip

Conventional AI:    Model is opaque (billions of parameters)
VeerOS Fabric:      "Model" is inspectable episodes + explicit rules + scored strategies

Conventional AI:    Non-deterministic (sampling, FP variance, batch effects)
VeerOS Fabric:      Deterministic (same history → same decision → same outcome)
```

The fabric doesn't use a model. **The fabric IS the model.**

---

## Target Application Domains

The AI-native architecture is designed for domains where autonomous decision-making,
heterogeneous hardware, and real-time coordination intersect.

### Edge AI & Industrial IoT

**Problem:** A factory floor with 200 sensors, 10 gateways, and 2 cloud GPUs
requires MQTT + Node-RED + Kubernetes + TensorFlow Serving — 5+ middleware layers.

**VeerOS approach:** One `intent submit pipeline` call. The kernel decomposes
across the fabric — sensors ingest, gateways normalize, GPU runs inference. No
middleware, no container runtime, no orchestrator. The Memory Engine stores
calibration data and anomaly thresholds; the Episodic Memory records detection
history for model retraining.

**Distribution profile:** `dist-edge` (sensors) + `dist-ai` (gateways) +
`dist-cloud` (GPU nodes).

### Autonomous Robotics & Drones

**Problem:** A delivery drone must simultaneously navigate, avoid obstacles,
manage battery, and communicate with base — traditionally requiring ROS2 with
complex node lifecycle management and inter-process communication.

**VeerOS approach:** Each function is a kernel agent with a compute budget and
deadline. If the navigation agent detects low battery, it submits
`intent submit admin emergency-return-to-base` — the kernel re-plans in
microseconds. Agent hierarchies let a fleet coordinator spawn per-drone children.
Budget enforcement guarantees safety-critical agents meet deadlines.

**Distribution profile:** `dist-rt` (flight controller) + `dist-edge` (perception).

### Satellite & Space Systems

**Problem:** CubeSats have severe power and compute constraints. Traditional
Linux-based flight software has high overhead and poor determinism.

**VeerOS approach:** The RISC-V compute module, camera, and radio are fabric
nodes. Ground control sends an imaging intent; the kernel budgets compute ticks,
schedules capture only when power is sufficient, queues downlink for the next
ground pass. Episodic Memory provides flight history for anomaly diagnosis.
`dist-minimal` fits in < 512 KB.

**Distribution profile:** `dist-minimal` or `dist-rt`.

### Telecom & 5G Network Functions

**Problem:** Network Function Virtualization (NFV) requires heavy container
infrastructure to deploy packet forwarding, beamforming, and encryption functions
across heterogeneous hardware.

**VeerOS approach:** Network functions become agents with placement constraints
(e.g., `latency < 1ms` → FPGA node). If a node degrades, the Intent Scheduler
re-places the agent on the next-best node. The Fabric tracks per-node capabilities
(crypto accelerator, NIC offload) for optimal placement.

**Distribution profile:** `dist-firewall` + `dist-cluster`.

### Medical Devices & Wearables

**Problem:** Safety-critical real-time constraints (insulin pump timing) cannot
be guaranteed by a general-purpose userspace scheduler.

**VeerOS approach:** Sensor agent runs on wearable RISC-V, stores readings in
Persistent Memory, phone agent detects dangerous trends and spawns a
critical-priority dosage agent. Kernel-level budget enforcement guarantees
deadline compliance. Episodic Memory provides a complete patient event log.

**Distribution profile:** `dist-rt` (implantable) + `dist-edge` (phone gateway).

### Distributed ML / AI Pipelines

**Problem:** Distributed training requires Slurm, Ray, or Horovod with manual
resource management and no unified observability.

**VeerOS approach:** `intent submit pipeline distributed training across GPUs`.
The Fabric maps GPU-capable nodes; the scheduler spawns data-loader, trainer, and
checkpoint agents with the right placement constraints. Node overloads → automatic
agent migration. Episodic Memory records training runs for experiment tracking.

**Distribution profile:** `dist-ai` + `dist-cluster`.

### Defense & Tactical C2

**Problem:** Disconnected, intermittent, limited (DIL) networks make centralized
orchestration impossible.

**VeerOS approach:** Nodes join the fabric when in range. `intent submit monitor
persistent surveillance of sector 7` — the kernel decomposes across available
assets, re-plans when a drone goes offline, records all decisions in Episodic
Memory for after-action review. Persistent Memory stores rules of engagement
that agents enforce autonomously. No cloud dependency.

**Distribution profile:** `dist-cluster` + `dist-rt`.

### Enterprise Security & Zero Trust

**Problem:** Enterprise security requires a stack of point products — ClearPass
for NAC, CrowdStrike for EDR, Zscaler for ZTNA — each with agents, appliances,
and cloud consoles. The OS doesn't participate in security enforcement.

**VeerOS approach:** The kernel IS the security appliance. 802.1X authentication
happens at the packet level — no separate RADIUS appliance. EDR operates at
syscall granularity — every process's behavior profiled against learned baselines,
anomalies detected and responded to in microseconds. ZTNA evaluates device
posture, user identity, and request context on every access — not just at tunnel
establishment. `sec-nac`, `sec-edr`, `sec-ztna` feature flags compose the
security surface needed for each deployment.

**Distribution profile:** `dist-security` (all three) or individual `sec-*` flags.

### Enterprise Fleet & Device Management

**Problem:** Managing a fleet of devices requires Intune/JAMF/SCCM servers,
enrollment protocols, compliance scanners, and remote management agents.

**VeerOS approach:** `cluster join` = MDM enrollment. Device presents hardware
attestation (TPM/eFuse/boot measurements), receives a signed certificate,
auto-populates inventory record. Compliance rules evaluated continuously in
kernel — non-compliant devices quarantined or auto-remediated. Configuration
profiles pushed via fabric gossip. OTA firmware updates, remote wipe, fleet
queries — all via `intent submit` or `fleet` shell commands.

**Distribution profile:** `dist-fleet`.

### Global-Scale WAN Fabric

**Problem:** Connecting devices across NAT, cellular, and multi-site networks
requires VPN infrastructure — Tailscale, ZeroTier, or complex IPsec tunnels.

**VeerOS approach:** The execution fabric extends to WAN transparently. STUN
discovery + UDP hole punching + TURN relay fallback ensure connectivity through
any NAT topology. All tunnels PQC-hybrid encrypted (ChaCha20-Poly1305 +
ML-KEM). Latency-aware and bandwidth-aware routing selects optimal paths.
WAN partitions handled gracefully — nodes operate autonomously and sync on
reconnection. The intent scheduler is WAN-aware: heavy workloads avoid
constrained links, latency-sensitive agents stay local.

**Distribution profile:** any profile + `fabric-wan`.

---

## ZeroServices Architecture

VeerOS reimagines microservices as kernel primitives. Instead of deploying
containers with sidecar proxies, services are kernel objects with built-in
identity, routing, encryption, observability, and resilience.

### Conceptual Stack

```
┌──────────────────────────────────────────────────────────────┐
│  Application Code (pure business logic)                      │
│  register("payments") → expose(443) → serve()                │
├──────────────────────────────────────────────────────────────┤
│  Kernel — ZeroServices Layer                                 │
│  ┌───────────┐ ┌────────────┐ ┌────────────┐ ┌───────────┐  │
│  │  Service   │ │   Service  │ │   Load     │ │  Circuit  │  │
│  │  Registry  │ │   Router   │ │  Balancer  │ │  Breaker  │  │
│  │ (gossip-   │ │ (name →    │ │ (per-svc   │ │ (per-svc  │  │
│  │  replicated│ │  endpoint) │ │  strategy) │ │  state)   │  │
│  └───────────┘ └────────────┘ └────────────┘ └───────────┘  │
│  ┌───────────┐ ┌────────────┐ ┌────────────┐ ┌───────────┐  │
│  │  mTLS     │ │  Rate      │ │  Dist.     │ │  Edge     │  │
│  │  (auto)   │ │  Limiter   │ │  Tracing   │ │  (ZTNA +  │  │
│  │           │ │            │ │  (auto)    │ │  expose)  │  │
│  └───────────┘ └────────────┘ └────────────┘ └───────────┘  │
├──────────────────────────────────────────────────────────────┤
│  Kernel — AI-Native + Classic Layer (agents, fabric, IPC)    │
└──────────────────────────────────────────────────────────────┘
```

### Syscall Surface (0xD0–0xD5)

| Nr | Name | Purpose |
|----|------|---------|
| 0xD0 | `SYS_SVC_REGISTER` | Register service with name, version, capabilities |
| 0xD1 | `SYS_SVC_DEREGISTER` | Graceful deregistration with drain |
| 0xD2 | `SYS_SVC_DISCOVER` | Find services by name/capability/version |
| 0xD3 | `SYS_SVC_EXPOSE` | Expose service on public port with auth + rate limiting |
| 0xD4 | `SYS_SVC_CALL` | Service-to-service call (kernel routes + load balances) |
| 0xD5 | `SYS_SVC_METRICS` | Query per-service metrics (latency, errors, throughput) |

### What ZeroServices Eliminates

```
Traditional Stack                → VeerOS ZeroServices
─────────────────────────────────────────────────────────────
Kubernetes Pods + Deployments    → Kernel service objects
Docker / containerd runtime      → Kernel service lifecycle
Istio / Linkerd sidecar proxies  → Kernel-native mTLS + routing
Envoy data plane                 → Kernel IPC + fabric transport
Consul / CoreDNS service disc.  → Gossip-replicated service registry
Kong / Nginx / Envoy API GW     → ZTNA edge + kernel rate limiting
Prometheus + Grafana metrics     → Kernel ring buffer metrics
Jaeger / Zipkin tracing          → Kernel auto-injected trace context
cert-manager TLS provisioning    → Kernel keystore + auto-rotation
```

**Total: ~15 infrastructure services eliminated → 0 sidecars, 0 proxies, 0 gateways.**

---

## Enterprise Security Architecture

### Kernel-Native Security Stack

```
┌──────────────────────────────────────────────────────────────┐
│                    Unified Security Policy                    │
│  (capability-based, continuous evaluation, per-request)       │
├──────────┬──────────────────┬────────────────────────────────┤
│   NAC    │       EDR        │           ZTNA                 │
│  802.1X  │  Syscall monitor │  Continuous auth               │
│  RADIUS  │  Behavior learn  │  Device posture                │
│  Posture │  Anomaly detect  │  Request-level policy          │
│  Quarant.│  Auto-response   │  Context-aware access          │
├──────────┴──────────────────┴────────────────────────────────┤
│  Kernel — PQC Crypto · Capabilities · Isolation Domains      │
└──────────────────────────────────────────────────────────────┘
```

**NAC** — 802.1X authenticator at the packet level; RADIUS client for external
AAA; device posture assessment on fabric join; capability-based network
segmentation replaces VLANs; non-compliant devices quarantined automatically.

**EDR** — per-process syscall frequency histograms; behavioral baseline learning;
real-time anomaly scoring; automated response from audit log to process kill;
forensic data preserved in episodic memory.

**ZTNA** — every request evaluated against (user identity × device posture ×
resource sensitivity × network context); continuous re-evaluation, not
one-time VPN auth; micro-segmented access — each service requires explicit
capability grant.

---

## Unified Console Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  Operator Shell (any node)                                   │
│  ┌──────────┐ ┌──────────────┐ ┌──────────────────────────┐ │
│  │ attach   │ │ broadcast    │ │ dmesg/top/ps --fabric    │ │
│  │ <node>   │ │ <cmd>        │ │ (fleet-wide view)        │ │
│  └────┬─────┘ └──────┬───────┘ └────────────┬─────────────┘ │
│       │              │                      │               │
│  ─────┼──────────────┼──────────────────────┼─── fabric ───│
│       ▼              ▼                      ▼               │
│  ┌─────────┐  ┌──────────┐          ┌──────────────┐       │
│  │ Node A  │  │ Node B   │  ···     │ Node N       │       │
│  │ shell   │  │ executes │          │ streams logs │       │
│  └─────────┘  └──────────┘          └──────────────┘       │
└──────────────────────────────────────────────────────────────┘
```

Any node's shell can manage any other node:
- `attach <node>` — remote shell session over encrypted fabric channel
- `broadcast <cmd>` — execute across all nodes, aggregate output
- `select <group> <cmd>` — target node groups
- `dmesg --fabric` — time-correlated kernel logs fleet-wide
- `top --fabric` — cluster-wide CPU/memory/task monitoring
- `ps --fabric` — all processes across all nodes
- `auditlog --fabric` — merged security audit trail

---

## Structured Event System (Events > Logs)

VeerOS replaces string-based logging with typed, structured kernel events as
the fundamental observability primitive.

```
┌──────────────────────────────────────────────────────────────┐
│  Kernel Event Bus (ring buffer)                              │
│  KernelEvent { timestamp, node_id, type, source, payload }  │
│  Types: Log | Metric | Trace | Security | Lifecycle         │
├──────────┬───────────┬───────────┬───────────────────────────┤
│  Serial  │  VGA /    │  Network  │  Storage   │  Memory     │
│  (panic  │  Display  │  (fabric  │  (SD/NVMe  │  (episodic  │
│  safe)   │  (rich)   │  stream)  │  persist)  │  for AI)    │
└──────────┴───────────┴───────────┴───────────┴──────────────┘
```

Every action emits a typed event:
```
{ type: "Trace", operation: "auth.login", latency: 12, status: "ok" }
{ type: "Security", threat_level: "High", category: "anomaly", pid: 42 }
{ type: "Metric", name: "cpu.usage", value: 0.73, node: "rpi5-edge" }
```

- **Multi-sink fanout** — same event delivered to serial (last-resort), display
  (rich), network (global), and storage (persistent) simultaneously
- **Priority-aware** — panic events guaranteed delivery; debug events shed under
  backpressure
- **Queryable** — `events where type=Security AND threat_level > Medium since 1h`

Replaces: ELK, Prometheus, Jaeger, Datadog, Splunk — with zero instrumentation.

---

## Function Invocation Model (Invoke > Endpoint)

Beyond ZeroServices: everything is a function invocation, not an HTTP endpoint.

```
Traditional:  Client → DNS → LB → Gateway → Service → DB
VeerOS:       Client → invoke("auth.login", payload) → result
```

- **Identity-based routing** — routing by cryptographic caller/callee identity,
  not IP/DNS/URL. `NodeId + FunctionId` globally unique.
- **Ephemeral execution** — functions execute, return, release. No long-running
  server processes required.
- **Short-lived identity** — per-invocation tokens with configurable TTL (30s
  default); auto-rotated; replaces long-lived API keys.
- **Event triggers** — `on_event("user.created", "email.welcome")` — kernel
  event bus triggers downstream functions automatically.
- **Versioned** — `invoke("auth.login@v2", payload)` routes to specific version;
  traffic splitting for canary deploys.

---

## State Fabric (Distributed Data Plane)

Compute is stateless. State lives in the fabric. Replaces databases, caches,
queues, and config stores.

```
┌──────────────────────────────────────────────────────────────┐
│  State Fabric                                                │
│  ┌────────────┐  ┌────────────────┐  ┌────────────────────┐ │
│  │  Key-Value │  │  Streams       │  │  Objects           │ │
│  │  (→ Redis, │  │  (→ Kafka,     │  │  (→ S3, MinIO)     │ │
│  │   etcd)    │  │   RabbitMQ)    │  │                    │ │
│  └────────────┘  └────────────────┘  └────────────────────┘ │
│  Consistency: Strong | Session | Causal | Eventual           │
│  Replication: locality-aware, CRDT merge, offline tolerance  │
└──────────────────────────────────────────────────────────────┘
```

- **CRDT support** — conflict-free merge on partition heal; counters, sets,
  registers, maps — no data loss in DIL environments
- **Locality-aware** — hot keys migrate toward consumers; cold data stays
  at origin; configurable replication factor
- **State triggers** — state mutations trigger function invocations; enables
  reactive patterns without polling
- **State-aware scheduling** — co-locate compute with its data

---

## Fabric Protocol & Transport

All fabric communication uses one compact binary protocol over QUIC.

```
FabricFrame (16-byte header + payload):
┌────────┬──────────┬───────┬──────────────┬──────────┬─────────┐
│version │ msg_type │ flags │ correlation  │ sender   │ payload │
│  u8    │   u8     │ u16   │   u32        │ NodeId   │ [u8]    │
└────────┴──────────┴───────┴──────────────┴──────────┴─────────┘
Message types: Invoke | StateOp | Event | Gossip | Console | Control
```

- **QUIC transport** — multiplexed streams, 0-RTT resumption, connection
  migration across IP changes, built-in TLS 1.3
- **Zero-copy forwarding** — relay nodes forward without deserializing payload
- **Decentralized scheduling** — peer-coordinated placement via gossip; no
  central orchestrator; cost/energy-aware scoring

---

## InterFabric Architecture (IFP / VeerLink)

InterFabric is NOT networking — it is **federated invocation between trust
domains**. Independent VeerOS fabrics communicate without collapsing isolation
or reintroducing gateways.

### Mental Model

```
Old World:
  Org A → API Gateway → Internet → API Gateway → Org B

VeerOS World:
  Fabric A → Federation Layer → Fabric B
  (identity + policy at boundary, no gateways)
```

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Fabric A (fabric://veer.prod.india)                            │
│  ┌──────────────────────────────────────────────────────────┐   │
│  │ Execution + State + Policy + Events (internal)           │   │
│  └────────────────────────┬─────────────────────────────────┘   │
│                           │                                     │
│  ┌────────────────────────▼─────────────────────────────────┐   │
│  │ Federation Layer                                          │   │
│  │ • Outbound policy evaluation (can we call them?)          │   │
│  │ • Scoped identity token generation (never raw IDs)        │   │
│  │ • Trust contract enforcement                              │   │
│  └────────────────────────┬─────────────────────────────────┘   │
└───────────────────────────┼─────────────────────────────────────┘
                            │ IFP over QUIC (mTLS, PQC-hybrid)
┌───────────────────────────┼─────────────────────────────────────┐
│  ┌────────────────────────▼─────────────────────────────────┐   │
│  │ Federation Layer                                          │   │
│  │ • Inbound policy evaluation (do we accept this?)          │   │
│  │ • Scoped token verification (signed claims only)          │   │
│  │ • Audit logging (immutable, hash-chained)                 │   │
│  └────────────────────────┬─────────────────────────────────┘   │
│                           │                                     │
│  ┌────────────────────────▼─────────────────────────────────┐   │
│  │ Execution + State + Policy + Events (internal)           │   │
│  └──────────────────────────────────────────────────────────┘   │
│  Fabric B (fabric://partner.analytics.eu)                       │
└─────────────────────────────────────────────────────────────────┘
```

### Key Properties

- **Cryptographic fabric identity** — `fabric://name` URI scheme; Ed25519+ML-DSA
  root keypair per fabric; trust established via explicit handshake, never implicit
- **Scoped identity tokens** — per-invocation, short-lived, CBOR-encoded;
  internal identities (PIDs, service names) never cross the boundary
- **Bilateral policy enforcement** — both fabrics independently evaluate every
  invocation; deny-by-default; no ambient authority even between allied fabrics
- **Trust levels** — `Untrusted → Verified → Trusted → Allied` — progressive
  trust with increasing access; revocable instantly (< 1 second)
- **Cross-fabric invocation** — `invoke("analytics.process", payload, { target:
  "fabric://partner" })` — synchronous, async, or event bridging
- **Selective state sharing** — opt-in replication of specific keys/streams;
  governed by data classification and jurisdiction constraints
- **Cross-fabric observability** — trace context propagated across boundary
  (internal spans redacted); federation audit log immutable and hash-chained

### Isolation Guarantees

Even with federation: no shared runtime, no shared memory, no shared network,
no implicit trust. Only: signed requests, verified execution, audited flows.

---

## What VeerOS Replaces

```
Traditional Cloud Stack              VeerOS Equivalent
─────────────────────────             ──────────────────
Kubernetes / Nomad                    Intent Engine + Intent Scheduler
etcd / Consul / ZooKeeper            Persistent Memory (in-kernel KV)
Redis / Memcached                     State Fabric KV (CRDT-backed)
Kafka / RabbitMQ / SQS               State Fabric Streams (append-only)
Prometheus / Grafana / OTel          Structured Event System (typed, not strings)
ELK / Splunk / Datadog               KernelEvent bus + multi-sink fanout
Istio / Envoy service mesh           ZeroServices (kernel-native routing + mTLS)
Kong / Nginx / Traefik API GW        SYS_SVC_EXPOSE (kernel edge)
REST / gRPC endpoints                invoke("name.function") — identity-routed
cert-manager / Vault TLS             Kernel keystore + auto-rotation
Airflow / Temporal / Celery          Intent decomposition → agent DAGs
Docker / containerd runtime          Kernel service lifecycle + WASM sandbox
PagerDuty / health checks            Kernel heartbeat + auto-replan
Ansible / Terraform                  Persistent Memory + deploy intents
ClearPass / Aruba / ISE (NAC)        Kernel 802.1X + posture engine
CrowdStrike / Trellix (EDR)         Kernel syscall-level EDR
Zscaler / Cloudflare (ZTNA)         Kernel ZTNA — continuous auth
Intune / JAMF / SCCM (MDM)          Fabric enrollment + fleet management
Tailscale / ZeroTier (WAN VPN)       WAN-scale fabric with NAT traversal + QUIC
B2B API gateways (both sides)        InterFabric — federated invocation (IFP)
OAuth2 / OIDC token exchange         Scoped identity tokens (kernel-signed)
VPC peering / shared networks         Zero shared network (invocation only)
```

**Total infrastructure components eliminated:** ~35 → kernel primitives.

---

## Interactive Demo

VeerOS ships with a host-runnable demo that exercises all AI-native subsystems
using real kernel code (not mocks):

```bash
cargo run -p veeros-demo              # interactive shell
./scripts/demo.sh --scenario deploy   # service deployment walkthrough
./scripts/demo.sh --scenario full     # all scenarios
```

Available demo scenarios:
- **deploy** — end-to-end service deployment (fabric → config → intent → agents → outcome)
- **pipeline** — 4-stage data pipeline with auto-decomposition
- **monitor** — monitoring agent swarm across heterogeneous fabric nodes

See [docs/demo-walkthrough.md](demo-walkthrough.md) for full usage guide.

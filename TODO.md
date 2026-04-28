# VeerOS Build Tracker

This file is the persistent progress tracker for VeerOS and should be updated in every development session.

## [2026-04-28] Session Sync — Aura / Graph Fabric / VAS Architecture
- Architecture leap: defined the next-generation VeerOS communication and trust model.
  - **Fold** = secure compute sandbox (current `fold_engine` + `veer-vm`) — name preserved for runtime cell.
  - **Aura** = dynamic, overlapping trust context of belonging (personal / family / team / company / city / country / planet); replaces single-network/zone/realm thinking.
  - **Fabric** = encrypted, multiplexed transport backbone (QUIC-class, identity-routed).
  - **Zone** = physical / administrative locality (where things reside).
  - **Governor** = per-Aura policy + trust + AI stewardship plane.
  - **Vault / Flow** = secure storage / live session primitives.
- New addressing model: **VAS (VeerOS Addressing Standard)** — typed tuple syntax `type{atom,atom,atom}` (e.g. `svc{render,company,live}`, `aur{design,private,open}`, `fld{worker,gpu,warm}`); resolves to compact binary IDs; replaces URI/slash baggage.
  - Layering rule: `fabric://...` remains federation trust-domain identity; VAS tuples remain object/service identity.
- Communication model: **Graph Fabric** — entities as vertices, weighted edges (trust, latency, cost, affinity, membership); requests are *solved* via `argmin` over `αL + βC − γT − δA` rather than statically routed; Auras are subgraphs; Folds are movable compute vertices.
  - Determinism contract: same policy version + same state snapshot => same decision; adaptive telemetry updates may change future decisions, but remain auditable.
- Algebraic / category-theoretic v2 layer (longer-term): typed morphisms, permission set algebra, functorial migration across zones, semiring routing scores, monadic side-effect control for AI agents.
- Updated existing READMEs (`crates/veer_vm/README.md`, `crates/fold_engine/README.md`) to reflect Linux+macOS(HVF)+Windows(stub) host scope and x86_64 / riscv32 / aarch64(planned) guest scope — no longer Linux-only.
- New design docs landed under `docs/`:
  - `docs/veeros-addressing-standard.md` — VAS v1 grammar, resolution, policy integration.
  - `docs/veeros-aura-governor-spec.md` — Aura Governor v1 (membership, policy, trust, audit, AI stewardship).
  - `docs/veeros-graph-fabric-architecture.md` — Graph Fabric v1 (vertices, edges, weights, optimization equation, rollout).
  - `docs/veeros-implementation-roadmap.md` — 5-phase roadmap from control plane → planetary federation.
- Tagline locked: **"Folds compute. Auras belong. Fabric connects."**

## VeerOS Aura / Graph Fabric / VAS — Implementation Track (NEW)
- [ ] **Phase A — Foundation (Control Plane)**
  - [ ] Freeze and publish VAS v1 grammar (`type{atom,...}`) + reserved type prefixes (`usr`, `dev`, `fld`, `aur`, `svc`, `vlt`, `agt`, `zon`, `nod`, `evt`)
  - [x] VAS parser + canonical-form normalizer crate (`crates/vas`)
  - [x] Compact binary ID encoding (64/128-bit) + atom registry
  - [x] `VeerResolve` MVP — local resolver with cache + Aura-scoped lookup
  - [x] Aura object model (identity, members, parent/child links, governor ref)
  - [x] Fold ↔ Aura membership wiring in `fold_engine` (a Fold can declare `auras = [...]`)
  - [x] Minimal Aura Governor — static TOML policy, membership add/remove, audit log
  - [ ] CLI/SDK: `veer aura join/create/share/leave`, `veer fold launch --aura ...`, `veer connect svc{...}`
  - [ ] Legacy compatibility gateway (DNS / IP fallback, classic socket bridge)
- [ ] **Phase B — Dynamic Graph Fabric**
  - [ ] Live graph core (in-process, then distributed) — vertices, edges, weights
  - [ ] Telemetry feed: latency, load, trust score, locality, GPU/CPU availability
  - [ ] Solver engine: `argmin (αL + βC − γT − δA)` for service / Fold selection
  - [ ] Algebraic policy engine — composable permission sets, time-bound capability tokens
  - [ ] Aura Governor v2 — dynamic policy, inheritance, explainability traces
  - [ ] Fabric transport runtime (QUIC-class, multiplexed, encrypted, resumable)
  - [ ] Visual graph explorer / decision-trace debugger (why was node X chosen?)
- [ ] **Phase C — AI-Native & Multi-Aura**
  - [ ] AI assistant integration in Aura Governor (anomaly detection, policy suggestions)
  - [ ] Temporary / scoped Aura membership for agents (`expires`, `scope`)
  - [ ] Multi-Aura participation for Folds, devices, users (overlap graph)
  - [ ] Policy inheritance + delegation + federation across Auras
  - [ ] Advanced permission algebra (set ops, intersections, time bounds, context predicates)
  - [ ] High-level developer SDK for Aura / Fold / Service orchestration
- [ ] **Phase D — Full Orchestration & Mobility**
  - [ ] Fold migration + replication across devices and zones (functorial mapping)
  - [ ] Graph-driven workload placement + continuous re-optimization (with hysteresis)
  - [ ] Aura-aware resource scheduling (GPU slices, storage, bandwidth quotas)
  - [ ] Cross-zone, cross-Aura flows and data movement primitives
  - [ ] Policy-driven, explainable automation (every decision auditable)
  - [ ] Visual workflow composition + audit trails
- [ ] **Phase E — Federation & Planetary Scale**
  - [ ] Federated Aura Governors (city / country / planet tiers)
  - [ ] Global resolver mesh + signed namespace ownership
  - [ ] Public / private Aura discovery, signed invite tokens
  - [ ] Autonomous agent orchestration across federated Auras
  - [ ] Distributed trust scoring + anomaly detection
  - [ ] Open API for third-party Aura / Governor / Service extensions
- [ ] **Always-on / cross-cutting**
  - [ ] Security, privacy, auditability by design (zero-trust default)
  - [ ] Human-centric, intent-based UX (`Join Aura`, `Launch Fold`, never `Configure subnet`)
  - [ ] Legacy compatibility + graceful migration path
  - [ ] Continuous research feedback loop (graph algebra v2, category-theory layer)

## [2026-04-25] Session Sync
- Fold: cgroups v2, seccomp, rootless mode, TOML manifest, state registry, CLI improvements, integration with veer-vm, roadmap updated (see fold_engine/README.md)
- veer-vm: KVM-based microVMM, snapshot/restore, virtio-mmio, direct ISO boot, sensor feed, integration with fold, see veer_vm/README.md
- ESP32C6 dist sensors: Virtual sensor subsystem, shell commands for sensor list/read/set, device files under /dev/sensor/, integration in QEMU and Xiao targets, see kernel/qemu_esp32c6 and kernel/xiao_esp32c6
- veer-connect: Secure shell/file transfer (push/pull), X25519 handshake, VSC protocol, CLI and Python client, see veer-connect crate and scripts/veeros-connect
- Programming model: Shell scripts (build-esp32c6.sh, build-qemu-esp32c6.sh) support dist profiles, WiFi/BLE/802.15.4, auto-flash/monitor, and remote shell via veer-connect


## VeerOS Fold Architecture (Secure Envelope)
- [~] Fold Engine: Native, cross-platform (Linux, MacOS, Windows) secure compute envelope for VeerOS workloads (Phase 8B+)
  - [x] Design Fold as lightweight, secure, rapidly deployable runtime unit (not a container/VM) — host crate `fold_engine` scaffolded with platform `Engine` trait + `PlatformEngine` cfg-selected backend
  - [~] Implement Fold Engine in Rust: process isolation, namespaces, cgroups, seccomp, capabilities, overlayfs, veth, signed manifests, immutable root images
    - [x] Linux namespaces MVP — PID/MOUNT/UTS/IPC/NET via `unshare` + double-fork (init becomes PID 1); `mount /proc`, optional `pivot_root` into rootfs, `sethostname`, `execve`; launcher reports init PID to parent over pipe; stdout/stderr captured to per-fold log
    - [x] Structured launcher→parent error pipe (`O_CLOEXEC` tag-framed: OK=init_pid, ERR=msg); parent surfaces real unshare/mount/pivot_root failure text instead of "launcher closed pipe"
    - [x] User namespace + uid_map/gid_map (rootless spawn) — verified running without sudo; seccomp+namespaces+hostname all functional as unprivileged user on kernels with `kernel.unprivileged_userns_clone=1`
    - [x] cgroups v2 (cpu.max / memory.max / memory.swap.max / pids.max) — auto-detects parent from `/proc/self/cgroup`, creates `fold-<name>/`, attaches init PID after fork, removes on `fold rm`; orphan-safe error path (SIGKILLs init + cleans cgroup dir on failure); helpful `systemd-run --user --scope --property=Delegate=yes` hint on rootless EACCES
    - [x] seccomp default-deny profile — raw BPF via `prctl(PR_SET_SECCOMP)`, `NO_NEW_PRIVS`; default denylist covers keyring, module load, `bpf`, `kexec_*`, `reboot`, `unshare`/`setns`, `mount`/`umount2`/`pivot_root`, swap, time-set, `perf_event_open`, `quotactl`, `ptrace`, `personality`, `acct`; per-fold extra `deny = [...]`; verified: `mount` inside fold returns EPERM
    - [ ] veth pair + optional bridge attachment
    - [ ] overlayfs rootfs layering
    - [ ] Ed25519-signed manifests via VeerOS `crypto` crate
    - [ ] Immutable root images (content-addressed)
  - [~] CLI/API for fold management: spawn, list, logs, scale, move, gpu attach, sleep, refold, archive
    - [x] `fold spawn --manifest <path>` — TOML manifest loader + validation
    - [x] `fold list` — reads `$XDG_STATE_HOME/veeros/fold/*.json`, shows pid/status/started/cmd
    - [x] `fold logs <name> [--follow]` — streams captured stdout/stderr, tail-f polls at 250ms
    - [x] `fold stop <name>` — SIGTERM to init PID
    - [x] `fold rm <name> [--force]` — refuses on live fold unless `--force`
    - [ ] `fold scale`, `fold move`, `fold gpu attach`, `fold sleep`, `fold refold`, `fold archive`
  - [ ] Support for resource governance, network identity, GPU/accelerator access, snapshot/migration, trust policies
  - [ ] Ensure security by default: least privilege, default deny, signed images, verified launch, isolated secrets, minimal syscall surface, audit trails
  - [~] Cross-platform support: Linux (namespaces/cgroups), MacOS (sandbox/launchd), Windows (job objects, containers) — Linux MVP done; macOS + Windows are compile-time `StubEngine` returning `Unsupported`
  - [x] Track progress and update as Fold features are implemented
  - [x] cgroups v2, seccomp, rootless mode, TOML manifest, state registry, CLI improvements, integration with veer-vm, see fold_engine/README.md

## VeerOS microVMM (`veer-vm`) — KVM-based, Firecracker-class
- [~] Lightweight alternative to QEMU for running VeerOS inside a Fold — crate `crates/veer_vm` producing binary `veer-vm`
  - [x] Phase 1 — Boot VeerOS x86_64 kernel via `/dev/kvm`
    - [x] ELF loader (PT_LOAD → `p_paddr`) using `object` crate
    - [x] `mmap`-backed single-region guest memory registered via `KVM_SET_USER_MEMORY_REGION`
    - [x] Firecracker-style direct entry into 32-bit protected mode — flat segments set via `KVM_SET_SREGS`, no in-memory GDT required
    - [x] Multiboot v1 entry state: EAX=0x2BADB002, EBX=info-ptr, EIP=ELF entry, CR0.PE=1, CR0.PG=0
    - [x] Minimal Multiboot v1 info struct (flags=0x1, mem_lower=640 KiB, mem_upper computed)
    - [x] 16550A UART emulation on PIO 0x3F8–0x3FF → host stdout (THR/LSR/LCR/IER/SCR + DLAB latch)
    - [x] `KVM_RUN` loop dispatching `IoIn`/`IoOut`/`MmioRead`/`MmioWrite`/`Hlt`/`Shutdown`
    - [x] Verified: VeerOS boots through GDT/IDT/paging/SYSCALL init, prints full boot banner, enters scheduler
  - [ ] Phase 2 — Timer interrupts + interactive shell
    - [x] In-kernel IRQ chip via `KVM_CREATE_IRQCHIP` — PIC + IOAPIC + LAPIC handled entirely by the host kernel; all `0xFEC0_xxxx`/`0xFEE0_xxxx` MMIO disappears from userspace, `HLT` is handled in-kernel (CPU idles waiting for IRQ instead of exiting to userspace)
    - [x] In-kernel PIT via `KVM_CREATE_PIT2` — real calibrated timer (`timer 2783 ticks/ms` vs. `4294967295` garbage in Phase 1), scheduler runs at least one task after boot
    - [x] UART RX path — raw-mode stdin on a dedicated reader thread pushes bytes into a shared `VecDeque`, 16550 emulation reports `LSR.DR`/`IIR` correctly and asserts IRQ 4 via `KVM_IRQ_LINE` only when `IER.ERBFI` is set
    - [x] QEMU-style `Ctrl-A x` escape sequence to quit the VMM cleanly
    - [x] Signal handling — SIGTERM/SIGHUP flip a global `SHUTDOWN` flag; SIGUSR1 (no-op handler) breaks `KVM_RUN` via EINTR so the run loop re-checks the flag; terminal restored on drop via RAII guard
    - [x] Verify interactive shell end-to-end on a real TTY (typing `ls` etc.)
  - [ ] Phase 3 — I/O via virtio-mmio
    - [ ] virtio-mmio transport + virtio-console, virtio-net
    - [x] virtio-blk-PCI transport (legacy, I/O BAR at 0xC000) — guest driver in `soc/qemu_pc/src/virtio_blk.rs` works unmodified
    - [x] virtio-net-PCI transport (legacy, I/O BAR at 0xC100) with TAP backend — `--tap <ifname>` CLI flag, dedicated RX reader thread, F_MAC negotiation, dup(2)'d TAP fd for rx/tx separation
    - [x] Direct VeerOS ISO boot (skip ELF-only path)
  - [~] Phase 4 — Integration with Fold
    - [x] `fold vm spawn` subcommand — synthesizes a `veer-vm` manifest in-memory (kernel path + memory + name + optional rootless) and hands it to the Linux engine; no external TOML needed; binary auto-discovered via `target/{release,debug}/veer-vm` or `$PATH`
    - [x] Rootless-friendly defaults — cgroup limits are opt-in (`--memory-cap`, `--pids-max`); works out of the box under `--user-ns` without needing a delegated memory controller
    - [x] Verified end-to-end: `fold vm spawn → fold list → fold logs` shows full VeerOS boot sequence (banner through scheduler start) running inside a namespaced fold with default seccomp denylist + user+pid+mount+net+ipc+uts namespaces
    - [x] Default fold manifest `examples/veer-vm.toml` (declarative form for `fold spawn --manifest`)
    - [~] Snapshot / restore (RAM + vCPU/irqchip/PIT/UART + virtio-blk + virtio-net transport/device state done; dirty-log/live migration remain)
    - [x] Sensor feed (UART FIFO) for EdgeFabric/IoT integration
    - [x] Integration with fold (see fold_engine/README.md)
    - [ ] Phase 5 — aarch64 KVM backend (for RPi5 guest kernels)
  - [ ] Phase 6 — ESP32-C6 (RISC-V) VeerOS distribution runnable under `veer-vm`
    - [x] Build-support glue: pick/produce a flat ELF image of the `kernel-qemu-esp32c6` / `kernel-xiao-esp32c6` distribution consumable by the VMM (no ESP-IDF bootloader, no flash layout)
    - [~] Minimal RISC-V guest backend in `veer-vm` (cross-host virtualizer available: QEMU riscv32 on x86_64/AArch64, riscv64-host KVM scaffolded; PLIC/timer interrupt delivery + riscv64-host runtime validation pending)
    - [x] `fold vm spawn --arch riscv32 --kernel <esp32c6.elf>` wiring
    - [ ] Verified boot through scheduler + shell under `veer-vm` on Linux host (no real hardware)

## V1 Scope
- [ ] Bootable microkernel on ESP32 RISC-V (C3/C6/H2) and Xtensa (S3)
- [ ] Multi-architecture support — ARM64 (RPi family, QEMU/KVM), x86-64 (QEMU/KVM), RISC-V 32/64
- [ ] Distribution variants via Rust feature flags — from `dist-minimal` (bare MCU) to `dist-cloud` (full cluster); see Phase 8 for complete catalog
- [ ] Configurable single-user / multi-user system (feature-gated)
- [ ] Security-first architecture — capability-based access, isolation domains, PQC-ready crypto, extensible security model
- [ ] AI-native OS — inference engine, NL shell, autonomous agents, on-device and cloud AI as first-class primitives
- [ ] Distributed OS — multiple VeerOS nodes form a single coherent system (cluster membership, distributed scheduler, shared VFS)
- [ ] Cloud-native platform — built-in orchestration, service mesh, service discovery, rolling deployments, observability
- [ ] Network appliance mode — firewall, packet filtering, NAT, VPN gateway, traffic shaping as a distribution profile (`dist-firewall`)
- [ ] ZeroServices architecture — services as kernel objects, kernel-native routing + mTLS + load balancing, eliminates API gateways entirely (Phase 20)
- [ ] Enterprise security — kernel-native NAC (replaces ClearPass/Aruba), EDR (replaces Trellix/CrowdStrike), ZTNA (replaces Zscaler) — all built into the kernel, not bolted on (Phase 16)
- [ ] Enterprise device management — fabric-native enrollment, compliance engine, fleet OTA, replaces Intune/JAMF/SCCM (Phase 17)
- [ ] WAN-scale fabric — fabric nodes communicate over public Internet with PQC-hybrid encryption, NAT traversal, relay mesh (Phase 18)
- [ ] Unified console abstraction — single management plane spanning ESP32 → RPi → x86-64 clusters, cross-node shell, distributed logs (Phase 19)
- [ ] Structured event system — typed kernel events replace string logs; multi-sink fanout (serial/VGA/network/storage); events as THE observability primitive (Phase 19E)
- [ ] State Fabric — distributed data plane with KV, streams, objects; locality-aware replication, CRDTs, offline tolerance; replaces Redis/Kafka/etcd (Phase 21)
- [ ] Fabric protocol — binary QUIC-native wire protocol; zero-copy forwarding; decentralized peer-coordinated scheduling (Phase 22)
- [ ] Developer SDK — high-level Rust SDK, WASM polyglot support, CLI toolchain; incremental migration path from existing infrastructure (Phase 23)
- [ ] WASM sandbox — portable WebAssembly execution sandbox for lightweight, cross-arch function isolation (Phase 8B)
- [ ] Function invocation model — `invoke("name.function", payload)` replaces URLs/endpoints; ephemeral, versioned, identity-routed (Phase 20A′)
- [ ] InterFabric Protocol (IFP) / VeerLink — cross-fabric federation; independent fabrics communicate via trust-bound, identity-driven invocation without shared networks or API gateways (Phase 24)
- [ ] Fabric Client / VeerUX — next-generation UI replacing the browser; fabric-native composable views, semantic navigation, identity-native auth, adaptive rendering from serial to pixel framebuffer, AI-generated interfaces; eliminates URLs, REST, cookies, JS frameworks (Phase 25)
- [ ] VeerFlow interaction layer — VeerFlow branded Fabric Client; invoke-first, live-by-default, identity-native; command palette + semantic navigation; context stack replaces tabs; canvas + panels; time-travel UI (Phase 25)
- [ ] VDF (Veer Definition Format) — declarative view DSL (YAML-like) that compiles to `invoke()` + `subscribe()` calls; reactive bindings, policy-aware components, composable views, offline-first (Phase 25C)
- [ ] IFP binary wire protocol — QUIC-native, Ed25519-signed, CBOR-encoded invocations; 8 message types (INVOKE/DATA/RESULT/ERROR/CANCEL/HEARTBEAT/POLICY_HINT/FEDERATION); per-invoke identity, streaming, priorities, federation envelope (Phase 22A / 24G)
- [ ] Living Systems / Integrity Engine — continuous behavioral integrity as kernel primitive; self-observation, self-explanation, self-repair, self-learning; integrity signals replace defect management; living graph of system health (Phase 26)

## Logical Phase Order (Primitives → Complex)
_Phases are numbered by historical creation order. Read in this dependency order for logical flow:_

```
TIER 0 — Hardware Foundations
  Phase 1   Foundation (Complete)
  Phase 2   ESP32-C6 Bring-up (Complete)

TIER 1 — Kernel Primitives
  Phase 3   Runtime & Services
  Phase 6   Process Model Overhaul (6A–6J: arch abstraction, process/threads, memory, sync, channels, async, sockets, VFS, users)

TIER 2 — Hardware Targets
  Phase 7   Multi-Architecture (ARM64, RISC-V 64, x86-64, Xtensa, all BSPs, WiFi/BLE/802.15.4)

TIER 3 — Security Foundations
  Phase 8   Security Architecture (8A–8J: capabilities, isolation, crypto, secure boot, TLS/SSH, audit, firewall, hardware security, policy)

TIER 4 — Heterogeneous Compute
  Phase 8K  Unified Accelerator Interface (UAI: GPU/FPGA/QPU/NPU)
  Phase 9   Quantum CoProcessor Support

TIER 5 — Intelligence
  Phase 10  AI as First-Class OS Citizen (inference, NL shell, agents, vision, voice, RAG)
  Phase 14  AI-Native Execution Kernel (agents, intents, memory engine, fabric, scheduler) — Implemented
  Phase 26  Living Systems / Integrity Engine (self-observation, self-explanation, self-repair, self-learning) — NEW

TIER 6 — Distributed Systems
  Phase 11  Distributed OS / Cluster (membership, consensus, distributed scheduler/IPC/VFS)
  Phase 15  Distributed Fabric, Zero Trust & ZKP Security
  Phase 18  WAN-Scale Fabric (NAT traversal, Internet-scale mesh, WAN-aware scheduling) — NEW
  Phase 19  Unified Console Abstraction (cross-node shell, distributed logs, fleet ops, structured events) — NEW
  Phase 21  State Fabric — distributed data plane (KV, streams, objects, CRDTs) — NEW
  Phase 22  Fabric Protocol & Transport — binary QUIC-native wire protocol, decentralized scheduling — NEW

TIER 7 — Platform Services
  Phase 20  ZeroServices Architecture & API Gateway Elimination (+ function invocation model) — NEW
  Phase 12  Cloud Platform (orchestration, observability, auto-scaling, multi-tenancy)

TIER 8 — Network & Security Appliance
  Phase 13  Network Appliance / Firewall OS
  Phase 16  Enterprise Security — NAC + EDR + ZTNA — NEW

TIER 9 — Enterprise
  Phase 17  Enterprise Device Management — NEW
  Phase 23  Developer SDK & Adoption Path — NEW

TIER 10 — Inter-Fabric Federation
  Phase 24  InterFabric Protocol (IFP) / VeerLink — cross-fabric trust, federated invocation, identity federation — NEW

TIER 11 — User Experience
  Phase 25  Fabric Client / VeerFlow — adaptive rendering, composable views, VDF view DSL, semantic navigation, AI-native UI, dist-desktop — NEW

CROSS-CUTTING
  Phase 4   Distribution Profiles (feature flags, per-target defaults, component matrix)
```

## Phase 1 — Foundation (Complete)
- [x] Initialize Rust workspace with modular crates
- [x] Define `arch` abstraction crate
- [x] Define `microkernel` crate with capability + scheduler profiles
- [x] Add ESP32 RISC-V BSP crate skeleton
- [x] Add `distributions` crate for feature-flag distro model
- [x] Add target/linker configuration and memory maps for first board
- [x] Add kernel entry + panic/alloc strategy

## Phase 2 — Bring-up on ESP32 RISC-V (Complete)
- [x] UART0 MMIO driver + `Serial` / `Console` traits
- [x] Boot banner via early console
- [x] Hardware bringup checklist document
- [x] Interrupt controller driver (ESP32 CLIC-like matrix)
- [x] SYSTIMER periodic tick driver
- [x] `InterruptController` + `TickTimer` arch traits
- [x] `TaskContext` arch type (RISC-V register file)
- [x] Scheduler with TCB table + round-robin / priority pick
- [x] Idle task wired into boot path
- [x] IPC module — mailbox-per-task, send/recv/block semantics
- [x] RISC-V trap entry/exit assembly (`global_asm!`) + Rust dispatcher
- [x] Preemptive context switch via timer ISR + scheduler
- [x] ecall exception handler stub for future IPC syscalls

## Phase 3 — Runtime and Services
- [x] Serial RX support (`read_byte` / `has_data`) in arch + ESP32 UART driver
- [x] Console \n → \r\n translation for serial terminals
- [x] Interactive shell crate — readline line editor with history, built-in commands (help, version, sysinfo, uname, echo, clear, logo, exit, history, set, vi/edit, man)
- [x] Host-runnable demo binary (`veeros-demo`) — raw terminal, stdin/stdout Serial
- [x] Shell task wired into ESP32 kernel boot path
- [x] QEMU `virt` BSP — NS16550a UART + CLINT timer drivers
- [x] `kernel-qemu-virt` ELF boots real RISC-V under `qemu-system-riscv32`
- [x] VeerOS shell running interactively on real RISC-V via QEMU
- [x] Per-crate linker scripts via build.rs (ESP32 + QEMU coexist)
- [x] Preemptive multitasking — `_veer_start_first_task` asm, `scheduler.start()`, MPIE/MPP mret boot
- [x] Named tasks in TCB (idle, shell visible in `tasks` command)
- [x] Shell `uptime` command (reads scheduler tick counter via callback)
- [x] Shell `tasks` / `ps` command (live task table via callback)
- [x] Timer ISR preempts shell ↔ idle with real context switches
- [x] Formal syscall ABI — ecall-based, numbered ranges (task/ipc/io/time/mem/debug)
- [x] Kernel syscall dispatcher (`dispatch.rs`) + `SyscallAction` enum
- [x] Userlib crate — `sys`, `task`, `ipc`, `io`, `time` modules with raw ecall wrappers
- [x] `print!` / `println!` macros via `SYS_WRITE_BUF` syscall
- [x] IPC send/recv/poll syscalls wired end-to-end
- [x] Sleep/wake syscalls with `wake_sleepers()` in timer tick
- [x] Userlib sample tasks verified on QEMU (hello, timer, ipc-tx, ipc-rx)
- [ ] Driver isolation model — integrates with 8A capabilities (drivers require `Device` + `MmioRegion` caps)
- [ ] Memory management baseline for embedded targets — foundation for 8B isolation domains
- [ ] Optional app runtime service
- [ ] Optional real-time scheduling service

## Phase 6 — Process Model Overhaul (Multi-Arch Foundation)

### 6A — Architecture Abstraction Layer
_Make `arch` crate truly architecture-neutral so ARM64, RISC-V 64, and x86-64 can coexist._

- [x] **`SavedContext` trait** — replace concrete `TaskContext { gpr: [usize; 32] }` with a trait (`size_of`, `set_sp`, `set_pc`, `set_arg`, `get_ret`, `zero`) implemented per-arch
- [x] **Per-arch context modules** — `arch/src/riscv32.rs` with `Riscv32Context` implementing `SavedContext` (ARM64/RV64/x86-64/Xtensa stubs planned)
- [x] **Arch-specific trap entry/exit** — factored `_veer_trap_entry` + `_veer_start_first_task` assembly into `arch::riscv32` module; kernel trap.rs files now contain only board-specific Rust dispatchers
- [x] **`usize` portability audit** — verified scheduler, IPC, allocator are portable; only known issue was sleep target truncation (fixed by 6B `wakeup_tick: u64` field)
- [x] **`SyscallAbi` trait** — abstract ecall (RISC-V) / svc (ARM) / syscall (x86) instruction + register convention — folded into `SavedContext` trait (`get_syscall_nr`, `get_arg`, `set_ret`, `advance_pc`)
- [x] **Conditional `TaskContext` in `arch`** — `#[cfg(target_arch)]` type alias dispatch in `arch/src/lib.rs`; microkernel uses `SavedContext` trait methods
- [ ] **ARM64 stub** — `arch_arm64` crate: `SavedContext` with 31 GPRs + SP + PC + PSTATE, trap frame for EL1→EL0
- [ ] **RISC-V 64 stub** — `arch_riscv64` crate: same 32 GPRs but `usize = u64`, S-mode mcause→scause
- [x] **x86-64 stub** — `arch_x86_64` crate: `SavedContext` with 16 GPRs + RIP + RFLAGS + kernel_word; x86-64 SysV calling convention (RDI/RSI/RDX/RCX/R8/R9 args, RAX syscall nr/return); wired into `arch::TaskContext` via `#[cfg(target_arch = "x86_64")]`

### 6B — Process + Thread Model
_Introduce proper process/thread separation. Processes own address spaces; threads run within them._

- [x] **`BlockReason` enum** — `None`, `Sleep`, `IpcRecv`, `Join` — stored in TCB alongside `TaskState::Blocked`
- [x] **Replace `gpr[0]` sleep hack** — added `wakeup_tick: u64` field to `Tcb`; `wake_sleepers()` now checks `block_reason == Sleep` + `wakeup_tick`; no more u64→usize truncation
- [x] **`SYS_SPAWN` syscall (0x05)** — creates a new task with parent tracking; returns child task ID; userlib `task::spawn()` wrapper
- [x] **`SYS_JOIN` syscall (0x06)** — blocks caller until target exits; `SYS_EXIT` wakes joiners and delivers exit code; userlib `task::join()` wrapper
- [x] **Parent-child relationship** — `parent: usize` + `join_target: usize` + `exit_code: usize` fields in `Tcb`; exit wakes all joiners
- [x] **`Process` struct** — address space ID (ASID), capability token set (see 8A), resource quotas, child list, exit status, owning `DomainId` (see 8B)
- [x] **`Thread` struct** (replaces current `Tcb`)** — belongs to a `Process`, has own stack + context, share process memory
- [x] **Thread states** — extend `TaskState` → `Ready`, `Running`, `Blocked(BlockReason)`, `Suspended`, `Zombie`
- [x] **`SYS_SPAWN_PROCESS` syscall** — create a new process with a separate address space
- [x] **Per-process resource accounting** — track heap usage, open handles, thread count per process
- [x] **Thread-local storage (TLS)** — `tp` register (RISC-V x4) pointing to per-thread data area

### 6C — Memory Management + Isolation
_Hardware-enforced memory isolation using PMP (RISC-V) / MPU (ARM Cortex-M) / page tables (MMU targets)._

- [x] **`MemPerms` + `TaskMemRegion` + `TaskRegions` types** — `MemPerms` bitflags (R/W/X), per-task region array (`MAX_TASK_REGIONS=4`), `validate_user_ptr()` helper in `arch` crate
- [x] **RISC-V PMP driver** — `arch::riscv32::pmp` module: CSR read/write helpers (`pmpaddr0`–`15`, `pmpcfg0`–`3`), TOR-mode entry programming, `apply_task_regions()` called on every context switch
- [x] **Per-task memory regions** — `regions: TaskRegions` + `region_count` in TCB; `create_task()` auto-grants stack RW region; `grant_region()` API for additional regions
- [x] **Pointer validation** — `check_user_ptr()` in dispatcher; `SYS_WRITE_BUF`, `SYS_PANIC`, `SYS_FUTEX_WAIT` verify user pointers against task regions (legacy mode: allow all if `region_count==0`)
- [x] **Stack guard regions** — 64-byte no-access region below each stack auto-granted at task creation; PMP denies access (enforced in U-mode)
- [x] **`SYS_MEM_REGION_COUNT` / `SYS_MEM_REGION_INFO` syscalls** — userspace can query its own granted memory regions
- [ ] **Kernel/user split** — M-mode kernel + U-mode tasks on RISC-V; EL1/EL0 on ARM64; ring 0/3 on x86-64
- [ ] **RISC-V S-mode support** — for 64-bit targets with MMU (Sv39/Sv48 page tables)
- [ ] **ARM64 page tables** — 4K pages, TTBR0/TTBR1 split, ASID tagging

### 6D — Synchronization Primitives
_Kernel-backed locking and signaling for safe concurrent access._

- [x] **`SYS_FUTEX_WAIT` / `SYS_FUTEX_WAKE` syscalls** — Linux-style futex as the universal building block; `FutexTable` with 32-slot wait queue, address-keyed
- [x] **Userlib `Mutex<T>`** — volatile read/write + futex mutex (no atomics on riscv32imc), `no_std` compatible, RAII `MutexGuard`
- [x] **Userlib `Condvar`** — condition variable on top of futex (sequence counter design)
- [x] **Userlib `Semaphore`** — counting semaphore (bounded concurrency control) via futex
- [x] **Priority inheritance** — in kernel futex: boost holder's priority to max of all waiters
- [ ] **Deadlock detection** — optional: track wait-for graph in kernel, surface via debug syscall
- [x] **`RwLock<T>`** — reader-writer lock (multiple readers xor one writer)
- [ ] **Atomic operations support** — RISC-V A extension (lr/sc, amo*) or fallback kernel-mediated CAS on rv32imc

### 6E — Message Queues + Channels
_Replace single-slot mailbox with proper IPC primitives._

- [x] **Bounded message queue** — ring buffer (depth 8) per channel, pool of 8 channels; `ChanMsg { word0, word1 }`
- [x] **`SYS_CHAN_CREATE` / `SYS_CHAN_CLOSE`** — create/destroy anonymous channels from fixed pool
- [x] **`SYS_CHAN_SEND` / `SYS_CHAN_RECV`** — blocking send (full→ChanSend) and recv (empty→ChanRecv) with PC rewind; close wakes all blocked tasks
- [x] **`SYS_MQ_POLL`** — check if queue has messages without consuming
- [x] **Typed channels** — userlib wrapper: `Channel<T>` for typed, zero-copy (within address space) message passing
- [ ] **Multicast / publish-subscribe** — notification groups for event broadcasting
- [ ] **Keep legacy single-slot IPC** as a fast path for simple request/reply patterns
- [x] **`arg1` fix** — return all 4 message words through `a0`–`a3` in userlib `recv()`

### 6F — Async/Await Runtime
_Cooperative concurrency within a thread — many logical tasks on one stack._

- [x] **Kernel `SYS_POLL_SET` syscall** — register interest in multiple events (IPC, timer, I/O ready)
- [x] **Kernel `SYS_POLL_WAIT` syscall** — block until any registered event fires (like epoll_wait)
- [x] **Userlib executor** — single-threaded `no_std` async executor: `block_on` + noop `Waker` + `SYS_POLL_WAIT` sleep
- [x] **Userlib `AsyncTimer`** — `Future` that resolves after N ticks via `POLL_TIMER`
- [x] **Userlib `AsyncRecv` / `AsyncSend`** — `Future` types for channel recv/send via `POLL_CHAN_READABLE/WRITABLE`
- [x] **Userlib `poll` module** — raw `poll_set()` / `poll_wait()` syscall wrappers + event constants
- [x] **Cooperative yield point** — `Poll::Pending` → `SYS_POLL_WAIT(MAX)` in executor, not busy spin
- [ ] **`async fn` task entry** — allow task entry points to be `async fn() -> !` with executor loop  _(deferred)_
- [ ] **Cancellation** — drop-based cleanup for in-flight async operations  _(deferred)_

### 6G — Sockets (IPC + Network)
_Unified socket API spanning local IPC and network transports._

- [x] **`SYS_SOCKET` / `SYS_BIND` / `SYS_LISTEN` / `SYS_ACCEPT`** — BSD-style socket syscalls
- [x] **`SYS_CONNECT` / `SYS_SEND` / `SYS_RECV` / `SYS_CLOSE`** — data transfer syscalls
- [x] **Local (Unix-domain) sockets** — in-kernel ring buffer between two processes, no network overhead
- [ ] **TCP sockets** — wrap smoltcp TCP in socket handle, expose to userspace
- [ ] **UDP sockets** — wrap smoltcp UDP for datagram services
- [x] **Socket handle table** — per-process file descriptor / handle table (small fixed array initially)

- [ ] **`select` / `poll` / `epoll`-style multiplexing** — ties into async `SYS_POLL_SET`
- [x] **Userlib `TcpStream` / `TcpListener`** — safe Rust wrappers in userlib

### 6H — Documentation / Man Pages
_Built-in documentation accessible from the shell._

- [x] **`man` shell command** — display help for syscalls, commands, and concepts
- [x] **Embedded man page store** — `&[(&str, &str)]` table in `.rodata`, keyed by topic name
- [x] **Syscall man pages** — one entry per syscall (yield, exit, send, recv, sleep, alloc, etc.)
- [x] **Shell command help** — `man help`, `man tasks`, `man wifi`, `man bt`, `man zigbee`
- [x] **Concept pages** — `man scheduler`, `man ipc`, `man memory`, `man boot`
- [ ] **Pager** — basic `--More--` pagination for long man pages on small terminals

### 6I — User Identity + Multi-User Support
_Configurable single-user vs multi-user system. Feature-gated: `single-user` (default on embedded) vs `multi-user`._

#### Core Identity Model
- [x] **`multi-user` / `single-user` feature flags** — `single-user` default for embedded targets (no login, implicit root); `multi-user` enables full identity system
- [x] **`UserId` (UID) type** — `u16` user identifier; UID 0 = root/system, UID 1–65534 = normal users, 65535 = nobody
- [x] **`GroupId` (GID) type** — `u16` group identifier for coarse-grained access grouping
- [x] **`UserEntry` struct** — `uid`, `gid`, `name: &str`, `password_hash: [u8; 32]`, `home_dir`, `shell`, `flags` (enabled/disabled/locked)
- [x] **User table** — static `[UserEntry; MAX_USERS]` (8–16 slots); stored in `.rodata` for single-user, kernel RAM for multi-user
- [x] **Group table** — static `[GroupEntry; MAX_GROUPS]` with membership bitmask per user

#### Authentication
- [x] **`SYS_LOGIN` syscall** — validate username + password, return session token on success
- [x] **`SYS_LOGOUT` syscall** — invalidate session, terminate user's processes (optional)
- [x] **Password hashing** — lightweight hash (SHA-256 or SipHash) for credential verification; no plaintext storage
- [x] **Login shell flow** (`multi-user` only) — boot → `login:` prompt → authenticate → spawn user shell with UID set
- [x] **Auto-login** (`single-user`) — skip authentication, all processes run as UID 0 (root)
- [x] **Session token** — kernel-issued opaque `u32` token tied to UID; passed in process descriptor on spawn
- [x] **Failed login lockout** — optional: 3 failed attempts → 30s cooldown (prevents brute-force on serial/SSH)

#### Per-User Process Ownership
- [x] **UID field in `Process` struct** — every process tagged with owner's UID at spawn time
- [x] **`SYS_GETUID` / `SYS_GETGID` syscalls** — return calling process's UID/GID
- [x] **`SYS_SETUID` syscall** — privilege escalation (root only); allows spawning processes as another user
- [ ] **Process visibility** — `ps`/`tasks` shows owner; non-root users see only their own processes (configurable)
- [ ] **Signal/kill permissions** — `SYS_KILL` restricted: users can only signal their own processes; root can signal any

#### Resource & Capability Permissions
- [ ] **Per-user resource limits** — max processes, max memory, max open handles (enforced at `SYS_SPAWN`/`SYS_ALLOC`)
- [ ] **Capability ownership** — capabilities (MMIO, IRQ, network) granted per-user or per-group; checked at syscall boundary
- [ ] **IPC permissions** — optional: restrict which UIDs can send to privileged service ports
- [ ] **Driver access control** — only root (UID 0) or designated group can register/interact with hardware drivers

#### Shell Integration
- [x] **`whoami` command** — display current user name and UID
- [x] **`su` command** — switch user (requires target user's password or root privilege)
- [x] **`users` command** — list logged-in users and their sessions
- [x] **`useradd` / `userdel` commands** — runtime user management (root only, `multi-user` feature)
- [x] **`passwd` command** — change password for current user (or any user if root)
- [x] **Shell prompt** — include username: `user@veeros $` (multi-user) vs `veeros $` (single-user)
- [x] **`logout` command** — end session and return to login prompt (`multi-user` only)

#### Distribution Integration
_See Phase 4D (User Model Defaults) for the complete profile → user model mapping._

- [ ] **`dist-minimal` / `dist-rt`** — default `single-user` (no auth overhead, bare-metal feel)
- [ ] **`dist-app` / `dist-full`** — default `multi-user` on QEMU/network targets; `single-user` on ESP32 unless explicitly enabled
- [ ] **`kernel-qemu-virt`** — `multi-user` auto-enabled when `net` feature active (remote access requires auth)
- [ ] **`kernel-xiao-esp32c6`** — always `single-user` by default (serial-only, physical access = trusted)

### 6J — Virtual Filesystem (VFS)
_Unified file abstraction for in-memory files, device nodes, and future block storage. Everything is a file._

#### Prerequisites (from existing TODO items)
- [x] **File descriptor table** — per-process `[Option<FileDescriptor>; MAX_FDS]` (16 slots); tracks open files, cursor position, flags; `Process.open_handles` field finally wired up
- [x] **`BlockDevice` trait in `arch`** — `read_block(lba, buf)`, `write_block(lba, buf)`, `block_size()`, `block_count()`; needed for future flash/SD/NVMe drivers
- [x] **`DisplayDevice` trait in `arch`** — `width()`, `height()`, `pitch()`, `bpp()`, `set_pixel()`, `fill_rect()`, `clear()`, `flush()`; framebuffer / SPI LCD abstraction
- [x] **`InputDevice` trait in `arch`** — `poll_event()`, `has_event()`; `InputEvent` enum (KeyPress, KeyRelease, MouseMove, MouseButton, None)
- [x] **GPIO / SPI / I2C traits in `arch`** — `GpioPin` (`set_mode`, `read`, `write`), `SpiBus` (`configure`, `transfer`, `write`), `I2cBus` (`configure`, `write_read`, `write_to`, `read_from`)

#### VFS Core (`crates/microkernel/src/vfs.rs`)
- [x] **`Inode` struct** — `{ id, kind: InodeKind, size, perms: MemPerms, data_offset, data_len, parent, children_head, next_sibling, name: [u8;28] }` — fits in 64 bytes (small block)
- [x] **`InodeKind` enum** — `File`, `Directory`, `Device(major, minor)`, `Symlink`, `Pipe`
- [x] **`InodeTable`** — fixed array `[Inode; MAX_INODES]` (64–128 slots); static allocation, no heap
- [x] **`FileDescriptor` struct** — `{ inode_id, cursor: usize, flags: OpenFlags }` — per-process open file state
- [x] **`OpenFlags`** — `O_RDONLY`, `O_WRONLY`, `O_RDWR`, `O_CREAT`, `O_TRUNC`, `O_APPEND`
- [x] **Path resolution** — walk `/path/to/file` from root inode, follow directory children; `..` and `.` support
- [x] **Mount table** — `MountTable` with `[MountEntry; MAX_MOUNTS]` (4 slots); `FsType` enum (None/RamFs/Fat32); 1-based mount_id stored in inode `dev_major` field; mount/unmount/get operations
- [x] **VFS operations vtable** — dispatch in `dispatch.rs` routes read/write/open/truncate to RamFS or FAT32 based on inode's `dev_major` (mount_id); `SYS_MOUNT` (0xAE) / `SYS_UMOUNT` (0xAF) syscalls

#### RamFS — In-Memory Filesystem (`crates/microkernel/src/ramfs.rs`)
- [x] **Data storage** — dedicated `[u8; RAMFS_SIZE]` pool (8 KB on ESP32, 64 KB on QEMU/RPi); file data stored as contiguous byte ranges
- [x] **Block allocation** — simple bump allocator within the ramfs data pool; compaction deferred
- [x] **Create/read/write/delete** — full CRUD on in-memory files; O(1) read/write via offset+len
- [x] **Directories** — inode with `InodeKind::Directory`; children linked via `children_head`/`next_sibling`
- [x] **Auto-populate root** — boot creates `/`, `/dev`, `/tmp`, `/etc`; optional `/etc/motd`, `/etc/hostname`

#### DevFS — Device Nodes (`/dev/`)
- [x] **`/dev/null`** — reads return EOF, writes discard
- [x] **`/dev/zero`** — reads return 0x00, writes discard
- [x] **`/dev/console`** — reads/writes go to kernel serial console (bridges `SYS_READ_BYTE`/`SYS_WRITE_BUF`)
- [x] **`/dev/random`** — reads return random bytes (hardware RNG when available, PRNG fallback)
- [x] **Device major/minor** — `InodeKind::Device(major, minor)` dispatches to registered device drivers

#### Filesystem Syscalls (0xA0–0xAF)
- [x] **`SYS_OPEN` (0xA0)** — `open(path_ptr, path_len, flags)` → fd
- [x] **`SYS_CLOSE` (0xA1)** — `close(fd)` → 0/err
- [x] **`SYS_READ` (0xA2)** — `read(fd, buf_ptr, count)` → bytes_read
- [x] **`SYS_WRITE` (0xA3)** — `write(fd, buf_ptr, count)` → bytes_written
- [x] **`SYS_SEEK` (0xA4)** — `seek(fd, offset, whence)` → new_position (whence: SET=0, CUR=1, END=2)
- [x] **`SYS_STAT` (0xA5)** — `stat(path_ptr, path_len, stat_buf_ptr)` → 0/err
- [x] **`SYS_FSTAT` (0xA6)** — `fstat(fd, stat_buf_ptr)` → 0/err
- [x] **`SYS_MKDIR` (0xA7)** — `mkdir(path_ptr, path_len)` → 0/err
- [x] **`SYS_UNLINK` (0xA8)** — `unlink(path_ptr, path_len)` → 0/err (files + empty dirs)
- [x] **`SYS_READDIR` (0xA9)** — `readdir(fd, entry_buf_ptr, max_entries)` → count
- [x] **`SYS_TRUNCATE` (0xAA)** — `truncate(fd, new_size)` → 0/err
- [x] **`SYS_RENAME` (0xAB)** — `rename(old_ptr, old_len, new_ptr, new_len)` → 0/err
- [x] **`SYS_GETCWD` (0xAC)** — `getcwd(buf_ptr, buf_len)` → bytes_written
- [x] **`SYS_CHDIR` (0xAD)** — `chdir(path_ptr, path_len)` → 0/err

#### Userlib FS Module (`crates/userlib/src/fs.rs`)
- [x] **`open()` / `close()`** — safe wrappers around `SYS_OPEN`/`SYS_CLOSE`
- [x] **`read()` / `write()`** — safe wrappers with pointer validation
- [ ] **`File` struct** — RAII wrapper holding an fd; auto-closes on drop; implements `Read`/`Write`-like traits
- [x] **`stat()` / `readdir()`** — directory enumeration helpers
- [x] **`mkdir()` / `unlink()` / `rename()`** — filesystem mutation wrappers
- [x] **`getcwd()` / `chdir()`** — working directory management

#### Shell File Commands
- [x] **`ls`** — list directory contents (`ls` = cwd, `ls /path` = specified dir); show name, size, type
- [x] **`cat`** — print file contents to console (`cat /etc/motd`)
- [x] **`mkdir`** — create directory (`mkdir /tmp/test`)
- [x] **`touch`** — create empty file or update timestamp
- [x] **`rm`** — remove file (`rm /tmp/test.txt`)
- [x] **`rmdir`** — remove empty directory
- [x] **`cp`** — copy file (`cp /etc/motd /tmp/motd.bak`)
- [x] **`mv`** — move/rename file or directory
- [x] **`echo >` / `echo >>`** — redirect output to file (create/append)
- [x] **`pwd`** — print working directory
- [x] **`cd`** — change working directory
- [x] **`stat`** — show file/directory metadata (size, type, permissions)
- [x] **`hexdump`** — hex dump of file contents
- [x] **`write`** — write text to a file (`write /tmp/hello.txt Hello World!`)
- [x] **`tree`** — recursive directory listing


- [x] `NetworkDevice` trait in `arch` crate (transport-agnostic NIC abstraction)
- [x] VIRTIO-NET MMIO driver in `bsp-qemu-virt` (probes QEMU virt slots)
- [x] `net` crate — smoltcp TCP/IP stack integration + `DeviceAdapter` PHY bridge
- [x] `TcpSerial` — implements `Serial` trait over a TCP socket (shell-over-TCP)
- [x] Network listener task in `kernel-qemu-virt` (auto-probes NIC, listens on port 2323)
- [x] `MAX_TASKS` bumped to 16 (supports idle + shell + net + future sessions)
- [x] QEMU launch instructions with `-device virtio-net-device` + user-net port forwarding
- [ ] Lightweight SSH-compatible server (or custom encrypted shell protocol) — uses 8C crypto + 8E TLS
- [ ] Authentication model (key-based or password) — integrates with 6I user identity system + 8C key management
- [ ] Remote shell session multiplexing (attach shell task to network socket, per-user sessions)
- [x] QEMU user-net or TAP networking for development/testing
- [ ] ESP32 Wi-Fi driver integration for real-hardware remote access

### ESP32-C6 Wi-Fi — Full Stack (RF → IP → Shell-over-TCP)
_Bring up real Wi-Fi on XIAO ESP32-C6: associate with AP, get an IP via DHCP, and serve a VeerOS shell over TCP so the device is accessible from the network._

> **Current blocker (W1):** `register_chipv7_phy()` hangs inside ppTask context during `wifi_hw_start → phy_enable`. The call chain is `esp_wifi_start() → ppTask → wifi_start_process() → wifi_hw_start() → phy_enable() → register_chipv7_phy() → BLOCKS`. Serial trace ends at `...rggCY12`. **Next step:** call `register_chipv7_phy()` early from the wifi-drv thread (before `esp_wifi_init_internal`), matching esp-wifi's init order. See [docs/wifi-bringup-esp32c6.md](docs/wifi-bringup-esp32c6.md) for full analysis.

#### Prerequisites
- [x] **WiFi driver skeleton** — `crates/soc/esp32/src/wifi.rs`: `WifiManager` state machine, `Esp32Wifi` driver struct, MAC init, clock/modem enable
- [x] **WiFi shell commands** — `wifi scan/list/set/connect/status` wired through `ShellEnv.wifi_cmd`
- [x] **smoltcp TCP/IP stack** — already integrated in `crates/net/` with `DeviceAdapter` bridge
- [x] **`TcpSerial`** — `Serial` trait over TCP socket (shell-over-TCP, proven on QEMU)
- [x] **SYSTIMER + interrupt pipeline** — working preemptive scheduler on ESP32-C6

#### Phase W1 — Espressif Radio Firmware Integration
_The ESP32-C6 WiFi/BLE RF is driven by proprietary Espressif blobs (libphy.a, libcoexist.a, libpp.a, etc.). We must link and initialize them._

- [x] **Obtain esp-wifi blobs** — using `esp-wifi-sys` v0.8.1 crate; provides `libphy.a`, `libnet80211.a`, `libpp.a`, `libcore.a` prebuilt for ESP32-C6
- [x] **Link blobs into kernel** — `build.rs` links blob archives via `esp-wifi-sys` link search paths; all extern symbols resolved
- [x] **Implement blob FFI shim** — `wifi_os_adapter.rs` (~1700 lines): full `wifi_osi_funcs_t` table, recursive mutexes, semaphores, timers, queues, task create/delete, malloc/free, event_post, ISR dispatch, trace markers
- [-] **PHY calibration** — `register_chipv7_phy()` implemented but **blocks** inside ppTask context during `wifi_hw_start → phy_enable`; next step: call PHY cal early from wifi-drv thread before `esp_wifi_init_internal` (see `docs/wifi-bringup-esp32c6.md`)
- [x] **WiFi supplicant init** — `esp_supplicant_init()` succeeds (init Step 6); WPA2/WPA3 supplicant ready
- [x] **Coexistence init** — all coex stubs (`coex_init`, `coex_deinit`, `coex_enable`, etc.) return 0; no-op implementation working
- [x] **Reference: `esp-wifi` crate** — esp-wifi v0.15.1 used extensively as design reference for OSI adapter, init sequence, and interrupt pipeline

#### Phase W2 — WiFi STA Association
_Connect to an access point and complete the WPA handshake._

- [x] **Scan implementation** — `esp_wifi_scan_start()` / `esp_wifi_scan_get_ap_num()` / `esp_wifi_scan_get_ap_records()` wired; scan command triggers blob scan API
- [-] **Station mode connect** — `esp_wifi_set_mode(STA)` → `esp_wifi_set_config()` → `esp_wifi_connect()` implemented; **blocked by PHY calibration issue** (see W1)
- [x] **Event handling** — `esp_event_post()` FFI callback implemented; returns 0 on WIFI_EVENT_STA_START and other events; event dispatch → `WifiManager` state transitions
- [x] **State machine updates** — `WifiManager` state machine complete: Uninitialized → Ready → Scanning → Configured → Connecting → Connected / Disconnected
- [ ] **Auto-reconnect** — on disconnect event, retry connect with backoff (1s, 2s, 4s, max 30s)
- [ ] **`wifi status` shows RSSI** — read RSSI from blob and display signal strength in shell

#### Phase W3 — DHCP + IP Configuration
_Acquire an IP address from the network._

- [x] **DHCP client in smoltcp** — enable smoltcp's `dhcpv4` feature; wire `Dhcpv4Client` into the network stack; working on QEMU PC (x86-64) with both NAT and bridge/TAP networking
- [x] **`NetworkDevice` impl for ESP32 WiFi** — `WifiNetProxy` struct in `main.rs` bridges `Esp32Wifi` TX/RX ring buffers to smoltcp `Device` trait; `RxToken`/`TxToken` wired
- [ ] **IP assignment callback** — on DHCP lease, store IP in `WifiManager.ip`, update smoltcp interface, print `[wifi] got IP: x.x.x.x`
- [ ] **DNS resolver** — minimal DNS stub or smoltcp DNS feature for hostname resolution
- [ ] **Static IP fallback** — `wifi ip set <ip> <mask> <gw>` shell command for manual configuration
- [ ] **`ifconfig` / `ip` shell command** — display interface IP, netmask, gateway, MAC, RSSI

#### Phase W4 — Shell-over-TCP (Remote Access)
_Serve the VeerOS shell on a TCP port so you can `nc <device-ip> 2323` or `ssh` in from any machine on the LAN._

- [ ] **ESP32 net listener task** — new kernel task (`net-srv`) that listens on TCP port 2323 (reuse `net` crate listener pattern from QEMU)
- [ ] **`TcpSerial` on ESP32** — instantiate `TcpSerial` backed by smoltcp TCP socket → ESP32 WiFi NIC
- [ ] **WiFi shell session** — on TCP accept, spawn a shell task attached to `TcpSerial` (same as QEMU net shell)
- [ ] **Network tick integration** — smoltcp `poll()` called from timer ISR or dedicated net task loop (receive/transmit frames)
- [ ] **`wifi connect` triggers full stack** — single shell command: associate → DHCP → start net listener → print IP + port
- [ ] **Boot auto-connect** — if SSID configured, auto-connect at boot and start TCP shell server
- [ ] **Connection status LED** — optional: blink onboard LED to indicate WiFi state (connecting/connected/error)

#### Phase W5 — Security + Hardening
- [ ] **WPA3-SAE support** — ensure supplicant blob supports WPA3 for modern routers
- [ ] **Encrypted shell protocol** — TLS or lightweight encrypted channel over TCP (integrates with Phase 8C crypto)
- [ ] **Authentication** — password or key-based login for TCP shell (integrates with Phase 6I user identity)
- [ ] **Rate limiting** — limit TCP connection attempts to prevent brute-force
- [ ] **Firewall rules** — simple port allow/deny table in kernel (default: only port 2323 open)

### ESP32-C6 Bluetooth LE — Full Stack (HCI → GAP/GATT → HID/Services)
_Bring up real BLE on XIAO ESP32-C6: initialize HCI transport, scan/advertise, connect to peripherals, and support HOGP HID input + custom GATT services._

#### Prerequisites
- [x] **BLE driver skeleton** — `crates/soc/esp32/src/ble.rs`: `BleManager` state machine, `Esp32Ble` driver, modem clock enable, BB reset
- [x] **BLE HID client (HOGP)** — `crates/soc/esp32/src/ble_hid.rs`: `HogpManager` (4 devices), GATT UUID constants, boot keyboard/mouse report parsing
- [x] **Shell commands** — `bt scan/list/advertise/stop/status` wired through `ShellEnv.bt_cmd`
- [x] **Input subsystem** — `crates/microkernel/src/input.rs`: keyboard + mouse queues, `/dev/keyboard` and `/dev/mouse` device nodes
- [x] **Driver task** — BLE driver task runs in M-mode, enables BLE clocks, resets BB, verifies MMIO at `0x600A_C000`

#### Phase B1 — Espressif BLE Firmware / HCI Transport
_ESP32-C6 BLE controller is firmware-driven. Need HCI command/event transport layer._

- [ ] **Obtain BLE blobs** — extract `libbtbb.a`, `libbtdm_app.a` (or `esp-ble` equivalent) from ESP-IDF v5.x; or evaluate `esp-wifi` crate's BLE support
- [ ] **Link BLE blobs** — add archives to `build.rs`; resolve extern FFI symbols
- [ ] **HCI transport layer** — implement shared-memory HCI between RISC-V CPU and BLE controller (command ring → controller, event ring → host)
- [ ] **HCI command serialization** — build HCI command packets: `Reset`, `Read_BD_ADDR`, `LE_Set_Scan_Parameters`, `LE_Set_Scan_Enable`, `LE_Create_Connection`, `LE_Set_Advertising_Data`, `LE_Set_Advertising_Enable`, `Disconnect`
- [ ] **HCI event parsing** — parse event packets: `Command_Complete`, `Command_Status`, `LE_Meta_Event` (advertising reports, connection complete), `Disconnection_Complete`
- [ ] **FFI OS shim** — provide C-callable functions BLE blobs expect: `malloc`/`free`, `vTaskDelay`, timer, mutex, semaphore

#### Phase B2 — GAP (Scanning + Advertising + Connection)
_Generic Access Profile — device discovery and link management._

- [ ] **Real BLE scanning** — `LE_Set_Scan_Parameters` + `LE_Set_Scan_Enable` → receive `LE_Advertising_Report` events → parse AD structures (flags, name, service UUIDs, TX power)
- [ ] **Populate scan results** — replace hardcoded 5 fake devices with real advertising reports; update `BleScanResult` with parsed AD data
- [ ] **BLE advertising** — `LE_Set_Advertising_Data` (device name, flags, service UUIDs) + `LE_Set_Advertising_Enable`; configurable interval
- [ ] **Connection establishment** — `LE_Create_Connection` with target device address → handle `LE_Connection_Complete` event → store connection handle
- [ ] **Disconnect handling** — `Disconnect` command + `Disconnection_Complete` event → clean up state, notify `HogpManager`
- [ ] **`bt scan` shows real devices** — live RF scan with RSSI, device name, address type, connectable flag
- [ ] **`bt connect <addr>` command** — connect to a specific BLE device by address

#### Phase B3 — L2CAP + ATT + GATT Client
_Protocol stack for attribute discovery and data exchange._

- [ ] **L2CAP basic mode** — connection-oriented channel for ATT (CID 0x0004); segment/reassemble L2CAP PDUs
- [ ] **ATT protocol client** — `ATT_READ_BY_GROUP_TYPE_REQ` (service discovery), `ATT_READ_BY_TYPE_REQ` (characteristic discovery), `ATT_FIND_INFORMATION_REQ` (descriptor discovery), `ATT_READ_REQ`, `ATT_WRITE_REQ`, `ATT_HANDLE_VALUE_NTF`
- [ ] **GATT service discovery** — enumerate primary services → characteristics → descriptors; cache in `HidHandles`
- [ ] **Notification subscription** — write `0x0001` to CCCD handle (Client Characteristic Configuration Descriptor) to enable notifications
- [ ] **GATT client cache** — store discovered services/characteristics per-connection for fast re-access

#### Phase B4 — HOGP HID (Keyboard + Mouse over BLE)
_HID Over GATT Profile — connect BLE keyboards and mice._

- [ ] **HOGP service discovery** — find HID Service (UUID 0x1812), Report Map (0x2A4B), Boot Keyboard Input (0x2A22), Boot Mouse Input (0x2A33)
- [ ] **Set Protocol Mode** — write `0x00` (boot protocol) to Protocol Mode characteristic (0x2A4E) for simple 8-byte keyboard / 3-byte mouse reports
- [ ] **Subscribe to input reports** — enable notifications on Boot Keyboard Input and/or Boot Mouse Input CCCDs
- [ ] **Feed HID reports to input subsystem** — on notification, call `process_notification()` → `InputSubsystem.feed_keyboard_report()` / `feed_mouse_report()`; already implemented in `ble_hid.rs`
- [ ] **BLE keyboard → VeerOS shell** — keystrokes from BLE keyboard appear at `root@veeros>` prompt via `/dev/keyboard`
- [ ] **Multi-device support** — connect up to 4 HID devices simultaneously via `HogpManager`
- [ ] **`input` command shows real BLE HID devices** — connected device name, type, battery level

#### Phase B5 — GATT Server (Custom Services)
_Expose VeerOS services over BLE for mobile/PC configuration._

- [ ] **GATT server framework** — register custom services with characteristic array; handle ATT read/write requests from central
- [ ] **Device Information Service (DIS)** — standard service (0x180A): manufacturer, model, firmware version, hardware revision
- [ ] **VeerOS Config Service** — custom GATT service: read/write WiFi SSID+password, read system uptime, read task list
- [ ] **BLE serial console** — Nordic UART Service (NUS) compatible: TX/RX characteristics for shell-over-BLE (alternative to WiFi TCP)
- [ ] **OTA firmware update** — BLE-based firmware transfer service for field updates without USB cable

#### Phase B6 — Security + Pairing
- [ ] **LE Secure Connections** — ECDH key exchange + AES-CCM encryption (BLE 4.2+ Secure Connections)
- [ ] **Pairing modes** — Just Works (no MITM), Passkey Entry, Numeric Comparison
- [ ] **Bond storage** — store LTK/IRK in NVS for reconnection without re-pairing
- [ ] **Privacy** — resolvable private addresses (RPA) to prevent BLE tracking
- [ ] **Coexistence with WiFi** — shared 2.4 GHz antenna arbitration via esp-coex (already stub in `modem.rs`)

### ESP32-C6 IEEE 802.15.4 — Full Stack (MAC → 6LoWPAN → Zigbee/Thread)
_Bring up real 802.15.4 radio on XIAO ESP32-C6: initialize the MAC peripheral, transmit/receive frames, and support Zigbee and Thread networking._

#### Prerequisites
- [x] **802.15.4 driver skeleton** — `crates/soc/esp32/src/ieee802154.rs`: `RadioManager` state machine, `Esp32Ieee802154` driver, register offset map
- [x] **Shell commands** — `zigbee init/scan/channel/panid/send/list/status` wired through `ShellEnv.zigbee_cmd`
- [x] **Driver task** — 802.15.4 driver task runs in M-mode, enables 802.15.4 clocks, resets MAC, verifies MMIO at `0x600A_3000`
- [x] **Register map** — `IEEE802154_BASE` (0x600A_3000) with offsets: CTRL, TX_POWER, ED_SCAN, CHANNEL, TX_FIFO, RX_FIFO, PAN_ID, SHORT_ADDR, EXT_ADDR, INT_ENA, INT_CLR

#### Phase Z1 — MAC Peripheral Init + Register I/O
_Write the hardware registers to bring the 802.15.4 MAC out of reset and into a usable state._

- [ ] **MAC init sequence** — write `REG_CHANNEL` (default ch 15), `REG_PAN_ID` (0xFFFF), `REG_SHORT_ADDR` (0xFFFF), `REG_EXT_ADDR_LO/HI` (from eFuse MAC or random)
- [ ] **TX power configuration** — write `REG_TX_POWER` (default 0 dBm for C6)
- [ ] **Interrupt enable** — write `REG_INT_ENA` for TX-done, RX-done, ED-scan-done; wire IEEE 802.15.4 IRQ source through INTMATRIX → PLIC → driver task via `drv_irq_wait()`
- [ ] **`Esp32Ieee802154::init()` returns `Ok(())`** — replace stub with real register init; set `initialised = true`
- [ ] **`set_channel()` writes hardware** — write channel (11–26) to `REG_CHANNEL`
- [ ] **`set_pan_id()` writes hardware** — write PAN ID to `REG_PAN_ID`
- [ ] **Verify with `zigbee status`** — channel/PAN ID read back from registers match shell display

#### Phase Z2 — TX + RX Frame Path
_Transmit and receive raw 802.15.4 frames._

- [ ] **TX path** — write frame (≤127 bytes, prepend PHR length byte) to `REG_TX_FIFO` → trigger TX via `REG_CTRL` → wait for TX-done interrupt → check status
- [ ] **RX path** — enable RX in `REG_CTRL` → on RX-done interrupt, read frame from `REG_RX_FIFO` → parse PHR + MHR (frame control, sequence number, addressing) → deliver to upper layer
- [ ] **Auto-ACK** — configure hardware auto-acknowledgment for frames with ACK request bit set
- [ ] **Frame filtering** — configure hardware PAN ID / address filtering to reject non-matching frames
- [ ] **CSMA-CA** — use hardware CSMA-CA for contention-based channel access (or implement slotted CSMA in software)
- [ ] **`zigbee send <data>` transmits over air** — build a data frame with addressing and send via TX FIFO
- [ ] **`zigbee recv` command** — display received frames (hex dump + parsed header)

#### Phase Z3 — Energy Detection + Real Scanning
_Scan the 2.4 GHz band for active 802.15.4 networks._

- [ ] **ED scan** — use `REG_ED_SCAN` to measure energy level on channels 11–26; returns RSSI/ED per channel
- [ ] **Active scan** — send beacon request frames on each channel → collect beacon responses → parse PAN descriptor (PAN ID, coordinator address, superframe spec)
- [ ] **Populate real scan results** — replace 4 hardcoded fake networks with actual beacon data
- [ ] **Protocol detection** — distinguish Zigbee vs Thread vs generic 802.15.4 from beacon payload / network layer headers
- [ ] **`zigbee scan` shows real networks** — live RF scan with PAN ID, channel, coordinator, protocol, LQI, permit-join status

#### Phase Z4 — Zigbee Stack (ZigBee 3.0)
_Full Zigbee protocol stack for home automation and IoT sensor networks._

- [ ] **NWK layer** — network formation (coordinator), join (router/end-device), mesh routing (AODV), network-layer encryption (NWK key)
- [ ] **APS layer** — application support: binding table, group management, APS-level encryption (link key)
- [ ] **ZDO (Zigbee Device Object)** — device/service discovery, network management commands, permit joining
- [ ] **ZCL (Zigbee Cluster Library)** — implement key clusters: On/Off (0x0006), Level Control (0x0008), Color Control (0x0300), Temperature Measurement (0x0402), Occupancy Sensing (0x0406)
- [ ] **Zigbee coordinator mode** — form a PAN, assign short addresses, manage routing table
- [ ] **Zigbee end-device mode** — join existing PAN, periodic polling for sleepy end devices
- [ ] **Zigbee2MQTT compatibility** — standard ZCL reporting so off-the-shelf coordinators (CC2531, SONOFF) can discover VeerOS Zigbee devices
- [ ] **`zigbee join <panid>` command** — join an existing Zigbee network
- [ ] **`zigbee form` command** — create a new Zigbee PAN as coordinator

#### Phase Z5 — Thread / OpenThread (Thread 1.3)
_Thread networking for IP-based IoT mesh — native IPv6 over 802.15.4._

- [ ] **6LoWPAN** — IPv6 header compression (RFC 6282) for IEEE 802.15.4 frames; fragmentation/reassembly
- [ ] **MLE (Mesh Link Establishment)** — discover routers, attach to network, negotiate link parameters
- [ ] **Thread network roles** — Leader, Router, REED (Router-Eligible End Device), SED (Sleepy End Device)
- [ ] **Thread Commissioner / Joiner** — secure device commissioning (DTLS handshake + PSKc)
- [ ] **CoAP** — Constrained Application Protocol for Thread service discovery and management
- [ ] **SRP (Service Registration Protocol)** — register services on Thread Border Router
- [ ] **DNS-SD over Thread** — mDNS-like service discovery for Thread devices
- [ ] **OpenThread port** — evaluate porting OpenThread (C library) as an alternative to from-scratch implementation; provide platform abstraction layer
- [ ] **Border Router stub** — if WiFi is also active, relay Thread traffic to WiFi/IP network (Thread Border Router function)
- [ ] **`thread attach` command** — join an existing Thread network
- [ ] **`thread dataset` command** — view/set Thread network dataset (PAN ID, channel, network key, mesh-local prefix)

#### Phase Z6 — Matter (Project CHIP)
_Matter application layer on top of Thread (or WiFi) for smart home interoperability._

- [ ] **Matter device types** — On/Off Light, Dimmable Light, Temperature Sensor, Door Lock, etc.
- [ ] **Matter commissioning** — BLE-based commissioning flow (QR code / manual pairing code → Thread/WiFi onboarding)
- [ ] **Matter clusters** — implement Matter application clusters mapped to ZCL equivalents
- [ ] **Interop with Apple Home / Google Home / Alexa** — standard Matter certification path

#### Phase Z7 — Security + Coexistence
- [ ] **802.15.4 MAC security** — AES-128-CCM frame encryption/authentication (security level 5)
- [ ] **Zigbee network key management** — Trust Center key distribution, transport key, network key rotation
- [ ] **Thread security** — DTLS for commissioning, MLE frame encryption, network key rotation
- [ ] **RF coexistence** — 802.15.4 shares 2.4 GHz with WiFi and BLE; coordinate via esp-coex or time-division scheduling
- [ ] **Channel selection** — auto-select least-interfered 802.15.4 channel based on WiFi channel and ED scan

## Phase 4 — Distribution Profiles (Unified Reference)
_All VeerOS distribution profiles — from bare-metal MCU to cloud platform. Profiles are Rust feature flags in `distributions/src/lib.rs`. Two orthogonal axes: **profile** (scheduler + capabilities) × **components** (shell, net, AI, cluster, firewall, etc.). Profiles are additive — any combination of feature flags is valid._

### 4A — Foundation Profiles (Complete)
_Implemented and shipping. Feature flags live in `distributions/src/lib.rs`._

- [x] Distribution matrix design — two axes: profile (minimal/app/rt/full) × components (shell/net/userlib/samples/wifi/ble/ieee802154)
- [x] `distributions` crate restructured — aligned feature names (`dist-minimal`/`dist-app`/`dist-rt`/`dist-full`), component flags, documentation
- [x] `kernel-qemu-virt` — optional deps: shell, net, userlib, smoltcp; profiles auto-bundle components; default = `dist-app`
- [x] `kernel-xiao-esp32c6` — optional deps: shell; radio features: wifi, ble, ieee802154; default = `dist-minimal` + shell + all radios
- [x] `#[cfg(feature)]` gates across both kernel binaries — conditional compilation of net_task, shell_task, sample tasks, driver registrations, radio managers
- [x] `minimal` distribution build recipe — `--no-default-features --features dist-minimal` (bare scheduler + idle task only)
- [x] `app` distribution build recipe — `--features dist-app` (shell + net + userlib + samples)
- [x] `real-time` distribution build recipe — `--features dist-rt` (priority scheduler, combine with component flags)
- [x] `full` distribution build recipe — `--features dist-full` (all components + priority scheduler)

### 4B — Complete Distribution Catalog

#### Profile Hierarchy
```
dist-minimal                    (bare scheduler, IPC, VM, driver isolation)
├── dist-app                    (+ shell, net, userlib, samples)
│   ├── dist-ai                 (+ full AI stack, NL shell, NPU backends)
│   ├── dist-cluster            (+ cluster membership, distributed sched/IPC/VFS)
│   │   └── dist-cloud          (+ orchestration, service mesh, API gateway, observability)
│   ├── dist-desktop            (+ Fabric Client, composable views, pixel rendering, window compositor)
│   └── dist-full               (all Tier 1 components + priority scheduler)
├── dist-rt                     (+ priority real-time scheduler)
│   └── dist-xrt                (+ accelerator/GPU/FPGA/QPU via UAI)
├── dist-edge                   (+ edge AI inference, WiFi/BLE, sensor pipeline)
│   └── dist-gateway            (+ Thread border router, Zigbee, MQTT broker)
└── dist-firewall               (+ packet filter, NAT, VPN, DPI, traffic shaping)
```

#### Tier 1 — Foundation (Implemented)

| Profile | Base | Scheduler | Capabilities |
|---------|------|-----------|--------------|
| `dist-minimal` | — | round-robin | IPC, VIRTUAL_MEMORY, DRIVER_ISOLATION |
| `dist-app` | dist-minimal | application | + NETWORK_STACK, APPLICATION_RUNTIME, shell, net, userlib, samples |
| `dist-rt` | dist-minimal | priority | + REAL_TIME_SCHEDULER |
| `dist-xrt` | dist-rt | priority | + ACCEL (GPU/FPGA/QPU via UAI); feature-gated `accel` |
| `dist-full` | dist-app | priority | all Tier 1 components |

#### Tier 2 — Specialized (Planned)

| Profile | Base | Key Additions | Target Hardware |
|---------|------|---------------|-----------------|
| `dist-edge` | dist-minimal | AI inference (keyword/anomaly), WiFi/BLE, sensor pipeline | ESP32 family, RPi Zero |
| `dist-ai` | dist-app | full AI stack (inference engine, NL shell, model zoo, NPU backends) | RPi 4/5, x86-64 ≥ 2 GB RAM |
| `dist-cluster` | dist-app | cluster membership, distributed scheduler, distributed IPC, shared VFS | RPi 3+, x86-64, ARM64 |
| `dist-cloud` | dist-cluster | orchestration, service mesh, API gateway, ingress, observability, auto-scaling | x86-64 KVM, ARM64 KVM |
| `dist-firewall` | dist-minimal | packet filter, NAT, VPN, DPI, traffic shaping, firewall rules engine | x86-64, ARM64, RPi 4/5 |
| `dist-gateway` | dist-edge | Thread border router, Zigbee coordinator, MQTT broker, protocol translation | RPi 3+, ESP32-S3 |
| `dist-desktop` | dist-app | Fabric Client, composable views, pixel rendering, window compositor, AI-native UI | RPi 4/5 (HDMI), x86-64 (VGA/HDMI) |

### 4C — Component Composition Matrix
_Which components ship with each profile. ✓ = included, opt = available on capable hardware, — = not included._

```
                 dist-   dist-  dist-  dist-  dist-  dist-  dist-    dist-     dist-    dist-      dist-     dist-
                 minimal app    rt     xrt    full   edge   ai       cluster   cloud    firewall   gateway   desktop
──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
Shell              —      ✓      —      —      ✓      —      ✓        ✓         ✓        ✓          —         ✓
Net (TCP/IP)       —      ✓      —      —      ✓      opt    ✓        ✓         ✓        ✓          ✓         ✓
Userlib            —      ✓      —      —      ✓      —      ✓        ✓         ✓        —          —         ✓
Samples            —      ✓      —      —      ✓      —      ✓        —         —        —          —         ✓
WiFi               —      opt    —      —      opt    ✓      opt      opt       —        —          ✓         opt
BLE                —      opt    —      —      opt    ✓      opt      —         —        —          ✓         opt
IEEE 802.15.4      —      opt    —      —      opt    opt    —        —         —        —          ✓         —
RT scheduler       —      —      ✓      ✓      ✓      —      —        —         —        —          —         —
Accelerator/UAI    —      —      —      ✓      —      —      opt      —         —        —          —         opt
AI inference       —      —      —      —      —      ✓      ✓        —         opt      —          —         opt
NL shell           —      —      —      —      —      —      ✓        —         opt      —          —         ✓
NPU/GPU offload    —      —      —      —      —      —      ✓        —         opt      —          —         opt
Cluster membership —      —      —      —      —      —      —        ✓         ✓        —          —         —
Distributed sched  —      —      —      —      —      —      —        ✓         ✓        —          —         —
Distributed IPC    —      —      —      —      —      —      —        ✓         ✓        —          —         —
Shared VFS         —      —      —      —      —      —      —        ✓         ✓        —          —         —
Orchestration      —      —      —      —      —      —      —        —         ✓        —          —         —
Service mesh       —      —      —      —      —      —      —        —         ✓        —          —         —
Packet filter      —      —      —      —      —      —      —        —         ✓        ✓          ✓         —
NAT                —      —      —      —      —      —      —        —         —        ✓          ✓         —
VPN (WireGuard)    —      —      —      —      —      —      —        —         —        ✓          —         —
DPI / IDS          —      —      —      —      —      —      —        —         —        ✓          —         —
Traffic shaping    —      —      —      —      —      —      —        —         —        ✓          —         —
Thread / Zigbee    —      —      —      —      —      —      —        —         —        —          ✓         —
MQTT broker        —      —      —      —      —      —      —        —         —        —          ✓         —
FC Text Views      —      —      —      —      —      —      —        —         —        —          —         ✓
FC Pixel Render    —      —      —      —      —      —      —        —         —        —          —         ✓
Window Compositor  —      —      —      —      —      —      —        —         —        —          —         ✓
CVP Runtime        —      —      —      —      —      —      —        —         —        —          —         ✓
Semantic Nav       —      —      —      —      —      —      —        —         —        —          —         ✓
──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
```

### 4D — Cross-Domain Mappings
_How distribution profiles map to security tiers, AI features, network features, and user model defaults._

#### Security Tier Defaults
```
  dist-minimal    → sec-base
  dist-app        → sec-base + sec-crypto
  dist-rt         → sec-base
  dist-xrt        → sec-base + sec-hw
  dist-full       → sec-base + sec-sandbox + sec-crypto + sec-crypto-pqc + sec-hw + sec-verified
  dist-edge       → sec-base + sec-crypto + sec-hw
  dist-ai         → sec-base + sec-sandbox + sec-crypto + sec-hw
  dist-cluster    → sec-base + sec-sandbox + sec-crypto + sec-crypto-pqc + sec-verified
  dist-cloud      → sec-base + sec-sandbox + sec-crypto + sec-crypto-pqc + sec-network + sec-hw + sec-verified
  dist-firewall   → sec-base + sec-sandbox + sec-crypto + sec-network + sec-hw
  dist-gateway    → sec-base + sec-sandbox + sec-crypto + sec-network + sec-hw
  dist-desktop    → sec-base + sec-sandbox + sec-crypto + sec-hw + sec-verified
```

#### AI Tier Defaults
```
  dist-minimal    → (none)
  dist-app        → (none, opt-in via ai feature)
  dist-rt         → (none)
  dist-xrt        → (none, opt-in via ai feature)
  dist-full       → (none, opt-in via ai feature)
  dist-edge       → ai (core: keyword spotter, anomaly detector)
  dist-ai         → ai + ai-npu + ai-cloud + ai-os (full AI stack)
  dist-cluster    → (none, opt-in via ai feature)
  dist-cloud      → ai-cloud (optional: cloud inference gateway)
  dist-firewall   → (none, opt-in: ai-assisted DPI)
  dist-gateway    → ai (core: edge anomaly detection)
  dist-desktop    → ai + ai-npu (optional: NL shell, AI-generated views)
```

#### Network Feature Defaults
```
  dist-minimal    → (none)
  dist-app        → net (TCP/IP via smoltcp)
  dist-rt         → (none)
  dist-xrt        → (none)
  dist-full       → net
  dist-edge       → net (optional)
  dist-ai         → net
  dist-cluster    → net + cluster-ipc
  dist-cloud      → net + net-firewall + cluster-ipc
  dist-firewall   → net + net-firewall + net-nat + net-vpn + net-dpi + net-shape
  dist-gateway    → net + net-firewall + net-nat
  dist-desktop    → net + net-fabric (State Fabric subscriptions, invoke() routing)
```

#### User Model Defaults
```
  dist-minimal    → single-user (no auth overhead, bare-metal feel)
  dist-app        → multi-user on QEMU/network targets; single-user on ESP32
  dist-rt         → single-user
  dist-xrt        → single-user
  dist-full       → multi-user
  dist-edge       → single-user
  dist-ai         → multi-user
  dist-cluster    → multi-user (remote access requires auth)
  dist-cloud      → multi-user + RBAC + tenant isolation
  dist-firewall   → single-user (serial admin) or multi-user (SSH admin)
  dist-gateway    → single-user
  dist-desktop    → multi-user (identity-native auth, per-user view sandboxes)
```

### 4E — Per-Target Platform Defaults
_Recommended default distribution for each hardware target._

```
  ESP32-C3 (400 KB)     → dist-minimal (serial-only MCU)
  ESP32-C6 (512 KB)     → dist-minimal + shell + radios  (current default)
  ESP32-S3 (512 KB)     → dist-edge (WiFi + BLE + edge AI)
  RPi Zero 2 W (512 MB) → dist-edge (IoT hub, sensor gateway)
  RP2350 (520 KB SRAM)  → dist-minimal (microcontroller class)
  RPi 3 (1 GB)          → dist-app or dist-cluster
  RPi 4 (4/8 GB)        → dist-ai or dist-cluster (Coral TPU: dist-ai)
  RPi 5 (8 GB)          → dist-ai (Hailo-8, full AI-OS) or dist-desktop (full UX) or dist-full
  QEMU virt (RISC-V)    → dist-app (development/testing)
  QEMU PC (x86-64)      → dist-full or dist-firewall
  x86-64 KVM            → dist-cloud (production cluster/VM)
  x86-64 bare-metal     → dist-firewall (network appliance) or dist-desktop (workstation)
  ARM64 KVM             → dist-cloud (production cluster/VM)
  ARM64 bare-metal      → dist-cluster or dist-ai
```

### 4F — Hardware Security Capabilities (Per-SoC)
_Available `sec-hw` features on each target platform._

```
  ESP32-C3/C6     → AES/SHA/RSA accel, eFuse, flash encrypt, Secure Boot V2, DS peripheral, HMAC
  ESP32-S3        → same as C6 + World Controller (TrustZone-like)
  RPi 3/Zero      → (limited) OTP, VideoCore secure boot
  RPi 4           → GIC, SMMU (limited), OTP
  RPi 5           → GIC, PAC (Cortex-A76), OTP, SMMU (via RP1)
  RP2350          → ARM MPU (8 regions), TrustZone-M, OTP
  RISC-V 64       → PMP/ePMP, Sv39/48 MMU, Zkr/Zkn (if supported)
  x86-64          → TPM 2.0, AES-NI, SHA-NI, RDRAND, SMEP/SMAP, NX, SGX (optional), SEV (AMD)
```

### 4G — AI Hardware Capabilities (Per-Target)
_Available AI inference features on each target platform._

```
  ESP32-C6 (320 KB)      → ai: keyword spotter, anomaly detector (INT8, < 50 KB models)
  RPi Zero 2 W (512 MB)  → ai: image classification, small models
  RPi 3 (1 GB)           → ai + ai-cloud: local small models + cloud LLM
  RPi 4 (4/8 GB)         → ai + ai-npu + ai-cloud: TinyLlama local, Coral TPU
  RPi 5 (8 GB)           → ai + ai-npu + ai-cloud + ai-os: Phi-3-mini local, Hailo-8, full AI-OS
  QEMU virt              → ai + ai-cloud: development/testing
  x86-64 (≥ 2 GB)       → ai + ai-npu + ai-cloud + ai-os: full stack, CUDA/ROCm/oneAPI backends
```

### 4H — Build Recipes
_Cargo commands for building each distribution profile._

```sh
# Tier 1 — Foundation (implemented)
cargo build -p kernel-qemu-virt --no-default-features --features dist-minimal   # bare scheduler
cargo build -p kernel-qemu-virt --features dist-app                             # shell + net + userlib
cargo build -p kernel-qemu-virt --features dist-rt                              # priority scheduler
cargo build -p kernel-qemu-virt --features dist-rt,accel                        # RT + accelerators (dist-xrt)
cargo build -p kernel-qemu-virt --features dist-full                            # everything
cargo build -p kernel-xiao-esp32c6 --features dist-full,wifi,ble,ieee802154     # full ESP32 with radios

# Tier 2 — Specialized (planned)
cargo build -p kernel-qemu-virt --features dist-edge                            # IoT edge node
cargo build -p kernel-qemu-pc   --features dist-ai                              # AI-native workstation
cargo build -p kernel-qemu-pc   --features dist-cluster                         # distributed OS node
cargo build -p kernel-qemu-pc   --features dist-cloud                           # cloud platform
cargo build -p kernel-qemu-pc   --features dist-firewall                        # network appliance
cargo build -p kernel-qemu-pc   --features dist-desktop                         # Fabric Client workstation
cargo build -p kernel-raspi5    --features dist-desktop                         # RPi5 UX station
cargo build -p kernel-xiao-esp32c6 --features dist-gateway,wifi,ieee802154      # IoT gateway

# Mix-and-match (any combination valid)
cargo build -p kernel-qemu-pc   --features dist-minimal,shell                   # minimal + shell only
cargo build -p kernel-qemu-pc   --features dist-firewall,ai                     # firewall + AI-assisted DPI
cargo build -p kernel-qemu-pc   --features dist-desktop,ai                      # desktop + AI-generated views
cargo build -p kernel-qemu-pc   --features dist-cluster,accel                   # cluster + accelerator
```

### 4I — Distribution Implementation Tasks (Planned)
#### ESP32C6 dist sensors
- [x] Virtual sensor subsystem in QEMU and Xiao ESP32C6 targets
- [x] Shell commands: `sensor list`, `sensor read <name>`, `sensor set <name> <value>`
- [x] Device files under `/dev/sensor/` for host/EdgeFabric injection
- [x] Default sensors: temperature, humidity, pressure, light

#### Crate + Feature Gate Updates
- [ ] **`distributions/src/lib.rs` expansion** — add `Distribution::Edge`, `Ai`, `Cluster`, `Cloud`, `Firewall`, `Gateway`, `ExtendedRealTime`, `Desktop` variants; update `active_distribution()` priority chain
- [ ] **Feature composition rules** — profiles are additive: `dist-cloud` = `dist-cluster` + `cloud-orchestrate` + `cloud-mesh` + `cloud-observe`; `dist-desktop` = `dist-app` + `fc-views` + `fc-render` + `fc-compositor` + `fc-nav`; validate at compile time
- [ ] **Cross-feature dependency validation** — `dist-cloud` requires `dist-cluster`; `dist-gateway` requires `dist-edge`; `dist-xrt` requires `dist-rt` + `accel`; `dist-desktop` requires `dist-app` + framebuffer driver; compile-time errors on invalid combos
- [ ] **Build flag matrix** — `distributions/src/lib.rs` updated with new feature gates; CI matrix covers at least dist-minimal, dist-app, dist-full, dist-firewall, dist-cloud per target

#### Per-Kernel Wiring
- [ ] **kernel-qemu-virt** — wire new profiles: `dist-edge` (AI + sensor tasks), `dist-cluster` (cluster membership task)
- [ ] **kernel-qemu-pc** — wire all profiles: primary target for dist-ai, dist-cloud, dist-firewall, dist-cluster, dist-desktop
- [ ] **kernel-xiao-esp32c6** — wire `dist-edge` (default for ESP32 AI nodes), `dist-gateway` (Thread/Zigbee + MQTT)
- [ ] **kernel-raspi5** — wire `dist-ai` (NPU + Hailo), `dist-cluster` (distributed node), `dist-desktop` (HDMI UX), `dist-full`

#### Profile-Specific Components (cross-references)
- [ ] **dist-edge components** — see Phase 10 (AI inference), Phase Z5–Z6 (802.15.4 + Thread + Matter)
- [ ] **dist-ai components** — see Phase 10 (full AI stack: inference, NL shell, agents, RAG, voice)
- [ ] **dist-cluster components** — see Phase 11 (cluster membership, distributed scheduler, distributed IPC, shared VFS)
- [ ] **dist-cloud components** — see Phase 12 (orchestration, service mesh, API gateway, ingress, observability, auto-scaling, multi-tenancy)
- [ ] **dist-firewall components** — see Phase 13 (packet filter, NAT, routing, VPN, DPI, traffic shaping)
- [ ] **dist-gateway components** — see Phase Z5–Z6 (Thread border router, Zigbee coordinator) + MQTT broker
- [ ] **dist-xrt components** — see Phase 8K (UAI: accelerator registry, submit/poll/cancel, FPGA bitstream, QPU circuits)
- [ ] **dist-desktop components** — see Phase 25 (Fabric Client: composable views, semantic nav, pixel render, window compositor, CVP runtime, AI-native UI, multi-device continuity)

## Phase 7 — Multi-Architecture Targets

### 7C — SD Card + FAT32 Storage (Complete)
_Block device drivers and FAT32 filesystem for persistent storage across RPi5 and ESP32-C6._

- [x] **RPi5 EMMC2 SDHCI driver** (`crates/soc/raspi5/src/sd.rs`) — BCM2712 EMMC2 at 0x10_7D00_4000; full SDHCI register interface; SDv2/SDHC init sequence (CMD0→CMD8→ACMD41→CMD2→CMD3→CMD7→CMD16); CMD17/CMD24 single-block read/write; implements `BlockDevice` trait
- [x] **ESP32-C6 SPI-mode SD driver** (`crates/soc/esp32/src/sdspi.rs`) — GPSPI2 at 0x6000_3000; SPI-mode SD protocol (CMD0→CMD8→ACMD41→CMD58→CMD16); GPIO chip-select; CMD17/CMD24 block I/O; implements `BlockDevice` trait
- [x] **SD modules wired into SoC crates** — `pub mod sd` in raspi5/lib.rs, `pub mod sdspi` in esp32/lib.rs, `SPI2_BASE` in esp32/mem.rs
- [x] **FAT32 filesystem driver** (`crates/microkernel/src/fat32.rs`) — device-agnostic (~580 lines); BPB parsing, FAT chain walking, 8.3 filenames, cluster allocation/free, read/write/truncate/create; uses `BlockReadFn`/`BlockWriteFn` function pointers for block I/O
- [x] **Mount table + VFS integration** — `FsType` enum, `MountEntry` struct, `MountTable` in vfs.rs; 1-based mount_id system; dispatch.rs routes file I/O to RamFS or FAT32 based on inode `dev_major`
- [x] **`SYS_MOUNT` (0xAE) / `SYS_UMOUNT` (0xAF) syscalls** — mount FAT32 volumes at VFS paths; unmount with FAT flush
- [x] **Shell storage commands** — `mount` (list/mount), `umount`/`unmount`, `lsblk`; ShellEnv callbacks: `mount_list`, `mount_fs`, `umount_fs`, `lsblk`
- [x] **Kernel wiring** — `Fat32Cell`/`FAT32` + `MountCell`/`MOUNTS` statics in all 3 kernels; trap.rs passes to dispatch(); ShellEnv callbacks wired; all 4 build targets clean (0 errors)

### 7D — USB / Bluetooth HID Input (Complete)
_USB keyboard/mouse via xHCI (RPi5) and Bluetooth keyboard/mouse via BLE HOGP (ESP32-C6). Unified input subsystem with `/dev/keyboard` and `/dev/mouse` device nodes._

- [x] **HID core** (`crates/microkernel/src/hid.rs`) — USB HID scancode→ASCII tables (US QWERTY, shifted/unshifted); modifier bitmask constants (Ctrl/Shift/Alt/GUI); `hid_key_to_event()` converter; `InputQueue` 64-slot ring buffer; `KeyboardState` (6-key rollover, press/release from 8-byte boot reports); `MouseState` (button tracking, move/button events from 3-byte boot reports)
- [x] **USB Host Controller trait** (`crates/arch/src/lib.rs`) — `UsbDirection`, `UsbSpeed`, `UsbEpType` enums; `UsbDeviceInfo` struct; `UsbHostController` trait (init, reset, port_count, port_connected, port_reset, control_transfer, interrupt_in, device_info)
- [x] **RPi5 xHCI driver** (`crates/soc/raspi5/src/xhci.rs`) — xHCI register definitions (capability/operational/PORTSC); `Xhci` struct; full init sequence (halt→reset→MaxSlotsEn→port scan); `UsbHostController` impl; speed detection; USB HID class constants + SET_PROTOCOL/GET_REPORT SETUP helpers; xHCI0 (USB 3.0) + xHCI1 (USB 2.0) via RP1 southbridge
- [x] **ESP32-C6 BLE HOGP client** (`crates/soc/esp32/src/ble_hid.rs`) — GATT HID Service UUIDs; `BleHidType`/`HogpState` enums; `HidHandles` GATT characteristic cache; `BleHidDevice` per-device state; `HogpManager` (4 devices, scan/connect/disconnect/subscribe); boot keyboard/mouse report parsing
- [x] **Input subsystem** (`crates/microkernel/src/input.rs`) — `InputSubsystem` struct; keyboard + mouse queues; `feed_keyboard_report()` / `feed_mouse_report()` / `feed_event()`; `kbd_read()` (ASCII) + `mouse_read()` (4-byte encoded packets); status display
- [x] **Device dispatch** — `/dev/keyboard` (major 1, minor 0) + `/dev/mouse` (major 1, minor 1); `dev_read()`/`dev_write()` extended in dispatch.rs; VFS device node creation in all 3 kernels
- [x] **Shell commands** — `input` (status + BLE HID + USB lists), `lsusb` (USB device listing); ShellEnv callbacks: `input_status`, `usb_list`, `ble_hid_list`
- [x] **Kernel wiring** — `InputCell`/`INPUT` statics in all 3 kernels; trap.rs passes `input` to dispatch(); ShellEnv callbacks wired (QEMU: input_status, RPi5: input_status + usb_list, ESP32: input_status + ble_hid_list); all 4 build targets clean (0 errors)

### ARM64 (QEMU virt + KVM)
_Full AArch64 bring-up on QEMU `virt` machine and Linux KVM. QEMU-TCG for development, KVM for near-native performance on ARM64 hosts (RPi 5, Apple Silicon, Ampere, Graviton)._

#### ARM64 Architecture Crate
- [ ] **`arch_aarch64` crate** — `SavedContext` for AArch64: 31 GPRs (X0–X30) + SP_EL0 + ELR_EL1 + SPSR_EL1 + TPIDR_EL0 (TLS); `SavedContext` trait impl with `set_pc`/`get_pc`/`advance_pc`, `set_sp`/`get_sp`, `set_arg`/`get_arg` (X0–X7), `set_ret`/`get_ret` (X0), `get_syscall_nr` (X8)
- [ ] **Exception vector table** — `VBAR_EL1` aligned vector table (4 × 4 entries): Sync/IRQ/FIQ/SError × {currentEL_SP0, currentEL_SPx, lowerEL_AArch64, lowerEL_AArch32}; full 31-GPR + SP + PSTATE + ELR save/restore in assembly
- [ ] **Syscall entry** — `svc #0` from EL0 → sync exception at EL1; extract syscall number from X8, args from X0–X5; dispatch via `dispatch.rs`; return via `eret`
- [ ] **Context switch** — save callee-saved regs (X19–X30, SP), swap task pointers, restore; timer IRQ preemption from EL0 and EL1
- [ ] **`#[cfg(target_arch = "aarch64")]`** — wire `TaskContext = Aarch64Context` type alias in `arch/src/lib.rs`

#### ARM64 SoC + Kernel (QEMU virt)
- [ ] **`soc-qemu-virt-aarch64` crate** — PL011 UART (0x0900_0000), GICv2 (dist 0x0800_0000, cpu 0x0801_0000), ARM generic timer (CNTP_*), VIRTIO MMIO (0x0a00_0000+), RTC (PL031), flash (CFI), PCIe host bridge
- [ ] **`kernel-qemu-virt-aarch64` crate** — `aarch64-unknown-none-softfloat` target, EL2→EL1 drop (PSCI or direct), DTB from x0, linker script (RAM 0x4000_0000+)
- [ ] **EL2→EL1 transition** — set `HCR_EL2.RW=1` (AArch64 at EL1), configure `SCTLR_EL1`, `eret` to EL1 `_start_el1`; handle both EL2 (QEMU default) and EL1 (KVM) boot
- [ ] **GICv2 driver** — distributor init (GICD_CTLR, GICD_ISENABLER, GICD_IPRIORITYR, GICD_ITARGETSR), CPU interface init (GICC_CTLR, GICC_PMR), IRQ acknowledge (`GICC_IAR`) → dispatch → end (`GICC_EOIR`)
- [ ] **GICv3 driver (stubs)** — system register interface (`ICC_*_EL1`), redistributor per-core; needed for KVM on modern ARM64 hosts
- [ ] **ARM generic timer** — `CNTFRQ_EL0` for frequency, `CNTPCT_EL0` for monotonic time, `CNTP_TVAL_EL0` + `CNTP_CTL_EL0` for periodic tick (10ms); timer IRQ (PPI 30) → GIC → scheduler
- [ ] **PL011 UART** — TX/RX MMIO (UARTDR, UARTFR, UARTIBRD, UARTFBRD, UARTLCR_H, UARTCR); polled + interrupt-driven modes; implements `Serial` trait
- [ ] **VIRTIO-NET** — reuse existing VIRTIO MMIO driver from QEMU RISC-V (same register set); share `net` crate TCP/IP stack
- [ ] **VIRTIO-BLK** — block device for testing FAT32/VFS without SD hardware
- [ ] **VIRTIO-GPU (future)** — framebuffer for graphical console; `DisplayDevice` trait impl

#### ARM64 MMU + Memory Management
- [ ] **4 KB granule page tables** — 4-level (L0→L3) translation, 48-bit VA (256 TB), `TTBR0_EL1` (user, 0x0000...) / `TTBR1_EL1` (kernel, 0xFFFF...)
- [ ] **Kernel identity map** — map kernel text/data/stack 1:1 at boot; MMIO regions mapped as Device-nGnRnE
- [ ] **Per-process page tables** — each `Process` gets own L0 table in `TTBR0_EL1`; ASID tagging (`TTBR0_EL1[63:48]`) avoids TLB flush on switch
- [ ] **EL1/EL0 privilege split** — kernel pages PXN (Privileged Execute Never cleared), user pages UXN cleared; AP[2:1] for R/W/RO permissions
- [ ] **Demand paging stubs** — translation fault (ESR_EL1 DFSC/IFSC) → allocate page → map → resume; foundation for mmap/swap

#### KVM Acceleration
- [ ] **QEMU `-enable-kvm` validation** — `qemu-system-aarch64 -M virt -cpu host -enable-kvm` on ARM64 Linux hosts; verify VeerOS boots at near-native speed
- [ ] **KVM boot differences** — KVM starts guest at EL1 (not EL2); detect via `CurrentEL` read; skip EL2→EL1 transition
- [ ] **GICv3 for KVM** — KVM prefers GICv3 (`-M virt,gic-version=3`); system register access for IRQ management (`ICC_IAR1_EL1`, `ICC_EOIR1_EL1`, `ICC_SRE_EL1`)
- [ ] **VirtIO performance** — KVM + vhost-net for near-native networking; verify no MMIO emulation bottlenecks
- [ ] **Apple Silicon / UTM** — verify QEMU on macOS Apple Silicon via Hypervisor.framework (similar to KVM); document UTM setup
- [ ] **AWS Graviton / Ampere Altra** — CI/CD pipeline on ARM64 cloud instances with KVM; automated boot + test

#### ARM64 SMP (Multi-Core)
- [ ] **PSCI CPU_ON** — bring up secondary cores via PSCI `CPU_ON` (SMC/HVC call); each core enters `_secondary_start` → init GIC CPU interface → enter idle
- [ ] **Per-core state** — per-CPU idle task, per-CPU GIC interface, per-CPU timer; TPIDR_EL1 points to per-CPU data struct
- [ ] **Scheduler SMP** — run-queue per core, work stealing, IPI for cross-core wake-up (SGI via GIC)
- [ ] **Spinlocks** — `LDXR`/`STXR` (load-exclusive/store-exclusive) based spinlocks for SMP kernel data structures

#### ARM64 QEMU Launch Recipes
```
# TCG (any host)
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 256M \
  -kernel target/aarch64-unknown-none-softfloat/release/kernel-qemu-virt-aarch64 \
  -nographic -serial stdio \
  -device virtio-net-device,netdev=n0 -netdev user,id=n0,hostfwd=tcp::2323-:2323

# KVM (ARM64 Linux host)
qemu-system-aarch64 -M virt -cpu host -enable-kvm -m 256M \
  -kernel target/aarch64-unknown-none-softfloat/release/kernel-qemu-virt-aarch64 \
  -nographic -serial stdio
```

### RISC-V 64
- [ ] `soc-qemu-virt-riscv64` crate — reuse NS16550/CLINT with `usize = u64`
- [ ] `kernel-qemu-virt-riscv64` — `riscv64gc-unknown-none-elf` target, S-mode with SBI
- [ ] S-mode trap delegation — `sstatus`/`scause`/`sepc` instead of M-mode CSRs
- [ ] Sv39 page table support (if MMU path enabled)

### ESP32-C3 (RISC-V riscv32imc, 400 KB SRAM)
_Full bring-up on ESP32-C3: same RISC-V ISA as C6 but simpler — WiFi 4 + BLE 5.0, no 802.15.4, no USB. Single-core. Ideal for cost-optimized IoT nodes._

#### ESP32-C3 Architecture + SoC
- [ ] **`soc-esp32c3` crate** — ESP32-C3 peripherals: UART0 (0x6000_0000), UART1 (0x6001_0000), interrupt matrix (0x600C_2000), SysTimer (0x6002_3000), WDT (TG0/TG1), GPIO (0x6000_4000), SPI2 (0x6000_3000), I2C0 (0x6001_3000), RNG (0x6002_6000), eFuse (0x6000_8800)
- [ ] **`kernel-esp32c3` crate** — `riscv32imc-unknown-none-elf` target, M-mode boot, linker script (IRAM 0x4037_C000, DRAM 0x3FC8_0000, flash 0x4200_0000)
- [ ] **ESP32-C3 memory map** — 400 KB SRAM total (IRAM 0x4037_C000–0x4037_FFFF + DRAM 0x3FC8_0000–0x3FCE_FFFF), 16 KB RTC FAST (0x5000_0000), 4 MB flash (0x4200_0000)
- [ ] **UART0 driver** — TX/RX MMIO, polled + interrupt-driven; implements `Serial` trait; console I/O
- [ ] **WDT disable** — disable TG0 WDT + TG1 WDT + super WDT + RTC WDT early in boot (same pattern as C6)
- [ ] **SysTimer driver** — 52-bit counter, 3 comparators; periodic tick for scheduler; implements `TickTimer` trait
- [ ] **Interrupt matrix** — route peripheral IRQs to CPU interrupt lines; PLIC-like priority/enable; implements `InterruptController` trait
- [ ] **GPIO driver** — 22 GPIOs, function select, pull up/down, drive strength; GPIO_OUT/SET/CLR registers; implements `GpioPin` trait
- [ ] **SPI driver** — GPSPI2 for SD card and external peripherals; SPI-mode SD reuse from C6
- [ ] **I2C driver** — I2C0 for sensors; standard/fast mode
- [ ] **Build script + linker** — `build.rs` with `esp-wifi-sys` blob linkage (same structure as C6), `link/esp32c3.x` linker script

#### ESP32-C3 WiFi + BLE
- [ ] **WiFi blob integration** — `esp-wifi-sys` provides ESP32-C3 blobs (`libphy.a`, `libnet80211.a`, `libpp.a`); same OSI adapter pattern as C6 (`wifi_os_adapter.rs`)
- [ ] **WiFi STA mode** — scan, connect, WPA2/WPA3; shared `WifiManager` state machine from C6
- [ ] **WiFi AP mode** — software access point for configuration; captive portal for initial setup
- [ ] **BLE integration** — BLE 5.0 via Espressif blobs; HCI transport, GAP scan/advertise, GATT client/server
- [ ] **BLE HID client** — reuse `HogpManager` from C6 for BLE keyboard/mouse
- [ ] **Coexistence** — WiFi + BLE shared antenna arbitration via esp-coex stubs
- [ ] **DHCP + TCP shell** — smoltcp integration, shell-over-TCP on port 2323 (reuse net crate)

#### ESP32-C3 Specific
- [ ] **Hardware AES** — AES accelerator at 0x6003_A000 for crypto performance (8C integration)
- [ ] **Hardware SHA** — SHA accelerator at 0x6003_B000; offload hash computation
- [ ] **Hardware RSA** — RSA accelerator for public-key operations
- [ ] **Temperature sensor** — on-chip temperature sensor via SAR ADC; expose via `/dev/temp`
- [ ] **Deep sleep support** — RTC domain wakeup (timer, GPIO, UART); ultra-low-power mode for battery IoT
- [ ] **Flash encryption** — eFuse-based flash encryption (AES-XTS-256) for secure storage
- [ ] **Secure boot V2** — RSA-3072 signature verification from eFuse key
- [ ] **QEMU validation** — `qemu-system-riscv32 -M esp32c3` (if available) or shared QEMU virt testing

#### ESP32-C3 QEMU Launch Recipe
```
# Build
cargo build --release -p kernel-esp32c3

# Flash via esptool
esptool.py --chip esp32c3 --port /dev/ttyUSB0 write_flash \
  0x0 target/riscv32imc-unknown-none-elf/release/kernel-esp32c3
```

### Xtensa (ESP32-S3) — Full Bring-Up
_ESP32-S3: Xtensa LX7 dual-core, WiFi 802.11 b/g/n, BLE 5.0, 512 KB SRAM, 2–8 MB PSRAM, USB-OTG, PIE vector extensions for AI acceleration. The most capable ESP32 variant._

#### ESP32-S3 Architecture Crate
- [ ] **`arch_xtensa` crate** — `SavedContext` for Xtensa windowed ABI: A0–A15 + SAR + PS + PC + LBEG/LEND/LCOUNT (zero-overhead loop) + WINDOWBASE/WINDOWSTART; trap/exception frame layout
- [ ] **Window overflow/underflow handlers** — Xtensa register windowing: `WindowOverflow4/8/12` + `WindowUnderflow4/8/12` exception vectors; critical for function call ABI
- [ ] **Exception vector table** — vectors at 0x4003_7000 (VECBASE): Reset, DebugException, NMI, KernelException, UserException, DoubleException, plus window handlers
- [ ] **Context switch** — save/restore windowed registers (A0–A15 + special regs); flush register windows via `ROTW` + spill; swap task pointers
- [ ] **Syscall entry** — Xtensa `SYSCALL` instruction (causes exception level 1); extract args from A2–A7, syscall number from A2; return via `RFE`
- [ ] **`#[cfg(target_arch = "xtensa")]`** — wire `TaskContext = XtensaContext` type alias in `arch/src/lib.rs`

#### ESP32-S3 SoC + Kernel
- [ ] **`soc-esp32s3` crate** — ESP32-S3 peripherals: UART0 (0x6000_0000), UART1 (0x6001_0000), UART2 (0x6002_E000), interrupt matrix PRO_CPU (0x600C_2000) + APP_CPU (0x600C_2800), SysTimer (0x6002_3000), GPIO (0x6000_4000, 49 GPIOs), SPI2/SPI3 (0x6000_3000/0x6002_4000), I2C0/I2C1, GDMA (0x6003_F000), RNG (0x6003_5110), USB-OTG (0x6008_0000), LCD_CAM (0x6004_1000), ADC1/ADC2, SDMMC (0x6000_6000)
- [ ] **`kernel-esp32s3` crate** — `xtensa-esp32s3-none-elf` target, single-core PRO_CPU boot (APP_CPU parked), linker script
- [ ] **ESP32-S3 memory map** — 512 KB SRAM: IRAM (0x4037_0000–0x4037_FFFF), DRAM (0x3FC8_8000–0x3FCE_FFFF); 16 KB RTC FAST (0x600F_E000); 4/8/16 MB flash (0x4200_0000); optional 2–8 MB PSRAM (0x3C00_0000)
- [ ] **UART0 driver** — TX/RX MMIO; implements `Serial` trait
- [ ] **WDT disable** — TG0/TG1 + super WDT + RTC WDT
- [ ] **SysTimer driver** — periodic tick; implements `TickTimer` trait
- [ ] **Interrupt controller** — level + edge triggered, 32 CPU interrupts per core, priority 1–15; `InterruptController` trait
- [ ] **GPIO driver** — 49 GPIOs, strapping pins, function select, pull up/down; implements `GpioPin` trait
- [ ] **SPI driver** — SPI2 (GP-SPI) + SPI3; GDMA support for bulk transfers
- [ ] **I2C driver** — I2C0 + I2C1; standard/fast mode; sensor interface
- [ ] **GDMA controller** — General DMA for SPI, I2C, UART, LCD_CAM; channel allocation, linked-list descriptors

#### ESP32-S3 USB-OTG
- [ ] **USB-OTG peripheral** — ESP32-S3 built-in USB 1.1 OTG at 0x6008_0000; full-speed (12 Mbps)
- [ ] **USB CDC-ACM** — USB serial console as alternative to UART0; implements `Serial` trait
- [ ] **USB HID device** — present as USB HID keyboard/mouse (for demo/testing)
- [ ] **USB MSC device** — USB mass storage class; expose SD/flash as USB drive for easy file transfer
- [ ] **USB host mode** — enumerate external USB devices (keyboards, flash drives); reuse HID framework from 7D

#### ESP32-S3 Dual-Core SMP
- [ ] **APP_CPU bring-up** — write entry address to `SYSTEM_CORE_1_CONTROL_0_REG` (0x600C_0000); un-stall via `SYSTEM_CORE_1_CONTROL_1_REG`; APP_CPU enters `_secondary_start`
- [ ] **Per-core idle tasks** — each core runs independent idle task; core affinity for tasks
- [ ] **Per-core interrupt routing** — interrupt matrix routes peripherals to PRO_CPU or APP_CPU independently
- [ ] **Cross-core signaling** — IPC interrupt (interrupt line 0) for cross-core wake-up; hardware spinlocks via ATOMIC_LOCKER
- [ ] **SMP scheduler** — per-core run queue, work stealing, core affinity bitmask in TCB

#### ESP32-S3 WiFi + BLE
- [ ] **WiFi blob integration** — `esp-wifi-sys` ESP32-S3 blobs; same OSI adapter pattern; dual-band WiFi 802.11 b/g/n (2.4 GHz)
- [ ] **WiFi STA/AP modes** — station + soft-AP; concurrent STA+AP for provisioning
- [ ] **BLE 5.0** — Espressif BLE blobs; HCI transport; GAP + GATT; shared framework with C6
- [ ] **Coexistence** — WiFi + BLE coex on shared 2.4 GHz radio
- [ ] **WiFi throughput** — S3 has stronger CPU (dual-core 240 MHz); target higher throughput than C6

#### ESP32-S3 AI Acceleration (PIE)
- [ ] **PIE (Processor Instruction Extensions)** — ESP32-S3 Xtensa PIE SIMD: 128-bit vector ops, 8/16-bit integer MAC; accelerates INT8 inference
- [ ] **PIE operator kernels** — optimized MatMul, Conv2D, depthwise-conv using PIE intrinsics; 4–8× speedup over scalar
- [ ] **AI inference backend** — `PieBackend: InferenceBackend`; auto-dispatch quantized models to PIE; fallback to scalar for unsupported ops
- [ ] **Keyword spotter on PIE** — 20 KB wake-word model running at < 2ms inference via PIE acceleration
- [ ] **Camera + vision pipeline** — LCD_CAM peripheral for camera input (OV2640/OV5640); capture frame → PIE inference → classification

#### ESP32-S3 PSRAM
- [ ] **PSRAM init** — detect octal SPI PSRAM size (2/4/8 MB) at boot; configure cache-through mapping at 0x3C00_0000
- [ ] **PSRAM heap** — extend kernel heap into PSRAM for large allocations (model weights, frame buffers)
- [ ] **PSRAM-backed model store** — load AI models from flash into PSRAM for fast inference; memory-mapped access
- [ ] **PSRAM for frame buffers** — camera + LCD frame buffers in PSRAM (avoids SRAM pressure)

#### ESP32-S3 Peripherals
- [ ] **SDMMC host** — native SD/MMC interface (4-bit data bus); faster than SPI-mode SD; FAT32 integration
- [ ] **ADC** — ADC1 (10 channels) + ADC2 (10 channels); 12-bit resolution; sensor input
- [ ] **DAC** — 2-channel 8-bit DAC for audio output
- [ ] **LCD interface** — parallel 8/16-bit LCD via LCD_CAM; SPI LCD support; `DisplayDevice` trait impl
- [ ] **Camera interface** — DVP 8/16-bit camera via LCD_CAM; OV2640 frame capture
- [ ] **Touch sensor** — 14 capacitive touch GPIOs; touch-based UI input

#### ESP32-S3 Build Recipe
```
# Build
cargo build --release -p kernel-esp32s3

# Flash via esptool
esptool.py --chip esp32s3 --port /dev/ttyUSB0 write_flash \
  0x0 target/xtensa-esp32s3-none-elf/release/kernel-esp32s3
```

### Raspberry Pi Family (5, 4, 3, Zero)
_Full Raspberry Pi lineup — from the flagship RPi 5 (Cortex-A76, 8 GB) down to the tiny Zero 2 W (Cortex-A53, 512 MB). All AArch64. Primary focus: real hardware, tested end-to-end._

#### RPi Common (shared across all models)
- [ ] **`arch_arm64` crate** — `SavedContext` for AArch64 (X0–X30 + SP + PC + SPSR_EL1 + ELR_EL1), EL1 exception vectors, `svc` for syscalls, `eret` for return-to-user
- [ ] **AArch64 exception vectors** — VBAR_EL1 vector table (sync/IRQ/FIQ/SError × currentEL/lowerEL), full context save/restore (31 GPRs + SP + PSTATE)
- [ ] **MMU + page tables** — 4 KB granule, 48-bit VA (4-level), TTBR0_EL1 (user) / TTBR1_EL1 (kernel), ASID tagging per process
- [ ] **EL1/EL0 split** — kernel at EL1, user tasks at EL0; `svc #0` trap for syscalls, timer IRQ via CNTP_EL0
- [ ] **ARM generic timer** — CNTPCT_EL0 for timekeeping, CNTP_TVAL_EL0 for tick interrupt, per-core timers
- [ ] **GIC (interrupt controller)** — GICv2 (RPi 3/4/Zero), GICv3 stubs (RPi 5); distributor + CPU interface, IRQ priority + enable/disable
- [ ] **PL011 UART driver** — shared across all RPi models (base address varies per SoC); TX/RX, baud rate config, interrupt-driven RX
- [x] **Mailbox interface** — VideoCore mailbox for firmware queries: board revision, memory size, MAC address, serial number; BCM2712 base `0x10_7C013880`, property tag channel 8
- [x] **GPIO subsystem** — RP1 GPIO driver (`crates/soc/raspi5/src/gpio.rs`): 28 GPIOs via PCIe-mapped MMIO; per-pin STATUS/CTRL registers at 8-byte stride; function select (Alt0–Alt8), mode (Input/Output/Alt), pull (None/Up/Down), drive strength, schmitt trigger, slew rate; RIO atomic SET/CLR/XOR for output; `write_pin_list()` for shell status display
- [ ] **SD/eMMC boot** — RPi firmware loads `kernel8.img` from FAT32 partition; VeerOS as flat AArch64 binary, `config.txt` options
- [x] **USB host (xHCI DMA engine)** — Full xHCI 1.2 implementation with TRB ring infrastructure (Command Ring, Event Ring with ERST, Transfer Rings per-endpoint); DCBAA; statically-allocated DMA buffers (no heap); device enumeration (enable_slot → address_device → GET_DESCRIPTOR → parse VID/PID/class); HID endpoint configuration (parse config descriptor → find interrupt-IN EP → SET_CONFIGURATION → Configure Endpoint); boot protocol keyboard/mouse support; `write_port_list()`/`write_device_list()` for shell; dual controller (xHCI0 USB 3.0 + xHCI1 USB 2.0)
- [x] **Framebuffer console** — mailbox-allocated framebuffer (640×480 @ 32bpp) + 8×8 bitmap font (double-height → 8×16) for HDMI output; `FbConsole` implements `Serial` with UART mirror; 80×30 text terminal
- [ ] **Multi-core SMP** — secondary cores parked in spin-table or PSCI; kernel brings up cores 1–3 via mailbox/PSCI, per-core idle tasks
- [ ] **Device Tree parsing** — read DTB blob passed by firmware at boot (x0 register); extract memory map, interrupt routing, peripheral addresses
- [ ] **QEMU `raspi` machines** — `qemu-system-aarch64 -M raspi3b` (RPi 3), `-M raspi4b` (RPi 4) for CI/development without hardware
- [ ] **Real hardware CI/CD** — automated build → SD card image → serial console test via UART; probe-rs or custom test harness

#### Raspberry Pi 5 (BCM2712 — Cortex-A76, 4/8 GB)
_Flagship. Quad-core Cortex-A76 @ 2.4 GHz, 4/8 GB LPDDR4X, PCIe 2.0 x1 (NVMe), RP1 southbridge._

- [x] **`soc-raspi5` crate** — BCM2712 peripherals: RP1 southbridge (UART, SPI, I2C, GPIO via PCIe-mapped MMIO), GIC-400, ARM Generic Timer, mailbox, framebuffer, SD/EMMC2, xHCI USB
- [x] **`kernel-raspi5` crate** — `aarch64-unknown-none` target, AArch64 boot (EL2→EL1), trap vectors, context switch, full shell + VFS
- [x] **RP1 southbridge drivers** — UART (PL011), GPIO (28 pins, function select, pull, drive strength), SPI (DW APB SSI, SPI0–5, polled mode), I2C (DW APB I2C, I2C0–6, Standard/Fast mode, error handling)
- [x] **GIC-400** — GICv2 distributor + CPU interface; init, enable/disable interrupt, set priority, acknowledge/end
- [x] **xHCI USB host** — Full DMA engine + device enumeration + HID polling (see Phase 7D)
- [x] **SD/EMMC2** — SDHCI driver, SDv2/SDHC init, CMD17/CMD24 single-block R/W, BlockDevice trait
- [x] **Framebuffer console** — mailbox-allocated 640×480 @ 32bpp + 8×16 bitmap font; HDMI + UART dual output
- [x] **Platform trait** — init_cpu (FP/NEON enable via CPACR_EL1), init_interrupts (GIC init), init_timer (10ms tick)
- [x] **Shell callbacks** — lsblk (real SD card size), usb_list (real xHCI port/device info), input_status, all VFS/user/task callbacks
- [ ] **PCIe controller** — BCM2712 PCIe root complex init; enumerate bus, map BARs; needed for RP1 + NVMe
- [ ] **NVMe storage** — PCIe NVMe controller driver for boot/data storage (4 GB+ scenarios)
- [ ] **RPi 5 clock/power** — PMIC control, clock tree configuration, thermal throttle awareness
- [ ] **Ethernet (BCM54213PE)** — RP1 GENET MAC + BCM54213PE PHY; requires full MAC/DMA driver (~2000 lines)
- [ ] **Multi-core SMP** — secondary cores 1–3 via PSCI; per-core idle tasks
- [ ] **Camera/display stubs** — MIPI CSI-2/DSI via RP1 (future: camera input for AI/vision workloads)
- [ ] **VideoCore VII GPU (future)** — compute shaders for AI inference acceleration (Phase 10 integration)

#### Raspberry Pi 4 (BCM2711 — Cortex-A72, 1/2/4/8 GB)
_Workhorse. Quad-core Cortex-A72 @ 1.8 GHz, up to 8 GB LPDDR4, USB 3.0, Gigabit Ethernet._

- [ ] **`soc-rpi4` crate** — BCM2711 peripherals: UART (PL011 + mini-UART), GICv2 (GIC-400), PCIe for USB3/xHCI
- [ ] **`kernel-rpi4` crate** — `aarch64-unknown-none` target, 1/2/4/8 GB memory layouts
- [ ] **GICv2 (GIC-400)** — distributor at 0xFF841000, CPU interface at 0xFF842000; shared peripheral interrupts (SPI), per-core PPIs
- [ ] **BCM2711 memory map** — 0x0000_0000–0xFFFF_FFFF (low peripheral), 0x4_C000_0000 (high peripheral, 8 GB model)
- [ ] **Gigabit Ethernet driver (stubs)** — BCM54213PE GENET; needed for network boot, remote shell on real hardware
- [ ] **USB 3.0 (VL805 xHCI)** — PCIe-attached USB 3.0 host; keyboard, storage, USB-serial
- [ ] **HDMI framebuffer** — dual micro-HDMI; mailbox-based framebuffer allocation
- [ ] **QEMU validation** — `qemu-system-aarch64 -M raspi4b -m 2G -dtb bcm2711-rpi-4-b.dtb`

#### Raspberry Pi 3 Model B+ (BCM2837B0 — Cortex-A53, 1 GB)
_Entry-level 64-bit. Quad-core Cortex-A53 @ 1.4 GHz, 1 GB LPDDR2, WiFi, BLE._

- [ ] **`soc-rpi3` crate** — BCM2837B0 peripherals: PL011 UART, legacy interrupt controller (not GIC — custom BCM IRQ), system timer
- [ ] **`kernel-rpi3` crate** — `aarch64-unknown-none` target, 1 GB split (ARM/GPU via `config.txt gpu_mem`)
- [ ] **BCM legacy interrupt controller** — not GIC; BCM2835-style IRQ registers at 0x3F00_B200; basic + GPU pending registers
- [ ] **BCM2837 memory map** — peripherals at 0x3F000000 (bus) / 0x7E000000 (phys), 1 GB DRAM at 0x0
- [ ] **WiFi/BLE (stubs)** — CYW43455 via SDIO; future: wireless networking, BLE for IoT
- [ ] **QEMU validation** — `qemu-system-aarch64 -M raspi3b -kernel kernel8.img -serial stdio`

#### Raspberry Pi Zero 2 W (BCM2710A1 — Cortex-A53, 512 MB)
_Tiny form factor. Quad-core Cortex-A53 @ 1 GHz, 512 MB LPDDR2, WiFi, BLE. Ideal for embedded/IoT deployment._

- [ ] **`soc-rpi-zero2w` crate** — BCM2710A1 (same die as BCM2837, clock-limited); reuse `soc-rpi3` with clock/memory overrides
- [ ] **`kernel-rpi-zero2w` crate** — `aarch64-unknown-none` target, 512 MB memory constraint, minimal feature set
- [ ] **Reduced memory profile** — 512 MB total; kernel heap capped, MAX_TASKS reduced (8), aggressive memory accounting
- [ ] **Power-efficient idle** — WFI-based idle loop for battery-powered use cases; clock scaling
- [ ] **WiFi/BLE (shared)** — CYW43438 via SDIO; shared driver with RPi 3
- [ ] **Mini-UART console** — GPIO 14/15 UART for headless serial access (primary debug interface)
- [ ] **IoT deployment profile** — `dist-minimal` + `single-user`, optimized for sensor hubs, gateways, edge AI nodes

#### Raspberry Pi Pico 2 / RP2350 (Cortex-M33 + Hazard3 RISC-V, 520 KB SRAM)
_Microcontroller class. Dual-core Cortex-M33 or RISC-V Hazard3 (selectable), 520 KB SRAM, 4 MB flash. Dual-ISA support._

- [ ] **`arch_arm_cm33` crate** — `SavedContext` for Cortex-M33 (R0–R12 + SP + LR + PC + xPSR + EXC_RETURN), Thumb-2, TrustZone-M stubs
- [ ] **`soc-rp2350` crate** — RP2350 UART (PL011), SysTick, NVIC, PLL/XOSC clock init
- [ ] **`kernel-rp2350` crate** — `thumbv8m.main-none-eabihf` target, boot2 flash header, vector table
- [ ] **ARM MPU isolation** — 8-region MPU, per-task region programming on context switch
- [ ] **Dual-core SMP** — core1 via SIO FIFO mailbox (0xD000_0000), per-core idle tasks, hardware spinlocks
- [ ] **RISC-V Hazard3 mode** — boot in RISC-V mode (reuse `arch_riscv32`), ISA selected via OTP/boot pin
- [ ] **PIO + USB serial** — Programmable I/O state machines, TinyUSB CDC-ACM console

### x86-64 (QEMU PC + KVM)
_Full x86-64 bring-up on QEMU `q35`/`pc` machine and Linux KVM. The path to running VeerOS on standard PCs, servers, and cloud VMs. KVM gives near-native performance for development, testing, and production edge deployments._

#### x86-64 Architecture Crate
- [x] **`arch_x86_64` crate** — `SavedContext` for x86-64: 16 GPRs (RAX–R15) + RIP + RFLAGS + kernel_word; `SavedContext` trait impl; `get_syscall_nr` from RAX, args from RDI/RSI/RDX/RCX/R8/R9 (SysV ABI); wired into `arch::TaskContext` via `#[cfg(target_arch = "x86_64")]`
- [x] **GDT (Global Descriptor Table)** — 7-entry GDT: null, kernel CS (0x08), kernel DS (0x10), user DS (0x18), user CS (0x20), TSS (0x28-0x30); boot GDT in assembly + full Rust GDT with TSS and Ring 3 segments
- [x] **TSS (Task State Segment)** — 104-byte TSS with RSP0 (updated on context switch), IST1 for NMI/double-fault (4 KiB dedicated stack), IOPB; loaded via `ltr`
- [x] **IDT (Interrupt Descriptor Table)** — 256-entry IDT; ISR stubs (0–31 exceptions with/without error code, 32–47 IRQs, 48–255 software); each stub saves all 15 GPRs → calls `_veer_trap_dispatch_x86` → restores from (possibly different) TrapFrame → `iretq`; IDT loaded via `lidt`
- [x] **`syscall`/`sysret` fast path** — IA32_STAR (kernel CS=0x08, user base=0x10), IA32_LSTAR→`_veer_syscall_entry`, IA32_FMASK clears IF/DF/TF/AC; entry swaps to kernel RSP, builds TrapFrame, calls `_veer_trap_dispatch_x86`, restores via `sysretq`; SCE enabled in IA32_EFER
- [x] **Context switch** — TrapFrame ↔ X86_64Context save/restore on timer tick and syscall; `restore_context_to_frame` / `save_frame_to_context` + `iretq`-based first-task start; FPU/SSE lazy save deferred
- [x] **`#[cfg(target_arch = "x86_64")]`** — `TaskContext = X86_64Context` type alias in `arch/src/lib.rs` (already done in Phase 6A)

#### x86-64 Boot (Multiboot / UEFI)
- [x] **Multiboot v1 header** — `.multiboot` section in kernel ELF; ALIGN + MEMINFO flags; checksum; loaded by QEMU `-kernel`; Multiboot2 upgrade deferred
- [x] **Boot assembly** — `_start` in `.code32`: save Multiboot info (ESI/EDI), zero page tables, fill PML4→PDPT→PD identity map (512 × 2MB huge pages = 1 GiB), enable PAE (CR4.PAE), load CR3, set IA32_EFER.LME, enable paging (CR0.PG), load boot GDT, far-jump to `.code64`, reload segment regs, set RSP, zero BSS, call `_rust_start`
- [x] **Identity-map bootstrap page tables** — 3 × 4 KiB pages (PML4, PDPT, PD) in linker `.page_tables` section; 2 MB huge pages identity-map first 1 GiB; higher-half mapping deferred
- [ ] **UEFI boot path (future)** — `x86_64-unknown-uefi` stub that exits boot services, sets up page tables, jumps to kernel; for real hardware without Multiboot
- [ ] **Boot info parsing** — Multiboot info struct: memory map (E820), framebuffer, ACPI RSDP pointer, boot command line

#### x86-64 SoC + Kernel (QEMU q35)
- [x] **`soc-qemu-pc` crate** — COM1 UART (I/O ports 0x3F8), 8259 PIC remap, 8254 PIT timer; `Platform` trait impl; `inb`/`outb`/`io_wait` I/O port helpers
- [x] **`kernel-qemu-pc` crate** — `x86_64-unknown-none` target, Multiboot v1 boot, linker script (kernel at 1 MB physical); full kernel init: heap, VFS, scheduler, shell, sample tasks; `x86_64-unknown-none` added to `rust-toolchain.toml`
- [x] **Serial console (COM1)** — I/O port 0x3F8; 115200 8N1; FIFO enabled; polled TX/RX; `has_data()` via LSR; implements `Serial` trait
- [x] **Local APIC** — MMIO driver at 0xFEE0_0000; SIVR enable, LVT timer periodic mode, timer calibration via PIT channel 2 (1 ms reference), EOI, IPI/INIT-SIPI for SMP bring-up
- [x] **I/O APIC** — MMIO driver at 0xFEC0_0000; 24-entry redirection table; `route_irq()` for edge/physical ISA defaults; `init()` routes timer→32, kbd→33, COM1→36; mask/unmask per-pin
- [ ] **ACPI table parsing** — RSDP → RSDT/XSDT → MADT (APIC topology), FADT (PM timer, shutdown), HPET table; minimal AML interpreter deferred
- [x] **HPET timer** — driver at 0xFED0_0000; 64-bit counter, timer 0 periodic mode with legacy replacement routing; `init(period_us)` with auto-tick counting; fallback to PIT 8254
- [x] **PIC 8259 (legacy)** — remap IRQs 0–15 to vectors 32–47; unmask IRQ0 (timer) + IRQ2 (cascade); `send_eoi()` + `unmask()` helpers; used for initial bring-up before APIC
- [x] **PIT 8254 timer** — channel 0, mode 2 (rate generator), ~1 ms period (divisor from 1.193182 MHz); `TickTimer` trait impl; software tick counter via `AtomicU64`
- [x] **PS/2 keyboard** — IRQ1 handler reads scan codes from port 0x60; scan-code set 1 → ASCII with Shift/Ctrl/CapsLock; feeds into kernel ring buffer; `console_read_byte` reads from buffer
- [x] **VGA text mode (early boot)** — 80×25 @ 0xB8000; scroll, cursor tracking, hardware cursor via CRTC ports; implements `Serial` trait with PS/2 keyboard polling for `read_byte`; boot banner mirrored to VGA
- [x] **VIRTIO-PCI** — PCI config space enumeration; VIRTIO devices as PCI functions; virtio-net-pci legacy driver with static TX buffer, spin-wait completion, 16-buffer RX pool; working with TAP/bridge and NAT
- [x] **PCI bus enumeration** — mechanism 1 via ports 0xCF8/0xCFC; walks bus 0, devices 0–31, multi-function aware; reads vendor/device ID, class/subclass, BARs, IRQ line; `class_name()` for display; wired into boot log + `hwinfo` shell command

#### x86-64 Memory Management
- [x] **4-level page tables** — `map_page()` walks/allocates PML4→PDPT→PD→PT with 4 KiB pages; `unmap_page()` with `invlpg`; `identity_map_range()` helper; `load_cr3()`/`read_cr3()`/`flush_tlb()`; PTE flags (P/W/U/NX/HUGE/GLOBAL)
- [ ] **Higher-half kernel** — kernel linked at 0xFFFF_8000_0000_0000+; user space in lower half 0x0000_0000–0x0000_7FFF_FFFF_FFFF; canonical address enforcement
- [ ] **Per-process page tables** — each `Process` gets own PML4; `CR3` swap on context switch; PCID (Process Context Identifier) to avoid TLB flush
- [x] **Physical memory allocator** — bitmap-based FrameAllocator (256 MiB / 4 KiB = 65536 frames); `init(usable_start, usable_end)` from `__kernel_end` linker symbol; `alloc_frame()`/`free_frame()`; integrated into boot sequence with free count logged
- [ ] **Kernel heap** — `slab` or bump allocator for kernel-internal allocations; mapped in higher-half
- [ ] **Ring 0/Ring 3 split** — kernel pages with `Supervisor` bit; user pages with `User` bit; NX (No-Execute) on data pages; SMEP + SMAP enforcement

#### KVM Acceleration
- [x] **QEMU `-enable-kvm` validation** — `qemu-system-x86_64 -enable-kvm -cpu host -M q35` on x86-64 Linux hosts; verified with KVM acceleration, DHCP + SSH working
- [ ] **KVM boot differences** — KVM provides proper hardware timer (TSC/APIC) fidelity; verify LAPIC timer works in KVM mode (vs TCG emulation quirks)
- [ ] **KVM paravirt clock** — `kvm_clock` MSR (0x4B564D01) for stable TSC; KVM pvclock for accurate timekeeping under VM migration
- [ ] **KVM VIRTIO (vhost)** — vhost-net kernel module for near-native networking; vhost-blk for disk I/O; bypasses QEMU userspace emulation
- [ ] **Nested virtualization (future)** — VeerOS as a hypervisor: Intel VT-x (VMX) support, run guest VMs inside VeerOS; foundation for MicroVM isolation (Phase 8B)
- [ ] **Cloud deployment** — test on AWS EC2 (metal/kvm), GCP Compute Engine, Azure VM; automated CI with `qemu-system-x86_64 -enable-kvm`

#### x86-64 SMP (Multi-Core)
- [ ] **BSP/AP model** — Bootstrap Processor (BSP) runs boot, then wakes Application Processors (APs) via LAPIC INIT-SIPI-SIPI sequence
- [ ] **AP trampoline** — real-mode trampoline code at < 1 MB; AP wakes in real mode → protected mode → long mode → jumps to `_ap_start`
- [ ] **Per-CPU state** — per-CPU Local APIC, per-CPU TSS, per-CPU idle task; `GS_BASE` MSR points to per-CPU data struct
- [ ] **Scheduler SMP** — per-core run queue, cross-core IPI wake-up (LAPIC ICR), work stealing
- [ ] **Spinlocks** — `lock cmpxchg`-based spinlocks; ticket locks or MCS locks for fairness

#### x86-64 Hardware Targets (Beyond QEMU)
- [ ] **Intel NUC / Mini-PC** — UEFI boot, NVMe storage, Intel Ethernet, USB keyboard; full desktop-class VeerOS
- [ ] **Intel N100 / Celeron edge boxes** — low-power x86-64 for IoT gateways; 8–16 GB RAM; runs full AI stack (Phase 10)
- [ ] **Framework Laptop (stubs)** — keyboard, trackpad, USB-C, eDP display; long-term goal for VeerOS-on-laptop

#### x86-64 QEMU Launch Recipes
```
# TCG (any host)
qemu-system-x86_64 -M q35 -cpu qemu64 -m 256M \
  -kernel target/x86_64-unknown-none/release/kernel-qemu-pc \
  -nographic -serial stdio \
  -device virtio-net-pci,netdev=n0 -netdev user,id=n0,hostfwd=tcp::2323-:2323

# KVM (x86-64 Linux host)
qemu-system-x86_64 -M q35 -cpu host -enable-kvm -m 256M \
  -kernel target/x86_64-unknown-none/release/kernel-qemu-pc \
  -nographic -serial stdio

# KVM + vhost-net (production-like)
qemu-system-x86_64 -M q35 -cpu host -enable-kvm -m 1G -smp 4 \
  -kernel target/x86_64-unknown-none/release/kernel-qemu-pc \
  -nographic -serial stdio \
  -netdev tap,id=n0,vhost=on -device virtio-net-pci,netdev=n0
```

## Phase 8 — Security Architecture
_Security-first design — capability-based access, isolation domains, hybrid PQC-ready crypto, secure boot chain, extensible security model. Every subsystem respects these boundaries. Scales from bare-metal MCU (ESP32) to clustered systems, VMs, containers, routers/gateways, firewalls, quantum coprocessors, and AI accelerators/ASICs. Feature-gated tiers: `sec-base` (always on), `sec-sandbox`, `sec-crypto`, `sec-crypto-pqc`, `sec-verified`, `sec-network`, `sec-hw`._

### 8A — Capability-Based Access Control (Core)
_The foundation — every resource access requires an unforgeable capability token. No ambient authority. Zero-cost on `dist-minimal` (capabilities compile to no-op checks when `sec-base` is the only tier). Scales from 32-slot tables on ESP32 to 4096-slot tables on x86-64 cluster nodes._

#### Capability Primitives
- [ ] **`Capability` type** — unforgeable kernel-issued token: `{ id: u32, resource: ResourceKind, rights: Rights, owner: ProcessId, issuer: ProcessId, generation: u16, expiry: Option<u64> }`
- [ ] **`ResourceKind` enum** — comprehensive resource taxonomy:
  - **Core:** `Memory(region)`, `IpcPort(id)`, `Interrupt(line)`, `MmioRegion(base,size)`, `Device(driver_id)`, `Socket(handle)`, `File(inode_id)`, `ProcessControl(pid)`
  - **Crypto:** `CryptoKey(key_id)`, `CryptoEngine(hw_id)` (for hardware AES/SHA accelerator access)
  - **Quantum:** `Quantum(qpu_id)`, `QuantumCircuit(job_id)`
  - **AI:** `AiModel(model_id)`, `AiAccelerator(backend_id)`, `NpuSlice(npu_id, partition)`
  - **Network:** `NetworkInterface(nic_id)`, `FirewallRule(rule_id)`, `VpnTunnel(tunnel_id)`, `NetworkNamespace(ns_id)`, `PacketFilter(chain_id)`
  - **Isolation:** `Container(container_id)`, `MicroVM(vm_id)`, `Domain(domain_id)`
  - **Hardware:** `GpioPin(pin)`, `SpiBus(bus_id)`, `I2cBus(bus_id)`, `DmaChannel(ch_id)`, `Accelerator(asic_id)`, `PcieFunction(bdf)`
  - **Cluster:** `ClusterNode(node_id)`, `DistributedLock(lock_id)`, `ServiceEndpoint(svc_id)`
- [ ] **`Rights` bitflags** — `READ`, `WRITE`, `EXECUTE`, `GRANT` (can delegate to child), `REVOKE`, `MAP`, `SEND`, `RECV`, `ADMIN`, `CONFIGURE`, `MONITOR`, `PASSTHROUGH` (DMA/device passthrough)
- [ ] **Capability table** — per-process; size selected by target: `[Option<Capability>; 32]` (ESP32), `[Option<Capability>; 64]` (RPi), `[Option<Capability>; 256]` (x86-64), `[Option<Capability>; 4096]` (cluster node)
- [ ] **Generation counter** — prevents use-after-revoke: capability ID + generation must match; stale caps instantly rejected

#### Capability Syscalls
- [ ] **`SYS_CAP_CREATE` syscall** — kernel mints a new capability (root/parent only); specify resource, rights, optional expiry
- [ ] **`SYS_CAP_GRANT` syscall** — delegate capability (with optional rights restriction + time-to-live) from parent → child process
- [ ] **`SYS_CAP_REVOKE` syscall** — revoke a capability from a process (cascading: revokes all delegated children)
- [ ] **`SYS_CAP_QUERY` syscall** — list capabilities held by calling process; filter by `ResourceKind`
- [ ] **`SYS_CAP_INSPECT` syscall** — introspect a capability: resource, rights, issuer, expiry (requires `MONITOR` right on domain)
- [ ] **`SYS_CAP_TRANSFER` syscall** — atomically move a capability between processes (one-shot, receiver must accept)
- [ ] **`SYS_CAP_SEAL` syscall** — seal a capability: remove `GRANT` right permanently (cannot be re-delegated)

#### Enforcement
- [ ] **Enforcement in syscall dispatcher** — every resource-accessing syscall checks capability table before proceeding; deny = `EPERM`
- [ ] **Boot capabilities** — init/root process receives full capability set; spawned processes inherit only what parent grants
- [ ] **Capability-aware IPC** — send capabilities across process boundaries via IPC (cap transfer in message metadata)
- [ ] **Time-bounded capabilities** — optional expiry tick; kernel auto-revokes expired caps on next access check
- [ ] **Delegation depth limit** — configurable max delegation chain length (default 8); prevents unbounded cap propagation
- [ ] **Hardware-backed cap storage (future)** — on targets with TrustZone/SGX/PMP ePMP, store cap table in protected memory

### 8B — Isolation Domains (Sandboxing / Containers / MicroVMs)
_Hierarchical isolation levels — from lightweight sandboxes on ESP32 to hardware-enforced MicroVMs on x86-64 KVM. Supports network namespace isolation for router/firewall profiles, accelerator isolation for ASIC/QPU, and multi-tenant workload separation for cluster deployments._

#### Domain Model
- [ ] **`IsolationDomain` struct** — `{ id, level: IsolationLevel, parent: Option<DomainId>, process_set, resource_caps, cap_ceiling: Rights, memory_budget, cpu_budget, net_namespace: Option<NetNsId>, fs_root: Option<InodeId> }`
- [ ] **`IsolationLevel` enum** — `Shared` (soft, same address space), `Sandbox` (restricted caps, no raw HW access), `Container` (separate memory region + namespace), `MicroVM` (hardware-enforced: PMP/MPU/hypervisor), `SecureEnclave` (TrustZone/SGX)
- [ ] **Domain hierarchy** — domains nest: MicroVM contains Containers, Containers contain Sandboxes; flat on embedded (`Sandbox` only on ESP32)
- [ ] **`SYS_DOMAIN_CREATE` / `SYS_DOMAIN_DESTROY`** — create/tear down an isolation domain (requires `ADMIN` capability)
- [ ] **`SYS_DOMAIN_ENTER`** — spawn a process inside a domain (inherits domain restrictions)
- [ ] **`SYS_DOMAIN_QUERY`** — introspect domain's resource usage, policy, and contained processes
- [ ] **`SYS_DOMAIN_MIGRATE`** — live-migrate a container/MicroVM between cluster nodes (requires `ClusterNode` + `Domain` caps)

#### Sandbox (Software Isolation — All Targets)
- [ ] **Syscall filter** — per-domain allowlist of permitted syscall numbers (like seccomp-BPF); deny returns `EPERM`
- [ ] **Capability ceiling** — domain defines max rights any process within can hold (even if parent granted more); prevents privilege escalation
- [ ] **Namespace isolation** — sandboxed processes see only IPC ports / resources within their domain
- [ ] **Resource quotas** — memory ceiling, CPU time budget, max processes, max open handles per domain; enforced at syscall boundary
- [ ] **`seccomp`-style profiles** — predefined: `io-only` (read/write/yield), `compute-only` (no IPC/IO), `network-only`, `sensor-only` (GPIO/I2C/SPI read), `full`
- [ ] **ESP32/MCU sandbox** — PMP-backed: 2 regions (code RX + data RW) per sandboxed task; remaining PMP entries guard kernel; minimal overhead

#### Container (Memory-Isolated Workloads — RPi / x86-64 / RISC-V 64)
- [ ] **Per-container memory region** — dedicated PMP/MPU region or page table ASID; processes inside can't access host memory
- [ ] **Container namespaces** — PID namespace (PIDs internal to container), IPC namespace, network namespace (virtual NIC), mount namespace (private VFS root)
- [ ] **Container image** — flat binary or ELF loaded into container's memory region at spawn; signature-verified (8D)
- [ ] **Virtual filesystem per container** — per-container read-only `.rodata` slice for config/data; no global FS namespace leak; optional writable overlay in RAM
- [ ] **Container lifecycle** — create → start → pause → resume → stop → destroy; state machine enforced in kernel
- [ ] **Inter-container IPC** — only via explicit kernel-mediated channels with capabilities; no shared memory by default
- [ ] **Container networking** — virtual NIC per container (veth-like); kernel bridges to host NIC or firewall chain; per-container IP address + routing table
- [ ] **Container resource cgroups** — CPU shares, memory limits, I/O bandwidth limits per container (lightweight cgroup-like accounting)
- [ ] **OCI-compatible image format (future)** — load container images from standard OCI bundles on FAT32/NVMe storage

#### MicroVM (Hardware-Enforced Isolation — MMU Targets)
- [ ] **RISC-V H-extension support** — hypervisor extension (hgatp, VS/VU modes) for rv64 targets; stage-2 page tables for guest physical → host physical
- [ ] **ARM VHE / EL2 support** — type-2 hypervisor on ARM64 (stage-2 page tables, VGIC, virtual timer); RPi 4/5 as VM hosts
- [ ] **x86-64 VMX support** — Intel VT-x: VMCS per guest, VMLAUNCH/VMRESUME, EPT (Extended Page Tables), I/O bitmap, MSR bitmap
- [ ] **MicroVM descriptor** — virtual CPU count, memory size, device passthrough list, boot image, virtio device list
- [ ] **Trap-and-emulate** — guest traps forwarded to host handler; minimal device model (serial + virtio-net + virtio-blk)
- [ ] **Device passthrough** — grant a MicroVM direct access to a physical MMIO / PCIe device (e.g., NVMe, GPU, NIC); IOMMU/SMMU protection
- [ ] **Lightweight VMM** — < 10K lines; no BIOS emulation; direct kernel boot into guest; VeerOS-on-VeerOS nesting
- [ ] **Live migration (cluster)** — snapshot MicroVM state → transfer to another cluster node → resume; pre-copy memory migration

#### WASM Sandbox (Portable, Lightweight Execution — All Targets)
_WebAssembly as a universal execution sandbox: portable bytecode, memory-safe, capability-constrained. Lighter than containers, more portable than MicroVMs._

- [ ] **WASM interpreter** — `no_std` WASM bytecode interpreter in kernel; MVP spec (i32/i64/f32/f64, linear memory, tables); validates modules before execution
- [ ] **WASM linear memory sandbox** — each WASM module gets a bounded linear memory region; no access to kernel or other modules; memory limits enforced at instantiation
- [ ] **WASI-like host imports** — WASI-compatible host function interface: `fd_read`, `fd_write`, `clock_time_get`, `random_get`; capability-gated per import
- [ ] **WASM → fabric execution** — WASM modules as fabric execution units; `invoke("module.function", payload)` dispatched to nearest capable node; lighter than container migration
- [ ] **WASM module registry** — store compiled WASM modules in persistent memory; version-tagged; deploy fleet-wide via fabric gossip
- [ ] **WASM AOT compilation (future)** — ahead-of-time compile WASM to native code per-arch at deploy time; JIT on capable targets (x86-64, ARM64); interpret on MCUs
- [ ] **WASM capability restrictions** — per-module capability ceiling: which syscalls, which fabric nodes, which memory regions; `Domain::Wasm(module_id)` in isolation hierarchy
- [ ] **WASM + ZeroServices** — WASM modules as ZeroService handlers: `svc register("auth.login", wasm_module_id)` → invocations sandboxed in WASM, routed by kernel

#### Network Namespace Isolation (Router / Gateway / Firewall Profiles)
- [ ] **`NetNamespace` struct** — isolated network stack instance: own interfaces, routing table, firewall rules, ARP/NDP cache, socket table
- [ ] **Virtual interface pairs (veth)** — kernel-internal virtual Ethernet link connecting two namespaces; zero-copy packet forwarding
- [ ] **Per-namespace routing table** — independent IP routing per namespace; enables VRF (Virtual Routing and Forwarding) for multi-tenant gateways
- [ ] **Per-namespace firewall** — independent packet filter chain per namespace; `dist-firewall` profile runs each WAN/LAN zone in separate namespace
- [ ] **Bridge/switch domain** — L2 bridge between virtual interfaces; VLAN tagging; MAC learning table (for switch/router profiles)
- [ ] **Namespace-aware sockets** — `SYS_SOCKET` respects calling process's network namespace; cross-namespace only via explicit cap

#### Accelerator / ASIC Isolation
- [ ] **DMA isolation** — ensure accelerator DMA cannot access memory outside its granted region; IOMMU on x86-64/ARM, PMP-guarded on RISC-V, SoC-specific on ESP32
- [ ] **Per-accelerator domain** — each ASIC/NPU/QPU runs inside an isolation domain; kernel mediates all data transfer
- [ ] **Accelerator capability** — `Accelerator(asic_id)` with `EXECUTE`, `CONFIGURE`, `PASSTHROUGH` rights; prevents unauthorized firmware update or register access
- [ ] **Side-channel mitigation** — flush accelerator caches/state between user switches; timing-independent result delivery
- [ ] **Hot-plug/remove** — USB-attached accelerators (Coral, Hailo) can be securely enumerated, granted to a domain, and revoked on unplug

### 8C — Cryptographic Framework (Hybrid PQC-Ready)
_Pluggable crypto with algorithm agility. Hybrid approach: classical + post-quantum algorithms in parallel — secure if either holds. Hardware offload on capable SoCs. Scales from software-only SHA-256 on ESP32-C3 to AES-NI + AVX2 accelerated PQC on x86-64._

#### Crypto Trait Layer (`crates/crypto/`)
- [ ] **`crypto` crate** — `no_std`, `no_alloc` trait definitions; zero runtime cost when unused
- [ ] **`Hash` trait** — `update(&[u8])`, `finalize() -> Digest`; implementors: SHA-256, SHA-3-256, SHA-512, BLAKE3
- [ ] **`Mac` trait** — message authentication: `update(&[u8])`, `finalize() -> Tag`; HMAC-SHA256, KMAC, Poly1305
- [ ] **`Kdf` trait** — key derivation: `derive(ikm, salt, info, len) -> [u8]`; HKDF-SHA256, HKDF-SHA3, Argon2id (password-based)
- [ ] **`Aead` trait** — authenticated encryption: `seal/open(key, nonce, aad, plaintext) -> ciphertext`; AES-256-GCM, ChaCha20-Poly1305, AES-256-SIV (nonce-misuse resistant)
- [ ] **`Sign` trait** — digital signatures: `sign(key, msg) -> Sig`, `verify(pubkey, msg, sig) -> bool`
- [ ] **`Kem` trait** — key encapsulation: `encapsulate(pubkey) -> (shared_secret, ciphertext)`, `decapsulate(privkey, ciphertext) -> shared_secret`
- [ ] **`HybridKem` trait** — composite KEM: runs classical + PQC KEM in parallel, combines shared secrets via KDF; secure if either scheme holds
- [ ] **`HybridSign` trait** — composite signature: signs with both classical + PQC schemes, verifies both; document accepts if either valid (configurable: AND or OR policy)
- [ ] **`Rng` trait** — cryptographic RNG: `fill_bytes(&mut [u8])`; backed by hardware TRNG or DRBG; health-tested on every call
- [ ] **`Cipher` trait** — raw block/stream cipher for low-level use: `encrypt_block`, `decrypt_block`; AES-256, ChaCha20
- [ ] **Algorithm registry** — static dispatch via generics (no heap); feature flags select which algorithms are compiled in
- [ ] **`CryptoProvider` facade** — single entry point: `CryptoProvider::hash()`, `.sign()`, `.kem()`, `.aead()`; auto-selects best algorithm for target (PQC preferred if available → classical fallback)

#### Classical Algorithms (`sec-crypto-classical` feature)
- [ ] **SHA-256 / SHA-512** — `no_std` implementation or thin wrapper over `sha2` crate; HMAC, HKDF derived
- [ ] **SHA-3-256 / SHAKE128 / SHAKE256** — Keccak-based; used internally by PQC algorithms; important for quantum-resistant hashing
- [ ] **BLAKE3** — fast parallel hash; tree-hashing mode for large files / measured boot
- [ ] **AES-256-GCM** — for authenticated encryption (TLS, secure IPC); hardware AES on ESP32-C3/C6/S3, AES-NI on x86-64
- [ ] **AES-256-SIV** — nonce-misuse-resistant AEAD; preferred for at-rest encryption (flash, NVS, container images)
- [ ] **ChaCha20-Poly1305** — software-friendly AEAD (fallback for cores without AES hardware; faster than AES in software on riscv32imc)
- [ ] **Ed25519** — signature scheme for authentication, secure boot signature verification, SSH host keys
- [ ] **X25519** — ECDH key agreement (SSH key exchange, TLS handshake, secure IPC session keys)
- [ ] **ECDSA P-256** — for compatibility with existing PKI, TLS certificates, ESP32 Secure Boot V2
- [ ] **RSA-3072 (verify only)** — signature verification for ESP32 Secure Boot V2 eFuse keys; no private key operations (too expensive on MCU)
- [ ] **Argon2id** — password hashing for multi-user login (8C KDF); memory-hard, resists GPU/ASIC attacks; configurable memory cost per target

#### Hardware Crypto Acceleration (`sec-hw` feature)
- [ ] **ESP32-C3 AES accelerator** — AES engine at `0x6003_A000`; offload AES-128/256 encrypt/decrypt; `Aead` trait impl wraps hardware
- [ ] **ESP32-C3 SHA accelerator** — SHA engine at `0x6003_B000`; offload SHA-1/224/256; `Hash` trait impl wraps hardware
- [ ] **ESP32-C3 RSA accelerator** — large-number modular exponentiation; offload RSA-3072 verify; `Sign::verify()` fast path
- [ ] **ESP32-C6/S3 hardware crypto** — same AES/SHA/RSA accelerators + HMAC peripheral; unified driver shared across ESP32 family
- [ ] **ESP32 Digital Signature (DS)** — hardware-protected private key: key stored in encrypted eFuse, DS peripheral signs without exposing key to CPU; used for device identity attestation
- [ ] **x86-64 AES-NI** — `aesenc`/`aesenclast`/`aesdec` instructions for AES-256-GCM; detected via CPUID at boot; `Aead` fast path
- [ ] **x86-64 SHA-NI** — SHA instruction extensions; `sha256rnds2`/`sha256msg1`/`sha256msg2`; `Hash` fast path
- [ ] **x86-64 AVX2/512** — SIMD acceleration for PQC (NTT, polynomial multiplication); 4-8x speedup for ML-KEM/ML-DSA
- [ ] **ARM Crypto Extensions (ARMv8-A)** — `AESE`/`AESD`/`SHA256H`/`PMULL` instructions on Cortex-A53/A72/A76; `Aead`/`Hash` fast paths
- [ ] **ARM NEON** — 128-bit SIMD for PQC NTT; used on RPi 3/4/5 Cortex-A series
- [ ] **RISC-V Scalar Crypto (Zkn/Zks)** — AES/SHA instructions on supporting cores (future RISC-V with Zkn extension); `Aead`/`Hash` fast path
- [ ] **Hardware RNG per platform** — ESP32 `RNG_DATA_REG` (0x6002_6000), RISC-V `seed` CSR (Zkr), x86 `RDRAND`/`RDSEED`, ARM `RNDR`; health test + DRBG reseeding

#### Post-Quantum Algorithms — Hybrid Approach (`sec-crypto-pqc` feature)
_Hybrid PQC: always combine classical + post-quantum algorithms. Both run in parallel; shared secrets / signatures combined. System remains secure even if one scheme breaks. Compliant with NIST SP 800-227 hybrid key establishment guidance._

- [ ] **ML-KEM (Kyber)** — NIST FIPS 203 key encapsulation; ML-KEM-512 for constrained (ESP32), ML-KEM-768 default, ML-KEM-1024 for high-security (cluster/cloud)
- [ ] **ML-DSA (Dilithium)** — NIST FIPS 204 digital signatures; ML-DSA-44 for constrained, ML-DSA-65 default, ML-DSA-87 for critical (secure boot, cluster auth)
- [ ] **SLH-DSA (SPHINCS+)** — NIST FIPS 205 stateless hash-based signatures; conservative fallback (larger but mathematically simplest assumption — hash security only)
- [ ] **FN-DSA (Falcon)** — NIST selected lattice-based signature; compact signatures (smaller than Dilithium); requires careful floating-point or integer-only sampler
- [ ] **HQC (code-based KEM)** — NIST round 4 candidate; alternative KEM based on error-correcting codes (not lattice); diversifies assumptions
- [ ] **Hybrid KEM: X25519 + ML-KEM** — concatenate X25519 and ML-KEM ciphertexts; combine shared secrets via HKDF: `ss = HKDF(X25519_ss || MLKEM_ss)`; secure if either holds
- [ ] **Hybrid KEM: X25519 + ML-KEM + HQC (triple)** — three-algorithm composite for maximum diversity; optional `pqc-paranoid` feature
- [ ] **Hybrid Sign: Ed25519 + ML-DSA** — produce both signatures; verifier checks both; transition-safe (classical verifiers ignore PQC part)
- [ ] **Hybrid Sign: Ed25519 + SLH-DSA** — hash-based fallback hybrid; largest signature but most conservative security assumption
- [ ] **PQC parameter profiles** — `pqc-128` (ML-KEM-512 + ML-DSA-44), `pqc-192` (ML-KEM-768 + ML-DSA-65), `pqc-256` (ML-KEM-1024 + ML-DSA-87); selected via feature flag
- [ ] **Stack/memory budget validation** — ML-KEM-768 needs ~3KB stack, ML-DSA-65 ~5KB, SLH-DSA-128s ~2.5KB; validate fits in per-target task stacks (ESP32: 4KB, RPi: 8KB, x86: 16KB)
- [ ] **Constant-time implementation** — all PQC implementations must be constant-time (no secret-dependent branches/memory access); verified via `dudect` or similar timing tests
- [ ] **No-FPU PQC** — all PQC algorithms must work on targets without FPU (riscv32imc, Cortex-M33); integer-only NTT, rejection sampling

#### Crypto Services
- [ ] **Kernel keystore** — `[KeySlot; MAX_KEYS]` in protected kernel memory; keys never exposed to userspace raw; MAX_KEYS: 8 (ESP32), 32 (RPi), 256 (x86-64)
- [ ] **Key types** — `Symmetric(aead_key)`, `SigningKeyPair(privkey, pubkey)`, `KemKeyPair(privkey, pubkey)`, `HybridSigningKey(classical, pqc)`, `HybridKemKey(classical, pqc)`, `PreSharedKey(psk)`, `DerivedKey(parent_id, context)`
- [ ] **Key lifecycle** — generate → store → use → rotate → archive → destroy; state machine per key; `KeyState` enum
- [ ] **Key rotation** — automatic rotation based on usage count or time; old key kept for decrypt-only (grace period); new key minted atomically
- [ ] **`SYS_CRYPTO_HASH` / `SYS_CRYPTO_MAC`** — hash / MAC syscalls; operate on user-provided data
- [ ] **`SYS_CRYPTO_SIGN` / `SYS_CRYPTO_VERIFY`** — sign/verify using key handle; auto-selects hybrid if `sec-crypto-pqc` enabled
- [ ] **`SYS_CRYPTO_ENCRYPT` / `SYS_CRYPTO_DECRYPT`** — AEAD encrypt/decrypt using key handle; nonce auto-generated by kernel (prevents reuse)
- [ ] **`SYS_CRYPTO_KEM_ENCAP` / `SYS_CRYPTO_KEM_DECAP`** — hybrid KEM key exchange from userspace; returns combined shared secret
- [ ] **`SYS_CRYPTO_KEM_KEYGEN`** — generate a new KEM keypair (hybrid: generates both classical + PQC keypairs internally)
- [ ] **`SYS_CRYPTO_RNG`** — fill buffer with cryptographically secure random bytes; health-checked hardware RNG + DRBG
- [ ] **`SYS_CRYPTO_NEGOTIATE`** — client/server agree on best mutual algorithm: callers propose algorithm sets, kernel intersects and picks strongest
- [ ] **Key capability** — `CryptoKey(key_id)` capability required to use a key; revocable, non-transferable for private keys; `SIGN`, `VERIFY`, `ENCRYPT`, `DECRYPT`, `DERIVE` sub-rights
- [ ] **Crypto algorithm negotiation** — trait-based: callers request `AlgorithmClass::Kem` and kernel picks best available (hybrid PQC preferred → classical fallback → error)
- [ ] **Zeroization** — all key material zeroed on free/drop; `Zeroize` trait on all key structs; compiler fence to prevent optimization

### 8D — Secure Boot + Verified Launch
_Complete chain of trust from power-on to running user processes. Per-platform boot chains for the full VeerOS target spectrum. Hybrid PQC signatures for future-proofing._

#### Boot Chain Architecture
- [ ] **`BootStage` enum** — `Rom`, `Bootloader`, `Kernel`, `InitProcess`, `UserProcess`, `ContainerImage`, `MicroVMImage`; each stage measured + verified
- [ ] **`MeasurementRegister`** — software PCR-like accumulator: `extend(hash) → new_hash = SHA-256(old_hash || hash)`; one register per boot stage; stored in kernel-protected memory
- [ ] **Boot attestation report** — `[MeasurementRegister; 8]` covering each stage; queryable via `SYS_ATTESTATION_REPORT` syscall; can be sent to remote verifier
- [ ] **Hybrid boot signatures** — kernel and process images signed with Ed25519 + ML-DSA dual signature; bootloader verifies both
- [ ] **Algorithm agility in bootloader** — boot image header specifies signature algorithm(s); bootloader supports multiple verifiers; forward-compatible with new PQC standards

#### ESP32 Secure Boot (C3 / C6 / S3)
- [ ] **Secure Boot V2 integration** — Espressif eFuse-based: RSA-3072 or ECDSA-P256 public key hash burned into eFuse block; ROM verifies first-stage bootloader
- [ ] **Two-stage boot** — ROM → signed first-stage bootloader (verifies partition table + app image) → signed VeerOS kernel; full chain of trust
- [ ] **Flash encryption** — AES-XTS-256 flash encryption (key in eFuse); protects firmware at rest against physical extraction; `FLASH_ENCRYPTION` eFuse bit
- [ ] **eFuse key provisioning tooling** — `veer-provision` CLI tool: generate signing keypair, burn public key hash to eFuse, burn flash encryption key; one-time irreversible; confirms via serial prompt
- [ ] **Rollback protection (ESP32)** — `SECURE_BOOT_DIGEST` eFuse counter; reject images with version lower than burned value; increment on each OTA update
- [ ] **ESP32 Secure Boot + VeerOS hybrid** — first-stage bootloader verifies VeerOS kernel with Espressif RSA/ECDSA; VeerOS kernel then verifies user processes with Ed25519 + ML-DSA hybrid
- [ ] **NVS encryption** — encrypt non-volatile storage partition (WiFi credentials, keys) with flash encryption key; prevents plaintext credential extraction

#### ARM64 Secure Boot (Raspberry Pi / QEMU)
- [ ] **RPi firmware chain** — VideoCore GPU ROM → `bootcode.bin` → `start4.elf` → `config.txt` → `kernel8.img` (VeerOS); RPi firmware is closed-source but verified by Broadcom ROM
- [ ] **DTB integrity** — hash device tree blob passed at boot (x0); measure into boot attestation register; detect DTB tampering
- [ ] **`config.txt` lockdown** — document recommended settings: `kernel=kernel8.img`, `arm_64bit=1`, `enable_uart=1`; warn if `uart_2ndstage` enables debug UART in production
- [ ] **ARM Trusted Firmware (TF-A)** — for QEMU virt / production ARM64: BL1 (ROM) → BL2 (trusted boot firmware) → BL31 (EL3 runtime) → BL33 (VeerOS at EL2/EL1); each stage signed + measured
- [ ] **UEFI Secure Boot (ARM64)** — for generic ARM64 servers (Ampere, Graviton): UEFI Secure Boot with signed EFI stub; VeerOS EFI binary signed with Microsoft/custom PK chain
- [ ] **Measured boot on ARM64** — each stage hashes next stage into measurement register; fTPM (firmware TPM) accumulates PCR values; attestation via `SYS_ATTESTATION_REPORT`
- [ ] **Kernel image signing** — VeerOS `kernel8.img` signed with Ed25519 + ML-DSA; verified by TF-A BL33 loader or custom shim

#### x86-64 Secure Boot (QEMU / Bare Metal)
- [ ] **UEFI Secure Boot** — Secure Boot variables: PK (Platform Key), KEK (Key Exchange Key), db (allowed signatures), dbx (revoked); VeerOS EFI loader signed with enrolled key
- [ ] **Multiboot2 measured boot** — when loaded by GRUB2: GRUB measures itself + kernel into TPM PCRs; VeerOS extends PCR with own measurements
- [ ] **TPM 2.0 integration** — read PCR values from hardware/firmware TPM; seal/unseal kernel keys to TPM PCR state; attestation quotes
- [ ] **Intel Boot Guard (production)** — ACM (Authenticated Code Module) verifies initial firmware; chain extends to UEFI → VeerOS; hardware root of trust
- [ ] **Measured launch (Intel TXT / AMD SEV)** — hardware-measured launch for MicroVM isolation; hypervisor measured by CPU before execution
- [ ] **AMD SEV / SEV-SNP (future)** — encrypted VM memory; attestation report from AMD PSP; MicroVM isolation with confidential computing guarantees

#### RISC-V Secure Boot (QEMU / Boards)
- [ ] **OpenSBI measured boot** — SBI firmware (M-mode) → VeerOS (S-mode); SBI hashes VeerOS binary before jump; measurement stored in SBI-accessible register
- [ ] **RISC-V ePMP guarding** — enhanced PMP: lock M-mode entries so S-mode kernel cannot relax its own permissions; bootloader sets locked regions before jump
- [ ] **Boot ROM verification (MCU)** — on future RISC-V MCU boards with OTP ROM: ROM verifies first-stage loader; similar to ESP32 Secure Boot pattern

#### Common Boot Security
- [ ] **Immutable kernel .text** — mark kernel code region read-only + execute after boot; PMP/MPU/page-table enforced; no self-modifying code; panic on write attempt
- [ ] **Kernel .rodata protection** — mark read-only data (man pages, certificates, measurement registers) as RO + no-execute
- [ ] **Stack guard pages** — guard page (no-access) below every kernel stack; hardware-caught stack overflow → panic (not silent corruption)
- [ ] **Process image verification** — verify signature/hash of ELF/binary before loading into a process/container; reject unsigned images if `sec-verified` enabled
- [ ] **Container image verification** — container images must be signed; signature checked at `SYS_DOMAIN_CREATE`; hash measured into attestation register
- [ ] **Rollback protection (generic)** — monotonic version counter in persistent storage (eFuse / flash / TPM NV); reject images older than current version
- [ ] **Secure firmware update (OTA)** — download signed image → verify signature (hybrid PQC) → verify version monotonicity → atomic swap → reboot; brick-proof A/B partitioning on supported targets
- [ ] **Anti-downgrade for crypto** — cannot flash firmware with weaker crypto (e.g., classical-only) once PQC has been deployed; enforced by version counter + algorithm floor

### 8E — Secure Communication
_Encrypted channels for IPC, network, and debug interfaces. PQC-ready from day one. Covers local serial, LAN TCP, WAN VPN, inter-cluster mesh, and inter-domain IPC._

#### TLS 1.3 (Embedded)
- [ ] **Minimal TLS 1.3 client/server** — `no_std` + smoltcp integration; handshake + record layer; PSK and certificate-based authentication
- [ ] **PQC cipher suites** — TLS 1.3 with ML-KEM hybrid key exchange (X25519Kyber768Draft00) + ML-DSA certificates (draft-ietf-tls-hybrid)
- [ ] **Classical cipher suites** — `TLS_AES_256_GCM_SHA384`, `TLS_CHACHA20_POLY1305_SHA256`; selected based on hardware crypto availability
- [ ] **Certificate store** — small trusted CA certificate table in `.rodata` for TLS peer verification; max 4 CAs on ESP32, 16 on RPi, 64 on x86
- [ ] **Client certificates** — mTLS for device-to-device and device-to-cloud authentication; device identity from ESP32 DS peripheral or kernel keystore
- [ ] **Session resumption** — TLS 1.3 PSK tickets; avoid full handshake on reconnect; critical for constrained devices
- [ ] **ACME / auto-cert** — automatic Let's Encrypt certificate provisioning for network-connected devices with DNS

#### SSH Protocol
- [x] **SSH-2 server** — lightweight SSH-2 (RFC 4253/4254); shell channel with interactive VeerOS shell; chacha20-poly1305@openssh.com cipher, curve25519-sha256 KEX, password auth; working on QEMU PC over LAN (bridge/TAP)
- [ ] **PQC key exchange** — `sntrup761x25519-sha512` (hybrid) or `mlkem768x25519-sha256` (NIST hybrid); negotiated during KEX
- [x] **Host key management** — Ed25519 host key generated on first boot; stored in kernel keystore; `ssh-keygen`-compatible format for known_hosts
- [ ] **Public key authentication** — `authorized_keys` stored in VFS `/etc/ssh/`; Ed25519 + ML-DSA hybrid keys supported
- [ ] **SCP/SFTP server (stubs)** — file transfer over SSH; integrates with VFS (Phase 6J)

#### VPN / Tunnel Security (`sec-network` feature)
- [ ] **WireGuard protocol** — Noise IK handshake (X25519 + ChaCha20-Poly1305); kernel-mode tunnel interface; PQC hybrid extension: X25519 + ML-KEM for handshake
- [ ] **IPsec / ESP (stubs)** — Encapsulating Security Payload for inter-site VPN; IKEv2 key exchange with PQC hybrid; needed for `dist-firewall` / `dist-gateway`
- [ ] **DTLS 1.3** — datagram TLS for UDP services (CoAP, Thread commissioning, VoIP); `no_std` implementation
- [ ] **Tunnel interface** — kernel virtual network interface (`tun0`); packets encrypted/decrypted in kernel; capability-gated `VpnTunnel(tunnel_id)`

#### Inter-Domain / IPC Encryption
- [ ] **Encrypted IPC** — optional: inter-domain IPC messages encrypted with per-channel session key (for MicroVM ↔ host, container ↔ host)
- [ ] **IPC session key establishment** — on channel creation, kernel runs hybrid KEM between domains; session key used for AEAD on all messages
- [ ] **Inter-cluster TLS mesh** — cluster nodes communicate over mTLS; per-node certificate issued by cluster CA; PQC hybrid cipher suites
- [ ] **Secure multicast** — group key distribution for publish-subscribe IPC across domains; re-keyed on member join/leave

#### Physical Interface Security
- [ ] **Secure serial** — optional encrypted UART channel (for physical debug port protection); pre-shared key + ChaCha20-Poly1305
- [ ] **JTAG/SWD lockdown** — document how to disable debug interfaces on production devices: ESP32 JTAG disable eFuse, ARM debug authentication
- [ ] **USB security** — USB enumeration filtering: only allow known VID/PID combinations; prevent USB-based attacks (BadUSB); capability-gated USB access

### 8F — Audit, Monitoring + Intrusion Detection
_Security logging and runtime integrity monitoring. Scales from a 256-entry ring buffer on ESP32 to a persistent structured audit log on x86-64 with network export._

#### Audit Logging
- [x] **Security audit log** — ring buffer of security events; size: 128 entries (ESP32 dist-minimal), 512 (default), 4096 (dist-full); `AuditLog` struct with 16-byte entries (tick, pid, uid, event, flags, detail, detail2); wired into all 4 kernel targets
- [x] **`SYS_AUDIT_READ` / `SYS_AUDIT_COUNT` syscalls** (0xE0–0xE1) — userspace can read audit entries and query count; kernel-only write via dispatcher
- [x] **Audit event types** — `AuditEvent` enum (repr u8): `CapDenied`, `CapDropped`, `CapSetChild`, `AuthSuccess`, `AuthFail`, `Logout`, `UidChange`, `ProcessSpawn`, `ProcessExit`, `Mount`, `Unmount`, `RngSeeded`, `BootMeasurement`, `NetAccept`, `FirewallDrop`, `UserEvent`; auto-logged on cap denial, cap drop, cap set_child, boot RNG seeding
- [ ] **Audit log export** — forward audit events to remote syslog (UDP/TCP/TLS) or MQTT topic for fleet monitoring; `dist-cloud` auto-enables
- [x] **Shell `auditlog` command** — displays recent security events with formatted table (tick, pid, uid, event name, detail); accepts optional count argument; verified on ESP32-C6 hardware
- [ ] **Tamper-evident log** — each entry includes hash of previous entry (hash chain); detect log truncation/modification; rooted in boot measurement

#### Runtime Integrity Monitoring
- [ ] **Stack canaries** — compiler-based (`-Z stack-protector=strong`) or manual canary words; trap on corruption
- [ ] **Control flow integrity (CFI)** — RISC-V Zicfilp (landing pad) + Zicfiss (shadow stack) on supporting cores; ARM BTI (Branch Target Identification) on ARMv8.5+; software CFI fallback via shadow return stack
- [ ] **Heap integrity checks** — allocator metadata validation on every alloc/free; red-zone bytes around allocations; panic on corruption
- [ ] **Kernel code integrity** — periodic hash of `.text` section compared against boot measurement; detect runtime code modification
- [ ] **W^X enforcement** — no memory region is both writable and executable simultaneously; enforced by PMP/MPU/page tables on all targets
- [ ] **ASLR (Address Space Layout Randomization)** — randomize process base address, stack, heap on MMU targets (RPi, x86-64); kernel KASLR on x86-64

#### Intrusion Detection
- [ ] **Syscall anomaly detector** — per-process syscall sequence model (frequency histogram or Markov chain); flags unusual patterns → audit log
- [ ] **Brute-force detection** — track failed auth attempts per source (serial, SSH, TCP); auto-lockout after threshold (configurable: 3–10 attempts)
- [ ] **Network anomaly detection** — port scan detection, SYN flood detection, unusual traffic volume; auto-add firewall deny rule
- [ ] **Runtime attestation** — device can prove its boot measurements + running software to a remote verifier; challenge-response protocol using kernel keystore

### 8G — Network Security (Firewall / Gateway / Router)
_Packet filtering, NAT, DPI, and traffic security for `dist-firewall` and `dist-gateway` profiles. Capability-gated: `FirewallRule`, `PacketFilter`, `NetworkNamespace` resources._

#### Packet Filter Engine
- [ ] **`PacketFilter` struct** — ordered rule chain: `[Rule; MAX_RULES]`; each rule: match conditions → action (ACCEPT / DROP / REJECT / LOG / REDIRECT)
- [ ] **Match conditions** — src/dst IP, src/dst port, protocol (TCP/UDP/ICMP), interface, direction (IN/OUT/FORWARD), connection state (NEW/ESTABLISHED/RELATED)
- [ ] **Rule chains** — INPUT, OUTPUT, FORWARD (like iptables); per-interface and per-namespace chains; default policy: DROP (whitelist model)
- [ ] **Stateful tracking** — connection tracking table: `[ConnTrackEntry; MAX_CONNS]`; track TCP state machine, UDP timeout, ICMP echo matching
- [ ] **NAT (Network Address Translation)** — SNAT (masquerade outbound), DNAT (port forward inbound), 1:1 NAT; conntrack-integrated translation table
- [ ] **Rate limiting** — per-rule token-bucket rate limiter; prevent SYN flood, ICMP flood, brute-force
- [ ] **`SYS_FIREWALL_ADD` / `SYS_FIREWALL_DEL` / `SYS_FIREWALL_LIST`** — syscalls to manage rules (requires `FirewallRule` + `ADMIN` cap)
- [ ] **Shell `fw` / `iptables`-like commands** — `fw add INPUT -s 10.0.0.0/8 -p tcp --dport 22 -j ACCEPT`, `fw list`, `fw flush`

#### Deep Packet Inspection (DPI) — Optional
- [ ] **Protocol detection** — identify application layer protocol (HTTP/HTTPS/DNS/MQTT/CoAP/SSH) from packet payload patterns
- [ ] **DNS filtering** — inspect DNS queries/responses; blocklist domains; return NXDOMAIN for blocked; integrates with ad-blocking / parental control
- [ ] **TLS SNI inspection** — extract Server Name Indication from TLS ClientHello (unencrypted); filter by hostname without breaking encryption
- [ ] **IDS/IPS rules** — simple pattern-matching rules (Snort-lite); detect known attack signatures in packet payloads; LOG or DROP action

#### Traffic Shaping + QoS
- [ ] **Traffic classes** — classify packets into queues (real-time, interactive, bulk, background) based on DSCP, port, or DPI result
- [ ] **Token-bucket shaper** — per-class bandwidth limiter; ensures fair sharing and latency guarantees for priority traffic
- [ ] **WAN uplink management** — `dist-gateway`: shape upstream traffic to avoid buffer-bloat; SQM (Smart Queue Management) equivalent

### 8H — Hardware Security Integration (Per-SoC)
_Leverage hardware security features on each supported SoC. Feature-gated: `sec-hw`._

#### ESP32 Family (C3 / C6 / S3)
- [ ] **eFuse controller** — read/write eFuse blocks for key storage, secure boot config, flash encryption config; one-time programmable
- [ ] **Flash encryption (AES-XTS-256)** — transparent encrypt/decrypt of flash contents; key in eFuse; enabled per partition
- [ ] **Secure Boot V2** — ROM-rooted signature verification; RSA-3072 or ECDSA-P256 key in eFuse
- [ ] **Digital Signature (DS) peripheral** — hardware-protected ECDSA signing; private key encrypted at rest in eFuse, decrypted only inside DS peripheral; used for device attestation, mTLS client cert
- [ ] **HMAC peripheral** — hardware-backed HMAC-SHA256 with eFuse-stored key; used for secure token generation, NVS encryption key derivation
- [ ] **World Controller (ESP32-C6)** — hardware-enforced privilege separation (two "worlds": secure + normal); similar to ARM TrustZone; separate PMP configs per world
- [ ] **JTAG disable** — burn `JTAG_DISABLE` eFuse in production; prevent debug access to running firmware

#### ARM64 (Raspberry Pi / Generic)
- [ ] **ARM TrustZone** — EL3 Secure Monitor, Secure world (S-EL1/S-EL0), Normal world (EL1/EL0); VeerOS kernel in Normal world; secure crypto services in Secure world (optional OP-TEE)
- [ ] **ARM Pointer Authentication (PAC)** — ARMv8.3+ (RPi 5 Cortex-A76): sign return addresses with per-process key; detect ROP/JOP attacks; enable via `SCTLR_EL1.EnIA`
- [ ] **ARM Memory Tagging (MTE)** — ARMv8.5+ (future RPi): 4-bit tag per 16-byte granule; detect use-after-free, buffer overflow at hardware speed
- [ ] **ARM Branch Target Identification (BTI)** — ARMv8.5+: indirect branch targets must be BTI instructions; prevents JOP attacks; enable via `SCTLR_EL1.BT1`
- [ ] **RPi OTP (One-Time Programmable)** — BCM2712 OTP memory for device-specific secrets; read via mailbox; use for device identity seed

#### x86-64
- [ ] **TPM 2.0** — Trusted Platform Module for measured boot, key sealing, attestation; `tpm2` kernel driver (MMIO or TIS interface)
- [ ] **Intel SGX (enclaves)** — isolated execution environment for sensitive crypto operations; enclave holds keystore; future: SGX-backed `SecureEnclave` isolation level
- [ ] **AMD SEV / SEV-SNP** — encrypted VM memory; integrity protection; attestation from AMD PSP; used for confidential MicroVMs
- [ ] **Intel TDX (future)** — Trust Domain Extensions for confidential VMs; hardware-enforced isolation from hypervisor
- [ ] **SMEP / SMAP** — Supervisor Mode Execution/Access Prevention; prevent kernel from executing/reading user memory accidentally; enabled at boot
- [ ] **NX (No-Execute) bit** — all data pages marked NX; all code pages marked read-only; W^X enforcement

#### RISC-V
- [ ] **PMP (Physical Memory Protection)** — 16 PMP entries (riscv32imc); kernel locks entries 0–3 for kernel code/data; remaining entries per-task
- [ ] **ePMP (Enhanced PMP)** — machine security configuration: lock M-mode entries, deny S/U-mode access to unlisted regions (whitelist model)
- [ ] **Zkr (Entropy Source)** — hardware random number generator via `seed` CSR; direct fill for `Rng` trait
- [ ] **Zkn (Scalar Crypto)** — AES/SHA scalar instructions; constant-time hardware crypto primitives
- [ ] **Zicfilp + Zicfiss** — landing pad + shadow stack for forward/backward CFI; enable when hardware supports

### 8I — Security Policy Engine (Extensible Security Model)
_Modular, rule-based security policy framework. Policies are declarative configurations, not hard-coded logic. Enables custom security profiles for different deployment scenarios._

#### Policy Framework
- [ ] **`SecurityPolicy` struct** — named policy: `{ name, rules: [PolicyRule; MAX_RULES], default_action: Deny }` applied to a domain or system-wide
- [ ] **`PolicyRule` struct** — `{ subject: SubjectMatch, object: ObjectMatch, action: Action, effect: Allow|Deny|Audit }`
- [ ] **Subject matching** — by UID, GID, process name, domain ID, capability set; wildcards supported
- [ ] **Object matching** — by resource kind, specific resource ID, path pattern, syscall number
- [ ] **Policy evaluation** — first-match-wins ordered rule list; default deny if no rule matches; audit-only mode for testing
- [ ] **`SYS_POLICY_LOAD` / `SYS_POLICY_QUERY`** — load/query active security policy (requires `ADMIN` cap on system domain)
- [ ] **Policy profiles** — predefined for common scenarios:
  - `policy-iot-sensor` — minimal: GPIO read + IPC send + sleep; deny all else
  - `policy-network-service` — socket + file + IPC; deny raw hardware
  - `policy-router` — network interfaces + firewall + NAT; deny user processes from raw hardware
  - `policy-hypervisor` — VM management + capability admin; deny direct device access
  - `policy-development` — permissive: allow most syscalls; audit-only deny (for dev/test)

#### Mandatory Access Control (MAC) — Future
- [ ] **Label-based MAC** — every process and object tagged with security label (like SELinux/SMACK); kernel enforces label-to-label rules
- [ ] **Information flow control** — prevent data from flowing from high-security to low-security domains; enforced at IPC and file write boundaries
- [ ] **Bell-LaPadula model (stubs)** — no-read-up, no-write-down; for government/military classification scenarios
- [ ] **Biba integrity model (stubs)** — no-write-up, no-read-down; ensures high-integrity processes can't be corrupted by low-integrity data

### 8J — Security Feature Integration Matrix
_How security tiers map to features. For profile → security tier mapping and per-SoC hardware capabilities, see Phase 4D and 4F._

```
                    sec-base   sec-sandbox  sec-crypto  sec-crypto-pqc  sec-network  sec-hw    sec-verified
                    (always)   (feature)    (feature)   (feature)       (feature)    (feature) (feature)
────────────────────────────────────────────────────────────────────────────────────────────────────────────
Capabilities          ✓           ✓             ✓            ✓              ✓           ✓          ✓
Syscall filter        ─           ✓             ─            ─              ─           ─          ✓
Isolation domains     ─           ✓             ─            ─              ─           ─          ✓
Containers            ─           ✓             ─            ─              ─           ─          ✓
MicroVM               ─           ─             ─            ─              ─           ─          ✓
Net namespaces        ─           ✓             ─            ─              ✓           ─          ✓
Crypto traits         ─           ─             ✓            ✓              ─           ─          ✓
Classical crypto      ─           ─             ✓            ✓              ─           ─          ✓
Hybrid PQC            ─           ─             ─            ✓              ─           ─          ✓
HW crypto accel       ─           ─             ─            ─              ─           ✓          ✓
Secure boot           ─           ─             ─            ─              ─           ✓          ✓
Measured boot         ─           ─             ─            ─              ─           ─          ✓
TLS 1.3               ─           ─             ✓            ✓              ✓           ─          ✓
SSH server            ─           ─             ✓            ✓              ✓           ─          ✓
VPN / WireGuard       ─           ─             ─            ─              ✓           ─          ✓
Firewall / NAT        ─           ─             ─            ─              ✓           ─          ✓
DPI / IDS             ─           ─             ─            ─              ✓           ─          ✓
Audit log             ─           ✓             ─            ─              ─           ─          ✓
CFI / canaries        ─           ─             ─            ─              ─           ✓          ✓
W^X enforcement       ✓           ✓             ✓            ✓              ✓           ✓          ✓
Policy engine         ─           ✓             ─            ─              ─           ─          ✓
Runtime attestation   ─           ─             ─            ─              ─           ─          ✓
────────────────────────────────────────────────────────────────────────────────────────────────────────────

→ See Phase 4D for security tier defaults per distribution profile.
→ See Phase 4F for hardware security capabilities per SoC.
```

## Phase 8K — Unified Accelerator Interface (UAI)
_Single kernel-facing contract for co-processors, hardware accelerators, FPGAs, and quantum processors. Hardware-agnostic control-plane that abstracts vendor protocols behind one async submit/poll/cancel model. Targets x86-64 and ARM64 hosts; stubbed on RISC-V. Feature-gated: `accel` (core traits + registry), `accel-fpga` (bitstream programming), `accel-quantum` (QPU-specific extensions)._

### 8K-Core — Abstraction Layer (Complete)
_Kernel module, capability bit, syscall ABI, and dispatcher stubs landed._

- [x] **`AcceleratorClass` enum** — `Coprocessor`, `Accelerator`, `Fpga`, `Quantum`; covers every heterogeneous compute endpoint class
- [x] **`AcceleratorBus` enum** — `Mmio`, `Pcie`, `Cxl`, `Virtio`, `Spi`, `I2c`, `SharedMemory`, `Vendor`; interconnect abstraction
- [x] **`AcceleratorTransport` enum** — `DoorbellQueue`, `Mailbox`, `RingBuffer`, `RegisterCommand`; execution transport model
- [x] **`AcceleratorCapabilities` struct** — `max_queues`, `max_transfer_bytes`, `dma_coherency`, `supports_preemption`, `supports_sriov`, `supports_bitstream_reconfig`, `supports_quantum`, `max_physical_qubits`, `max_logical_qubits`
- [x] **`QuantumInfo` struct** — `model` (Gate/Annealing/Analog/Simulator), `t1_ns`, `t2_ns`, `gate_error_ppm`, `readout_error_ppm`; per-QPU fidelity metadata
- [x] **`AcceleratorDevice` descriptor** — id, class, bus, transport, irq_line, control/queue MMIO regions, capabilities, optional quantum info
- [x] **`WorkDescriptor` struct** — generic job: queue, `WorkloadClass` (Vector/Matrix/Signal/Crypto/BitstreamProgram/QuantumCircuit/QuantumSampling/Vendor), opcode, flags, in/out buffers, requested_qubits, deadline_tick
- [x] **`CompletionRecord` struct** — token, `CompletionState` (Pending/Running/Done/Failed/Cancelled/Timeout), status_code, bytes_written
- [x] **`AcceleratorRuntime` trait** — `device()`, `submit()`, `poll()`, `cancel()`, `fence()`; implemented by concrete drivers
- [x] **`AcceleratorRegistry`** — fixed-size `[Option<AcceleratorDevice>; 16]`; register, lookup by id, count by class
- [x] **`ACCEL` process capability** — new `ProcessCaps::ACCEL` bit (1 << 18); included in `user_default()` set; checked by dispatcher
- [x] **Syscall ABI (0xB0–0xB6)** — `SYS_ACCEL_COUNT`, `SYS_ACCEL_INFO`, `SYS_ACCEL_SUBMIT`, `SYS_ACCEL_POLL`, `SYS_ACCEL_CANCEL`, `SYS_FPGA_PROGRAM`, `SYS_QPU_SUBMIT`; all capability-gated; placeholder handlers return `usize::MAX` until drivers wired
- [x] **Design document** — `docs/accelerator-interface.md` with architecture, security model, and integration roadmap

### 8K-Runtime — Kernel Integration (Planned)
_Wire the registry into BSPs, implement first concrete backends, add userlib wrappers._

- [ ] **Instantiate `AcceleratorRegistry` in kernel BSPs** — static cell in qemu_virt, raspi5, and future x86-64 kernel; passed to dispatcher
- [ ] **Wire syscall handlers to registry** — `SYS_ACCEL_COUNT` returns `registry.count()`; `SYS_ACCEL_INFO` copies device descriptor to userland buffer; submit/poll/cancel route to matched `AcceleratorRuntime`
- [ ] **Scheduler integration** — new `BlockReason::AccelWait(token)` variant; task blocks on `SYS_ACCEL_POLL` and wakes on completion IRQ or polling tick
- [ ] **DMA buffer management** — contiguous physical buffer allocation for device I/O; coherency fence calls before/after ownership transfer
- [ ] **Userlib `accelerator` module** — safe wrappers: `accel_count()`, `accel_info(idx)`, `accel_submit(dev, work)`, `accel_poll(dev, token)`, `accel_cancel(dev, token)`, `fpga_program(dev, bitstream)`, `qpu_submit(dev, circuit, qubits)`
- [ ] **Shell `accel` command** — `accel list` (enumerate devices), `accel info <id>` (show capabilities), `accel status` (queue depths)

### 8K-Backends — Concrete Drivers (Planned)
_Vendor-specific drivers mapping to the generic `AcceleratorRuntime` trait._

- [ ] **VIRTIO-ACCEL backend** — QEMU virtio accelerator device; doorbell/queue transport; used for CI / development testing
- [ ] **FPGA bitstream loader (generic)** — `SYS_FPGA_PROGRAM`: validate bitstream header, write to FPGA config port (MMIO or SPI); support iCE40 / ECP5 / Xilinx partial reconfig
- [ ] **PCIe accelerator enumeration** — on x86-64 / ARM64 PCIe hosts, scan for accelerator BARs by PCI class code; auto-register discovered devices
- [ ] **Google Coral Edge TPU driver** — USB-attached INT8 inference accelerator; register as `AcceleratorClass::Accelerator`; submit TFLite delegate jobs
- [ ] **Hailo-8 M.2 driver** — 13 TOPS NPU on RPi 5 M.2 HAT+; PCIe BAR-mapped control; INT8 inference delegation
- [ ] **Quantum simulator bridge** — register Phase 9 simulator as `AcceleratorClass::Quantum` + `QuantumModel::Simulator`; route `SYS_QPU_SUBMIT` to state-vector engine
- [ ] **Remote QPU proxy** — network-attached QPU exposed as local device via kernel channel; TLS-secured command/result relay
- [ ] **CXL accelerator stubs** — CXL Type 2 device enumeration; shared host memory for coherent accelerator access (future x86-64 servers)

### 8K-Security — Isolation & Accounting (Planned)
_Ensure accelerator access is safe, auditable, and isolated._

- [ ] **Per-device capability tokens** — `ResourceKind::Accelerator(id)` with `EXECUTE`, `CONFIGURE`, `PASSTHROUGH` rights; revocable per process
- [ ] **DMA isolation enforcement** — IOMMU / SMMU / PMP guard accelerator DMA regions; prevent device-initiated memory corruption
- [ ] **Accelerator audit events** — `AuditEvent::AccelSubmit`, `AccelComplete`, `AccelDenied` logged to security audit ring (Phase 8F)
- [ ] **Side-channel mitigation** — flush accelerator caches/state between user switches; timing-independent result delivery
- [ ] **Hot-plug / hot-remove** — USB/PCIe accelerator enumeration on plug; secure revocation of capabilities on unplug; in-flight jobs cancelled
- [ ] **Resource accounting** — per-process accelerator time budget; kernel tracks cumulative device-seconds; quota enforcement at submit

## Phase 9 — Quantum CoProcessor Support
_Hardware quantum coprocessor interface + extensible simulator/emulator. Designed for the NISQ era and beyond — supports noisy intermediate-scale circuits today, fault-tolerant quantum computing tomorrow. Feature-gated: `quantum` (simulator always included), `quantum-hw` (real QPU drivers), `quantum-cloud` (remote QPU access)._

### 9A — Quantum Abstraction Layer (`crates/quantum/`)
_Architecture-neutral trait layer — same API for simulators, local QPU hardware, cloud QPUs, and FPGA emulators. Inspired by Qiskit/Cirq/Pennylane but `no_std`-first._

- [ ] **`quantum` crate** — `no_std`, `no_alloc` core types and traits; zero cost when feature disabled; versioned circuit IR
- [ ] **`Qubit` type** — opaque handle: `Qubit(u16)` index into QPU register file; lifetime-tracked (use-after-measure = compile error)
- [ ] **`Gate` enum** — standard gate set:
  - Single-qubit: `H`, `X`, `Y`, `Z`, `S`, `Sdg`, `T`, `Tdg`, `Rx(θ)`, `Ry(θ)`, `Rz(θ)`, `U(θ,φ,λ)` (universal single-qubit)
  - Two-qubit: `CNOT`/`CX`, `CZ`, `CY`, `SWAP`, `iSWAP`, `ECR`, `Rxx(θ)`, `Ryy(θ)`, `Rzz(θ)` (Ising coupling gates)
  - Three-qubit: `Toffoli`/`CCX`, `Fredkin`/`CSWAP`, `CCZ`
  - Parameterized: all rotation gates take `FixedPoint<i32, 16>` angle (no FPU required on embedded)
- [ ] **`Circuit` struct** — DAG-based circuit IR; `heapless::Vec<GateOp, MAX_CIRCUIT_OPS>` (configurable 256–4096 ops); supports barriers, classical registers, mid-circuit measurement
- [ ] **`Measurement` type** — classical bit result: `Zero` | `One`; `ClassicalRegister([Measurement; N])` for batch readout
- [ ] **`QuantumBackend` trait** — `allocate(n) -> Result<QubitRange>`, `apply(gate, qubits)`, `measure(qubit) -> Measurement`, `execute_circuit(&Circuit) -> ClassicalRegister`, `reset()`, `backend_info() -> BackendInfo`
- [ ] **`BackendInfo` struct** — `name`, `backend_type` (Simulator/Hardware/Cloud/FPGA), `max_qubits`, `native_gates`, `connectivity_map`, `gate_fidelities`, `t1_t2_times`, `queue_depth`
- [ ] **`QuantumError` enum** — `NotEnoughQubits`, `InvalidQubit`, `GateNotSupported`, `CircuitTooLarge`, `DecoherenceTimeout`, `HardwareError`, `CalibrationExpired`, `CloudTimeout`, `TranspileError`
- [ ] **Circuit builder API** — fluent: `Circuit::new(4).h(0).cnot(0,1).rz(1, PI/4).barrier().measure_all()` — compiles to gate DAG
- [ ] **Qubit topology** — `ConnectivityMap`: adjacency list of physical qubit connections; backends declare supported 2-qubit gate pairs
- [ ] **Backend registry** — static dispatch: `QuantumBackend` implementations registered at compile time via feature flags; runtime selection via capability token

### 9B — Quantum Simulator / Emulator
_Full state-vector simulator + density matrix simulator for development, testing, and NISQ-era algorithm prototyping._

- [ ] **State-vector simulator** — `2^n` complex amplitudes (`[Complex<f32>; 2^N]`); N ≤ 16 on embedded (64 KB), N ≤ 24 on RPi 5 (128 MB for 24 qubits), N ≤ 30+ on desktop
- [ ] **`Complex<f32>` type** — `{ re: f32, im: f32 }` with `mul`, `add`, `norm_sq`, `conj`; no libm dependency; `Complex<f64>` for targets with FPU
- [ ] **Gate matrices** — compile-time 2×2 / 4×4 / 8×8 unitaries; const-evaluated for native gate set; runtime matrices for parameterized gates
- [ ] **State-vector evolution** — apply gate by iterating amplitude pairs; single-qubit: O(2^n), two-qubit: O(2^n), optimized cache-friendly traversal
- [ ] **Measurement simulation** — Born-rule probabilistic collapse; TRNG (`SYS_CRYPTO_RNG`) or seeded PRNG; mid-circuit measurement with conditional gates
- [ ] **Density matrix simulator** — `2^n × 2^n` density matrix for mixed-state simulation; enables noise modeling, decoherence, partial trace
- [ ] **Noise model framework** — pluggable noise channels:
  - Gate errors: depolarizing, bit-flip, phase-flip, amplitude damping, phase damping
  - Readout errors: asymmetric bit-flip on measurement (configurable per-qubit)
  - Thermal relaxation: T1/T2 time-based decoherence between gate operations
  - Custom noise: user-defined Kraus operators for exotic noise models
- [ ] **`SimulatorBackend` struct** — implements `QuantumBackend`; configurable: state-vector (fast, noiseless) or density-matrix (slower, noisy)
- [ ] **Stabilizer/Clifford fast path** — detect Clifford-only circuits (H, S, CNOT, measurement) and use Gottesman-Knill O(n²) simulator instead of exponential state vector
- [ ] **Tensor network backend (future)** — for circuits with low entanglement, MPS/MPO-based simulation scales to 50+ qubits on limited RAM
- [ ] **Qubit limit autodetection** — probe available heap at init, set `max_n = floor(log2(avail_bytes / 8))`
- [ ] **Circuit execution engine** — iterate circuit DAG; apply gates, perform mid-circuit measurements, evaluate classical conditionals
- [ ] **Deterministic mode** — fixed PRNG seed for reproducible results; essential for kernel-level testing
- [ ] **Performance baseline** — targets: 12-qubit Hadamard < 1ms on rv32imc @ 160MHz; 20-qubit QFT < 100ms on Cortex-A76 (RPi 5)
- [ ] **Shot-based execution** — run circuit N times (shots), return histogram of measurement outcomes; matches real QPU workflow

### 9C — Circuit Compilation + Transpilation
_Transform abstract circuits into hardware-executable form — gate decomposition, qubit routing, optimization._

- [ ] **OpenQASM 3.0 parser** — parse OpenQASM 3.0 text format into VeerOS `Circuit` IR; enables interop with Qiskit/Cirq/Pennylane
- [ ] **Gate decomposition** — decompose arbitrary single-qubit U(θ,φ,λ) into native gate set (e.g., Rz-SX-Rz for IBM backends)
- [ ] **Qubit routing / mapping** — map logical qubits to physical qubits respecting hardware connectivity; insert SWAP gates as needed
- [ ] **Circuit optimization passes** — cancel adjacent inverse gates (H·H → I, X·X → I), merge rotations (Rz(a)·Rz(b) → Rz(a+b)), template matching
- [ ] **Transpiler pipeline** — configurable pass manager: `[Unroll3q, Decompose, Optimize1q, Route, Optimize1q]`; each pass = trait impl
- [ ] **Scheduling** — ALAP/ASAP gate scheduling with gate duration awareness; minimize idle qubit decoherence
- [ ] **Backend-specific transpile** — auto-transpile circuit for target backend's native gates + topology on `execute_circuit()`

### 9D — Quantum Syscalls + Kernel Integration
_Kernel-mediated access to quantum resources — simulator or real hardware, capability-controlled._

- [ ] **`SYS_Q_ALLOC` syscall** — allocate N qubits from QPU/simulator; returns qubit handle base; requires `Quantum` capability (8A)
- [ ] **`SYS_Q_GATE` syscall** — apply a gate: `syscall3(SYS_Q_GATE, gate_id, qubit0, qubit1)`
- [ ] **`SYS_Q_MEASURE` syscall** — measure a qubit, collapse state, return classical bit; supports mid-circuit measurement
- [ ] **`SYS_Q_CIRCUIT_SUBMIT` syscall** — submit `Circuit` buffer for batch execution; returns job handle
- [ ] **`SYS_Q_RESULT` syscall** — poll/retrieve results of submitted circuit job (async-compatible with Phase 6F poll)
- [ ] **`SYS_Q_RESET` syscall** — release qubits, reset simulator/QPU state, free resources
- [ ] **`SYS_Q_STATUS` syscall** — query backend: qubit count, type, error rates, queue depth, calibration age
- [ ] **`SYS_Q_TRANSPILE` syscall** — server-side transpile a circuit for a specific backend (useful for constrained clients)
- [ ] **Quantum resource capability** — `ResourceKind::Quantum(qpu_id)` with `ALLOCATE`, `EXECUTE`, `TRANSPILE`, `ADMIN` rights
- [ ] **Scheduler integration** — circuit execution blocks task (`Blocked(QpuWait)`); poll-compatible for async quantum workflows
- [ ] **Multi-tenant isolation** — per-process qubit namespaces; process A's qubits invisible to process B; enforced by capability system

### 9E — QPU Hardware + Cloud Backends
_Extensible backend system — local coprocessors, FPGA emulators, and cloud quantum services._

#### Local Hardware Backends
- [ ] **SPI/I2C QPU interface** — generic driver for QPU coprocessor attached via SPI/I2C (command/response protocol, CRC-protected)
- [ ] **QPU command protocol** — serialize `Circuit` → binary frame; deserialize results; versioned protocol for forward compatibility
- [ ] **QPU calibration service** — periodic calibration reads: T1/T2, gate fidelities, readout error rates; cache in kernel, expire after configurable TTL
- [ ] **FPGA quantum emulator** — interface to FPGA-based tensor/state-vector accelerator (iCE40 / ECP5 / Xilinx) over SPI; offload heavy simulation from CPU
- [ ] **Hot-swap backend** — runtime backend switching without recompile; `SYS_Q_STATUS` reports active backend; switchable via admin capability

#### Cloud QPU Backends (`quantum-cloud` feature)
- [ ] **Cloud backend trait** — `CloudQPU: QuantumBackend` — submits circuits over network, polls for results, handles queue/priority
- [ ] **IBM Quantum bridge** — REST API client (Qiskit Runtime compatible); circuit → OpenQASM 3.0 → HTTP POST; TLS 1.3 (uses 8E)
- [ ] **Amazon Braket bridge** — submit circuits to AWS managed QPUs (IonQ, Rigetti, OQC) via Braket API
- [ ] **Azure Quantum bridge** — submit to Azure Quantum (Quantinuum, IonQ, Pasqal) via REST
- [ ] **Google Quantum bridge (future)** — Cirq-compatible circuit submission to Google Sycamore/Willow processors
- [ ] **Job queue + retry** — cloud jobs are async; kernel tracks job IDs, polls status, retries on transient failures
- [ ] **Cost awareness** — cloud backends report estimated cost (QPU-seconds) before execution; user can set budget caps
- [ ] **Result caching** — cache measurement histograms for identical circuits (deterministic backends); saves cloud QPU cost

### 9F — Quantum Error Correction (QEC)
_Prepare for fault-tolerant quantum computing — error correction codes, syndrome decoding, logical qubits._

- [ ] **`LogicalQubit` type** — represents an error-corrected logical qubit composed of multiple physical qubits
- [ ] **Repetition code** — simplest QEC: 3-physical-qubits per logical qubit, majority-vote decoding; pedagogical starting point
- [ ] **Steane [[7,1,3]] code** — 7 physical → 1 logical qubit; implements transversal H, S, CNOT on logical level
- [ ] **Surface code (stubs)** — `d × d` lattice; syndrome extraction circuit generation; minimum-weight perfect matching (MWPM) decoder stub
- [ ] **Syndrome measurement** — ancilla qubit measurement circuits; extract error syndromes without collapsing data qubits
- [ ] **Decoder trait** — `Decoder::decode(syndrome) -> Correction`; implementations: lookup table (small codes), MWPM (surface code)
- [ ] **Logical gate operations** — apply gates at logical qubit level; compiler translates to physical-level circuit + error correction rounds
- [ ] **QEC overhead tracker** — report physical-to-logical qubit ratio, correction success rate, logical error rate per round

### 9G — Quantum-Classical Hybrid Workflows
_Variational algorithms, optimization loops, and hybrid quantum-classical computing — the dominant NISQ-era paradigm._

- [ ] **Parameterized circuits** — `Circuit` with `Parameter` placeholders; bind concrete values at execution time (avoids retranspilation)
- [ ] **VQE (Variational Quantum Eigensolver)** — kernel loop: prepare trial state → measure energy → classical optimizer → update parameters; built-in example for H2 molecule
- [ ] **QAOA (Quantum Approximate Optimization)** — alternating problem/mixer layers; included example for MaxCut on small graphs
- [ ] **Classical optimizer interface** — `Optimizer` trait: `minimize(f, x0) -> x_min`; built-in: gradient-free Nelder-Mead, COBYLA; `no_alloc` with fixed-size working set
- [ ] **Parameter shift rule** — compute gradients of quantum circuits by evaluating at shifted parameters; enables gradient-based optimizers
- [ ] **Expectation value estimation** — run circuit multiple shots, compute ⟨ψ|H|ψ⟩ from measurement statistics for Pauli Hamiltonians
- [ ] **Quantum machine learning (stubs)** — parameterized quantum circuits as ML model layers; forward/backward pass via parameter shift; integrates with Phase 10 AI

### 9H — Userlib Quantum API
_Ergonomic Rust API for quantum programming from userspace._

- [ ] **`userlib::quantum` module** — `alloc_qubits(n)`, `h(q)`, `cnot(q0,q1)`, `rz(q, angle)`, `measure(q)`, `reset()`; wraps syscalls
- [ ] **`QuantumCircuit` builder** — `let c = QuantumCircuit::new(4).h(0).cnot(0,1).rz(1, PI/4).measure_all(); c.submit()`
- [ ] **Async quantum execution** — `async fn run_circuit(c) -> Result<ClassicalRegister>` — integrates with Phase 6F async runtime
- [ ] **Bell state example** — `h(q0); cnot(q0,q1); measure(q0); measure(q1)` — entanglement demo, verify 50/50 correlated outcomes
- [ ] **GHZ state example** — N-qubit maximally entangled state: `h(q0); cnot(q0,q1); ... cnot(q0,qN)` — scalability test
- [ ] **Grover search example** — oracle + diffusion for N=4 (2 qubits) and N=16 (4 qubits); demonstrate quadratic speedup
- [ ] **Quantum teleportation** — 3-qubit protocol: entanglement, Bell measurement, conditional correction
- [ ] **Deutsch-Jozsa example** — determine constant vs balanced oracle in 1 query (vs N/2+1 classical)
- [ ] **Bernstein-Vazirani example** — find hidden bit-string in 1 query
- [ ] **VQE example** — variational H2 ground state energy estimation with classical optimizer loop
- [ ] **OpenQASM import** — `QuantumCircuit::from_qasm("OPENQASM 3.0; ...")` — load circuits from standard format
- [ ] **Shell `quantum` command** — interactive quantum REPL: `q> alloc 2`, `q> h 0`, `q> cnot 0 1`, `q> measure`, `q> status`, `q> backend`
- [ ] **Shell `qsim` command** — run built-in demos: `qsim bell`, `qsim ghz 5`, `qsim grover 4`, `qsim vqe`; display histograms + statistics
- [ ] **Shell `qasm` command** — load and execute OpenQASM files: `qasm run circuit.qasm --shots 1024`

## Phase 10 — AI as First-Class OS Citizen
_Machine learning inference, neural processing unit abstraction, and AI-assisted OS services — built into the kernel as a core capability, not a userspace afterthought. Feature-gated: `ai` (core traits + tiny inference), `ai-npu` (hardware accelerator), `ai-cloud` (cloud inference), `ai-os` (AI-enhanced kernel services), `ai-nlp` (natural language shell), `ai-agents` (autonomous agents), `ai-vision` (computer vision pipeline), `ai-voice` (speech I/O)._

### 10A — Neural Processing Abstraction Layer (`crates/ai/`)
_Unified interface for inference across CPU, GPU compute, NPU, TPU, and cloud endpoints._

- [ ] **`ai` crate** — `no_std`, `no_alloc` core types and traits; zero cost when `ai` feature disabled
- [ ] **`Tensor` type** — fixed-size N-dimensional array: `Tensor<T, SHAPE>` where `T: TensorElement` (f32, f16, i8, u8); shape known at compile time or runtime (small-vec backed)
- [ ] **`TensorElement` trait** — `f32` (baseline), `f16`/`bf16` (half-precision), `i8`/`u8` (quantized); conversion between types
- [ ] **`Model` trait** — `load(data: &[u8]) -> Result<Self>`, `predict(input: &Tensor) -> Result<Tensor>`, `info() -> ModelInfo`
- [ ] **`ModelInfo` struct** — `name`, `format` (ONNX/TFLite/GGUF/custom), `input_shape`, `output_shape`, `param_count`, `quant_type`, `memory_required`
- [ ] **`InferenceBackend` trait** — `run(model, input) -> Result<Tensor>`; implementations: CPU, NPU, GPU-compute, cloud
- [ ] **`BackendSelector`** — auto-select best backend based on model size, available hardware, latency budget; fallback chain: NPU → GPU → CPU → cloud
- [ ] **Operator registry** — extensible op table: `Conv2D`, `MatMul`, `ReLU`, `Softmax`, `LayerNorm`, `Attention`, `Embedding`, `Tokenizer`; new ops added via trait impl
- [ ] **Memory-mapped model loading** — models stored in flash/SD, memory-mapped into process address space; zero-copy weight access on MMU targets

### 10B — Tiny Inference Engine (On-Device)
_Optimized inference runtime for microcontrollers and embedded Linux-class targets. Runs models from 10 KB (sensor anomaly) to 100 MB+ (LLMs on RPi 5)._

#### Core Inference Runtime
- [ ] **TFLite Micro integration** — parse TFLite FlatBuffer model format; execute quantized INT8/UINT8 models; arena-based memory (no heap alloc during inference)
- [ ] **ONNX Runtime Micro** — minimal ONNX model parser; topological-sort operator execution; INT8 + FP32 kernels
- [ ] **GGUF model format** — parse GGUF (llama.cpp format) for LLM weight loading; quantization: Q4_0, Q4_K_M, Q5_K_M, Q8_0
- [ ] **Operator kernels (CPU)** — optimized `no_std` implementations:
  - MatMul: tiled, SIMD-optimized (NEON on ARM64, auto-vectorized on rv32imc)
  - Conv2D: im2col + MatMul, depthwise-separable fast path
  - Attention: scaled dot-product attention, KV-cache for autoregressive LLMs
  - Activation: ReLU, GELU, SiLU/Swish, Sigmoid, Softmax
  - Normalization: LayerNorm, RMSNorm, BatchNorm
  - Pooling: MaxPool, AvgPool, GlobalAvgPool
  - Tokenizer: BPE/SentencePiece decoder for text generation
- [ ] **Quantization support** — INT8 symmetric/asymmetric, INT4 (GPTQ/AWQ-style), dynamic quantization; per-tensor and per-channel scales
- [ ] **KV-cache management** — fixed-size key-value cache for autoregressive text generation; ring buffer eviction for long contexts
- [ ] **Streaming inference** — token-by-token LLM generation with yield between tokens (cooperative with scheduler)

#### Model Zoo (Built-in / Reference)
- [ ] **Keyword spotter (20 KB)** — tiny CNN for wake-word detection; runs on ESP32-C6 (< 5ms inference)
- [ ] **Anomaly detector (10 KB)** — autoencoder for sensor anomaly detection; runs on any target
- [ ] **Image classifier (200 KB)** — MobileNet-v2 INT8 for simple vision tasks; runs on RPi Zero 2 W+
- [ ] **TinyLlama / SmolLM (< 500 MB)** — Q4-quantized small LLM for on-device text generation; runs on RPi 4/5 (2+ GB RAM)
- [ ] **Phi-3-mini (2 GB Q4)** — Microsoft's efficient SLM; runs on RPi 5 (4 GB+) for local AI assistant
- [ ] **Whisper-tiny (75 MB)** — speech-to-text; audio input → text tokens; runs on RPi 4/5

### 10C — Hardware Accelerator Backends
_NPU, GPU compute, and specialized AI silicon drivers._

- [ ] **NPU abstraction trait** — `NpuBackend: InferenceBackend`; common interface for all neural accelerators
- [ ] **RPi 5 VideoCore VII GPU compute** — leverage GPU shader cores for matrix multiplication; dispatched via mailbox
- [ ] **ESP32-S3 vector extensions** — PIE (Processor Instruction Extensions) for 128-bit SIMD dot products; accelerates INT8 inference
- [ ] **Coral Edge TPU (USB)** — Google Coral accelerator via USB host; INT8 TFLite models delegated to TPU; RPi 4/5 via USB 3.0
- [ ] **Intel Movidius / Hailo-8 (future)** — USB/M.2 AI accelerators; RPi 5 M.2 HAT+ for Hailo-8 (13 TOPS INT8)
- [ ] **RISC-V vector extensions (future)** — RVV 1.0 SIMD for matrix ops on rv64gc targets with V extension
- [ ] **DMA-accelerated data movement** — zero-copy tensor transfer between CPU and accelerator memory; DMA descriptors for bulk weight loading

### 10D — AI Syscalls + Kernel Integration
_Kernel-mediated AI inference — resource-controlled, capability-gated, scheduler-aware._

- [ ] **`SYS_AI_MODEL_LOAD` syscall** — load model from memory region into inference engine; returns model handle; capability-gated `ResourceKind::AiModel(model_id)`
- [ ] **`SYS_AI_PREDICT` syscall** — run inference: `syscall3(SYS_AI_PREDICT, model_handle, input_ptr, output_ptr)`; blocks until complete
- [ ] **`SYS_AI_PREDICT_ASYNC` syscall** — submit inference job, return immediately with job handle; poll via Phase 6F poll subsystem
- [ ] **`SYS_AI_MODEL_INFO` syscall** — query model metadata: input/output shapes, param count, backend, estimated latency
- [ ] **`SYS_AI_MODEL_UNLOAD` syscall** — release model and associated memory
- [ ] **`SYS_AI_STATUS` syscall** — query AI subsystem: available backends, memory usage, active models, pending jobs
- [ ] **`SYS_AI_GENERATE` syscall** — LLM token generation: submit prompt, receive token stream (integrates with channels 6E for streaming)
- [ ] **Inference scheduling** — AI jobs have priority + deadline; kernel schedules inference between real-time tasks; preemptible long-running inference
- [ ] **Memory budgeting** — per-process AI memory quota; large models require explicit memory capability; OOM triggers graceful degradation (cloud fallback)
- [ ] **AI capability** — `ResourceKind::AiModel(id)` + `ResourceKind::AiAccelerator(backend_id)`; `LOAD`, `EXECUTE`, `ADMIN` rights

### 10E — AI-Enhanced OS Services (`ai-os` feature)
_The OS itself uses AI to improve scheduling, security, and user experience. The AI is the kernel's copilot._

#### Predictive Scheduling
- [ ] **Task behavior model** — tiny RNN/MLP (< 5 KB) predicts task CPU burst length, sleep duration, IPC patterns from recent history
- [ ] **Predictive priority adjustment** — scheduler uses model predictions to pre-boost tasks about to become I/O-ready; reduces latency
- [ ] **Power-aware scheduling** — model predicts idle periods; proactively enters low-power states; wake-up prediction reduces resume latency
- [ ] **Adaptive time-slice tuning** — AI adjusts scheduler time quantum per-task based on workload classification (interactive vs batch vs real-time)
- [ ] **Thermal-aware task placement (SMP)** — on multi-core targets, model predicts per-core thermal trajectory; migrate hot tasks to cooler cores before throttling

#### Anomaly Detection + Security
- [ ] **Syscall anomaly detector** — per-process syscall sequence model (Markov chain or tiny LSTM); flags unusual patterns → security audit log (8F)
- [ ] **Memory access anomaly** — detect unusual memory access patterns that may indicate exploitation; raise `IntegrityViolation` event
- [ ] **Network traffic classifier** — classify inbound packets (benign/suspicious/malicious) using tiny CNN on packet headers; integrates with firewall rules
- [ ] **Behavioral process fingerprinting** — learn normal syscall/IPC/memory patterns per process; detect compromised processes deviating from profile
- [ ] **AI-powered intrusion detection** — correlate anomalies across syscall, network, and memory domains; generate threat score; auto-quarantine above threshold
- [ ] **Adversarial robustness** — model hardening against evasion attacks; input validation before inference; rate-limit anomaly detector updates

#### Sensor Fusion + IoT Intelligence
- [ ] **Sensor pipeline** — raw sensor data → preprocessing → inference → action; declarative configuration: `{sensor: "temp", model: "anomaly", action: "alert"}`
- [ ] **Edge inference orchestrator** — fleet of VeerOS devices coordinate inference: split model across nodes, aggregate results
- [ ] **Federated learning (stubs)** — on-device model training with gradient sharing; no raw data leaves the device; privacy-preserving AI
- [ ] **Predictive maintenance** — learn sensor baselines, predict hardware failure (fan, motor, battery) before it happens; alert via IPC/network
- [ ] **Time-series forecasting** — tiny temporal model for sensor prediction: temperature, vibration, power consumption; enables proactive control loops

#### Auto-Tuning + Self-Optimizing Kernel
- [ ] **Memory allocator tuning** — AI-selected allocation strategy per-workload (bump vs slab vs buddy); learned from allocation pattern history
- [ ] **I/O scheduler optimization** — model predicts disk/flash access patterns; reorder and coalesce block I/O requests; reduce latency and wear
- [ ] **Network stack tuning** — auto-tune TCP window size, retransmit timers, buffer counts based on observed RTT and throughput
- [ ] **Self-healing kernel** — detect repeated service crashes, auto-restart with different configuration; learn stable config over time

### 10F — Cloud AI Integration (`ai-cloud` feature)
_Offload heavy inference to cloud when local resources are insufficient._

- [ ] **Cloud inference trait** — `CloudAI: InferenceBackend`; abstracts provider-specific APIs
- [ ] **OpenAI-compatible API client** — HTTP POST to `/v1/chat/completions`; streaming token reception via SSE; TLS 1.3 (uses 8E)
- [ ] **Ollama bridge** — connect to local/remote Ollama instance for self-hosted LLM inference
- [ ] **Anthropic / Google / AWS Bedrock bridges** — pluggable cloud LLM providers
- [ ] **Hybrid inference** — small model runs locally (first-pass), escalate to cloud for complex queries; latency-aware routing
- [ ] **Token budget management** — per-process cloud AI token quota; kernel tracks usage, denies when budget exhausted
- [ ] **Offline fallback** — graceful degradation when cloud unreachable: use smaller local model, queue requests, or return cached responses
- [ ] **Privacy controls** — per-process policy: `local-only` (never send data to cloud), `cloud-ok` (user consented), `anonymized` (strip PII before sending)

### 10G — Userlib AI API
_Ergonomic Rust API for AI inference from userspace._

- [ ] **`userlib::ai` module** — `load_model(data)`, `predict(model, input)`, `predict_async(model, input)`, `generate(model, prompt)`; wraps syscalls
- [ ] **`TensorView` / `TensorMut`** — zero-copy views into user-allocated tensor buffers; type-safe shape checking
- [ ] **Streaming text generation** — `for token in model.generate_stream(prompt) { print!("{}", token); }` — channel-based token streaming
- [ ] **Image classification example** — load MobileNet, inference on test image, print top-5 classes
- [ ] **Keyword detection example** — continuous audio monitoring, trigger action on wake word
- [ ] **Anomaly detection example** — feed sensor readings, flag outliers
- [ ] **LLM chat example** — interactive conversation with on-device or cloud LLM
- [ ] **Shell `ai` command** — `ai predict <model> <input>`, `ai chat`, `ai status`, `ai models`
- [ ] **Shell `ai chat` command** — interactive LLM conversation: `ai chat --model phi3-mini --local` or `ai chat --cloud`

### 10H — AI Feature Integration Matrix
_How AI tiers map to features. For profile → AI tier mapping and per-target AI capabilities, see Phase 4D and 4G._

```
                     ai (core)  ai-npu    ai-cloud   ai-os
                     (feature)  (feature) (feature)  (feature)
────────────────────────────────────────────────────────────────
Tensor types           ✓          ✓          ✓          ✓
CPU inference          ✓          ✓          ✓          ✓
INT8 quantized         ✓          ✓          ✓          ✓
NPU/GPU offload        ─          ✓          ─          ✓
Cloud inference        ─          ─          ✓          ✓
LLM generation         ✓*         ✓          ✓          ✓
Predictive sched       ─          ─          ─          ✓
Anomaly detection      ─          ─          ─          ✓
Smart shell            ─          ─          ✓          ✓
────────────────────────────────────────────────────────────────
* LLM on CPU requires sufficient RAM (≥ 2 GB for Q4 models)

→ See Phase 4G for AI hardware capabilities per target.
→ See Phase 4D for AI tier defaults per distribution profile.
```

### 10I — Natural Language Shell (`ai-nlp` feature)
_The VeerOS shell understands natural language. Toggle with `ai on`/`ai off`. When active, plain English commands get translated to shell commands. When off, the shell behaves as a traditional CLI._

#### NL Shell Core
- [ ] **`ai on` / `ai off` toggle** — shell variable `ai_mode: bool`; persists in `ShellVars`; displayed in prompt (`veeros [AI]>` vs `veeros>`); default off
- [ ] **`ai` command** — `ai on` / `ai off` / `ai status` / `ai query <text>` / `ai explain <cmd>`; gateway to NL features
- [ ] **Intent classifier** — tiny model (< 50 KB) classifies NL input into intent categories: `FileOp`, `ProcessMgmt`, `NetworkCmd`, `SystemInfo`, `ConfigChange`, `Search`, `Help`, `Unknown`
- [ ] **Entity extraction** — parse file paths, process names, IP addresses, port numbers, flag values from NL input
- [ ] **NL → Command translator** — `"show me what's running"` → `ps`, `"list files in /tmp"` → `ls /tmp`, `"connect to wifi MyNet"` → `wifi connect MyNet`
- [ ] **Confirmation prompt** — before executing translated command, show: `→ ps [Y/n]?`; user confirms or edits; bypass with `ai! <text>` (force-execute)
- [ ] **Context-aware suggestions** — track recent commands; `"do that again but for /var"` → re-run last command with path substituted
- [ ] **Conversational mode** — `ai chat` enters multi-turn conversation; context window of last 5 exchanges; `exit` returns to normal shell
- [ ] **`ai explain <cmd>`** — explain what a command does: `ai explain "mount /dev/sd0 /mnt fat32"` → human-readable explanation
- [ ] **Error recovery** — on command failure, AI suggests fix: `"Permission denied" → "Try: su root, then re-run"`
- [ ] **Safety guardrails** — NL commands that would destroy data (`rm -rf /`, `format`) require double confirmation; AI warns about destructive operations

#### NL Models + Backends
- [ ] **Local tiny NL model (< 5 MB)** — distilled intent classifier + entity extractor; runs on ESP32-S3+ / RPi; no network required
- [ ] **Cloud LLM backend** — for complex queries, escalate to cloud LLM (OpenAI/Anthropic/Ollama); requires `ai-cloud` feature + network
- [ ] **Hybrid pipeline** — local intent classification first; if confidence < threshold, escalate to cloud; minimizes latency and cost
- [ ] **Custom training data** — VeerOS-specific command corpus; fine-tune on OS commands, man pages, system concepts
- [ ] **Offline command dictionary** — fallback: keyword→command lookup table (50+ common patterns) when no model/cloud available

### 10J — AI Agents (`ai-agents` feature)
_Autonomous task execution — AI agents that can plan, execute multi-step operations, and recover from failures._

- [ ] **Agent framework** — `Agent` struct: goal description, action plan (sequence of shell commands), execution state, rollback plan
- [ ] **`ai agent <goal>`** — describe a goal in plain text; agent decomposes into steps, executes sequentially, reports progress
- [ ] **Tool use** — agents can invoke shell commands, read files, query system status, call other agents; sandboxed via capabilities (8A)
- [ ] **Planning engine** — LLM-based or rule-based planner; generate action plan from goal + context; re-plan on failure
- [ ] **Execution sandbox** — agents run in restricted domain (8B sandbox); limited syscall set; memory/CPU budget
- [ ] **Rollback on failure** — agent tracks modifications; on error, attempts undo (delete created files, restart stopped services)
- [ ] **Agent registry** — pre-built agents: `setup-wifi` (configure and connect), `deploy-app` (load and start process), `diagnose-network` (troubleshoot connectivity), `optimize-system` (tune kernel parameters)
- [ ] **Human-in-the-loop** — agents pause at dangerous operations and request user approval; `--auto` flag for fully autonomous mode
- [ ] **Multi-agent coordination** — agents can delegate sub-tasks to specialist agents; shared context via IPC channels
- [ ] **Agent audit log** — all agent actions logged to security audit (8F); reviewable via `ai agent log`

### 10K — Computer Vision Pipeline (`ai-vision` feature)
_Camera input → inference → action. Integrated with ESP32-S3 LCD_CAM and RPi camera modules._

- [ ] **Frame capture abstraction** — `CameraDevice` trait: `capture_frame() -> FrameBuffer`; implementations for ESP32-S3 DVP, RPi CSI-2
- [ ] **Frame preprocessing** — resize, crop, normalize, color space conversion (RGB→grayscale, YUV→RGB); fixed-point math
- [ ] **Image classification** — MobileNet-v2 INT8 on captured frames; top-K class labels
- [ ] **Object detection** — YOLO-tiny / SSD-MobileNet for bounding box detection; real-time on RPi 4/5
- [ ] **Face detection** — lightweight face detector for presence/count sensing; no recognition (privacy)
- [ ] **OCR (basic)** — character recognition for reading displays, labels, signs; 7-segment + printed text
- [ ] **Motion detection** — frame differencing for security/trigger applications; zero-model, pure image processing
- [ ] **Vision pipeline config** — declarative: `{camera: "csi0", model: "mobilenet", action: "classify", interval_ms: 1000}`
- [ ] **`/dev/camera`** — device node for frame capture; `read()` returns latest frame; `ioctl` for resolution/format config
- [ ] **Shell `vision` command** — `vision capture` (save frame), `vision classify` (run model), `vision detect` (object detection), `vision stream` (continuous)
- [ ] **Vision → network** — stream classification results via MQTT/TCP for remote monitoring dashboards

### 10L — Voice / Speech I/O (`ai-voice` feature)
_Microphone input → speech recognition → command execution. Speaker output → text-to-speech._

- [ ] **Audio capture abstraction** — `AudioDevice` trait: `read_samples(buf, count)`, `sample_rate()`, `channels()`; I2S/PDM microphone drivers
- [ ] **I2S driver** — ESP32-S3 I2S peripheral for digital microphone (INMP441, SPH0645); RPi I2S for USB audio class
- [ ] **Voice Activity Detection (VAD)** — energy-based + tiny model VAD; detect speech onset/offset; avoid processing silence
- [ ] **Wake word detection** — always-on keyword spotter (< 20 KB model); `"Hey Veer"` triggers speech capture; runs on ESP32-C6+
- [ ] **Speech-to-Text (STT)** — Whisper-tiny (75 MB) on RPi 4/5 for on-device transcription; cloud STT fallback
- [ ] **Text-to-Speech (TTS)** — tiny TTS model for spoken feedback; I2S/speaker output; `say "WiFi connected"`
- [ ] **Voice command pipeline** — wake word → capture → STT → NL shell (10I) → execute → TTS response
- [ ] **`/dev/mic`** — device node for audio capture; `/dev/speaker` for audio output
- [ ] **Shell `voice` command** — `voice listen` (start capturing), `voice say <text>`, `voice status`
- [ ] **Hands-free mode** — continuous voice command loop: listen → transcribe → execute → speak result → listen

### 10M — On-Device Training / Fine-Tuning (`ai-train` feature)
_Learn and adapt on the device itself — federated learning, transfer learning, continual adaptation._

- [ ] **Gradient computation** — backpropagation through small networks (< 100K params); fixed-point gradients on embedded
- [ ] **SGD optimizer** — stochastic gradient descent with momentum; `no_alloc`, fixed working set
- [ ] **Transfer learning** — freeze pre-trained feature layers, fine-tune classifier head on device-specific data
- [ ] **Federated learning framework** — on-device training → encrypted gradient upload → aggregation server → updated model download; privacy-preserving
- [ ] **Continual learning** — model adapts to changing data distribution over time; catastrophic forgetting mitigation via EWC/replay buffer
- [ ] **Training data collection** — sensor data + labels stored in flash/SD; incremental dataset building
- [ ] **Model export** — save fine-tuned model to flash in TFLite/ONNX/GGUF format; hot-swap without reboot
- [ ] **Training scheduler** — kernel-aware: run training during idle periods; pause when real-time tasks need CPU; battery-aware on portable devices

### 10N — Retrieval-Augmented Generation (RAG) for VeerOS
_LLM + local knowledge base. AI assistant that knows about YOUR VeerOS instance — its configuration, logs, man pages, and documentation._

- [ ] **Document index** — index man pages, `/etc/` config files, kernel log, command history into vector store
- [ ] **Embedding model** — tiny sentence embedding model (< 10 MB) for semantic search; INT8 quantized
- [ ] **Vector store** — fixed-size vector database in RAM/PSRAM (cosine similarity search); `heapless::Vec` backed
- [ ] **RAG pipeline** — user query → embed → top-K retrieval → augment LLM prompt with context → generate response
- [ ] **Auto-indexing** — on boot, index system docs; on config change, re-index affected files; incremental
- [ ] **`ai ask <question>`** — RAG-powered Q&A: `ai ask "how do I mount an SD card?"` → retrieves man page + example, generates answer
- [ ] **Context injection** — system status (uptime, memory, running tasks) automatically injected into LLM context for aware responses

## Phase 11 — Distributed OS (VeerOS Cluster)
_Multiple VeerOS nodes form a single coherent computer. Processes can spawn on any node, IPC crosses node boundaries transparently, and a unified VFS presents all nodes' storage as one namespace. Feature-gated: `cluster` (core membership + discovery), `cluster-sched` (distributed scheduler), `cluster-vfs` (shared filesystem), `cluster-ipc` (cross-node channels)._

### 11A — Cluster Membership + Discovery
_Node discovery, health monitoring, and membership management._

- [ ] **Node identity** — `NodeId` (128-bit UUID, derived from MAC or hardware serial); `NodeInfo { id, hostname, arch, capabilities, memory, cores, ip_addr, uptime }`
- [ ] **mDNS/DNS-SD discovery** — broadcast `_veeros._tcp.local` service; auto-discover peers on LAN; zero-config clustering
- [ ] **Gossip protocol** — SWIM-based (Scalable Weakly-consistent Infection-style Membership); heartbeat + suspicion + death detection; O(log N) convergence
- [ ] **Cluster join/leave** — `cluster join <addr>` / `cluster leave`; graceful drain (migrate tasks) before leave; forced eviction on unresponsive nodes
- [ ] **Membership table** — `[NodeEntry; MAX_CLUSTER_NODES]` (32–256 nodes); replicated across all members via gossip
- [ ] **Health monitoring** — periodic heartbeat (1s); suspicion timer (5s); dead declaration (15s); configurable per-cluster
- [ ] **Split-brain detection** — partition detection via quorum; minority partition enters read-only mode; auto-heal on reconnect
- [ ] **Node roles** — `Leader` (elected, coordinates), `Worker` (runs tasks), `Gateway` (ingress/egress), `Storage` (persistent data); configurable per-node
- [ ] **Shell `cluster` commands** — `cluster status`, `cluster nodes`, `cluster join <addr>`, `cluster leave`, `cluster elect`, `cluster drain <node>`
- [ ] **Bootstrap modes** — static seeds (predefined IP list), mDNS auto-discovery, cloud seed (fetch peers from a registry endpoint)

### 11B — Consensus + Coordination
_Distributed agreement for leader election, configuration updates, and atomic operations._

- [ ] **Raft consensus** — `no_std` Raft implementation: leader election, log replication, commit; persistent log in flash/SD; 3/5/7-node quorum
- [ ] **Leader election** — automatic leader election on cluster formation or leader failure; election timeout + randomized backoff
- [ ] **Distributed configuration** — cluster-wide key-value store (Raft-replicated); `/etc/cluster.conf` synced across all nodes
- [ ] **Distributed locks** — `SYS_CLUSTER_LOCK` / `SYS_CLUSTER_UNLOCK` — cross-node mutex via Raft; fencing tokens for correctness
- [ ] **Atomic counters** — cluster-wide monotonic counters (useful for distributed IDs, sequence numbers)
- [ ] **Etcd-compatible API (stubs)** — basic key-value watch/put/get compatible with etcd wire protocol for tooling interop

### 11C — Distributed Scheduler
_Transparent process migration and placement — the cluster acts as one big computer._

- [ ] **Global task registry** — leader maintains cluster-wide task table; each node reports local tasks via gossip
- [ ] **`SYS_REMOTE_SPAWN`** — spawn a process on a specific node or let scheduler pick: `spawn_remote(binary, args, node_hint)`
- [ ] **Placement policies** — round-robin, least-loaded, affinity-based (pin to node with required hardware), anti-affinity (spread replicas)
- [ ] **Resource-aware placement** — scheduler considers CPU, memory, accelerators (NPU, radio), network proximity
- [ ] **Task migration** — checkpoint process state → serialize → transfer to target node → resume; requires architecture compatibility
- [ ] **Cross-node process visibility** — `ps` shows all processes across cluster with `[node]` prefix; `kill` works across nodes
- [ ] **Resource quotas** — per-node and per-user cluster-wide resource limits; prevents single user from consuming all cluster resources
- [ ] **Scheduler plugins** — pluggable scheduling strategies: `SchedulerPlugin` trait; custom placement logic for domain-specific workloads

### 11D — Distributed IPC
_Transparent cross-node message passing — channels and sockets work identically whether local or remote._

- [ ] **Cluster-aware channels** — `SYS_CHAN_CREATE` with `CHAN_FLAG_CLUSTER` flag; kernel routes messages across nodes transparently
- [ ] **Cross-node message transport** — TCP/TLS between nodes for reliable message delivery; UDP for low-latency unreliable
- [ ] **Location-transparent addressing** — `NodeId:ChannelId` globally unique; sender doesn't need to know receiver's node
- [ ] **Cluster sockets** — sockets can `connect()` to `NodeId:Port`; kernel routes to correct node
- [ ] **Service ports** — well-known cluster-wide service names: `"cluster://log-service"`, `"cluster://config-store"`; resolved via membership table
- [ ] **Message serialization** — compact binary serialization for cross-node messages; version-tagged for compatibility
- [ ] **Backpressure** — flow control between nodes; sender blocks when receiver overwhelmed; prevents cascading failures
- [ ] **Encrypted cross-node IPC** — all inter-node communication encrypted with per-link session keys (TLS 1.3, Phase 8E)

### 11E — Distributed VFS
_One filesystem namespace spanning all cluster nodes — files accessible from any node._

- [ ] **Global namespace** — `/cluster/<node>/` mount points auto-created; root node mounts all peers: `/cluster/node2/`, `/cluster/node3/`
- [ ] **Remote file operations** — `open("/cluster/node2/data/file.txt")` transparently routes I/O over network to node2
- [ ] **NFS-like protocol** — lightweight RPC for file ops (open/read/write/stat/readdir/close); runs over cluster IPC
- [ ] **Caching** — local read cache with TTL; write-through for consistency; cache invalidation via gossip
- [ ] **Replicated directories** — mark directories for N-way replication across nodes; write quorum for durability
- [ ] **Path-based routing** — `/local/` always stays on current node; `/cluster/` routes to remote; `/shared/` = replicated
- [ ] **Consistency levels** — configurable per-file: `strong` (linearizable), `eventual` (AP), `session` (read-your-writes)
- [ ] **Storage pooling** — aggregate free space across nodes; distributed block allocator for large files spanning multiple nodes

### 11F — Cluster Observability + Management
_Monitoring, debugging, and operating the cluster._

- [ ] **Cluster metrics** — per-node: CPU, memory, network, disk, task count; aggregated at leader; exposed via `/proc/cluster/`
- [ ] **Distributed logging** — `klog` entries tagged with `NodeId`; aggregated at leader or forwarded to external log collector
- [ ] **`cluster top`** — `top`-like view across all nodes: global process list, per-node resource usage, network I/O
- [ ] **Node drain + cordon** — `cluster drain <node>` migrates all tasks; `cluster cordon <node>` prevents new task placement
- [ ] **Rolling restart** — restart nodes one-by-one without downtime; drain → restart → rejoin → uncordon
- [ ] **Cluster events** — event stream: node_joined, node_left, task_migrated, leader_elected, split_brain; subscribe via channel

## Phase 12 — Cloud Platform (VeerOS Cloud)
_VeerOS as a cloud-native operating system with built-in orchestration, service mesh, and platform services. Not running ON the cloud — VeerOS IS the cloud. Feature-gated: `cloud-orchestrate`, `cloud-mesh`, `cloud-observe`, `cloud-api`._

### 12A — Container + Service Orchestration
_Schedule and manage containerized workloads across the cluster — a native, sidecar-free alternative to Kubernetes._

- [ ] **Service definition** — `ServiceSpec { name, image, replicas, resources, ports, env, health_check, restart_policy }`
- [ ] **Desired state reconciliation** — controller loop: compare desired state vs actual → schedule/kill/restart to converge; runs on leader
- [ ] **Service lifecycle** — create → scale → update (rolling) → pause → resume → destroy
- [ ] **Rolling deployments** — update containers one at a time; health check between steps; auto-rollback on failure
- [ ] **Replica placement** — spread replicas across nodes (anti-affinity); respect resource requests/limits
- [ ] **Restart policies** — `always`, `on-failure`, `never`; configurable backoff (1s, 2s, 4s, max 5m)
- [ ] **Resource requests + limits** — CPU, memory, accelerator quotas per service; scheduler enforces at placement time
- [ ] **Namespaces** — logical grouping of services; resource quotas per namespace; RBAC per namespace
- [ ] **Labels + selectors** — key-value labels on services/containers; selector-based queries for grouping/targeting
- [ ] **CronJob scheduler** — time-based service execution: `CronSpec { schedule: "*/5 * * * *", service }` ; Raft-replicated schedule

### 12B — Service Mesh (Native, Sidecar-Free)
_Service-to-service communication with built-in load balancing, retries, circuit breaking, and mTLS. No sidecar bloat — the kernel IS the mesh._

- [ ] **Service registry** — all services auto-registered at start; `{ name, node, port, health, metadata }`; gossip-replicated
- [ ] **Service discovery** — `connect("my-service")` resolves to healthy instance via registry; client-side or kernel-mediated
- [ ] **Load balancing** — round-robin, least-connections, weighted, random; per-service configurable; kernel routes at socket layer
- [ ] **Health checks** — TCP connect, HTTP GET, custom probe; configurable interval, timeout, threshold; unhealthy → removed from LB
- [ ] **Circuit breaker** — per-service failure counter; open circuit after N failures → fast-fail for timeout period → half-open probe → close
- [ ] **Retry policy** — automatic retries with backoff: `{ retries: 3, backoff: "exponential", max_delay_ms: 5000 }`
- [ ] **Timeout policy** — per-request timeout; per-service default; kernel enforces at socket/channel level
- [ ] **mTLS (mutual TLS)** — all service-to-service traffic encrypted; auto-provisioned per-service certificates (Phase 8E + 8C)
- [ ] **Rate limiting** — per-service request rate limits; token bucket algorithm; 429 response on overflow
- [ ] **Traffic splitting** — canary deployments: route X% traffic to new version; header-based routing for A/B testing
- [ ] **Observability injection** — auto-inject trace headers (W3C Trace Context); latency histograms per service pair

### 12C — API Gateway + Ingress
_External traffic entry point — routing, authentication, rate limiting, protocol translation._

- [ ] **Ingress controller** — listen on public ports (80/443); route inbound traffic to backend services by hostname/path
- [ ] **Route table** — `{ host: "api.example.com", path: "/v1/*", service: "api-svc", port: 8080 }`; regex path matching
- [ ] **TLS termination** — terminate TLS at ingress; forward plaintext to backend services (or re-encrypt for mTLS)
- [ ] **Authentication** — API key validation, JWT verification, OAuth2 token introspection at gateway level
- [ ] **Rate limiting** — per-client, per-route rate limits; API key-based quotas
- [ ] **Request/response transformation** — header injection, path rewriting, body transformation (JSON→CBOR for embedded clients)
- [ ] **WebSocket support** — HTTP upgrade → WebSocket pass-through to backend services
- [ ] **gRPC proxy** — HTTP/2 gRPC routing; content-type detection for automatic protocol handling
- [ ] **CORS handling** — configurable Cross-Origin Resource Sharing headers per route

### 12D — Secrets + Configuration Management
_Secure secret storage and dynamic configuration for cluster services._

- [ ] **Secret store** — encrypted key-value store (Raft-replicated); at-rest encryption via kernel keystore (8C); per-namespace access control
- [ ] **`SYS_SECRET_GET` / `SYS_SECRET_PUT`** — syscalls for secret access; requires `Secret(name)` capability
- [ ] **Secret injection** — services declare secret refs in spec; kernel injects into environment or mounted tmpfs at start
- [ ] **Secret rotation** — automatic key/cert rotation with configurable TTL; services notified via event channel
- [ ] **ConfigMap** — non-secret configuration data; Raft-replicated; mountable as virtual files in container namespace
- [ ] **Hot reload** — config changes trigger notification to running services; services can subscribe to config change events
- [ ] **Shell `secret` commands** — `secret create <name> <value>`, `secret get <name>`, `secret list`, `secret delete <name>`, `secret rotate <name>`

### 12E — Observability Stack
_Metrics, traces, and logs — built into the kernel, not bolted on._

#### Metrics
- [ ] **Kernel metrics collector** — per-service: request count, latency histogram (P50/P95/P99), error rate, active connections
- [ ] **System metrics** — CPU per-core, memory usage, network I/O, disk I/O, scheduler stats; sampled at 1s intervals
- [ ] **Prometheus exposition** — `/metrics` HTTP endpoint on each node; Prometheus-compatible text format for scraping
- [ ] **Push metrics** — UDP/TCP push to remote collector for environments without pull infrastructure
- [ ] **Custom metrics** — `SYS_METRIC_EMIT` syscall; services publish custom counters/gauges/histograms

#### Distributed Tracing
- [ ] **W3C Trace Context** — auto-propagate `traceparent`/`tracestate` headers across service calls; kernel injects at socket layer
- [ ] **Span collection** — per-request span: start time, duration, service, operation, status, parent span ID
- [ ] **Trace storage** — ring buffer of recent traces in kernel memory; configurable depth (1K–100K spans)
- [ ] **Jaeger/Zipkin export** — serialize traces to Jaeger Thrift or Zipkin JSON format; push to external collector
- [ ] **Trace query** — `trace list`, `trace show <trace_id>`; filter by service, latency, error status

#### Logging
- [ ] **Structured logging** — JSON log entries: `{ ts, level, node, service, msg, trace_id, fields }`
- [ ] **Log aggregation** — forward logs from all nodes to leader or external collector (syslog, Loki, Elasticsearch)
- [ ] **Log levels** — per-service configurable: `trace`, `debug`, `info`, `warn`, `error`; runtime-adjustable
- [ ] **Shell `logs` command** — `logs <service> [--tail N] [--follow] [--node <id>] [--level warn]`

#### Alerting
- [ ] **Alert rules** — threshold-based: `if error_rate > 0.05 for 5m → alert`; configurable per-service
- [ ] **Alert channels** — IPC notification, shell `alerts` command, network webhook (HTTP POST), GPIO (LED/buzzer on embedded)
- [ ] **Alert silencing** — `alert silence <rule> --duration 1h`; prevents alert storms during maintenance

### 12F — Auto-Scaling + Resource Management
_Dynamic scaling of services based on load._

- [ ] **Horizontal Pod Autoscaler (HPA) equivalent** — scale service replicas based on CPU/memory/custom metric thresholds
- [ ] **Scale-to-zero** — idle services scaled down to 0 replicas; re-created on first request (cold start ~100ms target)
- [ ] **Vertical scaling** — adjust resource limits dynamically based on observed usage; recommend right-size
- [ ] **Cluster autoscaler stubs** — for cloud/VM environments: provision/deprovision nodes based on pending workload
- [ ] **Resource pressure signals** — kernel signals `MemoryPressure`, `CpuPressure`, `DiskPressure` to orchestrator; triggers eviction/migration
- [ ] **Eviction policy** — low-priority services evicted first under resource pressure; priority-based preemption
- [ ] **Cost-aware scheduling** — on heterogeneous clusters (mix of RPi + x86), prefer cheaper nodes; spot-instance awareness for cloud

### 12G — Multi-Tenancy
_Isolate tenants sharing the same cluster._

- [ ] **Tenant model** — `TenantId` maps to a set of namespaces + resource quotas + RBAC policies
- [ ] **Network isolation** — per-tenant virtual network; no cross-tenant traffic without explicit policy
- [ ] **Storage isolation** — per-tenant VFS namespace; separate encryption keys per tenant
- [ ] **Resource quotas** — per-tenant CPU/memory/storage/network limits; enforced at scheduler and kernel level
- [ ] **Billing stubs** — resource usage tracking per-tenant; exportable for chargeback/billing integration
- [ ] **Tenant admin** — `tenant create <name>`, `tenant quota set <name> <resource> <limit>`, `tenant list`, `tenant delete <name>`

## Phase 13 — Network Appliance / Firewall OS (`dist-firewall`)
_VeerOS as a firewall, router, VPN gateway, and network security appliance. A dedicated distribution profile for network infrastructure. Feature-gated: `net-firewall`, `net-nat`, `net-vpn`, `net-dpi`, `net-shape`._

### 13A — Packet Filter + Firewall Engine
_Stateful packet filtering with rule chains — the core of VeerOS-as-firewall._

- [ ] **Packet filter engine** — per-interface rule chains: INPUT, OUTPUT, FORWARD; match on protocol, src/dst IP, port, interface, state
- [ ] **Rule structure** — `FilterRule { chain, priority, match_criteria, action: Accept|Drop|Reject|Log|Jump(chain), counter }`
- [ ] **Match criteria** — IP src/dst (CIDR), protocol (TCP/UDP/ICMP/any), port range, interface, connection state (NEW/ESTABLISHED/RELATED), rate limit
- [ ] **Stateful inspection** — connection tracking table (conntrack): track TCP state machine, UDP pseudo-connections, ICMP echo tracking; ESTABLISHED packets fast-path
- [ ] **Default policies** — per-chain default (ACCEPT/DROP); recommended: INPUT=DROP, OUTPUT=ACCEPT, FORWARD=DROP
- [ ] **Rule processing** — first-match wins; priority ordering; counters (packets/bytes) per rule for monitoring
- [ ] **IPv4 + IPv6** — dual-stack filtering; separate rule sets or unified with address family match
- [ ] **ebtables / bridge filtering (stubs)** — L2 frame filtering for bridged interfaces
- [ ] **Shell `fw` commands** — `fw add INPUT -s 10.0.0.0/8 -p tcp --dport 22 -j ACCEPT`, `fw list`, `fw delete <id>`, `fw flush`, `fw default INPUT DROP`
- [ ] **Rule persistence** — save rules to `/etc/firewall.rules`; auto-load on boot; `fw save` / `fw restore`
- [ ] **Logging** — dropped/rejected packets logged with timestamp, src/dst, protocol; rate-limited logging to prevent log flood

### 13B — NAT (Network Address Translation)
_IP masquerading, port forwarding, and DNAT/SNAT for routing between networks._

- [ ] **SNAT / Masquerade** — rewrite source IP for outbound traffic from private network; conntrack-based reply mapping
- [ ] **DNAT / Port forwarding** — rewrite destination IP:port to forward inbound connections to internal servers
- [ ] **1:1 NAT** — static bidirectional IP mapping for DMZ hosts
- [ ] **NAT table** — `NatRule { chain: PREROUTING|POSTROUTING, match, action: SNAT(ip)|DNAT(ip:port)|MASQUERADE }`
- [ ] **Hairpin NAT** — internal hosts access internal services via external IP; rewrite on both PREROUTING and POSTROUTING
- [ ] **Connection tracking integration** — NAT entries tied to conntrack; reply packets automatically de-NATted
- [ ] **Shell `nat` commands** — `nat add masquerade -o eth0`, `nat add dnat -p tcp --dport 80 --to 192.168.1.10:8080`, `nat list`
- [ ] **NAT ALG stubs** — Application Layer Gateway for protocols that embed IP/port in payload (FTP, SIP); basic FTP passive mode

### 13C — Routing + Multi-Interface
_IP routing between multiple network interfaces — VeerOS as a router._

- [ ] **Routing table** — `RouteEntry { destination: CIDR, gateway: Option<IP>, interface, metric, flags }`; longest-prefix match
- [ ] **Static routes** — `route add 10.0.0.0/8 via 192.168.1.1 dev eth0`; persistent in `/etc/routes`
- [ ] **Default gateway** — `route add default via 192.168.1.1`
- [ ] **Multi-interface support** — multiple `NetworkDevice` instances (eth0, wlan0, tun0, br0); independent IP config per interface
- [ ] **Interface management** — `ifconfig <iface> <ip> netmask <mask> up/down`; `ip addr add/del`
- [ ] **ARP table** — `arp` command; ARP request/reply handling per interface; proxy ARP for bridging
- [ ] **DHCP server** — lightweight DHCP server for LAN interfaces; IP pool, lease management, options (DNS, gateway, NTP)
- [ ] **DNS forwarder** — cache-and-forward DNS queries; configurable upstream servers; ad-block lists (optional)
- [ ] **Dynamic routing (stubs)** — OSPF/BGP protocol stubs for future enterprise routing; initially static only
- [ ] **Policy routing** — route based on source IP, port, or mark (for multi-WAN setups)

### 13D — VPN Gateway
_Encrypted tunnel endpoints — VeerOS as a VPN concentrator._

- [ ] **WireGuard** — native WireGuard implementation: Noise IK handshake, ChaCha20-Poly1305 data, Curve25519 keys; `tun` virtual interface
- [ ] **WireGuard config** — `wg set wg0 private-key <key> listen-port 51820 peer <pubkey> allowed-ips 10.0.0.0/24 endpoint <ip>:51820`
- [ ] **IPsec (stubs)** — IKEv2 + ESP for enterprise VPN interop; AES-GCM or ChaCha20-Poly1305 for data plane
- [ ] **Site-to-site VPN** — connect two VeerOS networks; automatic route injection; failover with backup tunnels
- [ ] **Road warrior VPN** — remote client connects to VeerOS gateway; split or full tunnel; DNS push
- [ ] **VPN + NAT integration** — VPN traffic + masquerade for internet access through tunnel; split-horizon DNS
- [ ] **PQC VPN (future)** — WireGuard with ML-KEM hybrid key exchange for post-quantum VPN security (Phase 8C integration)
- [ ] **Shell `vpn` commands** — `vpn status`, `vpn add peer <pubkey> <endpoint>`, `vpn up/down <interface>`

### 13E — Traffic Shaping + QoS
_Bandwidth control, rate limiting, and quality of service prioritization._

- [ ] **Traffic classes** — `QosClass { name, priority, rate_limit, burst, ceil }`; hierarchical: parent + child classes
- [ ] **HTB (Hierarchical Token Bucket)** — rate limiting with borrowing: guaranteed rate + ceiling rate per class
- [ ] **Packet classification** — classify into QoS classes by: src/dst IP, port, protocol, DSCP, firewall mark
- [ ] **Per-interface shaping** — independent shaping per network interface; ingress + egress
- [ ] **Priority queuing** — latency-sensitive traffic (VoIP, SSH) prioritized over bulk (downloads, backups)
- [ ] **Rate limiting** — per-IP, per-service, per-interface rate caps; token bucket with configurable burst
- [ ] **Shell `qos` commands** — `qos add class <name> rate 10mbit ceil 100mbit`, `qos classify -p tcp --dport 22 -c priority`, `qos status`
- [ ] **Bandwidth monitoring** — per-interface, per-class byte/packet counters; real-time throughput display

### 13F — Deep Packet Inspection (DPI)
_Protocol identification and application-level filtering — understand what's in the traffic._

- [ ] **Protocol detection** — identify application protocols (HTTP, HTTPS/TLS, DNS, SSH, MQTT, CoAP) from packet patterns; no decryption
- [ ] **TLS fingerprinting** — JA3/JA4 TLS client fingerprints; identify clients by their TLS hello parameters
- [ ] **Application filter rules** — block/allow by detected protocol: `fw add FORWARD -m app --app bittorrent -j DROP`
- [ ] **DNS filtering** — inspect DNS queries; block domains from configurable blocklists (ad-blocking, malware, parental control)
- [ ] **HTTP header inspection** — for plaintext HTTP: filter by Host, URL path, User-Agent; redirect or block
- [ ] **IDS/IPS integration** — pattern-matching engine for known attack signatures (Snort/Suricata rule format stubs); alert or block
- [ ] **AI-assisted DPI (Phase 10 integration)** — ML classifier for encrypted traffic identification; detect anomalous flows without decryption

### 13G — Network Appliance Hardware Targets
_Specific hardware profiles optimized for firewall/router deployment._

- [ ] **RPi 4/5 router** — dual NIC via USB-C Ethernet adapter (WAN) + built-in Ethernet (LAN); WiFi AP mode for wireless clients
- [ ] **x86-64 mini-PC** — Intel N100 with dual/quad NIC; NVMe for log storage; 8–16 GB RAM; production-grade firewall
- [ ] **ESP32-S3 IoT gateway** — WiFi AP + STA simultaneous; BLE gateway; 802.15.4 border router; packet filtering at the edge
- [ ] **ARM64 cloud VM** — VeerOS as virtual network appliance; KVM virtio-net multi-queue; cloud security group enforcement
- [ ] **Appliance image builder** — script to produce ready-to-flash images with pre-configured firewall rules, VPN, DHCP; zero-touch deploy

### 13H — Firewall Feature Integration Matrix
```
                    net-firewall  net-nat   net-vpn   net-dpi   net-shape
                    (feature)     (feature) (feature) (feature) (feature)
───────────────────────────────────────────────────────────────────────────
Packet filter          ✓            ✓          ─          ─          ─
Conntrack              ✓            ✓          ─          ✓          ─
NAT/SNAT/DNAT          ─            ✓          ─          ─          ─
WireGuard VPN          ─            ─          ✓          ─          ─
IPsec (stubs)          ─            ─          ✓          ─          ─
Protocol detection     ─            ─          ─          ✓          ─
DNS filtering          ─            ─          ─          ✓          ─
IDS/IPS                ─            ─          ─          ✓          ─
Traffic shaping        ─            ─          ─          ─          ✓
QoS classes            ─            ─          ─          ─          ✓
Bandwidth monitor      ─            ─          ─          ─          ✓
───────────────────────────────────────────────────────────────────────────

→ See Phase 4D for network feature defaults per distribution profile.
```

---

## Phase 14 — AI-Native Execution Kernel (Implemented)
_The OS kernel treats autonomous agents, goals, and memory as first-class primitives — not userspace libraries. Agents replace processes as the primary execution unit for AI workloads. Intents replace manual commands with declarative goals. The kernel plans, schedules, and orchestrates autonomously._

_**Determinism guarantee**: every learning mechanism in Phase 14 is fully deterministic. Intent decomposition is rule-based (not stochastic). Placement scoring uses explicit weighted formulas. Episodic memory is append-only with exact replay. Given the same history, the fabric always makes the same decision. LLMs (Phase 10) are an opt-in inference tool for human interaction — never the learning substrate. The fabric itself IS the model: distributed episodic memories, placement scores, behavioral baselines, and repair strategies — all inspectable, auditable, and reproducible._

**Status**: Core kernel modules implemented. Syscall ABI defined (0xF0–0xFF). Wired into dispatch + all 5 kernel targets. Compiles clean.

### 14A — Agent Primitive (`microkernel::agent`)
_Agents are autonomous execution units that own goals, working memory, hierarchies, and compute budgets. They map 1:1 to kernel threads but carry rich metadata for AI-aware scheduling._

- [x] **`AgentState` lifecycle** — `Free → Spawned → Planning → Executing → Blocked → Completed / Failed`; state machine enforced by `AgentTable`
- [x] **`Goal` struct** — description (64 bytes), priority (`Background`/`Normal`/`Elevated`/`Critical`/`Realtime`), optional deadline tick, tick budget, linked intent ID
- [x] **`AgentContext` (working memory)** — 8-slot key-value store per agent (32-byte keys, 64-byte values); context carried across planning/execution phases
- [x] **`AgentCb` (control block)** — goal, task_id link, parent/children hierarchy (MAX_CHILDREN=8), tick budget tracking, spawn/complete timestamps
- [x] **`AgentTable`** — 32 agent slots; `spawn()`, `transition()`, `complete()`, `fail()`, `destroy()`, `tick()` (budget enforcement, returns expired agents)
- [x] **`AgentMessage`** — 4-word envelope for inter-agent communication via kernel channels; `pack()`/`unpack()` serialization
- [x] **`AgentBlockReason`** — `WaitingForChild`, `WaitingForResource`, `WaitingForIntent`, `WaitingForMessage`, `WaitingForPlacement`
- [x] **Hierarchical agents** — parent→child relationships; parent tracks children IDs; depth limited to `MAX_GOAL_DEPTH=8`
- [ ] **Agent factory registry** — pre-registered agent blueprints (entry point + capability set) for common intents
- [ ] **Agent migration** — migrate agent state between fabric nodes; serialize `AgentCb` + context → remote node
- [ ] **Shell `agents` command** — list active agents, their goals, states, budget usage; `agent spawn <goal>`, `agent kill <id>`

### 14B — Intent Engine (`microkernel::intent`)
_Declarative goal decomposition — users submit "what" they want, the kernel figures out "how"._

- [x] **`IntentClass` taxonomy** — `Compute`, `Deploy`, `Monitor`, `Communicate`, `Data`, `Admin`, `Pipeline`, `Custom`
- [x] **`IntentConstraint` system** — `MaxLatencyMs`, `NodeAffinity`, `MinReliability`, `MaxCost`, `Locality`, `SecurityLevel`; up to 4 constraints per intent
- [x] **`IntentStatus` lifecycle** — `Free → Pending → Planning → Active → Fulfilled / Failed / Cancelled`
- [x] **`IntentDescriptor`** — ID, class, status, description (64 bytes), priority, constraints, root_agent, submitter_task, timestamps
- [x] **`Plan` (step DAG)** — up to 16 `PlanStep`s with `StepRelation` (`Independent`/`DependsOn(idx)`/`Parallel`); dependency-aware readiness check
- [x] **Rule-based decomposition** — `Compute→1 step`, `Deploy→3 steps (validate→provision→verify)`, `Data→2 steps (acquire→transform)`, etc.
- [x] **`IntentEngine`** — 16 intent slots; `submit()`, `decompose()`, `cancel()`, `tick()` (sync agent completion → intent status), `status()`
- [ ] **Constraint solver** — advanced constraint satisfaction for multi-constraint intents; Pareto-optimal placement
- [ ] **NL intent parsing (userspace)** — natural language → `IntentClass` + constraints via AI model; bridges Phase 10I NL shell
- [ ] **Composite intents** — intent that decomposes into sub-intents (not just plan steps); recursive decomposition
- [ ] **Intent templates** — pre-defined intent schemas for common operations (deploy-service, scale-up, backup-data)

### 14C — Memory Engine (`microkernel::memory_engine`)
_Three-tier memory system: per-agent context (hot), persistent knowledge (warm), episodic history (cold)._

- [x] **Context memory** — per-agent 8-slot key-value store in `AgentContext`; fast O(n) lookup; carried through agent lifecycle
- [x] **`PersistentMemory`** — global key-value store with `MemoryTag` (System/Preference/Cache/Config/Relation/Skill/UserKnow/Observation) and `MemoryScope` (Global/Intent/Agent/Process); LRU eviction; confidence scoring; read-count tracking
- [x] **`EpisodicMemory`** — ring buffer of `Episode` records with `EpisodeKind` (12 variants: TaskSpawned, AgentSpawned, IntentSubmitted, ...) and `EpisodeOutcome` (Success/Failure/Partial/Pending); `success_rate()`, `find_by_intent()`, `find_latest()`
- [x] **`MemoryEngine`** — owns persistent + episodic memories; distribution-profile sized (32/128/512 persistent, 64/256/1024 episodic)
- [x] **Syscall interface** — `SYS_MEMORY_STORE` (0xF8), `SYS_MEMORY_QUERY` (0xF9) for userspace read/write to persistent memory
- [ ] **Memory consolidation** — periodic sweep: merge duplicate keys, decay low-confidence entries, promote high-read entries
- [ ] **Cross-agent memory sharing** — agents in same intent can share a scoped memory partition
- [ ] **Flash-backed persistence** — serialize persistent memory to flash/SD on shutdown; restore on boot
- [ ] **Temporal queries** — query episodic memory by time range, success rate in window, trend detection

### 14D — Execution Fabric (`microkernel::fabric`)
_Unified compute plane spanning heterogeneous nodes — from ESP32 to cloud VMs._

- [x] **`FabricNode`** — health state, name, architecture (`Riscv32`/`Riscv64`/`Aarch64`/`X86_64`/`Xtensa`), locality zone (`Local`/`Rack`/`DataCenter`/`Region`/`Global`), capability bitmask (16 caps: Compute, GpuCompute, NpuInference, Storage, ...), resource snapshot (cores, MHz, RAM, load, agent count), heartbeat/RTT tracking
- [x] **`PlacementConstraint`** — required capability, min cores/RAM, max RTT, preferred zone, prefer-low-load flag
- [x] **`ExecutionFabric`** — 1/8/64 nodes by distribution profile; `register_local()`, `register_remote()`, `heartbeat()`, `check_health()`, `select_node()` (weighted scoring: capability match → resource fit → locality → load)
- [x] **Health monitoring** — periodic heartbeat timeout check; nodes transition `Healthy → Degraded → Offline`; overload detection from load %
- [ ] **Cluster discovery** — mDNS/gossip protocol for automatic node discovery in LAN; Phase 11 integration
- [ ] **Remote agent dispatch** — serialize agent goal + context, send to remote fabric node, await completion
- [ ] **Resource reservation** — reserve cores/RAM on target node before agent placement; release on completion
- [ ] **Fabric dashboard** — shell `fabric` command: list nodes, health, capabilities, load; `fabric add/remove`

### 14E — Intent Scheduler (`microkernel::intent_sched`)
_Meta-scheduler that orchestrates the AI-native execution loop — sits above the task scheduler._

- [x] **6-phase tick cycle** — (1) decompose pending intents, (2) assign agents to ready plan steps with placement, (3) monitor executing agents (budget/deadline enforcement), (4) sync intent status from agents, (5) record outcomes in episodic memory, (6) fabric health check
- [x] **`IntentSchedStats`** — intents submitted/fulfilled/failed, agents spawned/completed/failed, replans count
- [x] **Budget enforcement** — agents exceeding tick budget marked failed; intent re-plan triggered if possible
- [x] **Placement integration** — `build_placement_constraint()` maps intent constraints → fabric placement; critical goals prefer low-load nodes
- [x] **Episodic feedback loop** — successful strategies stored in persistent memory; failed patterns recorded for future avoidance
- [x] **Rate-limited execution** — scheduler runs every `INTENT_SCHED_INTERVAL=10` ticks; fabric health every 100 ticks
- [ ] **Adaptive scheduling** — learn optimal tick interval from workload patterns; throttle under high load
- [ ] **Priority inversion prevention** — detect when low-priority intent blocks high-priority agent; reorder or preempt
- [ ] **Speculative execution** — pre-spawn agents for likely next steps while current step executes
- [ ] **SLA tracking** — per-intent latency/success SLA; alert when SLA at risk; escalate priority automatically

### 14F — AI-Native Syscall ABI (0xF0–0xFF)
_Kernel-mediated agent and intent lifecycle — capability-gated, architecture-neutral._

- [x] **`SYS_AGENT_SPAWN` (0xF0)** — spawn agent with goal description, priority, entry point, stack; returns agent ID
- [x] **`SYS_AGENT_STATUS` (0xF1)** — query agent state + ticks_used
- [x] **`SYS_AGENT_COMPLETE` (0xF2)** — mark agent completed (5) or failed (6)
- [x] **`SYS_AGENT_CTX_SET` (0xF3)** — set key-value in calling agent's context memory
- [x] **`SYS_AGENT_CTX_GET` (0xF4)** — get value from calling agent's context memory
- [x] **`SYS_INTENT_SUBMIT` (0xF5)** — submit intent with class, description, priority; returns intent ID
- [x] **`SYS_INTENT_STATUS` (0xF6)** — query intent status
- [x] **`SYS_INTENT_CANCEL` (0xF7)** — cancel in-flight intent
- [x] **`SYS_MEMORY_STORE` (0xF8)** — store key-value to persistent memory
- [x] **`SYS_MEMORY_QUERY` (0xF9)** — query persistent memory by key
- [x] **`SYS_FABRIC_STATUS` (0xFA)** — query fabric node counts (total, healthy)
- [x] **`SYS_INTENT_SCHED_STATS` (0xFB)** — query scheduler statistics
- [x] **`SYS_AGENT_COUNT` (0xFC)** — get active agent count
- [x] **Capability gates** — `ProcessCaps::AGENT`, `INTENT`, `MEMORY_ENGINE`, `FABRIC` (bits 19–22)
- [ ] **Userlib wrappers** — `userlib::agent`, `userlib::intent`, `userlib::memory` modules wrapping raw syscalls
- [ ] **Async intent API** — submit intent, receive channel notification on fulfillment; integrates with Phase 6F poll

### 14G — Kernel Integration
_All 5 kernel targets wired with AI-native subsystem statics and dispatch._

- [x] **Static instances** — `AgentTable`, `IntentEngine`, `MemoryEngine`, `ExecutionFabric`, `IntentScheduler` declared in each kernel `main.rs`
- [x] **Dispatch wiring** — all 5 subsystems passed to `dispatch()` function; capability enforcement for all 13 new syscalls
- [x] **x86_64 (qemu_pc)** — compiled and verified
- [x] **RISC-V 32 (qemu_virt, esp32c3, xiao_esp32c6)** — dispatch call updated
- [x] **AArch64 (raspi5)** — dispatch call updated

### 14H — Userlib + Shell + Demo (Implemented)
_Full userspace API, shell commands, man pages, and interactive demo binary for all AI-native subsystems._

- [x] **Userlib modules** — `userlib::agent` (spawn/status/complete/fail/ctx_set/ctx_get/count), `userlib::intent` (submit/status/cancel/sched_stats), `userlib::memory` (store/query/fabric_status)
- [x] **Shell commands** — `agents` (list/status), `intent` (submit/status/cancel), `memory`/`kv` (store/query/list), `fabric` (node list/stats)
- [x] **Demo command** — 3 interactive scenarios: deploy (service deployment), pipeline (4-stage data processing), monitor (agent swarm)
- [x] **Man pages** — `agents`, `intent`, `memory`, `fabric`, `demo`
- [x] **Demo binary** — `veeros-demo` with 4-node pre-populated fabric (x86 host, ARM64 RPi5, RISC-V ESP32, cloud GPU), seeded memory, all AI callbacks wired
- [x] **Shell callbacks** — 6 new `ShellEnv` callback fields wired across all 5 kernel targets
- [x] **Launcher** — `scripts/demo.sh` with `--scenario` mode
- [x] **Documentation** — `docs/demo-walkthrough.md`, README marketing update, `docs/architecture.md` application domains

---

## Phase 15 — Distributed Fabric, Zero Trust & ZKP Security (In Progress)
_Transform the local-only execution fabric into a real distributed system spanning heterogeneous nodes — secured by Zero Trust identity, Zero Knowledge capability proofs, and PQC-hybrid cryptography at the kernel level. No other embedded OS provides this._

**Goal**: Any VeerOS instance — an ESP32 sensor, a Raspberry Pi gateway, a cloud GPU, a phone app — joins a single coherent fabric with cryptographic identity, capability attestation, encrypted transport, agent migration, and distributed memory. All without a central coordinator.

### 15A — Fabric Wire Protocol (`microkernel::fabric_proto`)
_Binary wire format for all fabric operations. Fixed-size, `no_alloc`, parseable on a 32-bit MCU._

- [ ] **`FabricMsg` enum** — message types: `NodeAnnounce`, `NodeHeartbeat`, `IntentForward`, `AgentMigrate`, `MemorySync`, `MemoryQuery`, `CapabilityProof`, `Challenge`, `ChallengeResponse`, `Ack`, `Nack`
- [ ] **Binary encoding** — `[u8; N]` serialization with type-tag + length + payload; no serde, no alloc
- [ ] **Message framing** — 4-byte header (magic `0xVE`, version, msg_type, payload_len) + payload + HMAC-SHA256 integrity tag
- [ ] **Version negotiation** — protocol version in header; receivers reject unknown versions
- [ ] **Sequence numbers** — monotonic u32 per-peer for replay protection
- [ ] **Max message size** — 512 bytes (fits in single UDP datagram or BLE packet)

### 15B — Zero Trust Node Identity (`microkernel::node_identity`)
_Every node has a cryptographic identity. No implicit trust based on network location._

- [ ] **Node keypair** — Ed25519 (classical) + ML-DSA-65 (PQC hybrid) per-node identity keypair, generated at first boot, stored in persistent memory
- [ ] **Node ID** — SHA-256 hash of public key = 32-byte globally unique node identifier
- [ ] **Attestation certificate** — self-signed statement: `{node_id, arch, capabilities, zone, timestamp, signature}` — nodes present this on join
- [ ] **Mutual authentication** — challenge-response: A sends nonce → B signs nonce+A's_node_id → A verifies; then reverse. Both sides authenticated before any data flows
- [ ] **Session key derivation** — after mutual auth, derive per-session ChaCha20-Poly1305 key via HKDF(shared_nonce, node_ids, "veeros-fabric-session-v1")
- [ ] **Capability-based authorization** — `ProcessCaps::FABRIC_ADMIN` for join/remove, `ProcessCaps::FABRIC_READ` for queries, `ProcessCaps::FABRIC_MIGRATE` for agent migration
- [ ] **Trust levels** — `TrustLevel` enum: `Untrusted` (just joined, challenge pending), `Verified` (mutual auth passed), `Attested` (ZKP capability proof verified), `Revoked`
- [ ] **Certificate revocation** — node can broadcast revocation of another node's cert; peers stop accepting messages from revoked nodes

### 15C — Zero Knowledge Capability Proofs (`microkernel::zkp`)
_Nodes prove they have capabilities without revealing full resource profiles. Agents prove goal completion without leaking processed data._

- [ ] **Schnorr-based ZKP** — efficient, `no_std`-friendly Σ-protocol over SHA-256 commitments
- [ ] **Capability commitment** — node commits to capability bitmask: `C = H(capabilities || salt)`, reveals `C` publicly
- [ ] **Selective disclosure** — prove "I have GPU capability" without revealing full bitmask: construct Merkle proof over individual capability bits
- [ ] **Agent completion proof** — agent proves it reached `Completed` state with specific output hash, without revealing the data: `proof = ZKP{H(output) == claimed_hash}`
- [ ] **Memory entry proofs** — prove a persistent memory entry exists with a certain tag/scope without revealing the value
- [ ] **Proof verification** — deterministic verifier in `< 200 lines`, runs on riscv32imc within tick budget
- [ ] **Challenge-proof protocol** — integrated into fabric join handshake: after mutual auth, verifier challenges prover's claimed capabilities via ZKP before granting `Attested` trust level

### 15D — PQC-Hybrid Fabric Encryption (`microkernel::fabric_crypto`)
_All inter-node communication encrypted with PQC-hybrid AEAD. Defense against harvest-now-decrypt-later attacks._

- [ ] **Session establishment** — X25519 + ML-KEM-768 hybrid KEM for key exchange (uses existing `crypto::hybrid` combiner)
- [ ] **Channel encryption** — every `FabricMsg` payload encrypted with ChaCha20-Poly1305 using session key + per-message nonce (counter-based)
- [ ] **Key rotation** — session keys rotated every 2^32 messages or 1 hour (whichever first); re-derive via HKDF with new salt
- [ ] **Forward secrecy** — previous session keys zeroized after rotation; compromise of current key doesn't reveal past traffic
- [ ] **Per-message authentication** — HMAC-SHA256 over (header + encrypted_payload + sequence_number); prevents tampering + replay
- [ ] **Downgrade prevention** — if node advertises PQC capability, classical-only sessions are rejected
- [ ] **Crypto agility** — `CryptoMode` (Classical/Hybrid/PqcOnly) negotiated at session start; allows gradual fleet-wide migration

### 15E — Mesh Transport (`microkernel::mesh`)
_Decentralized node discovery and message routing. No central broker._

- [ ] **Discovery protocol** — periodic `NodeAnnounce` broadcast (multicast on LAN, BLE advertisement on IoT, gossip on WAN)
- [ ] **Gossip protocol** — each node shares its neighbor table with peers; convergence in O(log N) rounds
- [ ] **Peer table** — `PeerTable` struct: per-peer state (node_id, trust_level, session_key, last_seen, rtt_us, address)
- [ ] **Message routing** — if destination not a direct peer, forward via lowest-RTT path (greedy geographic routing)
- [ ] **Transport abstraction** — `MeshTransport` trait with `send(node_id, msg)` / `recv() → (node_id, msg)` — implemented over TCP, UDP, BLE, UART, SPI
- [ ] **Backpressure** — per-peer send queue (8 messages); drop lowest-priority messages on overflow
- [ ] **Partition tolerance** — nodes continue operating locally during network partition; auto-rejoin and re-sync on reconnection
- [ ] **NAT traversal** — optional STUN-like hole punching for nodes behind NAT (WAN deployments)

### 15F — Agent Serialization & Migration
_Agents can be frozen on one node and resumed on another — the kernel primitive for workload mobility._

- [ ] **`AgentSnapshot`** — serialized representation of `AgentCb`: state, goal, context, ticks_used, parent/children references (by node_id + agent_id)
- [ ] **Snapshot syscall** — `SYS_AGENT_SNAPSHOT` freezes agent → `AgentSnapshot` bytes
- [ ] **Restore syscall** — `SYS_AGENT_RESTORE` on destination node creates agent from snapshot
- [ ] **Migration protocol** — source: freeze → snapshot → encrypt → send `AgentMigrate` msg; destination: receive → decrypt → verify → restore → ack
- [ ] **Context transfer** — agent's working memory (8 KV slots) migrated atomically with the agent
- [ ] **Child reassignment** — if parent migrates, children updated with new parent location (node_id, agent_id)
- [ ] **Integrity verification** — SHA-256 hash of snapshot included in `AgentMigrate` message; destination verifies before restore

### 15G — Distributed Memory Sync
_Persistent and episodic memory replicated across the fabric for resilience and coordination._

- [ ] **`MemorySync` message** — carries key/value/tag/scope + origin node_id + vector clock
- [ ] **Conflict resolution** — last-writer-wins with vector clock tie-breaking; configurable per-tag (e.g., Config entries merge, Cache entries overwrite)
- [ ] **Scoped replication** — `MemoryScope::Global` entries replicated to all nodes; `MemoryScope::Agent` entries follow agent migration only
- [ ] **Episodic gossip** — recent episodes broadcast to fabric peers for distributed learning; ring buffer merge by tick ordering
- [ ] **Bandwidth budget** — sync rate limited per-peer to prevent flooding constrained links (e.g., BLE = 2 entries/sec)
- [ ] **Consistency model** — eventual consistency with causal ordering (vector clocks); no distributed consensus overhead

### 15H — Cross-Node Intent Dispatch
_The Intent Scheduler places agents on the best node — including remote nodes._

- [ ] **Remote agent spawn** — when `select_node()` returns a remote node, send `IntentForward` message instead of local spawn
- [ ] **Remote status tracking** — agent status updates flow back via `AgentMigrate` (status variant) messages
- [ ] **Re-planning on failure** — if remote node goes offline, intent scheduler re-decomposes and re-places on surviving nodes
- [ ] **Distributed plan DAG** — plan steps track which node each agent lives on; cross-node dependencies resolved via `MemorySync`
- [ ] **Load-aware placement** — `NodeResources.cpu_load` and `active_agents` updated via heartbeat; scheduler prefers least-loaded capable node

### 15I — Kernel Integration & Feature Flags

- [ ] **`dist-cluster` feature** — gates all distributed code; `dist-minimal` stays single-node with zero overhead
- [ ] **New modules in `lib.rs`** — `fabric_proto`, `node_identity`, `zkp`, `fabric_crypto`, `mesh`
- [ ] **New syscalls** — `SYS_NODE_ID` (0xE0), `SYS_PEER_COUNT` (0xE1), `SYS_MESH_SEND` (0xE2), `SYS_MESH_RECV` (0xE3), `SYS_AGENT_SNAPSHOT` (0xE4), `SYS_AGENT_RESTORE` (0xE5), `SYS_ZKP_PROVE` (0xE6), `SYS_ZKP_VERIFY` (0xE7)
- [ ] **ProcessCaps** — `FABRIC_ADMIN` (bit 23), `FABRIC_MIGRATE` (bit 24), `ZKP` (bit 25)
- [ ] **All 5 kernel targets** — static instances + dispatch wiring for new subsystems

---

## Phase 16 — Enterprise Security: Kernel-Native NAC + EDR + ZTNA
_The kernel IS the security appliance. No agents, no sidecars, no bolt-on products. Every syscall, every packet, every authentication event is visible to the kernel — making it the only entity that can truly enforce Zero Trust. Replaces ClearPass/Aruba (NAC), Trellix/CrowdStrike (EDR), and Zscaler/Cloudflare (ZTNA) with kernel primitives. Feature-gated: `sec-nac`, `sec-edr`, `sec-ztna`._

### 16A — Network Access Control (replaces ClearPass / Aruba / ISE)
_Every device must prove identity and posture before touching the network. The kernel enforces this at the packet level — no separate appliance needed._

- [ ] **802.1X authenticator** — kernel-native EAP authenticator on wired/wireless interfaces; supports EAP-TLS (certificate), EAP-TTLS, PEAP; integrates with Phase 6I user identity + Phase 8C crypto
- [ ] **RADIUS client** — lightweight RADIUS protocol client for external AAA server integration; PAP, CHAP, MS-CHAPv2; retransmit + failover
- [ ] **Device posture assessment** — at fabric join time, every node evaluates peers: firmware version, security features enabled (PQC, secure boot, encryption), compliance flags; non-compliant nodes quarantined
- [ ] **Network segmentation via capabilities** — replace VLANs with capability-based segmentation: `NetworkNamespace(ns_id)` + `NetworkInterface(nic_id)` capabilities determine which networks a process/device can access
- [ ] **Certificate-based device authentication** — each VeerOS node has a device certificate (Ed25519 + ML-DSA hybrid, Phase 8C); presented during 802.1X or fabric join; no MAC-address-based trust
- [ ] **Guest / quarantine network** — non-compliant or unknown devices routed to isolated network namespace; limited access (captive portal, remediation server only)
- [ ] **MAC Authentication Bypass (MAB)** — fallback for legacy devices without 802.1X supplicant; MAC allow-list with audit logging; flagged as `TrustLevel::Legacy`
- [ ] **Dynamic authorization (CoA)** — RADIUS Change-of-Authorization support; re-evaluate access mid-session on policy change or posture change
- [ ] **NAC policy profiles** — `nac-open` (allow all, audit only), `nac-posture` (check posture, quarantine non-compliant), `nac-strict` (802.1X required, no exceptions), `nac-zero-trust` (continuous re-auth)
- [ ] **Shell `nac` commands** — `nac status`, `nac clients`, `nac policy set <profile>`, `nac quarantine <node>`, `nac allow <node>`
- [ ] **Integration with ZTNA (16C)** — NAC posture feeds into ZTNA access decisions; device health is a continuous access condition

### 16B — Endpoint Detection & Response (replaces Trellix / CrowdStrike / Defender)
_The kernel is the sensor. No userspace agent can see what the kernel sees. Every syscall, every IPC message, every memory access, every packet — observed at the source of truth._

- [ ] **Syscall behavior monitoring** — per-process syscall frequency histogram + sequence tracking; updated on every syscall dispatch; zero overhead (counter array in `Process` struct)
- [ ] **Behavioral baseline learning** — during first N minutes (configurable), learn normal syscall/IPC/memory patterns per process; store as compact profile in persistent memory (Phase 14C)
- [ ] **Anomaly detection engine** — compare real-time behavior against learned baseline; scoring: `ThreatScore = Σ(deviation × weight)` per dimension (syscall freq, IPC targets, memory access, network); threshold-triggered alert
- [ ] **Detection rules** — declarative rule definitions: `if process.syscall_rate(SYS_OPEN) > 100/s AND process.net_connections > 10 THEN alert(high)` — compiled to efficient kernel checks
- [ ] **Automated response actions** — configurable per-threat-level:
  - `Low` → audit log entry, alert to console
  - `Medium` → restrict capabilities (drop network, restrict IPC), alert
  - `High` → isolate process in sandbox domain (Phase 8B), revoke all non-essential capabilities
  - `Critical` → kill process, snapshot state for forensics, alert fleet
- [ ] **Capability revocation on detection** — immediate `SYS_CAP_REVOKE` cascade for compromised process; prevents lateral movement before human response
- [ ] **IOC (Indicator of Compromise) matching** — maintain hash table of known-bad: file hashes (SHA-256), network indicators (IP/domain), syscall sequences; compare on file open, network connect, process spawn
- [ ] **Forensic snapshot** — on high/critical detection, capture: process memory regions, open file descriptors, capability set, recent syscall history (last 256), IPC connections, network sockets; stored to persistent memory or SD
- [ ] **Threat intelligence feed integration** — periodic IOC list update via fabric sync or HTTPS fetch; capability-gated (`SEC_ADMIN`)
- [ ] **Cross-node correlation** — fabric-wide anomaly correlation: if same anomaly pattern appears on multiple nodes simultaneously → elevated fleet-wide alert; uses episodic memory (Phase 14C) for cross-node event sharing
- [ ] **EDR telemetry export** — structured security event export (JSON/CBOR) via syslog, MQTT, or HTTPS to external SIEM; rate-limited to prevent bandwidth exhaustion
- [ ] **Shell `edr` commands** — `edr status`, `edr threats`, `edr baseline <process>`, `edr rules list`, `edr quarantine <pid>`, `edr forensic <pid>`
- [ ] **Integration with audit log (Phase 8F)** — all EDR events recorded as `AuditEvent::ThreatDetected`, `AuditEvent::ThreatResponse`, `AuditEvent::BaselineDeviation`

### 16C — Zero Trust Network Access (replaces Zscaler / Cloudflare ZTNA / BeyondCorp)
_No implicit trust — ever. Not on the local network, not on the fabric, not between processes on the same node. Every connection is authenticated, authorized, and continuously verified. The kernel enforces this without any proxy or gateway._

- [ ] **Per-flow identity verification** — every new connection (TCP, IPC, fabric channel) carries a cryptographic identity assertion: `(node_cert, user_token, device_posture, timestamp, signature)`
- [ ] **Identity assertion protocol** — first bytes of any connection carry a compact identity frame (< 128 bytes); kernel validates before passing data to application; rejected connections never reach userspace
- [ ] **Continuous authentication** — connections re-verified periodically (configurable: 1 min – 1 hour); posture change mid-session triggers re-evaluation; revoke on failure
- [ ] **Context-aware access policies** — access decisions based on multi-factor context:
  - **Identity:** who (user + device certificate)
  - **Posture:** device health (firmware version, secure boot, encryption status, baseline compliance)
  - **Location:** network zone (local/rack/datacenter/wan), IP range, geographic region
  - **Time:** access windows (business hours only, maintenance windows)
  - **Behavior:** current threat score from EDR (16B), recent anomaly count
- [ ] **Micro-segmentation as side effect** — capabilities (Phase 8A) naturally enforce least-privilege access; ZTNA adds *continuous* verification on top; no separate policy engine needed
- [ ] **ZTNA policy rules** — `ZtnaRule { subject: IdentityMatch, resource: ServiceMatch, conditions: [ContextCondition], effect: Allow|Deny|Challenge }`; evaluated on every connection attempt
- [ ] **Step-up authentication** — for sensitive resources, require additional verification: re-enter password, hardware token, biometric challenge (via connected device)
- [ ] **Session tokens** — kernel-issued, time-bounded, cryptographically signed session tokens; bound to (user, device, node); non-transferable
- [ ] **Policy decision caching** — cache recent allow decisions for (identity, resource) pairs; invalidate on posture change; reduces per-request overhead to near-zero for repeated access patterns
- [ ] **ZTNA fabric integration** — every fabric peer authenticated via mutual PQC-TLS (Phase 15B); ZTNA layer adds *authorization* on top of authenticated channels
- [ ] **No implicit LAN trust** — devices on the same physical network get NO implicit access; every connection verified identically whether source is local switch or Internet
- [ ] **Shell `ztna` commands** — `ztna status`, `ztna policy list/add/remove`, `ztna sessions`, `ztna deny <identity>`, `ztna audit`
- [ ] **ZTNA metrics** — connections allowed/denied/challenged per service, per identity; latency of policy evaluation; cache hit rate

### 16D — Enterprise Security Integration Matrix
_How the three pillars work together — each reinforces the others._

```
                        NAC (16A)   EDR (16B)   ZTNA (16C)
                        ─────────   ─────────   ──────────
Device identity           ✓                        ✓
Device posture            ✓            ✓           ✓
Network segmentation      ✓                        ✓
Threat detection                       ✓
Automated response                     ✓           ✓
Continuous monitoring     ✓            ✓           ✓
Per-flow authorization                              ✓
Behavioral analysis                    ✓           ✓
IOC matching                           ✓
Forensics                              ✓
Audit trail               ✓            ✓           ✓

Replaces:
  ClearPass / Aruba ISE / Cisco ISE    →  Phase 16A (NAC)
  Trellix / CrowdStrike / Defender     →  Phase 16B (EDR)
  Zscaler / Cloudflare ZTNA / Prisma   →  Phase 16C (ZTNA)
  Forescout / NAC appliances           →  Phase 16A + 16C
  Tanium / endpoint visibility         →  Phase 16B (kernel = sensor)
```

### 16E — Distribution Profile Integration
- [ ] **`sec-nac` feature** — enables 802.1X authenticator, RADIUS client, posture assessment; auto-enabled in `dist-cloud`, `dist-firewall`
- [ ] **`sec-edr` feature** — enables behavioral monitoring, anomaly detection, automated response; auto-enabled in `dist-full`, `dist-cloud`, `dist-firewall`, `dist-ai`
- [ ] **`sec-ztna` feature** — enables per-flow identity verification, context-aware policies, continuous auth; auto-enabled in `dist-cloud`, `dist-cluster`
- [ ] **`dist-firewall` default** — all three (`sec-nac` + `sec-edr` + `sec-ztna`); VeerOS-as-security-appliance replaces dedicated NAC/EDR/ZTNA products
- [ ] **`dist-minimal` / `dist-rt`** — none enabled by default (no overhead); opt-in via feature flags
- [ ] **`dist-edge` / `dist-gateway`** — `sec-edr` + `sec-ztna` enabled; NAC optional (depends on network role)

---

## Phase 17 — Enterprise Device Management (replaces Intune / JAMF / SCCM)
_Fabric join IS device enrollment. The OS manages itself and its fleet — no MDM server, no agent, no cloud dependency for basic operations. Replaces Microsoft Intune, JAMF Pro, SCCM, and Workspace ONE with kernel-native fleet management. Feature-gated: `device-mgmt`._

### 17A — Device Enrollment & Identity
_Every VeerOS node is self-describing. Joining the fabric automatically enrolls the device into the management plane._

- [ ] **Fabric join = MDM enrollment** — `cluster join` (Phase 11A) automatically registers device in fleet inventory; no separate enrollment flow; one operation replaces: MDM enroll + certificate provisioning + inventory scan + compliance check
- [ ] **Device certificate provisioning** — on first boot, generate Ed25519 + ML-DSA hybrid device identity keypair (Phase 8C); on fabric join, request signed device certificate from fleet CA (leader node or designated CA); cert stored in kernel keystore
- [ ] **Hardware attestation** — device proves firmware integrity via measurement registers (Phase 8D): TPM PCR values (x86-64), eFuse state (ESP32), boot measurements (ARM64); attestation report sent during enrollment
- [ ] **Device inventory record** — auto-populated at enrollment: `DeviceRecord { node_id, arch, soc, firmware_version, capabilities, memory_total, storage_total, security_features, enrollment_timestamp, last_seen, compliance_status }`
- [ ] **Device groups & tags** — organize devices: `group:sensors`, `group:gateways`, `tag:floor-3`, `tag:production`; used for targeted policy application and fleet operations
- [ ] **Auto-discovery enrollment** — mDNS-discovered devices on trusted networks auto-enrolled with `TrustLevel::Verified` after mutual authentication (Phase 15B); unknown networks require manual approval
- [ ] **Enrollment profiles** — pre-configured enrollment templates: `profile-iot-sensor` (minimal, single-user, locked-down), `profile-gateway` (NAC + firewall), `profile-workstation` (multi-user, full security), `profile-cloud-node` (cluster + ZTNA)

### 17B — Policy & Compliance Engine
_Declarative compliance rules — the kernel evaluates device state against policy and auto-remediates._

- [ ] **Compliance rules** — `ComplianceRule { id, name, check: ComplianceCheck, action: RemediationAction, severity: Critical|High|Medium|Low }`
- [ ] **Compliance checks** — built-in check types:
  - `MinFirmwareVersion(version)` — reject devices running old firmware
  - `SecureBootEnabled` — require secure boot chain (Phase 8D)
  - `EncryptionEnabled` — require flash encryption (ESP32) or disk encryption (x86-64)
  - `PqcCryptoEnabled` — require PQC-hybrid crypto stack active
  - `PasswordPolicyMet` — minimum password length/complexity (Phase 6I)
  - `FirewallEnabled` — packet filter active with deny-default policy
  - `MaxIdleTime(seconds)` — auto-lock after inactivity
  - `AllowedApps(list)` — only permitted process images may run
  - `RequiredSecurityTier(tier)` — minimum `sec-*` feature set enabled
- [ ] **Configuration profiles** — named bundles of settings applied to devices: network config (SSID, static IP, DNS), security policies, feature flags, resource limits, shell customization
- [ ] **Profile assignment** — assign profiles to individual devices, groups, or tags; priority ordering (device-specific > group > fleet-default)
- [ ] **Continuous compliance monitoring** — kernel evaluates compliance rules every N ticks (configurable, default 60s); status updated in `DeviceRecord.compliance_status`
- [ ] **Auto-remediation** — on policy violation:
  - `Low` → log warning, notify console
  - `Medium` → apply corrective configuration automatically (e.g., enable firewall)
  - `High` → restrict capabilities, quarantine from fabric until remediated
  - `Critical` → isolate from network, alert fleet, require manual intervention
- [ ] **Drift detection** — compare current device state against last-applied profile; detect unauthorized changes; auto-correct or alert
- [ ] **Compliance event audit** — all compliance checks, violations, and remediations logged to security audit (Phase 8F): `AuditEvent::ComplianceCheck`, `ComplianceViolation`, `ComplianceRemediation`

### 17C — Fleet Operations
_Operate the entire fleet from any console using declarative intents._

- [ ] **OTA firmware updates** — push signed firmware images to devices over fabric; A/B partitioning (write to inactive slot, verify, switch, rollback on failure); integrates with Phase 8D secure boot + rollback protection
- [ ] **Staged rollouts** — `update --staged canary:2 → 10% → 50% → 100%`; monitor health between stages; auto-rollback if error rate exceeds threshold
- [ ] **Remote wipe** — `device wipe <node>` — securely erase all user data, keys, and configuration; reset to factory firmware; requires `DEVICE_ADMIN` capability + confirmation
- [ ] **Remote lock / unlock** — `device lock <node>` — disable shell access, restrict to heartbeat-only; `device unlock` restores normal operation
- [ ] **Health attestation** — periodic challenge-response: fleet CA challenges device to prove current boot measurements + compliance state; non-responsive or failed devices flagged
- [ ] **Fleet-wide intents** — `intent submit "enforce security-baseline across fleet"` → intent engine (Phase 14B) decomposes to per-device compliance enforcement agents across fabric
- [ ] **Patch state tracking** — per-device firmware version + last update timestamp + pending updates; fleet-wide patch coverage dashboard
- [ ] **Device lifecycle** — `device provision` → `device deploy` → `device monitor` → `device decommission` → `device wipe`; state machine per device
- [ ] **Bulk operations** — `device group:sensors update firmware v2.1.0` — apply operation to all devices in a group

### 17D — Shell Integration
- [ ] **`device` command** — `device list`, `device info <node>`, `device comply <node>`, `device update <node>`, `device lock/unlock <node>`, `device wipe <node>`, `device group <op>`, `device tag <op>`
- [ ] **`fleet` command** — `fleet status` (compliance summary), `fleet update` (staged rollout), `fleet policy set <profile>`, `fleet audit` (recent compliance events)
- [ ] **`compliance` command** — `compliance status` (per-device), `compliance rules list`, `compliance rules add <rule>`, `compliance check <node>`, `compliance report`

### 17E — What Device Management Eliminates
```
Traditional MDM Stack           → VeerOS Device Management
─────────────────────────────────────────────────────────────
Microsoft Intune                → Phase 17 (fabric-native)
JAMF Pro                        → Phase 17 (no macOS bias)
SCCM / ConfigMgr               → Phase 17 (no Windows dependency)
VMware Workspace ONE            → Phase 17 + Phase 20 (ZeroServices)
MobileIron / Ivanti             → Phase 17 (kernel-native)
MDM enrollment server           → Fabric join (one operation)
Certificate authority server    → Fleet CA on leader node
Compliance scanner agent        → Kernel compliance engine (no agent)
Patch management server         → Fabric OTA with A/B partitioning
```

---

## Phase 18 — WAN-Scale Fabric
_Extend the execution fabric beyond LAN to span the public Internet. Nodes behind NAT, across continents, on cellular networks — all part of one coherent fabric with the same security guarantees. Feature-gated: `fabric-wan`._

### 18A — NAT Traversal
_Most real-world devices are behind NAT. The fabric must punch through._

- [ ] **STUN client** — discover public IP:port via STUN server (self-hosted or well-known); cache mapping with TTL refresh
- [ ] **STUN server** — any fabric node with a public IP can serve as STUN reflector for peers; auto-elected based on `FabricNode.locality_zone == Global`
- [ ] **UDP hole punching** — WireGuard-style coordinated UDP NAT traversal: both peers send initial packets to each other's STUN-discovered endpoints simultaneously
- [ ] **TURN relay fallback** — for symmetric NAT (double-NAT, CGNAT) where hole punching fails: relay data through a fabric node with public connectivity; encrypted end-to-end (relay sees only ciphertext)
- [ ] **ICE-like candidate negotiation** — gather candidates: host IP, STUN-mapped IP, TURN relay; try in priority order (host → STUN → TURN); select fastest working path
- [ ] **Connection state persistence** — cache successful NAT traversal state in persistent memory (Phase 14C); on reconnect, try cached endpoint first before full discovery
- [ ] **Cellular/mobile support** — handle IP address changes (cellular handoff, roaming); reconnect with session resumption (TLS 1.3 PSK tickets, Phase 8E)

### 18B — WAN Mesh Overlay
_Encrypted tunnels over public Internet forming a virtual mesh. Every link PQC-hybrid encrypted._

- [ ] **Encrypted tunnel establishment** — on WAN peer discovery, establish ChaCha20-Poly1305 tunnel (WireGuard-compatible Noise IK handshake with ML-KEM hybrid KEM for PQC, Phase 8C)
- [ ] **Automatic tunnel lifecycle** — tunnels created on demand when fabric peers are WAN-separated; torn down after idle timeout (configurable, default 30 min); re-established on next message
- [ ] **Multi-hop routing** — if direct tunnel fails (NAT/firewall blocks), route through intermediate fabric nodes; greedy geographic routing based on locality zone
- [ ] **Latency-based path selection** — continuously measure RTT per WAN link; route messages via lowest-latency path; failover to alternate paths on degradation
- [ ] **Bandwidth-aware routing** — estimate available bandwidth per WAN link (via packet timing); avoid routing heavy payloads (agent migration, memory sync bulk) through constrained links (cellular, satellite)
- [ ] **Graceful degradation** — nodes operate independently during WAN partition; queue outbound fabric messages (bounded, with priority eviction); sync on reconnection
- [ ] **WAN interface abstraction** — `WanTransport` trait over: TCP (reliable), UDP (low-latency), WebSocket (firewall-friendly), QUIC (future); auto-select based on network environment

### 18C — WAN-Aware Fabric Scheduling
_The intent scheduler understands WAN characteristics — latency, bandwidth, cost — when placing agents._

- [ ] **Per-link metrics** — fabric heartbeats measure and record: RTT (ms), jitter, packet loss rate, estimated bandwidth; stored in `FabricNode` peer metadata
- [ ] **WAN placement constraints** — `PlacementConstraint::MaxRttMs(50)` — reject nodes with RTT above threshold for latency-sensitive agents; `PreferLocal` — prefer same-zone nodes
- [ ] **Data locality optimization** — prefer placing agents on nodes that already have required data (avoid WAN data transfer); `PlacementConstraint::RequireData(key)` checks persistent memory locality
- [ ] **Cross-region replication policies** — configureable per `MemoryScope`: `Global` entries replicated to all WAN peers (higher bandwidth cost); `Local` stays on-LAN; `Agent` follows agent migration
- [ ] **Bandwidth budgets** — per-WAN-link bandwidth allocation; intent scheduler accounts for agent data transfer needs before placement; prevents bandwidth starvation
- [ ] **Cost-aware scheduling** — WAN links may have metered bandwidth (cellular, cloud egress); fabric scheduler considers cost-per-byte; prefer local compute for cost-sensitive workloads
- [ ] **Tiered sync frequency** — high-bandwidth LAN peers sync every heartbeat (1s); WAN peers sync at reduced frequency (10s–60s) to conserve bandwidth; configurable per-link

### 18D — WAN Security
_WAN links face adversarial networks. Security is non-negotiable._

- [ ] **All WAN traffic PQC-hybrid encrypted** — no exceptions; downgrade to classical-only rejected; fabric nodes without PQC capability cannot join WAN mesh
- [ ] **Certificate pinning** — WAN peers validate each other's certificates against known fabric CA; pin on first successful connection; alert on certificate change (TOFU model for initial join)
- [ ] **DDoS mitigation at fabric edge** — rate-limit inbound tunnel establishment requests per source IP; SYN cookie equivalent for fabric handshake; connection limits per peer
- [ ] **Rate limiting per WAN peer** — per-peer message rate cap (prevents compromised node from flooding); configurable per trust level
- [ ] **Geographic access policies** — optional: restrict fabric membership by IP geolocation, ASN, or configured allow-list; prevents unauthorized foreign node joins
- [ ] **WAN firewall integration** — WAN tunnel traffic passes through packet filter (Phase 13A) like any other interface; per-tunnel firewall rules
- [ ] **Fabric split detection** — detect WAN partition vs node failure; quorum-based (majority partition continues as authoritative; minority enters read-only); auto-heal with consistency reconciliation on rejoin

---

## Phase 19 — Unified Console Abstraction
_One management plane for the entire fleet — from a single ESP32 sensor to a thousand-node cluster. Any node's shell can manage any other node. Distributed logs, fleet-wide commands, and cross-node diagnostics without any central controller. Feature-gated: `console-fabric`._

### 19A — Cross-Node Shell
_`attach` to any fabric node and get a remote shell. Or broadcast commands to the entire fleet._

- [ ] **`attach <node>` command** — open a remote shell session on any fabric node; terminal I/O tunneled over encrypted fabric channel; shell prompt shows remote node: `user@remote-node>`
- [ ] **`detach` command** — return to local shell; remote session persists (background) until explicitly closed or timeout
- [ ] **`broadcast <cmd>` command** — execute command on all fabric nodes; aggregate output with `[node-name]` prefix per line; wait for all responses or timeout
- [ ] **`select <group> <cmd>` command** — execute on node group: `select group:sensors "sysinfo"` — runs on all nodes tagged `group:sensors`; output aggregated
- [ ] **Fabric-aware tab completion** — complete remote node names, remote file paths, remote process names; metadata fetched via lightweight fabric queries
- [ ] **Session multiplexing** — maintain multiple `attach` sessions simultaneously; `sessions list`, `sessions switch <id>`, `sessions close <id>`
- [ ] **Console transport** — remote shell data carried as `FabricMsg::ConsoleData` over encrypted fabric channel; flow control + backpressure
- [ ] **Latency compensation** — for WAN-attached nodes, buffer keystrokes and send in batches; echo prediction; configurable `console.wan_mode` setting

### 19B — Distributed Logs & Observability
_See the entire fleet's kernel logs, processes, and resources from any console._

- [ ] **`dmesg --fabric`** — kernel log messages from all fabric nodes, time-correlated by fabric clock; merge-sorted by timestamp; filter by node with `--node <name>`
- [ ] **`top --fabric`** — cluster-wide resource view: per-node CPU, memory, task count, network I/O; auto-refresh; highlight overloaded nodes
- [ ] **`ps --fabric`** — all processes across all nodes with `[node-name]` prefix; sortable by node, CPU, memory; `kill --node <name> <pid>`
- [ ] **`uptime --fabric`** — uptime, load average, and health status for all nodes; highlight degraded/offline
- [ ] **Log correlation** — cross-reference events by intent ID, agent ID, or timestamp range; `logs --intent <id>` shows related events across all nodes that participated
- [ ] **Real-time event stream** — `events --fabric --follow` — live stream of fabric events (join, leave, agent spawn, intent submit, threat detected) fleet-wide
- [ ] **Distributed `auditlog`** — `auditlog --fabric` — security audit events from all nodes, merged and sortable; essential for security investigations
- [ ] **Log export** — `logs export --format json --since 1h` — export aggregated logs to file or network endpoint (syslog, MQTT, HTTP)

### 19C — Fleet Operations from Console
_Declarative, intent-driven fleet management from the shell._

- [ ] **`intent submit <goal> across <group>`** — fleet-wide intent execution: `intent submit "update firmware to v2.1" across group:edge-nodes` → intent engine decomposes across fabric
- [ ] **`fw --fabric add ...`** — deploy firewall rules fleet-wide: `fw --fabric add INPUT -p tcp --dport 22 -j ACCEPT` → applies to all nodes (or filtered by group)
- [ ] **`update --fabric --staged`** — coordinated firmware update with staging: canary → percentage → full; monitor health between stages
- [ ] **`compliance --fabric status`** — fleet compliance dashboard: per-node compliance score, violations, pending remediations
- [ ] **`diag <node>`** — remote diagnostics: connectivity test (ping fabric + WAN), resource usage, recent errors, security status, compliance state
- [ ] **`snapshot --fabric`** — capture fleet-wide state snapshot: node list, resource usage, running tasks, compliance status, active intents; save to file for analysis

### 19D — Console Federation
_No single point of failure — any node can be the management console._

- [ ] **Peer-to-peer management** — any node with shell access can manage any other node; no designated "management server"; distributed by design
- [ ] **Console session roaming** — start a fleet operation on node A, continue monitoring from node B; session state (attach sessions, log filters, intent tracking) synced via fabric
- [ ] **Console audit trail** — every remote command logged with: who (user), from where (source node), to where (target node/group), what (command), when (timestamp); immutable log in security audit (Phase 8F)
- [ ] **Role-based console access** — `ConsoleRole` enum: `Admin` (full fleet ops), `Operator` (view + restart + update), `Viewer` (read-only logs + status); enforced by capabilities
- [ ] **Console bandwidth management** — for constrained WAN links, console traffic deprioritized vs operational fabric traffic; configurable QoS class for management data

### 19E — Structured Event System (Events > Logs)
_Every kernel action emits a typed, structured event — not a string log. Events are THE primitive for logs, metrics, traces, and security signals. All console/observability surfaces consume the same event stream._

- [ ] **`KernelEvent` typed primitive** — `KernelEvent { timestamp, node_id, event_type: EventType, source, payload }` — replaces string-based `klog` as the fundamental observability unit
- [ ] **`EventType` enum** — `Log(level, module, msg)`, `Metric(name, value, tags)`, `Trace(span_id, parent, operation, latency)`, `Security(threat_level, category, detail)`, `Lifecycle(component, state_change)` — every event is typed, not a string
- [ ] **Event bus** — kernel-internal ring buffer of `KernelEvent`; all subsystems emit events into the bus; zero-allocation for hot-path events (pre-allocated slots)
- [ ] **Multi-sink fanout** — event bus distributes to multiple sinks simultaneously:
  - `Serial` — last-resort reliable sink for panic/boot events; priority-filtered (panic > error > info)
  - `VGA/Framebuffer` — rich local visualization; color-coded by event type
  - `Network` — fabric-distributed event streaming to remote consoles
  - `Storage` — persistent event log on SD/NVMe for post-mortem analysis
  - `Memory` — episodic memory ring buffer (Phase 14C) for AI agent consumption
- [ ] **Priority-aware delivery** — events have priority levels: `Panic` > `Security` > `Error` > `Metric` > `Debug`; low-priority events dropped under backpressure; panic events guaranteed delivery to all sinks
- [ ] **Event filtering** — per-sink filter: `{ min_level: Info, event_types: [Log, Security], source_nodes: [node1, node2] }` — subscribers see only what they need
- [ ] **Real-time streaming** — `events --follow --type trace,security` — live event stream with typed filtering; replaces `dmesg -w` with structured queries
- [ ] **Event query language** — simple query syntax: `events where type=Security AND threat_level > Medium since 1h` — evaluates against in-memory ring buffer; no external query engine
- [ ] **Event export** — `events export --format json|cbor --since 1h --to syslog://remote:514` — structured export to external systems; CBOR for bandwidth-constrained links
- [ ] **Shell `events` command** — `events [--follow] [--type <type>] [--node <name>] [--since <duration>] [--fabric]` — replaces ad-hoc log grep with typed event queries

---

## Phase 20 — ZeroServices Architecture & API Gateway Elimination
_Services are kernel objects. Not containers, not processes with sidecars. The kernel provides service identity, routing, mTLS, load balancing, and observability natively. This eliminates the entire service mesh + API gateway stack. Feature-gated: `zero-svc`._

### 20A — ZeroServices Model
_The fundamental shift: services are first-class kernel primitives, not userspace infrastructure._

- [ ] **Service as kernel object** — `KernelService { id, name, owner_process, capability_set, health, version, endpoints, metrics }` — managed by kernel, not by a container runtime
- [ ] **Service identity = capability token** — each service has a unique `ServiceCapability(svc_id)` token; all access requires presenting valid capability; no ambient authority
- [ ] **Kernel-native routing** — service-to-service calls dispatched by kernel: caller invokes `SYS_SVC_CALL(name, msg)` → kernel resolves name → routes to healthy instance → delivers response; zero sidecar, zero proxy
- [ ] **Built-in mTLS** — all service-to-service communication encrypted with per-service session keys (Phase 8E); kernel auto-provisions and rotates certificates; zero configuration
- [ ] **Built-in load balancing** — kernel distributes requests across service instances using configurable strategy; no external LB needed
- [ ] **Built-in observability** — kernel records per-service metrics (request count, latency histogram, error rate, active connections) in ring buffer; queryable via `SYS_SVC_METRICS`; no Prometheus sidecar needed
- [ ] **Service lifecycle** — `register → healthy → serving → draining → deregistered`; health checks and lifecycle managed by kernel; restart policy per-service
- [ ] **Intent-driven service management** — `intent submit "deploy service payments with 3 replicas"` → intent engine (Phase 14B) handles placement, scaling, health monitoring

### 20A′ — Function Invocation Model (Invoke > Endpoint)
_Beyond services to direct function invocation. No URLs, no endpoints — everything is `invoke("name.function", payload)`. Functions are ephemeral, versioned, and placement-independent._

- [ ] **`SYS_INVOKE` (0xD6)** — `invoke("auth.login", payload)` → kernel resolves function → routes to handler (local process, remote node, or WASM sandbox) → returns result; single-call replaces HTTP request lifecycle
- [ ] **Function registry** — functions registered as named entry points within services or standalone: `register_fn("auth.login", handler_fn)` — discoverable by name across fabric
- [ ] **Ephemeral execution** — functions execute, return, and release resources; no long-running server processes required; kernel manages handler pool
- [ ] **Versioned functions** — `invoke("auth.login@v2", payload)` routes to specific version; traffic splitting between versions for canary deploys
- [ ] **Identity-based routing** — routing decisions based on caller identity + callee name, NOT network addresses; no DNS, no IP-based routing; `NodeId + FunctionId` globally unique
- [ ] **Event triggers** — functions invoked automatically on events: `on_event("user.created", "email.welcome")` — kernel event bus triggers downstream functions
- [ ] **Short-lived identity tokens** — per-invocation identity tokens with configurable TTL (default: 30s); auto-rotated; replaces long-lived API keys and service account credentials; goes beyond SPIFFE model
- [ ] **Distributed quota enforcement** — per-function, per-caller quotas enforced at kernel level across all fabric nodes; gossip-replicated counters; prevents abuse without central rate limiter

### 20B — Service Registration & Discovery
_Services announce themselves to the kernel. Discovery is a syscall, not a DNS query._

- [ ] **`SYS_SVC_REGISTER` (0xD0)** — register a service: name, version, capabilities required, health check endpoint; returns service handle; capability-gated
- [ ] **`SYS_SVC_DEREGISTER` (0xD1)** — graceful deregistration with drain period (serve in-flight requests, reject new)
- [ ] **`SYS_SVC_DISCOVER` (0xD2)** — find services by name, capability, version constraint; returns list of healthy endpoints; `discover("payments", version >= "2.0")`
- [ ] **Gossip-replicated service registry** — service registrations propagated across fabric nodes via gossip (Phase 11A/15E); discovery works for local AND remote services transparently
- [ ] **Health-integrated discovery** — only healthy service instances returned by `SYS_SVC_DISCOVER`; unhealthy instances auto-removed from registry after failed health checks
- [ ] **Version-aware routing** — multiple versions of the same service can coexist; traffic split by policy: canary (5% to v2, 95% to v1), blue-green (instant switch), header-based (test version via header)
- [ ] **Service dependency tracking** — kernel tracks which services call which; dependency graph queryable for debugging and impact analysis

### 20C — Kernel-Native Load Balancing & Resilience
_The kernel IS the load balancer, circuit breaker, and retry engine._

- [ ] **Load balancing strategies** — per-service configurable: `RoundRobin`, `LeastConnections`, `WeightedRandom`, `LatencyBased`, `ConsistentHash(key)`; default: `LeastConnections`
- [ ] **Circuit breaker** — per-service failure tracking: open circuit after N failures in window → fast-fail for configurable period → half-open probe → close on success; prevents cascade failures
- [ ] **Retry with backoff** — automatic retry on transient failures: configurable max retries, initial delay, backoff multiplier, max delay; jitter to prevent thundering herd
- [ ] **Timeout enforcement** — per-service and per-request timeout at kernel level; caller gets `ETIMEDOUT` without guessing; no hung connections
- [ ] **Bulkhead isolation** — per-service connection limits; prevent one service's load from starving others; configurable max concurrent requests
- [ ] **Graceful degradation** — if all instances of a service are unhealthy, return cached response (if available) or clear error (not hang); configurable fallback per-service

### 20D — API Gateway Elimination
_External clients connect directly to fabric edge nodes. The kernel handles auth, rate limiting, and routing — no gateway tier._

- [ ] **`SYS_SVC_EXPOSE` (0xD3)** — expose an internal service on a public port with access policy: `expose("payments", port=443, auth=ZTNA, rate_limit=1000/s)` — single syscall replaces entire API gateway
- [ ] **External authentication** — fabric edge nodes (public-facing) validate external client identity: mTLS client certificates, JWT verification (RS256/Ed25519), API key validation, OAuth2 token introspection — all kernel-native
- [ ] **Rate limiting** — per-client, per-service, per-API-key token bucket rate limiter enforced at kernel level; `429 Too Many Requests` response; configurable burst
- [ ] **Request routing** — external request hits fabric edge → kernel resolves service name from Host/path/SNI → routes to healthy instances across fabric; no reverse proxy needed
- [ ] **Protocol translation** — kernel-level protocol bridge: HTTP → kernel IPC (for simple services), gRPC → kernel channels (for streaming), WebSocket → kernel channels (for real-time)
- [ ] **TLS termination at edge** — fabric edge nodes terminate external TLS (Phase 8E); forward to internal services over kernel IPC or fabric mTLS; ACME/Let's Encrypt auto-renewal
- [ ] **CORS handling** — configurable Cross-Origin headers per exposed service; no application code changes needed
- [ ] **Response caching** — kernel-level response cache for idempotent GET requests; configurable TTL per-service; cache invalidation via service notification
- [ ] **Intent-driven exposure** — `intent submit "expose service payments on port 443 with rate-limit 1000/s and JWT auth"` → intent engine configures everything

### 20E — ZeroServices Observability
_Every service call is measured by the kernel — no instrumentation needed._

- [ ] **Per-service metrics** — kernel automatically records for every service: request count, latency (P50/P95/P99), error count, error rate, active connections, bytes transferred; queryable via `SYS_SVC_METRICS` (0xD5)
- [ ] **Distributed tracing** — kernel auto-injects trace context (W3C Trace Context) into service-to-service calls; span collection without any application code
- [ ] **Service dependency map** — kernel builds real-time service call graph from observed traffic; `svc graph` shell command shows dependencies
- [ ] **Health dashboard** — `svc status` shows all services: healthy instances, request rate, error rate, latency; `svc status --fabric` for fleet-wide view
- [ ] **Alerting** — threshold-based alerts: `if payments.error_rate > 0.05 for 5m → alert`; delivered via console, fabric event, or webhook

### 20F — Shell Integration
- [ ] **`svc` command** — `svc list`, `svc register <name>`, `svc discover <name>`, `svc status [--fabric]`, `svc expose <name> <port>`, `svc graph`, `svc metrics <name>`, `svc health <name>`
- [ ] **`intent submit` integration** — `intent submit "deploy service X with 3 replicas and expose on port 443"` → full lifecycle managed by intent engine

### 20G — What ZeroServices Eliminates
```
Traditional Stack                → VeerOS ZeroServices
─────────────────────────────────────────────────────────────
Kubernetes Pods + Deployments    → Kernel service objects
Docker / containerd runtime      → Kernel service lifecycle
Istio / Linkerd sidecar proxies  → Kernel-native mTLS + routing
Envoy data plane                 → Kernel IPC + fabric transport
Consul / CoreDNS service disc.  → Gossip-replicated service registry
Kong / Nginx / Envoy API GW     → ZTNA edge + kernel rate limiting
APISIX / Traefik ingress        → SYS_SVC_EXPOSE syscall
Prometheus + Grafana metrics     → Kernel ring buffer metrics
Jaeger / Zipkin tracing          → Kernel auto-injected trace context
cert-manager TLS provisioning    → Kernel keystore + auto-rotation
etcd / Consul config store       → Fabric-replicated persistent memory
Kubernetes Service + Endpoints   → Kernel service + health integration
PodDisruptionBudget / HPA        → Kernel circuit breaker + intent scaling
Service account + RBAC           → Capability-based service identity

Total components eliminated: ~15 infrastructure services → 0 sidecars, 0 proxies, 0 gateways
```

---

## Phase 21 — State Fabric (Distributed Data Plane)
_Global, distributed, service-less state layer. Compute is stateless; state lives in the fabric. Replaces databases, caches, queues, and config stores with a unified kernel-native data plane. Feature-gated: `state-fabric`._

### 21A — State Primitives
_Three data models — key-value, streams, and objects — all locality-aware and persistent._

- [ ] **Key-value store** — `SYS_STATE_PUT(key, value, opts)` / `SYS_STATE_GET(key)` — distributed KV extending persistent memory (Phase 14C) with strong consistency options; replaces Redis, etcd, Consul KV
- [ ] **Stream primitive** — `SYS_STATE_STREAM_WRITE(stream, event)` / `SYS_STATE_STREAM_READ(stream, offset)` — append-only distributed event streams; replaces Kafka topics and message queues; bounded or unbounded; consumer groups with offset tracking
- [ ] **Object store** — `SYS_STATE_OBJ_PUT(key, data, metadata)` / `SYS_STATE_OBJ_GET(key)` — large binary blobs stored across fabric nodes; content-addressed (SHA-256); replaces S3/MinIO for local data
- [ ] **TTL + eviction** — per-key TTL; LRU eviction when memory budget exceeded; configurable per data class
- [ ] **Namespaced state** — state keys scoped to service identity: `payments:customer:123` — namespace isolation via capabilities
- [ ] **Transactions (mini)** — `SYS_STATE_TXN(ops[])` — atomic multi-key operations within a single partition; no cross-partition transactions (keep simple)

### 21B — Replication & Consistency
_Locality-aware replication with tunable consistency — from strong to eventual._

- [ ] **Consistency levels** — per-operation configurable: `Strong` (linearizable, quorum write/read), `Session` (read-your-writes within session), `Eventual` (AP, fastest), `Causal` (vector-clock ordered)
- [ ] **CRDT support** — conflict-free replicated data types for eventual consistency: counters (G-Counter, PN-Counter), sets (OR-Set), registers (LWW-Register, MV-Register), maps (OR-Map); automatic merge on partition heal
- [ ] **Locality-aware placement** — state replicated to nodes based on access patterns: hot keys migrate toward consumers; configurable replication factor (1–N)
- [ ] **Offline tolerance** — nodes accumulate writes during partition; CRDT-merge on reconnection; no data loss; designed for DIL (disconnected, intermittent, limited) environments
- [ ] **Anti-entropy protocol** — background Merkle-tree comparison between replicas; detect and repair divergence; configurable sync interval
- [ ] **Partition-aware operations** — during network partition, `Strong` reads fail fast; `Eventual` reads serve local state; operations tagged with partition context for later reconciliation

### 21C — State Fabric Integration
- [ ] **Stateless compute + persistent state** — agents and functions read/write state via State Fabric; execution is ephemeral, state survives; clean functional model
- [ ] **State triggers** — `on_state_change("key_pattern", function)` — state mutations trigger function invocations (Phase 20A′); enables reactive patterns without polling
- [ ] **State-aware scheduling** — intent scheduler considers state locality when placing agents; co-locate compute with its state to minimize latency
- [ ] **State migration** — hot state follows workload migration; pre-fetch state to destination node before agent migration; lazy migration for cold keys
- [ ] **Shell `state` commands** — `state get <key>`, `state put <key> <value>`, `state stream <name> [--follow]`, `state info` (replication status, partition health, key count)

---

## Phase 22 — Fabric Protocol & Transport
_Binary, zero-copy, QUIC-native protocol for all fabric communication. Every fabric message — invocations, state sync, events, management — uses the same wire protocol. Feature-gated: `fabric-proto`._

### 22A — Wire Protocol (IFP — InterFabric Protocol)
_Compact binary framing for minimal overhead on constrained links. IFP is the unified wire protocol for intra-fabric and inter-fabric communication._

- [ ] **IFP common header** — fixed + varint fields: `{ version: u8, msg_type: u8, flags: u16, header_len: varint, invocation_id: [u8; 16] (UUID), trace_id: [u8; 16], timestamp: varint (µs since epoch), body_len: varint, extensions: TLV[] }` — all messages share this header for tracing and correlation
- [ ] **Message types** — 8 core types: `INVOKE (0x01)` (function call), `DATA (0x02)` (streaming chunks, bidirectional), `RESULT (0x03)` (success response), `ERROR (0x04)` (failure with structured code), `CANCEL (0x05)` (abort invocation), `HEARTBEAT (0x06)` (keepalive), `POLICY_HINT (0x07)` (early permission feedback for UX), `FEDERATION (0x08)` (cross-fabric envelope)
- [ ] **INVOKE body** — CBOR-encoded: `{ function: "auth.login", version: "v1", args: {…}, caller: { id, fabric }, capabilities: ["auth.invoke"], deadline_ms: 5000, priority: P0-P3, target: { fabric, constraints: { region, latency_ms } }, idempotency_key: optional }`
- [ ] **RESULT body** — `{ status: "OK", result: {…}, metrics: { latency_ms, node } }` — small results inline; large results streamed via DATA frames
- [ ] **ERROR body** — `{ code: "POLICY_DENIED|NOT_FOUND|TIMEOUT|RESOURCE_EXHAUSTED|FABRIC_UNAVAILABLE", message, retryable: bool, details: {…} }` — structured error codes, not HTTP status
- [ ] **DATA streaming** — `{ seq: u32, chunk: [u8], eof: bool }` — ordered per QUIC stream; backpressure via QUIC flow control; used for large payloads and live subscriptions
- [ ] **Identity block (TLV extension)** — mandatory on INVOKE: `{ sig_alg: "ed25519", signature: [u8; 64], cert_chain: [leaf, intermediate], claims: { sub, fabric, exp, caps } }` — detached signature over `hash(header + body)`; verified before execution
- [ ] **Priority & QoS** — header flags encode priority: `P0` (panic/control), `P1` (user actions), `P2` (background), `P3` (debug); fabric can preempt and shed load (drop P3 first)
- [ ] **Subscription pattern** — streaming invocation: `INVOKE(subscribe) → continuous DATA(events) → END on cancel`; replaces WebSockets/SSE
- [ ] **Serialization** — CBOR (default, self-describing, compact); optional protobuf for schema-defined services; `zstd` compression flag for large DATA frames
- [ ] **Zero-copy forwarding** — intermediate relay nodes forward frames without deserializing payload; only header inspected for routing
- [ ] **Fragmentation** — large messages fragmented into MTU-sized frames; reassembled at receiver; ordered delivery guarantee within session

### 22B — QUIC Transport
_QUIC as primary transport — multiplexed, encrypted, NAT-friendly, zero-RTT._

- [ ] **QUIC implementation** — `no_std` QUIC 1.0 (RFC 9000) in kernel; UDP-based; built-in TLS 1.3; connection migration; 0-RTT resumption
- [ ] **Multiplexed streams** — each message type on its own QUIC stream; no head-of-line blocking between invocations and state sync; priority-based scheduling
- [ ] **Connection migration** — QUIC connections survive IP address changes (cellular handoff, roaming, VM migration); connection ID-based, not IP-based
- [ ] **0-RTT resumption** — for known peers, first message sent with 0-RTT; cached session tickets in persistent memory; reduces reconnection latency to zero
- [ ] **Congestion control** — BBR or CUBIC congestion control per connection; WAN links auto-detected; ECN support
- [ ] **Transport selection** — `FabricTransport` trait: QUIC (default + preferred), TCP (fallback for QUIC-blocked networks), UDP (raw, for time-critical), WebSocket (firewall-friendly HTTP fallback); auto-negotiated per peer

### 22C — Decentralized Scheduling
_No central orchestrator — scheduling decisions peer-coordinated via gossip and local autonomy._

- [ ] **Peer-coordinated placement** — each node computes placement scores locally using gossip-derived global view; nodes bid for workloads; highest-scoring node wins; no leader required for placement decisions
- [ ] **Gossip-based load sharing** — each node periodically advertises: current load, available capacity, capabilities, latency to peers; all nodes maintain approximate global view
- [ ] **Cost/energy-aware scoring** — placement score includes energy cost (battery level for IoT, power draw for servers) and monetary cost (cloud instance pricing); configurable weight per factor
- [ ] **Local-first execution** — prefer local execution when constraints allow; remote placement only when local resources insufficient or policy requires it
- [ ] **Graceful degradation on partition** — during network partition, each partition schedules independently using last-known state; workload redistribution on partition heal
- [ ] **Scheduling protocol** — `FabricMsg::ScheduleRequest(workload)` → nodes reply with `ScheduleBid(score, constraints_met)` → requester selects winner → `ScheduleAccept(node_id)` — 3-message handshake

---

## Phase 23 — Developer SDK & Adoption Path
_How developers write applications for VeerOS, and how organizations migrate from existing infrastructure. Feature-gated: `sdk`._

### 23A — Developer SDK
_High-level SDK for building VeerOS applications — from functions to full distributed systems._

- [ ] **`veeros-sdk` crate** — high-level Rust SDK wrapping syscalls: `veeros::invoke("auth.login", &payload)`, `veeros::state::get("key")`, `veeros::events::emit(event)` — ergonomic API over raw syscalls
- [ ] **Function definition macro** — `#[veeros::function] fn login(req: LoginRequest) -> LoginResponse { ... }` — generates registration, serialization, capability declarations
- [ ] **State bindings** — `veeros::state::kv::<T>(key)` typed KV access; `veeros::state::stream::<T>(name)` for event streams; compile-time type safety
- [ ] **Event emission** — `veeros::event!(Level::Info, "user logged in", user_id = id)` — structured event macro with compile-time type checking; replaces `println!` / `log::info!` patterns
- [ ] **Service composition** — `veeros::service("payments").version("2.0").replicas(3).expose(443).deploy()` — declarative service definition in code
- [ ] **Testing harness** — `veeros::test::fabric()` — in-process simulated fabric for unit testing functions without real hardware; mock state, mock events, mock invocations
- [ ] **CLI toolchain** — `veeros-cli`: `veeros build` (cross-compile), `veeros deploy` (push to fabric), `veeros invoke` (test from host), `veeros logs` (stream events), `veeros status` (fleet health)

### 23B — WASM & Polyglot Support
_Write VeerOS functions in any language that compiles to WASM._

- [ ] **WASM SDK** — `veeros-wasm` host import bindings: `invoke`, `state_get`, `state_put`, `event_emit`, `identity` — callable from any WASM-capable language
- [ ] **Rust → WASM** — `cargo build --target wasm32-wasi` + `veeros deploy` — zero-config path from Rust source to running function on fabric
- [ ] **C/C++ → WASM** — Emscripten / clang WASI target; header file `veeros.h` wrapping host imports
- [ ] **TinyGo → WASM** — Go functions compiled to WASM via TinyGo; `veeros-go` package wrapping host imports
- [ ] **AssemblyScript → WASM** — TypeScript-like language compiled to WASM; `@veeros/sdk` npm package
- [ ] **Module size budgets** — per-target WASM module size limits: ESP32 (64 KB), RPi (1 MB), x86 (10 MB); compile-time validation

### 23C — Migration Path from Existing Systems
_Incremental adoption — VeerOS runs alongside existing infrastructure, replacing components one at a time._

- [ ] **HTTP ↔ Invoke bridge** — edge proxy translates `POST /api/auth/login` → `invoke("auth.login", body)` — existing clients keep HTTP, VeerOS functions don't need HTTP awareness
- [ ] **Kubernetes sidecar mode** — VeerOS agent runs as a K8s sidecar; intercepts service-to-service calls; routes via fabric when both endpoints are VeerOS; passthrough otherwise; gradual migration
- [ ] **State bridge** — `veeros-bridge-redis` reads/writes from both Redis and State Fabric; dual-write during migration; cut over when ready; similar bridges for etcd, PostgreSQL
- [ ] **Event bridge** — `veeros-bridge-kafka` consumes Kafka topics → emits VeerOS events (and vice versa); enables mixed-infrastructure event pipelines
- [ ] **Prometheus exporter** — expose VeerOS metrics in Prometheus format on `/metrics` endpoint; existing Grafana dashboards keep working during migration
- [ ] **OCI container import** — run existing Docker/OCI containers inside VeerOS container isolation (Phase 8B); gradual refactor to WASM/native functions
- [ ] **Migration playbook** — step-by-step guide: (1) deploy VeerOS edge nodes alongside existing, (2) bridge state/events, (3) migrate functions one by one, (4) cut over networking, (5) decommission old infrastructure

---

## Phase 24 — InterFabric Protocol (IFP) / VeerLink
_Cross-fabric federation — independent VeerOS fabrics communicate without collapsing isolation or reintroducing gateways. InterFabric is NOT networking; it is federated invocation between trust domains. Each fabric retains its own identity system, policy engine, and execution fabric. Feature-gated: `interfabric`._

_Naming convention: **InterFabric** (architecture concept), **IFP** (wire protocol), **VeerLink** (product layer), **Fabration** (internal codename)._

### 24A — Fabric Identity & Trust Model
_Each fabric is a cryptographic trust domain with its own root identity. Federation is explicit, never implicit._

- [ ] **Fabric root identity** — each fabric has a unique cryptographic identity: `FabricId { name: "veer.prod.india", public_key: Ed25519+ML-DSA, cert_chain }` — like cluster identity but at federation level
- [ ] **Fabric URI scheme** — `fabric://veer.prod.india`, `fabric://partner.analytics.eu` — globally unique, human-readable, DNS-independent addressing
- [ ] **Fabric certificate authority** — each fabric has its own root CA; federation does NOT share CAs; trust established via explicit certificate exchange during federation handshake
- [ ] **Trust contracts** — `FederationTrust { peer_fabric: FabricId, trust_level: TrustLevel, established: Timestamp, expires: Option<Timestamp>, constraints: PolicySet }` — explicit, bilateral, time-bounded
- [ ] **Trust levels** — `Untrusted` (no federation), `Verified` (identity verified, policy-controlled), `Trusted` (mutual trust, broader access), `Allied` (deep integration, shared state possible)
- [ ] **Trust revocation** — instant revocation of federation trust: `SYS_FEDERATION_REVOKE(fabric_id)` → all active channels terminated, pending invocations cancelled, trust entry removed; takes effect in < 1 second across all nodes
- [ ] **No implicit trust** — even federated fabrics have zero ambient authority; every cross-fabric invocation evaluated against both sender and receiver policy independently

### 24B — Federation Handshake Protocol
_Step-by-step protocol for establishing inter-fabric trust. Designed for hostile network environments._

- [ ] **Discovery** — fabric entry points advertised via: (1) manual configuration (`federation add fabric://partner.analytics.eu at 203.0.113.1:4433`), (2) DNS SRV records (`_interfabric._quic.partner.analytics.eu`), (3) well-known HTTPS endpoint (`.well-known/interfabric`)
- [ ] **Handshake step 1: Hello** — initiator sends `IFP_HELLO { fabric_id, public_key, supported_versions, capabilities }` — no secrets exchanged yet
- [ ] **Handshake step 2: Challenge** — responder verifies initiator's public key against pre-shared trust anchor; sends `IFP_CHALLENGE { nonce, responder_fabric_id, responder_public_key }`
- [ ] **Handshake step 3: Prove** — initiator signs `{ nonce, initiator_fabric_id, responder_fabric_id, timestamp }` with fabric private key; sends `IFP_PROVE { signature, trust_proposal }`
- [ ] **Handshake step 4: Accept** — responder verifies signature; evaluates trust proposal against local policy; sends `IFP_ACCEPT { mutual_signature, session_key_material, trust_contract }` — session established
- [ ] **Session key derivation** — ML-KEM + X25519 hybrid key exchange → per-session ChaCha20-Poly1305 keys; forward secrecy; key rotation every 24h or 1M messages
- [ ] **Handshake failure modes** — `IFP_REJECT(reason)`: `UntrustedFabric`, `PolicyDenied`, `VersionMismatch`, `CertificateExpired`, `RateLimited` — clear error semantics
- [ ] **Re-federation** — periodic re-handshake to refresh trust and rotate keys; configurable interval (default 24h); zero-downtime re-keying

### 24C — Identity Federation
_Cross-fabric identity translation — never expose raw internal identities to external fabrics._

- [ ] **Scoped identity tokens** — per-invocation, short-lived, scoped: `ScopedToken { caller: "frontend.app", fabric: "veer.prod.india", claims: { role: "analytics-client", trust_level: "verified" }, expires: Timestamp, signature }` — signed by source fabric's root key
- [ ] **Identity translation** — source fabric maps internal identity to federated claims; receiving fabric maps federated claims to local authorization decisions; no shared identity namespace
- [ ] **Claim format** — compact binary token (CBOR-encoded, not JWT) for wire efficiency: `{ iss: FabricId, sub: CallerId, aud: TargetFabricId, iat: u64, exp: u64, claims: Map<String, Value>, sig: [u8; 64] }`
- [ ] **Claim verification** — receiving fabric verifies: (1) signature against source fabric's known public key, (2) expiry, (3) audience matches self, (4) claims satisfy local policy — all in kernel, no external token service
- [ ] **Identity non-leakage** — internal identities (process IDs, thread IDs, internal service names) NEVER cross fabric boundary; only federated claims visible to peers
- [ ] **Delegation chains** — Fabric A invokes Fabric B which invokes Fabric C: delegation chain tracked in token; each hop adds its identity; final receiver sees full call chain for audit
- [ ] **Identity caching** — verified fabric public keys cached in persistent memory with TTL; avoid re-verification on every invocation; cache invalidated on trust revocation

### 24D — Cross-Fabric Invocation
_Federated function invocation — `invoke("analytics.process", payload, { target: "fabric://partner.analytics.eu" })`._

- [ ] **`SYS_INVOKE_REMOTE` (0xE0)** — `invoke("function.name", payload, target_fabric)` → kernel routes through federation layer → scoped identity attached → secure channel → remote fabric evaluates → executes → returns result
- [ ] **Invocation lifecycle** — `Submitted → PolicyCheck → IdentityScoped → Transmitted → RemoteReceived → RemotePolicyCheck → Executing → Completed/Failed` — full state machine with timeout at each stage
- [ ] **Synchronous invocation** — request/response; caller blocks (or async-awaits) until remote fabric returns result; timeout configurable per-invocation (default 30s)
- [ ] **Asynchronous invocation** — fire-and-forget with optional callback: `invoke_async("analytics.ingest", payload, target_fabric, callback: "ingest.complete")` — result delivered via event when ready
- [ ] **Cross-fabric event bridging** — Fabric A emits structured events → federation bridge → Fabric B consumes as local events; configurable event filters at federation boundary; useful for analytics, async workflows, decoupling
- [ ] **Batch invocation** — `invoke_batch("analytics.process", payloads[], target_fabric)` — amortize federation overhead for bulk operations; kernel batches into single wire message
- [ ] **Invocation routing** — source fabric selects nearest gateway node of target fabric based on latency + health; NOT DNS-based; fabric registry maps FabricId → reachable entry nodes
- [ ] **Fallback modes** — if synchronous path fails: automatic retry → alternate gateway node → degrade to async → queue for later delivery (configurable per policy)

### 24E — Inter-Fabric Policy Engine
_Each fabric enforces its own policy independently. No implicit trust — even between federated fabrics._

- [ ] **Outbound policy** — source fabric evaluates before sending: `{ allow: true, if: { target.fabric: "partner.analytics.eu", caller.role: "analytics-client", action: "analytics.*" } }` — can we call this external fabric?
- [ ] **Inbound policy** — receiving fabric evaluates on arrival: `{ allow: true, if: { source.fabric: "veer.prod.india", caller.claims.role: "analytics-client", action: "analytics.process" } }` — do we accept this call?
- [ ] **Policy DSL** — declarative policy language for inter-fabric rules:
  ```
  federation policy "allow-analytics" {
    when source.fabric == "veer.prod.india"
    and  caller.role in ["analytics-client", "admin"]
    and  action matches "analytics.*"
    then allow
    audit always
  }
  ```
- [ ] **Policy evaluation order** — deny-by-default; explicit allow rules required; most-specific rule wins; audit trail for every decision
- [ ] **Rate limiting** — per-fabric, per-function rate limits at federation boundary; distributed quota enforcement; prevents cross-fabric abuse
- [ ] **Data governance** — payload inspection rules: `{ block_if: payload.contains("PII") }` — prevent sensitive data from crossing fabric boundaries; configurable per trust level
- [ ] **Policy sync** — bilateral policy negotiation during federation handshake; each side declares what it offers and what it accepts; incompatible policies = federation rejected

### 24F — Shared State & Data Exchange
_Controlled state sharing between fabrics — from zero sharing to selective replication._

- [ ] **No shared state (default)** — federated fabrics share NOTHING by default; all data exchange is explicit invocation; no ambient state leakage
- [ ] **Selective state replication** — opt-in: `state share "analytics.results" with fabric://partner.analytics.eu read-only` — specific keys/streams replicated to peer fabric; policy-controlled, audit-logged
- [ ] **State materialization** — cross-fabric state queries materialized as local read-only copies: `state get "fabric://partner/analytics.results.latest"` → cached locally with TTL; invalidation via federation events
- [ ] **Cross-fabric streams** — Fabric A produces stream events → federation bridge → Fabric B consumes as local stream; bounded buffer at boundary; backpressure propagated
- [ ] **Data boundary enforcement** — all shared state tagged with governance metadata: `{ classification: "internal", retention: "30d", jurisdictions: ["IN"] }` — receiving fabric must honor governance constraints

### 24G — Inter-Fabric Routing & Transport
_Fabric-to-fabric connectivity without reintroducing networking abstractions._

- [ ] **Fabric registry** — `FabricRegistry { fabric_id → [EntryNode { addr, latency, health, capabilities }] }` — maps fabric identities to reachable entry points; gossip-updated within each fabric; manually configured or discovered for external fabrics
- [ ] **Federation entry nodes** — designated nodes at fabric edge that accept incoming federation connections; identity enforcement at boundary; NOT API gateways — no business logic, only identity + policy + routing
- [ ] **Latency-aware federation routing** — select entry node with lowest RTT; continuous measurement; failover to alternate entry nodes on degradation
- [ ] **Relay federation** — Fabric A cannot directly reach Fabric C but can reach Fabric B which can reach C: `A → B(relay) → C` — end-to-end encrypted (B sees only ciphertext); relay trust explicitly configured
- [ ] **IFP wire format** — extends FabricFrame (Phase 22A) with federation fields: `IFP_Frame { fabric_frame_header, source_fabric: FabricId, target_fabric: FabricId, scoped_token: [u8], payload }` — compatible with intra-fabric protocol
- [ ] **Transport** — IFP runs over QUIC (Phase 22B); federation sessions multiplexed; 0-RTT for established federations; connection migration supported

### 24H — Cross-Fabric Observability
_End-to-end visibility across fabric boundaries without exposing internal details._

- [ ] **Cross-fabric trace context** — trace IDs propagated across federation boundary; each fabric adds its own spans; end-to-end latency visible; internal spans NOT exported (only summary)
- [ ] **Federation events** — `KernelEvent { type: Federation, fabric_id, action, result, latency }` — every cross-fabric invocation emits a structured event; queryable via `events --type federation`
- [ ] **Cross-fabric metrics** — per-peer-fabric metrics: invocation count, latency (P50/P95/P99), error rate, bytes transferred; exposed via `federation status`
- [ ] **Federation audit log** — immutable audit trail: every cross-fabric invocation logged with: source fabric, caller identity, target function, policy decision, result, timestamp; tamper-evident (hash-chained)
- [ ] **Cross-fabric health** — periodic heartbeat between federated fabrics; health status visible in `federation status`; auto-degrade to async mode on latency spike

### 24I — Shell Integration
- [ ] **`federation` command** — `federation list`, `federation add <fabric_uri> [at <addr>]`, `federation remove <fabric_id>`, `federation status [<fabric_id>]`, `federation trust <fabric_id> <level>`, `federation revoke <fabric_id>`
- [ ] **`invoke --fabric` flag** — `invoke "analytics.process" --fabric partner.analytics.eu --payload '{"data": "..."}'` — cross-fabric invocation from shell
- [ ] **`events --fabric-id`** — filter events by source/target fabric: `events --fabric-id partner.analytics.eu --type federation`
- [ ] **`state --federated`** — `state list --federated` shows shared state; `state share <key> with <fabric>` manages sharing

### 24J — Failure & Security Model
- [ ] **Fabric compromise response** — if peer fabric is suspected compromised: `federation revoke <fabric_id>` → all channels severed instantly; pending invocations cancelled; queued events purged; trust entry marked `Revoked` with timestamp; event emitted fleet-wide
- [ ] **Network partition** — federation retries via alternate entry nodes; degrades to async mode; queues outbound invocations (bounded, priority-evicted); resumes on reconnection
- [ ] **Policy mismatch** — invocation rejected at boundary with clear error: `FederationError::PolicyDenied { reason, policy_id }` — no silent failures
- [ ] **Replay protection** — per-session monotonic sequence numbers; nonce in every invocation; duplicate detection window (5 min default)
- [ ] **DDoS protection** — per-fabric rate limits at entry nodes; proof-of-work challenge for new federations; established federations exempt

### 24K — What InterFabric Eliminates
```
Traditional Inter-Org Stack          → VeerOS InterFabric (IFP / VeerLink)
─────────────────────────────────────────────────────────────────────────
API Gateways (both sides)            → Federation entry nodes (identity only)
OAuth2 / OIDC token services         → Scoped identity tokens (kernel-signed)
VPN tunnels between orgs             → IFP over QUIC (identity-based, not network)
DNS-based service discovery          → Fabric registry (cryptographic identity)
Shared network / VPC peering         → Zero shared network (invocation only)
B2B integration middleware           → Direct cross-fabric invocation
API versioning infrastructure        → Versioned function identity
Cross-org observability stitching    → Federation trace context propagation
Trust management / PKI overhead      → Federation handshake + trust contracts

Result: Inter-org communication without API gateways, VPNs, or shared networks.
```

---

## Phase 25 — Fabric Client / VeerFlow (Next-Generation User Interface)
### 25M — veer-connect (Secure Connect)
- [x] Secure shell and file transfer (push/pull) for VeerOS nodes
- [x] X25519 handshake, VSC protocol, encrypted session
- [x] CLI (Rust) and Python client (scripts/veeros-connect)
- [x] Used for ESP32C6 remote shell and file access
_The browser is dead. VeerOS replaces it with a fabric-native runtime that binds directly to `invoke()`, State Fabric, and identity — no URLs, no REST, no cookies, no origin policy. The Fabric Client is simultaneously an OS shell, application runtime, and composable UI surface. It renders adaptively from serial ASCII to rich pixel framebuffer based on device capabilities. Feature-gated: `fabric-client` (core runtime), `fc-views` (composable views), `fc-render` (pixel rendering), `fc-remote` (remote Fabric Client protocol)._

_Product naming: **VeerFlow** (user-facing brand — "Don't browse. Flow."), **Fabric Client** (architecture term), **FC** (code prefix), **VFCR** (VeerFlow Client Runtime). Design principles: invoke-first, live-by-default, identity-native, composable, context-preserving, zero-chrome._

### Why the Browser Dies in VeerOS

The traditional browser exists because the web is built on URLs, HTTP servers, and document fetching. In VeerOS + ZeroServices + State Fabric, **none of those primitives exist**:

```
Browser World (HTTP era)              VeerOS World (Fabric era)
────────────────────────              ────────────────────────
HTML pages                         →  Composable Views bound to functions + state
URLs (https://app.com/dashboard)   →  view("dashboard", { data: invoke("analytics.summary") })
REST APIs / fetch()                →  invoke("service.function", payload)
Cookies / JWT / Sessions           →  Identity-native (hardware-backed keys, capability tokens)
Request/response polling           →  subscribe("alerts.stream") — real-time by default
Same-origin policy / CORS          →  Capability-based access — security moves to runtime
Download JS bundle from server     →  Install signed View Package (verified, sandboxed, local)
Address bar / hyperlinks           →  navigate("workspace.analytics") — semantic, not location-based
WebSocket hacks                    →  State Fabric subscriptions — live by design
PWA offline bolt-on                →  Offline-first by design (CRDT sync, local state cache)
```

**Result**: Entire classes of web attacks (XSS, CSRF, clickjacking, supply-chain JS attacks) **disappear** because there are no origins, no cookies, no injected scripts, no ambient authority, and no untrusted code execution.

### 25A — Fabric Client Architecture (VFCR — VeerFlow Client Runtime)
_The layered architecture of the Fabric Client — from identity to pixels. Architecture name: **VFCR** (VeerFlow Client Runtime). Nine logical modules: Identity Manager (Layer 1), Invocation Engine (Layer 2), State Sync Engine (Layer 3), Event Engine (Layer 3½), View Runtime (Layer 4), Policy Engine / Security Sandbox (Layer 5), Transport Layer (QUIC/mTLS), Local Storage (offline-first). Event-driven concurrency: async everywhere, backpressure-aware, internal queues. Offline-first: local state cache, sync later, CRDT conflict resolution. Multi-fabric: federation handled automatically at transport level._

#### Layer 1 — Identity Layer (Built-in, Zero-Config)
- [ ] **Device + user identity** — hardware-backed Ed25519 + ML-DSA keypair (Phase 8C); no login forms for already-authenticated users; biometric/PIN unlock for screen resume
- [ ] **Session binding** — Fabric Client session cryptographically bound to (user_id, device_id, node_id); non-transferable; hardware attestation on capable devices (TPM, eFuse, Secure Enclave)
- [ ] **Identity cache** — authenticated identity persists across view navigations; no per-view re-auth (unlike web's per-origin cookie model); re-auth only on explicit privilege escalation
- [ ] **Multi-identity support** — switch between user identities without restart; each identity has separate capability set, view history, and state subscriptions

#### Layer 2 — Invocation Engine (replaces HTTP/fetch)
- [ ] **`fc_invoke(name, payload)` API** — view code calls fabric functions directly; kernel routes to local process, remote node, or WASM sandbox (Phase 20A′); no fetch(), no XMLHttpRequest, no REST
- [ ] **Streaming invocations** — `fc_invoke_stream("llm.generate", prompt)` returns a channel of tokens; view updates incrementally as tokens arrive; native streaming replaces SSE/WebSocket hacks
- [ ] **Batch invocations** — `fc_invoke_batch([("user.profile", {}), ("user.notifications", {})])` — parallel fan-out; view receives all results atomically; replaces Promise.all() over HTTP
- [ ] **Invocation caching** — Fabric Client caches idempotent invocation results locally; cache key = (function_name, payload_hash, version); TTL per function; stale-while-revalidate pattern
- [ ] **Error propagation** — invocation errors carry structured metadata: `{ code, function, node, retry_after, fallback_result }` — views handle gracefully without generic "500 Internal Server Error"

#### Layer 3 — State Sync Engine (replaces polling/WebSockets)
- [ ] **`fc_subscribe(key_pattern)` API** — view subscribes to State Fabric keys (Phase 21); updates pushed to view in real-time via kernel event bus; no polling, no WebSocket setup
- [ ] **Reactive bindings** — state changes automatically trigger view re-render of affected components; fine-grained: only components bound to changed keys re-render (not entire view tree)
- [ ] **Offline state cache** — subscribed state cached locally in persistent memory (Phase 14C); views render from cache when fabric is unreachable; CRDT merge (Phase 21B) on reconnection
- [ ] **Optimistic updates** — view applies state change locally immediately, syncs to fabric asynchronously; rollback on conflict; configurable per-key (optimistic vs pessimistic)
- [ ] **State windowing** — for large datasets, subscribe to a window: `fc_subscribe("logs.*", { limit: 100, offset: "latest" })` — server-side filtering reduces bandwidth

#### Layer 3½ — Event Engine (replaces notifications / push)
- [ ] **`fc_events(filter)` API** — subscribe to typed event streams: system events, user events, fabric events, audit events; filterable by severity, source, category
- [ ] **Event stream panel** — continuous scrolling stream replacing notification popups; events rendered inline as compact cards with timestamp, source, severity badge
- [ ] **Event correlation** — related events grouped by TraceID (from IFP header); click to expand full causal chain across nodes
- [ ] **Event persistence** — events stored in local ring buffer for offline replay; synced to State Fabric for cross-device continuity
- [ ] **Backpressure** — event stream respects consumer rate; slow consumers get priority-filtered subset (P0/P1 only); configurable per-stream

#### Layer 4 — View Runtime (replaces browser rendering engine)
- [ ] **Adaptive rendering engine** — same view definition renders differently based on output device:
  - **Serial/SSH** (80×25 text): ASCII art, box-drawing characters, ANSI colors, text tables
  - **VGA text mode** (80×25/132×50): enhanced text with colors, cursor, simple widgets
  - **Framebuffer** (640×480+ pixel): bitmap font rendering, rectangles, lines, basic widgets, images
  - **Remote FC client** (arbitrary): full resolution rendering on connected display device
- [ ] **Component model** — views composed of typed components: `Text`, `Table`, `List`, `Input`, `Button`, `Chart`, `Gauge`, `Image`, `Container`, `Grid`, `Tabs`, `Modal`
- [ ] **Layout engine** — flexbox-inspired layout: `Row`, `Column`, `Stack`, `Grid`, `Scroll`; components specify `min_size`, `max_size`, `grow`, `shrink`, `align`; layout computed per render target's dimensions
- [ ] **Styling** — per-component style: `{ fg, bg, bold, italic, underline, border, padding, margin }` — maps to ANSI codes (text) or pixel drawing (framebuffer); theme system with presets
- [ ] **Event propagation** — input events (key press, mouse click, touch, scroll) bubble through component tree; components declare `on_key`, `on_click`, `on_submit` handlers
- [ ] **View lifecycle** — `mount() → render() → update(state_change) → unmount()`; components are stateless functions of (props + fabric state); side effects only via invocations
- [ ] **`no_std` + `no_alloc` rendering** — view tree and layout computed in fixed-size arenas; suitable for ESP32 (text-only) through x86-64 (full pixel rendering); configurable arena sizes per target

#### Layer 5 — Policy Sandbox (replaces same-origin policy)
- [ ] **Per-view capability restriction** — each view runs with a subset of the user's capabilities; declared in View Package manifest: `{ requires: ["invoke:analytics.*", "state:read:dashboard.*"], denies: ["invoke:admin.*"] }`
- [ ] **Invocation allowlist** — view can only `fc_invoke()` functions listed in its manifest; attempts to invoke unlisted functions return `EPERM`; kernel enforces, not runtime
- [ ] **State access control** — view's `fc_subscribe()` and `fc_state_get()` restricted to declared key patterns; prevents data exfiltration across views
- [ ] **Input sanitization** — all user input from view components validated by kernel before passing to invocation payload; prevents injection attacks at the boundary
- [ ] **View isolation** — views from different packages run in separate capability domains (Phase 8B); no ambient cross-view data access; explicit capability grants for inter-view communication
- [ ] **Resource limits** — per-view: max memory (arena size), max concurrent invocations, max subscriptions, max render rate; prevents single view from starving the system

### 25B — Semantic Navigation (replaces URLs)
_Navigation is by intent and identity, not by location. No address bar. No links. No 404._

- [ ] **`navigate(target, params)` API** — `navigate("workspace.analytics")`, `navigate("user.profile", {id: 123})`, `navigate("settings.network.wifi")` — semantic, hierarchical, human-readable
- [ ] **Navigation registry** — views register navigation targets at install time: `register_nav("dashboard.main", view_fn, { icon: "grid", label: "Dashboard" })`; discovered by shell `nav list` and launcher
- [ ] **Deep navigation** — `navigate("device.node.raspi5.processes")` — navigate across fabric nodes; Fabric Client routes to remote node transparently; displays remote view locally
- [ ] **Back/forward history** — navigation stack maintained per-session; `nav back`, `nav forward`, `nav history`; stack persists across view switches (not page loads — views are instant)
- [ ] **Navigation intent resolution** — ambiguous targets resolved by AI: `navigate("show me the network")` → AI intent classifier (Phase 10I) maps to `navigate("dashboard.network.overview")`; confirmation prompt
- [ ] **Breadcrumb trail** — current navigation path displayed: `Home > Workspace > Analytics > Revenue`; each segment clickable/navigable
- [ ] **Favorites / pinned views** — `nav pin "dashboard.main"` — quick access to frequently used views; stored in user's persistent memory
- [ ] **Cross-fabric navigation** — `navigate("fabric://partner.analytics.eu/dashboard.summary")` — federated view access via InterFabric (Phase 24); identity-scoped, policy-checked at boundary
- [ ] **No 404** — navigation targets are registered or unregistered; unregistered targets return `NavigationError::NotFound { suggestions: [similar_targets] }` with AI-powered suggestions

### 25C — Composable View Packages (CVP)
_The replacement for web apps. Signed, versioned, sandboxed, locally-executed modules that combine data binding + rendering + interaction._

#### Package Format
- [ ] **View Package structure** — `{ manifest.toml, view.wasm|view.elf, assets/, theme.toml }` — single archive (.cvp file), content-addressed (SHA-256 hash = package ID)
- [ ] **Manifest format** — TOML manifest declaring:
  ```toml
  [package]
  name = "analytics-dashboard"
  version = "2.1.0"
  author = "fabric://veer.prod.india"
  description = "Real-time analytics dashboard"
  min_render_level = "text"  # text | framebuffer | remote
  
  [capabilities]
  invoke = ["analytics.*", "user.getProfile"]
  state_read = ["analytics.metrics.*", "user.preferences"]
  state_write = ["user.preferences.dashboard"]
  
  [navigation]
  targets = [
    { path = "dashboard.analytics", label = "Analytics", icon = "chart" },
    { path = "dashboard.analytics.detail", label = "Detail View" }
  ]
  
  [resources]
  max_memory_kb = 256
  max_subscriptions = 32
  max_concurrent_invocations = 8
  ```
- [ ] **Package signing** — Ed25519 + ML-DSA hybrid signature (Phase 8C) over package hash; signer = author's fabric identity; kernel verifies before installation
- [ ] **Package verification chain** — optional: multi-signer (author + auditor + fleet admin); configurable trust policy per fleet: `{ require_author: true, require_auditor: false }`

#### Package Lifecycle
- [ ] **`SYS_VIEW_INSTALL` (0xC0)** — install a CVP: verify signature → check capability budget → register navigation targets → store in VFS `/views/<name>/` → return view handle
- [ ] **`SYS_VIEW_UNINSTALL` (0xC1)** — remove CVP: deregister navigation targets → revoke capabilities → clean storage → notify active instances to unmount
- [ ] **`SYS_VIEW_LIST` (0xC2)** — list installed view packages with metadata (name, version, signer, installed_at, last_used)
- [ ] **`SYS_VIEW_LAUNCH` (0xC3)** — instantiate a view: load WASM/ELF → create view domain (Phase 8B sandbox) → bind capabilities → call mount() → begin rendering
- [ ] **`SYS_VIEW_NAVIGATE` (0xC4)** — navigate to a registered target; kernel resolves → launches/switches view → passes params
- [ ] **`SYS_VIEW_SUBSCRIBE` (0xC5)** — subscribe to state fabric key(s) for reactive updates
- [ ] **`SYS_VIEW_INVOKE` (0xC6)** — capability-checked fabric invocation from within a view
- [ ] **Version management** — multiple versions coexist; `navigate("dashboard@v2")` for explicit; canary traffic splitting between versions
- [ ] **Auto-update** — views declare update channel; Fabric Client checks for updates via fabric gossip; auto-install + atomic switch (old version kept as rollback)
- [ ] **Over-the-fabric distribution** — `view publish "analytics-dashboard.cvp"` → package distributed to fabric nodes via gossip; fleet-wide deployment without a package server

#### View Execution Models
- [ ] **WASM views** — view logic compiled to WASM (Phase 8B sandbox); polyglot: Rust, C, Go, AssemblyScript → WASM → CVP; most portable, most isolated
- [ ] **Native views** — view logic compiled to native ELF per-arch; used for system views (settings, diagnostics) that need direct kernel access; capability-restricted
- [ ] **Hybrid views** — native frame with embedded WASM components; system chrome (title bar, nav) is native; content area runs sandboxed WASM view
- [ ] **Shell-integrated views** — special views that enhance shell commands: `ls --view` renders file listing as an interactive tree (framebuffer) or formatted table (text); progressive enhancement

### 25C′ — Veer Definition Format (VDF)
_Declarative view DSL that compiles to `invoke()` + `subscribe()`. The replacement for HTML/CSS/JS. A single `.vdf` file (YAML-like) describes data bindings, layout, components, permissions, and navigation — the runtime does the rest._

#### VDF Structure
- [ ] **View declaration** — top-level `view:` block with `name`, `title`, `description`, `icon`; each `.vdf` file = one view; views compose by nesting `view:` references
- [ ] **Data bindings** — `data:` block binds named variables to `invoke()` calls or `subscribe()` streams:
  ```yaml
  data:
    metrics: { invoke: "analytics.getMetrics", args: { range: $params.range } }
    live_traffic: { subscribe: "analytics.traffic.*" }
    user: { invoke: "user.getProfile", args: { id: $identity.id } }
  ```
- [ ] **Context variables** — built-in reactive variables available in all expressions: `$identity` (current user), `$params` (navigation parameters), `$selected` (currently selected item), `$now` (current time), `$device` (display capabilities)
- [ ] **Computed / derived state** — `computed:` block for derived values: `{ total: "sum(metrics.values)", trend: "delta(metrics, -1h)" }` — reactive, recalculated on source change
- [ ] **Conditional rendering** — `showIf:` on any component: `showIf: canInvoke("admin.deleteUser")` — UI auto-adapts to caller's permissions; no separate "admin" vs "user" views
- [ ] **Layout system** — `layout:` block defines structure using `grid`, `stack`, `split`, `tabs`, `scroll`; responsive breakpoints per render tier; flexbox-like grow/shrink semantics

#### VDF Components
- [ ] **Typed components** — `chart:` (line, bar, pie, sparkline), `table:` (sortable, filterable, paginated), `stream:` (live-updating event list), `card:` (key-value display), `button:` (action trigger), `input:` (text/number/select), `banner:` (alert/info/warning), `view:` (nested view reference)
- [ ] **Action bindings** — `onClick:`, `onSubmit:`, `onSelect:` → `invoke("function", { args })` with context: `onClick: invoke("order.cancel", { id: $selected.id })`
- [ ] **Reactive update** — data bindings auto-update components; no manual state management; subscribe sources push changes → affected components re-render
- [ ] **Debug section** — optional `debug:` block enables trace overlay: shows raw invoke/subscribe calls, data flow, render timing; toggled in dev mode

#### VDF Compilation & Lifecycle
- [ ] **VDF parser** — YAML-like parser (no external YAML dependency; purpose-built for VDF subset) → intermediate representation (IR) of view tree + binding graph
- [ ] **IR → invoke()/subscribe() compilation** — VDF IR compiled to optimized sequence of `fc_invoke()` + `fc_subscribe()` calls + component tree; dead code elimination for hidden components
- [ ] **Lifecycle** — Load `.vdf` → parse → resolve permissions (check `canInvoke` for all data bindings) → execute bindings → render components → stream updates → unmount on navigate-away
- [ ] **Hot reload** — `vdf watch <file>` monitors `.vdf` file for changes; re-parses and re-renders without losing state; developer workflow

#### VDF Package Integration
- [ ] **Package manifest** — CVP manifest (25C) extended with VDF-specific fields:
  ```toml
  [vdf]
  entry = "views/main.vdf"
  views = ["views/main.vdf", "views/detail.vdf", "views/settings.vdf"]
  
  [vdf.navigation]
  targets = [
    { path = "dashboard.analytics", view = "views/main.vdf", label = "Analytics" },
    { path = "dashboard.analytics.detail", view = "views/detail.vdf" }
  ]
  ```
- [ ] **Capability inference** — VDF parser extracts all `invoke()` and `subscribe()` targets from data bindings → auto-generates capability requirements for manifest; author reviews + approves
- [ ] **View composability** — views reference other views: `{ type: view, src: "dashboard.widget.traffic" }` → nested CVP loaded + rendered within parent; capability scoping inherited

### 25D — Progressive Rendering Tiers
_One view definition, multiple rendering fidelities. The Fabric Client adapts to the display device._

#### Tier 0 — Serial / SSH (Minimum Viable Display)
- [ ] **ASCII renderer** — component tree → ANSI escape codes; box drawing (┌─┐│└─┘), colors (16 + 256-color), cursor positioning; 80×25 minimum
- [ ] **Text table renderer** — `Table` component → aligned columns with header, separator, row data; auto-column-width
- [ ] **Text gauge / progress** — `Gauge` component → `[████████░░] 75%`; `Spinner` → rotating `|/-\`; `Sparkline` → `▁▂▃▅▇▅▃▂`
- [ ] **Text chart** — `Chart` component → ASCII bar chart, mini line chart using braille characters (⠁⠂⠄⡀⢀)
- [ ] **Input widgets** — `Input` → readline-style with label; `Select` → arrow-key selection from list; `Checkbox` → `[x]` toggle; `RadioGroup`
- [ ] **Responsive text layout** — detect terminal size via ANSI `\e[18t` query or `stty`; reflow layout on resize; min-width graceful degradation
- [ ] **Mouse support (optional)** — xterm mouse reporting (`\e[?1000h`) for click-to-interact on capable terminals; fallback to keyboard-only

#### Tier 1 — VGA/Framebuffer Text Mode
- [ ] **Enhanced text rendering** — hardware cursor, full 256-color support, bold/blink attributes, larger terminal (132×50 possible)
- [ ] **Pseudographics** — enhanced box drawing with double-line characters, simple window chrome, shadow effects
- [ ] **Split pane layout** — divide screen into resizable panes; run multiple views simultaneously (tmux-like)

#### Tier 2 — Pixel Framebuffer (HDMI / LCD)
- [ ] **Bitmap font rendering** — scalable bitmap fonts (8×16, 12×24, 16×32); anti-aliased rendering on 32bpp framebuffer; Unicode glyph support (Basic Latin + common symbols)
- [ ] **Geometric primitives** — lines, rectangles, circles, filled/outlined; rounded corners; alpha blending on 32bpp
- [ ] **Widget rendering** — pixel-perfect buttons, scroll bars, input fields, dropdown menus, tabs, tree views; themed via `theme.toml`
- [ ] **Image support** — decode and display BMP/PNG images (TinyBMP, minipng decoders); photo thumbnails, icons, logos; scaled to widget bounds
- [ ] **Double buffering** — off-screen buffer → atomic flip/copy to visible framebuffer; tear-free rendering; dirty-region optimization (redraw only changed areas)
- [ ] **Window compositor (simple)** — multiple overlapping view windows with z-order; title bar + close/minimize; drag to move; not a full WM — just composited rectangles
- [ ] **Hardware cursor** — mouse cursor rendered separately from framebuffer on capable hardware; zero-latency pointer movement

#### Tier 3 — Remote Fabric Client
- [ ] **FC Remote Protocol** — binary protocol for transmitting view component trees and state deltas to a remote rich client; component-level diffing (not pixel streaming)
- [ ] **Component serialization** — view tree → compact binary representation → encrypted fabric channel → remote client renders natively using platform UI toolkit
- [ ] **Thin client mode** — remote client renders VeerOS views using native platform widgets (macOS Cocoa, Windows WPF, GTK, web/Canvas); VeerOS sends semantics, client renders pixels
- [ ] **Adaptive quality** — degrade rendering fidelity over high-latency/low-bandwidth links: reduce update rate, batch state changes, drop non-essential components
- [ ] **Screen sharing** — `view share <session_id>` — another user can observe (read-only) or collaborate (interactive) on the same view; cursor positions shared in real-time

### 25E — Device-Adaptive UI
_The Fabric Client detects capabilities and adapts the experience accordingly._

- [ ] **Capability probing** — at init, Fabric Client probes: display type (none/serial/VGA/framebuffer/remote), resolution, color depth, input devices (keyboard/mouse/touch/voice/BLE HID), locale, accessibility needs
- [ ] **Render tier selection** — auto-select optimal rendering tier based on probed capabilities: serial → Tier 0, VGA → Tier 1, framebuffer → Tier 2, remote client → Tier 3
- [ ] **Input adaptation** — views respond to available input: keyboard-only (arrow keys + enter), keyboard+mouse (click + scroll), touch (swipe + tap + pinch), voice ("select item 3"), BLE HID gamepad
- [ ] **Resolution-responsive layout** — views re-layout when resolution changes (window resize, display switch, `attach` to different node); components reflow, overflow → scroll
- [ ] **Locale-aware rendering** — date/time format, number format, text direction (LTR/RTL stubs), timezone; stored in user preferences (State Fabric)
- [ ] **Accessibility** — high-contrast mode (auto-detect or user preference), large text mode (2x font scale), screen reader support (component tree → text description for TTS, Phase 10L), keyboard-navigable with visible focus indicators

### 25F — Multi-Device Continuity
_A view started on one device can be continued on another. The fabric makes the UI location-independent._

- [ ] **View session persistence** — active view's state (navigation path, input values, scroll position, subscriptions) stored in State Fabric under user's identity
- [ ] **`view push <node>`** — transfer active view to another device's display: `view push raspi5` moves the dashboard from ESP32 serial to RPi5 HDMI; re-renders at target's tier
- [ ] **`view pull <node>`** — pull a view from remote device to local display; the remote device shows "view transferred to <node>"
- [ ] **Seamless handoff** — view state synced via State Fabric; target device re-subscribes to same state keys; no data loss during transfer; < 1s transition
- [ ] **Multi-display** — single user uses multiple displays simultaneously: ESP32 serial for status, RPi5 HDMI for dashboard, laptop remote FC for management; each display shows different views but same identity
- [ ] **Follow-me views** — mark a view as "follow": it automatically appears on the closest device to the user (requires location context from BLE beacons or proximity sensing)

### 25G — Shell → View Continuum
_The shell is the simplest view. Views are the richest shell. There is no hard boundary._

- [ ] **Shell as Tier 0 Fabric Client** — the existing VeerOS shell (Phase 3) IS the Fabric Client running in text-only mode; `invoke()` maps to shell callbacks; state subscriptions map to `watch` commands
- [ ] **`--view` flag on shell commands** — `ps --view` renders process list as an interactive table (sortable columns, highlight selected, kill on enter); `top --view` renders as auto-refreshing dashboard with gauges
- [ ] **`view` shell command** — `view <target>` navigates to a view: `view dashboard`, `view status.network`, `view settings.wifi`; if no framebuffer, renders as text TUI
- [ ] **`view list`** — show installed view packages; `view install <path|url>`; `view uninstall <name>`; `view update <name>`
- [ ] **Shell widgets** — interactive shell components for common patterns: `select "Choose network:" [WiFi1 WiFi2 WiFi3]` → returns selection; `confirm "Delete file?"` → returns bool; `progress "Updating..." 0.75`
- [ ] **View → shell fallback** — if a view requires framebuffer but only serial is available, gracefully degrade: render key information as text, warn about reduced fidelity
- [ ] **Inline view embedding** — shell command output can embed mini-views: `sysinfo` shows a brief text dashboard; `netstat --view` shows live-updating connection table

### 25H — AI-Native UI
_Views can be generated, modified, and operated by AI. Natural language produces visual results._

- [ ] **NL → View generation** — `"show me a dashboard of network traffic"` → AI (Phase 10I) generates a view definition with: Table of connections, traffic gauge, line chart of throughput; renders immediately
- [ ] **AI view assistance** — `"add a filter for high-latency connections"` while viewing a dashboard → AI modifies the active view: adds input component, subscribes to filtered state key, re-renders
- [ ] **Conversational view building** — multi-turn: `"show processes"` → table view → `"sort by memory"` → re-sorted → `"highlight anything over 100MB"` → conditional styling applied → `"save this as my-process-view"` → persisted as CVP
- [ ] **Smart defaults** — AI pre-selects sensible components based on data type: timestamps → timeline chart, proportions → pie/donut gauge, categories → bar chart, metrics → sparkline
- [ ] **Intent-driven UI** — `intent submit "create a monitoring dashboard for the sensor fleet"` → intent engine (Phase 14B) decomposes: identify metrics → create state subscriptions → generate view → deploy as CVP → navigate to it
- [ ] **AI accessibility** — voice-operated view navigation for hands-free operation: `"go to the network dashboard"` → navigate; `"read the error count"` → TTS reads value; `"acknowledge all alerts"` → invokes action

### 25I — Built-in System Views
_Views that ship with VeerOS — the "system apps" that replace traditional system utilities._

- [ ] **Dashboard view** — system overview: CPU/memory gauges, task count, uptime, network status, fabric health; auto-refresh; adapts from text sparklines (serial) to graphical gauges (framebuffer)
- [ ] **Process manager view** — interactive `top`-equivalent: sortable columns (PID, name, CPU%, MEM, state), real-time update via state subscription, kill/suspend actions, process detail drill-down
- [ ] **Network monitor view** — interface list, traffic graphs, active connections, firewall rules, DNS cache; live sparklines per interface
- [ ] **Fabric explorer view** — cluster visualization: node list with health/load/capabilities, topology graph (text: tree layout; pixel: force-directed graph), agent migration flows
- [ ] **File browser view** — tree navigation of VFS; file preview (text files rendered, hex for binary); mkdir, rename, delete actions; drag-and-drop on capable displays
- [ ] **Log viewer view** — structured event browser (Phase 19E): filterable by type/level/node/time; auto-scroll with pause; regex search; export
- [ ] **Settings view** — system configuration: network (WiFi/Ethernet), security (firewall rules, user management), display (theme, font size), AI (model selection, cloud API keys)
- [ ] **Security dashboard view** — audit log browser, threat score gauges, EDR alerts (Phase 16B), ZTNA session list, compliance status
- [ ] **Device fleet view** — MDM dashboard (Phase 17): device list, compliance status, firmware versions, staged update progress, health map
- [ ] **Intent tracker view** — active intents with plan step visualization (DAG), agent states, execution progress; drill-down to individual agent context

### 25J — Fabric Client Developer Experience
_Tools and APIs for building views._

- [ ] **View SDK** — `veeros-view` crate: component constructors (`Text::new("hello")`, `Table::new(headers, rows)`, `Gauge::new(0.75)`), layout helpers (`Row::new([...])`, `Column::new([...])`), event handlers, state bindings
- [ ] **View template macro** — `view!` declarative macro:
  ```rust
  view! {
    Column {
      Text { "System Dashboard" style: bold }
      Row {
        Gauge { value: state("cpu.load"), label: "CPU" }
        Gauge { value: state("mem.used_pct"), label: "Memory" }
      }
      Table {
        headers: ["PID", "Name", "CPU%", "State"]
        rows: invoke("os.process_list")
        on_select: |row| navigate("process.detail", { pid: row.pid })
      }
    }
  }
  ```
- [ ] **Hot reload** — during development, change view source → recompile WASM → FC detects new version → hot-swap without losing state subscriptions; < 500ms cycle
- [ ] **View testing framework** — `veeros-view-test` crate: mock fabric state, simulate input events, assert rendered output (snapshot testing for each render tier)
- [ ] **Component library** — reusable component collection: `DataTable`, `TimeSeriesChart`, `NetworkGraph`, `TreeView`, `Terminal`, `CodeEditor`, `MarkdownRenderer`, `JsonViewer`
- [ ] **Theme system** — `theme.toml` files; built-in themes: `midnight` (dark), `daylight` (light), `hacker` (green-on-black), `solarized`; user-selectable via settings view
- [ ] **View debugger** — `view debug` overlays component boundaries, shows state bindings, event flow, render timing; serial and framebuffer modes

### 25K — What the Fabric Client Eliminates

```
Traditional Stack                     → VeerOS Fabric Client / VeerUX
─────────────────────────────────────────────────────────────────────────
Web browser (Chrome, Firefox)         → Fabric Client runtime (kernel-native)
HTML/CSS/JS rendering engine          → Adaptive component renderer (text → pixel)
URLs / DNS resolution                 → Semantic navigation (navigate("target"))
HTTP/REST/GraphQL APIs                → invoke("function", payload)
Cookies / JWT / OAuth tokens          → Identity-native (hardware-backed keys)
WebSockets / SSE / polling            → State Fabric subscriptions (real-time default)
Same-origin policy / CORS             → Capability-based view sandboxing
npm / CDN / JS bundle download        → Signed View Packages (local, verified)
React / Vue / Angular frameworks      → Composable View SDK (no_std, no_alloc)
Electron / Tauri desktop wrappers     → Native pixel rendering (framebuffer)
Chrome DevTools / React DevTools      → view debug overlay + component inspector
App Store / Play Store                → Over-the-fabric package distribution
Progressive Web Apps (PWA)            → Offline-first by design (CRDT state cache)
Web accessibility tools               → Built-in accessibility (screen reader, high-contrast)
Responsive web design                 → Device-adaptive rendering (serial → pixel)

Security attacks eliminated:
  XSS (cross-site scripting)          → No script injection (views are compiled WASM)
  CSRF (cross-site request forgery)   → No cookies/sessions to forge
  Clickjacking                        → No iframes, no embed
  Supply-chain JS attacks             → Signed packages with verified author identity
  Cookie theft / session hijacking    → Hardware-bound identity tokens
  Man-in-the-browser                  → No browser extension model
  DOM-based injection                 → No DOM; typed component tree
```

### 25L — Distribution Profile Integration

- [ ] **`fc-text` feature** — Tier 0 text-mode Fabric Client (serial/SSH/VGA); included in `dist-app`, `dist-full`, `dist-ai`, `dist-cluster`, `dist-cloud`, `dist-firewall`
- [ ] **`fc-views` feature** — Composable View Package runtime; included in `dist-full`, `dist-desktop`, `dist-ai`
- [ ] **`fc-render` feature** — Tier 2 pixel framebuffer rendering; included in `dist-desktop`, `dist-ai` (on capable hardware)
- [ ] **`fc-remote` feature** — Tier 3 remote Fabric Client protocol; included in `dist-desktop`, `dist-cloud`
- [ ] **`dist-desktop` profile (NEW)** — full desktop/workstation distribution: `dist-full` + `fc-views` + `fc-render` + `fc-remote` + `ai-nlp`; targets RPi 4/5 (HDMI), x86-64 (VGA/HDMI); adds window compositor, system views, theme engine

```
                 dist-   dist-  dist-  dist-  dist-  dist-  dist-    dist-     dist-    dist-      dist-       dist-
                 minimal app    rt     xrt    full   edge   ai       cluster   cloud    firewall   gateway     desktop
───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
FC Text (Tier 0)   —      ✓      —      —      ✓      —      ✓        ✓         ✓        ✓          —           ✓
FC Views (CVP)     —      —      —      —      ✓      —      ✓        —         —        —          —           ✓
FC Render (Pixel)  —      —      —      —      —      —      opt      —         —        —          —           ✓
FC Remote (T3)     —      —      —      —      —      —      opt      —         ✓        —          —           ✓
System Views       —      —      —      —      ✓      —      ✓        ✓         ✓        ✓          —           ✓
Window Compositor  —      —      —      —      —      —      —        —         —        —          —           ✓
Theme Engine       —      —      —      —      ✓      —      ✓        —         —        —          —           ✓
View Packages      —      —      —      —      ✓      —      ✓        —         —        —          —           ✓
───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
```

### 25M — Per-Target UI Capabilities
_What UI surface each hardware target supports._

```
  ESP32-C3 (400 KB)     →  Tier 0 only (serial text), no views, no render
  ESP32-C6 (512 KB)     →  Tier 0 (serial text), optional SPI LCD (Tier 2 limited: 240×320)
  ESP32-S3 (512 KB+)    →  Tier 0 (serial), Tier 2 (SPI LCD via LCD_CAM: 320×480); touch input
  RPi Zero 2 W (512 MB) →  Tier 0 (serial), Tier 2 (HDMI framebuffer: 1280×720); limited views
  RPi 3 (1 GB)          →  Tier 0–2 (HDMI 1080p); basic system views; keyboard+mouse via USB
  RPi 4 (4/8 GB)        →  Tier 0–3 (dual HDMI 4K); full views + remote FC; compositor candidate
  RPi 5 (8 GB)          →  Tier 0–3 (dual HDMI 4K); full desktop profile; AI-generated views
  QEMU virt (RISC-V)    →  Tier 0 (serial); Tier 2 (virtio-gpu future); development/testing
  QEMU PC (x86-64)      →  Tier 0 (serial+VGA), Tier 1 (VGA text), Tier 2 (VGA framebuffer)
  x86-64 bare-metal     →  Tier 0–3 (VGA/HDMI/DisplayPort); full desktop; compositor; multi-monitor
```

### 25N — Fabric Client Roadmap Integration

```
TIER 0–10 (existing tiers as documented above)

TIER 11 — User Experience
  Phase 25  Fabric Client / VeerFlow — adaptive rendering, composable views, VDF view DSL, semantic navigation, AI-native UI — NEW
```

#### Phase Dependencies
```
Phase 25A (Identity Layer)      ← Phase 6I (User Identity), Phase 8C (Crypto), Phase 15B (Node Identity)
Phase 25A (Invocation Engine)   ← Phase 20A′ (Function Invocation), Phase 20 (ZeroServices)
Phase 25A (State Sync Engine)   ← Phase 21 (State Fabric), Phase 21B (CRDTs)
Phase 25A (View Runtime)        ← Phase 6J (VFS for view storage), Phase 7D (HID Input), existing Framebuffer Console
Phase 25A (Policy Sandbox)      ← Phase 8A (Capabilities), Phase 8B (Isolation Domains), Phase 8B WASM Sandbox
Phase 25C (View Packages)       ← Phase 8B (WASM Sandbox), Phase 8D (Secure Boot / Package Signing)
Phase 25F (Multi-Device)        ← Phase 21 (State Fabric), Phase 19A (Cross-Node Shell)
Phase 25H (AI-Native UI)        ← Phase 10I (NL Shell), Phase 14B (Intent Engine), Phase 10L (Voice)
Phase 25L (dist-desktop)        ← Phase 4 (Distribution Profiles), Phase 7D (USB HID), RPi5 framebuffer
```

---

## Phase 26 — Living Systems / Integrity Engine
### 26H — Programming Model: Shell Scripts for ESP32C6
- [x] `build-esp32c6.sh` and `build-qemu-esp32c6.sh` support dist profiles, WiFi/BLE/802.15.4, auto-flash/monitor
- [x] Remote shell via veer-connect
- [x] Usage: see script headers and docs/wifi-bringup-esp32c6.md
_VeerOS replaces "bugs", "defects", and "QA" with a continuous, real-time behavioral integrity system. Every component continuously evaluates its own correctness, confidence, and alignment with declared intent. Deviations produce integrity signals — not stack traces — that form a living graph of system health. The system self-observes, self-explains, self-repairs, and self-learns. No QA environments, no defect backlogs, no release gates. Feature-gated: `integrity` (core primitive), `integrity-observe` (self-observation), `integrity-explain` (causal reasoning), `integrity-repair` (autonomous fix), `integrity-learn` (feedback loop)._

_**Determinism guarantee**: all observation, scoring, anomaly detection, repair ranking, and knowledge distillation are deterministic — same history → same decision, always. LLM assistance (self-explanation text, fix proposal generation) is opt-in and always validated in simulation before deployment. The integrity engine operates correctly without LLMs; they accelerate human comprehension, they don't make autonomous decisions._

_Dependencies: Phase 14 (Agents / Intent Engine / Memory Engine), Phase 19E (Structured Events / KernelEvent), Phase 8F (Audit subsystem), Phase 10I (NL / AI inference)._

### 26A — Integrity Primitive
_The foundational kernel primitive. Every evaluable component carries an integrity envelope that defines expected behavior boundaries._

- [ ] **`IntegrityEnvelope` struct** — per-component behavioral boundary: `{ component_id, intent_hash, expected_behavior: BehaviorSpec, confidence_floor: f32, observation_interval_ms: u32, repair_policy: RepairPolicy }` — stored in component metadata
- [ ] **`BehaviorSpec` definition** — declarative specification of expected behavior: input/output ranges, latency bounds, state transition rules, invariants; compiled from intent declarations (Phase 14B)
- [ ] **`IntegritySignal` type** — continuous metrics emitted by every observed component: `{ component_id, timestamp, confidence: f32, alignment: f32, anomaly_score: f32, context: SignalContext }` — not binary pass/fail but continuous confidence
- [ ] **`SYS_INTEGRITY_REGISTER` (0xD0)** — register a component for integrity observation: attach `IntegrityEnvelope`, initialize baseline metrics, begin observation loop
- [ ] **`SYS_INTEGRITY_QUERY` (0xD1)** — query current integrity state of a component or subsystem: returns `IntegrityReport { signals, trend, confidence_delta, last_deviation }`
- [ ] **Integrity signal bus** — dedicated event channel for integrity signals; subscribers (repair engine, audit, shell, dashboard) receive filtered streams; priority-ordered (critical deviations first)
- [ ] **Signal persistence** — integrity signals written to episodic memory (Phase 14C) for trend analysis and learning; ring buffer with configurable retention per-component

### 26B — Self-Observation
_Components continuously evaluate their own correctness, confidence, and alignment against their integrity envelope._

- [ ] **Observation loop** — each registered component runs a periodic self-check: evaluate current state against `BehaviorSpec` → compute confidence score → emit `IntegritySignal` → schedule next check
- [ ] **Confidence scoring** — multi-dimensional score: `correctness` (output matches spec), `timeliness` (within latency bounds), `resource_compliance` (within budget), `alignment` (behavior matches declared intent); weighted composite → single confidence float [0.0, 1.0]
- [ ] **Anomaly detection** — statistical baseline built from first N observations; subsequent observations scored against baseline; z-score > threshold → anomaly signal; adaptive baseline that evolves with legitimate behavioral changes
- [ ] **Cross-component correlation** — integrity signals from interacting components correlated by TraceID; if A's confidence drops and B (which A invokes) also drops → causal chain identified
- [ ] **Intent drift detection** — compare actual execution patterns against declared intent (Phase 14B); gradual drift (behavior slowly diverging from intent) detected via moving-window analysis; alert before hard failure
- [ ] **Resource envelope monitoring** — observe CPU time, memory allocation, invocation count, state writes against declared budgets; budget overrun → integrity signal with `resource_compliance` drop
- [ ] **Observation cost control** — self-observation itself has a CPU/memory budget; observation frequency auto-adjusts based on component stability (stable → less frequent; anomalous → more frequent)

### 26C — Self-Explanation
_When deviations occur, the system produces causal reasoning and confidence deltas — not stack traces and error codes._

- [ ] **Causal reasoning engine** — on integrity signal with confidence < threshold: trace backward through invocation chain (TraceID), state dependencies, and event timeline → produce `CausalExplanation { root_cause, contributing_factors, confidence_delta_chain, timeline }`
- [ ] **Confidence delta chain** — track how confidence changed over time: `[{ t: 100ms, confidence: 0.95, event: "invoke analytics.compute" }, { t: 150ms, confidence: 0.72, event: "timeout from state.get" }, ...]` — shows exactly when and why confidence dropped
- [ ] **Natural language explanation** — `CausalExplanation` rendered to human-readable text via AI inference (Phase 10I): _"The analytics dashboard slowed because the metrics service on node-3 exceeded its memory budget, causing state reads to timeout. Confidence dropped from 0.95 to 0.72 over 50ms."_
- [ ] **Deviation classification** — classify deviations: `Transient` (recoverable, e.g., network blip), `Degradation` (progressive, e.g., memory leak), `Violation` (hard boundary crossed), `Drift` (slow intent misalignment); each class triggers different repair strategy
- [ ] **Explanation history** — all explanations stored in episodic memory indexed by component + time; queryable: `explain component="analytics" since="1h ago"` → returns all deviations and their causal chains
- [ ] **Shell integration** — `integrity explain <component>` — show latest deviation explanation in human-readable form; `integrity explain --trace <trace_id>` — show full causal chain for a specific invocation

### 26D — Self-Repair
_Fixes generated, validated in simulation, and deployed autonomously. No human in the loop for known repair patterns._

- [ ] **Repair strategy registry** — predefined repair actions for common deviation classes: `Transient → retry with backoff`, `ResourceOverrun → shrink budget + reschedule`, `Timeout → switch to fallback node`, `Drift → re-anchor to intent`; extensible via agent definitions (Phase 14A)
- [ ] **Autonomous fix generation** — for unknown deviations: AI inference (Phase 10I) analyzes `CausalExplanation` + component `BehaviorSpec` + repair history → proposes corrective action: parameter adjustment, routing change, component restart, fallback activation
- [ ] **Simulation sandbox** — proposed fix applied in isolated simulation environment: clone component state → apply fix → run synthetic workload → evaluate integrity signals; fix accepted only if simulated confidence ≥ threshold
- [ ] **Staged deployment** — accepted fix deployed in stages: single instance → canary percentage → full rollout; integrity monitored at each stage; automatic rollback if confidence drops
- [ ] **Repair budget** — per-component limit on autonomous repairs per time window; prevents repair loops; exceeded budget → escalate to human (shell alert + dashboard notification)
- [ ] **Repair audit trail** — every repair action logged to audit subsystem (Phase 8F): `{ component, deviation, proposed_fix, simulation_result, deployment_stage, outcome }` — full accountability
- [ ] **`SYS_INTEGRITY_REPAIR` (0xD2)** — manual repair trigger: `integrity repair <component> --strategy <name>` — apply specific repair strategy; useful for human-guided recovery
- [ ] **Rollback** — every repair creates a rollback checkpoint; if repair worsens integrity → automatic revert to pre-repair state; rollback chain maintained (up to N checkpoints)

### 26E — Self-Learning
_Every integrity signal feeds back into future decisions and execution strategies. The system gets smarter over time._

- [ ] **Episodic feedback loop** — completed repair cycles stored as episodes in memory engine (Phase 14C): `{ deviation, explanation, repair_action, outcome, confidence_before, confidence_after, duration }` — training data for future repairs
- [ ] **Pattern recognition** — AI inference analyzes episode history: identify recurring deviation patterns, time-of-day correlations, load-dependent failures, seasonal trends; generate preventive rules
- [ ] **Predictive integrity** — based on learned patterns, predict upcoming deviations: _"Node-3 memory usage trend suggests budget overrun in ~2h based on similar episodes on 2026-03-01 and 2026-03-15"_ → preemptive repair before failure
- [ ] **Baseline evolution** — observation baselines updated based on learned patterns; legitimate behavioral changes (e.g., after code update) auto-accepted after observation window; prevents false positives
- [ ] **Repair strategy refinement** — successful repair strategies promoted (higher priority, shorter simulation); failed strategies demoted or retired; strategy effectiveness score maintained
- [ ] **Fleet learning** — integrity patterns shared across fabric nodes (via State Fabric / gossip); a repair strategy proven on one node is available to all nodes; configurable sharing policy (opt-in per fleet)
- [ ] **Knowledge distillation** — periodically compress episode history into compact rules: `IF deviation_class == Timeout AND component_type == StateRead AND time_of_day IN [02:00, 04:00] THEN preemptive_repair(increase_timeout_20%)` — reduces memory footprint while preserving learning

### 26F — Integrity Graph
_All integrity signals form a live graph of system health. The graph is queryable, visualizable, and actionable._

- [ ] **Graph data structure** — directed graph: nodes = components, edges = invocation/state dependencies; each node carries current `IntegritySignal`; edges carry latency + confidence propagation weight
- [ ] **Real-time aggregation** — subsystem-level integrity computed by aggregating component signals: `subsystem_confidence = weighted_avg(component_confidences)` — cascading: fabric confidence = avg(subsystem_confidences)
- [ ] **Health dashboard view** — VeerFlow view (Phase 25 / VDF) rendering integrity graph: color-coded nodes (green/yellow/red by confidence), animated edges showing data flow, drill-down to component detail
- [ ] **`integrity status`** — shell command showing system-wide integrity summary: overall confidence, top-N deviating components, active repairs, recent explanations
- [ ] **`integrity graph <component>`** — show dependency graph for a component: upstream + downstream, confidence propagation, bottleneck identification
- [ ] **Alerting** — configurable thresholds: `integrity alert when confidence < 0.5 for component "analytics.*"` — triggers shell notification, event stream entry, optional `invoke("ops.page", { ... })`
- [ ] **Historical replay** — `integrity replay --from "2h ago" --to "1h ago"` — replay integrity graph state over time; identify when deviations started and how they propagated

### 26G — Shell Integration & UX
_Living Systems presented to users through the shell and VeerFlow views._

- [ ] **`integrity` shell command family**:
  - `integrity status` — system-wide health summary
  - `integrity observe <component>` — show live integrity signals for a component
  - `integrity explain <component>` — latest deviation with causal explanation
  - `integrity repair <component>` — trigger manual repair
  - `integrity history <component>` — deviation + repair history timeline
  - `integrity graph [component]` — dependency graph with health overlay
  - `integrity learn --stats` — learning system statistics (episodes, patterns, predictions)
  - `integrity config <component> --confidence-floor 0.8` — configure per-component thresholds
- [ ] **Integrity indicators in system views** — `ps` shows per-process integrity score; `top` includes integrity column; `sysinfo` includes overall system integrity percentage
- [ ] **VeerFlow integrity dashboard** — dedicated system view (Phase 25I) with live integrity graph, event stream, active repairs, prediction timeline; auto-installed as system view

### 26H — Phase Dependencies
```
Phase 26A (Integrity Primitive)    ← Phase 14A (Agent Table), Phase 14B (Intent Engine), Phase 19E (Structured Events)
Phase 26B (Self-Observation)       ← Phase 26A, Phase 8F (Audit Ring Buffer), Phase 14C (Memory Engine)
Phase 26C (Self-Explanation)       ← Phase 26B, Phase 10I (NL/AI Inference), Phase 14C (Episodic Memory)
Phase 26D (Self-Repair)            ← Phase 26C, Phase 8B (Isolation Domains / Simulation), Phase 14A (Agents)
Phase 26E (Self-Learning)          ← Phase 26D, Phase 14C (Episodic Memory), Phase 10I (AI Inference)
Phase 26F (Integrity Graph)        ← Phase 26B, Phase 21 (State Fabric), Phase 25 (VeerFlow for dashboard)
Phase 26G (Shell Integration)      ← Phase 26A-F, Phase 3 (Shell), Phase 25I (System Views)
```

---

## Session Log
- 2026-02-26: Bootstrapped workspace and crate architecture, documented design, and enabled distribution feature model.
- 2026-02-26: Installed Rust toolchain in container, added ESP32 kernel entry crate, linker script, and target-specific cargo checks.
- 2026-02-26: Added UART0 MMIO driver, Serial/Console arch traits, boot banner, and hardware bringup docs.
- 2026-02-26: Phase 2 core — added interrupt controller driver, systimer tick driver, TaskContext, Scheduler with TCB table, and idle task. All distros + chip variants build clean (0 warnings).
- 2026-02-26: Phase 2 complete — added IPC mailbox module, RISC-V trap entry/exit via global_asm!, Rust trap dispatcher with timer-tick context switch and ecall handler. All builds clean.
- 2026-02-27: Phase 3 shell — added Serial RX to arch + ESP32 UART, built interactive shell crate, host demo binary (veeros-demo) with raw terminal. Shell task registered in ESP32 kernel boot. Full boot → shell verified via `cargo run -p veeros-demo`.
- 2026-02-26: Preemptive multitasking — `_veer_start_first_task` asm loads TaskContext + mret with MPIE=1/MPP=M into first task. Timer ISR (1ms CLINT tick) preempts shell↔idle via round-robin scheduler. Named TCBs, `uptime` and `tasks` shell commands. QEMU poweroff on `exit`. All builds clean.
- 2026-03-10: Phase 5 kickoff — remote shell over TCP. Added `NetworkDevice` trait to arch, VIRTIO-NET MMIO driver in QEMU BSP, `net` crate (smoltcp TCP/IP + `TcpSerial` Serial-over-TCP bridge), network listener task in kernel-qemu-virt (port 2323). MAX_TASKS bumped to 16. All kernels (ESP32 + QEMU) build clean with 0 warnings. QEMU launch instructions with `-device virtio-net-device` + hostfwd documented.
- 2026-03-10: Fixed VIRTIO MMIO v2 (modern) driver — rewrote virtio_net.rs with contiguous page-aligned VqRegion, split desc/avail/used pointers, VIRTIO_F_VERSION_1 feature negotiation. Fixed TcpSerial to use `may_recv()`/`may_send()` for proper remote-close detection (was stuck in CLOSE_WAIT). Fixed socket re-listen with `abort()` to skip TIME_WAIT. Remote shell fully working: banner, commands (`help`, `tasks`, `uptime`, `sysinfo`, `logo`), and sequential reconnections all verified over TCP.
- Phase 6A complete — `SavedContext` trait (17 methods), `Riscv32Context` in `arch::riscv32`, factored trap assembly into arch crate (removed duplication from both kernel trap.rs), usize portability audit done.
- Phase 6B partial — `BlockReason` enum (`None`/`Sleep`/`IpcRecv`/`Join`), `wakeup_tick: u64` replaces gpr[0] hack, `SYS_SPAWN` (0x05) + `SYS_JOIN` (0x06) syscalls, parent/child tracking + exit code delivery, userlib `spawn()`/`join()` wrappers. All 4 builds clean, QEMU boot verified.
- 2026-03-16: Phase 6A — `SavedContext` trait + architecture abstraction. Defined `SavedContext` trait in `arch` crate with portable accessors (`set_pc/get_pc/advance_pc`, `set_sp/get_sp`, `set_status/get_status`, `set_arg/get_arg`, `set_ret/get_ret`, `get_syscall_nr`, `get_kernel_word/set_kernel_word`). Moved concrete `TaskContext` to `arch::riscv32::Riscv32Context` module with `SavedContext` impl. Added `#[cfg(target_arch)]` type alias in `arch/src/lib.rs`. Refactored microkernel `dispatch.rs` (all gpr[] → trait methods), `task.rs` (create_task uses set_pc/set_sp), and both kernel `main.rs` files (set_status). Zero raw register index access outside of riscv32.rs. All 4 build configs (QEMU default/minimal, ESP32-C6, host demo) pass. QEMU boot verified — all 7 tasks running, IPC + timers working. Also added Phase 8 security architecture (capabilities, isolation domains, PQC crypto, secure boot) and Phase 6I multi-user support to TODO.
- 2026-03-16: Phase 6D + 6E — Synchronization primitives and bounded channels. Implemented kernel `FutexTable` (32-slot address-keyed wait queue) in `crates/microkernel/src/futex.rs`, `SYS_FUTEX_WAIT`/`SYS_FUTEX_WAKE` syscalls. Discovered riscv32imc has NO atomic instructions — rewrote userlib `sync.rs` with `UnsafeCell<usize>` + `read_volatile`/`write_volatile` (kernel futex provides serialization). Userlib `Mutex<T>`, `Condvar`, `Semaphore`. Implemented bounded channels in `crates/microkernel/src/channel.rs` (8 channels, depth-8 ring buffers, blocking send/recv with PC rewind, close wakes all). Userlib `channel` module. All 3 builds clean, QEMU boot verified with all tasks running.
- 2026-03-16: Phase 6C (partial) — Memory management + isolation. Added `MemPerms`, `TaskMemRegion`, `TaskRegions`, `validate_user_ptr()` to arch crate. Implemented RISC-V PMP driver (`arch::riscv32::pmp`) with CSR helpers for `pmpaddr`/`pmpcfg`, TOR-mode region programming, `apply_task_regions()` called on every context switch in both QEMU and ESP32 trap handlers. Per-task memory regions in TCB: `create_task()` auto-grants stack RW + 64-byte stack guard (NONE perms). Pointer validation in syscall dispatcher: `SYS_WRITE_BUF`, `SYS_PANIC`, `SYS_FUTEX_WAIT` check user pointers against task regions. Added `SYS_MEM_REGION_COUNT`/`SYS_MEM_REGION_INFO` syscalls. All 3 builds clean (0 warnings), QEMU boot verified — all 7 tasks running correctly with PMP context switch. Remaining: U-mode transition, S-mode/MMU support, ARM64 page tables.
- 2026-03-16: Phase 6F — Async/await runtime. Kernel poll subsystem: `PollTable` in `microkernel::poll` with per-task event registration (TIMER, IPC, CHAN_READABLE, CHAN_WRITABLE, TASK_EXIT), `check()` evaluates events, `wake_poll_waiters()` called from timer tick. `SYS_POLL_SET` (0x60) / `SYS_POLL_WAIT` (0x61) syscalls in dispatch.rs with non-blocking (timeout=0), blocking, and deadline modes. `PollCell` statics wired into both QEMU and ESP32 kernel trap handlers. Userlib: `poll` module (raw `poll_set`/`poll_wait` wrappers), `async_rt` module with `block_on` executor (noop `Waker`, `Poll::Pending` → `SYS_POLL_WAIT`), `AsyncTimer` (POLL_TIMER), `AsyncRecv` (POLL_CHAN_READABLE), `AsyncSend` (POLL_CHAN_WRITABLE) futures. All 3 builds clean, QEMU boot verified.
- Shell enhancements — readline line editor, advanced vi, history & set commands. Created `line_ed.rs`: full readline-style `LineEditor` with `History` ring buffer (32 entries, dedup), cursor movement (Ctrl-A/E/B/F), kill-line (Ctrl-U/K/W), transpose (Ctrl-T), clear (Ctrl-L), arrow keys, Alt-b/f/d word movement. Rewrote Shell struct to use `LineEditor` + `ShellVars` (vi_number, tabstop, showmatch, autoindent, prompt). Added `history` command (show/N/clear) and `set` command (view/modify shell vars). Enhanced `vi.rs` (1600+ lines): 5 modes (Normal/Insert/Replace/Command/Search), `ViSettings` struct, count prefixes on commands, `e` word-end, `H/M/L` screen-relative, `Ctrl-D/U` half-page scroll, `f/F/t/T` find-char-in-line, `/` and `?` search with `n/N/*`, `R` replace mode, `~` case toggle, `D/C` delete/change-to-end, `dw/d$/d0/cc/cw/c$` motions, `>>` / `<<` indent/dedent, `%` bracket matching, `.` repeat last edit with `LastEdit`/`EditKind` tracking, `:set` (number/tabstop/autoindent/showmatch/showmode), `:s/pat/rep/[g]` substitute, line numbers in `draw_screen()`. Added man pages for vi, history, set. All 4 targets build clean (0 warnings).
- RPi5 full stub implementation — Replaced ALL remaining stubs with real hardware implementations. **Trivial stubs fixed**: GIC `disable_interrupt()` → GICD_ICENABLER write; Platform `init_cpu` → FP/NEON enable (CPACR_EL1); `init_interrupts` → GIC-400 init; `init_timer` → 10ms tick; `console_read_byte()` → PL011 UART read. **New RP1 drivers**: GPIO (28 pins, function/mode/pull/drive/schmitt/slew, RIO atomic outputs), SPI (DW APB SSI, SPI0–5, polled full-duplex), I2C (DW APB I2C, I2C0–6, Standard/Fast mode, write/read/write_read). **xHCI DMA engine**: TRB rings (Command+Event+Transfer), DCBAA, statically-allocated buffers (no heap), cycle bit management. **USB enumeration**: enable_slot → address_device → GET_DESCRIPTOR → parse VID/PID; HID endpoint config → SET_CONFIGURATION → Configure Endpoint; boot protocol keyboard/mouse. **Shell callbacks**: `lsblk` queries real SD card (sector count, MiB), `usb_list` queries real xHCI ports + enumerated devices. All 4 targets build clean (0 errors, 0 warnings).
- 2026-03-16: Phase 6D/6E/6G/6H completion — RwLock<T> (futex-based reader-writer lock), priority inheritance in kernel futex (base_priority field, boost on wait, restore on wake). SYS_CHAN_POLL (0x5C) non-blocking channel depth query + userlib poll() wrapper. Typed channels: Channel<T> generic wrapper with compile-time size check. 4-word channel messages (ChanMsg expanded to word0–word3), syscall5/syscall_ret4 in userlib sys.rs. Phase 6G: BSD-style sockets — SocketTable (16 slots), Domain::Local/Inet, SockType::Stream/Dgram, full lifecycle (create/bind/listen/accept/connect/send/recv/close), 256-byte RingBuf per socket direction, BlockReason::SockAccept/SockSend/SockRecv. Socket syscalls 0x70–0x77 wired into dispatch with pointer validation. Userlib socket module with Domain/SockType enums. Phase 6H: embedded man page system — `MAN_PAGES` static table (18 topics: scheduler, ipc, memory, boot, yield, exit, spawn, join, sleep, send, recv, channel, socket, futex, sync, tasks, help, poll), `man` shell command with topic listing. All 3 targets build clean.
- 2026-04-15: Phase 14 — AI-Native Execution Kernel. Implemented 5 core kernel modules: (1) `agent.rs` — Agent as first-class primitive (AgentState lifecycle, Goal with priority/deadline/budget, AgentContext 8-slot working memory, AgentCb with parent/child hierarchy, AgentTable 32 slots, AgentMessage for inter-agent IPC), (2) `intent.rs` — Intent Engine (IntentClass taxonomy, constraint system, IntentDescriptor, Plan with 16-step DAG and StepRelation dependency tracking, rule-based decomposition), (3) `memory_engine.rs` — Three-tier Memory (PersistentMemory with MemoryTag/MemoryScope/LRU eviction/confidence scoring, EpisodicMemory ring buffer with 12 EpisodeKind variants and success_rate analytics, distribution-profile sizing), (4) `fabric.rs` — Execution Fabric (FabricNode with 16 NodeCapability flags, NodeArch/LocalityZone/NodeHealth, PlacementConstraint, weighted node selection with capability→resource→locality→load scoring), (5) `intent_sched.rs` — Intent Scheduler meta-scheduler (6-phase tick: decompose→assign→monitor→sync→record→health, budget enforcement, episodic feedback loop). Added 13 AI-native syscalls (0xF0–0xFF) to syscall.rs. Wired all modules into lib.rs. Extended dispatch.rs with full syscall handlers + 4 new ProcessCaps (AGENT/INTENT/MEMORY_ENGINE/FABRIC bits 19–22). Updated all 5 kernel targets (qemu_pc, qemu_virt, esp32c3, xiao_esp32c6, raspi5) with static subsystem instances and dispatch call wiring. x86_64 builds clean (0 errors). Added Phase 14 to TODO.md with 7 subsections (14A–14G) covering implemented items and remaining work.
- 2026-04-15: Phase 14H — Userlib + Shell + Demo. Created 3 userlib modules (`userlib::agent`, `userlib::intent`, `userlib::memory`) wrapping all 13 AI-native syscalls. Added 6 shell callbacks (`get_agent_list`, `agent_cmd`, `get_intent_list`, `intent_cmd`, `memory_cmd`, `get_fabric_status`) to `ShellEnv`. Implemented 4 shell commands (`agents`, `intent`, `memory`/`kv`, `fabric`) with dispatch + subcommands. Added `demo` command with 3 interactive scenarios (`deploy`, `pipeline`, `monitor`). Added 5 man pages (`agents`, `intent`, `memory`, `fabric`, `demo`). Updated `veeros-demo` binary with fully-wired AI-native callbacks: pre-populated 4-node execution fabric (x86 host, ARM64 RPi5 edge, RISC-V ESP32 sensor, cloud GPU), seeded persistent memory, live agent/intent/memory/fabric operations. Added `scripts/demo.sh` launcher + `docs/demo-walkthrough.md` usage guide. Updated all 8 `ShellEnv` initializers across 5 kernel targets. Updated README with marketing-ready content (application domains, comparison table, quick start). Updated architecture.md with 7 application domain sections and "What VeerOS Replaces" comparison. All builds clean.
- 2026-07-12: SSH server + bridge networking + crypto hardening. **SSH crate** (`crates/ssh`): full SSH-2 server — `transport.rs` (binary packet protocol, chacha20-poly1305@openssh.com encryption, packet sequence MAC), `kex.rs` (curve25519-sha256 key exchange, derive_keys with SHA-256), `auth.rs` (password authentication), `channel.rs` (interactive shell channel, window flow control), `server.rs` (session state machine: KEX→auth→shell), `client.rs` (SSH client stub). **Crypto fixes** (`crates/crypto`): Ed25519 keypair generation, Curve25519 scalar clamping, X25519 Diffie-Hellman; `edpk_probe` example. **virtio-net TX fix** (`crates/soc/qemu_pc/src/virtio_net.rs`): static `TX_BUF` with 16-byte alignment replaces stack-local buffer — fixes use-after-free with TAP async processing; single descriptor + spin-wait for device consumption. **DHCP** (`crates/kernel/qemu_pc/src/net.rs`): smoltcp DHCPv4 client acquires real LAN IP (192.168.29.x) via bridge/TAP. **Bridge/TAP networking**: `scripts/run-qemu-pc.sh` with NAT and bridge modes; TAP device attached to br0; DHCP + ping + SSH all verified over LAN. **Build**: GRUB ISO boot, `scripts/build-qemu-pc.sh` updated. **Page fault fix**: x86-64 page fault handler with proper error code + CR2. **Ring 3 gating**: conditional ring-3 transition behind feature flag. x86-64 builds clean.

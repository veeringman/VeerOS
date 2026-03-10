# VeerOS Build Tracker

This file is the persistent progress tracker for VeerOS and should be updated in every development session.

## V1 Scope
- [ ] Bootable microkernel on ESP32 RISC-V (C3/C6/H2)
- [ ] Modular architecture interfaces for future embedded CPU families
- [ ] Distribution variants via Rust feature flags

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
- [x] Interactive shell crate — line editor, built-in commands (help, version, sysinfo, uname, echo, clear, logo, exit)
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
- [ ] Driver isolation model
- [ ] Memory management baseline for embedded targets
- [ ] Optional app runtime service
- [ ] Optional real-time scheduling service

## Phase 5 — Remote Access (SSH / Equivalent)
- [x] `NetworkDevice` trait in `arch` crate (transport-agnostic NIC abstraction)
- [x] VIRTIO-NET MMIO driver in `bsp-qemu-virt` (probes QEMU virt slots)
- [x] `net` crate — smoltcp TCP/IP stack integration + `DeviceAdapter` PHY bridge
- [x] `TcpSerial` — implements `Serial` trait over a TCP socket (shell-over-TCP)
- [x] Network listener task in `kernel-qemu-virt` (auto-probes NIC, listens on port 2323)
- [x] `MAX_TASKS` bumped to 16 (supports idle + shell + net + future sessions)
- [x] QEMU launch instructions with `-device virtio-net-device` + user-net port forwarding
- [ ] Lightweight SSH-compatible server (or custom encrypted shell protocol)
- [ ] Authentication model (key-based or password)
- [ ] Remote shell session multiplexing (attach shell task to network socket)
- [x] QEMU user-net or TAP networking for development/testing
- [ ] ESP32 Wi-Fi driver integration for real-hardware remote access

## Phase 4 — Distribution Profiles
- [ ] `minimal` distribution build recipe
- [ ] `app` distribution build recipe
- [ ] `real-time` distribution build recipe
- [ ] `full` distribution build recipe

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
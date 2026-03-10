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
- [x] Formal syscall ABI — ecall-based, numbered ranges (task/ipc/io/time/mem/debug)
- [x] Kernel syscall dispatcher (`dispatch.rs`) + `SyscallAction` enum
- [x] Userlib crate — `sys`, `task`, `ipc`, `io`, `time` modules with raw ecall wrappers
- [x] `print!` / `println!` macros via `SYS_WRITE_BUF` syscall
- [x] IPC send/recv/poll syscalls wired end-to-end
- [x] Sleep/wake syscalls with `wake_sleepers()` in timer tick
- [x] Userlib sample tasks verified on QEMU (hello, timer, ipc-tx, ipc-rx)
- [ ] Driver isolation model
- [ ] Memory management baseline for embedded targets
- [ ] Optional app runtime service
- [ ] Optional real-time scheduling service

## Phase 6 — Process Model Overhaul (Multi-Arch Foundation)

### 6A — Architecture Abstraction Layer
_Make `arch` crate truly architecture-neutral so ARM64, RISC-V 64, and x86-64 can coexist._

- [ ] **`SavedContext` trait** — replace concrete `TaskContext { gpr: [usize; 32] }` with a trait (`size_of`, `set_sp`, `set_pc`, `set_arg`, `get_ret`, `zero`) implemented per-arch
- [ ] **Per-arch context modules** — `arch_riscv32/context.rs`, `arch_riscv64/context.rs`, `arch_arm64/context.rs`, `arch_x86_64/context.rs`
- [ ] **Arch-specific trap entry/exit** — factor `_veer_trap_entry` assembly into per-arch crates (currently duplicated in both kernel trap.rs)
- [ ] **`usize` portability audit** — ensure scheduler, IPC, allocator work with both 32-bit and 64-bit word sizes
- [ ] **`SyscallAbi` trait** — abstract ecall (RISC-V) / svc (ARM) / syscall (x86) instruction + register convention
- [ ] **Conditional `TaskContext` in `arch`** — `#[cfg(target_arch)]` dispatch or associated type on `Platform` trait
- [ ] **ARM64 stub** — `arch_arm64` crate: `SavedContext` with 31 GPRs + SP + PC + PSTATE, trap frame for EL1→EL0
- [ ] **RISC-V 64 stub** — `arch_riscv64` crate: same 32 GPRs but `usize = u64`, S-mode mcause→scause
- [ ] **x86-64 stub** — `arch_x86_64` crate: `SavedContext` with 16 GPRs + RIP + RFLAGS + segment regs

### 6B — Process + Thread Model
_Introduce proper process/thread separation. Processes own address spaces; threads run within them._

- [ ] **`Process` struct** — address space ID (ASID), capability token set, resource quotas, child list, exit status
- [ ] **`Thread` struct** (replaces current `Tcb`)** — belongs to a `Process`, has own stack + context, share process memory
- [ ] **Thread states** — extend `TaskState` → `Ready`, `Running`, `Blocked(BlockReason)`, `Suspended`, `Zombie`
- [ ] **`BlockReason` enum** — `Sleep(u64)`, `IpcRecv`, `MutexWait(MutexId)`, `CondWait(CondId)`, `Join(ThreadId)`, `IoWait`
- [ ] **Replace `gpr[0]` sleep hack** — add `wakeup_tick: u64` field to `Thread`/`Tcb` struct
- [ ] **`SYS_SPAWN` syscall** — create a new thread within the calling process from userspace
- [ ] **`SYS_SPAWN_PROCESS` syscall** — create a new process with a separate address space
- [ ] **`SYS_JOIN` syscall** — wait for a thread to exit, retrieve exit code
- [ ] **Per-process resource accounting** — track heap usage, open handles, thread count per process
- [ ] **Parent-child relationship** — process tree, orphan reparenting, `SYS_WAIT`
- [ ] **Thread-local storage (TLS)** — `tp` register (RISC-V x4) pointing to per-thread data area

### 6C — Memory Management + Isolation
_Hardware-enforced memory isolation using PMP (RISC-V) / MPU (ARM Cortex-M) / page tables (MMU targets)._

- [ ] **`MemoryProtection` trait** — abstract PMP, MPU, and MMU behind a common interface (`grant_region`, `revoke_region`, `switch_context`)
- [ ] **RISC-V PMP driver** — configure PMP entries per-task on context switch (8–16 regions)
- [ ] **Per-process memory regions** — stack, heap, .text, .rodata tracked in process descriptor
- [ ] **Pointer validation** — `SYS_WRITE_BUF` / `SYS_PANIC` verify user pointers against granted regions before access
- [ ] **Stack guard regions** — PMP/MPU region below each stack to trap overflow
- [ ] **Kernel/user split** — M-mode kernel + U-mode tasks on RISC-V; EL1/EL0 on ARM64; ring 0/3 on x86-64
- [ ] **RISC-V S-mode support** — for 64-bit targets with MMU (Sv39/Sv48 page tables)
- [ ] **ARM64 page tables** — 4K pages, TTBR0/TTBR1 split, ASID tagging

### 6D — Synchronization Primitives
_Kernel-backed locking and signaling for safe concurrent access._

- [ ] **`SYS_FUTEX_WAIT` / `SYS_FUTEX_WAKE` syscalls** — Linux-style futex as the universal building block
- [ ] **Userlib `Mutex<T>`** — spin-then-futex mutex built on `SYS_FUTEX_WAIT/WAKE`, `no_std` compatible
- [ ] **Userlib `Condvar`** — condition variable on top of futex
- [ ] **Userlib `Semaphore`** — counting semaphore (bounded concurrency control)
- [ ] **Priority inheritance** — in kernel futex: boost holder's priority to max of all waiters
- [ ] **Deadlock detection** — optional: track wait-for graph in kernel, surface via debug syscall
- [ ] **`RwLock<T>`** — reader-writer lock (multiple readers xor one writer)
- [ ] **Atomic operations support** — RISC-V A extension (lr/sc, amo*) or fallback kernel-mediated CAS on rv32imc

### 6E — Message Queues + Channels
_Replace single-slot mailbox with proper IPC primitives._

- [ ] **Bounded message queue** — ring buffer (configurable depth, e.g., 8–64 slots) per queue, not per task
- [ ] **`SYS_MQ_CREATE` / `SYS_MQ_DESTROY`** — create/destroy a named or anonymous queue
- [ ] **`SYS_MQ_SEND` / `SYS_MQ_RECV`** — blocking and non-blocking variants with timeout
- [ ] **`SYS_MQ_POLL`** — check if queue has messages without consuming
- [ ] **Typed channels** — userlib wrapper: `Channel<T>` for typed, zero-copy (within address space) message passing
- [ ] **Multicast / publish-subscribe** — notification groups for event broadcasting
- [ ] **Keep legacy single-slot IPC** as a fast path for simple request/reply patterns
- [ ] **`arg1` fix** — return all 4 message words through `a0`–`a3` in userlib `recv()`

### 6F — Async/Await Runtime
_Cooperative concurrency within a thread — many logical tasks on one stack._

- [ ] **Kernel `SYS_POLL_SET` syscall** — register interest in multiple events (IPC, timer, I/O ready)
- [ ] **Kernel `SYS_POLL_WAIT` syscall** — block until any registered event fires (like epoll_wait)
- [ ] **Userlib executor** — single-threaded `no_std` async executor: task queue, `Waker` integration with `SYS_POLL_WAIT`
- [ ] **Userlib `AsyncTimer`** — `Future` that yields until `SYS_SLEEP` completes
- [ ] **Userlib `AsyncRecv`** — `Future` that yields until IPC message or MQ message arrives
- [ ] **`async fn` task entry** — allow task entry points to be `async fn() -> !` with executor loop
- [ ] **Cooperative yield point** — `core::task::Poll::Pending` triggers `SYS_POLL_WAIT`, not busy spin
- [ ] **Cancellation** — drop-based cleanup for in-flight async operations

### 6G — Sockets (IPC + Network)
_Unified socket API spanning local IPC and network transports._

- [ ] **`SYS_SOCKET` / `SYS_BIND` / `SYS_LISTEN` / `SYS_ACCEPT`** — BSD-style socket syscalls
- [ ] **`SYS_CONNECT` / `SYS_SEND` / `SYS_RECV` / `SYS_CLOSE`** — data transfer syscalls
- [ ] **Local (Unix-domain) sockets** — in-kernel ring buffer between two processes, no network overhead
- [ ] **TCP sockets** — wrap smoltcp TCP in socket handle, expose to userspace
- [ ] **UDP sockets** — wrap smoltcp UDP for datagram services
- [ ] **Socket handle table** — per-process file descriptor / handle table (small fixed array initially)

- [ ] **`select` / `poll` / `epoll`-style multiplexing** — ties into async `SYS_POLL_SET`
- [ ] **Userlib `TcpStream` / `TcpListener`** — safe Rust wrappers in userlib

### 6H — Documentation / Man Pages
_Built-in documentation accessible from the shell._

- [ ] **`man` shell command** — display help for syscalls, commands, and concepts
- [ ] **Embedded man page store** — `&[(&str, &str)]` table in `.rodata`, keyed by topic name
- [ ] **Syscall man pages** — one entry per syscall (yield, exit, send, recv, sleep, alloc, etc.)
- [ ] **Shell command help** — `man help`, `man tasks`, `man wifi`, `man bt`, `man zigbee`
- [ ] **Concept pages** — `man scheduler`, `man ipc`, `man memory`, `man boot`
- [ ] **Pager** — basic `--More--` pagination for long man pages on small terminals

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

## Phase 4 — Distribution Profiles (Complete)
- [x] Distribution matrix design — two axes: profile (minimal/app/rt/full) × components (shell/net/userlib/samples/wifi/ble/ieee802154)
- [x] `distributions` crate restructured — aligned feature names (`dist-minimal`/`dist-app`/`dist-rt`/`dist-full`), component flags, documentation
- [x] `kernel-qemu-virt` — optional deps: shell, net, userlib, smoltcp; profiles auto-bundle components; default = `dist-app`
- [x] `kernel-xiao-esp32c6` — optional deps: shell; radio features: wifi, ble, ieee802154; default = `dist-minimal` + shell + all radios
- [x] `#[cfg(feature)]` gates across both kernel binaries — conditional compilation of net_task, shell_task, sample tasks, driver registrations, radio managers
- [x] `minimal` distribution build recipe — `--no-default-features --features dist-minimal` (bare scheduler + idle task only)
- [x] `app` distribution build recipe — `--features dist-app` (shell + net + userlib + samples)
- [x] `real-time` distribution build recipe — `--features dist-rt` (priority scheduler, combine with component flags)
- [x] `full` distribution build recipe — `--features dist-full` (all components + priority scheduler)

## Phase 7 — Multi-Architecture Targets

### ARM64
- [ ] `soc-qemu-virt-aarch64` crate — PL011 UART, GICv2 interrupt controller, ARM generic timer
- [ ] `kernel-qemu-virt-aarch64` — `aarch64-unknown-none` target, EL1 boot, PSCI
- [ ] ARM64 exception vector table — sync/IRQ/FIQ/SError handlers, context save/restore
- [ ] QEMU `virt` machine aarch64 validation

### RISC-V 64
- [ ] `soc-qemu-virt-riscv64` crate — reuse NS16550/CLINT with `usize = u64`
- [ ] `kernel-qemu-virt-riscv64` — `riscv64gc-unknown-none-elf` target, S-mode with SBI
- [ ] S-mode trap delegation — `sstatus`/`scause`/`sepc` instead of M-mode CSRs
- [ ] Sv39 page table support (if MMU path enabled)

### x86-64
- [ ] `soc-qemu-pc` crate — serial (COM1 0x3F8), APIC timer, PIC/IOAPIC
- [ ] `kernel-qemu-pc` — `x86_64-unknown-none` target, multiboot2 boot, long mode
- [ ] IDT setup — interrupt descriptor table, ISR stubs, syscall via `syscall`/`sysret`
- [ ] GDT + TSS — kernel/user segment selectors, per-CPU task state segment

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
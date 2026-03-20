# VeerOS Build Tracker

This file is the persistent progress tracker for VeerOS and should be updated in every development session.

## V1 Scope
- [ ] Bootable microkernel on ESP32 RISC-V (C3/C6/H2) and Xtensa (S3)
- [ ] Multi-architecture support — ARM64 (RPi family, QEMU/KVM), x86-64 (QEMU/KVM), RISC-V 32/64
- [ ] Distribution variants via Rust feature flags — from `dist-minimal` (bare MCU) to `dist-cloud` (full cluster)
- [ ] Configurable single-user / multi-user system (feature-gated)
- [ ] Security-first architecture — capability-based access, isolation domains, PQC-ready crypto, extensible security model
- [ ] AI-native OS — inference engine, NL shell, autonomous agents, on-device and cloud AI as first-class primitives
- [ ] Distributed OS — multiple VeerOS nodes form a single coherent system (cluster membership, distributed scheduler, shared VFS)
- [ ] Cloud-native platform — built-in orchestration, service mesh, service discovery, rolling deployments, observability
- [ ] Network appliance mode — firewall, packet filtering, NAT, VPN gateway, traffic shaping as a distribution profile (`dist-firewall`)

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
- [ ] **x86-64 stub** — `arch_x86_64` crate: `SavedContext` with 16 GPRs + RIP + RFLAGS + segment regs

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
- [ ] **`multi-user` / `single-user` feature flags** — `single-user` default for embedded targets (no login, implicit root); `multi-user` enables full identity system
- [ ] **`UserId` (UID) type** — `u16` user identifier; UID 0 = root/system, UID 1–65534 = normal users, 65535 = nobody
- [ ] **`GroupId` (GID) type** — `u16` group identifier for coarse-grained access grouping
- [ ] **`UserEntry` struct** — `uid`, `gid`, `name: &str`, `password_hash: [u8; 32]`, `home_dir`, `shell`, `flags` (enabled/disabled/locked)
- [ ] **User table** — static `[UserEntry; MAX_USERS]` (8–16 slots); stored in `.rodata` for single-user, kernel RAM for multi-user
- [ ] **Group table** — static `[GroupEntry; MAX_GROUPS]` with membership bitmask per user

#### Authentication
- [ ] **`SYS_LOGIN` syscall** — validate username + password, return session token on success
- [ ] **`SYS_LOGOUT` syscall** — invalidate session, terminate user's processes (optional)
- [ ] **Password hashing** — lightweight hash (SHA-256 or SipHash) for credential verification; no plaintext storage
- [ ] **Login shell flow** (`multi-user` only) — boot → `login:` prompt → authenticate → spawn user shell with UID set
- [ ] **Auto-login** (`single-user`) — skip authentication, all processes run as UID 0 (root)
- [ ] **Session token** — kernel-issued opaque `u32` token tied to UID; passed in process descriptor on spawn
- [ ] **Failed login lockout** — optional: 3 failed attempts → 30s cooldown (prevents brute-force on serial/SSH)

#### Per-User Process Ownership
- [ ] **UID field in `Process` struct** — every process tagged with owner's UID at spawn time
- [ ] **`SYS_GETUID` / `SYS_GETGID` syscalls** — return calling process's UID/GID
- [ ] **`SYS_SETUID` syscall** — privilege escalation (root only); allows spawning processes as another user
- [ ] **Process visibility** — `ps`/`tasks` shows owner; non-root users see only their own processes (configurable)
- [ ] **Signal/kill permissions** — `SYS_KILL` restricted: users can only signal their own processes; root can signal any

#### Resource & Capability Permissions
- [ ] **Per-user resource limits** — max processes, max memory, max open handles (enforced at `SYS_SPAWN`/`SYS_ALLOC`)
- [ ] **Capability ownership** — capabilities (MMIO, IRQ, network) granted per-user or per-group; checked at syscall boundary
- [ ] **IPC permissions** — optional: restrict which UIDs can send to privileged service ports
- [ ] **Driver access control** — only root (UID 0) or designated group can register/interact with hardware drivers

#### Shell Integration
- [ ] **`whoami` command** — display current user name and UID
- [ ] **`su` command** — switch user (requires target user's password or root privilege)
- [ ] **`users` command** — list logged-in users and their sessions
- [ ] **`useradd` / `userdel` commands** — runtime user management (root only, `multi-user` feature)
- [ ] **`passwd` command** — change password for current user (or any user if root)
- [ ] **Shell prompt** — include username: `user@veeros $` (multi-user) vs `veeros $` (single-user)

#### Distribution Integration
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

- [ ] **DHCP client in smoltcp** — enable smoltcp's `dhcpv4` feature; wire `Dhcpv4Client` into the network stack
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

## Phase 4 — Distribution Profiles (Complete + Expansion Planned)
- [x] Distribution matrix design — two axes: profile (minimal/app/rt/full) × components (shell/net/userlib/samples/wifi/ble/ieee802154)
- [x] `distributions` crate restructured — aligned feature names (`dist-minimal`/`dist-app`/`dist-rt`/`dist-full`), component flags, documentation
- [x] `kernel-qemu-virt` — optional deps: shell, net, userlib, smoltcp; profiles auto-bundle components; default = `dist-app`
- [x] `kernel-xiao-esp32c6` — optional deps: shell; radio features: wifi, ble, ieee802154; default = `dist-minimal` + shell + all radios
- [x] `#[cfg(feature)]` gates across both kernel binaries — conditional compilation of net_task, shell_task, sample tasks, driver registrations, radio managers
- [x] `minimal` distribution build recipe — `--no-default-features --features dist-minimal` (bare scheduler + idle task only)
- [x] `app` distribution build recipe — `--features dist-app` (shell + net + userlib + samples)
- [x] `real-time` distribution build recipe — `--features dist-rt` (priority scheduler, combine with component flags)
- [x] `full` distribution build recipe — `--features dist-full` (all components + priority scheduler)

### Extended Distribution Profiles (Planned)
_New profiles to cover edge AI, cluster/distributed, cloud platform, and network appliance deployments._

- [ ] **`dist-edge`** — IoT edge node: `dist-minimal` + AI inference (keyword/anomaly) + WiFi/BLE + sensor pipeline; targets ESP32 family, RPi Zero
- [ ] **`dist-ai`** — AI-native: `dist-app` + full AI stack (inference engine, NL shell, model zoo, NPU backends); targets RPi 4/5, x86-64 with ≥ 2 GB RAM
- [ ] **`dist-cluster`** — Distributed OS node: `dist-app` + cluster membership + distributed scheduler + distributed IPC + shared VFS; targets RPi 3+, x86-64, ARM64
- [ ] **`dist-cloud`** — Cloud platform: `dist-cluster` + orchestration + service mesh + API gateway + ingress + observability + auto-scaling; targets x86-64 KVM, ARM64 KVM
- [ ] **`dist-firewall`** — Network appliance: `dist-minimal` + packet filter + NAT + VPN + traffic shaping + DPI + firewall rules engine; targets x86-64, ARM64, RPi 4/5
- [ ] **`dist-gateway`** — IoT gateway: `dist-edge` + Thread border router + Zigbee coordinator + MQTT broker + protocol translation; targets RPi 3+, ESP32-S3
- [ ] **Feature composition** — profiles are additive: `dist-cloud` = `dist-cluster` + `cloud-orchestrate` + `cloud-mesh` + `cloud-observe`; any combination valid
- [ ] **Build flag matrix** — `distributions/src/lib.rs` updated with new feature gates; cross-feature dependency validation at compile time
- [ ] **Per-target defaults** — ESP32-C3/C6: `dist-edge`; RPi Zero: `dist-edge`; RPi 4/5: `dist-ai` or `dist-cluster`; x86-64 KVM: `dist-cloud`; x86-64 bare: `dist-firewall`

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
- [ ] **`arch_x86_64` crate** — `SavedContext` for x86-64: 16 GPRs (RAX–R15) + RIP + RFLAGS + RSP + CS + SS + FS_BASE (TLS); `SavedContext` trait impl; `get_syscall_nr` from RAX, args from RDI/RSI/RDX/R10/R8/R9 (Linux ABI)
- [ ] **GDT (Global Descriptor Table)** — kernel CS/DS (Ring 0), user CS/DS (Ring 3), TSS descriptor; loaded via `lgdt` at boot
- [ ] **TSS (Task State Segment)** — per-CPU TSS with `RSP0` (kernel stack on privilege transition), IST (Interrupt Stack Table) entries for NMI/DF/MCE
- [ ] **IDT (Interrupt Descriptor Table)** — 256-entry IDT; ISR stubs (0–31 exceptions, 32–47 IRQs, 48+ software); each stub saves all GPRs → calls Rust handler → `iretq`
- [ ] **`syscall`/`sysret` fast path** — MSR setup (`IA32_STAR`, `IA32_LSTAR`, `IA32_FMASK`); `syscall` entry saves RCX/R11, loads kernel RSP from per-CPU, dispatches, `sysret` back to Ring 3
- [ ] **Context switch** — save callee-saved (RBX, RBP, R12–R15, RSP), swap task pointers, restore; FPU/SSE state via `xsave`/`xrstor` (lazy or eager)
- [ ] **`#[cfg(target_arch = "x86_64")]`** — wire `TaskContext = X86_64Context` type alias in `arch/src/lib.rs`

#### x86-64 Boot (Multiboot2 / UEFI)
- [ ] **Multiboot2 header** — `.multiboot2` section in kernel ELF; tags: framebuffer request, module align, EFI services; loaded by GRUB2 or QEMU `-kernel`
- [ ] **Boot assembly** — `_start` in long mode (Multiboot2 hands off in 32-bit protected mode → set up 64-bit page tables → jump to long mode → call `kernel_main`)
- [ ] **Identity-map bootstrap page tables** — 2 MB huge pages, first 4 GB identity-mapped; kernel maps itself into high half (0xFFFF_8000_0000_0000+) before enabling full paging
- [ ] **UEFI boot path (future)** — `x86_64-unknown-uefi` stub that exits boot services, sets up page tables, jumps to kernel; for real hardware without Multiboot
- [ ] **Boot info parsing** — Multiboot2 info struct: memory map (E820), framebuffer, ACPI RSDP pointer, boot command line

#### x86-64 SoC + Kernel (QEMU q35)
- [ ] **`soc-qemu-pc` crate** — COM1 UART (I/O ports 0x3F8), APIC (Local APIC + I/O APIC), HPET/PIT timer, ACPI tables, VGA/framebuffer, VIRTIO-PCI, PS/2 keyboard
- [ ] **`kernel-qemu-pc` crate** — `x86_64-unknown-none` target, Multiboot2 boot, custom linker script (kernel at 1 MB physical, higher-half virtual at 0xFFFF_8000_0010_0000)
- [ ] **Serial console (COM1)** — I/O port 0x3F8; divisor latch for baud rate; polled TX/RX + IRQ4 interrupt-driven RX; implements `Serial` trait
- [ ] **Local APIC** — MMIO at 0xFEE0_0000 (or MSR-based x2APIC); timer in periodic mode (IRQ vector 32); ICR for IPI; spurious vector; EOI
- [ ] **I/O APIC** — MMIO at 0xFEC0_0000; redirection table entries for ISA IRQs (COM1→IRQ4, keyboard→IRQ1, HPET→IRQ0/2); route to LAPIC
- [ ] **ACPI table parsing** — RSDP → RSDT/XSDT → MADT (APIC topology), FADT (PM timer, shutdown), HPET table; minimal AML interpreter deferred
- [ ] **HPET timer** — High Precision Event Timer; 64-bit monotonic counter; periodic comparator for tick interrupt; fallback to PIT 8254 if no HPET
- [ ] **PIC 8259 (legacy)** — remap to vectors 32–47, then mask all (use APIC); needed for initial boot before APIC init
- [ ] **PS/2 keyboard** — IRQ1 scancode processing; scan set 1 → ASCII; implements `InputDevice` trait; primary input for early boot
- [ ] **VGA text mode (early boot)** — 80×25 @ 0xB8000; boot messages before framebuffer init; implements `Serial` trait
- [ ] **VIRTIO-PCI** — PCI config space enumeration; VIRTIO devices as PCI functions; reuse VIRTIO-NET/BLK backends with PCI transport
- [ ] **PCI bus enumeration** — walk bus 0–255, device 0–31, function 0–7; read config space (BAR, class code, vendor/device ID); build device table

#### x86-64 Memory Management
- [ ] **4-level page tables** — PML4 → PDPT → PD → PT; 4 KB pages (+ 2 MB / 1 GB huge pages for kernel mapping)
- [ ] **Higher-half kernel** — kernel linked at 0xFFFF_8000_0000_0000+; user space in lower half 0x0000_0000–0x0000_7FFF_FFFF_FFFF; canonical address enforcement
- [ ] **Per-process page tables** — each `Process` gets own PML4; `CR3` swap on context switch; PCID (Process Context Identifier) to avoid TLB flush
- [ ] **Physical memory allocator** — bitmap or buddy allocator initialized from E820 memory map; 4 KB frame granularity
- [ ] **Kernel heap** — `slab` or bump allocator for kernel-internal allocations; mapped in higher-half
- [ ] **Ring 0/Ring 3 split** — kernel pages with `Supervisor` bit; user pages with `User` bit; NX (No-Execute) on data pages; SMEP + SMAP enforcement

#### KVM Acceleration
- [ ] **QEMU `-enable-kvm` validation** — `qemu-system-x86_64 -enable-kvm -cpu host -M q35` on x86-64 Linux hosts; verify VeerOS boots at near-native speed
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
_Design the security model as a core OS primitive, not a bolt-on. Every subsystem respects these boundaries. Feature-gated tiers: `sec-base` (always on), `sec-sandbox`, `sec-crypto`, `sec-verified`._

### 8A — Capability-Based Access Control (Core)
_The foundation — every resource access requires a capability token. No ambient authority._

- [ ] **`Capability` type** — unforgeable kernel-issued token: `{ id: u32, resource: ResourceKind, rights: Rights, owner: ProcessId }`
- [ ] **`ResourceKind` enum** — `Memory(region)`, `IpcPort(id)`, `Interrupt(line)`, `MmioRegion(base,size)`, `Device(driver_id)`, `Socket(handle)`, `File(path)`, `ProcessControl(pid)`, `CryptoKey(key_id)`, `Quantum(qpu_id)`, `AiModel(model_id)`, `AiAccelerator(backend_id)`
- [ ] **`Rights` bitflags** — `READ`, `WRITE`, `EXECUTE`, `GRANT` (can delegate to child), `REVOKE`, `MAP`, `SEND`, `RECV`, `ADMIN`
- [ ] **Capability table** — per-process fixed array `[Option<Capability>; MAX_CAPS_PER_PROCESS]` (32–64 slots)
- [ ] **`SYS_CAP_CREATE` syscall** — kernel mints a new capability (root/parent only)
- [ ] **`SYS_CAP_GRANT` syscall** — delegate a capability (with optional rights restriction) from parent → child process
- [ ] **`SYS_CAP_REVOKE` syscall** — revoke a capability from a process (cascading: revokes all delegated children)
- [ ] **`SYS_CAP_QUERY` syscall** — list capabilities held by calling process
- [ ] **Enforcement in syscall dispatcher** — every resource-accessing syscall checks capability table before proceeding; deny = `EPERM`
- [ ] **Boot capabilities** — init/root process receives full capability set; spawned processes inherit only what parent grants
- [ ] **Capability-aware IPC** — send capabilities across process boundaries via IPC (cap transfer in message metadata)

### 8B — Isolation Domains (Sandboxing / Containers / MicroVMs)
_Hierarchical isolation levels — from lightweight sandboxes to hardware-enforced virtual machines._

#### Domain Model
- [ ] **`IsolationDomain` struct** — `{ id, level: IsolationLevel, parent: Option<DomainId>, process_set, resource_caps, memory_budget, cpu_budget }`
- [ ] **`IsolationLevel` enum** — `Shared` (soft, same address space), `Sandbox` (restricted caps, no raw HW access), `Container` (separate memory region + namespace), `MicroVM` (hardware-enforced: PMP/MPU/hypervisor)
- [ ] **Domain hierarchy** — domains nest: MicroVM contains Containers, Containers contain Sandboxes
- [ ] **`SYS_DOMAIN_CREATE` / `SYS_DOMAIN_DESTROY`** — create/tear down an isolation domain (requires `ADMIN` capability)
- [ ] **`SYS_DOMAIN_ENTER`** — spawn a process inside a domain (inherits domain restrictions)
- [ ] **`SYS_DOMAIN_QUERY`** — introspect domain's resource usage and policy

#### Sandbox (Software Isolation)
- [ ] **Syscall filter** — per-domain allowlist of permitted syscall numbers (like seccomp-BPF); deny returns `EPERM`
- [ ] **Capability ceiling** — domain defines max rights any process within can hold (even if parent granted more)
- [ ] **Namespace isolation** — sandboxed processes see only IPC ports / resources within their domain
- [ ] **Resource quotas** — memory ceiling, CPU time budget, max processes, max open handles per domain
- [ ] **`seccomp`-style profiles** — predefined profiles: `io-only` (read/write/yield), `compute-only` (no IPC/IO), `network-only`, `full`

#### Container (Memory-Isolated Workloads)
- [ ] **Per-container memory region** — dedicated PMP/MPU region or page table ASID; processes inside can't access host memory
- [ ] **Container namespaces** — PID namespace (PIDs internal to container), IPC namespace, network namespace (virtual NIC)
- [ ] **Container image** — flat binary or ELF loaded into container's memory region at spawn
- [ ] **Virtual filesystem stub** — per-container read-only `.rodata` slice for config/data; no global FS namespace leak
- [ ] **Container lifecycle** — create → start → pause → resume → stop → destroy; state machine enforced in kernel
- [ ] **Inter-container IPC** — only via explicit kernel-mediated channels with capabilities; no shared memory by default

#### MicroVM (Hardware-Enforced Isolation) — Future / MMU Targets
- [ ] **RISC-V H-extension support** — hypervisor extension (hgatp, VS/VU modes) for rv64 targets
- [ ] **ARM VHE / EL2 support** — type-2 hypervisor on ARM64 (stage-2 page tables, VGIC)
- [ ] **MicroVM descriptor** — virtual CPU count, memory size, device passthrough list, boot image
- [ ] **Trap-and-emulate** — guest traps forwarded to host handler; minimal device model
- [ ] **Device passthrough** — grant a MicroVM direct access to a physical MMIO device (e.g., SPI flash, radio)
- [ ] **Lightweight VMM** — < 10K lines; no BIOS emulation; direct kernel boot into guest

### 8C — Cryptographic Framework (PQC-Ready)
_Pluggable crypto with algorithm agility. Classical algorithms today, post-quantum drop-in tomorrow._

#### Crypto Trait Layer (`crates/crypto/`)
- [ ] **`crypto` crate** — `no_std`, `no_alloc` trait definitions; zero runtime cost when unused
- [ ] **`Hash` trait** — `update(&[u8])`, `finalize() -> Digest`; implementors: SHA-256, SHA-3-256, BLAKE3
- [ ] **`Kdf` trait** — key derivation: `derive(ikm, salt, info, len) -> [u8]`; HKDF-SHA256, HKDF-SHA3
- [ ] **`Aead` trait** — authenticated encryption: `seal/open(key, nonce, aad, plaintext) -> ciphertext`; AES-256-GCM, ChaCha20-Poly1305
- [ ] **`Sign` trait** — digital signatures: `sign(key, msg) -> Sig`, `verify(pubkey, msg, sig) -> bool`
- [ ] **`Kem` trait** — key encapsulation: `encapsulate(pubkey) -> (shared_secret, ciphertext)`, `decapsulate(privkey, ciphertext) -> shared_secret`
- [ ] **`Rng` trait** — cryptographic RNG: `fill_bytes(&mut [u8])`; backed by hardware TRNG or DRBG
- [ ] **Algorithm registry** — static dispatch via generics (no heap); feature flags select which algorithms are compiled in

#### Classical Algorithms (`sec-crypto-classical` feature)
- [ ] **SHA-256** — `no_std` implementation or thin wrapper over `sha2` crate (hash, HMAC, HKDF)
- [ ] **AES-256-GCM** — for authenticated encryption (TLS, secure IPC); hardware AES on ESP32-S3/C6 if available
- [ ] **ChaCha20-Poly1305** — software-friendly AEAD (fallback for cores without AES-NI)
- [ ] **Ed25519** — signature scheme for authentication, secure boot signature verification
- [ ] **X25519** — ECDH key agreement (SSH key exchange, TLS handshake)
- [ ] **Hardware RNG** — ESP32 `RNG_DATA_REG` (0x6002_6000), RISC-V `seed` CSR (Zkr), x86 `RDRAND`/`RDSEED`

#### Post-Quantum Algorithms (`sec-crypto-pqc` feature)
- [ ] **ML-KEM (Kyber)** — NIST FIPS 203 key encapsulation; ML-KEM-768 as default (balance of size vs security)
- [ ] **ML-DSA (Dilithium)** — NIST FIPS 204 digital signatures; ML-DSA-65 for general use
- [ ] **SLH-DSA (SPHINCS+)** — NIST FIPS 205 stateless hash-based signatures (fallback, larger but conservative)
- [ ] **Hybrid KEM** — X25519 + ML-KEM composite: classical + PQ in parallel, secure if either holds
- [ ] **Hybrid signatures** — Ed25519 + ML-DSA composite for transition period
- [ ] **PQC parameter profiles** — `pqc-128`, `pqc-192`, `pqc-256` security levels; selected via feature flag
- [ ] **Stack/memory budget** — ML-KEM-768 needs ~3KB stack, ML-DSA-65 ~5KB; validate fits in embedded task stacks

#### Crypto Services
- [ ] **Kernel keystore** — `[KeySlot; MAX_KEYS]` in protected kernel memory; keys never exposed to userspace raw
- [ ] **`SYS_CRYPTO_HASH` / `SYS_CRYPTO_SIGN` / `SYS_CRYPTO_VERIFY` / `SYS_CRYPTO_ENCRYPT` / `SYS_CRYPTO_DECRYPT`** — syscalls that operate on key handles (not raw key material)
- [ ] **`SYS_CRYPTO_KEM_ENCAP` / `SYS_CRYPTO_KEM_DECAP`** — PQC key exchange from userspace
- [ ] **`SYS_CRYPTO_RNG`** — fill buffer with cryptographically secure random bytes
- [ ] **Key capability** — `CryptoKey(key_id)` capability required to use a key; revocable, non-transferable for private keys
- [ ] **Crypto algorithm negotiation** — trait-based: callers request `AlgorithmClass::Kem` and kernel picks best available (PQC preferred → classical fallback)

### 8D — Secure Boot + Verified Launch
_Chain of trust from power-on to running user processes._

- [ ] **Boot signature verification** — kernel image signed with Ed25519 (+ ML-DSA hybrid for PQC); verified by bootloader before jump
- [ ] **ESP32 Secure Boot V2** — integrate with Espressif's eFuse-based secure boot (RSA-3072 / ECDSA key burned in eFuse)
- [ ] **Measured boot** — hash each boot stage into a measurement register (software TPM-like accumulator)
- [ ] **Process image verification** — verify signature/hash of ELF/binary before loading into a process/container
- [ ] **Immutable kernel .text** — mark kernel code region read-only + execute after boot; no self-modifying code
- [ ] **eFuse key provisioning** — tooling to burn signing keys into ESP32 eFuse block (one-time, irreversible)
- [ ] **Rollback protection** — monotonic version counter in eFuse/flash; reject images older than current version

### 8E — Secure Communication
_Encrypted channels for IPC, network, and debug interfaces._

- [ ] **TLS 1.3 (embedded)** — minimal TLS 1.3 client/server for TCP sockets; `no_std` + smoltcp integration
- [ ] **PQC cipher suites** — TLS 1.3 with ML-KEM hybrid key exchange + ML-DSA certificates (draft-ietf-tls-hybrid)
- [ ] **Encrypted IPC** — optional: inter-domain IPC messages encrypted with per-channel session key (for MicroVM ↔ host)
- [ ] **SSH protocol** — lightweight SSH-2 server (integrates with Phase 5); PQC key exchange via `sntrup761x25519-sha512` or ML-KEM hybrid
- [ ] **Secure serial** — optional encrypted UART channel (for physical debug port protection)
- [ ] **Certificate store** — small trusted CA certificate table in `.rodata` for TLS peer verification
- [ ] **ACME / auto-cert** — future: automatic certificate provisioning for network-connected devices

### 8F — Audit, Monitoring + Intrusion Detection
_Security logging and runtime integrity monitoring._

- [ ] **Security audit log** — ring buffer of security events: login attempts, capability grants/revokes, policy violations, crypto operations
- [ ] **`SYS_AUDIT_LOG` syscall** — userspace processes can append to audit log (if they hold `AUDIT_WRITE` capability)
- [ ] **Audit event types** — `AuthSuccess`, `AuthFail`, `CapGranted`, `CapRevoked`, `PolicyDenied`, `SyscallFiltered`, `IntegrityViolation`, `BootMeasurement`
- [ ] **Shell `auditlog` command** — display recent security events (root only)
- [ ] **Stack canaries** — compiler-based (`-Z stack-protector=strong`) or manual canary words; trap on corruption
- [ ] **Control flow integrity (CFI)** — RISC-V Zicfilp (landing pad) + Zicfiss (shadow stack) on supporting cores; software CFI fallback
- [ ] **Heap integrity checks** — allocator metadata validation on every alloc/free; panic on corruption
- [ ] **Runtime attestation** — device can prove its boot measurements + running software to a remote verifier

### 8G — Security Feature Integration Matrix
_How security tiers map to distribution profiles._

```
                    sec-base   sec-sandbox   sec-crypto   sec-verified
                    (always)   (feature)     (feature)    (feature)
────────────────────────────────────────────────────────────────────────
Capabilities          ✓           ✓             ✓            ✓
Syscall filter        ─           ✓             ─            ✓
Isolation domains     ─           ✓             ─            ✓
Containers            ─           ✓             ─            ✓
MicroVM               ─           ─             ─            ✓
Crypto traits         ─           ─             ✓            ✓
Classical crypto      ─           ─             ✓            ✓
PQC crypto            ─           ─             ✓*           ✓
Secure boot           ─           ─             ─            ✓
Measured boot         ─           ─             ─            ✓
TLS 1.3               ─           ─             ✓            ✓
Audit log             ─           ✓             ─            ✓
CFI / canaries        ─           ─             ─            ✓
────────────────────────────────────────────────────────────────────────
* PQC requires sec-crypto + sec-crypto-pqc sub-feature

Profile defaults:
  dist-minimal  → sec-base
  dist-app      → sec-base + sec-crypto
  dist-rt       → sec-base
  dist-full     → sec-base + sec-sandbox + sec-crypto + sec-verified
```

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
_How AI tiers map to hardware targets and distribution profiles._

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

Target capabilities:
  ESP32-C6 (320 KB)   → ai: keyword spotter, anomaly detector
  RPi Zero 2 W (512M) → ai: image classification, small models
  RPi 3 (1 GB)        → ai + ai-cloud: local small models + cloud LLM
  RPi 4 (4/8 GB)      → ai + ai-npu + ai-cloud: TinyLlama local, Coral TPU
  RPi 5 (8 GB)        → ai + ai-npu + ai-cloud + ai-os: Phi-3-mini local, Hailo-8, full AI-OS
  QEMU virt            → ai + ai-cloud: development/testing
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

Profile defaults:
  dist-firewall      → net-firewall + net-nat + net-vpn + net-dpi + net-shape
  dist-gateway       → net-firewall + net-nat (subset for IoT edge)
  dist-cloud         → net-firewall (basic filtering for cloud workloads)
```

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
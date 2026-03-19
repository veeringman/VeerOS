# VeerOS Build Tracker

This file is the persistent progress tracker for VeerOS and should be updated in every development session.

## V1 Scope
- [ ] Bootable microkernel on ESP32 RISC-V (C3/C6/H2)
- [ ] Modular architecture interfaces for future embedded CPU families
- [ ] Distribution variants via Rust feature flags
- [ ] Configurable single-user / multi-user system (feature-gated)
- [ ] Security-first architecture — capability-based access, isolation domains, PQC-ready crypto, extensible security model

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

#### Prerequisites
- [x] **WiFi driver skeleton** — `crates/soc/esp32/src/wifi.rs`: `WifiManager` state machine, `Esp32Wifi` driver struct, MAC init, clock/modem enable
- [x] **WiFi shell commands** — `wifi scan/list/set/connect/status` wired through `ShellEnv.wifi_cmd`
- [x] **smoltcp TCP/IP stack** — already integrated in `crates/net/` with `DeviceAdapter` bridge
- [x] **`TcpSerial`** — `Serial` trait over TCP socket (shell-over-TCP, proven on QEMU)
- [x] **SYSTIMER + interrupt pipeline** — working preemptive scheduler on ESP32-C6

#### Phase W1 — Espressif Radio Firmware Integration
_The ESP32-C6 WiFi/BLE RF is driven by proprietary Espressif blobs (libphy.a, libcoexist.a, libpp.a, etc.). We must link and initialize them._

- [ ] **Obtain esp-wifi blobs** — extract `libphy.a`, `libnet80211.a`, `libcoexist.a`, `libpp.a`, `libcore.a`, `libwpa_supplicant.a` from ESP-IDF v5.x or esp-wifi-sys crate
- [ ] **Link blobs into kernel** — add `.a` archives to `build.rs` link search, resolve extern symbols (`esp_wifi_init`, `esp_wifi_start`, `esp_wifi_connect`, etc.)
- [ ] **Implement blob FFI shim** — provide C-callable functions the blobs expect: `malloc`/`free` (→ VeerOS heap), `printf` (→ klog), `vTaskDelay` (→ sleep syscall), timer/mutex/semaphore OS abstractions
- [ ] **PHY calibration** — call `esp_phy_init()` with calibration data from NVS or defaults; RF registers init
- [ ] **WiFi supplicant init** — initialize WPA/WPA2/WPA3 supplicant from blob (handles 4-way handshake)
- [ ] **Coexistence init** — esp-coex init for WiFi/BLE shared antenna (already stub in modem.rs)
- [ ] **Alternative: `esp-wifi` crate** — evaluate using `esp-wifi` (Rust) from esp-rs project as a higher-level alternative to raw blobs

#### Phase W2 — WiFi STA Association
_Connect to an access point and complete the WPA handshake._

- [ ] **Scan implementation** — call blob `esp_wifi_scan_start()` → populate `scan_results` in `WifiManager`
- [ ] **Station mode connect** — `esp_wifi_set_mode(WIFI_MODE_STA)` → `esp_wifi_set_config()` with SSID/password → `esp_wifi_connect()`
- [ ] **Event handling** — register event callback for `WIFI_EVENT_STA_CONNECTED`, `WIFI_EVENT_STA_DISCONNECTED`, `IP_EVENT_STA_GOT_IP`
- [ ] **State machine updates** — drive `WifiManager` state: Configured → Connecting → Connected / Disconnected based on events
- [ ] **Auto-reconnect** — on disconnect event, retry connect with backoff (1s, 2s, 4s, max 30s)
- [ ] **`wifi status` shows RSSI** — read RSSI from blob and display signal strength in shell

#### Phase W3 — DHCP + IP Configuration
_Acquire an IP address from the network._

- [ ] **DHCP client in smoltcp** — enable smoltcp's `dhcpv4` feature; wire `Dhcpv4Client` into the network stack
- [ ] **`NetworkDevice` impl for ESP32 WiFi** — bridge between `Esp32Wifi` TX/RX packet buffers and smoltcp's `Device` trait
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

### Xtensa (ESP32-S3)
- [ ] `arch_xtensa` crate — `SavedContext` for Xtensa windowed ABI (A0–A15 + SAR + PS + PC), trap/exception frame
- [ ] `soc-esp32s3` crate — ESP32-S3 UART0 (0x6000_0000), interrupt matrix, SysTimer, WDT disable
- [ ] `kernel-esp32s3` crate — `xtensa-esp32s3-none-elf` target, single-core PRO_CPU boot (APP_CPU parked)
- [ ] Xtensa trap entry/exit — `call0` ABI exception vector, window overflow/underflow handlers
- [ ] ESP32-S3 memory map — 512 KB SRAM (IRAM 0x4037_0000 / DRAM 0x3FC8_8000), 16 KB RTC FAST
- [ ] ESP32-S3 interrupt controller — level + edge triggered, 32 CPU interrupts, priority 1–15
- [ ] Dual-core SMP stub — APP_CPU bring-up via `SYSTEM_CORE_1_CONTROL_0_REG`, per-core idle tasks
- [ ] ESP32-S3 radio drivers (stubs) — WiFi 802.11 b/g/n, BLE 5.0 (shared RF with C6 radio abstraction)
- [ ] PSRAM support — optional 2–8 MB octal SPI PSRAM mapped at 0x3C00_0000 (cache-through)
- [ ] USB-OTG serial — ESP32-S3 USB-OTG peripheral for console I/O (alternative to UART0)

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

### x86-64
- [ ] `soc-qemu-pc` crate — serial (COM1 0x3F8), APIC timer, PIC/IOAPIC
- [ ] `kernel-qemu-pc` — `x86_64-unknown-none` target, multiboot2 boot, long mode
- [ ] IDT setup — interrupt descriptor table, ISR stubs, syscall via `syscall`/`sysret`
- [ ] GDT + TSS — kernel/user segment selectors, per-CPU task state segment

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
_Machine learning inference, neural processing unit abstraction, and AI-assisted OS services — built into the kernel as a core capability, not a userspace afterthought. Feature-gated: `ai` (core traits + tiny inference), `ai-npu` (hardware accelerator), `ai-cloud` (cloud inference), `ai-os` (AI-enhanced kernel services)._

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

#### Anomaly Detection + Security
- [ ] **Syscall anomaly detector** — per-process syscall sequence model (Markov chain or tiny LSTM); flags unusual patterns → security audit log (8F)
- [ ] **Memory access anomaly** — detect unusual memory access patterns that may indicate exploitation; raise `IntegrityViolation` event
- [ ] **Network traffic classifier** — classify inbound packets (benign/suspicious/malicious) using tiny CNN on packet headers; integrates with firewall rules

#### Smart Shell + User Experience
- [ ] **Shell command prediction** — suggest next command based on history + context (tiny GPT-2-like model or n-gram)
- [ ] **Natural language commands (future)** — `veeros> "show me running tasks sorted by priority"` → auto-translates to `tasks --sort=priority`
- [ ] **On-device AI assistant** — LLM-powered help system: `veeros> ai "how do I create a new process?"` → contextual VeerOS documentation answer
- [ ] **Log summarization** — AI-generated summary of recent audit log, boot messages, or error patterns

#### Sensor Fusion + IoT Intelligence
- [ ] **Sensor pipeline** — raw sensor data → preprocessing → inference → action; declarative configuration: `{sensor: "temp", model: "anomaly", action: "alert"}`
- [ ] **Edge inference orchestrator** — fleet of VeerOS devices coordinate inference: split model across nodes, aggregate results
- [ ] **Federated learning (stubs)** — on-device model training with gradient sharing; no raw data leaves the device; privacy-preserving AI

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
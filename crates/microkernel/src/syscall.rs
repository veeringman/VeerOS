//! VeerOS syscall ABI — shared between kernel and userlib.
//!
//! # Register convention (RISC-V)
//!
//! ```text
//! a7 (x17) — syscall number
//! a0 (x10) — argument 0 / return value 0
//! a1 (x11) — argument 1 / return value 1
//! a2 (x12) — argument 2
//! a3 (x13) — argument 3
//! ```
//!
//! The `ecall` instruction traps into the kernel. The handler reads `a7`
//! to determine which service is requested, processes arguments from
//! `a0`–`a3`, writes results back into `a0`–`a1` in the saved context,
//! advances `mepc` by 4, and returns.
//!
//! # Syscall numbering
//!
//! Numbers are grouped by subsystem:
//!
//! | Range     | Subsystem        |
//! |-----------|------------------|
//! | 0x00–0x0F | Task control     |
//! | 0x10–0x1F | IPC              |
//! | 0x20–0x2F | I/O / Console    |
//! | 0x30–0x3F | Time             |
//! | 0x40–0x4F | Memory           |/// | 0x50–0x57 | Synchronization  |
/// | 0x58–0x5F | Channels         |//! | 0x80–0x8F | Debug / Diag     |

// ═══════════════════════════════════════════════════════════════════════════
// Task control (0x00–0x0F)
// ═══════════════════════════════════════════════════════════════════════════

/// Yield the current timeslice.  Takes no arguments, returns nothing.
pub const SYS_YIELD: usize = 0x00;

/// Exit the current task.  `a0` = exit code.  Does not return.
pub const SYS_EXIT: usize = 0x01;

/// Get the current task's ID.  Returns task index in `a0`.
pub const SYS_TASK_ID: usize = 0x02;

/// Get the current task's priority.  Returns priority in `a0`.
pub const SYS_TASK_PRIORITY: usize = 0x03;

/// Get the number of active (non-Free) tasks.  Returns count in `a0`.
pub const SYS_TASK_COUNT: usize = 0x04;

/// Spawn a new task.
///   a0 = entry point (function pointer)
///   a1 = stack pointer (top of pre-allocated stack)
///   a2 = stack bottom address
///   a3 = priority
/// Returns: a0 = new task ID on success, `usize::MAX` on failure.
pub const SYS_SPAWN: usize = 0x05;

/// Wait for a task to exit and retrieve its exit code.
///   a0 = task ID to join
/// Returns: a0 = exit code of the joined task.
/// Blocks the caller until the target task exits.
pub const SYS_JOIN: usize = 0x06;

/// Spawn a new process with its own address space.
///   a0 = entry point (function pointer for initial thread)
///   a1 = stack top
///   a2 = stack bottom
///   a3 = priority
/// Returns: a0 = new process ID on success, `usize::MAX` on failure.
pub const SYS_SPAWN_PROCESS: usize = 0x07;

/// Get the calling thread's process ID.
/// Returns: a0 = process ID.
pub const SYS_PROCESS_ID: usize = 0x08;

/// Get the number of threads in a process.
///   a0 = process ID
/// Returns: a0 = thread count (0 if invalid).
pub const SYS_THREAD_COUNT: usize = 0x09;

/// Get the calling thread's TLS base pointer.
/// Returns: a0 = TLS base address (0 if not set).
pub const SYS_TLS_GET: usize = 0x0A;

/// Set the calling thread's TLS base pointer.
///   a0 = TLS base address
pub const SYS_TLS_SET: usize = 0x0B;

// ═══════════════════════════════════════════════════════════════════════════
// IPC (0x10–0x1F)
// ═══════════════════════════════════════════════════════════════════════════

/// Send a message.
///   a0 = destination task ID
///   a1 = opcode
///   a2 = arg0
///   a3 = arg1
/// Returns: a0 = 1 on success, 0 on failure.
pub const SYS_IPC_SEND: usize = 0x10;

/// Receive a message (blocking).
///   (no arguments)
/// Returns: a0 = sender | (opcode << 8), a1 = msg.arg0
/// If no message is pending, the task is blocked until one arrives.
/// On wakeup, returns the message contents.
pub const SYS_IPC_RECV: usize = 0x11;

/// Non-blocking check: is there a pending message?
///   (no arguments)
/// Returns: a0 = 1 if message pending, 0 otherwise.
pub const SYS_IPC_POLL: usize = 0x12;

// ═══════════════════════════════════════════════════════════════════════════
// I/O / Console (0x20–0x2F)
// ═══════════════════════════════════════════════════════════════════════════

/// Write a single byte to the kernel console.
///   a0 = byte value (low 8 bits)
pub const SYS_WRITE_BYTE: usize = 0x20;

/// Write a buffer to the kernel console.
///   a0 = pointer to buffer
///   a1 = length in bytes
pub const SYS_WRITE_BUF: usize = 0x21;

/// Read a byte from the kernel console (blocking).
/// Returns: a0 = byte value.
pub const SYS_READ_BYTE: usize = 0x22;

// ═══════════════════════════════════════════════════════════════════════════
// Time (0x30–0x3F)
// ═══════════════════════════════════════════════════════════════════════════

/// Get the current tick counter.
/// Returns: a0 = low 32 bits, a1 = high 32 bits.
pub const SYS_TICK: usize = 0x30;

/// Sleep for N ticks (approximate — unblocks on the next tick after N).
///   a0 = number of ticks to sleep
pub const SYS_SLEEP: usize = 0x31;

// ═══════════════════════════════════════════════════════════════════════════
// Memory (0x40–0x4F)
// ═══════════════════════════════════════════════════════════════════════════

/// Allocate a block of memory from the kernel heap.
///   a0 = size in bytes
/// Returns: a0 = pointer (0 on failure).
pub const SYS_ALLOC: usize = 0x40;

/// Free a previously allocated block.
///   a0 = pointer
///   a1 = size (must match the original alloc)
pub const SYS_FREE: usize = 0x41;

/// Query how many memory regions are granted to the calling task.
/// Returns: a0 = region count.
pub const SYS_MEM_REGION_COUNT: usize = 0x42;

/// Query a specific memory region by index.
///   a0 = region index (0-based)
/// Returns: a0 = base address, a1 = size.
/// Returns a0=0, a1=0 if index is out of range.
pub const SYS_MEM_REGION_INFO: usize = 0x43;

// ═══════════════════════════════════════════════════════════════════════════
// Synchronization (0x50–0x5F)
// ═══════════════════════════════════════════════════════════════════════════

/// Futex wait — if `*addr == expected`, block the calling task.
///   a0 = addr (pointer to a usize futex word)
///   a1 = expected value
/// Returns: a0 = 0 on wakeup, 1 if value didn't match (no block).
pub const SYS_FUTEX_WAIT: usize = 0x50;

/// Futex wake — wake up to `count` tasks waiting on `addr`.
///   a0 = addr (pointer to a usize futex word)
///   a1 = count (max tasks to wake; 1 for mutex unlock, usize::MAX for broadcast)
/// Returns: a0 = number of tasks actually woken.
pub const SYS_FUTEX_WAKE: usize = 0x51;

// ═══════════════════════════════════════════════════════════════════════════
// Channels (0x58–0x5F)
// ═══════════════════════════════════════════════════════════════════════════

/// Create a bounded channel.
///   (no arguments)
/// Returns: a0 = channel ID on success, `usize::MAX` on failure.
pub const SYS_CHAN_CREATE: usize = 0x58;

/// Send on a channel (blocks if full).
///   a0 = channel ID
///   a1 = word0
///   a2 = word1
/// Returns: a0 = 1 on success, 0 on closed channel.
pub const SYS_CHAN_SEND: usize = 0x59;

/// Receive from a channel (blocks if empty).
///   a0 = channel ID
/// Returns: a0 = word0, a1 = word1.  If channel closed: a0 = usize::MAX, a1 = 0.
pub const SYS_CHAN_RECV: usize = 0x5A;

/// Close a channel (wake all blocked senders/receivers).
///   a0 = channel ID
/// Returns: a0 = 1 on success, 0 on invalid.
pub const SYS_CHAN_CLOSE: usize = 0x5B;

/// Non-blocking poll: check if channel has messages.
///   a0 = channel ID
/// Returns: a0 = number of messages in the queue (0 if empty or invalid).
pub const SYS_CHAN_POLL: usize = 0x5C;

// ═══════════════════════════════════════════════════════════════════════════
// Poll / Async events (0x60–0x6F)
// ═══════════════════════════════════════════════════════════════════════════

/// Clear the calling task's event set and register interest in events.
///   a0 = event mask (bitmask of `PollEvent` interest flags)
///   a1 = optional parameter (e.g., channel ID for CHAN_READABLE, task ID for TASK_EXIT)
/// Returns: a0 = 0 on success.
///
/// The event mask is an OR of `POLL_*` constants.
pub const SYS_POLL_SET: usize = 0x60;

/// Block until any registered event fires (or has already fired).
///   a0 = timeout in ticks (0 = poll without blocking, usize::MAX = wait forever)
/// Returns: a0 = bitmask of events that fired.
///          If timeout expires with no events: a0 = 0.
pub const SYS_POLL_WAIT: usize = 0x61;

// ═══════════════════════════════════════════════════════════════════════════
// Sockets (0x70–0x7F)
// ═══════════════════════════════════════════════════════════════════════════

/// Create a socket.
///   a0 = domain (0 = LOCAL, 1 = INET)
///   a1 = socket type (0 = STREAM, 1 = DGRAM)
/// Returns: a0 = socket handle on success, `usize::MAX` on failure.
pub const SYS_SOCKET: usize = 0x70;

/// Bind a socket to an address.
///   a0 = socket handle
///   a1 = address / port number
/// Returns: a0 = 0 on success, `usize::MAX` on failure.
pub const SYS_BIND: usize = 0x71;

/// Mark a socket as listening for connections.
///   a0 = socket handle
///   a1 = backlog (ignored for now)
/// Returns: a0 = 0 on success, `usize::MAX` on failure.
pub const SYS_LISTEN: usize = 0x72;

/// Accept a connection on a listening socket (blocks until a client connects).
///   a0 = listening socket handle
/// Returns: a0 = new connected socket handle, or `usize::MAX` on error.
pub const SYS_ACCEPT: usize = 0x73;

/// Connect to a remote socket address.
///   a0 = socket handle
///   a1 = address / port number
/// Returns: a0 = 0 on success, `usize::MAX` on failure.
pub const SYS_CONNECT: usize = 0x74;

/// Send data on a connected socket.
///   a0 = socket handle
///   a1 = pointer to buffer
///   a2 = length in bytes
/// Returns: a0 = number of bytes sent, or `usize::MAX` on error.
pub const SYS_SOCK_SEND: usize = 0x75;

/// Receive data from a connected socket (blocks if no data).
///   a0 = socket handle
///   a1 = pointer to buffer
///   a2 = max bytes to read
/// Returns: a0 = number of bytes received, 0 on EOF, `usize::MAX` on error.
pub const SYS_SOCK_RECV: usize = 0x76;

/// Close a socket and release resources.
///   a0 = socket handle
/// Returns: a0 = 0 on success, `usize::MAX` on failure.
pub const SYS_SOCK_CLOSE: usize = 0x77;

/// Event interest flags (passed in `a0` of `SYS_POLL_SET`).
/// These correspond to wakeup sources that the poll subsystem checks.
pub const POLL_TIMER: usize = 1 << 0;   // Timer expired (param = ticks to wait)
pub const POLL_IPC: usize = 1 << 1;     // IPC message pending
pub const POLL_CHAN_READABLE: usize = 1 << 2;  // Channel has data (param = chan_id)
pub const POLL_CHAN_WRITABLE: usize = 1 << 3;  // Channel has space (param = chan_id)
pub const POLL_TASK_EXIT: usize = 1 << 4;      // Task has exited (param = task_id)

// ═══════════════════════════════════════════════════════════════════════════
// Debug / diagnostics (0x80–0x8F)
// ═══════════════════════════════════════════════════════════════════════════

/// Kernel panic from user space — prints a message and halts.
///   a0 = pointer to message (null-terminated or with length in a1)
///   a1 = length
pub const SYS_PANIC: usize = 0x80;

/// Get the platform name string.
/// Returns: a0 = pointer to static string, a1 = length.
pub const SYS_PLATFORM_NAME: usize = 0x81;

// ═══════════════════════════════════════════════════════════════════════════
// User Identity (0x90–0x9F)
// ═══════════════════════════════════════════════════════════════════════════

/// Get the calling process's user ID.
/// Returns: a0 = UID.
pub const SYS_GETUID: usize = 0x90;

/// Get the calling process's group ID.
/// Returns: a0 = GID.
pub const SYS_GETGID: usize = 0x91;

/// Set the calling process's UID (root only).
///   a0 = new UID
/// Returns: a0 = 0 on success, `usize::MAX` on permission denied.
pub const SYS_SETUID: usize = 0x92;

/// Login: validate credentials and create a session.
///   a0 = pointer to username string
///   a1 = username length
///   a2 = pointer to password string
///   a3 = password length
/// Returns: a0 = session token on success, 0 on failure.
pub const SYS_LOGIN: usize = 0x93;

/// Logout: invalidate the session associated with this process.
/// Returns: a0 = 1 on success, 0 on failure.
pub const SYS_LOGOUT: usize = 0x94;

// ═══════════════════════════════════════════════════════════════════════════
// Filesystem (0xA0–0xAF)
// ═══════════════════════════════════════════════════════════════════════════

/// Open a file or directory.
///   a0 = pointer to path string
///   a1 = path length
///   a2 = flags (OpenFlags bitmask)
/// Returns: a0 = file descriptor on success, `usize::MAX` on error.
pub const SYS_OPEN: usize = 0xA0;

/// Close a file descriptor.
///   a0 = fd
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_CLOSE: usize = 0xA1;

/// Read from a file descriptor.
///   a0 = fd
///   a1 = pointer to buffer
///   a2 = max bytes to read
/// Returns: a0 = bytes read, 0 on EOF, `usize::MAX` on error.
pub const SYS_READ: usize = 0xA2;

/// Write to a file descriptor.
///   a0 = fd
///   a1 = pointer to buffer
///   a2 = bytes to write
/// Returns: a0 = bytes written, `usize::MAX` on error.
pub const SYS_WRITE: usize = 0xA3;

/// Seek within a file.
///   a0 = fd
///   a1 = offset
///   a2 = whence (0=SET, 1=CUR, 2=END)
/// Returns: a0 = new position, `usize::MAX` on error.
pub const SYS_SEEK: usize = 0xA4;

/// Stat a path (get file metadata).
///   a0 = pointer to path string
///   a1 = path length
///   a2 = pointer to StatBuf
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_STAT: usize = 0xA5;

/// Fstat (stat an open file descriptor).
///   a0 = fd
///   a1 = pointer to StatBuf
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_FSTAT: usize = 0xA6;

/// Create a directory.
///   a0 = pointer to path string
///   a1 = path length
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_MKDIR: usize = 0xA7;

/// Unlink (remove) a file or empty directory.
///   a0 = pointer to path string
///   a1 = path length
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_UNLINK: usize = 0xA8;

/// Read directory entries.
///   a0 = fd (must be an open directory)
///   a1 = pointer to DirEntry array
///   a2 = max entries
/// Returns: a0 = number of entries written, `usize::MAX` on error.
pub const SYS_READDIR: usize = 0xA9;

/// Truncate a file to a given size.
///   a0 = fd
///   a1 = new size
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_TRUNCATE: usize = 0xAA;

/// Rename / move a file or directory.
///   a0 = pointer to old path
///   a1 = old path length
///   a2 = pointer to new path
///   a3 = new path length
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_RENAME: usize = 0xAB;

/// Get the current working directory.
///   a0 = pointer to buffer
///   a1 = buffer length
/// Returns: a0 = bytes written, `usize::MAX` on error.
pub const SYS_GETCWD: usize = 0xAC;

/// Change the current working directory.
///   a0 = pointer to path string
///   a1 = path length
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_CHDIR: usize = 0xAD;

/// Mount a filesystem.
///   a0 = pointer to device label string  (e.g. "sd0")
///   a1 = label length
///   a2 = pointer to mount-path string   (e.g. "/mnt/sd")
///   a3 = mount-path length
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_MOUNT: usize = 0xAE;

/// Unmount a filesystem.
///   a0 = pointer to mount-path string
///   a1 = path length
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_UMOUNT: usize = 0xAF;

// ── Hardware / peripheral syscalls (0xB0 – 0xBF) ────────────────────────

/// GPIO set mode.
///   a0 = pin   (0–27)
///   a1 = mode  (0 = input, 1 = output)
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_GPIO_SET_MODE: usize = 0xB0;

/// GPIO read.
///   a0 = pin
/// Returns: a0 = 0 or 1.
pub const SYS_GPIO_READ: usize = 0xB1;

/// GPIO write.
///   a0 = pin
///   a1 = 0 or 1
/// Returns: a0 = 0 on success.
pub const SYS_GPIO_WRITE: usize = 0xB2;

/// GPIO set pull-up/down.
///   a0 = pin
///   a1 = 0 (none), 1 (up), 2 (down)
/// Returns: a0 = 0 on success.
pub const SYS_GPIO_SET_PULL: usize = 0xB3;

/// I2C write.
///   a0 = bus
///   a1 = 7-bit addr
///   a2 = pointer to data
///   a3 = length
/// Returns: a0 = 0 on success, error code on failure.
pub const SYS_I2C_WRITE: usize = 0xB4;

/// I2C read.
///   a0 = bus
///   a1 = 7-bit addr
///   a2 = pointer to buffer
///   a3 = length
/// Returns: a0 = bytes read, `usize::MAX` on error.
pub const SYS_I2C_READ: usize = 0xB5;

/// SPI transfer.
///   a0 = bus
///   a1 = pointer to TX buffer
///   a2 = pointer to RX buffer
///   a3 = length
/// Returns: a0 = 0 on success.
pub const SYS_SPI_TRANSFER: usize = 0xB6;

/// Get board temperature in millidegrees C.
/// Returns: a0 = temperature in millidegrees (signed, fits in usize bits).
pub const SYS_GET_TEMP: usize = 0xB7;

/// Get platform/hardware info string.
///   a0 = pointer to output buffer
///   a1 = buffer length
/// Returns: a0 = bytes written.
pub const SYS_HW_INFO: usize = 0xB8;

// ═══════════════════════════════════════════════════════════════════════════
// Driver / userspace I/O (0xC0–0xCF)
// ═══════════════════════════════════════════════════════════════════════════

/// Read a 32-bit MMIO register (kernel validates PMP grant).
///   a0 = physical address
/// Returns: a0 = value read.
pub const SYS_DRV_MMIO_READ32: usize = 0xC0;

/// Write a 32-bit MMIO register (kernel validates PMP grant).
///   a0 = physical address
///   a1 = value to write
/// Returns: a0 = 0 on success, `usize::MAX` on denied.
pub const SYS_DRV_MMIO_WRITE32: usize = 0xC1;

/// Block until the driver's assigned IRQ fires.
///   a0 = IRQ line the driver is registered for
/// Returns: a0 = 0 on wakeup.
pub const SYS_DRV_IRQ_WAIT: usize = 0xC2;

/// Acknowledge a driver IRQ (re-enable the interrupt).
///   a0 = IRQ line
/// Returns: a0 = 0 on success.
pub const SYS_DRV_IRQ_ACK: usize = 0xC3;

/// Register the calling task as a driver server on a named channel.
///   a0 = channel ID (previously created via SYS_CHAN_CREATE)
///   a1 = driver handle (from kernel's DriverRegistry)
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_DRV_REGISTER: usize = 0xC4;

/// Send a log/status message to the kernel console (for driver debug).
///   a0 = pointer to message
///   a1 = length
/// Returns: a0 = 0.
pub const SYS_DRV_LOG: usize = 0xC5;

// ═══════════════════════════════════════════════════════════════════════════
// Capability management (0xD0–0xDF)
// ═══════════════════════════════════════════════════════════════════════════

/// Query own capability bits.
///   (no args)
/// Returns: a0 = ProcessCaps bits.
pub const SYS_CAP_GET: usize = 0xD0;

/// Drop capabilities from own process (irrevocable).
///   a0 = ProcessCaps bits to remove
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_CAP_DROP: usize = 0xD1;

/// Set capability mask for a child process (before it starts execution).
///   a0 = child pid
///   a1 = ProcessCaps bits to set (must be subset of own caps)
/// Returns: a0 = 0 on success, `usize::MAX` on error.
pub const SYS_CAP_SET_CHILD: usize = 0xD2;

// ═══════════════════════════════════════════════════════════════════════════
// Audit (0xE0–0xEF)
// ═══════════════════════════════════════════════════════════════════════════

/// Read audit log entries.
///   a0 = start index (0 = oldest)
///   a1 = pointer to output buffer (caller-provided AuditEntry array)
///   a2 = max entries to read
/// Returns: a0 = number of entries written.
pub const SYS_AUDIT_READ: usize = 0xE0;

/// Get audit log statistics.
///   (no args)
/// Returns: a0 = total events recorded, a1 = current entry count in buffer.
pub const SYS_AUDIT_COUNT: usize = 0xE1;

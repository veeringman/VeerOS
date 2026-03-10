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
//! | 0x40–0x4F | Memory           |
//! | 0x80–0x8F | Debug / Diag     |

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

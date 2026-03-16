//! Task control syscalls.
//!
//! These wrap the kernel's task management syscalls into a safe,
//! ergonomic Rust API.

use crate::sys;

// Syscall numbers (must match microkernel::syscall)
const SYS_YIELD: usize      = 0x00;
const SYS_EXIT: usize       = 0x01;
const SYS_TASK_ID: usize    = 0x02;
const SYS_TASK_PRIORITY: usize = 0x03;
const SYS_TASK_COUNT: usize = 0x04;
const SYS_SPAWN: usize      = 0x05;
const SYS_JOIN: usize       = 0x06;
const SYS_SPAWN_PROCESS: usize = 0x07;
const SYS_PROCESS_ID: usize = 0x08;
const SYS_THREAD_COUNT: usize = 0x09;
const SYS_TLS_GET: usize    = 0x0A;
const SYS_TLS_SET: usize    = 0x0B;

/// Yield the current timeslice to the scheduler.
///
/// The task remains `Ready` and will be scheduled again when its turn
/// comes. Use this in polling loops to avoid starving other tasks.
#[inline]
pub fn yield_now() {
    sys::syscall0(SYS_YIELD);
}

/// Exit the current task with the given exit code.
///
/// The task's TCB slot is freed and will not be scheduled again.
/// This function never returns.
#[inline]
pub fn exit(code: u32) -> ! {
    sys::syscall1(SYS_EXIT, code as usize);
    // The kernel will not return here, but the compiler needs convincing.
    loop {
        core::hint::spin_loop();
    }
}

/// Get the current task's index (ID) in the kernel task table.
#[inline]
pub fn id() -> usize {
    sys::syscall0(SYS_TASK_ID)
}

/// Get the current task's priority level.
#[inline]
pub fn priority() -> u8 {
    sys::syscall0(SYS_TASK_PRIORITY) as u8
}

/// Get the number of active (non-Free) tasks in the system.
#[inline]
pub fn task_count() -> usize {
    sys::syscall0(SYS_TASK_COUNT)
}

/// Spawn a new task.
///
/// Returns the new task's ID on success, or `None` if the task table is full.
///
/// # Arguments
/// * `entry` — function pointer the new task will begin executing.
/// * `stack_top` — top (highest address) of the new task's stack.
/// * `stack_bottom` — bottom (lowest address) of the new task's stack.
/// * `priority` — scheduling priority (higher = more important).
#[inline]
pub fn spawn(entry: fn(), stack_top: usize, stack_bottom: usize, priority: u8) -> Option<usize> {
    let id = sys::syscall4(SYS_SPAWN, entry as usize, stack_top, stack_bottom, priority as usize);
    if id == usize::MAX { None } else { Some(id) }
}

/// Wait for a task to exit and retrieve its exit code.
///
/// Blocks the calling task until the target task has exited.
/// Returns the exit code the target passed to `exit()`.
#[inline]
pub fn join(task_id: usize) -> usize {
    sys::syscall1(SYS_JOIN, task_id)
}

/// Get the current task's process ID.
#[inline]
pub fn process_id() -> usize {
    sys::syscall0(SYS_PROCESS_ID)
}

/// Get the number of threads in the current process.
#[inline]
pub fn thread_count() -> usize {
    sys::syscall0(SYS_THREAD_COUNT)
}

/// Get the current thread's TLS base pointer.
#[inline]
pub fn tls_get() -> usize {
    sys::syscall0(SYS_TLS_GET)
}

/// Set the current thread's TLS base pointer.
#[inline]
pub fn tls_set(base: usize) {
    sys::syscall1(SYS_TLS_SET, base);
}

/// Spawn a new process with an initial thread.
///
/// Returns `(process_id, thread_id)` on success, or `None` if full.
///
/// # Arguments
/// * `entry` — entry point for the new process's initial thread.
/// * `stack_top` — top (highest address) of the initial thread's stack.
/// * `stack_bottom` — bottom (lowest address) of the initial thread's stack.
/// * `priority` — scheduling priority of the initial thread.
#[inline]
pub fn spawn_process(entry: fn(), stack_top: usize, stack_bottom: usize, priority: u8) -> Option<(usize, usize)> {
    let result = sys::syscall4(SYS_SPAWN_PROCESS, entry as usize, stack_top, stack_bottom, priority as usize);
    if result == usize::MAX { None } else {
        let pid = result >> 16;
        let tid = result & 0xFFFF;
        Some((pid, tid))
    }
}

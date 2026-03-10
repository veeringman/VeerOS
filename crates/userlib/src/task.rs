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

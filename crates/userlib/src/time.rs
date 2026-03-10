//! Time-related syscall wrappers.
//!
//! All durations are expressed in kernel tick units. The tick rate is
//! platform-dependent (typically 1 ms on QEMU, varies on real hardware).

use crate::sys;

// Syscall numbers (must match microkernel::syscall)
const SYS_TICK: usize = 0x30;
const SYS_SLEEP: usize = 0x31;

/// Return the current kernel tick count.
///
/// The 64-bit counter is returned in two registers: low word in `a0`,
/// high word in `a1`.
#[inline]
pub fn ticks() -> u64 {
    let (lo, hi) = sys::syscall2(SYS_TICK, 0, 0);
    (lo as u64) | ((hi as u64) << 32)
}

/// Sleep for at least `duration` ticks.
///
/// The task is placed in a blocked state and will be woken by the
/// kernel's timer ISR once the tick counter exceeds the target. The
/// actual delay may be slightly longer than requested due to scheduling
/// latency.
#[inline]
pub fn sleep(duration: u32) {
    sys::syscall1(SYS_SLEEP, duration as usize);
}

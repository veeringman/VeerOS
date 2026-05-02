//! HPET (High Precision Event Timer) driver.
//!
//! The HPET provides a monotonic 64-bit counter at ≥10 MHz and up to 32
//! comparison registers for periodic / one-shot interrupts. It's accessed
//! via MMIO at a base address discovered in the ACPI HPET table.
//!
//! On QEMU, the HPET is always present at the default address 0xFED0_0000
//! when using the q35 or pc machine type.

use core::sync::atomic::{AtomicU64, Ordering};

/// Default HPET MMIO base (QEMU).
const HPET_BASE: usize = 0xFED0_0000;

// ─── Register offsets (byte offsets from base) ──────────────────────────

/// General Capabilities and ID register (64-bit, RO).
const REG_CAP_ID: usize = 0x000;
/// General Configuration register (64-bit, R/W).
const REG_CONFIG: usize = 0x010;
/// General Interrupt Status register (64-bit, R/W-clear).
const REG_INT_STATUS: usize = 0x020;
/// Main Counter Value register (64-bit, R/W).
const REG_COUNTER: usize = 0x0F0;

/// Timer N Configuration and Capabilities (64-bit, R/W).
/// Timer N starts at 0x100 + 0x20*N.
const fn timer_config(n: usize) -> usize {
    0x100 + 0x20 * n
}
/// Timer N Comparator Value (64-bit, R/W).
const fn timer_comparator(n: usize) -> usize {
    0x108 + 0x20 * n
}

// ─── Configuration bits ─────────────────────────────────────────────────

/// Enable the main HPET counter.
const CONFIG_ENABLE: u64 = 1 << 0;
/// Enable legacy replacement routing (Timer 0 → IRQ 0, Timer 1 → IRQ 8).
const CONFIG_LEGACY: u64 = 1 << 1;

/// Timer config: interrupt enable.
const TN_INT_ENB: u64 = 1 << 2;
/// Timer config: periodic mode enable.
const TN_PERIODIC: u64 = 1 << 3;
/// Timer config: supports periodic.
const TN_PER_CAP: u64 = 1 << 4;
/// Timer config: 64-bit capable.
const TN_64BIT_CAP: u64 = 1 << 5;
/// Timer config: set accumulator (write 1 to write to comparator in periodic mode).
const TN_SET_ACC: u64 = 1 << 6;
/// Timer config: force 32-bit mode.
const TN_32BIT_MODE: u64 = 1 << 8;

// ─── MMIO helpers ───────────────────────────────────────────────────────

#[inline]
unsafe fn hpet_read64(offset: usize) -> u64 {
    let ptr = (HPET_BASE + offset) as *const u64;
    unsafe { ptr.read_volatile() }
}

#[inline]
unsafe fn hpet_write64(offset: usize, val: u64) {
    let ptr = (HPET_BASE + offset) as *mut u64;
    unsafe {
        ptr.write_volatile(val);
    }
}

// ─── Tick counter (for TickTimer trait) ──────────────────────────────────

/// Software tick counter, incremented by HPET timer 0 comparator match.
pub static HPET_TICK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Cached period in femtoseconds (set by `init()`).
static mut PERIOD_FS: u64 = 0;

/// HPET timer handle.
pub struct Hpet;

impl Hpet {
    pub const fn new() -> Self {
        Self
    }
}

// ─── Public API ─────────────────────────────────────────────────────────

/// Read the HPET counter period in femtoseconds from the capabilities register.
pub fn period_femtoseconds() -> u64 {
    unsafe { hpet_read64(REG_CAP_ID) >> 32 }
}

/// Read the number of comparators (timers) available.
pub fn num_timers() -> usize {
    let cap = unsafe { hpet_read64(REG_CAP_ID) };
    (((cap >> 8) & 0x1F) + 1) as usize
}

/// Read the main counter value.
pub fn counter() -> u64 {
    unsafe { hpet_read64(REG_COUNTER) }
}

/// Initialise the HPET: stop counter, reset to 0, configure timer 0 in
/// periodic mode for the given period in microseconds, then start.
///
/// Uses "legacy replacement" routing: Timer 0 → IRQ 0 → I/O APIC pin 2
/// (or LAPIC vector 32 if routed via I/O APIC).
///
/// Returns `true` if initialisation succeeded, `false` if the HPET timer 0
/// does not support periodic mode.
pub fn init(period_us: u32) -> bool {
    unsafe {
        // Cache the counter period.
        PERIOD_FS = period_femtoseconds();
        if PERIOD_FS == 0 {
            return false;
        }

        // Stop the counter.
        hpet_write64(REG_CONFIG, 0);

        // Reset main counter to 0.
        hpet_write64(REG_COUNTER, 0);

        // Check if timer 0 supports periodic mode.
        let t0_cap = hpet_read64(timer_config(0));
        if t0_cap & TN_PER_CAP == 0 {
            return false;
        }

        // Compute comparator value: period_us in counter ticks.
        // ticks = period_us * 1_000_000_000_000 / period_fs
        let period_fs = PERIOD_FS;
        let ticks = (period_us as u64)
            .checked_mul(1_000_000_000) // us → fs partial (avoid overflow by splitting)
            .unwrap_or(u64::MAX)
            .checked_mul(1_000)
            .unwrap_or(u64::MAX)
            / period_fs;

        // Configure timer 0: periodic, interrupt enable, 32-bit or 64-bit.
        let mut cfg = TN_INT_ENB | TN_PERIODIC | TN_SET_ACC;
        if t0_cap & TN_64BIT_CAP == 0 {
            cfg |= TN_32BIT_MODE;
        }
        hpet_write64(timer_config(0), cfg);
        hpet_write64(timer_comparator(0), ticks);

        // Enable the main counter with legacy replacement routing.
        hpet_write64(REG_CONFIG, CONFIG_ENABLE | CONFIG_LEGACY);

        true
    }
}

/// Acknowledge a timer 0 interrupt (clear the interrupt status bit).
pub fn clear_timer0() {
    HPET_TICK_COUNTER.fetch_add(1, Ordering::Relaxed);
    unsafe {
        hpet_write64(REG_INT_STATUS, 1 << 0); // clear T0 status
    }
}

/// Read the monotonic counter in microseconds.
pub fn counter_us() -> u64 {
    let period_fs = unsafe { PERIOD_FS };
    if period_fs == 0 {
        return 0;
    }
    // counter * period_fs / 1_000_000_000_000 = microseconds... but that overflows.
    // Use: (counter * period_fs) / 1_000_000_000 → nanoseconds, then / 1000 → us.
    // Still can overflow — use u128.
    let c = counter() as u128;
    let pfs = period_fs as u128;
    ((c * pfs) / 1_000_000_000_000u128) as u64
}

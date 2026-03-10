//! CLINT (Core Local Interruptor) timer driver for QEMU `virt`.
//!
//! The CLINT lives at 0x0200_0000 and provides:
//!   mtime      (0xBFF8) — 64-bit free-running counter
//!   mtimecmp_0 (0x4000) — 64-bit comparator for hart 0
//!
//! When mtime >= mtimecmp, the machine timer interrupt fires (mcause 7).

use core::cell::Cell;

use arch::TickTimer;

const CLINT_BASE: usize = 0x0200_0000;
const MTIMECMP_OFFSET: usize = 0x4000;
const MTIME_OFFSET: usize = 0xBFF8;

/// QEMU virt CLINT runs at 10 MHz.
const CLINT_FREQ_HZ: u64 = 10_000_000;

// ---------------------------------------------------------------------------
// MMIO helpers
// ---------------------------------------------------------------------------

#[inline(always)]
unsafe fn read64(addr: usize) -> u64 {
    // RISC-V 32-bit: two 32-bit reads; read hi-lo-hi to avoid tearing.
    let lo_ptr = addr as *const u32;
    let hi_ptr = (addr + 4) as *const u32;
    loop {
        let hi1 = unsafe { core::ptr::read_volatile(hi_ptr) };
        let lo = unsafe { core::ptr::read_volatile(lo_ptr) };
        let hi2 = unsafe { core::ptr::read_volatile(hi_ptr) };
        if hi1 == hi2 {
            return ((hi1 as u64) << 32) | (lo as u64);
        }
    }
}

#[inline(always)]
unsafe fn write64(addr: usize, val: u64) {
    let lo_ptr = addr as *mut u32;
    let hi_ptr = (addr + 4) as *mut u32;
    // Write max first to avoid spurious interrupts during the two-word write.
    unsafe {
        core::ptr::write_volatile(hi_ptr, u32::MAX);
        core::ptr::write_volatile(lo_ptr, val as u32);
        core::ptr::write_volatile(hi_ptr, (val >> 32) as u32);
    }
}

// ---------------------------------------------------------------------------
// CLINT driver
// ---------------------------------------------------------------------------

pub struct Clint {
    /// Tick period in CLINT ticks (set by configure_tick).
    period: Cell<u64>,
}

impl Clint {
    pub const fn new() -> Self {
        Self { period: Cell::new(0) }
    }

    /// Read the raw mtime counter.
    pub fn mtime(&self) -> u64 {
        unsafe { read64(CLINT_BASE + MTIME_OFFSET) }
    }

    /// Schedule the next compare interrupt at mtime + period.
    pub fn schedule_next(&self) {
        let next = self.mtime() + self.period.get();
        unsafe { write64(CLINT_BASE + MTIMECMP_OFFSET, next) };
    }
}

impl TickTimer for Clint {
    fn configure_tick(&self, period_us: u32) {
        let ticks = (CLINT_FREQ_HZ * period_us as u64) / 1_000_000;
        self.period.set(ticks);

        // Arm the first compare.
        let first = self.mtime() + ticks;
        unsafe { write64(CLINT_BASE + MTIMECMP_OFFSET, first) };
    }

    fn clear_pending(&self) {
        // On CLINT, clearing the pending interrupt = schedule the next compare.
        self.schedule_next();
    }

    fn counter_us(&self) -> u64 {
        self.mtime() / (CLINT_FREQ_HZ / 1_000_000)
    }
}

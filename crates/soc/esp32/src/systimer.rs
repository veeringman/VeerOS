//! ESP32-C3 SYSTIMER driver for periodic tick and free-running counter.
//!
//! The SYSTIMER peripheral has 3 comparators and a 52-bit free-running counter
//! clocked at 16 MHz (62.5 ns resolution). We use comparator 0 in periodic
//! mode as the kernel tick source.
//!
//! Reference: ESP32-C3 Technical Reference Manual, Chapter 11 (System Timer).

#![allow(dead_code)]

use arch::TickTimer;

// ---------------------------------------------------------------------------
// Base address
// ---------------------------------------------------------------------------

#[cfg(feature = "c3")]
const SYSTIMER_BASE: usize = 0x6002_3000;

#[cfg(feature = "c6")]
const SYSTIMER_BASE: usize = 0x6000_A000;

#[cfg(feature = "h2")]
const SYSTIMER_BASE: usize = 0x6002_3000;

#[cfg(all(
    not(feature = "c3"),
    not(feature = "c6"),
    not(feature = "h2"),
))]
const SYSTIMER_BASE: usize = 0x6002_3000;

// ---------------------------------------------------------------------------
// Register offsets
// ---------------------------------------------------------------------------

/// Main configuration register.
const CONF_REG: usize = 0x00;

/// Trigger a snapshot of unit 0 counter into the value registers.
const UNIT0_OP: usize = 0x04;

/// Comparator 0 target value (high / low) for periodic alarm.
const TARGET0_HI: usize = 0x1C;
const TARGET0_LO: usize = 0x20;

/// Comparator 0 period for periodic mode (26-bit).
const TARGET0_CONF: usize = 0x34;

/// Write to apply comparator 0 config (load trigger).
const COMP0_LOAD: usize = 0x50;

/// Unit 0 value registers (52-bit counter split across two words).
const UNIT0_VALUE_HI: usize = 0x40;
const UNIT0_VALUE_LO: usize = 0x44;

/// Comparator 0 interrupt enable / clear.
const INT_ENA: usize = 0x64;
const INT_CLR: usize = 0x6C;

/// SYSTIMER clock: 16 MHz.
const TICKS_PER_US: u32 = 16;

// ---------------------------------------------------------------------------
// MMIO helpers
// ---------------------------------------------------------------------------

#[inline(always)]
unsafe fn mmio_read(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
unsafe fn mmio_write(addr: usize, val: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// Zero-sized handle to the SYSTIMER peripheral.
pub struct SysTimer;

impl SysTimer {
    pub const fn new() -> Self {
        Self
    }

    /// Read the 52-bit free-running counter.
    fn read_counter_raw(&self) -> u64 {
        // Trigger a snapshot of unit 0 into the value registers.
        unsafe { mmio_write(SYSTIMER_BASE + UNIT0_OP, 1 << 30) };
        // Wait for the value to be valid (bit 29).
        while unsafe { mmio_read(SYSTIMER_BASE + UNIT0_OP) } & (1 << 29) == 0 {
            core::hint::spin_loop();
        }
        let lo = unsafe { mmio_read(SYSTIMER_BASE + UNIT0_VALUE_LO) } as u64;
        let hi = unsafe { mmio_read(SYSTIMER_BASE + UNIT0_VALUE_HI) } as u64;
        (hi << 32) | lo
    }
}

impl TickTimer for SysTimer {
    fn configure_tick(&self, period_us: u32) {
        let ticks = period_us.saturating_mul(TICKS_PER_US);

        unsafe {
            // Enable SYSTIMER clock (bit 0) and unit 0 counter (bit 24).
            let conf = mmio_read(SYSTIMER_BASE + CONF_REG);
            mmio_write(SYSTIMER_BASE + CONF_REG, conf | (1 << 0) | (1 << 24));

            // Set comparator 0 period mode with the computed tick count.
            // Bit 30 = period mode enable.
            mmio_write(SYSTIMER_BASE + TARGET0_CONF, (1 << 30) | (ticks & 0x03FF_FFFF));

            // Apply comparator 0 config by writing to COMP0_LOAD.
            mmio_write(SYSTIMER_BASE + COMP0_LOAD, 1);

            // Enable comparator 0 interrupt (bit 0 of INT_ENA).
            mmio_write(SYSTIMER_BASE + INT_ENA, 1);
        }
    }

    fn clear_pending(&self) {
        unsafe {
            // Clear comparator 0 interrupt.
            mmio_write(SYSTIMER_BASE + INT_CLR, 1);
            // Re-arm the alarm: the ESP32-C6 hardware latches the alarm as
            // disabled after a match.  Toggle TARGET0_WORK_EN (CONF bit 24)
            // to re-start the periodic comparator — this matches ESP-IDF's
            // systimer_ll_enable_alarm().
            let conf = mmio_read(SYSTIMER_BASE + CONF_REG);
            mmio_write(SYSTIMER_BASE + CONF_REG, conf & !(1 << 24));
            mmio_write(SYSTIMER_BASE + CONF_REG, conf | (1 << 24));
        }
    }

    fn counter_us(&self) -> u64 {
        self.read_counter_raw() / (TICKS_PER_US as u64)
    }
}

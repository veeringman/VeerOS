//! ARM Generic Timer driver for Raspberry Pi 5.
//!
//! Uses AArch64 system registers — no MMIO addresses needed.
//!
//! Registers used:
//!   CNTFRQ_EL0   — counter frequency (set by firmware, ~54 MHz on Pi 5)
//!   CNTPCT_EL0   — physical counter value (64-bit, read-only)
//!   CNTP_TVAL_EL0— timer value (signed, counts down to fire)
//!   CNTP_CTL_EL0 — control: ENABLE (bit 0), IMASK (bit 1), ISTATUS (bit 2)

use arch::TickTimer;
use core::cell::Cell;

pub struct ArmGenericTimer {
    freq_hz: Cell<u64>,
    period_ticks: Cell<u64>,
}

impl ArmGenericTimer {
    pub const fn new() -> Self {
        Self {
            freq_hz: Cell::new(0),
            period_ticks: Cell::new(0),
        }
    }

    /// Read CNTFRQ_EL0 (counter frequency in Hz).
    #[cfg(target_arch = "aarch64")]
    #[inline]
    pub fn frequency(&self) -> u64 {
        let f: u64;
        unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) f, options(nomem, nostack)) };
        f
    }

    /// Read CNTPCT_EL0 (physical counter).
    #[cfg(target_arch = "aarch64")]
    #[inline]
    pub fn counter(&self) -> u64 {
        let v: u64;
        unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) v, options(nomem, nostack)) };
        v
    }

    /// Arm the timer to fire after the configured period.
    #[cfg(target_arch = "aarch64")]
    pub fn schedule_next(&self) {
        let tval = self.period_ticks.get();
        unsafe {
            core::arch::asm!("msr cntp_tval_el0, {}", in(reg) tval, options(nomem, nostack));
        }
    }

    /// Enable the physical timer — ENABLE=1, IMASK=0.
    #[cfg(target_arch = "aarch64")]
    pub fn enable(&self) {
        let ctl: u64 = 1; // ENABLE=1, IMASK=0
        unsafe {
            core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) ctl, options(nomem, nostack));
        }
    }

    /// Disable the physical timer.
    #[cfg(target_arch = "aarch64")]
    pub fn disable(&self) {
        let ctl: u64 = 0;
        unsafe {
            core::arch::asm!("msr cntp_ctl_el0, {}", in(reg) ctl, options(nomem, nostack));
        }
    }

    // Stubs for non-aarch64 (host compilation).
    #[cfg(not(target_arch = "aarch64"))]
    pub fn frequency(&self) -> u64 { 54_000_000 }
    #[cfg(not(target_arch = "aarch64"))]
    pub fn counter(&self) -> u64 { 0 }
    #[cfg(not(target_arch = "aarch64"))]
    pub fn schedule_next(&self) {}
    #[cfg(not(target_arch = "aarch64"))]
    pub fn enable(&self) {}
    #[cfg(not(target_arch = "aarch64"))]
    pub fn disable(&self) {}
}

impl TickTimer for ArmGenericTimer {
    fn configure_tick(&self, period_us: u32) {
        let freq = self.frequency();
        self.freq_hz.set(freq);
        let ticks = (freq * period_us as u64) / 1_000_000;
        self.period_ticks.set(ticks);
        self.schedule_next();
        self.enable();
    }

    fn clear_pending(&self) {
        // Re-arm the timer for the next period.
        self.schedule_next();
    }

    fn counter_us(&self) -> u64 {
        let freq = self.freq_hz.get();
        if freq == 0 {
            return 0;
        }
        self.counter() * 1_000_000 / freq
    }
}

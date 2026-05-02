//! ARM generic timer driver for the AArch64 virtual-machine target.

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

    #[cfg(target_arch = "aarch64")]
    #[inline]
    pub fn frequency(&self) -> u64 {
        let freq: u64;
        unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack)) };
        freq
    }

    #[cfg(target_arch = "aarch64")]
    #[inline]
    pub fn counter(&self) -> u64 {
        let count: u64;
        unsafe { core::arch::asm!("mrs {}, cntvct_el0", out(reg) count, options(nomem, nostack)) };
        count
    }

    #[cfg(target_arch = "aarch64")]
    pub fn schedule_next(&self) {
        let tval = self.period_ticks.get();
        unsafe { core::arch::asm!("msr cntv_tval_el0, {}", in(reg) tval, options(nomem, nostack)) };
    }

    #[cfg(target_arch = "aarch64")]
    pub fn enable(&self) {
        unsafe { core::arch::asm!("msr cntv_ctl_el0, {}", in(reg) 1u64, options(nomem, nostack)) };
    }

    #[cfg(not(target_arch = "aarch64"))]
    pub fn frequency(&self) -> u64 {
        24_000_000
    }
    #[cfg(not(target_arch = "aarch64"))]
    pub fn counter(&self) -> u64 {
        0
    }
    #[cfg(not(target_arch = "aarch64"))]
    pub fn schedule_next(&self) {}
    #[cfg(not(target_arch = "aarch64"))]
    pub fn enable(&self) {}
}

impl TickTimer for ArmGenericTimer {
    fn configure_tick(&self, period_us: u32) {
        let freq = self.frequency();
        self.freq_hz.set(freq);
        self.period_ticks.set((freq * period_us as u64) / 1_000_000);
        self.schedule_next();
        self.enable();
    }

    fn clear_pending(&self) {
        self.schedule_next();
    }

    fn counter_us(&self) -> u64 {
        let freq = self.freq_hz.get();
        if freq == 0 {
            0
        } else {
            self.counter() * 1_000_000 / freq
        }
    }
}

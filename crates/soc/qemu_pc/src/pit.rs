//! Intel 8254 PIT (Programmable Interval Timer) driver.
//!
//! Channel 0 at I/O port 0x40, command register at 0x43.
//! The PIT oscillates at 1.193182 MHz. We program channel 0 in
//! rate-generator mode (mode 2) for periodic timer interrupts (IRQ 0).

use crate::outb;
use arch::TickTimer;
use core::cell::Cell;
use core::sync::atomic::{AtomicU64, Ordering};

/// PIT base frequency in Hz.
const PIT_FREQ_HZ: u32 = 1_193_182;

// I/O ports.
const PIT_CH0_DATA: u16 = 0x40;
const PIT_CMD: u16 = 0x43;

// Command byte: channel 0, lo/hi byte, mode 2 (rate generator).
const CMD_CH0_MODE2: u8 = 0x34; // 00_11_010_0

/// Software tick counter (incremented by the timer ISR).
pub static TICK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 8254 PIT timer controller.
pub struct Pit8254 {
    /// Configured tick period in microseconds.
    period_us: Cell<u32>,
}

impl Pit8254 {
    pub const fn new() -> Self {
        Self {
            period_us: Cell::new(0),
        }
    }

    /// Increment the software tick counter. Called from the timer ISR.
    pub fn tick(&self) {
        TICK_COUNTER.fetch_add(1, Ordering::Relaxed);
    }
}

impl TickTimer for Pit8254 {
    fn configure_tick(&self, period_us: u32) {
        // Compute the PIT divisor: divisor = PIT_FREQ_HZ * period_us / 1_000_000.
        // For 1 ms periods: 1_193_182 * 1000 / 1_000_000 ≈ 1193.
        let divisor = ((PIT_FREQ_HZ as u64) * (period_us as u64) / 1_000_000) as u16;
        let divisor = if divisor == 0 { 1 } else { divisor };

        self.period_us.set(period_us);

        unsafe {
            outb(PIT_CMD, CMD_CH0_MODE2);
            outb(PIT_CH0_DATA, (divisor & 0xFF) as u8); // low byte
            outb(PIT_CH0_DATA, ((divisor >> 8) & 0xFF) as u8); // high byte
        }
    }

    fn clear_pending(&self) {
        // PIT automatically reloads in mode 2 — nothing to ack here.
        // The PIC EOI is sent separately by the ISR handler.
        self.tick();
    }

    fn counter_us(&self) -> u64 {
        // Approximate: ticks × period_us.
        let ticks = TICK_COUNTER.load(Ordering::Relaxed);
        let period = self.period_us.get() as u64;
        if period > 0 {
            ticks * period
        } else {
            ticks * 1000
        }
    }
}

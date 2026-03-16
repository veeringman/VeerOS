//! SoC-level drivers for Raspberry Pi 5 (BCM2712, Cortex-A76).
//!
//! Peripherals:
//!   UART : PL011 via RP1 southbridge (PCIe-mapped MMIO)
//!   Timer: ARM Generic Timer (system registers, no MMIO)
//!   GIC  : GIC-400 (GICv2) — basic distributor + CPU interface
//!
//! The RPi firmware initialises the PL011 UART at 115200-8-N-1 before
//! handing off to `kernel8.img`.  We only need to read/write PL011
//! registers for early console output (no baud-rate setup required).

#![no_std]

use arch::{Platform, TickTimer};

pub mod gic;
pub mod gpio;
pub mod i2c;
pub mod mem;
pub mod spi;
pub mod timer;
pub mod uart;
pub mod mailbox;
pub mod board;
pub mod fb;
pub mod font;
pub mod fbcon;
pub mod sd;
pub mod xhci;

/// Raspberry Pi 5 platform descriptor.
pub struct Raspi5;

impl Raspi5 {
    pub const fn new() -> Self {
        Self
    }
}

impl Platform for Raspi5 {
    fn name(&self) -> &'static str {
        "Raspberry Pi 5 (BCM2712)"
    }
    fn init_cpu(&self) {
        // CPU already initialised by RPi firmware (EL2→EL1 done in _start).
        // Enable FP/NEON: CPACR_EL1 FPEN = 0b11.
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("mrs {tmp}, cpacr_el1",
                             "orr {tmp}, {tmp}, #(0x3 << 20)",
                             "msr cpacr_el1, {tmp}",
                             "isb",
                             tmp = out(reg) _);
        }
    }
    fn init_interrupts(&self) {
        gic::Gic400::new().init();
    }
    fn init_timer(&self) {
        let t = timer::ArmGenericTimer::new();
        t.configure_tick(10_000); // 10 ms tick
    }
}

/// Return a PL011 UART handle (firmware-initialised).
pub fn default_serial() -> uart::Pl011 {
    uart::Pl011::new()
}

/// Return an ARM generic‐timer handle.
pub fn system_timer() -> timer::ArmGenericTimer {
    timer::ArmGenericTimer::new()
}

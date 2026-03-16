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

use arch::Platform;

pub mod gic;
pub mod mem;
pub mod timer;
pub mod uart;
pub mod mailbox;
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
    fn init_cpu(&self) {}
    fn init_interrupts(&self) {}
    fn init_timer(&self) {}
}

/// Return a PL011 UART handle (firmware-initialised).
pub fn default_serial() -> uart::Pl011 {
    uart::Pl011::new()
}

/// Return an ARM generic‐timer handle.
pub fn system_timer() -> timer::ArmGenericTimer {
    timer::ArmGenericTimer::new()
}

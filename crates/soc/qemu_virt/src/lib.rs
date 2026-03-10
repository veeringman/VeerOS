//! BSP for the QEMU `virt` RISC-V 32-bit machine.
//!
//! Peripherals:
//!   UART : NS16550a at 0x1000_0000
//!   CLINT: at 0x0200_0000  (mtime / mtimecmp)
//!   PLIC : at 0x0C00_0000  (not used yet)

#![no_std]

use arch::Platform;

pub mod clint;
pub mod uart;
pub mod virtio_net;

pub struct QemuVirt;

impl QemuVirt {
    pub const fn new() -> Self {
        Self
    }
}

impl Platform for QemuVirt {
    fn name(&self) -> &'static str {
        "QEMU virt (RISC-V 32)"
    }
    fn init_cpu(&self) {}
    fn init_interrupts(&self) {}
    fn init_timer(&self) {}
}

// ── convenient accessors ─────────────────────────────────────────────────

pub fn default_serial() -> uart::Ns16550 {
    uart::Ns16550::new()
}

pub fn system_timer() -> clint::Clint {
    clint::Clint::new()
}

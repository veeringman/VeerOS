//! Generic AArch64 virtual-machine platform for VeerOS.
//!
//! This target is for desktop/server-class virtual execution under hosts such as
//! Apple Silicon Hypervisor.framework. It intentionally avoids Raspberry Pi
//! firmware, RP1, GPIO, framebuffer, and board-specific assumptions.

#![no_std]

use arch::{Platform, TickTimer};

pub mod mem;
pub mod timer;
pub mod uart;
pub mod virtio_blk;
pub mod virtio_net;

pub struct Aarch64Virt;

impl Aarch64Virt {
    pub const fn new() -> Self {
        Self
    }
}

impl Platform for Aarch64Virt {
    fn name(&self) -> &'static str {
        "AArch64 virtual machine"
    }

    fn init_cpu(&self) {
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!(
                "mrs {tmp}, cpacr_el1",
                "orr {tmp}, {tmp}, #(0x3 << 20)",
                "msr cpacr_el1, {tmp}",
                "isb",
                tmp = out(reg) _,
            );
        }
    }

    fn init_interrupts(&self) {}

    fn init_timer(&self) {
        let timer = timer::ArmGenericTimer::new();
        timer.configure_tick(10_000);
    }
}

pub fn default_serial() -> uart::Pl011 {
    uart::Pl011::new()
}

pub fn system_timer() -> timer::ArmGenericTimer {
    timer::ArmGenericTimer::new()
}

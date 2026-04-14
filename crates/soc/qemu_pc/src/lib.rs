//! SoC-level drivers for QEMU `q35` / `pc` x86-64 machines.
//!
//! Provides COM1 serial, legacy 8259 PIC, and 8254 PIT timer —
//! enough for a bootable kernel with preemptive scheduling and shell.

#![no_std]

use arch::Platform;

pub mod serial;
pub mod pic;
pub mod pit;
pub mod ps2kbd;
pub mod vga;
pub mod lapic;
pub mod ioapic;
pub mod hpet;
pub mod pci;
pub mod mm;

// ─── I/O port helpers ────────────────────────────────────────────────────

/// Write a byte to an x86 I/O port.
///
/// # Safety
/// Caller must ensure the port address is valid.
#[inline(always)]
pub unsafe fn outb(port: u16, val: u8) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack, preserves_flags));
    }
    #[cfg(not(target_arch = "x86_64"))]
    { let _ = (port, val); }
}

/// Read a byte from an x86 I/O port.
///
/// # Safety
/// Caller must ensure the port address is valid.
#[inline(always)]
pub unsafe fn inb(port: u16) -> u8 {
    #[cfg(target_arch = "x86_64")]
    {
        let val: u8;
        unsafe {
            core::arch::asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack, preserves_flags));
        }
        val
    }
    #[cfg(not(target_arch = "x86_64"))]
    { let _ = port; 0 }
}

/// Short I/O delay (read from port 0x80, which is unused POST diagnostic).
#[inline(always)]
pub unsafe fn io_wait() {
    unsafe { outb(0x80, 0); }
}

// ─── Platform trait ──────────────────────────────────────────────────────

pub struct QemuPc;

impl QemuPc {
    pub const fn new() -> Self {
        Self
    }
}

impl Platform for QemuPc {
    fn name(&self) -> &'static str {
        "QEMU PC (x86-64)"
    }

    fn init_cpu(&self) {
        // Nothing special needed at boot — the Multiboot stub already
        // put us into 64-bit long mode with paging on.
    }

    fn init_interrupts(&self) {
        // Remap the legacy PIC so IRQ 0-15 don't collide with CPU exceptions.
        pic::init();
    }

    fn init_timer(&self) {
        // PIT channel 0 already configured via pit::Pit8254::configure_tick.
    }
}

/// Return the default serial port (COM1).
pub fn default_serial() -> serial::Com1 {
    serial::Com1::new()
}

/// Return the system timer (PIT channel 0).
pub fn system_timer() -> pit::Pit8254 {
    pit::Pit8254::new()
}

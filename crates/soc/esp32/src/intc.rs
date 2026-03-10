//! ESP32-C3 interrupt controller driver.
//!
//! The ESP32-C3 uses a simplified CLIC-like interrupt matrix. Each peripheral
//! interrupt source is mapped to a CPU interrupt line via the INTERRUPT_CORE0
//! peripheral. We expose this through the `arch::InterruptController` trait.
//!
//! Reference: ESP32-C3 Technical Reference Manual, Chapter 8 (Interrupt Matrix).

#![allow(dead_code)]

use arch::InterruptController;

// ---------------------------------------------------------------------------
// Base addresses
// ---------------------------------------------------------------------------

/// INTERRUPT_CORE0 base (interrupt matrix).
#[cfg(feature = "c3")]
const INTC_BASE: usize = 0x600C_2000;

#[cfg(feature = "c6")]
const INTC_BASE: usize = 0x600C_2000;

#[cfg(feature = "h2")]
const INTC_BASE: usize = 0x600C_2000;

#[cfg(all(
    not(feature = "c3"),
    not(feature = "c6"),
    not(feature = "h2"),
))]
const INTC_BASE: usize = 0x600C_2000;

// ---------------------------------------------------------------------------
// Register offsets
// ---------------------------------------------------------------------------

/// Per-source mapping register stride (one 32-bit reg per source).
/// Writing a CPU interrupt number (1–31) to MAP_REG[source] routes that
/// peripheral source to the chosen CPU interrupt line.
const MAP_REG_OFFSET: usize = 0x000;

/// Interrupt enable register — one bit per CPU interrupt line (1–31).
const INT_ENABLE_REG: usize = 0x104;

/// Interrupt priority registers — one per CPU interrupt line.
/// Priority 0 = disabled; 1–15 valid priorities.
const INT_PRI_BASE: usize = 0x114;

/// CPU interrupt threshold register — interrupts with priority ≤ threshold
/// are masked.
const INT_THRESH_REG: usize = 0x190;

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

/// Zero-sized handle to the ESP32-C3 interrupt matrix / controller.
pub struct Esp32Intc;

impl Esp32Intc {
    pub const fn new() -> Self {
        Self
    }

    /// Map peripheral source `src` to CPU interrupt line `cpu_int` (1–31).
    pub fn map_source(&self, src: u16, cpu_int: u8) {
        let reg = INTC_BASE + MAP_REG_OFFSET + (src as usize) * 4;
        unsafe { mmio_write(reg, cpu_int as u32) };
    }

    /// Set the CPU interrupt priority threshold.
    /// Interrupts with priority ≤ threshold are masked.
    pub fn set_threshold(&self, threshold: u8) {
        unsafe { mmio_write(INTC_BASE + INT_THRESH_REG, threshold as u32) };
    }
}

impl InterruptController for Esp32Intc {
    fn enable_interrupt(&self, irq: u16) {
        let val = unsafe { mmio_read(INTC_BASE + INT_ENABLE_REG) };
        unsafe { mmio_write(INTC_BASE + INT_ENABLE_REG, val | (1 << irq)) };
    }

    fn disable_interrupt(&self, irq: u16) {
        let val = unsafe { mmio_read(INTC_BASE + INT_ENABLE_REG) };
        unsafe { mmio_write(INTC_BASE + INT_ENABLE_REG, val & !(1 << irq)) };
    }

    fn set_priority(&self, irq: u16, priority: u8) {
        let reg = INTC_BASE + INT_PRI_BASE + (irq as usize) * 4;
        unsafe { mmio_write(reg, priority as u32) };
    }

    fn enable_global(&self) {
        // Set mstatus.MIE (bit 3) via CSR.
        unsafe {
            core::arch::asm!("csrsi mstatus, 0x8", options(nomem, nostack));
        }
    }

    fn disable_global(&self) {
        // Clear mstatus.MIE (bit 3) via CSR.
        unsafe {
            core::arch::asm!("csrci mstatus, 0x8", options(nomem, nostack));
        }
    }
}

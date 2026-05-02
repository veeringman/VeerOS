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
const INTC_BASE: usize = 0x6001_0000;

#[cfg(feature = "h2")]
const INTC_BASE: usize = 0x600C_2000;

#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2"),))]
const INTC_BASE: usize = 0x600C_2000;

// ---------------------------------------------------------------------------
// Register offsets
// ---------------------------------------------------------------------------

/// Per-source mapping register stride (one 32-bit reg per source).
/// Writing a CPU interrupt number (1–31) to MAP_REG[source] routes that
/// peripheral source to the chosen CPU interrupt line.
const MAP_REG_OFFSET: usize = 0x000;

// ---------------------------------------------------------------------------
// PLIC base (ESP32-C6 uses PLIC at 0x2000_1000 for interrupt
// enable / priority / threshold, separate from the INTMATRIX).
// ---------------------------------------------------------------------------

#[cfg(feature = "c6")]
const PLIC_BASE: usize = 0x2000_1000;
#[cfg(not(feature = "c6"))]
const PLIC_BASE: usize = 0x0; // unused on non-C6

/// PLIC machine-external-interrupt enable (1 bit per CPU int line).
const PLIC_MXINT_ENABLE: usize = 0x00;
/// PLIC interrupt type: 0=level, 1=edge (1 bit per CPU int line).
const PLIC_MXINT_TYPE: usize = 0x04;
/// PLIC edge-interrupt clear (write 1 to clear, 1 bit per CPU int line).
const PLIC_MXINT_CLEAR: usize = 0x08;
/// PLIC per-interrupt priority: base + 0x10 + 4*int_num  (bits [3:0]).
const PLIC_MXINT_PRI_BASE: usize = 0x10;
/// PLIC interrupt threshold (bits [7:0]).
const PLIC_MXINT_THRESH: usize = 0x90;

// Legacy offsets used by ESP32-C3 (INTMATRIX contains enable/pri/thresh).
#[cfg(not(feature = "c6"))]
const INT_ENABLE_REG: usize = 0x104;
#[cfg(not(feature = "c6"))]
const INT_PRI_BASE: usize = 0x114;
#[cfg(not(feature = "c6"))]
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
        #[cfg(feature = "c6")]
        unsafe {
            mmio_write(PLIC_BASE + PLIC_MXINT_THRESH, threshold as u32)
        };
        #[cfg(not(feature = "c6"))]
        unsafe {
            mmio_write(INTC_BASE + INT_THRESH_REG, threshold as u32)
        };
    }
}

impl InterruptController for Esp32Intc {
    fn enable_interrupt(&self, irq: u16) {
        #[cfg(feature = "c6")]
        {
            // Set interrupt type to level-triggered (clear the type bit).
            let typ = unsafe { mmio_read(PLIC_BASE + PLIC_MXINT_TYPE) };
            unsafe { mmio_write(PLIC_BASE + PLIC_MXINT_TYPE, typ & !(1 << irq)) };
            // Clear any stale edge-pending state.
            unsafe { mmio_write(PLIC_BASE + PLIC_MXINT_CLEAR, 1 << irq) };

            let val = unsafe { mmio_read(PLIC_BASE + PLIC_MXINT_ENABLE) };
            unsafe { mmio_write(PLIC_BASE + PLIC_MXINT_ENABLE, val | (1 << irq)) };
            // ESP32-C6 also requires the per-interrupt mie CSR bit to be set.
            let mask = 1u32 << irq;
            unsafe {
                core::arch::asm!("csrs mie, {0}", in(reg) mask, options(nomem, nostack));
            }
        }
        #[cfg(not(feature = "c6"))]
        {
            let val = unsafe { mmio_read(INTC_BASE + INT_ENABLE_REG) };
            unsafe { mmio_write(INTC_BASE + INT_ENABLE_REG, val | (1 << irq)) };
        }
    }

    fn disable_interrupt(&self, irq: u16) {
        #[cfg(feature = "c6")]
        {
            let val = unsafe { mmio_read(PLIC_BASE + PLIC_MXINT_ENABLE) };
            unsafe { mmio_write(PLIC_BASE + PLIC_MXINT_ENABLE, val & !(1 << irq)) };
            let mask = 1u32 << irq;
            unsafe {
                core::arch::asm!("csrc mie, {0}", in(reg) mask, options(nomem, nostack));
            }
        }
        #[cfg(not(feature = "c6"))]
        {
            let val = unsafe { mmio_read(INTC_BASE + INT_ENABLE_REG) };
            unsafe { mmio_write(INTC_BASE + INT_ENABLE_REG, val & !(1 << irq)) };
        }
    }

    fn set_priority(&self, irq: u16, priority: u8) {
        #[cfg(feature = "c6")]
        {
            let reg = PLIC_BASE + PLIC_MXINT_PRI_BASE + (irq as usize) * 4;
            unsafe { mmio_write(reg, priority as u32) };
        }
        #[cfg(not(feature = "c6"))]
        {
            let reg = INTC_BASE + INT_PRI_BASE + (irq as usize) * 4;
            unsafe { mmio_write(reg, priority as u32) };
        }
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

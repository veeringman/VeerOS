//! GIC-400 (GICv2) driver for Raspberry Pi 5 (BCM2712).
//!
//! Provides minimal distributor + CPU-interface initialisation needed
//! for the ARM generic-timer PPI to generate IRQs.
//!
//! **Addresses**: The constants below are based on available BCM2712
//! documentation.  Verify against the device-tree blob for your
//! firmware version if the kernel hangs during GIC init.

use arch::InterruptController;

use crate::mem;

const GICD_BASE: usize = mem::GIC_DIST_BASE;
const GICC_BASE: usize = mem::GIC_CPU_BASE;

// Distributor registers.
const GICD_CTLR: usize = 0x000;
const GICD_ISENABLER0: usize = 0x100; // PPI/SGI enable set (bank 0)
const GICD_IPRIORITYR: usize = 0x400; // Interrupt priority (byte array)

// CPU interface registers.
const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004; // Priority mask
const GICC_IAR: usize = 0x00C; // Interrupt acknowledge
const GICC_EOIR: usize = 0x010; // End of interrupt

#[inline]
unsafe fn mmio_read(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn mmio_write(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

pub struct Gic400;

impl Gic400 {
    pub const fn new() -> Self {
        Self
    }

    /// Initialise the GIC distributor and CPU interface.
    ///
    /// After this call, PPI #30 (ARM physical timer) is enabled at
    /// highest priority.
    pub fn init(&self) {
        unsafe {
            // Enable distributor.
            mmio_write(GICD_BASE + GICD_CTLR, 1);

            // Enable CPU interface, allow all priorities.
            mmio_write(GICC_BASE + GICC_CTLR, 1);
            mmio_write(GICC_BASE + GICC_PMR, 0xFF);

            // Enable PPI #30 (physical timer) — bit 30 in bank 0.
            let ena = mmio_read(GICD_BASE + GICD_ISENABLER0);
            mmio_write(GICD_BASE + GICD_ISENABLER0, ena | (1 << 30));

            // Set priority of IRQ 30 to 0 (highest).
            // IRQ 30 is byte 30 in the priority register array.
            let reg_off = (30 / 4) * 4; // aligned to 4-byte word
            let byte_off = 30 % 4;
            let mut val = mmio_read(GICD_BASE + GICD_IPRIORITYR + reg_off);
            val &= !(0xFF << (byte_off * 8));
            mmio_write(GICD_BASE + GICD_IPRIORITYR + reg_off, val);
        }
    }

    /// Acknowledge the current pending IRQ, returning its IRQ number.
    pub fn acknowledge(&self) -> u32 {
        unsafe { mmio_read(GICC_BASE + GICC_IAR) & 0x3FF }
    }

    /// Signal end-of-interrupt for the given IRQ.
    pub fn end_of_interrupt(&self, irq: u32) {
        unsafe { mmio_write(GICC_BASE + GICC_EOIR, irq) };
    }
}

impl InterruptController for Gic400 {
    fn enable_interrupt(&self, irq: u16) {
        let bank = (irq / 32) as usize;
        let bit = irq % 32;
        unsafe {
            let addr = GICD_BASE + GICD_ISENABLER0 + bank * 4;
            let val = mmio_read(addr);
            mmio_write(addr, val | (1 << bit));
        }
    }

    fn disable_interrupt(&self, _irq: u16) {
        // ISENABLER is set-enable — use ICENABLER (offset 0x180) for disable.
        // Stubbed for now; disable via GICD_ICENABLER when needed.
    }

    fn set_priority(&self, irq: u16, priority: u8) {
        let reg_off = ((irq as usize) / 4) * 4;
        let byte_off = (irq as usize) % 4;
        unsafe {
            let mut val = mmio_read(GICD_BASE + GICD_IPRIORITYR + reg_off);
            val &= !(0xFF << (byte_off * 8));
            val |= (priority as u32) << (byte_off * 8);
            mmio_write(GICD_BASE + GICD_IPRIORITYR + reg_off, val);
        }
    }

    fn enable_global(&self) {
        unsafe {
            mmio_write(GICD_BASE + GICD_CTLR, 1);
            mmio_write(GICC_BASE + GICC_CTLR, 1);
        }
    }

    fn disable_global(&self) {
        unsafe {
            mmio_write(GICC_BASE + GICC_CTLR, 0);
        }
    }
}

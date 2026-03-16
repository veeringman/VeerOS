//! PL011 UART driver for Raspberry Pi 5.
//!
//! On the RPi 5 the GPIO-header UART (GPIO 14/15) is routed through
//! the RP1 southbridge.  The firmware maps the RP1 BAR at
//! `RP1_BAR_BASE` and UART0 sits at offset `RP1_UART0_OFF`.
//!
//! The firmware already configures 115200-8-N-1 before loading
//! `kernel8.img`, so we only need read / write access to the PL011
//! Data Register (DR) and Flag Register (FR).

use arch::Serial;

use crate::mem;

/// PL011 UART0 base address (RP1 BAR + offset).
const UART_BASE: usize = mem::RP1_BAR_BASE + mem::RP1_UART0_OFF;

// PL011 register offsets.
const DR: usize = 0x00; // Data Register
const FR: usize = 0x18; // Flag Register

// Flag Register bits.
const FR_TXFF: u32 = 1 << 5; // TX FIFO full
const FR_RXFE: u32 = 1 << 4; // RX FIFO empty

#[inline]
unsafe fn mmio_read(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn mmio_write(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

/// PL011 UART handle (zero-size — pure MMIO).
pub struct Pl011;

impl Pl011 {
    pub const fn new() -> Self {
        Self
    }
}

impl Serial for Pl011 {
    fn write_byte(&self, byte: u8) {
        // Wait until TX FIFO has room.
        while (unsafe { mmio_read(UART_BASE + FR) } & FR_TXFF) != 0 {
            core::hint::spin_loop();
        }
        unsafe { mmio_write(UART_BASE + DR, byte as u32) };
    }

    fn read_byte(&self) -> u8 {
        while !self.has_data() {
            core::hint::spin_loop();
        }
        (unsafe { mmio_read(UART_BASE + DR) }) as u8
    }

    fn has_data(&self) -> bool {
        (unsafe { mmio_read(UART_BASE + FR) } & FR_RXFE) == 0
    }
}

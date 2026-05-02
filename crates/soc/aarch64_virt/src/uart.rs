//! PL011 UART driver for the generic AArch64 virtual-machine target.

use arch::Serial;

use crate::mem;

const DR: usize = 0x00;
const FR: usize = 0x18;
const FR_TXFF: u32 = 1 << 5;
const FR_RXFE: u32 = 1 << 4;

#[inline]
unsafe fn mmio_read(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn mmio_write(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

#[derive(Clone, Copy)]
pub struct Pl011;

impl Pl011 {
    pub const fn new() -> Self {
        Self
    }
}

impl Serial for Pl011 {
    fn write_byte(&self, byte: u8) {
        while (unsafe { mmio_read(mem::UART0_BASE + FR) } & FR_TXFF) != 0 {
            core::hint::spin_loop();
        }
        unsafe { mmio_write(mem::UART0_BASE + DR, byte as u32) };
    }

    fn read_byte(&self) -> u8 {
        while !self.has_data() {
            core::hint::spin_loop();
        }
        unsafe { mmio_read(mem::UART0_BASE + DR) as u8 }
    }

    fn has_data(&self) -> bool {
        (unsafe { mmio_read(mem::UART0_BASE + FR) } & FR_RXFE) == 0
    }
}

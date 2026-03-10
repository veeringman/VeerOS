//! NS16550a UART driver for the QEMU `virt` RISC-V machine.
//!
//! The UART sits at 0x1000_0000 and is memory-mapped with byte-wide
//! registers at natural offsets.  QEMU initialises it in FIFO mode so
//! we can use it immediately without any setup sequence.

use arch::Serial;

const UART_BASE: usize = 0x1000_0000;

// Register offsets
const RBR: usize = 0x00; // Receive Buffer Register (read)
const THR: usize = 0x00; // Transmit Holding Register (write)
const LSR: usize = 0x05; // Line Status Register

// LSR bits
const LSR_DATA_READY: u8 = 1 << 0;
const LSR_THR_EMPTY: u8 = 1 << 5;

// ---------------------------------------------------------------------------
// Tiny MMIO helpers (byte-wide for 16550)
// ---------------------------------------------------------------------------

#[inline(always)]
unsafe fn mmio_read8(addr: usize) -> u8 {
    unsafe { core::ptr::read_volatile(addr as *const u8) }
}

#[inline(always)]
unsafe fn mmio_write8(addr: usize, val: u8) {
    unsafe { core::ptr::write_volatile(addr as *mut u8, val) }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

pub struct Ns16550;

impl Ns16550 {
    pub const fn new() -> Self {
        Self
    }
}

impl Serial for Ns16550 {
    fn write_byte(&self, byte: u8) {
        // Spin until THR is empty.
        while (unsafe { mmio_read8(UART_BASE + LSR) } & LSR_THR_EMPTY) == 0 {
            core::hint::spin_loop();
        }
        unsafe { mmio_write8(UART_BASE + THR, byte) };
    }

    fn read_byte(&self) -> u8 {
        // Spin until data is ready.
        while !self.has_data() {
            core::hint::spin_loop();
        }
        unsafe { mmio_read8(UART_BASE + RBR) }
    }

    fn has_data(&self) -> bool {
        (unsafe { mmio_read8(UART_BASE + LSR) } & LSR_DATA_READY) != 0
    }
}

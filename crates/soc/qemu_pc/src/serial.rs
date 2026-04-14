//! COM1 NS8250/16550 UART driver via x86 I/O ports.
//!
//! Base I/O port 0x3F8 — standard PC COM1.

use arch::Serial;
use crate::{inb, outb};

/// COM1 base I/O port.
const COM1_BASE: u16 = 0x3F8;

// Register offsets from base.
const THR: u16 = 0; // Transmit Holding Register (write)
const RBR: u16 = 0; // Receive Buffer Register (read)
const DLL: u16 = 0; // Divisor Latch Low (DLAB=1)
const DLH: u16 = 1; // Divisor Latch High (DLAB=1)
const IER: u16 = 1; // Interrupt Enable Register
const FCR: u16 = 2; // FIFO Control Register (write)
const LCR: u16 = 3; // Line Control Register
const MCR: u16 = 4; // Modem Control Register
const LSR: u16 = 5; // Line Status Register

// LSR bits.
const LSR_DATA_READY: u8 = 1 << 0;
const LSR_THR_EMPTY: u8 = 1 << 5;

/// COM1 serial port controller.
pub struct Com1;

impl Com1 {
    pub const fn new() -> Self {
        Self
    }

    /// Initialise COM1: 115200 baud, 8N1, FIFO enabled.
    pub fn init(&self) {
        unsafe {
            // Disable all interrupts.
            outb(COM1_BASE + IER, 0x00);

            // Enable DLAB (set baud rate divisor).
            outb(COM1_BASE + LCR, 0x80);

            // Set divisor to 1 (115200 baud).
            outb(COM1_BASE + DLL, 0x01);
            outb(COM1_BASE + DLH, 0x00);

            // 8 bits, no parity, one stop bit (8N1), clear DLAB.
            outb(COM1_BASE + LCR, 0x03);

            // Enable FIFO, clear them, 14-byte threshold.
            outb(COM1_BASE + FCR, 0xC7);

            // IRQs enabled, RTS/DSR set.
            outb(COM1_BASE + MCR, 0x0B);
        }
    }
}

impl Serial for Com1 {
    fn write_byte(&self, byte: u8) {
        unsafe {
            // Wait for transmit holding register to be empty.
            while (inb(COM1_BASE + LSR) & LSR_THR_EMPTY) == 0 {
                core::hint::spin_loop();
            }
            outb(COM1_BASE + THR, byte);
        }
    }

    fn read_byte(&self) -> u8 {
        unsafe {
            // Wait for data to be available.
            while (inb(COM1_BASE + LSR) & LSR_DATA_READY) == 0 {
                core::hint::spin_loop();
            }
            inb(COM1_BASE + RBR)
        }
    }

    fn has_data(&self) -> bool {
        unsafe { (inb(COM1_BASE + LSR) & LSR_DATA_READY) != 0 }
    }
}

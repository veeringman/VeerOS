//! Minimal UART0 driver for the ESP32-C3 / C6 / H2 RISC-V family.
//!
//! This driver talks directly to the UART MMIO registers so it can work
//! before any HAL or runtime crate is initialised — exactly what we need
//! for an early boot banner.
//!
//! Reference: ESP32-C3 Technical Reference Manual, Chapter 26 (UART).

#![allow(dead_code)]

use arch::Serial;

// ---------------------------------------------------------------------------
// Base addresses per chip variant
// ---------------------------------------------------------------------------

#[cfg(feature = "c3")]
const UART0_BASE: usize = 0x6000_0000;

#[cfg(feature = "c6")]
const UART0_BASE: usize = 0x6000_0000;

#[cfg(feature = "h2")]
const UART0_BASE: usize = 0x6000_0000;

#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2"),))]
const UART0_BASE: usize = 0x6000_0000; // sensible default

// ---------------------------------------------------------------------------
// Register offsets (common across C3/C6/H2)
// ---------------------------------------------------------------------------

/// TX FIFO write port — write a byte here to enqueue it.
const UART_FIFO_REG: usize = 0x00;

/// Status register -- TX FIFO count in bits [23:16], RX FIFO count in bits [7:0].
const UART_STATUS_REG: usize = 0x1C;

/// Maximum number of bytes in the hardware TX FIFO.
const TX_FIFO_DEPTH: u32 = 128;

// ---------------------------------------------------------------------------
// Tiny MMIO helpers (volatile, no caching assumptions)
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
// UART0 driver type
// ---------------------------------------------------------------------------

/// Zero-sized handle to UART0. No runtime state needed — all state lives in
/// the hardware registers.
pub struct Uart0;

impl Uart0 {
    pub const fn new() -> Self {
        Self
    }

    /// Number of bytes currently sitting in the TX FIFO.
    #[inline]
    fn tx_fifo_count(&self) -> u32 {
        let status = unsafe { mmio_read(UART0_BASE + UART_STATUS_REG) };
        (status >> 16) & 0xFF
    }

    /// Spin until there is room for at least one byte in the TX FIFO.
    #[inline]
    fn wait_tx_fifo_not_full(&self) {
        while self.tx_fifo_count() >= TX_FIFO_DEPTH {
            core::hint::spin_loop();
        }
    }

    /// Number of bytes currently sitting in the RX FIFO.
    #[inline]
    fn rx_fifo_count(&self) -> u32 {
        let status = unsafe { mmio_read(UART0_BASE + UART_STATUS_REG) };
        status & 0xFF
    }
}

impl Serial for Uart0 {
    fn write_byte(&self, byte: u8) {
        self.wait_tx_fifo_not_full();
        unsafe {
            mmio_write(UART0_BASE + UART_FIFO_REG, byte as u32);
        }
    }

    fn read_byte(&self) -> u8 {
        // Spin until a byte arrives in the RX FIFO.
        while self.rx_fifo_count() == 0 {
            core::hint::spin_loop();
        }
        (unsafe { mmio_read(UART0_BASE + UART_FIFO_REG) }) as u8
    }

    fn has_data(&self) -> bool {
        self.rx_fifo_count() > 0
    }
}

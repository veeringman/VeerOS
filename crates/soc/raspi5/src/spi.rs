#![allow(dead_code)]
//! RP1 SPI master driver for Raspberry Pi 5.
//!
//! The RP1 southbridge provides multiple SPI controllers (SPI0–SPI5).
//! Each is a DW APB SSI (Synopsys DesignWare) compatible controller.
//! This driver implements polled mode SPI transfers using FIFO TX/RX.
//!
//! Default pin mapping:
//!   SPI0: MOSI=GPIO10, MISO=GPIO9, SCLK=GPIO11, CE0=GPIO8, CE1=GPIO7
//!   SPI1: MOSI=GPIO20, MISO=GPIO19, SCLK=GPIO21, CE0=GPIO18, CE1=GPIO17
//!
//! Reference: DW APB SSI datasheet, RP1 datasheet §5 (SPI).

use crate::mem;

// ── RP1 SPI base offsets within the BAR ──────────────────────────────────
const SPI0_BASE: usize = mem::RP1_BAR_BASE + 0x5_0000;
const SPI1_BASE: usize = mem::RP1_BAR_BASE + 0x5_4000;
const SPI2_BASE: usize = mem::RP1_BAR_BASE + 0x5_8000;
const SPI3_BASE: usize = mem::RP1_BAR_BASE + 0x5_C000;
const SPI4_BASE: usize = mem::RP1_BAR_BASE + 0x6_0000;
const SPI5_BASE: usize = mem::RP1_BAR_BASE + 0x6_4000;

// ── DW APB SSI registers ─────────────────────────────────────────────────
const CTRLR0:    usize = 0x00;  // Control register 0
const CTRLR1:    usize = 0x04;  // Number of data frames (rx-only mode)
const SSIENR:    usize = 0x08;  // SSI enable
const SER:       usize = 0x10;  // Slave enable (chip select bitmask)
const BAUDR:     usize = 0x14;  // Baud rate divisor (even values, min 2)
const TXFTLR:    usize = 0x18;  // TX FIFO threshold level
const RXFTLR:    usize = 0x1C;  // RX FIFO threshold level
const TXFLR:     usize = 0x20;  // TX FIFO level (current entries)
const RXFLR:     usize = 0x24;  // RX FIFO level (current entries)
const SR:        usize = 0x28;  // Status register
const DR:        usize = 0x60;  // Data register (TX/RX FIFO access)

// Status register bits
const SR_BUSY:   u32 = 1 << 0;  // SSI busy
const SR_TFNF:   u32 = 1 << 1;  // TX FIFO not full
const SR_TFE:    u32 = 1 << 2;  // TX FIFO empty
const SR_RFNE:   u32 = 1 << 3;  // RX FIFO not empty

// CTRLR0 fields
const CTRLR0_DFS_MASK: u32  = 0xF;        // data frame size [3:0] (DFS-1)
const CTRLR0_SCPH: u32      = 1 << 6;     // serial clock phase (CPHA)
const CTRLR0_SCPOL: u32     = 1 << 7;     // serial clock polarity (CPOL)
const CTRLR0_TMOD_SHIFT: u32 = 8;         // transfer mode [9:8]
const CTRLR0_TMOD_MASK: u32  = 0x3 << 8;

// Transfer modes
const TMOD_TX_AND_RX: u32 = 0x0;
const TMOD_TX_ONLY: u32   = 0x1;
const TMOD_RX_ONLY: u32   = 0x2;

/// FIFO depth (RP1 SPI FIFOs are 16 entries deep).
const FIFO_DEPTH: u32 = 16;

/// SPI clock polarity and phase.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SpiMode {
    Mode0, // CPOL=0, CPHA=0
    Mode1, // CPOL=0, CPHA=1
    Mode2, // CPOL=1, CPHA=0
    Mode3, // CPOL=1, CPHA=1
}

/// An RP1 SPI controller instance.
pub struct Rp1Spi {
    base: usize,
}

// ── Low-level MMIO ──────────────────────────────────────────────────────

#[inline]
unsafe fn mmio_read(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn mmio_write(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

impl Rp1Spi {
    /// Create a handle for specific SPI controller.
    pub const fn new(base: usize) -> Self {
        Self { base }
    }

    /// SPI0 (GPIO 7-11).
    pub const fn spi0() -> Self { Self::new(SPI0_BASE) }
    /// SPI1 (GPIO 17-21).
    pub const fn spi1() -> Self { Self::new(SPI1_BASE) }
    /// SPI2.
    pub const fn spi2() -> Self { Self::new(SPI2_BASE) }
    /// SPI3.
    pub const fn spi3() -> Self { Self::new(SPI3_BASE) }
    /// SPI4.
    pub const fn spi4() -> Self { Self::new(SPI4_BASE) }
    /// SPI5.
    pub const fn spi5() -> Self { Self::new(SPI5_BASE) }

    #[inline]
    fn reg_read(&self, offset: usize) -> u32 {
        unsafe { mmio_read(self.base + offset) }
    }

    #[inline]
    fn reg_write(&self, offset: usize, val: u32) {
        unsafe { mmio_write(self.base + offset, val) }
    }

    /// Disable the SPI controller (required before configuration).
    fn disable(&self) {
        self.reg_write(SSIENR, 0);
    }

    /// Enable the SPI controller.
    fn enable(&self) {
        self.reg_write(SSIENR, 1);
    }

    /// Initialise the SPI controller.
    ///
    /// - `mode`: clock polarity/phase
    /// - `freq_div`: baud rate divisor (must be even, minimum 2).
    ///   SPI clock = RP1 SPI input clock / freq_div.
    ///   At 200 MHz input, freq_div=20 gives 10 MHz SPI clock.
    /// - `bits`: data frame size (4–16, typically 8).
    pub fn init(&self, mode: SpiMode, freq_div: u16, bits: u8) {
        self.disable();

        // Configure CTRLR0: frame size, polarity, phase, TX+RX mode.
        let dfs = ((bits.clamp(4, 16) - 1) as u32) & CTRLR0_DFS_MASK;
        let cpol = match mode {
            SpiMode::Mode2 | SpiMode::Mode3 => CTRLR0_SCPOL,
            _ => 0,
        };
        let cpha = match mode {
            SpiMode::Mode1 | SpiMode::Mode3 => CTRLR0_SCPH,
            _ => 0,
        };
        self.reg_write(CTRLR0, dfs | cpol | cpha | (TMOD_TX_AND_RX << CTRLR0_TMOD_SHIFT));

        // Baud rate divisor (must be even, minimum 2).
        let div = (freq_div.max(2) & !1) as u32;
        self.reg_write(BAUDR, div);

        // FIFO thresholds.
        self.reg_write(TXFTLR, 0);
        self.reg_write(RXFTLR, 0);

        self.enable();
    }

    /// Select chip-select line(s). `cs_mask` is a bitmask (e.g., 1 = CE0).
    pub fn set_cs(&self, cs_mask: u8) {
        self.reg_write(SER, cs_mask as u32);
    }

    /// Wait until the SPI controller is idle.
    fn wait_idle(&self) {
        while self.reg_read(SR) & SR_BUSY != 0 {}
    }

    /// Drain any leftover data from the RX FIFO.
    fn drain_rx(&self) {
        while self.reg_read(SR) & SR_RFNE != 0 {
            let _ = self.reg_read(DR);
        }
    }

    /// Perform a full-duplex SPI transfer.
    ///
    /// Sends bytes from `tx` and simultaneously receives into `rx`.
    /// Both buffers must be the same length. For write-only, pass a
    /// throwaway `rx` buffer. For read-only, fill `tx` with 0x00/0xFF.
    pub fn transfer(&self, tx: &[u8], rx: &mut [u8]) {
        let len = tx.len().min(rx.len());
        self.drain_rx();

        let mut tx_idx = 0usize;
        let mut rx_idx = 0usize;

        while rx_idx < len {
            // Fill TX FIFO.
            while tx_idx < len && self.reg_read(SR) & SR_TFNF != 0 {
                self.reg_write(DR, tx[tx_idx] as u32);
                tx_idx += 1;
            }
            // Drain RX FIFO.
            while rx_idx < len && self.reg_read(SR) & SR_RFNE != 0 {
                rx[rx_idx] = self.reg_read(DR) as u8;
                rx_idx += 1;
            }
        }
    }

    /// Write bytes without caring about received data.
    pub fn write_bytes(&self, data: &[u8]) {
        self.drain_rx();

        let mut tx_idx = 0usize;
        let mut rx_count = 0usize;

        while rx_count < data.len() {
            while tx_idx < data.len() && self.reg_read(SR) & SR_TFNF != 0 {
                self.reg_write(DR, data[tx_idx] as u32);
                tx_idx += 1;
            }
            while self.reg_read(SR) & SR_RFNE != 0 {
                let _ = self.reg_read(DR);
                rx_count += 1;
            }
        }
    }

    /// Read bytes, sending 0xFF for each clock cycle.
    pub fn read_bytes(&self, buf: &mut [u8]) {
        self.drain_rx();

        let mut tx_idx = 0usize;
        let mut rx_idx = 0usize;

        while rx_idx < buf.len() {
            while tx_idx < buf.len() && self.reg_read(SR) & SR_TFNF != 0 {
                self.reg_write(DR, 0xFF);
                tx_idx += 1;
            }
            while rx_idx < buf.len() && self.reg_read(SR) & SR_RFNE != 0 {
                buf[rx_idx] = self.reg_read(DR) as u8;
                rx_idx += 1;
            }
        }
    }

    /// Transfer a single byte (full-duplex) and return the received byte.
    pub fn transfer_byte(&self, out: u8) -> u8 {
        self.drain_rx();
        self.reg_write(DR, out as u32);
        while self.reg_read(SR) & SR_RFNE == 0 {}
        self.reg_read(DR) as u8
    }
}

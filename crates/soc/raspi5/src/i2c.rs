#![allow(dead_code)]
//! RP1 I2C master driver for Raspberry Pi 5.
//!
//! The RP1 southbridge provides multiple I2C controllers (I2C0–I2C6).
//! Each is a DW APB I2C (Synopsys DesignWare) compatible controller.
//! This driver implements polled-mode I2C master transactions.
//!
//! Default pin mapping:
//!   I2C0: SDA=GPIO0,  SCL=GPIO1   (camera/display HAT)
//!   I2C1: SDA=GPIO2,  SCL=GPIO3   (user-facing, header pins 3 & 5)
//!   I2C3: SDA=GPIO4,  SCL=GPIO5   (additional user I2C)
//!
//! Reference: DW APB I2C datasheet, RP1 datasheet §4 (I2C).

use crate::mem;

// ── RP1 I2C base offsets within the BAR ──────────────────────────────────
const I2C0_BASE: usize = mem::RP1_BAR_BASE + 0x7_0000;
const I2C1_BASE: usize = mem::RP1_BAR_BASE + 0x7_4000;
const I2C2_BASE: usize = mem::RP1_BAR_BASE + 0x7_8000;
const I2C3_BASE: usize = mem::RP1_BAR_BASE + 0x7_C000;
const I2C4_BASE: usize = mem::RP1_BAR_BASE + 0x8_0000;
const I2C5_BASE: usize = mem::RP1_BAR_BASE + 0x8_4000;
const I2C6_BASE: usize = mem::RP1_BAR_BASE + 0x8_8000;

// ── DW APB I2C registers ─────────────────────────────────────────────────
const IC_CON:           usize = 0x00;  // Control register
const IC_TAR:           usize = 0x04;  // Target address
const IC_DATA_CMD:      usize = 0x10;  // Data buffer + command
const IC_SS_SCL_HCNT:   usize = 0x14;  // Standard speed SCL high count
const IC_SS_SCL_LCNT:   usize = 0x18;  // Standard speed SCL low count
const IC_FS_SCL_HCNT:   usize = 0x1C;  // Fast speed SCL high count
const IC_FS_SCL_LCNT:   usize = 0x20;  // Fast speed SCL low count
const IC_INTR_STAT:     usize = 0x2C;  // Interrupt status
const IC_INTR_MASK:     usize = 0x30;  // Interrupt mask
const IC_RAW_INTR_STAT: usize = 0x34;  // Raw interrupt status
const IC_RX_TL:         usize = 0x38;  // RX FIFO threshold
const IC_TX_TL:         usize = 0x3C;  // TX FIFO threshold
const IC_CLR_INTR:      usize = 0x40;  // Clear all interrupts
const IC_CLR_TX_ABRT:   usize = 0x54;  // Clear TX_ABRT
const IC_ENABLE:        usize = 0x6C;  // Enable register
const IC_STATUS:        usize = 0x70;  // Status register
const IC_TXFLR:         usize = 0x74;  // TX FIFO level
const IC_RXFLR:         usize = 0x78;  // RX FIFO level
const IC_TX_ABRT_SRC:   usize = 0x80;  // TX abort source
const IC_ENABLE_STATUS: usize = 0x9C;  // Enable status

// IC_CON bits
const CON_MASTER_MODE:  u32 = 1 << 0;
const CON_SPEED_STD:    u32 = 1 << 1;  // Standard mode (100 kHz)
const CON_SPEED_FAST:   u32 = 2 << 1;  // Fast mode (400 kHz)
const CON_10BIT_SLAVE:  u32 = 1 << 3;
const CON_10BIT_MASTER: u32 = 1 << 4;
const CON_RESTART_EN:   u32 = 1 << 5;
const CON_SLAVE_DISABLE: u32 = 1 << 6;

// IC_DATA_CMD bits
const DATA_CMD_READ:    u32 = 1 << 8;   // 1 = read, 0 = write
const DATA_CMD_STOP:    u32 = 1 << 9;   // Generate STOP after this byte
const DATA_CMD_RESTART: u32 = 1 << 10;  // Generate RESTART before this byte

// IC_STATUS bits
const STATUS_ACTIVITY:  u32 = 1 << 0;
const STATUS_TFNF:      u32 = 1 << 1;  // TX FIFO not full
const STATUS_TFE:       u32 = 1 << 2;  // TX FIFO empty
const STATUS_RFNE:      u32 = 1 << 3;  // RX FIFO not empty

// IC_RAW_INTR_STAT bits
const RAW_TX_ABRT:      u32 = 1 << 6;
const RAW_TX_EMPTY:     u32 = 1 << 4;

// RP1 I2C input clock (assumed 200 MHz from RP1 PLL).
const I2C_CLK_HZ: u32 = 200_000_000;
const FIFO_DEPTH: u32 = 16;

/// I2C clock speed selection.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum I2cSpeed {
    /// Standard mode — 100 kHz.
    Standard,
    /// Fast mode — 400 kHz.
    Fast,
}

/// I2C transaction result.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum I2cError {
    /// Slave did not ACK the address.
    Nack,
    /// Arbitration lost.
    ArbitrationLost,
    /// Transfer timed out.
    Timeout,
}

/// An RP1 I2C controller instance.
pub struct Rp1I2c {
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

impl Rp1I2c {
    /// Create a handle for a specific I2C controller.
    pub const fn new(base: usize) -> Self {
        Self { base }
    }

    /// I2C0 (GPIO 0/1).
    pub const fn i2c0() -> Self { Self::new(I2C0_BASE) }
    /// I2C1 (GPIO 2/3 — header pins 3 & 5).
    pub const fn i2c1() -> Self { Self::new(I2C1_BASE) }
    /// I2C2.
    pub const fn i2c2() -> Self { Self::new(I2C2_BASE) }
    /// I2C3 (GPIO 4/5).
    pub const fn i2c3() -> Self { Self::new(I2C3_BASE) }
    /// I2C4.
    pub const fn i2c4() -> Self { Self::new(I2C4_BASE) }
    /// I2C5.
    pub const fn i2c5() -> Self { Self::new(I2C5_BASE) }
    /// I2C6.
    pub const fn i2c6() -> Self { Self::new(I2C6_BASE) }

    #[inline]
    fn reg_read(&self, offset: usize) -> u32 {
        unsafe { mmio_read(self.base + offset) }
    }

    #[inline]
    fn reg_write(&self, offset: usize, val: u32) {
        unsafe { mmio_write(self.base + offset, val) }
    }

    /// Disable the I2C controller (required before configuration).
    fn disable(&self) {
        self.reg_write(IC_ENABLE, 0);
        // Wait for disable to take effect.
        let mut timeout = 100_000u32;
        while self.reg_read(IC_ENABLE_STATUS) & 1 != 0 && timeout > 0 {
            timeout -= 1;
        }
    }

    /// Enable the I2C controller.
    fn enable(&self) {
        self.reg_write(IC_ENABLE, 1);
    }

    /// Initialise the I2C controller in master mode.
    pub fn init(&self, speed: I2cSpeed) {
        self.disable();

        // Master mode, restart enabled, slave disabled.
        let speed_bits = match speed {
            I2cSpeed::Standard => CON_SPEED_STD,
            I2cSpeed::Fast     => CON_SPEED_FAST,
        };
        self.reg_write(IC_CON,
            CON_MASTER_MODE | speed_bits | CON_RESTART_EN | CON_SLAVE_DISABLE);

        // SCL timing: high/low counts for desired frequency.
        // For 200 MHz input clock:
        //   Standard (100 kHz): period = 2000 clocks → H=1000, L=1000
        //   Fast (400 kHz):     period = 500 clocks  → H=250,  L=250
        match speed {
            I2cSpeed::Standard => {
                let hcnt = I2C_CLK_HZ / 100_000 / 2;
                let lcnt = hcnt;
                self.reg_write(IC_SS_SCL_HCNT, hcnt);
                self.reg_write(IC_SS_SCL_LCNT, lcnt);
            }
            I2cSpeed::Fast => {
                let hcnt = I2C_CLK_HZ / 400_000 / 2;
                let lcnt = hcnt;
                self.reg_write(IC_FS_SCL_HCNT, hcnt);
                self.reg_write(IC_FS_SCL_LCNT, lcnt);
            }
        }

        // FIFO thresholds.
        self.reg_write(IC_RX_TL, 0);
        self.reg_write(IC_TX_TL, 0);

        // Mask all interrupts (we poll).
        self.reg_write(IC_INTR_MASK, 0);

        self.enable();
    }

    /// Set the 7-bit target slave address.
    fn set_target(&self, addr: u8) {
        self.disable();
        self.reg_write(IC_TAR, (addr as u32) & 0x7F);
        self.enable();
    }

    /// Check for TX abort and clear it. Returns an error if aborted.
    fn check_abort(&self) -> Result<(), I2cError> {
        let raw = self.reg_read(IC_RAW_INTR_STAT);
        if raw & RAW_TX_ABRT != 0 {
            let src = self.reg_read(IC_TX_ABRT_SRC);
            let _ = self.reg_read(IC_CLR_TX_ABRT); // clear abort
            if src & (1 << 0) != 0 { // ABRT_7B_ADDR_NOACK
                return Err(I2cError::Nack);
            }
            if src & (1 << 7) != 0 { // ABRT_LOST
                return Err(I2cError::ArbitrationLost);
            }
            return Err(I2cError::Nack);
        }
        Ok(())
    }

    /// Wait until TX FIFO is not full, with abort check.
    fn wait_tx_ready(&self) -> Result<(), I2cError> {
        let mut timeout = 1_000_000u32;
        loop {
            self.check_abort()?;
            if self.reg_read(IC_STATUS) & STATUS_TFNF != 0 {
                return Ok(());
            }
            timeout -= 1;
            if timeout == 0 { return Err(I2cError::Timeout); }
        }
    }

    /// Wait until RX FIFO has data, with abort check.
    fn wait_rx_ready(&self) -> Result<(), I2cError> {
        let mut timeout = 1_000_000u32;
        loop {
            self.check_abort()?;
            if self.reg_read(IC_STATUS) & STATUS_RFNE != 0 {
                return Ok(());
            }
            timeout -= 1;
            if timeout == 0 { return Err(I2cError::Timeout); }
        }
    }

    /// Write `data` to the device at `addr`.
    ///
    /// Generates START + address + data bytes + STOP.
    pub fn write_to(&self, addr: u8, data: &[u8]) -> Result<(), I2cError> {
        if data.is_empty() { return Ok(()); }
        self.set_target(addr);

        // Clear any pending interrupts.
        let _ = self.reg_read(IC_CLR_INTR);

        for (i, &byte) in data.iter().enumerate() {
            self.wait_tx_ready()?;
            let mut cmd = byte as u32;
            if i == data.len() - 1 {
                cmd |= DATA_CMD_STOP;
            }
            self.reg_write(IC_DATA_CMD, cmd);
        }

        // Wait for TX FIFO to drain + controller idle.
        let mut timeout = 1_000_000u32;
        while self.reg_read(IC_STATUS) & STATUS_TFE == 0 || self.reg_read(IC_STATUS) & STATUS_ACTIVITY != 0 {
            self.check_abort()?;
            timeout -= 1;
            if timeout == 0 { return Err(I2cError::Timeout); }
        }
        self.check_abort()
    }

    /// Read `buf.len()` bytes from the device at `addr`.
    ///
    /// Generates START + address + repeated reads + STOP.
    pub fn read_from(&self, addr: u8, buf: &mut [u8]) -> Result<(), I2cError> {
        if buf.is_empty() { return Ok(()); }
        self.set_target(addr);
        let _ = self.reg_read(IC_CLR_INTR);

        let mut tx_idx = 0usize;
        let mut rx_idx = 0usize;

        while rx_idx < buf.len() {
            // Issue read commands (up to FIFO depth ahead of reads).
            while tx_idx < buf.len() && (tx_idx - rx_idx) < FIFO_DEPTH as usize {
                self.wait_tx_ready()?;
                let mut cmd = DATA_CMD_READ;
                if tx_idx == buf.len() - 1 {
                    cmd |= DATA_CMD_STOP;
                }
                self.reg_write(IC_DATA_CMD, cmd);
                tx_idx += 1;
            }
            // Collect received bytes.
            while rx_idx < tx_idx {
                self.wait_rx_ready()?;
                buf[rx_idx] = self.reg_read(IC_DATA_CMD) as u8;
                rx_idx += 1;
            }
        }
        self.check_abort()
    }

    /// Write `wr` bytes then read `rd` bytes in a single I2C transaction
    /// (uses RESTART between write and read phases).
    pub fn write_read(&self, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), I2cError> {
        if wr.is_empty() && rd.is_empty() { return Ok(()); }
        self.set_target(addr);
        let _ = self.reg_read(IC_CLR_INTR);

        // Write phase (no STOP).
        for &byte in wr.iter() {
            self.wait_tx_ready()?;
            self.reg_write(IC_DATA_CMD, byte as u32);
        }

        // Read phase with RESTART before first read.
        let mut tx_idx = 0usize;
        let mut rx_idx = 0usize;

        while rx_idx < rd.len() {
            while tx_idx < rd.len() && (tx_idx - rx_idx) < FIFO_DEPTH as usize {
                self.wait_tx_ready()?;
                let mut cmd = DATA_CMD_READ;
                if tx_idx == 0 && !wr.is_empty() {
                    cmd |= DATA_CMD_RESTART;
                }
                if tx_idx == rd.len() - 1 {
                    cmd |= DATA_CMD_STOP;
                }
                self.reg_write(IC_DATA_CMD, cmd);
                tx_idx += 1;
            }
            while rx_idx < tx_idx {
                self.wait_rx_ready()?;
                rd[rx_idx] = self.reg_read(IC_DATA_CMD) as u8;
                rx_idx += 1;
            }
        }
        self.check_abort()
    }
}

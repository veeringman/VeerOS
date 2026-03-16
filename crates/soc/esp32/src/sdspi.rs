//! SPI-mode SD card driver for ESP32-C6.
//!
//! The ESP32-C6 lacks a dedicated SD/MMC host controller, so SD cards
//! are accessed via the GPSPI2 (SPI2) peripheral in SPI mode.
//!
//! # Pin mapping (typical XIAO ESP32-C6 board)
//!   CS   : GPIO 18 (or any available GPIO)
//!   MOSI : GPIO 7  (SPI2 default)
//!   MISO : GPIO 2  (SPI2 default)
//!   CLK  : GPIO 6  (SPI2 default)
//!
//! # SPI-mode SD protocol
//!   - CMD0  with CS asserted → SPI mode entry
//!   - CMD8  → SDv2 detection
//!   - CMD58 → OCR read
//!   - ACMD41 → initialisation (OCR with HCS bit)
//!   - CMD16 → set block length to 512
//!   - CMD17 → single-block read
//!   - CMD24 → single-block write
//!
//! In SPI mode, commands are 6 bytes: `0x40 | cmd_idx`, 4 byte arg, CRC.
//! Responses are R1 (1 byte), R3/R7 (5 bytes), etc.

use arch::BlockDevice;

// ─── SPI2 (GPSPI2) base address ─────────────────────────────────────
/// ESP32-C6 GPSPI2 peripheral base.
const SPI2_BASE: usize = 0x6000_3000;

// ─── SPI registers (offsets from SPI2_BASE) ──────────────────────────
const SPI_CMD_REG:      usize = 0x00;
const SPI_ADDR_REG:     usize = 0x04;
const SPI_CTRL_REG:     usize = 0x08;
const SPI_CLOCK_REG:    usize = 0x0C;
const SPI_USER_REG:     usize = 0x10;
const SPI_USER1_REG:    usize = 0x14;
const SPI_USER2_REG:    usize = 0x18;
const SPI_MS_DLEN_REG:  usize = 0x1C;
const SPI_W0_REG:       usize = 0x58; // Data buffer W0–W15 at 0x58–0x94

/// GPIO output register for controlling CS pin.
const _GPIO_OUT_REG:     usize = 0x6009_1004;
const GPIO_OUT_W1TS:    usize = 0x6009_1008; // set
const GPIO_OUT_W1TC:    usize = 0x6009_100C; // clear
const GPIO_ENABLE_W1TS: usize = 0x6009_1024;

/// CS pin (GPIO 18 by default).
const CS_PIN: u32 = 18;
const CS_MASK: u32 = 1 << CS_PIN;

/// Block size.
const SD_BLOCK_SIZE: usize = 512;

/// Max init retries.
const MAX_INIT_RETRIES: u32 = 1000;

// ─── SD SPI commands ─────────────────────────────────────────────────

/// Build a 6-byte SPI-mode SD command frame.
fn sd_cmd_frame(cmd: u8, arg: u32) -> [u8; 6] {
    let mut frame = [0u8; 6];
    frame[0] = 0x40 | cmd;
    frame[1] = (arg >> 24) as u8;
    frame[2] = (arg >> 16) as u8;
    frame[3] = (arg >> 8) as u8;
    frame[4] = arg as u8;
    // CRC — only CMD0 and CMD8 require valid CRC in SPI mode.
    frame[5] = match cmd {
        0  => 0x95,
        8  => 0x87,
        _  => 0x01,  // dummy + stop bit
    };
    frame
}

// ─── SPI SD driver ───────────────────────────────────────────────────

/// SD card state.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SdState {
    Uninit,
    Ready,
    Error,
}

/// ESP32-C6 SPI-mode SD card driver.
#[derive(Clone, Copy)]
pub struct SpiSd {
    state: SdState,
    sdhc: bool,
    sectors: u64,
}

impl SpiSd {
    pub const fn new() -> Self {
        Self {
            state: SdState::Uninit,
            sdhc: false,
            sectors: 0,
        }
    }

    /// Initialise the SPI peripheral and SD card.
    pub fn init(&mut self) -> bool {
        // Configure SPI2 for low-speed init (~400 kHz)
        self.spi_init(400_000);

        // Configure CS as GPIO output, deassert
        self.cs_deassert();
        unsafe {
            write_reg(GPIO_ENABLE_W1TS, CS_MASK); // enable output
        }

        // Send ≥74 clock cycles with CS high (power-up sequence)
        self.cs_deassert();
        for _ in 0..10 {
            self.spi_transfer_byte(0xFF);
        }

        // CMD0 — GO_IDLE_STATE (CS asserted)
        let r1 = self.send_sd_cmd(0, 0);
        if r1 != 0x01 {
            self.state = SdState::Error;
            return false;
        }

        // CMD8 — SEND_IF_COND (SDv2 check: 0x1AA)
        let r1 = self.send_sd_cmd(8, 0x0000_01AA);
        if r1 == 0x01 {
            // SDv2 card — read 4-byte R7 response
            let mut r7 = [0u8; 4];
            for b in r7.iter_mut() {
                *b = self.spi_transfer_byte(0xFF);
            }
            if r7[2] != 0x01 || r7[3] != 0xAA {
                self.state = SdState::Error;
                return false;
            }

            // ACMD41 — SD_SEND_OP_COND with HCS bit
            let mut tries = MAX_INIT_RETRIES;
            loop {
                self.send_sd_cmd(55, 0); // CMD55 (APP prefix)
                let r1 = self.send_sd_cmd(41, 0x4000_0000);
                if r1 == 0x00 {
                    break;
                }
                tries -= 1;
                if tries == 0 {
                    self.state = SdState::Error;
                    return false;
                }
            }

            // CMD58 — READ_OCR to check CCS (Card Capacity Status)
            let r1 = self.send_sd_cmd(58, 0);
            if r1 == 0x00 {
                let mut ocr = [0u8; 4];
                for b in ocr.iter_mut() {
                    *b = self.spi_transfer_byte(0xFF);
                }
                self.sdhc = (ocr[0] & 0x40) != 0;
            }
        } else {
            self.state = SdState::Error;
            return false;
        }

        // CMD16 — SET_BLOCKLEN to 512 (for SDSCcards)
        if !self.sdhc {
            let r1 = self.send_sd_cmd(16, SD_BLOCK_SIZE as u32);
            if r1 != 0x00 {
                self.state = SdState::Error;
                return false;
            }
        }

        // Switch SPI to full speed (~20 MHz)
        self.spi_init(20_000_000);

        // Default capacity: 2GB / 512 = ~3.9M sectors
        self.sectors = 2 * 1024 * 1024 * 1024 / SD_BLOCK_SIZE as u64;

        self.state = SdState::Ready;
        true
    }

    pub fn is_ready(&self) -> bool {
        self.state == SdState::Ready
    }

    // ── SPI-mode SD command issue ────────────────────────────────────

    /// Send an SD command in SPI mode and return R1 response.
    fn send_sd_cmd(&self, cmd: u8, arg: u32) -> u8 {
        self.cs_assert();
        let frame = sd_cmd_frame(cmd, arg);
        for &b in &frame {
            self.spi_transfer_byte(b);
        }
        // Wait for response (R1: MSB = 0)
        let mut r1 = 0xFFu8;
        for _ in 0..16 {
            r1 = self.spi_transfer_byte(0xFF);
            if r1 & 0x80 == 0 {
                break;
            }
        }
        self.cs_deassert();
        self.spi_transfer_byte(0xFF); // Release bus
        r1
    }

    /// Read a single 512-byte block.
    fn read_block_raw(&self, lba: u64, buf: &mut [u8]) -> bool {
        if buf.len() < SD_BLOCK_SIZE { return false; }

        let addr = if self.sdhc { lba as u32 } else { (lba as u32) * SD_BLOCK_SIZE as u32 };

        self.cs_assert();
        let frame = sd_cmd_frame(17, addr); // CMD17
        for &b in &frame {
            self.spi_transfer_byte(b);
        }

        // Wait for R1
        let mut r1 = 0xFFu8;
        for _ in 0..16 {
            r1 = self.spi_transfer_byte(0xFF);
            if r1 & 0x80 == 0 { break; }
        }
        if r1 != 0x00 {
            self.cs_deassert();
            return false;
        }

        // Wait for data token (0xFE)
        let mut timeout = 100_000u32;
        loop {
            let token = self.spi_transfer_byte(0xFF);
            if token == 0xFE { break; }
            timeout -= 1;
            if timeout == 0 {
                self.cs_deassert();
                return false;
            }
        }

        // Read 512 bytes
        for i in 0..SD_BLOCK_SIZE {
            buf[i] = self.spi_transfer_byte(0xFF);
        }

        // Read (discard) 2-byte CRC
        self.spi_transfer_byte(0xFF);
        self.spi_transfer_byte(0xFF);

        self.cs_deassert();
        self.spi_transfer_byte(0xFF);
        true
    }

    /// Write a single 512-byte block.
    fn write_block_raw(&self, lba: u64, buf: &[u8]) -> bool {
        if buf.len() < SD_BLOCK_SIZE { return false; }

        let addr = if self.sdhc { lba as u32 } else { (lba as u32) * SD_BLOCK_SIZE as u32 };

        self.cs_assert();
        let frame = sd_cmd_frame(24, addr); // CMD24
        for &b in &frame {
            self.spi_transfer_byte(b);
        }

        // Wait for R1
        let mut r1 = 0xFFu8;
        for _ in 0..16 {
            r1 = self.spi_transfer_byte(0xFF);
            if r1 & 0x80 == 0 { break; }
        }
        if r1 != 0x00 {
            self.cs_deassert();
            return false;
        }

        // Send data token
        self.spi_transfer_byte(0xFF); // gap
        self.spi_transfer_byte(0xFE); // data token

        // Send 512 bytes
        for i in 0..SD_BLOCK_SIZE {
            self.spi_transfer_byte(buf[i]);
        }

        // Dummy CRC
        self.spi_transfer_byte(0xFF);
        self.spi_transfer_byte(0xFF);

        // Check data response token
        let resp = self.spi_transfer_byte(0xFF);
        if (resp & 0x1F) != 0x05 {
            self.cs_deassert();
            return false;
        }

        // Wait for write to complete (card pulls MISO low while busy)
        let mut timeout = 500_000u32;
        while self.spi_transfer_byte(0xFF) == 0x00 {
            timeout -= 1;
            if timeout == 0 {
                self.cs_deassert();
                return false;
            }
        }

        self.cs_deassert();
        self.spi_transfer_byte(0xFF);
        true
    }

    // ── SPI hardware abstraction ─────────────────────────────────────

    fn spi_init(&self, _clock_hz: u32) {
        // Configure SPI2 in master mode, SPI mode 0 (CPOL=0, CPHA=0)
        //
        // The ESP32-C6 SPI2 clock is derived from APB_CLK (typically 80 MHz).
        // For 400 kHz: divider = 80_000_000 / 400_000 = 200
        // For 20 MHz:  divider = 80_000_000 / 20_000_000 = 4
        //
        // SPI_CLOCK_REG layout:
        //   [17:12] clkcnt_l, [11:6] clkcnt_h, [5:0] clkcnt_n, [31] clk_equ_sysclk
        let apb_clk = 80_000_000u32;
        let div = apb_clk / _clock_hz;
        let n = if div < 2 { 1 } else { div - 1 };
        let h = n / 2;
        let l = n;

        unsafe {
            write_reg(SPI2_BASE + SPI_CLOCK_REG,
                      (l & 0x3F) | ((h & 0x3F) << 6) | ((n & 0x3F) << 12));
            // SPI_USER: enable MOSI, MISO, duplex mode
            write_reg(SPI2_BASE + SPI_USER_REG,
                      (1 << 27) |  // SPI_USR_MOSI
                      (1 << 28) |  // SPI_USR_MISO
                      (1 << 7));   // SPI_CK_OUT_EDGE (mode 0)
            // SPI_USER1: MOSI bit length = 7 (8 bits - 1), MISO same
            write_reg(SPI2_BASE + SPI_USER1_REG,
                      (7 << 17) |  // SPI_USR_MOSI_BITLEN
                      (7 << 0));   // SPI_USR_MISO_BITLEN
            // SPI_MS_DLEN: total data bits -1 = 7
            write_reg(SPI2_BASE + SPI_MS_DLEN_REG, 7);
            // SPI_CTRL: clear all special modes
            write_reg(SPI2_BASE + SPI_CTRL_REG, 0);
        }
    }

    fn spi_transfer_byte(&self, tx: u8) -> u8 {
        unsafe {
            // Write TX byte into W0
            write_reg(SPI2_BASE + SPI_W0_REG, tx as u32);
            // Start transfer
            write_reg(SPI2_BASE + SPI_CMD_REG, 1 << 18); // SPI_USR bit
            // Wait for completion
            while read_reg(SPI2_BASE + SPI_CMD_REG) & (1 << 18) != 0 {
                core::hint::spin_loop();
            }
            // Read RX byte from W0
            (read_reg(SPI2_BASE + SPI_W0_REG) & 0xFF) as u8
        }
    }

    fn cs_assert(&self) {
        unsafe { write_reg(GPIO_OUT_W1TC, CS_MASK); } // drive low
    }

    fn cs_deassert(&self) {
        unsafe { write_reg(GPIO_OUT_W1TS, CS_MASK); } // drive high
    }
}

// ─── BlockDevice implementation ──────────────────────────────────────

impl BlockDevice for SpiSd {
    fn read_block(&self, lba: u64, buf: &mut [u8]) -> bool {
        if self.state != SdState::Ready { return false; }
        self.read_block_raw(lba, buf)
    }

    fn write_block(&self, lba: u64, buf: &[u8]) -> bool {
        if self.state != SdState::Ready { return false; }
        self.write_block_raw(lba, buf)
    }

    fn block_size(&self) -> usize {
        SD_BLOCK_SIZE
    }

    fn block_count(&self) -> u64 {
        self.sectors
    }
}

// ─── Raw register helpers ────────────────────────────────────────────

#[inline(always)]
unsafe fn read_reg(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn write_reg(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

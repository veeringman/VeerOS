#![allow(dead_code)]
//! EMMC2 / SDHCI SD card driver for Raspberry Pi 5 (BCM2712).
//!
//! Implements `arch::BlockDevice` for the BCM2712's EMMC2 host controller.
//! The EMMC2 is an Arasan SDHCI-compatible controller located at a
//! firmware-mapped address.
//!
//! # Initialisation sequence (simplified SD memory card — SDv2/SDHC)
//!
//! 1. Reset controller (`CONTROL1` clock/reset bits)
//! 2. CMD0  — GO_IDLE_STATE
//! 3. CMD8  — SEND_IF_COND (voltage check, detect SDv2)
//! 4. ACMD41 — SD_SEND_OP_COND (SDHC bit, wait until not busy)
//! 5. CMD2  — ALL_SEND_CID
//! 6. CMD3  — SEND_RELATIVE_ADDR → RCA
//! 7. CMD7  — SELECT_CARD (transfer state)
//! 8. CMD16 — SET_BLOCKLEN (512 bytes)
//!
//! After init, reads use CMD17 (single block) / CMD18 (multi block),
//! writes use CMD24 (single block) / CMD25 (multi block).

use arch::BlockDevice;

// ─── EMMC2 register base ─────────────────────────────────────────────
/// EMMC2 base address on RPi5 (firmware-remapped, per DTB).
/// The BCM2712 maps EMMC2 at this address in the 40-bit PA space.
/// Confirmed via `dtc -I dtb -O dts bcm2712-rpi-5-b.dtb`.
const EMMC2_BASE: usize = 0x10_7D00_4000;

// ─── SDHCI register offsets ──────────────────────────────────────────
const REG_ARG2:         usize = 0x00;
const REG_BLKSIZECNT:   usize = 0x04;
const REG_ARG1:         usize = 0x08;
const REG_CMDTM:        usize = 0x0C;
const REG_RESP0:        usize = 0x10;
const REG_RESP1:        usize = 0x14;
const REG_RESP2:        usize = 0x18;
const REG_RESP3:        usize = 0x1C;
const REG_DATA:         usize = 0x20;
const REG_STATUS:       usize = 0x24;
const REG_CONTROL0:     usize = 0x28;
const REG_CONTROL1:     usize = 0x2C;
const REG_INTERRUPT:    usize = 0x30;
const REG_IRPT_MASK:    usize = 0x34;
const REG_IRPT_EN:      usize = 0x38;
const REG_CONTROL2:     usize = 0x3C;
const REG_SLOTISR_VER:  usize = 0xFC;

// ─── SDHCI status / interrupt bits ───────────────────────────────────
const INT_CMD_DONE:   u32 = 1 << 0;
const INT_DATA_DONE:  u32 = 1 << 1;
const INT_READ_RDY:   u32 = 1 << 5;
const INT_WRITE_RDY:  u32 = 1 << 4;
const INT_ERROR:      u32 = 0xFFFF_0000;

const STATUS_CMD_INHIBIT:  u32 = 1 << 0;
const STATUS_DAT_INHIBIT:  u32 = 1 << 1;
const STATUS_READ_AVAIL:   u32 = 1 << 11;
const STATUS_WRITE_AVAIL:  u32 = 1 << 10;

// ─── Command flags (CMDTM register layout) ──────────────────────────
const CMD_NEED_APP:    u32 = 0x8000_0000;  // our flag: must prefix with CMD55
const CMD_RSPNS_48:    u32 = 0x0002_0000;  // 48-bit response
const CMD_RSPNS_136:   u32 = 0x0001_0000;  // 136-bit response
const CMD_RSPNS_48B:   u32 = 0x0003_0000;  // 48-bit busy check
const CMD_IS_DATA:     u32 = 0x0020_0000;  // command involves data transfer
const CMD_DATA_READ:   u32 = 0x0010_0000;  // data direction: read
const CMD_CRCCHK_EN:   u32 = 0x0008_0000;  // enable CRC checking
const CMD_IXCHK_EN:    u32 = 0x0010_0000;  // enable index checking

// ─── SD commands ─────────────────────────────────────────────────────
const fn make_cmd(idx: u32) -> u32 {
    idx << 24
}

const CMD_GO_IDLE:     u32 = make_cmd(0);
const CMD_SEND_IF_COND: u32 = make_cmd(8) | CMD_RSPNS_48 | CMD_CRCCHK_EN;
const CMD_ALL_SEND_CID: u32 = make_cmd(2) | CMD_RSPNS_136;
const CMD_SEND_REL_ADDR: u32 = make_cmd(3) | CMD_RSPNS_48;
const CMD_SELECT_CARD: u32 = make_cmd(7) | CMD_RSPNS_48B;
const CMD_SET_BLOCKLEN: u32 = make_cmd(16) | CMD_RSPNS_48;
const CMD_READ_SINGLE: u32 = make_cmd(17) | CMD_RSPNS_48 | CMD_IS_DATA | CMD_DATA_READ;
const CMD_WRITE_SINGLE: u32 = make_cmd(24) | CMD_RSPNS_48 | CMD_IS_DATA;
const CMD_APP_CMD:     u32 = make_cmd(55) | CMD_RSPNS_48;
const ACMD_SEND_OP_COND: u32 = make_cmd(41) | CMD_RSPNS_48 | CMD_NEED_APP;
const CMD_SEND_CSD:    u32 = make_cmd(9) | CMD_RSPNS_136;

/// Block size — standard 512 bytes.
const SD_BLOCK_SIZE: usize = 512;

/// Maximum retries for ACMD41 initialisation.
const ACMD41_RETRIES: u32 = 100;

// ─── SD card driver ──────────────────────────────────────────────────

/// State of the SD card.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SdState {
    /// Not yet initialised.
    Uninit,
    /// Successfully initialised and ready.
    Ready,
    /// Init failed — card unusable.
    Error,
}

/// EMMC2 SD card driver.
#[derive(Clone, Copy)]
pub struct Emmc2Sd {
    base: usize,
    state: SdState,
    /// Relative Card Address (from CMD3).
    rca: u32,
    /// Is this an SDHC card (block-addressed)?
    sdhc: bool,
    /// Total sectors (from CSD or fallback).
    sectors: u64,
}

impl Emmc2Sd {
    /// Create a new (un-initialised) SD card handle.
    pub const fn new() -> Self {
        Self {
            base: EMMC2_BASE,
            state: SdState::Uninit,
            rca: 0,
            sdhc: false,
            sectors: 0,
        }
    }

    /// Attempt to initialise the SD card. Returns `true` on success.
    pub fn init(&mut self) -> bool {
        // Reset the controller
        self.write_reg(REG_CONTROL1, 0x0F00_0000); // reset all
        self.delay(100);

        // Wait for reset to complete
        let mut timeout = 10000u32;
        while self.read_reg(REG_CONTROL1) & 0x0700_0000 != 0 && timeout > 0 {
            timeout -= 1;
            self.delay(10);
        }
        if timeout == 0 {
            self.state = SdState::Error;
            return false;
        }

        // Enable internal clock and set a slow init clock (~400 kHz)
        // Divisor: base clock / 2 / div. With 200 MHz base, div=0x100 → ~390 kHz.
        let mut ctrl1 = self.read_reg(REG_CONTROL1);
        ctrl1 |= 0x0001;           // CLK_INTLEN
        ctrl1 |= 0x00E0_0000;      // DATA_TOUNIT = max
        ctrl1 &= !0x0000_FFC0;     // clear freq_sel
        ctrl1 |= 0x0080_0000 | (0x100 << 8); // SDCLK freq divider
        self.write_reg(REG_CONTROL1, ctrl1);
        self.delay(10);

        // Wait for clock stable
        timeout = 10000;
        while self.read_reg(REG_CONTROL1) & 0x02 == 0 && timeout > 0 {
            timeout -= 1;
            self.delay(10);
        }
        if timeout == 0 {
            self.state = SdState::Error;
            return false;
        }

        // Enable SD clock
        let ctrl1 = self.read_reg(REG_CONTROL1);
        self.write_reg(REG_CONTROL1, ctrl1 | 0x04);

        // Enable interrupts we care about
        self.write_reg(REG_IRPT_MASK, 0xFFFF_FFFF);
        self.write_reg(REG_IRPT_EN, 0xFFFF_FFFF);
        self.write_reg(REG_INTERRUPT, 0xFFFF_FFFF); // clear pending

        // CMD0 — GO_IDLE_STATE
        if !self.send_cmd(CMD_GO_IDLE, 0) {
            self.state = SdState::Error;
            return false;
        }

        // CMD8 — SEND_IF_COND (0x1AA = 2.7–3.6V, check pattern 0xAA)
        if !self.send_cmd(CMD_SEND_IF_COND, 0x1AA) {
            self.state = SdState::Error;
            return false;
        }
        let r = self.read_reg(REG_RESP0);
        if r & 0xFFF != 0x1AA {
            self.state = SdState::Error;
            return false;
        }

        // ACMD41 — SD_SEND_OP_COND (SDHC support bit set)
        let mut tries = ACMD41_RETRIES;
        loop {
            if !self.send_app_cmd(ACMD_SEND_OP_COND, 0x5100_0000) {
                self.state = SdState::Error;
                return false;
            }
            let ocr = self.read_reg(REG_RESP0);
            if ocr & 0x8000_0000 != 0 {
                // Card is ready
                self.sdhc = (ocr & 0x4000_0000) != 0;
                break;
            }
            tries -= 1;
            if tries == 0 {
                self.state = SdState::Error;
                return false;
            }
            self.delay(100);
        }

        // CMD2 — ALL_SEND_CID
        if !self.send_cmd(CMD_ALL_SEND_CID, 0) {
            self.state = SdState::Error;
            return false;
        }

        // CMD3 — SEND_RELATIVE_ADDR
        if !self.send_cmd(CMD_SEND_REL_ADDR, 0) {
            self.state = SdState::Error;
            return false;
        }
        self.rca = self.read_reg(REG_RESP0) & 0xFFFF_0000;

        // CMD9 — SEND_CSD (get card capacity)
        if self.send_cmd(CMD_SEND_CSD, self.rca) {
            let csd0 = self.read_reg(REG_RESP0);
            let csd1 = self.read_reg(REG_RESP1);
            let _csd2 = self.read_reg(REG_RESP2);

            if self.sdhc {
                // CSD v2: C_SIZE is in bits [69:48] of the 128-bit CSD
                let c_size = ((csd1 & 0x3F) << 16) | (csd0 >> 16);
                self.sectors = (c_size as u64 + 1) * 1024;
            } else {
                // CSD v1 fallback: assume common size
                self.sectors = 0; // will be filled by CMD9 parse or default
            }
        }
        if self.sectors == 0 {
            // Fallback: assume 2 GB card (~3.9M sectors)
            self.sectors = 2 * 1024 * 1024 * 1024 / SD_BLOCK_SIZE as u64;
        }

        // CMD7 — SELECT_CARD (enter transfer state)
        if !self.send_cmd(CMD_SELECT_CARD, self.rca) {
            self.state = SdState::Error;
            return false;
        }

        // CMD16 — SET_BLOCKLEN (512)
        if !self.sdhc {
            if !self.send_cmd(CMD_SET_BLOCKLEN, SD_BLOCK_SIZE as u32) {
                self.state = SdState::Error;
                return false;
            }
        }

        self.state = SdState::Ready;
        true
    }

    /// True if the card is initialised and ready.
    pub fn is_ready(&self) -> bool {
        self.state == SdState::Ready
    }

    // ── Low-level register I/O ───────────────────────────────────────

    #[inline(always)]
    fn read_reg(&self, offset: usize) -> u32 {
        unsafe { core::ptr::read_volatile((self.base + offset) as *const u32) }
    }

    #[inline(always)]
    fn write_reg(&self, offset: usize, val: u32) {
        unsafe { core::ptr::write_volatile((self.base + offset) as *mut u32, val); }
    }

    fn delay(&self, _us: u32) {
        // Simple spin delay — good enough for init.
        for _ in 0..(_us * 10) {
            core::hint::spin_loop();
        }
    }

    // ── Command issue ────────────────────────────────────────────────

    /// Wait for command inhibit to clear.
    fn wait_cmd_ready(&self) -> bool {
        let mut timeout = 100_000u32;
        while self.read_reg(REG_STATUS) & STATUS_CMD_INHIBIT != 0 {
            if timeout == 0 { return false; }
            timeout -= 1;
            core::hint::spin_loop();
        }
        true
    }

    /// Wait for data inhibit to clear.
    fn wait_data_ready(&self) -> bool {
        let mut timeout = 500_000u32;
        while self.read_reg(REG_STATUS) & STATUS_DAT_INHIBIT != 0 {
            if timeout == 0 { return false; }
            timeout -= 1;
            core::hint::spin_loop();
        }
        true
    }

    /// Send a command (non-APP). Returns `true` if CMD_DONE fires without error.
    fn send_cmd(&self, cmd: u32, arg: u32) -> bool {
        if !self.wait_cmd_ready() { return false; }
        self.write_reg(REG_INTERRUPT, 0xFFFF_FFFF); // clear pending
        self.write_reg(REG_ARG1, arg);
        self.write_reg(REG_CMDTM, cmd);

        // Wait for CMD_DONE or error
        let mut timeout = 100_000u32;
        loop {
            let irq = self.read_reg(REG_INTERRUPT);
            if irq & INT_ERROR != 0 {
                self.write_reg(REG_INTERRUPT, 0xFFFF_FFFF);
                return false;
            }
            if irq & INT_CMD_DONE != 0 {
                self.write_reg(REG_INTERRUPT, INT_CMD_DONE);
                return true;
            }
            timeout -= 1;
            if timeout == 0 { return false; }
            core::hint::spin_loop();
        }
    }

    /// Send an APP command (CMD55 prefix + the actual command).
    fn send_app_cmd(&self, cmd: u32, arg: u32) -> bool {
        let real_cmd = cmd & !CMD_NEED_APP;
        if !self.send_cmd(CMD_APP_CMD, self.rca) {
            return false;
        }
        self.send_cmd(real_cmd, arg)
    }

    /// Read a single 512-byte block at the given LBA.
    fn read_block_raw(&self, lba: u64, buf: &mut [u8]) -> bool {
        if buf.len() < SD_BLOCK_SIZE { return false; }
        if !self.wait_data_ready() { return false; }

        // Set block size and count = 1
        self.write_reg(REG_BLKSIZECNT, (1 << 16) | SD_BLOCK_SIZE as u32);

        // SDHC uses block addressing; SDSC uses byte addressing
        let addr = if self.sdhc { lba as u32 } else { (lba as u32) * SD_BLOCK_SIZE as u32 };

        self.write_reg(REG_INTERRUPT, 0xFFFF_FFFF);
        self.write_reg(REG_ARG1, addr);
        self.write_reg(REG_CMDTM, CMD_READ_SINGLE);

        // Wait for CMD_DONE
        if !self.wait_for_irq(INT_CMD_DONE) { return false; }

        // Wait for READ_RDY
        if !self.wait_for_irq(INT_READ_RDY) { return false; }

        // Read data 4 bytes at a time
        for i in (0..SD_BLOCK_SIZE).step_by(4) {
            let word = self.read_reg(REG_DATA);
            buf[i]   = (word & 0xFF) as u8;
            buf[i+1] = ((word >> 8) & 0xFF) as u8;
            buf[i+2] = ((word >> 16) & 0xFF) as u8;
            buf[i+3] = ((word >> 24) & 0xFF) as u8;
        }

        // Wait for DATA_DONE
        self.wait_for_irq(INT_DATA_DONE)
    }

    /// Write a single 512-byte block at the given LBA.
    fn write_block_raw(&self, lba: u64, buf: &[u8]) -> bool {
        if buf.len() < SD_BLOCK_SIZE { return false; }
        if !self.wait_data_ready() { return false; }

        self.write_reg(REG_BLKSIZECNT, (1 << 16) | SD_BLOCK_SIZE as u32);

        let addr = if self.sdhc { lba as u32 } else { (lba as u32) * SD_BLOCK_SIZE as u32 };

        self.write_reg(REG_INTERRUPT, 0xFFFF_FFFF);
        self.write_reg(REG_ARG1, addr);
        self.write_reg(REG_CMDTM, CMD_WRITE_SINGLE);

        if !self.wait_for_irq(INT_CMD_DONE) { return false; }
        if !self.wait_for_irq(INT_WRITE_RDY) { return false; }

        // Write data 4 bytes at a time
        for i in (0..SD_BLOCK_SIZE).step_by(4) {
            let word = (buf[i] as u32)
                     | ((buf[i+1] as u32) << 8)
                     | ((buf[i+2] as u32) << 16)
                     | ((buf[i+3] as u32) << 24);
            self.write_reg(REG_DATA, word);
        }

        self.wait_for_irq(INT_DATA_DONE)
    }

    /// Wait for a specific interrupt bit (with timeout).
    fn wait_for_irq(&self, mask: u32) -> bool {
        let mut timeout = 500_000u32;
        loop {
            let irq = self.read_reg(REG_INTERRUPT);
            if irq & INT_ERROR != 0 {
                self.write_reg(REG_INTERRUPT, 0xFFFF_FFFF);
                return false;
            }
            if irq & mask != 0 {
                self.write_reg(REG_INTERRUPT, mask);
                return true;
            }
            timeout -= 1;
            if timeout == 0 { return false; }
            core::hint::spin_loop();
        }
    }
}

// ─── BlockDevice implementation ──────────────────────────────────────

impl BlockDevice for Emmc2Sd {
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

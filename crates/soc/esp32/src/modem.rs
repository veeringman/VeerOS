//! ESP32-C6 modem subsystem register definitions.
//!
//! The modem subsystem includes the Wi-Fi MAC, BLE baseband, and
//! IEEE 802.15.4 MAC. All three share the 2.4 GHz RF front-end
//! and require coordinated power/clock management.
//!
//! These constants are derived from the ESP32-C6 Technical Reference
//! Manual and are used by userspace driver tasks via MMIO syscalls.

use core::fmt::{self, Write};

// ═══════════════════════════════════════════════════════════════════════════
// Modem clock / power control
// ═══════════════════════════════════════════════════════════════════════════

/// MODEM_LPCON (low-power controller) — manages radio clock gates.
#[cfg(feature = "c6")]
pub const MODEM_LPCON_BASE: usize = 0x600A_F000;
#[cfg(feature = "c3")]
pub const MODEM_LPCON_BASE: usize = 0x6003_5000;
#[cfg(all(not(feature = "c6"), not(feature = "c3")))]
pub const MODEM_LPCON_BASE: usize = 0x600A_F000;

/// MODEM_SYSCON — modem system configuration.
#[cfg(any(feature = "c6", feature = "h2"))]
pub const MODEM_SYSCON_BASE: usize = 0x600A_9800;
#[cfg(all(not(feature = "c6"), not(feature = "h2")))]
pub const MODEM_SYSCON_BASE: usize = 0x600A_9800;

/// Offset: modem clock enable register.
pub const MODEM_CLK_EN: usize = 0x00;
/// Offset: modem reset control register.
pub const MODEM_RST_CTRL: usize = 0x04;
/// Offset: Wi-Fi clock enable.
pub const WIFI_CLK_EN: usize = 0x14;
/// Offset: BLE clock enable.
pub const BLE_CLK_EN: usize = 0x18;

// ═══════════════════════════════════════════════════════════════════════════
// Wi-Fi MAC registers
// ═══════════════════════════════════════════════════════════════════════════

/// Wi-Fi MAC baseband base address (ESP32-C6).
#[cfg(feature = "c6")]
pub const WIFI_BB_BASE: usize = 0x600A_7800;
#[cfg(all(not(feature = "c6")))]
pub const WIFI_BB_BASE: usize = 0x600A_7800;

/// Wi-Fi MAC peripheral base.
#[cfg(feature = "c6")]
pub const WIFI_MAC_BASE: usize = 0x600A_4000;
#[cfg(all(not(feature = "c6")))]
pub const WIFI_MAC_BASE: usize = 0x600A_4000;

/// MMIO region covering the full Wi-Fi MAC + baseband.
pub const WIFI_MMIO_BASE: usize = 0x600A_4000;
pub const WIFI_MMIO_SIZE: usize = 0x8000; // 32 KiB covers MAC + BB

// ═══════════════════════════════════════════════════════════════════════════
// BLE controller registers
// ═══════════════════════════════════════════════════════════════════════════

/// BLE baseband base address.
#[cfg(feature = "c6")]
pub const BLE_BB_BASE: usize = 0x600A_C000;
#[cfg(feature = "c3")]
pub const BLE_BB_BASE: usize = 0x6001_C000;
#[cfg(all(not(feature = "c6"), not(feature = "c3")))]
pub const BLE_BB_BASE: usize = 0x600A_C000;

/// MMIO region for BLE.
pub const BLE_MMIO_BASE: usize = 0x600A_C000;
pub const BLE_MMIO_SIZE: usize = 0x1000;

// ═══════════════════════════════════════════════════════════════════════════
// IEEE 802.15.4 MAC registers
// ═══════════════════════════════════════════════════════════════════════════

/// IEEE 802.15.4 MAC peripheral base address.
#[cfg(any(feature = "c6", feature = "h2"))]
pub const IEEE802154_MAC_BASE: usize = 0x600A_3000;
#[cfg(all(not(feature = "c6"), not(feature = "h2")))]
pub const IEEE802154_MAC_BASE: usize = 0x600A_3000;

/// MMIO region for 802.15.4.
pub const IEEE802154_MMIO_BASE: usize = 0x600A_3000;
pub const IEEE802154_MMIO_SIZE: usize = 0x1000;

// ═══════════════════════════════════════════════════════════════════════════
// Shared modem MMIO region (covers all radio subsystems)
// ═══════════════════════════════════════════════════════════════════════════

/// The full modem register space spans 0x600A_3000 - 0x600B_0000.
pub const MODEM_MMIO_BASE: usize = 0x600A_3000;
pub const MODEM_MMIO_SIZE: usize = 0x0D000; // 52 KiB

// ═══════════════════════════════════════════════════════════════════════════
// Modem power-up / clock-enable bit masks
// ═══════════════════════════════════════════════════════════════════════════

/// Bit in MODEM_CLK_EN to enable Wi-Fi clock domain.
pub const CLK_WIFI_EN: u32 = 1 << 0;
/// Bit in MODEM_CLK_EN to enable BLE clock domain.
pub const CLK_BLE_EN: u32 = 1 << 1;
/// Bit in MODEM_CLK_EN to enable IEEE 802.15.4 clock domain.
pub const CLK_IEEE802154_EN: u32 = 1 << 2;
/// Bit in MODEM_CLK_EN to enable the shared RF front-end.
pub const CLK_FE_EN: u32 = 1 << 4;

/// Bit in MODEM_RST_CTRL to reset Wi-Fi MAC.
pub const RST_WIFI_MAC: u32 = 1 << 0;
/// Bit in MODEM_RST_CTRL to reset BLE baseband.
pub const RST_BLE_BB: u32 = 1 << 1;
/// Bit in MODEM_RST_CTRL to reset 802.15.4 MAC.
pub const RST_IEEE802154_MAC: u32 = 1 << 2;

// ═══════════════════════════════════════════════════════════════════════════
// WiFi MAC register offsets (from WIFI_MAC_BASE)
// ═══════════════════════════════════════════════════════════════════════════

/// WiFi MAC RX control register.
pub const WIFI_RX_CTRL: usize = 0x00;
/// WiFi MAC TX control register.
pub const WIFI_TX_CTRL: usize = 0x04;
/// WiFi MAC filter / BSSID register.
pub const WIFI_BSSID_FILTER: usize = 0x28;
/// WiFi MAC address low 32 bits.
pub const WIFI_MAC_ADDR_LO: usize = 0x40;
/// WiFi MAC address high 16 bits.
pub const WIFI_MAC_ADDR_HI: usize = 0x44;
/// WiFi channel configuration register.
pub const WIFI_CHANNEL_CFG: usize = 0x4C;
/// WiFi RX data buffer descriptor pointer.
pub const WIFI_RX_DESCR: usize = 0x80;
/// WiFi TX data buffer descriptor pointer.
pub const WIFI_TX_DESCR: usize = 0x84;
/// WiFi status register (connected/scanning/idle).
pub const WIFI_STATUS: usize = 0xC0;
/// WiFi interrupt status register.
pub const WIFI_INT_STATUS: usize = 0xC4;
/// WiFi interrupt enable register.
pub const WIFI_INT_ENA: usize = 0xC8;
/// WiFi interrupt clear register.
pub const WIFI_INT_CLR: usize = 0xCC;

// WiFi baseband register offsets (from WIFI_BB_BASE)
/// BB TX power control.
pub const BB_TX_POWER: usize = 0x00;
/// BB RF channel.
pub const BB_CHANNEL: usize = 0x68;
/// BB AGC (automatic gain control).
pub const BB_AGC_CTRL: usize = 0x6C;
/// BB RSSI read register.
pub const BB_RSSI: usize = 0x78;

// WiFi interrupt bit definitions
/// Bit: TX done.
pub const WIFI_INT_TX_DONE: u32 = 1 << 0;
/// Bit: RX done.
pub const WIFI_INT_RX_DONE: u32 = 1 << 1;
/// Bit: TX timeout.
pub const WIFI_INT_TX_TIMEOUT: u32 = 1 << 2;
/// Bit: Beacon received.
pub const WIFI_INT_BEACON: u32 = 1 << 4;
/// Bit: scan complete.
pub const WIFI_INT_SCAN_DONE: u32 = 1 << 5;

// ═══════════════════════════════════════════════════════════════════════════
// BLE controller register offsets (from BLE_BB_BASE)
// ═══════════════════════════════════════════════════════════════════════════

/// BLE control register.
pub const BLE_CTRL: usize = 0x00;
/// BLE status register.
pub const BLE_STATUS: usize = 0x04;
/// BLE TX power.
pub const BLE_TX_POWER: usize = 0x08;
/// BLE advertising parameters.
pub const BLE_ADV_PARAMS: usize = 0x10;
/// BLE scan parameters.
pub const BLE_SCAN_PARAMS: usize = 0x14;
/// BLE connection parameters.
pub const BLE_CONN_PARAMS: usize = 0x18;
/// BLE advertising enable.
pub const BLE_ADV_ENABLE: usize = 0x1C;
/// BLE scan enable.
pub const BLE_SCAN_ENABLE: usize = 0x20;
/// BLE data channel map.
pub const BLE_CHANNEL_MAP: usize = 0x24;
/// BLE local address low.
pub const BLE_ADDR_LO: usize = 0x30;
/// BLE local address high.
pub const BLE_ADDR_HI: usize = 0x34;
/// BLE interrupt status.
pub const BLE_INT_STATUS: usize = 0x40;
/// BLE interrupt enable.
pub const BLE_INT_ENA: usize = 0x44;
/// BLE interrupt clear.
pub const BLE_INT_CLR: usize = 0x48;
/// BLE whitelist register.
pub const BLE_WHITELIST: usize = 0x50;
/// BLE RX buffer descriptor.
pub const BLE_RX_DESCR: usize = 0x60;
/// BLE TX buffer descriptor.
pub const BLE_TX_DESCR: usize = 0x64;
/// BLE TX control.
pub const BLE_TX_CTRL: usize = 0x68;
/// BLE connection target address low.
pub const BLE_CONN_ADDR_LO: usize = 0x70;
/// BLE connection target address high.
pub const BLE_CONN_ADDR_HI: usize = 0x74;
/// BLE connection enable.
pub const BLE_CONN_ENABLE: usize = 0x78;
/// BLE connection handle (read-only, assigned by controller).
pub const BLE_CONN_HANDLE: usize = 0x7C;

// BLE control bits
pub const BLE_CTRL_ENABLE: u32 = 1 << 0;
pub const BLE_CTRL_RESET: u32 = 1 << 1;

// BLE interrupt bits
pub const BLE_INT_ADV_DONE: u32 = 1 << 0;
pub const BLE_INT_SCAN_DONE: u32 = 1 << 1;
pub const BLE_INT_CONN_DONE: u32 = 1 << 2;
pub const BLE_INT_RX_DONE: u32 = 1 << 3;
pub const BLE_INT_TX_DONE: u32 = 1 << 4;

// ═══════════════════════════════════════════════════════════════════════════
// IEEE 802.15.4 MAC additional register offsets (from IEEE802154_MAC_BASE)
// ═══════════════════════════════════════════════════════════════════════════

/// 802.15.4 MAC control register.
pub const ZB_MAC_CTRL: usize = 0x00;
/// 802.15.4 TX power register.
pub const ZB_TX_POWER: usize = 0x04;
/// 802.15.4 channel register.
pub const ZB_CHANNEL: usize = 0x08;
/// 802.15.4 PAN ID register.
pub const ZB_PAN_ID: usize = 0x0C;
/// 802.15.4 short address register.
pub const ZB_SHORT_ADDR: usize = 0x10;
/// 802.15.4 extended address low register.
pub const ZB_EXT_ADDR_LO: usize = 0x14;
/// 802.15.4 extended address high register.
pub const ZB_EXT_ADDR_HI: usize = 0x18;
/// 802.15.4 TX FIFO write port.
pub const ZB_TX_FIFO: usize = 0x20;
/// 802.15.4 RX FIFO read port / status.
pub const ZB_RX_FIFO: usize = 0x24;
/// 802.15.4 frame filter control.
pub const ZB_FRAME_FILTER: usize = 0x28;
/// 802.15.4 auto-ACK control.
pub const ZB_AUTO_ACK: usize = 0x2C;
/// 802.15.4 CCA/ED scan control.
pub const ZB_ED_SCAN: usize = 0x30;
/// 802.15.4 interrupt status.
pub const ZB_INT_STATUS: usize = 0x40;
/// 802.15.4 interrupt enable.
pub const ZB_INT_ENA: usize = 0x44;
/// 802.15.4 interrupt clear.
pub const ZB_INT_CLR: usize = 0x48;
/// 802.15.4 MAC status register.
pub const ZB_MAC_STATUS: usize = 0x50;
/// 802.15.4 energy detection result.
pub const ZB_ED_RESULT: usize = 0x54;
/// 802.15.4 RX frame length.
pub const ZB_RX_LEN: usize = 0x58;
/// 802.15.4 RX frame LQI.
pub const ZB_RX_LQI: usize = 0x5C;
/// 802.15.4 sequence number register.
pub const ZB_SEQ_NUM: usize = 0x60;
/// 802.15.4 TX frame length.
pub const ZB_TX_LEN: usize = 0x64;

// 802.15.4 MAC control bits
pub const ZB_CTRL_ENABLE: u32 = 1 << 0;
pub const ZB_CTRL_RX_ON: u32 = 1 << 1;
pub const ZB_CTRL_TX_START: u32 = 1 << 2;
pub const ZB_CTRL_ED_START: u32 = 1 << 3;
pub const ZB_CTRL_AUTO_ACK: u32 = 1 << 4;
pub const ZB_CTRL_COORD: u32 = 1 << 5;

// 802.15.4 interrupt bits
pub const ZB_INT_TX_DONE: u32 = 1 << 0;
pub const ZB_INT_RX_DONE: u32 = 1 << 1;
pub const ZB_INT_TX_FAIL: u32 = 1 << 2;
pub const ZB_INT_ED_DONE: u32 = 1 << 3;
pub const ZB_INT_ACK_RCVD: u32 = 1 << 4;

// ═══════════════════════════════════════════════════════════════════════════
// PMU registers (power management unit — modem clock gating)
// ═══════════════════════════════════════════════════════════════════════════

const PMU_BASE: usize = 0x600B_0000;
/// PMU HP-active ICG modem code — bits [31:30].
const PMU_HP_ACTIVE_ICG_MODEM: usize = PMU_BASE + 0x0C;
/// PMU HP-modem ICG modem code — bits [31:30].
const PMU_HP_MODEM_ICG_MODEM: usize = PMU_BASE + 0x40;
/// PMU HP-sleep ICG modem code — bits [31:30].
const PMU_HP_SLEEP_ICG_MODEM: usize = PMU_BASE + 0x74;
/// PMU immediate sleep sysclk — bit 28 = update_dig_icg_switch.
const PMU_IMM_SLEEP_SYSCLK: usize = PMU_BASE + 0xD0;
/// PMU immediate modem ICG — bit 31 = update_dig_icg_modem_en.
const PMU_IMM_MODEM_ICG: usize = PMU_BASE + 0xDC;
/// Immediate HP clock power (write-trigger). Gold `enable_pll_clk_impl`.
const PMU_IMM_HP_CK_POWER: usize = PMU_BASE + 0xCC;

const I2C_ANA_MST_BASE: usize = 0x600A_F800;
const I2C_ANA_CTRL0: usize = I2C_ANA_MST_BASE;
const I2C_ANA_CTRL1: usize = I2C_ANA_MST_BASE + 0x04;
const I2C_ANA_CONF0: usize = I2C_ANA_MST_BASE + 0x18;
const I2C_ANA_CONF1: usize = I2C_ANA_MST_BASE + 0x1C;
const I2C_ANA_CONF2: usize = I2C_ANA_MST_BASE + 0x20;
const PCR_SYSCLK_CONF: usize = 0x6009_6110;
const PCR_CPU_FREQ_CONF: usize = 0x6009_6118;

const REGI2C_BBPLL: u8 = 0x66;
const BBPLL_OC_REF: u8 = 2;
const BBPLL_OC_DIV: u8 = 3;
const BBPLL_OC_DR: u8 = 5;
const BBPLL_REG6: u8 = 6;
const BBPLL_REG9: u8 = 9;

// ═══════════════════════════════════════════════════════════════════════════
// MODEM_SYSCON register offsets (from MODEM_SYSCON_BASE = 0x600A_9800)
// ═══════════════════════════════════════════════════════════════════════════

/// State-based clock gating for modem subsystems.
const MODEM_SYSCON_CLK_CONF_POWER_ST: usize = MODEM_SYSCON_BASE + 0x0C;
/// WiFi BB/FE/MAC clock enables.
const MODEM_SYSCON_CLK_CONF1: usize = MODEM_SYSCON_BASE + 0x14;

// ═══════════════════════════════════════════════════════════════════════════
// MODEM_LPCON register offsets (from MODEM_LPCON_BASE = 0x600A_F000)
// ═══════════════════════════════════════════════════════════════════════════

/// WiFi LP clock source selection + divider.
const MODEM_LPCON_WIFI_LP_CLK_CONF: usize = MODEM_LPCON_BASE + 0x0C;
/// I2C master clock config — bit 0 = sel_160m.
const MODEM_LPCON_I2C_MST_CLK_CONF: usize = MODEM_LPCON_BASE + 0x10;
/// Main clock enables — bits: 0=wifipwr, 1=coex, 2=i2c_mst, 3=lp_timer.
const MODEM_LPCON_CLK_CONF: usize = MODEM_LPCON_BASE + 0x18;
/// State-based clock gating for LP modem subsystems.
const MODEM_LPCON_CLK_CONF_POWER_ST: usize = MODEM_LPCON_BASE + 0x20;
/// MODEM_LPCON reset config — bit 0=rst_wifipwr, 1=rst_coex, 2=rst_i2c_mst, 3=rst_lp_timer.
const MODEM_LPCON_RST_CONF: usize = MODEM_LPCON_BASE + 0x24;

/// MODEM_SYSCON CLK_CONF_FORCE_ON — force clocks on (bypass state gating).
const MODEM_SYSCON_CLK_CONF_FORCE_ON: usize = MODEM_SYSCON_BASE + 0x08;
/// MODEM_SYSCON CLK_CONF1_FORCE_ON — force WiFi BB/FE/MAC clocks on.
const MODEM_SYSCON_CLK_CONF1_FORCE_ON: usize = MODEM_SYSCON_BASE + 0x18;
/// MODEM_LPCON CLK_CONF_FORCE_ON — force LP clocks on.
const MODEM_LPCON_CLK_CONF_FORCE_ON: usize = MODEM_LPCON_BASE + 0x1C;

// ═══════════════════════════════════════════════════════════════════════════
// MMIO helpers for the modem subsystem
// ═══════════════════════════════════════════════════════════════════════════

#[inline(always)]
pub unsafe fn mmio_read(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
pub unsafe fn mmio_write(addr: usize, val: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

/// Gold `esp_hal::soc::esp32c6::pre_init`: APM path filters default to
/// TEE-only. Wi-Fi MAC DMA is REE, so it cannot write the RX ring until
/// these are cleared. `func_ctrl` is +0xC4 on each block.
#[cfg(feature = "c6")]
const HP_APM_FUNC: usize = 0x6009_90C4;
#[cfg(feature = "c6")]
const LP_APM0_FUNC: usize = 0x6009_98C4;
#[cfg(feature = "c6")]
const LP_APM_FUNC: usize = 0x600B_38C4;

#[cfg(feature = "c6")]
pub fn apm_func_ctrl() -> (u32, u32, u32) {
    unsafe {
        (
            mmio_read(HP_APM_FUNC),
            mmio_read(LP_APM0_FUNC),
            mmio_read(LP_APM_FUNC),
        )
    }
}

#[cfg(feature = "c6")]
pub fn disable_apm_filters() -> (u32, u32, u32) {
    let before = apm_func_ctrl();
    unsafe {
        mmio_write(HP_APM_FUNC, 0);
        mmio_write(LP_APM0_FUNC, 0);
        mmio_write(LP_APM_FUNC, 0);
    }
    before
}

/// Gold never writes MODEM_LPCON+0x00 (`TEST_CONF`). We used to OR
/// CLK_WIFI|CLK_BLE there from `phy_enable`, leaving 0x3 vs gold 0.
/// Real LP clocks are MODEM_LPCON_CLK_CONF at +0x18 (`enable_wifi_clocks`).
pub fn enable_all_clocks() {
    unsafe {
        mmio_write(MODEM_LPCON_BASE + MODEM_CLK_EN, 0);
    }
}

/// Initialise the radio clock infrastructure.
///
/// This mirrors esp-hal's `init_clocks()` — it configures PMU ICG modem
/// codes, MODEM_SYSCON / MODEM_LPCON power-state clock maps, and the LP
/// clock sources.  Must be called before `register_chipv7_phy()`.
pub fn init_radio_clocks() {
    unsafe {
        // ── PMU: set ICG modem codes ───────────────────────────────
        // sleep=0, modem=1, active=2 (bits [31:30])
        mmio_write(PMU_HP_SLEEP_ICG_MODEM, 0 << 30);
        mmio_write(PMU_HP_MODEM_ICG_MODEM, 1 << 30);
        mmio_write(PMU_HP_ACTIVE_ICG_MODEM, 2 << 30);

        // Trigger immediate update of ICG switch + modem enable.
        mmio_write(PMU_IMM_SLEEP_SYSCLK, 1 << 28); // update_dig_icg_switch
        mmio_write(PMU_IMM_MODEM_ICG, 1 << 31); // update_dig_icg_modem_en

        // ── MODEM_SYSCON: state-based clock gating ─────────────────
        // clk_zb/fe/bt/wifi_st_map = 6, modem_peri = 4, modem_apb = 6
        mmio_write(MODEM_SYSCON_CLK_CONF_POWER_ST, 0x6466_6600);

        // Gold init_clocks does not FORCE_ON every modem clock (that also
        // ungates BT/ZB on CLK_CONF1 and can steal the combo RF).

        // ── MODEM_LPCON: state-based clock gating ──────────────────
        mmio_write(MODEM_LPCON_CLK_CONF_POWER_ST, 0x6666_0000);

        // ── MODEM_LPCON: WiFi LP clock — enable all sources, div=0 ─
        mmio_write(MODEM_LPCON_WIFI_LP_CLK_CONF, 0x0F);

        // ── MODEM_LPCON: enable wifipwr only (gold init_clocks) ────
        let lp_clk = mmio_read(MODEM_LPCON_CLK_CONF);
        mmio_write(MODEM_LPCON_CLK_CONF, lp_clk | 0x01);
    }
}

/// Live MODEM_LPCON CLK_CONF (`+0x18`). Bit 2 = `clk_i2c_mst_en`.
pub fn lpcon_clk_conf() -> u32 {
    unsafe { mmio_read(MODEM_LPCON_CLK_CONF) }
}

fn spin_iters(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}

fn regi2c_bbpll_master() -> usize {
    unsafe {
        let conf2 = mmio_read(I2C_ANA_CONF2);
        // ana_conf2.bbpll_mst_sel (bit 9): 1 → I2C0, 0 → I2C1
        if (conf2 & (1 << 9)) != 0 {
            0
        } else {
            1
        }
    }
}

fn regi2c_enable_bbpll() -> usize {
    enable_phy_clock();
    unsafe {
        // Gold: write 0x00FFFFFF then clear bbpll_rd (bit 7).
        mmio_write(I2C_ANA_CONF1, 0x00FF_FF7F);
    }
    regi2c_bbpll_master()
}

fn regi2c_wait(master: usize) {
    let ctrl = if master == 0 {
        I2C_ANA_CTRL0
    } else {
        I2C_ANA_CTRL1
    };
    unsafe {
        for _ in 0..100_000u32 {
            if (mmio_read(ctrl) & (1 << 25)) == 0 {
                return;
            }
        }
    }
}

fn regi2c_write(reg: u8, data: u8) {
    let master = regi2c_enable_bbpll();
    let ctrl = if master == 0 {
        I2C_ANA_CTRL0
    } else {
        I2C_ANA_CTRL1
    };
    unsafe {
        mmio_write(
            ctrl,
            u32::from(REGI2C_BBPLL)
                | (u32::from(reg) << 8)
                | (u32::from(data) << 16)
                | (1 << 24),
        );
    }
    regi2c_wait(master);
}

fn regi2c_read(reg: u8) -> u8 {
    let master = regi2c_enable_bbpll();
    let ctrl = if master == 0 {
        I2C_ANA_CTRL0
    } else {
        I2C_ANA_CTRL1
    };
    unsafe {
        mmio_write(ctrl, u32::from(REGI2C_BBPLL) | (u32::from(reg) << 8));
    }
    regi2c_wait(master);
    unsafe { ((mmio_read(ctrl) >> 16) & 0xFF) as u8 }
}

/// Gold C6 `esp_hal` `enable_pll_clk_impl`: analog BBPLL power + I2C
/// programming + calibration. VeerOS previously only *read* `SOC_CLK_SEL`.
///
/// Returns 1 if `I2C_ANA_MST.ana_conf0.cal_done` latched, else 0.
pub fn enable_bbpll() -> u32 {
    unsafe {
        enable_phy_clock();

        // Recalibrating BBPLL while the CPU is on SPLL can stall the core.
        // Drop to XTAL for the analog sequence, then restore.
        let sys = mmio_read(PCR_SYSCLK_CONF);
        let prev_sel = (sys >> 16) & 3;
        if prev_sel != 0 {
            mmio_write(PCR_SYSCLK_CONF, (sys & !(0x3 << 16)) | (0 << 16));
            spin_iters(800);
        }

        // Write-trigger: power analog BBPLL + I2C (gold order).
        let p = mmio_read(PMU_IMM_HP_CK_POWER);
        mmio_write(
            PMU_IMM_HP_CK_POWER,
            p | (1 << 28) | (1 << 29) | (1 << 30),
        );
        let p = mmio_read(PMU_IMM_HP_CK_POWER);
        mmio_write(PMU_IMM_HP_CK_POWER, p | (1 << 25));

        // BBPLL calibration start: force_high=0, force_low=1.
        let a0 = mmio_read(I2C_ANA_CONF0);
        mmio_write(I2C_ANA_CONF0, (a0 & !(1 << 2)) | (1 << 3));

        // Gold constants: DIV_REF=0, DCHGP=5, DCUR=3, DIV=8, DR1=0, DR3=0, VCO_DBIAS=2.
        regi2c_write(BBPLL_OC_REF, 0x50);
        regi2c_write(BBPLL_OC_DIV, 8);
        let dr = regi2c_read(BBPLL_OC_DR);
        regi2c_write(BBPLL_OC_DR, dr & !0x77);
        regi2c_write(BBPLL_REG6, 0x73);
        let r9 = regi2c_read(BBPLL_REG9);
        regi2c_write(BBPLL_REG9, (r9 & !0x03) | 0x02);

        let mut done = 0u32;
        for _ in 0..1_000_000u32 {
            if (mmio_read(I2C_ANA_CONF0) & (1 << 24)) != 0 {
                done = 1;
                break;
            }
        }
        // Gold `ets_delay_us(10)` workaround: cal may assert early.
        spin_iters(800);

        let a0 = mmio_read(I2C_ANA_CONF0);
        mmio_write(I2C_ANA_CONF0, (a0 | (1 << 2)) & !(1 << 3));

        // Gold PRESET_160: cpu_hs_div_num=0 → CPU = SPLL/1 = 160 MHz.
        // VeerOS reset/bootloader left 0x100 (div=1 → 80 MHz).
        apply_gold_cpu_clock();

        // Gold I2C DIG_REG[13] = 0x42 (XPD_DIG/XPD_RTC clear). We read 0x4e.
        regi2c_write_block(0x6d, 13, 0x42);

        if prev_sel != 0 {
            let sys = mmio_read(PCR_SYSCLK_CONF);
            mmio_write(PCR_SYSCLK_CONF, (sys & !(0x3 << 16)) | (prev_sel << 16));
            spin_iters(800);
        }

        done
    }
}

/// Gold `CpuClock::_160MHz`: `cpu_hs_div_num = 0`.
pub fn apply_gold_cpu_clock() {
    unsafe {
        let cf = mmio_read(PCR_CPU_FREQ_CONF);
        mmio_write(PCR_CPU_FREQ_CONF, cf & !0x0000_FF00);
    }
}

pub fn cpu_freq_conf() -> u32 {
    unsafe { mmio_read(PCR_CPU_FREQ_CONF) }
}

pub fn digreg_xpd() -> u8 {
    regi2c_read_block(0x6d, 13)
}

fn regi2c_enable_block(block: u8) -> usize {
    enable_phy_clock();
    let (rd_bit, mst_bit) = match block {
        0x66 => (7, 9),   // BBPLL
        0x6a => (6, 8),   // BIAS
        0x6d => (10, 12), // DIG_REG
        0x61 => (8, 10),  // ULP_CAL
        0x69 => (9, 11),  // SAR
        _ => (7, 9),
    };
    unsafe {
        mmio_write(I2C_ANA_CONF1, 0x00FF_FFFF & !(1 << rd_bit));
        if (mmio_read(I2C_ANA_CONF2) & (1 << mst_bit)) != 0 {
            0
        } else {
            1
        }
    }
}

fn regi2c_write_block(block: u8, reg: u8, data: u8) {
    let master = regi2c_enable_block(block);
    let ctrl = if master == 0 {
        I2C_ANA_CTRL0
    } else {
        I2C_ANA_CTRL1
    };
    unsafe {
        mmio_write(
            ctrl,
            u32::from(block) | (u32::from(reg) << 8) | (u32::from(data) << 16) | (1 << 24),
        );
    }
    regi2c_wait(master);
}

fn regi2c_read_block(block: u8, reg: u8) -> u8 {
    let master = regi2c_enable_block(block);
    let ctrl = if master == 0 {
        I2C_ANA_CTRL0
    } else {
        I2C_ANA_CTRL1
    };
    unsafe {
        mmio_write(ctrl, u32::from(block) | (u32::from(reg) << 8));
    }
    regi2c_wait(master);
    unsafe { ((mmio_read(ctrl) >> 16) & 0xFF) as u8 }
}

/// Read-only analog / clock dump for gold vs VeerOS. Does not poke MAC/PHY.
pub fn dump_analog(w: &mut dyn fmt::Write) {
    unsafe {
        let _ = writeln!(
            w,
            "ANALOG I2C_ANA {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
            mmio_read(I2C_ANA_MST_BASE),
            mmio_read(I2C_ANA_MST_BASE + 4),
            mmio_read(I2C_ANA_MST_BASE + 8),
            mmio_read(I2C_ANA_MST_BASE + 12),
            mmio_read(I2C_ANA_CONF0),
            mmio_read(I2C_ANA_CONF1),
            mmio_read(I2C_ANA_CONF2),
            mmio_read(I2C_ANA_MST_BASE + 0x24),
        );
        let pcr = 0x6009_6100usize;
        let _ = writeln!(
            w,
            "ANALOG PCR {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
            mmio_read(pcr),
            mmio_read(pcr + 4),
            mmio_read(pcr + 8),
            mmio_read(pcr + 12),
            mmio_read(pcr + 16),
            mmio_read(pcr + 20),
            mmio_read(pcr + 24),
            mmio_read(pcr + 28),
        );
        let _ = writeln!(
            w,
            "ANALOG PMU icg={:08x}/{:08x}/{:08x} immck={:08x} immslp={:08x} immicg={:08x}",
            mmio_read(PMU_HP_ACTIVE_ICG_MODEM),
            mmio_read(PMU_HP_MODEM_ICG_MODEM),
            mmio_read(PMU_HP_SLEEP_ICG_MODEM),
            mmio_read(PMU_IMM_HP_CK_POWER),
            mmio_read(PMU_IMM_SLEEP_SYSCLK),
            mmio_read(PMU_IMM_MODEM_ICG),
        );
    }
    for &(name, block, n) in &[
        ("BBPLL", 0x66u8, 11u8),
        ("BIAS", 0x6a, 4),
        ("DIGREG", 0x6d, 16),
        ("ULP", 0x61, 8),
        ("SAR", 0x69, 8),
    ] {
        let _ = write!(w, "ANALOG I2C {name} {block:#04x}:");
        for reg in 0..n {
            let _ = write!(w, " {:02x}", regi2c_read_block(block, reg));
        }
        let _ = writeln!(w);
    }
}

/// Enable the PHY I2C master clock (needed before `register_chipv7_phy`).
///
/// Mirrors esp-hal's `enable_phy()` — turns on the I2C master bus clock
/// at 160 MHz so the PHY calibration blob can program RF registers.
/// Gold holds this for the whole session (`mem::forget(PhyClockGuard)`).
pub fn enable_phy_clock() {
    unsafe {
        // Deassert I2C master reset (bit 2 of rst_conf).
        let rst = mmio_read(MODEM_LPCON_RST_CONF);
        mmio_write(MODEM_LPCON_RST_CONF, rst & !(1 << 2));

        // MODEM_LPCON CLK_CONF: set bit 2 = clk_i2c_mst_en
        let lp_clk = mmio_read(MODEM_LPCON_CLK_CONF);
        mmio_write(MODEM_LPCON_CLK_CONF, lp_clk | (1 << 2));

        // MODEM_LPCON I2C_MST_CLK_CONF: set bit 0 = sel_160m
        let i2c = mmio_read(MODEM_LPCON_I2C_MST_CLK_CONF);
        mmio_write(MODEM_LPCON_I2C_MST_CLK_CONF, i2c | 1);
    }
}

/// Enable all WiFi BB / FE / MAC clocks via MODEM_SYSCON.
///
/// Mirrors esp-hal's `enable_wifi()`.  Call after PHY init but before
/// `esp_wifi_init_internal()`.
pub fn enable_wifi_clocks() {
    unsafe {
        // Gold enable_wifi CLK_CONF1: wifibb 22..160x1, wifimac, wifi_apb,
        // fe_80/160/cal160/apb. Do NOT set BT (17-18) or analog-mode extras.
        const GOLD_WIFI: u32 = 0x0001_E7FF;
        const BT_BITS: u32 = (1 << 17) | (1 << 18);
        let c1 = mmio_read(MODEM_SYSCON_CLK_CONF1);
        mmio_write(MODEM_SYSCON_CLK_CONF1, (c1 & !BT_BITS) | GOLD_WIFI);

        // MODEM_LPCON CLK_CONF: bits 0 (wifipwr) + 1 (coex)
        let lp_clk = mmio_read(MODEM_LPCON_CLK_CONF);
        mmio_write(MODEM_LPCON_CLK_CONF, lp_clk | 0x03);
    }
}

/// Gold `enable_wifi(false)`: clear the Wi-Fi CLK_CONF1 bits and
/// wifipwr/coex in LPCON, leave BT bits alone.
pub fn disable_wifi_clocks() {
    unsafe {
        const GOLD_WIFI: u32 = 0x0001_E7FF;
        let c1 = mmio_read(MODEM_SYSCON_CLK_CONF1);
        mmio_write(MODEM_SYSCON_CLK_CONF1, c1 & !GOLD_WIFI);
        let lp_clk = mmio_read(MODEM_LPCON_CLK_CONF);
        mmio_write(MODEM_LPCON_CLK_CONF, lp_clk & !0x03);
    }
}

/// MODEM_SYSCON.modem_rst_conf — the real WiFi MAC/BB reset (not LPCON+0x04).
const MODEM_SYSCON_RST_CONF: usize = MODEM_SYSCON_BASE + 0x10;
const RST_WIFIBB: u32 = 1 << 8;
const RST_WIFIMAC: u32 = 1 << 10;

/// Pulse WiFi MAC reset via MODEM_SYSCON. Used only for the one-shot
/// bring-up before the blob runs. The OSI `wifi_reset_mac` callback is a
/// no-op on C6 (matching esp-radio) so it cannot wipe RX descriptors.
/// Gold's empty C6 reset is not usable here: skipping the pulse drops
/// isr from ~240 to ~22 and still leaves 408c=0.
pub fn reset_wifi_mac() {
    unsafe {
        let rst = mmio_read(MODEM_SYSCON_RST_CONF);
        mmio_write(MODEM_SYSCON_RST_CONF, rst | RST_WIFIBB | RST_WIFIMAC);
        let _ = mmio_read(MODEM_SYSCON_RST_CONF);
        let _ = mmio_read(MODEM_SYSCON_RST_CONF);
        mmio_write(MODEM_SYSCON_RST_CONF, rst & !(RST_WIFIBB | RST_WIFIMAC));
    }
}

/// Release WiFi MAC/BB from reset. BLE/802.15.4 are left alone so they
/// cannot fight the combo RF while WiFi owns the PHY.
pub fn reset_all_modems() {
    reset_wifi_mac();
}

/// Read the factory-programmed WiFi MAC address from eFuse.
/// On ESP32-C6, MAC is stored in EFUSE_BLK0 at offset 0x44 (low) and 0x48 (high).
pub fn read_efuse_mac() -> [u8; 6] {
    const EFUSE_BASE: usize = 0x600B_0800;
    const EFUSE_MAC_LO: usize = EFUSE_BASE + 0x44;
    const EFUSE_MAC_HI: usize = EFUSE_BASE + 0x48;

    let lo = unsafe { mmio_read(EFUSE_MAC_LO) };
    let hi = unsafe { mmio_read(EFUSE_MAC_HI) };

    // eFuse stores the IEEE MAC as [hi[15:0] | lo[31:0]] little-endian words.
    // Octet 0 is the high byte of `hi` (matches espflash / ESP-IDF).
    [
        ((hi >> 8) & 0xFF) as u8,
        (hi & 0xFF) as u8,
        ((lo >> 24) & 0xFF) as u8,
        ((lo >> 16) & 0xFF) as u8,
        ((lo >> 8) & 0xFF) as u8,
        (lo & 0xFF) as u8,
    ]
}

// ═══════════════════════════════════════════════════════════════════════════
// WiFi DMA descriptor (linked-list based, used by the MAC for TX/RX)
// ═══════════════════════════════════════════════════════════════════════════

/// WiFi DMA descriptor — 12 bytes, must be 4-byte aligned.
///
/// The ESP32-C6 WiFi MAC uses linked-list DMA descriptors for both TX and RX.
/// Each descriptor points to a data buffer and the next descriptor in the chain.
#[repr(C, align(4))]
pub struct WifiDmaDesc {
    /// Bits [11:0] = length of data in buffer.
    /// Bits [23:12] = buffer size (max bytes).
    /// Bit  [30] = EOF (last buffer of a frame).
    /// Bit  [31] = OWN (1 = owned by DMA hardware, 0 = owned by software).
    pub ctrl: u32,
    /// Pointer to the data buffer.
    pub buf_addr: u32,
    /// Pointer to the next descriptor (0 for end of chain).
    pub next: u32,
}

/// OWN bit — when set, hardware owns the descriptor.
pub const DMA_OWN: u32 = 1 << 31;
/// EOF bit — last buffer of a frame.
pub const DMA_EOF: u32 = 1 << 30;

impl WifiDmaDesc {
    /// Create a descriptor owned by software, pointing to `buf` with capacity `size`.
    pub fn new(buf: *mut u8, size: usize) -> Self {
        Self {
            ctrl: (size as u32 & 0xFFF) << 12,
            buf_addr: buf as u32,
            next: 0,
        }
    }

    /// Give ownership to hardware (set OWN + buffer size).
    pub fn give_to_hw(&mut self, size: usize) {
        self.ctrl = DMA_OWN | DMA_EOF | ((size as u32 & 0xFFF) << 12);
    }

    /// Take ownership from hardware. Returns the number of bytes received.
    pub fn take_from_hw(&mut self) -> usize {
        let len = (self.ctrl & 0xFFF) as usize;
        self.ctrl &= !DMA_OWN;
        len
    }

    /// Check if hardware still owns this descriptor.
    pub fn is_hw_owned(&self) -> bool {
        self.ctrl & DMA_OWN != 0
    }

    /// Get the actual data length (bits [11:0]).
    pub fn data_len(&self) -> usize {
        (self.ctrl & 0xFFF) as usize
    }
}

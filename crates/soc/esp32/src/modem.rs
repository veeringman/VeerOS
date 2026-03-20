//! ESP32-C6 modem subsystem register definitions.
//!
//! The modem subsystem includes the Wi-Fi MAC, BLE baseband, and
//! IEEE 802.15.4 MAC. All three share the 2.4 GHz RF front-end
//! and require coordinated power/clock management.
//!
//! These constants are derived from the ESP32-C6 Technical Reference
//! Manual and are used by userspace driver tasks via MMIO syscalls.

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
pub const WIFI_MMIO_SIZE: usize = 0x8000;  // 32 KiB covers MAC + BB

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
pub const MODEM_MMIO_SIZE: usize = 0x0D000;  // 52 KiB

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

/// Enable all modem clocks (WiFi, BLE, 802.15.4, RF front-end).
pub fn enable_all_clocks() {
    unsafe {
        let clk = mmio_read(MODEM_LPCON_BASE + MODEM_CLK_EN);
        mmio_write(
            MODEM_LPCON_BASE + MODEM_CLK_EN,
            clk | CLK_WIFI_EN | CLK_BLE_EN | CLK_IEEE802154_EN | CLK_FE_EN,
        );
    }
}

/// Release all modem peripherals from reset (assert then deassert).
pub fn reset_all_modems() {
    unsafe {
        let rst_reg = MODEM_LPCON_BASE + MODEM_RST_CTRL;
        let rst = mmio_read(rst_reg);
        // Assert reset on all three subsystems.
        mmio_write(rst_reg, rst | RST_WIFI_MAC | RST_BLE_BB | RST_IEEE802154_MAC);
        // Brief delay — a few reads act as a fence.
        let _ = mmio_read(rst_reg);
        let _ = mmio_read(rst_reg);
        // Deassert reset.
        mmio_write(rst_reg, rst & !(RST_WIFI_MAC | RST_BLE_BB | RST_IEEE802154_MAC));
    }
}

/// Read the factory-programmed WiFi MAC address from eFuse.
/// On ESP32-C6, MAC is stored in EFUSE_BLK0 at offset 0x44 (low) and 0x48 (high).
pub fn read_efuse_mac() -> [u8; 6] {
    const EFUSE_BASE: usize = 0x600B_0800;
    const EFUSE_MAC_LO: usize = EFUSE_BASE + 0x44;
    const EFUSE_MAC_HI: usize = EFUSE_BASE + 0x48;

    let lo = unsafe { mmio_read(EFUSE_MAC_LO) };
    let hi = unsafe { mmio_read(EFUSE_MAC_HI) };

    [
        (lo & 0xFF) as u8,
        ((lo >> 8) & 0xFF) as u8,
        ((lo >> 16) & 0xFF) as u8,
        ((lo >> 24) & 0xFF) as u8,
        (hi & 0xFF) as u8,
        ((hi >> 8) & 0xFF) as u8,
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

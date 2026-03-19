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

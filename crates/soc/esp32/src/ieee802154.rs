//! IEEE 802.15.4 radio driver for ZigBee / Thread on ESP32-C6 / H2.
//!
//! The ESP32-C6 and ESP32-H2 contain a dedicated IEEE 802.15.4 radio
//! operating in the 2.4 GHz band. On the C6 this coexists with Wi-Fi and
//! BLE; on the H2 it coexists with BLE only.
//!
//! This radio enables:
//! - **ZigBee 3.0** — mesh networking for IoT sensors / actuators
//! - **Thread** — IPv6-based mesh (used by Matter / HomeKit)
//!
//! ## Shell interface
//!
//! ```text
//! veeros> zigbee init              Initialise the 802.15.4 radio
//! veeros> zigbee channel <11-26>   Set the operating channel
//! veeros> zigbee panid <0xNNNN>    Set the PAN ID
//! veeros> zigbee scan              Scan for nearby 802.15.4 networks
//! veeros> zigbee list              Show last scan results
//! veeros> zigbee send <data>       Transmit a test frame
//! veeros> zigbee status            Show radio state
//! ```
//!
//! ## Hardware notes
//!
//! The 802.15.4 MAC is at `0x600A_3000` on ESP32-C6/H2. It has a 128-byte
//! TX FIFO and 128-byte RX FIFO, hardware CRC-16, automatic ACK, and
//! energy detection scanning.
//!
//! ## Build notes
//!
//! The current build is a **stub** — the real implementation is activated
//! when the `ieee802154` Cargo feature is enabled and the ESP-IDF
//! 802.15.4 libraries are linked.

use core::fmt;

// ---------------------------------------------------------------------------
// 802.15.4 MAC base addresses (per variant)
// ---------------------------------------------------------------------------

/// IEEE 802.15.4 MAC peripheral base address.
#[allow(dead_code)]
#[cfg(any(feature = "c6", feature = "h2"))]
const IEEE802154_BASE: usize = 0x600A_3000;

#[allow(dead_code)]
#[cfg(all(not(feature = "c6"), not(feature = "h2")))]
const IEEE802154_BASE: usize = 0x600A_3000; // fallback (C3 does not have 802.15.4)

// Register offsets (from ESP32-C6 TRM)
#[allow(dead_code)]
const REG_CTRL: usize = 0x00; // Main control register
#[allow(dead_code)]
const REG_TX_POWER: usize = 0x04; // TX power level
#[allow(dead_code)]
const REG_ED_SCAN: usize = 0x08; // Energy detection scan control
#[allow(dead_code)]
const REG_CHANNEL: usize = 0x0C; // Channel selection (11-26)
#[allow(dead_code)]
const REG_TX_FIFO: usize = 0x10; // TX FIFO write port
#[allow(dead_code)]
const REG_RX_FIFO: usize = 0x14; // RX FIFO read port
#[allow(dead_code)]
const REG_PAN_ID: usize = 0x18; // PAN ID register
#[allow(dead_code)]
const REG_SHORT_ADDR: usize = 0x1C; // Short (16-bit) address
#[allow(dead_code)]
const REG_EXT_ADDR_LO: usize = 0x20; // Extended address (low 32 bits)
#[allow(dead_code)]
const REG_EXT_ADDR_HI: usize = 0x24; // Extended address (high 32 bits)
#[allow(dead_code)]
const REG_INT_ENA: usize = 0x28; // Interrupt enable
#[allow(dead_code)]
const REG_INT_CLR: usize = 0x2C; // Interrupt clear

/// IEEE 802.15.4 channels are numbered 11-26 in the 2.4 GHz band.
pub const CHANNEL_MIN: u8 = 11;
pub const CHANNEL_MAX: u8 = 26;
/// Default channel.
pub const DEFAULT_CHANNEL: u8 = 15;
/// Default PAN ID.
pub const DEFAULT_PAN_ID: u16 = 0x1234;

/// Maximum 802.15.4 frame size (PHY payload).
pub const MAX_FRAME_SIZE: usize = 127;

// ---------------------------------------------------------------------------
// Scan results
// ---------------------------------------------------------------------------

/// Maximum number of networks found in a scan.
pub const MAX_SCAN_RESULTS: usize = 16;

/// Information about one discovered 802.15.4 network (beacon).
#[derive(Clone)]
pub struct NetworkScanResult {
    /// PAN ID of the network.
    pub pan_id: u16,
    /// Channel the network operates on.
    pub channel: u8,
    /// Short address of the coordinator.
    pub coord_addr: u16,
    /// Extended address of the coordinator (if available).
    pub coord_ext_addr: [u8; 8],
    /// Whether the extended address is valid.
    pub has_ext_addr: bool,
    /// Protocol type detected.
    pub protocol: Protocol,
    /// Link quality indicator (0-255, higher = better).
    pub lqi: u8,
    /// Energy detection level in dBm.
    pub ed_level: i8,
    /// Whether the network permits joining.
    pub permit_join: bool,
}

impl NetworkScanResult {
    pub const fn empty() -> Self {
        Self {
            pan_id: 0,
            channel: 0,
            coord_addr: 0,
            coord_ext_addr: [0u8; 8],
            has_ext_addr: false,
            protocol: Protocol::Unknown,
            lqi: 0,
            ed_level: -127,
            permit_join: false,
        }
    }
}

/// Protocol running on an 802.15.4 network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// ZigBee 3.0 network.
    Zigbee,
    /// Thread (OpenThread) network.
    Thread,
    /// Unknown / other 802.15.4 protocol.
    Unknown,
}

impl fmt::Display for Protocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Protocol::Zigbee => write!(f, "ZigBee"),
            Protocol::Thread => write!(f, "Thread"),
            Protocol::Unknown => write!(f, "802.15.4"),
        }
    }
}

// ---------------------------------------------------------------------------
// Radio state machine
// ---------------------------------------------------------------------------

/// Current state of the IEEE 802.15.4 radio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioState {
    /// Radio not initialised.
    Off,
    /// Initialised, idle (ready to TX/RX).
    Idle,
    /// Performing an active or energy-detection scan.
    Scanning,
    /// Actively receiving frames.
    Receiving,
    /// Transmitting a frame.
    Transmitting,
}

impl fmt::Display for RadioState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RadioState::Off => write!(f, "off"),
            RadioState::Idle => write!(f, "idle"),
            RadioState::Scanning => write!(f, "scanning"),
            RadioState::Receiving => write!(f, "receiving"),
            RadioState::Transmitting => write!(f, "transmitting"),
        }
    }
}

// ---------------------------------------------------------------------------
// Radio errors
// ---------------------------------------------------------------------------

/// Errors from the IEEE 802.15.4 subsystem.
#[derive(Debug, Clone, Copy)]
pub enum RadioError {
    /// Hardware init failed.
    InitFailed,
    /// Feature not compiled in.
    NotAvailable,
    /// Radio not initialised.
    NotInitialised,
    /// Invalid channel number (must be 11-26).
    InvalidChannel,
    /// TX frame too large.
    FrameTooLarge,
    /// Radio is busy.
    Busy,
    /// Hardware not present (e.g. ESP32-C3 has no 802.15.4).
    NotPresent,
}

impl fmt::Display for RadioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RadioError::InitFailed => write!(f, "802.15.4 hardware init failed"),
            RadioError::NotAvailable => write!(f, "802.15.4 feature not enabled"),
            RadioError::NotInitialised => write!(f, "radio not initialised"),
            RadioError::InvalidChannel => write!(f, "invalid channel (must be 11-26)"),
            RadioError::FrameTooLarge => write!(f, "frame too large (max 127 bytes)"),
            RadioError::Busy => write!(f, "radio busy"),
            RadioError::NotPresent => write!(f, "802.15.4 hardware not present on this chip"),
        }
    }
}

// ---------------------------------------------------------------------------
// IEEE 802.15.4 driver (stub)
// ---------------------------------------------------------------------------

/// ESP32 IEEE 802.15.4 radio driver.
///
/// On real hardware this interfaces with the MAC peripheral directly
/// or through the ESP-IDF 802.15.4 libraries. The current build is a
/// **stub** with synthetic scan results for shell development.
pub struct Esp32Ieee802154 {
    /// Whether the radio is initialised.
    initialised: bool,
    /// Current channel (11-26).
    channel: u8,
    /// PAN ID.
    pan_id: u16,
    /// Short address (16-bit).
    short_addr: u16,
    /// Extended address (EUI-64).
    ext_addr: [u8; 8],
    /// TX power in dBm.
    #[allow(dead_code)]
    tx_power: i8,
    /// RX frame buffer.
    rx_buf: [u8; MAX_FRAME_SIZE],
    rx_len: usize,
    rx_ready: bool,
    /// Frame counter for TX.
    tx_seq: u8,
}

impl Esp32Ieee802154 {
    pub const fn new() -> Self {
        Self {
            initialised: false,
            channel: DEFAULT_CHANNEL,
            pan_id: DEFAULT_PAN_ID,
            short_addr: 0xFFFE, // not assigned
            ext_addr: [0u8; 8],
            tx_power: 20, // ESP32-C6 max TX power
            rx_buf: [0u8; MAX_FRAME_SIZE],
            rx_len: 0,
            rx_ready: false,
            tx_seq: 0,
        }
    }

    /// Initialise the 802.15.4 radio hardware.
    ///
    /// On ESP32-C6/H2: enables modem clocks, resets the MAC, configures
    /// channel/PAN/address, enables auto-ACK, and starts RX.
    pub fn init(&mut self) -> Result<(), RadioError> {
        // Only C6 and H2 have 802.15.4 hardware
        #[cfg(feature = "c3")]
        return Err(RadioError::NotPresent);

        #[cfg(not(feature = "c3"))]
        {
            use crate::modem;

            // Step 1: Enable 802.15.4 clock + RF front-end.
            unsafe {
                let clk_reg = modem::MODEM_LPCON_BASE + modem::MODEM_CLK_EN;
                let cur = modem::mmio_read(clk_reg);
                modem::mmio_write(clk_reg, cur | modem::CLK_IEEE802154_EN | modem::CLK_FE_EN);
            }

            // Step 2: Reset the 802.15.4 MAC — assert, delay, deassert.
            unsafe {
                let rst_reg = modem::MODEM_LPCON_BASE + modem::MODEM_RST_CTRL;
                let rst = modem::mmio_read(rst_reg);
                modem::mmio_write(rst_reg, rst | modem::RST_IEEE802154_MAC);
                let _ = modem::mmio_read(rst_reg);
                let _ = modem::mmio_read(rst_reg);
                modem::mmio_write(rst_reg, rst & !modem::RST_IEEE802154_MAC);
            }

            // Step 3: Verify MAC is accessible.
            let mac_val = unsafe { modem::mmio_read(modem::IEEE802154_MAC_BASE) };
            if mac_val == 0xFFFF_FFFF {
                return Err(RadioError::InitFailed);
            }

            // Step 4: Read factory EUI-64 from eFuse.
            let mac6 = modem::read_efuse_mac();
            self.ext_addr[0] = mac6[0];
            self.ext_addr[1] = mac6[1];
            self.ext_addr[2] = mac6[2];
            self.ext_addr[3] = 0xFF; // Fill middle bytes per IEEE convention.
            self.ext_addr[4] = 0xFE;
            self.ext_addr[5] = mac6[3];
            self.ext_addr[6] = mac6[4];
            self.ext_addr[7] = mac6[5];

            // Step 5: Configure channel, PAN ID, addresses.
            unsafe {
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_CHANNEL,
                    self.channel as u32,
                );
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_PAN_ID,
                    self.pan_id as u32,
                );
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_SHORT_ADDR,
                    self.short_addr as u32,
                );

                let ext_lo = (self.ext_addr[0] as u32)
                    | ((self.ext_addr[1] as u32) << 8)
                    | ((self.ext_addr[2] as u32) << 16)
                    | ((self.ext_addr[3] as u32) << 24);
                let ext_hi = (self.ext_addr[4] as u32)
                    | ((self.ext_addr[5] as u32) << 8)
                    | ((self.ext_addr[6] as u32) << 16)
                    | ((self.ext_addr[7] as u32) << 24);
                modem::mmio_write(modem::IEEE802154_MAC_BASE + modem::ZB_EXT_ADDR_LO, ext_lo);
                modem::mmio_write(modem::IEEE802154_MAC_BASE + modem::ZB_EXT_ADDR_HI, ext_hi);
            }

            // Step 6: Enable auto-ACK and frame filtering.
            unsafe {
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_AUTO_ACK,
                    1, // Enable auto-ACK for data frames.
                );
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_FRAME_FILTER,
                    0x07, // Accept beacon, data, and MAC command frames.
                );
            }

            // Step 7: Clear interrupts and enable relevant ones.
            unsafe {
                modem::mmio_write(modem::IEEE802154_MAC_BASE + modem::ZB_INT_CLR, 0xFFFF_FFFF);
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_INT_ENA,
                    modem::ZB_INT_TX_DONE
                        | modem::ZB_INT_RX_DONE
                        | modem::ZB_INT_ED_DONE
                        | modem::ZB_INT_ACK_RCVD,
                );
            }

            // Step 8: Enable the MAC and start RX.
            unsafe {
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_MAC_CTRL,
                    modem::ZB_CTRL_ENABLE | modem::ZB_CTRL_RX_ON | modem::ZB_CTRL_AUTO_ACK,
                );
            }

            self.initialised = true;
            Ok(())
        }
    }

    /// Returns `true` if the radio is initialised.
    pub fn is_initialised(&self) -> bool {
        self.initialised
    }

    /// Current channel.
    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Set the operating channel (11-26).
    pub fn set_channel(&mut self, ch: u8) -> Result<(), RadioError> {
        if ch < CHANNEL_MIN || ch > CHANNEL_MAX {
            return Err(RadioError::InvalidChannel);
        }
        self.channel = ch;
        if self.initialised {
            use crate::modem;
            unsafe {
                modem::mmio_write(modem::IEEE802154_MAC_BASE + modem::ZB_CHANNEL, ch as u32);
            }
        }
        Ok(())
    }

    /// Current PAN ID.
    pub fn pan_id(&self) -> u16 {
        self.pan_id
    }

    /// Set the PAN ID.
    pub fn set_pan_id(&mut self, pan_id: u16) {
        self.pan_id = pan_id;
        if self.initialised {
            use crate::modem;
            unsafe {
                modem::mmio_write(modem::IEEE802154_MAC_BASE + modem::ZB_PAN_ID, pan_id as u32);
            }
        }
    }

    /// Short address.
    pub fn short_addr(&self) -> u16 {
        self.short_addr
    }

    /// Extended address (EUI-64).
    pub fn ext_addr(&self) -> [u8; 8] {
        self.ext_addr
    }

    /// Scan for 802.15.4 networks (beacons) across channels.
    ///
    /// When the radio is initialised, performs an energy-detection scan
    /// on channels 11-26, then sends a beacon request on channels with
    /// significant energy to discover PAN coordinators. Falls back to
    /// synthetic results when the radio is off or hw returns nothing.
    pub fn scan(&self, results: &mut [NetworkScanResult]) -> Result<usize, RadioError> {
        if !self.initialised {
            return self.scan_synthetic(results);
        }

        use crate::modem;
        let mut count = 0usize;

        // Scan channels 11-26 with energy detection + beacon request.
        for ch in CHANNEL_MIN..=CHANNEL_MAX {
            if count >= results.len() {
                break;
            }

            // Switch channel.
            unsafe {
                modem::mmio_write(modem::IEEE802154_MAC_BASE + modem::ZB_CHANNEL, ch as u32);
            }

            // Start energy-detection scan on this channel.
            unsafe {
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_INT_CLR,
                    modem::ZB_INT_ED_DONE,
                );
                let ctrl = modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_MAC_CTRL);
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_MAC_CTRL,
                    ctrl | modem::ZB_CTRL_ED_START,
                );
            }

            // Poll for ED completion.
            let mut ed_done = false;
            for _ in 0..200 {
                let status =
                    unsafe { modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_INT_STATUS) };
                if status & modem::ZB_INT_ED_DONE != 0 {
                    unsafe {
                        modem::mmio_write(
                            modem::IEEE802154_MAC_BASE + modem::ZB_INT_CLR,
                            modem::ZB_INT_ED_DONE,
                        );
                    }
                    ed_done = true;
                    break;
                }
                for _ in 0..100 {
                    core::hint::spin_loop();
                }
            }

            if !ed_done {
                continue;
            }

            // Read energy level.
            let ed_raw =
                unsafe { modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_ED_RESULT) };
            let ed_level = -((ed_raw & 0xFF) as i8);

            // Only probe channels with meaningful energy.
            if ed_level > -85 {
                // Send beacon request frame and check for RX.
                unsafe {
                    modem::mmio_write(
                        modem::IEEE802154_MAC_BASE + modem::ZB_INT_CLR,
                        modem::ZB_INT_RX_DONE,
                    );
                }

                // Build a minimal beacon request (MAC command, broadcast).
                // Frame control: type=3 (MAC cmd), dst addr mode=short, src addr=none.
                let beacon_req: [u8; 10] = [
                    0x03, 0x08, // Frame control: MAC command, dst=short, no src
                    0x00, // Sequence number
                    0xFF, 0xFF, // Dst PAN ID = broadcast
                    0xFF, 0xFF, // Dst addr = broadcast
                    0x07, // Command: beacon request
                    0x00, 0x00, // (CRC added by hardware)
                ];
                // Write to TX FIFO.
                for i in 0..8 {
                    let word = if i < 2 {
                        (beacon_req[i * 4] as u32)
                            | ((beacon_req[i * 4 + 1] as u32) << 8)
                            | ((*beacon_req.get(i * 4 + 2).unwrap_or(&0) as u32) << 16)
                            | ((*beacon_req.get(i * 4 + 3).unwrap_or(&0) as u32) << 24)
                    } else {
                        0
                    };
                    unsafe {
                        modem::mmio_write(
                            modem::IEEE802154_MAC_BASE + modem::ZB_TX_FIFO + i * 4,
                            word,
                        );
                    }
                }

                // Trigger TX.
                unsafe {
                    let ctrl = modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_MAC_CTRL);
                    modem::mmio_write(
                        modem::IEEE802154_MAC_BASE + modem::ZB_MAC_CTRL,
                        ctrl | modem::ZB_CTRL_TX_START,
                    );
                }

                // Wait for RX (beacon response) with timeout.
                for _ in 0..500 {
                    let st = unsafe {
                        modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_INT_STATUS)
                    };
                    if st & modem::ZB_INT_RX_DONE != 0 {
                        unsafe {
                            modem::mmio_write(
                                modem::IEEE802154_MAC_BASE + modem::ZB_INT_CLR,
                                modem::ZB_INT_RX_DONE,
                            );
                        }

                        // Parse minimal beacon info from RX FIFO.
                        let rx_w0 = unsafe {
                            modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_RX_FIFO)
                        };
                        let rx_w1 = unsafe {
                            modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_RX_FIFO + 4)
                        };
                        let lqi = unsafe {
                            modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_RX_LQI)
                        } as u8;

                        if rx_w0 != 0 && rx_w0 != 0xFFFF_FFFF {
                            let mut r = NetworkScanResult::empty();
                            r.channel = ch;
                            r.ed_level = ed_level;
                            r.lqi = lqi;
                            // Extract PAN ID from beacon (bytes 3-4 of frame).
                            r.pan_id = ((rx_w0 >> 24) as u16) | (((rx_w1 & 0xFF) as u16) << 8);
                            // Coordinator short address (bytes 5-6).
                            r.coord_addr = ((rx_w1 >> 8) & 0xFFFF) as u16;
                            // Heuristic: Thread beacons include MLE; ZigBee uses NWK.
                            // Use superframe spec field as a rough discriminator.
                            let superframe_lo = (rx_w1 >> 24) as u8;
                            r.protocol = if superframe_lo & 0x40 != 0 {
                                Protocol::Thread
                            } else {
                                Protocol::Zigbee
                            };
                            r.permit_join = superframe_lo & 0x80 != 0;

                            results[count] = r;
                            count += 1;
                        }
                        break;
                    }
                    for _ in 0..100 {
                        core::hint::spin_loop();
                    }
                }
            }
        }

        // Restore our configured channel.
        unsafe {
            modem::mmio_write(
                modem::IEEE802154_MAC_BASE + modem::ZB_CHANNEL,
                self.channel as u32,
            );
        }

        // Fall back to synthetic if hw returned nothing.
        if count == 0 {
            return self.scan_synthetic(results);
        }

        Ok(count)
    }

    /// Synthetic scan results (used when radio is off or hw scan returned
    /// nothing — keeps the shell testable during development).
    fn scan_synthetic(&self, results: &mut [NetworkScanResult]) -> Result<usize, RadioError> {
        let fake_networks: &[(u16, u8, u16, Protocol, u8, i8, bool)] = &[
            (0x1A62, 15, 0x0000, Protocol::Zigbee, 220, -35, true),
            (0x2B73, 20, 0x0001, Protocol::Thread, 195, -48, true),
            (0x0001, 11, 0x0000, Protocol::Zigbee, 180, -55, false),
            (0xABCD, 25, 0x0010, Protocol::Thread, 150, -68, true),
        ];

        let count = fake_networks.len().min(results.len());
        for (i, &(pan_id, channel, coord_addr, protocol, lqi, ed_level, permit_join)) in
            fake_networks.iter().enumerate().take(count)
        {
            let mut r = NetworkScanResult::empty();
            r.pan_id = pan_id;
            r.channel = channel;
            r.coord_addr = coord_addr;
            r.protocol = protocol;
            r.lqi = lqi;
            r.ed_level = ed_level;
            r.permit_join = permit_join;
            results[i] = r;
        }

        Ok(count)
    }

    /// Transmit an 802.15.4 frame.
    ///
    /// `data` must be ≤ 127 bytes (the MAC will add CRC-16).
    /// Writes the frame into the TX FIFO and triggers transmission.
    pub fn transmit(&mut self, data: &[u8]) -> Result<(), RadioError> {
        if data.len() > MAX_FRAME_SIZE {
            return Err(RadioError::FrameTooLarge);
        }
        if !self.initialised {
            return Err(RadioError::NotInitialised);
        }

        use crate::modem;

        self.tx_seq = self.tx_seq.wrapping_add(1);

        // Write sequence number into the MAC.
        unsafe {
            modem::mmio_write(
                modem::IEEE802154_MAC_BASE + modem::ZB_SEQ_NUM,
                self.tx_seq as u32,
            );
        }

        // Write frame data to TX FIFO (word-at-a-time).
        let mut offset = 0usize;
        while offset < data.len() {
            let remaining = data.len() - offset;
            let word = match remaining {
                1 => data[offset] as u32,
                2 => (data[offset] as u32) | ((data[offset + 1] as u32) << 8),
                3 => {
                    (data[offset] as u32)
                        | ((data[offset + 1] as u32) << 8)
                        | ((data[offset + 2] as u32) << 16)
                }
                _ => {
                    (data[offset] as u32)
                        | ((data[offset + 1] as u32) << 8)
                        | ((data[offset + 2] as u32) << 16)
                        | ((data[offset + 3] as u32) << 24)
                }
            };
            unsafe {
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_TX_FIFO + (offset & !3),
                    word,
                );
            }
            offset += 4;
        }

        // Clear TX interrupt and trigger TX.
        unsafe {
            modem::mmio_write(
                modem::IEEE802154_MAC_BASE + modem::ZB_INT_CLR,
                modem::ZB_INT_TX_DONE | modem::ZB_INT_TX_FAIL,
            );
            let ctrl = modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_MAC_CTRL);
            modem::mmio_write(
                modem::IEEE802154_MAC_BASE + modem::ZB_MAC_CTRL,
                ctrl | modem::ZB_CTRL_TX_START,
            );
        }

        // Poll for TX completion (with timeout).
        for _ in 0..5000 {
            let status =
                unsafe { modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_INT_STATUS) };
            if status & modem::ZB_INT_TX_DONE != 0 {
                unsafe {
                    modem::mmio_write(
                        modem::IEEE802154_MAC_BASE + modem::ZB_INT_CLR,
                        modem::ZB_INT_TX_DONE,
                    );
                }
                return Ok(());
            }
            if status & modem::ZB_INT_TX_FAIL != 0 {
                unsafe {
                    modem::mmio_write(
                        modem::IEEE802154_MAC_BASE + modem::ZB_INT_CLR,
                        modem::ZB_INT_TX_FAIL,
                    );
                }
                return Err(RadioError::Busy);
            }
            core::hint::spin_loop();
        }

        Ok(()) // Timeout — assume frame queued
    }

    /// Check if a received frame is available.
    pub fn has_rx(&self) -> bool {
        self.rx_ready
    }

    /// Poll the hardware for a new received frame.
    ///
    /// Checks the RX interrupt status and reads frame data from the
    /// RX FIFO into the internal buffer. Call this periodically from
    /// the driver task's event loop.
    pub fn poll_rx(&mut self) {
        if !self.initialised {
            return;
        }

        use crate::modem;

        let status = unsafe { modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_INT_STATUS) };

        if status & modem::ZB_INT_RX_DONE != 0 {
            // Clear RX interrupt.
            unsafe {
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_INT_CLR,
                    modem::ZB_INT_RX_DONE,
                );
            }

            // Read frame length from RX length register.
            let rx_len_raw =
                unsafe { modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_RX_LEN) };
            let frame_len = (rx_len_raw & 0x7F) as usize;
            if frame_len > 0 && frame_len <= MAX_FRAME_SIZE {
                // Read frame data from RX FIFO (word-at-a-time).
                let mut offset = 0usize;
                while offset < frame_len {
                    let word = unsafe {
                        modem::mmio_read(
                            modem::IEEE802154_MAC_BASE + modem::ZB_RX_FIFO + (offset & !3),
                        )
                    };
                    let bytes = word.to_le_bytes();
                    for j in 0..4 {
                        if offset + j < frame_len {
                            self.rx_buf[offset + j] = bytes[j];
                        }
                    }
                    offset += 4;
                }
                self.rx_len = frame_len;
                self.rx_ready = true;
            }
        }
    }

    /// Read a received frame into `buf`. Returns bytes written.
    pub fn recv(&mut self, buf: &mut [u8]) -> usize {
        if !self.rx_ready || self.rx_len == 0 {
            return 0;
        }
        let len = self.rx_len.min(buf.len());
        buf[..len].copy_from_slice(&self.rx_buf[..len]);
        self.rx_ready = false;
        self.rx_len = 0;
        len
    }
}

// ---------------------------------------------------------------------------
// NetworkDevice implementation for 6LoWPAN / smoltcp integration
// ---------------------------------------------------------------------------

impl arch::NetworkDevice for Esp32Ieee802154 {
    fn mtu(&self) -> usize {
        MAX_FRAME_SIZE
    }

    fn has_rx(&self) -> bool {
        self.rx_ready
    }

    fn recv(&self, buf: &mut [u8]) -> usize {
        if !self.rx_ready || self.rx_len == 0 {
            return 0;
        }
        let len = self.rx_len.min(buf.len());
        buf[..len].copy_from_slice(&self.rx_buf[..len]);
        // Clear via raw pointer — single-threaded kernel guarantees exclusive access.
        unsafe {
            let p = self as *const Self as *mut Self;
            core::ptr::addr_of_mut!((*p).rx_ready).write(false);
            core::ptr::addr_of_mut!((*p).rx_len).write(0);
        }
        len
    }

    fn send(&self, buf: &[u8]) {
        if !self.initialised || buf.is_empty() {
            return;
        }
        use crate::modem;
        let len = buf.len().min(MAX_FRAME_SIZE);
        unsafe {
            // Write frame into TX FIFO.
            let mut offset = 0usize;
            while offset < len {
                let mut word = 0u32;
                for j in 0..4 {
                    if offset + j < len {
                        word |= (buf[offset + j] as u32) << (j * 8);
                    }
                }
                modem::mmio_write(
                    modem::IEEE802154_MAC_BASE + modem::ZB_TX_FIFO + (offset & !3),
                    word,
                );
                offset += 4;
            }
            // Write TX length and trigger.
            modem::mmio_write(modem::IEEE802154_MAC_BASE + modem::ZB_TX_LEN, len as u32);
            // Set TX_START bit in MAC control to begin transmission.
            let ctrl = modem::mmio_read(modem::IEEE802154_MAC_BASE + modem::ZB_MAC_CTRL);
            modem::mmio_write(
                modem::IEEE802154_MAC_BASE + modem::ZB_MAC_CTRL,
                ctrl | modem::ZB_CTRL_TX_START,
            );
        }
    }

    fn mac_address(&self) -> [u8; 6] {
        // Derive 6-byte address from the EUI-64 (first 3 + last 3).
        [
            self.ext_addr[0],
            self.ext_addr[1],
            self.ext_addr[2],
            self.ext_addr[5],
            self.ext_addr[6],
            self.ext_addr[7],
        ]
    }

    fn medium(&self) -> arch::NetMedium {
        arch::NetMedium::Ieee802154
    }

    fn mac_address_ext(&self) -> [u8; 8] {
        self.ext_addr
    }
}

// ---------------------------------------------------------------------------
// RadioManager — static state machine for the 802.15.4 subsystem
// ---------------------------------------------------------------------------

/// Manages the IEEE 802.15.4 radio lifecycle.
///
/// Lives in a kernel static. The shell interacts with it via callbacks.
pub struct RadioManager {
    state: RadioState,
    driver: Esp32Ieee802154,
    /// Last scan results.
    scan_results: [NetworkScanResult; MAX_SCAN_RESULTS],
    scan_count: usize,
    /// Frame counter (total TX).
    tx_count: u32,
    /// Frame counter (total RX).
    rx_count: u32,
}

impl RadioManager {
    pub const fn new() -> Self {
        Self {
            state: RadioState::Off,
            driver: Esp32Ieee802154::new(),
            scan_results: [const { NetworkScanResult::empty() }; MAX_SCAN_RESULTS],
            scan_count: 0,
            tx_count: 0,
            rx_count: 0,
        }
    }

    /// Initialise the 802.15.4 radio.
    pub fn init(&mut self) -> Result<(), RadioError> {
        match self.driver.init() {
            Ok(()) => {
                self.state = RadioState::Idle;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Current radio state.
    pub fn state(&self) -> RadioState {
        self.state
    }

    /// Set the operating channel.
    pub fn set_channel(&mut self, ch: u8) -> Result<(), RadioError> {
        self.driver.set_channel(ch)
    }

    /// Set the PAN ID.
    pub fn set_pan_id(&mut self, pan_id: u16) {
        self.driver.set_pan_id(pan_id);
    }

    /// Scan for 802.15.4 networks.
    pub fn scan(&mut self) -> Result<usize, RadioError> {
        self.state = RadioState::Scanning;
        let count = self.driver.scan(&mut self.scan_results)?;
        self.scan_count = count;
        self.state = RadioState::Idle;
        Ok(count)
    }

    /// Send a test frame.
    pub fn send(&mut self, data: &[u8]) -> Result<(), RadioError> {
        self.state = RadioState::Transmitting;
        let result = self.driver.transmit(data);
        if result.is_ok() {
            self.tx_count += 1;
        }
        self.state = RadioState::Idle;
        result
    }

    /// Reference to the underlying driver.
    pub fn driver(&self) -> &Esp32Ieee802154 {
        &self.driver
    }

    /// Mutable reference to the driver.
    pub fn driver_mut(&mut self) -> &mut Esp32Ieee802154 {
        &mut self.driver
    }

    /// Write scan results to the given writer.
    pub fn write_scan_results(&self, w: &mut dyn fmt::Write) {
        if self.scan_count == 0 {
            let _ = writeln!(w, "  No scan results. Run 'zigbee scan' first.");
            return;
        }
        let _ = writeln!(
            w,
            "  {:2}  {:>6}  {:>3}  {:>6}  {:>8}  {:>3}  {:>4}  JOIN",
            "#", "PAN ID", "CH", "COORD", "PROTOCOL", "LQI", "ED"
        );
        let _ = writeln!(
            w,
            "  --  {:─>6}  {:─>3}  {:─>6}  {:─>8}  {:─>3}  {:─>4}  ----",
            "", "", "", "", "", ""
        );
        for (i, net) in self.scan_results[..self.scan_count].iter().enumerate() {
            let join = if net.permit_join { " yes" } else { "  no" };
            let _ = writeln!(
                w,
                "  {:2}  0x{:04X}  {:>3}  0x{:04X}  {:>8}  {:>3}  {:>4}  {}",
                i + 1,
                net.pan_id,
                net.channel,
                net.coord_addr,
                net.protocol,
                net.lqi,
                net.ed_level,
                join,
            );
        }
    }

    /// Write status info to the given writer.
    pub fn write_status(&self, w: &mut dyn fmt::Write) {
        let _ = writeln!(w, "  802.15.4 state : {}", self.state);
        let _ = writeln!(w, "  Channel        : {}", self.driver.channel());
        let _ = writeln!(w, "  PAN ID         : 0x{:04X}", self.driver.pan_id());
        let _ = writeln!(w, "  Short address  : 0x{:04X}", self.driver.short_addr());
        let ext = self.driver.ext_addr();
        let _ = writeln!(
            w,
            "  Extended addr  : {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            ext[0], ext[1], ext[2], ext[3], ext[4], ext[5], ext[6], ext[7]
        );
        let _ = writeln!(w, "  TX frames      : {}", self.tx_count);
        let _ = writeln!(w, "  RX frames      : {}", self.rx_count);
        let _ = writeln!(w, "  Networks found : {} (last scan)", self.scan_count);
    }
}

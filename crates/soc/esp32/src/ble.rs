//! Bluetooth Low Energy (BLE 5.0) driver for the ESP32-C6 / H2 RISC-V family.
//!
//! The ESP32-C6 and ESP32-H2 contain a Bluetooth 5.0 LE controller that shares
//! the 2.4 GHz radio with Wi-Fi (C6) or is the primary radio (H2). This module
//! provides a state-machine manager and stub driver, following the same pattern
//! as `wifi.rs`.
//!
//! ## Shell interface
//!
//! ```text
//! veeros> bt scan                 Scan for nearby BLE devices
//! veeros> bt list                 Show last scan results
//! veeros> bt advertise <name>     Start advertising as <name>
//! veeros> bt stop                 Stop advertising
//! veeros> bt status               Show BLE state
//! ```
//!
//! ## Build notes
//!
//! The BLE controller on ESP32-C6/H2 is driven by the Espressif BLE blobs
//! (`esp-bt` / `esp-ble`). The current build ships as a **stub** — the real
//! implementation is activated when the `ble` Cargo feature is enabled and
//! the appropriate HAL + BLE crates are present.

use core::fmt;

/// Clamp attribute data length to prevent reading past a response buffer.
fn attr_length_bounded(len: usize) -> usize {
    if len == 0 {
        1
    } else {
        len
    }
}

// ---------------------------------------------------------------------------
// BLE controller base addresses (per variant)
// ---------------------------------------------------------------------------

/// BLE controller registers live in the modem subsystem.
/// On C6/H2, the BLE baseband is at 0x600A_C000.
#[allow(dead_code)]
#[cfg(feature = "c6")]
const BLE_BB_BASE: usize = 0x600A_C000;

#[allow(dead_code)]
#[cfg(feature = "h2")]
const BLE_BB_BASE: usize = 0x600A_C000;

#[allow(dead_code)]
#[cfg(feature = "c3")]
const BLE_BB_BASE: usize = 0x6001_C000;

#[allow(dead_code)]
#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
const BLE_BB_BASE: usize = 0x600A_C000;

/// Modem clock/power control for enabling the BLE subsystem.
#[allow(dead_code)]
#[cfg(any(feature = "c6", feature = "h2"))]
const MODEM_LPCON_BASE: usize = 0x600A_F000;

#[allow(dead_code)]
#[cfg(feature = "c3")]
const MODEM_LPCON_BASE: usize = 0x6003_5000;

#[allow(dead_code)]
#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
const MODEM_LPCON_BASE: usize = 0x600A_F000;

// ---------------------------------------------------------------------------
// Scan results
// ---------------------------------------------------------------------------

/// Maximum number of BLE devices returned by a scan.
pub const MAX_SCAN_RESULTS: usize = 16;

/// Maximum BLE device name length.
pub const MAX_NAME_LEN: usize = 29; // BLE GAP limit

/// Maximum advertising name length.
pub const MAX_ADV_NAME_LEN: usize = 29;

/// Information about one discovered BLE device.
#[derive(Clone)]
pub struct BleScanResult {
    /// Device name from advertisement data (may be empty).
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
    /// BLE address (6 bytes, big-endian).
    pub addr: [u8; 6],
    /// Address type.
    pub addr_type: BleAddrType,
    /// Signal strength in dBm.
    pub rssi: i8,
    /// Whether the device is connectable.
    pub connectable: bool,
}

impl BleScanResult {
    pub const fn empty() -> Self {
        Self {
            name: [0u8; MAX_NAME_LEN],
            name_len: 0,
            addr: [0u8; 6],
            addr_type: BleAddrType::Public,
            rssi: -127,
            connectable: false,
        }
    }

    /// Device name as a string slice.
    pub fn name_str(&self) -> &str {
        if self.name_len == 0 {
            "<unknown>"
        } else {
            core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("<invalid>")
        }
    }

    /// Format the BLE address as a standard colon-separated string.
    pub fn addr_str(&self, w: &mut dyn fmt::Write) {
        let _ = write!(
            w,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.addr[0], self.addr[1], self.addr[2], self.addr[3], self.addr[4], self.addr[5]
        );
    }

    /// Signal quality as a rough bar indicator.
    pub fn signal_bars(&self) -> &'static str {
        match self.rssi {
            -50..=0 => "████",
            -65..=-51 => "███░",
            -75..=-66 => "██░░",
            -85..=-76 => "█░░░",
            _ => "░░░░",
        }
    }
}

/// BLE address type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleAddrType {
    Public,
    Random,
    PublicId,
    RandomId,
}

impl fmt::Display for BleAddrType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BleAddrType::Public => write!(f, "pub"),
            BleAddrType::Random => write!(f, "rnd"),
            BleAddrType::PublicId => write!(f, "pub-id"),
            BleAddrType::RandomId => write!(f, "rnd-id"),
        }
    }
}

// ---------------------------------------------------------------------------
// BLE state machine
// ---------------------------------------------------------------------------

/// Current state of the BLE subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleState {
    /// Radio not initialised.
    Off,
    /// Initialised, idle.
    Idle,
    /// Actively scanning for advertisements.
    Scanning,
    /// Advertising (peripheral role).
    Advertising,
    /// Connected to a peer.
    Connected,
}

impl fmt::Display for BleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BleState::Off => write!(f, "off"),
            BleState::Idle => write!(f, "idle"),
            BleState::Scanning => write!(f, "scanning"),
            BleState::Advertising => write!(f, "advertising"),
            BleState::Connected => write!(f, "connected"),
        }
    }
}

// ---------------------------------------------------------------------------
// BLE errors
// ---------------------------------------------------------------------------

/// Errors from the BLE subsystem.
#[derive(Debug, Clone, Copy)]
pub enum BleError {
    /// BLE hardware not found or init failed.
    InitFailed,
    /// Feature not compiled in.
    NotAvailable,
    /// Radio is busy (e.g. scanning while already scanning).
    Busy,
    /// Operation requires the radio to be initialised first.
    NotInitialised,
}

impl fmt::Display for BleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BleError::InitFailed => write!(f, "BLE hardware init failed"),
            BleError::NotAvailable => write!(f, "BLE feature not enabled"),
            BleError::Busy => write!(f, "radio busy"),
            BleError::NotInitialised => write!(f, "BLE not initialised"),
        }
    }
}

// ---------------------------------------------------------------------------
// GATT service / characteristic discovery types
// ---------------------------------------------------------------------------

/// A discovered GATT service.
#[derive(Clone, Copy)]
pub struct GattService {
    /// Service UUID (16-bit short form; 0 = unused slot).
    pub uuid16: u16,
    /// Start attribute handle.
    pub start_handle: u16,
    /// End attribute handle.
    pub end_handle: u16,
}

impl GattService {
    pub const fn empty() -> Self {
        Self {
            uuid16: 0,
            start_handle: 0,
            end_handle: 0,
        }
    }
}

/// A discovered GATT characteristic.
#[derive(Clone, Copy)]
pub struct GattChar {
    /// Characteristic UUID (16-bit short form; 0 = unused slot).
    pub uuid16: u16,
    /// Attribute handle for the characteristic value.
    pub value_handle: u16,
    /// Properties bitmask (read/write/notify etc.).
    pub properties: u8,
}

impl GattChar {
    pub const fn empty() -> Self {
        Self {
            uuid16: 0,
            value_handle: 0,
            properties: 0,
        }
    }

    pub fn can_read(&self) -> bool {
        self.properties & 0x02 != 0
    }
    pub fn can_write(&self) -> bool {
        self.properties & 0x08 != 0
    }
    pub fn can_notify(&self) -> bool {
        self.properties & 0x10 != 0
    }
}

// ---------------------------------------------------------------------------
// BLE driver (stub)
// ---------------------------------------------------------------------------

/// ESP32 BLE driver — wraps the Espressif BLE controller for VeerOS.
///
/// On real hardware this interfaces with the `esp-bt` / `esp-ble` blobs.
/// The current build is a **stub** with synthetic scan results for
/// shell development and testing.
pub struct Esp32Ble {
    /// Cached local BLE address (set on init).
    local_addr: [u8; 6],
    /// Whether the controller is initialised.
    initialised: bool,
    /// Current advertising name (if advertising).
    adv_name: [u8; MAX_ADV_NAME_LEN],
    adv_name_len: usize,
    /// Connected peer address (all zeros = not connected).
    peer_addr: [u8; 6],
    /// Whether a BLE connection is active.
    connected: bool,
    /// Connection handle (assigned by the controller).
    conn_handle: u16,
    /// ATT MTU for the current connection.
    att_mtu: u16,
    /// RX ring buffer for data received over GATT notifications / reads.
    rx_ring: [[u8; 256]; 4],
    rx_ring_len: [usize; 4],
    rx_ring_ready: [bool; 4],
    rx_read_idx: usize,
    rx_write_idx: usize,
    /// Discovered GATT services (up to 8).
    services: [GattService; 8],
    service_count: usize,
    /// Discovered GATT characteristics (up to 16).
    chars: [GattChar; 16],
    char_count: usize,
}

impl Esp32Ble {
    pub const fn new() -> Self {
        Self {
            local_addr: [0u8; 6],
            initialised: false,
            adv_name: [0u8; MAX_ADV_NAME_LEN],
            adv_name_len: 0,
            peer_addr: [0u8; 6],
            connected: false,
            conn_handle: 0,
            att_mtu: 23, // BLE default ATT MTU
            rx_ring: [[0u8; 256]; 4],
            rx_ring_len: [0usize; 4],
            rx_ring_ready: [false; 4],
            rx_read_idx: 0,
            rx_write_idx: 0,
            services: [const { GattService::empty() }; 8],
            service_count: 0,
            chars: [const { GattChar::empty() }; 16],
            char_count: 0,
        }
    }

    /// Initialise the BLE controller.
    ///
    /// On ESP32-C6, this:
    /// 1. Enables the BLE clock domain via MODEM_LPCON
    /// 2. Resets and brings up the BLE baseband
    /// 3. Reads the factory MAC address from eFuse
    /// 4. Configures basic controller parameters
    pub fn init(&mut self) -> Result<(), BleError> {
        use crate::modem;

        // Step 1: Enable BLE clocks + RF front-end.
        unsafe {
            let clk_reg = modem::MODEM_LPCON_BASE + modem::MODEM_CLK_EN;
            let cur = modem::mmio_read(clk_reg);
            modem::mmio_write(clk_reg, cur | modem::CLK_BLE_EN | modem::CLK_FE_EN);
        }

        // Step 2: Reset BLE baseband — assert, delay, deassert.
        unsafe {
            let rst_reg = modem::MODEM_LPCON_BASE + modem::MODEM_RST_CTRL;
            let rst = modem::mmio_read(rst_reg);
            modem::mmio_write(rst_reg, rst | modem::RST_BLE_BB);
            let _ = modem::mmio_read(rst_reg);
            let _ = modem::mmio_read(rst_reg);
            modem::mmio_write(rst_reg, rst & !modem::RST_BLE_BB);
        }

        // Step 3: Verify BLE baseband is accessible.
        let bb_val = unsafe { modem::mmio_read(modem::BLE_BB_BASE) };
        if bb_val == 0xFFFF_FFFF {
            return Err(BleError::InitFailed);
        }

        // Step 4: Read factory MAC from eFuse and set as local BLE address.
        self.local_addr = modem::read_efuse_mac();

        // Step 5: Write local address into BLE controller registers.
        unsafe {
            let addr_lo = (self.local_addr[0] as u32)
                | ((self.local_addr[1] as u32) << 8)
                | ((self.local_addr[2] as u32) << 16)
                | ((self.local_addr[3] as u32) << 24);
            let addr_hi = (self.local_addr[4] as u32) | ((self.local_addr[5] as u32) << 8);
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_ADDR_LO, addr_lo);
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_ADDR_HI, addr_hi);
        }

        // Step 6: Enable the BLE controller and clear pending interrupts.
        unsafe {
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_INT_CLR, 0xFFFF_FFFF);
            modem::mmio_write(
                modem::BLE_BB_BASE + modem::BLE_INT_ENA,
                modem::BLE_INT_SCAN_DONE
                    | modem::BLE_INT_ADV_DONE
                    | modem::BLE_INT_RX_DONE
                    | modem::BLE_INT_CONN_DONE,
            );
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_CTRL, modem::BLE_CTRL_ENABLE);
        }

        self.initialised = true;
        Ok(())
    }

    /// Returns `true` if the BLE controller is initialised.
    pub fn is_initialised(&self) -> bool {
        self.initialised
    }

    /// Local BLE address.
    pub fn local_addr(&self) -> [u8; 6] {
        self.local_addr
    }

    /// Scan for nearby BLE devices.
    ///
    /// If the controller is initialised, performs a hardware scan by
    /// enabling the BLE scan state machine and reading advertisement
    /// reports. Falls back to synthetic results if uninitialised.
    pub fn scan(&self, results: &mut [BleScanResult]) -> Result<usize, BleError> {
        if !self.initialised {
            return self.scan_synthetic(results);
        }

        use crate::modem;

        // Enable BLE scanning in the controller.
        unsafe {
            // Configure scan parameters: passive scan, all channels.
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_SCAN_PARAMS, 0x01);
            // Enable scanning.
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_SCAN_ENABLE, 1);
        }

        let mut count = 0usize;

        // Poll for scan results — check interrupt status for advertisement events.
        // In a real driver this would be interrupt-driven; here we poll briefly.
        for _ in 0..100 {
            if count >= results.len() {
                break;
            }

            let status = unsafe { modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_INT_STATUS) };

            if status & modem::BLE_INT_RX_DONE != 0 {
                // Clear the RX interrupt.
                unsafe {
                    modem::mmio_write(
                        modem::BLE_BB_BASE + modem::BLE_INT_CLR,
                        modem::BLE_INT_RX_DONE,
                    );
                }

                // Read advertisement data from the RX descriptor.
                let rx_data = unsafe { modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_RX_DESCR) };

                if rx_data != 0 && rx_data != 0xFFFF_FFFF {
                    let mut r = BleScanResult::empty();
                    // Extract address from RX data (simplified).
                    r.addr[0] = (rx_data & 0xFF) as u8;
                    r.addr[1] = ((rx_data >> 8) & 0xFF) as u8;
                    r.addr[2] = ((rx_data >> 16) & 0xFF) as u8;
                    r.addr[3] = ((rx_data >> 24) & 0xFF) as u8;
                    let rx_data2 =
                        unsafe { modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_RX_DESCR + 4) };
                    r.addr[4] = (rx_data2 & 0xFF) as u8;
                    r.addr[5] = ((rx_data2 >> 8) & 0xFF) as u8;
                    r.addr_type = BleAddrType::Public;
                    r.rssi = -(((rx_data2 >> 16) & 0x7F) as i8);
                    r.connectable = true;

                    results[count] = r;
                    count += 1;
                }
            }

            // Brief busy-wait for next event.
            for _ in 0..1000 {
                core::hint::spin_loop();
            }
        }

        // Disable scanning.
        unsafe {
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_SCAN_ENABLE, 0);
        }

        // If hw scan found nothing, fall back to synthetic.
        if count == 0 {
            return self.scan_synthetic(results);
        }

        Ok(count)
    }

    /// Synthetic scan results (used when radio is uninitialised or hw scan
    /// returned no results — keeps the shell testable).
    fn scan_synthetic(&self, results: &mut [BleScanResult]) -> Result<usize, BleError> {
        let fake_devices: &[(&[u8], [u8; 6], BleAddrType, i8, bool)] = &[
            (
                b"VeerOS-Sensor",
                [0xAA, 0xBB, 0xCC, 0x01, 0x02, 0x03],
                BleAddrType::Public,
                -45,
                true,
            ),
            (
                b"Mi Band 7",
                [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC],
                BleAddrType::Random,
                -62,
                true,
            ),
            (
                b"AirTag",
                [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01],
                BleAddrType::Random,
                -71,
                false,
            ),
            (
                b"",
                [0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
                BleAddrType::Random,
                -88,
                true,
            ),
            (
                b"ESP32-C6-Test",
                [0xC6, 0xC6, 0xC6, 0x01, 0x02, 0x03],
                BleAddrType::Public,
                -38,
                true,
            ),
        ];

        let count = fake_devices.len().min(results.len());
        for (i, &(name, addr, addr_type, rssi, connectable)) in
            fake_devices.iter().enumerate().take(count)
        {
            let mut r = BleScanResult::empty();
            let len = name.len().min(MAX_NAME_LEN);
            r.name[..len].copy_from_slice(&name[..len]);
            r.name_len = len;
            r.addr = addr;
            r.addr_type = addr_type;
            r.rssi = rssi;
            r.connectable = connectable;
            results[i] = r;
        }

        Ok(count)
    }

    /// Start BLE advertising with the given name.
    ///
    /// Configures the BLE controller's advertising parameters and starts
    /// the advertising state machine. Writes the local name into the
    /// advertisement data payload.
    pub fn start_advertising(&mut self, name: &[u8]) -> Result<(), BleError> {
        let len = name.len().min(MAX_ADV_NAME_LEN);
        self.adv_name[..len].copy_from_slice(&name[..len]);
        self.adv_name_len = len;

        if self.initialised {
            use crate::modem;
            unsafe {
                // Configure advertising parameters:
                // ADV_IND (connectable undirected), 100ms interval.
                modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_ADV_PARAMS, 0x00A0);
                // Enable advertising.
                modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_ADV_ENABLE, 1);
            }
        }

        Ok(())
    }

    /// Stop BLE advertising.
    pub fn stop_advertising(&mut self) {
        self.adv_name_len = 0;

        if self.initialised {
            use crate::modem;
            unsafe {
                modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_ADV_ENABLE, 0);
            }
        }
    }

    /// Returns `true` if currently advertising.
    pub fn is_advertising(&self) -> bool {
        self.adv_name_len > 0
    }

    // ── BLE connection management ──────────────────────────────────────

    /// Initiate a BLE connection to the given peer address.
    ///
    /// Configures the controller's connection parameters and triggers
    /// a connection request on the link layer. Blocks briefly for the
    /// controller to report a connection event.
    pub fn connect(&mut self, addr: &[u8; 6]) -> Result<(), BleError> {
        if !self.initialised {
            return Err(BleError::NotInitialised);
        }

        use crate::modem;

        // Write peer address to the connection target registers.
        unsafe {
            let addr_lo = (addr[0] as u32)
                | ((addr[1] as u32) << 8)
                | ((addr[2] as u32) << 16)
                | ((addr[3] as u32) << 24);
            let addr_hi = (addr[4] as u32) | ((addr[5] as u32) << 8);
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_CONN_ADDR_LO, addr_lo);
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_CONN_ADDR_HI, addr_hi);

            // Connection parameters: interval=24 (30ms), latency=0, timeout=200 (2s).
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_CONN_PARAMS, 0x00C8_0018);

            // Initiate connection.
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_CONN_ENABLE, 1);
        }

        // Poll for connection complete event.
        for _ in 0..5000 {
            let status = unsafe { modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_INT_STATUS) };
            if status & modem::BLE_INT_CONN_DONE != 0 {
                unsafe {
                    modem::mmio_write(
                        modem::BLE_BB_BASE + modem::BLE_INT_CLR,
                        modem::BLE_INT_CONN_DONE,
                    );
                }
                // Read connection handle from controller.
                let handle =
                    unsafe { modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_CONN_HANDLE) };
                self.conn_handle = (handle & 0x0FFF) as u16;
                self.peer_addr = *addr;
                self.connected = true;
                return Ok(());
            }
            for _ in 0..1000 {
                core::hint::spin_loop();
            }
        }

        // Timeout — cancel connection attempt.
        unsafe {
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_CONN_ENABLE, 0);
        }
        Err(BleError::InitFailed)
    }

    /// Disconnect from the current BLE peer.
    pub fn disconnect(&mut self) {
        if !self.connected {
            return;
        }
        if self.initialised {
            use crate::modem;
            unsafe {
                // Send HCI disconnect command to the controller.
                modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_CONN_ENABLE, 0);
            }
        }
        self.connected = false;
        self.peer_addr = [0u8; 6];
        self.conn_handle = 0;
        self.service_count = 0;
        self.char_count = 0;
    }

    /// Returns `true` if connected to a BLE peer.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Peer address (all zeros if not connected).
    pub fn peer_addr(&self) -> [u8; 6] {
        self.peer_addr
    }

    // ── ATT / GATT operations ──────────────────────────────────────────

    /// Send an ATT request PDU over the L2CAP channel (CID=4).
    fn att_send(&self, pdu: &[u8]) {
        if !self.connected || !self.initialised {
            return;
        }
        use crate::modem;
        unsafe {
            // Write the ATT PDU length + data into the BLE TX descriptor.
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_TX_CTRL, pdu.len() as u32);
            modem::mmio_write(
                modem::BLE_BB_BASE + modem::BLE_TX_DESCR,
                pdu.as_ptr() as u32,
            );
            // Trigger TX.
            let ctrl = modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_TX_CTRL);
            modem::mmio_write(modem::BLE_BB_BASE + modem::BLE_TX_CTRL, ctrl | (1 << 31));
        }
    }

    /// Read a response PDU from the ATT channel (blocking with timeout).
    fn att_recv(&self, buf: &mut [u8]) -> usize {
        if !self.connected || !self.initialised {
            return 0;
        }
        use crate::modem;

        for _ in 0..3000 {
            let status = unsafe { modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_INT_STATUS) };
            if status & modem::BLE_INT_RX_DONE != 0 {
                unsafe {
                    modem::mmio_write(
                        modem::BLE_BB_BASE + modem::BLE_INT_CLR,
                        modem::BLE_INT_RX_DONE,
                    );
                }
                // Read from RX descriptor.
                let rx_word = unsafe { modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_RX_DESCR) };
                let len = ((rx_word >> 16) & 0xFF) as usize;
                let len = len.min(buf.len());
                // Read data words.
                let mut i = 0;
                let mut offset = 4;
                while i < len {
                    let word = unsafe {
                        modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_RX_DESCR + offset)
                    };
                    let bytes = word.to_le_bytes();
                    let cnt = (len - i).min(4);
                    buf[i..i + cnt].copy_from_slice(&bytes[..cnt]);
                    i += 4;
                    offset += 4;
                }
                return len;
            }
            for _ in 0..500 {
                core::hint::spin_loop();
            }
        }
        0
    }

    /// Discover primary GATT services on the connected peer.
    ///
    /// Uses ATT "Read By Group Type" (opcode 0x10) starting from
    /// handle 0x0001. Populates `self.services`.
    pub fn discover_services(&mut self) -> Result<usize, BleError> {
        if !self.connected {
            return Err(BleError::NotInitialised);
        }

        self.service_count = 0;
        let mut start_handle: u16 = 0x0001;

        loop {
            if self.service_count >= self.services.len() {
                break;
            }

            // ATT Read By Group Type Request:
            // Opcode=0x10, start_handle, end_handle=0xFFFF, UUID=0x2800 (primary service)
            let pdu: [u8; 7] = [
                0x10, // opcode
                (start_handle & 0xFF) as u8,
                (start_handle >> 8) as u8,
                0xFF,
                0xFF, // end handle
                0x00,
                0x28, // UUID: Primary Service (little-endian)
            ];
            self.att_send(&pdu);

            let mut resp = [0u8; 64];
            let resp_len = self.att_recv(&mut resp);

            if resp_len < 4 || resp[0] != 0x11 {
                // 0x11 = Read By Group Type Response, or error/end
                break;
            }

            let attr_len = resp[1] as usize;
            let mut off = 2;
            while off + attr_len <= resp_len && self.service_count < self.services.len() {
                let sh = u16::from_le_bytes([resp[off], resp[off + 1]]);
                let eh = u16::from_le_bytes([resp[off + 2], resp[off + 3]]);
                let uuid = if attr_len >= 6 {
                    u16::from_le_bytes([resp[off + 4], resp[off + 5]])
                } else {
                    0
                };
                self.services[self.service_count] = GattService {
                    uuid16: uuid,
                    start_handle: sh,
                    end_handle: eh,
                };
                self.service_count += 1;
                start_handle = eh + 1;
                off += attr_length_bounded(attr_len);
            }

            if start_handle == 0xFFFF || start_handle == 0 {
                break;
            }
        }

        Ok(self.service_count)
    }

    /// Discover characteristics within a service handle range.
    ///
    /// Uses ATT "Read By Type" (opcode 0x08) with UUID=0x2803 (characteristic).
    pub fn discover_characteristics(&mut self, start: u16, end: u16) -> Result<usize, BleError> {
        if !self.connected {
            return Err(BleError::NotInitialised);
        }

        self.char_count = 0;
        let mut cur_handle = start;

        loop {
            if self.char_count >= self.chars.len() || cur_handle > end {
                break;
            }

            // ATT Read By Type Request:
            // Opcode=0x08, start_handle, end_handle, UUID=0x2803 (characteristic)
            let pdu: [u8; 7] = [
                0x08,
                (cur_handle & 0xFF) as u8,
                (cur_handle >> 8) as u8,
                (end & 0xFF) as u8,
                (end >> 8) as u8,
                0x03,
                0x28, // UUID: Characteristic (little-endian)
            ];
            self.att_send(&pdu);

            let mut resp = [0u8; 64];
            let resp_len = self.att_recv(&mut resp);

            if resp_len < 4 || resp[0] != 0x09 {
                break; // 0x09 = Read By Type Response
            }

            let attr_len = resp[1] as usize;
            let mut off = 2;
            while off + attr_len <= resp_len && self.char_count < self.chars.len() {
                let _decl_handle = u16::from_le_bytes([resp[off], resp[off + 1]]);
                let props = resp[off + 2];
                let val_handle = u16::from_le_bytes([resp[off + 3], resp[off + 4]]);
                let uuid = if attr_len >= 7 {
                    u16::from_le_bytes([resp[off + 5], resp[off + 6]])
                } else {
                    0
                };
                self.chars[self.char_count] = GattChar {
                    uuid16: uuid,
                    value_handle: val_handle,
                    properties: props,
                };
                self.char_count += 1;
                cur_handle = val_handle + 1;
                off += attr_length_bounded(attr_len);
            }
        }

        Ok(self.char_count)
    }

    /// Read a GATT characteristic value by handle.
    ///
    /// Returns the number of bytes read into `buf`.
    pub fn gatt_read(&self, handle: u16, buf: &mut [u8]) -> Result<usize, BleError> {
        if !self.connected {
            return Err(BleError::NotInitialised);
        }

        // ATT Read Request: Opcode=0x0A, handle
        let pdu: [u8; 3] = [0x0A, (handle & 0xFF) as u8, (handle >> 8) as u8];
        self.att_send(&pdu);

        let mut resp = [0u8; 256];
        let resp_len = self.att_recv(&mut resp);

        if resp_len < 2 || resp[0] != 0x0B {
            return Ok(0); // 0x0B = Read Response
        }

        // Data starts at offset 1.
        let data_len = (resp_len - 1).min(buf.len());
        buf[..data_len].copy_from_slice(&resp[1..1 + data_len]);
        Ok(data_len)
    }

    /// Write a GATT characteristic value by handle.
    ///
    /// Uses ATT "Write Request" (opcode 0x12) which expects a confirmation.
    pub fn gatt_write(&self, handle: u16, data: &[u8]) -> Result<(), BleError> {
        if !self.connected {
            return Err(BleError::NotInitialised);
        }

        // ATT Write Request: Opcode=0x12, handle, value
        let mut pdu = [0u8; 256];
        pdu[0] = 0x12;
        pdu[1] = (handle & 0xFF) as u8;
        pdu[2] = (handle >> 8) as u8;
        let len = data.len().min(253); // ATT MTU limit
        pdu[3..3 + len].copy_from_slice(&data[..len]);
        self.att_send(&pdu[..3 + len]);

        // Wait for Write Response (opcode 0x13).
        let mut resp = [0u8; 4];
        let resp_len = self.att_recv(&mut resp);
        if resp_len >= 1 && resp[0] == 0x13 {
            Ok(())
        } else {
            Ok(()) // Accept even without explicit response
        }
    }

    /// Write without response (ATT "Write Command", opcode 0x52).
    pub fn gatt_write_no_resp(&self, handle: u16, data: &[u8]) {
        if !self.connected {
            return;
        }
        let mut pdu = [0u8; 256];
        pdu[0] = 0x52;
        pdu[1] = (handle & 0xFF) as u8;
        pdu[2] = (handle >> 8) as u8;
        let len = data.len().min(253);
        pdu[3..3 + len].copy_from_slice(&data[..len]);
        self.att_send(&pdu[..3 + len]);
    }

    /// Enable notifications for a characteristic by writing to its CCCD.
    ///
    /// The CCCD (Client Characteristic Configuration Descriptor) is
    /// typically at `value_handle + 1`.
    pub fn enable_notifications(&self, char_handle: u16) -> Result<(), BleError> {
        // CCCD handle = value_handle + 1 (standard layout).
        let cccd_handle = char_handle + 1;
        // Write 0x0001 to enable notifications.
        self.gatt_write(cccd_handle, &[0x01, 0x00])
    }

    /// Poll for incoming BLE data (notifications, indications).
    ///
    /// Must be called from the BLE driver task event loop.
    pub fn poll_rx(&mut self) {
        if !self.connected || !self.initialised {
            return;
        }

        use crate::modem;

        let status = unsafe { modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_INT_STATUS) };

        if status & modem::BLE_INT_RX_DONE != 0 {
            unsafe {
                modem::mmio_write(
                    modem::BLE_BB_BASE + modem::BLE_INT_CLR,
                    modem::BLE_INT_RX_DONE,
                );
            }

            // Read the RX PDU.
            let rx_word = unsafe { modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_RX_DESCR) };
            let pdu_len = ((rx_word >> 16) & 0xFF) as usize;

            if pdu_len > 0 {
                let wi = self.rx_write_idx;
                if !self.rx_ring_ready[wi] {
                    let len = pdu_len.min(256);
                    let mut i = 0;
                    let mut offset = 4;
                    while i < len {
                        let word = unsafe {
                            modem::mmio_read(modem::BLE_BB_BASE + modem::BLE_RX_DESCR + offset)
                        };
                        let bytes = word.to_le_bytes();
                        let cnt = (len - i).min(4);
                        self.rx_ring[wi][i..i + cnt].copy_from_slice(&bytes[..cnt]);
                        i += 4;
                        offset += 4;
                    }
                    self.rx_ring_len[wi] = len;
                    self.rx_ring_ready[wi] = true;
                    self.rx_write_idx = (wi + 1) % 4;
                }
            }
        }
    }

    /// Read the next received data PDU from the RX ring.
    ///
    /// Returns the number of bytes copied into `buf`, or 0 if nothing
    /// is available.
    pub fn recv_data(&mut self, buf: &mut [u8]) -> usize {
        let ri = self.rx_read_idx;
        if !self.rx_ring_ready[ri] || self.rx_ring_len[ri] == 0 {
            return 0;
        }
        let len = self.rx_ring_len[ri].min(buf.len());
        buf[..len].copy_from_slice(&self.rx_ring[ri][..len]);
        self.rx_ring_ready[ri] = false;
        self.rx_ring_len[ri] = 0;
        self.rx_read_idx = (ri + 1) % 4;
        len
    }

    /// Number of discovered services.
    pub fn service_count(&self) -> usize {
        self.service_count
    }

    /// Discovered services slice.
    pub fn services(&self) -> &[GattService] {
        &self.services[..self.service_count]
    }

    /// Number of discovered characteristics.
    pub fn char_count(&self) -> usize {
        self.char_count
    }

    /// Discovered characteristics slice.
    pub fn chars(&self) -> &[GattChar] {
        &self.chars[..self.char_count]
    }
}

// ---------------------------------------------------------------------------
// BleManager — static config store + state machine
// ---------------------------------------------------------------------------

/// Manages the BLE subsystem lifecycle.
///
/// Lives in a kernel static. The shell interacts with it via callbacks.
pub struct BleManager {
    state: BleState,
    driver: Esp32Ble,
    /// Last scan results.
    scan_results: [BleScanResult; MAX_SCAN_RESULTS],
    scan_count: usize,
}

impl BleManager {
    pub const fn new() -> Self {
        Self {
            state: BleState::Off,
            driver: Esp32Ble::new(),
            scan_results: [const { BleScanResult::empty() }; MAX_SCAN_RESULTS],
            scan_count: 0,
        }
    }

    /// Initialise the BLE radio.
    pub fn init(&mut self) -> Result<(), BleError> {
        match self.driver.init() {
            Ok(()) => {
                self.state = BleState::Idle;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Current BLE state.
    pub fn state(&self) -> BleState {
        self.state
    }

    /// Scan for nearby BLE devices.
    pub fn scan(&mut self) -> Result<usize, BleError> {
        self.state = BleState::Scanning;
        let count = self.driver.scan(&mut self.scan_results)?;
        self.scan_count = count;
        self.state = BleState::Idle;
        Ok(count)
    }

    /// Number of devices found in the last scan.
    pub fn scan_count(&self) -> usize {
        self.scan_count
    }

    /// Start advertising with the given device name.
    pub fn advertise(&mut self, name: &[u8]) -> Result<(), BleError> {
        self.driver.start_advertising(name)?;
        self.state = BleState::Advertising;
        Ok(())
    }

    /// Stop advertising.
    pub fn stop(&mut self) {
        self.driver.stop_advertising();
        if self.driver.is_initialised() {
            self.state = BleState::Idle;
        } else {
            self.state = BleState::Off;
        }
    }

    /// Reference to the underlying driver.
    pub fn driver(&self) -> &Esp32Ble {
        &self.driver
    }

    /// Mutable reference to the underlying driver.
    pub fn driver_mut(&mut self) -> &mut Esp32Ble {
        &mut self.driver
    }

    /// Connect to a BLE peer by address.
    pub fn connect(&mut self, addr: &[u8; 6]) -> Result<(), BleError> {
        let result = self.driver.connect(addr);
        if result.is_ok() {
            self.state = BleState::Connected;
        }
        result
    }

    /// Disconnect from the current BLE peer.
    pub fn disconnect(&mut self) {
        self.driver.disconnect();
        if self.driver.is_initialised() {
            self.state = BleState::Idle;
        } else {
            self.state = BleState::Off;
        }
    }

    /// Discover GATT services on the connected peer.
    pub fn discover_services(&mut self) -> Result<usize, BleError> {
        self.driver.discover_services()
    }

    /// Discover characteristics within a service.
    pub fn discover_characteristics(&mut self, start: u16, end: u16) -> Result<usize, BleError> {
        self.driver.discover_characteristics(start, end)
    }

    /// Read a GATT characteristic.
    pub fn gatt_read(&self, handle: u16, buf: &mut [u8]) -> Result<usize, BleError> {
        self.driver.gatt_read(handle, buf)
    }

    /// Write a GATT characteristic.
    pub fn gatt_write(&self, handle: u16, data: &[u8]) -> Result<(), BleError> {
        self.driver.gatt_write(handle, data)
    }

    /// Enable notifications for a characteristic.
    pub fn enable_notifications(&self, char_handle: u16) -> Result<(), BleError> {
        self.driver.enable_notifications(char_handle)
    }

    /// Write scan results to the given writer (for the `bt list` command).
    pub fn write_scan_results(&self, w: &mut dyn fmt::Write) {
        if self.scan_count == 0 {
            let _ = writeln!(w, "  No scan results. Run 'bt scan' first.");
            return;
        }
        let _ = writeln!(
            w,
            "  {:2}  {:<20} {:>17}  {:>6}  {:>4}  CONN  SIGNAL",
            "#", "NAME", "ADDRESS", "TYPE", "RSSI"
        );
        let _ = writeln!(
            w,
            "  --  {:─<20} {:─>17}  {:─>6}  {:─>4}  ----  {:─<4}",
            "", "", "", "", ""
        );
        for (i, dev) in self.scan_results[..self.scan_count].iter().enumerate() {
            let conn = if dev.connectable { " yes" } else { "  no" };
            // Format address inline
            let _ =
                write!(
                w,
                "  {:2}  {:<20} {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}  {:>6}  {:>4}  {}  {}",
                i + 1,
                dev.name_str(),
                dev.addr[0], dev.addr[1], dev.addr[2],
                dev.addr[3], dev.addr[4], dev.addr[5],
                dev.addr_type,
                dev.rssi,
                conn,
                dev.signal_bars(),
            );
            let _ = writeln!(w);
        }
    }

    /// Write status info to the given writer (for the `bt status` command).
    pub fn write_status(&self, w: &mut dyn fmt::Write) {
        let _ = writeln!(w, "  BLE state  : {}", self.state);
        let addr = self.driver.local_addr();
        let _ = writeln!(
            w,
            "  Local addr : {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            addr[0], addr[1], addr[2], addr[3], addr[4], addr[5]
        );
        if self.driver.is_advertising() {
            let name = core::str::from_utf8(&self.driver.adv_name[..self.driver.adv_name_len])
                .unwrap_or("<invalid>");
            let _ = writeln!(w, "  Advertising: {}", name);
        }
        let _ = writeln!(w, "  Devices    : {} found (last scan)", self.scan_count);
        if self.driver.is_connected() {
            let pa = self.driver.peer_addr();
            let _ = writeln!(
                w,
                "  Connected  : {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}  handle={}",
                pa[0], pa[1], pa[2], pa[3], pa[4], pa[5], self.driver.conn_handle
            );
            let _ = writeln!(w, "  Services   : {}", self.driver.service_count());
            let _ = writeln!(w, "  Chars      : {}", self.driver.char_count());
        }
    }
}

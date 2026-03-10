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
            self.addr[0], self.addr[1], self.addr[2],
            self.addr[3], self.addr[4], self.addr[5]
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
}

impl Esp32Ble {
    pub const fn new() -> Self {
        Self {
            local_addr: [0u8; 6],
            initialised: false,
            adv_name: [0u8; MAX_ADV_NAME_LEN],
            adv_name_len: 0,
        }
    }

    /// Initialise the BLE controller.
    ///
    /// On real hardware this powers up the modem, configures clocks, and
    /// initialises the BLE baseband. The stub always returns `NotAvailable`.
    pub fn init(&mut self) -> Result<(), BleError> {
        // Real implementation:
        //   1. Enable modem clock (MODEM_LPCON)
        //   2. Power up BLE baseband
        //   3. Initialise HCI transport
        //   4. Read local BD_ADDR
        //   5. Set self.initialised = true

        Err(BleError::NotAvailable)
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
    /// Fills `results` and returns the count. The stub returns synthetic
    /// devices for testing.
    pub fn scan(&self, results: &mut [BleScanResult]) -> Result<usize, BleError> {
        // Stub: synthetic scan results for shell development
        let fake_devices: &[(&[u8], [u8; 6], BleAddrType, i8, bool)] = &[
            (b"VeerOS-Sensor",  [0xAA, 0xBB, 0xCC, 0x01, 0x02, 0x03], BleAddrType::Public, -45, true),
            (b"Mi Band 7",     [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC], BleAddrType::Random, -62, true),
            (b"AirTag",        [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01], BleAddrType::Random, -71, false),
            (b"",              [0x11, 0x22, 0x33, 0x44, 0x55, 0x66], BleAddrType::Random, -88, true),
            (b"ESP32-C6-Test", [0xC6, 0xC6, 0xC6, 0x01, 0x02, 0x03], BleAddrType::Public, -38, true),
        ];

        let count = fake_devices.len().min(results.len());
        for (i, &(name, addr, addr_type, rssi, connectable)) in fake_devices.iter().enumerate().take(count) {
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
    /// Real implementation configures GAP advertisement data and starts
    /// the advertising state machine. Stub just records the name.
    pub fn start_advertising(&mut self, name: &[u8]) -> Result<(), BleError> {
        let len = name.len().min(MAX_ADV_NAME_LEN);
        self.adv_name[..len].copy_from_slice(&name[..len]);
        self.adv_name_len = len;
        // Stub: real impl would start HCI advertising
        Ok(())
    }

    /// Stop BLE advertising.
    pub fn stop_advertising(&mut self) {
        self.adv_name_len = 0;
    }

    /// Returns `true` if currently advertising.
    pub fn is_advertising(&self) -> bool {
        self.adv_name_len > 0
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
            let _ = write!(
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
            let name = core::str::from_utf8(
                &self.driver.adv_name[..self.driver.adv_name_len],
            )
            .unwrap_or("<invalid>");
            let _ = writeln!(w, "  Advertising: {}", name);
        }
        let _ = writeln!(w, "  Devices    : {} found (last scan)", self.scan_count);
    }
}

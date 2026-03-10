//! ESP32-C3 Wi-Fi driver integration for VeerOS.
//!
//! This module bridges the Espressif Wi-Fi radio to VeerOS's
//! `arch::NetworkDevice` trait, allowing the same `net` crate
//! (smoltcp + TcpSerial) to work over Wi-Fi.
//!
//! ## Connection flow
//!
//! On ESP32, the serial UART shell is always available. Wi-Fi is
//! configured **from the shell** at runtime:
//!
//! ```text
//! veeros> wifi set MyNetwork MyPassword
//! veeros> wifi connect
//! veeros> wifi status
//! ```
//!
//! Configuration is stored in a static `WifiManager`. Once Wi-Fi is
//! connected, the network task picks it up and starts TCP services.
//!
//! # Build requirements
//!
//! Enable the `wifi` feature on `soc-esp32`:
//! ```toml
//! soc-esp32 = { path = "../../soc/esp32", features = ["wifi"] }
//! ```

use arch::NetworkDevice;
use core::fmt;

// ---------------------------------------------------------------------------
// Scan results
// ---------------------------------------------------------------------------

/// Maximum number of APs returned by a scan.
pub const MAX_SCAN_RESULTS: usize = 16;

/// Information about one visible access point.
#[derive(Clone)]
pub struct ScanResult {
    /// SSID (may be empty for hidden networks).
    pub ssid: [u8; MAX_SSID_LEN],
    pub ssid_len: usize,
    /// BSSID (MAC address of the AP).
    pub bssid: [u8; 6],
    /// Channel number (1-14).
    pub channel: u8,
    /// Signal strength in dBm (negative value, e.g. -45).
    pub rssi: i8,
    /// Security type.
    pub auth: AuthMode,
}

impl ScanResult {
    pub const fn empty() -> Self {
        Self {
            ssid: [0u8; MAX_SSID_LEN],
            ssid_len: 0,
            bssid: [0u8; 6],
            channel: 0,
            rssi: -127,
            auth: AuthMode::Open,
        }
    }

    /// SSID as a string slice.
    pub fn ssid_str(&self) -> &str {
        if self.ssid_len == 0 {
            "<hidden>"
        } else {
            core::str::from_utf8(&self.ssid[..self.ssid_len]).unwrap_or("<invalid>")
        }
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

/// Wi-Fi authentication mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Open,
    WEP,
    WPA,
    WPA2,
    WPA3,
    WPA2WPA3,
}

impl fmt::Display for AuthMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthMode::Open => write!(f, "OPEN"),
            AuthMode::WEP => write!(f, "WEP"),
            AuthMode::WPA => write!(f, "WPA"),
            AuthMode::WPA2 => write!(f, "WPA2"),
            AuthMode::WPA3 => write!(f, "WPA3"),
            AuthMode::WPA2WPA3 => write!(f, "WPA2/3"),
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Maximum SSID length (IEEE 802.11).
pub const MAX_SSID_LEN: usize = 32;
/// Maximum passphrase length (WPA2).
pub const MAX_PASS_LEN: usize = 64;

/// Wi-Fi connection parameters.
pub struct WifiConfig {
    pub ssid: [u8; MAX_SSID_LEN],
    pub ssid_len: usize,
    pub password: [u8; MAX_PASS_LEN],
    pub pass_len: usize,
}

impl WifiConfig {
    /// Create an empty config.
    pub const fn empty() -> Self {
        Self {
            ssid: [0u8; MAX_SSID_LEN],
            ssid_len: 0,
            password: [0u8; MAX_PASS_LEN],
            pass_len: 0,
        }
    }

    /// Create a config from byte slices (truncates if too long).
    pub fn new(ssid: &[u8], password: &[u8]) -> Self {
        let mut cfg = Self::empty();
        cfg.ssid_len = ssid.len().min(MAX_SSID_LEN);
        cfg.pass_len = password.len().min(MAX_PASS_LEN);
        cfg.ssid[..cfg.ssid_len].copy_from_slice(&ssid[..cfg.ssid_len]);
        cfg.password[..cfg.pass_len].copy_from_slice(&password[..cfg.pass_len]);
        cfg
    }

    /// Returns `true` if an SSID has been configured.
    pub fn is_configured(&self) -> bool {
        self.ssid_len > 0
    }

    /// SSID as a string slice.
    pub fn ssid_str(&self) -> &str {
        core::str::from_utf8(&self.ssid[..self.ssid_len]).unwrap_or("<invalid>")
    }
}

// ---------------------------------------------------------------------------
// Wi-Fi state machine
// ---------------------------------------------------------------------------

/// Current state of the Wi-Fi subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiState {
    /// No SSID configured yet.
    Unconfigured,
    /// SSID/password set, not connected.
    Configured,
    /// Attempting to connect.
    Connecting,
    /// Associated with AP, IP obtained.
    Connected,
    /// Connection failed or lost.
    Disconnected,
}

impl fmt::Display for WifiState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WifiState::Unconfigured => write!(f, "unconfigured"),
            WifiState::Configured => write!(f, "configured (not connected)"),
            WifiState::Connecting => write!(f, "connecting..."),
            WifiState::Connected => write!(f, "connected"),
            WifiState::Disconnected => write!(f, "disconnected"),
        }
    }
}

// ---------------------------------------------------------------------------
// WifiManager — static config store + state machine
// ---------------------------------------------------------------------------

/// Manages Wi-Fi configuration and connection lifecycle.
///
/// Lives in a kernel static. The shell writes config via `set_credentials()`,
/// triggers connection via `connect()`, and reads state via `state()`.
pub struct WifiManager {
    config: WifiConfig,
    state: WifiState,
    /// The actual driver handle (initialised on first connect).
    driver: Esp32Wifi,
    /// IP address once connected (assigned via DHCP or static).
    pub ip: [u8; 4],
    /// Last scan results.
    scan_results: [ScanResult; MAX_SCAN_RESULTS],
    scan_count: usize,
}

impl WifiManager {
    pub const fn new() -> Self {
        Self {
            config: WifiConfig::empty(),
            state: WifiState::Unconfigured,
            driver: Esp32Wifi::new(),
            ip: [0; 4],
            scan_results: [const { ScanResult::empty() }; MAX_SCAN_RESULTS],
            scan_count: 0,
        }
    }

    /// Set the SSID and password. Does not connect yet.
    pub fn set_credentials(&mut self, ssid: &[u8], password: &[u8]) {
        self.config = WifiConfig::new(ssid, password);
        self.state = WifiState::Configured;
    }

    /// Attempt to connect using the stored credentials.
    ///
    /// On real hardware (with the `wifi` feature), this calls into the
    /// Espressif radio blobs. Without it, transitions to `Disconnected`
    /// with a descriptive error.
    pub fn connect(&mut self) -> Result<(), WifiError> {
        if !self.config.is_configured() {
            return Err(WifiError::NotConfigured);
        }

        self.state = WifiState::Connecting;

        match self.driver.init(&self.config) {
            Ok(()) => {
                self.state = WifiState::Connected;
                Ok(())
            }
            Err(e) => {
                self.state = WifiState::Disconnected;
                Err(e)
            }
        }
    }

    /// Disconnect from the current AP.
    pub fn disconnect(&mut self) {
        self.driver.connected = false;
        if self.config.is_configured() {
            self.state = WifiState::Configured;
        } else {
            self.state = WifiState::Unconfigured;
        }
    }

    /// Current Wi-Fi state.
    pub fn state(&self) -> WifiState {
        self.state
    }

    /// Reference to the stored config.
    pub fn config(&self) -> &WifiConfig {
        &self.config
    }

    /// Reference to the underlying driver (for passing to NetStack).
    pub fn driver(&self) -> &Esp32Wifi {
        &self.driver
    }

    /// Mutable reference to the driver (for polling).
    pub fn driver_mut(&mut self) -> &mut Esp32Wifi {
        &mut self.driver
    }

    /// Trigger a scan for nearby APs. Results stored in `scan_results`.
    pub fn scan(&mut self) -> Result<usize, WifiError> {
        let count = self.driver.scan(&mut self.scan_results)?;
        self.scan_count = count;
        Ok(count)
    }

    /// Number of APs found in the last scan.
    pub fn scan_count(&self) -> usize {
        self.scan_count
    }

    /// Write scan results to the given writer (for the `wifi list` command).
    pub fn write_scan_results(&self, w: &mut dyn fmt::Write) {
        if self.scan_count == 0 {
            let _ = writeln!(w, "  No scan results. Run 'wifi scan' first.");
            return;
        }
        let _ = writeln!(w, "  {:2}  {:<24} {:>4}  {:>3}  {:<6}  SIGNAL", "#", "SSID", "RSSI", "CH", "AUTH");
        let _ = writeln!(w, "  --  {:─<24} {:─>4}  {:─>3}  {:─<6}  {:─<4}", "", "", "", "", "");
        for (i, ap) in self.scan_results[..self.scan_count].iter().enumerate() {
            let _ = writeln!(
                w,
                "  {:2}  {:<24} {:>4}  {:>3}  {:<6}  {}",
                i + 1,
                ap.ssid_str(),
                ap.rssi,
                ap.channel,
                ap.auth,
                ap.signal_bars(),
            );
        }
    }

    /// Write status info to the given writer (for the `wifi status` command).
    pub fn write_status(&self, w: &mut dyn fmt::Write) {
        let _ = writeln!(w, "  Wi-Fi state : {}", self.state);
        if self.config.is_configured() {
            let _ = writeln!(w, "  SSID        : {}", self.config.ssid_str());
            let _ = writeln!(w, "  Password    : ********");
        } else {
            let _ = writeln!(w, "  SSID        : (none)");
        }
        if self.state == WifiState::Connected {
            let _ = writeln!(
                w,
                "  IP          : {}.{}.{}.{}",
                self.ip[0], self.ip[1], self.ip[2], self.ip[3]
            );
            let mac = self.driver.mac;
            let _ = writeln!(
                w,
                "  MAC         : {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Wi-Fi driver state
// ---------------------------------------------------------------------------

/// ESP32-C3 Wi-Fi driver — wraps the Espressif radio for VeerOS.
///
/// This struct holds the runtime state of the Wi-Fi connection.
/// It is meant to live in a static and be accessed by the network task.
pub struct Esp32Wifi {
    /// Cached MAC address.
    mac: [u8; 6],
    /// Whether we are associated with an AP.
    connected: bool,
    /// Internal RX buffer for one frame.
    rx_buf: [u8; 1514],
    rx_len: usize,
    rx_ready: bool,
}

/// Errors from the Wi-Fi subsystem.
#[derive(Debug, Clone, Copy)]
pub enum WifiError {
    /// Wi-Fi hardware not found or init failed.
    InitFailed,
    /// Could not associate with the configured SSID.
    ConnectionFailed,
    /// Feature not compiled in.
    NotAvailable,
    /// No SSID configured — call `set_credentials()` first.
    NotConfigured,
}

impl fmt::Display for WifiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WifiError::InitFailed => write!(f, "hardware init failed"),
            WifiError::ConnectionFailed => write!(f, "could not connect to AP"),
            WifiError::NotAvailable => write!(f, "wifi feature not enabled"),
            WifiError::NotConfigured => write!(f, "no SSID configured"),
        }
    }
}

impl Esp32Wifi {
    /// Create an uninitialised Wi-Fi handle.
    pub const fn new() -> Self {
        Self {
            mac: [0u8; 6],
            connected: false,
            rx_buf: [0u8; 1514],
            rx_len: 0,
            rx_ready: false,
        }
    }

    /// Initialise the Wi-Fi radio and connect to the configured AP.
    ///
    /// On real hardware this calls into the Espressif Wi-Fi blobs via
    /// `esp-wifi` / `esp-radio`.  The current build is a **stub** that
    /// returns `WifiError::NotAvailable` — the full implementation is
    /// activated when the `wifi` Cargo feature is enabled and the
    /// `esp-hal` + `esp-wifi` crates are present.
    ///
    /// # Requirements
    /// - Heap must be initialised (≥72 KiB for the Wi-Fi blobs).
    /// - SYSTIMER must be running (used for timeouts).
    pub fn init(&mut self, _config: &WifiConfig) -> Result<(), WifiError> {
        // ─── Real implementation (behind `wifi` feature) ─────
        //
        // When building with the `wifi` feature and esp-hal/esp-wifi:
        //
        //   1. Call esp_hal::init() for clock + peripheral setup
        //   2. Init esp_alloc heap (≥72 KiB)
        //   3. Init esp_wifi::wifi::new() with STA config
        //   4. Call controller.connect() (blocking)
        //   5. Read MAC from controller
        //   6. Start the internal RX polling loop
        //
        // For now, return NotAvailable on builds without the blob.

        Err(WifiError::NotAvailable)
    }

    /// Returns `true` if associated with an AP.
    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// Scan for nearby access points.
    ///
    /// Fills `results` with discovered APs and returns the count.
    /// On real hardware this calls esp-wifi's scan API.
    /// The stub returns a few fake APs for shell development/testing.
    pub fn scan(&self, results: &mut [ScanResult]) -> Result<usize, WifiError> {
        // ─── Stub: return synthetic scan results for testing ─────
        let fake_aps: &[(&[u8], [u8; 6], u8, i8, AuthMode)] = &[
            (b"VeerOS-Lab",     [0xAA, 0xBB, 0xCC, 0x11, 0x22, 0x33], 6, -42, AuthMode::WPA2),
            (b"HomeNetwork",    [0x10, 0x20, 0x30, 0x40, 0x50, 0x60], 1, -58, AuthMode::WPA2WPA3),
            (b"CoffeeShop",     [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01], 11, -71, AuthMode::WPA2),
            (b"OpenGuest",      [0x00, 0x11, 0x22, 0x33, 0x44, 0x55], 6, -80, AuthMode::Open),
        ];

        let count = fake_aps.len().min(results.len());
        for (i, &(ssid, bssid, ch, rssi, auth)) in fake_aps.iter().enumerate().take(count) {
            let mut r = ScanResult::empty();
            let len = ssid.len().min(MAX_SSID_LEN);
            r.ssid[..len].copy_from_slice(&ssid[..len]);
            r.ssid_len = len;
            r.bssid = bssid;
            r.channel = ch;
            r.rssi = rssi;
            r.auth = auth;
            results[i] = r;
        }

        Ok(count)
    }

    /// Poll the Wi-Fi driver for new received frames.
    ///
    /// Must be called frequently from the network task.
    /// On real hardware this drains the radio's RX queue.
    pub fn poll_rx(&mut self) {
        // Stub — real implementation reads from esp-wifi's internal queue.
    }
}

impl NetworkDevice for Esp32Wifi {
    fn mtu(&self) -> usize {
        1514
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
        // Note: in the real implementation, rx_ready/rx_len are cleared
        // via interior mutability (UnsafeCell) since NetworkDevice takes &self.
        len
    }

    fn send(&self, _buf: &[u8]) {
        // Stub — real implementation calls esp_wifi::wifi_transmit().
    }

    fn mac_address(&self) -> [u8; 6] {
        self.mac
    }
}

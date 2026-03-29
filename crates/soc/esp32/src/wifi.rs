//! ESP32-C6 Wi-Fi driver — bridges Espressif radio blobs to VeerOS.
//!
//! This module wraps the Espressif proprietary WiFi firmware blobs
//! (linked via `esp-wifi-sys`) and exposes them through VeerOS's
//! `arch::NetworkDevice` trait so that the `net` crate (smoltcp +
//! TcpSerial) works transparently over WiFi.
//!
//! ## Connection flow
//!
//! ```text
//! veeros> wifi set MyNetwork MyPassword
//! veeros> wifi connect
//! veeros> wifi status
//! ```
//!
//! Configuration is stored in a static `WifiManager`. Once Wi-Fi is
//! connected, the network task picks it up and starts TCP services.

use arch::NetworkDevice;
use core::ffi::c_void;
use core::fmt;
use core::ptr;

use esp_wifi_sys::include::{
    esp_interface_t_ESP_IF_WIFI_AP, esp_interface_t_ESP_IF_WIFI_STA,
    esp_wifi_connect_internal, esp_wifi_init_internal, esp_wifi_internal_free_rx_buffer,
    esp_wifi_internal_reg_rxcb, esp_wifi_internal_tx, esp_wifi_scan_get_ap_num,
    esp_wifi_scan_get_ap_records, esp_wifi_scan_start, esp_wifi_set_config,
    esp_wifi_set_mode, esp_wifi_set_tx_done_cb, esp_wifi_start, esp_supplicant_init,
    g_wifi_default_wpa_crypto_funcs, wifi_ap_record_t, wifi_config_t, wifi_init_config_t,
    wifi_interface_t_WIFI_IF_STA, wifi_mode_t_WIFI_MODE_NULL, wifi_mode_t_WIFI_MODE_STA, wifi_sta_config_t,
    ESP_OK, WIFI_INIT_CONFIG_MAGIC,
    wifi_auth_mode_t_WIFI_AUTH_OPEN, wifi_auth_mode_t_WIFI_AUTH_WEP,
    wifi_auth_mode_t_WIFI_AUTH_WPA_PSK, wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK,
    wifi_auth_mode_t_WIFI_AUTH_WPA3_PSK, wifi_auth_mode_t_WIFI_AUTH_WPA2_WPA3_PSK,
};

// ---------------------------------------------------------------------------
// RX ring buffer — filled by the blob's RX callback
// ---------------------------------------------------------------------------

const NUM_RX_DESC: usize = 8;
const FRAME_SIZE: usize = 1600;

/// RX ring — blob callback writes frames here, NetworkDevice reads them.
static mut RX_RING: [[u8; FRAME_SIZE]; NUM_RX_DESC] = [[0u8; FRAME_SIZE]; NUM_RX_DESC];
static mut RX_LEN: [usize; NUM_RX_DESC] = [0usize; NUM_RX_DESC];
static mut RX_READY: [bool; NUM_RX_DESC] = [false; NUM_RX_DESC];
static mut RX_WRITE: usize = 0;
static mut RX_READ: usize = 0;

/// RX callback registered with the blob via `esp_wifi_internal_reg_rxcb`.
///
/// Called from blob context whenever a WiFi frame is received.
/// We copy the payload into our ring buffer and release the blob's buffer.
static mut RX_CB_COUNT: u32 = 0;

unsafe extern "C" fn recv_cb_sta(
    buffer: *mut c_void,
    len: u16,
    eb: *mut c_void,
) -> i32 {
    unsafe { RX_CB_COUNT += 1; }
    let frame_len = len as usize;
    if frame_len > 0 && frame_len <= FRAME_SIZE {
        let wi = RX_WRITE;
        if !RX_READY[wi] {
            ptr::copy_nonoverlapping(buffer as *const u8, RX_RING[wi].as_mut_ptr(), frame_len);
            RX_LEN[wi] = frame_len;
            core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);
            RX_READY[wi] = true;
            RX_WRITE = (wi + 1) % NUM_RX_DESC;
        }
        // else: ring full, drop frame
    }
    esp_wifi_internal_free_rx_buffer(eb);
    0 // ESP_OK
}

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

/// Convert blob auth mode to our enum.
fn authmode_from_blob(m: u32) -> AuthMode {
    match m {
        x if x == wifi_auth_mode_t_WIFI_AUTH_OPEN => AuthMode::Open,
        x if x == wifi_auth_mode_t_WIFI_AUTH_WEP => AuthMode::WEP,
        x if x == wifi_auth_mode_t_WIFI_AUTH_WPA_PSK => AuthMode::WPA,
        x if x == wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK => AuthMode::WPA2,
        x if x == wifi_auth_mode_t_WIFI_AUTH_WPA3_PSK => AuthMode::WPA3,
        x if x == wifi_auth_mode_t_WIFI_AUTH_WPA2_WPA3_PSK => AuthMode::WPA2WPA3,
        _ => AuthMode::WPA2,
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
    Unconfigured,
    Configured,
    Connecting,
    Connected,
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
// WifiManager
// ---------------------------------------------------------------------------

pub struct WifiManager {
    config: WifiConfig,
    state: WifiState,
    driver: Esp32Wifi,
    pub ip: [u8; 4],
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

    pub fn set_credentials(&mut self, ssid: &[u8], password: &[u8]) {
        self.config = WifiConfig::new(ssid, password);
        self.state = WifiState::Configured;
    }

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

    pub fn disconnect(&mut self) {
        self.driver.connected = false;
        if self.config.is_configured() {
            self.state = WifiState::Configured;
        } else {
            self.state = WifiState::Unconfigured;
        }
    }

    pub fn state(&self) -> WifiState {
        self.state
    }

    pub fn config(&self) -> &WifiConfig {
        &self.config
    }

    pub fn driver(&self) -> &Esp32Wifi {
        &self.driver
    }

    pub fn driver_mut(&mut self) -> &mut Esp32Wifi {
        &mut self.driver
    }

    pub fn scan(&mut self) -> Result<usize, WifiError> {
        let count = self.driver.scan(&mut self.scan_results)?;
        self.scan_count = count;
        Ok(count)
    }

    pub fn scan_count(&self) -> usize {
        self.scan_count
    }

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
// Wi-Fi driver (blob-backed)
// ---------------------------------------------------------------------------

/// ESP32-C6 Wi-Fi driver backed by Espressif radio blobs.
pub struct Esp32Wifi {
    mac: [u8; 6],
    connected: bool,
    initialized: bool,
}

/// Errors from the Wi-Fi subsystem.
#[derive(Debug, Clone, Copy)]
pub enum WifiError {
    InitFailed,
    ConnectionFailed,
    NotAvailable,
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
    pub const fn new() -> Self {
        Self {
            mac: [0u8; 6],
            connected: false,
            initialized: false,
        }
    }

    /// Initialise the WiFi radio blobs and connect to the configured AP.
    pub fn init(&mut self, config: &WifiConfig) -> Result<(), WifiError> {
        use crate::modem;

        if self.initialized {
            // Already initialized — just connect (polled from driver task).
            let ret = unsafe { esp_wifi_connect_internal() };
            if ret != ESP_OK as i32 {
                return Err(WifiError::ConnectionFailed);
            }
            // esp_wifi_connect_internal() queued the connect command.
            // The WPA handshake happens asynchronously via interrupts + ppTask.
            // Yield to let ppTask process the handshake, then poll timers.
            for _ in 0..200u32 {
                crate::wifi_os_adapter::poll_timers();
                crate::wifi_os_adapter::yield_to_scheduler();
            }
            self.connected = true;
            return Ok(());
        }

        if !config.is_configured() {
            return Err(WifiError::NotConfigured);
        }

        // Step 1: Enable modem clocks and reset.
        modem::enable_all_clocks();
        modem::reset_all_modems();

        // Step 2: Read factory MAC from eFuse.
        self.mac = modem::read_efuse_mac();

        // Step 3: Ensure the OS adapter globals are set.
        unsafe {
            use crate::wifi_os_adapter::g_wifi_osi_funcs as local_osi;
            // Copy our OSI funcs to the blob's extern symbol.
            let dst = &raw mut esp_wifi_sys::include::g_wifi_osi_funcs;
            ptr::copy_nonoverlapping(&raw const local_osi, dst, 1);
            // Also set g_osi_funcs_p (ROM data pointer at fixed SRAM address).
            let g_osi_funcs_p = 0x4087ff6c as *mut *mut esp_wifi_sys::include::wifi_osi_funcs_t;
            core::ptr::write_volatile(g_osi_funcs_p, dst);
        }

        // Step 3b: Configure WiFi interrupt routing (INTMATRIX + PLIC).
        crate::wifi_os_adapter::setup_wifi_interrupts();

        // Step 3c: Early PHY calibration — run register_chipv7_phy now so
        // the blob's later phy_enable() from ppTask takes the fast wakeup path.
        crate::wifi_os_adapter::early_phy_init();

        // Step 4: Build wifi_init_config_t.
        let init_cfg = wifi_init_config_t {
            osi_funcs: unsafe { &raw mut esp_wifi_sys::include::g_wifi_osi_funcs },
            wpa_crypto_funcs: unsafe { g_wifi_default_wpa_crypto_funcs },
            static_rx_buf_num: 4,
            dynamic_rx_buf_num: 4,
            tx_buf_type: 0,           // static TX buffers
            static_tx_buf_num: 4,
            dynamic_tx_buf_num: 0,
            rx_mgmt_buf_type: 0,
            rx_mgmt_buf_num: 2,
            cache_tx_buf_num: 0,
            csi_enable: 0,
            ampdu_rx_enable: 0,
            ampdu_tx_enable: 0,
            amsdu_tx_enable: 0,
            nvs_enable: 0,
            nano_enable: 0,
            rx_ba_win: 0,
            wifi_task_core_id: 0,
            beacon_max_len: 752,
            mgmt_sbuf_num: 6,
            feature_caps: crate::wifi_os_adapter::WIFI_FEATURE_CAPS,
            sta_disconnected_pm: false,
            espnow_max_encrypt_num: 0,
            tx_hetb_queue_num: 3,
            dump_hesigb_enable: false,
            magic: WIFI_INIT_CONFIG_MAGIC as i32,
        };

        // Step 5: Initialize WiFi internals.
        let ret = unsafe { esp_wifi_init_internal(&init_cfg) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 6: Match esp-wifi init flow: force mode NULL first.
        let ret = unsafe { esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_NULL) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 7: Initialize supplicant (WPA2 handshake).
        let ret = unsafe { esp_supplicant_init() };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 8: Register RX callbacks and TX done callback.
        // Match esp-wifi: register callbacks for both STA and AP interfaces.
        let ret = unsafe { esp_wifi_internal_reg_rxcb(esp_interface_t_ESP_IF_WIFI_STA, Some(recv_cb_sta)) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }
        let ret = unsafe { esp_wifi_internal_reg_rxcb(esp_interface_t_ESP_IF_WIFI_AP, Some(recv_cb_sta)) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }
        let ret = unsafe { esp_wifi_set_tx_done_cb(None) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 9: Set STA mode.
        let ret = unsafe { esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_STA) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 10: Configure STA with SSID and password.
        let mut sta_cfg: wifi_sta_config_t = unsafe { core::mem::zeroed() };
        sta_cfg.ssid[..config.ssid_len].copy_from_slice(&config.ssid[..config.ssid_len]);
        sta_cfg.password[..config.pass_len].copy_from_slice(&config.password[..config.pass_len]);

        let mut wifi_cfg = wifi_config_t { sta: sta_cfg };
        let ret = unsafe {
            esp_wifi_set_config(wifi_interface_t_WIFI_IF_STA, &mut wifi_cfg)
        };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 11: Start WiFi.
        let ret = unsafe { esp_wifi_start() };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        self.initialized = true;

        // Defer connect until a subsequent call so start can settle asynchronously.
        Err(WifiError::ConnectionFailed)
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn rx_cb_count(&self) -> u32 {
        unsafe { RX_CB_COUNT }
    }

    /// Scan for nearby access points using the blob's scan API.
    pub fn scan(&self, results: &mut [ScanResult]) -> Result<usize, WifiError> {
        // Start a blocking scan (pass null config for default params).
        let ret = unsafe { esp_wifi_scan_start(ptr::null(), true) };
        if ret != ESP_OK as i32 {
            return Ok(0);
        }

        let mut ap_count: u16 = 0;
        unsafe { esp_wifi_scan_get_ap_num(&mut ap_count) };
        if ap_count == 0 {
            return Ok(0);
        }

        let max = ap_count.min(results.len() as u16).min(MAX_SCAN_RESULTS as u16);
        let mut records: [wifi_ap_record_t; MAX_SCAN_RESULTS] =
            unsafe { core::mem::zeroed() };
        let mut num = max;
        unsafe { esp_wifi_scan_get_ap_records(&mut num, records.as_mut_ptr()) };

        let count = num as usize;
        for i in 0..count {
            let r = &records[i];
            let mut sr = ScanResult::empty();
            // Copy SSID — find null terminator.
            let ssid_len = r.ssid.iter().position(|&b| b == 0).unwrap_or(33).min(MAX_SSID_LEN);
            sr.ssid[..ssid_len].copy_from_slice(&r.ssid[..ssid_len]);
            sr.ssid_len = ssid_len;
            sr.bssid = r.bssid;
            sr.channel = r.primary;
            sr.rssi = r.rssi;
            sr.auth = authmode_from_blob(r.authmode);
            results[i] = sr;
        }

        Ok(count)
    }

    /// Poll timers for the blob — call from the network task loop.
    pub fn poll_rx(&mut self) {
        crate::wifi_os_adapter::poll_timers();
        // Run blob tasks (ppTask, etc.) cooperatively.
        for i in 0..4 {
            crate::wifi_os_adapter::poll_blob_task(i, 1000);
        }
    }
}

impl NetworkDevice for Esp32Wifi {
    fn mtu(&self) -> usize {
        FRAME_SIZE
    }

    fn has_rx(&self) -> bool {
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Acquire);
        unsafe { RX_READY[RX_READ] }
    }

    fn recv(&self, buf: &mut [u8]) -> usize {
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Acquire);
        let ri = unsafe { RX_READ };
        if !unsafe { RX_READY[ri] } || unsafe { RX_LEN[ri] } == 0 {
            return 0;
        }
        let len = unsafe { RX_LEN[ri] }.min(buf.len());
        buf[..len].copy_from_slice(unsafe { &RX_RING[ri][..len] });
        unsafe {
            RX_READY[ri] = false;
            RX_LEN[ri] = 0;
            RX_READ = (ri + 1) % NUM_RX_DESC;
        }
        len
    }

    fn send(&self, buf: &[u8]) {
        if !self.connected || buf.is_empty() {
            return;
        }
        unsafe {
            esp_wifi_internal_tx(
                wifi_interface_t_WIFI_IF_STA,
                buf.as_ptr() as *mut c_void,
                buf.len() as u16,
            );
        }
    }

    fn mac_address(&self) -> [u8; 6] {
        self.mac
    }
}

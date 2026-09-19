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
use core::fmt::{self, Write};
use core::ptr;

#[cfg(feature = "c6")]
use esp_wifi_sys_esp32c6 as esp_wifi_sys;
use esp_wifi_sys::include::{
    esp_interface_t_ESP_IF_WIFI_AP, esp_interface_t_ESP_IF_WIFI_STA, esp_supplicant_init,
    esp_wifi_connect_internal, esp_wifi_init_internal, esp_wifi_internal_free_rx_buffer,
    esp_wifi_internal_reg_rxcb, esp_wifi_internal_tx, esp_wifi_scan_get_ap_num,
    esp_wifi_scan_get_ap_records, esp_wifi_scan_start, esp_wifi_set_config,
    esp_wifi_set_country, esp_wifi_set_mode, esp_wifi_set_protocol,
    esp_wifi_set_ps, esp_wifi_set_tx_done_cb, esp_wifi_start, g_wifi_default_wpa_crypto_funcs,
    wifi_ap_record_t, wifi_auth_mode_t_WIFI_AUTH_OPEN, wifi_auth_mode_t_WIFI_AUTH_WEP,
    wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK, wifi_auth_mode_t_WIFI_AUTH_WPA2_WPA3_PSK,
    wifi_auth_mode_t_WIFI_AUTH_WPA3_PSK, wifi_auth_mode_t_WIFI_AUTH_WPA_PSK, wifi_config_t,
    wifi_country_policy_t_WIFI_COUNTRY_POLICY_MANUAL, wifi_country_t, wifi_init_config_t,
    wifi_interface_t_WIFI_IF_STA, wifi_mode_t_WIFI_MODE_NULL, wifi_mode_t_WIFI_MODE_STA,
    wifi_pmf_config_t, wifi_ps_type_t_WIFI_PS_NONE,
    wifi_sae_pwe_method_t_WPA3_SAE_PWE_BOTH, wifi_scan_config_t,
    wifi_scan_type_t_WIFI_SCAN_TYPE_ACTIVE, wifi_sta_config_t, ESP_OK, WIFI_INIT_CONFIG_MAGIC,
    WIFI_PROTOCOL_11B, WIFI_PROTOCOL_11G, WIFI_PROTOCOL_11N,
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
static mut RX_CB_ACCEPT_COUNT: u32 = 0;
static mut RX_CB_DROP_RING_FULL_COUNT: u32 = 0;
static mut RX_CB_BAD_LEN_COUNT: u32 = 0;
static mut RX_CB_FREE_COUNT: u32 = 0;
static mut RX_CB_REG_COUNT: u32 = 0;
static mut RX_CB_REG_LAST_STA_RET: i32 = 0;
static mut RX_CB_REG_LAST_AP_RET: i32 = 0;
static mut TX_DONE_COUNT: u32 = 0;

fn wifi_usb_trace(msg: &[u8]) {
    use arch::Serial;
    let usb = crate::usb_serial_jtag::UsbSerialJtag::new();
    for b in msg {
        usb.write_byte(*b);
    }
}

#[inline(never)]
fn clear_apm_after_start() {
    let (hp, lp0, lp) = crate::modem::disable_apm_filters();
    wifi_usb_hex(b"[wifi] apmhp=", hp);
    wifi_usb_hex(b"[wifi] apmlp0=", lp0);
    wifi_usb_hex(b"[wifi] apmlp=", lp);
}

fn wifi_usb_hex(tag: &[u8], v: u32) {
    const H: &[u8] = b"0123456789abcdef";
    let mut buf = [0u8; 10];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        buf[2 + i] = H[((v >> (28 - i * 4)) & 0xf) as usize];
    }
    wifi_usb_trace(tag);
    wifi_usb_trace(&buf);
    wifi_usb_trace(b"\n");
}

unsafe extern "C" fn recv_cb_sta(buffer: *mut c_void, len: u16, eb: *mut c_void) -> i32 {
    unsafe {
        RX_CB_COUNT += 1;
    }
    let frame_len = len as usize;
    if frame_len > 0 && frame_len <= FRAME_SIZE && !buffer.is_null() {
        let wi = RX_WRITE;
        if !RX_READY[wi] {
            ptr::copy_nonoverlapping(buffer as *const u8, RX_RING[wi].as_mut_ptr(), frame_len);
            RX_LEN[wi] = frame_len;
            core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::Release);
            RX_READY[wi] = true;
            RX_WRITE = (wi + 1) % NUM_RX_DESC;
            RX_CB_ACCEPT_COUNT += 1;
        } else {
            RX_CB_DROP_RING_FULL_COUNT += 1;
        }
    } else {
        RX_CB_BAD_LEN_COUNT += 1;
    }
    esp_wifi_internal_free_rx_buffer(eb);
    RX_CB_FREE_COUNT += 1;
    0 // ESP_OK
}

/// Gold `wifi_init` registers `Some(esp_wifi_tx_done_cb)`, never `None`.
/// The blob uses the hook to recycle TX state after probe/data frames.
unsafe extern "C" fn tx_done_cb(
    _ifidx: u8,
    _data: *mut u8,
    _data_len: *mut u16,
    _tx_status: bool,
) {
    unsafe {
        TX_DONE_COUNT = TX_DONE_COUNT.wrapping_add(1);
    }
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
        let _ = writeln!(
            w,
            "  {:2}  {:<24} {:>4}  {:>3}  {:<6}  SIGNAL",
            "#", "SSID", "RSSI", "CH", "AUTH"
        );
        let _ = writeln!(
            w,
            "  --  {:─<24} {:─>4}  {:─>3}  {:─<6}  {:─<4}",
            "", "", "", "", ""
        );
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
            // Handshake is asynchronous (MAC ISR + ppTask). Wait for
            // WIFI_EVENT_STA_CONNECTED instead of assuming success.
            for _ in 0..400u32 {
                crate::wifi_os_adapter::yield_to_scheduler();
                if crate::wifi_os_adapter::wifi_sta_got_connected() {
                    self.connected = true;
                    return Ok(());
                }
            }
            return Err(WifiError::ConnectionFailed);
        }

        if !config.is_configured() {
            return Err(WifiError::NotConfigured);
        }

        // Gold RadioRefGuard::init() only runs init_radio_clocks (wifipwr).
        // BB/MAC clocks come from the blob's wifi_clock_enable; I2C+cal from
        // phy_enable. Enabling those here, then pulsing MAC, was not gold.
        // Analog BBPLL (esp_hal::init / enable_pll_clk_impl) is the missing
        // C6 side effect: VeerOS used to only read SOC_CLK_SEL.
        crate::gpio::enable_xiao_onboard_antenna();
        wifi_usb_hex(b"[wifi] bbpll=", modem::enable_bbpll());
        wifi_usb_hex(b"[wifi] cpuf=", modem::cpu_freq_conf());
        wifi_usb_hex(b"[wifi] dig13=", u32::from(modem::digreg_xpd()));
        modem::init_radio_clocks();
        crate::gpio::enable_xiao_onboard_antenna();

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

        // Gold `wifi::new` enables the internal event mask before init.
        crate::wifi_os_adapter::enable_wifi_events();
        wifi_usb_trace(b"[wifi] init_internal\n");

        // Step 4: Build wifi_init_config_t.
        let init_cfg = wifi_init_config_t {
            osi_funcs: unsafe { &raw mut esp_wifi_sys::include::g_wifi_osi_funcs },
            wpa_crypto_funcs: unsafe { g_wifi_default_wpa_crypto_funcs },
            // Match esp-radio 0.18 / esp-wifi-sys C6 blob compile-time config.
            // tx_buf_type MUST be 1 (dynamic); the blob is built with
            // CONFIG_ESP_WIFI_TX_BUFFER_TYPE=1. Static TX (0) leaves MAC RX
            // descriptor slots 0x600a408c–4094 empty.
            static_rx_buf_num: 10,
            dynamic_rx_buf_num: 32,
            tx_buf_type: 1,
            static_tx_buf_num: 0,
            dynamic_tx_buf_num: 32,
            rx_mgmt_buf_type: 0,
            rx_mgmt_buf_num: 5,
            cache_tx_buf_num: 0,
            csi_enable: 0,
            ampdu_rx_enable: 1,
            ampdu_tx_enable: 1,
            amsdu_tx_enable: 0,
            nvs_enable: 0,
            nano_enable: 0,
            rx_ba_win: 6,
            wifi_task_core_id: 0,
            beacon_max_len: 752,
            mgmt_sbuf_num: 32,
            feature_caps: crate::wifi_os_adapter::WIFI_FEATURE_CAPS,
            sta_disconnected_pm: false,
            espnow_max_encrypt_num: 7,
            tx_hetb_queue_num: 3,
            dump_hesigb_enable: false,
            magic: WIFI_INIT_CONFIG_MAGIC as i32,
        };

        // Step 5: Initialize WiFi internals.
        let ret = unsafe { esp_wifi_init_internal(&init_cfg) };
        wifi_usb_trace(b"[wifi] init_internal done\n");
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Gold wifi_init: NULL mode + supplicant + rxcb, then country/ps.
        wifi_usb_trace(b"[wifi] set_mode NULL\n");
        let ret = unsafe { esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_NULL) };
        wifi_usb_trace(b"[wifi] set_mode NULL done\n");
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 7: Initialize supplicant (WPA2 handshake).
        wifi_usb_trace(b"[wifi] supplicant\n");
        let ret = unsafe { esp_supplicant_init() };
        wifi_usb_trace(b"[wifi] supplicant done\n");
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 8: Register RX callbacks and TX done callback.
        wifi_usb_trace(b"[wifi] rxcb\n");
        // Match esp-wifi: register callbacks for both STA and AP interfaces.
        let ret = unsafe {
            esp_wifi_internal_reg_rxcb(esp_interface_t_ESP_IF_WIFI_STA, Some(recv_cb_sta))
        };
        unsafe {
            RX_CB_REG_COUNT = RX_CB_REG_COUNT.wrapping_add(1);
            RX_CB_REG_LAST_STA_RET = ret;
        }
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }
        let ret = unsafe {
            esp_wifi_internal_reg_rxcb(esp_interface_t_ESP_IF_WIFI_AP, Some(recv_cb_sta))
        };
        unsafe {
            RX_CB_REG_COUNT = RX_CB_REG_COUNT.wrapping_add(1);
            RX_CB_REG_LAST_AP_RET = ret;
        }
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }
        let ret = unsafe { esp_wifi_set_tx_done_cb(Some(tx_done_cb)) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Gold: country + PS_NONE after wifi_init, before set_config/start.
        let country = wifi_country_t {
            cc: [b'C' as _, b'N' as _, 0],
            schan: 1,
            nchan: 13,
            max_tx_power: 20,
            policy: wifi_country_policy_t_WIFI_COUNTRY_POLICY_MANUAL,
        };
        wifi_usb_trace(b"[wifi] set_country\n");
        let ret = unsafe { esp_wifi_set_country(&country) };
        wifi_usb_trace(b"[wifi] set_country done\n");
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }
        let ret = unsafe { esp_wifi_set_ps(wifi_ps_type_t_WIFI_PS_NONE) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 9: Set STA mode.
        wifi_usb_trace(b"[wifi] set_mode STA\n");
        let ret = unsafe { esp_wifi_set_mode(wifi_mode_t_WIFI_MODE_STA) };
        wifi_usb_trace(b"[wifi] set_mode STA done\n");
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 10: Configure STA with SSID and password.
        wifi_usb_trace(b"[wifi] set_config\n");
        let mut sta_cfg: wifi_sta_config_t = unsafe { core::mem::zeroed() };
        sta_cfg.ssid[..config.ssid_len].copy_from_slice(&config.ssid[..config.ssid_len]);
        sta_cfg.password[..config.pass_len].copy_from_slice(&config.password[..config.pass_len]);
        // Gold `apply_sta_config` (StationConfig::default): Fast scan,
        // sort-by-signal, WPA2 threshold, SAE PWE both, 1 retry.
        sta_cfg.scan_method = 0;
        sta_cfg.listen_interval = 3;
        sta_cfg.sort_method = 0;
        sta_cfg.threshold.rssi = -99;
        sta_cfg.threshold.authmode = wifi_auth_mode_t_WIFI_AUTH_WPA2_PSK;
        sta_cfg.pmf_cfg = wifi_pmf_config_t {
            capable: true,
            required: false,
        };
        sta_cfg.sae_pwe_h2e = wifi_sae_pwe_method_t_WPA3_SAE_PWE_BOTH;
        sta_cfg.failure_retry_cnt = 1;

        let mut wifi_cfg = wifi_config_t { sta: sta_cfg };
        let ret = unsafe { esp_wifi_set_config(wifi_interface_t_WIFI_IF_STA, &mut wifi_cfg) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Gold default Protocols: 11b|11g|11n, applied before start.
        let proto = (WIFI_PROTOCOL_11B | WIFI_PROTOCOL_11G | WIFI_PROTOCOL_11N) as u8;
        let ret = unsafe { esp_wifi_set_protocol(wifi_interface_t_WIFI_IF_STA, proto) };
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        // Step 11: Start WiFi.
        wifi_usb_trace(b"[wifi] start\n");
        let ret = unsafe { esp_wifi_start() };
        wifi_usb_trace(b"[wifi] start done\n");
        clear_apm_after_start();
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }
        for _ in 0..8 {
            crate::wifi_os_adapter::yield_to_scheduler();
        }
        // Gold AFTER_NEW keeps phy_i2c_init1's RFCAL 0x6b[2]=0x72 and
        // RF63 0x63[4]=0x33. bb_init → tx_cap_init_loop leaves 0x52 here.
        restore_gold_rf_i2c();
        extern "C" {
            fn rfrx_sat_rst(on: u32);
            fn phy_check_rx_sat();
            fn rfpll_cap_correct_new(log: u32);
        }
        unsafe {
            rfrx_sat_rst(1);
            phy_check_rx_sat();
            rfpll_cap_correct_new(0);
        }
        let (p1, p5, p12) = rfpll_snap();
        unsafe {
            PLL_HOLD_1 = p1;
            PLL_HOLD_5 = p5;
        }
        wifi_usb_hex(b"[wifi] pll621=", u32::from(p1));
        wifi_usb_hex(b"[wifi] pll625=", u32::from(p5));
        wifi_usb_hex(b"[wifi] pll62c=", u32::from(p12));
        // Gold AFTER_NEW already has a live TSF (44c8≠0, 44cc=0x40) and
        // 408c moving. We never touch 0x600AD000; 44cc still has bit12.
        let (t0, t1, tlo, thi) = tsf_diag();
        wifi_usb_hex(b"[wifi] tsf50=", t0);
        wifi_usb_hex(b"[wifi] tsf58=", t1);
        wifi_usb_hex(b"[wifi] tsflo=", tlo);
        wifi_usb_hex(b"[wifi] tsfhi=", thi);
        let (c4, c8, cc, d4, c74) = mac_time_diag();
        wifi_usb_hex(b"[wifi] 44c4=", c4);
        wifi_usb_hex(b"[wifi] 44c8=", c8);
        wifi_usb_hex(b"[wifi] 44cc=", cc);
        wifi_usb_hex(b"[wifi] 44d4=", d4);
        wifi_usb_hex(b"[wifi] 4c74=", c74);
        arm_mac_tsf();
        let (t0, t1, tlo, thi) = tsf_diag();
        wifi_usb_hex(b"[wifi] tsf50b=", t0);
        wifi_usb_hex(b"[wifi] tsf58b=", t1);
        wifi_usb_hex(b"[wifi] tsflo2=", tlo);
        wifi_usb_hex(b"[wifi] tsfhi2=", thi);
        let (c4, c8, cc, d4, c74) = mac_time_diag();
        wifi_usb_hex(b"[wifi] 44c4b=", c4);
        wifi_usb_hex(b"[wifi] 44c8b=", c8);
        wifi_usb_hex(b"[wifi] 44ccb=", cc);
        wifi_usb_hex(b"[wifi] 44d4b=", d4);
        wifi_usb_hex(b"[wifi] 4c74b=", c74);
        let rxdesc = unsafe { core::ptr::read_volatile(0x600A_408C as *const u32) };
        if rxdesc != 0 {
            wifi_usb_trace(b"[wifi] rxdesc live\n");
        } else {
            wifi_usb_trace(b"[wifi] rxdesc still 0\n");
        }

        // Some firmware paths can reset callback hooks during start/mode transitions.
        let ret = unsafe {
            esp_wifi_internal_reg_rxcb(esp_interface_t_ESP_IF_WIFI_STA, Some(recv_cb_sta))
        };
        unsafe {
            RX_CB_REG_COUNT = RX_CB_REG_COUNT.wrapping_add(1);
            RX_CB_REG_LAST_STA_RET = ret;
        }
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }
        let ret = unsafe {
            esp_wifi_internal_reg_rxcb(esp_interface_t_ESP_IF_WIFI_AP, Some(recv_cb_sta))
        };
        unsafe {
            RX_CB_REG_COUNT = RX_CB_REG_COUNT.wrapping_add(1);
            RX_CB_REG_LAST_AP_RET = ret;
        }
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }

        crate::gpio::enable_xiao_onboard_antenna();
        crate::modem::enable_phy_clock();
        snapshot_rx_probe();
        wifi_usb_hex(
            b"[wifi] calret=",
            crate::wifi_os_adapter::phy_cal_ret() as u32,
        );

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

    pub fn rx_diag(&self) -> (u32, u32, u32, u32, u32) {
        unsafe {
            (
                RX_CB_COUNT,
                RX_CB_ACCEPT_COUNT,
                RX_CB_DROP_RING_FULL_COUNT,
                RX_CB_BAD_LEN_COUNT,
                RX_CB_FREE_COUNT,
            )
        }
    }

    pub fn rx_reg_diag(&self) -> (u32, i32, i32) {
        unsafe {
            (
                RX_CB_REG_COUNT,
                RX_CB_REG_LAST_STA_RET,
                RX_CB_REG_LAST_AP_RET,
            )
        }
    }

    /// Scan for nearby access points using the blob's scan API.
    pub fn scan(&self, results: &mut [ScanResult]) -> Result<usize, WifiError> {
        // Gold: active scan, min 120 ms / max 150 ms per channel, 1 ms OSI tick.
        let mut cfg: wifi_scan_config_t = unsafe { core::mem::zeroed() };
        cfg.show_hidden = true;
        cfg.scan_type = wifi_scan_type_t_WIFI_SCAN_TYPE_ACTIVE;
        cfg.scan_time.active.min = 120;
        cfg.scan_time.active.max = 150;
        // Gold mid-scan walks RFPLL / 0x63[4] per channel. Holding those
        // here blocked retune so SCAN_DONE never posted (ev_last stayed 2).
        let ret = unsafe { esp_wifi_scan_start(&cfg, false) };
        wifi_usb_hex(b"[wifi] scan_start=", ret as u32);
        if ret != ESP_OK as i32 {
            return Err(WifiError::InitFailed);
        }
        let deadline = crate::systimer::now_us().saturating_add(20_000 * 1000);
        loop {
            if crate::wifi_os_adapter::wait_wifi_event(1, 20) {
                wifi_usb_trace(b"[wifi] scan_done\n");
                break;
            }
            if crate::systimer::now_us() >= deadline {
                wifi_usb_trace(b"[wifi] scan_timeout\n");
                return Ok(0);
            }
        }

        let mut ap_count: u16 = 0;
        unsafe { esp_wifi_scan_get_ap_num(&mut ap_count) };
        wifi_usb_hex(b"[wifi] ap_count=", u32::from(ap_count));
        if ap_count == 0 {
            return Ok(0);
        }

        let max = ap_count
            .min(results.len() as u16)
            .min(MAX_SCAN_RESULTS as u16);
        let mut records: [wifi_ap_record_t; MAX_SCAN_RESULTS] = unsafe { core::mem::zeroed() };
        let mut num = max;
        unsafe { esp_wifi_scan_get_ap_records(&mut num, records.as_mut_ptr()) };

        let count = num as usize;
        for i in 0..count {
            let r = &records[i];
            let mut sr = ScanResult::empty();
            // Copy SSID — find null terminator.
            let ssid_len = r
                .ssid
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(33)
                .min(MAX_SSID_LEN);
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
        // Timers fire on the dedicated wifi-tmr task (gold timer worker).
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

static mut RX_PROBE_DESC: [u32; 3] = [0; 3];
static mut RX_PROBE_BUF: [u8; 32] = [0; 32];
static mut RX_PROBE_DESC_MUT: u32 = 0;
static mut RX_PROBE_BUF_MUT: u32 = 0;
static mut RX_PROBE_ARMED: bool = false;
static mut RX_PROBE_MACINT_OR: u32 = 0;

fn rx_desc_base() -> usize {
    let slot = unsafe { core::ptr::read_volatile(0x600A_4084 as *const u32) };
    0x4080_0000 | (slot & 0x00FF_FFFF) as usize
}

fn snapshot_rx_probe() {
    let addr = rx_desc_base();
    if addr < 0x4080_0000 || addr >= 0x4088_0000 {
        return;
    }
    unsafe {
        for i in 0..3 {
            RX_PROBE_DESC[i] = core::ptr::read_volatile((addr as *const u32).add(i));
        }
        let buf = RX_PROBE_DESC[1] as usize;
        if buf >= 0x4080_0000 && buf < 0x4088_0000 {
            ptr::copy_nonoverlapping(buf as *const u8, RX_PROBE_BUF.as_mut_ptr(), 32);
        }
        RX_PROBE_ARMED = true;
    }
}

/// `4c04` is the enable mask gold keeps at `0x006d0000` through config.
pub fn mac_int_diag() -> (u32, u32, u32) {
    unsafe {
        (
            core::ptr::read_volatile(0x600A_4C04 as *const u32),
            core::ptr::read_volatile(0x600A_4C40 as *const u32),
            core::ptr::read_volatile(0x600A_4C48 as *const u32),
        )
    }
}

/// AGC window the sat/gain path uses (`0x600a708c` energy, `0x600a785c` live).
pub fn bb_agc_diag() -> (u32, u32, u32) {
    unsafe {
        (
            core::ptr::read_volatile(0x600A_705C as *const u32),
            core::ptr::read_volatile(0x600A_708C as *const u32),
            core::ptr::read_volatile(0x600A_785C as *const u32),
        )
    }
}

/// `(desc_mut, buf_mut, rxdesc, macint_or)` — Case A/B/C for the RX probe.
pub fn poll_rx_probe() -> (u32, u32, u32, u32) {
    let rxdesc = unsafe { core::ptr::read_volatile(0x600A_408C as *const u32) };
    let ev = unsafe { core::ptr::read_volatile(0x600A_4C48 as *const u32) };
    unsafe {
        RX_PROBE_MACINT_OR |= ev;
    }
    if unsafe { !RX_PROBE_ARMED } {
        return (0, 0, rxdesc, unsafe { RX_PROBE_MACINT_OR });
    }
    let addr = rx_desc_base();
    if addr < 0x4080_0000 || addr >= 0x4088_0000 {
        return (
            unsafe { RX_PROBE_DESC_MUT },
            unsafe { RX_PROBE_BUF_MUT },
            rxdesc,
            unsafe { RX_PROBE_MACINT_OR },
        );
    }
    unsafe {
        let mut desc_now = [0u32; 3];
        for i in 0..3 {
            desc_now[i] = core::ptr::read_volatile((addr as *const u32).add(i));
        }
        if desc_now != RX_PROBE_DESC {
            RX_PROBE_DESC_MUT = RX_PROBE_DESC_MUT.wrapping_add(1);
            RX_PROBE_DESC = desc_now;
        }
        let buf = desc_now[1] as usize;
        if buf >= 0x4080_0000 && buf < 0x4088_0000 {
            let mut now = [0u8; 32];
            ptr::copy_nonoverlapping(buf as *const u8, now.as_mut_ptr(), 32);
            if now != RX_PROBE_BUF {
                RX_PROBE_BUF_MUT = RX_PROBE_BUF_MUT.wrapping_add(1);
                RX_PROBE_BUF = now;
            }
        }
        (
            RX_PROBE_DESC_MUT,
            RX_PROBE_BUF_MUT,
            rxdesc,
            RX_PROBE_MACINT_OR,
        )
    }
}

/// MAC 0x600A4000 (512 words) + BB 0x600A7800 (128) + FE / SYSCON / LPCON.
pub fn dump_mac_bb(w: &mut dyn fmt::Write) {
    dump_regs(w, "MAC", 0x600A_4000, 512);
    dump_regs(w, "MACINT", 0x600A_4C00, 64);
    dump_regs(w, "AGC", 0x600A_7000, 128);
    dump_regs(w, "BB", 0x600A_7800, 128);
    dump_regs(w, "FE", 0x600A_8000, 256);
    dump_regs(w, "PHYA", 0x600A_9800, 128);
    dump_regs(w, "LPCON", 0x600A_F000, 32);
    dump_regs(w, "ZB", 0x600A_3000, 32);
    dump_regs(w, "TSF", 0x600A_D000, 32);
    let slot = unsafe { core::ptr::read_volatile(0x600A_4084 as *const u32) };
    let addr = 0x4080_0000 | (slot & 0x00FF_FFFF) as usize;
    let _ = writeln!(w, "DESCRING slot={:#010x} addr={:#010x}", slot, addr);
    dump_regs(w, "DESC", addr, 32);
    let buf = unsafe { core::ptr::read_volatile((addr + 4) as *const u32) } as usize;
    if buf >= 0x4080_0000 && buf < 0x4088_0000 {
        dump_regs(w, "RXBUF", buf, 16);
    }
    let r42f4 = unsafe { core::ptr::read_volatile(0x600A_42F4 as *const u32) };
    let _ = writeln!(w, "42f4={:#010x}", r42f4);
    crate::modem::dump_analog(w);
    dump_phy_rf_i2c(w);
}

fn phy_rom_i2c() -> Option<(extern "C" fn(u32, u32, u32) -> u32, extern "C" fn(u32, u32, u32, u32))> {
    extern "C" {
        fn phy_get_romfuncs() -> *const u32;
    }
    let funs = unsafe { phy_get_romfuncs() };
    if funs.is_null() {
        return None;
    }
    let rd = unsafe { *funs.add(0x50 / 4) };
    let wr = unsafe { *funs.add(0x58 / 4) };
    if rd == 0 || wr == 0 {
        return None;
    }
    Some(unsafe {
        (
            core::mem::transmute(rd),
            core::mem::transmute(wr),
        )
    })
}

/// Snap RFCAL[2] / RF63[4] without writing.
pub fn rf_i2c_snap() -> (u8, u8) {
    match phy_rom_i2c() {
        Some((read, _)) => (read(0x6b, 1, 2) as u8, read(0x63, 1, 4) as u8),
        None => (0, 0),
    }
}

/// RFPLL `0x62[1]`, `[5]`, `[12]` — gold stays `c3`/`c3`/`00`; we have `b2`/`b2`/`04`.
pub fn rfpll_snap() -> (u8, u8, u8) {
    match phy_rom_i2c() {
        Some((read, _)) => (
            read(0x62, 1, 1) as u8,
            read(0x62, 1, 5) as u8,
            read(0x62, 1, 0xc) as u8,
        ),
        None => (0, 0, 0),
    }
}

static mut PLL_HOLD_1: u8 = 0;
static mut PLL_HOLD_5: u8 = 0;

/// Gold `0x63[4]` stays `0x33`. Write only that byte — no `phy_i2c_init1`.
pub fn hold_rf63() -> u8 {
    match phy_rom_i2c() {
        Some((read, write)) => {
            write(0x63, 1, 4, 0x33);
            read(0x63, 1, 4) as u8
        }
        None => 0,
    }
}

/// Write the post-`rfpll_cap_correct_new` caps back. Does not call `phy_i2c_init1`.
pub fn hold_rfpll() -> (u8, u8, u8) {
    let (c1, c5) = unsafe { (PLL_HOLD_1, PLL_HOLD_5) };
    if c1 == 0 && c5 == 0 {
        return rfpll_snap();
    }
    if let Some((read, write)) = phy_rom_i2c() {
        write(0x62, 1, 1, u32::from(c1));
        write(0x62, 1, 5, u32::from(c5));
        write(0x62, 1, 0xc, 0);
        (
            read(0x62, 1, 1) as u8,
            read(0x62, 1, 5) as u8,
            read(0x62, 1, 0xc) as u8,
        )
    } else {
        (0, 0, 0)
    }
}

/// TSF window at `0x600AD000`. Enable bits are 31 of +0x50 / +0x58;
/// time is latched from +0x20 / +0x24 via `hal_mac_tsf_get_time`.
pub fn tsf_diag() -> (u32, u32, u32, u32) {
    unsafe {
        (
            core::ptr::read_volatile(0x600A_D050 as *const u32),
            core::ptr::read_volatile(0x600A_D058 as *const u32),
            core::ptr::read_volatile(0x600A_D020 as *const u32),
            core::ptr::read_volatile(0x600A_D024 as *const u32),
        )
    }
}

/// Gold AFTER_NEW on this chip: `44c4` live (`520d4800`→`0e685800`),
/// `44c8=00800000`, `44d4=e9a10000`. VeerOS left `44c4=0`.
/// No blob store to `44c4` — it is a HW counter.
pub fn mac_time_diag() -> (u32, u32, u32, u32, u32) {
    unsafe {
        (
            core::ptr::read_volatile(0x600A_44C4 as *const u32),
            core::ptr::read_volatile(0x600A_44C8 as *const u32),
            core::ptr::read_volatile(0x600A_44CC as *const u32),
            core::ptr::read_volatile(0x600A_44D4 as *const u32),
            core::ptr::read_volatile(0x600A_4C74 as *const u32),
        )
    }
}

/// Turn on both TSF ports the blob uses, drop the extra `44cc` bit12
/// that gold AFTER_NEW does not have, and clear the stray `4c74`.
pub fn arm_mac_tsf() {
    extern "C" {
        fn tsf_hal_set_tsf_enable(port: u32) -> i32;
    }
    unsafe {
        let _ = tsf_hal_set_tsf_enable(0);
        let _ = tsf_hal_set_tsf_enable(1);
        core::ptr::write_volatile(0x600A_44CC as *mut u32, 0x0000_0040);
        core::ptr::write_volatile(0x600A_4C74 as *mut u32, 0);
        // Gold 44c4 is a free-running HW counter (no blob store). Seed it
        // if still zero so we can see whether the clock is gated.
        if core::ptr::read_volatile(0x600A_44C4 as *const u32) == 0 {
            core::ptr::write_volatile(0x600A_44C4 as *mut u32, 1);
        }
    }
}

/// FE DC / IQ words gold keeps near `0x0162`/`0xff5a`/`0xff34`.
pub fn fe_dc_diag() -> (u32, u32, u32) {
    unsafe {
        (
            core::ptr::read_volatile(0x600A_8038 as *const u32),
            core::ptr::read_volatile(0x600A_803C as *const u32),
            core::ptr::read_volatile(0x600A_8044 as *const u32),
        )
    }
}

/// Replay `phy_i2c_init1` + gold `0x63[4]=0x33`.
pub fn hold_gold_rf_i2c() -> (u8, u8) {
    extern "C" {
        fn phy_i2c_init1();
    }
    unsafe {
        phy_i2c_init1();
    }
    if let Some((read, write)) = phy_rom_i2c() {
        write(0x63, 1, 4, 0x33);
        (read(0x6b, 1, 2) as u8, read(0x63, 1, 4) as u8)
    } else {
        (0, 0)
    }
}

fn restore_gold_rf_i2c() {
    let (a, b) = hold_gold_rf_i2c();
    wifi_usb_hex(b"[wifi] rf6b2=", u32::from(a));
    wifi_usb_hex(b"[wifi] rf634=", u32::from(b));
}

/// PHY ROM I2C slaves the public regi2c map does not cover.
/// `g_phyFuns[0x50]` is `i2c_read(block, host, reg)` — same path as
/// `rf_init` / `filter_dcap_set` / `phy_i2c_init1`.
fn dump_phy_rf_i2c(w: &mut dyn fmt::Write) {
    extern "C" {
        fn phy_get_romfuncs() -> *const u32;
    }
    let funs = unsafe { phy_get_romfuncs() };
    if funs.is_null() {
        let _ = writeln!(w, "ANALOG I2C RF romfuncs=null");
        return;
    }
    let slot = unsafe { *funs.add(0x50 / 4) };
    if slot == 0 {
        let _ = writeln!(w, "ANALOG I2C RF i2c_read=0");
        return;
    }
    let read: extern "C" fn(u32, u32, u32) -> u32 = unsafe { core::mem::transmute(slot) };
    for &(name, block, n) in &[
        ("RFPLL", 0x62u8, 16u8),
        ("RF63", 0x63, 16),
        ("RXRF", 0x67, 32),
        ("RFCAL", 0x6b, 16),
    ] {
        let _ = write!(w, "ANALOG I2C {name} {block:#04x}:");
        for reg in 0..n {
            let v = read(u32::from(block), 1, u32::from(reg));
            let _ = write!(w, " {:02x}", v as u8);
        }
        let _ = writeln!(w);
    }
}

fn dump_regs(w: &mut dyn fmt::Write, tag: &str, base: usize, words: usize) {
    let _ = writeln!(w, "REGDUMP VEEROS-{} base={:#010x} words={}", tag, base, words);
    let mut i = 0;
    while i < words {
        let r = |off: usize| -> u32 {
            unsafe { core::ptr::read_volatile((base + (i + off) * 4) as *const u32) }
        };
        let _ = writeln!(
            w,
            "{:#010x}: {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
            base + i * 4,
            r(0), r(1), r(2), r(3), r(4), r(5), r(6), r(7)
        );
        i += 8;
    }
}

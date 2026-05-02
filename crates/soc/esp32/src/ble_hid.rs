//! BLE HID-over-GATT Profile (HOGP) client for the ESP32-C6.
//!
//! Connects to BLE keyboards and mice that expose the HID Service
//! (UUID 0x1812) and subscribes to HID Report characteristics to
//! receive input events.
//!
//! # GATT HID Service structure
//!
//! ```text
//! Service: Human Interface Device (0x1812)
//!   ├─ Characteristic: HID Information   (0x2A4A) — read
//!   ├─ Characteristic: Report Map        (0x2A4B) — read (HID report descriptor)
//!   ├─ Characteristic: HID Control Point (0x2A4C) — write
//!   ├─ Characteristic: Report            (0x2A4D) — read/notify (input reports)
//!   │   └─ Descriptor: Report Reference  (0x2908) — report ID + type
//!   ├─ Characteristic: Boot Keyboard Input Report   (0x2A22) — read/notify
//!   ├─ Characteristic: Boot Keyboard Output Report  (0x2A32) — read/write
//!   └─ Characteristic: Boot Mouse Input Report      (0x2A33) — read/notify
//! ```
//!
//! # Status
//!
//! Structural implementation — defines the GATT UUIDs, characteristic
//! handles, notification subscription, and boot-protocol report parsing.
//! The actual BLE controller interaction (HCI commands, L2CAP, ATT) is
//! stubbed pending integration with the ESP32-C6 BLE stack.

use arch::InputEvent;

// ═══════════════════════════════════════════════════════════════════════════
// BLE GATT UUIDs for HID service
// ═══════════════════════════════════════════════════════════════════════════

/// HID Service UUID (16-bit short form).
pub const UUID_HID_SERVICE: u16 = 0x1812;

/// HID Information characteristic.
pub const UUID_HID_INFORMATION: u16 = 0x2A4A;
/// Report Map characteristic (HID report descriptor).
pub const UUID_REPORT_MAP: u16 = 0x2A4B;
/// HID Control Point characteristic.
pub const UUID_HID_CONTROL_POINT: u16 = 0x2A4C;
/// Report characteristic (generic HID report).
pub const UUID_REPORT: u16 = 0x2A4D;
/// Boot Keyboard Input Report characteristic.
pub const UUID_BOOT_KBD_INPUT: u16 = 0x2A22;
/// Boot Keyboard Output Report characteristic (LED state etc.).
pub const UUID_BOOT_KBD_OUTPUT: u16 = 0x2A32;
/// Boot Mouse Input Report characteristic.
pub const UUID_BOOT_MOUSE_INPUT: u16 = 0x2A33;
/// Client Characteristic Configuration Descriptor (enable notifications).
pub const UUID_CCCD: u16 = 0x2902;
/// Report Reference Descriptor.
pub const _UUID_REPORT_REF: u16 = 0x2908;

// ═══════════════════════════════════════════════════════════════════════════
// HID device type
// ═══════════════════════════════════════════════════════════════════════════

/// Type of HID device detected via GATT service discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleHidType {
    /// No HID device / not yet classified.
    Unknown,
    /// Boot keyboard (has Boot Keyboard Input Report).
    Keyboard,
    /// Boot mouse (has Boot Mouse Input Report).
    Mouse,
    /// Both keyboard and mouse reports present.
    Combo,
}

// ═══════════════════════════════════════════════════════════════════════════
// HOGP connection state
// ═══════════════════════════════════════════════════════════════════════════

/// State of a HOGP connection to one BLE HID device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HogpState {
    /// Not connected.
    Disconnected,
    /// BLE connection established, discovering services.
    Discovering,
    /// HID service found, subscribing to notifications.
    Subscribing,
    /// Fully connected and receiving HID reports.
    Active,
    /// Connection failed or lost.
    Error,
}

// ═══════════════════════════════════════════════════════════════════════════
// GATT characteristic handle cache
// ═══════════════════════════════════════════════════════════════════════════

/// Discovered GATT handle values for HID service characteristics.
/// These are populated during service discovery and used for
/// notification subscriptions and report reads.
#[derive(Clone, Copy)]
pub struct HidHandles {
    /// Attribute handle for Boot Keyboard Input Report (0 = not found).
    pub boot_kbd_input: u16,
    /// CCCD handle for Boot Keyboard Input Report notifications.
    pub boot_kbd_input_cccd: u16,
    /// Attribute handle for Boot Mouse Input Report (0 = not found).
    pub boot_mouse_input: u16,
    /// CCCD handle for Boot Mouse Input Report notifications.
    pub boot_mouse_input_cccd: u16,
    /// Attribute handle for generic Report characteristic (0 = not found).
    pub report: u16,
    /// CCCD handle for generic Report notifications.
    pub report_cccd: u16,
    /// Attribute handle for HID Control Point.
    pub control_point: u16,
}

impl HidHandles {
    pub const fn empty() -> Self {
        Self {
            boot_kbd_input: 0,
            boot_kbd_input_cccd: 0,
            boot_mouse_input: 0,
            boot_mouse_input_cccd: 0,
            report: 0,
            report_cccd: 0,
            control_point: 0,
        }
    }

    /// Determine device type from discovered handles.
    pub fn device_type(&self) -> BleHidType {
        let has_kbd = self.boot_kbd_input != 0;
        let has_mouse = self.boot_mouse_input != 0;
        match (has_kbd, has_mouse) {
            (true, true) => BleHidType::Combo,
            (true, false) => BleHidType::Keyboard,
            (false, true) => BleHidType::Mouse,
            (false, false) => BleHidType::Unknown,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// BLE HID device
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum BLE device address length (6 bytes for BD_ADDR).
pub const BD_ADDR_LEN: usize = 6;

/// Maximum device name length.
pub const MAX_NAME_LEN: usize = 20;

/// Represents one connected (or connecting) BLE HID device.
pub struct BleHidDevice {
    /// BLE device address (BD_ADDR, 6 bytes).
    pub addr: [u8; BD_ADDR_LEN],
    /// Device name (from GAP advertisement or GATT query).
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
    /// BLE connection handle (from HCI LE Connection Complete event).
    pub conn_handle: u16,
    /// Connection state.
    pub state: HogpState,
    /// Discovered GATT handles for HID characteristics.
    pub handles: HidHandles,
    /// Detected HID device type.
    pub hid_type: BleHidType,
    /// Active flag.
    pub active: bool,
}

impl BleHidDevice {
    pub const fn empty() -> Self {
        Self {
            addr: [0; BD_ADDR_LEN],
            name: [0; MAX_NAME_LEN],
            name_len: 0,
            conn_handle: 0,
            state: HogpState::Disconnected,
            handles: HidHandles::empty(),
            hid_type: BleHidType::Unknown,
            active: false,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// HOGP manager
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum simultaneously connected BLE HID devices.
pub const MAX_BLE_HID_DEVICES: usize = 4;

/// BLE HOGP manager — scans for HID devices, connects, and receives reports.
pub struct HogpManager {
    /// Connected/connecting HID devices.
    pub devices: [BleHidDevice; MAX_BLE_HID_DEVICES],
    /// Number of active device slots.
    pub device_count: usize,
    /// Whether the manager has been initialized.
    pub initialized: bool,
}

impl HogpManager {
    pub const fn new() -> Self {
        Self {
            devices: [
                BleHidDevice::empty(),
                BleHidDevice::empty(),
                BleHidDevice::empty(),
                BleHidDevice::empty(),
            ],
            device_count: 0,
            initialized: false,
        }
    }

    /// Initialize the HOGP manager.
    ///
    /// On real hardware, this would:
    /// 1. Ensure BLE stack is initialized
    /// 2. Set up GATT client callbacks
    /// 3. Start scanning for HID service advertisements
    pub fn init(&mut self) -> bool {
        // Stub — real implementation needs BLE controller init via HCI.
        self.initialized = true;
        true
    }

    /// Start scanning for BLE HID devices.
    ///
    /// Scans for devices advertising the HID service UUID (0x1812).
    /// When a device is found, it is added to the device list in
    /// `Disconnected` state, ready for `connect()`.
    pub fn start_scan(&mut self) -> bool {
        if !self.initialized {
            return false;
        }
        // Stub — would issue HCI LE Set Scan Parameters + LE Set Scan Enable.
        // Advertisement reports containing UUID 0x1812 trigger device discovery.
        true
    }

    /// Stop scanning.
    pub fn stop_scan(&mut self) -> bool {
        // Stub — would issue HCI LE Set Scan Enable (disable).
        true
    }

    /// Initiate connection to a discovered BLE HID device.
    ///
    /// - `slot`: index into `devices[]`
    ///
    /// On success, transitions the device from `Disconnected` → `Discovering`.
    pub fn connect(&mut self, slot: usize) -> bool {
        if slot >= MAX_BLE_HID_DEVICES || !self.devices[slot].active {
            return false;
        }
        if self.devices[slot].state != HogpState::Disconnected {
            return false;
        }

        // Stub — would issue HCI LE Create Connection with the device's BD_ADDR.
        // On connection complete event: state → Discovering, start GATT service discovery.
        self.devices[slot].state = HogpState::Discovering;
        true
    }

    /// Disconnect a BLE HID device.
    pub fn disconnect(&mut self, slot: usize) -> bool {
        if slot >= MAX_BLE_HID_DEVICES || !self.devices[slot].active {
            return false;
        }
        // Stub — would issue HCI Disconnect.
        self.devices[slot].state = HogpState::Disconnected;
        true
    }

    /// Process a BLE notification (called from the BLE interrupt/event handler).
    ///
    /// - `conn_handle`: BLE connection handle from HCI event
    /// - `attr_handle`: GATT attribute handle that sent the notification
    /// - `data`: notification payload (HID report bytes)
    ///
    /// Returns an `InputEvent` if the notification is a HID report, or `None`.
    pub fn process_notification(
        &self,
        conn_handle: u16,
        attr_handle: u16,
        data: &[u8],
    ) -> InputEvent {
        // Find the device by connection handle.
        let dev = match self
            .devices
            .iter()
            .find(|d| d.active && d.conn_handle == conn_handle && d.state == HogpState::Active)
        {
            Some(d) => d,
            None => return InputEvent::None,
        };

        // Determine which characteristic this notification is for.
        if attr_handle == dev.handles.boot_kbd_input && data.len() >= 8 {
            // Boot keyboard report: [mods, 0, key0..key5]
            // Return the first newly pressed key as a raw KeyPress event.
            // The kernel's InputSubsystem / KeyboardState handles full conversion.
            let _mods = data[0];
            for &key in &data[2..8] {
                if key != 0 {
                    // Return raw usage code; kernel will translate via hid.rs tables
                    return InputEvent::KeyPress(key);
                }
            }
            InputEvent::None
        } else if attr_handle == dev.handles.boot_mouse_input && data.len() >= 3 {
            // Boot mouse report: [buttons, dx, dy]
            let dx = data[1] as i8 as i16;
            let dy = data[2] as i8 as i16;
            if dx != 0 || dy != 0 {
                InputEvent::MouseMove { dx, dy }
            } else {
                let buttons = data[0];
                if buttons != 0 {
                    InputEvent::MouseButton {
                        button: 0,
                        pressed: buttons & 1 != 0,
                    }
                } else {
                    InputEvent::None
                }
            }
        } else {
            InputEvent::None
        }
    }

    /// Enable notifications on Boot Keyboard/Mouse Input Report characteristics.
    ///
    /// Called after GATT service discovery completes.
    /// Writes 0x0001 (enable notifications) to the CCCD descriptor.
    pub fn subscribe_notifications(&mut self, slot: usize) -> bool {
        if slot >= MAX_BLE_HID_DEVICES || !self.devices[slot].active {
            return false;
        }
        let dev = &mut self.devices[slot];
        if dev.state != HogpState::Discovering {
            return false;
        }

        // Stub — would issue GATT Write Request to each CCCD handle:
        //   ATT_WRITE_REQ(handle=cccd_handle, value=[0x01, 0x00])
        //
        // For boot keyboard: write to dev.handles.boot_kbd_input_cccd
        // For boot mouse: write to dev.handles.boot_mouse_input_cccd

        dev.state = HogpState::Subscribing;
        // On write response, transition to Active.
        dev.state = HogpState::Active;
        true
    }

    /// Write formatted device list to writer.
    pub fn write_device_list(&self, w: &mut dyn core::fmt::Write) {
        if self.device_count == 0 {
            let _ = writeln!(w, "  (no BLE HID devices)");
            return;
        }
        for (i, dev) in self.devices.iter().enumerate() {
            if !dev.active {
                continue;
            }
            let name = core::str::from_utf8(&dev.name[..dev.name_len]).unwrap_or("?");
            let _ = writeln!(
                w,
                "  [{}] {} {:?} ({:?}) {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                i,
                name,
                dev.hid_type,
                dev.state,
                dev.addr[0],
                dev.addr[1],
                dev.addr[2],
                dev.addr[3],
                dev.addr[4],
                dev.addr[5]
            );
        }
    }
}

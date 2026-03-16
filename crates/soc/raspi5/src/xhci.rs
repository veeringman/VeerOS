//! xHCI (USB 3.x) host controller driver for Raspberry Pi 5.
//!
//! The RPi 5 USB ports are provided by an **RP1 southbridge**
//! (PCIe-attached), which hosts two xHCI controllers:
//!
//! - **xHCI 0** — USB 3.0 / USB 2.0 ports (2× USB 3.0 Type-A)
//! - **xHCI 1** — USB 2.0 only ports (2× USB 2.0 Type-A)
//!
//! Both are standard xHCI 1.2 compliant.  The base addresses are
//! discovered via PCIe BAR mapping by the RPi firmware and appear
//! in the device tree.
//!
//! # xHCI architecture (simplified)
//!
//! ```text
//! ┌──────────────────────────────────────────┐
//! │  Capability Registers  (read-only)       │
//! │  ├─ CAPLENGTH, HCSPARAMS1/2/3, HCCPARAMS│
//! ├──────────────────────────────────────────┤
//! │  Operational Registers                   │
//! │  ├─ USBCMD, USBSTS, DNCTRL, CRCR, DCBAAP│
//! │  ├─ CONFIG (MaxSlotsEn)                  │
//! │  └─ PORTSC[0..n] (port status/control)   │
//! ├──────────────────────────────────────────┤
//! │  Runtime Registers                       │
//! │  └─ Interrupter[0..n] (IMAN, IMOD, ERST)│
//! ├──────────────────────────────────────────┤
//! │  Doorbell Array                          │
//! │  └─ DB[0..n] (ring doorbell per slot)    │
//! └──────────────────────────────────────────┘
//!
//! Host memory structures:
//!   • DCBAA — Device Context Base Address Array
//!   • Transfer Rings — per-endpoint TRB ring buffers
//!   • Command Ring — host→controller commands (TRBs)
//!   • Event Ring — controller→host notifications
//! ```
//!
//! # Status
//!
//! This is a **structural stub** — register definitions, init
//! sequence, and port detection are implemented.  Actual data
//! transfers and device enumeration require DMA-capable memory
//! regions and are left for a future phase.

use arch::{UsbHostController, UsbDeviceInfo, UsbSpeed};

// ═══════════════════════════════════════════════════════════════════════════
// RP1 xHCI base addresses (from RPi 5 device tree)
// ═══════════════════════════════════════════════════════════════════════════

/// xHCI controller 0 base (USB 3.0 ports), mapped via RP1 BAR.
/// Device tree: `usb@200000` under RP1 node.
const XHCI0_BASE: usize = super::mem::RP1_BAR_BASE + 0x20_0000;

/// xHCI controller 1 base (USB 2.0 only ports).
/// Device tree: `usb@300000` under RP1 node.
const XHCI1_BASE: usize = super::mem::RP1_BAR_BASE + 0x30_0000;

// ═══════════════════════════════════════════════════════════════════════════
// xHCI register offsets — Capability registers
// ═══════════════════════════════════════════════════════════════════════════

/// Capability register length + interface version (8-bit length at [7:0]).
const CAP_CAPLENGTH:    usize = 0x00;
/// Host Controller Interface Version (16 bits at [31:16] of offset 0x00).
const _CAP_HCIVERSION:   usize = 0x02;
/// Structural parameters 1 — MaxSlots [7:0], MaxIntrs [18:8], MaxPorts [31:24].
const CAP_HCSPARAMS1:   usize = 0x04;
/// Structural parameters 2 — IST, ERST Max, SPB Max.
const _CAP_HCSPARAMS2:   usize = 0x08;
/// Structural parameters 3 — U1/U2 device exit latency.
const _CAP_HCSPARAMS3:   usize = 0x0C;
/// Capability parameters 1 — AC64, CSZ, MaxPSASize, xECP, etc.
const _CAP_HCCPARAMS1:   usize = 0x10;
/// Doorbell array offset (from base).
const CAP_DBOFF:        usize = 0x14;
/// Runtime register space offset (from base).
const CAP_RTSOFF:       usize = 0x18;

// ═══════════════════════════════════════════════════════════════════════════
// xHCI register offsets — Operational registers (base + CAPLENGTH)
// ═══════════════════════════════════════════════════════════════════════════

/// USB Command register.
const OP_USBCMD:    usize = 0x00;
/// USB Status register.
const OP_USBSTS:    usize = 0x04;
/// Device notification control.
const _OP_DNCTRL:    usize = 0x14;
/// Command Ring Control Register (64-bit).
const _OP_CRCR:      usize = 0x18;
/// Device Context Base Address Array Pointer (64-bit).
const _OP_DCBAAP:    usize = 0x30;
/// Configure register — MaxSlotsEn.
const OP_CONFIG:    usize = 0x38;
/// Port Status and Control registers start at offset 0x400 from operational base.
/// Each port occupies 16 bytes: PORTSC, PORTPMSC, PORTLI, PORTHLPMC.
const OP_PORTSC_BASE: usize = 0x400;

// ─── USBCMD bits ─────────────────────────────────────────────────────
const USBCMD_RUN:    u32 = 1 << 0;  // Run/Stop
const USBCMD_HCRST:  u32 = 1 << 1;  // Host Controller Reset
const _USBCMD_INTE:   u32 = 1 << 2;  // Interrupter Enable
const _USBCMD_HSEE:   u32 = 1 << 3;  // Host System Error Enable

// ─── USBSTS bits ─────────────────────────────────────────────────────
const USBSTS_HCH:    u32 = 1 << 0;  // HC Halted
const _USBSTS_HSE:    u32 = 1 << 2;  // Host System Error
const _USBSTS_EINT:   u32 = 1 << 3;  // Event Interrupt
const USBSTS_CNR:    u32 = 1 << 11; // Controller Not Ready

// ─── PORTSC bits ─────────────────────────────────────────────────────
const PORTSC_CCS:     u32 = 1 << 0;   // Current Connect Status
const PORTSC_PED:     u32 = 1 << 1;   // Port Enabled/Disabled
const PORTSC_PR:      u32 = 1 << 4;   // Port Reset
const _PORTSC_PP:      u32 = 1 << 9;   // Port Power
const PORTSC_SPEED_MASK: u32 = 0xF << 10; // Port Speed [13:10]
const PORTSC_SPEED_SHIFT: u32 = 10;
const PORTSC_CSC:     u32 = 1 << 17;  // Connect Status Change (W1C)
const _PORTSC_PRC:     u32 = 1 << 21;  // Port Reset Change (W1C)
/// Bits that are RW1C (write-1-to-clear) — must preserve these as 0 when
/// writing other fields to avoid accidentally clearing them.
const _PORTSC_RW1C_MASK: u32 = PORTSC_CSC | (1 << 18) | (1 << 19) | (1 << 20)
                              | (1 << 21) | (1 << 22) | (1 << 23);

// xHCI port speed encoding.
const XHCI_SPEED_FULL:  u32 = 1;
const XHCI_SPEED_LOW:   u32 = 2;
const XHCI_SPEED_HIGH:  u32 = 3;
const XHCI_SPEED_SUPER: u32 = 4;

// ═══════════════════════════════════════════════════════════════════════════
// xHCI driver state
// ═══════════════════════════════════════════════════════════════════════════

/// Maximum number of root hub ports we track.
const MAX_PORTS: usize = 8;

/// Maximum device slots supported.
const MAX_SLOTS: usize = 16;

/// Per-port state.
#[derive(Clone, Copy)]
struct PortState {
    connected: bool,
    enabled: bool,
    speed: UsbSpeed,
    slot_id: u8,
}

impl PortState {
    const fn empty() -> Self {
        Self {
            connected: false,
            enabled: false,
            speed: UsbSpeed::Full,
            slot_id: 0,
        }
    }
}

/// xHCI host controller driver for one controller instance.
pub struct Xhci {
    /// Memory-mapped base address of the controller.
    base: usize,
    /// Length of the capability register space.
    cap_length: usize,
    /// Offset to operational registers (base + cap_length).
    op_base: usize,
    /// Offset to runtime registers.
    _rt_base: usize,
    /// Offset to doorbell array.
    _db_base: usize,
    /// Number of physical ports.
    num_ports: usize,
    /// Maximum device slots.
    _max_slots: usize,
    /// Per-port tracking.
    ports: [PortState; MAX_PORTS],
    /// Controller running flag.
    running: bool,
}

impl Xhci {
    /// Create a new xHCI driver instance for the controller at `base`.
    pub const fn new(base: usize) -> Self {
        Self {
            base,
            cap_length: 0,
            op_base: 0,
            _rt_base: 0,
            _db_base: 0,
            num_ports: 0,
            _max_slots: 0,
            ports: [PortState::empty(); MAX_PORTS],
            running: false,
        }
    }

    /// Create driver for xHCI controller 0 (USB 3.0 ports).
    pub const fn xhci0() -> Self {
        Self::new(XHCI0_BASE)
    }

    /// Create driver for xHCI controller 1 (USB 2.0-only ports).
    pub const fn xhci1() -> Self {
        Self::new(XHCI1_BASE)
    }

    // ─── Low-level register access ───────────────────────────────

    #[inline]
    unsafe fn read32(&self, offset: usize) -> u32 {
        unsafe { core::ptr::read_volatile((self.base + offset) as *const u32) }
    }

    #[inline]
    unsafe fn write32(&self, offset: usize, val: u32) {
        unsafe { core::ptr::write_volatile((self.base + offset) as *mut u32, val) }
    }

    #[inline]
    unsafe fn op_read32(&self, offset: usize) -> u32 {
        unsafe { core::ptr::read_volatile((self.op_base + offset) as *const u32) }
    }

    #[inline]
    unsafe fn op_write32(&self, offset: usize, val: u32) {
        unsafe { core::ptr::write_volatile((self.op_base + offset) as *mut u32, val) }
    }

    /// Read PORTSC for a port (0-indexed).
    unsafe fn portsc_read(&self, port: usize) -> u32 {
        unsafe { self.op_read32(OP_PORTSC_BASE + port * 0x10) }
    }

    /// Write PORTSC for a port (careful with RW1C bits).
    unsafe fn portsc_write(&self, port: usize, val: u32) {
        unsafe { self.op_write32(OP_PORTSC_BASE + port * 0x10, val) }
    }

    // ─── Helper: busy-wait for a condition ───────────────────────

    fn wait_bits(addr: usize, mask: u32, expected: u32, timeout_loops: u32) -> bool {
        for _ in 0..timeout_loops {
            let val = unsafe { core::ptr::read_volatile(addr as *const u32) };
            if val & mask == expected {
                return true;
            }
            // Small delay — compiler fence to prevent optimisation.
            for _ in 0..100 {
                core::hint::spin_loop();
            }
        }
        false
    }

    // ─── Port speed decode ───────────────────────────────────────

    fn decode_speed(portsc: u32) -> UsbSpeed {
        match (portsc & PORTSC_SPEED_MASK) >> PORTSC_SPEED_SHIFT {
            XHCI_SPEED_LOW   => UsbSpeed::Low,
            XHCI_SPEED_FULL  => UsbSpeed::Full,
            XHCI_SPEED_HIGH  => UsbSpeed::High,
            XHCI_SPEED_SUPER => UsbSpeed::Super,
            _                => UsbSpeed::Full,
        }
    }

    /// Scan all ports and update internal state.
    pub fn scan_ports(&mut self) {
        for i in 0..self.num_ports.min(MAX_PORTS) {
            let portsc = unsafe { self.portsc_read(i) };
            self.ports[i].connected = portsc & PORTSC_CCS != 0;
            self.ports[i].enabled = portsc & PORTSC_PED != 0;
            if self.ports[i].connected {
                self.ports[i].speed = Self::decode_speed(portsc);
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// UsbHostController trait implementation
// ═══════════════════════════════════════════════════════════════════════════

impl UsbHostController for Xhci {
    fn init(&mut self) -> bool {
        // 1. Read capability registers.
        let caplength_raw = unsafe { self.read32(CAP_CAPLENGTH) };
        self.cap_length = (caplength_raw & 0xFF) as usize;
        self.op_base = self.base + self.cap_length;

        let hcsparams1 = unsafe { self.read32(CAP_HCSPARAMS1) };
        self.num_ports = ((hcsparams1 >> 24) & 0xFF) as usize;
        self._max_slots = (hcsparams1 & 0xFF) as usize;

        let dboff = unsafe { self.read32(CAP_DBOFF) } as usize;
        self._db_base = self.base + dboff;

        let rtsoff = unsafe { self.read32(CAP_RTSOFF) } as usize;
        self._rt_base = self.base + rtsoff;

        // 2. Wait for Controller Not Ready to clear.
        if !Self::wait_bits(self.op_base + OP_USBSTS, USBSTS_CNR, 0, 10_000) {
            return false;
        }

        // 3. Halt the controller if running.
        let cmd = unsafe { self.op_read32(OP_USBCMD) };
        if cmd & USBCMD_RUN != 0 {
            unsafe { self.op_write32(OP_USBCMD, cmd & !USBCMD_RUN) };
            if !Self::wait_bits(self.op_base + OP_USBSTS, USBSTS_HCH, USBSTS_HCH, 10_000) {
                return false;
            }
        }

        // 4. Reset the controller.
        unsafe { self.op_write32(OP_USBCMD, USBCMD_HCRST) };
        if !Self::wait_bits(self.op_base + OP_USBCMD, USBCMD_HCRST, 0, 100_000) {
            return false;
        }
        if !Self::wait_bits(self.op_base + OP_USBSTS, USBSTS_CNR, 0, 10_000) {
            return false;
        }

        // 5. Set MaxSlotsEn.
        let max_slots = self._max_slots.min(MAX_SLOTS);
        unsafe { self.op_write32(OP_CONFIG, max_slots as u32) };

        // NOTE: Full xHCI initialisation would continue with:
        //   6. Allocate DCBAA (Device Context Base Address Array)
        //   7. Set DCBAAP register
        //   8. Allocate Command Ring, set CRCR
        //   9. Allocate Event Ring Segment Table, configure interrupter 0
        //   10. Start the controller (USBCMD.RUN = 1)
        //   11. Ring command doorbell for Enable Slot / Address Device
        //
        // These steps require DMA-capable memory allocation which is
        // deferred to a future phase. For now, we scan ports to detect
        // connected devices.

        // 6. Scan ports for connected devices.
        self.scan_ports();

        self.running = true;
        true
    }

    fn reset(&mut self) -> bool {
        unsafe { self.op_write32(OP_USBCMD, USBCMD_HCRST) };
        let ok = Self::wait_bits(self.op_base + OP_USBCMD, USBCMD_HCRST, 0, 100_000);
        self.running = false;
        ok
    }

    fn port_count(&self) -> usize {
        self.num_ports.min(MAX_PORTS)
    }

    fn port_connected(&self, port: usize) -> bool {
        if port >= self.num_ports.min(MAX_PORTS) {
            return false;
        }
        self.ports[port].connected
    }

    fn port_reset(&mut self, port: usize) -> Option<UsbSpeed> {
        if port >= self.num_ports.min(MAX_PORTS) || !self.ports[port].connected {
            return None;
        }

        // Issue port reset.
        let portsc = unsafe { self.portsc_read(port) };
        // Preserve non-RW1C bits, set Port Reset.
        let val = (portsc & 0x0E00_C3E0) | PORTSC_PR;
        unsafe { self.portsc_write(port, val) };

        // Wait for reset to complete (PED set, PR cleared).
        if !Self::wait_bits(
            self.op_base + OP_PORTSC_BASE + port * 0x10,
            PORTSC_PR,
            0,
            50_000,
        ) {
            return None;
        }

        // Read back speed.
        let portsc = unsafe { self.portsc_read(port) };
        let speed = Self::decode_speed(portsc);
        self.ports[port].speed = speed;
        self.ports[port].enabled = portsc & PORTSC_PED != 0;

        // Clear Connect Status Change.
        unsafe { self.portsc_write(port, portsc | PORTSC_CSC) };

        Some(speed)
    }

    fn control_transfer(
        &mut self,
        _addr: u8,
        _setup: &[u8; 8],
        _data: Option<&mut [u8]>,
    ) -> Option<usize> {
        // Requires Command Ring + Transfer Ring + DCBAA — deferred.
        None
    }

    fn interrupt_in(
        &mut self,
        _addr: u8,
        _ep: u8,
        _buf: &mut [u8],
    ) -> Option<usize> {
        // Requires Transfer Ring scheduling — deferred.
        None
    }

    fn device_info(&self, _addr: u8) -> Option<UsbDeviceInfo> {
        // Requires device enumeration — deferred.
        None
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// USB HID class driver (boot protocol)
// ═══════════════════════════════════════════════════════════════════════════

/// USB HID class codes.
pub const USB_CLASS_HID: u8 = 0x03;
/// Boot interface subclass.
pub const _USB_SUBCLASS_BOOT: u8 = 0x01;
/// Keyboard protocol.
pub const USB_PROTOCOL_KEYBOARD: u8 = 0x01;
/// Mouse protocol.
pub const USB_PROTOCOL_MOUSE: u8 = 0x02;

/// Standard USB request types.
const _REQ_GET_DESCRIPTOR: u8 = 0x06;
const _REQ_SET_PROTOCOL: u8 = 0x0B;

/// USB HID SET_PROTOCOL — switch to boot protocol (simpler reports).
///
/// Returns the 8-byte SETUP packet for SET_PROTOCOL(boot).
pub fn make_set_boot_protocol_setup(interface: u8) -> [u8; 8] {
    [
        0x21,            // bmRequestType: class, interface, host-to-device
        _REQ_SET_PROTOCOL, // bRequest
        0x00, 0x00,      // wValue: 0 = boot protocol
        interface, 0x00, // wIndex: interface number
        0x00, 0x00,      // wLength: 0
    ]
}

/// USB HID GET_REPORT — request an input report from the device.
///
/// Returns the 8-byte SETUP packet for GET_REPORT(input, report_id=0).
pub fn make_get_report_setup(interface: u8, length: u16) -> [u8; 8] {
    [
        0xA1,               // bmRequestType: class, interface, device-to-host
        0x01,               // bRequest: GET_REPORT
        0x00, 0x01,         // wValue: report type = input (01), report ID = 0
        interface, 0x00,    // wIndex: interface number
        (length & 0xFF) as u8, (length >> 8) as u8, // wLength
    ]
}

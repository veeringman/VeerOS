#![allow(dead_code)]
//! xHCI (USB 3.x) host controller driver for Raspberry Pi 5.
//!
//! The RPi 5 USB ports are provided by an **RP1 southbridge**
//! (PCIe-attached), which hosts two xHCI controllers:
//!
//! - **xHCI 0** — USB 3.0 / USB 2.0 ports (2× USB 3.0 Type-A)
//! - **xHCI 1** — USB 2.0 only ports (2× USB 2.0 Type-A)
//!
//! Both are standard xHCI 1.2 compliant.  This driver implements
//! the full xHCI lifecycle: controller init, DMA ring setup, device
//! enumeration, control transfers, and interrupt-IN polling for HID.

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
const CAP_HCIVERSION:   usize = 0x02;
/// Structural parameters 1 — MaxSlots [7:0], MaxIntrs [18:8], MaxPorts [31:24].
const CAP_HCSPARAMS1:   usize = 0x04;
/// Structural parameters 2 — IST, ERST Max, SPB Max.
const CAP_HCSPARAMS2:   usize = 0x08;
/// Structural parameters 3 — U1/U2 device exit latency.
const CAP_HCSPARAMS3:   usize = 0x0C;
/// Capability parameters 1 — AC64, CSZ, MaxPSASize, xECP, etc.
const CAP_HCCPARAMS1:   usize = 0x10;
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
const OP_DNCTRL:    usize = 0x14;
/// Command Ring Control Register (64-bit).
const OP_CRCR:      usize = 0x18;
/// Device Context Base Address Array Pointer (64-bit).
const OP_DCBAAP:    usize = 0x30;
/// Configure register — MaxSlotsEn.
const OP_CONFIG:    usize = 0x38;
/// Port Status and Control registers start at offset 0x400 from operational base.
/// Each port occupies 16 bytes: PORTSC, PORTPMSC, PORTLI, PORTHLPMC.
const OP_PORTSC_BASE: usize = 0x400;

// ─── USBCMD bits ─────────────────────────────────────────────────────
const USBCMD_RUN:    u32 = 1 << 0;  // Run/Stop
const USBCMD_HCRST:  u32 = 1 << 1;  // Host Controller Reset
const USBCMD_INTE:   u32 = 1 << 2;  // Interrupter Enable
const USBCMD_HSEE:   u32 = 1 << 3;  // Host System Error Enable

// ─── USBSTS bits ─────────────────────────────────────────────────────
const USBSTS_HCH:    u32 = 1 << 0;  // HC Halted
const USBSTS_HSE:    u32 = 1 << 2;  // Host System Error
const USBSTS_EINT:   u32 = 1 << 3;  // Event Interrupt
const USBSTS_CNR:    u32 = 1 << 11; // Controller Not Ready

// ─── PORTSC bits ─────────────────────────────────────────────────────
const PORTSC_CCS:     u32 = 1 << 0;   // Current Connect Status
const PORTSC_PED:     u32 = 1 << 1;   // Port Enabled/Disabled
const PORTSC_PR:      u32 = 1 << 4;   // Port Reset
const PORTSC_PP:      u32 = 1 << 9;   // Port Power
const PORTSC_SPEED_MASK: u32 = 0xF << 10; // Port Speed [13:10]
const PORTSC_SPEED_SHIFT: u32 = 10;
const PORTSC_CSC:     u32 = 1 << 17;  // Connect Status Change (W1C)
const PORTSC_PRC:     u32 = 1 << 21;  // Port Reset Change (W1C)
/// Bits that are RW1C (write-1-to-clear) — must preserve these as 0 when
/// writing other fields to avoid accidentally clearing them.
const PORTSC_RW1C_MASK: u32 = PORTSC_CSC | (1 << 18) | (1 << 19) | (1 << 20)
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

/// Number of TRBs per ring (must be power of 2, last is Link TRB).
const RING_SIZE: usize = 32;

/// Size of a single TRB (16 bytes, per xHCI spec).
const TRB_SIZE: usize = 16;

/// xHCI context entry size: 32 bytes (CSZ=0) or 64 bytes (CSZ=1).
/// We assume CSZ=0 (32-byte slot/endpoint contexts).
const CTX_SIZE: usize = 32;

/// Size of a Device Context (slot ctx + 31 endpoint ctxs).
const DEV_CTX_SIZE: usize = CTX_SIZE * 32; // 1024 bytes

/// Size of an Input Context (Input Control Ctx + slot + 31 EP ctxs).
const INPUT_CTX_SIZE: usize = CTX_SIZE * 33; // 1056 bytes

// ─── TRB Type codes ──────────────────────────────────────────────────
const TRB_NORMAL:           u32 = 1;
const TRB_SETUP_STAGE:      u32 = 2;
const TRB_DATA_STAGE:       u32 = 3;
const TRB_STATUS_STAGE:     u32 = 4;
const TRB_LINK:             u32 = 6;
const TRB_ENABLE_SLOT:      u32 = 9;
const TRB_DISABLE_SLOT:     u32 = 10;
const TRB_ADDRESS_DEVICE:   u32 = 11;
const TRB_CONFIGURE_EP:     u32 = 12;
const TRB_EVALUATE_CTX:     u32 = 13;
const TRB_NOOP_CMD:         u32 = 23;

// Event TRB types
const TRB_TRANSFER_EVENT:   u32 = 32;
const TRB_CMD_COMPLETION:   u32 = 33;
const TRB_PORT_STATUS_CHANGE: u32 = 34;

// TRB status/completion codes (bits [31:24] of dword 2)
const TRB_COMP_SUCCESS:      u8 = 1;
const TRB_COMP_SHORT_PKT:   u8 = 13;

// TRB field bit positions
const TRB_CYCLE_BIT: u32     = 1 << 0;
const TRB_TOGGLE_CYCLE: u32  = 1 << 1; // for Link TRBs
const TRB_IOC: u32           = 1 << 5; // Interrupt On Completion
const TRB_IDT: u32           = 1 << 6; // Immediate Data
const TRB_TYPE_SHIFT: u32    = 10;
const TRB_DIR_IN: u32        = 1 << 16; // Data direction: IN (device→host)

// ─── Interrupter registers (at runtime base + 0x20 per interrupter) ──
const IMAN:  usize = 0x00;  // Interrupter Management
const IMOD:  usize = 0x04;  // Interrupter Moderation
const ERSTSZ: usize = 0x08; // Event Ring Segment Table Size
const ERSTBA: usize = 0x10; // Event Ring Segment Table Base Address (64-bit)
const ERDP:  usize = 0x18;  // Event Ring Dequeue Pointer (64-bit)

// ─── Max packet sizes per speed ──────────────────────────────────────
fn max_packet_for_speed(speed: UsbSpeed) -> u16 {
    match speed {
        UsbSpeed::Low   => 8,
        UsbSpeed::Full  => 64,
        UsbSpeed::High  => 64,
        UsbSpeed::Super => 512,
    }
}

/// Map UsbSpeed to xHCI speed encoding for Slot Context.
fn speed_to_xhci(speed: UsbSpeed) -> u32 {
    match speed {
        UsbSpeed::Full  => 1,
        UsbSpeed::Low   => 2,
        UsbSpeed::High  => 3,
        UsbSpeed::Super => 4,
    }
}

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

/// A 16-byte Transfer Request Block.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Trb {
    param: u64,   // dword 0-1 (parameter / data buffer pointer)
    status: u32,  // dword 2 (status / transfer length / completion code)
    control: u32, // dword 3 (cycle bit, TRB type, flags)
}

impl Trb {
    const fn zero() -> Self {
        Self { param: 0, status: 0, control: 0 }
    }

    /// Get TRB type field.
    fn trb_type(&self) -> u32 {
        (self.control >> TRB_TYPE_SHIFT) & 0x3F
    }

    /// Get completion code from event TRB.
    fn completion_code(&self) -> u8 {
        (self.status >> 24) as u8
    }

    /// Get slot ID from event/command TRB (bits [31:24] of control).
    fn slot_id(&self) -> u8 {
        (self.control >> 24) as u8
    }
}

/// A TRB ring (command or transfer).
#[repr(C, align(64))]
struct TrbRing {
    trbs: [Trb; RING_SIZE],
    enqueue: usize,
    cycle: u32, // producer cycle state (PCS)
}

impl TrbRing {
    const fn new() -> Self {
        Self {
            trbs: [Trb::zero(); RING_SIZE],
            enqueue: 0,
            cycle: 1, // start with cycle bit = 1
        }
    }

    /// Initialise ring: clear all TRBs and set Link TRB pointing back.
    fn init(&mut self) {
        for t in self.trbs.iter_mut() {
            *t = Trb::zero();
        }
        self.enqueue = 0;
        self.cycle = 1;
        // Last TRB is a Link TRB pointing to start of ring.
        let base = self.trbs.as_ptr() as u64;
        let link = &mut self.trbs[RING_SIZE - 1];
        link.param = base;
        link.status = 0;
        link.control = (TRB_LINK << TRB_TYPE_SHIFT) | TRB_TOGGLE_CYCLE | self.cycle;
    }

    /// Physical address of the ring base.
    fn phys_addr(&self) -> u64 {
        self.trbs.as_ptr() as u64
    }

    /// Enqueue a TRB onto the ring, return its physical address.
    fn enqueue_trb(&mut self, param: u64, status: u32, mut control: u32) -> u64 {
        let idx = self.enqueue;
        // Set cycle bit.
        control = (control & !TRB_CYCLE_BIT) | self.cycle;
        self.trbs[idx] = Trb { param, status, control };

        // Advance.
        let addr = &self.trbs[idx] as *const _ as u64;
        self.enqueue += 1;
        if self.enqueue >= RING_SIZE - 1 {
            // Wrap: update Link TRB cycle and toggle PCS.
            let link = &mut self.trbs[RING_SIZE - 1];
            link.control = (link.control & !TRB_CYCLE_BIT) | self.cycle;
            self.cycle ^= 1;
            self.enqueue = 0;
        }
        addr
    }
}

/// Event Ring Segment Table Entry.
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct ErstEntry {
    base_addr: u64,
    size: u32,     // number of TRBs in this segment
    _reserved: u32,
}

/// Event ring (consumer side).
#[repr(C, align(64))]
struct EventRing {
    trbs: [Trb; RING_SIZE],
    erst: [ErstEntry; 1], // single-segment
    dequeue: usize,
    cycle: u32, // consumer cycle state (CCS)
}

impl EventRing {
    const fn new() -> Self {
        Self {
            trbs: [Trb::zero(); RING_SIZE],
            erst: [ErstEntry { base_addr: 0, size: 0, _reserved: 0 }],
            dequeue: 0,
            cycle: 1,
        }
    }

    fn init(&mut self) {
        for t in self.trbs.iter_mut() {
            *t = Trb::zero();
        }
        self.dequeue = 0;
        self.cycle = 1;
        // Set ERST to point to our ring segment.
        self.erst[0] = ErstEntry {
            base_addr: self.trbs.as_ptr() as u64,
            size: RING_SIZE as u32,
            _reserved: 0,
        };
    }

    fn erst_addr(&self) -> u64 {
        self.erst.as_ptr() as u64
    }

    /// Dequeue the next event TRB, if available.
    fn dequeue_event(&mut self) -> Option<Trb> {
        let trb = self.trbs[self.dequeue];
        if (trb.control & TRB_CYCLE_BIT) != self.cycle {
            return None; // no new event
        }
        self.dequeue += 1;
        if self.dequeue >= RING_SIZE {
            self.dequeue = 0;
            self.cycle ^= 1;
        }
        Some(trb)
    }

    /// Current dequeue pointer (physical address) for ERDP update.
    fn dequeue_phys(&self) -> u64 {
        &self.trbs[self.dequeue] as *const _ as u64
    }
}

/// DCBAA: Device Context Base Address Array.
/// Entry 0 is the Scratchpad Buffer Array pointer (or 0).
/// Entries 1..MAX_SLOTS are device context pointers.
#[repr(C, align(64))]
struct Dcbaa {
    entries: [u64; MAX_SLOTS + 1],
}

impl Dcbaa {
    const fn new() -> Self {
        Self { entries: [0; MAX_SLOTS + 1] }
    }
}

/// Device context memory (one per slot).
#[repr(C, align(64))]
struct DeviceContext {
    data: [u8; DEV_CTX_SIZE],
}

impl DeviceContext {
    const fn new() -> Self {
        Self { data: [0; DEV_CTX_SIZE] }
    }
}

/// Input context for Address Device / Configure Endpoint commands.
#[repr(C, align(64))]
struct InputContext {
    data: [u8; INPUT_CTX_SIZE],
}

impl InputContext {
    const fn new() -> Self {
        Self { data: [0; INPUT_CTX_SIZE] }
    }

    fn clear(&mut self) {
        self.data.fill(0);
    }

    /// Write a 32-bit value at byte offset into the context.
    fn write32(&mut self, offset: usize, val: u32) {
        if offset + 4 <= self.data.len() {
            self.data[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        }
    }

    /// Read a 32-bit value at byte offset from the context.
    fn read32(&self, offset: usize) -> u32 {
        if offset + 4 <= self.data.len() {
            u32::from_le_bytes(self.data[offset..offset + 4].try_into().unwrap_or([0; 4]))
        } else {
            0
        }
    }

    fn phys_addr(&self) -> u64 {
        self.data.as_ptr() as u64
    }
}

/// Tracked state for an enumerated USB device.
#[derive(Clone, Copy)]
struct UsbDevice {
    active: bool,
    slot_id: u8,
    port: u8,
    speed: UsbSpeed,
    address: u8,
    vendor_id: u16,
    product_id: u16,
    class: u8,
    subclass: u8,
    protocol: u8,
    max_packet_ep0: u16,
}

impl UsbDevice {
    const fn empty() -> Self {
        Self {
            active: false, slot_id: 0, port: 0,
            speed: UsbSpeed::Full, address: 0,
            vendor_id: 0, product_id: 0,
            class: 0, subclass: 0, protocol: 0,
            max_packet_ep0: 8,
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
    rt_base: usize,
    /// Offset to doorbell array.
    db_base: usize,
    /// Number of physical ports.
    num_ports: usize,
    /// Maximum device slots (from HCSPARAMS1).
    max_slots: usize,
    /// Per-port tracking.
    ports: [PortState; MAX_PORTS],
    /// Controller running flag.
    running: bool,
    /// Context size (32 or 64 bytes).
    ctx_size: usize,

    // ─── DMA memory regions (statically allocated) ───────────────
    cmd_ring: TrbRing,
    event_ring: EventRing,
    dcbaa: Dcbaa,
    dev_contexts: [DeviceContext; MAX_SLOTS],
    input_ctx: InputContext,
    /// Per-slot transfer ring for EP0 (control).
    ep0_rings: [TrbRing; MAX_SLOTS],
    /// Per-slot transfer ring for interrupt-IN endpoint (EP 1 IN = DCI 3).
    int_in_rings: [TrbRing; MAX_SLOTS],
    /// Enumerated device tracking.
    devices: [UsbDevice; MAX_SLOTS],
    /// Number of active devices.
    pub num_devices: usize,
}

impl Xhci {
    /// Create a new xHCI driver instance for the controller at `base`.
    pub const fn new(base: usize) -> Self {
        Self {
            base,
            cap_length: 0,
            op_base: 0,
            rt_base: 0,
            db_base: 0,
            num_ports: 0,
            max_slots: 0,
            ports: [PortState::empty(); MAX_PORTS],
            running: false,
            ctx_size: CTX_SIZE,
            cmd_ring: TrbRing::new(),
            event_ring: EventRing::new(),
            dcbaa: Dcbaa::new(),
            dev_contexts: [const { DeviceContext::new() }; MAX_SLOTS],
            input_ctx: InputContext::new(),
            ep0_rings: [const { TrbRing::new() }; MAX_SLOTS],
            int_in_rings: [const { TrbRing::new() }; MAX_SLOTS],
            devices: [UsbDevice::empty(); MAX_SLOTS],
            num_devices: 0,
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

    // ─── Doorbell ────────────────────────────────────────────────

    /// Ring a doorbell. slot=0 for command ring, 1..MAX_SLOTS for device EPs.
    fn ring_doorbell(&self, slot: u8, target: u32) {
        let addr = self.db_base + (slot as usize) * 4;
        unsafe { core::ptr::write_volatile(addr as *mut u32, target) };
    }

    /// Ring the command doorbell (slot 0, target 0).
    fn ring_cmd_doorbell(&self) {
        self.ring_doorbell(0, 0);
    }

    /// Ring doorbell for EP0 (DCI 1) on a given slot.
    fn ring_ep0_doorbell(&self, slot_id: u8) {
        self.ring_doorbell(slot_id, 1);
    }

    /// Ring doorbell for interrupt-IN endpoint (DCI 3 = EP1 IN) on a slot.
    fn ring_int_in_doorbell(&self, slot_id: u8) {
        self.ring_doorbell(slot_id, 3);
    }

    // ─── Write interrupter register ──────────────────────────────

    fn intr_reg(&self, offset: usize) -> usize {
        self.rt_base + 0x20 + offset // interrupter 0 starts at runtime + 0x20
    }

    unsafe fn write_intr32(&self, offset: usize, val: u32) {
        core::ptr::write_volatile(self.intr_reg(offset) as *mut u32, val);
    }

    unsafe fn write_intr64(&self, offset: usize, val: u64) {
        core::ptr::write_volatile(self.intr_reg(offset) as *mut u64, val);
    }

    unsafe fn read_intr32(&self, offset: usize) -> u32 {
        core::ptr::read_volatile(self.intr_reg(offset) as *const u32)
    }

    // ─── DMA ring setup ─────────────────────────────────────────

    /// Set up the Command Ring, Event Ring, and DCBAA, then start the HC.
    fn setup_dma_and_start(&mut self) -> bool {
        // 1. Init command ring.
        self.cmd_ring.init();

        // 2. Init event ring.
        self.event_ring.init();

        // 3. Init DCBAA (all zeroes — no scratchpad, no devices yet).
        self.dcbaa.entries.fill(0);

        // 4. Write DCBAAP.
        let dcbaap = self.dcbaa.entries.as_ptr() as u64;
        unsafe {
            core::ptr::write_volatile((self.op_base + OP_DCBAAP) as *mut u64, dcbaap);
        }

        // 5. Write CRCR (Command Ring Control Register).
        //    Low 6 bits: RCS=1 (ring cycle state), rest = phys addr.
        let crcr = self.cmd_ring.phys_addr() | 1; // cycle bit = 1
        unsafe {
            core::ptr::write_volatile((self.op_base + OP_CRCR) as *mut u64, crcr);
        }

        // 6. Configure interrupter 0.
        unsafe {
            // ERST size = 1 segment.
            self.write_intr32(ERSTSZ, 1);
            // ERDP = start of event ring.
            self.write_intr64(ERDP, self.event_ring.trbs.as_ptr() as u64);
            // ERSTBA = address of ERST (must be written after ERSTSZ).
            self.write_intr64(ERSTBA, self.event_ring.erst_addr());
            // Enable interrupter (IMAN IP=1, IE=1).
            self.write_intr32(IMAN, 0x3);
            // Set moderation (moderate to reduce interrupt storm).
            self.write_intr32(IMOD, 0x00000FA0); // ~4000 interval
        }

        // 7. Enable device notification.
        unsafe { self.op_write32(OP_DNCTRL, 0x2); } // notification enable

        // 8. Start the controller: RUN + INTE.
        unsafe {
            let cmd = self.op_read32(OP_USBCMD);
            self.op_write32(OP_USBCMD, cmd | USBCMD_RUN | USBCMD_INTE);
        }

        // 9. Wait for HCH to clear (running).
        if !Self::wait_bits(self.op_base + OP_USBSTS, USBSTS_HCH, 0, 10_000) {
            return false;
        }

        true
    }

    // ─── Command execution ──────────────────────────────────────

    /// Issue a command TRB and wait for the completion event. Returns the event TRB.
    fn execute_command(&mut self, param: u64, status: u32, control: u32) -> Option<Trb> {
        self.cmd_ring.enqueue_trb(param, status, control);
        self.ring_cmd_doorbell();

        // Poll event ring for completion.
        for _ in 0..1_000_000u32 {
            if let Some(evt) = self.event_ring.dequeue_event() {
                // Update ERDP to acknowledge.
                unsafe {
                    self.write_intr64(ERDP, self.event_ring.dequeue_phys() | (1 << 3));
                }
                if evt.trb_type() == TRB_CMD_COMPLETION {
                    return Some(evt);
                }
                // Got a different event (port status change, etc.) — keep polling.
                continue;
            }
            core::hint::spin_loop();
        }
        None
    }

    /// Issue Enable Slot command, return assigned slot_id (1-based) or None.
    fn enable_slot(&mut self) -> Option<u8> {
        let ctrl = TRB_ENABLE_SLOT << TRB_TYPE_SHIFT;
        let evt = self.execute_command(0, 0, ctrl)?;
        if evt.completion_code() == TRB_COMP_SUCCESS {
            let slot = evt.slot_id();
            if slot > 0 && slot as usize <= MAX_SLOTS {
                return Some(slot);
            }
        }
        None
    }

    /// Issue Address Device command for a slot.
    fn address_device(&mut self, slot_id: u8, bsr: bool) -> bool {
        let input_phys = self.input_ctx.phys_addr();
        let mut ctrl = TRB_ADDRESS_DEVICE << TRB_TYPE_SHIFT;
        ctrl |= (slot_id as u32) << 24;
        if bsr { ctrl |= 1 << 9; } // Block Set Address Request
        match self.execute_command(input_phys, 0, ctrl) {
            Some(evt) => {
                let cc = evt.completion_code();
                cc == TRB_COMP_SUCCESS
            }
            None => false,
        }
    }

    /// Issue Configure Endpoint command for a slot.
    fn configure_endpoint(&mut self, slot_id: u8) -> bool {
        let input_phys = self.input_ctx.phys_addr();
        let ctrl = (TRB_CONFIGURE_EP << TRB_TYPE_SHIFT) | ((slot_id as u32) << 24);
        match self.execute_command(input_phys, 0, ctrl) {
            Some(evt) => evt.completion_code() == TRB_COMP_SUCCESS,
            None => false,
        }
    }

    // ─── Control transfer (EP0) ─────────────────────────────────

    /// Perform a control transfer on EP0 of the given slot.
    /// Returns the number of bytes transferred on success.
    fn control_xfer(
        &mut self,
        slot_idx: usize,
        setup: &[u8; 8],
        data: Option<&mut [u8]>,
    ) -> Option<usize> {
        let slot_id = self.devices[slot_idx].slot_id;
        let ring = &mut self.ep0_rings[slot_idx];

        // 1. Setup Stage TRB (8-byte SETUP packet as immediate data).
        let setup_param = u64::from_le_bytes(*setup);
        let setup_status = 8u32; // TRB transfer length = 8
        let dir_flag = if data.is_some() && (setup[0] & 0x80) != 0 { 3u32 } else { 2u32 };
        let setup_ctrl = (TRB_SETUP_STAGE << TRB_TYPE_SHIFT) | TRB_IDT | (dir_flag << 16);
        ring.enqueue_trb(setup_param, setup_status, setup_ctrl);

        // 2. Data Stage TRB (if data phase).
        let data_len = data.as_ref().map_or(0, |d| d.len());
        let data_ptr = data.as_ref().map_or(0u64, |d| d.as_ptr() as u64);
        if data_len > 0 {
            let dir = if (setup[0] & 0x80) != 0 { TRB_DIR_IN } else { 0 };
            let data_ctrl = (TRB_DATA_STAGE << TRB_TYPE_SHIFT) | dir;
            ring.enqueue_trb(data_ptr, data_len as u32, data_ctrl);
        }

        // 3. Status Stage TRB.
        let status_dir = if data_len > 0 && (setup[0] & 0x80) != 0 { 0 } else { TRB_DIR_IN };
        let status_ctrl = (TRB_STATUS_STAGE << TRB_TYPE_SHIFT) | TRB_IOC | status_dir;
        ring.enqueue_trb(0, 0, status_ctrl);

        // 4. Ring EP0 doorbell.
        self.ring_ep0_doorbell(slot_id);

        // 5. Wait for Transfer Event.
        for _ in 0..1_000_000u32 {
            if let Some(evt) = self.event_ring.dequeue_event() {
                unsafe {
                    self.write_intr64(ERDP, self.event_ring.dequeue_phys() | (1 << 3));
                }
                if evt.trb_type() == TRB_TRANSFER_EVENT {
                    let cc = evt.completion_code();
                    if cc == TRB_COMP_SUCCESS || cc == TRB_COMP_SHORT_PKT {
                        let residual = evt.status & 0xFFFFFF;
                        let transferred = if data_len > 0 { data_len - residual as usize } else { 0 };
                        return Some(transferred);
                    }
                    return None; // error completion
                }
            }
            core::hint::spin_loop();
        }
        None
    }

    // ─── Device enumeration ─────────────────────────────────────

    /// Enumerate a device on a port: enable slot, address, get descriptor.
    fn enumerate_port(&mut self, port: usize) -> bool {
        let speed = self.ports[port].speed;
        let max_pkt = max_packet_for_speed(speed);

        // 1. Enable Slot.
        let slot_id = match self.enable_slot() {
            Some(s) => s,
            None => return false,
        };
        let slot_idx = (slot_id - 1) as usize;
        if slot_idx >= MAX_SLOTS { return false; }

        // 2. Prepare Input Context for Address Device.
        self.input_ctx.clear();

        // Input Control Context (offset 0): Add flags = slot + EP0
        self.input_ctx.write32(0x04, 0x3); // A0 (slot) + A1 (EP0)

        // Slot Context (at CTX_SIZE offset):
        // dword 0: Route String (0), Speed [23:20], Context Entries [31:27] = 1
        let slot_dw0 = (speed_to_xhci(speed) << 20) | (1u32 << 27);
        self.input_ctx.write32(CTX_SIZE + 0, slot_dw0);
        // dword 1: Root Hub Port Number [23:16]
        self.input_ctx.write32(CTX_SIZE + 4, ((port as u32 + 1) & 0xFF) << 16);

        // EP0 Context (at CTX_SIZE*2 offset):
        // dword 1: EP Type [5:3] = 4 (Control), MaxPacketSize [31:16], CErr [2:1] = 3
        let ep_dw1 = (4u32 << 3) | (3u32 << 1) | ((max_pkt as u32) << 16);
        self.input_ctx.write32(CTX_SIZE * 2 + 4, ep_dw1);

        // Init EP0 transfer ring for this slot.
        self.ep0_rings[slot_idx].init();
        // dword 2-3: TR Dequeue Pointer (64-bit, DCS=1)
        let ep0_ring_phys = self.ep0_rings[slot_idx].phys_addr() | 1; // DCS=1
        // Write as two 32-bit halves
        self.input_ctx.write32(CTX_SIZE * 2 + 8, ep0_ring_phys as u32);
        self.input_ctx.write32(CTX_SIZE * 2 + 12, (ep0_ring_phys >> 32) as u32);

        // Point DCBAA to device context.
        self.dcbaa.entries[slot_id as usize] = self.dev_contexts[slot_idx].data.as_ptr() as u64;
        self.dev_contexts[slot_idx].data.fill(0);

        // 3. Address Device (BSR=false — sets USB address).
        if !self.address_device(slot_id, false) {
            return false;
        }

        // 4. Read device descriptor (GET_DESCRIPTOR, type=1, 18 bytes).
        let mut desc = [0u8; 18];
        let setup_get_dev_desc: [u8; 8] = [
            0x80, // bmRequestType: device-to-host, standard, device
            0x06, // bRequest: GET_DESCRIPTOR
            0x00, 0x01, // wValue: descriptor type=1 (device), index=0
            0x00, 0x00, // wIndex: 0
            0x12, 0x00, // wLength: 18
        ];

        let _xfer = self.control_xfer(slot_idx, &setup_get_dev_desc, Some(&mut desc));

        // 5. Parse descriptor and store device info.
        let vid = u16::from_le_bytes([desc[8], desc[9]]);
        let pid = u16::from_le_bytes([desc[10], desc[11]]);
        let dev_class = desc[4];
        let dev_subclass = desc[5];
        let dev_protocol = desc[6];

        self.devices[slot_idx] = UsbDevice {
            active: true,
            slot_id,
            port: port as u8,
            speed,
            address: slot_id, // xHCI assigns address = slot_id typically
            vendor_id: vid,
            product_id: pid,
            class: dev_class,
            subclass: dev_subclass,
            protocol: dev_protocol,
            max_packet_ep0: max_pkt,
        };
        self.ports[port].slot_id = slot_id;
        self.num_devices += 1;

        true
    }

    /// After enumeration, if a device is HID, configure the interrupt-IN endpoint.
    /// Returns true if an interrupt-IN EP was set up and configured.
    fn configure_hid_endpoint(&mut self, slot_idx: usize) -> bool {
        let dev = &self.devices[slot_idx];
        if !dev.active { return false; }

        // Read configuration descriptor (first 9 bytes to get total length).
        let mut cfg_buf = [0u8; 64];
        let setup_get_cfg: [u8; 8] = [
            0x80, 0x06,
            0x00, 0x02, // wValue: config descriptor, index 0
            0x00, 0x00,
            0x40, 0x00, // wLength: 64
        ];

        let bytes = match self.control_xfer(slot_idx, &setup_get_cfg, Some(&mut cfg_buf)) {
            Some(n) => n,
            None => return false,
        };
        if bytes < 9 { return false; }

        // Parse: find an Interface descriptor with class=HID (0x03)
        // and an Endpoint descriptor for interrupt-IN.
        let mut offset = 0usize;
        let mut found_hid_iface = false;
        let mut int_in_ep: u8 = 0;
        let mut int_in_max_pkt: u16 = 8;
        let mut int_in_interval: u8 = 10;

        while offset + 2 <= bytes {
            let desc_len = cfg_buf[offset] as usize;
            let desc_type = cfg_buf[offset + 1];
            if desc_len < 2 || offset + desc_len > bytes { break; }

            if desc_type == 4 && desc_len >= 9 { // Interface descriptor
                let iface_class = cfg_buf[offset + 5];
                found_hid_iface = iface_class == 0x03; // HID
            } else if desc_type == 5 && desc_len >= 7 && found_hid_iface { // Endpoint descriptor
                let ep_addr = cfg_buf[offset + 2];
                let ep_attr = cfg_buf[offset + 3];
                if (ep_attr & 0x03) == 0x03 && (ep_addr & 0x80) != 0 {
                    // Interrupt IN endpoint found
                    int_in_ep = ep_addr & 0x0F;
                    int_in_max_pkt = u16::from_le_bytes([cfg_buf[offset + 4], cfg_buf[offset + 5]]);
                    int_in_interval = cfg_buf[offset + 6];
                    break;
                }
            }
            offset += desc_len;
        }

        if int_in_ep == 0 { return false; }

        // Set Configuration (usually config value = 1).
        let setup_set_cfg: [u8; 8] = [
            0x00, 0x09, // SET_CONFIGURATION
            0x01, 0x00, // wValue: config 1
            0x00, 0x00,
            0x00, 0x00,
        ];
        if self.control_xfer(slot_idx, &setup_set_cfg, None).is_none() {
            return false;
        }

        // Prepare Input Context for Configure Endpoint.
        self.input_ctx.clear();

        // Input Control Context: Add flag for the interrupt-IN EP.
        // DCI for EP N IN = 2*N + 1. E.g., EP1 IN = DCI 3.
        let dci = (int_in_ep as u32) * 2 + 1;
        self.input_ctx.write32(0x04, 1 << dci); // Add context flag

        // Slot Context: update Context Entries to include this EP.
        // Copy current slot context from device context.
        let slot_dw0 = self.input_ctx.read32(CTX_SIZE + 0);
        let new_ctx_entries = dci.max((slot_dw0 >> 27) & 0x1F);
        self.input_ctx.write32(CTX_SIZE + 0, (slot_dw0 & !(0x1F << 27)) | (new_ctx_entries << 27));

        // EP Context for interrupt-IN at CTX_SIZE * (dci + 1):
        let ep_off = self.ctx_size * (dci as usize + 1);
        // dword 0: Interval [23:16], CErr [2:1] = 3
        let interval_exp = if int_in_interval > 0 { int_in_interval - 1 } else { 0 };
        self.input_ctx.write32(ep_off, ((interval_exp as u32) << 16) | (3u32 << 1));
        // dword 1: EP Type = 7 (Interrupt IN) [5:3], MaxPacketSize [31:16]
        let ep_dw1 = (7u32 << 3) | ((int_in_max_pkt as u32) << 16);
        self.input_ctx.write32(ep_off + 4, ep_dw1);
        // dword 2-3: TR Dequeue Pointer for this EP's ring.
        self.int_in_rings[slot_idx].init();
        let ring_phys = self.int_in_rings[slot_idx].phys_addr() | 1; // DCS=1
        self.input_ctx.write32(ep_off + 8, ring_phys as u32);
        self.input_ctx.write32(ep_off + 12, (ring_phys >> 32) as u32);

        // Also need to add slot context flag
        let flags = self.input_ctx.read32(0x04);
        self.input_ctx.write32(0x04, flags | 1); // add slot context (A0)

        self.configure_endpoint(self.devices[slot_idx].slot_id)
    }

    /// Set a HID device to boot protocol (simpler fixed-size reports).
    fn set_boot_protocol(&mut self, slot_idx: usize, interface: u8) -> bool {
        let setup = make_set_boot_protocol_setup(interface);
        self.control_xfer(slot_idx, &setup, None).is_some()
    }

    /// Queue an interrupt-IN transfer on the device's interrupt endpoint.
    fn queue_interrupt_in(&mut self, slot_idx: usize, buf: &mut [u8]) {
        let ring = &mut self.int_in_rings[slot_idx];
        let ptr = buf.as_mut_ptr() as u64;
        let ctrl = (TRB_NORMAL << TRB_TYPE_SHIFT) | TRB_IOC;
        ring.enqueue_trb(ptr, buf.len() as u32, ctrl);
        self.ring_int_in_doorbell(self.devices[slot_idx].slot_id);
    }

    /// Poll the event ring for a transfer completion. Non-blocking.
    fn poll_event(&mut self) -> Option<Trb> {
        let evt = self.event_ring.dequeue_event()?;
        unsafe {
            self.write_intr64(ERDP, self.event_ring.dequeue_phys() | (1 << 3));
        }
        Some(evt)
    }

    /// Enumerate all connected ports.
    pub fn enumerate_all(&mut self) {
        for port in 0..self.num_ports.min(MAX_PORTS) {
            if self.ports[port].connected && self.ports[port].slot_id == 0 {
                // Reset the port first.
                if self.port_reset_inner(port).is_some() {
                    self.enumerate_port(port);
                }
            }
        }
        // Configure HID endpoints for any HID devices found.
        for i in 0..MAX_SLOTS {
            if self.devices[i].active
                && (self.devices[i].class == USB_CLASS_HID
                    || self.devices[i].class == 0) // class in interface
            {
                if self.configure_hid_endpoint(i) {
                    // Try to set boot protocol (best-effort).
                    self.set_boot_protocol(i, 0);
                }
            }
        }
    }

    /// Internal port reset (shared between trait impl and enumerate_all).
    fn port_reset_inner(&mut self, port: usize) -> Option<UsbSpeed> {
        if port >= self.num_ports.min(MAX_PORTS) || !self.ports[port].connected {
            return None;
        }
        let portsc = unsafe { self.portsc_read(port) };
        let val = (portsc & 0x0E00_C3E0) | PORTSC_PR;
        unsafe { self.portsc_write(port, val) };
        if !Self::wait_bits(
            self.op_base + OP_PORTSC_BASE + port * 0x10,
            PORTSC_PR, 0, 50_000,
        ) {
            return None;
        }
        let portsc = unsafe { self.portsc_read(port) };
        let speed = Self::decode_speed(portsc);
        self.ports[port].speed = speed;
        self.ports[port].enabled = portsc & PORTSC_PED != 0;
        unsafe { self.portsc_write(port, portsc | PORTSC_CSC) };
        Some(speed)
    }

    /// Write USB device list to a formatter (for shell).
    pub fn write_device_list(&self, w: &mut dyn core::fmt::Write) {
        let _ = writeln!(w, "  Slot  Port  Speed  VID:PID    Class  Description");
        for dev in &self.devices {
            if !dev.active { continue; }
            let spd = match dev.speed {
                UsbSpeed::Low   => "Low  ",
                UsbSpeed::Full  => "Full ",
                UsbSpeed::High  => "High ",
                UsbSpeed::Super => "Super",
            };
            let class_str = match dev.class {
                0x00 => "Composite",
                0x01 => "Audio",
                0x02 => "CDC",
                0x03 => "HID",
                0x08 => "Mass Storage",
                0x09 => "Hub",
                0x0E => "Video",
                0xFF => "Vendor",
                _    => "Unknown",
            };
            let _ = writeln!(w, "  {:>3}   {:>3}   {spd}  {:04X}:{:04X}  0x{:02X}   {class_str}",
                dev.slot_id, dev.port + 1, dev.vendor_id, dev.product_id, dev.class);
        }
        if self.num_devices == 0 {
            let _ = writeln!(w, "  (no devices enumerated)");
        }
    }

    /// Write port status to a formatter (for shell).
    pub fn write_port_list(&self, w: &mut dyn core::fmt::Write) {
        let _ = writeln!(w, "  Port  Status    Speed");
        for i in 0..self.num_ports.min(MAX_PORTS) {
            let p = &self.ports[i];
            let status = if p.enabled { "enabled " }
                         else if p.connected { "connected" }
                         else { "empty    " };
            let spd = if p.connected {
                match p.speed {
                    UsbSpeed::Low   => "Low",
                    UsbSpeed::Full  => "Full",
                    UsbSpeed::High  => "High",
                    UsbSpeed::Super => "Super",
                }
            } else { "-" };
            let _ = writeln!(w, "  {:>3}   {status}  {spd}", i + 1);
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
        self.max_slots = (hcsparams1 & 0xFF) as usize;

        let hccparams1 = unsafe { self.read32(CAP_HCCPARAMS1) };
        // CSZ bit (bit 2): 0 = 32-byte contexts, 1 = 64-byte contexts.
        self.ctx_size = if hccparams1 & (1 << 2) != 0 { 64 } else { 32 };

        let dboff = unsafe { self.read32(CAP_DBOFF) } as usize;
        self.db_base = self.base + dboff;

        let rtsoff = unsafe { self.read32(CAP_RTSOFF) } as usize;
        self.rt_base = self.base + rtsoff;

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
        let max_slots = self.max_slots.min(MAX_SLOTS);
        unsafe { self.op_write32(OP_CONFIG, max_slots as u32) };

        // 6. Set up DMA rings, DCBAA, event ring, and start the controller.
        if !self.setup_dma_and_start() {
            return false;
        }

        // 7. Scan ports for connected devices.
        self.scan_ports();

        // 8. Enumerate all connected devices.
        self.enumerate_all();

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
        self.port_reset_inner(port)
    }

    fn control_transfer(
        &mut self,
        addr: u8,
        setup: &[u8; 8],
        data: Option<&mut [u8]>,
    ) -> Option<usize> {
        // Find device by address (slot_id).
        let slot_idx = self.devices.iter().position(|d| d.active && d.address == addr)?;
        self.control_xfer(slot_idx, setup, data)
    }

    fn interrupt_in(
        &mut self,
        addr: u8,
        _ep: u8,
        buf: &mut [u8],
    ) -> Option<usize> {
        let slot_idx = self.devices.iter().position(|d| d.active && d.address == addr)?;
        self.queue_interrupt_in(slot_idx, buf);
        // Poll for completion (blocking, with timeout).
        for _ in 0..500_000u32 {
            if let Some(evt) = self.poll_event() {
                if evt.trb_type() == TRB_TRANSFER_EVENT {
                    let cc = evt.completion_code();
                    if cc == TRB_COMP_SUCCESS || cc == TRB_COMP_SHORT_PKT {
                        let residual = evt.status & 0xFFFFFF;
                        return Some(buf.len() - residual as usize);
                    }
                    return None;
                }
            }
            core::hint::spin_loop();
        }
        None
    }

    fn device_info(&self, addr: u8) -> Option<UsbDeviceInfo> {
        let dev = self.devices.iter().find(|d| d.active && d.address == addr)?;
        Some(UsbDeviceInfo {
            address: dev.address,
            speed: dev.speed,
            vendor_id: dev.vendor_id,
            product_id: dev.product_id,
            class: dev.class,
            subclass: dev.subclass,
            protocol: dev.protocol,
        })
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

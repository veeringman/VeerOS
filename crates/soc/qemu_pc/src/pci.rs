//! PCI Configuration Space access via I/O ports 0xCF8/0xCFC (mechanism 1).
//!
//! Provides bus enumeration, device discovery, and BAR reading for the
//! x86 PCI host bridge. On QEMU q35, this gives access to all emulated
//! PCI/PCIe devices (VIRTIO-PCI, AHCI, USB xHCI, etc.).

use crate::{inb, outb};

/// PCI config address port.
const PCI_CONFIG_ADDR: u16 = 0x0CF8;
/// PCI config data port.
const PCI_CONFIG_DATA: u16 = 0x0CFC;

/// Maximum number of devices we can track.
pub const MAX_PCI_DEVICES: usize = 32;

/// Represents a discovered PCI device.
#[derive(Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub header_type: u8,
    pub irq_line: u8,
}

impl PciDevice {
    pub const EMPTY: Self = Self {
        bus: 0,
        device: 0,
        function: 0,
        vendor_id: 0xFFFF,
        device_id: 0xFFFF,
        class_code: 0,
        subclass: 0,
        prog_if: 0,
        header_type: 0,
        irq_line: 0xFF,
    };

    /// Returns `true` if this slot is populated.
    pub fn is_present(&self) -> bool {
        self.vendor_id != 0xFFFF
    }

    /// Returns the BDF (Bus:Device.Function) as a human-readable string.
    pub fn bdf_fmt(&self, buf: &mut [u8; 12]) -> usize {
        // Format: "BB:DD.F"
        let mut i = 0;
        fn hex_nibble(v: u8) -> u8 {
            if v < 10 { b'0' + v } else { b'a' + v - 10 }
        }
        buf[i] = hex_nibble(self.bus >> 4); i += 1;
        buf[i] = hex_nibble(self.bus & 0xF); i += 1;
        buf[i] = b':'; i += 1;
        buf[i] = hex_nibble(self.device >> 4); i += 1;
        buf[i] = hex_nibble(self.device & 0xF); i += 1;
        buf[i] = b'.'; i += 1;
        buf[i] = hex_nibble(self.function & 0x7); i += 1;
        i
    }
}

/// PCI device table — holds all discovered devices.
pub struct PciDevices {
    pub devices: [PciDevice; MAX_PCI_DEVICES],
    pub count: usize,
}

impl PciDevices {
    pub const fn new() -> Self {
        Self {
            devices: [PciDevice::EMPTY; MAX_PCI_DEVICES],
            count: 0,
        }
    }

    /// Add a device to the table. Returns the index if successful.
    pub fn add(&mut self, dev: PciDevice) -> Option<usize> {
        if self.count >= MAX_PCI_DEVICES {
            return None;
        }
        let idx = self.count;
        self.devices[idx] = dev;
        self.count += 1;
        Some(idx)
    }
}

// ─── Config space I/O ───────────────────────────────────────────────────

/// Build a PCI configuration address for mechanism 1.
fn pci_addr(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    (1u32 << 31) // enable bit
        | ((bus as u32) << 16)
        | (((device & 0x1F) as u32) << 11)
        | (((function & 0x07) as u32) << 8)
        | ((offset & 0xFC) as u32) // align to DWORD
}

/// Read a 32-bit value from PCI configuration space.
pub fn config_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let addr = pci_addr(bus, device, function, offset);
    unsafe {
        // Write address as 4 bytes via outb (can also use outl).
        let addr_bytes = addr.to_le_bytes();
        outb(PCI_CONFIG_ADDR, addr_bytes[0]);
        outb(PCI_CONFIG_ADDR + 1, addr_bytes[1]);
        outb(PCI_CONFIG_ADDR + 2, addr_bytes[2]);
        outb(PCI_CONFIG_ADDR + 3, addr_bytes[3]);

        // Read data.
        let b0 = inb(PCI_CONFIG_DATA) as u32;
        let b1 = (inb(PCI_CONFIG_DATA + 1) as u32) << 8;
        let b2 = (inb(PCI_CONFIG_DATA + 2) as u32) << 16;
        let b3 = (inb(PCI_CONFIG_DATA + 3) as u32) << 24;
        b0 | b1 | b2 | b3
    }
}

/// Read a 16-bit value from PCI configuration space.
pub fn config_read16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let dword = config_read32(bus, device, function, offset & 0xFC);
    let shift = ((offset & 0x02) as u32) * 8;
    ((dword >> shift) & 0xFFFF) as u16
}

/// Read an 8-bit value from PCI configuration space.
pub fn config_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let dword = config_read32(bus, device, function, offset & 0xFC);
    let shift = ((offset & 0x03) as u32) * 8;
    ((dword >> shift) & 0xFF) as u8
}

/// Write a 32-bit value to PCI configuration space.
pub fn config_write32(bus: u8, device: u8, function: u8, offset: u8, val: u32) {
    let addr = pci_addr(bus, device, function, offset);
    unsafe {
        let addr_bytes = addr.to_le_bytes();
        outb(PCI_CONFIG_ADDR, addr_bytes[0]);
        outb(PCI_CONFIG_ADDR + 1, addr_bytes[1]);
        outb(PCI_CONFIG_ADDR + 2, addr_bytes[2]);
        outb(PCI_CONFIG_ADDR + 3, addr_bytes[3]);

        let val_bytes = val.to_le_bytes();
        outb(PCI_CONFIG_DATA, val_bytes[0]);
        outb(PCI_CONFIG_DATA + 1, val_bytes[1]);
        outb(PCI_CONFIG_DATA + 2, val_bytes[2]);
        outb(PCI_CONFIG_DATA + 3, val_bytes[3]);
    }
}

/// Read a BAR (Base Address Register) from PCI config space.
///
/// BAR 0 is at offset 0x10, BAR 1 at 0x14, ..., BAR 5 at 0x24.
/// Returns the raw BAR value. Bit 0: 0=MMIO, 1=I/O.
pub fn read_bar(bus: u8, device: u8, function: u8, bar_index: u8) -> u32 {
    let offset = 0x10 + bar_index * 4;
    config_read32(bus, device, function, offset)
}

/// Determine BAR size by writing all-1s, reading back, masking, and
/// computing size = ~(masked_value) + 1.
pub fn bar_size(bus: u8, device: u8, function: u8, bar_index: u8) -> u32 {
    let offset = 0x10 + bar_index * 4;
    let orig = config_read32(bus, device, function, offset);
    config_write32(bus, device, function, offset, 0xFFFF_FFFF);
    let raw = config_read32(bus, device, function, offset);
    config_write32(bus, device, function, offset, orig); // restore

    if raw == 0 {
        return 0;
    }

    let is_io = (raw & 1) != 0;
    let mask = if is_io { raw & 0xFFFF_FFFC } else { raw & 0xFFFF_FFF0 };
    (!mask).wrapping_add(1)
}

/// Enumerate all PCI devices on bus 0 (single-segment, no bridges).
///
/// For a full enumeration you'd walk bridges recursively; this is
/// sufficient for QEMU q35 with a flat device layout.
pub fn enumerate(table: &mut PciDevices) {
    for bus in 0u8..=0 {
        for dev in 0u8..32 {
            let vendor = config_read16(bus, dev, 0, 0x00);
            if vendor == 0xFFFF {
                continue; // no device
            }

            let header_type = config_read8(bus, dev, 0, 0x0E);
            let max_func = if header_type & 0x80 != 0 { 8u8 } else { 1u8 };

            for func in 0..max_func {
                let vendor = config_read16(bus, dev, func, 0x00);
                if vendor == 0xFFFF {
                    continue;
                }
                let device_id = config_read16(bus, dev, func, 0x02);
                let rev_class = config_read32(bus, dev, func, 0x08);
                let class_code = ((rev_class >> 24) & 0xFF) as u8;
                let subclass = ((rev_class >> 16) & 0xFF) as u8;
                let prog_if = ((rev_class >> 8) & 0xFF) as u8;
                let ht = config_read8(bus, dev, func, 0x0E);
                let irq = config_read8(bus, dev, func, 0x3C);

                table.add(PciDevice {
                    bus,
                    device: dev,
                    function: func,
                    vendor_id: vendor,
                    device_id,
                    class_code,
                    subclass,
                    prog_if,
                    header_type: ht & 0x7F,
                    irq_line: irq,
                });
            }
        }
    }
}

/// Get a human-readable PCI class description.
pub fn class_name(class_code: u8, subclass: u8) -> &'static str {
    match (class_code, subclass) {
        (0x00, _) => "Unclassified",
        (0x01, 0x00) => "SCSI",
        (0x01, 0x01) => "IDE",
        (0x01, 0x06) => "SATA",
        (0x01, 0x08) => "NVMe",
        (0x01, _) => "Storage",
        (0x02, 0x00) => "Ethernet",
        (0x02, 0x80) => "Network",
        (0x02, _) => "Network",
        (0x03, 0x00) => "VGA",
        (0x03, _) => "Display",
        (0x04, _) => "Multimedia",
        (0x05, _) => "Memory",
        (0x06, 0x00) => "Host Bridge",
        (0x06, 0x01) => "ISA Bridge",
        (0x06, 0x04) => "PCI Bridge",
        (0x06, _) => "Bridge",
        (0x07, _) => "Serial",
        (0x08, _) => "System",
        (0x0C, 0x03) => "USB",
        (0x0C, _) => "Serial Bus",
        _ => "Unknown",
    }
}

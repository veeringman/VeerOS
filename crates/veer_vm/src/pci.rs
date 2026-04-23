//! Minimal PCI host bridge: Configuration Mechanism #1 (ports 0xCF8/0xCFC).
//!
//! We emulate just enough for VeerOS's `soc/qemu_pc/src/pci.rs` enumerator to
//! find the virtio-blk-pci device at BDF (0, 0, 0). One device, one bus, no
//! bridges. BAR0 is an I/O BAR whose port range dispatches to the device.
//!
//! Layout:
//!   * 0xCF8 (4 bytes) — CONFIG_ADDRESS (bit 31 = enable).
//!   * 0xCFC (1/2/4 bytes) — CONFIG_DATA, access through the currently
//!     latched address. The low two bits of the address select a byte offset
//!     within the dword for unaligned accesses.

use std::sync::Arc;
use std::sync::Mutex;

use crate::virtio::blk::VirtioBlk;
use crate::virtio::VirtioTransport;

pub const PCI_CONFIG_ADDR: u16 = 0xCF8;
pub const PCI_CONFIG_DATA: u16 = 0xCFC;

/// The virtio-blk-pci device lives at BDF (0, 0, 0) and uses I/O BAR0 at
/// this base port. Size is 256 bytes (well above the 0x14 + device-config
/// footprint that the driver touches).
pub const VIRTIO_BLK_PIO_BASE: u16 = 0xC000;
pub const VIRTIO_BLK_PIO_SIZE: u16 = 256;

/// PCI IRQ line we advertise in the config-space IRQ field. Not actually
/// used — the guest virtio-blk driver polls the used ring instead of
/// enabling interrupts. Kept for completeness / future use.
pub const VIRTIO_BLK_IRQ: u8 = 11;

/// Virtio PCI vendor & transitional device IDs.
const VIRTIO_VENDOR: u16 = 0x1AF4;
const VIRTIO_DEV_BLK: u16 = 0x1001;

/// Mass storage / other — guest only looks at vendor+device so this is
/// mostly cosmetic.
const CLASS_MASS_STORAGE: u8 = 0x01;
const SUBCLASS_OTHER: u8 = 0x80;

/// Subsystem device ID for virtio-blk per the legacy spec.
const VIRTIO_SUBSYSTEM_BLK: u16 = 0x0002;

/// BAR0 writable-bits mask. Size = 256 bytes → low 8 bits are read-only.
/// Bit 0 is also read-only (= 1, marking this as an I/O BAR).
const BAR0_WRITABLE_MASK: u32 = 0xFFFF_FF00;

pub struct PciHost {
    /// Latched CONFIG_ADDRESS (ports 0xCF8..=0xCFB, 32-bit write).
    addr: u32,
    /// The single virtio-blk device at BDF (0, 0, 0).
    blk: Option<Arc<Mutex<VirtioBlk>>>,
    /// Writable portion of BAR0. Low 8 bits are always zero; bit 0 reads as 1.
    bar0_writable: u32,
    /// Command register (we track it but ignore effects; guest usually
    /// enables I/O space by setting bit 0).
    command: u16,
}

impl PciHost {
    pub fn new() -> Self {
        Self {
            addr: 0,
            blk: None,
            // Default BAR0 address (pre-configured so that guest's probing
            // still finds it even if it never rewrites BAR0; real drivers
            // always rewrite, but defensive).
            bar0_writable: VIRTIO_BLK_PIO_BASE as u32 & BAR0_WRITABLE_MASK,
            command: 0,
        }
    }

    pub fn set_blk(&mut self, dev: Arc<Mutex<VirtioBlk>>) {
        self.blk = Some(dev);
    }

    /// Currently configured BAR0 base port (minus the I/O marker bit).
    pub fn bar0_base(&self) -> u16 {
        (self.bar0_writable & BAR0_WRITABLE_MASK) as u16
    }

    /// Write to CONFIG_ADDRESS (0xCF8, must be 4 bytes).
    pub fn write_addr(&mut self, val: u32) {
        self.addr = val;
    }

    /// Read CONFIG_ADDRESS.
    pub fn read_addr(&self) -> u32 {
        self.addr
    }

    /// Decode the currently latched address. Returns `None` if bit 31
    /// (enable) is clear, the bus / function is non-zero, or the device
    /// slot is not populated.
    fn decoded(&self) -> Option<DecodedAddr> {
        if self.addr & 0x8000_0000 == 0 {
            return None;
        }
        let bus = ((self.addr >> 16) & 0xFF) as u8;
        let dev = ((self.addr >> 11) & 0x1F) as u8;
        let func = ((self.addr >> 8) & 0x07) as u8;
        let offset = (self.addr & 0xFC) as u8;
        if bus != 0 || func != 0 || dev != 0 {
            return None;
        }
        if self.blk.is_none() {
            return None;
        }
        Some(DecodedAddr { offset })
    }

    /// Read CONFIG_DATA, honouring unaligned sub-dword accesses via the low
    /// two bits of CONFIG_ADDRESS. `width` is 1, 2 or 4.
    pub fn read_data(&self, width: usize, buf: &mut [u8]) {
        let byte_offset = (self.addr & 0x03) as usize;
        let dword = self.decoded()
            .map(|d| self.read_dword(d.offset))
            .unwrap_or(0xFFFF_FFFF);
        let bytes = dword.to_le_bytes();
        let n = width.min(buf.len()).min(4 - byte_offset);
        buf[..n].copy_from_slice(&bytes[byte_offset..byte_offset + n]);
        for b in &mut buf[n..] {
            *b = 0xFF;
        }
    }

    /// Write to CONFIG_DATA.
    pub fn write_data(&mut self, width: usize, data: &[u8]) {
        let byte_offset = (self.addr & 0x03) as usize;
        let Some(d) = self.decoded() else { return; };
        // Read-modify-write at dword granularity.
        let mut dword = self.read_dword(d.offset);
        let mut bytes = dword.to_le_bytes();
        let n = width.min(data.len()).min(4 - byte_offset);
        bytes[byte_offset..byte_offset + n].copy_from_slice(&data[..n]);
        dword = u32::from_le_bytes(bytes);
        self.write_dword(d.offset, dword);
    }

    fn read_dword(&self, offset: u8) -> u32 {
        match offset {
            0x00 => (VIRTIO_DEV_BLK as u32) << 16 | VIRTIO_VENDOR as u32,
            0x04 => {
                // command (low16) | status (high16). Status: capabilities bit(4)=0, etc.
                self.command as u32
            }
            0x08 => {
                // rev(0) | prog_if(0) | subclass | class
                (CLASS_MASS_STORAGE as u32) << 24
                    | (SUBCLASS_OTHER as u32) << 16
            }
            0x0C => 0, // cache line / latency / header type / BIST — header_type=0
            0x10 => self.bar0_writable | 0x1, // I/O BAR marker
            0x14 | 0x18 | 0x1C | 0x20 | 0x24 => 0, // BAR1..BAR5 unpopulated
            0x28 => 0, // cardbus CIS
            0x2C => {
                // subsystem vendor (low16) | subsystem id (high16)
                VIRTIO_VENDOR as u32 | (VIRTIO_SUBSYSTEM_BLK as u32) << 16
            }
            0x30 => 0, // expansion ROM base
            0x34 => 0, // capabilities ptr (no caps — legacy)
            0x38 => 0, // reserved
            0x3C => {
                // irq_line | irq_pin(1 = INTA) | min_gnt | max_lat
                VIRTIO_BLK_IRQ as u32 | (1u32 << 8)
            }
            _ => 0,
        }
    }

    fn write_dword(&mut self, offset: u8, val: u32) {
        match offset {
            0x04 => {
                self.command = (val & 0xFFFF) as u16;
            }
            0x10 => {
                // BAR0: store writable bits only. The BAR-sizing trick
                // (write 0xFFFFFFFF, read back) works automatically because
                // the read path ORs bit 0 back in and low 8 bits stay 0.
                self.bar0_writable = val & BAR0_WRITABLE_MASK;
            }
            _ => {
                // Silently ignore writes to other read-only fields.
            }
        }
    }
}

struct DecodedAddr {
    offset: u8,
}

/// Get a reference to the virtio-blk device if it's installed (for the PIO
/// dispatcher that needs to route BAR0 I/O writes).
impl PciHost {
    pub fn blk(&self) -> Option<Arc<Mutex<VirtioBlk>>> {
        self.blk.clone()
    }
}

//! Minimal PCI host bridge: Configuration Mechanism #1 (ports 0xCF8/0xCFC).
//!
//! Hosts up to two virtio-legacy-PCI devices on bus 0:
//!   * BDF 0:0.0 — virtio-blk-pci (I/O BAR0 @ `VIRTIO_BLK_PIO_BASE`)
//!   * BDF 0:1.0 — virtio-net-pci (I/O BAR0 @ `VIRTIO_NET_PIO_BASE`)
//!
//! Each device has its own BAR0 writable register so the guest can
//! reprogram them independently during enumeration. Other BARs are
//! unpopulated; MSI-X is disabled.

use std::sync::Arc;
use std::sync::Mutex;

use crate::virtio::blk::VirtioBlk;
use crate::virtio::net::VirtioNet;

pub const PCI_CONFIG_ADDR: u16 = 0xCF8;
pub const PCI_CONFIG_DATA: u16 = 0xCFC;

/// Default BAR0 window for virtio-blk-pci (256 bytes).
pub const VIRTIO_BLK_PIO_BASE: u16 = 0xC000;
pub const VIRTIO_BLK_PIO_SIZE: u16 = 256;

/// Default BAR0 window for virtio-net-pci (256 bytes, immediately after blk).
pub const VIRTIO_NET_PIO_BASE: u16 = 0xC100;
pub const VIRTIO_NET_PIO_SIZE: u16 = 256;

/// Advertised IRQ lines (drivers poll the rings — IRQs unused).
pub const VIRTIO_BLK_IRQ: u8 = 11;
pub const VIRTIO_NET_IRQ: u8 = 10;

// ── Virtio PCI IDs (transitional / legacy) ───────────────────────────────
const VIRTIO_VENDOR: u16 = 0x1AF4;
const VIRTIO_DEV_BLK: u16 = 0x1001;
const VIRTIO_DEV_NET: u16 = 0x1000;

const VIRTIO_SUBSYSTEM_BLK: u16 = 0x0002;
const VIRTIO_SUBSYSTEM_NET: u16 = 0x0001;

const CLASS_MASS_STORAGE: u8 = 0x01;
const CLASS_NETWORK: u8 = 0x02;
const SUBCLASS_OTHER: u8 = 0x80;

/// BAR0 writable mask — low 8 bits read-only (size = 256 bytes), bit 0
/// reads back as 1 (I/O BAR marker).
const BAR0_WRITABLE_MASK: u32 = 0xFFFF_FF00;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Blk = 0,
    Net = 1,
}

pub struct PciHost {
    /// Latched CONFIG_ADDRESS.
    addr: u32,

    // Slot 0: virtio-blk
    blk: Option<Arc<Mutex<VirtioBlk>>>,
    bar0_blk: u32,
    cmd_blk: u16,

    // Slot 1: virtio-net
    net: Option<Arc<Mutex<VirtioNet>>>,
    bar0_net: u32,
    cmd_net: u16,
}

impl PciHost {
    pub fn new() -> Self {
        Self {
            addr: 0,
            blk: None,
            bar0_blk: VIRTIO_BLK_PIO_BASE as u32 & BAR0_WRITABLE_MASK,
            cmd_blk: 0,
            net: None,
            bar0_net: VIRTIO_NET_PIO_BASE as u32 & BAR0_WRITABLE_MASK,
            cmd_net: 0,
        }
    }

    pub fn set_blk(&mut self, dev: Arc<Mutex<VirtioBlk>>) { self.blk = Some(dev); }
    pub fn set_net(&mut self, dev: Arc<Mutex<VirtioNet>>) { self.net = Some(dev); }

    /// Current BAR0 base port for the virtio-blk device (0 if absent).
    pub fn blk_bar0_base(&self) -> u16 {
        if self.blk.is_none() { return 0; }
        (self.bar0_blk & BAR0_WRITABLE_MASK) as u16
    }

    /// Current BAR0 base port for the virtio-net device (0 if absent).
    pub fn net_bar0_base(&self) -> u16 {
        if self.net.is_none() { return 0; }
        (self.bar0_net & BAR0_WRITABLE_MASK) as u16
    }

    pub fn write_addr(&mut self, val: u32) { self.addr = val; }
    pub fn read_addr(&self) -> u32 { self.addr }

    fn decoded(&self) -> Option<(Slot, u8)> {
        if self.addr & 0x8000_0000 == 0 { return None; }
        let bus = ((self.addr >> 16) & 0xFF) as u8;
        let dev = ((self.addr >> 11) & 0x1F) as u8;
        let func = ((self.addr >> 8) & 0x07) as u8;
        let offset = (self.addr & 0xFC) as u8;
        if bus != 0 || func != 0 { return None; }
        let slot = match dev {
            0 if self.blk.is_some() => Slot::Blk,
            1 if self.net.is_some() => Slot::Net,
            _ => return None,
        };
        Some((slot, offset))
    }

    pub fn read_data(&self, width: usize, buf: &mut [u8]) {
        let byte_offset = (self.addr & 0x03) as usize;
        let dword = self.decoded()
            .map(|(s, off)| self.read_dword(s, off))
            .unwrap_or(0xFFFF_FFFF);
        let bytes = dword.to_le_bytes();
        let n = width.min(buf.len()).min(4 - byte_offset);
        buf[..n].copy_from_slice(&bytes[byte_offset..byte_offset + n]);
        for b in &mut buf[n..] { *b = 0xFF; }
    }

    pub fn write_data(&mut self, width: usize, data: &[u8]) {
        let byte_offset = (self.addr & 0x03) as usize;
        let Some((slot, offset)) = self.decoded() else { return; };
        let mut bytes = self.read_dword(slot, offset).to_le_bytes();
        let n = width.min(data.len()).min(4 - byte_offset);
        bytes[byte_offset..byte_offset + n].copy_from_slice(&data[..n]);
        self.write_dword(slot, offset, u32::from_le_bytes(bytes));
    }

    fn read_dword(&self, slot: Slot, offset: u8) -> u32 {
        match slot {
            Slot::Blk => self.read_blk_dword(offset),
            Slot::Net => self.read_net_dword(offset),
        }
    }

    fn write_dword(&mut self, slot: Slot, offset: u8, val: u32) {
        match slot {
            Slot::Blk => self.write_blk_dword(offset, val),
            Slot::Net => self.write_net_dword(offset, val),
        }
    }

    fn read_blk_dword(&self, offset: u8) -> u32 {
        match offset {
            0x00 => (VIRTIO_DEV_BLK as u32) << 16 | VIRTIO_VENDOR as u32,
            0x04 => self.cmd_blk as u32,
            0x08 => (CLASS_MASS_STORAGE as u32) << 24 | (SUBCLASS_OTHER as u32) << 16,
            0x0C => 0,
            0x10 => self.bar0_blk | 0x1,
            0x14 | 0x18 | 0x1C | 0x20 | 0x24 => 0,
            0x28 => 0,
            0x2C => VIRTIO_VENDOR as u32 | (VIRTIO_SUBSYSTEM_BLK as u32) << 16,
            0x30 => 0,
            0x34 => 0,
            0x38 => 0,
            0x3C => VIRTIO_BLK_IRQ as u32 | (1u32 << 8),
            _ => 0,
        }
    }

    fn write_blk_dword(&mut self, offset: u8, val: u32) {
        match offset {
            0x04 => { self.cmd_blk = (val & 0xFFFF) as u16; }
            0x10 => { self.bar0_blk = val & BAR0_WRITABLE_MASK; }
            _ => {}
        }
    }

    fn read_net_dword(&self, offset: u8) -> u32 {
        match offset {
            0x00 => (VIRTIO_DEV_NET as u32) << 16 | VIRTIO_VENDOR as u32,
            0x04 => self.cmd_net as u32,
            0x08 => (CLASS_NETWORK as u32) << 24 | (SUBCLASS_OTHER as u32) << 16,
            0x0C => 0,
            0x10 => self.bar0_net | 0x1,
            0x14 | 0x18 | 0x1C | 0x20 | 0x24 => 0,
            0x28 => 0,
            0x2C => VIRTIO_VENDOR as u32 | (VIRTIO_SUBSYSTEM_NET as u32) << 16,
            0x30 => 0,
            0x34 => 0,
            0x38 => 0,
            0x3C => VIRTIO_NET_IRQ as u32 | (1u32 << 8),
            _ => 0,
        }
    }

    fn write_net_dword(&mut self, offset: u8, val: u32) {
        match offset {
            0x04 => { self.cmd_net = (val & 0xFFFF) as u16; }
            0x10 => { self.bar0_net = val & BAR0_WRITABLE_MASK; }
            _ => {}
        }
    }
}

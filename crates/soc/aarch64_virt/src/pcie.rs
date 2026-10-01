//! Minimal PCIe ECAM access for the QEMU `virt` board.
//!
//! The `virt` machine maps the PCIe enhanced configuration space at
//! `0x1000_0000` (16 MiB → 256 buses). No host-bridge setup is needed for
//! bus 0: config space is directly addressable and devices on bus 0 just
//! work. This module scans bus 0 for the Bochs display (`1234:1111`) and
//! assigns its BARs, since no firmware runs before us to do it.
//!
//! Absence of a display (e.g. under `veer-vm`/HVF without PCI) is not an
//! error — probing simply finds nothing and the kernel keeps its serial
//! console.

/// PCIe ECAM base on QEMU `virt` (64-bit window at 16 GiB + 256 MiB;
/// the 32-bit `0x1000_0000` range is MMIO, not config space).
pub const ECAM_BASE: usize = 0x40_1000_0000;

/// Vendor/device we look for: QEMU `bochs-display` / standard VGA.
pub const BOCHS_VENDOR: u16 = 0x1234;
pub const BOCHS_DEVICE: u16 = 0x1111;

/// Where we park assigned BARs (inside the PCIe MMIO window, clear of
/// ECAM `0x1000_0000..0x10FF_FFFF`, devices `< 0x0B00_0000`, and DRAM).
pub const LFB_BAR_ADDR: u32 = 0x1200_0000;
/// 16 MiB default LFB (`vga_mem_mb`); matched by QEMU unless overridden.
pub const LFB_BAR_SIZE: u32 = 16 * 1024 * 1024;
/// Dispi MMIO register block (4 KiB).
pub const DISPI_BAR_ADDR: u32 = 0x1300_0000;

/// PCI command register bits we enable: MEM + BusMaster.
const PCI_CMD_MEM: u16 = 0x0002;
const PCI_CMD_MASTER: u16 = 0x0004;

fn ecam_addr(bus: u8, dev: u8, func: u8, offset: u16) -> usize {
    ECAM_BASE
        + ((bus as usize) << 20)
        + ((dev as usize) << 15)
        + ((func as usize) << 12)
        + (offset as usize)
}

#[inline]
fn cfg_read32(bus: u8, dev: u8, func: u8, offset: u16) -> u32 {
    unsafe { core::ptr::read_volatile(ecam_addr(bus, dev, func, offset) as *const u32) }
}

#[inline]
fn cfg_read16(bus: u8, dev: u8, func: u8, offset: u16) -> u16 {
    unsafe { core::ptr::read_volatile(ecam_addr(bus, dev, func, offset) as *const u16) }
}

#[inline]
fn cfg_write32(bus: u8, dev: u8, func: u8, offset: u16, val: u32) {
    unsafe { core::ptr::write_volatile(ecam_addr(bus, dev, func, offset) as *mut u32, val) }
}

#[inline]
fn cfg_write16(bus: u8, dev: u8, func: u8, offset: u16, val: u16) {
    unsafe { core::ptr::write_volatile(ecam_addr(bus, dev, func, offset) as *mut u16, val) }
}

/// A Bochs display found on the bus, before BAR assignment.
#[derive(Clone, Copy, Debug)]
pub struct BochsPci {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
}

/// Any PCI device header we care about.
#[derive(Clone, Copy, Debug)]
pub struct PciId {
    pub bus: u8,
    pub dev: u8,
    pub func: u8,
    pub vendor: u16,
    pub device: u16,
    pub class: u8,
}

impl PciId {
    pub const EMPTY: Self = Self {
        bus: 0,
        dev: 0,
        func: 0,
        vendor: 0,
        device: 0,
        class: 0,
    };
}

/// Scan bus 0 for devices with `vendor`. Up to `out.len()` hits.
pub fn find_by_vendor(vendor: u16, out: &mut [PciId]) -> usize {
    let mut n = 0;
    for dev in 0u8..32 {
        if cfg_read16(0, dev, 0, 0x00) == 0xFFFF {
            continue;
        }
        let header = (cfg_read32(0, dev, 0, 0x0C) >> 16) & 0xFF;
        let funcs = if header & 0x80 != 0 { 8u8 } else { 1u8 };
        for func in 0..funcs {
            if cfg_read16(0, dev, func, 0x00) != vendor {
                continue;
            }
            if n < out.len() {
                let cls = (cfg_read32(0, dev, func, 0x08) >> 24) as u8;
                out[n] = PciId {
                    bus: 0,
                    dev,
                    func,
                    vendor,
                    device: cfg_read16(0, dev, func, 0x02),
                    class: cls,
                };
                n += 1;
            }
        }
    }
    n
}

impl PciId {
    fn bar_offset(index: u8) -> u16 {
        0x10 + (index as u16) * 4
    }

    /// Raw BAR value (bit 0: 0 = MMIO).
    pub fn bar_raw(&self, index: u8) -> u32 {
        cfg_read32(self.bus, self.dev, self.func, Self::bar_offset(index))
    }

    /// Assign a 32-bit MMIO BAR.
    pub fn assign_bar(&self, index: u8, addr: u32) {
        cfg_write32(self.bus, self.dev, self.func, Self::bar_offset(index), addr);
    }

    /// Enable MEM + bus mastering.
    pub fn enable_mem(&self) {
        let cmd = cfg_read16(self.bus, self.dev, self.func, 0x04);
        cfg_write16(
            self.bus,
            self.dev,
            self.func,
            0x04,
            cmd | PCI_CMD_MEM | PCI_CMD_MASTER,
        );
    }

    /// First capability pointer (0 = none). Requires status bit 4 set.
    pub fn cap_ptr(&self) -> u8 {
        let status = cfg_read16(self.bus, self.dev, self.func, 0x06);
        if status & 0x10 == 0 {
            return 0;
        }
        (cfg_read32(self.bus, self.dev, self.func, 0x34) & 0xFF) as u8
    }

    /// Read config byte (for walking capability lists).
    pub fn cfg8(&self, offset: u16) -> u8 {
        let dword = cfg_read32(self.bus, self.dev, self.func, offset & 0xFC);
        ((dword >> ((offset & 3) * 8)) & 0xFF) as u8
    }

    /// Read config dword.
    pub fn cfg32(&self, offset: u16) -> u32 {
        cfg_read32(self.bus, self.dev, self.func, offset & 0xFC)
    }
}

/// Scan bus 0 for `1234:1111`. Returns `None` when absent (HVF/serial-only).
pub fn find_bochs_display() -> Option<BochsPci> {
    for dev in 0u8..32 {
        if cfg_read16(0, dev, 0, 0x00) == 0xFFFF {
            continue;
        }
        let header = (cfg_read32(0, dev, 0, 0x0C) >> 16) & 0xFF;
        let funcs = if header & 0x80 != 0 { 8u8 } else { 1u8 };
        for func in 0..funcs {
            if cfg_read16(0, dev, func, 0x00) != BOCHS_VENDOR {
                continue;
            }
            if cfg_read16(0, dev, func, 0x02) != BOCHS_DEVICE {
                continue;
            }
            return Some(BochsPci { bus: 0, dev, func });
        }
    }
    None
}

impl BochsPci {
    /// Assign BARs and enable MEM + bus mastering. Returns
    /// `(lfb_phys, dispi_mmio_base)`.
    pub fn assign_bars(&self) -> (usize, usize) {
        let id = PciId {
            bus: self.bus,
            dev: self.dev,
            func: self.func,
            vendor: BOCHS_VENDOR,
            device: BOCHS_DEVICE,
            class: 0x03,
        };
        // BAR0: 32-bit prefetchable LFB.
        id.assign_bar(0, LFB_BAR_ADDR);
        // BAR2: 32-bit non-prefetchable dispi MMIO.
        id.assign_bar(2, DISPI_BAR_ADDR);
        id.enable_mem();
        (LFB_BAR_ADDR as usize, DISPI_BAR_ADDR as usize)
    }
}

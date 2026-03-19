//! BRCM PCIe host controller driver for BCM2712 (Raspberry Pi 5).
//!
//! The BCM2712 SoC has multiple PCIe root complexes. The RP1 southbridge
//! is attached to PCIe2 at `0x10_0012_0000` on Raspberry Pi 5.
//!
//! This driver:
//!   1. Scans all PCIe controllers for an active link
//!   2. Reads the RP1 BAR from PCIe config space
//!   3. Programs the outbound memory window so CPU can access RP1 MMIO
//!
//! Reference: Linux `drivers/pci/controller/pcie-brcmstb.c`

use core::ptr::{read_volatile, write_volatile};

// ─── BCM2712 PCIe controller base addresses ─────────────────────
// From bcm2712-rpi-5-b.dtb (under axi node, identity-mapped):
//   pcie@1000100000 → 0x10_0010_0000  reg size 0x9310, domain 0, 1 lane
//   pcie@1000110000 → 0x10_0011_0000  reg size 0x9310, domain 1, 1 lane
//   pcie@1000120000 → 0x10_0012_0000  reg size 0x9310, domain 2, 4 lanes ← RP1

const PCIE_CONTROLLERS: [(usize, &str); 3] = [
    (0x10_0010_0000, "PCIe0"),
    (0x10_0011_0000, "PCIe1"),
    (0x10_0012_0000, "PCIe2"),
];

// ─── BRCM PCIe register offsets ─────────────────────────────────
// Misc registers
const MISC_CPU_2_PCIE_LO:   usize = 0x400C;
const MISC_CPU_2_PCIE_HI:   usize = 0x4010;
const MISC_PCIE_CTRL:       usize = 0x4064;
const MISC_PCIE_STATUS:     usize = 0x4068;
const MISC_MISC_CTRL:       usize = 0x4008;
const MISC_MEM_WIN0_BASELIM: usize = 0x4070;
const MISC_MEM_WIN0_BASE_HI: usize = 0x4080;
const MISC_MEM_WIN0_LIM_HI: usize = 0x4084;
const RGR1_SW_INIT_1:       usize = 0x9210;
const HARD_DEBUG:           usize = 0x4304;

// RC config registers used by Linux bring-up sequence.
const RC_CFG_PRIV1_ID_VAL3: usize = 0x043C;
const RC_CFG_VENDOR_SPEC1:  usize = 0x0188;

// PCIE_MISC_MISC_CTRL bit fields (from Linux pcie-brcmstb).
const MISC_CTRL_RCB_64B_MODE: u32 = 0x0000_0080;
const MISC_CTRL_RCB_MPS_MODE: u32 = 0x0000_0400;
const MISC_CTRL_SCB_ACCESS_EN: u32 = 0x0000_1000;
const MISC_CTRL_CFG_READ_UR_MODE: u32 = 0x0000_2000;
const MISC_CTRL_MAX_BURST_SIZE_MASK: u32 = 0x0030_0000;
const MISC_CTRL_MAX_BURST_512B: u32 = 0x0020_0000;

// Reset/control bits from Linux brcmstb.
const SW_INIT_GENERIC_MASK: u32 = 0x2;          // bridge sw init bit
const PCIE_CTRL_PERSTB_MASK: u32 = 0x4;         // inverted meaning on bcm2712
const HARD_DEBUG_SERDES_IDDQ_MASK: u32 = 0x0800_0000;

// Extended config space access
const EXT_CFG_DATA:         usize = 0x8000;
const EXT_CFG_INDEX:        usize = 0x9000;

// RP1 identification
const RP1_VENDOR_DEVICE: u32 = 0x0001_1de4; // VID=0x1de4, DID=0x0001

// ─── PCIe link status bits ──────────────────────────────────────
const STATUS_PHYLINKUP:  u32 = 1 << 4;
const STATUS_DL_ACTIVE:  u32 = 1 << 5;
const STATUS_LINK_MASK:  u32 = STATUS_PHYLINKUP | STATUS_DL_ACTIVE;

// ─── Result of RP1 discovery ────────────────────────────────────

/// Information about a discovered RP1 southbridge.
#[derive(Clone, Copy)]
pub struct Rp1Info {
    /// PCIe controller base this RP1 is attached to.
    pub pcie_base: usize,
    /// CPU-visible base address of RP1 BAR (after outbound window setup).
    pub bar_base: usize,
    /// Size of the BAR region.
    pub bar_size: usize,
    /// PCIe controller name (for diagnostics).
    pub controller: &'static str,
}

// ─── Safe MMIO helpers ──────────────────────────────────────────

#[inline]
unsafe fn read32(addr: usize) -> u32 {
    read_volatile(addr as *const u32)
}

#[inline]
unsafe fn write32(addr: usize, val: u32) {
    write_volatile(addr as *mut u32, val);
}

/// Try reading a u32. Returns None if the address faults or returns a dead pattern.
/// Uses the caller-provided safe_read function for fault-tolerant probing.
fn probe_read32_with<P: Fn(usize) -> Option<u32>>(addr: usize, safe_read: &P) -> Option<u32> {
    let val = safe_read(addr)?;
    if val == 0xDEADDEAD || val == 0xFFFFFFFF {
        None
    } else {
        Some(val)
    }
}

#[inline]
fn cfg_valid(val: u32) -> bool {
    val != 0 && val != 0xFFFF_FFFF && val != 0xDEAD_DEAD
}

// ─── PCIe config space access ───────────────────────────────────

/// Data synchronisation barrier — ensures preceding stores are visible
/// to the hardware before any subsequent loads.
#[inline(always)]
fn dsb_sy() {
    #[cfg(target_arch = "aarch64")]
    unsafe { core::arch::asm!("dsb sy", options(nomem, nostack)); }
}

/// Read a 32-bit value from PCIe config space (bus/dev/fn/reg).
fn cfg_read32(pcie_base: usize, bus: u8, dev: u8, func: u8, reg: u16) -> u32 {
    // Linux brcmstb map semantics:
    //   idx = PCIE_ECAM_OFFSET(bus, devfn, 0)
    //   data ptr = EXT_CFG_DATA + PCIE_ECAM_REG(where)
    // For root bus/devfn=0, config space is directly at base + reg.
    if bus == 0 && dev == 0 && func == 0 {
        return unsafe { read32(pcie_base + (reg as usize & 0xFFC)) };
    }

    let devfn: u32 = ((dev as u32) << 3) | (func as u32);
    let idx: u32 = ((bus as u32) << 20) | (devfn << 12);
    unsafe {
        write32(pcie_base + EXT_CFG_INDEX, idx);
        // Flush posted write, matching common Broadcom access pattern.
        let _ = read32(pcie_base + EXT_CFG_INDEX);
        dsb_sy();
        for _ in 0..200u32 { core::hint::spin_loop(); }
        read32(pcie_base + EXT_CFG_DATA + (reg as usize & 0xFFC))
    }
}

/// Write a 32-bit value to PCIe config space.
fn cfg_write32(pcie_base: usize, bus: u8, dev: u8, func: u8, reg: u16, val: u32) {
    if bus == 0 && dev == 0 && func == 0 {
        unsafe {
            write32(pcie_base + (reg as usize & 0xFFC), val);
        }
        return;
    }

    let devfn: u32 = ((dev as u32) << 3) | (func as u32);
    let idx: u32 = ((bus as u32) << 20) | (devfn << 12);
    unsafe {
        write32(pcie_base + EXT_CFG_INDEX, idx);
        let _ = read32(pcie_base + EXT_CFG_INDEX);
        dsb_sy();
        for _ in 0..200u32 { core::hint::spin_loop(); }
        write32(pcie_base + EXT_CFG_DATA + (reg as usize & 0xFFC), val);
    }
}

// ─── Outbound window programming ────────────────────────────────

/// Program the PCIe outbound memory window so that CPU accesses at
/// `cpu_addr` are translated to PCIe address `pcie_addr` for `size` bytes.
///
/// This uses memory window 0 (the only one we need for RP1).
fn set_outbound_window(pcie_base: usize, cpu_addr: u64, pcie_addr: u64, size: u64) {
    // The BRCM controller uses a base/limit scheme:
    //   MEM_WIN0_BASELIM: [31:20] = CPU base[31:20], [15:0] = CPU limit[31:20]-1
    //   MEM_WIN0_BASE_HI: CPU base[63:32]
    //   MEM_WIN0_LIM_HI:  CPU limit[63:32]
    //   CPU_2_PCIE_LO:    PCIe offset low (PCIe addr - CPU addr)[31:0]
    //   CPU_2_PCIE_HI:    PCIe offset high (PCIe addr - CPU addr)[63:32]

    let cpu_end = cpu_addr + size - 1;

    // Base/Limit register: base[31:20] in bits [31:20], limit[31:20] in bits [15:0]
    let base_limit = ((cpu_addr as u32) & 0xFFF0_0000)
                   | (((cpu_end as u32) >> 20) & 0xFFFF);

    let base_hi = (cpu_addr >> 32) as u32;
    let limit_hi = (cpu_end >> 32) as u32;

    // CPU-to-PCIe offset (usually identity: offset = 0 when cpu_addr == pcie_addr)
    let offset = pcie_addr.wrapping_sub(cpu_addr);
    let offset_lo = offset as u32;
    let offset_hi = (offset >> 32) as u32;

    unsafe {
        write32(pcie_base + MISC_CPU_2_PCIE_LO, offset_lo);
        write32(pcie_base + MISC_CPU_2_PCIE_HI, offset_hi);
        write32(pcie_base + MISC_MEM_WIN0_BASELIM, base_limit);
        write32(pcie_base + MISC_MEM_WIN0_BASE_HI, base_hi);
        write32(pcie_base + MISC_MEM_WIN0_LIM_HI, limit_hi);
    }
}

/// Apply the minimal Broadcom RC setup used by Linux before config enumeration.
fn prep_controller_for_cfg(pcie_base: usize) {
    unsafe {
        // Enable SCB access path + robust config/read behavior.
        let mut tmp = read32(pcie_base + MISC_MISC_CTRL);
        tmp |= MISC_CTRL_SCB_ACCESS_EN
            | MISC_CTRL_CFG_READ_UR_MODE
            | MISC_CTRL_RCB_MPS_MODE
            | MISC_CTRL_RCB_64B_MODE;
        tmp = (tmp & !MISC_CTRL_MAX_BURST_SIZE_MASK) | MISC_CTRL_MAX_BURST_512B;
        write32(pcie_base + MISC_MISC_CTRL, tmp);

        // Set RC class code to PCI bridge (default can look like endpoint mode).
        let mut idv3 = read32(pcie_base + RC_CFG_PRIV1_ID_VAL3);
        idv3 = (idv3 & !0x00FF_FFFF) | 0x0006_0400;
        write32(pcie_base + RC_CFG_PRIV1_ID_VAL3, idv3);

        // Force little-endian for inbound BAR2 path.
        let mut vs1 = read32(pcie_base + RC_CFG_VENDOR_SPEC1);
        vs1 &= !0x0C;
        write32(pcie_base + RC_CFG_VENDOR_SPEC1, vs1);

        dsb_sy();
        for _ in 0..500u32 { core::hint::spin_loop(); }
    }
}

/// Nudge controller into RC link-training path (Linux-inspired sequence).
fn retrain_link<F: FnMut(core::fmt::Arguments<'_>)>(
    pcie_base: usize,
    mut log: F,
) {
    unsafe {
        // Assert bridge SW init (reset), short delay, then deassert.
        let mut sw = read32(pcie_base + RGR1_SW_INIT_1);
        sw |= SW_INIT_GENERIC_MASK;
        write32(pcie_base + RGR1_SW_INIT_1, sw);
        for _ in 0..2_000u32 { core::hint::spin_loop(); }

        sw &= !SW_INIT_GENERIC_MASK;
        write32(pcie_base + RGR1_SW_INIT_1, sw);

        // Bring SerDes out of IDDQ/powerdown.
        let mut hd = read32(pcie_base + HARD_DEBUG);
        hd &= !HARD_DEBUG_SERDES_IDDQ_MASK;
        write32(pcie_base + HARD_DEBUG, hd);

        // Toggle PERST# (bit meaning inverted for bcm2712 path in Linux).
        let mut ctrl = read32(pcie_base + MISC_PCIE_CTRL);
        ctrl &= !PCIE_CTRL_PERSTB_MASK; // assert PERST
        write32(pcie_base + MISC_PCIE_CTRL, ctrl);
        for _ in 0..6_000u32 { core::hint::spin_loop(); }

        ctrl |= PCIE_CTRL_PERSTB_MASK; // deassert PERST
        write32(pcie_base + MISC_PCIE_CTRL, ctrl);

        // Allow LTSSM to progress.
        for _ in 0..20_000u32 { core::hint::spin_loop(); }

        let st = read32(pcie_base + MISC_PCIE_STATUS);
        let phy = if st & STATUS_PHYLINKUP != 0 { "UP" } else { "down" };
        let dl = if st & STATUS_DL_ACTIVE != 0 { "ACTIVE" } else { "inactive" };
        log(format_args!("  retrain status={:#010x} PHY={} DL={}", st, phy, dl));
    }
}

/// Enable the RP1 device on PCIe: set bus-master and memory-space enable
/// in the PCI command register.
fn enable_device(pcie_base: usize, bus: u8, dev: u8, func: u8) {
    let cmd = cfg_read32(pcie_base, bus, dev, func, 0x04);
    // Bit 1 = Memory Space Enable, Bit 2 = Bus Master Enable
    let new_cmd = cmd | 0x06;
    cfg_write32(pcie_base, bus, dev, func, 0x04, new_cmd);
}

// ─── Main entry point ───────────────────────────────────────────

/// Scan all BCM2712 PCIe controllers, find RP1, and program the
/// outbound window so the CPU can access RP1 at `RP1_BAR_BASE`.
///
/// The `log` callback receives formatted diagnostic strings for HDMI output.
/// Returns `Some(Rp1Info)` if RP1 was found and configured.
pub fn init_rp1<F: FnMut(core::fmt::Arguments<'_>), P: Fn(usize) -> Option<u32>>(mut log: F, safe_read: P) -> Option<Rp1Info> {
    use super::mem::RP1_BAR_BASE;

    // ── Fast path: check if firmware already mapped RP1 ──────
    log(format_args!("probe firmware BAR @ {:#x}", RP1_BAR_BASE));
    match safe_read(RP1_BAR_BASE + 0x3_0000 + 0xFE0) {
        Some(pid) if pid != 0 && pid != 0xFFFFFFFF && pid != 0xDEADDEAD => {
            log(format_args!("RP1 already mapped by firmware (PID={:#x})", pid));
            return Some(Rp1Info {
                pcie_base: 0,
                bar_base: RP1_BAR_BASE,
                bar_size: 4 * 1024 * 1024,
                controller: "firmware",
            });
        }
        Some(v) => log(format_args!("  firmware probe: {:#x} (not alive)", v)),
        None    => log(format_args!("  firmware probe: FAULT")),
    }

    // Prefer PCIe2 first (RP1 is wired there on Pi 5 DTB), then try others.
    let order = [2usize, 0usize, 1usize];
    for &idx in &order {
        let (base, name) = PCIE_CONTROLLERS[idx];
        log(format_args!("--- {} @ {:#x} ---", name, base));

        // Force a fresh link-training attempt each pass.
        retrain_link(base, |args| log(args));

        // Align with Linux brcmstb pre-enumeration setup.
        prep_controller_for_cfg(base);

        // Check if controller is accessible
        match probe_read32_with(base, &safe_read) {
            Some(v) => { log(format_args!("  RC VID:DID  = {:#010x}", v)); }
            None    => { log(format_args!("  not accessible")); continue; }
        }

        // Check link status
        let status = unsafe { read32(base + MISC_PCIE_STATUS) };
        let phy = if status & STATUS_PHYLINKUP != 0 { "UP" } else { "down" };
        let dl  = if status & STATUS_DL_ACTIVE != 0 { "ACTIVE" } else { "inactive" };
        log(format_args!("  LINK_STATUS = {:#010x} PHY={} DL={}", status, phy, dl));

        let link_up = (status & STATUS_LINK_MASK) != 0;
        if !link_up {
            log(format_args!("  link DOWN — skipping"));
            continue;
        }

        // Bridge bus numbers
        let bus_reg = unsafe { read32(base + 0x18) };
        let secondary = ((bus_reg >> 8) & 0xFF) as u8;
        log(format_args!("  BUS reg={:#010x} sec={}", bus_reg, secondary));
        let sec_bus = if secondary == 0 {
            let new_bus = (bus_reg & 0xFF00_0000) | 0x0001_0100;
            unsafe { write32(base + 0x18, new_bus); }
            dsb_sy();
            for _ in 0..500u32 { core::hint::spin_loop(); }
            log(format_args!("  programmed bus 1"));
            1u8
        } else {
            secondary
        };

        // Scan likely busses/devices/functions. RP1 is usually at bus 1 dev 0 fn 0,
        // but firmware/bridge setup can vary while bring-up is unstable.
        // Firmware bus register can be garbage if RC is partially initialised.
        // Probe sane downstream bus numbers explicitly.
        let mut buses = [1u8, 2u8, 3u8, sec_bus];
        if sec_bus == 0 || sec_bus > 32 {
            buses[3] = 1;
        }

        let mut found: Option<(u8, u8, u8, u32)> = None;
        let mut seen: [(u8, u8, u8, u32); 8] = [(0, 0, 0, 0); 8];
        let mut seen_n: usize = 0;
        for &bus in &buses {
            for dev in 0u8..8 {
                for func in 0u8..2 {
                    let vid = cfg_read32(base, bus, dev, func, 0x00);
                    if !cfg_valid(vid) {
                        continue;
                    }
                    log(format_args!("  dev {}:{}:{} VID:DID={:#010x}", bus, dev, func, vid));
                    if seen_n < seen.len() {
                        seen[seen_n] = (bus, dev, func, vid);
                        seen_n += 1;
                    }
                    if vid == RP1_VENDOR_DEVICE {
                        found = Some((bus, dev, func, vid));
                        break;
                    }
                }
                if found.is_some() {
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }

        if seen_n == 0 {
            log(format_args!("[pcie-sum] {} eps=0 uniq=0 rp1=no", name));
        } else {
            // Compact digest: count unique VIDs among sampled endpoints.
            let mut uniq: [u32; 8] = [0; 8];
            let mut uniq_n = 0usize;
            for i in 0..seen_n {
                let v = seen[i].3;
                let mut exists = false;
                for j in 0..uniq_n {
                    if uniq[j] == v {
                        exists = true;
                        break;
                    }
                }
                if !exists && uniq_n < uniq.len() {
                    uniq[uniq_n] = v;
                    uniq_n += 1;
                }
            }
            let first = seen[0];
            let rp1_yes = if found.is_some() { "yes" } else { "no" };
            log(format_args!(
                "[pcie-sum] {} eps={} uniq={} first={:#010x}@{}:{}:{} rp1={}",
                name,
                seen_n,
                uniq_n,
                first.3,
                first.0,
                first.1,
                first.2,
                rp1_yes
            ));
        }

        let (rp1_bus, rp1_dev, rp1_func, _) = match found {
            Some(t) => t,
            None => {
                log(format_args!("  RP1 VID:DID not found on this controller"));
                continue;
            }
        };

        log(format_args!("  *** RP1 FOUND at {}:{}:{} ***", rp1_bus, rp1_dev, rp1_func));

        let bar0 = cfg_read32(base, rp1_bus, rp1_dev, rp1_func, 0x10);
        let bar1 = cfg_read32(base, rp1_bus, rp1_dev, rp1_func, 0x14);
        let cmd  = cfg_read32(base, rp1_bus, rp1_dev, rp1_func, 0x04);
        log(format_args!("  BAR0={:#010x} BAR1={:#010x} CMD={:#010x}", bar0, bar1, cmd));

        // BAR size discovery
        cfg_write32(base, rp1_bus, rp1_dev, rp1_func, 0x10, 0xFFFFFFFF);
        let bar0_size_mask = cfg_read32(base, rp1_bus, rp1_dev, rp1_func, 0x10);
        cfg_write32(base, rp1_bus, rp1_dev, rp1_func, 0x10, bar0);

        let bar_size = if bar0_size_mask != 0 && bar0_size_mask != 0xFFFFFFFF {
            let mask = bar0_size_mask & !0xF;
            (!mask).wrapping_add(1) as usize
        } else {
            4 * 1024 * 1024
        };

        let pcie_bar = ((bar1 as u64) << 32) | ((bar0 as u64) & !0xF);
        log(format_args!("  PCIe BAR = {:#x}, size = {} KiB", pcie_bar, bar_size / 1024));

        // Program outbound window
        let window_size = if bar_size >= 0x40_0000 { bar_size } else { 0x40_0000 };
        enable_device(base, rp1_bus, rp1_dev, rp1_func);

        // Try mapping using BAR value first (firmware-assigned), then DTB-style
        // PCIe address 0, then identity mapping as a last resort.
        let map_candidates = [pcie_bar, 0, RP1_BAR_BASE as u64];
        for &pcie_addr in &map_candidates {
            log(format_args!("  map CPU {:#x} -> PCIe {:#x}", RP1_BAR_BASE, pcie_addr));
            set_outbound_window(base, RP1_BAR_BASE as u64, pcie_addr, window_size as u64);
            for _ in 0..10_000u32 { core::hint::spin_loop(); }

            let pid = safe_read(RP1_BAR_BASE + 0x3_0000 + 0xFE0).unwrap_or(0xDEAD_DEAD);
            log(format_args!("  verify UART PID = {:#010x}", pid));
            if cfg_valid(pid) {
                return Some(Rp1Info {
                    pcie_base: base,
                    bar_base: RP1_BAR_BASE,
                    bar_size: window_size,
                    controller: name,
                });
            }
        }

        log(format_args!("  RP1 found but MMIO not accessible"));
    }

    None
}

/// Diagnostic: dump all PCIe controller state. Returns formatted lines
/// through the callback for display on framebuffer.
pub fn dump_diagnostics<F: FnMut(core::fmt::Arguments<'_>), P: Fn(usize) -> Option<u32>>(mut log: F, safe_read: P) {
    for &(base, name) in &PCIE_CONTROLLERS {
        log(format_args!("--- {} @ {:#x} ---", name, base));

        let rc_vid = probe_read32_with(base, &safe_read);
        match rc_vid {
            Some(v) => log(format_args!("  RC VID:DID  = {:#010x}", v)),
            None    => { log(format_args!("  not accessible")); continue; }
        }

        let status = unsafe { read32(base + MISC_PCIE_STATUS) };
        let phy = if status & STATUS_PHYLINKUP != 0 { "UP" } else { "down" };
        let dl  = if status & STATUS_DL_ACTIVE != 0 { "ACTIVE" } else { "inactive" };
        log(format_args!("  PCIE_STATUS = {:#010x} (PHY={}, DL={})", status, phy, dl));

        let link_up = (status & STATUS_LINK_MASK) != 0;
        if !link_up {
            log(format_args!("  link DOWN — skipping config reads"));
            continue;
        }

        // Bridge bus numbers
        let bus_reg = unsafe { read32(base + 0x18) };
        let primary = (bus_reg & 0xFF) as u8;
        let secondary = ((bus_reg >> 8) & 0xFF) as u8;
        let subordinate = ((bus_reg >> 16) & 0xFF) as u8;
        log(format_args!("  BUS_REG     = {:#010x} (pri={}, sec={}, sub={})",
            bus_reg, primary, secondary, subordinate));

        // Outbound window state
        let win_lo = unsafe { read32(base + MISC_CPU_2_PCIE_LO) };
        let win_hi = unsafe { read32(base + MISC_CPU_2_PCIE_HI) };
        log(format_args!("  CPU_2_PCIE  = {:#010x}:{:#010x}", win_hi, win_lo));

        let bl = unsafe { read32(base + MISC_MEM_WIN0_BASELIM) };
        let bh = unsafe { read32(base + MISC_MEM_WIN0_BASE_HI) };
        let lh = unsafe { read32(base + MISC_MEM_WIN0_LIM_HI) };
        log(format_args!("  WIN0 BASE/LIM={:#010x} HI={:#010x}/{:#010x}", bl, bh, lh));

        // Try reading device on the firmware-assigned secondary bus
        let sec = if secondary != 0 { secondary } else { 1 };
        let dev_vid = cfg_read32(base, sec, 0, 0, 0x00);
        log(format_args!("  Dev VID:DID = {:#010x} (bus={})", dev_vid, sec));
        if dev_vid == 0xFFFFFFFF || dev_vid == 0 {
            // Also try bus 1 if different
            if sec != 1 {
                let vid1 = cfg_read32(base, 1, 0, 0, 0x00);
                log(format_args!("  Dev VID:DID = {:#010x} (bus=1 fallback)", vid1));
            }
            continue;
        }

        let bar0 = cfg_read32(base, sec, 0, 0, 0x10);
        let bar1 = cfg_read32(base, sec, 0, 0, 0x14);
        log(format_args!("  Dev BAR0    = {:#010x}", bar0));
        log(format_args!("  Dev BAR1    = {:#010x}", bar1));

        let cmd = cfg_read32(base, sec, 0, 0, 0x04);
        log(format_args!("  Dev CMD/STS = {:#010x}", cmd));
    }
}

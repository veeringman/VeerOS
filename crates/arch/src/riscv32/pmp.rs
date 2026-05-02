//! RISC-V Physical Memory Protection (PMP) driver.
//!
//! PMP provides per-hart memory access control enforced in hardware.
//! On RV32, up to 16 entries are available via CSRs `pmpcfg0`–`pmpcfg3`
//! (4 packed 8-bit configs per register) and `pmpaddr0`–`pmpaddr15`
//! (physical addresses right-shifted by 2).
//!
//! Address-matching modes:
//! - **OFF** (A=0): entry disabled
//! - **TOR** (A=1): Top-of-Range — region is `[pmpaddr[i-1] .. pmpaddr[i])`
//! - **NA4** (A=2): Naturally Aligned 4-byte region
//! - **NAPOT** (A=3): Naturally Aligned Power-of-Two region
//!
//! Permission bits: R (read), W (write), X (execute), L (lock — applies
//! the rule to M-mode as well and cannot be cleared until reset).
//!
//! # Usage
//!
//! The kernel programs PMP entries on each context switch to enforce
//! per-task memory isolation.  Layout:
//!
//! | Entry | Purpose                                    |
//! |-------|--------------------------------------------|
//! |  0–1  | Task stack (TOR: `[stack_bottom..stack_top)`) |
//! |  2–3  | Task code/rodata region (TOR)              |
//! |  4–5  | Kernel code+data (TOR, locked)             |
//! | 6–15  | Available for additional per-task grants    |

use crate::{MemPerms, TaskMemRegion, TaskRegions};

#[cfg(target_arch = "riscv32")]
use crate::MAX_TASK_REGIONS;

// ─── PMP config bit definitions ─────────────────────────────────────────

pub const PMP_R: u8 = 1 << 0;
pub const PMP_W: u8 = 1 << 1;
pub const PMP_X: u8 = 1 << 2;
pub const PMP_A_OFF: u8 = 0 << 3;
pub const PMP_A_TOR: u8 = 1 << 3;
pub const PMP_A_NA4: u8 = 2 << 3;
pub const PMP_A_NAPOT: u8 = 3 << 3;
pub const PMP_L: u8 = 1 << 7;

/// Maximum PMP entries supported on standard rv32.
pub const MAX_PMP_ENTRIES: usize = 16;

// ─── CSR read/write helpers ─────────────────────────────────────────────

/// Write a `pmpaddr` CSR by index (0–15).
///
/// `addr` is the physical byte address — this function right-shifts by 2
/// before writing.
///
/// # Safety
/// Caller must be running in M-mode.
#[cfg(target_arch = "riscv32")]
pub unsafe fn write_pmpaddr(index: usize, addr: usize) {
    let val = addr >> 2;
    match index {
        0 => core::arch::asm!("csrw pmpaddr0,  {0}", in(reg) val, options(nomem, nostack)),
        1 => core::arch::asm!("csrw pmpaddr1,  {0}", in(reg) val, options(nomem, nostack)),
        2 => core::arch::asm!("csrw pmpaddr2,  {0}", in(reg) val, options(nomem, nostack)),
        3 => core::arch::asm!("csrw pmpaddr3,  {0}", in(reg) val, options(nomem, nostack)),
        4 => core::arch::asm!("csrw pmpaddr4,  {0}", in(reg) val, options(nomem, nostack)),
        5 => core::arch::asm!("csrw pmpaddr5,  {0}", in(reg) val, options(nomem, nostack)),
        6 => core::arch::asm!("csrw pmpaddr6,  {0}", in(reg) val, options(nomem, nostack)),
        7 => core::arch::asm!("csrw pmpaddr7,  {0}", in(reg) val, options(nomem, nostack)),
        8 => core::arch::asm!("csrw pmpaddr8,  {0}", in(reg) val, options(nomem, nostack)),
        9 => core::arch::asm!("csrw pmpaddr9,  {0}", in(reg) val, options(nomem, nostack)),
        10 => core::arch::asm!("csrw pmpaddr10, {0}", in(reg) val, options(nomem, nostack)),
        11 => core::arch::asm!("csrw pmpaddr11, {0}", in(reg) val, options(nomem, nostack)),
        12 => core::arch::asm!("csrw pmpaddr12, {0}", in(reg) val, options(nomem, nostack)),
        13 => core::arch::asm!("csrw pmpaddr13, {0}", in(reg) val, options(nomem, nostack)),
        14 => core::arch::asm!("csrw pmpaddr14, {0}", in(reg) val, options(nomem, nostack)),
        15 => core::arch::asm!("csrw pmpaddr15, {0}", in(reg) val, options(nomem, nostack)),
        _ => {}
    }
}

/// Write a `pmpcfg` register (0–3).  Each register packs 4 entries.
///
/// # Safety
/// Caller must be running in M-mode.
#[cfg(target_arch = "riscv32")]
pub unsafe fn write_pmpcfg(reg: usize, val: u32) {
    match reg {
        0 => core::arch::asm!("csrw pmpcfg0, {0}", in(reg) val, options(nomem, nostack)),
        1 => core::arch::asm!("csrw pmpcfg1, {0}", in(reg) val, options(nomem, nostack)),
        2 => core::arch::asm!("csrw pmpcfg2, {0}", in(reg) val, options(nomem, nostack)),
        3 => core::arch::asm!("csrw pmpcfg3, {0}", in(reg) val, options(nomem, nostack)),
        _ => {}
    }
}

/// Read a `pmpcfg` register (0–3).
///
/// # Safety
/// Caller must be running in M-mode.
#[cfg(target_arch = "riscv32")]
pub unsafe fn read_pmpcfg(reg: usize) -> u32 {
    let val: u32;
    match reg {
        0 => core::arch::asm!("csrr {0}, pmpcfg0", out(reg) val, options(nomem, nostack)),
        1 => core::arch::asm!("csrr {0}, pmpcfg1", out(reg) val, options(nomem, nostack)),
        2 => core::arch::asm!("csrr {0}, pmpcfg2", out(reg) val, options(nomem, nostack)),
        3 => core::arch::asm!("csrr {0}, pmpcfg3", out(reg) val, options(nomem, nostack)),
        _ => {
            val = 0;
        }
    }
    val
}

// ─── High-level PMP configuration ───────────────────────────────────────

/// Configuration for a single PMP TOR entry pair.
///
/// TOR mode uses two adjacent PMP entries: the lower-numbered entry holds
/// the region base address, and the upper-numbered entry holds the region
/// top address plus the permission config.
#[cfg(target_arch = "riscv32")]
struct TorEntry {
    base: usize,
    top: usize,
    cfg: u8,
}

/// Convert `MemPerms` to PMP permission bits.
#[allow(dead_code)]
fn perms_to_pmp(perms: MemPerms) -> u8 {
    let mut bits = 0u8;
    if perms.contains(MemPerms::READ) {
        bits |= PMP_R;
    }
    if perms.contains(MemPerms::WRITE) {
        bits |= PMP_W;
    }
    if perms.contains(MemPerms::EXECUTE) {
        bits |= PMP_X;
    }
    bits
}

/// Apply per-task PMP configuration.
///
/// Programs up to `MAX_TASK_REGIONS` TOR regions (2 PMP entries each)
/// from the task's region array, plus a catch-all deny entry.
///
/// PMP layout (entries are consumed in pairs for TOR mode):
///   - entries 0..(2*region_count): per-task regions
///   - remaining entries: disabled (A=OFF)
///
/// # Safety
/// Must be called in M-mode with interrupts disabled.
#[cfg(target_arch = "riscv32")]
pub unsafe fn apply_task_regions(regions: &TaskRegions, region_count: usize) {
    // Build TOR entry list from task regions.
    let mut entries: [TorEntry; MAX_TASK_REGIONS] = [
        TorEntry {
            base: 0,
            top: 0,
            cfg: PMP_A_OFF,
        },
        TorEntry {
            base: 0,
            top: 0,
            cfg: PMP_A_OFF,
        },
        TorEntry {
            base: 0,
            top: 0,
            cfg: PMP_A_OFF,
        },
        TorEntry {
            base: 0,
            top: 0,
            cfg: PMP_A_OFF,
        },
        TorEntry {
            base: 0,
            top: 0,
            cfg: PMP_A_OFF,
        },
        TorEntry {
            base: 0,
            top: 0,
            cfg: PMP_A_OFF,
        },
        TorEntry {
            base: 0,
            top: 0,
            cfg: PMP_A_OFF,
        },
        TorEntry {
            base: 0,
            top: 0,
            cfg: PMP_A_OFF,
        },
    ];
    let n = if region_count > MAX_TASK_REGIONS {
        MAX_TASK_REGIONS
    } else {
        region_count
    };

    for i in 0..n {
        let r = &regions[i];
        if r.size > 0 {
            entries[i] = TorEntry {
                base: r.base,
                top: r.base + r.size,
                cfg: PMP_A_TOR | perms_to_pmp(r.perms),
            };
        }
    }

    // Pack pmpcfg values. Each TOR pair uses two PMP entry indices:
    //   pair i → pmpaddr[2*i] = base, pmpaddr[2*i+1] = top
    //   pmpcfg for entry 2*i = OFF (base marker, no permissions)
    //   pmpcfg for entry 2*i+1 = TOR + perms
    let mut cfgs = [0u8; MAX_PMP_ENTRIES];
    for i in 0..MAX_TASK_REGIONS {
        let base_idx = i * 2;
        let top_idx = base_idx + 1;
        if i < n && entries[i].cfg != PMP_A_OFF {
            cfgs[base_idx] = PMP_A_OFF; // base address marker
            cfgs[top_idx] = entries[i].cfg;
        }
        // else: entries remain OFF (deny)
    }

    // Write pmpaddr CSRs.
    for i in 0..MAX_TASK_REGIONS {
        let base_idx = i * 2;
        let top_idx = base_idx + 1;
        if i < n && entries[i].cfg != PMP_A_OFF {
            write_pmpaddr(base_idx, entries[i].base);
            write_pmpaddr(top_idx, entries[i].top);
        } else {
            write_pmpaddr(base_idx, 0);
            write_pmpaddr(top_idx, 0);
        }
    }
    // Clear remaining pmpaddr entries (8..15).
    for idx in (MAX_TASK_REGIONS * 2)..MAX_PMP_ENTRIES {
        write_pmpaddr(idx, 0);
    }

    // Write pmpcfg registers. On RV32, pmpcfg0 packs entries 0–3,
    // pmpcfg1 packs entries 4–7, etc.
    for reg in 0..4u32 {
        let base = (reg as usize) * 4;
        let packed = (cfgs[base] as u32)
            | ((cfgs[base + 1] as u32) << 8)
            | ((cfgs[base + 2] as u32) << 16)
            | ((cfgs[base + 3] as u32) << 24);
        write_pmpcfg(reg as usize, packed);
    }
}

/// Clear all PMP entries (set config to OFF, addresses to 0).
///
/// # Safety
/// Must be called in M-mode.
#[cfg(target_arch = "riscv32")]
pub unsafe fn clear_all() {
    for reg in 0..4 {
        write_pmpcfg(reg, 0);
    }
    for idx in 0..MAX_PMP_ENTRIES {
        write_pmpaddr(idx, 0);
    }
}

/// Apply combined process + thread PMP regions.
///
/// Process regions (code, data, heap) are loaded first, then per-thread
/// regions (stack + guard).  The total region count must not exceed
/// `MAX_PMP_ENTRIES / 2` = 8 TOR pairs.
///
/// # Safety
/// Must be called in M-mode with interrupts disabled.
#[cfg(target_arch = "riscv32")]
pub unsafe fn apply_combined_regions(
    proc_regions: &[TaskMemRegion],
    proc_count: usize,
    thread_regions: &TaskRegions,
    thread_count: usize,
) {
    let max_tor = MAX_PMP_ENTRIES / 2; // 8 TOR pairs
    let total = proc_count + thread_count;
    let _n = if total > max_tor { max_tor } else { total };

    let mut cfgs = [0u8; MAX_PMP_ENTRIES];

    // Helper: program one TOR pair at PMP index `pair_idx`.
    let mut pair = 0usize;

    // Process regions first.
    let pc = if proc_count > max_tor {
        max_tor
    } else {
        proc_count
    };
    for i in 0..pc {
        let r = &proc_regions[i];
        if r.size > 0 && pair < max_tor {
            let base_idx = pair * 2;
            let top_idx = base_idx + 1;
            write_pmpaddr(base_idx, r.base);
            write_pmpaddr(top_idx, r.base + r.size);
            cfgs[base_idx] = PMP_A_OFF;
            cfgs[top_idx] = PMP_A_TOR | perms_to_pmp(r.perms);
            pair += 1;
        }
    }

    // Thread regions (stack + guard).
    let tc = if thread_count > (max_tor - pair) {
        max_tor - pair
    } else {
        thread_count
    };
    for i in 0..tc {
        let r = &thread_regions[i];
        if r.size > 0 && pair < max_tor {
            let base_idx = pair * 2;
            let top_idx = base_idx + 1;
            write_pmpaddr(base_idx, r.base);
            write_pmpaddr(top_idx, r.base + r.size);
            cfgs[base_idx] = PMP_A_OFF;
            cfgs[top_idx] = PMP_A_TOR | perms_to_pmp(r.perms);
            pair += 1;
        }
    }

    // Clear remaining PMP entries.
    for idx in (pair * 2)..MAX_PMP_ENTRIES {
        write_pmpaddr(idx, 0);
    }

    // Write pmpcfg registers.
    for reg in 0..4u32 {
        let base = (reg as usize) * 4;
        let packed = (cfgs[base] as u32)
            | ((cfgs[base + 1] as u32) << 8)
            | ((cfgs[base + 2] as u32) << 16)
            | ((cfgs[base + 3] as u32) << 24);
        write_pmpcfg(reg as usize, packed);
    }
}

// ─── Non-riscv32 stubs (host build) ────────────────────────────────────

#[cfg(not(target_arch = "riscv32"))]
pub unsafe fn write_pmpaddr(_index: usize, _addr: usize) {}

#[cfg(not(target_arch = "riscv32"))]
pub unsafe fn write_pmpcfg(_reg: usize, _val: u32) {}

#[cfg(not(target_arch = "riscv32"))]
pub unsafe fn read_pmpcfg(_reg: usize) -> u32 {
    0
}

#[cfg(not(target_arch = "riscv32"))]
pub unsafe fn apply_task_regions(_regions: &TaskRegions, _region_count: usize) {}

#[cfg(not(target_arch = "riscv32"))]
pub unsafe fn apply_combined_regions(
    _proc_regions: &[TaskMemRegion],
    _proc_count: usize,
    _thread_regions: &TaskRegions,
    _thread_count: usize,
) {
}

#[cfg(not(target_arch = "riscv32"))]
pub unsafe fn clear_all() {}

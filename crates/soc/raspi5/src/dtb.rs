//! Minimal Flattened Device Tree (FDT) parser for boot diagnostics.
//!
//! Parses the DTB blob passed by the VideoCore firmware in x0 at boot.
//! Extracts PCIe controller addresses, RP1 BAR mapping, and other
//! hardware configuration that the firmware set up before ARM cores boot.
//!
//! FDT binary format (v17):
//!   Header (40 bytes), memory reservation block, structure block, strings block.
//!   Structure block tokens: FDT_BEGIN_NODE(1), FDT_END_NODE(2), FDT_PROP(3), FDT_NOP(4), FDT_END(9).

use core::fmt;

// ─── FDT constants ──────────────────────────────────────────────

const FDT_MAGIC: u32 = 0xD00D_FEED;
const FDT_BEGIN_NODE: u32 = 0x0000_0001;
const FDT_END_NODE: u32 = 0x0000_0002;
const FDT_PROP: u32 = 0x0000_0003;
const FDT_NOP: u32 = 0x0000_0004;
const FDT_END: u32 = 0x0000_0009;

// ─── Big-endian helpers ─────────────────────────────────────────

#[inline]
fn be32(ptr: *const u8) -> u32 {
    unsafe {
        ((*ptr.add(0) as u32) << 24)
            | ((*ptr.add(1) as u32) << 16)
            | ((*ptr.add(2) as u32) << 8)
            | (*ptr.add(3) as u32)
    }
}

#[inline]
fn be64(ptr: *const u8) -> u64 {
    ((be32(ptr) as u64) << 32) | (be32(unsafe { ptr.add(4) }) as u64)
}

// ─── String comparison on raw pointers ──────────────────────────

/// Compare a null-terminated C string at `ptr` to a Rust `&str`.
unsafe fn streq(ptr: *const u8, s: &str) -> bool {
    let sb = s.as_bytes();
    for i in 0..sb.len() {
        if *ptr.add(i) != sb[i] {
            return false;
        }
    }
    *ptr.add(sb.len()) == 0
}

/// Check if the C string at `ptr` starts with `prefix`.
unsafe fn starts_with(ptr: *const u8, prefix: &str) -> bool {
    let pb = prefix.as_bytes();
    for i in 0..pb.len() {
        let c = *ptr.add(i);
        if c == 0 || c != pb[i] {
            return false;
        }
    }
    true
}

/// Get the length of a null-terminated C string (max 256).
unsafe fn cstrlen(ptr: *const u8) -> usize {
    let mut len = 0;
    while len < 256 && *ptr.add(len) != 0 {
        len += 1;
    }
    len
}

// ─── FDT header ─────────────────────────────────────────────────

struct FdtHeader {
    totalsize: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    version: u32,
}

fn parse_header(base: *const u8) -> Option<FdtHeader> {
    let magic = be32(base);
    if magic != FDT_MAGIC {
        return None;
    }
    Some(FdtHeader {
        totalsize: be32(unsafe { base.add(4) }),
        off_dt_struct: be32(unsafe { base.add(8) }),
        off_dt_strings: be32(unsafe { base.add(12) }),
        version: be32(unsafe { base.add(20) }),
    })
}

// ─── Public API ─────────────────────────────────────────────────

/// Dump key DTB nodes relevant to PCIe/RP1 bringup.
///
/// Walks the structure block and prints:
/// - DTB header info (size, version)
/// - All `pcie@*` and `rp1` nodes with their `reg` and `ranges` properties
/// - The `memory@*` node for DRAM layout
/// - Any `compatible` string containing "brcm" or "rp1"
///
/// `log` callback receives formatted lines for display.
pub fn dump_pcie_dtb_info<F: FnMut(fmt::Arguments<'_>)>(dtb_ptr: usize, mut log: F) {
    if dtb_ptr == 0 {
        log(format_args!("DTB pointer is NULL"));
        return;
    }

    let base = dtb_ptr as *const u8;
    let hdr = match parse_header(base) {
        Some(h) => h,
        None => {
            let magic = be32(base);
            log(format_args!("Bad DTB magic: {:#010x} (expected {:#010x})", magic, FDT_MAGIC));
            return;
        }
    };

    log(format_args!(
        "DTB @ {:#x}: {} bytes, v{}, struct@{:#x}, strings@{:#x}",
        dtb_ptr, hdr.totalsize, hdr.version, hdr.off_dt_struct, hdr.off_dt_strings
    ));

    let struct_base = unsafe { base.add(hdr.off_dt_struct as usize) };
    let strings_base = unsafe { base.add(hdr.off_dt_strings as usize) };

    // Walk the structure block
    let mut offset: usize = 0;
    let struct_end = hdr.totalsize as usize - hdr.off_dt_struct as usize;
    let mut depth: usize = 0;

    // Track current node name (for filtering properties)
    // We use a simple flag to know if we're inside a "pcie@" or "rp1" node
    let mut inside_pcie: bool = false;
    let mut inside_interesting: bool = false;
    let mut pcie_depth: usize = 0;

    while offset + 4 <= struct_end {
        let token = be32(unsafe { struct_base.add(offset) });
        offset += 4;

        match token {
            FDT_BEGIN_NODE => {
                let name_ptr = unsafe { struct_base.add(offset) };
                let name_len = unsafe { cstrlen(name_ptr) };

`   `   `                                   `````````````                // Check if this is a PCIe or RP1 node (skip memory@ — too verbose)
                let is_pcie = unsafe { starts_with(name_ptr, "pcie@") };
                let is_rp1 = unsafe { starts_with(name_ptr, "rp1") };

                if is_pcie || is_rp1 {
                    inside_interesting = true;
                    pcie_depth = depth;
                    inside_pcie = true;

                    // Print node name
                    // Safe: name_len is bounded to 256 by cstrlen
                    let name_slice = unsafe {
                        core::str::from_utf8_unchecked(core::slice::from_raw_parts(name_ptr, name_len))
                    };
                    log(format_args!("  NODE: {}", name_slice));
                }

                // Skip past the null-terminated name (4-byte aligned)
                offset += (name_len + 4) & !3;
                depth += 1;
            }
            FDT_END_NODE => {
                if depth > 0 {
                    depth -= 1;
                }
                if inside_interesting && depth <= pcie_depth {
                    inside_interesting = false;
                    inside_pcie = false;
                }
            }
            FDT_PROP => {
                if offset + 8 > struct_end {
                    break;
                }
                let val_len = be32(unsafe { struct_base.add(offset) }) as usize;
                let name_off = be32(unsafe { struct_base.add(offset + 4) }) as usize;
                offset += 8;

                let prop_name = unsafe { strings_base.add(name_off) };

                if inside_interesting {
                    let val_ptr = unsafe { struct_base.add(offset) };

                    let pname_len = unsafe { cstrlen(prop_name) };
                    let pname = unsafe {
                        core::str::from_utf8_unchecked(core::slice::from_raw_parts(prop_name, pname_len))
                    };

                    // Print relevant properties
                    let is_reg = unsafe { streq(prop_name, "reg") };
                    let is_ranges = unsafe { streq(prop_name, "ranges") };
                    let is_compatible = unsafe { streq(prop_name, "compatible") };
                    let is_status = unsafe { streq(prop_name, "status") };
                    let is_msi_parent = unsafe { streq(prop_name, "msi-parent") };
                    let is_bus_range = unsafe { streq(prop_name, "bus-range") };

                    if is_compatible && val_len > 0 && val_len <= 256 {
                        let s = unsafe {
                            core::str::from_utf8_unchecked(
                                core::slice::from_raw_parts(val_ptr, val_len.min(128))
                            )
                        };
                        // Replace nulls between strings with commas for display
                        log(format_args!("    compatible = {:?}", s.trim_end_matches('\0')));
                    } else if is_status && val_len > 0 && val_len <= 32 {
                        let s = unsafe {
                            core::str::from_utf8_unchecked(
                                core::slice::from_raw_parts(val_ptr, val_len.min(32))
                            )
                        };
                        log(format_args!("    status = {:?}", s.trim_end_matches('\0')));
                    } else if is_reg {
                        // Print reg as hex u64 pairs (address, size)
                        let mut i = 0;
                        let mut pair = 0;
                        while i + 8 <= val_len && pair < 4 {
                            let addr = be64(unsafe { val_ptr.add(i) });
                            if i + 16 <= val_len {
                                let size = be64(unsafe { val_ptr.add(i + 8) });
                                log(format_args!("    reg[{}] = {:#018x} size {:#x}", pair, addr, size));
                                i += 16;
                            } else {
                                log(format_args!("    reg[{}] = {:#018x}", pair, addr));
                                i += 8;
                            }
                            pair += 1;
                        }
                        if val_len > 0 && val_len < 16 {
                            // Might be u32-sized cells
                            let mut i = 0;
                            let mut pair = 0;
                            while i + 4 <= val_len && pair < 4 {
                                let v = be32(unsafe { val_ptr.add(i) });
                                log(format_args!("    reg_u32[{}] = {:#010x}", pair, v));
                                i += 4;
                                pair += 1;
                            }
                        }
                    } else if is_ranges {
                        // Print first few range entries (child_bus, parent_cpu, size)
                        // Ranges are typically 7×u32 = 28 bytes per entry for PCIe
                        // (3 cells child, 2 cells parent, 2 cells size)
                        let n_entries = val_len / 28;
                        for e in 0..n_entries.min(3) {
                            let eoff = e * 28;
                            let flags = be32(unsafe { val_ptr.add(eoff) });
                            let child_hi = be32(unsafe { val_ptr.add(eoff + 4) });
                            let child_lo = be32(unsafe { val_ptr.add(eoff + 8) });
                            let parent = be64(unsafe { val_ptr.add(eoff + 12) });
                            let size = be64(unsafe { val_ptr.add(eoff + 20) });
                            log(format_args!(
                                "    ranges[{}]: flags={:#x} child={:#x}:{:#x} parent={:#018x} size={:#x}",
                                e, flags, child_hi, child_lo, parent, size
                            ));
                        }
                        if val_len % 28 != 0 {
                            log(format_args!("    ranges: {} bytes (non-standard stride)", val_len));
                        }
                    } else if is_bus_range && val_len >= 8 {
                        let lo = be32(val_ptr);
                        let hi = be32(unsafe { val_ptr.add(4) });
                        log(format_args!("    bus-range = {}..{}", lo, hi));
                    } else if val_len <= 4 && val_len > 0 {
                        let v = be32(val_ptr);
                        log(format_args!("    {} = {:#x}", pname, v));
                    }
                }

                // Skip past value (4-byte aligned)
                offset += (val_len + 3) & !3;
            }
            FDT_NOP => {}
            FDT_END => break,
            _ => {
                log(format_args!("  unknown token {:#x} at offset {:#x}", token, offset - 4));
                break;
            }
        }
    }
}

//! Multiboot v1 info structure (minimal).
//!
//! VeerOS boot asm reads `EBX` into `ESI` and passes it through to the Rust
//! entrypoint but the current qemu_pc kernel does not parse the info struct
//! at boot, so we only need to provide a valid-looking header so future
//! kernel code doesn't trip.

use anyhow::Result;

use crate::memory::GuestMem;

/// Multiboot magic value placed in EAX on entry.
pub const MULTIBOOT1_BOOTLOADER_MAGIC: u32 = 0x2BADB002;

/// Minimal Multiboot v1 info struct — 32 bytes, all flags-advertised fields
/// zeroed. Flags bit meanings (set = field is valid):
///   bit 0: mem_lower / mem_upper
///   bit 6: mmap_length / mmap_addr
///   etc.
///
/// Setting `flags = 0` tells the kernel "no optional info provided".
#[repr(C)]
#[derive(Default)]
pub struct MultibootInfo {
    pub flags: u32,
    pub mem_lower: u32,
    pub mem_upper: u32,
    pub boot_device: u32,
    pub cmdline: u32,
    pub mods_count: u32,
    pub mods_addr: u32,
    pub syms: [u32; 4],
    pub mmap_length: u32,
    pub mmap_addr: u32,
}

/// Write a minimal Multiboot info struct at the given guest physical
/// address and return the same address for convenience (kernel expects
/// `EBX == this address`).
pub fn write_info(guest: &GuestMem, at: u64, total_mem_bytes: u64) -> Result<u64> {
    let info = MultibootInfo {
        // Bit 0 = mem_lower/mem_upper valid.
        flags: 0x0000_0001,
        // Multiboot convention: mem_lower = KiB below 1 MiB (max 640).
        mem_lower: 640,
        // mem_upper = KiB of "upper" memory (starting at 1 MiB).
        mem_upper: ((total_mem_bytes.saturating_sub(1024 * 1024)) / 1024) as u32,
        ..Default::default()
    };

    // SAFETY: `MultibootInfo` is `#[repr(C)]` with no padding of interest;
    // its bytes are well-defined.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            &info as *const _ as *const u8,
            std::mem::size_of::<MultibootInfo>(),
        )
    };
    guest.write(at, bytes)?;
    Ok(at)
}

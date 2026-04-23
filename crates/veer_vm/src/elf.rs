//! ELF kernel loader (Multiboot v1 compatible).
//!
//! Supports 64-bit ELF with program headers. For each `PT_LOAD` segment we
//! copy `p_filesz` bytes from the file to guest physical address `p_paddr`,
//! then zero the tail `p_memsz - p_filesz` (BSS). We use `p_paddr` because
//! Multiboot kernels are typically linked with `paddr == vaddr` in the
//! identity-mapped boot region.

use anyhow::{bail, Context, Result};
use object::read::elf::{ElfFile64, ProgramHeader};
use object::{Object, ObjectKind};
use std::fs;
use std::path::Path;

use crate::memory::GuestMem;

/// Result of loading a kernel image.
pub struct LoadedKernel {
    /// Entry point reported by the ELF header (guest physical address).
    pub entry: u64,
    /// Highest address written to, rounded up to page boundary. Callers
    /// place post-kernel data (multiboot info, modules) after this.
    pub end: u64,
}

pub fn load(path: &Path, guest: &GuestMem) -> Result<LoadedKernel> {
    let bytes = fs::read(path)
        .with_context(|| format!("reading kernel ELF {}", path.display()))?;
    load_bytes(&bytes, &path.display().to_string(), guest)
}

pub fn load_bytes(bytes: &[u8], image_name: &str, guest: &GuestMem) -> Result<LoadedKernel> {
    let elf: ElfFile64 = ElfFile64::parse(&*bytes)
        .with_context(|| format!("parsing ELF {image_name}"))?;

    match elf.kind() {
        ObjectKind::Executable => {}
        other => bail!("expected executable ELF, got {other:?}"),
    }

    let entry = elf.entry();
    if entry == 0 {
        bail!("ELF has no entry point");
    }

    let endian = elf.endian();
    let mut end = 0u64;
    let mut loaded_segments = 0usize;

    for seg in elf.elf_program_headers() {
        // PT_LOAD = 1
        if seg.p_type(endian) != object::elf::PT_LOAD {
            continue;
        }
        let paddr = seg.p_paddr(endian);
        let filesz = seg.p_filesz(endian) as usize;
        let memsz = seg.p_memsz(endian) as usize;
        let offset = seg.p_offset(endian) as usize;

        if memsz == 0 { continue; }
        if filesz > memsz {
            bail!("malformed PT_LOAD: filesz {filesz} > memsz {memsz}");
        }
        let file_end = offset.checked_add(filesz)
            .context("PT_LOAD offset+filesz overflow")?;
        if file_end > bytes.len() {
            bail!("PT_LOAD extends past end of ELF file");
        }

        // Copy file bytes.
        if filesz > 0 {
            guest.write(paddr, &bytes[offset..file_end])
                .with_context(|| format!("writing segment paddr={paddr:#x} filesz={filesz:#x}"))?;
        }
        // Zero the BSS tail.
        let bss = memsz - filesz;
        if bss > 0 {
            let tail = guest.slice_mut(paddr + filesz as u64, bss)?;
            tail.fill(0);
        }

        let seg_end = paddr + memsz as u64;
        if seg_end > end { end = seg_end; }
        loaded_segments += 1;
    }

    if loaded_segments == 0 {
        bail!("no PT_LOAD segments found in ELF");
    }

    // Align end to 4 KiB.
    end = (end + 0xFFF) & !0xFFF;

    Ok(LoadedKernel { entry, end })
}

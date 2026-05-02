//! Minimal ISO9660 reader for VeerOS boot images.
//!
//! `build-qemu-pc.sh` creates `build/veeros.iso` with the Multiboot kernel at
//! `/boot/kernel.elf`. For `veer-vm` we do not emulate an optical drive; we
//! extract that kernel on the host side and then reuse the existing ELF boot
//! path.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

const SECTOR_SIZE: usize = 2048;
const PVD_SECTOR: usize = 16;
const VOLUME_TYPE_PRIMARY: u8 = 1;
const VOLUME_TYPE_TERMINATOR: u8 = 255;
const ROOT_RECORD_OFFSET: usize = 156;
const FLAG_DIRECTORY: u8 = 0x02;

pub fn extract_boot_kernel(path: &Path) -> Result<Vec<u8>> {
    let image = fs::read(path).with_context(|| format!("reading ISO {}", path.display()))?;
    let pvd = primary_volume_descriptor(&image)
        .with_context(|| format!("parsing ISO {}", path.display()))?;
    let root = parse_record(&pvd[ROOT_RECORD_OFFSET..]).context("parsing ISO root directory")?;
    let boot = find_child(&image, root.extent, root.size, "boot")?
        .ok_or_else(|| anyhow::anyhow!("ISO missing /boot directory"))?;
    anyhow::ensure!(boot.is_dir(), "ISO /boot entry is not a directory");
    let kernel = find_child(&image, boot.extent, boot.size, "kernel.elf")?
        .ok_or_else(|| anyhow::anyhow!("ISO missing /boot/kernel.elf"))?;
    anyhow::ensure!(!kernel.is_dir(), "ISO /boot/kernel.elf is a directory");

    let start = lba_offset(kernel.extent)?;
    let end = start
        .checked_add(kernel.size as usize)
        .context("kernel extent overflow")?;
    if end > image.len() {
        bail!("ISO /boot/kernel.elf extends past end of image");
    }
    Ok(image[start..end].to_vec())
}

fn primary_volume_descriptor(image: &[u8]) -> Result<&[u8]> {
    let mut sector = PVD_SECTOR;
    loop {
        let start = sector
            .checked_mul(SECTOR_SIZE)
            .context("PVD sector overflow")?;
        let end = start
            .checked_add(SECTOR_SIZE)
            .context("PVD range overflow")?;
        if end > image.len() {
            bail!("ISO ended before primary volume descriptor");
        }
        let desc = &image[start..end];
        anyhow::ensure!(
            &desc[1..6] == b"CD001",
            "invalid ISO9660 volume descriptor signature"
        );
        match desc[0] {
            VOLUME_TYPE_PRIMARY => return Ok(desc),
            VOLUME_TYPE_TERMINATOR => bail!("ISO has no primary volume descriptor"),
            _ => sector += 1,
        }
    }
}

fn find_child(
    image: &[u8],
    dir_extent: u32,
    dir_size: u32,
    target: &str,
) -> Result<Option<Record>> {
    let start = lba_offset(dir_extent)?;
    let end = start
        .checked_add(dir_size as usize)
        .context("directory extent overflow")?;
    if end > image.len() {
        bail!("directory record extends past end of ISO");
    }

    let target = target.to_ascii_uppercase();
    let dir = &image[start..end];
    let mut offset = 0usize;
    while offset < dir.len() {
        let len = dir[offset] as usize;
        if len == 0 {
            let next_sector = ((offset / SECTOR_SIZE) + 1) * SECTOR_SIZE;
            offset = next_sector;
            continue;
        }
        let rec_end = offset
            .checked_add(len)
            .context("directory record overflow")?;
        if rec_end > dir.len() {
            bail!("truncated directory record in ISO");
        }
        let rec = parse_record(&dir[offset..rec_end])?;
        if rec.name == target {
            return Ok(Some(rec));
        }
        offset = rec_end;
    }
    Ok(None)
}

fn lba_offset(lba: u32) -> Result<usize> {
    (lba as usize)
        .checked_mul(SECTOR_SIZE)
        .context("LBA offset overflow")
}

#[derive(Clone, Debug)]
struct Record {
    extent: u32,
    size: u32,
    flags: u8,
    name: String,
}

impl Record {
    fn is_dir(&self) -> bool {
        (self.flags & FLAG_DIRECTORY) != 0
    }
}

fn parse_record(bytes: &[u8]) -> Result<Record> {
    anyhow::ensure!(!bytes.is_empty(), "empty ISO directory record");
    let len = bytes[0] as usize;
    anyhow::ensure!(len >= 34, "short ISO directory record");
    anyhow::ensure!(bytes.len() >= len, "truncated ISO directory record");

    let extent = u32::from_le_bytes(bytes[2..6].try_into().unwrap());
    let size = u32::from_le_bytes(bytes[10..14].try_into().unwrap());
    let flags = bytes[25];
    let name_len = bytes[32] as usize;
    let name_end = 33usize
        .checked_add(name_len)
        .context("ISO filename overflow")?;
    anyhow::ensure!(name_end <= len, "truncated ISO filename");
    let name = normalize_name(&bytes[33..name_end]);

    Ok(Record {
        extent,
        size,
        flags,
        name,
    })
}

fn normalize_name(raw: &[u8]) -> String {
    match raw {
        [0] => ".".into(),
        [1] => "..".into(),
        _ => {
            let text = String::from_utf8_lossy(raw);
            let base = text.split(';').next().unwrap_or(&text);
            base.to_ascii_uppercase()
        }
    }
}

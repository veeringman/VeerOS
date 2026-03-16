//! FAT32 filesystem driver for VeerOS.
//!
//! Provides read/write access to FAT32-formatted block devices (SD cards).
//! Device-agnostic: uses function pointers for block I/O so the microkernel
//! has no hardware dependencies.
//!
//! # Limitations (current implementation)
//! - Single FAT32 partition supported (no MBR parsing — assumes partition
//!   starts at sector 0, or caller provides the partition offset).
//! - 8.3 filenames only (no LFN support).
//! - Two 512-byte scratch buffers (~1 KB RAM overhead).

use crate::vfs::{InodeTable, InodeKind, MAX_INODES, NO_INODE};

// ─── Constants ───────────────────────────────────────────────────────

const SECTOR_SIZE: usize = 512;

/// End-of-chain markers (any value >= this is EOC).
const FAT_EOC: u32 = 0x0FFF_FFF8;
/// Free cluster marker.
const FAT_FREE: u32 = 0x0000_0000;
/// 28-bit mask for FAT32 entries.
const FAT_MASK: u32 = 0x0FFF_FFFF;

// Directory entry attribute bits.
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_LFN: u8 = 0x0F;

// BPB offsets.
const BPB_BPS: usize = 11;
const BPB_SPC: usize = 13;
const BPB_RSVD: usize = 14;
const BPB_NFAT: usize = 16;
const BPB_TOTSEC32: usize = 32;
const BPB_FATSZ32: usize = 36;
const BPB_ROOTCLUS: usize = 44;

// Directory entry field offsets (each entry = 32 bytes).
const DE_NAME: usize = 0;
const DE_ATTR: usize = 11;
const DE_CLUSHI: usize = 20;
const DE_CLUSLO: usize = 26;
const DE_SIZE: usize = 28;

// ─── Block I/O function pointer types ────────────────────────────────

pub type BlockReadFn = fn(u64, &mut [u8]) -> bool;
pub type BlockWriteFn = fn(u64, &[u8]) -> bool;

// ─── FAT32 driver ────────────────────────────────────────────────────

/// FAT32 filesystem state.
pub struct Fat32 {
    // ── BPB cache ──
    bps: u16,
    spc: u8,
    rsvd: u16,
    nfat: u8,
    fatsz: u32,
    root_clust: u32,

    // ── Derived ──
    fat_start: u32,
    data_start: u32,

    // ── Block I/O ──
    read_blk: Option<BlockReadFn>,
    write_blk: Option<BlockWriteFn>,

    /// Partition LBA offset (0 if raw device, or MBR partition start).
    part_offset: u32,

    // ── Scratch buffers ──
    buf: [u8; SECTOR_SIZE],
    fat_buf: [u8; SECTOR_SIZE],
    fat_sec: u32,
    fat_dirty: bool,

    // ── State ──
    pub mounted: bool,
    pub mount_inode: u16,
}

impl Fat32 {
    pub const fn new() -> Self {
        Self {
            bps: 0,
            spc: 0,
            rsvd: 0,
            nfat: 0,
            fatsz: 0,
            root_clust: 0,
            fat_start: 0,
            data_start: 0,
            read_blk: None,
            write_blk: None,
            part_offset: 0,
            buf: [0u8; SECTOR_SIZE],
            fat_buf: [0u8; SECTOR_SIZE],
            fat_sec: 0xFFFF_FFFF,
            fat_dirty: false,
            mounted: false,
            mount_inode: NO_INODE,
        }
    }

    // ─────────────────────────────────────────────────────────────────
    // Mount / unmount
    // ─────────────────────────────────────────────────────────────────

    /// Mount a FAT32 partition.
    ///
    /// `mount_inode` is the VFS directory inode where the filesystem is
    /// mounted.  `mount_id` is the slot index (1–4) written to child
    /// inodes so the dispatcher can route I/O correctly.
    pub fn mount(
        &mut self,
        read_fn: BlockReadFn,
        write_fn: BlockWriteFn,
        part_offset: u32,
        mount_inode: u16,
        inodes: &mut InodeTable,
        mount_id: u8,
    ) -> bool {
        self.read_blk = Some(read_fn);
        self.write_blk = Some(write_fn);
        self.part_offset = part_offset;

        // Read BPB (sector 0 of the partition).
        if !self.read_sector(0) {
            return false;
        }

        self.bps = le16(&self.buf, BPB_BPS);
        self.spc = self.buf[BPB_SPC];
        self.rsvd = le16(&self.buf, BPB_RSVD);
        self.nfat = self.buf[BPB_NFAT];
        self.fatsz = le32(&self.buf, BPB_FATSZ32);
        self.root_clust = le32(&self.buf, BPB_ROOTCLUS);

        // Basic sanity checks.
        if self.bps == 0 || self.spc == 0 || self.nfat == 0 || self.fatsz == 0 {
            return false;
        }

        // Derived fields.
        self.fat_start = self.rsvd as u32;
        self.data_start = self.fat_start + (self.nfat as u32) * self.fatsz;

        // Populate the root directory into VFS.
        self.mounted = true;
        self.mount_inode = mount_inode;
        self.fat_sec = 0xFFFF_FFFF; // invalidate FAT cache

        self.populate_dir(inodes, mount_inode, self.root_clust, mount_id);
        true
    }

    /// Unmount — flush FAT cache and mark inactive.
    pub fn unmount(&mut self) {
        self.flush_fat();
        self.mounted = false;
        self.mount_inode = NO_INODE;
    }

    // ─────────────────────────────────────────────────────────────────
    // Directory population — read FAT32 dir → create VFS inodes
    // ─────────────────────────────────────────────────────────────────

    /// Read a FAT32 directory (starting at `cluster`) and create VFS
    /// inodes as children of `dir_inode`.
    pub fn populate_dir(
        &mut self,
        inodes: &mut InodeTable,
        dir_inode: u16,
        cluster: u32,
        mount_id: u8,
    ) -> bool {
        let mut clust = cluster;
        'chain: loop {
            let base_sec = self.cluster_to_sector(clust);
            for sec_off in 0..self.spc as u32 {
                if !self.read_sector(base_sec + sec_off) {
                    return false;
                }
                let entries = SECTOR_SIZE / 32;
                for e in 0..entries {
                    let off = e * 32;
                    let first = self.buf[off + DE_NAME];
                    if first == 0x00 {
                        break 'chain; // end of directory
                    }
                    if first == 0xE5 {
                        continue; // deleted
                    }
                    let attr = self.buf[off + DE_ATTR];
                    if attr == ATTR_LFN {
                        continue; // LFN entry
                    }
                    if attr & ATTR_VOLUME_ID != 0 {
                        continue; // volume label
                    }

                    // Skip . and .. entries.
                    if self.buf[off] == b'.' {
                        continue;
                    }

                    // Parse 8.3 name.
                    let mut namebuf = [0u8; 13];
                    let namelen = parse_83(&self.buf[off..off + 11], &mut namebuf);

                    let is_dir = attr & ATTR_DIRECTORY != 0;
                    let first_cluster = (le16(&self.buf, off + DE_CLUSHI) as u32) << 16
                        | le16(&self.buf, off + DE_CLUSLO) as u32;
                    let size = le32(&self.buf, off + DE_SIZE);

                    // Skip if a child with this name already exists.
                    let name = core::str::from_utf8(&namebuf[..namelen]).unwrap_or("");
                    if inodes.find_child(dir_inode, name).is_some() {
                        continue;
                    }

                    // Allocate VFS inode.
                    let id = match inodes.alloc() {
                        Some(id) => id,
                        None => return false, // inode table full
                    };
                    let inode = &mut inodes.inodes[id as usize];
                    inode.kind = if is_dir {
                        InodeKind::Directory
                    } else {
                        InodeKind::File
                    };
                    inode.size = size;
                    inode.data_offset = first_cluster; // repurposed: first cluster
                    inode.data_cap = 0;
                    inode.parent = dir_inode;
                    inode.children_head = NO_INODE;
                    inode.next_sibling = NO_INODE;
                    inode.dev_major = mount_id; // tag as FAT32
                    inode.dev_minor = 0;
                    inode.set_name(name);

                    // Link into parent's children list.
                    let old_head = inodes.inodes[dir_inode as usize].children_head;
                    inodes.inodes[id as usize].next_sibling = old_head;
                    inodes.inodes[dir_inode as usize].children_head = id;
                }
            }

            // Follow FAT chain.
            clust = match self.next_cluster(clust) {
                Some(c) => c,
                None => break,
            };
        }
        true
    }

    // ─────────────────────────────────────────────────────────────────
    // File read
    // ─────────────────────────────────────────────────────────────────

    /// Read from a FAT32 file. `inode_id` must be tagged with a mount_id
    /// (dev_major > 0) and its `data_offset` holds the first cluster.
    pub fn read(
        &mut self,
        inodes: &InodeTable,
        inode_id: u16,
        offset: u32,
        out: &mut [u8],
    ) -> usize {
        let idx = inode_id as usize;
        if idx >= MAX_INODES {
            return 0;
        }
        let inode = &inodes.inodes[idx];
        if inode.kind != InodeKind::File {
            return 0;
        }
        let fsize = inode.size;
        if offset >= fsize {
            return 0;
        }
        let avail = (fsize - offset) as usize;
        let to_read = out.len().min(avail);
        if to_read == 0 {
            return 0;
        }

        let first_cluster = inode.data_offset;
        let cluster_bytes = self.spc as u32 * self.bps as u32;
        if cluster_bytes == 0 {
            return 0;
        }

        // Walk the FAT chain to the cluster containing `offset`.
        let skip_clusters = offset / cluster_bytes;
        let mut clust = first_cluster;
        for _ in 0..skip_clusters {
            clust = match self.next_cluster(clust) {
                Some(c) => c,
                None => return 0,
            };
        }

        let mut pos = 0usize;
        let mut byte_off = (offset % cluster_bytes) as usize;

        while pos < to_read {
            let sec_in_clust = byte_off / SECTOR_SIZE;
            let off_in_sec = byte_off % SECTOR_SIZE;
            let sec = self.cluster_to_sector(clust) + sec_in_clust as u32;

            if !self.read_sector(sec) {
                break;
            }

            let chunk = (SECTOR_SIZE - off_in_sec).min(to_read - pos);
            out[pos..pos + chunk]
                .copy_from_slice(&self.buf[off_in_sec..off_in_sec + chunk]);
            pos += chunk;
            byte_off += chunk;

            // Move to next cluster if we've consumed this one.
            if byte_off >= cluster_bytes as usize {
                byte_off = 0;
                clust = match self.next_cluster(clust) {
                    Some(c) => c,
                    None => break,
                };
            }
        }
        pos
    }

    // ─────────────────────────────────────────────────────────────────
    // File write
    // ─────────────────────────────────────────────────────────────────

    /// Write to a FAT32 file. Allocates clusters as needed.
    pub fn write(
        &mut self,
        inodes: &mut InodeTable,
        inode_id: u16,
        offset: u32,
        data: &[u8],
    ) -> usize {
        let idx = inode_id as usize;
        if idx >= MAX_INODES || data.is_empty() {
            return 0;
        }
        if inodes.inodes[idx].kind != InodeKind::File {
            return 0;
        }

        let cluster_bytes = self.spc as u32 * self.bps as u32;
        if cluster_bytes == 0 {
            return 0;
        }

        // Ensure the file has at least one cluster.
        let mut first_cluster = inodes.inodes[idx].data_offset;
        if first_cluster == 0 {
            first_cluster = match self.alloc_cluster() {
                Some(c) => c,
                None => return 0,
            };
            inodes.inodes[idx].data_offset = first_cluster;
        }

        let _need_end = offset as u64 + data.len() as u64;

        // Walk chain, extending as necessary, to cover offset + data.len().
        let skip_clusters = offset / cluster_bytes;
        let mut clust = first_cluster;
        for _ in 0..skip_clusters {
            let next = self.next_cluster(clust);
            clust = match next {
                Some(c) => c,
                None => {
                    let nc = match self.alloc_cluster() {
                        Some(c) => c,
                        None => return 0,
                    };
                    self.write_fat_entry(clust, nc);
                    nc
                }
            };
        }

        let mut pos = 0usize;
        let mut byte_off = (offset % cluster_bytes) as usize;

        while pos < data.len() {
            let sec_in_clust = byte_off / SECTOR_SIZE;
            let off_in_sec = byte_off % SECTOR_SIZE;
            let sec = self.cluster_to_sector(clust) + sec_in_clust as u32;

            // For partial-sector writes, read-modify-write.
            let chunk = (SECTOR_SIZE - off_in_sec).min(data.len() - pos);
            if chunk < SECTOR_SIZE {
                // Read existing sector first.
                self.read_sector(sec);
            }
            self.buf[off_in_sec..off_in_sec + chunk]
                .copy_from_slice(&data[pos..pos + chunk]);
            if !self.write_sector_out(sec) {
                break;
            }

            pos += chunk;
            byte_off += chunk;

            if byte_off >= cluster_bytes as usize {
                byte_off = 0;
                let next = self.next_cluster(clust);
                clust = match next {
                    Some(c) => c,
                    None => {
                        if pos < data.len() {
                            let nc = match self.alloc_cluster() {
                                Some(c) => c,
                                None => break,
                            };
                            self.write_fat_entry(clust, nc);
                            nc
                        } else {
                            break;
                        }
                    }
                };
            }
        }

        // Update file size if we wrote past the end.
        let new_end = offset + pos as u32;
        if new_end > inodes.inodes[idx].size {
            inodes.inodes[idx].size = new_end;
        }

        self.flush_fat();
        pos
    }

    // ─────────────────────────────────────────────────────────────────
    // File truncate (free cluster chain, reset size)
    // ─────────────────────────────────────────────────────────────────

    pub fn truncate(&mut self, inodes: &mut InodeTable, inode_id: u16) -> bool {
        let idx = inode_id as usize;
        if idx >= MAX_INODES {
            return false;
        }
        let first_cluster = inodes.inodes[idx].data_offset;
        if first_cluster >= 2 {
            self.free_chain(first_cluster);
        }
        inodes.inodes[idx].data_offset = 0;
        inodes.inodes[idx].size = 0;
        self.flush_fat();
        true
    }

    // ─────────────────────────────────────────────────────────────────
    // Create a new file in a FAT32 directory
    // ─────────────────────────────────────────────────────────────────

    /// Create a new (empty) file under `parent_inode`. Returns the new
    /// VFS inode ID, or `None` if the inode table or the directory is full.
    pub fn create_file(
        &mut self,
        inodes: &mut InodeTable,
        parent_inode: u16,
        name: &str,
        mount_id: u8,
    ) -> Option<u16> {
        // Build 8.3 name.
        let raw83 = make_83(name);

        // Find the parent's cluster (stored in data_offset, or root_clust).
        let pidx = parent_inode as usize;
        if pidx >= MAX_INODES {
            return None;
        }
        let parent_cluster = if inodes.inodes[pidx].data_offset != 0 {
            inodes.inodes[pidx].data_offset
        } else {
            self.root_clust
        };

        // Scan the directory for a free 32-byte slot.
        let mut clust = parent_cluster;
        'chain: loop {
            let base_sec = self.cluster_to_sector(clust);
            for sec_off in 0..self.spc as u32 {
                let sec = base_sec + sec_off;
                if !self.read_sector(sec) {
                    return None;
                }
                for e in 0..(SECTOR_SIZE / 32) {
                    let off = e * 32;
                    let first = self.buf[off];
                    if first == 0x00 || first == 0xE5 {
                        // Found a free slot — write the new entry.
                        self.buf[off..off + 11].copy_from_slice(&raw83);
                        self.buf[off + DE_ATTR] = 0x20; // ATTR_ARCHIVE
                        // Zero remaining fields.
                        for b in &mut self.buf[off + 12..off + 32] {
                            *b = 0;
                        }
                        if !self.write_sector_out(sec) {
                            return None;
                        }

                        // Create VFS inode.
                        let id = inodes.alloc()?;
                        let inode = &mut inodes.inodes[id as usize];
                        inode.kind = InodeKind::File;
                        inode.size = 0;
                        inode.data_offset = 0;
                        inode.data_cap = 0;
                        inode.parent = parent_inode;
                        inode.children_head = NO_INODE;
                        inode.next_sibling = NO_INODE;
                        inode.dev_major = mount_id;
                        inode.dev_minor = 0;
                        inode.set_name(name);

                        let old_head =
                            inodes.inodes[parent_inode as usize].children_head;
                        inodes.inodes[id as usize].next_sibling = old_head;
                        inodes.inodes[parent_inode as usize].children_head = id;

                        return Some(id);
                    }
                }
            }
            clust = match self.next_cluster(clust) {
                Some(c) => c,
                None => break 'chain,
            };
        }
        None // directory full
    }

    // ═════════════════════════════════════════════════════════════════
    // Internal helpers
    // ═════════════════════════════════════════════════════════════════

    /// Convert a cluster number to the absolute sector number.
    fn cluster_to_sector(&self, c: u32) -> u32 {
        self.data_start + (c - 2) * self.spc as u32
    }

    /// Follow the FAT chain: return the next cluster, or `None` if EOC.
    fn next_cluster(&mut self, c: u32) -> Option<u32> {
        let entry = self.read_fat_entry(c);
        if entry >= FAT_EOC || entry < 2 {
            None
        } else {
            Some(entry)
        }
    }

    /// Read one FAT entry for cluster `c`.
    fn read_fat_entry(&mut self, c: u32) -> u32 {
        let fat_offset = c * 4;
        let fat_sector = self.fat_start + fat_offset / self.bps as u32;
        let off_in_sec = (fat_offset % self.bps as u32) as usize;

        if self.fat_sec != fat_sector {
            self.flush_fat();
            let abs_sec = self.part_offset + fat_sector;
            if let Some(read) = self.read_blk {
                if !read(abs_sec as u64, &mut self.fat_buf) {
                    return FAT_EOC;
                }
            }
            self.fat_sec = fat_sector;
        }
        le32(&self.fat_buf, off_in_sec) & FAT_MASK
    }

    /// Write one FAT entry.
    fn write_fat_entry(&mut self, c: u32, val: u32) {
        let fat_offset = c * 4;
        let fat_sector = self.fat_start + fat_offset / self.bps as u32;
        let off_in_sec = (fat_offset % self.bps as u32) as usize;

        // Ensure the correct FAT sector is loaded.
        if self.fat_sec != fat_sector {
            self.flush_fat();
            let abs_sec = self.part_offset + fat_sector;
            if let Some(read) = self.read_blk {
                read(abs_sec as u64, &mut self.fat_buf);
            }
            self.fat_sec = fat_sector;
        }

        // Preserve the upper 4 bits.
        let old = le32(&self.fat_buf, off_in_sec);
        let new = (old & 0xF000_0000) | (val & FAT_MASK);
        self.fat_buf[off_in_sec] = new as u8;
        self.fat_buf[off_in_sec + 1] = (new >> 8) as u8;
        self.fat_buf[off_in_sec + 2] = (new >> 16) as u8;
        self.fat_buf[off_in_sec + 3] = (new >> 24) as u8;
        self.fat_dirty = true;
    }

    /// Allocate a single free cluster, mark it as EOC.
    fn alloc_cluster(&mut self) -> Option<u32> {
        // Simple linear scan starting from cluster 2.
        // A smarter implementation would cache the last allocated cluster.
        let total_data_sec = le32(&self.buf, BPB_TOTSEC32).saturating_sub(self.data_start);
        let total_clusters = total_data_sec / self.spc as u32;
        let max_cluster = total_clusters + 2;

        for c in 2..max_cluster {
            let entry = self.read_fat_entry(c);
            if entry == FAT_FREE {
                self.write_fat_entry(c, FAT_EOC | 0x0FFF_FFFF);
                return Some(c);
            }
        }
        None
    }

    /// Free an entire cluster chain starting at `start`.
    fn free_chain(&mut self, start: u32) {
        let mut c = start;
        loop {
            let next = self.read_fat_entry(c);
            self.write_fat_entry(c, FAT_FREE);
            if next >= FAT_EOC || next < 2 {
                break;
            }
            c = next;
        }
    }

    /// Flush the cached FAT sector back to disk (both FAT copies).
    fn flush_fat(&mut self) -> bool {
        if !self.fat_dirty || self.fat_sec == 0xFFFF_FFFF {
            return true;
        }
        if let Some(write) = self.write_blk {
            let abs_sec = self.part_offset + self.fat_sec;
            if !write(abs_sec as u64, &self.fat_buf) {
                return false;
            }
            // Also write the second FAT copy.
            if self.nfat > 1 {
                let sec2 = abs_sec + self.fatsz;
                write(sec2 as u64, &self.fat_buf);
            }
        }
        self.fat_dirty = false;
        true
    }

    /// Read a sector (relative to partition start) into `self.buf`.
    fn read_sector(&mut self, sec: u32) -> bool {
        if let Some(read) = self.read_blk {
            read((self.part_offset + sec) as u64, &mut self.buf)
        } else {
            false
        }
    }

    /// Write `self.buf` to a sector (relative to partition start).
    fn write_sector_out(&mut self, sec: u32) -> bool {
        if let Some(write) = self.write_blk {
            write((self.part_offset + sec) as u64, &self.buf)
        } else {
            false
        }
    }
}

// ─── 8.3 name helpers ────────────────────────────────────────────────

/// Parse a raw FAT32 8.3 name into a lowercase "name.ext" string.
/// Returns the number of bytes written to `out`.
fn parse_83(raw: &[u8], out: &mut [u8; 13]) -> usize {
    let mut pos = 0;
    // Base name (first 8 chars, trim trailing spaces).
    for i in 0..8 {
        if raw[i] == b' ' {
            break;
        }
        out[pos] = to_lower(raw[i]);
        pos += 1;
    }
    // Extension (chars 8–10, trim trailing spaces).
    if raw[8] != b' ' {
        out[pos] = b'.';
        pos += 1;
        for i in 8..11 {
            if raw[i] == b' ' {
                break;
            }
            out[pos] = to_lower(raw[i]);
            pos += 1;
        }
    }
    pos
}

/// Build a FAT32 8.3 directory-entry name from a VFS name.
/// If the name doesn't fit, it is truncated to 8.3 format.
fn make_83(name: &str) -> [u8; 11] {
    let mut out = [b' '; 11];
    let bytes = name.as_bytes();

    // Find last dot for extension split.
    let dot = bytes.iter().rposition(|&b| b == b'.');

    let (base, ext) = match dot {
        Some(d) => (&bytes[..d], &bytes[d + 1..]),
        None => (bytes, &[][..]),
    };

    let base_len = base.len().min(8);
    for i in 0..base_len {
        out[i] = to_upper(base[i]);
    }
    let ext_len = ext.len().min(3);
    for i in 0..ext_len {
        out[8 + i] = to_upper(ext[i]);
    }
    out
}

#[inline]
fn to_lower(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' {
        b + 32
    } else {
        b
    }
}

#[inline]
fn to_upper(b: u8) -> u8 {
    if b >= b'a' && b <= b'z' {
        b - 32
    } else {
        b
    }
}

// ─── Little-endian helpers ───────────────────────────────────────────

#[inline]
fn le16(buf: &[u8], off: usize) -> u16 {
    (buf[off] as u16) | ((buf[off + 1] as u16) << 8)
}

#[inline]
fn le32(buf: &[u8], off: usize) -> u32 {
    (buf[off] as u32)
        | ((buf[off + 1] as u32) << 8)
        | ((buf[off + 2] as u32) << 16)
        | ((buf[off + 3] as u32) << 24)
}

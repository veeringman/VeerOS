//! RamFS — in-memory filesystem data storage for VeerOS.
//!
//! File data is stored in a dedicated static byte pool, separate from
//! the kernel heap, to avoid fragmenting shared memory. A simple bump
//! allocator hands out contiguous byte ranges for file content.
//!
//! # Pool sizing
//! - ESP32-C6: 8 KB (tight — suitable for config files, small logs)
//! - QEMU / RPi: 64 KB (generous for development/demos)

use crate::vfs::{InodeKind, InodeTable, MAX_INODES};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// RamFS data pool size: 64 KB for hosted/QEMU builds, 8 KB for ESP32.
#[cfg(not(feature = "ramfs-small"))]
pub const RAMFS_POOL_SIZE: usize = 64 * 1024;

#[cfg(feature = "ramfs-small")]
pub const RAMFS_POOL_SIZE: usize = 8 * 1024;

/// Minimum allocation unit for file data (helps reduce fragmentation).
pub const RAMFS_ALLOC_ALIGN: usize = 64;

// ---------------------------------------------------------------------------
// RamFS
// ---------------------------------------------------------------------------

/// In-memory file data storage backed by a static byte pool.
pub struct RamFs {
    /// The data pool.
    pool: [u8; RAMFS_POOL_SIZE],
    /// Bump pointer: next free byte offset in the pool.
    next_free: usize,
}

impl RamFs {
    pub const fn new() -> Self {
        Self {
            pool: [0u8; RAMFS_POOL_SIZE],
            next_free: 0,
        }
    }

    /// Total pool size.
    pub fn capacity(&self) -> usize {
        RAMFS_POOL_SIZE
    }

    /// Bytes currently allocated.
    pub fn used(&self) -> usize {
        self.next_free
    }

    /// Bytes remaining.
    pub fn free(&self) -> usize {
        RAMFS_POOL_SIZE.saturating_sub(self.next_free)
    }

    /// Allocate `size` bytes from the pool. Returns the offset, or `None`.
    fn pool_alloc(&mut self, size: usize) -> Option<u32> {
        // Round up to alignment.
        let aligned = (size + RAMFS_ALLOC_ALIGN - 1) & !(RAMFS_ALLOC_ALIGN - 1);
        if self.next_free + aligned > RAMFS_POOL_SIZE {
            return None;
        }
        let offset = self.next_free as u32;
        self.next_free += aligned;
        Some(offset)
    }

    /// Read from a file inode's data. Returns bytes read.
    pub fn read(&self, inodes: &InodeTable, inode_id: u16, offset: u32, buf: &mut [u8]) -> usize {
        let idx = inode_id as usize;
        if idx >= MAX_INODES {
            return 0;
        }
        let inode = &inodes.inodes[idx];
        if inode.kind != InodeKind::File {
            return 0;
        }
        let file_size = inode.size;
        if offset >= file_size {
            return 0;
        }
        let available = (file_size - offset) as usize;
        let to_read = if buf.len() < available {
            buf.len()
        } else {
            available
        };
        let start = (inode.data_offset + offset) as usize;
        if start + to_read > RAMFS_POOL_SIZE {
            return 0;
        }
        buf[..to_read].copy_from_slice(&self.pool[start..start + to_read]);
        to_read
    }

    /// Write to a file inode's data at `offset`. Grows the file if needed.
    /// Returns bytes written.
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

        let needed_end = offset as usize + data.len();

        // Check if we need to allocate or grow.
        let inode_cap = inodes.inodes[idx].data_cap as usize;
        if inode_cap == 0 {
            // First write: allocate.
            let alloc_size = if needed_end > RAMFS_ALLOC_ALIGN {
                needed_end
            } else {
                RAMFS_ALLOC_ALIGN
            };
            match self.pool_alloc(alloc_size) {
                Some(off) => {
                    inodes.inodes[idx].data_offset = off;
                    inodes.inodes[idx].data_cap =
                        ((alloc_size + RAMFS_ALLOC_ALIGN - 1) & !(RAMFS_ALLOC_ALIGN - 1)) as u32;
                }
                None => return 0,
            }
        } else if needed_end > inode_cap {
            // Need more space. If the inode is at the end of the pool,
            // we can extend in-place. Otherwise, reallocate.
            let inode_end = inodes.inodes[idx].data_offset as usize + inode_cap;
            if inode_end == self.next_free {
                // Extend in place.
                let extra = needed_end - inode_cap;
                let aligned_extra = (extra + RAMFS_ALLOC_ALIGN - 1) & !(RAMFS_ALLOC_ALIGN - 1);
                if self.next_free + aligned_extra > RAMFS_POOL_SIZE {
                    return 0;
                }
                self.next_free += aligned_extra;
                inodes.inodes[idx].data_cap += aligned_extra as u32;
            } else {
                // Reallocate: copy to new location.
                let new_cap = if needed_end > inode_cap * 2 {
                    needed_end
                } else {
                    inode_cap * 2
                };
                match self.pool_alloc(new_cap) {
                    Some(new_off) => {
                        let old_off = inodes.inodes[idx].data_offset as usize;
                        let old_size = inodes.inodes[idx].size as usize;
                        // Copy existing data.
                        // We can't use copy_from_slice on overlapping regions,
                        // but pool_alloc always gives us addresses past next_free
                        // so there's no overlap.
                        let new_start = new_off as usize;
                        for i in 0..old_size {
                            self.pool[new_start + i] = self.pool[old_off + i];
                        }
                        inodes.inodes[idx].data_offset = new_off;
                        inodes.inodes[idx].data_cap =
                            ((new_cap + RAMFS_ALLOC_ALIGN - 1) & !(RAMFS_ALLOC_ALIGN - 1)) as u32;
                        // Note: old space is leaked (no compaction).
                    }
                    None => return 0,
                }
            }
        }

        // Write data.
        let start = inodes.inodes[idx].data_offset as usize + offset as usize;
        let to_write = data.len();
        if start + to_write > RAMFS_POOL_SIZE {
            return 0;
        }
        self.pool[start..start + to_write].copy_from_slice(data);

        // Update file size.
        let new_end = offset as u32 + to_write as u32;
        if new_end > inodes.inodes[idx].size {
            inodes.inodes[idx].size = new_end;
        }
        to_write
    }

    /// Truncate a file to `new_size` bytes.
    pub fn truncate(&self, inodes: &mut InodeTable, inode_id: u16, new_size: u32) -> bool {
        let idx = inode_id as usize;
        if idx >= MAX_INODES {
            return false;
        }
        if inodes.inodes[idx].kind != InodeKind::File {
            return false;
        }
        if new_size <= inodes.inodes[idx].size {
            inodes.inodes[idx].size = new_size;
            true
        } else {
            false
        }
    }

    /// Create an initial file with content (e.g., `/etc/motd`).
    pub fn create_with_content(
        &mut self,
        inodes: &mut InodeTable,
        parent: u16,
        name: &str,
        content: &[u8],
    ) -> Option<u16> {
        let id = inodes.create_file_in(parent, name)?;
        if !content.is_empty() {
            let written = self.write(inodes, id, 0, content);
            if written != content.len() {
                // Failed to write — clean up.
                inodes.unlink(id);
                return None;
            }
        }
        Some(id)
    }
}

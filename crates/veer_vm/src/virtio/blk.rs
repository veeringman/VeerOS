//! Virtio-blk device — file-backed block storage.
//!
//! Implements VIRTIO 1.0 *legacy* (transitional) semantics, matching the
//! guest driver in `crates/soc/qemu_pc/src/virtio_blk.rs`. That driver:
//!
//!   * reads device-config `capacity` (u64 at cfg-offset 0),
//!   * allocates a 3-descriptor chain per request:
//!       `[header (R) → data (R for WRITE, W for READ) → status (W)]`,
//!   * polls the used ring for completion.
//!
//! We perform the I/O synchronously on the backing file via `pread`/`pwrite`
//! on the raw fd, so request ordering is preserved.

use anyhow::{bail, Context, Result};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::FileExt;
use std::path::Path;

use super::{
    pop_avail, push_used, walk_chain, DescItem, VirtioDevice, VirtioTransport,
    VirtioTransportSnapshot, STATUS_DRIVER_OK,
};
use crate::memory::GuestMem;

/// Sector size in the legacy virtio-blk interface (always 512).
const SECTOR_SIZE: usize = 512;

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;

const VIRTIO_BLK_S_OK: u8 = 0;
const VIRTIO_BLK_S_IOERR: u8 = 1;
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct BlkReqHeader {
    req_type: u32,
    reserved: u32,
    sector: u64,
}

pub struct VirtioBlk {
    transport: VirtioTransport,
    file: File,
    /// Total capacity in 512-byte sectors.
    capacity: u64,
    /// Whether we should surface as read-only to the guest (F_RO).
    read_only: bool,
}

#[derive(Clone)]
pub struct VirtioBlkSnapshot {
    pub transport: VirtioTransportSnapshot,
    pub capacity: u64,
    pub read_only: bool,
}

impl VirtioBlk {
    pub fn open(path: &Path, read_only: bool) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(!read_only)
            .open(path)
            .with_context(|| format!("open disk image {}", path.display()))?;
        let size = file.metadata()?.len();
        if size == 0 {
            bail!("disk image {} is empty", path.display());
        }
        if size % SECTOR_SIZE as u64 != 0 {
            bail!(
                "disk image {} size {} is not a multiple of 512",
                path.display(),
                size,
            );
        }
        let capacity = size / SECTOR_SIZE as u64;

        // Feature negotiation: advertise only F_RO if we opened read-only.
        // We intentionally skip F_SIZE_MAX / F_SEG_MAX / F_GEOMETRY — the
        // guest driver ignores them all.
        let features = if read_only {
            1u32 << 5 /* VIRTIO_BLK_F_RO */
        } else {
            0
        };
        let transport = VirtioTransport::new(/*num_queues=*/ 1, features);

        Ok(Self {
            transport,
            file,
            capacity,
            read_only,
        })
    }

    pub fn capacity_sectors(&self) -> u64 {
        self.capacity
    }
    pub fn driver_ok(&self) -> bool {
        self.transport.device_status & STATUS_DRIVER_OK != 0
    }

    pub fn snapshot_state(&self) -> VirtioBlkSnapshot {
        VirtioBlkSnapshot {
            transport: self.transport.snapshot(),
            capacity: self.capacity,
            read_only: self.read_only,
        }
    }

    pub fn restore_state(&mut self, snap: &VirtioBlkSnapshot) -> Result<()> {
        if self.capacity != snap.capacity {
            bail!(
                "virtio-blk capacity mismatch: snapshot={} sectors device={} sectors",
                snap.capacity,
                self.capacity
            );
        }
        if self.read_only != snap.read_only {
            bail!(
                "virtio-blk read_only mismatch: snapshot={} device={}",
                snap.read_only,
                self.read_only
            );
        }
        self.transport.restore(&snap.transport)
    }
}

impl VirtioDevice for VirtioBlk {
    fn config_read(&self, cfg_offset: u16, buf: &mut [u8]) {
        // Config area layout (legacy blk): [capacity:u64 @ 0, ...]
        // Anything beyond 8 bytes is unused; return 0.
        let mut raw = [0u8; 16];
        raw[0..8].copy_from_slice(&self.capacity.to_le_bytes());
        let off = cfg_offset as usize;
        for (i, b) in buf.iter_mut().enumerate() {
            *b = if off + i < raw.len() { raw[off + i] } else { 0 };
        }
    }

    fn transport(&self) -> &VirtioTransport {
        &self.transport
    }
    fn transport_mut(&mut self) -> &mut VirtioTransport {
        &mut self.transport
    }

    fn notify(&mut self, queue_idx: u16, mem: &GuestMem) -> Result<bool> {
        if queue_idx != 0 {
            // virtio-blk only has one queue.
            return Ok(false);
        }
        // Take the queue state out by index, do not hold a &mut self.transport
        // across the push_used call which also needs &mut queue.
        let mut any = false;
        loop {
            let head = {
                let q = &mut self.transport.queues[0];
                if !q.is_ready() {
                    return Ok(false);
                }
                match pop_avail(mem, q)? {
                    Some(h) => h,
                    None => break,
                }
            };
            let q_ref = &self.transport.queues[0]; // immutable snapshot for walk
            let chain = walk_chain(mem, q_ref, head)?;

            let bytes_written = match self.process_one(&chain, mem) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("[veer-vm] virtio-blk: request failed: {e:#}");
                    // Still report completion (status byte already written
                    // as IOERR by process_one on failure path when it can).
                    0
                }
            };

            {
                let q = &mut self.transport.queues[0];
                push_used(mem, q, head, bytes_written)?;
            }
            self.transport.isr_status |= 0x1; // queue event
            any = true;
        }
        Ok(any)
    }
}

impl VirtioBlk {
    /// Process a single 3+ descriptor chain. Returns the total number of
    /// bytes the device wrote into writable descriptors (used-ring `len`).
    fn process_one(&mut self, chain: &[DescItem], mem: &GuestMem) -> Result<u32> {
        if chain.len() < 2 {
            bail!("blk request chain too short ({} descriptors)", chain.len());
        }
        // Split into readable (device reads → guest->device) and writable
        // (device writes → device->guest).
        let (readable, writable): (Vec<_>, Vec<_>) =
            chain.iter().enumerate().partition(|(_, d)| !d.writable);

        if readable.is_empty() || writable.is_empty() {
            bail!("blk request missing readable or writable descriptors");
        }

        // First readable = request header (16 bytes).
        let hdr_desc = readable[0].1;
        if (hdr_desc.len as usize) < std::mem::size_of::<BlkReqHeader>() {
            bail!("blk header descriptor too small ({} bytes)", hdr_desc.len);
        }
        let hdr_slice = mem.slice_mut(hdr_desc.addr, hdr_desc.len as usize)?;
        let header = unsafe { *(hdr_slice.as_ptr() as *const BlkReqHeader) };

        // Last writable = status byte.
        let (status_pos_in_chain, status_desc) = *writable.last().unwrap();
        if status_desc.len < 1 {
            bail!("blk status descriptor too small");
        }
        // Helper to write the status byte at the end of the chain.
        let write_status = |mem: &GuestMem, byte: u8| -> Result<()> {
            let s = mem.slice_mut(status_desc.addr, 1)?;
            s[0] = byte;
            Ok(())
        };

        // Dispatch.
        match header.req_type {
            VIRTIO_BLK_T_IN => {
                // Data descriptors = all writable except the final status byte.
                let data_total: u64 = writable
                    .iter()
                    .filter(|(pos, _)| *pos != status_pos_in_chain)
                    .map(|(_, d)| d.len as u64)
                    .sum();
                if data_total % SECTOR_SIZE as u64 != 0 {
                    write_status(mem, VIRTIO_BLK_S_IOERR)?;
                    bail!("blk read data total {} not sector-multiple", data_total);
                }
                if header.sector + data_total / SECTOR_SIZE as u64 > self.capacity {
                    write_status(mem, VIRTIO_BLK_S_IOERR)?;
                    bail!("blk read past end of disk");
                }
                let mut off = header.sector * SECTOR_SIZE as u64;
                let mut copied_into_writable: u64 = 0;
                for (pos, d) in writable.iter() {
                    if *pos == status_pos_in_chain {
                        continue;
                    }
                    let dst = mem.slice_mut(d.addr, d.len as usize)?;
                    self.file
                        .read_exact_at(dst, off)
                        .with_context(|| format!("blk read sector {}", header.sector))?;
                    off += d.len as u64;
                    copied_into_writable += d.len as u64;
                }
                write_status(mem, VIRTIO_BLK_S_OK)?;
                // used.len = bytes written into writable area, including
                // the status byte per the virtio spec.
                Ok((copied_into_writable + 1) as u32)
            }
            VIRTIO_BLK_T_OUT => {
                if self.read_only {
                    write_status(mem, VIRTIO_BLK_S_IOERR)?;
                    return Ok(1);
                }
                // Data descriptors = all readable except the first (header).
                let data_total: u64 = readable.iter().skip(1).map(|(_, d)| d.len as u64).sum();
                if data_total % SECTOR_SIZE as u64 != 0 {
                    write_status(mem, VIRTIO_BLK_S_IOERR)?;
                    bail!("blk write data total {} not sector-multiple", data_total);
                }
                if header.sector + data_total / SECTOR_SIZE as u64 > self.capacity {
                    write_status(mem, VIRTIO_BLK_S_IOERR)?;
                    bail!("blk write past end of disk");
                }
                let mut off = header.sector * SECTOR_SIZE as u64;
                for (_, d) in readable.iter().skip(1) {
                    let src = mem.slice_mut(d.addr, d.len as usize)?;
                    self.file
                        .write_all_at(src, off)
                        .with_context(|| format!("blk write sector {}", header.sector))?;
                    off += d.len as u64;
                }
                write_status(mem, VIRTIO_BLK_S_OK)?;
                Ok(1)
            }
            other => {
                eprintln!("[veer-vm] virtio-blk: unsupported request type {other}");
                write_status(mem, VIRTIO_BLK_S_UNSUPP)?;
                Ok(1)
            }
        }
    }
}

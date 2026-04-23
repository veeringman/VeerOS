//! Virtio-legacy-PCI transport (queue registers + virtqueue walking).
//!
//! Provides [`VirtioTransport`], the shared register block every virtio
//! legacy device exposes at the front of its I/O BAR (offsets 0x00..0x14),
//! and [`Virtqueue`], the split-ring walker that iterates descriptors in
//! guest physical memory.
//!
//! Device-specific config starts at BAR offset 0x14 (when MSI-X is
//! disabled, which is always the case for us). Devices implement
//! [`VirtioDevice`] to plug device-specific features/config/queue-handling.

use anyhow::{bail, Result};
use std::sync::Arc;

use crate::memory::GuestMem;

pub mod blk;
pub mod net;

// ── Legacy register offsets (within the I/O BAR) ─────────────────────────
pub const REG_DEVICE_FEATURES: u16 = 0x00; // 32-bit, RO
pub const REG_GUEST_FEATURES:  u16 = 0x04; // 32-bit, RW
pub const REG_QUEUE_ADDRESS:   u16 = 0x08; // 32-bit, RW (PFN)
pub const REG_QUEUE_SIZE:      u16 = 0x0C; // 16-bit, RO
pub const REG_QUEUE_SELECT:    u16 = 0x0E; // 16-bit, RW
pub const REG_QUEUE_NOTIFY:    u16 = 0x10; // 16-bit, WO
pub const REG_DEVICE_STATUS:   u16 = 0x12; // 8-bit, RW
pub const REG_ISR_STATUS:      u16 = 0x13; // 8-bit, RO (clear-on-read)
pub const REG_DEVICE_CONFIG:   u16 = 0x14; // device-specific config starts here

pub const VRING_DESC_F_NEXT:  u16 = 1;
pub const VRING_DESC_F_WRITE: u16 = 2;

/// Advertised queue size (must be a power of two ≤ guest's max of 256).
pub const QUEUE_SIZE: u16 = 128;

pub const STATUS_ACKNOWLEDGE: u8 = 1;
pub const STATUS_DRIVER:      u8 = 2;
pub const STATUS_DRIVER_OK:   u8 = 4;
pub const STATUS_FEATURES_OK: u8 = 8;
pub const STATUS_FAILED:      u8 = 128;

// ── In-guest split-ring layout ───────────────────────────────────────────

/// Virtqueue descriptor (16 bytes, matches the spec exactly).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VringDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}

/// Per-queue state inside the VMM.
#[derive(Default)]
pub struct QueueState {
    /// PFN programmed by the guest (0 = not configured).
    pub pfn: u32,
    /// Advertised size (we pick [`QUEUE_SIZE`]).
    pub size: u16,
    /// Last avail.idx we consumed (monotonic wrapping u16).
    pub last_avail_idx: u16,
    /// Next used.idx we will write (monotonic wrapping u16).
    pub next_used_idx: u16,
}

impl QueueState {
    pub fn new() -> Self {
        Self { pfn: 0, size: QUEUE_SIZE, last_avail_idx: 0, next_used_idx: 0 }
    }

    pub fn is_ready(&self) -> bool { self.pfn != 0 }
    pub fn base_gpa(&self) -> u64 { (self.pfn as u64) << 12 }

    pub fn desc_gpa(&self) -> u64 { self.base_gpa() }
    pub fn avail_gpa(&self) -> u64 {
        self.base_gpa() + 16 * self.size as u64
    }
    pub fn used_gpa(&self) -> u64 {
        let desc_sz = 16 * self.size as u64;
        let avail_sz = 6 + 2 * self.size as u64;
        let combined = desc_sz + avail_sz;
        // Guest aligns used ring up to PAGE_SIZE (4096) after desc+avail.
        self.base_gpa() + align_up(combined, 4096)
    }
}

fn align_up(x: u64, a: u64) -> u64 { (x + a - 1) & !(a - 1) }

// ── Transport (common register block) ────────────────────────────────────

pub struct VirtioTransport {
    /// Features advertised to the guest (low 32 bits — legacy interface).
    pub device_features: u32,
    /// Features the guest accepted (after negotiation).
    pub guest_features: u32,
    /// Device status byte (STATUS_* bits).
    pub device_status: u8,
    /// ISR status (bit 0 = queue event, bit 1 = config change).
    /// Cleared on read per the spec.
    pub isr_status: u8,
    /// Currently selected queue (for REG_QUEUE_*).
    pub queue_select: u16,
    /// All queues. Index is queue idx (0 for blk's requestq).
    pub queues: Vec<QueueState>,
}

impl VirtioTransport {
    pub fn new(num_queues: usize, device_features: u32) -> Self {
        Self {
            device_features,
            guest_features: 0,
            device_status: 0,
            isr_status: 0,
            queue_select: 0,
            queues: (0..num_queues).map(|_| QueueState::new()).collect(),
        }
    }

    /// Read from one of the common-header registers. Returns the value
    /// left-padded into a 4-byte little-endian buffer; the caller takes
    /// `buf.len()` bytes starting at offset 0. Offsets are *register*
    /// offsets (0x00..0x14) not BAR offsets.
    pub fn read_reg(&mut self, offset: u16, buf: &mut [u8]) {
        let val: u32 = match offset {
            REG_DEVICE_FEATURES => self.device_features,
            REG_GUEST_FEATURES  => self.guest_features,
            REG_QUEUE_ADDRESS   => {
                self.queues.get(self.queue_select as usize)
                    .map(|q| q.pfn).unwrap_or(0)
            }
            REG_QUEUE_SIZE => QUEUE_SIZE as u32,
            REG_QUEUE_SELECT => self.queue_select as u32,
            REG_QUEUE_NOTIFY => 0, // write-only; guest never reads
            REG_DEVICE_STATUS => self.device_status as u32,
            REG_ISR_STATUS => {
                let v = self.isr_status as u32;
                self.isr_status = 0; // clear-on-read
                v
            }
            _ => 0,
        };
        let bytes = val.to_le_bytes();
        let n = buf.len().min(4);
        buf[..n].copy_from_slice(&bytes[..n]);
    }

    pub fn write_reg(&mut self, offset: u16, data: &[u8]) {
        // Widen to u32 by zero-extending the incoming bytes.
        let mut bytes = [0u8; 4];
        let n = data.len().min(4);
        bytes[..n].copy_from_slice(&data[..n]);
        let val = u32::from_le_bytes(bytes);
        match offset {
            REG_GUEST_FEATURES => { self.guest_features = val; }
            REG_QUEUE_ADDRESS => {
                if let Some(q) = self.queues.get_mut(self.queue_select as usize) {
                    q.pfn = val;
                }
            }
            REG_QUEUE_SELECT => { self.queue_select = val as u16; }
            REG_DEVICE_STATUS => { self.device_status = val as u8; }
            _ => { /* device_features, queue_size, isr are RO; ignore */ }
        }
    }
}

// ── Virtqueue walker ─────────────────────────────────────────────────────

/// A single descriptor extracted from the guest ring, with its raw index
/// and direction already interpreted.
pub struct DescItem {
    pub addr: u64,
    pub len: u32,
    pub writable: bool,
}

/// Walk a descriptor chain starting at `head`. Returns the chain's
/// descriptors in order. Stops on missing NEXT flag or if the chain
/// exceeds `queue_size` (malformed guest).
pub fn walk_chain(
    mem: &GuestMem,
    queue: &QueueState,
    head: u16,
) -> Result<Vec<DescItem>> {
    let mut items = Vec::new();
    let mut cur = head;
    for _ in 0..queue.size {
        let gpa = queue.desc_gpa() + (cur as u64) * 16;
        let slot = mem.slice_mut(gpa, 16)?;
        let desc = unsafe { *(slot.as_ptr() as *const VringDesc) };
        items.push(DescItem {
            addr: desc.addr,
            len: desc.len,
            writable: desc.flags & VRING_DESC_F_WRITE != 0,
        });
        if desc.flags & VRING_DESC_F_NEXT == 0 {
            return Ok(items);
        }
        cur = desc.next;
    }
    bail!("descriptor chain exceeded queue size (malformed guest ring)")
}

/// Read a u16 from guest memory at `gpa`.
pub fn read_u16(mem: &GuestMem, gpa: u64) -> Result<u16> {
    let s = mem.slice_mut(gpa, 2)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

pub fn write_u16(mem: &GuestMem, gpa: u64, v: u16) -> Result<()> {
    let s = mem.slice_mut(gpa, 2)?;
    s.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

pub fn write_u32(mem: &GuestMem, gpa: u64, v: u32) -> Result<()> {
    let s = mem.slice_mut(gpa, 4)?;
    s.copy_from_slice(&v.to_le_bytes());
    Ok(())
}

/// Pop one entry from the avail ring if any are pending. Returns the head
/// descriptor index. Increments `last_avail_idx`.
pub fn pop_avail(mem: &GuestMem, queue: &mut QueueState) -> Result<Option<u16>> {
    // avail struct: [flags:u16, idx:u16, ring:[u16;size], used_event:u16]
    let avail_idx = read_u16(mem, queue.avail_gpa() + 2)?;
    // Memory fence after reading avail.idx (acquire): ensures we observe
    // descriptor writes that happened before the guest bumped the index.
    std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
    if avail_idx == queue.last_avail_idx {
        return Ok(None);
    }
    let ring_off = 4 + (queue.last_avail_idx as u64 % queue.size as u64) * 2;
    let head = read_u16(mem, queue.avail_gpa() + ring_off)?;
    queue.last_avail_idx = queue.last_avail_idx.wrapping_add(1);
    Ok(Some(head))
}

/// Push a used-ring entry and bump used.idx.
///
/// `id` is the head descriptor index; `len` is the total number of bytes
/// the device wrote into writable descriptors (per the virtio spec).
pub fn push_used(mem: &GuestMem, queue: &mut QueueState, id: u16, len: u32) -> Result<()> {
    // used struct: [flags:u16, idx:u16, ring:[(id:u32,len:u32);size], avail_event:u16]
    let slot = queue.next_used_idx as u64 % queue.size as u64;
    let elem_off = 4 + slot * 8;
    write_u32(mem, queue.used_gpa() + elem_off, id as u32)?;
    write_u32(mem, queue.used_gpa() + elem_off + 4, len)?;
    // Release fence: ensure the element write is visible before idx bump.
    std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
    queue.next_used_idx = queue.next_used_idx.wrapping_add(1);
    write_u16(mem, queue.used_gpa() + 2, queue.next_used_idx)?;
    Ok(())
}

/// Every concrete virtio device (blk, net, console…) implements this.
pub trait VirtioDevice: Send {
    /// Read a byte from the device-specific config area (BAR offset ≥ 0x14).
    fn config_read(&self, cfg_offset: u16, buf: &mut [u8]);
    /// Handle a QUEUE_NOTIFY write with the given queue index. Returns
    /// `Ok(true)` if any used entries were pushed (caller may raise IRQ).
    fn notify(&mut self, queue_idx: u16, mem: &GuestMem) -> Result<bool>;
    /// Access the transport for BAR offsets 0x00..0x14.
    fn transport_mut(&mut self) -> &mut VirtioTransport;
    fn transport(&self) -> &VirtioTransport;
}

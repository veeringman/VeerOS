//! VIRTIO block device driver (virtio-blk-pci, legacy interface).
//!
//! Provides sector-level read/write access to a VIRTIO block device.
//! The driver uses a single requestq (queue 0) and processes requests
//! synchronously via polling.
//!
//! Request format (3-descriptor chain):
//!   0: VirtioBlkReqHeader (type, reserved, sector)  — device-readable
//!   1: Data buffer (512 * N bytes)                   — device-readable (write) or writable (read)
//!   2: Status byte                                   — device-writable

use crate::mm::FrameAllocator;
use crate::virtio::{self, Virtqueue, VRING_DESC_F_NEXT, VRING_DESC_F_WRITE};

/// Block size (fixed 512 bytes for legacy virtio-blk).
pub const SECTOR_SIZE: usize = 512;

/// VIRTIO block request types.
const VIRTIO_BLK_T_IN: u32 = 0; // read
const VIRTIO_BLK_T_OUT: u32 = 1; // write

/// VIRTIO block status codes.
const VIRTIO_BLK_S_OK: u8 = 0;
#[allow(dead_code)]
const VIRTIO_BLK_S_IOERR: u8 = 1;
#[allow(dead_code)]
const VIRTIO_BLK_S_UNSUPP: u8 = 2;

/// Block feature bits.
#[allow(dead_code)]
const VIRTIO_BLK_F_SIZE_MAX: u32 = 1 << 1;
#[allow(dead_code)]
const VIRTIO_BLK_F_SEG_MAX: u32 = 1 << 2;
#[allow(dead_code)]
const VIRTIO_BLK_F_GEOMETRY: u32 = 1 << 4;
#[allow(dead_code)]
const VIRTIO_BLK_F_RO: u32 = 1 << 5;
#[allow(dead_code)]
const VIRTIO_BLK_F_BLK_SIZE: u32 = 1 << 6;

/// VIRTIO block request header (16 bytes).
#[repr(C)]
struct VirtioBlkReqHeader {
    req_type: u32,
    reserved: u32,
    sector: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// Block device state
// ═══════════════════════════════════════════════════════════════════════════

/// A VIRTIO block device.
pub struct VirtioBlk {
    /// I/O base address (from PCI BAR0).
    pub io_base: u16,
    /// Request queue (queue 0).
    pub queue: Option<Virtqueue>,
    /// Device capacity in sectors.
    pub capacity: u64,
    /// Whether the device is read-only.
    pub read_only: bool,
    /// Whether the device has been initialized.
    pub active: bool,
    /// Scratch buffer for request headers (statically allocated to avoid heap).
    req_header: VirtioBlkReqHeader,
    /// Status byte for completed requests.
    status_byte: u8,
}

impl VirtioBlk {
    pub const fn new() -> Self {
        Self {
            io_base: 0,
            queue: None,
            capacity: 0,
            read_only: false,
            active: false,
            req_header: VirtioBlkReqHeader {
                req_type: 0,
                reserved: 0,
                sector: 0,
            },
            status_byte: 0xFF,
        }
    }

    /// Initialize the VIRTIO block device.
    ///
    /// `io_base` — I/O port base from PCI BAR0.
    /// `alloc` — frame allocator for virtqueue memory.
    pub fn init(&mut self, io_base: u16, alloc: &mut FrameAllocator) -> bool {
        self.io_base = io_base;

        // Reset device.
        virtio::device_reset(io_base);

        // Acknowledge + driver.
        virtio::device_set_status(io_base, virtio::STATUS_ACKNOWLEDGE);
        virtio::device_set_status(io_base, virtio::STATUS_DRIVER);

        // Read features.
        let features = virtio::device_features(io_base);
        self.read_only = (features & VIRTIO_BLK_F_RO) != 0;

        // Accept a minimal feature set.
        virtio::guest_features(io_base, 0);

        // Set up queue 0 (requestq).
        match Virtqueue::new(io_base, 0, alloc) {
            Some(q) => self.queue = Some(q),
            None => {
                virtio::device_set_status(io_base, virtio::STATUS_FAILED);
                return false;
            }
        }

        // Read capacity from device config (offset 0, u64).
        self.capacity = virtio::config_read64(io_base, 0);

        // Mark driver OK.
        virtio::device_set_status(io_base, virtio::STATUS_DRIVER_OK);
        self.active = true;
        true
    }

    /// Read sectors from the block device.
    ///
    /// `sector` — starting sector number.
    /// `buf` — destination buffer (must be at least `count * 512` bytes).
    /// `count` — number of sectors to read.
    ///
    /// Returns `true` on success.
    pub fn read_sectors(&mut self, sector: u64, buf: &mut [u8], count: usize) -> bool {
        if !self.active || buf.len() < count * SECTOR_SIZE {
            return false;
        }
        self.do_request(VIRTIO_BLK_T_IN, sector, buf.as_mut_ptr(), count)
    }

    /// Write sectors to the block device.
    ///
    /// `sector` — starting sector number.
    /// `buf` — source buffer (must be at least `count * 512` bytes).
    /// `count` — number of sectors to write.
    ///
    /// Returns `true` on success.
    pub fn write_sectors(&mut self, sector: u64, buf: &[u8], count: usize) -> bool {
        if !self.active || self.read_only || buf.len() < count * SECTOR_SIZE {
            return false;
        }
        self.do_request(VIRTIO_BLK_T_OUT, sector, buf.as_ptr() as *mut u8, count)
    }

    /// Submit a block I/O request and wait for completion.
    fn do_request(&mut self, req_type: u32, sector: u64, data: *mut u8, count: usize) -> bool {
        let queue = match self.queue.as_mut() {
            Some(q) => q,
            None => return false,
        };

        // Need 3 descriptors.
        if queue.free_count < 3 {
            return false;
        }

        // Set up request header.
        self.req_header = VirtioBlkReqHeader {
            req_type,
            reserved: 0,
            sector,
        };
        self.status_byte = 0xFF;

        let d0 = match queue.alloc_desc() {
            Some(d) => d,
            None => return false,
        };
        let d1 = match queue.alloc_desc() {
            Some(d) => d,
            None => {
                queue.free_desc(d0);
                return false;
            }
        };
        let d2 = match queue.alloc_desc() {
            Some(d) => d,
            None => {
                queue.free_desc(d1);
                queue.free_desc(d0);
                return false;
            }
        };

        // Descriptor 0: request header (device-readable).
        unsafe {
            let desc = &mut *queue.desc.add(d0 as usize);
            desc.addr = &self.req_header as *const VirtioBlkReqHeader as u64;
            desc.len = core::mem::size_of::<VirtioBlkReqHeader>() as u32;
            desc.flags = VRING_DESC_F_NEXT;
            desc.next = d1;
        }

        // Descriptor 1: data buffer.
        unsafe {
            let desc = &mut *queue.desc.add(d1 as usize);
            desc.addr = data as u64;
            desc.len = (count * SECTOR_SIZE) as u32;
            desc.flags = VRING_DESC_F_NEXT;
            if req_type == VIRTIO_BLK_T_IN {
                desc.flags |= VRING_DESC_F_WRITE; // device writes to this buffer
            }
            desc.next = d2;
        }

        // Descriptor 2: status byte (device-writable).
        unsafe {
            let desc = &mut *queue.desc.add(d2 as usize);
            desc.addr = &self.status_byte as *const u8 as u64;
            desc.len = 1;
            desc.flags = VRING_DESC_F_WRITE;
            desc.next = 0;
        }

        // Submit and poll for completion.
        queue.submit(d0);

        // Spin-wait for the used ring to advance.
        let mut timeout = 1_000_000u32;
        while queue.poll_used().is_none() {
            core::hint::spin_loop();
            timeout -= 1;
            if timeout == 0 {
                // Timeout — free descriptors and fail.
                queue.free_desc(d2);
                queue.free_desc(d1);
                queue.free_desc(d0);
                return false;
            }
        }

        // Free descriptors.
        queue.free_desc(d2);
        queue.free_desc(d1);
        queue.free_desc(d0);

        self.status_byte == VIRTIO_BLK_S_OK
    }

    /// Device capacity in bytes.
    pub fn capacity_bytes(&self) -> u64 {
        self.capacity * SECTOR_SIZE as u64
    }

    /// Device capacity in sectors.
    pub fn capacity_sectors(&self) -> u64 {
        self.capacity
    }
}

//! Minimal VIRTIO-NET MMIO driver for the QEMU `virt` RISC-V machine.
//!
//! QEMU's `virt` board exposes VIRTIO MMIO devices starting at 0x1000_1000
//! with 0x1000 spacing.  We probe for a net device (device ID 1) and drive
//! it with a bare-minimum two-virtqueue (RX + TX) implementation.
//!
//! Supports both legacy (MMIO v1) and modern (MMIO v2) transports.
//!
//! Reference: VIRTIO 1.1 spec §§ 2.1, 4.2 (MMIO), 5.1 (Net device).

use arch::NetworkDevice;
use core::cell::UnsafeCell;
use core::sync::atomic::{fence, Ordering};

// ═══════════════════════════════════════════════════════════════════════════
// MMIO register offsets
// ═══════════════════════════════════════════════════════════════════════════

const MAGIC_VALUE: usize = 0x000;
const VERSION: usize = 0x004;
const DEVICE_ID: usize = 0x008;
const DEVICE_FEATURES: usize = 0x010;
const DEVICE_FEATURES_SEL: usize = 0x014;
const DRIVER_FEATURES: usize = 0x020;
const DRIVER_FEATURES_SEL: usize = 0x024;
const QUEUE_SEL: usize = 0x030;
const QUEUE_NUM_MAX: usize = 0x034;
const QUEUE_NUM: usize = 0x038;
const QUEUE_READY: usize = 0x044; // modern (v2)
const QUEUE_NOTIFY: usize = 0x050;
const INTERRUPT_STATUS: usize = 0x060;
const INTERRUPT_ACK: usize = 0x064;
const STATUS: usize = 0x070;
const QUEUE_DESC_LOW: usize = 0x080; // modern (v2)
const QUEUE_DESC_HIGH: usize = 0x084;
const QUEUE_AVAIL_LOW: usize = 0x090;
const QUEUE_AVAIL_HIGH: usize = 0x094;
const QUEUE_USED_LOW: usize = 0x0A0;
const QUEUE_USED_HIGH: usize = 0x0A4;
const NET_MAC_BASE: usize = 0x100;

// Legacy-only registers (MMIO v1)
const GUEST_PAGE_SIZE: usize = 0x028;
const QUEUE_ALIGN: usize = 0x03C;
const QUEUE_PFN: usize = 0x040;

// ═══════════════════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════════════════

const VIRTIO_MAGIC: u32 = 0x74726976; // "virt"
const VIRTIO_NET_DEVICE_ID: u32 = 1;

const STATUS_ACK: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;

const VIRTIO_NET_F_MAC: u32 = 1 << 5;
// Modern devices require VIRTIO_F_VERSION_1 (bit 32 = word1 bit 0)
const VIRTIO_F_VERSION_1_BIT: u32 = 1 << 0; // in feature word 1

const RX_QUEUE: u32 = 0;
const TX_QUEUE: u32 = 1;

const QUEUE_SIZE: u16 = 16;
const PAGE_SIZE: u32 = 4096;

const FRAME_BUF_SIZE: usize = 1526;
const VIRTIO_NET_HDR_SIZE: usize = 10;

// ═══════════════════════════════════════════════════════════════════════════
// MMIO helpers
// ═══════════════════════════════════════════════════════════════════════════

#[inline(always)]
unsafe fn read32(base: usize, off: usize) -> u32 {
    core::ptr::read_volatile((base + off) as *const u32)
}

#[inline(always)]
unsafe fn write32(base: usize, off: usize, val: u32) {
    core::ptr::write_volatile((base + off) as *mut u32, val);
}

#[inline(always)]
unsafe fn read8(base: usize, off: usize) -> u8 {
    core::ptr::read_volatile((base + off) as *const u8)
}

// ═══════════════════════════════════════════════════════════════════════════
// Virtqueue structures (packed exactly as the spec requires)
// ═══════════════════════════════════════════════════════════════════════════

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

const VIRTQ_DESC_F_WRITE: u16 = 2;

/// A single contiguous, page-aligned region that holds desc + avail + used
/// in the layout the VIRTIO spec requires.
///
/// Layout for QUEUE_SIZE=16:
///   [0x000..0x100)  16 descriptors × 16 bytes = 256 bytes
///   [0x100..0x126)  avail ring: 2+2+2*16+2 = 38 bytes
///   [0x126..0x1000) padding
///   [0x1000..0x1088) used ring: 2+2+8*16+2 = 134 bytes
///
/// Total: 2 pages.  Both legacy PFN and modern split-pointer init work.
const VQ_REGION_SIZE: usize = 8192; // 2 pages

#[repr(C, align(4096))]
struct VqRegion {
    raw: UnsafeCell<[u8; VQ_REGION_SIZE]>,
}

// SAFETY: single-core bare-metal, no concurrent writers.
unsafe impl Sync for VqRegion {}

impl VqRegion {
    const fn new() -> Self {
        Self {
            raw: UnsafeCell::new([0u8; VQ_REGION_SIZE]),
        }
    }

    fn base_addr(&self) -> usize {
        self.raw.get() as *const u8 as usize
    }

    /// Descriptor table at offset 0.
    fn desc_ptr(&self) -> *mut VirtqDesc {
        self.base_addr() as *mut VirtqDesc
    }

    /// Available ring right after the descriptor table.
    /// Offset = QUEUE_SIZE * sizeof(VirtqDesc) = 16 * 16 = 256.
    fn avail_flags_ptr(&self) -> *mut u16 {
        (self.base_addr() + QUEUE_SIZE as usize * 16) as *mut u16
    }
    fn avail_idx_ptr(&self) -> *mut u16 {
        (self.base_addr() + QUEUE_SIZE as usize * 16 + 2) as *mut u16
    }
    fn avail_ring_ptr(&self) -> *mut u16 {
        (self.base_addr() + QUEUE_SIZE as usize * 16 + 4) as *mut u16
    }

    /// Used ring at the next page boundary (offset 0x1000).
    fn used_addr(&self) -> usize {
        self.base_addr() + PAGE_SIZE as usize
    }
    #[allow(dead_code)]
    fn used_flags_ptr(&self) -> *mut u16 {
        self.used_addr() as *mut u16
    }
    fn used_idx_ptr(&self) -> *const u16 {
        (self.used_addr() + 2) as *const u16
    }
    fn used_ring_id_ptr(&self, i: usize) -> *const u32 {
        (self.used_addr() + 4 + i * 8) as *const u32
    }
    fn used_ring_len_ptr(&self, i: usize) -> *const u32 {
        (self.used_addr() + 4 + i * 8 + 4) as *const u32
    }
}

/// Per-queue frame buffers (separate from the VQ region).
#[repr(C, align(16))]
struct VqBufs {
    bufs: UnsafeCell<[[u8; FRAME_BUF_SIZE]; QUEUE_SIZE as usize]>,
    last_used_idx: UnsafeCell<u16>,
}

unsafe impl Sync for VqBufs {}

impl VqBufs {
    const fn new() -> Self {
        Self {
            bufs: UnsafeCell::new([[0u8; FRAME_BUF_SIZE]; QUEUE_SIZE as usize]),
            last_used_idx: UnsafeCell::new(0),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Static storage (two queues: RX=0, TX=1)
// ═══════════════════════════════════════════════════════════════════════════

static RX_REGION: VqRegion = VqRegion::new();
static TX_REGION: VqRegion = VqRegion::new();
static RX_BUFS: VqBufs = VqBufs::new();
static TX_BUFS: VqBufs = VqBufs::new();

// ═══════════════════════════════════════════════════════════════════════════
// VirtioNet driver
// ═══════════════════════════════════════════════════════════════════════════

pub struct VirtioNet {
    base: usize,
    mac: [u8; 6],
    mmio_version: u32,
}

impl VirtioNet {
    /// Probe VIRTIO MMIO slots for a net device and initialise it.
    pub fn probe() -> Option<Self> {
        for i in 0..8 {
            let base = 0x1000_1000 + i * 0x1000;
            unsafe {
                let magic = read32(base, MAGIC_VALUE);
                let version = read32(base, VERSION);
                let dev_id = read32(base, DEVICE_ID);
                if magic == VIRTIO_MAGIC
                    && (version == 1 || version == 2)
                    && dev_id == VIRTIO_NET_DEVICE_ID
                {
                    return Some(Self::init(base, version));
                }
            }
        }
        None
    }

    /// Return the detected MMIO version (1=legacy, 2=modern) for diagnostics.
    pub fn mmio_version(&self) -> u32 {
        self.mmio_version
    }

    unsafe fn init(base: usize, version: u32) -> Self {
        let is_modern = version == 2;

        // ── Reset ────────────────────────────────────────
        write32(base, STATUS, 0);
        fence(Ordering::SeqCst);

        // ── Acknowledge ──────────────────────────────────
        write32(base, STATUS, STATUS_ACK);
        write32(base, STATUS, STATUS_ACK | STATUS_DRIVER);

        // ── Negotiate features ───────────────────────────
        // Word 0
        write32(base, DEVICE_FEATURES_SEL, 0);
        let dev_features_0 = read32(base, DEVICE_FEATURES);
        let mut accepted_0 = dev_features_0 & VIRTIO_NET_F_MAC;

        // Word 1 (modern devices require VIRTIO_F_VERSION_1)
        if is_modern {
            write32(base, DEVICE_FEATURES_SEL, 1);
            let _dev_features_1 = read32(base, DEVICE_FEATURES);
            write32(base, DRIVER_FEATURES_SEL, 1);
            write32(base, DRIVER_FEATURES, VIRTIO_F_VERSION_1_BIT);
        } else {
            write32(base, DRIVER_FEATURES_SEL, 1);
            write32(base, DRIVER_FEATURES, 0);
        }

        write32(base, DRIVER_FEATURES_SEL, 0);
        write32(base, DRIVER_FEATURES, accepted_0);

        write32(
            base,
            STATUS,
            STATUS_ACK | STATUS_DRIVER | STATUS_FEATURES_OK,
        );
        fence(Ordering::SeqCst);
        let st = read32(base, STATUS);
        if st & STATUS_FEATURES_OK == 0 {
            // Feature negotiation failed — try without MAC
            accepted_0 = 0;
        }

        // Legacy: set guest page size
        if !is_modern {
            write32(base, GUEST_PAGE_SIZE, PAGE_SIZE);
        }

        // ── Init RX queue ────────────────────────────────
        Self::init_queue(base, RX_QUEUE, &RX_REGION, &RX_BUFS, true, is_modern);

        // ── Init TX queue ────────────────────────────────
        Self::init_queue(base, TX_QUEUE, &TX_REGION, &TX_BUFS, false, is_modern);

        // ── Driver OK ────────────────────────────────────
        write32(
            base,
            STATUS,
            STATUS_ACK | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        );
        fence(Ordering::SeqCst);

        // ── Read MAC address ─────────────────────────────
        let mut mac = [0u8; 6];
        if accepted_0 & VIRTIO_NET_F_MAC != 0 {
            for j in 0..6 {
                mac[j] = read8(base, NET_MAC_BASE + j);
            }
        } else {
            mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
        }

        Self {
            base,
            mac,
            mmio_version: version,
        }
    }

    unsafe fn init_queue(
        base: usize,
        idx: u32,
        region: &VqRegion,
        qbufs: &VqBufs,
        is_rx: bool,
        is_modern: bool,
    ) {
        write32(base, QUEUE_SEL, idx);

        let max = read32(base, QUEUE_NUM_MAX);
        if max == 0 {
            return;
        }
        let num = (QUEUE_SIZE as u32).min(max);
        write32(base, QUEUE_NUM, num);

        // Zero the region.
        let raw = &mut *region.raw.get();
        raw.fill(0);
        *qbufs.last_used_idx.get() = 0;

        let desc_addr = region.base_addr();
        let avail_addr = desc_addr + QUEUE_SIZE as usize * 16;
        let used_addr = region.used_addr();

        if is_modern {
            // Modern (v2): tell device the three addresses separately.
            write32(base, QUEUE_DESC_LOW, desc_addr as u32);
            write32(base, QUEUE_DESC_HIGH, 0);
            write32(base, QUEUE_AVAIL_LOW, avail_addr as u32);
            write32(base, QUEUE_AVAIL_HIGH, 0);
            write32(base, QUEUE_USED_LOW, used_addr as u32);
            write32(base, QUEUE_USED_HIGH, 0);
        } else {
            // Legacy (v1): PFN-based, device computes avail/used from the
            // contiguous region starting at PFN * page_size.
            write32(base, QUEUE_ALIGN, PAGE_SIZE);
            write32(base, QUEUE_PFN, (desc_addr as u32) / PAGE_SIZE);
        }

        // ── Populate descriptors ─────────────────────────
        let bufs = &mut *qbufs.bufs.get();
        for i in 0..num as usize {
            let d = region.desc_ptr().add(i);
            (*d).addr = bufs[i].as_ptr() as u64;
            (*d).len = FRAME_BUF_SIZE as u32;
            (*d).flags = if is_rx { VIRTQ_DESC_F_WRITE } else { 0 };
            (*d).next = 0;
        }

        // ── Pre-fill RX available ring ───────────────────
        if is_rx {
            let avail_ring = region.avail_ring_ptr();
            for i in 0..num as usize {
                avail_ring.add(i).write_volatile(i as u16);
            }
            fence(Ordering::Release);
            // flags = 0 (no suppress)
            region.avail_flags_ptr().write_volatile(0);
            region.avail_idx_ptr().write_volatile(num as u16);
            fence(Ordering::Release);
        }

        if is_modern {
            write32(base, QUEUE_READY, 1);
        }

        // Notify device about available RX buffers.
        if is_rx {
            fence(Ordering::SeqCst);
            write32(base, QUEUE_NOTIFY, idx);
        }
    }
}

impl NetworkDevice for VirtioNet {
    fn mtu(&self) -> usize {
        1514
    }

    fn has_rx(&self) -> bool {
        unsafe {
            fence(Ordering::Acquire);
            let used_idx = RX_REGION.used_idx_ptr().read_volatile();
            let last = *RX_BUFS.last_used_idx.get();
            used_idx != last
        }
    }

    fn recv(&self, buf: &mut [u8]) -> usize {
        unsafe {
            fence(Ordering::Acquire);
            let used_idx = RX_REGION.used_idx_ptr().read_volatile();
            let last = *RX_BUFS.last_used_idx.get();

            if used_idx == last {
                return 0;
            }

            let slot = last as usize % QUEUE_SIZE as usize;
            let desc_id = RX_REGION.used_ring_id_ptr(slot).read_volatile() as usize;
            let total_len = RX_REGION.used_ring_len_ptr(slot).read_volatile() as usize;

            let bufs = &*RX_BUFS.bufs.get();
            let frame_start = VIRTIO_NET_HDR_SIZE;
            let frame_len = total_len.saturating_sub(frame_start);
            let copy_len = frame_len.min(buf.len());
            buf[..copy_len]
                .copy_from_slice(&bufs[desc_id][frame_start..frame_start + copy_len]);

            // Re-post this buffer to the available ring.
            let avail_idx = RX_REGION.avail_idx_ptr().read_volatile();
            let ring_slot = avail_idx as usize % QUEUE_SIZE as usize;
            RX_REGION.avail_ring_ptr().add(ring_slot).write_volatile(desc_id as u16);
            fence(Ordering::Release);
            RX_REGION
                .avail_idx_ptr()
                .write_volatile(avail_idx.wrapping_add(1));
            fence(Ordering::Release);
            write32(self.base, QUEUE_NOTIFY, RX_QUEUE);

            *RX_BUFS.last_used_idx.get() = last.wrapping_add(1);

            // Ack interrupt if pending.
            let isr = read32(self.base, INTERRUPT_STATUS);
            if isr != 0 {
                write32(self.base, INTERRUPT_ACK, isr);
            }

            copy_len
        }
    }

    fn send(&self, buf: &[u8]) {
        unsafe {
            let avail_idx = TX_REGION.avail_idx_ptr().read_volatile();
            let slot = avail_idx as usize % QUEUE_SIZE as usize;

            let bufs = &mut *TX_BUFS.bufs.get();
            // Virtio-net header (all zeros = no offload).
            bufs[slot][..VIRTIO_NET_HDR_SIZE].fill(0);
            let copy_len = buf.len().min(FRAME_BUF_SIZE - VIRTIO_NET_HDR_SIZE);
            bufs[slot][VIRTIO_NET_HDR_SIZE..VIRTIO_NET_HDR_SIZE + copy_len]
                .copy_from_slice(&buf[..copy_len]);

            // Update descriptor.
            let d = TX_REGION.desc_ptr().add(slot);
            (*d).addr = bufs[slot].as_ptr() as u64;
            (*d).len = (VIRTIO_NET_HDR_SIZE + copy_len) as u32;
            (*d).flags = 0;
            (*d).next = 0;

            TX_REGION.avail_ring_ptr().add(slot).write_volatile(slot as u16);
            fence(Ordering::Release);
            TX_REGION
                .avail_idx_ptr()
                .write_volatile(avail_idx.wrapping_add(1));
            fence(Ordering::Release);
            write32(self.base, QUEUE_NOTIFY, TX_QUEUE);

            // Spin briefly for TX completion.
            let target = avail_idx.wrapping_add(1);
            for _ in 0..100_000 {
                fence(Ordering::Acquire);
                if TX_REGION.used_idx_ptr().read_volatile() == target {
                    break;
                }
                core::hint::spin_loop();
            }
        }
    }

    fn mac_address(&self) -> [u8; 6] {
        self.mac
    }
}

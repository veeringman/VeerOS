//! Minimal VIRTIO-NET MMIO v2 driver for the `aarch64-virt` guest.
//!
//! The VeerOS HVF host backend (`veer_vm::backend::hvf_aarch64`) exposes a
//! single virtio-mmio v2 net device at a fixed MMIO address (`0x0A00_0000`).
//! This driver targets that single fixed device — no probe loop is needed.
//!
//! Reference: VIRTIO 1.1 spec §§ 4.2 (MMIO), 5.1 (Net device).

use arch::NetworkDevice;
use core::cell::UnsafeCell;
use core::sync::atomic::{fence, Ordering};

// ═══════════════════════════════════════════════════════════════════════════
// MMIO base — must match `VIRTIO_MMIO_NET_BASE` in `hvf_aarch64.rs`
// ═══════════════════════════════════════════════════════════════════════════

const MMIO_BASE: usize = 0x0A00_0000;

// ═══════════════════════════════════════════════════════════════════════════
// MMIO register offsets (VIRTIO 1.1 §4.2.2)
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
const QUEUE_READY: usize = 0x044;
const QUEUE_NOTIFY: usize = 0x050;
const INTERRUPT_STATUS: usize = 0x060;
const INTERRUPT_ACK: usize = 0x064;
const STATUS: usize = 0x070;
const QUEUE_DESC_LOW: usize = 0x080;
const QUEUE_DESC_HIGH: usize = 0x084;
const QUEUE_AVAIL_LOW: usize = 0x090;
const QUEUE_AVAIL_HIGH: usize = 0x094;
const QUEUE_USED_LOW: usize = 0x0A0;
const QUEUE_USED_HIGH: usize = 0x0A4;
const NET_MAC_BASE: usize = 0x100;

// ═══════════════════════════════════════════════════════════════════════════
// Device / feature constants
// ═══════════════════════════════════════════════════════════════════════════

const VIRTIO_MAGIC: u32 = 0x74726976; // "virt"
const VIRTIO_NET_DEVICE_ID: u32 = 1;

const STATUS_ACK: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;

const VIRTIO_NET_F_MAC: u32 = 1 << 5;
/// VIRTIO_F_VERSION_1 is bit 32 (word-1, bit 0) — required for v2.
const VIRTIO_F_VERSION_1_BIT: u32 = 1 << 0;

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
// Virtqueue structures
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

/// One contiguous, page-aligned region that holds desc + avail + used.
///
/// Layout for QUEUE_SIZE = 16:
///   [0x0000..0x0100)  16 descriptors × 16 bytes = 256 bytes
///   [0x0100..0x0126)  avail ring header (2+2) + 16 u16 + padding
///   [0x1000..0x1088)  used ring: 2+2 + 16×8 + 2 = 134 bytes
///
/// Two pages total.
const VQ_REGION_SIZE: usize = 8192;

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

    fn desc_ptr(&self) -> *mut VirtqDesc {
        self.base_addr() as *mut VirtqDesc
    }

    fn avail_flags_ptr(&self) -> *mut u16 {
        (self.base_addr() + QUEUE_SIZE as usize * 16) as *mut u16
    }
    fn avail_idx_ptr(&self) -> *mut u16 {
        (self.base_addr() + QUEUE_SIZE as usize * 16 + 2) as *mut u16
    }
    fn avail_ring_ptr(&self) -> *mut u16 {
        (self.base_addr() + QUEUE_SIZE as usize * 16 + 4) as *mut u16
    }

    fn used_addr(&self) -> usize {
        self.base_addr() + PAGE_SIZE as usize
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

/// Per-queue frame buffers.
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
// Static virtqueue storage
// ═══════════════════════════════════════════════════════════════════════════

static RX_REGION: VqRegion = VqRegion::new();
static TX_REGION: VqRegion = VqRegion::new();
static RX_BUFS: VqBufs = VqBufs::new();
static TX_BUFS: VqBufs = VqBufs::new();

// ═══════════════════════════════════════════════════════════════════════════
// VirtioMmioNet driver
// ═══════════════════════════════════════════════════════════════════════════

/// Guest-side virtio-mmio v2 network driver.
///
/// Call [`VirtioMmioNet::init`] once at boot.  Then use the [`NetworkDevice`]
/// implementation for smoltcp.
pub struct VirtioMmioNet {
    mac: [u8; 6],
}

impl VirtioMmioNet {
    /// Probe the fixed MMIO address and initialise the device.
    ///
    /// Returns `None` when no virtio-net device is present (wrong magic /
    /// device ID).  Always call this before touching the `NetworkDevice`
    /// methods.
    pub fn init() -> Option<Self> {
        unsafe {
            let magic = read32(MMIO_BASE, MAGIC_VALUE);
            let version = read32(MMIO_BASE, VERSION);
            let dev_id = read32(MMIO_BASE, DEVICE_ID);

            if magic != VIRTIO_MAGIC || version != 2 || dev_id != VIRTIO_NET_DEVICE_ID {
                return None;
            }

            Some(Self::do_init())
        }
    }

    unsafe fn do_init() -> Self {
        let base = MMIO_BASE;

        // ── Reset ────────────────────────────────────────
        write32(base, STATUS, 0);
        fence(Ordering::SeqCst);

        // ── Acknowledge device ───────────────────────────
        write32(base, STATUS, STATUS_ACK);
        write32(base, STATUS, STATUS_ACK | STATUS_DRIVER);

        // ── Negotiate features (word 0) ──────────────────
        write32(base, DEVICE_FEATURES_SEL, 0);
        let dev_f0 = read32(base, DEVICE_FEATURES);
        let accepted_0 = dev_f0 & VIRTIO_NET_F_MAC;

        write32(base, DRIVER_FEATURES_SEL, 0);
        write32(base, DRIVER_FEATURES, accepted_0);

        // Word 1: require VIRTIO_F_VERSION_1 for modern transport.
        write32(base, DEVICE_FEATURES_SEL, 1);
        let _dev_f1 = read32(base, DEVICE_FEATURES);
        write32(base, DRIVER_FEATURES_SEL, 1);
        write32(base, DRIVER_FEATURES, VIRTIO_F_VERSION_1_BIT);

        // Commit features.
        write32(
            base,
            STATUS,
            STATUS_ACK | STATUS_DRIVER | STATUS_FEATURES_OK,
        );
        fence(Ordering::SeqCst);

        // ── Set up virtqueues ────────────────────────────
        Self::init_queue(base, RX_QUEUE, &RX_REGION, &RX_BUFS, true);
        Self::init_queue(base, TX_QUEUE, &TX_REGION, &TX_BUFS, false);

        // ── Driver OK ────────────────────────────────────
        write32(
            base,
            STATUS,
            STATUS_ACK | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        );
        fence(Ordering::SeqCst);

        // ── Read MAC address ─────────────────────────────
        let mut mac = [0x52u8, 0x54, 0x00, 0x12, 0x34, 0x56];
        if accepted_0 & VIRTIO_NET_F_MAC != 0 {
            for (i, b) in mac.iter_mut().enumerate() {
                *b = read8(base, NET_MAC_BASE + i);
            }
        }

        Self { mac }
    }

    unsafe fn init_queue(base: usize, idx: u32, region: &VqRegion, qbufs: &VqBufs, is_rx: bool) {
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

        // Modern (v2): three-pointer split virtqueue.
        write32(base, QUEUE_DESC_LOW, desc_addr as u32);
        write32(base, QUEUE_DESC_HIGH, 0);
        write32(base, QUEUE_AVAIL_LOW, avail_addr as u32);
        write32(base, QUEUE_AVAIL_HIGH, 0);
        write32(base, QUEUE_USED_LOW, used_addr as u32);
        write32(base, QUEUE_USED_HIGH, 0);

        // Populate descriptor table.
        let bufs = &mut *qbufs.bufs.get();
        for i in 0..num as usize {
            let d = region.desc_ptr().add(i);
            (*d).addr = bufs[i].as_ptr() as u64;
            (*d).len = FRAME_BUF_SIZE as u32;
            (*d).flags = if is_rx { VIRTQ_DESC_F_WRITE } else { 0 };
            (*d).next = 0;
        }

        // Pre-fill available ring for RX.
        if is_rx {
            let avail_ring = region.avail_ring_ptr();
            for i in 0..num as usize {
                avail_ring.add(i).write_volatile(i as u16);
            }
            fence(Ordering::Release);
            region.avail_flags_ptr().write_volatile(0);
            region.avail_idx_ptr().write_volatile(num as u16);
            fence(Ordering::Release);
        }

        write32(base, QUEUE_READY, 1);

        // Notify device of RX buffer availability.
        if is_rx {
            fence(Ordering::SeqCst);
            write32(base, QUEUE_NOTIFY, idx);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// NetworkDevice impl
// ═══════════════════════════════════════════════════════════════════════════

impl NetworkDevice for VirtioMmioNet {
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
            buf[..copy_len].copy_from_slice(&bufs[desc_id][frame_start..frame_start + copy_len]);

            // Re-post this descriptor to the available ring.
            let avail_idx = RX_REGION.avail_idx_ptr().read_volatile();
            let ring_slot = avail_idx as usize % QUEUE_SIZE as usize;
            RX_REGION
                .avail_ring_ptr()
                .add(ring_slot)
                .write_volatile(desc_id as u16);
            fence(Ordering::Release);
            RX_REGION
                .avail_idx_ptr()
                .write_volatile(avail_idx.wrapping_add(1));
            fence(Ordering::Release);
            write32(MMIO_BASE, QUEUE_NOTIFY, RX_QUEUE);

            *RX_BUFS.last_used_idx.get() = last.wrapping_add(1);

            // Ack any pending interrupt.
            let isr = read32(MMIO_BASE, INTERRUPT_STATUS);
            if isr != 0 {
                write32(MMIO_BASE, INTERRUPT_ACK, isr);
            }

            copy_len
        }
    }

    fn send(&self, buf: &[u8]) {
        unsafe {
            let avail_idx = TX_REGION.avail_idx_ptr().read_volatile();
            let slot = avail_idx as usize % QUEUE_SIZE as usize;

            let bufs = &mut *TX_BUFS.bufs.get();
            // Virtio-net header (zero = no offload).
            bufs[slot][..VIRTIO_NET_HDR_SIZE].fill(0);
            let copy_len = buf.len().min(FRAME_BUF_SIZE - VIRTIO_NET_HDR_SIZE);
            bufs[slot][VIRTIO_NET_HDR_SIZE..VIRTIO_NET_HDR_SIZE + copy_len]
                .copy_from_slice(&buf[..copy_len]);

            let d = TX_REGION.desc_ptr().add(slot);
            (*d).addr = bufs[slot].as_ptr() as u64;
            (*d).len = (VIRTIO_NET_HDR_SIZE + copy_len) as u32;
            (*d).flags = 0;
            (*d).next = 0;

            TX_REGION
                .avail_ring_ptr()
                .add(slot)
                .write_volatile(slot as u16);
            fence(Ordering::Release);
            TX_REGION
                .avail_idx_ptr()
                .write_volatile(avail_idx.wrapping_add(1));
            fence(Ordering::Release);
            write32(MMIO_BASE, QUEUE_NOTIFY, TX_QUEUE);

            // Wait briefly for TX completion.
            let target = avail_idx.wrapping_add(1);
            for _ in 0..500_000u32 {
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

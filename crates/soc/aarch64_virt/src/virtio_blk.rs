//! Minimal VIRTIO-BLK MMIO v2 driver for the `aarch64-virt` guest.

use arch::BlockDevice;
use core::cell::UnsafeCell;
use core::sync::atomic::{fence, Ordering};

use crate::mem;

const MMIO_BASE: usize = mem::VIRTIO_BLK_BASE;

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
const CONFIG_BASE: usize = 0x100;

const VIRTIO_MAGIC: u32 = 0x74726976;
const VIRTIO_BLK_DEVICE_ID: u32 = 2;
const VIRTIO_F_VERSION_1_BIT: u32 = 1 << 0;

const STATUS_ACK: u32 = 1;
const STATUS_DRIVER: u32 = 2;
const STATUS_DRIVER_OK: u32 = 4;
const STATUS_FEATURES_OK: u32 = 8;

const QUEUE_SIZE: u16 = 8;
const PAGE_SIZE: usize = 4096;
const VQ_REGION_SIZE: usize = 8192;
const SECTOR_SIZE: usize = 512;

const VIRTQ_DESC_F_NEXT: u16 = 1;
const VIRTQ_DESC_F_WRITE: u16 = 2;

const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;
const VIRTIO_BLK_S_OK: u8 = 0;

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

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

#[repr(C, align(4096))]
struct VqRegion {
    raw: UnsafeCell<[u8; VQ_REGION_SIZE]>,
}

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
        self.base_addr() + PAGE_SIZE
    }

    fn used_idx_ptr(&self) -> *const u16 {
        (self.used_addr() + 2) as *const u16
    }
}

#[repr(C, align(16))]
struct BlkStorage {
    header: UnsafeCell<[u8; 16]>,
    data: UnsafeCell<[u8; SECTOR_SIZE]>,
    status: UnsafeCell<u8>,
    next_avail: UnsafeCell<u16>,
    last_used: UnsafeCell<u16>,
}

unsafe impl Sync for BlkStorage {}

impl BlkStorage {
    const fn new() -> Self {
        Self {
            header: UnsafeCell::new([0u8; 16]),
            data: UnsafeCell::new([0u8; SECTOR_SIZE]),
            status: UnsafeCell::new(0),
            next_avail: UnsafeCell::new(0),
            last_used: UnsafeCell::new(0),
        }
    }
}

static BLK_REGION: VqRegion = VqRegion::new();
static BLK_STORAGE: BlkStorage = BlkStorage::new();

pub struct VirtioMmioBlk {
    capacity: u64,
}

impl VirtioMmioBlk {
    pub fn init() -> Option<Self> {
        unsafe {
            let magic = read32(MMIO_BASE, MAGIC_VALUE);
            let version = read32(MMIO_BASE, VERSION);
            let dev_id = read32(MMIO_BASE, DEVICE_ID);
            if magic != VIRTIO_MAGIC || version != 2 || dev_id != VIRTIO_BLK_DEVICE_ID {
                return None;
            }
            Some(Self::do_init())
        }
    }

    unsafe fn do_init() -> Self {
        write32(MMIO_BASE, STATUS, 0);
        fence(Ordering::SeqCst);
        write32(MMIO_BASE, STATUS, STATUS_ACK);
        write32(MMIO_BASE, STATUS, STATUS_ACK | STATUS_DRIVER);

        write32(MMIO_BASE, DEVICE_FEATURES_SEL, 0);
        let _dev_f0 = read32(MMIO_BASE, DEVICE_FEATURES);
        write32(MMIO_BASE, DRIVER_FEATURES_SEL, 0);
        write32(MMIO_BASE, DRIVER_FEATURES, 0);
        write32(MMIO_BASE, DEVICE_FEATURES_SEL, 1);
        let _dev_f1 = read32(MMIO_BASE, DEVICE_FEATURES);
        write32(MMIO_BASE, DRIVER_FEATURES_SEL, 1);
        write32(MMIO_BASE, DRIVER_FEATURES, VIRTIO_F_VERSION_1_BIT);

        write32(
            MMIO_BASE,
            STATUS,
            STATUS_ACK | STATUS_DRIVER | STATUS_FEATURES_OK,
        );
        fence(Ordering::SeqCst);

        Self::init_queue();

        write32(
            MMIO_BASE,
            STATUS,
            STATUS_ACK | STATUS_DRIVER | STATUS_FEATURES_OK | STATUS_DRIVER_OK,
        );
        fence(Ordering::SeqCst);

        let mut cap_bytes = [0u8; 8];
        for (i, b) in cap_bytes.iter_mut().enumerate() {
            *b = read8(MMIO_BASE, CONFIG_BASE + i);
        }
        let capacity = u64::from_le_bytes(cap_bytes);
        Self { capacity }
    }

    unsafe fn init_queue() {
        write32(MMIO_BASE, QUEUE_SEL, 0);
        let max = read32(MMIO_BASE, QUEUE_NUM_MAX);
        if max == 0 {
            return;
        }
        write32(MMIO_BASE, QUEUE_NUM, (QUEUE_SIZE as u32).min(max));

        (*BLK_REGION.raw.get()).fill(0);
        *BLK_STORAGE.next_avail.get() = 0;
        *BLK_STORAGE.last_used.get() = 0;

        let base = BLK_REGION.base_addr();
        let avail = base + QUEUE_SIZE as usize * 16;
        let used = BLK_REGION.used_addr();
        write32(MMIO_BASE, QUEUE_DESC_LOW, base as u32);
        write32(MMIO_BASE, QUEUE_DESC_HIGH, 0);
        write32(MMIO_BASE, QUEUE_AVAIL_LOW, avail as u32);
        write32(MMIO_BASE, QUEUE_AVAIL_HIGH, 0);
        write32(MMIO_BASE, QUEUE_USED_LOW, used as u32);
        write32(MMIO_BASE, QUEUE_USED_HIGH, 0);
        BLK_REGION.avail_flags_ptr().write_volatile(0);
        write32(MMIO_BASE, QUEUE_READY, 1);
    }

    pub fn capacity_sectors(&self) -> u64 {
        self.capacity
    }

    fn request(&self, req_type: u32, lba: u64, buf: &mut [u8]) -> bool {
        if buf.len() < SECTOR_SIZE || lba >= self.capacity {
            return false;
        }
        unsafe {
            let header = &mut *BLK_STORAGE.header.get();
            header[..4].copy_from_slice(&req_type.to_le_bytes());
            header[4..8].copy_from_slice(&0u32.to_le_bytes());
            header[8..16].copy_from_slice(&lba.to_le_bytes());

            if req_type == VIRTIO_BLK_T_OUT {
                (&mut *BLK_STORAGE.data.get()).copy_from_slice(&buf[..SECTOR_SIZE]);
            }
            *BLK_STORAGE.status.get() = 0xff;

            let desc = BLK_REGION.desc_ptr();
            *desc.add(0) = VirtqDesc {
                addr: header.as_ptr() as u64,
                len: 16,
                flags: VIRTQ_DESC_F_NEXT,
                next: 1,
            };
            *desc.add(1) = VirtqDesc {
                addr: (*BLK_STORAGE.data.get()).as_ptr() as u64,
                len: SECTOR_SIZE as u32,
                flags: if req_type == VIRTIO_BLK_T_IN {
                    VIRTQ_DESC_F_WRITE | VIRTQ_DESC_F_NEXT
                } else {
                    VIRTQ_DESC_F_NEXT
                },
                next: 2,
            };
            *desc.add(2) = VirtqDesc {
                addr: BLK_STORAGE.status.get() as u64,
                len: 1,
                flags: VIRTQ_DESC_F_WRITE,
                next: 0,
            };

            let avail_idx = *BLK_STORAGE.next_avail.get();
            let ring_slot = avail_idx as usize % QUEUE_SIZE as usize;
            BLK_REGION.avail_ring_ptr().add(ring_slot).write_volatile(0);
            fence(Ordering::Release);
            BLK_REGION
                .avail_idx_ptr()
                .write_volatile(avail_idx.wrapping_add(1));
            *BLK_STORAGE.next_avail.get() = avail_idx.wrapping_add(1);
            fence(Ordering::SeqCst);
            write32(MMIO_BASE, QUEUE_NOTIFY, 0);

            let target = (*BLK_STORAGE.last_used.get()).wrapping_add(1);
            for _ in 0..5_000_000u32 {
                fence(Ordering::Acquire);
                if BLK_REGION.used_idx_ptr().read_volatile() == target {
                    *BLK_STORAGE.last_used.get() = target;
                    break;
                }
                core::hint::spin_loop();
            }

            let isr = read32(MMIO_BASE, INTERRUPT_STATUS);
            if isr != 0 {
                write32(MMIO_BASE, INTERRUPT_ACK, isr);
            }

            if *BLK_STORAGE.status.get() != VIRTIO_BLK_S_OK {
                return false;
            }

            if req_type == VIRTIO_BLK_T_IN {
                buf[..SECTOR_SIZE].copy_from_slice(&*BLK_STORAGE.data.get());
            }
            true
        }
    }
}

impl BlockDevice for VirtioMmioBlk {
    fn read_block(&self, lba: u64, buf: &mut [u8]) -> bool {
        self.request(VIRTIO_BLK_T_IN, lba, buf)
    }

    fn write_block(&self, lba: u64, buf: &[u8]) -> bool {
        if buf.len() < SECTOR_SIZE {
            return false;
        }
        let mut tmp = [0u8; SECTOR_SIZE];
        tmp.copy_from_slice(&buf[..SECTOR_SIZE]);
        self.request(VIRTIO_BLK_T_OUT, lba, &mut tmp)
    }

    fn block_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn block_count(&self) -> u64 {
        self.capacity
    }
}

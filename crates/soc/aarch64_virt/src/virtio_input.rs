//! Virtio input driver: keyboard + tablet/mouse over one event queue.
//!
//! Each `-device virtio-{keyboard,tablet}-pci` exposes a single event
//! queue of `virtio_input_event` structs (`{type u16, code u16,
//! value u32}`, LE). The driver pre-posts device-writable buffers, polls
//! the used ring (no interrupts), and decodes into [`arch::InputEvent`].
//!
//! Keyboard codes are Linux `input-event-codes.h` values mapped to ASCII
//! (shift-aware); arrows reuse the `uart_kbd` `0x81..0x84` codes so UI
//! layers treat both keyboards identically. Pointer motion (relative or
//! absolute) becomes `MouseMove`; buttons become `MouseButton`.

use arch::InputEvent;
use core::cell::UnsafeCell;
use core::sync::atomic::{fence, Ordering};

use crate::pcie::PciId;
use crate::virtio_pci::VirtioPci;

// ─── Virtio IDs / event protocol ───────────────────────────────────────────

/// Virtio device ID for input (PCI modern ID `0x1040 + 18`).
pub const VIRTIO_INPUT_PCI_DEVICE: u16 = 0x1052;
/// Virtio vendor ID.
pub const VIRTIO_PCI_VENDOR: u16 = 0x1AF4;

/// Input event queue index.
const EVENTQ: u16 = 0;
/// Ring capacity (must cover the device queue size; QEMU uses 64).
const RING_CAP: usize = 64;
/// Bytes per `virtio_input_event`.
const EV_SIZE: usize = 8;

// Event types.
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
// SYN codes.
const SYN_REPORT: u16 = 0;
// REL codes.
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
// ABS codes.
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
// Key codes (a subset of linux/input-event-codes.h).
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;
const KEY_CAPSLOCK: u16 = 58;
const BTN_LEFT: u16 = 272;
const BTN_RIGHT: u16 = 273;
const BTN_MIDDLE: u16 = 274;

// Device-config query (VIRTIO 1.1 §5.7.4): select=EV_BITS(5),
// subsel=event type. Bit BTN_LEFT set ⇒ pointer/tablet, else keyboard.
const CFG_SELECT_KEYS: u8 = 5;
const CFG_SUBSEL_KEY: u8 = 1;

// ─── Queue memory (caller-provided statics) ────────────────────────────────

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

    fn base(&self) -> usize {
        self.raw.get() as *const u8 as usize
    }
}

struct EvBufs {
    bufs: UnsafeCell<[[u8; EV_SIZE]; RING_CAP]>,
    last_used: UnsafeCell<u16>,
    avail_idx: UnsafeCell<u16>,
}

// SAFETY: same single-core rationale.
unsafe impl Sync for EvBufs {}

impl EvBufs {
    const fn new() -> Self {
        Self {
            bufs: UnsafeCell::new([[0u8; EV_SIZE]; RING_CAP]),
            last_used: UnsafeCell::new(0),
            avail_idx: UnsafeCell::new(0),
        }
    }
}

static KBD_REGION: VqRegion = VqRegion::new();
static KBD_BUFS: EvBufs = EvBufs::new();
static PTR_REGION: VqRegion = VqRegion::new();
static PTR_BUFS: EvBufs = EvBufs::new();

// ─── Driver ────────────────────────────────────────────────────────────────

/// One virtio input device (keyboard or pointer).
pub struct VirtioInput {
    transport: VirtioPci,
    size: u16,
    region: &'static VqRegion,
    bufs: &'static EvBufs,
    is_pointer: bool,
    shift: u8,
    caps: bool,
    rel_x: i32,
    rel_y: i32,
    abs_x: Option<i32>,
    abs_y: Option<i32>,
    /// Last ABS-bitmap size seen (bring-up diagnostics).
    pub dbg_abs_size: u8,
    /// Last KEY-bitmap size seen (bring-up diagnostics).
    pub dbg_key_size: u8,
}

impl VirtioInput {
    /// Probe `id` (already matched as an input device), set up the event
    /// queue from the `kbd` (or pointer) statics, and return the driver
    /// plus the next free MMIO window address. `kbd` selects which static
    /// ring pair to use. Failure carries a static stage tag.
    pub fn init(id: &PciId, window: usize, kbd: bool) -> Result<(Self, usize), &'static str> {
        let (transport, next) = VirtioPci::init(id, window)?;
        let (region, bufs): (&'static VqRegion, &'static EvBufs) = if kbd {
            (&KBD_REGION, &KBD_BUFS)
        } else {
            (&PTR_REGION, &PTR_BUFS)
        };
        let desc = region.base() as u64;
        let avail = (region.base() + RING_CAP * 16) as u64;
        let used = (region.base() + 4096) as u64;
        let size = unsafe { transport.setup_queue(EVENTQ, desc, avail, used) };
        if size == 0 {
            return Err("no-queue");
        }
        if size as usize > RING_CAP {
            return Err("ring-cap");
        }
        let mut this = Self {
            transport,
            size,
            region,
            bufs,
            is_pointer: false,
            shift: 0,
            caps: false,
            rel_x: 0,
            rel_y: 0,
            abs_x: None,
            abs_y: None,
            dbg_abs_size: 0,
            dbg_key_size: 0,
        };
        unsafe {
            this.post_all();
            this.transport.driver_ok();
            this.transport.notify(EVENTQ);
        }
        this.is_pointer = unsafe { this.query_pointer() };
        Ok((this, next))
    }

    /// Post every descriptor as device-writable.
    unsafe fn post_all(&mut self) {
        let base = self.region.base();
        // Zero avail header first.
        core::ptr::write_volatile((base + RING_CAP * 16) as *mut u16, 0);
        core::ptr::write_volatile((base + RING_CAP * 16 + 2) as *mut u16, 0);
        for i in 0..self.size as usize {
            let d = (base + i * 16) as *mut VirtqDesc;
            let buf = (*self.bufs.bufs.get())[i].as_ptr() as u64;
            (*d).addr = buf;
            (*d).len = EV_SIZE as u32;
            (*d).flags = VIRTQ_DESC_F_WRITE;
            (*d).next = 0;
            core::ptr::write_volatile((base + RING_CAP * 16 + 4 + i * 2) as *mut u16, i as u16);
        }
        fence(Ordering::Release);
        core::ptr::write_volatile((base + RING_CAP * 16 + 2) as *mut u16, self.size);
        *self.bufs.avail_idx.get() = self.size;
        *self.bufs.last_used.get() = 0;
    }

    /// Query event-bits: pointer devices expose absolute axes (tablet)
    /// or button bits (mouse). Keyboards have neither.
    ///
    /// NOTE: `&mut self` — records raw bitmap sizes for diagnostics.
    unsafe fn query_pointer(&mut self) -> bool {
        if self.transport.device_cfg == 0 {
            return false;
        }
        // ABS bitmap nonempty (tablet axes)?
        if self.ev_bitmap_nonempty(3) {
            return true;
        }
        // BTN_LEFT (272) in the key bitmap (mouse buttons)?
        let cfg = self.transport.device_cfg;
        core::ptr::write_volatile(cfg as *mut u8, CFG_SELECT_KEYS);
        core::ptr::write_volatile((cfg + 1) as *mut u8, CFG_SUBSEL_KEY);
        fence(Ordering::SeqCst);
        let size = core::ptr::read_volatile((cfg + 2) as *const u8) as usize;
        self.dbg_key_size = size as u8;
        if size > 34 {
            return core::ptr::read_volatile((cfg + 8 + 34) as *const u8) & 0x01 != 0;
        }
        false
    }

    /// True when any bit in the event-type bitmap `ev` is set.
    unsafe fn ev_bitmap_nonempty(&mut self, ev: u8) -> bool {
        let cfg = self.transport.device_cfg;
        core::ptr::write_volatile(cfg as *mut u8, CFG_SELECT_KEYS);
        core::ptr::write_volatile((cfg + 1) as *mut u8, ev);
        fence(Ordering::SeqCst);
        let size = core::ptr::read_volatile((cfg + 2) as *const u8) as usize;
        self.dbg_abs_size = size as u8;
        let size = size.min(128);
        for i in 0..size {
            if core::ptr::read_volatile((cfg + 8 + i) as *const u8) != 0 {
                return true;
            }
        }
        false
    }

    fn used_idx(&self) -> u16 {
        unsafe { core::ptr::read_volatile((self.region.base() + 4096 + 2) as *const u16) }
    }

    /// Queue positions for diagnostics: `(avail_idx_posted, used_idx_seen)`.
    pub fn queue_pos(&self) -> (u16, u16) {
        unsafe {
            let avail = *self.bufs.avail_idx.get();
            (avail, self.used_idx())
        }
    }

    /// Drain completed events into `out`. Returns the count. Recycled
    /// descriptors are reposted and the queue kicked when any were taken.
    pub fn poll_into(&mut self, out: &mut [InputEvent]) -> usize {
        let mut n = 0;
        unsafe {
            fence(Ordering::Acquire);
            let mut last = *self.bufs.last_used.get();
            let used = self.used_idx();
            while last != used && n < out.len() {
                let slot = last as usize % self.size as usize;
                let desc_id = core::ptr::read_volatile(
                    (self.region.base() + 4096 + 4 + slot * 8) as *const u32,
                ) as usize;
                if desc_id < self.size as usize {
                    let ev = &(*self.bufs.bufs.get())[desc_id];
                    let typ = u16::from_le_bytes([ev[0], ev[1]]);
                    let code = u16::from_le_bytes([ev[2], ev[3]]);
                    let value = u32::from_le_bytes([ev[4], ev[5], ev[6], ev[7]]) as i32;
                    if let Some(e) = self.decode(typ, code, value) {
                        out[n] = e;
                        n += 1;
                    }
                    // SYN completions flush pointer motion below.
                    if typ == EV_SYN && code == SYN_REPORT {
                        if let Some(e) = self.flush_motion() {
                            if n < out.len() {
                                out[n] = e;
                                n += 1;
                            }
                        }
                    }
                    // Repost.
                    let avail = *self.bufs.avail_idx.get();
                    core::ptr::write_volatile(
                        (self.region.base()
                            + RING_CAP * 16
                            + 4
                            + (avail as usize % self.size as usize) * 2)
                            as *mut u16,
                        desc_id as u16,
                    );
                    fence(Ordering::Release);
                    *self.bufs.avail_idx.get() = avail.wrapping_add(1);
                }
                last = last.wrapping_add(1);
                fence(Ordering::Acquire);
            }
            *self.bufs.last_used.get() = last;
            if n > 0 {
                // Publish recycled descriptors before kicking.
                let avail = *self.bufs.avail_idx.get();
                core::ptr::write_volatile(
                    (self.region.base() + RING_CAP * 16 + 2) as *mut u16,
                    avail,
                );
                fence(Ordering::Release);
                self.transport.notify(EVENTQ);
            }
        }
        n
    }

    fn flush_motion(&mut self) -> Option<InputEvent> {
        let mut dx = self.rel_x;
        let mut dy = self.rel_y;
        self.rel_x = 0;
        self.rel_y = 0;
        // Absolute deltas were folded into rel in decode; emit if nonzero.
        dx = dx.clamp(-32767, 32767);
        dy = dy.clamp(-32767, 32767);
        if dx != 0 || dy != 0 {
            Some(InputEvent::MouseMove {
                dx: dx as i16,
                dy: dy as i16,
            })
        } else {
            None
        }
    }

    fn decode(&mut self, typ: u16, code: u16, value: i32) -> Option<InputEvent> {
        match typ {
            EV_KEY => {
                if code == KEY_LEFTSHIFT {
                    self.set_shift(0, value);
                    return None;
                }
                if code == KEY_RIGHTSHIFT {
                    self.set_shift(1, value);
                    return None;
                }
                if code == KEY_CAPSLOCK && value == 1 {
                    self.caps = !self.caps;
                    return None;
                }
                if code == BTN_LEFT || code == BTN_RIGHT || code == BTN_MIDDLE {
                    if value > 1 {
                        return None;
                    }
                    let button = match code {
                        BTN_LEFT => 0,
                        BTN_RIGHT => 1,
                        _ => 2,
                    };
                    return Some(InputEvent::MouseButton {
                        button,
                        pressed: value == 1,
                    });
                }
                let pressed = value == 1 || value == 2;
                let released = value == 0;
                if !pressed && !released {
                    return None;
                }
                key_ascii(code, self.shift != 0, self.caps).map(|b| {
                    if pressed {
                        InputEvent::KeyPress(b)
                    } else {
                        InputEvent::KeyRelease(b)
                    }
                })
            }
            EV_REL => {
                match code {
                    REL_X => self.rel_x = self.rel_x.saturating_add(value),
                    REL_Y => self.rel_y = self.rel_y.saturating_add(value),
                    _ => {}
                }
                None
            }
            EV_ABS => {
                match code {
                    ABS_X => {
                        if let Some(px) = self.abs_x {
                            self.rel_x = self.rel_x.saturating_add(value - px);
                        }
                        self.abs_x = Some(value);
                    }
                    ABS_Y => {
                        if let Some(py) = self.abs_y {
                            self.rel_y = self.rel_y.saturating_add(value - py);
                        }
                        self.abs_y = Some(value);
                    }
                    _ => {}
                }
                None
            }
            _ => None,
        }
    }

    fn set_shift(&mut self, side: u8, value: i32) {
        if value == 1 {
            self.shift |= 1 << side;
        } else if value == 0 {
            self.shift &= !(1 << side);
        }
    }

    /// Register-block bases for boot diagnostics.
    pub fn debug_bases(&self) -> (usize, usize, u32, usize, usize) {
        self.transport.debug_bases()
    }

    /// Programmed queue addresses + device size (guest/device agreement).
    pub fn debug_queue(&self) -> (u64, u64, u64, u16) {
        unsafe { self.transport.debug_queue(EVENTQ) }
    }

    /// Region base this driver polled (must equal programmed desc addr).
    pub fn region_base(&self) -> usize {
        self.region.base()
    }

    /// True when the device exposes pointer buttons (tablet/mouse).
    pub fn is_pointer(&self) -> bool {
        self.is_pointer
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VirtqDesc {
    addr: u64,
    len: u32,
    flags: u16,
    next: u16,
}

const VIRTQ_DESC_F_WRITE: u16 = 2;

/// Linux key code → ASCII. Caps Lock flips case for letters only.
fn key_ascii(code: u16, shift: bool, caps: bool) -> Option<u8> {
    let (lo, hi) = match code {
        2 => (b'1', b'!'),
        3 => (b'2', b'@'),
        4 => (b'3', b'#'),
        5 => (b'4', b'$'),
        6 => (b'5', b'%'),
        7 => (b'6', b'^'),
        8 => (b'7', b'&'),
        9 => (b'8', b'*'),
        10 => (b'9', b'('),
        11 => (b'0', b')'),
        12 => (b'-', b'_'),
        13 => (b'=', b'+'),
        16 => (b'q', b'Q'),
        17 => (b'w', b'W'),
        18 => (b'e', b'E'),
        19 => (b'r', b'R'),
        20 => (b't', b'T'),
        21 => (b'y', b'Y'),
        22 => (b'u', b'U'),
        23 => (b'i', b'I'),
        24 => (b'o', b'O'),
        25 => (b'p', b'P'),
        26 => (b'[', b'{'),
        27 => (b']', b'}'),
        30 => (b'a', b'A'),
        31 => (b's', b'S'),
        32 => (b'd', b'D'),
        33 => (b'f', b'F'),
        34 => (b'g', b'G'),
        35 => (b'h', b'H'),
        36 => (b'j', b'J'),
        37 => (b'k', b'K'),
        38 => (b'l', b'L'),
        39 => (b';', b':'),
        40 => (b'\'', b'"'),
        41 => (b'`', b'~'),
        43 => (b'\\', b'|'),
        44 => (b'z', b'Z'),
        45 => (b'x', b'X'),
        46 => (b'c', b'C'),
        47 => (b'v', b'V'),
        48 => (b'b', b'B'),
        49 => (b'n', b'N'),
        50 => (b'm', b'M'),
        51 => (b',', b'<'),
        52 => (b'.', b'>'),
        53 => (b'/', b'?'),
        57 => (b' ', b' '),
        28 => (0x0D, 0x0D),
        14 => (0x7F, 0x7F),
        15 => (0x09, 0x09),
        1 => (0x1B, 0x1B),
        103 => (crate::uart_kbd::KEY_UP, crate::uart_kbd::KEY_UP),
        108 => (crate::uart_kbd::KEY_DOWN, crate::uart_kbd::KEY_DOWN),
        106 => (crate::uart_kbd::KEY_RIGHT, crate::uart_kbd::KEY_RIGHT),
        105 => (crate::uart_kbd::KEY_LEFT, crate::uart_kbd::KEY_LEFT),
        111 => (0x7F, 0x7F),
        _ => return None,
    };
    let mut shifted = shift;
    if caps && lo.is_ascii_lowercase() {
        shifted = !shifted;
    }
    Some(if shifted { hi } else { lo })
}

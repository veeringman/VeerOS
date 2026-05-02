//! WiFi OS adapter — bridges Espressif blob callbacks to VeerOS.
//!
//! The WiFi firmware blobs call through a `wifi_osi_funcs_t` function
//! pointer table. This module provides all the implementations and
//! populates the global table that the blobs reference.

use core::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use core::ptr;

use esp_wifi_sys::include::{
    wifi_osi_funcs_t, ESP_WIFI_OS_ADAPTER_MAGIC, ESP_WIFI_OS_ADAPTER_VERSION,
};

use super::heap;
use super::modem;
use super::systimer;

// ═══════════════════════════════════════════════════════════════════════════
// Globals required by the blobs
// ═══════════════════════════════════════════════════════════════════════════

/// The OSI function table — referenced by the blobs via `g_wifi_osi_funcs`.
#[no_mangle]
pub static mut g_wifi_osi_funcs: wifi_osi_funcs_t = OSI_FUNCS;

/// WiFi event base string.
#[no_mangle]
pub static mut WIFI_EVENT: *const c_char = b"WIFI_EVENT\0".as_ptr() as *const c_char;

// g_log_level and g_misc_nvs are provided by libcore.a — do not duplicate.

/// WiFi feature caps (WPA3-SAE + Enterprise).
pub const WIFI_FEATURE_CAPS: u64 = (1 << 0) | (1 << 7);

#[no_mangle]
pub static mut g_wifi_feature_caps: u64 = WIFI_FEATURE_CAPS;

// ═══════════════════════════════════════════════════════════════════════════
// Timer compatibility (ets_timer)
// ═══════════════════════════════════════════════════════════════════════════

/// Software timer entry — the blobs pass around `ets_timer` pointers.
/// We store the callback and manage firing from our poll loop.
const MAX_TIMERS: usize = 8;

struct SoftTimer {
    active: bool,
    repeat: bool,
    callback: Option<unsafe extern "C" fn(*mut c_void)>,
    arg: *mut c_void,
    period_us: u64,
    next_fire: u64,
}

static mut TIMERS: [SoftTimer; MAX_TIMERS] = [const {
    SoftTimer {
        active: false,
        repeat: false,
        callback: None,
        arg: ptr::null_mut(),
        period_us: 0,
        next_fire: 0,
    }
}; MAX_TIMERS];

/// Timer handle map — maps blob timer pointer to our index.
/// We use the lower bits of the pointer as a simple hash.
static mut TIMER_MAP: [(usize, usize); MAX_TIMERS] = [(0, usize::MAX); MAX_TIMERS];

fn timer_slot_for(ptimer: *mut c_void) -> usize {
    let key = ptimer as usize;
    // Search for existing or empty slot.
    for i in 0..MAX_TIMERS {
        let (k, idx) = unsafe { TIMER_MAP[i] };
        if k == key && idx != usize::MAX {
            return idx;
        }
    }
    // Allocate new.
    for i in 0..MAX_TIMERS {
        let (_, idx) = unsafe { TIMER_MAP[i] };
        if idx == usize::MAX {
            // Find a free timer.
            for t in 0..MAX_TIMERS {
                if !unsafe { TIMERS[t].active } && unsafe { TIMERS[t].callback.is_none() } {
                    unsafe {
                        TIMER_MAP[i] = (key, t);
                    }
                    return t;
                }
            }
        }
    }
    0 // fallback
}

/// Poll all software timers — call from the WiFi task loop.
pub fn poll_timers() {
    let now = systimer::now_us();
    for i in 0..MAX_TIMERS {
        let t = unsafe { &mut TIMERS[i] };
        if t.active && now >= t.next_fire {
            if let Some(cb) = t.callback {
                unsafe { cb(t.arg) };
            }
            if t.repeat {
                t.next_fire = now + t.period_us;
            } else {
                t.active = false;
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Simple queue (ring buffer) for the blobs
// ═══════════════════════════════════════════════════════════════════════════

const MAX_QUEUES: usize = 8;
const QUEUE_BUF_SIZE: usize = 2048;

/// Layout: first field is `pc_head` (pointer to data) so that
/// `*(u32*)(queue_handle + 0)` returns a valid non-null address,
/// matching what the WiFi blobs expect from FreeRTOS QueueHandle_t.
#[repr(C)]
struct SimpleQueue {
    pc_head: *mut u8,     // offset 0 — mimics FreeRTOS pcHead
    pc_write_to: *mut u8, // offset 4 — mimics FreeRTOS pcWriteTo
    item_size: usize,
    capacity: usize,
    head: usize,
    tail: usize,
    count: usize,
    in_use: bool,
    data: [u8; QUEUE_BUF_SIZE],
}

static mut QUEUES: [SimpleQueue; MAX_QUEUES] = [const {
    SimpleQueue {
        pc_head: ptr::null_mut(),
        pc_write_to: ptr::null_mut(),
        item_size: 0,
        capacity: 0,
        head: 0,
        tail: 0,
        count: 0,
        in_use: false,
        data: [0; QUEUE_BUF_SIZE],
    }
}; MAX_QUEUES];

fn alloc_queue(queue_len: usize, item_size: usize) -> *mut c_void {
    for i in 0..MAX_QUEUES {
        let q = unsafe { &mut QUEUES[i] };
        if !q.in_use && queue_len * item_size <= QUEUE_BUF_SIZE {
            q.item_size = item_size;
            q.capacity = queue_len;
            q.head = 0;
            q.tail = 0;
            q.count = 0;
            q.in_use = true;
            // Set pc_head to point at data so blob deref at offset 0 works
            q.pc_head = q.data.as_mut_ptr();
            q.pc_write_to = q.data.as_mut_ptr();
            return q as *mut SimpleQueue as *mut c_void;
        }
    }
    ptr::null_mut()
}

fn get_queue(handle: *mut c_void) -> Option<&'static mut SimpleQueue> {
    if handle.is_null() {
        return None;
    }
    let addr = handle as usize;
    let base = unsafe { QUEUES.as_ptr() } as usize;
    let end = base + core::mem::size_of::<[SimpleQueue; MAX_QUEUES]>();
    if addr >= base && addr < end {
        // Find which SimpleQueue this address falls within
        let offset = addr - base;
        let queue_size = core::mem::size_of::<SimpleQueue>();
        let idx = offset / queue_size;
        let q = unsafe { &mut QUEUES[idx] };
        if q.in_use {
            return Some(q);
        }
    }
    None
}

fn queue_send_impl(q: &mut SimpleQueue, item: *const c_void) -> bool {
    if q.count >= q.capacity || item.is_null() {
        return false;
    }
    let offset = q.tail * q.item_size;
    unsafe {
        ptr::copy_nonoverlapping(
            item as *const u8,
            q.data.as_mut_ptr().add(offset),
            q.item_size,
        );
    }
    q.tail = (q.tail + 1) % q.capacity;
    q.count += 1;
    true
}

fn queue_recv_impl(q: &mut SimpleQueue, item: *mut c_void) -> bool {
    if q.count == 0 || item.is_null() {
        return false;
    }
    let offset = q.head * q.item_size;
    unsafe {
        ptr::copy_nonoverlapping(q.data.as_ptr().add(offset), item as *mut u8, q.item_size);
    }
    q.head = (q.head + 1) % q.capacity;
    q.count -= 1;
    true
}

// ═══════════════════════════════════════════════════════════════════════════
// Simple semaphore/mutex (spinlock-based, interrupts disabled)
// ═══════════════════════════════════════════════════════════════════════════

const MAX_SEMS: usize = 16;

/// Layout matches what blobs expect: returned handle is a real pointer.
/// Blobs may dereference semaphore handles like FreeRTOS QueueHandle_t.
///
/// When `recursive` is true, the semaphore acts as a recursive mutex:
/// the same task can lock it multiple times without deadlocking and must
/// unlock the same number of times before it becomes available.
#[repr(C)]
struct SimpleSem {
    count: i32,
    max: i32,
    in_use: bool,
    recursive: bool,
    owner: usize,   // task id that holds the recursive mutex (0 = unowned)
    recursion: u32, // nesting depth for recursive mutex
}

static mut SEMS: [SimpleSem; MAX_SEMS] = [const {
    SimpleSem {
        count: 0,
        max: 0,
        in_use: false,
        recursive: false,
        owner: 0,
        recursion: 0,
    }
}; MAX_SEMS];

static mut BLOB_TASK_ENTRIES: u32 = 0;
static mut SEM_ALLOC_COUNT: u32 = 0;
static mut SEM_TAKE_OK_COUNT: u32 = 0;
static mut SEM_TAKE_BLOCK_COUNT: u32 = 0;
static mut SEM_GIVE_COUNT: u32 = 0;
static mut QUEUE_SEND_COUNT: u32 = 0;
static mut QUEUE_RECV_OK_COUNT: u32 = 0;
static mut QUEUE_RECV_BLOCK_COUNT: u32 = 0;
static mut QUEUE_SEND_LAST_TAG: u32 = 0;
static mut QUEUE_RECV_LAST_TAG: u32 = 0;
static mut QUEUE_MAX_DEPTH: u32 = 0;
static mut EVENT_POST_COUNT: u32 = 0;
static mut EVENT_POST_LAST_ID: i32 = -1;

fn alloc_sem(max: u32, init: u32) -> *mut c_void {
    for i in 0..MAX_SEMS {
        let s = unsafe { &mut SEMS[i] };
        if !s.in_use {
            s.max = max as i32;
            s.count = init as i32;
            s.in_use = true;
            s.recursive = false;
            s.owner = 0;
            s.recursion = 0;
            unsafe {
                SEM_ALLOC_COUNT = SEM_ALLOC_COUNT.wrapping_add(1);
            }
            return s as *mut SimpleSem as *mut c_void;
        }
    }
    ptr::null_mut()
}

fn alloc_recursive_mutex() -> *mut c_void {
    for i in 0..MAX_SEMS {
        let s = unsafe { &mut SEMS[i] };
        if !s.in_use {
            s.max = 1;
            s.count = 1; // unlocked
            s.in_use = true;
            s.recursive = true;
            s.owner = 0;
            s.recursion = 0;
            unsafe {
                SEM_ALLOC_COUNT = SEM_ALLOC_COUNT.wrapping_add(1);
            }
            return s as *mut SimpleSem as *mut c_void;
        }
    }
    ptr::null_mut()
}

fn get_sem(handle: *mut c_void) -> Option<&'static mut SimpleSem> {
    let ptr = handle as *mut SimpleSem;
    if ptr.is_null() {
        return None;
    }
    let base = unsafe { SEMS.as_ptr() } as usize;
    let end = base + core::mem::size_of::<[SimpleSem; MAX_SEMS]>();
    let addr = ptr as usize;
    if addr >= base && addr < end {
        let s = unsafe { &mut *ptr };
        if s.in_use {
            return Some(s);
        }
    }
    None
}

// ═══════════════════════════════════════════════════════════════════════════
// Event groups (simple bitmask)
// ═══════════════════════════════════════════════════════════════════════════

const MAX_EVENT_GROUPS: usize = 4;

#[repr(C)]
struct SimpleEventGroup {
    bits: u32,
    in_use: bool,
}

static mut EVENT_GROUPS: [SimpleEventGroup; MAX_EVENT_GROUPS] = [const {
    SimpleEventGroup {
        bits: 0,
        in_use: false,
    }
}; MAX_EVENT_GROUPS];

// ═══════════════════════════════════════════════════════════════════════════
// Interrupt management
// ═══════════════════════════════════════════════════════════════════════════

/// Disable global interrupts and return previous mstatus.
fn interrupts_disable() -> u32 {
    let mstatus: u32;
    unsafe {
        core::arch::asm!("csrrci {}, mstatus, 0x8", out(reg) mstatus);
    }
    mstatus
}

/// Restore mstatus (re-enable interrupts if they were enabled).
fn interrupts_restore(mstatus: u32) {
    if mstatus & 0x8 != 0 {
        unsafe {
            core::arch::asm!("csrsi mstatus, 0x8");
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RNG peripheral
// ═══════════════════════════════════════════════════════════════════════════

/// ESP32-C6 RNG register.
const RNG_DATA_REG: usize = 0x6002_60B0;

fn hw_random() -> u32 {
    unsafe { core::ptr::read_volatile(RNG_DATA_REG as *const u32) }
}

// ═══════════════════════════════════════════════════════════════════════════
// PHY state tracking & calibration data
// ═══════════════════════════════════════════════════════════════════════════

static mut PHY_ENABLED: bool = false;
static mut PHY_CALIBRATED: bool = false;

/// PHY init data for ESP32-C6 (128 bytes, matches esp-wifi defaults at 20 dBm max TX power).
static PHY_INIT_DATA: esp_wifi_sys::include::esp_phy_init_data_t =
    esp_wifi_sys::include::esp_phy_init_data_t {
        params: [
            0x01, 0x00, 0x50, 0x50, 0x50, 0x50, 0x50, 0x4c, 0x4c, 0x4c, 0x4c, 0x48, 0x28, 0x28,
            0x28, 0x28, 0x4c, 0x4c, 0x4c, 0x4c, 0x48, 0x28, 0x28, 0x28, 0x28, 0x00, 0x00, 0x00,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x9b, 0x00,
        ],
    };

/// Calibration data buffer (zeroed = partial cal from scratch).
static mut PHY_CAL_DATA: esp_wifi_sys::include::esp_phy_calibration_data_t =
    esp_wifi_sys::include::esp_phy_calibration_data_t {
        version: [0; 4],
        mac: [0; 6],
        opaque: [0; 1894],
    };

/// PHY digital register backup memory (21 × 4 = 84 bytes).
const SOC_PHY_DIG_REGS_MEM_SIZE: usize = 21 * 4;
static mut SOC_PHY_DIG_REGS_MEM: [u8; SOC_PHY_DIG_REGS_MEM_SIZE] = [0u8; SOC_PHY_DIG_REGS_MEM_SIZE];

// ═══════════════════════════════════════════════════════════════════════════
// WiFi interrupt ISR storage & dispatch
// ═══════════════════════════════════════════════════════════════════════════
// WiFi interrupt routing & dispatch
// ═══════════════════════════════════════════════════════════════════════════

// INTMATRIX base on ESP32-C6.
const INTMATRIX_BASE: usize = 0x6001_0000;
const INTPRI_BASE: usize = 0x600C_5000;
const INTC_CORE0_INTR_STATUS0: usize = 0x134;
const INTC_CORE0_INTR_STATUS1: usize = 0x138;
const INTC_CPU_INT_ENABLE: usize = 0x00;
const INTC_CPU_INT_TYPE: usize = 0x04;
const INTC_CPU_INT_PRI_BASE: usize = 0x0C;
const INTC_CPU_INT_THRESH: usize = 0x8C;
const INTC_CPU_INT_CLEAR: usize = 0xA8;
// PLIC base on ESP32-C6.
const PLIC_BASE: usize = 0x2000_1000;
const PLIC_MXINT_ENABLE: usize = 0x00;
const PLIC_MXINT_TYPE: usize = 0x04;
const PLIC_MXINT_CLEAR: usize = 0x08;
const PLIC_EMIP_STATUS: usize = 0x0C;
const PLIC_MXINT_PRI_BASE: usize = 0x10;
const PLIC_MXINT_THRESH: usize = 0x90;
const MIE_MEIE_BIT: u32 = 1 << 11;

// WiFi interrupt source numbers (ESP32-C6 peripheral sources).
const WIFI_MAC_INTR_SOURCE: usize = 0;
const WIFI_MAC_NMI_SOURCE: usize = 1;
const WIFI_PWR_INTR_SOURCE: usize = 2;
const WIFI_BB_INTR_SOURCE: usize = 3;
const MODEM_PERI_TIMEOUT_INTR_SOURCE: usize = 34;

// Keep WiFi routed to CPU INT 1 (as requested by blob set_intr).
const WIFI_CPU_INT: usize = 1;
const WIFI_CPU_INT_ALT: usize = 2;
const WIFI_INACTIVE_CPU_INT: usize = 31; // sink for unwanted sources

/// Single stored WiFi ISR handler — both WIFI_MAC and WIFI_PWR call it.
/// Matches esp-wifi's ISR_INTERRUPT_1 approach.
static mut WIFI_ISR_FN: *mut c_void = core::ptr::null_mut();
static mut WIFI_ISR_ARG: *mut c_void = core::ptr::null_mut();

/// Diagnostic counters.
static mut WIFI_ISR_COUNT: u32 = 0;
static mut BLOB_TASKS_SPAWNED: u32 = 0;
static mut ISR_REGISTERED: bool = false;

/// Track what the blob passes to set_intr.
static mut BLOB_SET_INTR_SOURCE: [u32; 4] = [0xFFFF; 4];
static mut BLOB_SET_INTR_NUM: [u32; 4] = [0xFFFF; 4];
static mut BLOB_SET_INTR_COUNT: usize = 0;
static mut BLOB_INTS_ON_MASK: u32 = 0;
static mut BLOB_INTS_ON_COUNT: u32 = 0;
static mut BLOB_INTS_OFF_COUNT: u32 = 0;
static mut BLOB_INTS_SHADOW_MASK: u32 = 0;

/// Return diagnostic counters.
pub fn wifi_diag() -> (
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
    u32,
) {
    unsafe {
        let plic_en = core::ptr::read_volatile((PLIC_BASE + PLIC_MXINT_ENABLE) as *const u32);
        let emip = core::ptr::read_volatile((PLIC_BASE + PLIC_EMIP_STATUS) as *const u32);
        let thresh = core::ptr::read_volatile((PLIC_BASE + PLIC_MXINT_THRESH) as *const u32);
        let mie: u32;
        core::arch::asm!("csrr {0}, mie", out(reg) mie, options(nomem, nostack));
        let mip: u32;
        core::arch::asm!("csrr {0}, mip", out(reg) mip, options(nomem, nostack));
        let intc_en = core::ptr::read_volatile((INTPRI_BASE + INTC_CPU_INT_ENABLE) as *const u32);
        let intc_type = core::ptr::read_volatile((INTPRI_BASE + INTC_CPU_INT_TYPE) as *const u32);
        let intc_pri_wifi = core::ptr::read_volatile(
            (INTPRI_BASE + INTC_CPU_INT_PRI_BASE + WIFI_CPU_INT * 4) as *const u32,
        );
        let intc_pri_wifi_alt = core::ptr::read_volatile(
            (INTPRI_BASE + INTC_CPU_INT_PRI_BASE + WIFI_CPU_INT_ALT * 4) as *const u32,
        );
        let intc_thresh =
            core::ptr::read_volatile((INTPRI_BASE + INTC_CPU_INT_THRESH) as *const u32);
        let map_mac =
            core::ptr::read_volatile((INTMATRIX_BASE + WIFI_MAC_INTR_SOURCE * 4) as *const u32);
        let map_mac_nmi =
            core::ptr::read_volatile((INTMATRIX_BASE + WIFI_MAC_NMI_SOURCE * 4) as *const u32);
        let map_pwr =
            core::ptr::read_volatile((INTMATRIX_BASE + WIFI_PWR_INTR_SOURCE * 4) as *const u32);
        let st0 =
            core::ptr::read_volatile((INTMATRIX_BASE + INTC_CORE0_INTR_STATUS0) as *const u32);
        let st1 =
            core::ptr::read_volatile((INTMATRIX_BASE + INTC_CORE0_INTR_STATUS1) as *const u32);
        // Compact blob set_intr calls: pack first 2 as (src0 << 16 | num0) and (src1 << 16 | num1)
        let si0 = if BLOB_SET_INTR_COUNT > 0 {
            (BLOB_SET_INTR_SOURCE[0] << 16) | BLOB_SET_INTR_NUM[0]
        } else {
            0xDEAD
        };
        let si1 = if BLOB_SET_INTR_COUNT > 1 {
            (BLOB_SET_INTR_SOURCE[1] << 16) | BLOB_SET_INTR_NUM[1]
        } else {
            0xDEAD
        };
        (
            WIFI_ISR_COUNT,
            plic_en,
            emip,
            thresh,
            mie,
            mip,
            intc_en,
            intc_type,
            intc_pri_wifi,
            intc_pri_wifi_alt,
            intc_thresh,
            map_mac,
            map_mac_nmi,
            map_pwr,
            st0,
            st1,
            si0,
            si1,
            BLOB_INTS_ON_MASK,
            BLOB_INTS_ON_COUNT,
            BLOB_INTS_OFF_COUNT,
            BLOB_INTS_SHADOW_MASK,
            BLOB_TASKS_SPAWNED,
            BLOB_TASK_ENTRIES,
            SEM_ALLOC_COUNT,
            SEM_TAKE_OK_COUNT,
            SEM_TAKE_BLOCK_COUNT,
            SEM_GIVE_COUNT,
            QUEUE_SEND_COUNT,
            QUEUE_RECV_OK_COUNT | (QUEUE_RECV_BLOCK_COUNT << 16),
            QUEUE_SEND_LAST_TAG,
            QUEUE_RECV_LAST_TAG,
            QUEUE_MAX_DEPTH,
            EVENT_POST_COUNT,
            EVENT_POST_LAST_ID as u32,
        )
    }
}

/// Set up WiFi interrupt routing in INTMATRIX and PLIC.
/// Call once during WiFi init, before the blob registers its ISR.
pub fn setup_wifi_interrupts() {
    unsafe {
        // Route WIFI_MAC/PWR/NMI through INTMATRIX.
        core::ptr::write_volatile(
            (INTMATRIX_BASE + WIFI_MAC_INTR_SOURCE * 4) as *mut u32,
            WIFI_CPU_INT as u32,
        );
        core::ptr::write_volatile(
            (INTMATRIX_BASE + WIFI_PWR_INTR_SOURCE * 4) as *mut u32,
            WIFI_CPU_INT as u32,
        );
        core::ptr::write_volatile(
            (INTMATRIX_BASE + WIFI_MAC_NMI_SOURCE * 4) as *mut u32,
            WIFI_CPU_INT as u32,
        );
        core::ptr::write_volatile(
            (INTMATRIX_BASE + WIFI_BB_INTR_SOURCE * 4) as *mut u32,
            WIFI_INACTIVE_CPU_INT as u32,
        );
        core::ptr::write_volatile(
            (INTMATRIX_BASE + MODEM_PERI_TIMEOUT_INTR_SOURCE * 4) as *mut u32,
            WIFI_INACTIVE_CPU_INT as u32,
        );

        // Set priority 1 for WIFI_CPU_INT.
        core::ptr::write_volatile(
            (PLIC_BASE + PLIC_MXINT_PRI_BASE + WIFI_CPU_INT * 4) as *mut u32,
            1,
        );

        // Set level-triggered for WiFi line (clear type bit).
        let typ = core::ptr::read_volatile((PLIC_BASE + PLIC_MXINT_TYPE) as *const u32);
        core::ptr::write_volatile(
            (PLIC_BASE + PLIC_MXINT_TYPE) as *mut u32,
            typ & !(1 << WIFI_CPU_INT),
        );

        // Clear any stale pending state.
        core::ptr::write_volatile(
            (PLIC_BASE + PLIC_MXINT_CLEAR) as *mut u32,
            1 << WIFI_CPU_INT,
        );

        // Configure INTPRI side for the same CPU interrupt lines.
        let intc_type = core::ptr::read_volatile((INTPRI_BASE + INTC_CPU_INT_TYPE) as *const u32);
        core::ptr::write_volatile(
            (INTPRI_BASE + INTC_CPU_INT_TYPE) as *mut u32,
            intc_type & !(1 << WIFI_CPU_INT),
        );
        core::ptr::write_volatile(
            (INTPRI_BASE + INTC_CPU_INT_PRI_BASE + WIFI_CPU_INT * 4) as *mut u32,
            1,
        );
        core::ptr::write_volatile((INTPRI_BASE + INTC_CPU_INT_THRESH) as *mut u32, 0);
        core::ptr::write_volatile(
            (INTPRI_BASE + INTC_CPU_INT_CLEAR) as *mut u32,
            1 << WIFI_CPU_INT,
        );

        // NOTE: Do NOT enable CPU INT 2/3 in PLIC or mie here.
        // They will be enabled in set_isr() once the blob registers its handler.
        // Enabling level-triggered interrupts before an ISR is registered
        // causes an infinite re-entry loop.

        // Keep INTPRI CPU-int gate closed until ISR is registered.
        let intc_en = core::ptr::read_volatile((INTPRI_BASE + INTC_CPU_INT_ENABLE) as *const u32);
        core::ptr::write_volatile(
            (INTPRI_BASE + INTC_CPU_INT_ENABLE) as *mut u32,
            intc_en & !(1 << WIFI_CPU_INT),
        );
        BLOB_INTS_SHADOW_MASK &= !(1 << WIFI_CPU_INT);
    }
}

/// Dispatch a CPU interrupt line from the trap handler.
/// Returns `true` if it was a WiFi interrupt and was handled.
#[inline(never)]
pub fn wifi_isr_dispatch(cpu_int: usize) -> bool {
    if cpu_int != WIFI_CPU_INT && cpu_int != WIFI_CPU_INT_ALT && cpu_int != 11 {
        return false;
    }
    unsafe {
        WIFI_ISR_COUNT += 1;
    }
    let fnc = unsafe { WIFI_ISR_FN };
    if fnc.is_null() {
        return false;
    }
    let handler: unsafe extern "C" fn(*mut c_void) = unsafe { core::mem::transmute(fnc) };
    let arg = unsafe { WIFI_ISR_ARG };
    unsafe { handler(arg) };
    true
}

// ═══════════════════════════════════════════════════════════════════════════
// OSI function implementations
// ═══════════════════════════════════════════════════════════════════════════

unsafe extern "C" fn env_is_chip() -> bool {
    true
}

unsafe extern "C" fn set_intr(_cpu_no: i32, intr_source: u32, _intr_num: u32, _intr_prio: i32) {
    // Record what the blob requests.
    let idx = unsafe { BLOB_SET_INTR_COUNT };
    if idx < 4 {
        unsafe {
            BLOB_SET_INTR_SOURCE[idx] = intr_source;
            BLOB_SET_INTR_NUM[idx] = _intr_num;
            BLOB_SET_INTR_COUNT = idx + 1;
        }
    }
    // Match upstream esp-wifi C6 behavior: this callback is informational.
    // Routing is configured explicitly in `setup_wifi_interrupts()`.
}

unsafe extern "C" fn clear_intr(_intr_source: u32, _intr_num: u32) {
    // Clear pending for our WiFi CPU interrupt lines.
    core::ptr::write_volatile(
        (PLIC_BASE + PLIC_MXINT_CLEAR) as *mut u32,
        1 << WIFI_CPU_INT,
    );
    core::ptr::write_volatile(
        (INTPRI_BASE + INTC_CPU_INT_CLEAR) as *mut u32,
        1 << WIFI_CPU_INT,
    );
}

unsafe extern "C" fn set_isr(_n: i32, f: *mut c_void, arg: *mut c_void) {
    // Store the WiFi ISR handler. Both WIFI_MAC and WIFI_PWR use the same one.
    WIFI_ISR_FN = f;
    WIFI_ISR_ARG = arg;
    ISR_REGISTERED = !f.is_null();

    // Now that an ISR is registered, enable WiFi interrupt.
    unsafe {
        // Clear stale pending first.
        core::ptr::write_volatile(
            (PLIC_BASE + PLIC_MXINT_CLEAR) as *mut u32,
            1 << WIFI_CPU_INT,
        );
        // Enable WiFi CPU interrupt lines in PLIC.
        let en = core::ptr::read_volatile((PLIC_BASE + PLIC_MXINT_ENABLE) as *const u32);
        core::ptr::write_volatile(
            (PLIC_BASE + PLIC_MXINT_ENABLE) as *mut u32,
            en | (1 << WIFI_CPU_INT),
        );
        // Enable WiFi CPU interrupt lines in INTPRI gate.
        let intc_en = core::ptr::read_volatile((INTPRI_BASE + INTC_CPU_INT_ENABLE) as *const u32);
        core::ptr::write_volatile(
            (INTPRI_BASE + INTC_CPU_INT_ENABLE) as *mut u32,
            intc_en | (1 << WIFI_CPU_INT),
        );
        let intc_type = core::ptr::read_volatile((INTPRI_BASE + INTC_CPU_INT_TYPE) as *const u32);
        core::ptr::write_volatile(
            (INTPRI_BASE + INTC_CPU_INT_TYPE) as *mut u32,
            intc_type & !(1 << WIFI_CPU_INT),
        );
        core::ptr::write_volatile(
            (INTPRI_BASE + INTC_CPU_INT_PRI_BASE + WIFI_CPU_INT * 4) as *mut u32,
            1,
        );
        core::ptr::write_volatile((INTPRI_BASE + INTC_CPU_INT_THRESH) as *mut u32, 0);
        core::ptr::write_volatile(
            (INTPRI_BASE + INTC_CPU_INT_CLEAR) as *mut u32,
            1 << WIFI_CPU_INT,
        );
        BLOB_INTS_SHADOW_MASK |= 1 << WIFI_CPU_INT;
        // Enable in mie CSR.
        core::arch::asm!(
            "csrs mie, {0}",
            in(reg) (1u32 << WIFI_CPU_INT) | MIE_MEIE_BIT,
            options(nomem, nostack),
        );
    }
}

unsafe extern "C" fn ints_on(mask: u32) {
    unsafe {
        BLOB_INTS_ON_MASK |= mask;
    }
    unsafe {
        BLOB_INTS_ON_COUNT = BLOB_INTS_ON_COUNT.wrapping_add(1);
        BLOB_INTS_SHADOW_MASK |= mask;
    }
    // Match upstream: only touch INTPRI cpu_int_enable.
    // Do NOT toggle PLIC or mie — those are set once during set_isr().
    let intc_en = core::ptr::read_volatile((INTPRI_BASE + INTC_CPU_INT_ENABLE) as *const u32);
    core::ptr::write_volatile(
        (INTPRI_BASE + INTC_CPU_INT_ENABLE) as *mut u32,
        intc_en | mask,
    );
}
unsafe extern "C" fn ints_off(mask: u32) {
    unsafe {
        BLOB_INTS_OFF_COUNT = BLOB_INTS_OFF_COUNT.wrapping_add(1);
        BLOB_INTS_SHADOW_MASK &= !mask;
    }
    // Match upstream: only touch INTPRI cpu_int_enable.
    // Do NOT toggle PLIC or mie — those are set once during set_isr().
    let intc_en = core::ptr::read_volatile((INTPRI_BASE + INTC_CPU_INT_ENABLE) as *const u32);
    core::ptr::write_volatile(
        (INTPRI_BASE + INTC_CPU_INT_ENABLE) as *mut u32,
        intc_en & !mask,
    );
}

unsafe extern "C" fn is_from_isr() -> bool {
    false
}

unsafe extern "C" fn spin_lock_create() -> *mut c_void {
    // Return a non-null dummy. We use single-core interrupt-disable for synchronization.
    1 as *mut c_void
}

unsafe extern "C" fn spin_lock_delete(_lock: *mut c_void) {}

unsafe extern "C" fn wifi_int_disable(_wifi_int_mux: *mut c_void) -> u32 {
    interrupts_disable()
}

unsafe extern "C" fn wifi_int_restore(_wifi_int_mux: *mut c_void, tmp: u32) {
    interrupts_restore(tmp);
}

unsafe extern "C" fn task_yield_from_isr() {}

unsafe extern "C" fn semphr_create(max: u32, init: u32) -> *mut c_void {
    let h = alloc_sem(max, init);
    h
}

unsafe extern "C" fn semphr_delete(semphr: *mut c_void) {
    if let Some(s) = get_sem(semphr) {
        s.in_use = false;
    }
}

unsafe extern "C" fn semphr_take(semphr: *mut c_void, block_time_tick: u32) -> i32 {
    if let Some(s) = get_sem(semphr) {
        if s.count > 0 {
            s.count -= 1;
            SEM_TAKE_OK_COUNT = SEM_TAKE_OK_COUNT.wrapping_add(1);
            return 1;
        }
        // Blocking path: yield and retry.
        // portMAX_DELAY (0xFFFFFFFF) → block indefinitely.
        if block_time_tick > 0 {
            SEM_TAKE_BLOCK_COUNT = SEM_TAKE_BLOCK_COUNT.wrapping_add(1);
            let max_iters: u32 = if block_time_tick >= 0xFFFF_FF00 {
                u32::MAX
            } else {
                block_time_tick.max(500)
            };
            let mut i: u32 = 0;
            loop {
                poll_timers();
                if let Some(s) = get_sem(semphr) {
                    if s.count > 0 {
                        s.count -= 1;
                        SEM_TAKE_OK_COUNT = SEM_TAKE_OK_COUNT.wrapping_add(1);
                        return 1;
                    }
                }
                sys_sleep(1);
                i = i.wrapping_add(1);
                if max_iters != u32::MAX && i >= max_iters {
                    break;
                }
            }
        }
    }
    0
}

unsafe extern "C" fn semphr_give(semphr: *mut c_void) -> i32 {
    if let Some(s) = get_sem(semphr) {
        if s.count < s.max {
            s.count += 1;
        }
        SEM_GIVE_COUNT = SEM_GIVE_COUNT.wrapping_add(1);
        return 1;
    }
    0
}

unsafe extern "C" fn wifi_thread_semphr_get() -> *mut c_void {
    // Return a dummy semaphore.
    static mut THREAD_SEM: usize = 0;
    if unsafe { THREAD_SEM == 0 } {
        let s = alloc_sem(1, 0);
        unsafe { THREAD_SEM = s as usize };
        s
    } else {
        unsafe { THREAD_SEM as *mut c_void }
    }
}

unsafe extern "C" fn mutex_create() -> *mut c_void {
    alloc_sem(1, 1)
}

unsafe extern "C" fn recursive_mutex_create() -> *mut c_void {
    alloc_recursive_mutex()
}

unsafe extern "C" fn mutex_delete(mutex: *mut c_void) {
    semphr_delete(mutex);
}

unsafe extern "C" fn mutex_lock(mutex: *mut c_void) -> i32 {
    if let Some(s) = get_sem(mutex) {
        if s.recursive {
            let me = sys_task_id();
            if s.owner == me && s.recursion > 0 {
                // Already own it — just bump recursion count.
                s.recursion += 1;
                return 1;
            }
            // Not owned (or owned by another task) — take the underlying semaphore.
            // We must drop the borrow before calling semphr_take, which re-borrows.
        }
    }
    let ret = semphr_take(mutex, u32::MAX);
    if ret == 1 {
        if let Some(s) = get_sem(mutex) {
            if s.recursive {
                s.owner = sys_task_id();
                s.recursion = 1;
            }
        }
    }
    ret
}

unsafe extern "C" fn mutex_unlock(mutex: *mut c_void) -> i32 {
    if let Some(s) = get_sem(mutex) {
        if s.recursive {
            if s.recursion > 1 {
                s.recursion -= 1;
                return 1;
            }
            // Last unlock — release ownership before giving the semaphore.
            s.owner = 0;
            s.recursion = 0;
        }
    }
    semphr_give(mutex)
}

unsafe extern "C" fn queue_create(queue_len: u32, item_size: u32) -> *mut c_void {
    let h = alloc_queue(queue_len as usize, item_size as usize);
    h
}

unsafe extern "C" fn queue_delete(queue: *mut c_void) {
    if let Some(q) = get_queue(queue) {
        q.in_use = false;
    }
}

unsafe extern "C" fn queue_send(
    queue: *mut c_void,
    item: *mut c_void,
    _block_time_tick: u32,
) -> i32 {
    if let Some(q) = get_queue(queue) {
        let ok = queue_send_impl(q, item as *const c_void);
        if ok {
            QUEUE_SEND_COUNT = QUEUE_SEND_COUNT.wrapping_add(1);
            if !item.is_null() && q.item_size >= 4 {
                QUEUE_SEND_LAST_TAG = core::ptr::read_unaligned(item as *const u32);
            }
            let depth = q.count as u32;
            if depth > QUEUE_MAX_DEPTH {
                QUEUE_MAX_DEPTH = depth;
            }
        }
        return ok as i32;
    }
    0
}

unsafe extern "C" fn queue_send_from_isr(
    queue: *mut c_void,
    item: *mut c_void,
    _hptw: *mut c_void,
) -> i32 {
    queue_send(queue, item, 0)
}

unsafe extern "C" fn queue_send_to_back(
    queue: *mut c_void,
    item: *mut c_void,
    block_time_tick: u32,
) -> i32 {
    queue_send(queue, item, block_time_tick)
}

unsafe extern "C" fn queue_send_to_front(
    queue: *mut c_void,
    item: *mut c_void,
    _block_time_tick: u32,
) -> i32 {
    // Simplified: same as send_to_back (FIFO).
    if let Some(q) = get_queue(queue) {
        return queue_send_impl(q, item as *const c_void) as i32;
    }
    0
}

unsafe extern "C" fn queue_recv(
    queue: *mut c_void,
    item: *mut c_void,
    block_time_tick: u32,
) -> i32 {
    if let Some(q) = get_queue(queue) {
        if queue_recv_impl(q, item) {
            QUEUE_RECV_OK_COUNT = QUEUE_RECV_OK_COUNT.wrapping_add(1);
            if !item.is_null() && q.item_size >= 4 {
                QUEUE_RECV_LAST_TAG = core::ptr::read_unaligned(item as *const u32);
            }
            return 1;
        }
    }
    // Queue empty — if blocking requested, sleep/yield and retry.
    // portMAX_DELAY (0xFFFFFFFF) → block indefinitely.
    if block_time_tick > 0 {
        QUEUE_RECV_BLOCK_COUNT = QUEUE_RECV_BLOCK_COUNT.wrapping_add(1);
        let max_iters: u32 = if block_time_tick >= 0xFFFF_FF00 {
            u32::MAX
        } else {
            block_time_tick.max(500)
        };
        let mut i: u32 = 0;
        loop {
            poll_timers();
            if let Some(q) = get_queue(queue) {
                if queue_recv_impl(q, item) {
                    QUEUE_RECV_OK_COUNT = QUEUE_RECV_OK_COUNT.wrapping_add(1);
                    if !item.is_null() && q.item_size >= 4 {
                        QUEUE_RECV_LAST_TAG = core::ptr::read_unaligned(item as *const u32);
                    }
                    return 1;
                }
            }
            sys_sleep(1);
            i = i.wrapping_add(1);
            if max_iters != u32::MAX && i >= max_iters {
                break;
            }
        }
    }
    0
}

unsafe extern "C" fn queue_msg_waiting(queue: *mut c_void) -> u32 {
    if let Some(q) = get_queue(queue) {
        return q.count as u32;
    }
    0
}

unsafe extern "C" fn event_group_create() -> *mut c_void {
    for i in 0..MAX_EVENT_GROUPS {
        let eg = unsafe { &mut EVENT_GROUPS[i] };
        if !eg.in_use {
            eg.bits = 0;
            eg.in_use = true;
            return eg as *mut SimpleEventGroup as *mut c_void;
        }
    }
    ptr::null_mut()
}

fn get_event_group(handle: *mut c_void) -> Option<&'static mut SimpleEventGroup> {
    let ptr = handle as *mut SimpleEventGroup;
    if ptr.is_null() {
        return None;
    }
    let base = unsafe { EVENT_GROUPS.as_ptr() } as usize;
    let end = base + core::mem::size_of::<[SimpleEventGroup; MAX_EVENT_GROUPS]>();
    let addr = ptr as usize;
    if addr >= base && addr < end {
        let eg = unsafe { &mut *ptr };
        if eg.in_use {
            return Some(eg);
        }
    }
    None
}

unsafe extern "C" fn event_group_delete(event: *mut c_void) {
    if let Some(eg) = get_event_group(event) {
        eg.in_use = false;
    }
}

unsafe extern "C" fn event_group_set_bits(event: *mut c_void, bits: u32) -> u32 {
    if let Some(eg) = get_event_group(event) {
        eg.bits |= bits;
        return eg.bits;
    }
    0
}

unsafe extern "C" fn event_group_clear_bits(event: *mut c_void, bits: u32) -> u32 {
    if let Some(eg) = get_event_group(event) {
        let old = eg.bits;
        eg.bits &= !bits;
        return old;
    }
    0
}

unsafe extern "C" fn event_group_wait_bits(
    event: *mut c_void,
    bits_to_wait_for: u32,
    clear_on_exit: c_int,
    wait_for_all_bits: c_int,
    block_time_tick: u32,
) -> u32 {
    if let Some(eg) = get_event_group(event) {
        let max_iters: u32 = if block_time_tick >= 0xFFFF_FF00 {
            u32::MAX
        } else {
            block_time_tick.max(500)
        };
        let mut i: u32 = 0;
        loop {
            let val = eg.bits;
            let matched = if wait_for_all_bits != 0 {
                val & bits_to_wait_for == bits_to_wait_for
            } else {
                val & bits_to_wait_for != 0
            };
            if matched {
                if clear_on_exit != 0 {
                    eg.bits &= !bits_to_wait_for;
                }
                return val;
            }
            poll_timers();
            sys_sleep(1);
            i = i.wrapping_add(1);
            if max_iters != u32::MAX && i >= max_iters {
                break;
            }
        }
        return eg.bits;
    }
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// Blob task integration (ppTask, etc.) via kernel scheduler tasks
// ═══════════════════════════════════════════════════════════════════════════

const MAX_BLOB_TASKS: usize = 4;
const BLOB_TASK_STACK_SIZE: usize = 8192;

struct BlobTask {
    func: unsafe extern "C" fn(*mut c_void),
    param: *mut c_void,
    stack: [u8; BLOB_TASK_STACK_SIZE],
    active: bool,
    task_id: usize,
}

static mut BLOB_TASKS: [BlobTask; MAX_BLOB_TASKS] = [const {
    BlobTask {
        func: dummy_task_fn,
        param: ptr::null_mut(),
        stack: [0; BLOB_TASK_STACK_SIZE],
        active: false,
        task_id: usize::MAX,
    }
}; MAX_BLOB_TASKS];

/// Which blob task slot is currently executing on this core.
static mut CURRENT_BLOB_TASK: usize = usize::MAX;

unsafe extern "C" fn dummy_task_fn(_: *mut c_void) {}

#[cfg(target_arch = "riscv32")]
#[inline(always)]
fn sys_spawn(entry: usize, stack_top: usize, stack_bottom: usize, priority: usize) -> usize {
    let ret: usize;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") entry,
            in("a1") stack_top,
            in("a2") stack_bottom,
            in("a3") priority,
            in("a7") 0x05usize, // SYS_SPAWN
            lateout("a0") ret,
            options(nostack),
        );
    }
    ret
}

#[cfg(not(target_arch = "riscv32"))]
#[inline(always)]
fn sys_spawn(_entry: usize, _stack_top: usize, _stack_bottom: usize, _priority: usize) -> usize {
    usize::MAX
}

#[cfg(target_arch = "riscv32")]
#[inline(always)]
fn sys_yield() {
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") 0x00usize, // SYS_YIELD
            options(nostack),
        );
    }
}

#[cfg(not(target_arch = "riscv32"))]
#[inline(always)]
fn sys_yield() {}

#[cfg(target_arch = "riscv32")]
#[inline(always)]
fn sys_sleep(ticks: usize) {
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") ticks,
            in("a7") 0x31usize, // SYS_SLEEP
            options(nostack),
        );
    }
}

#[cfg(not(target_arch = "riscv32"))]
#[inline(always)]
fn sys_sleep(_ticks: usize) {}

#[cfg(target_arch = "riscv32")]
#[inline(always)]
fn sys_task_id() -> usize {
    let ret: usize;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") 0x02usize, // SYS_TASK_ID
            lateout("a0") ret,
            options(nostack),
        );
    }
    ret
}

#[cfg(not(target_arch = "riscv32"))]
#[inline(always)]
fn sys_task_id() -> usize {
    1
}

unsafe fn run_blob_task(idx: usize) -> ! {
    unsafe { CURRENT_BLOB_TASK = idx };
    unsafe {
        BLOB_TASK_ENTRIES = BLOB_TASK_ENTRIES.wrapping_add(1);
    }
    let (func, param) = unsafe {
        let t = &BLOB_TASKS[idx];
        (t.func, t.param)
    };
    unsafe { func(param) };
    unsafe { BLOB_TASKS[idx].active = false };
    loop {
        sys_sleep(1000);
    }
}

unsafe extern "C" fn blob_task_entry0() -> ! {
    unsafe { run_blob_task(0) }
}
unsafe extern "C" fn blob_task_entry1() -> ! {
    unsafe { run_blob_task(1) }
}
unsafe extern "C" fn blob_task_entry2() -> ! {
    unsafe { run_blob_task(2) }
}
unsafe extern "C" fn blob_task_entry3() -> ! {
    unsafe { run_blob_task(3) }
}

#[inline(always)]
fn blob_task_entry_for(idx: usize) -> usize {
    match idx {
        0 => blob_task_entry0 as *const () as usize,
        1 => blob_task_entry1 as *const () as usize,
        2 => blob_task_entry2 as *const () as usize,
        _ => blob_task_entry3 as *const () as usize,
    }
}

/// Blob tasks now run as normal scheduler tasks, so polling is unnecessary.
pub fn poll_blob_task(_idx: usize, _max_us: u64) -> bool {
    false
}

/// Yield to the scheduler so other tasks can run.
pub fn yield_to_scheduler() {
    sys_sleep(10);
}

fn blob_task_yield() {
    sys_yield();
}

unsafe extern "C" fn task_create_pinned_to_core(
    task_func: *mut c_void,
    _name: *const c_char,
    _stack_depth: u32,
    param: *mut c_void,
    _prio: u32,
    task_handle: *mut c_void,
    _core_id: u32,
) -> i32 {
    // Register blob task in a slot and spawn a real scheduler task for it.
    for i in 0..MAX_BLOB_TASKS {
        let t = unsafe { &mut BLOB_TASKS[i] };
        if !t.active {
            t.func = unsafe { core::mem::transmute(task_func) };
            t.param = param;
            t.active = true;
            let sb = t.stack.as_ptr() as usize;
            let st = sb + BLOB_TASK_STACK_SIZE;
            let child = sys_spawn(blob_task_entry_for(i), st, sb, _prio as usize);
            t.task_id = child;
            if !task_handle.is_null() {
                unsafe { *(task_handle as *mut usize) = child };
            }
            if child == usize::MAX {
                t.active = false;
                t.task_id = usize::MAX;
                return 0;
            }
            unsafe {
                BLOB_TASKS_SPAWNED += 1;
            }
            return 1;
        }
    }
    // All slots full.
    if !task_handle.is_null() {
        unsafe { *(task_handle as *mut usize) = usize::MAX };
    }
    0
}

unsafe extern "C" fn task_create(
    task_func: *mut c_void,
    name: *const c_char,
    stack_depth: u32,
    param: *mut c_void,
    prio: u32,
    task_handle: *mut c_void,
) -> i32 {
    task_create_pinned_to_core(task_func, name, stack_depth, param, prio, task_handle, 0)
}

unsafe extern "C" fn task_delete(_task_handle: *mut c_void) {
    // No task-kill syscall yet.
}

unsafe extern "C" fn task_delay(tick: u32) {
    // 1 tick = 10 ms in this port.
    let t = (tick as usize).max(1);
    sys_sleep(t);
}

unsafe extern "C" fn task_ms_to_tick(ms: u32) -> i32 {
    // 1 tick = 10ms.
    (ms / 10).max(1) as i32
}

unsafe extern "C" fn task_get_current_task() -> *mut c_void {
    sys_task_id() as *mut c_void
}

unsafe extern "C" fn task_get_max_priority() -> i32 {
    25
}

unsafe extern "C" fn osi_malloc(size: usize) -> *mut c_void {
    let p = unsafe { super::heap::malloc(size) };
    p
}

unsafe extern "C" fn osi_free(p: *mut c_void) {
    unsafe { super::heap::free(p) }
}

unsafe extern "C" fn event_post(
    _event_base: *const c_char,
    event_id: i32,
    _event_data: *mut c_void,
    _event_data_size: usize,
    _ticks_to_wait: u32,
) -> i32 {
    EVENT_POST_COUNT = EVENT_POST_COUNT.wrapping_add(1);
    EVENT_POST_LAST_ID = event_id;
    0
}

unsafe extern "C" fn get_free_heap_size() -> u32 {
    heap::free_heap_size() as u32
}

unsafe extern "C" fn rand() -> u32 {
    hw_random()
}

unsafe extern "C" fn dport_access_stall_other_cpu_start_wrap() {}
unsafe extern "C" fn dport_access_stall_other_cpu_end_wrap() {}
unsafe extern "C" fn wifi_apb80m_request() {}
unsafe extern "C" fn wifi_apb80m_release() {}

/// Run PHY calibration early (before `esp_wifi_init_internal`), matching
/// esp-wifi's init order.  When the blob later calls `phy_enable()` from
/// ppTask it will find `PHY_CALIBRATED == true` and take the fast
/// `phy_wakeup_init()` path instead of blocking in `register_chipv7_phy`.
pub fn early_phy_init() {
    unsafe {
        if PHY_CALIBRATED {
            return;
        }

        // Step 0: Populate g_phyFuns — the PHY blob's ROM function table.
        extern "C" {
            fn phy_get_romfuncs() -> *const u32;
            static mut g_phyFuns: *const u32;
        }
        g_phyFuns = phy_get_romfuncs();

        // Step 1: Full radio clock infrastructure (PMU, state maps, LP clocks)
        modem::init_radio_clocks();
        // Step 2: Legacy modem clock gate enables
        modem::enable_all_clocks();
        // Step 3: I2C master clock at 160 MHz (needed for RF register writes)
        modem::enable_phy_clock();
        // Step 4: Enable WiFi BB/FE/MAC clocks *before* PHY cal
        modem::enable_wifi_clocks();
        // Step 5: Run PHY calibration blob (partial cal, matching esp-wifi)
        esp_wifi_sys::include::register_chipv7_phy(
            &PHY_INIT_DATA as *const esp_wifi_sys::include::esp_phy_init_data_t,
            &raw mut PHY_CAL_DATA,
            esp_wifi_sys::include::esp_phy_calibration_mode_t_PHY_RF_CAL_NONE,
        );
        PHY_CALIBRATED = true;
        PHY_ENABLED = true;
    }
}

unsafe extern "C" fn phy_disable() {
    unsafe { PHY_ENABLED = false };
}

unsafe extern "C" fn phy_enable() {
    if !unsafe { PHY_ENABLED } {
        modem::init_radio_clocks();
        modem::enable_all_clocks();
        modem::enable_phy_clock();

        if !unsafe { PHY_CALIBRATED } {
            // First-time PHY init: run full calibration via blob.
            unsafe {
                esp_wifi_sys::include::register_chipv7_phy(
                    &PHY_INIT_DATA as *const esp_wifi_sys::include::esp_phy_init_data_t,
                    &raw mut PHY_CAL_DATA,
                    esp_wifi_sys::include::esp_phy_calibration_mode_t_PHY_RF_CAL_FULL,
                );
                PHY_CALIBRATED = true;
            }
        } else {
            // Already calibrated: quick wake-up.
            unsafe { esp_wifi_sys::include::phy_wakeup_init() };
        }

        unsafe { PHY_ENABLED = true };
    }
}

unsafe extern "C" fn phy_update_country_info(_country: *const c_char) -> c_int {
    0
}

unsafe extern "C" fn read_mac(mac: *mut u8, _type_: c_uint) -> c_int {
    let efuse_mac = modem::read_efuse_mac();
    for i in 0..6 {
        unsafe { *mac.add(i) = efuse_mac[i] };
    }
    0
}

unsafe extern "C" fn ets_timer_arm(timer: *mut c_void, ms: u32, repeat: bool) {
    let slot = timer_slot_for(timer);
    let t = unsafe { &mut TIMERS[slot] };
    t.period_us = ms as u64 * 1000;
    t.repeat = repeat;
    t.next_fire = systimer::now_us() + t.period_us;
    t.active = true;
}

unsafe extern "C" fn ets_timer_disarm(timer: *mut c_void) {
    let slot = timer_slot_for(timer);
    unsafe { TIMERS[slot].active = false };
}

unsafe extern "C" fn ets_timer_done(ptimer: *mut c_void) {
    ets_timer_disarm(ptimer);
}

unsafe extern "C" fn ets_timer_setfn(
    ptimer: *mut c_void,
    pfunction: *mut c_void,
    parg: *mut c_void,
) {
    let slot = timer_slot_for(ptimer);
    let t = unsafe { &mut TIMERS[slot] };
    t.callback = Some(unsafe { core::mem::transmute(pfunction) });
    t.arg = parg;
}

unsafe extern "C" fn ets_timer_arm_us(ptimer: *mut c_void, us: u32, repeat: bool) {
    let slot = timer_slot_for(ptimer);
    let t = unsafe { &mut TIMERS[slot] };
    t.period_us = us as u64;
    t.repeat = repeat;
    t.next_fire = systimer::now_us() + t.period_us;
    t.active = true;
}

unsafe extern "C" fn wifi_reset_mac() {
    modem::reset_all_modems();
}

unsafe extern "C" fn wifi_clock_enable() {
    modem::enable_all_clocks();
}

unsafe extern "C" fn wifi_clock_disable() {
    // Don't actually disable — other radios might need clocks.
}

unsafe extern "C" fn wifi_rtc_enable_iso() {}
unsafe extern "C" fn wifi_rtc_disable_iso() {}

unsafe extern "C" fn esp_timer_get_time() -> i64 {
    systimer::now_us() as i64
}

// NVS functions — all stubs (no persistent storage).
unsafe extern "C" fn nvs_set_i8(_handle: u32, _key: *const c_char, _value: i8) -> c_int {
    -1
}
unsafe extern "C" fn nvs_get_i8(_handle: u32, _key: *const c_char, _out: *mut i8) -> c_int {
    -1
}
unsafe extern "C" fn nvs_set_u8(_handle: u32, _key: *const c_char, _value: u8) -> c_int {
    -1
}
unsafe extern "C" fn nvs_get_u8(_handle: u32, _key: *const c_char, _out: *mut u8) -> c_int {
    -1
}
unsafe extern "C" fn nvs_set_u16(_handle: u32, _key: *const c_char, _value: u16) -> c_int {
    -1
}
unsafe extern "C" fn nvs_get_u16(_handle: u32, _key: *const c_char, _out: *mut u16) -> c_int {
    -1
}
unsafe extern "C" fn nvs_open(_name: *const c_char, _mode: c_uint, _handle: *mut u32) -> c_int {
    -1
}
unsafe extern "C" fn nvs_close(_handle: u32) {}
unsafe extern "C" fn nvs_commit(_handle: u32) -> c_int {
    -1
}
unsafe extern "C" fn nvs_set_blob(
    _handle: u32,
    _key: *const c_char,
    _value: *const c_void,
    _len: usize,
) -> c_int {
    -1
}
unsafe extern "C" fn nvs_get_blob(
    _handle: u32,
    _key: *const c_char,
    _out: *mut c_void,
    _len: *mut usize,
) -> c_int {
    -1
}
unsafe extern "C" fn nvs_erase_key(_handle: u32, _key: *const c_char) -> c_int {
    -1
}

unsafe extern "C" fn get_random(buf: *mut u8, len: usize) -> c_int {
    for i in 0..len {
        if i % 4 == 0 {
            let r = hw_random();
            let bytes = r.to_le_bytes();
            let remaining = len - i;
            let n = remaining.min(4);
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), buf.add(i), n) };
        }
    }
    0
}

unsafe extern "C" fn get_time(t: *mut c_void) -> c_int {
    // t points to a timeval {tv_sec: u64, tv_usec: u32}
    let us = systimer::now_us();
    if !t.is_null() {
        let tv = t as *mut [u32; 3]; // simplified: {sec_lo, sec_hi, usec}
        unsafe {
            let secs = us / 1_000_000;
            (*tv)[0] = secs as u32;
            (*tv)[1] = (secs >> 32) as u32;
            (*tv)[2] = (us % 1_000_000) as u32;
        }
    }
    0
}

unsafe extern "C" fn random() -> c_ulong {
    hw_random() as c_ulong
}

unsafe extern "C" fn slowclk_cal_get() -> u32 {
    // Return approximate RTC slow clock calibration value.
    // 150kHz RC oscillator → ~6667 ns per cycle.
    6667
}

// Log functions — no-op (disabled via None in OSI table).

unsafe extern "C" fn log_timestamp() -> u32 {
    (systimer::now_us() / 1000) as u32
}

// Memory allocation wrappers.
unsafe extern "C" fn malloc_internal(size: usize) -> *mut c_void {
    unsafe { super::heap::malloc(size) }
}

unsafe extern "C" fn realloc_internal(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { super::heap::realloc(ptr, size) }
}

unsafe extern "C" fn calloc_internal(n: usize, size: usize) -> *mut c_void {
    unsafe { super::heap::calloc(n, size) }
}

unsafe extern "C" fn zalloc_internal(size: usize) -> *mut c_void {
    unsafe { super::heap::calloc(1, size) }
}

unsafe extern "C" fn wifi_malloc(size: usize) -> *mut c_void {
    unsafe { super::heap::malloc(size) }
}

unsafe extern "C" fn wifi_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    unsafe { super::heap::realloc(ptr, size) }
}

unsafe extern "C" fn wifi_calloc(n: usize, size: usize) -> *mut c_void {
    unsafe { super::heap::calloc(n, size) }
}

unsafe extern "C" fn wifi_zalloc(size: usize) -> *mut c_void {
    unsafe { super::heap::calloc(1, size) }
}

unsafe extern "C" fn wifi_create_queue(queue_len: c_int, item_size: c_int) -> *mut c_void {
    let h = alloc_queue(queue_len as usize, item_size as usize);
    h
}

unsafe extern "C" fn wifi_delete_queue(queue: *mut c_void) {
    if let Some(q) = get_queue(queue) {
        q.in_use = false;
    }
}

// Coex functions — no-op (single radio, no coexistence needed).
unsafe extern "C" fn coex_init() -> c_int {
    0
}
unsafe extern "C" fn coex_deinit() {}
unsafe extern "C" fn coex_enable() -> c_int {
    0
}
unsafe extern "C" fn coex_disable() {}
unsafe extern "C" fn coex_status_get() -> u32 {
    0
}
unsafe extern "C" fn coex_wifi_request(_event: u32, _latency: u32, _duration: u32) -> c_int {
    0
}
unsafe extern "C" fn coex_wifi_release(_event: u32) -> c_int {
    0
}
unsafe extern "C" fn coex_wifi_channel_set(_primary: u8, _secondary: u8) -> c_int {
    0
}
unsafe extern "C" fn coex_event_duration_get(_event: u32, _duration: *mut u32) -> c_int {
    0
}
unsafe extern "C" fn coex_pti_get(_event: u32, _pti: *mut u8) -> c_int {
    0
}
unsafe extern "C" fn coex_schm_status_bit_clear(_type_: u32, _status: u32) {}
unsafe extern "C" fn coex_schm_status_bit_set(_type_: u32, _status: u32) {}
unsafe extern "C" fn coex_schm_interval_set(_interval: u32) -> c_int {
    0
}
unsafe extern "C" fn coex_schm_interval_get() -> u32 {
    0
}
unsafe extern "C" fn coex_schm_curr_period_get() -> u8 {
    0
}
unsafe extern "C" fn coex_schm_curr_phase_get() -> *mut c_void {
    ptr::null_mut()
}
unsafe extern "C" fn coex_schm_process_restart() -> c_int {
    0
}
unsafe extern "C" fn coex_schm_register_cb(
    _arg1: c_int,
    _cb: Option<unsafe extern "C" fn(c_int) -> c_int>,
) -> c_int {
    0
}
unsafe extern "C" fn coex_register_start_cb(_cb: Option<unsafe extern "C" fn() -> c_int>) -> c_int {
    0
}

unsafe extern "C" fn regdma_link_set_write_wait_content_dummy(
    _arg1: *mut c_void,
    _arg2: u32,
    _arg3: u32,
) {
}

unsafe extern "C" fn sleep_retention_find_link_by_id_dummy(_arg1: c_int) -> *mut c_void {
    ptr::null_mut()
}

unsafe extern "C" fn coex_schm_flexible_period_set(_arg1: u8) -> c_int {
    0
}
unsafe extern "C" fn coex_schm_flexible_period_get() -> u8 {
    0
}
unsafe extern "C" fn coex_schm_get_phase_by_idx(_arg1: c_int) -> *mut c_void {
    ptr::null_mut()
}

// ═══════════════════════════════════════════════════════════════════════════
// The global OSI function table
// ═══════════════════════════════════════════════════════════════════════════

const OSI_FUNCS: wifi_osi_funcs_t = wifi_osi_funcs_t {
    _version: ESP_WIFI_OS_ADAPTER_VERSION as i32,
    _env_is_chip: Some(env_is_chip),
    _set_intr: Some(set_intr),
    _clear_intr: Some(clear_intr),
    _set_isr: Some(set_isr),
    _ints_on: Some(ints_on),
    _ints_off: Some(ints_off),
    _is_from_isr: Some(is_from_isr),
    _spin_lock_create: Some(spin_lock_create),
    _spin_lock_delete: Some(spin_lock_delete),
    _wifi_int_disable: Some(wifi_int_disable),
    _wifi_int_restore: Some(wifi_int_restore),
    _task_yield_from_isr: Some(task_yield_from_isr),
    _semphr_create: Some(semphr_create),
    _semphr_delete: Some(semphr_delete),
    _semphr_take: Some(semphr_take),
    _semphr_give: Some(semphr_give),
    _wifi_thread_semphr_get: Some(wifi_thread_semphr_get),
    _mutex_create: Some(mutex_create),
    _recursive_mutex_create: Some(recursive_mutex_create),
    _mutex_delete: Some(mutex_delete),
    _mutex_lock: Some(mutex_lock),
    _mutex_unlock: Some(mutex_unlock),
    _queue_create: Some(queue_create),
    _queue_delete: Some(queue_delete),
    _queue_send: Some(queue_send),
    _queue_send_from_isr: Some(queue_send_from_isr),
    _queue_send_to_back: Some(queue_send_to_back),
    _queue_send_to_front: Some(queue_send_to_front),
    _queue_recv: Some(queue_recv),
    _queue_msg_waiting: Some(queue_msg_waiting),
    _event_group_create: Some(event_group_create),
    _event_group_delete: Some(event_group_delete),
    _event_group_set_bits: Some(event_group_set_bits),
    _event_group_clear_bits: Some(event_group_clear_bits),
    _event_group_wait_bits: Some(event_group_wait_bits),
    _task_create_pinned_to_core: Some(task_create_pinned_to_core),
    _task_create: Some(task_create),
    _task_delete: Some(task_delete),
    _task_delay: Some(task_delay),
    _task_ms_to_tick: Some(task_ms_to_tick),
    _task_get_current_task: Some(task_get_current_task),
    _task_get_max_priority: Some(task_get_max_priority),
    _malloc: Some(osi_malloc),
    _free: Some(osi_free),
    _event_post: Some(event_post),
    _get_free_heap_size: Some(get_free_heap_size),
    _rand: Some(rand),
    _dport_access_stall_other_cpu_start_wrap: Some(dport_access_stall_other_cpu_start_wrap),
    _dport_access_stall_other_cpu_end_wrap: Some(dport_access_stall_other_cpu_end_wrap),
    _wifi_apb80m_request: Some(wifi_apb80m_request),
    _wifi_apb80m_release: Some(wifi_apb80m_release),
    _phy_disable: Some(phy_disable),
    _phy_enable: Some(phy_enable),
    _phy_update_country_info: Some(phy_update_country_info),
    _read_mac: Some(read_mac),
    _timer_arm: Some(ets_timer_arm),
    _timer_disarm: Some(ets_timer_disarm),
    _timer_done: Some(ets_timer_done),
    _timer_setfn: Some(ets_timer_setfn),
    _timer_arm_us: Some(ets_timer_arm_us),
    _wifi_reset_mac: Some(wifi_reset_mac),
    _wifi_clock_enable: Some(wifi_clock_enable),
    _wifi_clock_disable: Some(wifi_clock_disable),
    _wifi_rtc_enable_iso: Some(wifi_rtc_enable_iso),
    _wifi_rtc_disable_iso: Some(wifi_rtc_disable_iso),
    _esp_timer_get_time: Some(esp_timer_get_time),
    _nvs_set_i8: Some(nvs_set_i8),
    _nvs_get_i8: Some(nvs_get_i8),
    _nvs_set_u8: Some(nvs_set_u8),
    _nvs_get_u8: Some(nvs_get_u8),
    _nvs_set_u16: Some(nvs_set_u16),
    _nvs_get_u16: Some(nvs_get_u16),
    _nvs_open: Some(nvs_open),
    _nvs_close: Some(nvs_close),
    _nvs_commit: Some(nvs_commit),
    _nvs_set_blob: Some(nvs_set_blob),
    _nvs_get_blob: Some(nvs_get_blob),
    _nvs_erase_key: Some(nvs_erase_key),
    _get_random: Some(get_random),
    _get_time: Some(get_time),
    _random: Some(random),
    _slowclk_cal_get: Some(slowclk_cal_get),
    _log_write: None,
    _log_writev: None,
    _log_timestamp: Some(log_timestamp),
    _malloc_internal: Some(malloc_internal),
    _realloc_internal: Some(realloc_internal),
    _calloc_internal: Some(calloc_internal),
    _zalloc_internal: Some(zalloc_internal),
    _wifi_malloc: Some(wifi_malloc),
    _wifi_realloc: Some(wifi_realloc),
    _wifi_calloc: Some(wifi_calloc),
    _wifi_zalloc: Some(wifi_zalloc),
    _wifi_create_queue: Some(wifi_create_queue),
    _wifi_delete_queue: Some(wifi_delete_queue),
    _coex_init: Some(coex_init),
    _coex_deinit: Some(coex_deinit),
    _coex_enable: Some(coex_enable),
    _coex_disable: Some(coex_disable),
    _coex_status_get: Some(coex_status_get),
    _coex_condition_set: None,
    _coex_wifi_request: Some(coex_wifi_request),
    _coex_wifi_release: Some(coex_wifi_release),
    _coex_wifi_channel_set: Some(coex_wifi_channel_set),
    _coex_event_duration_get: Some(coex_event_duration_get),
    _coex_pti_get: Some(coex_pti_get),
    _coex_schm_status_bit_clear: Some(coex_schm_status_bit_clear),
    _coex_schm_status_bit_set: Some(coex_schm_status_bit_set),
    _coex_schm_interval_set: Some(coex_schm_interval_set),
    _coex_schm_interval_get: Some(coex_schm_interval_get),
    _coex_schm_curr_period_get: Some(coex_schm_curr_period_get),
    _coex_schm_curr_phase_get: Some(coex_schm_curr_phase_get),
    _coex_schm_process_restart: Some(coex_schm_process_restart),
    _coex_schm_register_cb: Some(coex_schm_register_cb),
    _coex_register_start_cb: Some(coex_register_start_cb),
    #[cfg(feature = "c6")]
    _regdma_link_set_write_wait_content: Some(regdma_link_set_write_wait_content_dummy),
    #[cfg(feature = "c6")]
    _sleep_retention_find_link_by_id: Some(sleep_retention_find_link_by_id_dummy),
    _coex_schm_flexible_period_set: Some(coex_schm_flexible_period_set),
    _coex_schm_flexible_period_get: Some(coex_schm_flexible_period_get),
    _coex_schm_get_phase_by_idx: Some(coex_schm_get_phase_by_idx),
    _magic: ESP_WIFI_OS_ADAPTER_MAGIC as i32,
};

// ═══════════════════════════════════════════════════════════════════════════
// Additional C symbols required by the blobs
// ═══════════════════════════════════════════════════════════════════════════

/// `gettimeofday` — required by multiple blob libraries.
#[no_mangle]
pub unsafe extern "C" fn gettimeofday(tv: *mut c_void, _tz: *mut c_void) -> c_int {
    get_time(tv)
}

/// `esp_fill_random` — fill a buffer with hardware random bytes.
#[no_mangle]
pub unsafe extern "C" fn esp_fill_random(buf: *mut c_void, len: usize) {
    unsafe { get_random(buf as *mut u8, len) };
}

/// `vTaskDelay` — FreeRTOS delay function called by blobs.
#[no_mangle]
pub unsafe extern "C" fn vTaskDelay(ticks: u32) {
    task_delay(ticks);
}

// misc_nvs_init/deinit/restore are provided by libcore.a — do not duplicate.

/// PHY init/calibration stubs.
#[no_mangle]
pub unsafe extern "C" fn esp_phy_enable(_modem: u32) {}

#[no_mangle]
pub unsafe extern "C" fn esp_phy_disable(_modem: u32) {}

// register_chipv7_phy and phy_get_romfunc_addr are provided by libphy.a — do not duplicate.

#[no_mangle]
pub unsafe extern "C" fn esp_phy_erase_cal_data_in_nvs() -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn coex_bt_request(_event: u32, _latency: u32, _duration: u32) -> c_int {
    0
}

#[no_mangle]
pub unsafe extern "C" fn coex_bt_release(_event: u32) -> c_int {
    0
}

// ═══════════════════════════════════════════════════════════════════════════
// Non-ROM stubs required by the blob archives
// ═══════════════════════════════════════════════════════════════════════════

/// `__ffssi2` — find first set bit (1-indexed, 0 if none). GCC builtin.
#[no_mangle]
pub unsafe extern "C" fn __ffssi2(x: i32) -> i32 {
    if x == 0 {
        0
    } else {
        (x.trailing_zeros() + 1) as i32
    }
}

/// `esp_event_post` — post an event (stub: we don't use the event loop).
#[no_mangle]
pub unsafe extern "C" fn esp_event_post(
    _event_base: *const c_char,
    _event_id: i32,
    _event_data: *const c_void,
    _event_data_size: usize,
    _ticks_to_wait: u32,
) -> i32 {
    0 // ESP_OK
}

/// POSIX sleep stubs.
#[no_mangle]
pub unsafe extern "C" fn sleep(seconds: c_uint) -> c_uint {
    let us = (seconds as u64) * 1_000_000;
    let start = systimer::now_us();
    while systimer::now_us() - start < us {
        core::hint::spin_loop();
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn usleep(us: u32) -> c_int {
    let start = systimer::now_us();
    while systimer::now_us() - start < us as u64 {
        core::hint::spin_loop();
    }
    0
}

/// `strrchr` — find last occurrence of a character in a string.
#[no_mangle]
pub unsafe extern "C" fn strrchr(s: *const c_char, c: c_int) -> *const c_char {
    if s.is_null() {
        return ptr::null();
    }
    let mut last: *const c_char = ptr::null();
    let mut p = s;
    loop {
        let ch = unsafe { *p };
        if ch == c as c_char {
            last = p;
        }
        if ch == 0 {
            break;
        }
        p = unsafe { p.add(1) };
    }
    last
}

/// `strnlen` — length of string, at most maxlen.
#[no_mangle]
pub unsafe extern "C" fn strnlen(s: *const c_char, maxlen: usize) -> usize {
    if s.is_null() {
        return 0;
    }
    let mut i = 0usize;
    while i < maxlen && unsafe { *s.add(i) } != 0 {
        i += 1;
    }
    i
}

/// `putchar` — single character output (discard).
#[no_mangle]
pub unsafe extern "C" fn putchar(c: c_int) -> c_int {
    c
}

/// `rtc_clk_xtal_freq_get` — returns crystal frequency in MHz.
/// ESP32-C6 XIAO uses a 40 MHz crystal.
#[no_mangle]
pub unsafe extern "C" fn rtc_clk_xtal_freq_get() -> c_int {
    40
}

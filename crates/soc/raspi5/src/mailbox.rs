//! BCM2712 VideoCore mailbox interface.
//!
//! Provides communication with the VideoCore firmware via the property
//! tag protocol (channel 8). Used to allocate framebuffers, query board
//! info, set clock rates, etc.
//!
//! The mailbox hardware is identical across BCM2835 / BCM2711 / BCM2712;
//! only the register base address changes.

use crate::mem;

// ─── Register offsets from MBOX_BASE ─────────────────────────────────
const MBOX_READ:   usize = 0x00;
const MBOX_STATUS: usize = 0x18;
const MBOX_WRITE:  usize = 0x20;

// ─── Status register bits ────────────────────────────────────────────
const MBOX_FULL:  u32 = 0x8000_0000;
const MBOX_EMPTY: u32 = 0x4000_0000;

// ─── Channels ────────────────────────────────────────────────────────
const MBOX_CH_PROP: u32 = 8; // ARM → VC property tags

// ─── Response codes ──────────────────────────────────────────────────
const MBOX_RESPONSE_OK: u32 = 0x8000_0000;

// ─── Aligned buffer ──────────────────────────────────────────────────

/// 16-byte aligned mailbox request/response buffer.
///
/// The VideoCore reads this buffer from DRAM, so its physical address
/// must fit in 32 bits (our kernel lives well below the 1 GiB mark).
#[repr(align(16))]
pub struct MboxBuffer {
    pub data: [u32; 36],
}

impl MboxBuffer {
    pub const fn new() -> Self {
        Self { data: [0u32; 36] }
    }
}

/// Static buffer — single-threaded kernel, no contention.
static mut MBOX_BUF: MboxBuffer = MboxBuffer::new();

// ─── Low-level register access ───────────────────────────────────────

#[inline(always)]
unsafe fn read_reg(offset: usize) -> u32 {
    core::ptr::read_volatile((mem::MBOX_BASE + offset) as *const u32)
}

#[inline(always)]
unsafe fn write_reg(offset: usize, val: u32) {
    core::ptr::write_volatile((mem::MBOX_BASE + offset) as *mut u32, val);
}

/// Full data synchronisation barrier (visible to VideoCore).
#[inline(always)]
fn data_sync_barrier() {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("dsb sy", options(nomem, nostack));
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        // Host build — compiler fence is sufficient for correctness checks.
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
    }
}

// ─── Public API ──────────────────────────────────────────────────────

/// Send the static `MBOX_BUF` to the VideoCore on the property tag
/// channel and block until a response arrives.
///
/// The caller must populate `MBOX_BUF.data[..]` with a valid property
/// tag request before calling this function.
///
/// Returns `true` if the firmware responded with success.
///
/// # Safety
/// Caller must ensure no concurrent access to `MBOX_BUF`.
pub unsafe fn call() -> bool {
    let buf = &mut *core::ptr::addr_of_mut!(MBOX_BUF);
    let arm_addr = buf.data.as_ptr() as usize;
    let vc_addr = ((arm_addr + mem::ARM_TO_VC_BUS) & 0xFFFF_FFFF) as u32;

    // Ensure the request buffer writes are visible to the VC.
    data_sync_barrier();

    // Wait until mailbox is not full.
    while read_reg(MBOX_STATUS) & MBOX_FULL != 0 {
        core::hint::spin_loop();
    }

    // Write (VC bus address, 16-byte aligned) | channel.
    write_reg(MBOX_WRITE, (vc_addr & !0xF) | MBOX_CH_PROP);

    // Wait for a response on our channel.
    loop {
        while read_reg(MBOX_STATUS) & MBOX_EMPTY != 0 {
            core::hint::spin_loop();
        }
        let resp = read_reg(MBOX_READ);
        if (resp & 0xF) == MBOX_CH_PROP {
            data_sync_barrier();
            return buf.data[1] == MBOX_RESPONSE_OK;
        }
    }
}

/// Return a mutable reference to the static mailbox buffer.
///
/// # Safety
/// Caller must ensure exclusive access.
#[inline]
pub unsafe fn buffer() -> &'static mut MboxBuffer {
    &mut *core::ptr::addr_of_mut!(MBOX_BUF)
}

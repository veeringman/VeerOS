//! HDMI framebuffer driver for Raspberry Pi 5.
//!
//! Uses the VideoCore mailbox to request a linear framebuffer from the
//! GPU firmware.  Once allocated, the framebuffer is a simple array of
//! pixels in DRAM that the hardware continuously scans out to HDMI.
//!
//! Default resolution: 640×480 @ 32 bpp (classic 80×30 terminal with
//! 8×16 characters).

use crate::mailbox;
use crate::mem;

// ─── Property tag IDs ────────────────────────────────────────────────
const TAG_SET_PHYS_WH:    u32 = 0x0004_8003;
const TAG_SET_VIRT_WH:    u32 = 0x0004_8004;
const TAG_SET_DEPTH:      u32 = 0x0004_8005;
const TAG_SET_VIRT_OFF:   u32 = 0x0004_8009;
const TAG_ALLOC_BUFFER:   u32 = 0x0004_0001;
const TAG_GET_PITCH:      u32 = 0x0004_0008;
const TAG_END:            u32 = 0x0000_0000;

// ─── Framebuffer info returned after successful allocation ───────────

/// Metadata for an allocated framebuffer.
#[derive(Clone, Copy)]
pub struct FbInfo {
    /// Pointer to the start of pixel data (ARM physical, uncached).
    pub fb_ptr: *mut u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per row (pitch).
    pub pitch: u32,
    /// Bits per pixel.
    pub bpp: u32,
    /// Total size in bytes.
    pub size: u32,
}

// ─── Public API ──────────────────────────────────────────────────────

/// Request a framebuffer from the VideoCore firmware.
///
/// Returns `Some(FbInfo)` on success, `None` if the firmware refused
/// or the mailbox timed out.
pub fn init_framebuffer(width: u32, height: u32, bpp: u32) -> Option<FbInfo> {
    unsafe {
        let buf = mailbox::buffer();
        let d = &mut buf.data;

        // ── Build property tag request ────────────────────────
        //
        // Buffer layout (indices into u32 array):
        //  [0]     total buffer size (bytes)
        //  [1]     request/response code
        //  [2..6]  Set physical display size
        //  [7..11] Set virtual display size
        //  [12..15] Set depth
        //  [16..20] Set virtual offset
        //  [21..25] Allocate buffer
        //  [26..29] Get pitch
        //  [30]    End tag

        d[0]  = 31 * 4;        // buffer size (124 bytes, 16-byte aligned struct)
        d[1]  = 0;             // request

        // Tag 1: Set physical (display) size
        d[2]  = TAG_SET_PHYS_WH;
        d[3]  = 8;             // value buffer size
        d[4]  = 0;             // request
        d[5]  = width;
        d[6]  = height;

        // Tag 2: Set virtual size (same as physical — no scrolling)
        d[7]  = TAG_SET_VIRT_WH;
        d[8]  = 8;
        d[9]  = 0;
        d[10] = width;
        d[11] = height;

        // Tag 3: Set depth
        d[12] = TAG_SET_DEPTH;
        d[13] = 4;
        d[14] = 0;
        d[15] = bpp;

        // Tag 4: Set virtual offset (0,0)
        d[16] = TAG_SET_VIRT_OFF;
        d[17] = 8;
        d[18] = 0;
        d[19] = 0;
        d[20] = 0;

        // Tag 5: Allocate buffer (alignment = 16)
        d[21] = TAG_ALLOC_BUFFER;
        d[22] = 8;
        d[23] = 0;
        d[24] = 16;            // alignment
        d[25] = 0;             // size (filled by VC)

        // Tag 6: Get pitch
        d[26] = TAG_GET_PITCH;
        d[27] = 4;
        d[28] = 0;
        d[29] = 0;             // pitch (filled by VC)

        // End tag
        d[30] = TAG_END;

        // ── Send request ──────────────────────────────────────
        if !mailbox::call() {
            return None;
        }

        // ── Parse response ────────────────────────────────────
        let fb_bus_addr = d[24];
        let fb_size     = d[25];
        let pitch       = d[29];

        if fb_bus_addr == 0 || fb_size == 0 {
            return None;
        }

        // Convert VC bus address → ARM physical address.
        let arm_addr = mem::vc_bus_to_arm(fb_bus_addr);

        Some(FbInfo {
            fb_ptr: arm_addr as *mut u32,
            width,
            height,
            pitch,
            bpp,
            size: fb_size,
        })
    }
}

//! Virtio modern (1.x) PCI transport for QEMU `virt` PCIe devices.
//!
//! Parses the virtio PCI capability list, assignsBAR windows from the
//! caller-provided MMIO region, and performs the status/feature/queue
//! handshake. Queue rings themselves live in caller-owned static memory
//! (same split-virtqueue layout as the MMIO net driver); this module only
//! programs their addresses.
//!
//! Polling only — no MSI-X, no GIC wiring. Reference: VIRTIO 1.1 §4.1.

use crate::pcie::PciId;
use core::sync::atomic::{fence, Ordering};

// ─── Capability / config constants ─────────────────────────────────────────

/// Virtio PCI capability vendor ID.
const VIRTIO_PCI_CAP_ID: u8 = 0x09;

const CAP_COMMON: u8 = 1;
const CAP_NOTIFY: u8 = 2;
const CAP_ISR: u8 = 3;
const CAP_DEVICE: u8 = 4;

// Common-config field offsets.
const CC_DEV_FEAT_SEL: usize = 0;
const CC_DEV_FEAT: usize = 4;
const CC_DRV_FEAT_SEL: usize = 8;
const CC_DRV_FEAT: usize = 12;
const CC_NUM_QUEUES: usize = 18;
const CC_DEV_STATUS: usize = 20;
const CC_Q_SELECT: usize = 22;
const CC_Q_SIZE: usize = 24;
const CC_Q_ENABLE: usize = 28;
const CC_Q_NOTIFY_OFF: usize = 30;
const CC_Q_DESC: usize = 32;
const CC_Q_DRIVER: usize = 40;
const CC_Q_DEVICE: usize = 48;

// Status bits.
const ST_ACK: u8 = 1;
const ST_DRIVER: u8 = 2;
const ST_DRIVER_OK: u8 = 4;
const ST_FEATURES_OK: u8 = 8;

/// `VIRTIO_F_VERSION_1` (feature word 1, bit 0).
const F_VERSION_1: u32 = 1;

// ─── MMIO helpers ──────────────────────────────────────────────────────────

#[inline]
unsafe fn r8(base: usize, off: usize) -> u8 {
    core::ptr::read_volatile((base + off) as *const u8)
}

#[inline]
unsafe fn r16(base: usize, off: usize) -> u16 {
    core::ptr::read_volatile((base + off) as *const u16)
}

#[inline]
unsafe fn r32(base: usize, off: usize) -> u32 {
    core::ptr::read_volatile((base + off) as *const u32)
}

#[inline]
unsafe fn r64(base: usize, off: usize) -> u64 {
    core::ptr::read_volatile((base + off) as *const u64)
}

#[inline]
unsafe fn w8(base: usize, off: usize, val: u8) {
    core::ptr::write_volatile((base + off) as *mut u8, val);
}

#[inline]
unsafe fn w16(base: usize, off: usize, val: u16) {
    core::ptr::write_volatile((base + off) as *mut u16, val);
}

#[inline]
unsafe fn w32(base: usize, off: usize, val: u32) {
    core::ptr::write_volatile((base + off) as *mut u32, val);
}

#[inline]
unsafe fn w64(base: usize, off: usize, val: u64) {
    core::ptr::write_volatile((base + off) as *mut u64, val);
}

/// Parsed virtio PCI register blocks.
pub struct VirtioPci {
    common: usize,
    notify: usize,
    notify_mult: u32,
    isr: usize,
    /// Device-specific config base (may be 0 when absent).
    pub device_cfg: usize,
}

impl VirtioPci {
    /// Find capabilities, assign every used BAR out of `window` (bumped
    /// per BAR), reset the device, and negotiate `VIRTIO_F_VERSION_1`.
    /// Returns the transport plus the next free window address, or a
    /// static stage tag on failure (`no-caps`, `features`).
    pub fn init(id: &PciId, window: usize) -> Result<(Self, usize), &'static str> {
        let mut common = 0usize;
        let mut notify = 0usize;
        let mut notify_mult = 0u32;
        let mut isr = 0usize;
        let mut device_cfg = 0usize;
        let mut next = window;
        // One address per BAR: several capabilities usually share a BAR,
        // which must be assigned exactly once.
        let mut bar_addr = [0u32; 6];

        // Walk the capability list (PCI caps like MSI-X interleave
        // with virtio caps — follow `next` past anything foreign).
        let mut cap = id.cap_ptr();
        let mut guard = 0;
        while cap != 0 && guard < 32 {
            guard += 1;
            let off = cap as u16;
            let next_cap = id.cfg8(off + 1);
            if id.cfg8(off) != VIRTIO_PCI_CAP_ID {
                cap = next_cap;
                continue;
            }
            let cfg_type = id.cfg8(off + 3);
            let bar = id.cfg8(off + 4);
            let bar_off = id.cfg32(off + 8) as usize;
            // Assign 64 KiB per used BAR (far above real needs).
            const BAR_WINDOW: u32 = 0x1_0000;
            let bar_base = if (bar as usize) < bar_addr.len() && bar_addr[bar as usize] != 0 {
                bar_addr[bar as usize] as usize
            } else {
                let base = next;
                if (bar as usize) < bar_addr.len() {
                    id.assign_bar(bar, base as u32);
                    bar_addr[bar as usize] = base as u32;
                }
                next += BAR_WINDOW as usize;
                base
            };
            let base = bar_base + bar_off;
            match cfg_type {
                CAP_COMMON => common = base,
                CAP_NOTIFY => {
                    notify = base;
                    notify_mult = id.cfg32(off + 16);
                }
                CAP_ISR => isr = base,
                CAP_DEVICE => device_cfg = base,
                _ => {}
            }
            cap = next_cap;
        }
        if common == 0 || notify == 0 {
            return Err("no-caps");
        }
        id.enable_mem();

        unsafe {
            // Reset + acknowledge.
            w8(common, CC_DEV_STATUS, 0);
            fence(Ordering::SeqCst);
            w8(common, CC_DEV_STATUS, ST_ACK);
            w8(common, CC_DEV_STATUS, ST_ACK | ST_DRIVER);

            // Features: take nothing from word 0, require VERSION_1.
            w32(common, CC_DEV_FEAT_SEL, 0);
            let _ = r32(common, CC_DEV_FEAT);
            w32(common, CC_DRV_FEAT_SEL, 0);
            w32(common, CC_DRV_FEAT, 0);
            w32(common, CC_DEV_FEAT_SEL, 1);
            let _ = r32(common, CC_DEV_FEAT);
            w32(common, CC_DRV_FEAT_SEL, 1);
            w32(common, CC_DRV_FEAT, F_VERSION_1);

            w8(common, CC_DEV_STATUS, ST_ACK | ST_DRIVER | ST_FEATURES_OK);
            fence(Ordering::SeqCst);
            if r8(common, CC_DEV_STATUS) & ST_FEATURES_OK == 0 {
                return Err("features");
            }
        }
        Ok((
            Self {
                common,
                notify,
                notify_mult,
                isr,
                device_cfg,
            },
            next,
        ))
    }

    /// Program one split virtqueue. `desc/avail/used` are guest-physical
    /// (identity-mapped here). Returns the device queue size, which the
    /// caller must support (`size` ring entries).
    pub unsafe fn setup_queue(&self, idx: u16, desc: u64, avail: u64, used: u64) -> u16 {
        w16(self.common, CC_Q_SELECT, idx);
        let size = r16(self.common, CC_Q_SIZE);
        if size == 0 {
            return 0;
        }
        w64(self.common, CC_Q_DESC, desc);
        w64(self.common, CC_Q_DRIVER, avail);
        w64(self.common, CC_Q_DEVICE, used);
        // Enable via QueueEnable (write 1).
        w16(self.common, CC_Q_ENABLE, 1);
        fence(Ordering::SeqCst);
        size
    }

    /// Kick a queue after (re)posting buffers.
    pub unsafe fn notify(&self, idx: u16) {
        w16(self.common, CC_Q_SELECT, idx);
        let off = r16(self.common, CC_Q_NOTIFY_OFF) as usize;
        let addr = self.notify + off * self.notify_mult as usize;
        core::ptr::write_volatile(addr as *mut u16, idx);
        fence(Ordering::SeqCst);
    }

    /// Register-block bases for boot diagnostics:
    /// `(common, notify, notify_mult, isr, device_cfg)`.
    pub fn debug_bases(&self) -> (usize, usize, u32, usize, usize) {
        (
            self.common,
            self.notify,
            self.notify_mult,
            self.isr,
            self.device_cfg,
        )
    }

    /// Programmed queue addresses read back `(desc, avail, used)` plus
    /// the device-side queue size. Ground truth for guest/device agreement.
    pub unsafe fn debug_queue(&self, idx: u16) -> (u64, u64, u64, u16) {
        w16(self.common, CC_Q_SELECT, idx);
        (
            r64(self.common, CC_Q_DESC),
            r64(self.common, CC_Q_DRIVER),
            r64(self.common, CC_Q_DEVICE),
            r16(self.common, CC_Q_SIZE),
        )
    }

    /// Mark the driver live. Call after all queues are programmed —
    /// devices ignore queues and may gate config reads until this.
    pub unsafe fn driver_ok(&self) {
        let mut st = r8(self.common, CC_DEV_STATUS);
        st |= ST_DRIVER_OK;
        w8(self.common, CC_DEV_STATUS, st);
        fence(Ordering::SeqCst);
    }

    /// Acknowledge a pending ISR (best-effort; polling works regardless).
    pub unsafe fn ack_isr(&self) -> u8 {
        if self.isr == 0 {
            return 0;
        }
        r8(self.isr, 0)
    }
}

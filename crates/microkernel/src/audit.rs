//! Security audit log — structured ring buffer of security-relevant events.
//!
//! Unlike [`KernelLog`](crate::klog::KernelLog) (free-form text for `dmesg`),
//! the audit log stores fixed-size entries with machine-readable fields:
//! timestamp, process, user, event type, and a small detail payload.
//!
//! Size is compile-time selected per distribution profile:
//! - `dist-minimal` (ESP32): 128 entries (2 KB)
//! - default (RPi/QEMU):     512 entries
//! - `dist-full` (x86-64):  4096 entries
//!
//! The ring silently overwrites oldest entries when full — security
//! monitoring tools should drain the log periodically.

use core::fmt::{self, Write};

// ── Capacity per distribution tier ──────────────────────────────────────

#[cfg(feature = "dist-minimal")]
const AUDIT_CAPACITY: usize = 128;

#[cfg(not(any(feature = "dist-minimal", feature = "dist-full")))]
const AUDIT_CAPACITY: usize = 512;

#[cfg(feature = "dist-full")]
const AUDIT_CAPACITY: usize = 4096;

// ── Event types ─────────────────────────────────────────────────────────

/// Category of security event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AuditEvent {
    // ── Capability events ────────────────────────────────
    /// Syscall denied due to missing capability.
    CapDenied       = 0x01,
    /// Capability voluntarily dropped.
    CapDropped      = 0x02,
    /// Child process caps restricted.
    CapSetChild     = 0x03,

    // ── Authentication events ────────────────────────────
    /// Successful login / auth.
    AuthSuccess     = 0x10,
    /// Failed login / auth attempt.
    AuthFail        = 0x11,
    /// Logout.
    Logout          = 0x12,
    /// UID change (setuid).
    UidChange       = 0x13,

    // ── Process lifecycle ────────────────────────────────
    /// New process spawned.
    ProcessSpawn    = 0x20,
    /// Process exited.
    ProcessExit     = 0x21,

    // ── Filesystem / mount events ────────────────────────
    /// Filesystem mounted.
    Mount           = 0x30,
    /// Filesystem unmounted.
    Unmount         = 0x31,

    // ── Crypto events ────────────────────────────────────
    /// CSPRNG seeded from hardware entropy.
    RngSeeded       = 0x40,

    // ── Boot / integrity events ──────────────────────────
    /// Boot measurement recorded.
    BootMeasurement = 0x50,

    // ── Network events ───────────────────────────────────
    /// Inbound connection accepted.
    NetAccept       = 0x60,
    /// Firewall rule matched (drop/reject).
    FirewallDrop    = 0x61,

    // ── Generic ──────────────────────────────────────────
    /// Custom event logged by userspace via SYS_AUDIT_LOG.
    UserEvent       = 0xF0,
}

/// A single audit log entry (16 bytes — cache-friendly).
#[derive(Clone, Copy)]
#[repr(C)]
pub struct AuditEntry {
    /// Kernel tick at which the event occurred.
    pub tick: u32,
    /// Process ID that triggered the event.
    pub pid: u8,
    /// User ID of the process at event time.
    pub uid: u8,
    /// Event type.
    pub event: u8,
    /// Reserved / flags.
    pub flags: u8,
    /// Event-specific detail (e.g. denied syscall number, dropped cap bits).
    pub detail: u32,
    /// Secondary detail (e.g. required cap bits on denial).
    pub detail2: u32,
}

impl AuditEntry {
    const fn empty() -> Self {
        Self {
            tick: 0,
            pid: 0,
            uid: 0,
            event: 0,
            flags: 0,
            detail: 0,
            detail2: 0,
        }
    }
}

// ── Ring buffer ─────────────────────────────────────────────────────────

/// Fixed-capacity ring buffer of audit entries.
pub struct AuditLog {
    entries: [AuditEntry; AUDIT_CAPACITY],
    /// Write position (wraps).
    head: usize,
    /// Total entries ever written.
    total: usize,
}

impl AuditLog {
    pub const fn new() -> Self {
        Self {
            entries: [AuditEntry::empty(); AUDIT_CAPACITY],
            head: 0,
            total: 0,
        }
    }

    /// Record a security event.
    pub fn log(
        &mut self,
        tick: u32,
        pid: u8,
        uid: u8,
        event: AuditEvent,
        detail: u32,
        detail2: u32,
    ) {
        self.entries[self.head] = AuditEntry {
            tick,
            pid,
            uid,
            event: event as u8,
            flags: 0,
            detail,
            detail2,
        };
        self.head = (self.head + 1) % AUDIT_CAPACITY;
        self.total += 1;
    }

    /// Number of entries currently in the log (min of total, capacity).
    pub fn len(&self) -> usize {
        if self.total < AUDIT_CAPACITY {
            self.total
        } else {
            AUDIT_CAPACITY
        }
    }

    /// Total events ever recorded (including overwritten).
    pub fn total(&self) -> usize {
        self.total
    }

    /// Get an entry by index (0 = oldest still in buffer).
    /// Returns `None` if `idx >= len()`.
    pub fn get(&self, idx: usize) -> Option<&AuditEntry> {
        let count = self.len();
        if idx >= count {
            return None;
        }
        let start = if self.total <= AUDIT_CAPACITY {
            0
        } else {
            self.head // oldest entry is at head (just been overwritten next)
        };
        let pos = (start + idx) % AUDIT_CAPACITY;
        Some(&self.entries[pos])
    }

    /// Write human-readable dump of the last `max_entries` entries.
    pub fn dump(&self, w: &mut dyn fmt::Write, max_entries: usize) {
        let count = self.len();
        let show = if max_entries < count { count - max_entries } else { 0 };

        let _ = writeln!(w, "  TICK       PID  UID  EVENT            DETAIL");
        let _ = writeln!(w, "  ─────────  ───  ───  ───────────────  ──────────");

        for i in show..count {
            if let Some(e) = self.get(i) {
                let name = event_name(e.event);
                let _ = writeln!(
                    w,
                    "  {:>9}  {:>3}  {:>3}  {:<15}  0x{:08X} 0x{:08X}",
                    e.tick, e.pid, e.uid, name, e.detail, e.detail2,
                );
            }
        }

        if self.total > AUDIT_CAPACITY {
            let _ = writeln!(
                w,
                "  ({} events total, {} overwritten)",
                self.total,
                self.total - AUDIT_CAPACITY,
            );
        }
    }
}

/// Map event byte to display name.
fn event_name(e: u8) -> &'static str {
    match e {
        0x01 => "CAP_DENIED",
        0x02 => "CAP_DROPPED",
        0x03 => "CAP_SET_CHILD",
        0x10 => "AUTH_SUCCESS",
        0x11 => "AUTH_FAIL",
        0x12 => "LOGOUT",
        0x13 => "UID_CHANGE",
        0x20 => "PROC_SPAWN",
        0x21 => "PROC_EXIT",
        0x30 => "MOUNT",
        0x31 => "UNMOUNT",
        0x40 => "RNG_SEEDED",
        0x50 => "BOOT_MEASURE",
        0x60 => "NET_ACCEPT",
        0x61 => "FW_DROP",
        0xF0 => "USER_EVENT",
        _    => "UNKNOWN",
    }
}

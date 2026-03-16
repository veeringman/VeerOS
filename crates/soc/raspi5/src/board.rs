#![allow(dead_code)]
//! BCM2712 board information queries via VideoCore mailbox.
//!
//! Uses the property tag protocol (channel 8) to query:
//! - SoC temperature
//! - Board model / revision / serial
//! - ARM memory size
//! - Clock rates
//!
//! All functions use the shared static mailbox buffer,
//! so they must be called from a single-threaded context.

use crate::mailbox;

// ─── Property tag IDs ────────────────────────────────────────────
const TAG_GET_BOARD_MODEL:    u32 = 0x0001_0001;
const TAG_GET_BOARD_REVISION: u32 = 0x0001_0002;
const TAG_GET_BOARD_SERIAL:   u32 = 0x0001_0004;
const TAG_GET_ARM_MEMORY:     u32 = 0x0001_0005;
const TAG_GET_VC_MEMORY:      u32 = 0x0001_0006;
const TAG_GET_CLOCKS:         u32 = 0x0001_0007;
const TAG_GET_TEMPERATURE:    u32 = 0x0003_0006;
const TAG_GET_MAX_TEMP:       u32 = 0x0003_000A;
const TAG_GET_CLOCK_RATE:     u32 = 0x0003_0002;
const TAG_GET_MAX_CLOCK_RATE: u32 = 0x0003_0004;
const TAG_GET_VOLTAGE:        u32 = 0x0003_0003;
const TAG_GET_MAC_ADDRESS:    u32 = 0x0001_0003;

// ─── Clock IDs ──────────────────────────────────────────────────
const CLOCK_ARM:   u32 = 3;
const CLOCK_CORE:  u32 = 4;
const CLOCK_V3D:   u32 = 5;
const CLOCK_EMMC:  u32 = 1;
const CLOCK_EMMC2: u32 = 12;

// ─── Watchdog / reset registers ─────────────────────────────────
// BCM2712 uses the PM watchdog for reboot (same as BCM2835 design).
const PM_BASE: usize = 0x10_7D20_0000;
const PM_RSTC:  usize = PM_BASE + 0x1C;
const PM_WDOG:  usize = PM_BASE + 0x24;
const PM_PASSWORD: u32 = 0x5A00_0000;
const PM_RSTC_FULLRST: u32 = 0x20;

// ─── Helper: single-value mailbox query ─────────────────────────

/// Issue a single-tag mailbox query with one u32 input, returning
/// the first u32 of the response value buffer.
unsafe fn mbox_query_1(tag: u32, req_val: u32) -> Option<u32> {
    let buf = mailbox::buffer();
    buf.data[0] = 8 * 4;         // total buffer size (bytes)
    buf.data[1] = 0;             // request code
    buf.data[2] = tag;           // tag
    buf.data[3] = 8;             // value buffer size (bytes)
    buf.data[4] = 0;             // request/response code
    buf.data[5] = req_val;       // input value (e.g. clock ID, temp ID)
    buf.data[6] = 0;             // space for response
    buf.data[7] = 0;             // end tag
    if mailbox::call() {
        Some(buf.data[6])
    } else {
        None
    }
}

/// Issue a single-tag query with no input, returning one u32.
unsafe fn mbox_query_0(tag: u32) -> Option<u32> {
    let buf = mailbox::buffer();
    buf.data[0] = 7 * 4;
    buf.data[1] = 0;
    buf.data[2] = tag;
    buf.data[3] = 4;
    buf.data[4] = 0;
    buf.data[5] = 0;
    buf.data[6] = 0;
    if mailbox::call() {
        Some(buf.data[5])
    } else {
        None
    }
}

/// Issue a two-value query (e.g. ARM memory: base + size).
unsafe fn mbox_query_2(tag: u32) -> Option<(u32, u32)> {
    let buf = mailbox::buffer();
    buf.data[0] = 8 * 4;
    buf.data[1] = 0;
    buf.data[2] = tag;
    buf.data[3] = 8;
    buf.data[4] = 0;
    buf.data[5] = 0;
    buf.data[6] = 0;
    buf.data[7] = 0;
    if mailbox::call() {
        Some((buf.data[5], buf.data[6]))
    } else {
        None
    }
}

// ─── Public API ──────────────────────────────────────────────────

/// Read the SoC temperature in millidegrees Celsius.
/// Returns 0 if the query fails.
pub fn get_temperature() -> i32 {
    unsafe {
        mbox_query_1(TAG_GET_TEMPERATURE, 0).unwrap_or(0) as i32
    }
}

/// Read the maximum safe SoC temperature in millidegrees Celsius.
pub fn get_max_temperature() -> i32 {
    unsafe {
        mbox_query_1(TAG_GET_MAX_TEMP, 0).unwrap_or(0) as i32
    }
}

/// Read the board revision code.
pub fn get_board_revision() -> u32 {
    unsafe { mbox_query_0(TAG_GET_BOARD_REVISION).unwrap_or(0) }
}

/// Read the board model.
pub fn get_board_model() -> u32 {
    unsafe { mbox_query_0(TAG_GET_BOARD_MODEL).unwrap_or(0) }
}

/// Read the board serial number (lower 32 bits).
pub fn get_serial() -> u32 {
    unsafe { mbox_query_0(TAG_GET_BOARD_SERIAL).unwrap_or(0) }
}

/// Read ARM memory (base, size in bytes).
pub fn get_arm_memory() -> (u32, u32) {
    unsafe { mbox_query_2(TAG_GET_ARM_MEMORY).unwrap_or((0, 0)) }
}

/// Read VideoCore memory (base, size in bytes).
pub fn get_vc_memory() -> (u32, u32) {
    unsafe { mbox_query_2(TAG_GET_VC_MEMORY).unwrap_or((0, 0)) }
}

/// Read current clock rate for a given clock ID (in Hz).
pub fn get_clock_rate(clock_id: u32) -> u32 {
    unsafe { mbox_query_1(TAG_GET_CLOCK_RATE, clock_id).unwrap_or(0) }
}

/// Read max clock rate for a given clock ID (in Hz).
pub fn get_max_clock_rate(clock_id: u32) -> u32 {
    unsafe { mbox_query_1(TAG_GET_MAX_CLOCK_RATE, clock_id).unwrap_or(0) }
}

/// Read the MAC address via mailbox (returns 6 bytes, or all zeros on failure).
pub fn get_mac_address() -> [u8; 6] {
    unsafe {
        let buf = mailbox::buffer();
        buf.data[0] = 8 * 4;
        buf.data[1] = 0;
        buf.data[2] = TAG_GET_MAC_ADDRESS;
        buf.data[3] = 6;   // 6 bytes value buffer
        buf.data[4] = 0;
        buf.data[5] = 0;
        buf.data[6] = 0;
        buf.data[7] = 0;
        if mailbox::call() {
            let b0 = buf.data[5];
            let b1 = buf.data[6];
            [
                (b0 & 0xFF) as u8,
                ((b0 >> 8) & 0xFF) as u8,
                ((b0 >> 16) & 0xFF) as u8,
                ((b0 >> 24) & 0xFF) as u8,
                (b1 & 0xFF) as u8,
                ((b1 >> 8) & 0xFF) as u8,
            ]
        } else {
            [0u8; 6]
        }
    }
}

/// Decode board revision into a human-readable model string.
pub fn revision_to_model(rev: u32) -> &'static str {
    // New-style revision (bit 23 set)
    if rev & (1 << 23) != 0 {
        let model = (rev >> 4) & 0xFF;
        match model {
            0x00 => "Raspberry Pi 1 Model A",
            0x01 => "Raspberry Pi 1 Model B",
            0x02 => "Raspberry Pi 1 Model A+",
            0x03 => "Raspberry Pi 1 Model B+",
            0x04 => "Raspberry Pi 2 Model B",
            0x06 => "Raspberry Pi Compute Module 1",
            0x08 => "Raspberry Pi 3 Model B",
            0x09 => "Raspberry Pi Zero",
            0x0A => "Raspberry Pi Compute Module 3",
            0x0C => "Raspberry Pi Zero W",
            0x0D => "Raspberry Pi 3 Model B+",
            0x0E => "Raspberry Pi 3 Model A+",
            0x10 => "Raspberry Pi Compute Module 3+",
            0x11 => "Raspberry Pi 4 Model B",
            0x13 => "Raspberry Pi 400",
            0x14 => "Raspberry Pi Compute Module 4",
            0x17 => "Raspberry Pi 5",
            _ => "Raspberry Pi (unknown model)",
        }
    } else {
        "Raspberry Pi (old-style revision)"
    }
}

/// Write a full hardware info summary to a writer.
pub fn write_hw_info(w: &mut dyn core::fmt::Write) {
    let rev = get_board_revision();
    let model = revision_to_model(rev);
    let _ = writeln!(w, "  Board     : {} (rev 0x{:08X})", model, rev);

    let serial = get_serial();
    let _ = writeln!(w, "  Serial    : {:08X}", serial);

    let (arm_base, arm_size) = get_arm_memory();
    let arm_mb = arm_size / (1024 * 1024);
    let _ = writeln!(w, "  ARM memory: {} MiB (base 0x{:08X})", arm_mb, arm_base);

    let (vc_base, vc_size) = get_vc_memory();
    let vc_mb = vc_size / (1024 * 1024);
    let _ = writeln!(w, "  VC memory : {} MiB (base 0x{:08X})", vc_mb, vc_base);

    let temp = get_temperature();
    let deg = temp / 1000;
    let frac = ((temp % 1000).unsigned_abs() / 100) as u32;
    let max_temp = get_max_temperature();
    let max_deg = max_temp / 1000;
    let _ = writeln!(w, "  SoC temp  : {}.{}°C (max {}°C)", deg, frac, max_deg);

    let arm_hz = get_clock_rate(CLOCK_ARM);
    let arm_mhz = arm_hz / 1_000_000;
    let _ = writeln!(w, "  ARM clock : {} MHz", arm_mhz);

    let core_hz = get_clock_rate(CLOCK_CORE);
    let core_mhz = core_hz / 1_000_000;
    let _ = writeln!(w, "  Core clock: {} MHz", core_mhz);

    let mac = get_mac_address();
    let _ = writeln!(w, "  MAC addr  : {:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
}

// ─── Power control ──────────────────────────────────────────────

/// Trigger a hardware reboot via the PM watchdog.
pub fn reboot() -> ! {
    unsafe {
        // Set watchdog timeout to ~10 ticks (~150us)
        core::ptr::write_volatile(PM_WDOG as *mut u32, PM_PASSWORD | 10);
        // Full reset
        let mut val = core::ptr::read_volatile(PM_RSTC as *const u32);
        val = (val & !0x30) | PM_RSTC_FULLRST;
        core::ptr::write_volatile(PM_RSTC as *mut u32, PM_PASSWORD | val);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Halt the CPU (simple WFI loop, no actual power-off without PMIC).
pub fn shutdown() -> ! {
    // Disable interrupts and enter infinite WFI
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "msr daifset, #0xF", // mask all interrupts
            "1: wfi",
            "b 1b",
            options(noreturn)
        );
    }
    #[cfg(not(target_arch = "aarch64"))]
    loop {
        core::hint::spin_loop();
    }
}

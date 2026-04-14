//! Local APIC (Advanced Programmable Interrupt Controller) driver.
//!
//! The Local APIC is memory-mapped at 0xFEE0_0000 by default. It provides:
//! - Per-core timer in periodic/one-shot/TSC-deadline modes
//! - Inter-processor interrupts (IPI) for SMP
//! - Spurious interrupt vector management
//! - End-of-interrupt signalling
//!
//! On QEMU, the LAPIC is always present when `-cpu qemu64` or `-cpu host`
//! is used with the q35 or pc machine type.

/// Default Local APIC MMIO base address.
const LAPIC_BASE: usize = 0xFEE0_0000;

// ─── LAPIC register offsets (byte offsets from base) ─────────────────────

/// Local APIC ID register.
const REG_ID: usize = 0x020;
/// Local APIC version register.
const REG_VERSION: usize = 0x030;
/// Task Priority Register.
const REG_TPR: usize = 0x080;
/// End-Of-Interrupt register (write-only).
const REG_EOI: usize = 0x0B0;
/// Spurious Interrupt Vector Register.
const REG_SIVR: usize = 0x0F0;
/// Interrupt Command Register (low 32 bits).
const REG_ICR_LO: usize = 0x300;
/// Interrupt Command Register (high 32 bits).
const REG_ICR_HI: usize = 0x310;
/// LVT Timer register.
const REG_LVT_TIMER: usize = 0x320;
/// LVT LINT0 register.
const REG_LVT_LINT0: usize = 0x350;
/// LVT LINT1 register.
const REG_LVT_LINT1: usize = 0x360;
/// LVT Error register.
const REG_LVT_ERROR: usize = 0x370;
/// Timer initial count register.
const REG_TIMER_INIT: usize = 0x380;
/// Timer current count register.
const REG_TIMER_CURRENT: usize = 0x390;
/// Timer divide configuration register.
const REG_TIMER_DIV: usize = 0x3E0;

// ─── Constants ───────────────────────────────────────────────────────────

/// Spurious interrupt vector number (must have low 4 bits set, typically 0xFF).
const SPURIOUS_VEC: u32 = 0xFF;
/// APIC software enable bit in the SIVR register.
const SIVR_ENABLE: u32 = 1 << 8;

/// Timer mode: periodic.
const TIMER_PERIODIC: u32 = 1 << 17;
/// Timer mask bit (1 = masked/disabled).
const LVT_MASKED: u32 = 1 << 16;

/// Timer divide: divide by 16.
const TIMER_DIV_16: u32 = 0x03;

/// LAPIC timer interrupt vector.
pub const TIMER_VECTOR: u8 = 32;

// ─── MMIO reads/writes ──────────────────────────────────────────────────

#[inline]
unsafe fn lapic_read(offset: usize) -> u32 {
    let ptr = (LAPIC_BASE + offset) as *const u32;
    unsafe { ptr.read_volatile() }
}

#[inline]
unsafe fn lapic_write(offset: usize, val: u32) {
    let ptr = (LAPIC_BASE + offset) as *mut u32;
    unsafe { ptr.write_volatile(val); }
}

// ─── Public API ─────────────────────────────────────────────────────────

/// Read the Local APIC ID.
pub fn id() -> u32 {
    unsafe { (lapic_read(REG_ID) >> 24) & 0xFF }
}

/// Read the Local APIC version.
pub fn version() -> u32 {
    unsafe { lapic_read(REG_VERSION) & 0xFF }
}

/// Initialise the Local APIC: enable it, set spurious vector, unmask timer.
pub fn init() {
    unsafe {
        // Enable APIC + set spurious interrupt vector.
        lapic_write(REG_SIVR, SIVR_ENABLE | SPURIOUS_VEC);

        // Mask all LVT entries initially.
        lapic_write(REG_LVT_TIMER, LVT_MASKED);
        lapic_write(REG_LVT_LINT0, LVT_MASKED);
        lapic_write(REG_LVT_LINT1, LVT_MASKED);
        lapic_write(REG_LVT_ERROR, LVT_MASKED);

        // Set task priority to 0 (accept all interrupts).
        lapic_write(REG_TPR, 0);
    }
}

/// Configure the LAPIC timer in periodic mode.
///
/// `vector` — interrupt vector (typically 32).
/// `initial_count` — timer initial count value. The actual period depends
///   on the bus clock / divide ratio. Use `calibrate_timer()` to determine
///   a good initial count for a desired period.
pub fn configure_timer_periodic(vector: u8, initial_count: u32) {
    unsafe {
        // Set divide configuration.
        lapic_write(REG_TIMER_DIV, TIMER_DIV_16);

        // Set LVT Timer: periodic mode, desired vector, not masked.
        lapic_write(REG_LVT_TIMER, TIMER_PERIODIC | (vector as u32));

        // Set initial count (starts the timer immediately).
        lapic_write(REG_TIMER_INIT, initial_count);
    }
}

/// Calibrate the LAPIC timer using the PIT as a reference.
///
/// Returns the LAPIC timer count for approximately 1 ms.
/// This uses PIT channel 2 in one-shot mode as a ~1 ms reference.
pub fn calibrate_timer_1ms() -> u32 {
    // Use PIT channel 2 for calibration (doesn't interfere with channel 0).
    // PIT frequency: 1_193_182 Hz → 1 ms ≈ 1193 ticks.
    const PIT_1MS: u16 = 1193;

    unsafe {
        // Set up LAPIC timer with max count, divide by 16.
        lapic_write(REG_TIMER_DIV, TIMER_DIV_16);
        // Mask timer so it doesn't fire during calibration.
        lapic_write(REG_LVT_TIMER, LVT_MASKED);
        // Start with max initial count.
        lapic_write(REG_TIMER_INIT, 0xFFFF_FFFF);

        // PIT channel 2, mode 0 (one-shot), low/high byte.
        crate::outb(0x61, (crate::inb(0x61) & 0xFD) | 0x01); // gate high
        crate::outb(0x43, 0xB0); // channel 2, lo/hi, mode 0
        crate::outb(0x42, (PIT_1MS & 0xFF) as u8);
        crate::outb(0x42, ((PIT_1MS >> 8) & 0xFF) as u8);

        // Wait for PIT channel 2 output to go high (bit 5 of port 0x61).
        while crate::inb(0x61) & 0x20 == 0 {
            core::hint::spin_loop();
        }

        // Read LAPIC current count.
        let current = lapic_read(REG_TIMER_CURRENT);

        // Stop LAPIC timer.
        lapic_write(REG_TIMER_INIT, 0);

        // Elapsed = 0xFFFF_FFFF - current.
        0xFFFF_FFFFu32.wrapping_sub(current)
    }
}

/// Read the current LAPIC timer count.
pub fn timer_current() -> u32 {
    unsafe { lapic_read(REG_TIMER_CURRENT) }
}

/// Send End-Of-Interrupt to the Local APIC.
pub fn eoi() {
    unsafe { lapic_write(REG_EOI, 0); }
}

/// Send an IPI (Inter-Processor Interrupt) to another core.
///
/// `target_apic_id` — destination LAPIC ID.
/// `vector` — interrupt vector to deliver.
pub fn send_ipi(target_apic_id: u8, vector: u8) {
    unsafe {
        lapic_write(REG_ICR_HI, (target_apic_id as u32) << 24);
        // Fixed delivery, physical destination, vector.
        lapic_write(REG_ICR_LO, vector as u32);
    }
}

/// Send INIT IPI to a target processor (for SMP bring-up).
pub fn send_init_ipi(target_apic_id: u8) {
    unsafe {
        lapic_write(REG_ICR_HI, (target_apic_id as u32) << 24);
        // INIT: type=0b101, level=1, trigger=edge
        lapic_write(REG_ICR_LO, 0x0000_C500);
    }
}

/// Send Startup IPI (SIPI) to a target processor (for SMP bring-up).
///
/// `page` — the real-mode page (0x00–0xFF) where the AP trampoline code is
///   located. The AP starts executing at physical address `page * 0x1000`.
pub fn send_sipi(target_apic_id: u8, page: u8) {
    unsafe {
        lapic_write(REG_ICR_HI, (target_apic_id as u32) << 24);
        // SIPI: type=0b110, vector = page
        lapic_write(REG_ICR_LO, 0x0000_0600 | (page as u32));
    }
}

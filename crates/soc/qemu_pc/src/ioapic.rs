//! I/O APIC driver (typically at 0xFEC0_0000).
//!
//! The I/O APIC routes external interrupts (keyboard, COM1, HPET, etc.)
//! to Local APICs. Each I/O APIC has a redirection table — one 64-bit
//! entry per IRQ input pin specifying the delivery vector, destination
//! LAPIC, and trigger/polarity settings.
//!
//! On QEMU q35, there is one I/O APIC with 24 input pins (GSI 0–23).

/// Default I/O APIC MMIO base address.
const IOAPIC_BASE: usize = 0xFEC0_0000;

// I/O APIC registers (accessed indirectly via IOREGSEL + IOWIN).
const IOREGSEL: usize = 0x00; // register select (write index here)
const IOWIN: usize = 0x10;    // data window (read/write register value)

// Register indices.
const REG_ID: u8 = 0x00;
const REG_VER: u8 = 0x01;
// Redirection table entries start at 0x10 (low) / 0x11 (high).
const REG_REDIR_BASE: u8 = 0x10;

// ─── Redirection table entry flags ──────────────────────────────────────

/// Delivery mode: Fixed (000).
const DELIV_FIXED: u32 = 0 << 8;
/// Destination mode: Physical (0 = physical APIC ID).
const DEST_PHYSICAL: u32 = 0 << 11;
/// Polarity: Active high (default for ISA).
const POLARITY_HIGH: u32 = 0 << 13;
/// Trigger: Edge-triggered (default for ISA).
const TRIGGER_EDGE: u32 = 0 << 15;
/// Mask bit (1 = masked).
const MASKED: u32 = 1 << 16;

// ─── MMIO helpers ───────────────────────────────────────────────────────

unsafe fn ioapic_read(reg: u8) -> u32 {
    let base = IOAPIC_BASE as *mut u32;
    unsafe {
        base.add(IOREGSEL / 4).write_volatile(reg as u32);
        base.add(IOWIN / 4).read_volatile()
    }
}

unsafe fn ioapic_write(reg: u8, val: u32) {
    let base = IOAPIC_BASE as *mut u32;
    unsafe {
        base.add(IOREGSEL / 4).write_volatile(reg as u32);
        base.add(IOWIN / 4).write_volatile(val);
    }
}

// ─── Public API ─────────────────────────────────────────────────────────

/// Read the I/O APIC ID.
pub fn id() -> u32 {
    unsafe { (ioapic_read(REG_ID) >> 24) & 0x0F }
}

/// Get the maximum number of redirection entries (0-based).
pub fn max_entries() -> u32 {
    unsafe { ((ioapic_read(REG_VER) >> 16) & 0xFF) + 1 }
}

/// Read a redirection table entry (64-bit: low + high).
pub fn read_redir(irq: u8) -> u64 {
    let reg = REG_REDIR_BASE + irq * 2;
    let lo = unsafe { ioapic_read(reg) } as u64;
    let hi = unsafe { ioapic_read(reg + 1) } as u64;
    lo | (hi << 32)
}

/// Write a redirection table entry.
pub fn write_redir(irq: u8, entry: u64) {
    let reg = REG_REDIR_BASE + irq * 2;
    unsafe {
        ioapic_write(reg, entry as u32);
        ioapic_write(reg + 1, (entry >> 32) as u32);
    }
}

/// Mask all I/O APIC entries (set mask bit on all redirection entries).
pub fn mask_all() {
    let max = max_entries();
    for i in 0..max {
        let entry = read_redir(i as u8);
        write_redir(i as u8, entry | (MASKED as u64));
    }
}

/// Route an ISA IRQ to a specific vector on a target LAPIC.
///
/// Uses edge-triggered, active-high, physical destination (ISA defaults).
///
/// `irq` — I/O APIC input pin (0–23).
/// `vector` — interrupt vector to deliver (32–255).
/// `dest_apic_id` — target Local APIC ID (usually 0 for BSP).
pub fn route_irq(irq: u8, vector: u8, dest_apic_id: u8) {
    let lo = (vector as u32)
        | DELIV_FIXED
        | DEST_PHYSICAL
        | POLARITY_HIGH
        | TRIGGER_EDGE;
    let hi = (dest_apic_id as u32) << 24;
    let entry = (lo as u64) | ((hi as u64) << 32);
    write_redir(irq, entry);
}

/// Mask a specific I/O APIC entry.
pub fn mask(irq: u8) {
    let entry = read_redir(irq);
    write_redir(irq, entry | (MASKED as u64));
}

/// Unmask a specific I/O APIC entry.
pub fn unmask(irq: u8) {
    let entry = read_redir(irq);
    write_redir(irq, entry & !(MASKED as u64));
}

/// Initialise the I/O APIC: mask all entries, then set up standard ISA
/// routing for timer, keyboard, cascade, and COM1.
///
/// `bsp_apic_id` — the LAPIC ID of the bootstrap processor.
pub fn init(bsp_apic_id: u8) {
    // Mask all entries first.
    mask_all();

    // Route standard ISA IRQs to the BSP:
    // IRQ 0  → timer      → vector 32
    route_irq(0, 32, bsp_apic_id);
    // IRQ 1  → keyboard   → vector 33
    route_irq(1, 33, bsp_apic_id);
    // IRQ 2  → cascade    → masked (no slave PIC in APIC mode)
    write_redir(2, MASKED as u64);
    // IRQ 4  → COM1       → vector 36
    route_irq(4, 36, bsp_apic_id);
}

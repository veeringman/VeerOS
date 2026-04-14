//! ACPI table parsing for x86-64 (RSDP → RSDT → MADT/FADT/HPET).
//!
//! Discovers:
//! - MADT: Local APIC entries (for SMP) + I/O APIC address + IRQ overrides
//! - FADT: ACPI power management (PM1a_CNT for clean shutdown)
//! - HPET: verifies HPET MMIO base address
//!
//! RSDP is found by scanning the EBDA (0x9FC00-0x9FFFF) and the BIOS
//! read-only area (0xE0000-0xFFFFF) on 16-byte boundaries.

/// Maximum number of application processors we track.
pub const MAX_CPUS: usize = 16;

// ─── RSDP (Root System Description Pointer) ─────────────────────────────

/// ACPI 1.0 RSDP structure (20 bytes).
#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],  // "RSD PTR "
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,        // 0 = ACPI 1.0, 2 = ACPI 2.0+
    rsdt_address: u32,
}

// ─── SDT Header (common to all ACPI tables) ─────────────────────────────

/// ACPI System Description Table header (36 bytes).
#[repr(C, packed)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

// ─── MADT (Multiple APIC Description Table) ─────────────────────────────

/// MADT-specific fields after the standard header.
#[repr(C, packed)]
struct MadtHeader {
    header: SdtHeader,
    local_apic_addr: u32,
    flags: u32,
    // Variable-length entries follow.
}

/// MADT entry header.
#[repr(C, packed)]
struct MadtEntry {
    entry_type: u8,
    length: u8,
}

/// MADT entry type 0: Processor Local APIC.
#[repr(C, packed)]
struct MadtLocalApic {
    header: MadtEntry,
    acpi_processor_id: u8,
    apic_id: u8,
    flags: u32,
}

/// MADT entry type 1: I/O APIC.
#[repr(C, packed)]
struct MadtIoApic {
    header: MadtEntry,
    io_apic_id: u8,
    _reserved: u8,
    io_apic_address: u32,
    gsi_base: u32,
}

/// MADT entry type 2: Interrupt Source Override.
#[repr(C, packed)]
struct MadtIso {
    header: MadtEntry,
    bus: u8,
    source: u8,
    gsi: u32,
    flags: u16,
}

// ─── FADT (Fixed ACPI Description Table) ─────────────────────────────────

/// FADT fields we care about (offsets from header start).
#[repr(C, packed)]
struct Fadt {
    header: SdtHeader,
    firmware_ctrl: u32,
    dsdt: u32,
    _reserved0: u8,
    preferred_pm_profile: u8,
    sci_interrupt: u16,
    smi_command_port: u32,
    acpi_enable: u8,
    acpi_disable: u8,
    s4bios_req: u8,
    pstate_control: u8,
    pm1a_event_block: u32,
    pm1b_event_block: u32,
    pm1a_control_block: u32,
    pm1b_control_block: u32,
    // ...more fields follow, but we only need pm1a_control_block for shutdown.
}

// ─── HPET table ─────────────────────────────────────────────────────────

/// HPET ACPI table.
#[repr(C, packed)]
struct HpetTable {
    header: SdtHeader,
    event_timer_block_id: u32,
    base_address_space_id: u8,
    base_register_bit_width: u8,
    base_register_bit_offset: u8,
    _reserved: u8,
    base_address: u64,
    hpet_number: u8,
    min_clock_tick: u16,
    page_protection: u8,
}

// ═══════════════════════════════════════════════════════════════════════════
// Parsed ACPI info
// ═══════════════════════════════════════════════════════════════════════════

/// An application processor discovered via MADT.
#[derive(Clone, Copy)]
pub struct CpuInfo {
    pub apic_id: u8,
    pub acpi_id: u8,
    pub enabled: bool,
}

impl CpuInfo {
    pub const EMPTY: Self = Self { apic_id: 0, acpi_id: 0, enabled: false };
}

/// IRQ source override (ISA IRQ → GSI mapping).
#[derive(Clone, Copy)]
pub struct IrqOverride {
    pub bus: u8,
    pub source_irq: u8,
    pub gsi: u32,
    pub flags: u16,
    pub active: bool,
}

impl IrqOverride {
    pub const EMPTY: Self = Self { bus: 0, source_irq: 0, gsi: 0, flags: 0, active: false };
}

/// Parsed ACPI information.
pub struct AcpiInfo {
    /// Local APIC physical base address.
    pub local_apic_addr: u32,
    /// I/O APIC physical base address.
    pub io_apic_addr: u32,
    /// I/O APIC ID.
    pub io_apic_id: u8,
    /// I/O APIC GSI base.
    pub io_apic_gsi_base: u32,
    /// Bootstrap processor APIC ID.
    pub bsp_apic_id: u8,
    /// Number of CPUs found (including BSP).
    pub cpu_count: usize,
    /// CPU info table.
    pub cpus: [CpuInfo; MAX_CPUS],
    /// ACPI PM1a control block I/O port (for shutdown).
    pub pm1a_control_block: u32,
    /// SCI interrupt number.
    pub sci_interrupt: u16,
    /// HPET base address from ACPI (0 if not found).
    pub hpet_base: u64,
    /// IRQ source overrides (up to 16).
    pub irq_overrides: [IrqOverride; 16],
    pub irq_override_count: usize,
    /// Whether ACPI tables were found at all.
    pub valid: bool,
}

impl AcpiInfo {
    pub const fn new() -> Self {
        Self {
            local_apic_addr: 0xFEE0_0000,
            io_apic_addr: 0xFEC0_0000,
            io_apic_id: 0,
            io_apic_gsi_base: 0,
            bsp_apic_id: 0,
            cpu_count: 0,
            cpus: [CpuInfo::EMPTY; MAX_CPUS],
            pm1a_control_block: 0,
            sci_interrupt: 0,
            hpet_base: 0,
            irq_overrides: [IrqOverride::EMPTY; 16],
            irq_override_count: 0,
            valid: false,
        }
    }

    /// Number of APs (application processors, excluding BSP).
    pub fn ap_count(&self) -> usize {
        if self.cpu_count > 1 { self.cpu_count - 1 } else { 0 }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RSDP scanning
// ═══════════════════════════════════════════════════════════════════════════

/// Validate an ACPI checksum (sum of all bytes mod 256 must be 0).
fn acpi_checksum(ptr: *const u8, len: usize) -> bool {
    let mut sum: u8 = 0;
    for i in 0..len {
        sum = sum.wrapping_add(unsafe { *ptr.add(i) });
    }
    sum == 0
}

/// Search for the RSDP signature in a memory range (16-byte aligned).
fn find_rsdp_in_range(start: usize, end: usize) -> Option<usize> {
    let sig = b"RSD PTR ";
    let mut addr = start & !0xF; // align to 16 bytes
    while addr + 20 <= end {
        let ptr = addr as *const u8;
        let mut matches = true;
        for i in 0..8 {
            if unsafe { *ptr.add(i) } != sig[i] {
                matches = false;
                break;
            }
        }
        if matches && acpi_checksum(ptr, 20) {
            return Some(addr);
        }
        addr += 16;
    }
    None
}

/// Find the RSDP by scanning EBDA and the BIOS ROM area.
fn find_rsdp() -> Option<usize> {
    // Try EBDA (Extended BIOS Data Area) — segment pointer at 0x040E.
    let ebda_seg = unsafe { *(0x040E as *const u16) } as usize;
    let ebda_base = ebda_seg << 4;
    if ebda_base > 0x80000 && ebda_base < 0xA0000 {
        if let Some(addr) = find_rsdp_in_range(ebda_base, ebda_base + 1024) {
            return Some(addr);
        }
    }

    // Scan BIOS read-only memory area (0xE0000 - 0xFFFFF).
    find_rsdp_in_range(0xE0000, 0x100000)
}

// ═══════════════════════════════════════════════════════════════════════════
// Table parsing
// ═══════════════════════════════════════════════════════════════════════════

/// Parse the MADT (Multiple APIC Description Table).
fn parse_madt(madt_addr: usize, info: &mut AcpiInfo) {
    let madt = madt_addr as *const MadtHeader;
    let total_len = unsafe { (*madt).header.length } as usize;

    info.local_apic_addr = unsafe { (*madt).local_apic_addr };

    // Walk variable-length entries after the fixed header (44 bytes).
    let entries_start = madt_addr + core::mem::size_of::<MadtHeader>();
    let entries_end = madt_addr + total_len;
    let mut offset = entries_start;

    while offset + 2 <= entries_end {
        let entry = offset as *const MadtEntry;
        let entry_type = unsafe { (*entry).entry_type };
        let entry_len = unsafe { (*entry).length } as usize;
        if entry_len < 2 || offset + entry_len > entries_end {
            break;
        }

        match entry_type {
            0 => {
                // Type 0: Processor Local APIC
                let lapic = offset as *const MadtLocalApic;
                let flags = unsafe { (*lapic).flags };
                let apic_id = unsafe { (*lapic).apic_id };
                let acpi_id = unsafe { (*lapic).acpi_processor_id };

                // Bit 0: Processor Enabled, Bit 1: Online Capable
                if (flags & 0x03) != 0 && info.cpu_count < MAX_CPUS {
                    info.cpus[info.cpu_count] = CpuInfo {
                        apic_id,
                        acpi_id,
                        enabled: (flags & 1) != 0,
                    };
                    info.cpu_count += 1;
                }
            }
            1 => {
                // Type 1: I/O APIC
                let ioapic = offset as *const MadtIoApic;
                info.io_apic_addr = unsafe { (*ioapic).io_apic_address };
                info.io_apic_id = unsafe { (*ioapic).io_apic_id };
                info.io_apic_gsi_base = unsafe { (*ioapic).gsi_base };
            }
            2 => {
                // Type 2: Interrupt Source Override
                let iso = offset as *const MadtIso;
                if info.irq_override_count < 16 {
                    info.irq_overrides[info.irq_override_count] = IrqOverride {
                        bus: unsafe { (*iso).bus },
                        source_irq: unsafe { (*iso).source },
                        gsi: unsafe { (*iso).gsi },
                        flags: unsafe { (*iso).flags },
                        active: true,
                    };
                    info.irq_override_count += 1;
                }
            }
            _ => {
                // Types 3, 4, 5, etc. — skip.
            }
        }

        offset += entry_len;
    }

    // The first CPU is typically the BSP.
    if info.cpu_count > 0 {
        info.bsp_apic_id = info.cpus[0].apic_id;
    }
}

/// Parse the FADT (Fixed ACPI Description Table).
fn parse_fadt(fadt_addr: usize, info: &mut AcpiInfo) {
    let fadt = fadt_addr as *const Fadt;
    info.pm1a_control_block = unsafe { (*fadt).pm1a_control_block };
    info.sci_interrupt = unsafe { (*fadt).sci_interrupt };
}

/// Parse the HPET ACPI table.
fn parse_hpet_table(hpet_addr: usize, info: &mut AcpiInfo) {
    let hpet = hpet_addr as *const HpetTable;
    info.hpet_base = unsafe { (*hpet).base_address };
}

/// Parse the RSDT (Root System Description Table) and find sub-tables.
fn parse_rsdt(rsdt_addr: usize, info: &mut AcpiInfo) {
    let header = rsdt_addr as *const SdtHeader;
    let total_len = unsafe { (*header).length } as usize;
    let header_size = core::mem::size_of::<SdtHeader>();
    let num_entries = (total_len - header_size) / 4;

    let entries_ptr = (rsdt_addr + header_size) as *const u32;

    for i in 0..num_entries {
        let table_addr = unsafe { *entries_ptr.add(i) } as usize;
        if table_addr == 0 {
            continue;
        }

        let sig_ptr = table_addr as *const [u8; 4];
        let sig = unsafe { *sig_ptr };

        match &sig {
            b"APIC" => parse_madt(table_addr, info),
            b"FACP" => parse_fadt(table_addr, info),
            b"HPET" => parse_hpet_table(table_addr, info),
            _ => {}
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Public API
// ═══════════════════════════════════════════════════════════════════════════

/// Parse ACPI tables and populate an `AcpiInfo` struct.
///
/// Returns `true` if RSDP + RSDT were found and parsed successfully.
pub fn parse(info: &mut AcpiInfo) -> bool {
    let rsdp_addr = match find_rsdp() {
        Some(a) => a,
        None => return false,
    };

    let rsdp = rsdp_addr as *const Rsdp;
    let rsdt_addr = unsafe { (*rsdp).rsdt_address } as usize;
    if rsdt_addr == 0 {
        return false;
    }

    // Validate RSDT header signature.
    let sig = unsafe { *(rsdt_addr as *const [u8; 4]) };
    if &sig != b"RSDT" {
        return false;
    }

    // Validate RSDT checksum.
    let rsdt_len = unsafe { *((rsdt_addr + 4) as *const u32) } as usize;
    if rsdt_len < core::mem::size_of::<SdtHeader>() || rsdt_len > 0x10000 {
        return false;
    }
    if !acpi_checksum(rsdt_addr as *const u8, rsdt_len) {
        return false;
    }

    parse_rsdt(rsdt_addr, info);
    info.valid = true;
    true
}

/// Perform ACPI shutdown using the PM1a control block.
///
/// Sends SLP_TYP=5 + SLP_EN to PM1a_CNT_BLK.
/// Falls back to QEMU isa-debug-exit if ACPI shutdown doesn't work.
pub fn acpi_shutdown(pm1a_control: u32) {
    if pm1a_control != 0 {
        // SLP_TYPa=5 (bits 12:10) | SLP_EN (bit 13)
        let val: u16 = (5 << 10) | (1 << 13);
        unsafe {
            crate::outb(pm1a_control as u16, val as u8);
            crate::outb(pm1a_control as u16 + 1, (val >> 8) as u8);
        }
    }
    // Fallback: QEMU-specific ports.
    unsafe {
        crate::outb(0x604, 0x00);
        crate::outb(0x605, 0x20);
        crate::outb(0x501, 0x31);
    }
}

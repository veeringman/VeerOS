//! SMP (Symmetric Multi-Processing) bring-up for x86-64.
//!
//! Wakes application processors (APs) using the INIT-SIPI-SIPI protocol:
//!   1. Copy the AP trampoline code to a low-memory page (below 1 MiB).
//!   2. Send INIT IPI to the target AP.
//!   3. Wait 10 ms.
//!   4. Send SIPI (Startup IPI) with the trampoline page number.
//!   5. Wait for the AP to signal it has booted.
//!   6. Repeat SIPI if needed.
//!
//! Each AP starts in 16-bit real mode at `page * 0x1000`, transitions
//! through 32-bit protected mode + long mode, then enters the Rust
//! `ap_entry()` function.
//!
//! For now, APs park in a halt loop after reporting in. Per-CPU scheduling
//! can be layered on top.

use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use crate::acpi::MAX_CPUS;
use crate::lapic;

/// Physical address where the AP trampoline is copied (must be < 1 MiB,
/// page-aligned). 0x8000 is a conventional choice (32 KiB mark).
const AP_TRAMPOLINE_ADDR: usize = 0x8000;
/// Page number for the SIPI vector.
const AP_TRAMPOLINE_PAGE: u8 = (AP_TRAMPOLINE_ADDR >> 12) as u8;

/// AP boot status — each AP writes its APIC ID here when it boots.
static AP_BOOT_COUNT: AtomicU32 = AtomicU32::new(0);

/// Per-CPU online status.
static CPU_ONLINE: [AtomicU8; MAX_CPUS] = {
    const INIT: AtomicU8 = AtomicU8::new(0);
    [INIT; MAX_CPUS]
};

/// Per-CPU state.
#[derive(Clone, Copy)]
pub struct PerCpuState {
    pub apic_id: u8,
    pub online: bool,
    pub is_bsp: bool,
}

impl PerCpuState {
    pub const EMPTY: Self = Self { apic_id: 0, online: false, is_bsp: false };
}

/// Global per-CPU state table.
pub static mut CPU_STATE: [PerCpuState; MAX_CPUS] = [PerCpuState::EMPTY; MAX_CPUS];

// ═══════════════════════════════════════════════════════════════════════════
// AP trampoline (16-bit real mode → 64-bit long mode)
// ═══════════════════════════════════════════════════════════════════════════

/// AP trampoline machine code.
///
/// This is position-independent 16-bit code that:
/// 1. Loads a minimal GDT.
/// 2. Enables protected mode (CR0.PE).
/// 3. Enables PAE (CR4.PAE).
/// 4. Loads the kernel's CR3 (PML4).
/// 5. Enables long mode (IA32_EFER.LME).
/// 6. Enables paging (CR0.PG).
/// 7. Far-jumps to 64-bit code.
/// 8. Calls the Rust `_veer_ap_entry` function.
///
/// The trampoline expects these values at fixed offsets from its base:
///   base+0xF0: 64-bit — kernel PML4 physical address (CR3)
///   base+0xF8: 64-bit — address of `_veer_ap_entry` function
///
/// We generate this at compile time as a byte array because the AP starts
/// in real mode where our normal ELF sections aren't accessible.
#[cfg(target_arch = "x86_64")]
static AP_TRAMPOLINE: [u8; 256] = {
    let mut code = [0u8; 256];

    // === 16-bit real mode code (org 0x0000) ===
    // cli
    code[0] = 0xFA;
    // xor ax, ax ; zero data segments
    code[1] = 0x31; code[2] = 0xC0;
    // mov ds, ax
    code[3] = 0x8E; code[4] = 0xD8;
    // mov es, ax
    code[5] = 0x8E; code[6] = 0xC0;
    // mov ss, ax
    code[7] = 0x8E; code[8] = 0xD0;

    // lgdt [cs:GDT_PTR] — GDT pointer is at offset 0x80
    // We use an absolute address since CS base is the trampoline page.
    code[9] = 0x0F; code[10] = 0x01; code[11] = 0x16; // lgdt [0x80]
    code[12] = 0x80; code[13] = 0x00;

    // mov eax, cr0
    code[14] = 0x0F; code[15] = 0x20; code[16] = 0xC0;
    // or al, 1 (PE)
    code[17] = 0x0C; code[18] = 0x01;
    // mov cr0, eax
    code[19] = 0x0F; code[20] = 0x22; code[21] = 0xC0;

    // jmp 0x08:pm_entry (far jump to 32-bit, offset 0x30)
    code[22] = 0x66; code[23] = 0xEA;
    // The absolute target address will be patched: trampoline_base + 0x30
    // When copied to 0x8000, this is 0x00008030.
    code[24] = 0x30; code[25] = 0x80; code[26] = 0x00; code[27] = 0x00;
    code[28] = 0x08; code[29] = 0x00; // segment selector 0x08

    // Padding to offset 0x30.
    // 0x1E..0x30 = padding (already zero)

    // === 32-bit protected mode code (offset 0x30) ===
    // .code32
    // mov ax, 0x10 ; data segment
    code[0x30] = 0x66; code[0x31] = 0xB8; code[0x32] = 0x10; code[0x33] = 0x00;
    // mov ds, ax
    code[0x34] = 0x8E; code[0x35] = 0xD8;
    // mov es, ax
    code[0x36] = 0x8E; code[0x37] = 0xC0;
    // mov ss, ax
    code[0x38] = 0x8E; code[0x39] = 0xD0;

    // Enable PAE (CR4.PAE = bit 5)
    // mov eax, cr4
    code[0x3A] = 0x0F; code[0x3B] = 0x20; code[0x3C] = 0xE0;
    // or eax, 0x20
    code[0x3D] = 0x0D; code[0x3E] = 0x20; code[0x3F] = 0x00; code[0x40] = 0x00; code[0x41] = 0x00;
    // mov cr4, eax
    code[0x42] = 0x0F; code[0x43] = 0x22; code[0x44] = 0xE0;

    // Load CR3 with kernel PML4 (from trampoline+0xF0)
    // mov eax, [0x80F0] — will be patched with trampoline_base + 0xF0
    code[0x45] = 0xA1; code[0x46] = 0xF0; code[0x47] = 0x80; code[0x48] = 0x00; code[0x49] = 0x00;
    // mov cr3, eax
    code[0x4A] = 0x0F; code[0x4B] = 0x22; code[0x4C] = 0xD8;

    // Enable long mode (IA32_EFER.LME = bit 8)
    // mov ecx, 0xC0000080
    code[0x4D] = 0xB9; code[0x4E] = 0x80; code[0x4F] = 0x00; code[0x50] = 0x00; code[0x51] = 0xC0;
    // rdmsr
    code[0x52] = 0x0F; code[0x53] = 0x32;
    // or eax, 0x100
    code[0x54] = 0x0D; code[0x55] = 0x00; code[0x56] = 0x01; code[0x57] = 0x00; code[0x58] = 0x00;
    // wrmsr
    code[0x59] = 0x0F; code[0x5A] = 0x30;

    // Enable paging (CR0.PG = bit 31)
    // mov eax, cr0
    code[0x5B] = 0x0F; code[0x5C] = 0x20; code[0x5D] = 0xC0;
    // or eax, 0x80000000
    code[0x5E] = 0x0D; code[0x5F] = 0x00; code[0x60] = 0x00; code[0x61] = 0x00; code[0x62] = 0x80;
    // mov cr0, eax
    code[0x63] = 0x0F; code[0x64] = 0x22; code[0x65] = 0xC0;

    // Far jump to 64-bit (offset 0x70)
    // jmp 0x08:0x00008070
    code[0x66] = 0xEA;
    code[0x67] = 0x70; code[0x68] = 0x80; code[0x69] = 0x00; code[0x6A] = 0x00;
    code[0x6B] = 0x08; code[0x6C] = 0x00;

    // Padding to 0x70.

    // === 64-bit long mode code (offset 0x70) ===
    // In this section we need raw 64-bit opcodes.
    // mov rax, [rip + (0xF8 - 0x77)] → load _veer_ap_entry address from 0xF8
    // Actually, we use absolute addressing: mov rax, [0x80F8]
    // REX.W=1 (0x48), mov rax, moffs64 (0xA1), then 8-byte address
    code[0x70] = 0x48; code[0x71] = 0xA1;
    code[0x72] = 0xF8; code[0x73] = 0x80; code[0x74] = 0x00; code[0x75] = 0x00;
    code[0x76] = 0x00; code[0x77] = 0x00; code[0x78] = 0x00; code[0x79] = 0x00;
    // jmp rax
    code[0x7A] = 0xFF; code[0x7B] = 0xE0;

    // === GDT and GDT pointer at offset 0x80 ===
    // GDT pointer (6 bytes): limit (2) + base (4)
    // limit = 3*8 - 1 = 23 = 0x17
    code[0x80] = 0x17; code[0x81] = 0x00;
    // base = trampoline + 0x88 = 0x8088 (patched)
    code[0x82] = 0x88; code[0x83] = 0x80; code[0x84] = 0x00; code[0x85] = 0x00;

    // GDT entries at offset 0x88 (3 entries × 8 bytes = 24 bytes)
    // Entry 0: null (already zero)
    // Entry 1 (0x90): 64-bit code (0x00AF9A000000FFFF)
    code[0x90] = 0xFF; code[0x91] = 0xFF; code[0x92] = 0x00; code[0x93] = 0x00;
    code[0x94] = 0x00; code[0x95] = 0x9A; code[0x96] = 0xAF; code[0x97] = 0x00;
    // Entry 2 (0x98): data (0x00CF92000000FFFF)
    code[0x98] = 0xFF; code[0x99] = 0xFF; code[0x9A] = 0x00; code[0x9B] = 0x00;
    code[0x9C] = 0x00; code[0x9D] = 0x92; code[0x9E] = 0xCF; code[0x9F] = 0x00;

    // Data area at the end:
    // 0xF0: CR3 value (u64) — filled at runtime
    // 0xF8: _veer_ap_entry address (u64) — filled at runtime

    code
};

// Offset of data fields in the trampoline.
const TRAMP_CR3_OFFSET: usize = 0xF0;
const TRAMP_ENTRY_OFFSET: usize = 0xF8;

// ═══════════════════════════════════════════════════════════════════════════
// AP entry point (called from trampoline in 64-bit mode)
// ═══════════════════════════════════════════════════════════════════════════

/// Stack for each AP (4 KiB per core).
const AP_STACK_SIZE: usize = 4096;
#[repr(align(16))]
struct ApStacks([[u8; AP_STACK_SIZE]; MAX_CPUS]);
static mut AP_STACKS: ApStacks = ApStacks([[0u8; AP_STACK_SIZE]; MAX_CPUS]);

/// AP entry point — called by each application processor after long-mode transition.
///
/// At this point the AP has:
/// - 64-bit mode with paging enabled
/// - Shared page tables with the BSP (same CR3)
/// - No stack set up yet (we set it here)
#[unsafe(no_mangle)]
pub extern "C" fn _veer_ap_entry() -> ! {
    // Read this AP's LAPIC ID.
    let apic_id = lapic::id() as u8;

    // Set up a per-CPU stack.
    let cpu_idx = AP_BOOT_COUNT.fetch_add(1, Ordering::SeqCst) as usize + 1; // +1 because BSP is 0
    let stack_top = unsafe {
        AP_STACKS.0[cpu_idx].as_ptr().add(AP_STACK_SIZE)
    };

    // Switch to the per-CPU stack.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "mov rsp, {}",
            in(reg) stack_top as u64,
            options(nostack),
        );
    }

    // Initialize this AP's LAPIC.
    lapic::init();

    // Mark this CPU as online.
    if (cpu_idx) < MAX_CPUS {
        CPU_ONLINE[cpu_idx].store(1, Ordering::Release);
        unsafe {
            CPU_STATE[cpu_idx] = PerCpuState {
                apic_id,
                online: true,
                is_bsp: false,
            };
        }
    }

    // Park — APs spin in halt loop until per-CPU scheduler is added.
    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "x86_64"))]
        core::hint::spin_loop();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SMP bring-up
// ═══════════════════════════════════════════════════════════════════════════

/// Copy the AP trampoline to low memory and patch the data fields.
#[cfg(target_arch = "x86_64")]
fn install_trampoline() {
    unsafe {
        // Copy trampoline code to AP_TRAMPOLINE_ADDR.
        let dst = AP_TRAMPOLINE_ADDR as *mut u8;
        let src = AP_TRAMPOLINE.as_ptr();
        core::ptr::copy_nonoverlapping(src, dst, AP_TRAMPOLINE.len());

        // Patch CR3 value.
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
        let cr3_ptr = (AP_TRAMPOLINE_ADDR + TRAMP_CR3_OFFSET) as *mut u64;
        cr3_ptr.write_volatile(cr3);

        // Patch entry function address.
        let entry_ptr = (AP_TRAMPOLINE_ADDR + TRAMP_ENTRY_OFFSET) as *mut u64;
        entry_ptr.write_volatile(_veer_ap_entry as *const () as u64);
    }
}

/// Delay roughly `ms` milliseconds using a spin loop.
/// Very approximate — calibrated for ~1 GHz processors.
fn delay_ms(ms: u32) {
    for _ in 0..ms {
        for _ in 0..100_000u32 {
            core::hint::spin_loop();
        }
    }
}

/// Bring up application processors.
///
/// `ap_apic_ids` — slice of APIC IDs for APs (from ACPI MADT).
/// `bsp_apic_id` — the bootstrap processor's APIC ID.
///
/// Returns the number of APs that successfully booted.
#[cfg(target_arch = "x86_64")]
pub fn bring_up_aps(ap_apic_ids: &[u8], bsp_apic_id: u8) -> usize {
    // Set up BSP state.
    unsafe {
        CPU_STATE[0] = PerCpuState {
            apic_id: bsp_apic_id,
            online: true,
            is_bsp: true,
        };
    }
    CPU_ONLINE[0].store(1, Ordering::Release);

    if ap_apic_ids.is_empty() {
        return 0;
    }

    // Install the trampoline.
    install_trampoline();

    let mut booted = 0usize;

    for &ap_id in ap_apic_ids.iter() {
        if ap_id == bsp_apic_id {
            continue; // Skip BSP
        }

        let before = AP_BOOT_COUNT.load(Ordering::Acquire);

        // Send INIT IPI.
        lapic::send_init_ipi(ap_id);
        delay_ms(10);

        // Send first SIPI.
        lapic::send_sipi(ap_id, AP_TRAMPOLINE_PAGE);
        delay_ms(1);

        // Check if AP booted.
        if AP_BOOT_COUNT.load(Ordering::Acquire) == before {
            // Send second SIPI (per Intel spec).
            lapic::send_sipi(ap_id, AP_TRAMPOLINE_PAGE);
            delay_ms(1);
        }

        // Wait up to 100 ms for the AP to respond.
        let mut waited = 0u32;
        while AP_BOOT_COUNT.load(Ordering::Acquire) == before && waited < 100 {
            delay_ms(1);
            waited += 1;
        }

        if AP_BOOT_COUNT.load(Ordering::Acquire) > before {
            booted += 1;
        }
    }

    booted
}

/// Number of CPUs that have booted (BSP + APs).
pub fn online_cpu_count() -> usize {
    1 + AP_BOOT_COUNT.load(Ordering::Acquire) as usize
}

/// Non-x86 stubs.
#[cfg(not(target_arch = "x86_64"))]
pub fn bring_up_aps(_ap_apic_ids: &[u8], _bsp_apic_id: u8) -> usize { 0 }

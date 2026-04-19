//! x86-64 trap infrastructure: GDT, IDT, ISR stubs, and dispatch logic.
//!
//! The ISR stubs save all 16 GPRs into a `TrapFrame` on the stack, call the
//! Rust dispatcher, and restore from the (possibly modified) frame.

#[allow(unused_imports)]
use arch::{SavedContext, Serial, TaskContext, TickTimer};
#[allow(unused_imports)]
use microkernel::task::Scheduler;
#[allow(unused_imports)]
use microkernel::dispatch::{self, SyscallAction};

#[allow(unused_imports)]
use crate::{SCHEDULER, TIMER, IPC, HEAP, FUTEX, CHANNELS, POLL, PROCESSES,
            SOCKETS, USERS, INODES, RAMFS, FAT32S, MOUNTS, INPUT, DRIVERS, AUDIT,
            AGENTS, INTENTS, MEMORY_ENGINE, FABRIC, INTENT_SCHED};

// ═══════════════════════════════════════════════════════════════════════════
// Trap frame — matches the ISR stub push order
// ═══════════════════════════════════════════════════════════════════════════

/// Trap frame layout on the kernel stack after ISR common has pushed all GPRs.
///
/// The ISR stub pushes (bottom → top): vector, error_code (or 0),
/// then `isr_common` pushes rax, rcx, rdx, rbx, rsp_scratch, rbp, rsi, rdi,
/// r8…r15.  The CPU-pushed interrupt frame sits above the error code.
#[repr(C)]
pub struct TrapFrame {
    // Pushed by isr_common (stack grows down, so first push = highest offset).
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rax: u64,
    // Pushed by ISR stub.
    pub vector: u64,
    pub error_code: u64,
    // Pushed by CPU on interrupt/exception.
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

// ═══════════════════════════════════════════════════════════════════════════
// GDT — Global Descriptor Table (with TSS for Ring 3 support)
// ═══════════════════════════════════════════════════════════════════════════

/// GDT selectors:
///   0x00: null
///   0x08: 64-bit kernel code (Ring 0, L=1)
///   0x10: kernel data (Ring 0)
///   0x18: user data (Ring 3)    — must precede user code for SYSRET
///   0x20: 64-bit user code (Ring 3, L=1)
///   0x28: TSS descriptor (16 bytes, spans entries [5] and [6])
const GDT_ENTRIES: usize = 7;

/// Selector constants.
pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const USER_DS: u16 = 0x18 | 3;   // RPL=3
pub const USER_CS: u16 = 0x20 | 3;   // RPL=3
pub const TSS_SEL: u16 = 0x28;

/// x86-64 Task State Segment (104 bytes).
#[repr(C, packed)]
pub struct Tss {
    _reserved0: u32,
    /// RSP for privilege level 0 (kernel stack on Ring 3→0 transition).
    pub rsp0: u64,
    /// RSP for privilege level 1.
    pub rsp1: u64,
    /// RSP for privilege level 2.
    pub rsp2: u64,
    _reserved1: u64,
    /// Interrupt Stack Table entries (IST1–IST7).
    pub ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    /// I/O Map Base Address (offset from TSS base).
    pub iopb: u16,
}

impl Tss {
    pub const fn new() -> Self {
        Self {
            _reserved0: 0,
            rsp0: 0,
            rsp1: 0,
            rsp2: 0,
            _reserved1: 0,
            ist: [0; 7],
            _reserved2: 0,
            _reserved3: 0,
            iopb: 104, // no I/O bitmap — point past end
        }
    }
}

/// Static TSS (single-core).
static mut TSS: Tss = Tss::new();

/// Interrupt stack for IST1 (NMI / double fault / MCE).
#[repr(align(16))]
struct IstStack([u8; 4096]);
static mut IST1_STACK: IstStack = IstStack([0; 4096]);

/// Flat 64-bit GDT with Ring 3 segments and TSS.
#[repr(C, align(16))]
struct Gdt {
    entries: [u64; GDT_ENTRIES],
}

/// GDTR value (packed pointer + limit).
#[repr(C, packed)]
struct GdtPtr {
    limit: u16,
    base: u64,
}

static mut GDT: Gdt = Gdt {
    entries: [
        0x0000_0000_0000_0000, // 0x00: null
        0x00AF_9A00_0000_FFFF, // 0x08: 64-bit kernel code (DPL=0, L=1, P=1)
        0x00CF_9200_0000_FFFF, // 0x10: kernel data (DPL=0, P=1)
        0x00CF_F200_0000_FFFF, // 0x18: user data (DPL=3, P=1)
        0x00AF_FA00_0000_FFFF, // 0x20: 64-bit user code (DPL=3, L=1, P=1)
        0x0000_0000_0000_0000, // 0x28: TSS low (filled at runtime)
        0x0000_0000_0000_0000, // 0x30: TSS high (filled at runtime)
    ],
};

/// Load the GDT, set segment registers, install TSS. Call once at boot.
#[cfg(target_arch = "x86_64")]
pub fn load_gdt() {
    unsafe {
        // ── Fill TSS fields ──────────────────────────────
        // Set IST1 to the top of our dedicated stack.
        TSS.ist[0] = (&IST1_STACK.0 as *const u8 as u64) + 4096;
        // RSP0 will be set per-task on context switch; use boot stack for now.
        extern "C" {
            static __stack_top: u8;
        }
        TSS.rsp0 = &__stack_top as *const u8 as u64;

        // ── Build TSS descriptor (16-byte system segment) ────
        let tss_addr = &TSS as *const Tss as u64;
        let tss_len = (core::mem::size_of::<Tss>() - 1) as u64;

        // Low 8 bytes of TSS descriptor.
        let low: u64 = (tss_len & 0xFFFF)                    // limit[15:0]
            | ((tss_addr & 0xFFFF) << 16)                     // base[15:0]
            | (((tss_addr >> 16) & 0xFF) << 32)               // base[23:16]
            | (0x89u64 << 40)                                  // type=9 (64-bit TSS avail), P=1
            | (((tss_len >> 16) & 0xF) << 48)                 // limit[19:16]
            | (((tss_addr >> 24) & 0xFF) << 56);               // base[31:24]

        // High 8 bytes: base[63:32] + reserved.
        let high: u64 = (tss_addr >> 32) & 0xFFFF_FFFF;

        GDT.entries[5] = low;
        GDT.entries[6] = high;

        // ── Load GDT ────────────────────────────────────
        let ptr = GdtPtr {
            limit: (core::mem::size_of_val(&GDT.entries) - 1) as u16,
            base: GDT.entries.as_ptr() as u64,
        };
        core::arch::asm!(
            "lgdt [{0}]",
            // Reload CS via far return.
            "push 0x08",
            "lea rax, [rip + 2f]",
            "push rax",
            "retfq",
            "2:",
            // Reload data segment registers.
            "mov ax, 0x10",
            "mov ds, ax",
            "mov es, ax",
            "mov fs, ax",
            "mov gs, ax",
            "mov ss, ax",
            in(reg) &ptr,
            options(nostack),
        );

        // ── Load TSS ────────────────────────────────────
        core::arch::asm!(
            "ltr {0:x}",
            in(reg) TSS_SEL,
            options(nostack, nomem),
        );
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn load_gdt() {}

/// Update TSS.rsp0 and the SYSCALL kernel RSP with a new kernel stack pointer.
/// Called on context switch to ensure Ring 3→0 transitions use the correct stack.
pub fn set_kernel_stack(rsp0: u64) {
    unsafe {
        TSS.rsp0 = rsp0;
        // Also update the global used by the SYSCALL entry stub.
        extern "C" {
            static mut _veer_kernel_rsp: u64;
        }
        _veer_kernel_rsp = rsp0;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// SYSCALL / SYSRET MSR setup
// ═══════════════════════════════════════════════════════════════════════════

/// MSR addresses for SYSCALL/SYSRET.
const IA32_STAR: u32 = 0xC000_0081;
const IA32_LSTAR: u32 = 0xC000_0082;
const IA32_FMASK: u32 = 0xC000_0084;
const IA32_EFER: u32 = 0xC000_0080;

/// Write a Model-Specific Register.
#[cfg(target_arch = "x86_64")]
unsafe fn wrmsr(msr: u32, val: u64) {
    let lo = val as u32;
    let hi = (val >> 32) as u32;
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") lo,
            in("edx") hi,
            options(nomem, nostack),
        );
    }
}

/// Read a Model-Specific Register.
#[cfg(target_arch = "x86_64")]
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack),
        );
    }
    (hi as u64) << 32 | lo as u64
}

/// Set up the SYSCALL/SYSRET fast path MSRs.
///
/// After this:
/// - `syscall` from Ring 3 enters `_veer_syscall_entry` at Ring 0
/// - `sysret` returns to Ring 3 with the saved RIP/RFLAGS
///
/// STAR layout:
///   bits 47:32 = kernel CS (SYSCALL loads CS from here, SS from +8)
///   bits 63:48 = user CS base (SYSRET loads CS from here+16, SS from here+8)
///
/// For our GDT layout:
///   SYSCALL: kernel CS = 0x08, kernel SS = CS+8 = 0x10  ✓
///   SYSRET:  user base = 0x18 → user CS = 0x18+16 = 0x28… but SYSRET does +16 for CS
///            Actually: SYSRET 64-bit loads CS = STAR[63:48]+16, SS = STAR[63:48]+8
///            So user base = 0x10: CS = 0x10+16 = 0x20 ✓, SS = 0x10+8 = 0x18 ✓
#[cfg(target_arch = "x86_64")]
pub fn setup_syscall_msrs() {
    unsafe {
        // Enable SCE (System Call Extensions) in IA32_EFER.
        let efer = rdmsr(IA32_EFER);
        wrmsr(IA32_EFER, efer | 1); // bit 0 = SCE

        // STAR: kernel selectors in [47:32], user base in [63:48].
        // Kernel CS = 0x08, user base = 0x10
        let star: u64 = (0x08u64 << 32) | (0x10u64 << 48);
        wrmsr(IA32_STAR, star);

        // LSTAR: entry point for SYSCALL instruction.
        extern "C" {
            fn _veer_syscall_entry();
        }
        wrmsr(IA32_LSTAR, _veer_syscall_entry as u64);

        // FMASK: bits to clear in RFLAGS on SYSCALL entry.
        // Clear IF (bit 9) + DF (bit 10) + TF (bit 8) + AC (bit 18).
        let fmask: u64 = (1 << 9) | (1 << 10) | (1 << 8) | (1 << 18);
        wrmsr(IA32_FMASK, fmask);
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn setup_syscall_msrs() {}

// ═══════════════════════════════════════════════════════════════════════════
// IDT — Interrupt Descriptor Table
// ═══════════════════════════════════════════════════════════════════════════

/// Single IDT entry (interrupt gate, 16 bytes in long mode).
#[derive(Clone, Copy)]
#[repr(C)]
struct IdtEntry {
    offset_lo: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_hi: u32,
    _reserved: u32,
}

impl IdtEntry {
    const EMPTY: Self = Self {
        offset_lo: 0,
        selector: 0,
        ist: 0,
        type_attr: 0,
        offset_mid: 0,
        offset_hi: 0,
        _reserved: 0,
    };

    /// Create an interrupt gate entry (DPL=0, present, 64-bit interrupt gate).
    fn new(handler: u64) -> Self {
        Self {
            offset_lo: handler as u16,
            selector: 0x08, // kernel CS
            ist: 0,
            type_attr: 0x8E, // P=1, DPL=00, type=1110 (interrupt gate)
            offset_mid: (handler >> 16) as u16,
            offset_hi: (handler >> 32) as u32,
            _reserved: 0,
        }
    }
}

/// IDTR value (packed pointer + limit).
#[repr(C, packed)]
struct IdtPtr {
    limit: u16,
    base: u64,
}

/// The IDT has 256 entries.
const IDT_ENTRIES: usize = 256;

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry::EMPTY; IDT_ENTRIES];

/// Install the IDT. Must be called after `load_gdt()`.
#[cfg(target_arch = "x86_64")]
pub fn load_idt() {
    // Fill IDT entries from the stub table generated by assembly.
    extern "C" {
        /// Array of 256 ISR stub function pointers, defined in assembly.
        static _veer_isr_table: [u64; IDT_ENTRIES];
    }

    unsafe {
        for i in 0..IDT_ENTRIES {
            let handler = *_veer_isr_table.as_ptr().add(i);
            if handler != 0 {
                IDT[i] = IdtEntry::new(handler);
            }
        }

        let ptr = IdtPtr {
            limit: (core::mem::size_of_val(&IDT) - 1) as u16,
            base: IDT.as_ptr() as u64,
        };
        core::arch::asm!("lidt [{0}]", in(reg) &ptr, options(nostack));
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn load_idt() {}

/// Enable hardware interrupts.
#[cfg(target_arch = "x86_64")]
pub fn enable_interrupts() {
    unsafe { core::arch::asm!("sti", options(nostack, nomem)); }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn enable_interrupts() {}

// ═══════════════════════════════════════════════════════════════════════════
// ISR stub assembly — generates 256 stub functions + table
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(r#"
// ---- ISR common: save all GPRs, call Rust, restore, iretq ----
.section .text
.balign 16
_veer_isr_common:
    // Save all GPRs (order must match TrapFrame layout).
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    // First argument: pointer to TrapFrame on the stack.
    mov  rdi, rsp
    // Call the Rust dispatcher.
    call _veer_trap_dispatch_x86

    // rax = pointer to TrapFrame to restore (may differ on context switch).
    mov  rsp, rax

    // Restore GPRs.
    pop  r15
    pop  r14
    pop  r13
    pop  r12
    pop  r11
    pop  r10
    pop  r9
    pop  r8
    pop  rdi
    pop  rsi
    pop  rbp
    pop  rbx
    pop  rdx
    pop  rcx
    pop  rax

    // Skip vector + error_code.
    add  rsp, 16

    iretq

// ---- Macro: generate ISR stub without error code ----
.macro ISR_NOERR num
.balign 16
_veer_isr_\num:
    push 0          // fake error code
    push \num       // vector number
    jmp  _veer_isr_common
.endm

// ---- Macro: generate ISR stub with error code (CPU pushes it) ----
.macro ISR_ERR num
.balign 16
_veer_isr_\num:
    // error code already pushed by CPU
    push \num       // vector number
    jmp  _veer_isr_common
.endm

// ---- Generate all 256 ISR stubs ----
// Exceptions 0–31: some have error codes, most don't.
ISR_NOERR 0
ISR_NOERR 1
ISR_NOERR 2
ISR_NOERR 3
ISR_NOERR 4
ISR_NOERR 5
ISR_NOERR 6
ISR_NOERR 7
ISR_ERR   8    // Double Fault
ISR_NOERR 9
ISR_ERR   10   // Invalid TSS
ISR_ERR   11   // Segment Not Present
ISR_ERR   12   // Stack-Segment Fault
ISR_ERR   13   // General Protection Fault
ISR_ERR   14   // Page Fault
ISR_NOERR 15
ISR_NOERR 16
ISR_ERR   17   // Alignment Check
ISR_NOERR 18
ISR_NOERR 19
ISR_NOERR 20
ISR_ERR   21   // Control Protection
ISR_NOERR 22
ISR_NOERR 23
ISR_NOERR 24
ISR_NOERR 25
ISR_NOERR 26
ISR_NOERR 27
ISR_NOERR 28
ISR_ERR   29   // VMM Communication
ISR_ERR   30   // Security Exception
ISR_NOERR 31

// IRQs 32–47 (remapped PIC) + software vectors 48–255.
.altmacro
.set i, 32
.rept 224
ISR_NOERR %i
.set i, i+1
.endr

// ---- ISR table: array of 256 function pointers ----
// Wrapper macro so we can use .altmacro %i expansion for label names.
.macro EMIT_ISR_QUAD num
    .quad _veer_isr_\num
.endm

// Keep in .text so local ISR stub labels are resolvable.
.section .text
.balign 8
.global _veer_isr_table
_veer_isr_table:
.set i, 0
.rept 256
EMIT_ISR_QUAD %i
.set i, i+1
.endr
.noaltmacro

// ---- SYSCALL fast-path entry point ----
// On `syscall` from Ring 3:
//   RCX = saved user RIP
//   R11 = saved user RFLAGS
//   RSP = still user RSP (must swap to kernel stack)
//   CS  = kernel CS (from STAR MSR)
//   SS  = kernel SS (from STAR MSR)
//
// We save the user state, set up a TrapFrame-like structure, and call the
// Rust syscall handler. On return we restore via `sysretq`.
.balign 16
.global _veer_syscall_entry
_veer_syscall_entry:
    // Swap to kernel stack.
    // We store user RSP in R10 temporarily, load kernel RSP from TSS.rsp0.
    // (GS_BASE-based per-CPU data would be better for SMP; for now use
    //  a simple global since we're single-core.)
    mov  r10, rsp              // save user RSP
    // Load kernel RSP from TSS.rsp0. TSS address is in our known static.
    // For simplicity, use a dedicated global (set by context switch).
    mov  rsp, qword ptr [_veer_kernel_rsp]

    // Build a TrapFrame on the kernel stack.
    // CPU-pushed frame: SS, RSP, RFLAGS, CS, RIP
    push 0x1B            // SS (user DS = 0x18 | 3 = 0x1B)
    push r10             // user RSP
    push r11             // user RFLAGS (saved by SYSCALL in R11)
    push 0x23            // CS (user CS = 0x20 | 3 = 0x23)
    push rcx             // user RIP (saved by SYSCALL in RCX)

    // ISR stub fields.
    push 0               // error_code
    push 0x80            // vector = 0x80 (syscall)

    // GPRs (same order as isr_common).
    push rax
    push rcx
    push rdx
    push rbx
    push rbp             // placeholder for rsp_scratch
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15

    // Call the Rust dispatcher (same as for int 0x80).
    mov  rdi, rsp
    call _veer_trap_dispatch_x86

    // rax = pointer to (possibly new) TrapFrame.
    mov  rsp, rax

    // Restore GPRs.
    pop  r15
    pop  r14
    pop  r13
    pop  r12
    pop  r11
    pop  r10
    pop  r9
    pop  r8
    pop  rdi
    pop  rsi
    pop  rbp
    add  rsp, 8          // skip rsp_scratch
    pop  rbx
    pop  rdx
    pop  rcx
    pop  rax

    // Skip vector + error_code.
    add  rsp, 16

    // Restore RIP → RCX, skip CS, RFLAGS → R11, RSP → RSP, skip SS.
    pop  rcx             // user RIP
    add  rsp, 8          // skip CS
    pop  r11             // user RFLAGS
    pop  rsp             // user RSP (skip SS implicitly — sysretq sets it)

    sysretq

// Kernel RSP for SYSCALL entry (updated on context switch).
.section .data
.balign 8
.global _veer_kernel_rsp
_veer_kernel_rsp:
    .quad 0
"#);

// ═══════════════════════════════════════════════════════════════════════════
// Context switch helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Copy a TrapFrame's register values into a task's X86_64Context.
///
/// In 64-bit long mode the CPU always pushes all 5 values on interrupt:
/// SS, RSP, RFLAGS, CS, RIP — even for same-privilege (Ring 0) interrupts.
/// Likewise, IRETQ always pops all 5.
#[cfg(target_arch = "x86_64")]
unsafe fn save_frame_to_context(frame: &TrapFrame, ctx: &mut TaskContext) {
    ctx.gpr[0]  = frame.rax as usize;  // rax
    ctx.gpr[1]  = frame.rcx as usize;  // rcx
    ctx.gpr[2]  = frame.rdx as usize;  // rdx
    ctx.gpr[3]  = frame.rbx as usize;  // rbx
    // In 64-bit mode the CPU always pushes RSP on interrupt.
    ctx.gpr[4]  = frame.rsp as usize;  // rsp
    ctx.gpr[5]  = frame.rbp as usize;  // rbp
    ctx.gpr[6]  = frame.rsi as usize;  // rsi
    ctx.gpr[7]  = frame.rdi as usize;  // rdi
    ctx.gpr[8]  = frame.r8  as usize;  // r8
    ctx.gpr[9]  = frame.r9  as usize;  // r9
    ctx.gpr[10] = frame.r10 as usize;  // r10
    ctx.gpr[11] = frame.r11 as usize;  // r11
    ctx.gpr[12] = frame.r12 as usize;  // r12
    ctx.gpr[13] = frame.r13 as usize;  // r13
    ctx.gpr[14] = frame.r14 as usize;  // r14
    ctx.gpr[15] = frame.r15 as usize;  // r15
    ctx.rip     = frame.rip as usize;
    ctx.rflags  = frame.rflags as usize;
}

/// Build an interrupt frame on the given task's stack and return a
/// pointer to the new TrapFrame.
///
/// In 64-bit long mode, IRETQ always pops 5 values: RIP, CS, RFLAGS, RSP, SS.
/// The CPU also always pushes all 5 on interrupt entry, even for same-privilege.
///
/// Layout (stack grows down, 176 bytes = full TrapFrame):
///   ss (8), rsp (8), rflags (8), cs (8), rip (8)  ← IRETQ frame
///   error_code (8, = 0), vector (8, = 0)           ← isr_common bookkeeping
///   rax..r15 (15 × 8 = 120)                       ← GPRs from context
///
/// `task_rsp`: the saved RSP of the task. After IRETQ this becomes the new RSP.
#[cfg(target_arch = "x86_64")]
unsafe fn build_ring0_frame(ctx: &TaskContext, task_rsp: usize) -> *mut TrapFrame {
    // Full TrapFrame = 176 bytes (22 × u64).
    let frame_ptr = (task_rsp - 176) as *mut TrapFrame;
    let f = unsafe { &mut *frame_ptr };

    // IRETQ frame (all 5 words — mandatory in 64-bit mode).
    f.rip    = ctx.rip     as u64;
    f.cs     = KERNEL_CS   as u64;
    f.rflags = ctx.rflags  as u64;
    f.rsp    = task_rsp    as u64;  // RSP after IRETQ
    f.ss     = KERNEL_DS   as u64;  // SS for kernel

    // ISR bookkeeping fields.
    f.vector     = 0;
    f.error_code = 0;

    // GPRs.
    f.rax = ctx.gpr[0]  as u64;
    f.rcx = ctx.gpr[1]  as u64;
    f.rdx = ctx.gpr[2]  as u64;
    f.rbx = ctx.gpr[3]  as u64;
    f.rbp = ctx.gpr[5]  as u64;
    f.rsi = ctx.gpr[6]  as u64;
    f.rdi = ctx.gpr[7]  as u64;
    f.r8  = ctx.gpr[8]  as u64;
    f.r9  = ctx.gpr[9]  as u64;
    f.r10 = ctx.gpr[10] as u64;
    f.r11 = ctx.gpr[11] as u64;
    f.r12 = ctx.gpr[12] as u64;
    f.r13 = ctx.gpr[13] as u64;
    f.r14 = ctx.gpr[14] as u64;
    f.r15 = ctx.gpr[15] as u64;

    frame_ptr
}

/// Write a task's X86_64Context back into a TrapFrame for iretq.
#[cfg(target_arch = "x86_64")]
unsafe fn restore_context_to_frame(ctx: &TaskContext, frame: &mut TrapFrame) {
    frame.rax    = ctx.gpr[0]  as u64;
    frame.rcx    = ctx.gpr[1]  as u64;
    frame.rdx    = ctx.gpr[2]  as u64;
    frame.rbx    = ctx.gpr[3]  as u64;
    frame.rsp    = ctx.gpr[4]  as u64;  // Ring 3 only — restored by iretq
    frame.rbp    = ctx.gpr[5]  as u64;
    frame.rsi    = ctx.gpr[6]  as u64;
    frame.rdi    = ctx.gpr[7]  as u64;
    frame.r8     = ctx.gpr[8]  as u64;
    frame.r9     = ctx.gpr[9]  as u64;
    frame.r10    = ctx.gpr[10] as u64;
    frame.r11    = ctx.gpr[11] as u64;
    frame.r12    = ctx.gpr[12] as u64;
    frame.r13    = ctx.gpr[13] as u64;
    frame.r14    = ctx.gpr[14] as u64;
    frame.r15    = ctx.gpr[15] as u64;
    frame.rip    = ctx.rip     as u64;
    frame.rflags = ctx.rflags  as u64;
    // Set segment selectors based on kernel_word:
    // kernel_word == 0 → kernel task (Ring 0), != 0 → user task (Ring 3).
    if ctx.kernel_word == 0 {
        frame.cs = KERNEL_CS as u64;
        frame.ss = KERNEL_DS as u64;
    } else {
        frame.cs = USER_CS as u64;
        frame.ss = USER_DS as u64;
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Vector constants
// ═══════════════════════════════════════════════════════════════════════════

/// PIT timer interrupt (IRQ 0 remapped to vector 32).
#[allow(dead_code)]
const VEC_TIMER: u64 = 32;
/// PS/2 keyboard interrupt (IRQ 1 remapped to vector 33).
#[allow(dead_code)]
const VEC_KEYBOARD: u64 = 33;
/// COM1 serial interrupt (IRQ 4 remapped to vector 36).
#[allow(dead_code)]
const VEC_COM1: u64 = 36;
/// Syscall vector (int 0x80).
#[allow(dead_code)]
const VEC_SYSCALL: u64 = 0x80;
/// Page fault vector.
#[allow(dead_code)]
const VEC_PAGE_FAULT: u64 = 14;
/// LAPIC spurious interrupt vector.
#[allow(dead_code)]
const VEC_SPURIOUS: u64 = 0xFF;

// ═══════════════════════════════════════════════════════════════════════════
// Rust trap dispatcher — called from _veer_isr_common
// ═══════════════════════════════════════════════════════════════════════════

/// Main dispatch entry point called from the ISR common stub.
///
/// Receives a pointer to the TrapFrame on the stack.
/// Returns a pointer to the TrapFrame to restore (may differ for context switch).
#[cfg(target_arch = "x86_64")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _veer_trap_dispatch_x86(frame: *mut TrapFrame) -> *mut TrapFrame {
    let f = unsafe { &mut *frame };
    let vector = f.vector;

    match vector {
        VEC_TIMER => handle_timer_tick(frame),
        VEC_KEYBOARD => handle_keyboard_irq(frame),
        VEC_COM1 => handle_com1_irq(frame),
        VEC_SYSCALL => handle_syscall(frame),
        VEC_PAGE_FAULT => handle_page_fault(frame),
        VEC_SPURIOUS => frame, // spurious — no EOI
        0..=31 => handle_exception(frame),
        _ => {
            // Unknown vector — send EOI to both PIC and LAPIC.
            if vector >= 32 && vector < 48 {
                soc_qemu_pc::pic::send_eoi((vector - 32) as u8);
            }
            soc_qemu_pc::lapic::eoi();
            frame
        }
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn handle_timer_tick(frame: *mut TrapFrame) -> *mut TrapFrame {
    // Send EOI to LAPIC (we're in IOAPIC mode, PIC is disabled).
    soc_qemu_pc::lapic::eoi();
    let timer = unsafe { &*TIMER.0.get() };
    timer.clear_pending();

    let sched: &mut Scheduler = unsafe { &mut *SCHEDULER.0.get() };

    // Save the current trap frame into the running task's context.
    // For Ring-0 tasks, the correct RSP is recovered inside save_frame_to_context.
    let f = unsafe { &*frame };
    unsafe { save_frame_to_context(f, &mut sched.tasks[sched.current].context); }

    let need_switch = sched.tick();
    dispatch::wake_sleepers(sched);

    // Wake poll-blocked tasks.
    let ipc = unsafe { &*IPC.0.get() };
    let channels = unsafe { &*CHANNELS.0.get() };
    let poll = unsafe { &mut *POLL.0.get() };
    microkernel::poll::wake_poll_waiters(poll, sched, ipc, channels);

    if need_switch {
        // tick() already picked the next task and set sched.current.
        let next = sched.current;
        let next_ctx = &sched.tasks[next].context;
        let next_rsp = next_ctx.gpr[4]; // saved RSP for the next task

        // Build a Ring-0 interrupt frame on the next task's own stack.
        // isr_common will do `mov rsp, rax` with this pointer, then pop
        // all GPRs, skip error+vector, and iretq into the next task.
        let new_frame = unsafe { build_ring0_frame(next_ctx, next_rsp) };

        // Keep _veer_kernel_rsp pointing at the new task's stack top.
        set_kernel_stack((sched.tasks[next].stack_bottom + sched.tasks[next].stack_size) as u64);

        return new_frame;
    }
    frame
}

#[cfg(target_arch = "x86_64")]
unsafe fn handle_keyboard_irq(frame: *mut TrapFrame) -> *mut TrapFrame {
    // Read all pending scan codes from PS/2 controller.
    while soc_qemu_pc::ps2kbd::has_data() {
        let scancode = soc_qemu_pc::ps2kbd::read_scancode();
        if let Some(ascii) = soc_qemu_pc::ps2kbd::scancode_to_ascii(scancode) {
            crate::kbd_buffer_push(ascii);
        }
    }
    soc_qemu_pc::lapic::eoi();
    frame
}

#[cfg(target_arch = "x86_64")]
unsafe fn handle_com1_irq(frame: *mut TrapFrame) -> *mut TrapFrame {
    // Read all pending bytes from COM1 serial RX.
    let serial = soc_qemu_pc::default_serial();
    while serial.has_data() {
        let byte = unsafe {
            soc_qemu_pc::inb(0x3F8)  // read RBR directly to avoid blocking
        };
        crate::kbd_buffer_push(byte);
    }
    soc_qemu_pc::lapic::eoi();
    frame
}

#[cfg(target_arch = "x86_64")]
unsafe fn handle_syscall(frame: *mut TrapFrame) -> *mut TrapFrame {
    let sched = unsafe { &mut *SCHEDULER.0.get() };

    // Save trap frame into the current task's context.
    let f = unsafe { &*frame };
    unsafe { save_frame_to_context(f, &mut sched.tasks[sched.current].context); }

    let ipc = unsafe { &mut *IPC.0.get() };
    let heap = unsafe { &mut *HEAP.0.get() };
    let futex = unsafe { &mut *FUTEX.0.get() };
    let channels = unsafe { &mut *CHANNELS.0.get() };
    let poll = unsafe { &mut *POLL.0.get() };
    let processes = unsafe { &mut *PROCESSES.0.get() };
    let sockets = unsafe { &mut *SOCKETS.0.get() };
    let users = unsafe { &mut *USERS.0.get() };
    let inodes = unsafe { &mut *INODES.0.get() };
    let ramfs = unsafe { &mut *RAMFS.0.get() };
    let fat32s: &mut [microkernel::fat32::Fat32; microkernel::fat32::MAX_FAT32] = unsafe {
        &mut *(core::ptr::addr_of!(FAT32S) as *const _ as *mut [microkernel::fat32::Fat32; microkernel::fat32::MAX_FAT32])
    };
    let mounts = unsafe { &mut *MOUNTS.0.get() };
    let input = unsafe { &mut *INPUT.0.get() };
    let drivers = unsafe { &mut *DRIVERS.0.get() };
    let audit = unsafe { &mut *AUDIT.0.get() };
    let agents = unsafe { &mut *AGENTS.0.get() };
    let intents = unsafe { &mut *INTENTS.0.get() };
    let memory = unsafe { &mut *MEMORY_ENGINE.0.get() };
    let fabric = unsafe { &mut *FABRIC.0.get() };
    let intent_sched = unsafe { &mut *INTENT_SCHED.0.get() };

    // Get a pointer to the current task's saved context for dispatch.
    let ctx_ptr = &mut sched.tasks[sched.current].context as *mut TaskContext;

    let action = unsafe {
        dispatch::dispatch(
            ctx_ptr,
            sched,
            ipc,
            heap,
            futex,
            channels,
            poll,
            processes,
            sockets,
            users,
            inodes,
            ramfs,
            fat32s,
            mounts,
            input,
            drivers,
            audit,
            agents,
            intents,
            memory,
            fabric,
            intent_sched,
            crate::console_write_byte,
            crate::console_read_byte,
        )
    };

    match action {
        SyscallAction::Resume => {
            // Write back (possibly modified) context into the trap frame.
            // Ring-0 iretq does not pop RSP/SS, so this path is simple.
            let f = unsafe { &mut *frame };
            unsafe { restore_context_to_frame(&sched.tasks[sched.current].context, f); }
            frame
        }
        SyscallAction::Reschedule | SyscallAction::TaskExited => {
            // Build a proper Ring-0 frame on the next task's own stack so
            // that isr_common iretq restores the correct RSP.
            let next = if let Some(n) = sched.pick_next() {
                use microkernel::task::TaskState;
                sched.current = n;
                sched.tasks[n].state = TaskState::Running;
                n
            } else {
                sched.current
            };
            let next_ctx = &sched.tasks[next].context;
            let next_rsp = next_ctx.gpr[4];
            set_kernel_stack((sched.tasks[next].stack_bottom + sched.tasks[next].stack_size) as u64);
            unsafe { build_ring0_frame(next_ctx, next_rsp) }
        }
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn handle_page_fault(frame: *mut TrapFrame) -> *mut TrapFrame {
    let f = unsafe { &*frame };

    // Read CR2 (faulting virtual address).
    let cr2: u64;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nostack, nomem));
    }

    let serial = soc_qemu_pc::default_serial();
    let mut con = arch::Console::new(serial);
    use core::fmt::Write;

    // Decode error code.
    let present = (f.error_code & 1) != 0;
    let write = (f.error_code & 2) != 0;
    let user = (f.error_code & 4) != 0;
    let rsvd = (f.error_code & 8) != 0;
    let ifetch = (f.error_code & 16) != 0;

    let _ = writeln!(con, "");
    let _ = writeln!(con, "*** PAGE FAULT at 0x{:016x}", cr2);
    let _ = writeln!(con, "    RIP=0x{:016x}  error=0x{:x}", f.rip, f.error_code);
    let _ = writeln!(con, "    {} {} {} {} {}",
        if present { "page-prot" } else { "non-present" },
        if write { "WRITE" } else { "READ" },
        if user { "user" } else { "kernel" },
        if rsvd { "reserved-bit" } else { "" },
        if ifetch { "instr-fetch" } else { "" },
    );
    let _ = writeln!(con, "    RSP=0x{:016x}  CS=0x{:x}", f.rsp, f.cs);

    if user {
        // User-mode page fault — kill the faulting task and reschedule.
        let sched = unsafe { &mut *SCHEDULER.0.get() };
        let _ = writeln!(con, "*** Killing task {} (page fault)", sched.tasks[sched.current].name);
        sched.tasks[sched.current].state = microkernel::task::TaskState::Zombie;
        if let Some(next) = sched.pick_next() {
            sched.current = next;
            sched.tasks[next].state = microkernel::task::TaskState::Running;
            let f_mut = unsafe { &mut *frame };
            unsafe { restore_context_to_frame(&sched.tasks[next].context, f_mut); }
        }
        return frame;
    }

    // Kernel page fault — fatal.
    let _ = writeln!(con, "*** Kernel page fault — system halted.");
    loop {
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn handle_exception(frame: *mut TrapFrame) -> *mut TrapFrame {
    let f = unsafe { &*frame };

    // For now, exceptions are fatal. Print vector + halt.
    // Use COM1 directly for diagnostic output.
    let serial = soc_qemu_pc::default_serial();
    let mut con = arch::Console::new(serial);
    use core::fmt::Write;
    let _ = writeln!(con, "");
    let _ = writeln!(con, "*** EXCEPTION #{} (error_code=0x{:x})", f.vector, f.error_code);
    // Raw dump: [15]=vector [16]=error_code [17]=rip [18]=cs [19]=rflags
    let raw = unsafe { core::slice::from_raw_parts(frame as *const u64, 20) };
    let _ = writeln!(con, "    frame=0x{:x}  raw[15..19]: {:x} {:x} {:x} {:x} {:x}",
        frame as usize, raw[15], raw[16], raw[17], raw[18], raw[19]);
    let _ = writeln!(con, "    RIP=0x{:016x}  RSP=0x{:016x}", f.rip, f.rsp);
    let _ = writeln!(con, "    RAX=0x{:016x}  RBX=0x{:016x}", f.rax, f.rbx);
    let _ = writeln!(con, "    RCX=0x{:016x}  RDX=0x{:016x}", f.rcx, f.rdx);
    let _ = writeln!(con, "    RFLAGS=0x{:016x}  CS=0x{:x}", f.rflags, f.cs);
    let _ = writeln!(con, "*** System halted.");

    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
        #[cfg(not(target_arch = "x86_64"))]
        core::hint::spin_loop();
    }
}

// Provide a no-op fallback for non-x86 builds (cargo check).
#[cfg(not(target_arch = "x86_64"))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _veer_trap_dispatch_x86(_frame: *mut TrapFrame) -> *mut TrapFrame {
    core::ptr::null_mut()
}

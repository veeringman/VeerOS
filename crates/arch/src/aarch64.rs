//! AArch64 (ARM64) saved context and trap entry/exit assembly.
//!
//! Layout of `Aarch64Context`:
//! - `gpr[0..31]` at offsets 0..248 (x0–x30, 8 bytes each)
//! - `sp` at offset 248 (SP_EL0 or stack pointer)
//! - `pc` at offset 256 (ELR_EL1)
//! - `pstate` at offset 264 (SPSR_EL1)
//! Total saved frame: 272 bytes  (plus `kernel_word` at 272, not saved by asm).
//!
//! The `global_asm!` block emits `_veer_trap_sync`, `_veer_trap_irq`,
//! `_veer_start_first_task`, and the VBAR_EL1 exception vector table.
//! The Rust dispatcher (`_veer_trap_dispatch`) is provided by each
//! kernel crate since interrupt routing is board-specific.

use super::SavedContext;


// ═══════════════════════════════════════════════════════════════════════════
// AArch64 exception vector table + context save / restore assembly
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
// ─────────────────────────────────────────────────────────────────
// Exception vector table (VBAR_EL1).
// 16 vectors × 128 bytes each = 2048 bytes, 2048-byte aligned.
// ─────────────────────────────────────────────────────────────────
.section .text._veer_vectors
.global  _veer_vectors
.balign  2048

_veer_vectors:
    // ── Current EL, SP_EL0 (not used — kernel always uses SP_EL1) ──
    b   _veer_unhandled         // Synchronous
    .balign 128
    b   _veer_unhandled         // IRQ
    .balign 128
    b   _veer_unhandled         // FIQ
    .balign 128
    b   _veer_unhandled         // SError
    .balign 128

    // ── Current EL, SP_ELx (this is the kernel — EL1 with SP_EL1) ──
    b   _veer_trap_sync         // Synchronous (SVC / data abort / etc.)
    .balign 128
    b   _veer_trap_irq          // IRQ (timer)
    .balign 128
    b   _veer_unhandled         // FIQ
    .balign 128
    b   _veer_unhandled         // SError
    .balign 128

    // ── Lower EL, AArch64 (not used yet) ──
    b   _veer_unhandled
    .balign 128
    b   _veer_unhandled
    .balign 128
    b   _veer_unhandled
    .balign 128
    b   _veer_unhandled
    .balign 128

    // ── Lower EL, AArch32 (not used) ──
    b   _veer_unhandled
    .balign 128
    b   _veer_unhandled
    .balign 128
    b   _veer_unhandled
    .balign 128
    b   _veer_unhandled
    .balign 128

// ─────────────────────────────────────────────────────────────────
// Unhandled exception — infinite loop (visible in debugger).
// ─────────────────────────────────────────────────────────────────
.global _veer_unhandled
_veer_unhandled:
    wfe
    b   _veer_unhandled

// ─────────────────────────────────────────────────────────────────
// Synchronous exception handler (SVC for syscalls).
//
// On entry:  SP = SP_EL1 (kernel stack of current task)
//            TPIDR_EL1 = pointer to current Aarch64Context buffer
//            ELR_EL1 = return address, SPSR_EL1 = saved PSTATE
// ─────────────────────────────────────────────────────────────────
.section .text._veer_trap_sync
.global  _veer_trap_sync
.balign  4

_veer_trap_sync:
    // Save original x0 on the stack temporarily.
    str   x0, [sp, #-16]!

    // Load context buffer pointer from TPIDR_EL1.
    mrs   x0, tpidr_el1

    // Save x1–x30 into context.
    str   x1,  [x0, #8]
    stp   x2,  x3,  [x0, #16]
    stp   x4,  x5,  [x0, #32]
    stp   x6,  x7,  [x0, #48]
    stp   x8,  x9,  [x0, #64]
    stp   x10, x11, [x0, #80]
    stp   x12, x13, [x0, #96]
    stp   x14, x15, [x0, #112]
    stp   x16, x17, [x0, #128]
    stp   x18, x19, [x0, #144]
    stp   x20, x21, [x0, #160]
    stp   x22, x23, [x0, #176]
    stp   x24, x25, [x0, #192]
    stp   x26, x27, [x0, #208]
    stp   x28, x29, [x0, #224]
    str   x30, [x0, #240]

    // Save original SP (undo our push).
    add   x1, sp, #16
    str   x1, [x0, #248]

    // Save ELR_EL1 → context.pc
    mrs   x1, elr_el1
    str   x1, [x0, #256]

    // Save SPSR_EL1 → context.pstate
    mrs   x1, spsr_el1
    str   x1, [x0, #264]

    // Recover original x0 from the stack and save it.
    ldr   x1, [sp]
    str   x1, [x0, #0]

    // Switch to kernel stack.
    ldr   x1, =__stack_top
    mov   sp, x1

    // Call Rust dispatcher: x0 = context pointer.
    //   fn _veer_trap_dispatch(ctx: *mut Aarch64Context) -> *mut Aarch64Context
    bl    _veer_trap_dispatch

    // x0 = (possibly new) context pointer.
    // Update TPIDR_EL1.
    msr   tpidr_el1, x0

    // Restore SPSR_EL1 and ELR_EL1.
    ldr   x1, [x0, #264]
    msr   spsr_el1, x1
    ldr   x1, [x0, #256]
    msr   elr_el1, x1

    // Restore x2–x30.
    ldr   x30, [x0, #240]
    ldp   x28, x29, [x0, #224]
    ldp   x26, x27, [x0, #208]
    ldp   x24, x25, [x0, #192]
    ldp   x22, x23, [x0, #176]
    ldp   x20, x21, [x0, #160]
    ldp   x18, x19, [x0, #144]
    ldp   x16, x17, [x0, #128]
    ldp   x14, x15, [x0, #112]
    ldp   x12, x13, [x0, #96]
    ldp   x10, x11, [x0, #80]
    ldp   x8,  x9,  [x0, #64]
    ldp   x6,  x7,  [x0, #48]
    ldp   x4,  x5,  [x0, #32]
    ldp   x2,  x3,  [x0, #16]

    // Restore SP from context.
    ldr   x1, [x0, #248]
    mov   sp, x1

    // Restore x1, then x0 (last — it was the base pointer).
    ldr   x1, [x0, #8]
    ldr   x0, [x0, #0]

    eret

// ─────────────────────────────────────────────────────────────────
// IRQ handler (timer interrupt).
// Same save/restore sequence as sync — reuses _veer_trap_dispatch.
// ─────────────────────────────────────────────────────────────────
.section .text._veer_trap_irq
.global  _veer_trap_irq
.balign  4

_veer_trap_irq:
    str   x0, [sp, #-16]!
    mrs   x0, tpidr_el1

    str   x1,  [x0, #8]
    stp   x2,  x3,  [x0, #16]
    stp   x4,  x5,  [x0, #32]
    stp   x6,  x7,  [x0, #48]
    stp   x8,  x9,  [x0, #64]
    stp   x10, x11, [x0, #80]
    stp   x12, x13, [x0, #96]
    stp   x14, x15, [x0, #112]
    stp   x16, x17, [x0, #128]
    stp   x18, x19, [x0, #144]
    stp   x20, x21, [x0, #160]
    stp   x22, x23, [x0, #176]
    stp   x24, x25, [x0, #192]
    stp   x26, x27, [x0, #208]
    stp   x28, x29, [x0, #224]
    str   x30, [x0, #240]

    add   x1, sp, #16
    str   x1, [x0, #248]
    mrs   x1, elr_el1
    str   x1, [x0, #256]
    mrs   x1, spsr_el1
    str   x1, [x0, #264]
    ldr   x1, [sp]
    str   x1, [x0, #0]

    ldr   x1, =__stack_top
    mov   sp, x1

    bl    _veer_trap_dispatch

    msr   tpidr_el1, x0

    ldr   x1, [x0, #264]
    msr   spsr_el1, x1
    ldr   x1, [x0, #256]
    msr   elr_el1, x1

    ldr   x30, [x0, #240]
    ldp   x28, x29, [x0, #224]
    ldp   x26, x27, [x0, #208]
    ldp   x24, x25, [x0, #192]
    ldp   x22, x23, [x0, #176]
    ldp   x20, x21, [x0, #160]
    ldp   x18, x19, [x0, #144]
    ldp   x16, x17, [x0, #128]
    ldp   x14, x15, [x0, #112]
    ldp   x12, x13, [x0, #96]
    ldp   x10, x11, [x0, #80]
    ldp   x8,  x9,  [x0, #64]
    ldp   x6,  x7,  [x0, #48]
    ldp   x4,  x5,  [x0, #32]
    ldp   x2,  x3,  [x0, #16]

    ldr   x1, [x0, #248]
    mov   sp, x1
    ldr   x1, [x0, #8]
    ldr   x0, [x0, #0]

    eret

// ─────────────────────────────────────────────────────────────────
// Start-first-task: load an Aarch64Context and eret into task code.
//   x0 = pointer to Aarch64Context
// ─────────────────────────────────────────────────────────────────
.section .text._veer_start_first_task
.global  _veer_start_first_task
.balign  4

_veer_start_first_task:
    // Store context pointer in TPIDR_EL1 for future trap entries.
    msr   tpidr_el1, x0

    // Restore SPSR_EL1 and ELR_EL1.
    ldr   x1, [x0, #264]
    msr   spsr_el1, x1
    ldr   x1, [x0, #256]
    msr   elr_el1, x1

    // Restore x2–x30.
    ldr   x30, [x0, #240]
    ldp   x28, x29, [x0, #224]
    ldp   x26, x27, [x0, #208]
    ldp   x24, x25, [x0, #192]
    ldp   x22, x23, [x0, #176]
    ldp   x20, x21, [x0, #160]
    ldp   x18, x19, [x0, #144]
    ldp   x16, x17, [x0, #128]
    ldp   x14, x15, [x0, #112]
    ldp   x12, x13, [x0, #96]
    ldp   x10, x11, [x0, #80]
    ldp   x8,  x9,  [x0, #64]
    ldp   x6,  x7,  [x0, #48]
    ldp   x4,  x5,  [x0, #32]
    ldp   x2,  x3,  [x0, #16]

    // Restore SP.
    ldr   x1, [x0, #248]
    mov   sp, x1

    // Restore x1, then x0.
    ldr   x1, [x0, #8]
    ldr   x0, [x0, #0]

    eret
"#
);

// ═══════════════════════════════════════════════════════════════════════════
// Saved context struct
// ═══════════════════════════════════════════════════════════════════════════

/// Saved CPU context for AArch64 (ARMv8-A / ARMv8.2-A).
///
/// The layout is `#[repr(C)]` and matches the trap frame saved by
/// the `_veer_trap_sync` / `_veer_trap_irq` assembly stubs.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Aarch64Context {
    /// General-purpose registers x0–x30.
    pub gpr: [usize; 31],
    /// Stack pointer (SP_EL1 at time of exception).
    pub sp: usize,
    /// Program counter (saved from ELR_EL1).
    pub pc: usize,
    /// Processor state (saved from SPSR_EL1).
    pub pstate: usize,
    /// Kernel bookkeeping word (not saved/restored by assembly).
    pub kernel_word: usize,
}

impl Aarch64Context {
    /// Create a zeroed context.  Usable in `const` initializers.
    pub const fn zero() -> Self {
        Self {
            gpr: [0; 31],
            sp: 0,
            pc: 0,
            pstate: 0,
            kernel_word: 0,
        }
    }
}

impl SavedContext for Aarch64Context {
    /// SVC instruction is 4 bytes.
    const INSTRUCTION_SIZE: usize = 4;

    fn zero() -> Self {
        Self::zero()
    }

    #[inline(always)]
    fn set_pc(&mut self, pc: usize) {
        self.pc = pc;
    }

    #[inline(always)]
    fn get_pc(&self) -> usize {
        self.pc
    }

    #[inline(always)]
    fn advance_pc(&mut self) {
        self.pc += Self::INSTRUCTION_SIZE;
    }

    #[inline(always)]
    fn set_sp(&mut self, sp: usize) {
        self.sp = sp;
    }

    #[inline(always)]
    fn get_sp(&self) -> usize {
        self.sp
    }

    #[inline(always)]
    fn set_status(&mut self, status: usize) {
        self.pstate = status;
    }

    #[inline(always)]
    fn get_status(&self) -> usize {
        self.pstate
    }

    /// AArch64 calling convention: x0–x7 are argument registers.
    #[inline(always)]
    fn set_arg(&mut self, index: usize, val: usize) {
        self.gpr[index] = val;
    }

    #[inline(always)]
    fn get_arg(&self, index: usize) -> usize {
        self.gpr[index]
    }

    /// Return values: x0, x1.
    #[inline(always)]
    fn set_ret(&mut self, index: usize, val: usize) {
        self.gpr[index] = val;
    }

    #[inline(always)]
    fn get_ret(&self, index: usize) -> usize {
        self.gpr[index]
    }

    /// Syscall number in x8 (Linux AArch64 convention).
    #[inline(always)]
    fn get_syscall_nr(&self) -> usize {
        self.gpr[8]
    }

    #[inline(always)]
    fn get_kernel_word(&self) -> usize {
        self.kernel_word
    }

    #[inline(always)]
    fn set_kernel_word(&mut self, val: usize) {
        self.kernel_word = val;
    }
}

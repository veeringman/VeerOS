//! RISC-V 32-bit saved context and trap entry/exit assembly.
//!
//! Layout of `Riscv32Context`:
//! - `gpr[0..32]` at offsets 0..128 (x0–x31, 4 bytes each)
//! - `pc` at offset 128 (mepc)
//! - `status` at offset 132 (mstatus)
//! Total size: 136 bytes.
//!
//! The `global_asm!` block emits `_veer_trap_entry` and
//! `_veer_start_first_task` — shared by all RISC-V 32-bit kernel
//! binaries.  The Rust dispatcher (`_veer_trap_dispatch`) is provided
//! by each kernel crate since interrupt routing is board-specific.

use super::SavedContext;

pub mod pmp;


// ═══════════════════════════════════════════════════════════════════════════
// RISC-V 32-bit trap entry / exit assembly
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "riscv32")]
core::arch::global_asm!(
    r#"
# ── Vector table for RISC-V vectored interrupt mode ──────────────────
# On ESP32-C6, the PLIC forces mtvec.mode = 1 (vectored).
# In vectored mode: exceptions → base+0, interrupt N → base + 4*N.
# Each slot is a 4-byte `j` (JAL x0) to the common handler.
.section .text._veer_vector_table
.global  _veer_vector_table
.balign  128
.option push
.option norvc

_veer_vector_table:
    j _veer_trap_entry   /* 0  exception */
    j _veer_trap_entry   /* 1  CPU int 1 (systimer) */
    j _veer_trap_entry   /* 2  */
    j _veer_trap_entry   /* 3  */
    j _veer_trap_entry   /* 4  */
    j _veer_trap_entry   /* 5  */
    j _veer_trap_entry   /* 6  */
    j _veer_trap_entry   /* 7  */
    j _veer_trap_entry   /* 8  */
    j _veer_trap_entry   /* 9  */
    j _veer_trap_entry   /* 10 */
    j _veer_trap_entry   /* 11 */
    j _veer_trap_entry   /* 12 */
    j _veer_trap_entry   /* 13 */
    j _veer_trap_entry   /* 14 */
    j _veer_trap_entry   /* 15 */
    j _veer_trap_entry   /* 16 */
    j _veer_trap_entry   /* 17 */
    j _veer_trap_entry   /* 18 */
    j _veer_trap_entry   /* 19 */
    j _veer_trap_entry   /* 20 */
    j _veer_trap_entry   /* 21 */
    j _veer_trap_entry   /* 22 */
    j _veer_trap_entry   /* 23 */
    j _veer_trap_entry   /* 24 */
    j _veer_trap_entry   /* 25 */
    j _veer_trap_entry   /* 26 */
    j _veer_trap_entry   /* 27 */
    j _veer_trap_entry   /* 28 */
    j _veer_trap_entry   /* 29 */
    j _veer_trap_entry   /* 30 */
    j _veer_trap_entry   /* 31 */
.option pop

# ── Common trap entry ────────────────────────────────────────────────
.section .text._veer_trap_entry
.global  _veer_trap_entry
.balign  4

_veer_trap_entry:
    # Reserve space for Riscv32Context: 32 GPRs + pc + status = 34 words = 136 bytes.
    addi  sp, sp, -136

    sw  x1,   4(sp)
    sw  x2,   8(sp)
    sw  x3,  12(sp)
    sw  x4,  16(sp)
    sw  x5,  20(sp)
    sw  x6,  24(sp)
    sw  x7,  28(sp)
    sw  x8,  32(sp)
    sw  x9,  36(sp)
    sw  x10, 40(sp)
    sw  x11, 44(sp)
    sw  x12, 48(sp)
    sw  x13, 52(sp)
    sw  x14, 56(sp)
    sw  x15, 60(sp)
    sw  x16, 64(sp)
    sw  x17, 68(sp)
    sw  x18, 72(sp)
    sw  x19, 76(sp)
    sw  x20, 80(sp)
    sw  x21, 84(sp)
    sw  x22, 88(sp)
    sw  x23, 92(sp)
    sw  x24, 96(sp)
    sw  x25, 100(sp)
    sw  x26, 104(sp)
    sw  x27, 108(sp)
    sw  x28, 112(sp)
    sw  x29, 116(sp)
    sw  x30, 120(sp)
    sw  x31, 124(sp)

    # Fix saved sp to pre-trap value.
    addi  t0, sp, 136
    sw    t0, 8(sp)

    # Save mepc and mstatus.
    csrr  t0, mepc
    sw    t0, 128(sp)
    csrr  t0, mstatus
    sw    t0, 132(sp)

    # Call Rust dispatcher: a0 = saved context pointer.
    mv    a0, sp
    call  _veer_trap_dispatch
    # a0 = context to restore.

    # Restore context.
    mv    sp, a0

    lw    t0, 128(sp)
    csrw  mepc, t0
    lw    t0, 132(sp)
    csrw  mstatus, t0

    lw  x1,   4(sp)
    lw  x3,  12(sp)
    lw  x4,  16(sp)
    lw  x5,  20(sp)
    lw  x6,  24(sp)
    lw  x7,  28(sp)
    lw  x8,  32(sp)
    lw  x9,  36(sp)
    lw  x10, 40(sp)
    lw  x11, 44(sp)
    lw  x12, 48(sp)
    lw  x13, 52(sp)
    lw  x14, 56(sp)
    lw  x15, 60(sp)
    lw  x16, 64(sp)
    lw  x17, 68(sp)
    lw  x18, 72(sp)
    lw  x19, 76(sp)
    lw  x20, 80(sp)
    lw  x21, 84(sp)
    lw  x22, 88(sp)
    lw  x23, 92(sp)
    lw  x24, 96(sp)
    lw  x25, 100(sp)
    lw  x26, 104(sp)
    lw  x27, 108(sp)
    lw  x28, 112(sp)
    lw  x29, 116(sp)
    lw  x30, 120(sp)
    lw  x31, 124(sp)

    lw  x2, 8(sp)

    mret

# ─────────────────────────────────────────────────────────────────
# Start-first-task: load a Riscv32Context and mret into user code.
#   a0 = pointer to Riscv32Context
# ─────────────────────────────────────────────────────────────────
.section .text._veer_start_first_task
.global  _veer_start_first_task
.balign  4

_veer_start_first_task:
    mv    sp, a0

    lw    t0, 128(sp)
    csrw  mepc, t0
    lw    t0, 132(sp)
    csrw  mstatus, t0

    lw  x1,   4(sp)
    lw  x3,  12(sp)
    lw  x4,  16(sp)
    lw  x5,  20(sp)
    lw  x6,  24(sp)
    lw  x7,  28(sp)
    lw  x8,  32(sp)
    lw  x9,  36(sp)
    lw  x10, 40(sp)
    lw  x11, 44(sp)
    lw  x12, 48(sp)
    lw  x13, 52(sp)
    lw  x14, 56(sp)
    lw  x15, 60(sp)
    lw  x16, 64(sp)
    lw  x17, 68(sp)
    lw  x18, 72(sp)
    lw  x19, 76(sp)
    lw  x20, 80(sp)
    lw  x21, 84(sp)
    lw  x22, 88(sp)
    lw  x23, 92(sp)
    lw  x24, 96(sp)
    lw  x25, 100(sp)
    lw  x26, 104(sp)
    lw  x27, 108(sp)
    lw  x28, 112(sp)
    lw  x29, 116(sp)
    lw  x30, 120(sp)
    lw  x31, 124(sp)

    lw  x2, 8(sp)

    mret
"#
);

/// Saved CPU context for RISC-V 32-bit (RV32I/RV32IMC/RV32IMAC).
///
/// The layout is `#[repr(C)]` and matches the trap frame pushed by
/// `_veer_trap_entry` assembly.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Riscv32Context {
    /// General-purpose registers x0–x31.
    ///
    /// Note: x0 is hardwired to zero in hardware, but the saved slot
    /// (`gpr[0]`) is reused by the kernel for bookkeeping (e.g., sleep
    /// target tick).
    pub gpr: [usize; 32],
    /// Program counter (saved from `mepc` CSR).
    pub pc: usize,
    /// Machine status register (saved from `mstatus` CSR).
    pub status: usize,
}

impl Riscv32Context {
    /// Create a zeroed context. Usable in `const` initializers (e.g., TCB tables).
    pub const fn zero() -> Self {
        Self {
            gpr: [0; 32],
            pc: 0,
            status: 0,
        }
    }
}

impl SavedContext for Riscv32Context {
    const INSTRUCTION_SIZE: usize = 4; // 4 bytes for ecall (even with C extension, ecall is 32-bit)

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
        self.gpr[2] = sp; // x2 = sp
    }

    #[inline(always)]
    fn get_sp(&self) -> usize {
        self.gpr[2]
    }

    #[inline(always)]
    fn set_status(&mut self, status: usize) {
        self.status = status;
    }

    #[inline(always)]
    fn get_status(&self) -> usize {
        self.status
    }

    #[inline(always)]
    fn set_arg(&mut self, index: usize, val: usize) {
        // a0=x10, a1=x11, a2=x12, a3=x13
        self.gpr[10 + index] = val;
    }

    #[inline(always)]
    fn get_arg(&self, index: usize) -> usize {
        self.gpr[10 + index]
    }

    #[inline(always)]
    fn set_ret(&mut self, index: usize, val: usize) {
        // Return values: a0=x10, a1=x11
        self.gpr[10 + index] = val;
    }

    #[inline(always)]
    fn get_ret(&self, index: usize) -> usize {
        self.gpr[10 + index]
    }

    #[inline(always)]
    fn get_syscall_nr(&self) -> usize {
        self.gpr[17] // a7 = x17
    }

    #[inline(always)]
    fn get_kernel_word(&self) -> usize {
        self.gpr[0] // x0 slot — free for kernel use in saved context
    }

    #[inline(always)]
    fn set_kernel_word(&mut self, val: usize) {
        self.gpr[0] = val;
    }
}

//! x86_64 saved context definition.
//!
//! This is the architecture-neutral kernel-facing register frame used by
//! scheduler and syscall dispatch code. Trap/interrupt entry assembly is
//! platform-specific and will be added by x86_64 kernel BSP crates.

use super::SavedContext;

/// Saved CPU context for x86_64.
///
/// Register order in `gpr`:
/// - 0: rax
/// - 1: rcx
/// - 2: rdx
/// - 3: rbx
/// - 4: rsp
/// - 5: rbp
/// - 6: rsi
/// - 7: rdi
/// - 8: r8
/// - 9: r9
/// - 10: r10
/// - 11: r11
/// - 12: r12
/// - 13: r13
/// - 14: r14
/// - 15: r15
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct X86_64Context {
    /// General-purpose registers.
    pub gpr: [usize; 16],
    /// Instruction pointer.
    pub rip: usize,
    /// Flags register.
    pub rflags: usize,
    /// Kernel bookkeeping word.
    pub kernel_word: usize,
}

impl X86_64Context {
    /// Create a zeroed context. Usable in `const` initializers.
    pub const fn zero() -> Self {
        Self {
            gpr: [0; 16],
            rip: 0,
            rflags: 0,
            kernel_word: 0,
        }
    }
}

impl SavedContext for X86_64Context {
    /// On x86-64, the `syscall` instruction saves the *return* RIP into RCX,
    /// and `_veer_syscall_entry` pushes that as `frame.rip`. So the saved PC
    /// already points past the `syscall` instruction — `advance_pc` is a no-op.
    const INSTRUCTION_SIZE: usize = 0;

    fn zero() -> Self {
        Self::zero()
    }

    #[inline(always)]
    fn set_pc(&mut self, pc: usize) {
        self.rip = pc;
    }

    #[inline(always)]
    fn get_pc(&self) -> usize {
        self.rip
    }

    #[inline(always)]
    fn advance_pc(&mut self) {
        self.rip += Self::INSTRUCTION_SIZE;
    }

    #[inline(always)]
    fn set_sp(&mut self, sp: usize) {
        self.gpr[4] = sp; // rsp
    }

    #[inline(always)]
    fn get_sp(&self) -> usize {
        self.gpr[4]
    }

    #[inline(always)]
    fn set_status(&mut self, status: usize) {
        self.rflags = status;
    }

    #[inline(always)]
    fn get_status(&self) -> usize {
        self.rflags
    }

    #[inline(always)]
    fn set_arg(&mut self, index: usize, val: usize) {
        // Syscall argument registers: rdi, rsi, rdx, r10, r8, r9.
        // arg[3] is r10 (not rcx) because `syscall` clobbers rcx with the return RIP.
        const ARG_REGS: [usize; 6] = [7, 6, 2, 10, 8, 9];
        if index < ARG_REGS.len() {
            self.gpr[ARG_REGS[index]] = val;
        }
    }

    #[inline(always)]
    fn get_arg(&self, index: usize) -> usize {
        // Syscall argument registers: rdi, rsi, rdx, r10, r8, r9.
        // arg[3] is r10 (not rcx) because `syscall` clobbers rcx with the return RIP.
        const ARG_REGS: [usize; 6] = [7, 6, 2, 10, 8, 9];
        if index < ARG_REGS.len() {
            self.gpr[ARG_REGS[index]]
        } else {
            0
        }
    }

    #[inline(always)]
    fn set_ret(&mut self, index: usize, val: usize) {
        // Return registers: rax, rdx, rdi, rsi.
        match index {
            0 => self.gpr[0] = val, // rax
            1 => self.gpr[2] = val, // rdx
            2 => self.gpr[7] = val, // rdi
            3 => self.gpr[6] = val, // rsi
            _ => {}
        }
    }

    #[inline(always)]
    fn get_ret(&self, index: usize) -> usize {
        // Return registers: rax, rdx, rdi, rsi.
        match index {
            0 => self.gpr[0], // rax
            1 => self.gpr[2], // rdx
            2 => self.gpr[7], // rdi
            3 => self.gpr[6], // rsi
            _ => 0,
        }
    }

    #[inline(always)]
    fn get_syscall_nr(&self) -> usize {
        self.gpr[0] // rax
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

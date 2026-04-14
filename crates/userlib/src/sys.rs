//! Raw syscall interface — the lowest layer of the userlib.
//!
//! On RISC-V: `ecall` with syscall number in `a7`, args in `a0`–`a4`.
//! On AArch64: `svc #0` with syscall number in `x8`, args in `x0`–`x4`.
//! On x86-64: `int 0x80` with syscall number in `rax`, args in `rdi`, `rsi`,
//!   `rdx`, `r10`, `r8`, `r9`. Return value in `rax`. Unlike `syscall`,
//!   `int 0x80` works at Ring 0 and preserves all caller-saved registers.
//! On other hosts: no-ops returning 0 (allows `cargo check` everywhere).

/// Issue a syscall with 0 arguments.  Returns `a0`.
#[inline(always)]
pub fn syscall0(nr: usize) -> usize {
    let ret: usize;
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::arch::asm!(
            "ecall",
            in("x17") nr,
            lateout("x10") ret,
            options(nostack),
        );
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") nr,
            lateout("x0") ret,
            options(nostack),
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") nr => ret,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(any(target_arch = "riscv32", target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = nr;
        ret = 0;
    }
    ret
}

/// Issue a syscall with 1 argument.  Returns `a0`.
#[inline(always)]
pub fn syscall1(nr: usize, a0: usize) -> usize {
    let ret: usize;
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::arch::asm!(
            "ecall",
            in("x17") nr,
            inlateout("x10") a0 => ret,
            options(nostack),
        );
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") nr,
            inlateout("x0") a0 => ret,
            options(nostack),
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") nr => ret,
            in("rdi") a0,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(any(target_arch = "riscv32", target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = (nr, a0);
        ret = 0;
    }
    ret
}

/// Issue a syscall with 2 arguments.  Returns `(a0, a1)`.
#[inline(always)]
pub fn syscall2(nr: usize, a0: usize, a1: usize) -> (usize, usize) {
    let r0: usize;
    let r1: usize;
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::arch::asm!(
            "ecall",
            in("x17") nr,
            inlateout("x10") a0 => r0,
            inlateout("x11") a1 => r1,
            options(nostack),
        );
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") nr,
            inlateout("x0") a0 => r0,
            inlateout("x1") a1 => r1,
            options(nostack),
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        // Second return value comes back in rsi (preserved by iretq path).
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") nr => r0,
            in("rdi") a0,
            inlateout("rsi") a1 => r1,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(any(target_arch = "riscv32", target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = (nr, a0, a1);
        r0 = 0;
        r1 = 0;
    }
    (r0, r1)
}

/// Issue a syscall with 3 arguments.  Returns `a0`.
#[inline(always)]
pub fn syscall3(nr: usize, a0: usize, a1: usize, a2: usize) -> usize {
    let ret: usize;
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::arch::asm!(
            "ecall",
            in("x17") nr,
            inlateout("x10") a0 => ret,
            in("x11") a1,
            in("x12") a2,
            options(nostack),
        );
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") nr,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            options(nostack),
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") nr => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(any(target_arch = "riscv32", target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = (nr, a0, a1, a2);
        ret = 0;
    }
    ret
}

/// Issue a syscall with 4 arguments.  Returns `a0`.
#[inline(always)]
pub fn syscall4(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize) -> usize {
    let ret: usize;
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::arch::asm!(
            "ecall",
            in("x17") nr,
            inlateout("x10") a0 => ret,
            in("x11") a1,
            in("x12") a2,
            in("x13") a3,
            options(nostack),
        );
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") nr,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            options(nostack),
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        // 4th argument in r10.
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") nr => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(any(target_arch = "riscv32", target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = (nr, a0, a1, a2, a3);
        ret = 0;
    }
    ret
}

/// Issue a syscall with 5 arguments.  Returns `a0`.
#[inline(always)]
pub fn syscall5(nr: usize, a0: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> usize {
    let ret: usize;
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::arch::asm!(
            "ecall",
            in("x17") nr,
            inlateout("x10") a0 => ret,
            in("x11") a1,
            in("x12") a2,
            in("x13") a3,
            in("x14") a4,
            options(nostack),
        );
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") nr,
            inlateout("x0") a0 => ret,
            in("x1") a1,
            in("x2") a2,
            in("x3") a3,
            in("x4") a4,
            options(nostack),
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") nr => ret,
            in("rdi") a0,
            in("rsi") a1,
            in("rdx") a2,
            in("r10") a3,
            in("r8")  a4,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(any(target_arch = "riscv32", target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = (nr, a0, a1, a2, a3, a4);
        ret = 0;
    }
    ret
}

/// Issue a syscall that returns 4 values through `a0`–`a3`.
#[inline(always)]
pub fn syscall_ret4(nr: usize, a0: usize) -> (usize, usize, usize, usize) {
    let r0: usize;
    let r1: usize;
    let r2: usize;
    let r3: usize;
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::arch::asm!(
            "ecall",
            in("x17") nr,
            inlateout("x10") a0 => r0,
            lateout("x11") r1,
            lateout("x12") r2,
            lateout("x13") r3,
            options(nostack),
        );
    }
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "svc #0",
            in("x8") nr,
            inlateout("x0") a0 => r0,
            lateout("x1") r1,
            lateout("x2") r2,
            lateout("x3") r3,
            options(nostack),
        );
    }
    #[cfg(target_arch = "x86_64")]
    unsafe {
        // Kernel returns word0 in rax, word1 in rdx, word2 in rdi, word3 in rsi.
        core::arch::asm!(
            "int 0x80",
            inlateout("rax") nr => r0,
            inlateout("rdi") a0 => r2,
            lateout("rdx") r1,
            lateout("rsi") r3,
            options(nostack, preserves_flags),
        );
    }
    #[cfg(not(any(target_arch = "riscv32", target_arch = "aarch64", target_arch = "x86_64")))]
    {
        let _ = (nr, a0);
        r0 = 0;
        r1 = 0;
        r2 = 0;
        r3 = 0;
    }
    (r0, r1, r2, r3)
}

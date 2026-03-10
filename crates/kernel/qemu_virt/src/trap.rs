//! Trap dispatcher for the QEMU `virt` RISC-V kernel.
//!
//! Reuses the same global_asm trap entry/exit as kernel-esp32.
//! The dispatch logic differs: QEMU virt uses standard CLINT (mcause 7
//! for machine timer) instead of the ESP32 interrupt matrix.

#[allow(unused_imports)]
use arch::{TaskContext, TickTimer};
#[allow(unused_imports)]
use microkernel::task::Scheduler;

#[allow(unused_imports)]
use crate::{SCHEDULER, TIMER};

// ═══════════════════════════════════════════════════════════════════════════
// RISC-V trap entry / exit (same for all RISC-V kernels)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "riscv32")]
core::arch::global_asm!(
    r#"
.section .text._veer_trap_entry
.global  _veer_trap_entry
.balign  4

_veer_trap_entry:
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

    addi  t0, sp, 136
    sw    t0, 8(sp)

    csrr  t0, mepc
    sw    t0, 128(sp)
    csrr  t0, mstatus
    sw    t0, 132(sp)

    mv    a0, sp
    call  _veer_trap_dispatch
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
# Start-first-task: load a TaskContext and mret into user code.
#   a0 = pointer to TaskContext
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

// ═══════════════════════════════════════════════════════════════════════════
// mcause constants (standard RISC-V privilege spec)
// ═══════════════════════════════════════════════════════════════════════════

#[allow(dead_code)]
const MCAUSE_INTERRUPT_BIT: usize = 1 << 31;
#[allow(dead_code)]
const MCAUSE_MACHINE_TIMER: usize = 7;
#[allow(dead_code)]
const MCAUSE_ECALL_MMODE: usize = 11;

// ═══════════════════════════════════════════════════════════════════════════
// Rust trap dispatcher
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "riscv32")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _veer_trap_dispatch(ctx: *mut TaskContext) -> *mut TaskContext {
    let mcause: usize;
    unsafe {
        core::arch::asm!("csrr {}, mcause", out(reg) mcause, options(nomem, nostack));
    }

    let is_interrupt = (mcause & MCAUSE_INTERRUPT_BIT) != 0;
    let code = mcause & !MCAUSE_INTERRUPT_BIT;

    if is_interrupt {
        handle_interrupt(ctx, code)
    } else {
        handle_exception(ctx, code)
    }
}

#[cfg(target_arch = "riscv32")]
unsafe fn handle_interrupt(ctx: *mut TaskContext, code: usize) -> *mut TaskContext {
    match code {
        MCAUSE_MACHINE_TIMER => handle_timer_tick(ctx),
        _ => ctx,
    }
}

#[cfg(target_arch = "riscv32")]
unsafe fn handle_timer_tick(ctx: *mut TaskContext) -> *mut TaskContext {
    // Ack the timer by scheduling the next compare event.
    let timer = unsafe { &*TIMER.0.get() };
    timer.clear_pending();

    let sched: &mut Scheduler = unsafe { &mut *SCHEDULER.0.get() };
    sched.save_current_context(unsafe { &*ctx });

    let need_switch = sched.tick();
    if need_switch {
        if let Some(new_ctx) = sched.current_context_mut() {
            return new_ctx as *mut TaskContext;
        }
    }
    ctx
}

#[cfg(target_arch = "riscv32")]
unsafe fn handle_exception(ctx: *mut TaskContext, code: usize) -> *mut TaskContext {
    match code {
        MCAUSE_ECALL_MMODE => {
            let c = unsafe { &mut *ctx };
            c.pc += 4; // skip ecall instruction
            ctx
        }
        _ => loop {
            core::hint::spin_loop();
        },
    }
}

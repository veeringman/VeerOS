//! Trap dispatcher for the QEMU `virt` RISC-V kernel.
//!
//! The trap entry/exit assembly (`_veer_trap_entry`, `_veer_start_first_task`)
//! lives in `arch::riscv32` and is shared by all RISC-V 32-bit kernels.
//! This file provides the board-specific Rust dispatcher that the assembly
//! calls into.

#[allow(unused_imports)]
use arch::{TaskContext, TickTimer};
#[allow(unused_imports)]
use arch::riscv32::pmp;
#[allow(unused_imports)]
use microkernel::task::Scheduler;
#[allow(unused_imports)]
use microkernel::dispatch::{self, SyscallAction};

#[allow(unused_imports)]
use crate::{SCHEDULER, TIMER, IPC, HEAP, FUTEX, CHANNELS, POLL, PROCESSES, SOCKETS, USERS, INODES, RAMFS, FAT32, MOUNTS, INPUT};

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
// PMP helper — apply memory regions for the current task
// ═══════════════════════════════════════════════════════════════════════════

/// Program PMP entries for the currently selected task.
///
/// Combines process-level regions (code, data, heap) with
/// thread-level regions (stack + guard) and programs PMP.
#[cfg(target_arch = "riscv32")]
unsafe fn apply_pmp_for_current(sched: &Scheduler) {
    let cur = sched.current;
    if cur < sched.tasks.len() {
        let tcb = &sched.tasks[cur];
        let pid = tcb.process_id;
        let procs = unsafe { &*PROCESSES.0.get() };
        if pid < microkernel::process::MAX_PROCESSES {
            let p = &procs.processes[pid];
            pmp::apply_combined_regions(
                &p.regions[..p.region_count],
                p.region_count,
                &tcb.regions,
                tcb.region_count,
            );
        } else {
            pmp::apply_task_regions(&tcb.regions, tcb.region_count);
        }
    }
}

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
    dispatch::wake_sleepers(sched);

    // Wake poll-blocked tasks whose events fired or timeout expired.
    let ipc = unsafe { &*IPC.0.get() };
    let channels = unsafe { &*CHANNELS.0.get() };
    let poll = unsafe { &mut *POLL.0.get() };
    microkernel::poll::wake_poll_waiters(poll, sched, ipc, channels);

    if need_switch {
        // Apply PMP for the new task before returning its context.
        apply_pmp_for_current(sched);
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
            let sched = unsafe { &mut *SCHEDULER.0.get() };
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
            let fat32 = unsafe { &mut *FAT32.0.get() };
            let mounts = unsafe { &mut *MOUNTS.0.get() };
            let input = unsafe { &mut *INPUT.0.get() };

            let action = unsafe {
                dispatch::dispatch(
                    ctx,
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
                    fat32,
                    mounts,
                    input,
                    crate::console_write_byte,
                    crate::console_read_byte,
                )
            };

            match action {
                SyscallAction::Resume => ctx,
                SyscallAction::Reschedule | SyscallAction::TaskExited => {
                    // Save AFTER dispatch so pc+4 and any gpr writes are captured.
                    sched.save_current_context(unsafe { &*ctx });
                    if let Some(next) = sched.pick_next() {
                        use microkernel::task::TaskState;
                        sched.current = next;
                        sched.tasks[next].state = TaskState::Running;
                        // Apply PMP before returning the new context pointer.
                        apply_pmp_for_current(sched);
                        &mut sched.tasks[next].context as *mut TaskContext
                    } else {
                        ctx
                    }
                }
            }
        }
        _ => loop {
            core::hint::spin_loop();
        },
    }
}

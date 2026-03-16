//! Trap dispatcher for the Raspberry Pi 5 AArch64 kernel.
//!
//! The exception vector table and context save/restore assembly live in
//! `arch::aarch64` and are shared by all AArch64 kernel binaries.
//! This file provides the board-specific Rust dispatcher that the
//! assembly stubs call into.

#[allow(unused_imports)]
use arch::{TaskContext, TickTimer};
#[allow(unused_imports)]
use microkernel::task::Scheduler;
#[allow(unused_imports)]
use microkernel::dispatch::{self, SyscallAction};

#[allow(unused_imports)]
use crate::{SCHEDULER, TIMER, IPC, HEAP, FUTEX, CHANNELS, POLL, PROCESSES, SOCKETS, USERS, GIC, INODES, RAMFS, FAT32, MOUNTS, INPUT};

// ═══════════════════════════════════════════════════════════════════════════
// AArch64 ESR_EL1 constants
// ═══════════════════════════════════════════════════════════════════════════

/// Exception class for SVC (supervisor call) from AArch64: EC = 0b010101.
#[allow(dead_code)]
const ESR_EC_SVC64: u64 = 0x15;

/// ARM physical timer PPI number.
#[allow(dead_code)]
const TIMER_IRQ: u32 = 30;

// ═══════════════════════════════════════════════════════════════════════════
// Rust trap dispatcher (called from assembly)
// ═══════════════════════════════════════════════════════════════════════════

/// Unified trap dispatcher.
///
/// The assembly stubs (`_veer_trap_sync`, `_veer_trap_irq`) save the full
/// register set and call this function with a pointer to the saved context.
/// We return a (possibly different) context pointer for the assembly to
/// restore.
#[cfg(target_arch = "aarch64")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _veer_trap_dispatch(ctx: *mut TaskContext) -> *mut TaskContext {
    // Read ESR_EL1 to determine the exception type.
    let esr: u64;
    unsafe {
        core::arch::asm!("mrs {}, esr_el1", out(reg) esr, options(nomem, nostack));
    }

    let ec = (esr >> 26) & 0x3F; // Exception Class

    match ec {
        ESR_EC_SVC64 => handle_svc(ctx),
        _ => {
            // Check if this is a timer IRQ (called from _veer_trap_irq).
            // The GIC will tell us which interrupt fired.
            handle_irq(ctx)
        }
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn handle_svc(ctx: *mut TaskContext) -> *mut TaskContext {
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
            sched.save_current_context(unsafe { &*ctx });
            if let Some(next) = sched.pick_next() {
                use microkernel::task::TaskState;
                sched.current = next;
                sched.tasks[next].state = TaskState::Running;
                &mut sched.tasks[next].context as *mut TaskContext
            } else {
                ctx
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn handle_irq(ctx: *mut TaskContext) -> *mut TaskContext {
    let gic = unsafe { &*GIC.0.get() };
    let irq = gic.acknowledge();

    if irq == TIMER_IRQ {
        handle_timer_tick(ctx, irq)
    } else if irq >= 1022 {
        // Spurious interrupt — ignore.
        ctx
    } else {
        // Unknown IRQ — signal end and continue.
        gic.end_of_interrupt(irq);
        ctx
    }
}

#[cfg(target_arch = "aarch64")]
unsafe fn handle_timer_tick(ctx: *mut TaskContext, irq: u32) -> *mut TaskContext {
    // Ack the timer by scheduling the next compare event.
    let timer = unsafe { &*TIMER.0.get() };
    timer.clear_pending();

    // Signal end-of-interrupt to GIC.
    let gic = unsafe { &*GIC.0.get() };
    gic.end_of_interrupt(irq);

    let sched = unsafe { &mut *SCHEDULER.0.get() };
    sched.save_current_context(unsafe { &*ctx });

    let need_switch = sched.tick();
    dispatch::wake_sleepers(sched);

    // Wake poll-blocked tasks whose events fired or timeout expired.
    let ipc = unsafe { &*IPC.0.get() };
    let channels = unsafe { &*CHANNELS.0.get() };
    let poll = unsafe { &mut *POLL.0.get() };
    microkernel::poll::wake_poll_waiters(poll, sched, ipc, channels);

    if need_switch {
        if let Some(new_ctx) = sched.current_context_mut() {
            return new_ctx as *mut TaskContext;
        }
    }
    ctx
}

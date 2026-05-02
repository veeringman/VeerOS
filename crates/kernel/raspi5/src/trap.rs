//! Trap dispatcher for the Raspberry Pi 5 AArch64 kernel.
//!
//! The exception vector table and context save/restore assembly live in
//! `arch::aarch64` and are shared by all AArch64 kernel binaries.
//! This file provides the board-specific Rust dispatcher that the
//! assembly stubs call into.

#[allow(unused_imports)]
use arch::{TaskContext, TickTimer};
#[allow(unused_imports)]
use microkernel::dispatch::{self, SyscallAction};
#[allow(unused_imports)]
use microkernel::task::Scheduler;

#[allow(unused_imports)]
use crate::{
    AGENTS, AUDIT, CHANNELS, DRIVERS, FABRIC, FAT32, FUTEX, GIC, HEAP, INODES, INPUT, INTENTS,
    INTENT_SCHED, IPC, MEMORY_ENGINE, MOUNTS, POLL, PROCESSES, RAMFS, SCHEDULER, SOCKETS, TIMER,
    USERS,
};

// ═══════════════════════════════════════════════════════════════════════════
// AArch64 ESR_EL1 constants
// ═══════════════════════════════════════════════════════════════════════════

/// Exception class for SVC (supervisor call) from AArch64: EC = 0b010101.
#[allow(dead_code)]
const ESR_EC_SVC64: u64 = 0x15;

/// Exception class for data abort from current EL: EC = 0b100101.
#[allow(dead_code)]
const ESR_EC_DABORT_CEL: u64 = 0x25;

/// ARM physical timer PPI number.
#[allow(dead_code)]
const TIMER_IRQ: u32 = 30;

// ═══════════════════════════════════════════════════════════════════════════
// Safe MMIO probe — catches data aborts during memory probes
// ═══════════════════════════════════════════════════════════════════════════

/// Set to `true` when a probe is in progress. If a data abort occurs
/// while this is set, the handler skips the faulting instruction
/// instead of parking.
pub(crate) static mut PROBE_ACTIVE: bool = false;
/// Set to `true` if a data abort was caught during a probe.
pub(crate) static mut PROBE_FAULTED: bool = false;
// Rust trap dispatcher (called from assembly)
// ═══════════════════════════════════════════════════════════════════════════

/// Unified trap dispatcher.
///
/// The assembly stubs (`_veer_trap_sync`, `_veer_trap_irq`) save the full
/// register set and call this function with a pointer to the saved context.
/// We return a (possibly different) context pointer for the assembly to
/// restore.
/// Synchronous exception dispatcher (SVC, data abort, etc.).
/// Called from `_veer_trap_sync` assembly.
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
        ESR_EC_DABORT_CEL => {
            // Data abort from current EL.
            // If a probe is active, skip the faulting instruction.
            if unsafe { PROBE_ACTIVE } {
                unsafe {
                    PROBE_FAULTED = true;
                }
                // Advance PC in the saved context (not ELR_EL1 directly,
                // because the asm stub restores ELR_EL1 from ctx.pc).
                unsafe {
                    (*ctx).pc += 4;
                }
                ctx
            } else {
                // Unexpected data abort — park.
                ctx
            }
        }
        _ => {
            // Unexpected sync exception — park.
            ctx
        }
    }
}

/// IRQ dispatcher. Called from `_veer_trap_irq` assembly.
/// Separate entry point avoids reading stale ESR_EL1.
#[cfg(target_arch = "aarch64")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _veer_irq_dispatch(ctx: *mut TaskContext) -> *mut TaskContext {
    handle_irq(ctx)
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
    let drivers = unsafe { &mut *DRIVERS.0.get() };
    let audit = unsafe { &mut *AUDIT.0.get() };
    let agents = unsafe { &mut *AGENTS.0.get() };
    let intents = unsafe { &mut *INTENTS.0.get() };
    let memory = unsafe { &mut *MEMORY_ENGINE.0.get() };
    let fabric = unsafe { &mut *FABRIC.0.get() };
    let intent_sched = unsafe { &mut *INTENT_SCHED.0.get() };

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
        SyscallAction::Resume => ctx,
        SyscallAction::Reschedule | SyscallAction::TaskExited => {
            if let Some(next) = sched.pick_next() {
                use microkernel::task::TaskState;
                sched.current = next;
                sched.tasks[next].state = TaskState::Running;
                &mut sched.tasks[next].context as *mut TaskContext
            } else {
                // Nothing else runnable. If the current task is still
                // in a runnable state (Ready), keep it going.
                // If it's Blocked or Free, we must NOT resume it.
                use microkernel::task::TaskState;
                let cur = sched.current;
                if cur < sched.tasks.len()
                    && (sched.tasks[cur].state == TaskState::Ready
                        || sched.tasks[cur].state == TaskState::Running)
                {
                    sched.tasks[cur].state = TaskState::Running;
                    ctx
                } else {
                    // No runnable task at all — switch to idle spin.
                    // Find any Ready task (including priority 0).
                    for i in 0..sched.tasks.len() {
                        if sched.tasks[i].state == TaskState::Ready {
                            sched.current = i;
                            sched.tasks[i].state = TaskState::Running;
                            return &mut sched.tasks[i].context as *mut TaskContext;
                        }
                    }
                    // Truly nothing — just return ctx (should not happen
                    // if idle task exists).
                    ctx
                }
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

    let _tick_switch = sched.tick();
    dispatch::wake_sleepers(sched);

    // Diagnostic: dump task states at tick 6000 (6 seconds), then halt at 8000.
    if sched.ticks == 6000 {
        // Helper to print a hex nibble
        fn hex_nibble(n: u8) -> u8 {
            if n < 10 {
                b'0' + n
            } else {
                b'a' + (n - 10)
            }
        }
        fn print_hex16(val: usize) {
            // Print the low 16 bits (4 hex digits) — enough to distinguish PCs.
            for shift in (0..16).rev().step_by(4) {
                crate::console_write_byte(hex_nibble(((val >> shift) & 0xF) as u8));
            }
        }

        // Big visible separator
        for _ in 0..3u8 {
            crate::console_write_byte(b'\n');
        }
        for _ in 0..40u8 {
            crate::console_write_byte(b'=');
        }
        crate::console_write_byte(b'\n');
        // Print task states: idx=state@PC
        for i in 0..8usize {
            use arch::SavedContext;
            use microkernel::task::TaskState;
            let ch = match sched.tasks[i].state {
                TaskState::Free => b'F',
                TaskState::Ready => b'R',
                TaskState::Running => b'*',
                TaskState::Blocked => b'B',
                TaskState::Suspended => b'S',
                TaskState::Zombie => b'Z',
            };
            crate::console_write_byte(b'0' + i as u8);
            crate::console_write_byte(b'=');
            crate::console_write_byte(ch);
            // Print block reason if blocked
            if sched.tasks[i].state == TaskState::Blocked {
                use microkernel::task::BlockReason;
                let br = match sched.tasks[i].block_reason {
                    BlockReason::Sleep => b's',
                    BlockReason::IpcRecv => b'i',
                    BlockReason::Join => b'j',
                    BlockReason::Futex => b'f',
                    _ => b'?',
                };
                crate::console_write_byte(br);
            }
            // Print PC for non-Free tasks
            if sched.tasks[i].state != TaskState::Free {
                crate::console_write_byte(b'@');
                print_hex16(sched.tasks[i].context.get_pc());
            }
            crate::console_write_byte(b' ');
        }
        crate::console_write_byte(b'\n');
        for _ in 0..40u8 {
            crate::console_write_byte(b'=');
        }
        crate::console_write_byte(b'\n');
    }
    // At tick 8000, halt so user can read screen.
    if sched.ticks == 8000 {
        crate::console_write_byte(b'\n');
        // Print "HALT" and spin forever
        crate::console_write_byte(b'H');
        crate::console_write_byte(b'A');
        crate::console_write_byte(b'L');
        crate::console_write_byte(b'T');
        crate::console_write_byte(b'\n');
        loop {
            core::hint::spin_loop();
        }
    }

    // Poll USB HID keyboards every 8 ticks (~8 ms at 1 kHz).
    if sched.ticks % 8 == 0 {
        let input = unsafe { &mut *crate::INPUT.0.get() };
        if input.active {
            let xhci0 = unsafe { &mut *crate::XHCI0.0.get() };
            xhci0.poll_hid_keyboards(|report| {
                input.feed_keyboard_report(report);
            });
            let xhci1 = unsafe { &mut *crate::XHCI1.0.get() };
            xhci1.poll_hid_keyboards(|report| {
                input.feed_keyboard_report(report);
            });
        }
    }

    // Wake poll-blocked tasks whose events fired or timeout expired.
    let ipc = unsafe { &*IPC.0.get() };
    let channels = unsafe { &*CHANNELS.0.get() };
    let poll = unsafe { &mut *POLL.0.get() };
    microkernel::poll::wake_poll_waiters(poll, sched, ipc, channels);

    // Always re-evaluate after waking sleepers/poll — a higher-priority
    // task may have just become Ready.
    {
        use microkernel::task::TaskState;
        // Mark current Running→Ready so pick_next considers all.
        if sched.current < sched.tasks.len()
            && sched.tasks[sched.current].state == TaskState::Running
        {
            sched.tasks[sched.current].state = TaskState::Ready;
        }
        if let Some(next) = sched.pick_next() {
            sched.current = next;
            sched.tasks[next].state = TaskState::Running;
            return &mut sched.tasks[next].context as *mut TaskContext;
        }
        // No task ready — only re-mark current as Running if it was
        // demoted to Ready above (not if it's Free or Blocked).
        if sched.current < sched.tasks.len() && sched.tasks[sched.current].state == TaskState::Ready
        {
            sched.tasks[sched.current].state = TaskState::Running;
        }
    }
    ctx
}

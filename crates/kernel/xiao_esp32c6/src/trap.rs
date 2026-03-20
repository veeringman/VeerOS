//! Trap dispatcher for the ESP32-C6 RISC-V kernel.
//!
//! The trap entry/exit assembly (`_veer_trap_entry`, `_veer_start_first_task`)
//! lives in `arch::riscv32` and is shared by all RISC-V 32-bit kernels.
//! This file provides the board-specific Rust dispatcher — ESP32-C3/C6
//! uses the SysTimer mapped to CPU interrupt line 1 (not standard CLINT
//! mcause 7).

#[allow(unused_imports)]
use arch::{TaskContext, TickTimer};
#[allow(unused_imports)]
use arch::riscv32::pmp;
#[allow(unused_imports)]
use microkernel::task::Scheduler;
#[allow(unused_imports)]
use microkernel::dispatch::{self, SyscallAction};

#[allow(unused_imports)]
use crate::{SCHEDULER, TIMER, IPC, HEAP, FUTEX, CHANNELS, POLL, PROCESSES, SOCKETS, USERS, INODES, RAMFS, FAT32, MOUNTS, INPUT, DRIVERS};
// mcause constants
// ═══════════════════════════════════════════════════════════════════════════

#[allow(dead_code)]
const MCAUSE_INTERRUPT_BIT: usize = 1 << 31;
/// ESP32-C3: SYSTIMER fires on CPU interrupt line 1 (not standard mcause 7).
#[allow(dead_code)]
const SYSTIMER_CPU_INT_CODE: usize = 1;
#[allow(dead_code)]
const MCAUSE_ECALL_UMODE: usize = 8;
#[allow(dead_code)]
const MCAUSE_ECALL_MMODE: usize = 11;

/// Program PMP entries for the currently selected task.
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
// Rust trap dispatcher (riscv32 only — host builds skip this)
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
        SYSTIMER_CPU_INT_CODE => handle_timer_tick(ctx),
        _ => {
            // Dispatch WiFi / BLE / other blob-registered ISRs.
            soc_esp32::wifi_os_adapter::wifi_isr_dispatch(code);
            ctx
        }
    }
}

#[cfg(target_arch = "riscv32")]
unsafe fn handle_timer_tick(ctx: *mut TaskContext) -> *mut TaskContext {
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
        MCAUSE_ECALL_UMODE | MCAUSE_ECALL_MMODE => {
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
                        apply_pmp_for_current(sched);
                        &mut sched.tasks[next].context as *mut TaskContext
                    } else {
                        ctx
                    }
                }
            }
        }
        _ => {
            // Unknown exception — print diagnostic and halt.
            crate::console_write_byte(b'!');
            crate::console_write_byte(b'E');
            // Emit mcause code as hex nibbles.
            let code_u8 = code as u8;
            let hi = (code_u8 >> 4) & 0xF;
            let lo = code_u8 & 0xF;
            crate::console_write_byte(if hi < 10 { b'0' + hi } else { b'a' + hi - 10 });
            crate::console_write_byte(if lo < 10 { b'0' + lo } else { b'a' + lo - 10 });
            // Emit mepc.
            let mepc: usize;
            unsafe { core::arch::asm!("csrr {}, mepc", out(reg) mepc, options(nomem, nostack)); }
            crate::console_write_byte(b'@');
            for shift in (0..8).rev() {
                let nib = ((mepc >> (shift * 4)) & 0xF) as u8;
                crate::console_write_byte(if nib < 10 { b'0' + nib } else { b'a' + nib - 10 });
            }
            crate::console_write_byte(b'\n');
            // Flush USB Serial JTAG so bytes reach the host
            unsafe {
                let usb_base: usize = 0x6000_F000;
                let conf = core::ptr::read_volatile((usb_base + 0x04) as *const u32);
                core::ptr::write_volatile((usb_base + 0x04) as *mut u32, conf | 1);
            }
            loop {
                core::hint::spin_loop();
            }
        },
    }
}

//! Trap dispatcher for the ESP32-C6 RISC-V kernel.
//!
//! The trap entry/exit assembly (`_veer_trap_entry`, `_veer_start_first_task`)
//! lives in `arch::riscv32` and is shared by all RISC-V 32-bit kernels.
//! This file provides the board-specific Rust dispatcher — ESP32-C3/C6
//! uses the SysTimer mapped to CPU interrupt line 1 (not standard CLINT
//! mcause 7).

#[allow(unused_imports)]
use arch::riscv32::pmp;
#[allow(unused_imports)]
use arch::{SavedContext, TaskContext, TickTimer};
#[allow(unused_imports)]
use microkernel::dispatch::{self, SyscallAction};
#[allow(unused_imports)]
use microkernel::task::Scheduler;

#[allow(unused_imports)]
use crate::{
    AGENTS, AUDIT, CHANNELS, DRIVERS, FABRIC, FAT32, FUTEX, HEAP, INODES, INPUT, INTENTS,
    INTENT_SCHED, IPC, MEMORY_ENGINE, MOUNTS, POLL, PROCESSES, RAMFS, SCHEDULER, SOCKETS, TIMER,
    USERS,
};
// mcause constants
// ═══════════════════════════════════════════════════════════════════════════

#[allow(dead_code)]
const MCAUSE_INTERRUPT_BIT: usize = 1 << 31;
/// ESP32-C6: systimer runs on CPU interrupt line 5.
#[allow(dead_code)]
const SYSTIMER_CPU_INT_CODE: usize = 5;
const MCAUSE_MACHINE_EXTERNAL: usize = 11;
#[allow(dead_code)]
const MCAUSE_ECALL_UMODE: usize = 8;
#[allow(dead_code)]
const MCAUSE_ECALL_MMODE: usize = 11;

static mut IRQ_CODE_EXT11_COUNT: u32 = 0;
static mut IRQ_CODE_1_COUNT: u32 = 0;
static mut IRQ_CODE_2_COUNT: u32 = 0;
static mut IRQ_OTHER_COUNT: u32 = 0;
static mut IRQ_LAST_CODE: u32 = 0;

pub fn irq_diag() -> (u32, u32, u32, u32, u32) {
    unsafe {
        (
            IRQ_CODE_EXT11_COUNT,
            IRQ_CODE_1_COUNT,
            IRQ_CODE_2_COUNT,
            IRQ_OTHER_COUNT,
            IRQ_LAST_CODE,
        )
    }
}

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

/// Refuse a switch to a TCB whose PC is not in mapped code (saw `!E01@80`).
fn task_pc_runnable(sched: &Scheduler, idx: usize) -> bool {
    if idx >= sched.tasks.len() {
        return false;
    }
    sched.tasks[idx].context.get_pc() >= 0x4000_0000
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
    unsafe {
        IRQ_LAST_CODE = code as u32;
        match code {
            MCAUSE_MACHINE_EXTERNAL => IRQ_CODE_EXT11_COUNT = IRQ_CODE_EXT11_COUNT.wrapping_add(1),
            1 => IRQ_CODE_1_COUNT = IRQ_CODE_1_COUNT.wrapping_add(1),
            2 => IRQ_CODE_2_COUNT = IRQ_CODE_2_COUNT.wrapping_add(1),
            _ => IRQ_OTHER_COUNT = IRQ_OTHER_COUNT.wrapping_add(1),
        }
    }
    match code {
        SYSTIMER_CPU_INT_CODE => handle_timer_tick(ctx),
        _ => {
            // Clear INTPRI pending latch BEFORE dispatching ISR (matches upstream).
            // Without this, level-triggered WiFi interrupts won't re-fire.
            unsafe {
                const INTPRI_BASE: usize = 0x600C_5000;
                const INTC_CPU_INT_CLEAR: usize = 0xA8;
                core::ptr::write_volatile(
                    (INTPRI_BASE + INTC_CPU_INT_CLEAR) as *mut u32,
                    1u32 << code,
                );
            }
            // Dispatch WiFi / BLE / other blob-registered ISRs.
            if !soc_esp32::wifi_os_adapter::wifi_isr_dispatch(code) {
                // No handler registered — mask this line to prevent
                // infinite re-entry on level-triggered interrupts.
                let mask = 1u32 << code;
                unsafe {
                    core::arch::asm!("csrc mie, {0}", in(reg) mask, options(nomem, nostack));
                }
                return ctx;
            }
            // Gold: yield_from_isr → rtos switches to the woken task
            // (ppTask) after the FIQ. Do that here via the saved frame;
            // never ecall or run blob task code on the ISR stack.
            if soc_esp32::wifi_os_adapter::take_yield_from_isr() {
                return switch_to_wifi_blob(ctx);
            }
            ctx
        }
    }
}

#[cfg(target_arch = "riscv32")]
unsafe fn switch_to_wifi_blob(ctx: *mut TaskContext) -> *mut TaskContext {
    use microkernel::task::TaskState;
    let sched: &mut Scheduler = unsafe { &mut *SCHEDULER.0.get() };
    sched.save_current_context(unsafe { &*ctx });
    if sched.current < sched.tasks.len() && sched.tasks[sched.current].state == TaskState::Running {
        sched.tasks[sched.current].state = TaskState::Ready;
    }
    let prev = sched.current;
    // Gold yield wakes the blocked receiver (ppTask), then switches to it.
    let mut ids = [usize::MAX; 4];
    let n = soc_esp32::wifi_os_adapter::blob_task_ids(&mut ids);
    for i in 0..n {
        let id = ids[i];
        if id < sched.tasks.len() && sched.tasks[id].state == TaskState::Blocked {
            sched.tasks[id].state = TaskState::Ready;
            sched.tasks[id].block_reason = microkernel::task::BlockReason::None;
        }
    }
    let mut wakes = [usize::MAX; 4];
    let wn = soc_esp32::wifi_os_adapter::take_isr_wake_tasks(&mut wakes);
    for i in 0..wn {
        let id = wakes[i];
        if id < sched.tasks.len() && sched.tasks[id].state == TaskState::Blocked {
            sched.tasks[id].state = TaskState::Ready;
            sched.tasks[id].block_reason = microkernel::task::BlockReason::None;
        }
    }
    let next = soc_esp32::wifi_os_adapter::first_ready_blob_task(|id| {
        id < sched.tasks.len() && sched.tasks[id].state == TaskState::Ready
    })
    .or_else(|| sched.pick_next());
    if let Some(next) = next {
        if !task_pc_runnable(sched, next) {
            return ctx;
        }
        sched.current = next;
        sched.tasks[next].state = TaskState::Running;
        if soc_esp32::wifi_os_adapter::is_blob_task_id(next)
            && !soc_esp32::wifi_os_adapter::is_blob_task_id(prev)
        {
            soc_esp32::wifi_os_adapter::note_pp_scheduled(sched.ticks);
        }
        apply_pmp_for_current(sched);
        if let Some(new_ctx) = sched.current_context_mut() {
            return new_ctx as *mut TaskContext;
        }
    }
    ctx
}

#[cfg(target_arch = "riscv32")]
unsafe fn handle_timer_tick(ctx: *mut TaskContext) -> *mut TaskContext {
    let timer = unsafe { &*TIMER.0.get() };
    timer.clear_pending();

    let sched: &mut Scheduler = unsafe { &mut *SCHEDULER.0.get() };
    sched.save_current_context(unsafe { &*ctx });
    let prev = sched.current;

    // Gold: expire timers / wake waiters first, then pick the highest
    // ready task. Waking after pick left ppTask Blocked for an extra tick.
    dispatch::wake_sleepers(sched);
    let need_switch = sched.tick();

    // Wake poll-blocked tasks whose events fired or timeout expired.
    let ipc = unsafe { &*IPC.0.get() };
    let channels = unsafe { &*CHANNELS.0.get() };
    let poll = unsafe { &mut *POLL.0.get() };
    microkernel::poll::wake_poll_waiters(poll, sched, ipc, channels);

    if need_switch {
        if task_pc_runnable(sched, sched.current) {
            apply_pmp_for_current(sched);
            if let Some(new_ctx) = sched.current_context_mut() {
                return new_ctx as *mut TaskContext;
            }
        }
        sched.current = prev;
        if prev < sched.tasks.len() {
            use microkernel::task::TaskState;
            sched.tasks[prev].state = TaskState::Running;
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
                    // Save AFTER dispatch so pc+4 and any gpr writes are captured.
                    sched.save_current_context(unsafe { &*ctx });
                    if let Some(next) = sched.pick_next() {
                        if !task_pc_runnable(sched, next) {
                            return ctx;
                        }
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
            unsafe {
                core::arch::asm!("csrr {}, mepc", out(reg) mepc, options(nomem, nostack));
            }
            crate::console_write_byte(b'@');
            for shift in (0..8).rev() {
                let nib = ((mepc >> (shift * 4)) & 0xF) as u8;
                crate::console_write_byte(if nib < 10 {
                    b'0' + nib
                } else {
                    b'a' + nib - 10
                });
            }
            crate::console_write_byte(b'\n');
            unsafe {
                let sched = &*SCHEDULER.0.get();
                let ra = (*ctx).gpr[1] as u32;
                let gp = (*ctx).gpr[3] as u32;
                let a0 = (*ctx).gpr[10] as u32;
                crate::console_write_byte(b'X');
                crate::console_write_byte(b' ');
                fn hex_u32(v: u32) {
                    for shift in (0..8).rev() {
                        let nib = ((v >> (shift * 4)) & 0xF) as u8;
                        crate::console_write_byte(if nib < 10 {
                            b'0' + nib
                        } else {
                            b'a' + nib - 10
                        });
                    }
                    crate::console_write_byte(b' ');
                }
                hex_u32(sched.current as u32);
                hex_u32(ra);
                hex_u32(gp);
                hex_u32(a0);
                for i in 0..6 {
                    if i < sched.tasks.len() {
                        hex_u32(sched.tasks[i].context.get_pc() as u32);
                    }
                }
                crate::console_write_byte(b'\n');
            }
            // Flush USB Serial JTAG so bytes reach the host
            unsafe {
                let usb_base: usize = 0x6000_F000;
                let conf = core::ptr::read_volatile((usb_base + 0x04) as *const u32);
                core::ptr::write_volatile((usb_base + 0x04) as *mut u32, conf | 1);
            }
            loop {
                core::hint::spin_loop();
            }
        }
    }
}

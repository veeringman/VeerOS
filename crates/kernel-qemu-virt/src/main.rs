//! VeerOS kernel for the QEMU `virt` RISC-V 32-bit machine.
//!
//! This is the **real** bare-metal kernel — `#![no_std]`, `#![no_main]`,
//! runs on `riscv32imc-unknown-none-elf` under QEMU with real trap vectors,
//! CLINT timer interrupts, and a preemptive scheduler.

#![no_std]
#![no_main]

mod trap;

use core::cell::UnsafeCell;
use core::fmt::Write;

use arch::{Console, TickTimer};
use bsp_qemu_virt::{default_serial, system_timer, clint::Clint, QemuVirt};
use microkernel::Kernel;
use microkernel::task::{Scheduler, TaskState};
use shell::{Shell, ShellEnv};

use panic_halt as _;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Kernel tick period (1 ms).
const TICK_PERIOD_US: u32 = 1_000;

// ---------------------------------------------------------------------------
// Static scheduler
// ---------------------------------------------------------------------------

struct SchedulerCell(UnsafeCell<Scheduler>);
unsafe impl Sync for SchedulerCell {}

static SCHEDULER: SchedulerCell = SchedulerCell(UnsafeCell::new(Scheduler::new()));

// ---------------------------------------------------------------------------
// Static timer handle (used by the trap dispatcher)
// ---------------------------------------------------------------------------

pub(crate) struct TimerCell(pub UnsafeCell<Clint>);
unsafe impl Sync for TimerCell {}

pub(crate) static TIMER: TimerCell = TimerCell(UnsafeCell::new(Clint::new()));

// ---------------------------------------------------------------------------
// Idle task
// ---------------------------------------------------------------------------

fn idle_task() -> ! {
    loop {
        // wfi (wait for interrupt) saves power under QEMU too.
        #[cfg(target_arch = "riscv32")]
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "riscv32"))]
        core::hint::spin_loop();
    }
}

#[repr(align(16))]
struct IdleStack([u8; 512]);
static IDLE_STACK: IdleStack = IdleStack([0u8; 512]);

// ---------------------------------------------------------------------------
// Shell task
// ---------------------------------------------------------------------------

#[repr(align(16))]
struct ShellStack([u8; 8192]);
static SHELL_STACK: ShellStack = ShellStack([0u8; 8192]);

fn shell_task() -> ! {
    let serial = default_serial();
    let mut con = Console::new(serial);
    let env = ShellEnv {
        version: VERSION,
        platform: "QEMU virt (RISC-V 32)",
        scheduler: "minimal",
        get_uptime_ticks: Some(get_uptime_ticks),
        get_task_list: Some(write_task_list),
    };
    let mut sh = Shell::new(env);
    sh.run(&mut con);

    // User typed `exit` — power off QEMU.
    let _ = writeln!(con, "VeerOS halted \u{2014} powering off.");
    qemu_poweroff();
}

// ---------------------------------------------------------------------------
// Scheduler query callbacks (injected into the shell via ShellEnv)
// ---------------------------------------------------------------------------

fn get_uptime_ticks() -> u64 {
    unsafe { (*SCHEDULER.0.get()).ticks }
}

fn write_task_list(w: &mut dyn core::fmt::Write) {
    let sched = unsafe { &*SCHEDULER.0.get() };
    let _ = writeln!(w, "  ID  STATE     PRI  NAME");
    let _ = writeln!(w, "  --  --------  ---  --------");
    for (i, t) in sched.tasks.iter().enumerate() {
        if t.state != TaskState::Free {
            let st = match t.state {
                TaskState::Free => "free",
                TaskState::Ready => "ready",
                TaskState::Running => "RUNNING",
                TaskState::Blocked => "blocked",
            };
            let _ = writeln!(w, "  {:2}  {:8}  {:3}  {}", i, st, t.priority, t.name);
        }
    }
    let _ = writeln!(w, "  ticks: {}", sched.ticks);
}

// ---------------------------------------------------------------------------
// QEMU power-off via SiFive Test device
// ---------------------------------------------------------------------------

fn qemu_poweroff() -> ! {
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::ptr::write_volatile(0x10_0000 as *mut u32, 0x5555);
    }
    loop {
        #[cfg(target_arch = "riscv32")]
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "riscv32"))]
        core::hint::spin_loop();
    }
}

/// RISC-V initial mstatus: MPIE=1 (bit 7) so mret enables interrupts,
/// MPP=M-mode (bits 12:11 = 0b11) so mret stays in machine mode.
const INITIAL_MSTATUS: usize = (1 << 7) | (3 << 11);

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

#[cfg(target_arch = "riscv32")]
core::arch::global_asm!(
    r#"
.section .text._start
.global  _start
.balign  4

_start:
    # Disable interrupts during init.
    csrci   mstatus, 0x8

    # Set up the kernel stack (linker-provided symbol).
    la      sp, __stack_top

    # Zero the BSS section.
    la      t0, __bss_start
    la      t1, __bss_end
1:  bge     t0, t1, 2f
    sw      zero, 0(t0)
    addi    t0, t0, 4
    j       1b
2:

    # Jump into Rust.
    j       _rust_start
"#
);

#[unsafe(no_mangle)]
pub extern "C" fn _rust_start() -> ! {
    // ── early console ────────────────────────────────────────
    let serial = default_serial();
    let mut con = Console::new(serial);

    // ── platform + kernel init ───────────────────────────────
    let platform = QemuVirt::new();
    let kernel = Kernel::new(platform);
    kernel.boot();

    // ── boot banner ──────────────────────────────────────────
    let _ = writeln!(con, "");
    let _ = writeln!(con, "========================================");
    let _ = writeln!(con, "  VeerOS v{VERSION}");
    let _ = writeln!(con, "  Platform : {}", kernel.platform_name());
    let _ = writeln!(con, "  Scheduler: {}", kernel.scheduler_label());
    let _ = writeln!(con, "========================================");
    let _ = writeln!(con, "");

    // ── install trap vector ──────────────────────────────────
    #[cfg(target_arch = "riscv32")]
    {
        extern "C" {
            fn _veer_trap_entry();
        }
        unsafe {
            let addr = _veer_trap_entry as *const () as usize;
            // Direct mode (LSB = 0).
            core::arch::asm!("csrw mtvec, {0}", in(reg) addr, options(nomem, nostack));
        }
        let _ = writeln!(con, "[boot] trap vector installed");
    }
    #[cfg(not(target_arch = "riscv32"))]
    {
        let _ = writeln!(con, "[boot] trap vector skipped (host build)");
    }

    // ── CLINT timer ──────────────────────────────────────────
    let timer = system_timer();
    timer.configure_tick(TICK_PERIOD_US);
    // Store the configured timer so the trap handler can access it.
    unsafe {
        *TIMER.0.get() = timer;
    }
    let _ = writeln!(con, "[boot] CLINT timer tick @ {} us", TICK_PERIOD_US);

    // ── enable machine timer interrupt via mie.MTIE ──────────
    #[cfg(target_arch = "riscv32")]
    unsafe {
        // MIE.MTIE = bit 7
        core::arch::asm!("csrs mie, {0}", in(reg) (1u32 << 7), options(nomem, nostack));
    }
    let _ = writeln!(con, "[boot] machine timer interrupt enabled");

    // ── scheduler + tasks ────────────────────────────────────
    unsafe {
        let sched = &mut *SCHEDULER.0.get();

        // Idle task (priority 0).
        let sb = IDLE_STACK.0.as_ptr() as usize;
        let st = sb + IDLE_STACK.0.len();
        if let Some(idx) = sched.create_task("idle", idle_task as *const () as usize, st, sb, 0) {
            sched.tasks[idx].context.status = INITIAL_MSTATUS;
        }

        // Shell task (priority 1).
        let sb = SHELL_STACK.0.as_ptr() as usize;
        let st = sb + SHELL_STACK.0.len();
        if let Some(idx) = sched.create_task("shell", shell_task as *const () as usize, st, sb, 1) {
            sched.tasks[idx].context.status = INITIAL_MSTATUS;
        }
    }
    let _ = writeln!(con, "[boot] idle task registered");
    let _ = writeln!(con, "[boot] shell task registered");

    // ── start the first task (never returns) ─────────────────
    let _ = writeln!(con, "[boot] starting scheduler — preemptive mode");
    let _ = writeln!(con, "");

    unsafe {
        let sched = &mut *SCHEDULER.0.get();
        let ctx_ptr = sched.start().expect("no runnable task");

        #[cfg(target_arch = "riscv32")]
        {
            extern "C" {
                fn _veer_start_first_task(ctx: *const arch::TaskContext) -> !;
            }
            _veer_start_first_task(ctx_ptr);
        }

        #[cfg(not(target_arch = "riscv32"))]
        {
            // Host build — just run the shell directly for `cargo check`.
            let _ = ctx_ptr;
            drop(con);
            shell_task();
        }
    }
}

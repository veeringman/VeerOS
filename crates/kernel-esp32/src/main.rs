#![no_std]
#![no_main]

mod trap;

use core::cell::UnsafeCell;
use core::fmt::Write;

use arch::{Console, InterruptController, TickTimer};
use bsp_esp32_riscv::{
    default_serial, interrupt_controller, system_timer,
    systimer::SysTimer, Esp32Riscv,
};
use microkernel::Kernel;
use microkernel::task::Scheduler;
use shell::{Shell, ShellEnv};

use panic_halt as _;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Kernel tick period in microseconds (1 ms).
const TICK_PERIOD_US: u32 = 1_000;

/// SYSTIMER comparator 0 is routed to CPU interrupt source 7 on ESP32-C3.
const SYSTIMER_IRQ_SOURCE: u16 = 7;
/// We map it to CPU interrupt line 1.
const SYSTIMER_CPU_INT: u8 = 1;

// ---------------------------------------------------------------------------
// Static scheduler (single-core, no heap)
// ---------------------------------------------------------------------------

/// Wrapper to put the scheduler in a static without `static mut`.
struct SchedulerCell(UnsafeCell<Scheduler>);
unsafe impl Sync for SchedulerCell {}

static SCHEDULER: SchedulerCell = SchedulerCell(UnsafeCell::new(Scheduler::new()));

// ---------------------------------------------------------------------------
// Static timer handle (used by the trap dispatcher to ack interrupts)
// ---------------------------------------------------------------------------

pub(crate) struct TimerCell(pub UnsafeCell<SysTimer>);
unsafe impl Sync for TimerCell {}

pub(crate) static TIMER: TimerCell = TimerCell(UnsafeCell::new(SysTimer::new()));

// ---------------------------------------------------------------------------
// Idle task — runs when nothing else is runnable
// ---------------------------------------------------------------------------

fn idle_task() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// Small stack for the idle task (lives in .bss).
#[repr(align(16))]
struct IdleStack([u8; 512]);
static IDLE_STACK: IdleStack = IdleStack([0u8; 512]);

// ---------------------------------------------------------------------------
// Shell task
// ---------------------------------------------------------------------------

/// Stack for the shell task (4 KiB — needs room for the line buffer, etc.).
#[repr(align(16))]
struct ShellStack([u8; 4096]);
static SHELL_STACK: ShellStack = ShellStack([0u8; 4096]);

fn shell_task() -> ! {
    let serial = default_serial();
    let mut con = Console::new(serial);
    let env = ShellEnv {
        version: VERSION,
        platform: "ESP32-C3 (RISC-V)",
        scheduler: "minimal",
        get_uptime_ticks: None,
        get_task_list: None,
    };
    let mut sh = Shell::new(env);
    loop {
        sh.run(&mut con);
    }
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // ── early console ────────────────────────────────────────
    let serial = default_serial();
    let mut con = Console::new(serial);

    // ── platform + kernel init ───────────────────────────────
    let platform = Esp32Riscv::new();
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

    // ── install trap vector (RISC-V only) ──────────────────────
    #[cfg(target_arch = "riscv32")]
    {
        extern "C" {
            fn _veer_trap_entry();
        }
        unsafe {
            let addr = _veer_trap_entry as *const () as usize;
            core::arch::asm!("csrw mtvec, {0}", in(reg) addr, options(nomem, nostack));
        }
        let _ = writeln!(con, "[boot] trap vector installed");
    }
    #[cfg(not(target_arch = "riscv32"))]
    {
        let _ = writeln!(con, "[boot] trap vector skipped (host build)");
    }

    // ── interrupt controller setup ───────────────────────────
    let intc = interrupt_controller();
    // Map SYSTIMER comparator 0 IRQ → CPU interrupt line
    intc.map_source(SYSTIMER_IRQ_SOURCE, SYSTIMER_CPU_INT);
    intc.set_priority(SYSTIMER_CPU_INT as u16, 1);
    intc.set_threshold(0);
    intc.enable_interrupt(SYSTIMER_CPU_INT as u16);
    let _ = writeln!(con, "[boot] interrupt controller configured");

    // ── system timer tick ────────────────────────────────────
    let timer = system_timer();
    timer.configure_tick(TICK_PERIOD_US);
    let _ = writeln!(con, "[boot] systimer tick @ {} us", TICK_PERIOD_US);

    // ── scheduler + idle task ────────────────────────────────
    unsafe {
        let sched = &mut *SCHEDULER.0.get();
        let stack_bottom = IDLE_STACK.0.as_ptr() as usize;
        let stack_top = stack_bottom + IDLE_STACK.0.len();
        sched.create_task(
            "idle",
            idle_task as *const () as usize,
            stack_top,
            stack_bottom,
            0, // lowest priority
        );
    }
    let _ = writeln!(con, "[boot] idle task registered");

    // ── shell task ───────────────────────────────────────────
    unsafe {
        let sched = &mut *SCHEDULER.0.get();
        let stack_bottom = SHELL_STACK.0.as_ptr() as usize;
        let stack_top = stack_bottom + SHELL_STACK.0.len();
        sched.create_task(
            "shell",
            shell_task as *const () as usize,
            stack_top,
            stack_bottom,
            1, // higher priority than idle
        );
    }
    let _ = writeln!(con, "[boot] shell task registered");

    // ── enable interrupts and enter idle ──────────────────────
    intc.enable_global();
    let _ = writeln!(con, "[boot] interrupts enabled — entering idle loop");

    loop {
        core::hint::spin_loop();
    }
}
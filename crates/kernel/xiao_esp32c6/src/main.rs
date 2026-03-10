#![no_std]
#![no_main]

mod trap;

use core::cell::UnsafeCell;
use core::fmt::Write;

use arch::{Console, InterruptController, TickTimer};
use soc_esp32::{
    default_serial, interrupt_controller, system_timer,
    systimer::SysTimer, Esp32Riscv,
};
use microkernel::Kernel;
use microkernel::alloc::Heap;
use microkernel::driver::{DriverCaps, DriverRegistry, MemRegion};
use microkernel::ipc::Ipc;
use microkernel::task::Scheduler;
#[cfg(feature = "shell")]
use microkernel::task::TaskState;
#[cfg(feature = "shell")]
use shell::{Shell, ShellEnv};

use panic_halt as _;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Kernel tick period in microseconds (1 ms).
const TICK_PERIOD_US: u32 = 1_000;

/// SYSTIMER comparator 0 is routed to CPU interrupt source 7 on ESP32-C3/C6.
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
// Kernel heap (16 KiB — ESP32-C6 has 512 KiB HP SRAM)
// ---------------------------------------------------------------------------

const HEAP_SIZE: usize = 16 * 1024;
const HEAP_SMALL_BYTES: usize = 8 * 1024;

#[repr(align(16))]
struct HeapRegion([u8; HEAP_SIZE]);
static mut HEAP_REGION: HeapRegion = HeapRegion([0u8; HEAP_SIZE]);

struct HeapCell(UnsafeCell<Heap>);
unsafe impl Sync for HeapCell {}
static HEAP: HeapCell = HeapCell(UnsafeCell::new(Heap::new()));

// ---------------------------------------------------------------------------
// IPC mailboxes
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub(crate) struct IpcCell(pub UnsafeCell<Ipc>);
unsafe impl Sync for IpcCell {}
#[allow(dead_code)]
pub(crate) static IPC: IpcCell = IpcCell(UnsafeCell::new(Ipc::new()));

// ---------------------------------------------------------------------------
// Static timer handle (used by the trap dispatcher to ack interrupts)
// ---------------------------------------------------------------------------

pub(crate) struct TimerCell(pub UnsafeCell<SysTimer>);
unsafe impl Sync for TimerCell {}

pub(crate) static TIMER: TimerCell = TimerCell(UnsafeCell::new(SysTimer::new()));

// ---------------------------------------------------------------------------
// Driver registry
// ---------------------------------------------------------------------------

struct RegistryCell(UnsafeCell<DriverRegistry>);
unsafe impl Sync for RegistryCell {}
static DRIVERS: RegistryCell = RegistryCell(UnsafeCell::new(DriverRegistry::new()));

// ---------------------------------------------------------------------------
// Wi-Fi manager (config store + state machine)
// ---------------------------------------------------------------------------

#[cfg(feature = "wifi")]
use soc_esp32::wifi::WifiManager;

#[cfg(feature = "wifi")]
struct WifiCell(UnsafeCell<WifiManager>);
#[cfg(feature = "wifi")]
unsafe impl Sync for WifiCell {}
#[cfg(feature = "wifi")]
static WIFI: WifiCell = WifiCell(UnsafeCell::new(WifiManager::new()));

// ---------------------------------------------------------------------------
// BLE manager
// ---------------------------------------------------------------------------

#[cfg(feature = "ble")]
use soc_esp32::ble::BleManager;

#[cfg(feature = "ble")]
struct BleCell(UnsafeCell<BleManager>);
#[cfg(feature = "ble")]
unsafe impl Sync for BleCell {}
#[cfg(feature = "ble")]
static BLE: BleCell = BleCell(UnsafeCell::new(BleManager::new()));

// ---------------------------------------------------------------------------
// IEEE 802.15.4 (ZigBee / Thread) manager
// ---------------------------------------------------------------------------

#[cfg(feature = "ieee802154")]
use soc_esp32::ieee802154::RadioManager;

#[cfg(feature = "ieee802154")]
struct RadioCell(UnsafeCell<RadioManager>);
#[cfg(feature = "ieee802154")]
unsafe impl Sync for RadioCell {}
#[cfg(feature = "ieee802154")]
static RADIO_802154: RadioCell = RadioCell(UnsafeCell::new(RadioManager::new()));

/// RISC-V initial mstatus: MPIE=1 so mret enables interrupts, MPP=M-mode.
const INITIAL_MSTATUS: usize = (1 << 7) | (3 << 11);

// ---------------------------------------------------------------------------
// Idle task — runs when nothing else is runnable
// ---------------------------------------------------------------------------

fn idle_task() -> ! {
    loop {
        #[cfg(target_arch = "riscv32")]
        unsafe {
            core::arch::asm!("wfi", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "riscv32"))]
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
#[cfg(feature = "shell")]
#[repr(align(16))]
struct ShellStack([u8; 4096]);
#[cfg(feature = "shell")]
static SHELL_STACK: ShellStack = ShellStack([0u8; 4096]);

#[cfg(feature = "shell")]
fn shell_task() -> ! {
    let serial = default_serial();
    let mut con = Console::new(serial);
    let env = ShellEnv {
        version: VERSION,
        platform: "ESP32-C6 (RISC-V)",
        scheduler: "minimal",
        get_uptime_ticks: Some(get_uptime_ticks),
        get_task_list: Some(write_task_list),
        get_mem_info: Some(write_mem_info),
        get_driver_list: Some(write_driver_list),
        #[cfg(feature = "wifi")]
        wifi_cmd: Some(wifi_command),
        #[cfg(not(feature = "wifi"))]
        wifi_cmd: None,
        #[cfg(feature = "ble")]
        bt_cmd: Some(bt_command),
        #[cfg(not(feature = "ble"))]
        bt_cmd: None,
        #[cfg(feature = "ieee802154")]
        zigbee_cmd: Some(zigbee_command),
        #[cfg(not(feature = "ieee802154"))]
        zigbee_cmd: None,
    };
    let mut sh = Shell::new(env);
    loop {
        sh.run(&mut con);
    }
}

// ---------------------------------------------------------------------------
// Assembly entry point (BSS zero, stack setup, jump to Rust)
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

// ---------------------------------------------------------------------------
// Entry (Rust)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn _rust_start() -> ! {
    // ── disable watchdogs (ROM bootloader enables them) ──────────
    soc_esp32::wdt::disable_watchdogs();

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
    // ── kernel heap ──────────────────────────────────────────
    unsafe {
        let region = &mut *core::ptr::addr_of_mut!(HEAP_REGION);
        (*HEAP.0.get()).init(&mut region.0, HEAP_SMALL_BYTES);
    }
    let _ = writeln!(
        con,
        "[boot] heap initialised ({} KiB)",
        HEAP_SIZE / 1024,
    );

    // ── register drivers ─────────────────────────────────────
    unsafe {
        let reg = &mut *DRIVERS.0.get();

        let uart = reg.register("uart0", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: false,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(uart, MemRegion::new(0x6000_0000, 0x100)).ok();

        let intc_id = reg.register("intc", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(intc_id, MemRegion::new(0x600C_2000, 0x200)).ok();

        let st = reg.register("systimer", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(st, MemRegion::new(0x6002_3000, 0x80)).ok();
        reg.grant_irq(st, SYSTIMER_CPU_INT as u16);
    }
    let _ = writeln!(con, "[boot] driver registry: 3 drivers registered");

    // ── register BLE driver ──────────────────────────────
    #[cfg(feature = "ble")]
    {
        unsafe {
            let reg = &mut *DRIVERS.0.get();
            let _ = reg.register("ble", DriverCaps {
                mmio_regions: 1,
                uses_interrupts: true,
                uses_dma: false,
                uses_network: false,
            });
        }
        let _ = writeln!(con, "[boot] BLE 5.0 driver registered");
    }

    // ── register IEEE 802.15.4 driver ────────────────────
    #[cfg(feature = "ieee802154")]
    {
        unsafe {
            let reg = &mut *DRIVERS.0.get();
            let drv = reg.register("ieee802154", DriverCaps {
                mmio_regions: 1,
                uses_interrupts: true,
                uses_dma: false,
                uses_network: true,
            });
            if let Ok(id) = drv {
                reg.grant_mmio(id, MemRegion::new(0x600A_3000, 0x1000)).ok();
            }
        }
        let _ = writeln!(con, "[boot] IEEE 802.15.4 (ZigBee/Thread) driver registered");
    }

    // ── install trap vector (RISC-V only) ────────────────────
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
    intc.map_source(SYSTIMER_IRQ_SOURCE, SYSTIMER_CPU_INT);
    intc.set_priority(SYSTIMER_CPU_INT as u16, 1);
    intc.set_threshold(0);
    intc.enable_interrupt(SYSTIMER_CPU_INT as u16);
    let _ = writeln!(con, "[boot] interrupt controller configured");

    // ── system timer tick ────────────────────────────────────
    let timer = system_timer();
    timer.configure_tick(TICK_PERIOD_US);
    unsafe { *TIMER.0.get() = timer; }
    let _ = writeln!(con, "[boot] systimer tick @ {} us", TICK_PERIOD_US);

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
        #[cfg(feature = "shell")]
        {
            let sb = SHELL_STACK.0.as_ptr() as usize;
            let st = sb + SHELL_STACK.0.len();
            if let Some(idx) = sched.create_task("shell", shell_task as *const () as usize, st, sb, 1) {
                sched.tasks[idx].context.status = INITIAL_MSTATUS;
            }
        }
    }
    let _ = writeln!(con, "[boot] idle task registered");
    #[cfg(feature = "shell")]
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
            let _ = ctx_ptr;
            drop(con);
            #[cfg(feature = "shell")]
            shell_task();
            #[cfg(not(feature = "shell"))]
            idle_task();
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler / driver query callbacks (injected into ShellEnv)
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
fn get_uptime_ticks() -> u64 {
    unsafe { (*SCHEDULER.0.get()).ticks }
}

#[cfg(feature = "shell")]
fn write_mem_info(w: &mut dyn core::fmt::Write) {
    unsafe {
        (*HEAP.0.get()).write_stats(w);
    }
}

#[cfg(feature = "shell")]
fn write_driver_list(w: &mut dyn core::fmt::Write) {
    unsafe {
        (*DRIVERS.0.get()).write_list(w);
    }
}

#[cfg(feature = "shell")]
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

/// Shell callback for `wifi <sub> <args>`.
///
/// Subcommands:
///   set <ssid> [password]   — configure credentials
///   connect                 — attempt to join the AP
///   disconnect              — leave the AP
///   status                  — show current state
#[cfg(feature = "wifi")]
fn wifi_command(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    let mgr = unsafe { &mut *WIFI.0.get() };

    match sub {
        "set" => {
            // args = "MySSID MyPassword" or "MySSID" (open network)
            let (ssid, pass) = match args.find(' ') {
                Some(i) => (&args[..i], args[i + 1..].trim()),
                None => (args, ""),
            };
            if ssid.is_empty() {
                let _ = writeln!(w, "  usage: wifi set <ssid> [password]");
                return;
            }
            mgr.set_credentials(ssid.as_bytes(), pass.as_bytes());
            let _ = writeln!(w, "  Wi-Fi credentials set (SSID: {})", ssid);
        }
        "connect" => {
            let _ = write!(w, "  Connecting...");
            match mgr.connect() {
                Ok(()) => {
                    let _ = writeln!(w, " connected!");
                }
                Err(e) => {
                    let _ = writeln!(w, " failed: {}", e);
                }
            }
        }
        "disconnect" => {
            mgr.disconnect();
            let _ = writeln!(w, "  Disconnected.");
        }
        "scan" => {
            let _ = write!(w, "  Scanning...");
            match mgr.scan() {
                Ok(n) => {
                    let _ = writeln!(w, " found {} network(s)", n);
                    mgr.write_scan_results(w);
                }
                Err(e) => {
                    let _ = writeln!(w, " failed: {}", e);
                }
            }
        }
        "list" | "ls" => {
            mgr.write_scan_results(w);
        }
        "status" | "info" | "" => {
            mgr.write_status(w);
        }
        _ => {
            let _ = writeln!(w, "  wifi subcommands:");
            let _ = writeln!(w, "    wifi scan                  Scan for networks");
            let _ = writeln!(w, "    wifi list                  Show last scan results");
            let _ = writeln!(w, "    wifi set <ssid> [password]  Set credentials");
            let _ = writeln!(w, "    wifi connect               Connect to AP");
            let _ = writeln!(w, "    wifi disconnect             Leave AP");
            let _ = writeln!(w, "    wifi status                Current state");
        }
    }
}

/// Shell callback for `bt <sub> <args>`.
///
/// Subcommands:
///   scan                  — scan for nearby BLE devices
///   list                  — show last scan results
///   advertise <name>      — start advertising as <name>
///   stop                  — stop advertising
///   status                — show BLE state
#[cfg(feature = "ble")]
fn bt_command(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    let mgr = unsafe { &mut *BLE.0.get() };

    match sub {
        "scan" => {
            let _ = write!(w, "  Scanning for BLE devices...");
            match mgr.scan() {
                Ok(n) => {
                    let _ = writeln!(w, " found {} device(s)", n);
                    mgr.write_scan_results(w);
                }
                Err(e) => {
                    let _ = writeln!(w, " failed: {}", e);
                }
            }
        }
        "list" | "ls" => {
            mgr.write_scan_results(w);
        }
        "advertise" | "adv" => {
            if args.is_empty() {
                let _ = writeln!(w, "  usage: bt advertise <name>");
                return;
            }
            match mgr.advertise(args.as_bytes()) {
                Ok(()) => {
                    let _ = writeln!(w, "  Advertising as '{}'", args);
                }
                Err(e) => {
                    let _ = writeln!(w, "  Failed to start advertising: {}", e);
                }
            }
        }
        "stop" => {
            mgr.stop();
            let _ = writeln!(w, "  Advertising stopped.");
        }
        "status" | "info" | "" => {
            mgr.write_status(w);
        }
        _ => {
            let _ = writeln!(w, "  bt subcommands:");
            let _ = writeln!(w, "    bt scan                    Scan for BLE devices");
            let _ = writeln!(w, "    bt list                    Show last scan results");
            let _ = writeln!(w, "    bt advertise <name>        Start advertising");
            let _ = writeln!(w, "    bt stop                    Stop advertising");
            let _ = writeln!(w, "    bt status                  Current BLE state");
        }
    }
}

/// Shell callback for `zigbee <sub> <args>`.
///
/// Subcommands:
///   init                — initialise the 802.15.4 radio
///   channel <11-26>     — set the operating channel
///   panid <0xNNNN>      — set the PAN ID
///   scan                — scan for 802.15.4 networks
///   list                — show last scan results
///   send <data>         — transmit a test frame
///   status              — show radio state
#[cfg(feature = "ieee802154")]
fn zigbee_command(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    let mgr = unsafe { &mut *RADIO_802154.0.get() };

    match sub {
        "init" => {
            let _ = write!(w, "  Initialising 802.15.4 radio...");
            match mgr.init() {
                Ok(()) => {
                    let _ = writeln!(w, " done");
                }
                Err(e) => {
                    let _ = writeln!(w, " failed: {}", e);
                }
            }
        }
        "channel" | "ch" => {
            if args.is_empty() {
                let _ = writeln!(w, "  Current channel: {}", mgr.driver().channel());
                return;
            }
            match args.parse::<u8>() {
                Ok(ch) => match mgr.set_channel(ch) {
                    Ok(()) => {
                        let _ = writeln!(w, "  Channel set to {}", ch);
                    }
                    Err(e) => {
                        let _ = writeln!(w, "  Error: {}", e);
                    }
                },
                Err(_) => {
                    let _ = writeln!(w, "  Invalid channel number (must be 11-26)");
                }
            }
        }
        "panid" => {
            if args.is_empty() {
                let _ = writeln!(w, "  Current PAN ID: 0x{:04X}", mgr.driver().pan_id());
                return;
            }
            // Parse hex with optional 0x prefix
            let hex_str = args.strip_prefix("0x").or_else(|| args.strip_prefix("0X")).unwrap_or(args);
            match u16::from_str_radix(hex_str, 16) {
                Ok(pan_id) => {
                    mgr.set_pan_id(pan_id);
                    let _ = writeln!(w, "  PAN ID set to 0x{:04X}", pan_id);
                }
                Err(_) => {
                    let _ = writeln!(w, "  Invalid PAN ID (use hex, e.g. 0x1234)");
                }
            }
        }
        "scan" => {
            let _ = write!(w, "  Scanning 802.15.4 channels...");
            match mgr.scan() {
                Ok(n) => {
                    let _ = writeln!(w, " found {} network(s)", n);
                    mgr.write_scan_results(w);
                }
                Err(e) => {
                    let _ = writeln!(w, " failed: {}", e);
                }
            }
        }
        "list" | "ls" => {
            mgr.write_scan_results(w);
        }
        "send" | "tx" => {
            if args.is_empty() {
                let _ = writeln!(w, "  usage: zigbee send <data>");
                return;
            }
            match mgr.send(args.as_bytes()) {
                Ok(()) => {
                    let _ = writeln!(w, "  Frame sent ({} bytes)", args.len());
                }
                Err(e) => {
                    let _ = writeln!(w, "  TX failed: {}", e);
                }
            }
        }
        "status" | "info" | "" => {
            mgr.write_status(w);
        }
        _ => {
            let _ = writeln!(w, "  zigbee subcommands:");
            let _ = writeln!(w, "    zigbee init                Init the 802.15.4 radio");
            let _ = writeln!(w, "    zigbee channel <11-26>     Set/show channel");
            let _ = writeln!(w, "    zigbee panid <0xNNNN>      Set/show PAN ID");
            let _ = writeln!(w, "    zigbee scan                Scan for networks");
            let _ = writeln!(w, "    zigbee list                Show last scan results");
            let _ = writeln!(w, "    zigbee send <data>         Transmit test frame");
            let _ = writeln!(w, "    zigbee status              Current radio state");
        }
    }
}

// ---------------------------------------------------------------------------
// Console I/O callbacks (used by the syscall dispatcher)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub(crate) fn console_write_byte(b: u8) {
    let serial = default_serial();
    let mut con = Console::new(serial);
    let _ = con.write_str(unsafe {
        core::str::from_utf8_unchecked(core::slice::from_ref(&b))
    });
}

#[allow(dead_code)]
pub(crate) fn console_read_byte() -> u8 {
    // Blocking read not yet supported on ESP32 — return 0xFF (no data).
    0xFF
}
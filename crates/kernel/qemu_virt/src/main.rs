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

use arch::{Console, NetworkDevice, TickTimer};
use soc_qemu_virt::{default_serial, system_timer, clint::Clint, QemuVirt};
use microkernel::Kernel;
use microkernel::alloc::Heap;
use microkernel::driver::{DriverCaps, DriverRegistry, MemRegion};
use microkernel::task::{Scheduler, TaskState};
use net::{NetStack, NetStorage, TcpSerial};
use shell::{Shell, ShellEnv};
use smoltcp::iface::SocketSet;
use smoltcp::wire::{IpCidr, Ipv4Address};

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
// Kernel heap (64 KiB — small pool 16 KiB, large pool 48 KiB)
// ---------------------------------------------------------------------------

/// QEMU virt has 16+ MiB RAM; 64 KiB for the heap is conservative.
const HEAP_SIZE: usize = 64 * 1024;
const HEAP_SMALL_BYTES: usize = 16 * 1024;

#[repr(align(16))]
struct HeapRegion([u8; HEAP_SIZE]);
static mut HEAP_REGION: HeapRegion = HeapRegion([0u8; HEAP_SIZE]);

struct HeapCell(UnsafeCell<Heap>);
unsafe impl Sync for HeapCell {}
static HEAP: HeapCell = HeapCell(UnsafeCell::new(Heap::new()));

// ---------------------------------------------------------------------------
// Driver registry
// ---------------------------------------------------------------------------

struct RegistryCell(UnsafeCell<DriverRegistry>);
unsafe impl Sync for RegistryCell {}
static DRIVERS: RegistryCell = RegistryCell(UnsafeCell::new(DriverRegistry::new()));

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
        get_mem_info: Some(write_mem_info),
        get_driver_list: Some(write_driver_list),
        wifi_cmd: None,
    };
    let mut sh = Shell::new(env);
    sh.run(&mut con);

    // User typed `exit` — power off QEMU.
    let _ = writeln!(con, "VeerOS halted \u{2014} powering off.");
    qemu_poweroff();
}

// ---------------------------------------------------------------------------
// Network / remote-shell task
// ---------------------------------------------------------------------------
/// TCP port for VeerOS remote shell (like SSH, unencrypted for now).
const REMOTE_SHELL_PORT: u16 = 2323;

/// FNV-1a hash of the remote shell password.
/// Default: "veeros" — override by changing this constant.
const REMOTE_PASSWORD_HASH: u32 = net::auth::fnv1a(b"veeros");

/// QEMU user-net default: guest is 10.0.2.15, gateway 10.0.2.2.
const GUEST_IP: [u8; 4] = [10, 0, 2, 15];
const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];

#[repr(align(16))]
struct NetStack0([u8; 8192]);
static NET_TASK_STACK: NetStack0 = NetStack0([0u8; 8192]);

// Static smoltcp socket-set storage (one socket for the listener).
static mut SOCKET_STORAGE: [smoltcp::iface::SocketStorage<'static>; 4] =
    [smoltcp::iface::SocketStorage::EMPTY; 4];
static mut NET_STORAGE: NetStorage = NetStorage::new();

// The network stack and socket set are stored globally so the poll_fn
// callback (called from TcpSerial) can drive them.
struct NetCell(UnsafeCell<Option<NetStack<soc_qemu_virt::virtio_net::VirtioNet>>>);
unsafe impl Sync for NetCell {}
static NET: NetCell = NetCell(UnsafeCell::new(None));

struct SocketSetCell(UnsafeCell<Option<SocketSet<'static>>>);
unsafe impl Sync for SocketSetCell {}
static SOCKETS: SocketSetCell = SocketSetCell(UnsafeCell::new(None));

/// Global poll function handed to TcpSerial so it can drive the stack
/// while blocking on read_byte / write_byte.
fn net_poll() {
    unsafe {
        if let (Some(stack), Some(sockets)) =
            (&mut *NET.0.get(), &mut *SOCKETS.0.get())
        {
            let ticks = (*SCHEDULER.0.get()).ticks;
            stack.poll(sockets, ticks);
        }
    }
}

/// The network listener task.
///
/// 1. Probes for the VIRTIO-NET device.
/// 2. Initialises smoltcp with a static IP.
/// 3. Listens on REMOTE_SHELL_PORT.
/// 4. On connection → runs a shell session over TCP.
/// 5. When the client disconnects, loops back to listen.
fn net_task() -> ! {
    // ── early console for log messages ───────────────────────
    let serial = default_serial();
    let mut con = Console::new(serial);

    let _ = writeln!(con, "[net] probing for VIRTIO-NET device...");

    let nic = match soc_qemu_virt::virtio_net::VirtioNet::probe() {
        Some(n) => n,
        None => {
            let _ = writeln!(con, "[net] no VIRTIO-NET device found — task halted");
            loop {
                #[cfg(target_arch = "riscv32")]
                unsafe {
                    core::arch::asm!("wfi", options(nomem, nostack));
                }
                #[cfg(not(target_arch = "riscv32"))]
                core::hint::spin_loop();
            }
        }
    };

    let mac = nic.mac_address();
    let _ = writeln!(
        con,
        "[net] found NIC  MMIO-v{}  MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        nic.mmio_version(), mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    // ── initialise smoltcp ───────────────────────────────────
    let ip = IpCidr::new(
        Ipv4Address::new(GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3]).into(),
        24,
    );
    let gw = Ipv4Address::new(GATEWAY_IP[0], GATEWAY_IP[1], GATEWAY_IP[2], GATEWAY_IP[3]);

    unsafe {
        let sockets_ref: &'static mut [smoltcp::iface::SocketStorage<'static>] =
            &mut *core::ptr::addr_of_mut!(SOCKET_STORAGE);
        let mut socket_set = SocketSet::new(sockets_ref);
        let storage = &mut *core::ptr::addr_of_mut!(NET_STORAGE);

        let stack = NetStack::new(nic, ip, gw, &mut socket_set, storage);

        // Store globally so net_poll() can reach them.
        *SOCKETS.0.get() = Some(socket_set);
        *NET.0.get() = Some(stack);
    }

    let _ = writeln!(
        con,
        "[net] IP {}.{}.{}.{} — listening on port {}",
        GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3], REMOTE_SHELL_PORT
    );

    // ── main accept loop ─────────────────────────────────────
    loop {
        // Start listening.
        unsafe {
            if let (Some(stack), Some(sockets)) =
                (&mut *NET.0.get(), &mut *SOCKETS.0.get())
            {
                stack.listen(sockets, REMOTE_SHELL_PORT);
            }
        }

        // Poll until a client connects.
        loop {
            net_poll();
            let connected = unsafe {
                if let (Some(stack), Some(sockets)) =
                    (&*NET.0.get(), &*SOCKETS.0.get())
                {
                    stack.is_connected(sockets)
                } else {
                    false
                }
            };
            if connected {
                break;
            }
            core::hint::spin_loop();
        }

        let _ = writeln!(con, "[net] client connected");

        // ── authenticate, then run the shell over TCP ────────
        unsafe {
            let handle = (*NET.0.get()).as_ref().unwrap().tcp_handle();
            let socket_set_ptr = (*SOCKETS.0.get()).as_mut().unwrap() as *mut SocketSet<'static>;
            let tcp_serial = TcpSerial::new(handle, socket_set_ptr, net_poll);
            let mut tcp_con = Console::new(tcp_serial);

            if net::auth::login_prompt(&mut tcp_con, REMOTE_PASSWORD_HASH) {
                let _ = writeln!(con, "[net] authentication succeeded — starting shell");
                let env = ShellEnv {
                    version: VERSION,
                    platform: "QEMU virt (RISC-V 32) [remote]",
                    scheduler: "minimal",
                    get_uptime_ticks: Some(get_uptime_ticks),
                    get_task_list: Some(write_task_list),
                    get_mem_info: Some(write_mem_info),
                    get_driver_list: Some(write_driver_list),
                    wifi_cmd: None,
                };
                let mut sh = Shell::new(env);
                sh.run(&mut tcp_con);
            } else {
                let _ = writeln!(con, "[net] authentication failed");
            }
        }

        let _ = writeln!(con, "[net] client disconnected — re-listening");

        // Abort the socket so it can be re-used immediately (skip TIME_WAIT).
        unsafe {
            if let Some(sockets) = &mut *SOCKETS.0.get() {
                let handle = (*NET.0.get()).as_ref().unwrap().tcp_handle();
                let socket = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
                socket.abort();
            }
        }

        // One final poll to flush the RST.
        net_poll();
    }
}

// ---------------------------------------------------------------------------
// Scheduler query callbacks (injected into the shell via ShellEnv)
// ---------------------------------------------------------------------------

fn get_uptime_ticks() -> u64 {
    unsafe { (*SCHEDULER.0.get()).ticks }
}

fn write_mem_info(w: &mut dyn core::fmt::Write) {
    unsafe {
        (*HEAP.0.get()).write_stats(w);
    }
}

fn write_driver_list(w: &mut dyn core::fmt::Write) {
    unsafe {
        (*DRIVERS.0.get()).write_list(w);
    }
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

    // ── kernel heap ──────────────────────────────────────────
    unsafe {
        let region = &mut *core::ptr::addr_of_mut!(HEAP_REGION);
        (*HEAP.0.get()).init(&mut region.0, HEAP_SMALL_BYTES);
    }
    let _ = writeln!(
        con,
        "[boot] heap initialised ({} KiB — small {}K + large {}K)",
        HEAP_SIZE / 1024,
        HEAP_SMALL_BYTES / 1024,
        (HEAP_SIZE - HEAP_SMALL_BYTES) / 1024,
    );

    // ── register drivers ─────────────────────────────────────
    unsafe {
        let reg = &mut *DRIVERS.0.get();

        // NS16550a UART
        let uart = reg.register("uart0", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(uart, MemRegion::new(0x1000_0000, 0x100)).ok();
        reg.grant_irq(uart, 10); // UART IRQ on QEMU virt

        // CLINT timer
        let clint = reg.register("clint", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(clint, MemRegion::new(0x0200_0000, 0x10000)).ok();
        reg.grant_irq(clint, 7); // machine timer

        // VIRTIO-NET
        let vnet = reg.register("virtio-net", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: true,
            uses_dma: true,
            uses_network: true,
        }).unwrap();
        reg.grant_mmio(vnet, MemRegion::new(0x1000_1000, 0x1000)).ok();
        reg.grant_irq(vnet, 1);
    }
    let _ = writeln!(con, "[boot] driver registry: 3 drivers registered");

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

        // Network listener task (priority 1).
        let sb = NET_TASK_STACK.0.as_ptr() as usize;
        let st = sb + NET_TASK_STACK.0.len();
        if let Some(idx) = sched.create_task("net", net_task as *const () as usize, st, sb, 1) {
            sched.tasks[idx].context.status = INITIAL_MSTATUS;
        }
    }
    let _ = writeln!(con, "[boot] idle task registered");
    let _ = writeln!(con, "[boot] shell task registered");
    let _ = writeln!(con, "[boot] net listener task registered (port {})", REMOTE_SHELL_PORT);

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

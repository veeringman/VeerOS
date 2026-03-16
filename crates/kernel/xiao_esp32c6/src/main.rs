#![no_std]
#![no_main]

mod trap;

use core::cell::UnsafeCell;
use core::fmt::Write;

use arch::{Console, InterruptController, SavedContext, TickTimer};
use soc_esp32::{
    default_serial, interrupt_controller, system_timer,
    systimer::SysTimer, Esp32Riscv,
};
use microkernel::Kernel;
use microkernel::alloc::Heap;
use microkernel::channel::Channels;
use microkernel::driver::{DriverCaps, DriverRegistry, MemRegion};
use microkernel::futex::FutexTable;
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
// Futex table
// ---------------------------------------------------------------------------

pub(crate) struct FutexCell(pub UnsafeCell<FutexTable>);
unsafe impl Sync for FutexCell {}
pub(crate) static FUTEX: FutexCell = FutexCell(UnsafeCell::new(FutexTable::new()));

// ---------------------------------------------------------------------------
// Channel pool
// ---------------------------------------------------------------------------

pub(crate) struct ChannelCell(pub UnsafeCell<Channels>);
unsafe impl Sync for ChannelCell {}
pub(crate) static CHANNELS: ChannelCell = ChannelCell(UnsafeCell::new(Channels::new()));

// ---------------------------------------------------------------------------
// Poll table
// ---------------------------------------------------------------------------

use microkernel::poll::PollTable;

pub(crate) struct PollCell(pub UnsafeCell<PollTable>);
unsafe impl Sync for PollCell {}
pub(crate) static POLL: PollCell = PollCell(UnsafeCell::new(PollTable::new()));

// ---------------------------------------------------------------------------
// Process table
// ---------------------------------------------------------------------------

use microkernel::process::ProcessTable;

pub(crate) struct ProcessCell(pub UnsafeCell<ProcessTable>);
unsafe impl Sync for ProcessCell {}
pub(crate) static PROCESSES: ProcessCell = ProcessCell(UnsafeCell::new(ProcessTable::new()));

// ---------------------------------------------------------------------------
// Socket table
// ---------------------------------------------------------------------------

use microkernel::socket::SocketTable;

pub(crate) struct SocketCell(pub UnsafeCell<SocketTable>);
unsafe impl Sync for SocketCell {}
pub(crate) static SOCKETS: SocketCell = SocketCell(UnsafeCell::new(SocketTable::new()));

// ---------------------------------------------------------------------------
// User table
// ---------------------------------------------------------------------------

use microkernel::user::UserTable;

pub(crate) struct UserCell(pub UnsafeCell<UserTable>);
unsafe impl Sync for UserCell {}
pub(crate) static USERS: UserCell = UserCell(UnsafeCell::new(UserTable::new()));

// ---------------------------------------------------------------------------
// VFS inode table
// ---------------------------------------------------------------------------

use microkernel::vfs::InodeTable;

pub(crate) struct InodeCell(pub UnsafeCell<InodeTable>);
unsafe impl Sync for InodeCell {}
pub(crate) static INODES: InodeCell = InodeCell(UnsafeCell::new(InodeTable::new()));

// ---------------------------------------------------------------------------
// RamFS
// ---------------------------------------------------------------------------

use microkernel::ramfs::RamFs;

pub(crate) struct RamFsCell(pub UnsafeCell<RamFs>);
unsafe impl Sync for RamFsCell {}
pub(crate) static RAMFS: RamFsCell = RamFsCell(UnsafeCell::new(RamFs::new()));

// ---------------------------------------------------------------------------
// FAT32 filesystem state
// ---------------------------------------------------------------------------

use microkernel::fat32::Fat32;

pub(crate) struct Fat32Cell(pub UnsafeCell<Fat32>);
unsafe impl Sync for Fat32Cell {}
pub(crate) static FAT32: Fat32Cell = Fat32Cell(UnsafeCell::new(Fat32::new()));

// ---------------------------------------------------------------------------
// Mount table
// ---------------------------------------------------------------------------

use microkernel::vfs::MountTable;

pub(crate) struct MountCell(pub UnsafeCell<MountTable>);
unsafe impl Sync for MountCell {}
pub(crate) static MOUNTS: MountCell = MountCell(UnsafeCell::new(MountTable::new()));

// ---------------------------------------------------------------------------
// Input subsystem
// ---------------------------------------------------------------------------

use microkernel::input::InputSubsystem;

pub(crate) struct InputCell(pub UnsafeCell<InputSubsystem>);
unsafe impl Sync for InputCell {}
pub(crate) static INPUT: InputCell = InputCell(UnsafeCell::new(InputSubsystem::new()));

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

// ---------------------------------------------------------------------------
// Kernel log ring buffer
// ---------------------------------------------------------------------------

use microkernel::klog::KernelLog;

pub(crate) struct KlogCell(pub UnsafeCell<KernelLog>);
unsafe impl Sync for KlogCell {}
pub(crate) static KLOG: KlogCell = KlogCell(UnsafeCell::new(KernelLog::new()));

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
        get_current_user: Some(get_current_user),
        get_user_list: Some(write_user_list),
        vfs_list_dir: Some(vfs_list_dir),
        vfs_read_file: Some(vfs_read_file),
        vfs_write_file: Some(vfs_write_file),
        vfs_mkdir: Some(vfs_mkdir),
        vfs_stat: Some(vfs_stat),
        vfs_unlink: Some(vfs_unlink),
        vfs_rename: Some(vfs_rename),
        vfs_getcwd: Some(vfs_getcwd),
        vfs_chdir: Some(vfs_chdir),
        vfs_tree: Some(vfs_tree),
        vfs_touch: Some(vfs_touch),
        mount_list: Some(mount_list),
        mount_fs: None,
        umount_fs: None,
        lsblk: Some(lsblk_info),
        input_status: Some(input_status),
        usb_list: None,
        ble_hid_list: Some(ble_hid_list),
        gpio_cmd: None,
        i2c_cmd: None,
        spi_cmd: None,
        hw_info: None,
        get_temp_millic: None,
        dmesg: Some(dmesg_info),
        reboot: None,
        shutdown: None,
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

    // ── VFS initialisation ───────────────────────────────────
    unsafe {
        let inodes = &mut *INODES.0.get();
        let ramfs = &mut *RAMFS.0.get();
        inodes.init_root();
        let dev_id = inodes.resolve(microkernel::vfs::ROOT_INODE, "/dev").unwrap_or(microkernel::vfs::NO_INODE);
        if dev_id != microkernel::vfs::NO_INODE {
            inodes.create_device_in(dev_id, "null", 0, 0);
            inodes.create_device_in(dev_id, "zero", 0, 1);
            inodes.create_device_in(dev_id, "console", 0, 2);
            inodes.create_device_in(dev_id, "random", 0, 3);
            inodes.create_device_in(dev_id, "keyboard", 1, 0);
            inodes.create_device_in(dev_id, "mouse", 1, 1);
        }
        let etc_id = inodes.resolve(microkernel::vfs::ROOT_INODE, "/etc").unwrap_or(microkernel::vfs::NO_INODE);
        if etc_id != microkernel::vfs::NO_INODE {
            ramfs.create_with_content(inodes, etc_id, "motd", b"Welcome to VeerOS!\n");
            ramfs.create_with_content(inodes, etc_id, "hostname", b"veeros-esp32c6\n");
        }
    }
    let _ = writeln!(con, "[boot] VFS initialised (ramfs {} KiB)", microkernel::ramfs::RAMFS_POOL_SIZE / 1024);

    // ── scheduler + tasks ────────────────────────────────────
    unsafe {
        let sched = &mut *SCHEDULER.0.get();
        let procs = &mut *PROCESSES.0.get();

        // Create process 0 (init/kernel process).
        procs.create("init", usize::MAX, 0, 0);

        // Initialize user table with default accounts.
        let user_tbl = &mut *USERS.0.get();
        user_tbl.init_defaults();

        // Idle task (priority 0).
        let sb = IDLE_STACK.0.as_ptr() as usize;
        let st = sb + IDLE_STACK.0.len();
        if let Some(idx) = sched.create_task("idle", idle_task as *const () as usize, st, sb, 0, 0) {
            sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
        }

        // Shell task (priority 1).
        #[cfg(feature = "shell")]
        {
            let sb = SHELL_STACK.0.as_ptr() as usize;
            let st = sb + SHELL_STACK.0.len();
            if let Some(idx) = sched.create_task("shell", shell_task as *const () as usize, st, sb, 1, 0) {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
            }
        }
    }
    // Set init process thread count to match all boot tasks.
    unsafe {
        use microkernel::task::TaskState;
        let sched = &*SCHEDULER.0.get();
        let procs = &mut *PROCESSES.0.get();
        let count = sched.tasks.iter().filter(|t| t.state != TaskState::Free).count();
        procs.processes[0].thread_count = count;
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
                TaskState::Suspended => "suspend",
                TaskState::Zombie => "zombie",
            };
            let _ = writeln!(w, "  {:2}  {:8}  {:3}  {}", i, st, t.priority, t.name);
        }
    }
    let _ = writeln!(w, "  ticks: {}", sched.ticks);
}

#[cfg(feature = "shell")]
fn get_current_user() -> (u16, &'static str) {
    unsafe {
        let users = &*USERS.0.get();
        let uid = (*PROCESSES.0.get()).processes[0].uid;
        (uid, users.name_for_uid(uid))
    }
}

#[cfg(feature = "shell")]
fn write_user_list(w: &mut dyn core::fmt::Write) {
    unsafe {
        let users = &*USERS.0.get();
        let _ = writeln!(w, "  USER     UID  STATUS");
        let _ = writeln!(w, "  -------  ---  ------");
        for i in 0..users.user_count {
            let u = &users.users[i];
            let active = users.sessions.iter().any(|s| s.active && s.uid == u.uid);
            let st = if active { "active" } else { "      " };
            let _ = writeln!(w, "  {:7}  {:3}  {}", u.name, u.uid, st);
        }
    }
}

// ---------------------------------------------------------------------------
// VFS callbacks (injected into ShellEnv)
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
fn vfs_list_dir(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let dir_id = if path == "." { Some(cwd) } else { inodes.resolve(cwd, path) };
        match dir_id {
            Some(id) if id != NO_INODE => {
                let inode = &inodes.inodes[id as usize];
                if inode.kind != InodeKind::Directory {
                    let _ = writeln!(w, "ls: '{}': not a directory", path);
                    return;
                }
                let mut child = inode.children_head;
                while child != NO_INODE {
                    let c = &inodes.inodes[child as usize];
                    let kind_ch = match c.kind {
                        InodeKind::Directory => 'd',
                        InodeKind::File => 'f',
                        InodeKind::Device => 'c',
                        _ => '?',
                    };
                    let _ = writeln!(w, "  {}  {:6}  {}", kind_ch, c.size, c.name_str());
                    child = c.next_sibling;
                }
            }
            _ => { let _ = writeln!(w, "ls: '{}': no such directory", path); }
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_read_file(path: &str, buf: &mut [u8]) -> usize {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let ramfs = &*RAMFS.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE { return 0; }
        if inodes.inodes[id as usize].kind != InodeKind::File { return 0; }
        ramfs.read(inodes, id, 0, buf)
    }
}

#[cfg(feature = "shell")]
fn vfs_write_file(path: &str, data: &[u8], append: bool) -> bool {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &mut *INODES.0.get();
        let ramfs = &mut *RAMFS.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let mut id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE {
            if let Some(slash) = path.rfind('/') {
                let parent_path = if slash == 0 { "/" } else { &path[..slash] };
                let name = &path[slash + 1..];
                let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
                if parent == NO_INODE || name.is_empty() { return false; }
                id = match inodes.create_file_in(parent, name) { Some(i) => i, None => return false };
            } else {
                id = match inodes.create_file_in(cwd, path) { Some(i) => i, None => return false };
            }
        }
        if inodes.inodes[id as usize].kind != InodeKind::File { return false; }
        let offset = if append { inodes.inodes[id as usize].size } else { 0 };
        if !append { ramfs.truncate(inodes, id, 0); }
        ramfs.write(inodes, id, offset, data) > 0
    }
}

#[cfg(feature = "shell")]
fn vfs_mkdir(path: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        if let Some(slash) = path.rfind('/') {
            let parent_path = if slash == 0 { "/" } else { &path[..slash] };
            let name = &path[slash + 1..];
            let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
            if parent == NO_INODE || name.is_empty() { return false; }
            inodes.mkdir_in(parent, name).is_some()
        } else {
            inodes.mkdir_in(cwd, path).is_some()
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_stat(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE {
            let _ = writeln!(w, "stat: '{}': no such file or directory", path);
            return;
        }
        let inode = &inodes.inodes[id as usize];
        let kind = match inode.kind { InodeKind::File => "file", InodeKind::Directory => "directory", InodeKind::Device => "device", _ => "unknown" };
        let _ = writeln!(w, "  File: {}", inode.name_str());
        let _ = writeln!(w, "  Type: {}", kind);
        let _ = writeln!(w, "  Size: {}", inode.size);
        let _ = writeln!(w, "  Inode: {}", id);
        let _ = writeln!(w, "  Parent: {}", inode.parent);
        if inode.kind == InodeKind::Device {
            let _ = writeln!(w, "  Device: {},{}", inode.dev_major, inode.dev_minor);
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_unlink(path: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE { return false; }
        inodes.unlink(id)
    }
}

#[cfg(feature = "shell")]
fn vfs_rename(old: &str, new: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let id = inodes.resolve(cwd, old).unwrap_or(NO_INODE);
        if id == NO_INODE { return false; }
        if let Some(slash) = new.rfind('/') {
            let parent_path = if slash == 0 { "/" } else { &new[..slash] };
            let name = &new[slash + 1..];
            let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
            if parent == NO_INODE || name.is_empty() { return false; }
            inodes.rename(id, parent, name)
        } else {
            inodes.rename(id, cwd, new)
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_getcwd(buf: &mut [u8]) -> usize {
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        inodes.build_path(cwd, buf)
    }
}

#[cfg(feature = "shell")]
fn vfs_chdir(path: &str) -> bool {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let procs = &mut *PROCESSES.0.get();
        let cwd = procs.processes[0].cwd;
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE { return false; }
        if inodes.inodes[id as usize].kind != InodeKind::Directory { return false; }
        procs.processes[0].cwd = id;
        true
    }
}

#[cfg(feature = "shell")]
fn vfs_tree(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, ROOT_INODE, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let start = if path == "/" { ROOT_INODE } else { inodes.resolve(cwd, path).unwrap_or(NO_INODE) };
        if start == NO_INODE { let _ = writeln!(w, "tree: '{}': no such directory", path); return; }
        let root = &inodes.inodes[start as usize];
        if root.kind != InodeKind::Directory { let _ = writeln!(w, "tree: '{}': not a directory", path); return; }
        let _ = writeln!(w, "{}", if path == "/" || path == "." { "/" } else { path });
        let mut stack: [(u16, u8); 64] = [(NO_INODE, 0); 64];
        let mut sp = 0usize;
        let mut kids: [u16; 64] = [NO_INODE; 64];
        let mut nk = 0usize;
        let mut ch = root.children_head;
        while ch != NO_INODE && nk < 64 { kids[nk] = ch; nk += 1; ch = inodes.inodes[ch as usize].next_sibling; }
        let mut i = nk;
        while i > 0 { i -= 1; if sp < 64 { stack[sp] = (kids[i], 1); sp += 1; } }
        while sp > 0 {
            sp -= 1;
            let (id, depth) = stack[sp];
            let node = &inodes.inodes[id as usize];
            for _ in 0..depth { w.write_str("  ").ok(); }
            let kind_ch = match node.kind { InodeKind::Directory => '/', InodeKind::Device => '*', _ => ' ' };
            let _ = writeln!(w, "{}{}", node.name_str(), kind_ch);
            if node.kind == InodeKind::Directory {
                let mut ck: [u16; 64] = [NO_INODE; 64];
                let mut cn = 0usize;
                let mut c = node.children_head;
                while c != NO_INODE && cn < 64 { ck[cn] = c; cn += 1; c = inodes.inodes[c as usize].next_sibling; }
                let mut j = cn;
                while j > 0 { j -= 1; if sp < 64 { stack[sp] = (ck[j], depth + 1); sp += 1; } }
            }
        }
    }
}

#[cfg(feature = "shell")]
fn vfs_touch(path: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        if inodes.resolve(cwd, path).unwrap_or(NO_INODE) != NO_INODE { return true; }
        if let Some(slash) = path.rfind('/') {
            let parent_path = if slash == 0 { "/" } else { &path[..slash] };
            let name = &path[slash + 1..];
            let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
            if parent == NO_INODE || name.is_empty() { return false; }
            inodes.create_file_in(parent, name).is_some()
        } else {
            inodes.create_file_in(cwd, path).is_some()
        }
    }
}

fn mount_list(w: &mut dyn core::fmt::Write) {
    unsafe {
        let mounts = &*MOUNTS.0.get();
        let inodes = &*INODES.0.get();
        let mut found = false;
        for (i, m) in mounts.mounts.iter().enumerate() {
            if m.active {
                let mut pathbuf = [0u8; 64];
                let plen = inodes.build_path(m.dir_inode, &mut pathbuf);
                let path = core::str::from_utf8(&pathbuf[..plen]).unwrap_or("?");
                let fstype = match m.fs_type {
                    microkernel::vfs::FsType::Fat32 => "fat32",
                    microkernel::vfs::FsType::RamFs => "ramfs",
                    _ => "none",
                };
                let _ = writeln!(w, "  {} on {} type {} (slot {})", m.label_str(), path, fstype, i + 1);
                found = true;
            }
        }
        if !found {
            let _ = writeln!(w, "  (no filesystems mounted)");
        }
    }
}

fn lsblk_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  NAME       TYPE   SIZE");
    let _ = writeln!(w, "  spi-sd     disk   (SD card via SPI2)");
}

fn input_status(w: &mut dyn core::fmt::Write) {
    let input = unsafe { &*INPUT.0.get() };
    input.write_status(w);
}

fn ble_hid_list(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  BLE HID-over-GATT (HOGP) client");
    let _ = writeln!(w, "  Status: ready (ESP32-C6 BLE available)");
    let _ = writeln!(w, "  Max devices: 4");
    let _ = writeln!(w, "  Use 'input scan' to discover BLE HID peripherals");
}

fn dmesg_info(w: &mut dyn core::fmt::Write) {
    let klog = unsafe { &*KLOG.0.get() };
    klog.dump(w);
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
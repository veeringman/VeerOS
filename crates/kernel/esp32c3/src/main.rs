#![no_std]
#![no_main]

mod trap;

use core::cell::UnsafeCell;
use core::fmt::Write;

// ---------------------------------------------------------------------------
// ESP-IDF app descriptor — required by the 2nd-stage bootloader to validate
// the application image.  The struct is 256 bytes and must live in the
// `.flash.appdesc` section so that `espflash` places it at the start of
// the first flash segment.
// ---------------------------------------------------------------------------

#[repr(C)]
struct EspAppDesc {
    magic_word: u32,
    secure_version: u32,
    reserv1: [u32; 2],
    version: [u8; 32],
    project_name: [u8; 32],
    time: [u8; 16],
    date: [u8; 16],
    idf_ver: [u8; 32],
    app_elf_sha256: [u8; 32],
    min_efuse_blk_rev_full: u16,
    max_efuse_blk_rev_full: u16,
    mmu_page_size: u8,
    reserv3: [u8; 3],
    reserv2: [u32; 18],
}

unsafe impl Sync for EspAppDesc {}

const fn str_to_arr<const N: usize>(s: &str) -> [u8; N] {
    let mut arr = [0u8; N];
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && i < N - 1 {
        arr[i] = bytes[i];
        i += 1;
    }
    arr
}

#[unsafe(link_section = ".veeros.appdesc")]
#[used]
static ESP_APP_DESC: EspAppDesc = EspAppDesc {
    magic_word: 0xABCD5432,
    secure_version: 0,
    reserv1: [0; 2],
    version: str_to_arr::<32>("0.1.0"),
    project_name: str_to_arr::<32>("VeerOS"),
    time: str_to_arr::<16>("00:00:00"),
    date: str_to_arr::<16>("Jan  1 2026"),
    idf_ver: str_to_arr::<32>("v5.5-veeros"),
    app_elf_sha256: [0; 32],
    min_efuse_blk_rev_full: 0,
    max_efuse_blk_rev_full: 0xFFFF,
    mmu_page_size: 16, // log2(64KB)
    reserv3: [0; 3],
    reserv2: [0; 18],
};

#[cfg(feature = "wifi")]
use arch::NetworkDevice;
use arch::{Console, InterruptController, SavedContext, Serial, TickTimer};
use microkernel::alloc::Heap;
use microkernel::channel::Channels;
use microkernel::driver::{DriverCaps, DriverRegistry, MemRegion};
use microkernel::futex::FutexTable;
use microkernel::ipc::Ipc;
use microkernel::task::Scheduler;
#[cfg(feature = "shell")]
use microkernel::task::TaskState;
use microkernel::Kernel;
#[cfg(feature = "shell")]
// Net imports (WiFi TCP/IP stack).
#[cfg(feature = "wifi")]
use net::{NetStack, NetStorage, TcpSerial};
use shell::{Shell, ShellEnv};
#[cfg(feature = "wifi")]
use smoltcp::iface::SocketSet;
#[cfg(feature = "wifi")]
use smoltcp::wire::{IpCidr, Ipv4Address};
use soc_esp32::{interrupt_controller, system_timer, systimer::SysTimer, usb_serial, Esp32Riscv};

// Custom panic handler that prints the panic message via USB serial.
#[panic_handler]
fn panic_handler(info: &core::panic::PanicInfo) -> ! {
    const USB_BASE: usize = 0x6000_F000;
    const EP1_REG: usize = USB_BASE + 0x00;
    const EP1_CONF: usize = USB_BASE + 0x04;

    fn usb_putc(byte: u8) {
        unsafe {
            core::ptr::write_volatile(EP1_REG as *mut u32, byte as u32);
        }
    }
    fn usb_flush() {
        unsafe {
            core::ptr::write_volatile(EP1_CONF as *mut u32, 1);
            for _ in 0..200_000u32 {
                let conf = core::ptr::read_volatile(EP1_CONF as *const u32);
                if conf & 2 != 0 {
                    break;
                }
                core::hint::spin_loop();
            }
        }
    }
    fn usb_puts(s: &[u8]) {
        for (i, &b) in s.iter().enumerate() {
            usb_putc(b);
            if (i + 1) % 60 == 0 {
                usb_flush();
            }
        }
        usb_flush();
    }

    usb_puts(b"\r\n*** PANIC ***\r\n");
    // Try to print location if available
    if let Some(loc) = info.location() {
        let file = loc.file().as_bytes();
        usb_puts(b"at ");
        usb_puts(file);
        usb_puts(b":");
        // Print line number
        let mut line = loc.line();
        let mut buf = [0u8; 10];
        let mut i = 0;
        if line == 0 {
            usb_putc(b'0');
        } else {
            while line > 0 {
                buf[i] = b'0' + (line % 10) as u8;
                line /= 10;
                i += 1;
            }
            while i > 0 {
                i -= 1;
                usb_putc(buf[i]);
            }
        }
        usb_flush();
    }
    usb_puts(b"\r\n");

    loop {
        core::hint::spin_loop();
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Kernel tick period in microseconds (1 ms).
const TICK_PERIOD_US: u32 = 1_000;

/// SYSTIMER comparator 0 interrupt source on ESP32-C3 (source 37).
const SYSTIMER_IRQ_SOURCE: u16 = 37;
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
// Kernel heap (16 KiB — ESP32-C3 has 400 KiB DRAM)
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

pub(crate) struct Fat32Cell(pub UnsafeCell<[Fat32; microkernel::fat32::MAX_FAT32]>);
unsafe impl Sync for Fat32Cell {}
pub(crate) static FAT32: Fat32Cell = Fat32Cell(UnsafeCell::new([
    Fat32::new(),
    Fat32::new(),
    Fat32::new(),
    Fat32::new(),
]));

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
// Audit log
// ---------------------------------------------------------------------------

use microkernel::audit::AuditLog;

pub(crate) struct AuditCell(pub UnsafeCell<AuditLog>);
unsafe impl Sync for AuditCell {}
pub(crate) static AUDIT: AuditCell = AuditCell(UnsafeCell::new(AuditLog::new()));

// ---------------------------------------------------------------------------
// AI-Native Execution subsystems
// ---------------------------------------------------------------------------

use microkernel::agent::AgentTable;
use microkernel::fabric::ExecutionFabric;
use microkernel::intent::IntentEngine;
use microkernel::intent_sched::IntentScheduler;
use microkernel::memory_engine::MemoryEngine;

pub(crate) struct AgentCell(pub UnsafeCell<AgentTable>);
unsafe impl Sync for AgentCell {}
pub(crate) static AGENTS: AgentCell = AgentCell(UnsafeCell::new(AgentTable::new()));

pub(crate) struct IntentCell(pub UnsafeCell<IntentEngine>);
unsafe impl Sync for IntentCell {}
pub(crate) static INTENTS: IntentCell = IntentCell(UnsafeCell::new(IntentEngine::new()));

pub(crate) struct MemoryEngineCell(pub UnsafeCell<MemoryEngine>);
unsafe impl Sync for MemoryEngineCell {}
pub(crate) static MEMORY_ENGINE: MemoryEngineCell =
    MemoryEngineCell(UnsafeCell::new(MemoryEngine::new()));

pub(crate) struct FabricCell(pub UnsafeCell<ExecutionFabric>);
unsafe impl Sync for FabricCell {}
pub(crate) static FABRIC: FabricCell = FabricCell(UnsafeCell::new(ExecutionFabric::new()));

pub(crate) struct IntentSchedCell(pub UnsafeCell<IntentScheduler>);
unsafe impl Sync for IntentSchedCell {}
pub(crate) static INTENT_SCHED: IntentSchedCell =
    IntentSchedCell(UnsafeCell::new(IntentScheduler::new()));

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
// Kernel log ring buffer
// ---------------------------------------------------------------------------

use microkernel::klog::KernelLog;

pub(crate) struct KlogCell(pub UnsafeCell<KernelLog>);
unsafe impl Sync for KlogCell {}
pub(crate) static KLOG: KlogCell = KlogCell(UnsafeCell::new(KernelLog::new()));

// ---------------------------------------------------------------------------
// WiFi net task statics (TCP/IP stack over WiFi)
// ---------------------------------------------------------------------------

#[cfg(feature = "wifi")]
/// TCP port for VeerOS remote shell over WiFi.
const REMOTE_SHELL_PORT: u16 = 2323;

#[cfg(feature = "wifi")]
/// FNV-1a hash of the remote shell password.
const REMOTE_PASSWORD_HASH: u32 = net::auth::fnv1a(b"veeros");

#[cfg(feature = "wifi")]
/// Static IP for the WiFi interface (configure for your network).
const WIFI_IP: [u8; 4] = [192, 168, 29, 101];
#[cfg(feature = "wifi")]
const WIFI_GATEWAY: [u8; 4] = [192, 168, 29, 1];

#[cfg(feature = "wifi")]
#[repr(align(16))]
struct NetTaskStack([u8; 8192]);
#[cfg(feature = "wifi")]
static mut NET_TASK_STACK: NetTaskStack = NetTaskStack([0u8; 8192]);

#[cfg(feature = "wifi")]
static mut SOCKET_STORAGE: [smoltcp::iface::SocketStorage<'static>; 4] =
    [smoltcp::iface::SocketStorage::EMPTY; 4];
#[cfg(feature = "wifi")]
static mut NET_STORAGE: NetStorage = NetStorage::new();

/// Thin wrapper that implements `NetworkDevice` by proxying to the
/// `Esp32Wifi` driver inside the global `WIFI` static.
#[cfg(feature = "wifi")]
struct WifiNetProxy;

#[cfg(feature = "wifi")]
impl arch::NetworkDevice for WifiNetProxy {
    fn mtu(&self) -> usize {
        1514
    }
    fn has_rx(&self) -> bool {
        unsafe { (*WIFI.0.get()).driver().has_rx() }
    }
    fn recv(&self, buf: &mut [u8]) -> usize {
        unsafe { (*WIFI.0.get()).driver().recv(buf) }
    }
    fn send(&self, buf: &[u8]) {
        unsafe { (*WIFI.0.get()).driver().send(buf) }
    }
    fn mac_address(&self) -> [u8; 6] {
        unsafe { (*WIFI.0.get()).driver().mac_address() }
    }
}

#[cfg(feature = "wifi")]
struct NetCell(UnsafeCell<Option<NetStack<WifiNetProxy>>>);
#[cfg(feature = "wifi")]
unsafe impl Sync for NetCell {}
#[cfg(feature = "wifi")]
static NET: NetCell = NetCell(UnsafeCell::new(None));

#[cfg(feature = "wifi")]
struct SocketSetCell(UnsafeCell<Option<SocketSet<'static>>>);
#[cfg(feature = "wifi")]
unsafe impl Sync for SocketSetCell {}
#[cfg(feature = "wifi")]
static NET_SOCKETS: SocketSetCell = SocketSetCell(UnsafeCell::new(None));

/// RISC-V initial mstatus: MPIE=1 so mret enables interrupts, MPP=M-mode.
const INITIAL_MSTATUS: usize = (1 << 7) | (3 << 11);

/// RISC-V mstatus for U-mode tasks: MPIE=1, MPP=U-mode (0).
/// When `mret` executes, MPP is copied to privilege and MPIE→MIE,
/// so the task runs in U-mode with interrupts enabled.
const UMODE_MSTATUS: usize = (1 << 7) | (0 << 11);

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
struct IdleStack([u8; 2048]);
static mut IDLE_STACK: IdleStack = IdleStack([0u8; 2048]);

// ---------------------------------------------------------------------------
// Shell task
// ---------------------------------------------------------------------------

/// Stack for the shell task.
#[cfg(feature = "shell")]
#[repr(align(16))]
struct ShellStack([u8; 4096]);
#[cfg(feature = "shell")]
static mut SHELL_STACK: ShellStack = ShellStack([0u8; 4096]);

#[cfg(feature = "shell")]
fn shell_task() -> ! {
    let serial = usb_serial();
    let mut con = Console::new(serial);
    let env = ShellEnv {
        version: VERSION,
        platform: "ESP32-C3 (RISC-V)",
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
        zigbee_cmd: None,
        sensor_cmd: None,
        sensor_cmd: None,
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
        caps_cmd: None,
        auditlog_cmd: None,
        ifconfig_cmd: None,
        ping_cmd: None,
        netstat_cmd: None,
        ssh_cmd: None,
        #[cfg(feature = "multi-user")]
        login: Some(do_login),
        #[cfg(not(feature = "multi-user"))]
        login: None,
        #[cfg(feature = "multi-user")]
        logout: Some(do_logout),
        #[cfg(not(feature = "multi-user"))]
        logout: None,
        #[cfg(feature = "multi-user")]
        change_password: Some(do_change_password),
        #[cfg(not(feature = "multi-user"))]
        change_password: None,
        #[cfg(feature = "multi-user")]
        add_user: Some(do_add_user),
        #[cfg(not(feature = "multi-user"))]
        add_user: None,
        #[cfg(feature = "multi-user")]
        remove_user: Some(do_remove_user),
        #[cfg(not(feature = "multi-user"))]
        remove_user: None,
        pre_authenticated: false,
        // AI-native
        get_agent_list: None,
        agent_cmd: None,
        get_intent_list: None,
        intent_cmd: None,
        memory_cmd: None,
        get_fabric_status: None,
        peers_cmd: None,
        mesh_cmd: None,
        zkp_cmd: None,
        hostname_cmd: None,
        df_cmd: None,
    };
    let mut sh = Shell::new(env);
    loop {
        sh.run(&mut con);
    }
}

// ---------------------------------------------------------------------------
// Wi-Fi network task — TCP/IP stack over WiFi (runs in M-mode)
// ---------------------------------------------------------------------------

/// Poll the smoltcp network stack and the WiFi driver's RX path.
#[cfg(feature = "wifi")]
fn net_poll() -> bool {
    // Drive WiFi RX so frames arrive in the ring buffer.
    unsafe {
        (*WIFI.0.get()).driver_mut().poll_rx();
    }
    unsafe {
        if let (Some(stack), Some(sockets)) = (&mut *NET.0.get(), &mut *NET_SOCKETS.0.get()) {
            let ticks = (*SCHEDULER.0.get()).ticks;
            stack.poll(sockets, ticks);
        }
    }
    true
}

#[cfg(feature = "wifi")]
fn net_poll_unlock() {
    // Single-threaded — no lock to release.
}

/// The Wi-Fi network listener task.
///
/// 1. Waits for the WiFi driver to be connected.
/// 2. Initialises smoltcp with a static IP (DHCP TODO).
/// 3. Listens on REMOTE_SHELL_PORT (2323).
/// 4. On connection → runs a shell session over TCP.
/// 5. When the client disconnects, loops back to listen.
#[cfg(feature = "wifi")]
fn net_task() -> ! {
    // Let the shell task print its banner first.
    for _ in 0..2_000_000u32 {
        core::hint::spin_loop();
    }
    let serial = usb_serial();
    let mut con = Console::new(serial);

    let _ = writeln!(con, "[net] waiting for WiFi association...");

    // Wait until WiFi is connected.
    loop {
        let connected = unsafe { (*WIFI.0.get()).driver().is_connected() };
        if connected {
            break;
        }
        for _ in 0..10_000 {
            core::hint::spin_loop();
        }
    }

    let mac = unsafe { (*WIFI.0.get()).driver().mac_address() };
    let _ = writeln!(
        con,
        "[net] WiFi associated  MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    // Initialise smoltcp.
    let ip = IpCidr::new(
        Ipv4Address::new(WIFI_IP[0], WIFI_IP[1], WIFI_IP[2], WIFI_IP[3]).into(),
        24,
    );
    let gw = Ipv4Address::new(
        WIFI_GATEWAY[0],
        WIFI_GATEWAY[1],
        WIFI_GATEWAY[2],
        WIFI_GATEWAY[3],
    );

    unsafe {
        let sockets_ref: &'static mut [smoltcp::iface::SocketStorage<'static>] =
            &mut *core::ptr::addr_of_mut!(SOCKET_STORAGE);
        let mut socket_set = SocketSet::new(sockets_ref);
        let storage = &mut *core::ptr::addr_of_mut!(NET_STORAGE);

        let stack = NetStack::new(WifiNetProxy, ip, gw, &mut socket_set, storage);

        *NET_SOCKETS.0.get() = Some(socket_set);
        *NET.0.get() = Some(stack);
    }

    let _ = writeln!(
        con,
        "[net] IP {}.{}.{}.{} — listening on port {}",
        WIFI_IP[0], WIFI_IP[1], WIFI_IP[2], WIFI_IP[3], REMOTE_SHELL_PORT
    );

    // Main accept loop.
    loop {
        // Start listening.
        unsafe {
            if let (Some(stack), Some(sockets)) = (&mut *NET.0.get(), &mut *NET_SOCKETS.0.get()) {
                stack.listen(sockets, REMOTE_SHELL_PORT);
            }
        }

        // Poll until a client connects.
        loop {
            net_poll();
            let connected = unsafe {
                if let (Some(stack), Some(sockets)) = (&*NET.0.get(), &*NET_SOCKETS.0.get()) {
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

        // Authenticate, then run the shell over TCP.
        unsafe {
            let handle = (*NET.0.get()).as_ref().unwrap().tcp_handle();
            let socket_set_ptr =
                (*NET_SOCKETS.0.get()).as_mut().unwrap() as *mut SocketSet<'static>;
            let tcp_serial = TcpSerial::new(handle, socket_set_ptr, net_poll, net_poll_unlock);
            let mut tcp_con = Console::new(tcp_serial);

            #[cfg(feature = "shell")]
            {
                if net::auth::login_prompt(&mut tcp_con, REMOTE_PASSWORD_HASH) {
                    let _ = writeln!(con, "[net] authentication succeeded — starting shell");
                    let env = ShellEnv {
                        version: VERSION,
                        platform: "ESP32-C3 (RISC-V)",
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
                        zigbee_cmd: None,
                        sensor_cmd: None,
                        sensor_cmd: None,
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
                        caps_cmd: None,
                        auditlog_cmd: None,
                        ifconfig_cmd: None,
                        ping_cmd: None,
                        netstat_cmd: None,
                        ssh_cmd: None,
                        login: None,
                        logout: None,
                        change_password: None,
                        add_user: None,
                        remove_user: None,
                        pre_authenticated: true,
                        // AI-native
                        get_agent_list: None,
                        agent_cmd: None,
                        get_intent_list: None,
                        intent_cmd: None,
                        memory_cmd: None,
                        get_fabric_status: None,
                        peers_cmd: None,
                        mesh_cmd: None,
                        zkp_cmd: None,
                        hostname_cmd: None,
                        df_cmd: None,
                    };
                    let mut sh = Shell::new(env);
                    sh.run(&mut tcp_con);
                } else {
                    let _ = writeln!(con, "[net] authentication failed");
                }
            }
            #[cfg(not(feature = "shell"))]
            {
                let _ = writeln!(tcp_con, "VeerOS net: no shell available");
            }
        }

        let _ = writeln!(con, "[net] client disconnected — re-listening");

        // Abort the socket so it can be re-used immediately.
        unsafe {
            if let Some(sockets) = &mut *NET_SOCKETS.0.get() {
                let handle = (*NET.0.get()).as_ref().unwrap().tcp_handle();
                let socket = sockets.get_mut::<smoltcp::socket::tcp::Socket>(handle);
                socket.abort();
            }
        }

        net_poll();
    }
}

// ---------------------------------------------------------------------------
// Userspace driver tasks — run in U-mode with PMP-granted MMIO regions
// ---------------------------------------------------------------------------

/// Stacks for driver tasks (live in .bss).
#[repr(align(16))]
struct DrvStack4K([u8; 4096]);
#[repr(align(16))]
struct DrvStack2K([u8; 2048]);

#[cfg(feature = "wifi")]
static mut WIFI_DRV_STACK: DrvStack4K = DrvStack4K([0u8; 4096]);
#[cfg(feature = "ble")]
static mut BLE_DRV_STACK: DrvStack2K = DrvStack2K([0u8; 2048]);

// ── Syscall wrappers (U-mode ecall) ────────────────────────────────────

/// Write a 32-bit value to an MMIO register via kernel syscall.
#[cfg(target_arch = "riscv32")]
#[inline(always)]
fn drv_mmio_write32(addr: usize, val: u32) {
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") addr,
            in("a1") val as usize,
            in("a7") 0xC1usize, // SYS_DRV_MMIO_WRITE32
            options(nostack),
        );
    }
}

/// Read a 32-bit value from an MMIO register via kernel syscall.
#[cfg(target_arch = "riscv32")]
#[inline(always)]
fn drv_mmio_read32(addr: usize) -> u32 {
    let ret: usize;
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") 0xC0usize, // SYS_DRV_MMIO_READ32
            inlateout("a0") addr => ret,
            options(nostack),
        );
    }
    ret as u32
}

/// Block until the assigned IRQ fires.
#[cfg(target_arch = "riscv32")]
#[inline(always)]
fn drv_irq_wait(irq_line: usize) {
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") irq_line,
            in("a7") 0xC2usize, // SYS_DRV_IRQ_WAIT
            options(nostack),
        );
    }
}

/// Log a message to the kernel console from a driver task.
#[cfg(target_arch = "riscv32")]
fn drv_log(msg: &[u8]) {
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") msg.as_ptr() as usize,
            in("a1") msg.len(),
            in("a7") 0xC5usize, // SYS_DRV_LOG
            options(nostack),
        );
    }
}

/// Yield CPU to the scheduler (SYS_YIELD = 1).
#[cfg(target_arch = "riscv32")]
#[inline(always)]
fn drv_yield() {
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a7") 0x00usize, // SYS_YIELD
            options(nostack),
        );
    }
}

/// Sleep for N ticks (SYS_SLEEP = 2).
#[cfg(target_arch = "riscv32")]
#[inline(always)]
fn drv_sleep(ticks: usize) {
    unsafe {
        core::arch::asm!(
            "ecall",
            in("a0") ticks,
            in("a7") 0x31usize, // SYS_SLEEP
            options(nostack),
        );
    }
}

// ── Wi-Fi driver task ──────────────────────────────────────────────────

#[cfg(all(feature = "wifi", target_arch = "riscv32"))]
fn wifi_driver_task() -> ! {
    drv_log(b"[wifi-drv] starting\n");

    // Read WiFi credentials from /etc/net/wifi config file.
    let mgr = unsafe { &mut *WIFI.0.get() };
    {
        let mut buf = [0u8; 256];
        let n = vfs_read_file("/etc/net/wifi", &mut buf);
        if n > 0 {
            let cfg = userlib::config::Config::parse(&buf[..n]);
            let ssid = cfg.get_bytes("ssid").unwrap_or(b"MARS");
            let pass = cfg.get_bytes("password").unwrap_or(b"");
            mgr.set_credentials(ssid, pass);
            drv_log(b"[wifi-drv] credentials from /etc/net/wifi\n");
        } else {
            drv_log(b"[wifi-drv] /etc/net/wifi not found, no credentials\n");
        }
    }

    // Retry until connected; WiFi start/ppTask bring-up is asynchronous.
    loop {
        if mgr.state() != soc_esp32::wifi::WifiState::Connected {
            drv_log(b"[wifi-drv] connecting...\n");
            match mgr.connect() {
                Ok(()) => {
                    mgr.ip = [192, 168, 29, 101];
                    drv_log(b"[wifi-drv] connected\n");
                }
                Err(_) => {
                    drv_log(b"[wifi-drv] connect failed, retrying\n");
                }
            }
        }
        drv_sleep(100);
    }
}

// ── BLE driver task ────────────────────────────────────────────────────

#[cfg(all(feature = "ble", target_arch = "riscv32"))]
fn ble_driver_task() -> ! {
    use soc_esp32::modem;

    drv_log(b"[ble-drv] starting\n");

    // Enable BLE clocks.
    let clk_reg = modem::MODEM_LPCON_BASE + modem::MODEM_CLK_EN;
    let cur = drv_mmio_read32(clk_reg);
    drv_mmio_write32(clk_reg, cur | modem::CLK_BLE_EN | modem::CLK_FE_EN);
    drv_log(b"[ble-drv] BLE clocks enabled\n");

    // Release BLE baseband from reset.
    let rst_reg = modem::MODEM_LPCON_BASE + modem::MODEM_RST_CTRL;
    let rst = drv_mmio_read32(rst_reg);
    drv_mmio_write32(rst_reg, rst | modem::RST_BLE_BB);
    drv_yield();
    drv_mmio_write32(rst_reg, rst & !modem::RST_BLE_BB);
    drv_log(b"[ble-drv] BLE baseband reset complete\n");

    // Verify MMIO access.
    let bb_val = drv_mmio_read32(modem::BLE_BB_BASE);
    if bb_val != usize::MAX as u32 {
        drv_log(b"[ble-drv] BLE BB accessible\n");
    } else {
        drv_log(b"[ble-drv] BLE BB read failed\n");
    }

    // Clear and enable BLE interrupts.
    drv_mmio_write32(modem::BLE_BB_BASE + modem::BLE_INT_CLR, 0xFFFF_FFFF);
    drv_mmio_write32(
        modem::BLE_BB_BASE + modem::BLE_INT_ENA,
        modem::BLE_INT_SCAN_DONE
            | modem::BLE_INT_ADV_DONE
            | modem::BLE_INT_RX_DONE
            | modem::BLE_INT_CONN_DONE
            | modem::BLE_INT_TX_DONE,
    );

    // Enable BLE controller.
    drv_mmio_write32(modem::BLE_BB_BASE + modem::BLE_CTRL, modem::BLE_CTRL_ENABLE);

    drv_log(b"[ble-drv] ready, entering event loop\n");

    // Main driver event loop: service BLE controller events.
    loop {
        let status = drv_mmio_read32(modem::BLE_BB_BASE + modem::BLE_INT_STATUS);

        if status & modem::BLE_INT_RX_DONE != 0 {
            drv_mmio_write32(
                modem::BLE_BB_BASE + modem::BLE_INT_CLR,
                modem::BLE_INT_RX_DONE,
            );
            let _rx = drv_mmio_read32(modem::BLE_BB_BASE + modem::BLE_RX_DESCR);
        }

        if status & modem::BLE_INT_ADV_DONE != 0 {
            drv_mmio_write32(
                modem::BLE_BB_BASE + modem::BLE_INT_CLR,
                modem::BLE_INT_ADV_DONE,
            );
        }

        if status & modem::BLE_INT_SCAN_DONE != 0 {
            drv_mmio_write32(
                modem::BLE_BB_BASE + modem::BLE_INT_CLR,
                modem::BLE_INT_SCAN_DONE,
            );
        }

        if status & modem::BLE_INT_TX_DONE != 0 {
            drv_mmio_write32(
                modem::BLE_BB_BASE + modem::BLE_INT_CLR,
                modem::BLE_INT_TX_DONE,
            );
        }

        if status & modem::BLE_INT_CONN_DONE != 0 {
            drv_mmio_write32(
                modem::BLE_BB_BASE + modem::BLE_INT_CLR,
                modem::BLE_INT_CONN_DONE,
            );
        }

        // Yield to scheduler.
        drv_sleep(10);
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

// Minimal early trap handler — prints mcause/mepc via USB serial FIFO
#[cfg(target_arch = "riscv32")]
core::arch::global_asm!(
    r#"
.section .text._early_trap_handler
.global  _early_trap_handler
.balign  4

_early_trap_handler:
    # Read mcause and mepc into a0/a1 so the Rust handler can use them.
    csrr    a0, mcause
    csrr    a1, mepc
    csrr    a2, mtval
    j       _early_trap_rust
"#
);

/// Called by the early trap handler assembly stub.
#[cfg(target_arch = "riscv32")]
#[unsafe(no_mangle)]
pub extern "C" fn _early_trap_rust(mcause: usize, mepc: usize, mtval: usize) -> ! {
    // Write directly to USB Serial JTAG FIFO (bypass all abstractions).
    const USB_BASE: usize = 0x6000_F000;
    const EP1_REG: usize = USB_BASE + 0x00;
    const EP1_CONF: usize = USB_BASE + 0x04;

    fn usb_putc(byte: u8) {
        unsafe {
            core::ptr::write_volatile(EP1_REG as *mut u32, byte as u32);
        }
    }
    fn usb_flush() {
        unsafe {
            core::ptr::write_volatile(EP1_CONF as *mut u32, 1); // WR_DONE
                                                                // Brief spin wait for host to consume
            for _ in 0..200_000u32 {
                let conf = core::ptr::read_volatile(EP1_CONF as *const u32);
                if conf & 2 != 0 {
                    break;
                } // SERIAL_IN_EP_DATA_FREE
                core::hint::spin_loop();
            }
        }
    }
    fn usb_puts(s: &[u8]) {
        for &b in s {
            usb_putc(b);
        }
        usb_flush();
    }
    fn usb_hex(val: usize) {
        let digits = b"0123456789ABCDEF";
        usb_putc(b'0');
        usb_putc(b'x');
        for i in (0..8).rev() {
            let nibble = (val >> (i * 4)) & 0xF;
            usb_putc(digits[nibble]);
        }
    }

    usb_puts(b"\r\n*** TRAP ***\r\n");
    usb_puts(b"mcause=");
    usb_hex(mcause);
    usb_puts(b"\r\nmepc=");
    usb_hex(mepc);
    usb_puts(b"\r\nmtval=");
    usb_hex(mtval);
    usb_puts(b"\r\n");

    loop {
        core::hint::spin_loop();
    }
}

// ---------------------------------------------------------------------------
// Entry (Rust)
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn _rust_start() -> ! {
    // ── disable watchdogs (ROM bootloader enables them) ──────────
    soc_esp32::wdt::disable_watchdogs();

    // ── install EARLY trap handler to diagnose crashes ──────────
    {
        extern "C" {
            fn _early_trap_handler();
        }
        unsafe {
            let addr = _early_trap_handler as *const () as usize;
            core::arch::asm!("csrw mtvec, {0}", in(reg) addr, options(nomem, nostack));
        }
    }

    // ── wait for USB Serial/JTAG enumeration ─────────────────
    // After a reset the USB host needs ~200 ms to re-enumerate
    // the CDC-ACM device.  Wait for first SOF from host.
    let serial = usb_serial();
    serial.wait_for_usb_ready();

    // ── early console ────────────────────────────────────────
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
    let _ = writeln!(con, "[boot] heap initialised ({} KiB)", HEAP_SIZE / 1024,);

    // ── seed system CSPRNG from hardware RNG ─────────────────
    {
        const RNG_DATA_REG: *const u32 = 0x6002_60B0 as *const u32;
        let mut seed = [0u8; 32];
        for chunk in seed.chunks_exact_mut(4) {
            let val = unsafe { core::ptr::read_volatile(RNG_DATA_REG) };
            chunk.copy_from_slice(&val.to_le_bytes());
        }
        unsafe {
            microkernel::seed_system_rng(seed);
        }
        let _ = writeln!(con, "[boot] system CSPRNG seeded from hardware RNG");
    }

    // ── register drivers ─────────────────────────────────────
    unsafe {
        let reg = &mut *DRIVERS.0.get();

        let uart = reg
            .register(
                "uart0",
                DriverCaps {
                    mmio_regions: 1,
                    uses_interrupts: false,
                    uses_dma: false,
                    uses_network: false,
                },
            )
            .unwrap();
        reg.grant_mmio(uart, MemRegion::new(0x6000_0000, 0x100))
            .ok();

        let intc_id = reg
            .register(
                "intc",
                DriverCaps {
                    mmio_regions: 1,
                    uses_interrupts: true,
                    uses_dma: false,
                    uses_network: false,
                },
            )
            .unwrap();
        reg.grant_mmio(intc_id, MemRegion::new(0x600C_2000, 0x200))
            .ok();

        let st = reg
            .register(
                "systimer",
                DriverCaps {
                    mmio_regions: 1,
                    uses_interrupts: true,
                    uses_dma: false,
                    uses_network: false,
                },
            )
            .unwrap();
        reg.grant_mmio(st, MemRegion::new(0x6002_3000, 0x80)).ok();
        reg.grant_irq(st, SYSTIMER_CPU_INT as u16);
    }
    let _ = writeln!(con, "[boot] driver registry: 3 drivers registered");

    // ── register BLE driver ──────────────────────────────
    #[cfg(feature = "ble")]
    {
        unsafe {
            let reg = &mut *DRIVERS.0.get();
            let _ = reg.register(
                "ble",
                DriverCaps {
                    mmio_regions: 1,
                    uses_interrupts: true,
                    uses_dma: false,
                    uses_network: false,
                },
            );
        }
        let _ = writeln!(con, "[boot] BLE 5.0 driver registered");
    }

    // ── install trap vector (RISC-V only) ────────────────────
    #[cfg(target_arch = "riscv32")]
    {
        extern "C" {
            fn _veer_vector_table();
        }
        unsafe {
            // ESP32-C3 uses direct mode (mode=0) — no PLIC vectored dispatch.
            let addr = _veer_vector_table as *const () as usize & !0x3;
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
    unsafe {
        *TIMER.0.get() = timer;
    }
    let _ = writeln!(con, "[boot] systimer tick @ {} us", TICK_PERIOD_US);

    // ── VFS initialisation ───────────────────────────────────
    unsafe {
        let inodes = &mut *INODES.0.get();
        let ramfs = &mut *RAMFS.0.get();
        inodes.init_root();
        let dev_id = inodes
            .resolve(microkernel::vfs::ROOT_INODE, "/dev")
            .unwrap_or(microkernel::vfs::NO_INODE);
        if dev_id != microkernel::vfs::NO_INODE {
            inodes.create_device_in(dev_id, "null", 0, 0);
            inodes.create_device_in(dev_id, "zero", 0, 1);
            inodes.create_device_in(dev_id, "console", 0, 2);
            inodes.create_device_in(dev_id, "random", 0, 3);
            inodes.create_device_in(dev_id, "keyboard", 1, 0);
            inodes.create_device_in(dev_id, "mouse", 1, 1);
        }
        let etc_id = inodes
            .resolve(microkernel::vfs::ROOT_INODE, "/etc")
            .unwrap_or(microkernel::vfs::NO_INODE);
        if etc_id != microkernel::vfs::NO_INODE {
            ramfs.create_with_content(inodes, etc_id, "motd", b"Welcome to VeerOS!\n");
            ramfs.create_with_content(inodes, etc_id, "hostname", b"veeros-esp32c3\n");

            // /etc/net/wifi — WiFi credentials from build-time or defaults.
            if let Some(net_id) = inodes.mkdir_in(etc_id, "net") {
                const WIFI_SSID: &str = match option_env!("VEEROS_WIFI_SSID") {
                    Some(s) => s,
                    None => "MARS",
                };
                const WIFI_PASS: &str = match option_env!("VEEROS_WIFI_PASS") {
                    Some(s) => s,
                    None => "Naitla123",
                };
                // Build config content into a stack buffer.
                let mut buf = [0u8; 256];
                let mut pos = 0usize;
                for &b in b"# VeerOS WiFi configuration\nssid=" {
                    if pos < buf.len() {
                        buf[pos] = b;
                        pos += 1;
                    }
                }
                for &b in WIFI_SSID.as_bytes() {
                    if pos < buf.len() {
                        buf[pos] = b;
                        pos += 1;
                    }
                }
                for &b in b"\npassword=" {
                    if pos < buf.len() {
                        buf[pos] = b;
                        pos += 1;
                    }
                }
                for &b in WIFI_PASS.as_bytes() {
                    if pos < buf.len() {
                        buf[pos] = b;
                        pos += 1;
                    }
                }
                if pos < buf.len() {
                    buf[pos] = b'\n';
                    pos += 1;
                }
                ramfs.create_with_content(inodes, net_id, "wifi", &buf[..pos]);
            }
        }
    }
    let _ = writeln!(
        con,
        "[boot] VFS initialised (ramfs {} KiB)",
        microkernel::ramfs::RAMFS_POOL_SIZE / 1024
    );

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
        let sb = (&raw const IDLE_STACK) as usize;
        let st = sb + core::mem::size_of::<IdleStack>();
        if let Some(idx) = sched.create_task("idle", idle_task as *const () as usize, st, sb, 0, 0)
        {
            sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
        }

        // Shell task (priority 1).
        #[cfg(feature = "shell")]
        {
            let sb = (&raw const SHELL_STACK) as usize;
            let st = sb + core::mem::size_of::<ShellStack>();
            if let Some(idx) =
                sched.create_task("shell", shell_task as *const () as usize, st, sb, 1, 0)
            {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
            }
        }

        // ── Userspace driver tasks (U-mode, PMP-isolated) ─────────

        #[cfg(all(feature = "wifi", target_arch = "riscv32"))]
        {
            use soc_esp32::modem;
            let sb = (&raw const WIFI_DRV_STACK) as usize;
            let st = sb + core::mem::size_of::<DrvStack4K>();
            if let Some(idx) = sched.create_task(
                "wifi-drv",
                wifi_driver_task as *const () as usize,
                st,
                sb,
                2,
                0,
            ) {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
                // Grant MMIO: modem clock/reset registers.
                sched.grant_region(
                    idx,
                    arch::TaskMemRegion {
                        base: modem::MODEM_LPCON_BASE,
                        size: 0x1000,
                        perms: arch::MemPerms::RW,
                    },
                );
                // Grant MMIO: WiFi MAC + baseband.
                sched.grant_region(
                    idx,
                    arch::TaskMemRegion {
                        base: modem::WIFI_MMIO_BASE,
                        size: modem::WIFI_MMIO_SIZE,
                        perms: arch::MemPerms::RW,
                    },
                );
            }
        }

        #[cfg(all(feature = "ble", target_arch = "riscv32"))]
        {
            use soc_esp32::modem;
            let sb = (&raw const BLE_DRV_STACK) as usize;
            let st = sb + core::mem::size_of::<DrvStack2K>();
            if let Some(idx) = sched.create_task(
                "ble-drv",
                ble_driver_task as *const () as usize,
                st,
                sb,
                2,
                0,
            ) {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
                // Grant MMIO: modem clock/reset.
                sched.grant_region(
                    idx,
                    arch::TaskMemRegion {
                        base: modem::MODEM_LPCON_BASE,
                        size: 0x1000,
                        perms: arch::MemPerms::RW,
                    },
                );
                // Grant MMIO: BLE baseband.
                sched.grant_region(
                    idx,
                    arch::TaskMemRegion {
                        base: modem::BLE_MMIO_BASE,
                        size: modem::BLE_MMIO_SIZE,
                        perms: arch::MemPerms::RW,
                    },
                );
            }
        }

        // Network listener task (priority 1) — TCP/IP over WiFi.
        #[cfg(feature = "wifi")]
        {
            let sb = (&raw const NET_TASK_STACK) as usize;
            let st = sb + core::mem::size_of::<NetTaskStack>();
            if let Some(idx) =
                sched.create_task("net", net_task as *const () as usize, st, sb, 1, 0)
            {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
            }
        }
    }
    // Set init process thread count to match all boot tasks.
    unsafe {
        use microkernel::task::TaskState;
        let sched = &*SCHEDULER.0.get();
        let procs = &mut *PROCESSES.0.get();
        let count = sched
            .tasks
            .iter()
            .filter(|t| t.state != TaskState::Free)
            .count();
        procs.processes[0].thread_count = count;
    }
    let _ = writeln!(con, "[boot] idle task registered");
    #[cfg(feature = "shell")]
    let _ = writeln!(con, "[boot] shell task registered");
    #[cfg(all(feature = "wifi", target_arch = "riscv32"))]
    let _ = writeln!(con, "[boot] wifi-drv task registered (M-mode)");
    #[cfg(all(feature = "ble", target_arch = "riscv32"))]
    let _ = writeln!(con, "[boot] ble-drv task registered (M-mode)");
    #[cfg(feature = "wifi")]
    let _ = writeln!(
        con,
        "[boot] net listener task registered (port {})",
        REMOTE_SHELL_PORT
    );

    // ── WiFi auto-connect (credentials loaded from /etc/net/wifi by driver) ──
    #[cfg(feature = "wifi")]
    {
        let _ = writeln!(
            con,
            "[boot] wifi: credentials in /etc/net/wifi (driver will load)"
        );
        // Direct FIFO debug marker (boot configured, connect deferred to wifi-drv task).
        unsafe {
            core::ptr::write_volatile(0x6000_f000 as *mut u32, b'Z' as u32);
            core::ptr::write_volatile(0x6000_f004 as *mut u32, 1);
        }
        let _ = writeln!(con, "[boot] wifi: connect deferred (driver will handle)");
    }

    // ── start the first task (never returns) ─────────────────
    let _ = writeln!(con, "[boot] starting scheduler — preemptive mode");

    unsafe {
        let sched = &mut *SCHEDULER.0.get();
        let ctx_ptr = sched.start().expect("no runnable task");
        let _ = writeln!(
            con,
            "[boot] first task: idx={} name={}",
            sched.current, sched.tasks[sched.current].name,
        );
        let _ = writeln!(con, "");

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
// Multi-user callbacks
// ---------------------------------------------------------------------------

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_login(username: &str, password: &[u8]) -> u32 {
    unsafe {
        let users = &mut *USERS.0.get();
        match users.login(username, password) {
            Ok(token) => {
                if let Some((uid, gid)) = users.session_info(token) {
                    (*PROCESSES.0.get()).processes[0].uid = uid;
                    (*PROCESSES.0.get()).processes[0].gid = gid;
                }
                token
            }
            Err(_) => 0,
        }
    }
}

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_logout() -> bool {
    unsafe {
        let users = &mut *USERS.0.get();
        let uid = (*PROCESSES.0.get()).processes[0].uid;
        let mut found = false;
        for s in users.sessions.iter_mut() {
            if s.active && s.uid == uid {
                *s = microkernel::user::Session::empty();
                found = true;
                break;
            }
        }
        (*PROCESSES.0.get()).processes[0].uid = microkernel::user::ROOT_UID;
        (*PROCESSES.0.get()).processes[0].gid = microkernel::user::ROOT_GID;
        found
    }
}

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_change_password(uid: u16, new_password: &[u8]) -> bool {
    unsafe { (*USERS.0.get()).change_password(uid, new_password) }
}

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_add_user(name: &'static str, gid: u16, password: &[u8]) -> Option<u16> {
    unsafe { (*USERS.0.get()).add_user(name, gid, password) }
}

#[cfg(all(feature = "shell", feature = "multi-user"))]
fn do_remove_user(uid: u16) -> bool {
    unsafe { (*USERS.0.get()).remove_user(uid) }
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
        let dir_id = if path == "." {
            Some(cwd)
        } else {
            inodes.resolve(cwd, path)
        };
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
            _ => {
                let _ = writeln!(w, "ls: '{}': no such directory", path);
            }
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
        if id == NO_INODE {
            return 0;
        }
        if inodes.inodes[id as usize].kind != InodeKind::File {
            return 0;
        }
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
                if parent == NO_INODE || name.is_empty() {
                    return false;
                }
                id = match inodes.create_file_in(parent, name) {
                    Some(i) => i,
                    None => return false,
                };
            } else {
                id = match inodes.create_file_in(cwd, path) {
                    Some(i) => i,
                    None => return false,
                };
            }
        }
        if inodes.inodes[id as usize].kind != InodeKind::File {
            return false;
        }
        let offset = if append {
            inodes.inodes[id as usize].size
        } else {
            0
        };
        if !append {
            ramfs.truncate(inodes, id, 0);
        }
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
            if parent == NO_INODE || name.is_empty() {
                return false;
            }
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
        let kind = match inode.kind {
            InodeKind::File => "file",
            InodeKind::Directory => "directory",
            InodeKind::Device => "device",
            _ => "unknown",
        };
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
        if id == NO_INODE {
            return false;
        }
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
        if id == NO_INODE {
            return false;
        }
        if let Some(slash) = new.rfind('/') {
            let parent_path = if slash == 0 { "/" } else { &new[..slash] };
            let name = &new[slash + 1..];
            let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
            if parent == NO_INODE || name.is_empty() {
                return false;
            }
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
        if id == NO_INODE {
            return false;
        }
        if inodes.inodes[id as usize].kind != InodeKind::Directory {
            return false;
        }
        procs.processes[0].cwd = id;
        true
    }
}

#[cfg(feature = "shell")]
fn vfs_tree(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, NO_INODE, ROOT_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = (*PROCESSES.0.get()).processes[0].cwd;
        let start = if path == "/" {
            ROOT_INODE
        } else {
            inodes.resolve(cwd, path).unwrap_or(NO_INODE)
        };
        if start == NO_INODE {
            let _ = writeln!(w, "tree: '{}': no such directory", path);
            return;
        }
        let root = &inodes.inodes[start as usize];
        if root.kind != InodeKind::Directory {
            let _ = writeln!(w, "tree: '{}': not a directory", path);
            return;
        }
        let _ = writeln!(
            w,
            "{}",
            if path == "/" || path == "." {
                "/"
            } else {
                path
            }
        );
        let mut stack: [(u16, u8); 64] = [(NO_INODE, 0); 64];
        let mut sp = 0usize;
        let mut kids: [u16; 64] = [NO_INODE; 64];
        let mut nk = 0usize;
        let mut ch = root.children_head;
        while ch != NO_INODE && nk < 64 {
            kids[nk] = ch;
            nk += 1;
            ch = inodes.inodes[ch as usize].next_sibling;
        }
        let mut i = nk;
        while i > 0 {
            i -= 1;
            if sp < 64 {
                stack[sp] = (kids[i], 1);
                sp += 1;
            }
        }
        while sp > 0 {
            sp -= 1;
            let (id, depth) = stack[sp];
            let node = &inodes.inodes[id as usize];
            for _ in 0..depth {
                w.write_str("  ").ok();
            }
            let kind_ch = match node.kind {
                InodeKind::Directory => '/',
                InodeKind::Device => '*',
                _ => ' ',
            };
            let _ = writeln!(w, "{}{}", node.name_str(), kind_ch);
            if node.kind == InodeKind::Directory {
                let mut ck: [u16; 64] = [NO_INODE; 64];
                let mut cn = 0usize;
                let mut c = node.children_head;
                while c != NO_INODE && cn < 64 {
                    ck[cn] = c;
                    cn += 1;
                    c = inodes.inodes[c as usize].next_sibling;
                }
                let mut j = cn;
                while j > 0 {
                    j -= 1;
                    if sp < 64 {
                        stack[sp] = (ck[j], depth + 1);
                        sp += 1;
                    }
                }
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
        if inodes.resolve(cwd, path).unwrap_or(NO_INODE) != NO_INODE {
            return true;
        }
        if let Some(slash) = path.rfind('/') {
            let parent_path = if slash == 0 { "/" } else { &path[..slash] };
            let name = &path[slash + 1..];
            let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
            if parent == NO_INODE || name.is_empty() {
                return false;
            }
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
                let _ = writeln!(
                    w,
                    "  {} on {} type {} (slot {})",
                    m.label_str(),
                    path,
                    fstype,
                    i + 1
                );
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
    let _ = writeln!(w, "  Status: ready (ESP32-C3 BLE available)");
    let _ = writeln!(w, "  Max devices: 4");
    let _ = writeln!(w, "  Use 'input scan' to discover BLE HID peripherals");
}

fn dmesg_info(w: &mut dyn core::fmt::Write) {
    let klog = unsafe { &*KLOG.0.get() };
    klog.dump(w);
}

/// Shell callback for `wifi <sub> <args>`.
#[cfg(feature = "wifi")]
fn wifi_command(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    let mgr = unsafe { &mut *WIFI.0.get() };

    match sub {
        "set" => {
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

// ---------------------------------------------------------------------------
// Console I/O callbacks (used by the syscall dispatcher)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub(crate) fn console_write_byte(b: u8) {
    let serial = usb_serial();
    let mut con = Console::new(serial);
    let _ = con.write_str(unsafe { core::str::from_utf8_unchecked(core::slice::from_ref(&b)) });
}

#[allow(dead_code)]
pub(crate) fn console_read_byte() -> u8 {
    let serial = usb_serial();
    if serial.has_data() {
        serial.read_byte()
    } else {
        0xFF
    }
}

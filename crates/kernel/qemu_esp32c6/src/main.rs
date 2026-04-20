//! VeerOS kernel for the QEMU-emulated ESP32-C6 virtual IoT node.
//!
//! Runs on the QEMU `virt` RISC-V 32-bit machine but presents an
//! ESP32-C6 personality:
//!   - 16 KiB heap (matching real ESP32-C6 constraints)
//!   - Simulated WiFi / BLE / 802.15.4 radio managers
//!   - Virtual sensor device files for EdgeFabric integration
//!   - Capability-based security + audit trail
//!
//! Hardware access goes through `soc-qemu-virt` (NS16550 UART + CLINT timer).

#![no_std]
#![no_main]

mod trap;

use core::cell::UnsafeCell;
use core::fmt::Write;

use arch::{Console, SavedContext, TickTimer};
#[cfg(feature = "net")]
use arch::NetworkDevice;
use soc_qemu_virt::{default_serial, system_timer, clint::Clint, QemuVirt};
use microkernel::Kernel;
use microkernel::alloc::Heap;
use microkernel::channel::Channels;
use microkernel::driver::{DriverCaps, DriverRegistry, MemRegion};
use microkernel::futex::FutexTable;
use microkernel::ipc::Ipc;
use microkernel::task::Scheduler;
#[cfg(feature = "shell")]
use microkernel::task::TaskState;
#[cfg(feature = "net")]
use net::{NetStack, NetStorage, TcpSerial};
#[cfg(feature = "shell")]
use shell::{Shell, ShellEnv};
#[cfg(feature = "net")]
use smoltcp::iface::{SocketHandle, SocketSet};
#[cfg(feature = "net")]
use smoltcp::socket::dhcpv4;


// Custom panic handler that prints to serial
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let serial = default_serial();
    let mut con = Console::new(serial);
    let _ = writeln!(con, "\r\n!!! PANIC: {}", info);
    loop {
        #[cfg(target_arch = "riscv32")]
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
        #[cfg(not(target_arch = "riscv32"))]
        core::hint::spin_loop();
    }
}

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
// Kernel heap — 16 KiB to match real ESP32-C6 SRAM constraints
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

pub(crate) struct IpcCell(pub UnsafeCell<Ipc>);
unsafe impl Sync for IpcCell {}
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

use microkernel::process::{ProcessTable, ProcessCaps};

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
pub(crate) static FAT32: Fat32Cell = Fat32Cell(UnsafeCell::new(
    [Fat32::new(), Fat32::new(), Fat32::new(), Fat32::new()]
));

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
// Driver registry
// ---------------------------------------------------------------------------

struct RegistryCell(UnsafeCell<DriverRegistry>);
unsafe impl Sync for RegistryCell {}
static DRIVERS: RegistryCell = RegistryCell(UnsafeCell::new(DriverRegistry::new()));

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

use microkernel::audit::{AuditLog, AuditEvent};

pub(crate) struct AuditCell(pub UnsafeCell<AuditLog>);
unsafe impl Sync for AuditCell {}
pub(crate) static AUDIT: AuditCell = AuditCell(UnsafeCell::new(AuditLog::new()));

// ---------------------------------------------------------------------------
// AI-Native Execution subsystems
// ---------------------------------------------------------------------------

use microkernel::agent::AgentTable;
use microkernel::intent::IntentEngine;
use microkernel::memory_engine::MemoryEngine;
use microkernel::fabric::ExecutionFabric;
use microkernel::intent_sched::IntentScheduler;

pub(crate) struct AgentCell(pub UnsafeCell<AgentTable>);
unsafe impl Sync for AgentCell {}
pub(crate) static AGENTS: AgentCell = AgentCell(UnsafeCell::new(AgentTable::new()));

pub(crate) struct IntentCell(pub UnsafeCell<IntentEngine>);
unsafe impl Sync for IntentCell {}
pub(crate) static INTENTS: IntentCell = IntentCell(UnsafeCell::new(IntentEngine::new()));

pub(crate) struct MemoryEngineCell(pub UnsafeCell<MemoryEngine>);
unsafe impl Sync for MemoryEngineCell {}
pub(crate) static MEMORY_ENGINE: MemoryEngineCell = MemoryEngineCell(UnsafeCell::new(MemoryEngine::new()));

pub(crate) struct FabricCell(pub UnsafeCell<ExecutionFabric>);
unsafe impl Sync for FabricCell {}
pub(crate) static FABRIC: FabricCell = FabricCell(UnsafeCell::new(ExecutionFabric::new()));

pub(crate) struct IntentSchedCell(pub UnsafeCell<IntentScheduler>);
unsafe impl Sync for IntentSchedCell {}
pub(crate) static INTENT_SCHED: IntentSchedCell = IntentSchedCell(UnsafeCell::new(IntentScheduler::new()));

// ---------------------------------------------------------------------------
// Static timer handle (used by the trap dispatcher)
// ---------------------------------------------------------------------------

pub(crate) struct TimerCell(pub UnsafeCell<Clint>);
unsafe impl Sync for TimerCell {}

pub(crate) static TIMER: TimerCell = TimerCell(UnsafeCell::new(Clint::new()));

// ---------------------------------------------------------------------------
// Kernel log ring buffer
// ---------------------------------------------------------------------------

use microkernel::klog::KernelLog;

pub(crate) struct KlogCell(pub UnsafeCell<KernelLog>);
unsafe impl Sync for KlogCell {}
pub(crate) static KLOG: KlogCell = KlogCell(UnsafeCell::new(KernelLog::new()));

// ═══════════════════════════════════════════════════════════════════════════
// Simulated Radio Managers
// ═══════════════════════════════════════════════════════════════════════════
//
// These stubs emulate the WiFi / BLE / 802.15.4 interfaces that a real
// ESP32-C6 provides.  No actual RF hardware — the state lives entirely
// in software so the shell "wifi", "bt", "zigbee" commands work.

#[cfg(feature = "wifi")]
mod sim_wifi {
    use core::fmt::Write;

    pub struct SimWifi {
        ssid: [u8; 32],
        ssid_len: usize,
        pass: [u8; 64],
        pass_len: usize,
        connected: bool,
    }

    impl SimWifi {
        pub const fn new() -> Self {
            Self { ssid: [0; 32], ssid_len: 0, pass: [0; 64], pass_len: 0, connected: false }
        }

        pub fn set_credentials(&mut self, ssid: &[u8], pass: &[u8]) {
            let sl = ssid.len().min(32);
            self.ssid[..sl].copy_from_slice(&ssid[..sl]);
            self.ssid_len = sl;
            let pl = pass.len().min(64);
            self.pass[..pl].copy_from_slice(&pass[..pl]);
            self.pass_len = pl;
        }

        pub fn connect(&mut self) -> Result<(), &'static str> {
            if self.ssid_len == 0 { return Err("no SSID configured"); }
            self.connected = true;
            Ok(())
        }

        pub fn disconnect(&mut self) { self.connected = false; }

        pub fn scan(&self) -> Result<usize, &'static str> {
            // Simulated: always finds 1 virtual network
            Ok(1)
        }

        pub fn write_scan_results(&self, w: &mut dyn Write) {
            let _ = writeln!(w, "  SSID              RSSI  CH  AUTH");
            let _ = writeln!(w, "  veeros-sim-ap     -42   6   WPA2");
        }

        pub fn write_status(&self, w: &mut dyn Write) {
            let ssid = core::str::from_utf8(&self.ssid[..self.ssid_len]).unwrap_or("?");
            let state = if self.connected { "connected" } else { "disconnected" };
            let _ = writeln!(w, "  WiFi: {} (SSID: {})", state, if self.ssid_len > 0 { ssid } else { "<none>" });
            let _ = writeln!(w, "  Mode: simulated (QEMU virtual)");
        }
    }
}

#[cfg(feature = "wifi")]
use core::cell::UnsafeCell as WifiUC;
#[cfg(feature = "wifi")]
struct WifiCell(WifiUC<sim_wifi::SimWifi>);
#[cfg(feature = "wifi")]
unsafe impl Sync for WifiCell {}
#[cfg(feature = "wifi")]
static WIFI: WifiCell = WifiCell(WifiUC::new(sim_wifi::SimWifi::new()));

#[cfg(feature = "ble")]
mod sim_ble {
    use core::fmt::Write;

    pub struct SimBle {
        advertising: bool,
        adv_name: [u8; 32],
        adv_len: usize,
    }

    impl SimBle {
        pub const fn new() -> Self {
            Self { advertising: false, adv_name: [0; 32], adv_len: 0 }
        }

        pub fn scan(&self) -> Result<usize, &'static str> { Ok(0) }

        pub fn write_scan_results(&self, w: &mut dyn Write) {
            let _ = writeln!(w, "  (no BLE devices in range — simulated)");
        }

        pub fn advertise(&mut self, name: &[u8]) -> Result<(), &'static str> {
            let l = name.len().min(32);
            self.adv_name[..l].copy_from_slice(&name[..l]);
            self.adv_len = l;
            self.advertising = true;
            Ok(())
        }

        pub fn stop(&mut self) { self.advertising = false; }

        pub fn write_status(&self, w: &mut dyn Write) {
            let state = if self.advertising { "advertising" } else { "idle" };
            let _ = writeln!(w, "  BLE: {} (simulated)", state);
            if self.advertising {
                let name = core::str::from_utf8(&self.adv_name[..self.adv_len]).unwrap_or("?");
                let _ = writeln!(w, "  Name: {}", name);
            }
        }
    }
}

#[cfg(feature = "ble")]
struct BleCell(UnsafeCell<sim_ble::SimBle>);
#[cfg(feature = "ble")]
unsafe impl Sync for BleCell {}
#[cfg(feature = "ble")]
static BLE: BleCell = BleCell(UnsafeCell::new(sim_ble::SimBle::new()));

#[cfg(feature = "ieee802154")]
mod sim_802154 {
    use core::fmt::Write;

    pub struct Sim802154 {
        initialised: bool,
        channel: u8,
        pan_id: u16,
    }

    impl Sim802154 {
        pub const fn new() -> Self {
            Self { initialised: false, channel: 11, pan_id: 0xFFFF }
        }

        pub fn init(&mut self) -> Result<(), &'static str> {
            self.initialised = true;
            Ok(())
        }

        pub fn channel(&self) -> u8 { self.channel }
        pub fn pan_id(&self) -> u16 { self.pan_id }

        pub fn set_channel(&mut self, ch: u8) -> Result<(), &'static str> {
            if !(11..=26).contains(&ch) { return Err("channel must be 11-26"); }
            self.channel = ch;
            Ok(())
        }

        pub fn set_pan_id(&mut self, id: u16) { self.pan_id = id; }

        pub fn scan(&self) -> Result<usize, &'static str> {
            if !self.initialised { return Err("radio not initialised"); }
            Ok(0)
        }

        pub fn write_scan_results(&self, w: &mut dyn Write) {
            let _ = writeln!(w, "  (no 802.15.4 networks found — simulated)");
        }

        pub fn send(&self, _data: &[u8]) -> Result<(), &'static str> {
            if !self.initialised { return Err("radio not initialised"); }
            Ok(())
        }

        pub fn write_status(&self, w: &mut dyn Write) {
            let state = if self.initialised { "ready" } else { "uninitialised" };
            let _ = writeln!(w, "  802.15.4: {} (simulated)", state);
            let _ = writeln!(w, "  Channel: {}  PAN ID: 0x{:04X}", self.channel, self.pan_id);
        }
    }
}

#[cfg(feature = "ieee802154")]
struct Radio802154Cell(UnsafeCell<sim_802154::Sim802154>);
#[cfg(feature = "ieee802154")]
unsafe impl Sync for Radio802154Cell {}
#[cfg(feature = "ieee802154")]
static RADIO_802154: Radio802154Cell = Radio802154Cell(UnsafeCell::new(sim_802154::Sim802154::new()));

// ═══════════════════════════════════════════════════════════════════════════
// Virtual Sensor Subsystem
// ═══════════════════════════════════════════════════════════════════════════
//
// Provides device files under /dev/sensor/ that EdgeFabric can inject
// sensor data into from the host.  Each sensor is a simple value that
// can be read via `cat /dev/sensor/<name>`.

/// Maximum number of virtual sensor slots.
const MAX_SENSORS: usize = 8;

struct VirtualSensor {
    name: [u8; 16],
    name_len: usize,
    unit: [u8; 8],
    unit_len: usize,
    /// Current value in fixed-point (value / 100.0).
    value_centi: i32,
    active: bool,
}

impl VirtualSensor {
    const fn empty() -> Self {
        Self {
            name: [0; 16], name_len: 0,
            unit: [0; 8], unit_len: 0,
            value_centi: 0,
            active: false,
        }
    }
}

struct SensorArray {
    sensors: [VirtualSensor; MAX_SENSORS],
}

impl SensorArray {
    const fn new() -> Self {
        Self { sensors: [
            VirtualSensor::empty(), VirtualSensor::empty(),
            VirtualSensor::empty(), VirtualSensor::empty(),
            VirtualSensor::empty(), VirtualSensor::empty(),
            VirtualSensor::empty(), VirtualSensor::empty(),
        ]}
    }

    fn add(&mut self, name: &str, unit: &str, initial: i32) -> bool {
        for s in self.sensors.iter_mut() {
            if !s.active {
                let nl = name.len().min(16);
                s.name[..nl].copy_from_slice(&name.as_bytes()[..nl]);
                s.name_len = nl;
                let ul = unit.len().min(8);
                s.unit[..ul].copy_from_slice(&unit.as_bytes()[..ul]);
                s.unit_len = ul;
                s.value_centi = initial;
                s.active = true;
                return true;
            }
        }
        false
    }

    fn find(&self, name: &str) -> Option<usize> {
        for (i, s) in self.sensors.iter().enumerate() {
            if s.active && &s.name[..s.name_len] == name.as_bytes() {
                return Some(i);
            }
        }
        None
    }

    fn write_list(&self, w: &mut dyn core::fmt::Write) {
        let _ = writeln!(w, "  NAME           VALUE       UNIT");
        let _ = writeln!(w, "  ─────────────  ──────────  ────");
        let mut count = 0;
        for s in &self.sensors {
            if s.active {
                let name = core::str::from_utf8(&s.name[..s.name_len]).unwrap_or("?");
                let unit = core::str::from_utf8(&s.unit[..s.unit_len]).unwrap_or("?");
                let whole = s.value_centi / 100;
                let frac = (s.value_centi % 100).unsigned_abs();
                let _ = writeln!(w, "  {:<13}  {:>6}.{:02}    {}", name, whole, frac, unit);
                count += 1;
            }
        }
        if count == 0 {
            let _ = writeln!(w, "  (no sensors attached)");
        }
    }

    /// Format a single sensor value into a buffer (for VFS read).
    fn read_value(&self, idx: usize, buf: &mut [u8]) -> usize {
        if idx >= MAX_SENSORS || !self.sensors[idx].active { return 0; }
        let s = &self.sensors[idx];
        let whole = s.value_centi / 100;
        let frac = (s.value_centi % 100).unsigned_abs();
        let unit = core::str::from_utf8(&s.unit[..s.unit_len]).unwrap_or("?");
        // Format: "22.50 C\n"
        let mut tmp = [0u8; 32];
        let mut pos = 0;
        // Simple integer formatting
        let neg = whole < 0;
        let abs_whole = if neg { (-whole) as u32 } else { whole as u32 };
        if neg && pos < tmp.len() { tmp[pos] = b'-'; pos += 1; }
        let mut digits = [0u8; 10];
        let mut nd = 0;
        let mut v = abs_whole;
        if v == 0 { digits[0] = b'0'; nd = 1; }
        else {
            while v > 0 && nd < 10 { digits[nd] = b'0' + (v % 10) as u8; nd += 1; v /= 10; }
        }
        let mut i = nd;
        while i > 0 { i -= 1; if pos < tmp.len() { tmp[pos] = digits[i]; pos += 1; } }
        if pos < tmp.len() { tmp[pos] = b'.'; pos += 1; }
        if pos < tmp.len() { tmp[pos] = b'0' + (frac / 10) as u8; pos += 1; }
        if pos < tmp.len() { tmp[pos] = b'0' + (frac % 10) as u8; pos += 1; }
        if pos < tmp.len() { tmp[pos] = b' '; pos += 1; }
        for &b in unit.as_bytes() { if pos < tmp.len() { tmp[pos] = b; pos += 1; } }
        if pos < tmp.len() { tmp[pos] = b'\n'; pos += 1; }
        let len = pos.min(buf.len());
        buf[..len].copy_from_slice(&tmp[..len]);
        len
    }
}

struct SensorCell(UnsafeCell<SensorArray>);
unsafe impl Sync for SensorCell {}
static SENSORS: SensorCell = SensorCell(UnsafeCell::new(SensorArray::new()));

// ---------------------------------------------------------------------------
// Idle task
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

#[repr(align(16))]
struct IdleStack([u8; 512]);
static IDLE_STACK: IdleStack = IdleStack([0u8; 512]);

// ---------------------------------------------------------------------------
// Shell task
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
#[repr(align(16))]
struct ShellStack([u8; 4096]);
#[cfg(feature = "shell")]
static SHELL_STACK: ShellStack = ShellStack([0u8; 4096]);

// ---------------------------------------------------------------------------
// Network task stack
// ---------------------------------------------------------------------------

#[cfg(feature = "net")]
/// TCP port for VeerOS remote shell.
const REMOTE_SHELL_PORT: u16 = 2323;

#[cfg(feature = "net")]
const REMOTE_PASSWORD_HASH: u32 = net::auth::fnv1a(b"veeros");

#[cfg(feature = "net")]
static mut DHCP_HANDLE: Option<SocketHandle> = None;
#[cfg(feature = "net")]
static mut DHCP_CONFIGURED: bool = false;

#[cfg(feature = "net")]
#[repr(align(16))]
struct NetStack0([u8; 16384]);
#[cfg(feature = "net")]
static NET_TASK_STACK: NetStack0 = NetStack0([0u8; 16384]);

#[cfg(feature = "net")]
static mut SOCKET_STORAGE: [smoltcp::iface::SocketStorage<'static>; 5] =
    [smoltcp::iface::SocketStorage::EMPTY; 5];
#[cfg(feature = "net")]
static mut NET_STORAGE: NetStorage = NetStorage::new();

#[cfg(feature = "net")]
struct NetCell(UnsafeCell<Option<NetStack<soc_qemu_virt::virtio_net::VirtioNet>>>);
#[cfg(feature = "net")]
unsafe impl Sync for NetCell {}
#[cfg(feature = "net")]
static NET: NetCell = NetCell(UnsafeCell::new(None));

#[cfg(feature = "net")]
struct SocketSetCell(UnsafeCell<Option<SocketSet<'static>>>);
#[cfg(feature = "net")]
unsafe impl Sync for SocketSetCell {}
#[cfg(feature = "net")]
static NET_SOCKETS: SocketSetCell = SocketSetCell(UnsafeCell::new(None));

#[cfg(feature = "net")]
fn net_poll() -> bool {
    unsafe {
        if let (Some(stack), Some(sockets)) =
            (&mut *NET.0.get(), &mut *NET_SOCKETS.0.get())
        {
            let ticks = (*SCHEDULER.0.get()).ticks;
            stack.poll(sockets, ticks);

            // Process DHCP events.
            if let Some(handle) = DHCP_HANDLE {
                let event = sockets.get_mut::<dhcpv4::Socket>(handle).poll();
                match event {
                    Some(dhcpv4::Event::Configured(config)) => {
                        let address = config.address;
                        let router = config.router;
                        let addr = address.address().0;
                        let prefix = address.prefix_len();
                        stack.apply_ip_config(address, router);
                        DHCP_CONFIGURED = true;
                        let serial = default_serial();
                        let mut c = Console::new(serial);
                        let _ = writeln!(c,
                            "[net] DHCP: acquired {}.{}.{}.{}/{}",
                            addr[0], addr[1], addr[2], addr[3], prefix,
                        );
                        if let Some(gw) = router {
                            let g = gw.0;
                            let _ = writeln!(c,
                                "[net] DHCP: gateway {}.{}.{}.{}",
                                g[0], g[1], g[2], g[3],
                            );
                        }
                    }
                    Some(dhcpv4::Event::Deconfigured) => {
                        DHCP_CONFIGURED = false;
                        let serial = default_serial();
                        let mut c = Console::new(serial);
                        let _ = writeln!(c, "[net] DHCP: lease expired, waiting for renewal");
                    }
                    None => {}
                }
            }
        }
    }
    true
}

#[cfg(feature = "net")]
fn net_poll_unlock() {}

// ---------------------------------------------------------------------------
// Shell task entry
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
fn shell_task() -> ! {
    let serial = default_serial();
    let mut con = Console::new(serial);
    let env = build_shell_env(false);
    let mut sh = Shell::new(env);
    sh.run(&mut con);

    // User typed `exit` — power off QEMU.
    let _ = writeln!(con, "VeerOS halted \u{2014} powering off.");
    qemu_poweroff();
}

/// Build a ShellEnv with all callbacks wired up.
#[cfg(feature = "shell")]
fn build_shell_env(pre_auth: bool) -> ShellEnv {
    ShellEnv {
        version: VERSION,
        platform: if pre_auth { "QEMU ESP32-C6 (RISC-V 32) [remote]" } else { "QEMU ESP32-C6 (RISC-V 32)" },
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
        df_cmd: None,
        input_status: Some(input_status),
        usb_list: None,
        ble_hid_list: None,
        gpio_cmd: None,
        i2c_cmd: None,
        spi_cmd: None,
        hw_info: Some(hw_info),
        get_temp_millic: Some(get_temp_millic),
        dmesg: Some(dmesg_info),
        reboot: None,
        shutdown: Some(do_shutdown),
        caps_cmd: Some(caps_command),
        auditlog_cmd: Some(auditlog_command),
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
        pre_authenticated: pre_auth,
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
    }
}

// ---------------------------------------------------------------------------
// Network / remote-shell task
// ---------------------------------------------------------------------------

#[cfg(feature = "net")]
fn net_task() -> ! {
    let serial = default_serial();
    let mut con = Console::new(serial);

    let _ = writeln!(con, "[net] probing for VIRTIO-NET device...");

    let nic = match soc_qemu_virt::virtio_net::VirtioNet::probe() {
        Some(n) => n,
        None => {
            let _ = writeln!(con, "[net] no VIRTIO-NET device found \u{2014} task halted");
            loop {
                #[cfg(target_arch = "riscv32")]
                unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
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

    let _ = writeln!(con, "[net] configuring DHCP stack...");

    unsafe {
        let sockets_ref: &'static mut [smoltcp::iface::SocketStorage<'static>] =
            &mut *core::ptr::addr_of_mut!(SOCKET_STORAGE);
        let mut socket_set = SocketSet::new(sockets_ref);
        let storage = &mut *core::ptr::addr_of_mut!(NET_STORAGE);

        let stack = NetStack::new_dhcp(nic, &mut socket_set, storage);

        let dhcp_socket = dhcpv4::Socket::new();
        let handle = socket_set.add(dhcp_socket);
        DHCP_HANDLE = Some(handle);

        *NET_SOCKETS.0.get() = Some(socket_set);
        *NET.0.get() = Some(stack);
    }

    let _ = writeln!(con, "[net] DHCP: client started, awaiting lease...");

    // Wait for DHCP lease before listening for connections.
    loop {
        net_poll();
        unsafe {
            if DHCP_CONFIGURED { break; }
        }
        core::hint::spin_loop();
    }

    let _ = writeln!(con, "[net] DHCP complete \u{2014} listening on port {}", REMOTE_SHELL_PORT);

    loop {
        unsafe {
            if let (Some(stack), Some(sockets)) =
                (&mut *NET.0.get(), &mut *NET_SOCKETS.0.get())
            {
                stack.listen(sockets, REMOTE_SHELL_PORT);
            }
        }

        loop {
            net_poll();
            let connected = unsafe {
                if let (Some(stack), Some(sockets)) =
                    (&*NET.0.get(), &*NET_SOCKETS.0.get())
                {
                    stack.is_connected(sockets)
                } else {
                    false
                }
            };
            if connected { break; }
            core::hint::spin_loop();
        }

        let _ = writeln!(con, "[net] client connected \u{2014} VSC handshake");

        unsafe {
            let handle = (*NET.0.get()).as_ref().unwrap().tcp_handle();
            let socket_set_ptr = (*NET_SOCKETS.0.get()).as_mut().unwrap() as *mut SocketSet<'static>;
            let tcp_serial = TcpSerial::new(handle, socket_set_ptr, net_poll, net_poll_unlock);

            // Generate a seed from timer + scheduler ticks for the handshake RNG.
            let mut seed = [0u8; 32];
            let ticks = (*SCHEDULER.0.get()).ticks;
            let cycle: u64;
            #[cfg(target_arch = "riscv32")]
            {
                let lo: u32;
                let hi: u32;
                core::arch::asm!("rdcycle {}", out(reg) lo);
                core::arch::asm!("rdcycleh {}", out(reg) hi);
                cycle = ((hi as u64) << 32) | (lo as u64);
            }
            #[cfg(not(target_arch = "riscv32"))]
            {
                cycle = 0x12345678_9abcdef0;
            }
            // Mix ticks + cycle into seed using simple hash-like spread.
            let tb = ticks.to_le_bytes();
            let cb = cycle.to_le_bytes();
            for i in 0..8 {
                seed[i] = tb[i];
                seed[i + 8] = cb[i];
                seed[i + 16] = tb[i] ^ cb[7 - i];
                seed[i + 24] = cb[i].wrapping_add(tb[7 - i]);
            }

            // Perform X25519 + ChaCha20-Poly1305 handshake.
            let channel = net::secure::server_handshake(&tcp_serial, seed);
            match channel {
                Some(mut ch) => {
                    let _ = writeln!(con, "[net] VSC handshake complete \u{2014} encrypted session");

                    // Read client mode byte.
                    let mode = net::secure::read_mode(&tcp_serial, &mut ch);
                    let _ = writeln!(con, "[net] mode: {:?}", mode);

                    match mode {
                        Some(net::secure::MODE_SHELL) | None => {
                            // Interactive shell mode.
                            let secure_serial = net::secure::SecureSerial::new(tcp_serial, ch);
                            let mut secure_con = Console::new(secure_serial);

                            #[cfg(feature = "shell")]
                            {
                                if net::auth::login_prompt(&mut secure_con, REMOTE_PASSWORD_HASH) {
                                    let _ = writeln!(con, "[net] authentication succeeded \u{2014} starting shell");
                                    let env = build_shell_env(true);
                                    let mut sh = Shell::new(env);
                                    sh.run(&mut secure_con);
                                } else {
                                    let _ = writeln!(con, "[net] authentication failed");
                                }
                            }
                            #[cfg(not(feature = "shell"))]
                            {
                                let _ = writeln!(secure_con, "VeerOS net: no shell available");
                            }
                        }
                        Some(net::secure::MODE_PUSH) => {
                            // File upload: authenticate via SecureSerial, then transfer.
                            let secure_serial = net::secure::SecureSerial::new(tcp_serial, ch);
                            let mut secure_con = Console::new(secure_serial);

                            if net::auth::login_prompt(&mut secure_con, REMOTE_PASSWORD_HASH) {
                                let _ = writeln!(con, "[net] push: authenticated \u{2014} receiving file");
                                // Unwrap the SecureSerial to get raw serial + channel back.
                                let (raw_serial, mut channel) = secure_con.into_inner().into_parts();
                                net::secure::handle_push(&raw_serial, &mut channel, vfs_write_file);
                                let _ = writeln!(con, "[net] push: transfer complete");
                            } else {
                                let _ = writeln!(con, "[net] push: authentication failed");
                            }
                        }
                        Some(net::secure::MODE_PULL) => {
                            // File download: authenticate via SecureSerial, then transfer.
                            let secure_serial = net::secure::SecureSerial::new(tcp_serial, ch);
                            let mut secure_con = Console::new(secure_serial);

                            if net::auth::login_prompt(&mut secure_con, REMOTE_PASSWORD_HASH) {
                                let _ = writeln!(con, "[net] pull: authenticated \u{2014} sending file");
                                let (raw_serial, mut channel) = secure_con.into_inner().into_parts();
                                net::secure::handle_pull(&raw_serial, &mut channel, vfs_read_file);
                                let _ = writeln!(con, "[net] pull: transfer complete");
                            } else {
                                let _ = writeln!(con, "[net] pull: authentication failed");
                            }
                        }
                        Some(_) => {
                            let _ = writeln!(con, "[net] unknown mode");
                        }
                    }
                }
                None => {
                    let _ = writeln!(con, "[net] VSC handshake failed");
                }
            }
        }

        let _ = writeln!(con, "[net] client disconnected \u{2014} re-listening");

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

// ═══════════════════════════════════════════════════════════════════════════
// Shell callbacks
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "shell")]
fn get_uptime_ticks() -> u64 {
    unsafe { (*SCHEDULER.0.get()).ticks }
}

#[cfg(feature = "shell")]
fn write_mem_info(w: &mut dyn core::fmt::Write) {
    unsafe { (*HEAP.0.get()).write_stats(w); }
}

#[cfg(feature = "shell")]
fn write_driver_list(w: &mut dyn core::fmt::Write) {
    unsafe { (*DRIVERS.0.get()).write_list(w); }
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
// WiFi / BLE / 802.15.4 shell commands (simulated)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "shell", feature = "wifi"))]
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
                Ok(()) => { let _ = writeln!(w, " connected!"); }
                Err(e) => { let _ = writeln!(w, " failed: {}", e); }
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
                Err(e) => { let _ = writeln!(w, " failed: {}", e); }
            }
        }
        "list" | "ls" => { mgr.write_scan_results(w); }
        "status" | "info" | "" => { mgr.write_status(w); }
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

#[cfg(all(feature = "shell", feature = "ble"))]
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
                Err(e) => { let _ = writeln!(w, " failed: {}", e); }
            }
        }
        "list" | "ls" => { mgr.write_scan_results(w); }
        "advertise" | "adv" => {
            if args.is_empty() {
                let _ = writeln!(w, "  usage: bt advertise <name>");
                return;
            }
            match mgr.advertise(args.as_bytes()) {
                Ok(()) => { let _ = writeln!(w, "  Advertising as '{}'", args); }
                Err(e) => { let _ = writeln!(w, "  Failed: {}", e); }
            }
        }
        "stop" => {
            mgr.stop();
            let _ = writeln!(w, "  Advertising stopped.");
        }
        "status" | "info" | "" => { mgr.write_status(w); }
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

#[cfg(all(feature = "shell", feature = "ieee802154"))]
fn zigbee_command(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    let mgr = unsafe { &mut *RADIO_802154.0.get() };

    match sub {
        "init" => {
            let _ = write!(w, "  Initialising 802.15.4 radio...");
            match mgr.init() {
                Ok(()) => { let _ = writeln!(w, " done"); }
                Err(e) => { let _ = writeln!(w, " failed: {}", e); }
            }
        }
        "channel" | "ch" => {
            if args.is_empty() {
                let _ = writeln!(w, "  Current channel: {}", mgr.channel());
                return;
            }
            match args.parse::<u8>() {
                Ok(ch) => match mgr.set_channel(ch) {
                    Ok(()) => { let _ = writeln!(w, "  Channel set to {}", ch); }
                    Err(e) => { let _ = writeln!(w, "  Error: {}", e); }
                },
                Err(_) => { let _ = writeln!(w, "  Invalid channel number (must be 11-26)"); }
            }
        }
        "panid" => {
            if args.is_empty() {
                let _ = writeln!(w, "  Current PAN ID: 0x{:04X}", mgr.pan_id());
                return;
            }
            let hex_str = args.strip_prefix("0x").or_else(|| args.strip_prefix("0X")).unwrap_or(args);
            match u16::from_str_radix(hex_str, 16) {
                Ok(pan_id) => {
                    mgr.set_pan_id(pan_id);
                    let _ = writeln!(w, "  PAN ID set to 0x{:04X}", pan_id);
                }
                Err(_) => { let _ = writeln!(w, "  Invalid PAN ID (use hex, e.g. 0x1234)"); }
            }
        }
        "scan" => {
            let _ = write!(w, "  Scanning 802.15.4 channels...");
            match mgr.scan() {
                Ok(n) => {
                    let _ = writeln!(w, " found {} network(s)", n);
                    mgr.write_scan_results(w);
                }
                Err(e) => { let _ = writeln!(w, " failed: {}", e); }
            }
        }
        "list" | "ls" => { mgr.write_scan_results(w); }
        "send" | "tx" => {
            if args.is_empty() {
                let _ = writeln!(w, "  usage: zigbee send <data>");
                return;
            }
            match mgr.send(args.as_bytes()) {
                Ok(()) => { let _ = writeln!(w, "  Frame sent ({} bytes)", args.len()); }
                Err(e) => { let _ = writeln!(w, "  TX failed: {}", e); }
            }
        }
        "status" | "info" | "" => { mgr.write_status(w); }
        _ => {
            let _ = writeln!(w, "  zigbee subcommands:");
            let _ = writeln!(w, "    zigbee init                Init 802.15.4 radio");
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
// Virtual sensor shell command
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
fn sensor_command(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    let sensors = unsafe { &mut *SENSORS.0.get() };

    match sub {
        "list" | "ls" | "" => { sensors.write_list(w); }
        "read" => {
            if args.is_empty() {
                let _ = writeln!(w, "  usage: sensor read <name>");
                return;
            }
            match sensors.find(args) {
                Some(idx) => {
                    let mut buf = [0u8; 32];
                    let len = sensors.read_value(idx, &mut buf);
                    let val = core::str::from_utf8(&buf[..len]).unwrap_or("?");
                    let _ = write!(w, "  {}", val);
                }
                None => { let _ = writeln!(w, "  sensor '{}' not found", args); }
            }
        }
        "set" => {
            // sensor set <name> <value_centi>
            let (name, val_str) = match args.find(' ') {
                Some(i) => (&args[..i], args[i + 1..].trim()),
                None => {
                    let _ = writeln!(w, "  usage: sensor set <name> <value*100>");
                    return;
                }
            };
            match sensors.find(name) {
                Some(idx) => {
                    let val = parse_i32(val_str);
                    sensors.sensors[idx].value_centi = val;
                    let _ = writeln!(w, "  sensor '{}' set to {}.{:02}", name, val / 100, (val % 100).unsigned_abs());
                }
                None => { let _ = writeln!(w, "  sensor '{}' not found", name); }
            }
        }
        "status" => {
            let active = sensors.sensors.iter().filter(|s| s.active).count();
            let _ = writeln!(w, "  Virtual sensor subsystem");
            let _ = writeln!(w, "  Active sensors: {} / {}", active, MAX_SENSORS);
            let _ = writeln!(w, "  Mode: simulated (QEMU virtual IoT)");
        }
        _ => {
            let _ = writeln!(w, "  sensor subcommands:");
            let _ = writeln!(w, "    sensor list                List all sensors");
            let _ = writeln!(w, "    sensor read <name>         Read sensor value");
            let _ = writeln!(w, "    sensor set <name> <value>  Inject value (value*100)");
            let _ = writeln!(w, "    sensor status              Subsystem status");
        }
    }
}

fn parse_i32(s: &str) -> i32 {
    let neg = s.starts_with('-');
    let s = if neg { &s[1..] } else { s };
    let mut n: i32 = 0;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            n = n.wrapping_mul(10).wrapping_add((b - b'0') as i32);
        } else {
            break;
        }
    }
    if neg { -n } else { n }
}

// ---------------------------------------------------------------------------
// Hardware info callbacks (ESP32-C6 personality)
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
fn hw_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  Board   : QEMU ESP32-C6 Virtual IoT Node");
    let _ = writeln!(w, "  SoC     : ESP32-C6 (emulated on RISC-V QEMU virt)");
    let _ = writeln!(w, "  CPU     : RV32IMC @ emulated");
    let _ = writeln!(w, "  SRAM    : 512 KiB (QEMU mapped)");
    let _ = writeln!(w, "  Heap    : {} KiB (small {} KiB + large {} KiB)", HEAP_SIZE / 1024, HEAP_SMALL_BYTES / 1024, (HEAP_SIZE - HEAP_SMALL_BYTES) / 1024);
    let _ = writeln!(w, "  Radios  : WiFi (sim), BLE (sim), 802.15.4 (sim)");
    let _ = writeln!(w, "  Sensors : virtual (EdgeFabric injectable)");
}

/// Simulated temperature — returns a constant 25.00 C.
#[cfg(feature = "shell")]
fn get_temp_millic() -> i32 {
    25_000
}

// ---------------------------------------------------------------------------
// Capability & audit shell callbacks
// ---------------------------------------------------------------------------

const CAP_NAMES: &[(u32, &str)] = &[
    (0, "task_basic"), (1, "mem"), (2, "time"), (3, "sync"),
    (4, "ipc"), (5, "channel"), (6, "poll"), (7, "console_io"),
    (8, "fs"), (9, "net"), (10, "spawn_thread"), (11, "spawn_process"),
    (12, "user_admin"), (13, "driver"), (14, "mount"), (15, "hw"),
    (16, "crypto"), (17, "cap_admin"),
];

#[cfg(feature = "shell")]
fn caps_command(sub: &str, args: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::process::ProcessState;

    let pt = unsafe { &mut *PROCESSES.0.get() };

    match sub {
        "" => {
            let _ = writeln!(w, "  PID  NAME             CAPS");
            let _ = writeln!(w, "  \u{2500}\u{2500}\u{2500}  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}");
            for (pid, p) in pt.processes.iter().enumerate() {
                if p.state == ProcessState::Free { continue; }
                let bits = p.caps.bits();
                let mut buf = [0u8; 128];
                let mut pos = 0;
                for &(bit, name) in CAP_NAMES {
                    if bits & (1 << bit) != 0 {
                        if pos > 0 && pos + 1 < buf.len() { buf[pos] = b','; pos += 1; }
                        let nb = name.as_bytes();
                        let end = (pos + nb.len()).min(buf.len());
                        buf[pos..end].copy_from_slice(&nb[..end - pos]);
                        pos = end;
                    }
                }
                let caps_str = core::str::from_utf8(&buf[..pos]).unwrap_or("?");
                let _ = writeln!(w, "  {:>3}  {:<15}  {}", pid, p.name, caps_str);
            }
        }
        _ if sub.as_bytes().first().map_or(false, |b| b.is_ascii_digit()) && args.is_empty() => {
            let pid = parse_usize(sub);
            if pid >= 8 {
                let _ = writeln!(w, "  invalid pid: {}", sub);
                return;
            }
            let p = &pt.processes[pid];
            if p.state == ProcessState::Free {
                let _ = writeln!(w, "  pid {} is not active", pid);
                return;
            }
            let bits = p.caps.bits();
            let _ = writeln!(w, "  Process {} ({})", pid, p.name);
            let _ = writeln!(w, "  Capabilities (0x{:05X}):", bits);
            for &(bit, name) in CAP_NAMES {
                let flag = if bits & (1 << bit) != 0 { "+" } else { "-" };
                let _ = writeln!(w, "    {} {}", flag, name);
            }
        }
        "drop" => {
            let (pid_str, cap_name) = match args.find(' ') {
                Some(i) => (&args[..i], args[i + 1..].trim()),
                None => {
                    let _ = writeln!(w, "  usage: caps drop <pid> <cap_name>");
                    return;
                }
            };
            let pid = parse_usize(pid_str);
            if pid >= 8 {
                let _ = writeln!(w, "  invalid pid: {}", pid_str);
                return;
            }
            if pt.processes[pid].state == ProcessState::Free {
                let _ = writeln!(w, "  pid {} is not active", pid);
                return;
            }
            let bit = CAP_NAMES.iter().find(|&&(_, n)| n == cap_name);
            match bit {
                Some(&(b, name)) => {
                    let mask = ProcessCaps::from_bits_truncate(1 << b);
                    pt.drop_caps(pid, mask);
                    let remaining = pt.processes[pid].caps.bits();
                    let tick = unsafe { (*SCHEDULER.0.get()).ticks } as u32;
                    let audit = unsafe { &mut *AUDIT.0.get() };
                    audit.log(tick, pid as u8, 0, AuditEvent::CapDropped, 1 << b, remaining);
                    let _ = writeln!(w, "  dropped '{}' from pid {} \u{2014} caps now 0x{:05X}",
                        name, pid, remaining);
                }
                None => {
                    let _ = writeln!(w, "  unknown capability: '{}'", cap_name);
                }
            }
        }
        _ => {
            let _ = writeln!(w, "  usage: caps              \u{2014} list all processes");
            let _ = writeln!(w, "         caps <pid>        \u{2014} show process capabilities");
            let _ = writeln!(w, "         caps drop <pid> <cap>  \u{2014} drop a capability");
        }
    }
}

fn parse_usize(s: &str) -> usize {
    let mut n: usize = 0;
    for b in s.bytes() {
        if b.is_ascii_digit() {
            n = n.wrapping_mul(10).wrapping_add((b - b'0') as usize);
        } else {
            return usize::MAX;
        }
    }
    n
}

#[cfg(feature = "shell")]
fn auditlog_command(args: &str, w: &mut dyn core::fmt::Write) {
    let audit = unsafe { &*AUDIT.0.get() };
    let max = if args.is_empty() { 32 } else {
        let n = parse_usize(args);
        if n == usize::MAX { 32 } else { n }
    };
    if audit.total() == 0 {
        let _ = writeln!(w, "  (no audit events recorded)");
    } else {
        audit.dump(w, max);
    }
}

// ---------------------------------------------------------------------------
// VFS callbacks
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
        let inode = &inodes.inodes[id as usize];
        if inode.kind != InodeKind::File { return 0; }
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
        let inode = &inodes.inodes[id as usize];
        if inode.kind != InodeKind::File { return false; }
        let offset = if append { inode.size } else { 0 };
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
        let start = if path == "/" { ROOT_INODE } else {
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
        let _ = writeln!(w, "{}", if path == "/" || path == "." { "/" } else { path });
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
        while i > 0 { i -= 1; if sp < 64 { stack[sp] = (kids[i], 1); sp += 1; } }
        while sp > 0 {
            sp -= 1;
            let (id, depth) = stack[sp];
            let node = &inodes.inodes[id as usize];
            for _ in 0..depth { w.write_str("  ").ok(); }
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

#[cfg(feature = "shell")]
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

#[cfg(feature = "shell")]
fn lsblk_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  NAME   TYPE   SIZE");
    let _ = writeln!(w, "  (no block devices \u{2014} virtual IoT node)");
}

#[cfg(feature = "shell")]
fn input_status(w: &mut dyn core::fmt::Write) {
    let input = unsafe { &*INPUT.0.get() };
    input.write_status(w);
}

#[cfg(feature = "shell")]
fn dmesg_info(w: &mut dyn core::fmt::Write) {
    let klog = unsafe { &*KLOG.0.get() };
    klog.dump(w);
}

// ---------------------------------------------------------------------------
// QEMU power-off via SiFive Test device
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
fn qemu_poweroff() -> ! {
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::ptr::write_volatile(0x10_0000 as *mut u32, 0x5555);
    }
    loop {
        #[cfg(target_arch = "riscv32")]
        unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
        #[cfg(not(target_arch = "riscv32"))]
        core::hint::spin_loop();
    }
}

#[cfg(feature = "shell")]
fn do_shutdown() {
    qemu_poweroff();
}

/// RISC-V initial mstatus: MPIE=1 (bit 7), MPP=M-mode (bits 12:11 = 0b11).
const INITIAL_MSTATUS: usize = (1 << 7) | (3 << 11);

// ═══════════════════════════════════════════════════════════════════════════
// Entry point
// ═══════════════════════════════════════════════════════════════════════════

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
    let _ = writeln!(con, "  VeerOS v{VERSION}  [ESP32-C6 Virtual]");
    let _ = writeln!(con, "  Platform : QEMU ESP32-C6 (RISC-V 32)");
    let _ = writeln!(con, "  Mode     : Virtual IoT Node");
    let _ = writeln!(con, "  Scheduler: {}", kernel.scheduler_label());
    let _ = writeln!(con, "========================================");
    let _ = writeln!(con, "");

    // ── kernel heap (constrained, 16 KiB) ────────────────────
    unsafe {
        let region = &mut *core::ptr::addr_of_mut!(HEAP_REGION);
        (*HEAP.0.get()).init(&mut region.0, HEAP_SMALL_BYTES);
    }
    let _ = writeln!(
        con,
        "[boot] heap initialised ({} KiB \u{2014} small {}K + large {}K)",
        HEAP_SIZE / 1024,
        HEAP_SMALL_BYTES / 1024,
        (HEAP_SIZE - HEAP_SMALL_BYTES) / 1024,
    );

    // ── register drivers ─────────────────────────────────────
    unsafe {
        let reg = &mut *DRIVERS.0.get();

        // NS16550a UART (QEMU virt)
        let uart = reg.register("uart0", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(uart, MemRegion::new(0x1000_0000, 0x100)).ok();
        reg.grant_irq(uart, 10);

        // CLINT timer (QEMU virt)
        let clint = reg.register("clint", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(clint, MemRegion::new(0x0200_0000, 0x10000)).ok();
        reg.grant_irq(clint, 7);

        // VIRTIO-NET (for simulated WiFi uplink)
        #[cfg(feature = "net")]
        {
            let vnet = reg.register("virtio-net", DriverCaps {
                mmio_regions: 1,
                uses_interrupts: true,
                uses_dma: true,
                uses_network: true,
            }).unwrap();
            reg.grant_mmio(vnet, MemRegion::new(0x1000_1000, 0x1000)).ok();
            reg.grant_irq(vnet, 1);
        }

        // Simulated radio peripherals (no real MMIO)
        #[cfg(feature = "wifi")]
        {
            let _ = reg.register("wifi-sim", DriverCaps {
                mmio_regions: 0, uses_interrupts: false, uses_dma: false, uses_network: true,
            });
        }
        #[cfg(feature = "ble")]
        {
            let _ = reg.register("ble-sim", DriverCaps {
                mmio_regions: 0, uses_interrupts: false, uses_dma: false, uses_network: false,
            });
        }
        #[cfg(feature = "ieee802154")]
        {
            let _ = reg.register("802154-sim", DriverCaps {
                mmio_regions: 0, uses_interrupts: false, uses_dma: false, uses_network: true,
            });
        }
    }
    let _ = writeln!(con, "[boot] driver registry initialised");

    // ── install trap vector ──────────────────────────────────
    #[cfg(target_arch = "riscv32")]
    {
        extern "C" { fn _veer_trap_entry(); }
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

    // ── CLINT timer ──────────────────────────────────────────
    let timer = system_timer();
    timer.configure_tick(TICK_PERIOD_US);
    unsafe { *TIMER.0.get() = timer; }
    let _ = writeln!(con, "[boot] CLINT timer tick @ {} us", TICK_PERIOD_US);

    // ── enable machine timer interrupt ───────────────────────
    #[cfg(target_arch = "riscv32")]
    unsafe {
        core::arch::asm!("csrs mie, {0}", in(reg) (1u32 << 7), options(nomem, nostack));
    }
    let _ = writeln!(con, "[boot] machine timer interrupt enabled");

    // ── VFS initialisation (ESP32-C6 layout) ─────────────────
    unsafe {
        let inodes = &mut *INODES.0.get();
        let ramfs = &mut *RAMFS.0.get();
        inodes.init_root();

        // Standard device nodes
        let dev_id = inodes.resolve(microkernel::vfs::ROOT_INODE, "/dev").unwrap_or(microkernel::vfs::NO_INODE);
        if dev_id != microkernel::vfs::NO_INODE {
            inodes.create_device_in(dev_id, "null", 0, 0);
            inodes.create_device_in(dev_id, "zero", 0, 1);
            inodes.create_device_in(dev_id, "console", 0, 2);
            inodes.create_device_in(dev_id, "random", 0, 3);

            // Virtual sensor device directory
            if let Some(sensor_dir) = inodes.mkdir_in(dev_id, "sensor") {
                // Pre-create default IoT sensors
                let sensors = &mut *SENSORS.0.get();
                sensors.add("temperature", "C", 2250);     // 22.50 C
                sensors.add("humidity", "%RH", 5500);       // 55.00 %RH
                sensors.add("pressure", "hPa", 101325);     // 1013.25 hPa
                sensors.add("light", "lux", 45000);          // 450.00 lux

                // Create device files for each sensor
                for s in sensors.sensors.iter() {
                    if s.active {
                        let name = core::str::from_utf8(&s.name[..s.name_len]).unwrap_or("unknown");
                        inodes.create_device_in(sensor_dir, name, 2, 0);
                    }
                }
            }
        }

        // /etc contents
        let etc_id = inodes.resolve(microkernel::vfs::ROOT_INODE, "/etc").unwrap_or(microkernel::vfs::NO_INODE);
        if etc_id != microkernel::vfs::NO_INODE {
            ramfs.create_with_content(inodes, etc_id, "motd", b"Welcome to VeerOS ESP32-C6 Virtual IoT Node!\n");
            ramfs.create_with_content(inodes, etc_id, "hostname", b"veeros-esp32c6-vm\n");

            // /etc/net/wifi — default WiFi config
            if let Some(net_id) = inodes.mkdir_in(etc_id, "net") {
                ramfs.create_with_content(inodes, net_id, "wifi",
                    b"# VeerOS WiFi configuration (simulated)\nssid=veeros-sim-ap\npassword=\n");
            }
        }
    }
    let _ = writeln!(con, "[boot] VFS initialised (ESP32-C6 layout)");
    let _ = writeln!(con, "[boot] virtual sensors: temperature, humidity, pressure, light");

    // ── scheduler + tasks ────────────────────────────────────
    unsafe {
        let sched = &mut *SCHEDULER.0.get();
        let procs = &mut *PROCESSES.0.get();

        procs.create("init", usize::MAX, 0, 0);

        let user_tbl = &mut *USERS.0.get();
        user_tbl.init_defaults();

        // Idle task (priority 0)
        let sb = IDLE_STACK.0.as_ptr() as usize;
        let st = sb + IDLE_STACK.0.len();
        if let Some(idx) = sched.create_task("idle", idle_task as *const () as usize, st, sb, 0, 0) {
            sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
        }

        // Shell task (priority 1)
        #[cfg(feature = "shell")]
        {
            let sb = SHELL_STACK.0.as_ptr() as usize;
            let st = sb + SHELL_STACK.0.len();
            if let Some(idx) = sched.create_task("shell", shell_task as *const () as usize, st, sb, 1, 0) {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
            }
        }

        // Network listener task (priority 1)
        #[cfg(feature = "net")]
        {
            let sb = NET_TASK_STACK.0.as_ptr() as usize;
            let st = sb + NET_TASK_STACK.0.len();
            if let Some(idx) = sched.create_task("net", net_task as *const () as usize, st, sb, 1, 0) {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
            }
        }
    }
    // Set thread count.
    unsafe {
        let sched = &*SCHEDULER.0.get();
        let procs = &mut *PROCESSES.0.get();
        let count = sched.tasks.iter().filter(|t| t.state != TaskState::Free).count();
        procs.processes[0].thread_count = count;
    }

    let _ = writeln!(con, "[boot] idle task registered");
    #[cfg(feature = "shell")]
    let _ = writeln!(con, "[boot] shell task registered");
    #[cfg(feature = "net")]
    let _ = writeln!(con, "[boot] net listener task registered (port {})", REMOTE_SHELL_PORT);

    // ── start the first task (never returns) ─────────────────
    let _ = writeln!(con, "[boot] starting scheduler \u{2014} virtual IoT node ready");
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
    0xFF
}

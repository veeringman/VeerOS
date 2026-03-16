//! VeerOS kernel for the QEMU `virt` RISC-V 32-bit machine.
//!
//! This is the **real** bare-metal kernel — `#![no_std]`, `#![no_main]`,
//! runs on `riscv32imc-unknown-none-elf` under QEMU with real trap vectors,
//! CLINT timer interrupts, and a preemptive scheduler.

#![no_std]
#![no_main]

mod trap;
#[cfg(feature = "samples")]
mod samples;

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
use smoltcp::iface::SocketSet;
#[cfg(feature = "net")]
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

#[cfg(feature = "shell")]
#[repr(align(16))]
struct ShellStack([u8; 8192]);
#[cfg(feature = "shell")]
static SHELL_STACK: ShellStack = ShellStack([0u8; 8192]);

// ---------------------------------------------------------------------------
// Sample task stacks (userlib tests)
// ---------------------------------------------------------------------------

#[cfg(feature = "samples")]
#[repr(align(16))]
struct SampleStack([u8; 4096]);

#[cfg(feature = "samples")]
static HELLO_STACK: SampleStack = SampleStack([0u8; 4096]);
#[cfg(feature = "samples")]
static TIMER_STACK: SampleStack = SampleStack([0u8; 4096]);
#[cfg(feature = "samples")]
static IPC_TX_STACK: SampleStack = SampleStack([0u8; 4096]);
#[cfg(feature = "samples")]
static IPC_RX_STACK: SampleStack = SampleStack([0u8; 4096]);

#[cfg(feature = "shell")]
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
        bt_cmd: None,
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
        ble_hid_list: None,
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
#[cfg(feature = "net")]
/// TCP port for VeerOS remote shell (like SSH, unencrypted for now).
const REMOTE_SHELL_PORT: u16 = 2323;

#[cfg(feature = "net")]
/// FNV-1a hash of the remote shell password.
/// Default: "veeros" — override by changing this constant.
const REMOTE_PASSWORD_HASH: u32 = net::auth::fnv1a(b"veeros");

#[cfg(feature = "net")]
/// QEMU user-net default: guest is 10.0.2.15, gateway 10.0.2.2.
const GUEST_IP: [u8; 4] = [10, 0, 2, 15];
#[cfg(feature = "net")]
const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];

#[cfg(feature = "net")]
#[repr(align(16))]
struct NetStack0([u8; 8192]);
#[cfg(feature = "net")]
static NET_TASK_STACK: NetStack0 = NetStack0([0u8; 8192]);

#[cfg(feature = "net")]
// Static smoltcp socket-set storage (one socket for the listener).
static mut SOCKET_STORAGE: [smoltcp::iface::SocketStorage<'static>; 4] =
    [smoltcp::iface::SocketStorage::EMPTY; 4];
#[cfg(feature = "net")]
static mut NET_STORAGE: NetStorage = NetStorage::new();

#[cfg(feature = "net")]
// The network stack and socket set are stored globally so the poll_fn
// callback (called from TcpSerial) can drive them.
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

/// Global poll function handed to TcpSerial so it can drive the stack
/// while blocking on read_byte / write_byte.
#[cfg(feature = "net")]
fn net_poll() {
    unsafe {
        if let (Some(stack), Some(sockets)) =
            (&mut *NET.0.get(), &mut *NET_SOCKETS.0.get())
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
#[cfg(feature = "net")]
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
        *NET_SOCKETS.0.get() = Some(socket_set);
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
                (&mut *NET.0.get(), &mut *NET_SOCKETS.0.get())
            {
                stack.listen(sockets, REMOTE_SHELL_PORT);
            }
        }

        // Poll until a client connects.
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
            if connected {
                break;
            }
            core::hint::spin_loop();
        }

        let _ = writeln!(con, "[net] client connected");

        // ── authenticate, then run the shell over TCP ────────
        unsafe {
            let handle = (*NET.0.get()).as_ref().unwrap().tcp_handle();
            let socket_set_ptr = (*NET_SOCKETS.0.get()).as_mut().unwrap() as *mut SocketSet<'static>;
            let tcp_serial = TcpSerial::new(handle, socket_set_ptr, net_poll);
            let mut tcp_con = Console::new(tcp_serial);

            #[cfg(feature = "shell")]
            {
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
                        bt_cmd: None,
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
                        ble_hid_list: None,
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

        // Abort the socket so it can be re-used immediately (skip TIME_WAIT).
        unsafe {
            if let Some(sockets) = &mut *NET_SOCKETS.0.get() {
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
        // Shell runs as init (pid 0) — read its UID from the process table.
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
// VFS callbacks (injected into the shell via ShellEnv)
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
                // Walk children linked list.
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
            // Create the file: split into parent + name.
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
                // Relative name in cwd.
                id = match inodes.create_file_in(cwd, path) {
                    Some(i) => i,
                    None => return false,
                };
            }
        }
        let inode = &inodes.inodes[id as usize];
        if inode.kind != InodeKind::File { return false; }
        let offset = if append { inode.size } else { 0 };
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
        // Resolve the new parent and name.
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
        // Iterative depth-first traversal with stack.
        // Stack entries: (inode_id, depth)
        let mut stack: [(u16, u8); 64] = [(NO_INODE, 0); 64];
        let mut sp = 0usize;
        // Push root children in reverse order so they print in order.
        let root = &inodes.inodes[start as usize];
        if root.kind != InodeKind::Directory {
            let _ = writeln!(w, "tree: '{}': not a directory", path);
            return;
        }
        let _ = writeln!(w, "{}", if path == "/" || path == "." { "/" } else { path });
        // Collect children into a small temp buffer, then push reversed.
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
            // Print indent.
            for _ in 0..depth {
                w.write_str("  ").ok();
            }
            let kind_ch = match node.kind {
                InodeKind::Directory => '/',
                InodeKind::Device => '*',
                _ => ' ',
            };
            let _ = writeln!(w, "{}{}", node.name_str(), kind_ch);
            // If directory, push children.
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
        // If already exists, success (touch existing = no-op).
        if inodes.resolve(cwd, path).unwrap_or(NO_INODE) != NO_INODE {
            return true;
        }
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
    let _ = writeln!(w, "  (no block devices — QEMU virtio-blk not yet implemented)");
}

#[cfg(feature = "shell")]
fn input_status(w: &mut dyn core::fmt::Write) {
    let input = unsafe { &*INPUT.0.get() };
    input.write_status(w);
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
    }
    #[cfg(feature = "net")]
    let driver_count = 3;
    #[cfg(not(feature = "net"))]
    let driver_count = 2;
    let _ = writeln!(con, "[boot] driver registry: {} drivers registered", driver_count);

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

    // ── VFS initialisation ───────────────────────────────────
    unsafe {
        let inodes = &mut *INODES.0.get();
        let ramfs = &mut *RAMFS.0.get();
        // Create root (/) and standard directories (/dev, /tmp, /etc).
        inodes.init_root();
        // Create device nodes.
        let dev_id = inodes.resolve(microkernel::vfs::ROOT_INODE, "/dev").unwrap_or(microkernel::vfs::NO_INODE);
        if dev_id != microkernel::vfs::NO_INODE {
            inodes.create_device_in(dev_id, "null", 0, 0);
            inodes.create_device_in(dev_id, "zero", 0, 1);
            inodes.create_device_in(dev_id, "console", 0, 2);
            inodes.create_device_in(dev_id, "random", 0, 3);
            inodes.create_device_in(dev_id, "keyboard", 1, 0);
            inodes.create_device_in(dev_id, "mouse", 1, 1);
        }
        // Populate /etc/motd and /etc/hostname.
        let etc_id = inodes.resolve(microkernel::vfs::ROOT_INODE, "/etc").unwrap_or(microkernel::vfs::NO_INODE);
        if etc_id != microkernel::vfs::NO_INODE {
            ramfs.create_with_content(inodes, etc_id, "motd", b"Welcome to VeerOS!\n");
            ramfs.create_with_content(inodes, etc_id, "hostname", b"veeros-qemu\n");
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

        // Network listener task (priority 1).
        #[cfg(feature = "net")]
        {
            let sb = NET_TASK_STACK.0.as_ptr() as usize;
            let st = sb + NET_TASK_STACK.0.len();
            if let Some(idx) = sched.create_task("net", net_task as *const () as usize, st, sb, 1, 0) {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
            }
        }

        // ── userlib sample tasks ─────────────────────────────
        #[cfg(feature = "samples")]
        {
            let sb = HELLO_STACK.0.as_ptr() as usize;
            let st = sb + HELLO_STACK.0.len();
            if let Some(idx) = sched.create_task("hello", samples::hello_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
            }

            let sb = TIMER_STACK.0.as_ptr() as usize;
            let st = sb + TIMER_STACK.0.len();
            if let Some(idx) = sched.create_task("timer", samples::timer_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
            }

            // IPC pair: sender (slot N) talks to receiver (slot N+1)
            let sb = IPC_TX_STACK.0.as_ptr() as usize;
            let st = sb + IPC_TX_STACK.0.len();
            if let Some(idx) = sched.create_task("ipc-tx", samples::ipc_sender_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_MSTATUS);
            }

            let sb = IPC_RX_STACK.0.as_ptr() as usize;
            let st = sb + IPC_RX_STACK.0.len();
            if let Some(idx) = sched.create_task("ipc-rx", samples::ipc_receiver_task as *const () as usize, st, sb, 2, 0) {
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
    #[cfg(feature = "net")]
    let _ = writeln!(con, "[boot] net listener task registered (port {})", REMOTE_SHELL_PORT);
    #[cfg(feature = "samples")]
    let _ = writeln!(con, "[boot] userlib sample tasks registered (hello, timer, ipc-tx, ipc-rx)");

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
    // Non-blocking read — return 0xFF if no data available.
    0xFF
}

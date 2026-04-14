//! VeerOS kernel for the QEMU `q35`/`pc` x86-64 machine.
//!
//! This is a bare-metal kernel — `#![no_std]`, `#![no_main]`.
//! Boots via Multiboot (v1) in 32-bit protected mode, transitions to
//! 64-bit long mode, then runs the full VeerOS scheduler with shell.

#![no_std]
#![no_main]

mod trap;
#[cfg(feature = "samples")]
mod samples;

use core::cell::UnsafeCell;
use core::fmt::Write;

use arch::{Console, SavedContext, Serial, TickTimer};
use soc_qemu_pc::{default_serial, system_timer, pit::Pit8254, QemuPc};
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

/// Kernel tick period (1 ms).
const TICK_PERIOD_US: u32 = 1_000;

// ---------------------------------------------------------------------------
// Static scheduler
// ---------------------------------------------------------------------------

struct SchedulerCell(UnsafeCell<Scheduler>);
unsafe impl Sync for SchedulerCell {}

pub(crate) static SCHEDULER: SchedulerCell = SchedulerCell(UnsafeCell::new(Scheduler::new()));

// ---------------------------------------------------------------------------
// Kernel heap (64 KiB — small pool 16 KiB, large pool 48 KiB)
// ---------------------------------------------------------------------------

const HEAP_SIZE: usize = 64 * 1024;
const HEAP_SMALL_BYTES: usize = 16 * 1024;

#[repr(align(16))]
struct HeapRegion([u8; HEAP_SIZE]);
static mut HEAP_REGION: HeapRegion = HeapRegion([0u8; HEAP_SIZE]);

struct HeapCell(UnsafeCell<Heap>);
unsafe impl Sync for HeapCell {}
pub(crate) static HEAP: HeapCell = HeapCell(UnsafeCell::new(Heap::new()));

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
pub(crate) static DRIVERS: RegistryCell = RegistryCell(UnsafeCell::new(DriverRegistry::new()));

// ---------------------------------------------------------------------------
// Audit log
// ---------------------------------------------------------------------------

use microkernel::audit::AuditLog;

pub(crate) struct AuditCell(pub UnsafeCell<AuditLog>);
unsafe impl Sync for AuditCell {}
pub(crate) static AUDIT: AuditCell = AuditCell(UnsafeCell::new(AuditLog::new()));

// ---------------------------------------------------------------------------
// PCI device table
// ---------------------------------------------------------------------------

use soc_qemu_pc::pci::PciDevices;

pub(crate) struct PciCell(pub UnsafeCell<PciDevices>);
unsafe impl Sync for PciCell {}
pub(crate) static PCI_DEVICES: PciCell = PciCell(UnsafeCell::new(PciDevices::new()));

// ---------------------------------------------------------------------------
// Frame allocator (physical memory)
// ---------------------------------------------------------------------------

use soc_qemu_pc::mm::FrameAllocator;

pub(crate) struct FrameAllocCell(pub UnsafeCell<FrameAllocator>);
unsafe impl Sync for FrameAllocCell {}
pub(crate) static FRAME_ALLOC: FrameAllocCell = FrameAllocCell(UnsafeCell::new(FrameAllocator::new()));

// ---------------------------------------------------------------------------
// Static timer handle (used by the trap dispatcher)
// ---------------------------------------------------------------------------

pub(crate) struct TimerCell(pub UnsafeCell<Pit8254>);
unsafe impl Sync for TimerCell {}

pub(crate) static TIMER: TimerCell = TimerCell(UnsafeCell::new(Pit8254::new()));

// ---------------------------------------------------------------------------
// Kernel log ring buffer
// ---------------------------------------------------------------------------

use microkernel::klog::KernelLog;

pub(crate) struct KlogCell(pub UnsafeCell<KernelLog>);
unsafe impl Sync for KlogCell {}
pub(crate) static KLOG: KlogCell = KlogCell(UnsafeCell::new(KernelLog::new()));

// ---------------------------------------------------------------------------
// ACPI info
// ---------------------------------------------------------------------------

use soc_qemu_pc::acpi::AcpiInfo;

pub(crate) struct AcpiCell(pub UnsafeCell<AcpiInfo>);
unsafe impl Sync for AcpiCell {}
pub(crate) static ACPI_INFO: AcpiCell = AcpiCell(UnsafeCell::new(AcpiInfo::new()));

// ---------------------------------------------------------------------------
// VIRTIO block device
// ---------------------------------------------------------------------------

use soc_qemu_pc::virtio_blk::VirtioBlk;

pub(crate) struct VirtioBlkCell(pub UnsafeCell<VirtioBlk>);
unsafe impl Sync for VirtioBlkCell {}
pub(crate) static VIRTIO_BLK: VirtioBlkCell = VirtioBlkCell(UnsafeCell::new(VirtioBlk::new()));

// ---------------------------------------------------------------------------
// VIRTIO network device
// ---------------------------------------------------------------------------

use soc_qemu_pc::virtio_net::VirtioNet;

pub(crate) struct VirtioNetCell(pub UnsafeCell<VirtioNet>);
unsafe impl Sync for VirtioNetCell {}
pub(crate) static VIRTIO_NET: VirtioNetCell = VirtioNetCell(UnsafeCell::new(VirtioNet::new()));

// ---------------------------------------------------------------------------
// Address space table (per-process page tables)
// ---------------------------------------------------------------------------

use soc_qemu_pc::mm::AddressSpaceTable;

pub(crate) struct AddrSpaceCell(pub UnsafeCell<AddressSpaceTable>);
unsafe impl Sync for AddrSpaceCell {}
pub(crate) static ADDR_SPACES: AddrSpaceCell = AddrSpaceCell(UnsafeCell::new(AddressSpaceTable::new()));

// ---------------------------------------------------------------------------
// Keyboard ring buffer (fed by IRQ1 / COM1 IRQ4 via trap handler)
// ---------------------------------------------------------------------------

const KBD_BUF_SIZE: usize = 64;

struct KbdRing {
    buf: [u8; KBD_BUF_SIZE],
    head: usize,
    tail: usize,
}

impl KbdRing {
    const fn new() -> Self {
        Self { buf: [0; KBD_BUF_SIZE], head: 0, tail: 0 }
    }
    fn push(&mut self, byte: u8) {
        let next = (self.head + 1) % KBD_BUF_SIZE;
        if next != self.tail {
            self.buf[self.head] = byte;
            self.head = next;
        }
    }
    fn pop(&mut self) -> Option<u8> {
        if self.head == self.tail {
            None
        } else {
            let byte = self.buf[self.tail];
            self.tail = (self.tail + 1) % KBD_BUF_SIZE;
            Some(byte)
        }
    }
    fn is_empty(&self) -> bool {
        self.head == self.tail
    }
}

struct KbdCell(UnsafeCell<KbdRing>);
unsafe impl Sync for KbdCell {}
static KBD_RING: KbdCell = KbdCell(UnsafeCell::new(KbdRing::new()));

/// Push a byte into the keyboard ring buffer (called from ISR).
pub(crate) fn kbd_buffer_push(byte: u8) {
    unsafe { (*KBD_RING.0.get()).push(byte); }
}

/// Pop a byte from the keyboard ring buffer, or return None.
fn kbd_buffer_pop() -> Option<u8> {
    unsafe { (*KBD_RING.0.get()).pop() }
}

/// Check if the keyboard buffer has data.
fn kbd_buffer_has_data() -> bool {
    unsafe { !(*KBD_RING.0.get()).is_empty() }
}

// ---------------------------------------------------------------------------
// Idle task
// ---------------------------------------------------------------------------

fn idle_task() -> ! {
    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("hlt", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "x86_64"))]
        core::hint::spin_loop();
    }
}

#[repr(align(16))]
struct IdleStack([u8; 4096]);
static IDLE_STACK: IdleStack = IdleStack([0u8; 4096]);

// ---------------------------------------------------------------------------
// Shell task
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
#[repr(align(16))]
struct ShellStack([u8; 32768]);
#[cfg(feature = "shell")]
static SHELL_STACK: ShellStack = ShellStack([0u8; 32768]);

// ---------------------------------------------------------------------------
// Sample task stacks (userlib tests)
// ---------------------------------------------------------------------------

#[cfg(feature = "samples")]
#[repr(align(16))]
struct SampleStack([u8; 16384]);

#[cfg(feature = "samples")]
static HELLO_STACK: SampleStack = SampleStack([0u8; 16384]);
#[cfg(feature = "samples")]
static TIMER_STACK: SampleStack = SampleStack([0u8; 16384]);
#[cfg(feature = "samples")]
static IPC_TX_STACK: SampleStack = SampleStack([0u8; 16384]);
#[cfg(feature = "samples")]
static IPC_RX_STACK: SampleStack = SampleStack([0u8; 16384]);

#[cfg(feature = "shell")]
fn shell_task() -> ! {
    let serial = default_serial();
    let mut con = Console::new(serial);
    let env = ShellEnv {
        version: VERSION,
        platform: "QEMU PC (x86-64)",
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
        gpio_cmd: None,
        i2c_cmd: None,
        spi_cmd: None,
        hw_info: Some(hw_info),
        get_temp_millic: None,
        dmesg: Some(dmesg_info),
        reboot: None,
        shutdown: Some(do_shutdown),
        caps_cmd: None,
        auditlog_cmd: None,
    };
    let mut sh = Shell::new(env);
    sh.run(&mut con);

    let _ = writeln!(con, "VeerOS halted \u{2014} powering off.");
    qemu_poweroff();
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
fn hw_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  Platform: QEMU PC (x86-64)");
    let _ = writeln!(w, "  Timer:    PIT 8254 (1 ms tick)");
    let _ = writeln!(w, "  Serial:   COM1 (115200 8N1)");
    let _ = writeln!(w, "  Display:  VGA text-mode (80x25)");
    let _ = writeln!(w, "  Input:    PS/2 keyboard");
    let _ = writeln!(w, "");

    // ACPI info.
    unsafe {
        let acpi = &*ACPI_INFO.0.get();
        if acpi.valid {
            let _ = writeln!(w, "  ACPI:     {} CPU(s), BSP APIC ID={}",
                acpi.cpu_count, acpi.bsp_apic_id);
            let _ = writeln!(w, "  LAPIC:    0x{:08x}", acpi.local_apic_addr);
            let _ = writeln!(w, "  I/O APIC: 0x{:08x} (ID={})", acpi.io_apic_addr, acpi.io_apic_id);
            if acpi.hpet_base != 0 {
                let _ = writeln!(w, "  HPET:     0x{:016x}", acpi.hpet_base);
            }
        } else {
            let _ = writeln!(w, "  ACPI:     not available");
        }
    }

    // SMP info.
    let cpus_online = soc_qemu_pc::smp::online_cpu_count();
    let _ = writeln!(w, "  CPUs:     {} online", cpus_online);
    let _ = writeln!(w, "");

    // VIRTIO devices.
    unsafe {
        let blk = &*VIRTIO_BLK.0.get();
        if blk.active {
            let _ = writeln!(w, "  virtio-blk: {} MiB ({} sectors){}",
                blk.capacity_bytes() / (1024 * 1024),
                blk.capacity_sectors(),
                if blk.read_only { " [RO]" } else { "" });
        }
        let net = &*VIRTIO_NET.0.get();
        if net.active {
            let mut mac_buf = [0u8; 18];
            let mac_len = net.mac_fmt(&mut mac_buf);
            let mac_str = core::str::from_utf8(&mac_buf[..mac_len]).unwrap_or("??");
            let _ = writeln!(w, "  virtio-net: MAC={}", mac_str);
        }
    }
    let _ = writeln!(w, "");

    let _ = writeln!(w, "  PCI Devices:");
    unsafe {
        let pci = &*PCI_DEVICES.0.get();
        if pci.count == 0 {
            let _ = writeln!(w, "    (none found)");
        } else {
            let _ = writeln!(w, "    BDF        VID:DID    Class");
            let _ = writeln!(w, "    -------    ---------  -----------");
            for i in 0..pci.count {
                let d = &pci.devices[i];
                let name = soc_qemu_pc::pci::class_name(d.class_code, d.subclass);
                let _ = writeln!(
                    w,
                    "    {:02x}:{:02x}.{}    {:04x}:{:04x}  {}",
                    d.bus, d.device, d.function,
                    d.vendor_id, d.device_id, name,
                );
            }
        }
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
    let _ = writeln!(w, "  NAME        TYPE     SIZE");
    let _ = writeln!(w, "  ----------  -------  --------");
    unsafe {
        let blk = &*VIRTIO_BLK.0.get();
        if blk.active {
            let _ = writeln!(w, "  vda         virtblk  {} MiB ({} sectors){}",
                blk.capacity_bytes() / (1024 * 1024),
                blk.capacity_sectors(),
                if blk.read_only { " [RO]" } else { "" });
        } else {
            let _ = writeln!(w, "  (no block devices)");
        }
    }
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
// QEMU power-off via ACPI I/O port and debug-exit device
// ---------------------------------------------------------------------------

#[cfg(feature = "shell")]
fn qemu_poweroff() -> ! {
    // Try ACPI shutdown using discovered PM1a_CNT_BLK.
    unsafe {
        let acpi = &*ACPI_INFO.0.get();
        if acpi.valid && acpi.pm1a_control_block != 0 {
            soc_qemu_pc::acpi::acpi_shutdown(acpi.pm1a_control_block);
        }
    }
    // Fallback: QEMU-specific ports (0x2000 = SLP_EN | SLP_TYP, split into two bytes).
    #[cfg(target_arch = "x86_64")]
    unsafe {
        soc_qemu_pc::outb(0x604, 0x00);
        soc_qemu_pc::outb(0x605, 0x20);
        soc_qemu_pc::outb(0x501, 0x31);
    }
    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
        #[cfg(not(target_arch = "x86_64"))]
        core::hint::spin_loop();
    }
}

#[cfg(feature = "shell")]
fn do_shutdown() {
    qemu_poweroff();
}

// ---------------------------------------------------------------------------
// x86-64 RFLAGS: IF=1 (interrupts enabled)
// ---------------------------------------------------------------------------

/// Initial RFLAGS for new tasks: IF=1 (bit 9) so timer interrupts work,
/// bit 1 is always 1 (reserved).
const INITIAL_RFLAGS: usize = (1 << 9) | (1 << 1);

// ═══════════════════════════════════════════════════════════════════════════
// Boot assembly: Multiboot header + 32→64 bit transition
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(r#"
// =====================================================================
// Multiboot v1 header — must be in the first 8 KiB of the binary.
// QEMU's -kernel flag understands Multiboot and loads the ELF directly.
// =====================================================================
.section .multiboot, "a"
.balign 4
multiboot_header:
    .long 0x1BADB002                         // magic
    .long 0x00000003                         // flags: ALIGN + MEMINFO
    .long -(0x1BADB002 + 0x00000003)         // checksum (magic+flags+check = 0)

// =====================================================================
// 32-bit boot entry point.
// Multiboot hands us 32-bit protected mode, paging off, A20 enabled.
// We set up identity-map page tables and transition to 64-bit long mode.
// =====================================================================
.section .text._start
.code32
.global _start
.balign 16

_start:
    // Disable interrupts.
    cli

    // Save Multiboot info pointer (EBX) and magic (EAX).
    mov esi, ebx       // Multiboot info struct pointer (preserved into 64-bit)
    mov edi, eax       // Multiboot magic (preserved into 64-bit)

    // ----- Zero the page table area (PML4 + PDPT + 4×PD = 6 × 4096 = 24 KiB) -----
    lea    eax, [__pml4]
    mov    ecx, (4096 * 6 / 4)
    xor    edx, edx
.zero_pt:
    mov    [eax], edx
    add    eax, 4
    dec    ecx
    jnz    .zero_pt

    // ----- Set up PML4 → PDPT → 4×PD identity map (full 4 GiB) -----
    // PML4[0] = &PDPT | PRESENT | WRITABLE
    lea    eax, [__pdpt]
    or     eax, 0x03     // PRESENT + WRITABLE
    lea    ebx, [__pml4]
    mov    [ebx], eax

    // PDPT[0..3] = &PD[i] | PRESENT | WRITABLE
    lea    ebx, [__pdpt]
    lea    eax, [__pd]
    or     eax, 0x03
    mov    [ebx],      eax
    add    eax, 4096            // PD for 2nd GiB
    mov    [ebx + 8],  eax
    add    eax, 4096            // PD for 3rd GiB
    mov    [ebx + 16], eax
    add    eax, 4096            // PD for 4th GiB
    mov    [ebx + 24], eax

    // PD[0..2047] = i * 2 MiB | PRESENT | WRITABLE | HUGE (PS bit 7)
    // 4 PDs × 512 entries = 2048 entries = 4 GiB
    lea    ebx, [__pd]
    xor    ecx, ecx      // i = 0
    xor    edx, edx      // address accumulator (low 32 bits)
.fill_pd:
    mov    eax, edx
    or     eax, 0x83     // PRESENT(0) + WRITABLE(1) + HUGE(7)
    mov    [ebx + ecx*8], eax
    mov    dword ptr [ebx + ecx*8 + 4], 0  // high 32 bits = 0 (< 4 GiB)
    add    edx, 0x200000 // += 2 MiB
    inc    ecx
    cmp    ecx, 2048
    jb     .fill_pd

    // ----- Enable PAE (CR4.PAE = bit 5) -----
    mov    eax, cr4
    or     eax, (1 << 5)
    mov    cr4, eax

    // ----- Load CR3 with PML4 base -----
    lea    eax, [__pml4]
    mov    cr3, eax

    // ----- Enable Long Mode via IA32_EFER (MSR 0xC0000080), bit 8 -----
    mov    ecx, 0xC0000080
    rdmsr
    or     eax, (1 << 8)   // LME = Long Mode Enable
    wrmsr

    // ----- Enable Paging (CR0.PG = bit 31) — activates long mode -----
    mov    eax, cr0
    or     eax, (1 << 31)
    mov    cr0, eax

    // ----- Load the boot GDT (defined below) and far-jump to 64-bit -----
    lgdt   [boot_gdt_ptr]
    // Far jump to 64-bit code segment — encoded manually because
    // LLVM's Intel-syntax assembler doesn't support `jmp seg:offset`.
    .byte  0xEA                    // far jmp (opcode)
    .long  .long_mode_entry        // 32-bit target offset
    .word  0x08                    // code segment selector

// =====================================================================
// Boot GDT (minimal, used only for the 32→64 trampoline).
// The kernel loads its own GDT in Rust later.
// =====================================================================
.balign 16
boot_gdt:
    .quad 0x0000000000000000   // 0x00: null
    .quad 0x00AF9A000000FFFF   // 0x08: 64-bit code (DPL=0, L=1, P=1)
    .quad 0x00CF92000000FFFF   // 0x10: data (DPL=0, P=1)
boot_gdt_end:

.balign 4
boot_gdt_ptr:
    .word boot_gdt_end - boot_gdt - 1   // limit
    .long boot_gdt                        // base (32-bit — fine for low identity map)

// =====================================================================
// 64-bit long mode entry.
// =====================================================================
.code64
.balign 16
.long_mode_entry:
    // Reload data segment registers with kernel data selector.
    mov    ax, 0x10
    mov    ds, ax
    mov    es, ax
    mov    fs, ax
    mov    gs, ax
    mov    ss, ax

    // Set up the 64-bit kernel stack.
    lea    rsp, [__stack_top]

    // Zero the BSS section.
    lea    rdi, [__bss_start]
    lea    rcx, [__bss_end]
    sub    rcx, rdi
    shr    rcx, 3
    xor    rax, rax
    cld
    rep    stosq

    // Call the Rust entry point (never returns).
    call   _rust_start

    // Should not reach here.
    hlt
    jmp    .long_mode_entry
"#);

// ═══════════════════════════════════════════════════════════════════════════
// Rust entry — called from the boot assembly once in 64-bit long mode.
// ═══════════════════════════════════════════════════════════════════════════

#[unsafe(no_mangle)]
pub extern "C" fn _rust_start() -> ! {
    // ── early console (COM1) ─────────────────────────────────
    let serial = default_serial();
    serial.init();
    let mut con = Console::new(serial);

    // ── platform + kernel init ───────────────────────────────
    let platform = QemuPc::new();
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

    // ── physical frame allocator ─────────────────────────────
    unsafe {
        extern "C" {
            static __kernel_end: u8;
        }
        let kernel_end = &__kernel_end as *const u8 as usize;
        // Round up to next page boundary.
        let usable_start = (kernel_end + 0xFFF) & !0xFFF;
        // Assume 16 MiB RAM (can be increased by passing -m to QEMU).
        let usable_end = 0x100000 + 16 * 1024 * 1024;
        let fa = &mut *FRAME_ALLOC.0.get();
        fa.init(usable_start, usable_end);
        let _ = writeln!(
            con,
            "[boot] frame allocator: {} KiB free ({} frames, start=0x{:x})",
            fa.free_count() * 4,
            fa.free_count(),
            usable_start,
        );
    }

    // ── register drivers ─────────────────────────────────────
    unsafe {
        let reg = &mut *DRIVERS.0.get();

        // COM1 UART
        let com1 = reg.register("com1", DriverCaps {
            mmio_regions: 0,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_irq(com1, 4); // IRQ 4

        // PIT timer
        let pit = reg.register("pit8254", DriverCaps {
            mmio_regions: 0,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_irq(pit, 0); // IRQ 0

        // PS/2 keyboard
        let _kbd = reg.register("ps2kbd", DriverCaps {
            mmio_regions: 0,
            uses_interrupts: true,
            uses_dma: false,
            uses_network: false,
        }).unwrap();

        // VGA text console
        let _vga = reg.register("vga-text", DriverCaps {
            mmio_regions: 1,
            uses_interrupts: false,
            uses_dma: false,
            uses_network: false,
        }).unwrap();
        reg.grant_mmio(_vga, MemRegion { base: 0xB8000, size: 80 * 25 * 2 });
    }
    let _ = writeln!(con, "[boot] driver registry: 4 drivers registered");

    // ── GDT + IDT ────────────────────────────────────────────
    trap::load_gdt();
    let _ = writeln!(con, "[boot] GDT loaded (kernel + user segments + TSS)");
    trap::load_idt();
    let _ = writeln!(con, "[boot] IDT loaded (256 vectors)");
    trap::setup_syscall_msrs();
    let _ = writeln!(con, "[boot] SYSCALL/SYSRET MSRs configured");

    // ── PIC remap ────────────────────────────────────────────
    // (Already called by Platform::init_interrupts via kernel.boot(),
    //  but we call it explicitly here so the banner order is clear.)
    let _ = writeln!(con, "[boot] PIC remapped (IRQ 0-15 → vectors 32-47)");
    let _ = writeln!(con, "[boot] PS/2 keyboard enabled (IRQ 1, vector 33)");
    let _ = writeln!(con, "[boot] COM1 RX enabled (IRQ 4, vector 36)");

    // ── VGA text-mode console ────────────────────────────────
    let vga = soc_qemu_pc::vga::VgaText::new();
    vga.clear();
    {
        let mut vcon = Console::new(vga);
        let _ = writeln!(vcon, "VeerOS v{VERSION} — {}", kernel.platform_name());
        let _ = writeln!(vcon, "VGA text console active (80x25)");
    }
    let _ = writeln!(con, "[boot] VGA text-mode console initialised (80x25 @ 0xB8000)");

    // ── PIT timer ────────────────────────────────────────────
    let timer = system_timer();
    timer.configure_tick(TICK_PERIOD_US);
    unsafe {
        *TIMER.0.get() = timer;
    }
    let _ = writeln!(con, "[boot] PIT timer tick @ {} us", TICK_PERIOD_US);

    // ── PCI bus enumeration ──────────────────────────────────
    unsafe {
        let pci = &mut *PCI_DEVICES.0.get();
        soc_qemu_pc::pci::enumerate(pci);
        let _ = writeln!(con, "[boot] PCI: {} devices found", pci.count);
        for i in 0..pci.count {
            let d = &pci.devices[i];
            let name = soc_qemu_pc::pci::class_name(d.class_code, d.subclass);
            let _ = writeln!(
                con,
                "  {:02x}:{:02x}.{} {:04x}:{:04x} {}",
                d.bus, d.device, d.function,
                d.vendor_id, d.device_id, name,
            );
        }
    }

    // ── ACPI table discovery ─────────────────────────────────
    unsafe {
        let acpi = &mut *ACPI_INFO.0.get();
        if soc_qemu_pc::acpi::parse(acpi) {
            let _ = writeln!(con, "[boot] ACPI: {} CPU(s) found (BSP APIC ID={})",
                acpi.cpu_count, acpi.bsp_apic_id);
            let _ = writeln!(con, "  LAPIC @ 0x{:08x}, I/O APIC @ 0x{:08x} (ID={})",
                acpi.local_apic_addr, acpi.io_apic_addr, acpi.io_apic_id);
            if acpi.hpet_base != 0 {
                let _ = writeln!(con, "  HPET  @ 0x{:016x}", acpi.hpet_base);
            }
            if acpi.pm1a_control_block != 0 {
                let _ = writeln!(con, "  FADT: PM1a_CNT=0x{:04x}, SCI_INT={}",
                    acpi.pm1a_control_block, acpi.sci_interrupt);
            }
            for i in 0..acpi.irq_override_count {
                let ovr = &acpi.irq_overrides[i];
                let _ = writeln!(con, "  IRQ override: ISA {} → GSI {} flags=0x{:x}",
                    ovr.source_irq, ovr.gsi, ovr.flags);
            }
            for i in 0..acpi.cpu_count {
                let cpu = &acpi.cpus[i];
                let _ = writeln!(con, "  CPU {}: APIC ID={}, ACPI ID={}, {}",
                    i, cpu.apic_id, cpu.acpi_id,
                    if cpu.enabled { "enabled" } else { "disabled" });
            }
        } else {
            let _ = writeln!(con, "[boot] ACPI: tables not found (using defaults)");
        }
    }

    // ── Local APIC + I/O APIC initialisation ─────────────────
    unsafe {
        let acpi = &*ACPI_INFO.0.get();

        // Init LAPIC (BSP).
        soc_qemu_pc::lapic::init();
        let lapic_ver = soc_qemu_pc::lapic::version();
        let bsp_id = soc_qemu_pc::lapic::id();
        let _ = writeln!(con, "[boot] LAPIC: BSP ID={}, version=0x{:02x}", bsp_id, lapic_ver);

        // Calibrate LAPIC timer.
        let ticks_1ms = soc_qemu_pc::lapic::calibrate_timer_1ms();
        let _ = writeln!(con, "[boot] LAPIC timer: {} ticks/ms (div16)", ticks_1ms);

        // Init I/O APIC with standard ISA routing.
        let bsp_apic_id = if acpi.valid { acpi.bsp_apic_id } else { bsp_id as u8 };
        soc_qemu_pc::ioapic::init(bsp_apic_id);
        let ioapic_entries = soc_qemu_pc::ioapic::max_entries();
        let _ = writeln!(con, "[boot] I/O APIC: {} entries, routing IRQs to BSP (ID={})",
            ioapic_entries, bsp_apic_id);

        // Disable legacy PIC — all interrupts now go through I/O APIC → LAPIC.
        soc_qemu_pc::pic::disable();
        let _ = writeln!(con, "[boot] legacy PIC disabled (IOAPIC mode)");

        // Register LAPIC + IOAPIC as drivers.
        {
            let reg = &mut *DRIVERS.0.get();
            let _ = reg.register("lapic", DriverCaps {
                mmio_regions: 1,
                uses_interrupts: true,
                uses_dma: false,
                uses_network: false,
            });
            let _ = reg.register("ioapic", DriverCaps {
                mmio_regions: 1,
                uses_interrupts: false,
                uses_dma: false,
                uses_network: false,
            });
        }
    }

    // ── NXE (No-Execute Enable) ──────────────────────────────
    soc_qemu_pc::mm::enable_nxe();
    let _ = writeln!(con, "[boot] NXE enabled (No-Execute page protection)");

    // ── Higher-half kernel mapping ───────────────────────────
    // Set up the higher-half mapping (PML4[256] = PML4[0]) for future use.
    // The kernel continues to run identity-mapped for now.
    unsafe {
        extern "C" {
            static __pml4: u8;
        }
        let pml4_addr = core::ptr::addr_of!(__pml4) as usize;
        let pml4 = &mut *(pml4_addr as *mut soc_qemu_pc::mm::PageTable);
        soc_qemu_pc::mm::setup_higher_half(pml4);
    }
    let _ = writeln!(con, "[boot] higher-half mapping installed (0xFFFF_8000_0000_0000)");

    // ── VIRTIO device probing ────────────────────────────────
    unsafe {
        let pci = &*PCI_DEVICES.0.get();
        let fa = &mut *FRAME_ALLOC.0.get();
        let reg = &mut *DRIVERS.0.get();

        for i in 0..pci.count {
            let d = &pci.devices[i];
            if d.vendor_id != soc_qemu_pc::virtio::VIRTIO_VENDOR {
                continue;
            }

            // Read BAR0 (I/O base for legacy VIRTIO).
            let bar0 = soc_qemu_pc::pci::read_bar(d.bus, d.device, d.function, 0);
            if bar0 & 1 == 0 { continue; } // not I/O BAR
            let io_base = (bar0 & 0xFFFF_FFFC) as u16;

            match d.device_id {
                soc_qemu_pc::virtio::VIRTIO_DEV_BLK => {
                    let blk = &mut *VIRTIO_BLK.0.get();
                    if blk.init(io_base, fa) {
                        let cap_mb = blk.capacity_bytes() / (1024 * 1024);
                        let _ = writeln!(con, "[boot] virtio-blk: {} MiB ({} sectors), io=0x{:x}",
                            cap_mb, blk.capacity_sectors(), io_base);
                        let _ = reg.register("virtio-blk", DriverCaps {
                            mmio_regions: 0,
                            uses_interrupts: true,
                            uses_dma: true,
                            uses_network: false,
                        });
                    } else {
                        let _ = writeln!(con, "[boot] virtio-blk: init failed (io=0x{:x})", io_base);
                    }
                }
                soc_qemu_pc::virtio::VIRTIO_DEV_NET => {
                    let net = &mut *VIRTIO_NET.0.get();
                    if net.init(io_base, fa) {
                        let mut mac_buf = [0u8; 18];
                        let mac_len = net.mac_fmt(&mut mac_buf);
                        let mac_str = core::str::from_utf8(&mac_buf[..mac_len]).unwrap_or("??");
                        let _ = writeln!(con, "[boot] virtio-net: MAC={}, io=0x{:x}", mac_str, io_base);
                        let _ = reg.register("virtio-net", DriverCaps {
                            mmio_regions: 0,
                            uses_interrupts: true,
                            uses_dma: true,
                            uses_network: true,
                        });
                    } else {
                        let _ = writeln!(con, "[boot] virtio-net: init failed (io=0x{:x})", io_base);
                    }
                }
                _ => {
                    let _ = writeln!(con, "[boot] virtio: unknown device 0x{:04x} (io=0x{:x})",
                        d.device_id, io_base);
                }
            }
        }
    }

    // ── SMP bring-up ─────────────────────────────────────────
    unsafe {
        let acpi = &*ACPI_INFO.0.get();
        if acpi.valid && acpi.cpu_count > 1 {
            // Collect AP APIC IDs.
            let mut ap_ids = [0u8; 16];
            let mut ap_count = 0usize;
            for i in 1..acpi.cpu_count {
                if acpi.cpus[i].enabled && ap_count < 16 {
                    ap_ids[ap_count] = acpi.cpus[i].apic_id;
                    ap_count += 1;
                }
            }
            let booted = soc_qemu_pc::smp::bring_up_aps(&ap_ids[..ap_count], acpi.bsp_apic_id);
            let total = soc_qemu_pc::smp::online_cpu_count();
            let _ = writeln!(con, "[boot] SMP: {}/{} APs booted ({} CPUs online)",
                booted, ap_count, total);
        } else {
            let _ = writeln!(con, "[boot] SMP: single-core mode");
        }
    }

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
            ramfs.create_with_content(inodes, etc_id, "hostname", b"veeros-qemu-pc\n");
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

        // Shell task.
        #[cfg(feature = "shell")]
        {
            let sb = SHELL_STACK.0.as_ptr() as usize;
            let st = sb + SHELL_STACK.0.len();
            if let Some(idx) = sched.create_task("shell", shell_task as *const () as usize, st, sb, 0, 0) {
                sched.tasks[idx].context.set_status(INITIAL_RFLAGS);
            }
        }

        // Idle task.
        let sb = IDLE_STACK.0.as_ptr() as usize;
        let st = sb + IDLE_STACK.0.len();
        if let Some(idx) = sched.create_task("idle", idle_task as *const () as usize, st, sb, 0, 0) {
            sched.tasks[idx].context.set_status(INITIAL_RFLAGS);
        }

        // Userlib sample tasks.
        #[cfg(feature = "samples")]
        {
            let sb = HELLO_STACK.0.as_ptr() as usize;
            let st = sb + HELLO_STACK.0.len();
            if let Some(idx) = sched.create_task("hello", samples::hello_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_RFLAGS);
            }

            let sb = TIMER_STACK.0.as_ptr() as usize;
            let st = sb + TIMER_STACK.0.len();
            if let Some(idx) = sched.create_task("timer", samples::timer_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_RFLAGS);
            }

            let sb = IPC_TX_STACK.0.as_ptr() as usize;
            let st = sb + IPC_TX_STACK.0.len();
            if let Some(idx) = sched.create_task("ipc-tx", samples::ipc_sender_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_RFLAGS);
            }

            let sb = IPC_RX_STACK.0.as_ptr() as usize;
            let st = sb + IPC_RX_STACK.0.len();
            if let Some(idx) = sched.create_task("ipc-rx", samples::ipc_receiver_task as *const () as usize, st, sb, 2, 0) {
                sched.tasks[idx].context.set_status(INITIAL_RFLAGS);
            }
        }
    }

    // Set init process thread count.
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
    #[cfg(feature = "samples")]
    let _ = writeln!(con, "[boot] userlib sample tasks registered (hello, timer, ipc-tx, ipc-rx)");

    // ── start the scheduler (never returns) ──────────────────
    let _ = writeln!(con, "[boot] starting scheduler — preemptive mode");
    let _ = writeln!(con, "");

    // Initialise _veer_kernel_rsp to the first runnable task's stack top
    // so that SYSCALL entry works immediately on boot before any context switch.
    #[cfg(feature = "shell")]
    {
        let sb = SHELL_STACK.0.as_ptr() as usize;
        let st = sb + SHELL_STACK.0.len();
        trap::set_kernel_stack(st as u64);
    }
    // Disable NMI via the legacy PC NMI control register (port 0x70 bit 7).
    // This prevents spurious NMIs from the q35 chipset before we can handle them.
    #[cfg(target_arch = "x86_64")]
    unsafe { soc_qemu_pc::outb(0x70, 0x80); }
    let _ = writeln!(con, "[boot] NMI disabled");

    // Enable interrupts and start the first task.
    // On x86-64 we don't have the RISC-V `_veer_start_first_task` trick —
    // instead we enable interrupts and jump directly to the first task.
    unsafe {
        let sched = &mut *SCHEDULER.0.get();
        let ctx_ptr = sched.start().expect("no runnable task");

        #[cfg(target_arch = "x86_64")]
        {
            // Build an iretq frame on the stack and jump to the first task.
            let ctx = &*ctx_ptr;
            _veer_start_first_task_x86(ctx);
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            // Host build fallback.
            let _ = ctx_ptr;
            drop(con);
            #[cfg(feature = "shell")]
            shell_task();
            #[cfg(not(feature = "shell"))]
            idle_task();
        }
    }
}

/// Start the first task by constructing an iretq frame and executing iretq.
/// This never returns.
///
/// All initial tasks run at Ring 0. For a same-privilege iretq (Ring 0 → Ring 0),
/// the CPU pops only RIP, CS, RFLAGS — it does NOT pop RSP or SS. We therefore
/// switch to the task's stack manually before building the 3-word iretq frame.
#[cfg(target_arch = "x86_64")]
unsafe fn _veer_start_first_task_x86(ctx: &arch::TaskContext) -> ! {
    let rip = ctx.rip as u64;
    let task_rsp = ctx.get_sp() as u64;

    unsafe {
        core::arch::asm!(
            // Switch to the task's stack, enable interrupts, jump to entry.
            // No iretq needed since all tasks run at Ring 0 (same privilege).
            "mov rsp, {rsp}",
            "sti",
            "jmp {entry}",
            rsp   = in(reg) task_rsp,
            entry = in(reg) rip,
            options(noreturn),
        );
    }
}

// ---------------------------------------------------------------------------
// Console I/O callbacks (used by the syscall dispatcher)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub(crate) fn console_write_byte(b: u8) {
    let serial = default_serial();
    serial.write_byte(b);
}

#[allow(dead_code)]
pub(crate) fn console_read_byte() -> u8 {
    // Try the IRQ-driven keyboard ring buffer first.
    if let Some(b) = kbd_buffer_pop() {
        return b;
    }
    // Fall back to polling COM1 serial.
    let serial = default_serial();
    if serial.has_data() {
        return serial.read_byte();
    }
    0xFF
}

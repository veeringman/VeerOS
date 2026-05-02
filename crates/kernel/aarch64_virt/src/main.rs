//! VeerOS kernel for a generic AArch64 virtual machine.
//!
//! This target is meant for desktop/server-class virtual hosts such as Apple
//! Silicon Hypervisor.framework.  Provides a VSC (VeerOS Secure Connect)
//! remote shell on port 2323, accessible via `veer-connect shell <ip> 2323`.

#![no_std]
#![no_main]

use core::cell::UnsafeCell;
use core::fmt::Write;

use arch::{BlockDevice, Console, Platform, Serial, TaskContext};
use panic_halt as _;
use soc_aarch64_virt::{default_serial, system_timer, Aarch64Virt};

#[cfg(feature = "net")]
use arch::NetworkDevice;
#[cfg(feature = "net")]
use net::{NetStack, NetStorage, TcpSerial};
#[cfg(all(feature = "net", feature = "shell"))]
use shell::{Shell, ShellEnv};
#[cfg(feature = "net")]
use smoltcp::iface::{SocketHandle, SocketSet};
#[cfg(feature = "net")]
use smoltcp::socket::tcp::{Socket as TcpSocket, SocketBuffer};
#[cfg(feature = "net")]
use smoltcp::wire::{IpCidr, Ipv4Address};

#[cfg(feature = "shell")]
use microkernel::{
    fat32::Fat32,
    ramfs::RamFs,
    vfs::{InodeTable, MountTable},
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(feature = "shell")]
const KERNEL_STACK_KIB: usize = 16 * 1024;

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(
    r#"
.section .text._start
.global _start
.balign 4

_start:
    mov x19, x0

    mrs x0, mpidr_el1
    and x0, x0, #0xff
    cbnz x0, _park

    mrs x0, CurrentEL
    lsr x0, x0, #2
    cmp x0, #2
    b.ne _at_el1

    mov x0, #(1 << 31)
    msr hcr_el2, x0
    mov x0, #3
    msr cnthctl_el2, x0
    msr cntvoff_el2, xzr
    mov x0, xzr
    msr sctlr_el1, x0
    adr x0, _at_el1
    msr elr_el2, x0
    mov x0, #0x3c5
    msr spsr_el2, x0
    eret

_at_el1:
    ldr x0, =__stack_top
    mov sp, x0

    ldr x0, =__bss_start
    ldr x1, =__bss_end
1:
    cmp x0, x1
    b.ge 2f
    str xzr, [x0], #8
    b 1b
2:
    ldr x0, =_veer_vectors
    msr vbar_el1, x0

    ldr x0, =DTB_PTR
    str x19, [x0]

    bl _rust_start

_park:
    wfe
    b _park
"#
);

#[unsafe(no_mangle)]
static mut DTB_PTR: usize = 0;

// ── Networking globals ───────────────────────────────────────────────────────

/// VSC port — must match the port used by `veer-connect shell`.
#[cfg(feature = "net")]
const VSC_PORT: u16 = 2323;

/// SSH port for the built-in SSH-2 server.
#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
const SSH_PORT: u16 = 22;

#[cfg(feature = "net")]
const VSC_SESSION_COUNT: usize = 2;
#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
const SSH_SESSION_COUNT: usize = 2;
#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
const SSH_SESSION_BUF_SIZE: usize = 8192;

/// vmnet shared bridge100: guest 192.168.2.100, gateway 192.168.2.1.
#[cfg(feature = "net")]
const GUEST_IP: [u8; 4] = [192, 168, 2, 100];
#[cfg(feature = "net")]
const GATEWAY_IP: [u8; 4] = [192, 168, 2, 1];

/// Fixed seed for ephemeral X25519 server keypair.
/// The client always generates a fresh ephemeral key, so per-session
/// confidentiality is preserved even with a static server seed.
#[cfg(feature = "net")]
const VSC_SEED: [u8; 32] = [
    0x56, 0x65, 0x65, 0x72, 0x4f, 0x53, 0x61, 0x36, 0x34, 0x2d, 0x56, 0x53, 0x43, 0x2d, 0x73, 0x65,
    0x65, 0x64, 0x2d, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x30, 0x31, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Ed25519 SSH host key seed.  Fixed for this development target so the
/// host key remains stable across test boots.
#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
const SSH_HOST_SEED: [u8; 32] = [
    0x56, 0x65, 0x65, 0x72, 0x4f, 0x53, 0x2d, 0x48, 0x6f, 0x73, 0x74, 0x4b, 0x65, 0x79, 0x53, 0x65,
    0x65, 0x64, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x41, 0x42, 0x43, 0x44,
];

#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
const SSH_HOST_PUBKEY: [u8; 32] = [
    0xe2, 0x91, 0x74, 0x12, 0xbb, 0x3a, 0x6f, 0x7e, 0x80, 0x07, 0x28, 0x7f, 0xc4, 0x27, 0x01, 0x65,
    0xcf, 0x5d, 0x05, 0x61, 0x63, 0xf0, 0x82, 0x4c, 0xda, 0x73, 0xdc, 0x70, 0x8d, 0x99, 0x94, 0x3b,
];

#[cfg(feature = "net")]
struct NetCell(UnsafeCell<Option<NetStack<soc_aarch64_virt::virtio_net::VirtioMmioNet>>>);
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
static mut SOCKET_STORAGE: [smoltcp::iface::SocketStorage<'static>; 4] =
    [smoltcp::iface::SocketStorage::EMPTY; 4];
#[cfg(feature = "net")]
static mut NET_STORAGE: NetStorage = NetStorage::new();

#[cfg(feature = "net")]
static mut VSC_EXTRA_STORAGE: [NetStorage; VSC_SESSION_COUNT - 1] = [NetStorage::new(); 1];
#[cfg(feature = "net")]
static mut VSC_HANDLES: [Option<SocketHandle>; VSC_SESSION_COUNT] = [None; VSC_SESSION_COUNT];
#[cfg(feature = "net")]
static mut VSC_ACTIVE: [bool; VSC_SESSION_COUNT] = [false; VSC_SESSION_COUNT];

#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
static mut SSH_HANDLES: [Option<SocketHandle>; SSH_SESSION_COUNT] = [None; SSH_SESSION_COUNT];
#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
static mut SSH_ACTIVE: [bool; SSH_SESSION_COUNT] = [false; SSH_SESSION_COUNT];
#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
static mut SSH_RX_BUFS: [[u8; SSH_SESSION_BUF_SIZE]; SSH_SESSION_COUNT] =
    [[0u8; SSH_SESSION_BUF_SIZE]; SSH_SESSION_COUNT];
#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
static mut SSH_TX_BUFS: [[u8; SSH_SESSION_BUF_SIZE]; SSH_SESSION_COUNT] =
    [[0u8; SSH_SESSION_BUF_SIZE]; SSH_SESSION_COUNT];

#[cfg(feature = "shell")]
struct InodeCell(UnsafeCell<InodeTable>);
#[cfg(feature = "shell")]
unsafe impl Sync for InodeCell {}
#[cfg(feature = "shell")]
static INODES: InodeCell = InodeCell(UnsafeCell::new(InodeTable::new()));

#[cfg(feature = "shell")]
struct RamFsCell(UnsafeCell<RamFs>);
#[cfg(feature = "shell")]
unsafe impl Sync for RamFsCell {}
#[cfg(feature = "shell")]
static RAMFS: RamFsCell = RamFsCell(UnsafeCell::new(RamFs::new()));

#[cfg(feature = "shell")]
struct Fat32Cell(UnsafeCell<[Fat32; microkernel::fat32::MAX_FAT32]>);
#[cfg(feature = "shell")]
unsafe impl Sync for Fat32Cell {}
#[cfg(feature = "shell")]
static FAT32: Fat32Cell = Fat32Cell(UnsafeCell::new([
    Fat32::new(),
    Fat32::new(),
    Fat32::new(),
    Fat32::new(),
]));

#[cfg(feature = "shell")]
struct BlkCell(UnsafeCell<Option<soc_aarch64_virt::virtio_blk::VirtioMmioBlk>>);
#[cfg(feature = "shell")]
unsafe impl Sync for BlkCell {}
#[cfg(feature = "shell")]
static BLK0: BlkCell = BlkCell(UnsafeCell::new(None));

#[cfg(feature = "shell")]
struct MountCell(UnsafeCell<MountTable>);
#[cfg(feature = "shell")]
unsafe impl Sync for MountCell {}
#[cfg(feature = "shell")]
static MOUNTS: MountCell = MountCell(UnsafeCell::new(MountTable::new()));

#[cfg(feature = "shell")]
struct CwdCell(UnsafeCell<u16>);
#[cfg(feature = "shell")]
unsafe impl Sync for CwdCell {}
#[cfg(feature = "shell")]
static CWD: CwdCell = CwdCell(UnsafeCell::new(microkernel::vfs::ROOT_INODE));

/// Current time in milliseconds, derived from the ARM generic timer.
#[cfg(feature = "net")]
fn now_ms() -> u64 {
    let t = system_timer();
    let freq = t.frequency();
    if freq == 0 {
        0
    } else {
        t.counter() * 1000 / freq
    }
}

/// Drive the smoltcp stack.  Called by `TcpSerial` during blocking I/O.
#[cfg(feature = "net")]
fn net_poll() -> bool {
    unsafe {
        if let (Some(stack), Some(sockets)) = (&mut *NET.0.get(), &mut *NET_SOCKETS.0.get()) {
            stack.poll(sockets, now_ms());
        }
    }
    true
}

#[cfg(feature = "net")]
fn net_poll_unlock() {}

#[cfg(feature = "net")]
enum AcceptedSession {
    Vsc(usize, SocketHandle),
    #[cfg(all(feature = "ssh", feature = "shell"))]
    Ssh(usize, SocketHandle),
}

#[cfg(feature = "net")]
fn listen_if_idle(sockets: &mut SocketSet<'static>, handle: SocketHandle, active: bool, port: u16) {
    if active {
        return;
    }
    let socket = sockets.get_mut::<TcpSocket>(handle);
    if !socket.is_listening() {
        if socket.is_open() {
            socket.abort();
        }
        socket.listen(port).ok();
    }
}

#[cfg(feature = "net")]
fn listen_all_sessions() {
    unsafe {
        if let Some(sockets) = (*NET_SOCKETS.0.get()).as_mut() {
            for i in 0..VSC_SESSION_COUNT {
                if let Some(handle) = VSC_HANDLES[i] {
                    listen_if_idle(sockets, handle, VSC_ACTIVE[i], VSC_PORT);
                }
            }

            #[cfg(all(feature = "ssh", feature = "shell"))]
            for i in 0..SSH_SESSION_COUNT {
                if let Some(handle) = SSH_HANDLES[i] {
                    listen_if_idle(sockets, handle, SSH_ACTIVE[i], SSH_PORT);
                }
            }
        }
    }
}

#[cfg(feature = "net")]
fn accepted_session() -> Option<AcceptedSession> {
    unsafe {
        let sockets = (*NET_SOCKETS.0.get()).as_ref()?;
        for i in 0..VSC_SESSION_COUNT {
            if VSC_ACTIVE[i] {
                continue;
            }
            if let Some(handle) = VSC_HANDLES[i] {
                let socket = sockets.get::<TcpSocket>(handle);
                // Only dispatch when the TCP connection is fully established
                // (may_recv() is true once we are in ESTABLISHED / CLOSE_WAIT /
                // FIN_WAIT_*). This avoids handing the socket to the handshake
                // code while still in SYN-RECEIVED.
                if !socket.is_listening() && socket.may_recv() {
                    return Some(AcceptedSession::Vsc(i, handle));
                }
            }
        }

        #[cfg(all(feature = "ssh", feature = "shell"))]
        for i in 0..SSH_SESSION_COUNT {
            if SSH_ACTIVE[i] {
                continue;
            }
            if let Some(handle) = SSH_HANDLES[i] {
                let socket = sockets.get::<TcpSocket>(handle);
                if !socket.is_listening() && socket.may_recv() {
                    return Some(AcceptedSession::Ssh(i, handle));
                }
            }
        }
    }
    None
}

#[cfg(feature = "net")]
fn release_session(handle: SocketHandle, port: u16) {
    unsafe {
        if let Some(sockets) = (*NET_SOCKETS.0.get()).as_mut() {
            let socket = sockets.get_mut::<TcpSocket>(handle);
            if socket.is_open() {
                socket.abort();
            }
            socket.listen(port).ok();
        }
    }
}

#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
fn ssh_verify_password(user: &[u8], pass: &[u8]) -> bool {
    user == b"root" && fnv1a(pass) == fnv1a(b"toor")
}

#[cfg(feature = "shell")]
fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c9dc5;
    for &byte in bytes {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x01000193);
    }
    hash
}

#[cfg(feature = "shell")]
fn shell_login(user: &str, pass: &[u8]) -> u32 {
    if user.as_bytes() == b"root" && fnv1a(pass) == fnv1a(b"toor") {
        1
    } else {
        0
    }
}

#[cfg(feature = "shell")]
fn current_root_user() -> (u16, &'static str) {
    (0, "root")
}

#[cfg(feature = "shell")]
fn write_task_list(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  PID  STATE     PRI  NAME");
    let _ = writeln!(w, "    0  running     0  kernel");
    let _ = writeln!(w, "    1  ready       1  net-listener");
    let _ = writeln!(w, "    2  ready       1  vsc-shell");
    let _ = writeln!(w, "    3  ready       1  ssh-shell");
}

#[cfg(feature = "shell")]
fn write_mem_info(w: &mut dyn core::fmt::Write) {
    unsafe {
        let ramfs = &*RAMFS.0.get();
        let _ = writeln!(w, "  RAM       : 512 MiB configured by launcher");
        let _ = writeln!(w, "  kernel stk: {} KiB", KERNEL_STACK_KIB);
        let _ = writeln!(
            w,
            "  ramfs     : {} KiB total, {} KiB used, {} KiB free",
            ramfs.capacity() / 1024,
            ramfs.used() / 1024,
            ramfs.free() / 1024
        );
    }
}

#[cfg(feature = "shell")]
fn blk0_read(lba: u64, buf: &mut [u8]) -> bool {
    unsafe {
        match (&*BLK0.0.get()).as_ref() {
            Some(blk) => blk.read_block(lba, buf),
            None => false,
        }
    }
}

#[cfg(feature = "shell")]
fn blk0_write(lba: u64, buf: &[u8]) -> bool {
    unsafe {
        match (&*BLK0.0.get()).as_ref() {
            Some(blk) => blk.write_block(lba, buf),
            None => false,
        }
    }
}

#[cfg(feature = "shell")]
fn write_driver_list(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  pl011-uart        console   ready");
    let _ = writeln!(w, "  arm-generic-timer timer     ready");
    let _ = writeln!(w, "  virtio-mmio-net   network   ready");
    let _ = writeln!(w, "  ramfs             fs        ready");
}

#[cfg(feature = "shell")]
fn ifconfig_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "vnet0: flags=UP,RUNNING mtu 1500");
    let _ = writeln!(
        w,
        "  inet {}.{}.{}.{}/24 gateway {}.{}.{}.{}",
        GUEST_IP[0],
        GUEST_IP[1],
        GUEST_IP[2],
        GUEST_IP[3],
        GATEWAY_IP[0],
        GATEWAY_IP[1],
        GATEWAY_IP[2],
        GATEWAY_IP[3]
    );
    let _ = writeln!(w, "  ports: vsc/tcp:{} ssh/tcp:{}", VSC_PORT, SSH_PORT);
}

#[cfg(feature = "shell")]
fn netstat_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "Proto Local Address          State");
    let _ = writeln!(w, "tcp   0.0.0.0:{:<5}       LISTEN", VSC_PORT);
    let _ = writeln!(w, "tcp   0.0.0.0:{:<5}       LISTEN", SSH_PORT);
}

#[cfg(feature = "shell")]
fn hw_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  board     : AArch64 virt");
    let _ = writeln!(w, "  hypervisor: Apple Silicon Hypervisor.framework");
    let _ = writeln!(w, "  uart      : PL011 @ 0x09000000");
    let _ = writeln!(w, "  net       : virtio-mmio @ 0x0A000000");
}

#[cfg(feature = "shell")]
fn dmesg_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "[boot] VeerOS AArch64 virtual target v{}", VERSION);
    let _ = writeln!(
        w,
        "[boot] VFS initialised: ramfs {} KiB",
        microkernel::ramfs::RAMFS_POOL_SIZE / 1024
    );
    let _ = writeln!(
        w,
        "[net] guest {}.{}.{}.{}/24",
        GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3]
    );
    let _ = writeln!(w, "[ssh] SSH server ready on port {}", SSH_PORT);
}

#[cfg(feature = "shell")]
fn init_vfs(console: &mut Console<soc_aarch64_virt::uart::Pl011>) {
    unsafe {
        let inodes = &mut *INODES.0.get();
        let ramfs = &mut *RAMFS.0.get();
        let mounts = &mut *MOUNTS.0.get();

        inodes.init_root();
        mounts.mount(
            microkernel::vfs::ROOT_INODE,
            microkernel::vfs::FsType::RamFs,
            "ram0",
        );
        *CWD.0.get() = microkernel::vfs::ROOT_INODE;

        let dev_id = inodes
            .resolve(microkernel::vfs::ROOT_INODE, "/dev")
            .unwrap_or(microkernel::vfs::NO_INODE);
        if dev_id != microkernel::vfs::NO_INODE {
            inodes.create_device_in(dev_id, "null", 0, 0);
            inodes.create_device_in(dev_id, "zero", 0, 1);
            inodes.create_device_in(dev_id, "console", 0, 2);
            inodes.create_device_in(dev_id, "random", 0, 3);
            inodes.create_device_in(dev_id, "vnet0", 2, 0);
        }

        let etc_id = inodes
            .resolve(microkernel::vfs::ROOT_INODE, "/etc")
            .unwrap_or(microkernel::vfs::NO_INODE);
        if etc_id != microkernel::vfs::NO_INODE {
            ramfs.create_with_content(inodes, etc_id, "hostname", b"veeros\n");
            ramfs.create_with_content(inodes, etc_id, "motd", b"Welcome to VeerOS AArch64 HVF.\n");
            ramfs.create_with_content(
                inodes,
                etc_id,
                "issue",
                b"VeerOS virtual machine login: root/toor\n",
            );
        }

        let tmp_id = inodes
            .resolve(microkernel::vfs::ROOT_INODE, "/tmp")
            .unwrap_or(microkernel::vfs::NO_INODE);
        if tmp_id != microkernel::vfs::NO_INODE {
            ramfs.create_with_content(
                inodes,
                tmp_id,
                "README",
                b"This is an in-memory VeerOS RamFS.\n",
            );
        }

        if let Some(blk) = soc_aarch64_virt::virtio_blk::VirtioMmioBlk::init() {
            let sectors = blk.block_count();
            *BLK0.0.get() = Some(blk);
            let disk_id = inodes
                .resolve(microkernel::vfs::ROOT_INODE, "/disk")
                .or_else(|| inodes.mkdir_in(microkernel::vfs::ROOT_INODE, "disk"))
                .unwrap_or(microkernel::vfs::NO_INODE);
            if disk_id != microkernel::vfs::NO_INODE {
                if let Some(mount_id) =
                    mounts.mount(disk_id, microkernel::vfs::FsType::Fat32, "vblk0")
                {
                    let fat = &mut (&mut *FAT32.0.get())[mount_id as usize - 1];
                    if fat.mount(blk0_read, blk0_write, 0, disk_id, inodes, mount_id) {
                        let _ = writeln!(
                            console,
                            "[boot] vblk0 FAT32 mounted at /disk ({} sectors)",
                            sectors
                        );
                    } else {
                        mounts.unmount(disk_id);
                        let _ = writeln!(
                            console,
                            "[boot] vblk0 present but FAT32 mount failed ({} sectors)",
                            sectors
                        );
                    }
                }
            }
        } else {
            let _ = writeln!(console, "[boot] no virtio-blk disk detected");
        }
    }

    let _ = writeln!(
        console,
        "[boot] VFS initialised (ramfs {} KiB)",
        microkernel::ramfs::RAMFS_POOL_SIZE / 1024
    );
}

#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
fn ssh_seed(slot: usize) -> [u8; 32] {
    let ticks = now_ms();
    let bytes = ticks.to_le_bytes();
    let mut seed = [0u8; 32];
    seed[..8].copy_from_slice(&bytes);
    seed[8..16].copy_from_slice(&bytes);
    seed[16..24].copy_from_slice(&bytes);
    seed[24..32].copy_from_slice(&bytes);
    seed[0] ^= slot as u8;
    seed
}

// ── Kernel entry point ───────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn _rust_start() -> ! {
    let platform = Aarch64Virt::new();
    platform.init_cpu();

    let serial = default_serial();
    let mut console = Console::new(serial);

    let _ = writeln!(console, "");
    let _ = writeln!(console, "VeerOS AArch64 virtual target v{}", VERSION);
    let _ = writeln!(console, "platform : {}", platform.name());
    let _ = writeln!(console, "uart     : PL011 @ 0x09000000");
    let _ = writeln!(console, "load     : 0x00080000");

    #[cfg(feature = "shell")]
    init_vfs(&mut console);

    run_net(console)
}

/// Networking accept loop (diverges).
///
/// When compiled without the `net` feature, parks the CPU.
#[cfg(feature = "net")]
fn run_net(mut console: Console<soc_aarch64_virt::uart::Pl011>) -> ! {
    let _ = writeln!(console, "[net] probing virtio-mmio net @ 0x0A000000...");

    let nic = match soc_aarch64_virt::virtio_net::VirtioMmioNet::init() {
        Some(n) => n,
        None => {
            let _ = writeln!(console, "[net] no virtio-net found; parking");
            loop {
                core::hint::spin_loop();
            }
        }
    };

    let mac = nic.mac_address();
    let _ = writeln!(
        console,
        "[net] MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

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
        let mut stack = NetStack::new(nic, ip, gw, &mut socket_set, storage);

        VSC_HANDLES[0] = Some(stack.tcp_handle());
        for i in 1..VSC_SESSION_COUNT {
            let extra = &mut (&mut *core::ptr::addr_of_mut!(VSC_EXTRA_STORAGE))[i - 1];
            VSC_HANDLES[i] = Some(stack.add_tcp_socket(&mut socket_set, extra));
        }

        #[cfg(all(feature = "ssh", feature = "shell"))]
        {
            for i in 0..SSH_SESSION_COUNT {
                let rx_buf =
                    SocketBuffer::new(&mut (&mut *core::ptr::addr_of_mut!(SSH_RX_BUFS))[i][..]);
                let tx_buf =
                    SocketBuffer::new(&mut (&mut *core::ptr::addr_of_mut!(SSH_TX_BUFS))[i][..]);
                let mut socket = TcpSocket::new(rx_buf, tx_buf);
                socket.set_nagle_enabled(false);
                SSH_HANDLES[i] = Some(socket_set.add(socket));
            }
        }

        *NET_SOCKETS.0.get() = Some(socket_set);
        *NET.0.get() = Some(stack);
    }

    let _ = writeln!(
        console,
        "[net] {}.{}.{}.{}/24 — VSC port {}",
        GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3], VSC_PORT
    );
    #[cfg(all(feature = "ssh", feature = "shell"))]
    let _ = writeln!(console, "[ssh] SSH server ready on port {}", SSH_PORT);

    loop {
        listen_all_sessions();

        // Poll until a client connects.
        let accepted = loop {
            net_poll();
            if let Some(session) = accepted_session() {
                break session;
            }
            core::hint::spin_loop();
        };

        match accepted {
            AcceptedSession::Vsc(slot, handle) => {
                unsafe {
                    VSC_ACTIVE[slot] = true;
                }
                serve_vsc_session(slot, handle, &mut console);
                release_session(handle, VSC_PORT);
                unsafe {
                    VSC_ACTIVE[slot] = false;
                }
            }
            #[cfg(all(feature = "ssh", feature = "shell"))]
            AcceptedSession::Ssh(slot, handle) => {
                unsafe {
                    SSH_ACTIVE[slot] = true;
                }
                serve_ssh_session(slot, handle, &mut console);
                release_session(handle, SSH_PORT);
                unsafe {
                    SSH_ACTIVE[slot] = false;
                }
            }
        }
    }
}

#[cfg(feature = "net")]
fn serve_vsc_session(
    slot: usize,
    handle: SocketHandle,
    console: &mut Console<soc_aarch64_virt::uart::Pl011>,
) {
    let _ = writeln!(console, "[vsc:{}] client connected", slot);

    unsafe {
        let socket_set_ptr = (*NET_SOCKETS.0.get()).as_mut().unwrap() as *mut SocketSet<'static>;
        let tcp_serial = TcpSerial::new(handle, socket_set_ptr, net_poll, net_poll_unlock);

        match net::secure::server_handshake(&tcp_serial, VSC_SEED) {
            None => {
                let _ = writeln!(console, "[vsc:{}] handshake failed", slot);
            }
            Some(mut channel) => {
                let _ = writeln!(console, "[vsc:{}] session established", slot);

                // Block until the client sends its mode byte. read_mode will
                // return None if the underlying TCP connection closes before
                // a valid frame arrives (read_byte returns 0x04 on EOF).
                match net::secure::read_mode(&tcp_serial, &mut channel) {
                    Some(net::secure::MODE_SHELL) => {
                        serve_shell(tcp_serial, channel, console, false);
                    }
                    _ => {
                        let _ = writeln!(console, "[vsc:{}] unsupported mode; disconnecting", slot);
                    }
                }
            }
        }
    }
}

#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
fn serve_ssh_session(
    slot: usize,
    handle: SocketHandle,
    console: &mut Console<soc_aarch64_virt::uart::Pl011>,
) {
    let _ = writeln!(console, "[ssh:{}] client connected", slot);

    unsafe {
        let socket_set_ptr = (*NET_SOCKETS.0.get()).as_mut().unwrap() as *mut SocketSet<'static>;
        let tcp_serial = TcpSerial::new(handle, socket_set_ptr, net_poll, net_poll_unlock);
        let config = ssh::server::SshServerConfig {
            host_seed: SSH_HOST_SEED,
            host_pubkey: SSH_HOST_PUBKEY,
            password_verify: ssh_verify_password,
        };
        let seed = ssh_seed(slot);
        let mut rng = crypto::rng::ChaChaRng::from_seed(seed);

        match ssh::server::run_ssh_handshake_with_trace(&tcp_serial, &config, &mut rng, |stage| {
            let _ = writeln!(console, "[ssh:{}] {}", slot, stage);
        }) {
            Some(mut bridge) => {
                let _ = writeln!(console, "[ssh:{}] handshake succeeded", slot);
                let bridge_ptr = &mut bridge as *mut ssh::server::SshShellBridge;
                let ssh_serial = SshSerial {
                    bridge: bridge_ptr,
                    tcp: &tcp_serial,
                };
                let mut ssh_con = Console::new(ssh_serial);
                let mut sh = Shell::new(shell_env("AArch64 virtual (HVF) [SSH]", true));
                if let Some(cmd_bytes) = bridge.exec_command() {
                    if let Ok(cmd) = core::str::from_utf8(cmd_bytes) {
                        sh.run_command(&mut ssh_con, cmd);
                    }
                } else {
                    sh.run(&mut ssh_con);
                }
                let br = &mut *bridge_ptr;
                br.close_channel(&tcp_serial);
                let _ = writeln!(console, "[ssh:{}] session ended", slot);
            }
            None => {
                let _ = writeln!(console, "[ssh:{}] handshake failed", slot);
            }
        }
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_list_dir(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = *CWD.0.get();
        let dir_id = if path.trim().is_empty() || path == "." {
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

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_read_file(path: &str, buf: &mut [u8]) -> usize {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let ramfs = &*RAMFS.0.get();
        let fat32 = &mut *FAT32.0.get();
        let mounts = &*MOUNTS.0.get();
        let cwd = *CWD.0.get();
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE || inodes.inodes[id as usize].kind != InodeKind::File {
            return 0;
        }
        let mount_id = inodes.inodes[id as usize].dev_major;
        if let Some(mount) = mounts.get(mount_id) {
            if mount.fs_type == microkernel::vfs::FsType::Fat32 {
                return fat32[mount_id as usize - 1].read(inodes, id, 0, buf);
            }
        }
        ramfs.read(inodes, id, 0, buf)
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_write_file(path: &str, data: &[u8], append: bool) -> bool {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &mut *INODES.0.get();
        let ramfs = &mut *RAMFS.0.get();
        let fat32 = &mut *FAT32.0.get();
        let mounts = &*MOUNTS.0.get();
        let cwd = *CWD.0.get();
        let mut id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE {
            if let Some(slash) = path.rfind('/') {
                let parent_path = if slash == 0 { "/" } else { &path[..slash] };
                let name = &path[slash + 1..];
                let parent = inodes.resolve(cwd, parent_path).unwrap_or(NO_INODE);
                if parent == NO_INODE || name.is_empty() {
                    return false;
                }
                let parent_mount = inodes.inodes[parent as usize].dev_major;
                id = if let Some(mount) = mounts.get(parent_mount) {
                    if mount.fs_type == microkernel::vfs::FsType::Fat32 {
                        match fat32[parent_mount as usize - 1].create_file(
                            inodes,
                            parent,
                            name,
                            parent_mount,
                        ) {
                            Some(new_id) => new_id,
                            None => return false,
                        }
                    } else {
                        match inodes.create_file_in(parent, name) {
                            Some(new_id) => new_id,
                            None => return false,
                        }
                    }
                } else {
                    match inodes.create_file_in(parent, name) {
                        Some(new_id) => new_id,
                        None => return false,
                    }
                };
            } else {
                let parent_mount = inodes.inodes[cwd as usize].dev_major;
                id = if let Some(mount) = mounts.get(parent_mount) {
                    if mount.fs_type == microkernel::vfs::FsType::Fat32 {
                        match fat32[parent_mount as usize - 1].create_file(
                            inodes,
                            cwd,
                            path,
                            parent_mount,
                        ) {
                            Some(new_id) => new_id,
                            None => return false,
                        }
                    } else {
                        match inodes.create_file_in(cwd, path) {
                            Some(new_id) => new_id,
                            None => return false,
                        }
                    }
                } else {
                    match inodes.create_file_in(cwd, path) {
                        Some(new_id) => new_id,
                        None => return false,
                    }
                };
            }
        }
        if inodes.inodes[id as usize].kind != InodeKind::File {
            return false;
        }
        let mount_id = inodes.inodes[id as usize].dev_major;
        if let Some(mount) = mounts.get(mount_id) {
            if mount.fs_type == microkernel::vfs::FsType::Fat32 {
                let offset = if append {
                    inodes.inodes[id as usize].size
                } else {
                    0
                };
                if !append {
                    fat32[mount_id as usize - 1].truncate(inodes, id);
                }
                return fat32[mount_id as usize - 1].write(inodes, id, offset, data) > 0;
            }
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

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_mkdir(path: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = *CWD.0.get();
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

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_stat(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = *CWD.0.get();
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

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_unlink(path: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let fat32 = &mut *FAT32.0.get();
        let mounts = &*MOUNTS.0.get();
        let cwd = *CWD.0.get();
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE {
            return false;
        }
        let mount_id = inodes.inodes[id as usize].dev_major;
        if let Some(mount) = mounts.get(mount_id) {
            if mount.fs_type == microkernel::vfs::FsType::Fat32 {
                return fat32[mount_id as usize - 1].unlink_file(inodes, id);
            }
        }
        inodes.unlink(id)
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_rename(old: &str, new: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let cwd = *CWD.0.get();
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

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_getcwd(buf: &mut [u8]) -> usize {
    unsafe {
        let inodes = &*INODES.0.get();
        inodes.build_path(*CWD.0.get(), buf)
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_chdir(path: &str) -> bool {
    use microkernel::vfs::{InodeKind, NO_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = *CWD.0.get();
        let id = inodes.resolve(cwd, path).unwrap_or(NO_INODE);
        if id == NO_INODE || inodes.inodes[id as usize].kind != InodeKind::Directory {
            return false;
        }
        *CWD.0.get() = id;
        true
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_tree(path: &str, w: &mut dyn core::fmt::Write) {
    use microkernel::vfs::{InodeKind, NO_INODE, ROOT_INODE};
    unsafe {
        let inodes = &*INODES.0.get();
        let cwd = *CWD.0.get();
        let start = if path.trim().is_empty() || path == "." {
            cwd
        } else if path == "/" {
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
            if path.trim().is_empty() || path == "." {
                "."
            } else {
                path
            }
        );
        let mut stack: [(u16, u8); 64] = [(NO_INODE, 0); 64];
        let mut sp = 0usize;
        let mut child = root.children_head;
        while child != NO_INODE && sp < stack.len() {
            stack[sp] = (child, 1);
            sp += 1;
            child = inodes.inodes[child as usize].next_sibling;
        }
        while sp > 0 {
            sp -= 1;
            let (id, depth) = stack[sp];
            let node = &inodes.inodes[id as usize];
            for _ in 0..depth {
                w.write_str("  ").ok();
            }
            let suffix = if node.kind == InodeKind::Directory {
                "/"
            } else {
                ""
            };
            let _ = writeln!(w, "{}{}", node.name_str(), suffix);
            if node.kind == InodeKind::Directory {
                let mut next = node.children_head;
                while next != NO_INODE && sp < stack.len() {
                    stack[sp] = (next, depth + 1);
                    sp += 1;
                    next = inodes.inodes[next as usize].next_sibling;
                }
            }
        }
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn vfs_touch(path: &str) -> bool {
    use microkernel::vfs::NO_INODE;
    unsafe {
        let inodes = &mut *INODES.0.get();
        let fat32 = &mut *FAT32.0.get();
        let mounts = &*MOUNTS.0.get();
        let cwd = *CWD.0.get();
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
            let parent_mount = inodes.inodes[parent as usize].dev_major;
            if let Some(mount) = mounts.get(parent_mount) {
                if mount.fs_type == microkernel::vfs::FsType::Fat32 {
                    return fat32[parent_mount as usize - 1]
                        .create_file(inodes, parent, name, parent_mount)
                        .is_some();
                }
            }
            inodes.create_file_in(parent, name).is_some()
        } else {
            let parent_mount = inodes.inodes[cwd as usize].dev_major;
            if let Some(mount) = mounts.get(parent_mount) {
                if mount.fs_type == microkernel::vfs::FsType::Fat32 {
                    return fat32[parent_mount as usize - 1]
                        .create_file(inodes, cwd, path, parent_mount)
                        .is_some();
                }
            }
            inodes.create_file_in(cwd, path).is_some()
        }
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn mount_fs(dev: &str, path: &str) -> bool {
    if dev != "vblk0" {
        return false;
    }
    unsafe {
        if (&*BLK0.0.get()).is_none() {
            return false;
        }
        let inodes = &mut *INODES.0.get();
        let mounts = &mut *MOUNTS.0.get();
        let fat32 = &mut *FAT32.0.get();
        let cwd = *CWD.0.get();
        let dir = inodes
            .resolve(cwd, path)
            .unwrap_or(microkernel::vfs::NO_INODE);
        if dir == microkernel::vfs::NO_INODE {
            return false;
        }
        for m in mounts.mounts.iter() {
            if m.active && m.dir_inode == dir {
                return true;
            }
        }
        let Some(mount_id) = mounts.mount(dir, microkernel::vfs::FsType::Fat32, "vblk0") else {
            return false;
        };
        if fat32[mount_id as usize - 1].mount(blk0_read, blk0_write, 0, dir, inodes, mount_id) {
            true
        } else {
            mounts.unmount(dir);
            false
        }
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn umount_fs(path: &str) -> bool {
    unsafe {
        let inodes = &*INODES.0.get();
        let mounts = &mut *MOUNTS.0.get();
        let cwd = *CWD.0.get();
        let dir = inodes
            .resolve(cwd, path)
            .unwrap_or(microkernel::vfs::NO_INODE);
        if dir == microkernel::vfs::NO_INODE {
            return false;
        }
        mounts.unmount(dir)
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn mount_list(w: &mut dyn core::fmt::Write) {
    unsafe {
        let mounts = &*MOUNTS.0.get();
        let inodes = &*INODES.0.get();
        let mut found = false;
        for (i, mount) in mounts.mounts.iter().enumerate() {
            if mount.active {
                let mut pathbuf = [0u8; 64];
                let plen = inodes.build_path(mount.dir_inode, &mut pathbuf);
                let path = core::str::from_utf8(&pathbuf[..plen]).unwrap_or("?");
                let fstype = match mount.fs_type {
                    microkernel::vfs::FsType::RamFs => "ramfs",
                    microkernel::vfs::FsType::Fat32 => "fat32",
                    _ => "none",
                };
                let _ = writeln!(
                    w,
                    "  {} on {} type {} (slot {})",
                    mount.label_str(),
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

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn lsblk_info(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  NAME       TYPE   SIZE");
    let _ = writeln!(
        w,
        "  ram0       ramfs  {} KiB",
        microkernel::ramfs::RAMFS_POOL_SIZE / 1024
    );
    unsafe {
        if let Some(blk) = (&*BLK0.0.get()).as_ref() {
            let _ = writeln!(w, "  vblk0      disk   {} KiB", blk.block_count() / 2);
        } else {
            let _ = writeln!(w, "  vblk0      disk   not attached");
        }
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn df_info(w: &mut dyn core::fmt::Write) {
    unsafe {
        let ramfs = &*RAMFS.0.get();
        let _ = writeln!(w, "Filesystem  Type   Size   Used   Avail  Mounted on");
        let _ = writeln!(
            w,
            "ram0        ramfs  {:5}K {:5}K {:5}K /",
            ramfs.capacity() / 1024,
            ramfs.used() / 1024,
            ramfs.free() / 1024
        );
        if let Some(blk) = (&*BLK0.0.get()).as_ref() {
            let _ = writeln!(
                w,
                "vblk0       fat32  {:5}K      ?      ? /disk",
                blk.block_count() / 2
            );
        }
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn shell_env(platform: &'static str, pre_authenticated: bool) -> ShellEnv {
    ShellEnv {
        version: VERSION,
        platform,
        scheduler: "cooperative-vm",
        get_uptime_ticks: Some(now_ms),
        get_task_list: Some(write_task_list),
        get_mem_info: Some(write_mem_info),
        get_driver_list: Some(write_driver_list),
        wifi_cmd: None,
        bt_cmd: None,
        zigbee_cmd: None,
        sensor_cmd: None,
        get_current_user: Some(current_root_user),
        get_user_list: None,
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
        mount_fs: Some(mount_fs),
        umount_fs: Some(umount_fs),
        lsblk: Some(lsblk_info),
        df_cmd: Some(df_info),
        input_status: None,
        usb_list: None,
        ble_hid_list: None,
        gpio_cmd: None,
        i2c_cmd: None,
        spi_cmd: None,
        hw_info: Some(hw_info),
        get_temp_millic: None,
        dmesg: Some(dmesg_info),
        reboot: None,
        shutdown: None,
        caps_cmd: None,
        auditlog_cmd: None,
        ifconfig_cmd: Some(ifconfig_info),
        ping_cmd: None,
        netstat_cmd: Some(netstat_info),
        ssh_cmd: None,
        login: Some(shell_login),
        logout: None,
        change_password: None,
        add_user: None,
        remove_user: None,
        pre_authenticated,
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
        sleep_ms: None,
    }
}

#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
struct SshSerial<'a> {
    bridge: *mut ssh::server::SshShellBridge,
    tcp: &'a TcpSerial,
}

#[cfg(all(feature = "net", feature = "ssh", feature = "shell"))]
impl Serial for SshSerial<'_> {
    fn write_byte(&self, byte: u8) {
        let bridge = unsafe { &mut *self.bridge };
        bridge.write_byte_to(self.tcp, byte);
    }

    fn read_byte(&self) -> u8 {
        let bridge = unsafe { &mut *self.bridge };
        bridge.read_byte_from(self.tcp)
    }

    fn has_data(&self) -> bool {
        let bridge = unsafe { &*self.bridge };
        bridge.is_open()
    }
}

#[cfg(feature = "net")]
#[cfg(feature = "shell")]
fn serve_shell(
    tcp_serial: TcpSerial,
    channel: net::secure::SecureChannel,
    console: &mut Console<soc_aarch64_virt::uart::Pl011>,
    pre_authenticated: bool,
) {
    let _ = writeln!(console, "[vsc] shell session");
    let sec = net::secure::SecureSerial::new(tcp_serial, channel);
    let mut remote_con = Console::new(sec);
    let mut sh = Shell::new(shell_env("AArch64 virtual (HVF)", pre_authenticated));
    sh.run(&mut remote_con);
    let _ = writeln!(console, "[vsc] session ended");
}

#[cfg(feature = "net")]
#[cfg(not(feature = "shell"))]
fn serve_shell(
    _tcp_serial: TcpSerial,
    _channel: net::secure::SecureChannel,
    console: &mut Console<soc_aarch64_virt::uart::Pl011>,
) {
    let _ = writeln!(console, "[vsc] shell not compiled in");
}

#[cfg(not(feature = "net"))]
fn run_net(_console: Console<soc_aarch64_virt::uart::Pl011>) -> ! {
    loop {
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("wfe", options(nomem, nostack));
        }
        #[cfg(not(target_arch = "aarch64"))]
        core::hint::spin_loop();
    }
}

#[cfg(target_arch = "aarch64")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _veer_trap_dispatch(ctx: *mut TaskContext) -> *mut TaskContext {
    ctx
}

#[cfg(target_arch = "aarch64")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _veer_irq_dispatch(ctx: *mut TaskContext) -> *mut TaskContext {
    ctx
}

//! Software RV32IMC interpreter.
//!
//! A minimal but correct in-process virtualizer for 32-bit RISC-V machine-
//! mode guests. Exists so we can run the ESP32-C6 kernel (linked for
//! QEMU-virt MMIO) on x86_64 / AArch64 development hosts without pulling in
//! QEMU-system-riscv32 or requiring a RISC-V host for KVM.
//!
//! Scope:
//!   * RV32I + RV32M + RV32C.
//!   * Machine-mode only (no U/S/H, no MMU, no PMP).
//!   * One hart.
//!   * MMIO:
//!       - NS16550 UART    at 0x1000_0000 → host stdout / stdin.
//!       - CLINT           at 0x0200_0000 → monotonic mtime + mtimecmp timer.
//!       - Virtio-MMIO v2  at 0x1000_1000 → TAP-backed virtio-net (slot 0).
//!         Slots 1-7 (0x1000_2000..0x1000_9000) return magic=0 → guest probe
//!         skips them automatically.
//!   * Timer interrupt (MTI) delivered via standard mstatus/mie/mip gates.
//!   * WFI → short sleep so we don't burn host CPU.
//!
//! Out of scope (for now):
//!   * External IRQs / PLIC — not needed; kernel polls virtio used-ring.
//!   * Snapshot/restore, --disk.

use anyhow::{bail, Context, Result};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::elf;
use crate::config::{BootSource, VmConfig};
use crate::memory::GuestMem;
use crate::termios_guard::RawMode;
use crate::vm::{self, SHUTDOWN};

// ── MMIO map ────────────────────────────────────────────────────────────

const UART_BASE: u32 = 0x1000_0000;
const UART_END:  u32 = 0x1000_0100;
const UART_RBR_THR: u32 = 0x00;
const UART_IER:     u32 = 0x01;
const UART_IIR_FCR: u32 = 0x02;
const UART_LCR:     u32 = 0x03;
const UART_MCR:     u32 = 0x04;
const UART_LSR:     u32 = 0x05;
const UART_LSR_DR:   u8 = 1 << 0;
const UART_LSR_THRE: u8 = 1 << 5;
const UART_LSR_TEMT: u8 = 1 << 6;

const CLINT_BASE: u32 = 0x0200_0000;
const CLINT_END:  u32 = 0x0201_0000;
const CLINT_MSIP:     u32 = 0x0000;
const CLINT_MTIMECMP: u32 = 0x4000;
const CLINT_MTIME:    u32 = 0xBFF8;
// CLINT clock presented to the guest.
// The real ESP32-C6 / qemu-virt CLINT runs at 10 MHz, which makes
// the guest's 1 ms scheduler tick fire every 1 ms real time — accurate
// but the debug interpreter (~50 M inst/s) needs ~2 ms to process each
// tick, leaving almost no idle time → ~100% host CPU.
//
// At 500 KHz the kernel's fixed 10 000-tick period = 20 ms real time.
// Tick processing costs ~2 ms, WFI sleeps ~18 ms → ≈10% CPU (debug).
// In release builds (~500 M inst/s) tick processing costs ~0.2 ms →
// ≈1% CPU.  All guest logic (timers, DHCP, shell) still works correctly;
// the virtual node just runs ~20× slower than real hardware, which is
// imperceptible in a demo environment.
const CLINT_FREQ_HZ: u64 = 500_000;

// ── Virtio-MMIO ─────────────────────────────────────────────────────────

// Slot 0 = virtio-net.  Slots 1-7 return magic=0 → guest probe skips them.
const VMNET_BASE: u32 = 0x1000_1000;
const VMNET_END:  u32 = 0x1000_2000;
// All 8 probed slots together:
const VMSLOT_END: u32 = 0x1000_9000;

// Virtio-MMIO v2 register offsets
const VM_MAGIC:            u32 = 0x000;
const VM_VERSION:          u32 = 0x004;
const VM_DEVICE_ID:        u32 = 0x008;
const VM_VENDOR_ID:        u32 = 0x00C;
const VM_DEVICE_FEATURES:  u32 = 0x010;
const VM_DEVICE_FEAT_SEL:  u32 = 0x014;
const VM_DRIVER_FEATURES:  u32 = 0x020;
const VM_DRIVER_FEAT_SEL:  u32 = 0x024;
const VM_QUEUE_SEL:        u32 = 0x030;
const VM_QUEUE_NUM_MAX:    u32 = 0x034;
const VM_QUEUE_NUM:        u32 = 0x038;
const VM_QUEUE_READY:      u32 = 0x044;
const VM_QUEUE_NOTIFY:     u32 = 0x050;
const VM_INTR_STATUS:      u32 = 0x060;
const VM_INTR_ACK:         u32 = 0x064;
const VM_STATUS:           u32 = 0x070;
const VM_QUEUE_DESC_LO:    u32 = 0x080;
const VM_QUEUE_DESC_HI:    u32 = 0x084;
const VM_QUEUE_AVAIL_LO:   u32 = 0x090;
const VM_QUEUE_AVAIL_HI:   u32 = 0x094;
const VM_QUEUE_USED_LO:    u32 = 0x0A0;
const VM_QUEUE_USED_HI:    u32 = 0x0A4;
const VM_CONFIG_BASE:      u32 = 0x100; // net: MAC at +0..+6

const VIRTIO_MAGIC: u32 = 0x7472_6976; // "virt"
const VIRTIO_NET_F_MAC: u32 = 1 << 5;
const VIRTIO_F_VERSION_1: u32 = 1 << 0; // in feature word 1
const VIRTIO_NET_HDR_SIZE: usize = 10;

const VQ_SIZE: u32 = 16; // must match guest QUEUE_SIZE=16

// ── CSRs ────────────────────────────────────────────────────────────────

const CSR_MSTATUS:  u16 = 0x300;
const CSR_MISA:     u16 = 0x301;
const CSR_MIE:      u16 = 0x304;
const CSR_MTVEC:    u16 = 0x305;
const CSR_MSCRATCH: u16 = 0x340;
const CSR_MEPC:     u16 = 0x341;
const CSR_MCAUSE:   u16 = 0x342;
const CSR_MTVAL:    u16 = 0x343;
const CSR_MIP:      u16 = 0x344;
const CSR_MHARTID:  u16 = 0xF14;
const CSR_MVENDORID: u16 = 0xF11;
const CSR_MARCHID:  u16 = 0xF12;
const CSR_MIMPID:   u16 = 0xF13;
const CSR_CYCLE:    u16 = 0xC00;
const CSR_TIME:     u16 = 0xC01;
const CSR_INSTRET:  u16 = 0xC02;
const CSR_CYCLEH:   u16 = 0xC80;
const CSR_TIMEH:    u16 = 0xC81;
const CSR_INSTRETH: u16 = 0xC82;

const MSTATUS_MIE:  u32 = 1 << 3;
const MSTATUS_MPIE: u32 = 1 << 7;
const MSTATUS_MPP_MASK: u32 = 0b11 << 11;

const MIE_MSIE: u32 = 1 << 3;
const MIE_MTIE: u32 = 1 << 7;
const MIE_MEIE: u32 = 1 << 11;

const MCAUSE_INT: u32 = 1 << 31;
const IRQ_MSI: u32 = 3;
const IRQ_MTI: u32 = 7;
const IRQ_MEI: u32 = 11;

const EXC_INST_ADDR_MISALIGNED: u32 = 0;
const EXC_INST_ACCESS_FAULT:    u32 = 1;
const EXC_ILLEGAL_INST:         u32 = 2;
const EXC_BREAKPOINT:           u32 = 3;
const EXC_LOAD_ADDR_MISALIGNED: u32 = 4;
const EXC_LOAD_ACCESS_FAULT:    u32 = 5;
const EXC_STORE_ADDR_MISALIGNED:u32 = 6;
const EXC_STORE_ACCESS_FAULT:   u32 = 7;
const EXC_ECALL_M:              u32 = 11;

// ── CPU state ──────────────────────────────────────────────────────────

struct Cpu {
    x: [u32; 32],
    pc: u32,
    mstatus: u32,
    mtvec: u32,
    mepc: u32,
    mcause: u32,
    mtval: u32,
    mie: u32,
    mscratch: u32,
    // mip software/external bits held here; MTIP is synthesized from the
    // CLINT model each check.
    mip_software: bool,
    mip_external: bool,
    wfi: bool,
    retired: u64,
}

impl Cpu {
    fn new(entry: u32) -> Self {
        Self {
            x: [0; 32],
            pc: entry,
            mstatus: 0,
            mtvec: 0,
            mepc: 0,
            mcause: 0,
            mtval: 0,
            mie: 0,
            mscratch: 0,
            mip_software: false,
            mip_external: false,
            wfi: false,
            retired: 0,
        }
    }

    #[inline(always)]
    fn set_reg(&mut self, r: usize, v: u32) {
        if r != 0 {
            self.x[r] = v;
        }
    }

    #[inline(always)]
    fn reg(&self, r: usize) -> u32 {
        self.x[r]
    }
}

// ── CLINT ──────────────────────────────────────────────────────────────

struct Clint {
    start: Instant,
    mtimecmp: u64,
    msip: u32,
}

impl Clint {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            mtimecmp: u64::MAX,
            msip: 0,
        }
    }

    fn mtime(&self) -> u64 {
        let ns = self.start.elapsed().as_nanos() as u128;
        ((ns * CLINT_FREQ_HZ as u128) / 1_000_000_000u128) as u64
    }

    fn timer_pending(&self) -> bool {
        self.mtime() >= self.mtimecmp
    }
}

// ── Virtio-MMIO device state ────────────────────────────────────────────

struct VirtQueue {
    num: u32,
    ready: bool,
    desc_addr: u32,   // guest physical
    avail_addr: u32,  // guest physical
    used_addr: u32,   // guest physical
    last_avail_idx: u16,
}

impl VirtQueue {
    fn new() -> Self {
        Self {
            num: VQ_SIZE,
            ready: false,
            desc_addr: 0,
            avail_addr: 0,
            used_addr: 0,
            last_avail_idx: 0,
        }
    }
}

pub(crate) struct VirtioNetDev {
    status: u32,
    device_features_sel: u32,
    driver_features: [u32; 2],
    queue_sel: u32,
    pub(crate) interrupt_status: u32,
    mac: [u8; 6],
    queues: [VirtQueue; 2],
    /// Raw TAP fd; -1 if not configured.
    pub(crate) tap_fd: std::os::unix::io::RawFd,
}

impl VirtioNetDev {
    fn new(mac: [u8; 6], tap_fd: std::os::unix::io::RawFd) -> Self {
        Self {
            status: 0,
            device_features_sel: 0,
            driver_features: [0; 2],
            queue_sel: 0,
            interrupt_status: 0,
            mac,
            queues: [VirtQueue::new(), VirtQueue::new()],
            tap_fd,
        }
    }
}

impl Drop for VirtioNetDev {
    fn drop(&mut self) {
        if self.tap_fd >= 0 {
            unsafe { libc::close(self.tap_fd); }
        }
    }
}

// ── Virtio MMIO register handlers ──────────────────────────────────────

fn vmnet_read_u32(dev: &VirtioNetDev, off: u32) -> u32 {
    match off {
        VM_MAGIC           => VIRTIO_MAGIC,
        VM_VERSION         => 2,
        VM_DEVICE_ID       => 1, // net
        VM_VENDOR_ID       => 0x0000_FFFF,
        VM_DEVICE_FEATURES => match dev.device_features_sel {
            0 => VIRTIO_NET_F_MAC,
            1 => VIRTIO_F_VERSION_1,
            _ => 0,
        },
        VM_QUEUE_NUM_MAX  => VQ_SIZE,
        VM_QUEUE_READY    => dev.queues[dev.queue_sel as usize & 1].ready as u32,
        VM_INTR_STATUS    => dev.interrupt_status,
        VM_STATUS         => dev.status,
        _ => 0,
    }
}

fn vmnet_read_u8(dev: &VirtioNetDev, off: u32) -> u8 {
    // MAC bytes at config offset 0x100..0x106
    if off >= VM_CONFIG_BASE && off < VM_CONFIG_BASE + 6 {
        return dev.mac[(off - VM_CONFIG_BASE) as usize];
    }
    // All other registers align to 4: delegate to u32 read.
    let word_off = off & !3;
    let byte_lane = off & 3;
    ((vmnet_read_u32(dev, word_off) >> (byte_lane * 8)) & 0xFF) as u8
}

fn vmnet_write(dev: &mut VirtioNetDev, guest: &Arc<GuestMem>, off: u32, val: u32) {
    let sel = (dev.queue_sel & 1) as usize;
    match off {
        VM_DEVICE_FEAT_SEL => { dev.device_features_sel = val; }
        VM_DRIVER_FEATURES => { dev.driver_features[dev.driver_features[1] as usize & 1] = val; }
        // Note: We detect which word by a secondary sel register in a real device, but
        // the kernel writes DriverFeaturesSel then DriverFeatures in sequence, so we
        // can track the sel here:
        VM_DRIVER_FEAT_SEL => { dev.driver_features[1] = val; } // reuse [1] as sel
        VM_QUEUE_SEL       => { dev.queue_sel = val; }
        VM_QUEUE_NUM       => { dev.queues[sel].num = val.min(VQ_SIZE); }
        VM_QUEUE_READY     => {
            dev.queues[sel].ready = val != 0;
        }
        VM_QUEUE_DESC_LO   => { dev.queues[sel].desc_addr  = val; }
        VM_QUEUE_DESC_HI   => { /* ignore high 32 bits — guest RAM is <4 GiB */ }
        VM_QUEUE_AVAIL_LO  => { dev.queues[sel].avail_addr = val; }
        VM_QUEUE_AVAIL_HI  => {}
        VM_QUEUE_USED_LO   => { dev.queues[sel].used_addr  = val; }
        VM_QUEUE_USED_HI   => {}
        VM_QUEUE_NOTIFY    => {
            match val {
                0 => { /* RX notification — guest posted new RX buffers; TAP thread tracks independently */ }
                1 => { process_tx(dev, guest); }
                _ => {}
            }
        }
        VM_INTR_ACK => { dev.interrupt_status &= !val; }
        VM_STATUS => {
            if val == 0 {
                // Reset
                dev.status = 0;
                dev.queue_sel = 0;
                dev.interrupt_status = 0;
                dev.driver_features = [0; 2];
                for q in &mut dev.queues {
                    *q = VirtQueue::new();
                }
            } else {
                dev.status = val;
            }
        }
        _ => {}
    }
}

// ── TX queue processing ─────────────────────────────────────────────────

fn process_tx(dev: &mut VirtioNetDev, guest: &Arc<GuestMem>) {
    let tap_fd = dev.tap_fd;
    let q = &mut dev.queues[1]; // TX
    if !q.ready || q.avail_addr == 0 { return; }

    loop {
        let avail_idx = g_r16(guest, q.avail_addr.wrapping_add(2));
        if q.last_avail_idx == avail_idx { break; }

        let ring_slot = (q.last_avail_idx as u32 % q.num) as u32;
        let desc_id = g_r16(guest, q.avail_addr.wrapping_add(4).wrapping_add(ring_slot * 2)) as usize;

        // Read 16-byte split descriptor: addr(u64) | len(u32) | flags(u16) | next(u16)
        let d = q.desc_addr.wrapping_add(desc_id as u32 * 16);
        let buf_addr = g_r32(guest, d) as u64;     // low 32 bits of addr
        let buf_len = g_r32(guest, d.wrapping_add(8)) as usize;

        // Strip 10-byte virtio-net header, send raw Ethernet frame.
        if tap_fd >= 0 && buf_len > VIRTIO_NET_HDR_SIZE {
            let frame_gpa = buf_addr as u32 + VIRTIO_NET_HDR_SIZE as u32;
            let frame_len = buf_len - VIRTIO_NET_HDR_SIZE;
            if let Ok(frame) = guest.slice_mut(frame_gpa as u64, frame_len) {
                unsafe {
                    libc::write(tap_fd, frame.as_ptr() as *const libc::c_void, frame_len);
                }
            }
        }

        // Write used ring entry.
        let used_idx = g_r16(guest, q.used_addr.wrapping_add(2));
        let used_slot = (used_idx as u32 % q.num) as u32;
        g_w32(guest, q.used_addr.wrapping_add(4).wrapping_add(used_slot * 8), desc_id as u32);
        g_w32(guest, q.used_addr.wrapping_add(4).wrapping_add(used_slot * 8 + 4), buf_len as u32);
        g_w16(guest, q.used_addr.wrapping_add(2), used_idx.wrapping_add(1));

        q.last_avail_idx = q.last_avail_idx.wrapping_add(1);
        dev.interrupt_status |= 1;
    }
}

// ── Direct guest memory read/write helpers (no MMIO dispatch) ──────────

#[inline] fn g_r16(mem: &Arc<GuestMem>, gpa: u32) -> u16 { mem.read_u16(gpa as u64).unwrap_or(0) }
#[inline] fn g_r32(mem: &Arc<GuestMem>, gpa: u32) -> u32 { mem.read_u32(gpa as u64).unwrap_or(0) }
#[inline] fn g_w16(mem: &Arc<GuestMem>, gpa: u32, v: u16) { let _ = mem.write_u16(gpa as u64, v); }
#[inline] fn g_w32(mem: &Arc<GuestMem>, gpa: u32, v: u32) { let _ = mem.write_u32(gpa as u64, v); }

// ── Open TAP fd ────────────────────────────────────────────────────────

fn open_tap(ifname: &str) -> anyhow::Result<std::os::unix::io::RawFd> {
    use anyhow::{bail, Context};
    if ifname.len() >= 16 {
        bail!("TAP interface name '{ifname}' too long (max 15 bytes)");
    }
    let fd = unsafe {
        libc::open(
            b"/dev/net/tun\0".as_ptr() as *const libc::c_char,
            libc::O_RDWR,
        )
    };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        bail!("open /dev/net/tun: {e}");
    }
    #[repr(C)]
    struct IfReq { name: [u8; 16], flags: u16, _pad: [u8; 22] }
    let mut req = IfReq { name: [0; 16], flags: (libc::IFF_TAP | libc::IFF_NO_PI) as u16, _pad: [0; 22] };
    req.name[..ifname.len()].copy_from_slice(ifname.as_bytes());
    const TUNSETIFF: libc::c_ulong = 0x400454ca;
    let rc = unsafe { libc::ioctl(fd, TUNSETIFF, &mut req as *mut IfReq) };
    if rc < 0 {
        let e = std::io::Error::last_os_error();
        unsafe { libc::close(fd); }
        bail!("TUNSETIFF on '{ifname}': {e} (create it first: `ip tuntap add dev {ifname} mode tap user $USER && ip link set {ifname} up`)");
    }
    Ok(fd)
}

// ── TAP rx thread ───────────────────────────────────────────────────────

fn tap_rx_thread(
    tap_fd: std::os::unix::io::RawFd,
    guest: Arc<GuestMem>,
    vnet: Arc<Mutex<VirtioNetDev>>,
) {
    let mut frame_buf = vec![0u8; 1600];
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) { break; }

        // poll with 100 ms timeout so we check SHUTDOWN periodically.
        let mut pfd = libc::pollfd { fd: tap_fd, events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut pfd, 1, 100) };
        if r <= 0 { continue; }

        let n = unsafe {
            libc::read(tap_fd, frame_buf.as_mut_ptr() as *mut libc::c_void, frame_buf.len())
        };
        if n <= 0 { break; }
        let n = n as usize;

        let mut dev = vnet.lock().unwrap();
        let q = &mut dev.queues[0]; // RX
        if !q.ready || q.avail_addr == 0 { continue; }

        let avail_idx = g_r16(&guest, q.avail_addr.wrapping_add(2));
        if q.last_avail_idx == avail_idx {
            // No RX buffers posted; drop frame.
            continue;
        }

        let ring_slot = (q.last_avail_idx as u32 % q.num) as u32;
        let desc_id = g_r16(&guest, q.avail_addr.wrapping_add(4).wrapping_add(ring_slot * 2)) as usize;

        let d = q.desc_addr.wrapping_add(desc_id as u32 * 16);
        let buf_addr = g_r32(&guest, d) as u32;
        let buf_len = g_r32(&guest, d.wrapping_add(8)) as usize;

        let total = (VIRTIO_NET_HDR_SIZE + n).min(buf_len);
        // Write 10-byte virtio-net header (all zeros = no offload).
        let _ = guest.write(buf_addr as u64, &[0u8; 10]);
        if total > VIRTIO_NET_HDR_SIZE {
            let frame_len = total - VIRTIO_NET_HDR_SIZE;
            let _ = guest.write(buf_addr as u64 + 10, &frame_buf[..frame_len]);
        }

        // Update used ring.
        let used_idx = g_r16(&guest, q.used_addr.wrapping_add(2));
        let used_slot = (used_idx as u32 % q.num) as u32;
        g_w32(&guest, q.used_addr.wrapping_add(4).wrapping_add(used_slot * 8), desc_id as u32);
        g_w32(&guest, q.used_addr.wrapping_add(4).wrapping_add(used_slot * 8 + 4), total as u32);
        g_w16(&guest, q.used_addr.wrapping_add(2), used_idx.wrapping_add(1));

        q.last_avail_idx = q.last_avail_idx.wrapping_add(1);
        dev.interrupt_status |= 1;
    }
}

// ── Entry point ────────────────────────────────────────────────────────

pub fn run(cfg: VmConfig) -> Result<()> {
    if matches!(cfg.boot, BootSource::Snapshot(_)) {
        bail!("snapshot/restore is currently x86_64-only");
    }
    if cfg.disk_path.is_some() {
        bail!("riscv32 software backend does not support --disk yet");
    }

    let kernel_path = match &cfg.boot {
        BootSource::Kernel(path) => path.clone(),
        BootSource::Snapshot(_) => unreachable!(),
    };
    if kernel_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("iso"))
        .unwrap_or(false)
    {
        bail!("--arch riscv32 requires a flat ELF kernel image; ISO boot is x86_64-only");
    }

    const RISCV32_GPA_BASE: u64 = 0x8000_0000;
    let guest = Arc::new(GuestMem::new_with_base(RISCV32_GPA_BASE, cfg.memory_bytes)?);
    let loaded = elf::load(&kernel_path, guest.as_ref())
        .with_context(|| format!("loading {}", kernel_path.display()))?;

    eprintln!(
        "[veer-vm] rv32-soft: loaded {} entry={:#x} end={:#x} memory={} MiB",
        kernel_path.display(),
        loaded.entry,
        loaded.end,
        cfg.memory_bytes / (1024 * 1024),
    );

    // Open TAP if requested.
    let tap_fd = if let Some(ifname) = &cfg.tap_name {
        let fd = open_tap(ifname)?;
        eprintln!(
            "[veer-vm] rv32-soft: virtio-net tap={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            ifname,
            cfg.mac[0], cfg.mac[1], cfg.mac[2], cfg.mac[3], cfg.mac[4], cfg.mac[5],
        );
        fd
    } else {
        -1
    };

    let vnet = Arc::new(Mutex::new(VirtioNetDev::new(cfg.mac, tap_fd)));

    vm::install_signal_handlers()?;
    let raw_guard = RawMode::enter()?;
    let interactive = raw_guard.is_active();
    if interactive {
        eprintln!("[veer-vm] raw mode engaged — press Ctrl-A x to quit");
    }
    let _raw_guard = raw_guard;

    let rx_queue = Arc::new(Mutex::new(VecDeque::<u8>::new()));
    let main_tid: libc::pthread_t = unsafe { libc::pthread_self() };
    {
        let q = rx_queue.clone();
        thread::Builder::new()
            .name("veer-vm-rv32-rx".into())
            .spawn(move || reader_thread(q, main_tid, interactive))
            .context("spawn rv32 stdin reader thread")?;
    }

    // Spawn TAP rx thread if TAP is configured.
    if tap_fd >= 0 {
        let guest_rx = guest.clone();
        let vnet_rx = vnet.clone();
        thread::Builder::new()
            .name("veer-vm-rv32-tap-rx".into())
            .spawn(move || tap_rx_thread(tap_fd, guest_rx, vnet_rx))
            .context("spawn rv32 TAP rx thread")?;
    }

    // Spawn sensor feed thread if a FIFO path is provided.
    // This thread blocks reading lines from the FIFO and injects them byte-by-
    // byte into the guest UART RX queue (same mechanism as stdin).
    if let Some(feed_path) = &cfg.sensor_feed {
        let feed_path = feed_path.clone();
        let q = rx_queue.clone();
        thread::Builder::new()
            .name("veer-vm-rv32-sensor".into())
            .spawn(move || sensor_feed_thread(feed_path, q))
            .context("spawn rv32 sensor feed thread")?;
    }

    let mut cpu = Cpu::new(loaded.entry as u32);
    cpu.set_reg(10, 0); // a0 = hartid
    cpu.set_reg(11, 0); // a1 = dtb ptr (none)

    let mut clint = Clint::new();

    eprintln!("[veer-vm] rv32-soft: entering guest at pc={:#x}", cpu.pc);
    eprintln!(
        "[veer-vm] rv32-soft: cpu-throttle={}ms (set --cpu-throttle-ms 0 to disable)",
        cfg.cpu_throttle_ms
    );

    const GPA_BASE: u32 = 0x8000_0000;
    let mem_end: u32 = GPA_BASE
        .checked_add(cfg.memory_bytes as u32)
        .ok_or_else(|| anyhow::anyhow!("memory size overflows u32"))?;

    let mut last_shutdown_check: u64 = 0;

    // ── Idle throttle ────────────────────────────────────────────────────────
    // The interpreter runs far faster than real ESP32-C6 hardware.  When the
    // guest has no blocking work (smoltcp DHCP poll loops, shell read loops)
    // it never executes WFI yet never does real work either — it just burns
    // host CPU spinning.
    //
    // Throttle strategy:
    //   • BUDGET_NS  — how long to run at full speed before forcing a sleep.
    //   • YIELD_US   — how long to sleep when the budget is exhausted.
    //
    // Budget 5 ms / sleep 4 ms — safety net for guests that never hit WFI.
    // With the 2 MHz CLINT, WFI already sleeps ~5 ms per tick, so this
    // throttle rarely fires; it's here to protect against pathological
    // spin-loops in guest code.
    const BUDGET_NS:  u128 = 5_000_000; // 5 ms run window
    let throttle_us: u64 = cfg.cpu_throttle_ms.saturating_mul(1_000);
    let mut last_wfi_exit = Instant::now();

    loop {
        if cpu.retired.wrapping_sub(last_shutdown_check) > 65536 {
            if SHUTDOWN.load(Ordering::SeqCst) {
                eprintln!("\r\n[veer-vm] shutdown requested — stopping guest");
                return Ok(());
            }
            last_shutdown_check = cpu.retired;
        }

        if maybe_take_interrupt(&mut cpu, &clint) {
            cpu.wfi = false;
        }

        if cpu.wfi {
            if SHUTDOWN.load(Ordering::SeqCst) { return Ok(()); }
            // Sleep until the next timer deadline rather than a fixed 200 µs.
            // mtimecmp is in CLINT_FREQ_HZ ticks; convert to microseconds.
            let mtime_now = clint.mtime();
            let sleep_us = if clint.mtimecmp > mtime_now {
                let ticks_left = clint.mtimecmp - mtime_now;
                // ticks_left * 1_000_000 / CLINT_FREQ_HZ = µs; cap at 10 ms.
                ((ticks_left * 1_000_000) / CLINT_FREQ_HZ).min(10_000)
            } else {
                0
            };
            // Always sleep at least 200 µs even when the timer is already
            // pending, so `yield_now` can't turn into a spin loop.
            thread::sleep(Duration::from_micros(sleep_us.max(200)));
            last_wfi_exit = Instant::now();
            continue;
        }

        // Busy-guest throttle: check every 16 K instructions to keep overhead low.
        // If the guest hasn't hit WFI for BUDGET_NS, yield the host thread.
        if throttle_us > 0
            && cpu.retired & 0x3FFF == 0
            && last_wfi_exit.elapsed().as_nanos() > BUDGET_NS
        {
            thread::sleep(Duration::from_micros(throttle_us));
            last_wfi_exit = Instant::now();
        }

        match step(&mut cpu, &guest, GPA_BASE, mem_end, &rx_queue, &mut clint, &vnet) {
            Ok(()) => {
                cpu.retired = cpu.retired.wrapping_add(1);
            }
            Err(StepError::Trap { cause, tval }) => {
                take_trap(&mut cpu, cause, tval);
            }
            Err(StepError::Halt) => {
                return Ok(());
            }
            Err(StepError::Fatal(e)) => {
                bail!("rv32-soft fatal error: {e}");
            }
        }
    }
}

// ── step / decode / execute ────────────────────────────────────────────

enum StepError {
    Trap { cause: u32, tval: u32 },
    Halt,
    Fatal(anyhow::Error),
}

impl From<anyhow::Error> for StepError {
    fn from(e: anyhow::Error) -> Self { StepError::Fatal(e) }
}

fn step(
    cpu: &mut Cpu,
    mem: &Arc<GuestMem>,
    gpa_base: u32,
    mem_end: u32,
    rx_queue: &Arc<Mutex<VecDeque<u8>>>,
    clint: &mut Clint,
    vnet: &Arc<Mutex<VirtioNetDev>>,
) -> Result<(), StepError> {
    // Fetch: try 16-bit first; if not compressed, fetch the upper halfword.
    let pc = cpu.pc;
    if pc < gpa_base || pc >= mem_end {
        return Err(StepError::Trap {
            cause: EXC_INST_ACCESS_FAULT,
            tval: pc,
        });
    }
    if (pc & 0x1) != 0 {
        return Err(StepError::Trap {
            cause: EXC_INST_ADDR_MISALIGNED,
            tval: pc,
        });
    }

    let lo = mem.read_u16(pc as u64).map_err(|_| StepError::Trap {
        cause: EXC_INST_ACCESS_FAULT,
        tval: pc,
    })?;
    if (lo & 0b11) != 0b11 {
        // 16-bit compressed instruction.
        cpu.pc = pc.wrapping_add(2);
        return execute_c(cpu, mem, gpa_base, mem_end, rx_queue, clint, vnet, lo, pc);
    }

    // 32-bit instruction: fetch upper halfword.
    let pc_hi = pc.wrapping_add(2);
    if pc_hi >= mem_end {
        return Err(StepError::Trap {
            cause: EXC_INST_ACCESS_FAULT,
            tval: pc_hi,
        });
    }
    let hi = mem.read_u16(pc_hi as u64).map_err(|_| StepError::Trap {
        cause: EXC_INST_ACCESS_FAULT,
        tval: pc_hi,
    })?;
    let inst = (lo as u32) | ((hi as u32) << 16);
    cpu.pc = pc.wrapping_add(4);
    execute_32(cpu, mem, gpa_base, mem_end, rx_queue, clint, vnet, inst, pc)
}

// ── 32-bit instruction execution ───────────────────────────────────────

fn execute_32(
    cpu: &mut Cpu,
    mem: &Arc<GuestMem>,
    gpa_base: u32,
    mem_end: u32,
    rx_queue: &Arc<Mutex<VecDeque<u8>>>,
    clint: &mut Clint,
    vnet: &Arc<Mutex<VirtioNetDev>>,
    inst: u32,
    pc: u32,
) -> Result<(), StepError> {
    let opcode = inst & 0x7F;
    let rd = ((inst >> 7) & 0x1F) as usize;
    let rs1 = ((inst >> 15) & 0x1F) as usize;
    let rs2 = ((inst >> 20) & 0x1F) as usize;
    let funct3 = (inst >> 12) & 0x7;
    let funct7 = (inst >> 25) & 0x7F;

    match opcode {
        0b0110111 => {
            // LUI
            let imm = inst & 0xFFFF_F000;
            cpu.set_reg(rd, imm);
        }
        0b0010111 => {
            // AUIPC
            let imm = inst & 0xFFFF_F000;
            cpu.set_reg(rd, pc.wrapping_add(imm));
        }
        0b1101111 => {
            // JAL
            let imm = sext_j(inst);
            let target = pc.wrapping_add(imm as u32);
            if (target & 0x1) != 0 {
                return Err(StepError::Trap {
                    cause: EXC_INST_ADDR_MISALIGNED,
                    tval: target,
                });
            }
            cpu.set_reg(rd, cpu.pc); // pc already advanced
            cpu.pc = target;
        }
        0b1100111 => {
            // JALR
            if funct3 != 0 {
                return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst });
            }
            let imm = sext_i(inst);
            let target = cpu.reg(rs1).wrapping_add(imm as u32) & !1u32;
            let link = cpu.pc;
            cpu.set_reg(rd, link);
            cpu.pc = target;
        }
        0b1100011 => {
            // BRANCH
            let imm = sext_b(inst);
            let a = cpu.reg(rs1);
            let b = cpu.reg(rs2);
            let take = match funct3 {
                0b000 => a == b,                 // BEQ
                0b001 => a != b,                 // BNE
                0b100 => (a as i32) < (b as i32),// BLT
                0b101 => (a as i32) >= (b as i32),// BGE
                0b110 => a < b,                  // BLTU
                0b111 => a >= b,                 // BGEU
                _ => return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst }),
            };
            if take {
                let target = pc.wrapping_add(imm as u32);
                if (target & 0x1) != 0 {
                    return Err(StepError::Trap {
                        cause: EXC_INST_ADDR_MISALIGNED,
                        tval: target,
                    });
                }
                cpu.pc = target;
            }
        }
        0b0000011 => {
            // LOAD
            let imm = sext_i(inst);
            let addr = cpu.reg(rs1).wrapping_add(imm as u32);
            let v = match funct3 {
                0b000 => load_u8(mem, gpa_base, mem_end, rx_queue, clint, vnet, addr)? as i8 as i32 as u32,
                0b001 => load_u16(mem, gpa_base, mem_end, addr)? as i16 as i32 as u32,
                0b010 => load_u32(mem, gpa_base, mem_end, rx_queue, clint, vnet, addr)?,
                0b100 => load_u8(mem, gpa_base, mem_end, rx_queue, clint, vnet, addr)? as u32,
                0b101 => load_u16(mem, gpa_base, mem_end, addr)? as u32,
                _ => return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst }),
            };
            cpu.set_reg(rd, v);
        }
        0b0100011 => {
            // STORE
            let imm = sext_s(inst);
            let addr = cpu.reg(rs1).wrapping_add(imm as u32);
            let val = cpu.reg(rs2);
            match funct3 {
                0b000 => store_u8(mem, gpa_base, mem_end, clint, vnet, addr, val as u8)?,
                0b001 => store_u16(mem, gpa_base, mem_end, addr, val as u16)?,
                0b010 => store_u32(mem, gpa_base, mem_end, clint, vnet, addr, val)?,
                _ => return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst }),
            }
        }
        0b0010011 => {
            // OP-IMM
            let imm = sext_i(inst);
            let a = cpu.reg(rs1);
            let shamt = (inst >> 20) & 0x1F;
            let v = match funct3 {
                0b000 => a.wrapping_add(imm as u32),             // ADDI
                0b010 => if (a as i32) < imm { 1 } else { 0 },   // SLTI
                0b011 => if a < (imm as u32) { 1 } else { 0 },   // SLTIU
                0b100 => a ^ (imm as u32),                        // XORI
                0b110 => a | (imm as u32),                        // ORI
                0b111 => a & (imm as u32),                        // ANDI
                0b001 => {
                    if funct7 != 0 {
                        return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst });
                    }
                    a << shamt
                }
                0b101 => match funct7 {
                    0b0000000 => a >> shamt,                      // SRLI
                    0b0100000 => ((a as i32) >> shamt) as u32,    // SRAI
                    _ => return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst }),
                },
                _ => unreachable!(),
            };
            cpu.set_reg(rd, v);
        }
        0b0110011 => {
            // OP
            let a = cpu.reg(rs1);
            let b = cpu.reg(rs2);
            let v = match (funct7, funct3) {
                (0b0000000, 0b000) => a.wrapping_add(b),                 // ADD
                (0b0100000, 0b000) => a.wrapping_sub(b),                 // SUB
                (0b0000000, 0b001) => a << (b & 0x1F),                   // SLL
                (0b0000000, 0b010) => if (a as i32) < (b as i32) { 1 } else { 0 }, // SLT
                (0b0000000, 0b011) => if a < b { 1 } else { 0 },         // SLTU
                (0b0000000, 0b100) => a ^ b,                             // XOR
                (0b0000000, 0b101) => a >> (b & 0x1F),                   // SRL
                (0b0100000, 0b101) => ((a as i32) >> (b & 0x1F) as i32) as u32, // SRA
                (0b0000000, 0b110) => a | b,                             // OR
                (0b0000000, 0b111) => a & b,                             // AND
                // RV32M
                (0b0000001, 0b000) => a.wrapping_mul(b),                 // MUL
                (0b0000001, 0b001) => {                                  // MULH
                    let aw = (a as i32) as i64;
                    let bw = (b as i32) as i64;
                    ((aw * bw) >> 32) as u32
                }
                (0b0000001, 0b010) => {                                  // MULHSU
                    let aw = (a as i32) as i64;
                    let bw = b as u64 as i64;
                    ((aw * bw) >> 32) as u32
                }
                (0b0000001, 0b011) => {                                  // MULHU
                    let aw = a as u64;
                    let bw = b as u64;
                    ((aw * bw) >> 32) as u32
                }
                (0b0000001, 0b100) => {                                  // DIV
                    if b == 0 {
                        u32::MAX
                    } else if a == 0x8000_0000 && b == 0xFFFF_FFFF {
                        0x8000_0000
                    } else {
                        ((a as i32).wrapping_div(b as i32)) as u32
                    }
                }
                (0b0000001, 0b101) => {                                  // DIVU
                    if b == 0 { u32::MAX } else { a / b }
                }
                (0b0000001, 0b110) => {                                  // REM
                    if b == 0 {
                        a
                    } else if a == 0x8000_0000 && b == 0xFFFF_FFFF {
                        0
                    } else {
                        ((a as i32).wrapping_rem(b as i32)) as u32
                    }
                }
                (0b0000001, 0b111) => {                                  // REMU
                    if b == 0 { a } else { a % b }
                }
                _ => return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst }),
            };
            cpu.set_reg(rd, v);
        }
        0b0001111 => {
            // MISC-MEM: FENCE / FENCE.I — no-op in our single-threaded model.
        }
        0b1110011 => {
            // SYSTEM
            if funct3 == 0 {
                match inst {
                    0x0000_0073 => {
                        // ECALL
                        return Err(StepError::Trap { cause: EXC_ECALL_M, tval: 0 });
                    }
                    0x0010_0073 => {
                        // EBREAK
                        return Err(StepError::Trap { cause: EXC_BREAKPOINT, tval: pc });
                    }
                    0x3020_0073 => {
                        // MRET
                        cpu.pc = cpu.mepc;
                        let mpie = (cpu.mstatus >> 7) & 1;
                        // Restore MIE from MPIE, set MPIE=1, clear MPP.
                        cpu.mstatus = (cpu.mstatus & !MSTATUS_MIE)
                            | (mpie << 3)
                            | MSTATUS_MPIE;
                        cpu.mstatus &= !MSTATUS_MPP_MASK;
                        return Ok(());
                    }
                    0x1050_0073 => {
                        // WFI
                        cpu.wfi = true;
                        return Ok(());
                    }
                    _ => {
                        return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst });
                    }
                }
            } else {
                // CSR ops
                let csr = ((inst >> 20) & 0xFFF) as u16;
                let uimm = rs1 as u32; // CSR*I encodes zimm in rs1 field
                let is_imm = funct3 & 0b100 != 0;
                let src = if is_imm { uimm } else { cpu.reg(rs1) };
                let old = csr_read(cpu, clint, csr).ok_or(StepError::Trap {
                    cause: EXC_ILLEGAL_INST,
                    tval: inst,
                })?;
                let new = match funct3 & 0b011 {
                    0b001 => src,                                   // CSRRW(I)
                    0b010 => old | src,                             // CSRRS(I)
                    0b011 => old & !src,                            // CSRRC(I)
                    _ => return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst }),
                };
                // Writes only happen for CSRRW, or CSRRS/C with rs1!=x0 (imm!=0).
                let do_write = match funct3 & 0b011 {
                    0b001 => true,
                    _ => if is_imm { uimm != 0 } else { rs1 != 0 },
                };
                if do_write {
                    csr_write(cpu, clint, csr, new).ok_or(StepError::Trap {
                        cause: EXC_ILLEGAL_INST,
                        tval: inst,
                    })?;
                }
                cpu.set_reg(rd, old);
            }
        }
        _ => {
            return Err(StepError::Trap { cause: EXC_ILLEGAL_INST, tval: inst });
        }
    }
    Ok(())
}

// ── Compressed (RV32C) instructions ─────────────────────────────────────

fn execute_c(
    cpu: &mut Cpu,
    mem: &Arc<GuestMem>,
    gpa_base: u32,
    mem_end: u32,
    rx_queue: &Arc<Mutex<VecDeque<u8>>>,
    clint: &mut Clint,
    vnet: &Arc<Mutex<VirtioNetDev>>,
    inst: u16,
    pc: u32,
) -> Result<(), StepError> {
    let op = inst & 0b11;
    let funct3 = (inst >> 13) & 0b111;
    let illegal = |inst: u16| StepError::Trap {
        cause: EXC_ILLEGAL_INST,
        tval: inst as u32,
    };

    // Helper: 3-bit compressed register → x8..x15
    let rs1p = |i: u16| ((i >> 7) & 0b111) as usize + 8;
    let rs2p = |i: u16| ((i >> 2) & 0b111) as usize + 8;

    match (op, funct3) {
        // ── Quadrant 0 ───────────────────────────────────
        (0b00, 0b000) => {
            // C.ADDI4SPN: rd' = sp + nzuimm
            let rd = rs2p(inst);
            let nzuimm = (((inst >> 7) & 0x30)       // [5:4]
                | ((inst >> 1) & 0x3C0)              // [9:6]
                | ((inst >> 4) & 0x04)               // [2]
                | ((inst >> 2) & 0x08)) as u32;      // [3]
            if nzuimm == 0 { return Err(illegal(inst)); }
            cpu.set_reg(rd, cpu.reg(2).wrapping_add(nzuimm));
        }
        (0b00, 0b010) => {
            // C.LW rd' = M[rs1' + uimm]
            let rs1 = rs1p(inst);
            let rd = rs2p(inst);
            let uimm = (((inst >> 7) & 0x38)         // [5:3]
                | ((inst << 1) & 0x40)               // [6]
                | ((inst >> 4) & 0x04)) as u32;      // [2]
            let addr = cpu.reg(rs1).wrapping_add(uimm);
            let v = load_u32(mem, gpa_base, mem_end, rx_queue, clint, vnet, addr)?;
            cpu.set_reg(rd, v);
        }
        (0b00, 0b110) => {
            // C.SW M[rs1' + uimm] = rs2'
            let rs1 = rs1p(inst);
            let rs2 = rs2p(inst);
            let uimm = (((inst >> 7) & 0x38)
                | ((inst << 1) & 0x40)
                | ((inst >> 4) & 0x04)) as u32;
            let addr = cpu.reg(rs1).wrapping_add(uimm);
            store_u32(mem, gpa_base, mem_end, clint, vnet, addr, cpu.reg(rs2))?;
        }
        // ── Quadrant 1 ───────────────────────────────────
        (0b01, 0b000) => {
            // C.ADDI / C.NOP
            let rd = ((inst >> 7) & 0x1F) as usize;
            let imm = c_imm6(inst);
            if rd == 0 {
                // C.NOP (imm must be 0 but we accept any)
            } else {
                cpu.set_reg(rd, cpu.reg(rd).wrapping_add(imm as u32));
            }
        }
        (0b01, 0b001) => {
            // C.JAL: rd=x1 = pc+2; pc += imm
            let imm = c_imm_cj(inst);
            let link = pc.wrapping_add(2);
            cpu.set_reg(1, link);
            cpu.pc = pc.wrapping_add(imm as u32);
        }
        (0b01, 0b010) => {
            // C.LI: rd = sext(imm)
            let rd = ((inst >> 7) & 0x1F) as usize;
            let imm = c_imm6(inst);
            cpu.set_reg(rd, imm as u32);
        }
        (0b01, 0b011) => {
            // C.ADDI16SP (rd=2) or C.LUI
            let rd = ((inst >> 7) & 0x1F) as usize;
            if rd == 2 {
                // C.ADDI16SP
                let imm = (((inst >> 2) & 0x10)       // [4]
                    | ((inst << 3) & 0x20)            // [5]
                    | ((inst << 1) & 0x40)            // [6]
                    | ((inst << 4) & 0x180)           // [8:7]
                    | ((inst >> 3) & 0x200)) as u32;  // [9]
                let mut imm = imm;
                if inst & (1 << 12) != 0 {
                    imm |= 0xFFFF_FC00;
                }
                if imm == 0 { return Err(illegal(inst)); }
                cpu.set_reg(2, cpu.reg(2).wrapping_add(imm));
            } else if rd != 0 {
                // C.LUI
                let mut imm = (((inst >> 2) & 0x1F) as u32) << 12;
                if inst & (1 << 12) != 0 {
                    imm |= 0xFFFC_0000;
                }
                if imm == 0 { return Err(illegal(inst)); }
                cpu.set_reg(rd, imm);
            } else {
                return Err(illegal(inst));
            }
        }
        (0b01, 0b100) => {
            // Miscellaneous ALU (CR/CA)
            let funct2 = (inst >> 10) & 0b11;
            let rd = rs1p(inst);
            match funct2 {
                0b00 => {
                    // C.SRLI
                    let shamt = c_shamt(inst);
                    cpu.set_reg(rd, cpu.reg(rd) >> shamt);
                }
                0b01 => {
                    // C.SRAI
                    let shamt = c_shamt(inst);
                    cpu.set_reg(rd, ((cpu.reg(rd) as i32) >> shamt) as u32);
                }
                0b10 => {
                    // C.ANDI
                    let imm = c_imm6(inst);
                    cpu.set_reg(rd, cpu.reg(rd) & (imm as u32));
                }
                0b11 => {
                    // CA: C.SUB / C.XOR / C.OR / C.AND (bit12=0)
                    let rs2 = rs2p(inst);
                    let sel = ((inst >> 10) & 0b100) | ((inst >> 5) & 0b11);
                    let a = cpu.reg(rd);
                    let b = cpu.reg(rs2);
                    let v = match sel {
                        0b000 => a.wrapping_sub(b), // C.SUB
                        0b001 => a ^ b,             // C.XOR
                        0b010 => a | b,             // C.OR
                        0b011 => a & b,             // C.AND
                        _ => return Err(illegal(inst)),
                    };
                    cpu.set_reg(rd, v);
                }
                _ => return Err(illegal(inst)),
            }
        }
        (0b01, 0b101) => {
            // C.J
            let imm = c_imm_cj(inst);
            cpu.pc = pc.wrapping_add(imm as u32);
        }
        (0b01, 0b110) => {
            // C.BEQZ
            let rs1 = rs1p(inst);
            if cpu.reg(rs1) == 0 {
                cpu.pc = pc.wrapping_add(c_imm_cb(inst) as u32);
            }
        }
        (0b01, 0b111) => {
            // C.BNEZ
            let rs1 = rs1p(inst);
            if cpu.reg(rs1) != 0 {
                cpu.pc = pc.wrapping_add(c_imm_cb(inst) as u32);
            }
        }
        // ── Quadrant 2 ───────────────────────────────────
        (0b10, 0b000) => {
            // C.SLLI
            let rd = ((inst >> 7) & 0x1F) as usize;
            let shamt = c_shamt(inst);
            if rd != 0 {
                cpu.set_reg(rd, cpu.reg(rd) << shamt);
            }
        }
        (0b10, 0b010) => {
            // C.LWSP rd = M[sp + uimm]
            let rd = ((inst >> 7) & 0x1F) as usize;
            if rd == 0 { return Err(illegal(inst)); }
            let uimm = (((inst >> 7) & 0x20)          // [5]
                | ((inst >> 2) & 0x1C)                // [4:2]
                | ((inst << 4) & 0xC0)) as u32;       // [7:6]
            let addr = cpu.reg(2).wrapping_add(uimm);
            let v = load_u32(mem, gpa_base, mem_end, rx_queue, clint, vnet, addr)?;
            cpu.set_reg(rd, v);
        }
        (0b10, 0b100) => {
            // C.JR / C.MV / C.EBREAK / C.JALR / C.ADD
            let rd = ((inst >> 7) & 0x1F) as usize;
            let rs2 = ((inst >> 2) & 0x1F) as usize;
            let bit12 = (inst >> 12) & 1 != 0;
            match (bit12, rd, rs2) {
                (false, r, 0) if r != 0 => {
                    // C.JR
                    cpu.pc = cpu.reg(r) & !1u32;
                }
                (false, r, s) if r != 0 && s != 0 => {
                    // C.MV rd = rs2
                    cpu.set_reg(r, cpu.reg(s));
                }
                (true, 0, 0) => {
                    // C.EBREAK
                    return Err(StepError::Trap { cause: EXC_BREAKPOINT, tval: pc });
                }
                (true, r, 0) if r != 0 => {
                    // C.JALR
                    let link = pc.wrapping_add(2);
                    let target = cpu.reg(r) & !1u32;
                    cpu.set_reg(1, link);
                    cpu.pc = target;
                }
                (true, r, s) if r != 0 && s != 0 => {
                    // C.ADD rd = rd + rs2
                    cpu.set_reg(r, cpu.reg(r).wrapping_add(cpu.reg(s)));
                }
                _ => return Err(illegal(inst)),
            }
        }
        (0b10, 0b110) => {
            // C.SWSP: M[sp + uimm] = rs2
            let rs2 = ((inst >> 2) & 0x1F) as usize;
            let uimm = (((inst >> 7) & 0x3C)          // [5:2]
                | ((inst >> 1) & 0xC0)) as u32;       // [7:6]
            let addr = cpu.reg(2).wrapping_add(uimm);
            store_u32(mem, gpa_base, mem_end, clint, vnet, addr, cpu.reg(rs2))?;
        }
        _ => return Err(illegal(inst)),
    }
    Ok(())
}

// ── immediate helpers ──────────────────────────────────────────────────

fn sext_i(inst: u32) -> i32 {
    ((inst as i32) >> 20) // already sign extends on i32
}

fn sext_s(inst: u32) -> i32 {
    let hi = ((inst >> 25) & 0x7F) as i32;
    let lo = ((inst >> 7) & 0x1F) as i32;
    let v = (hi << 5) | lo;
    // sign-extend from bit 11
    ((v << 20) >> 20)
}

fn sext_b(inst: u32) -> i32 {
    let b11 = ((inst >> 7) & 0x1) << 11;
    let b4_1 = ((inst >> 8) & 0xF) << 1;
    let b10_5 = ((inst >> 25) & 0x3F) << 5;
    let b12 = ((inst >> 31) & 0x1) << 12;
    let v = (b12 | b11 | b10_5 | b4_1) as i32;
    (v << 19) >> 19
}

fn sext_j(inst: u32) -> i32 {
    let b20 = ((inst >> 31) & 0x1) << 20;
    let b10_1 = ((inst >> 21) & 0x3FF) << 1;
    let b11 = ((inst >> 20) & 0x1) << 11;
    let b19_12 = ((inst >> 12) & 0xFF) << 12;
    let v = (b20 | b19_12 | b11 | b10_1) as i32;
    (v << 11) >> 11
}

fn c_imm6(inst: u16) -> i32 {
    let lo = ((inst >> 2) & 0x1F) as u32;
    let hi = ((inst >> 12) & 0x1) as u32;
    let v = (hi << 5) | lo;
    // sign-extend from bit 5
    ((v << 26) as i32) >> 26
}

fn c_shamt(inst: u16) -> u32 {
    // [5]=bit12, [4:0]=bits[6:2]
    let lo = ((inst >> 2) & 0x1F) as u32;
    let hi = ((inst >> 12) & 0x1) as u32;
    (hi << 5) | lo
}

fn c_imm_cj(inst: u16) -> i32 {
    // CJ-type imm[11|4|9:8|10|6|7|3:1|5]
    let i = inst as u32;
    let imm = ((i >> 1)  & 0x800)        // [11] = bit12
        | ((i >> 7) & 0x10)               // [4]  = bit11
        | ((i >> 1) & 0x300)              // [9:8]= bits10:9
        | ((i << 2) & 0x400)              // [10] = bit8
        | ((i >> 1) & 0x40)               // [6]  = bit7
        | ((i << 1) & 0x80)               // [7]  = bit6
        | ((i >> 2) & 0xE)                // [3:1]= bits5:3
        | ((i << 3) & 0x20);              // [5]  = bit2
    // sign-extend from bit 11
    ((imm as i32) << 20) >> 20
}

fn c_imm_cb(inst: u16) -> i32 {
    // CB-type imm[8|4:3|7:6|2:1|5]
    let i = inst as u32;
    let imm = ((i >> 4) & 0x100)          // [8]  = bit12
        | ((i >> 7) & 0x18)               // [4:3]= bits11:10
        | ((i << 1) & 0xC0)               // [7:6]= bits6:5
        | ((i >> 2) & 0x6)                // [2:1]= bits4:3
        | ((i << 3) & 0x20);              // [5]  = bit2
    ((imm as i32) << 23) >> 23
}

// ── Load / store with MMIO interception ────────────────────────────────

fn load_u8(
    mem: &Arc<GuestMem>,
    gpa_base: u32,
    mem_end: u32,
    rx_queue: &Arc<Mutex<VecDeque<u8>>>,
    clint: &Clint,
    vnet: &Arc<Mutex<VirtioNetDev>>,
    addr: u32,
) -> Result<u8, StepError> {
    if addr >= UART_BASE && addr < UART_END {
        return Ok(uart_read(rx_queue, addr - UART_BASE));
    }
    if addr >= CLINT_BASE && addr < CLINT_END {
        // Narrow reads from CLINT: uncommon; return low byte of word read.
        let word = clint_read(clint, addr & !3);
        return Ok(((word >> ((addr & 3) * 8)) & 0xFF) as u8);
    }
    if addr >= VMNET_BASE && addr < VMNET_END {
        let dev = vnet.lock().unwrap();
        return Ok(vmnet_read_u8(&dev, addr - VMNET_BASE));
    }
    if addr >= VMNET_END && addr < VMSLOT_END {
        return Ok(0);
    }
    if addr < gpa_base || addr >= mem_end {
        return Err(StepError::Trap {
            cause: EXC_LOAD_ACCESS_FAULT,
            tval: addr,
        });
    }
    mem.read_u8(addr as u64).map_err(|_| StepError::Trap {
        cause: EXC_LOAD_ACCESS_FAULT,
        tval: addr,
    })
}

fn load_u16(
    mem: &Arc<GuestMem>,
    gpa_base: u32,
    mem_end: u32,
    addr: u32,
) -> Result<u16, StepError> {
    if (addr & 1) != 0 {
        return Err(StepError::Trap {
            cause: EXC_LOAD_ADDR_MISALIGNED,
            tval: addr,
        });
    }
    if addr < gpa_base || addr.wrapping_add(2) > mem_end {
        return Err(StepError::Trap {
            cause: EXC_LOAD_ACCESS_FAULT,
            tval: addr,
        });
    }
    mem.read_u16(addr as u64).map_err(|_| StepError::Trap {
        cause: EXC_LOAD_ACCESS_FAULT,
        tval: addr,
    })
}

fn load_u32(
    mem: &Arc<GuestMem>,
    gpa_base: u32,
    mem_end: u32,
    rx_queue: &Arc<Mutex<VecDeque<u8>>>,
    clint: &Clint,
    vnet: &Arc<Mutex<VirtioNetDev>>,
    addr: u32,
) -> Result<u32, StepError> {
    if addr >= UART_BASE && addr < UART_END {
        return Ok(uart_read(rx_queue, addr - UART_BASE) as u32);
    }
    if addr >= CLINT_BASE && addr < CLINT_END {
        return Ok(clint_read(clint, addr - CLINT_BASE));
    }
    if addr >= VMNET_BASE && addr < VMNET_END {
        let dev = vnet.lock().unwrap();
        return Ok(vmnet_read_u32(&dev, addr - VMNET_BASE));
    }
    if addr >= VMNET_END && addr < VMSLOT_END {
        return Ok(0);
    }
    if (addr & 3) != 0 {
        return Err(StepError::Trap {
            cause: EXC_LOAD_ADDR_MISALIGNED,
            tval: addr,
        });
    }
    if addr < gpa_base || addr.wrapping_add(4) > mem_end {
        return Err(StepError::Trap {
            cause: EXC_LOAD_ACCESS_FAULT,
            tval: addr,
        });
    }
    mem.read_u32(addr as u64).map_err(|_| StepError::Trap {
        cause: EXC_LOAD_ACCESS_FAULT,
        tval: addr,
    })
}

fn store_u8(
    mem: &Arc<GuestMem>,
    gpa_base: u32,
    mem_end: u32,
    _clint: &mut Clint,
    vnet: &Arc<Mutex<VirtioNetDev>>,
    addr: u32,
    val: u8,
) -> Result<(), StepError> {
    if addr >= UART_BASE && addr < UART_END {
        uart_write(addr - UART_BASE, val);
        return Ok(());
    }
    if addr >= VMNET_BASE && addr < VMNET_END {
        vmnet_write(&mut vnet.lock().unwrap(), mem, addr - VMNET_BASE, val as u32);
        return Ok(());
    }
    if addr >= VMNET_END && addr < VMSLOT_END {
        return Ok(());
    }
    if addr < gpa_base || addr >= mem_end {
        return Err(StepError::Trap {
            cause: EXC_STORE_ACCESS_FAULT,
            tval: addr,
        });
    }
    mem.write_u8(addr as u64, val).map_err(|_| StepError::Trap {
        cause: EXC_STORE_ACCESS_FAULT,
        tval: addr,
    })
}

fn store_u16(
    mem: &Arc<GuestMem>,
    gpa_base: u32,
    mem_end: u32,
    addr: u32,
    val: u16,
) -> Result<(), StepError> {
    if (addr & 1) != 0 {
        return Err(StepError::Trap {
            cause: EXC_STORE_ADDR_MISALIGNED,
            tval: addr,
        });
    }
    if addr < gpa_base || addr.wrapping_add(2) > mem_end {
        return Err(StepError::Trap {
            cause: EXC_STORE_ACCESS_FAULT,
            tval: addr,
        });
    }
    mem.write_u16(addr as u64, val).map_err(|_| StepError::Trap {
        cause: EXC_STORE_ACCESS_FAULT,
        tval: addr,
    })
}

fn store_u32(
    mem: &Arc<GuestMem>,
    gpa_base: u32,
    mem_end: u32,
    clint: &mut Clint,
    vnet: &Arc<Mutex<VirtioNetDev>>,
    addr: u32,
    val: u32,
) -> Result<(), StepError> {
    if addr >= UART_BASE && addr < UART_END {
        uart_write(addr - UART_BASE, val as u8);
        return Ok(());
    }
    if addr >= CLINT_BASE && addr < CLINT_END {
        clint_write(clint, addr - CLINT_BASE, val);
        return Ok(());
    }
    if addr >= VMNET_BASE && addr < VMNET_END {
        vmnet_write(&mut vnet.lock().unwrap(), mem, addr - VMNET_BASE, val);
        return Ok(());
    }
    if addr >= VMNET_END && addr < VMSLOT_END {
        return Ok(());
    }
    if (addr & 3) != 0 {
        return Err(StepError::Trap {
            cause: EXC_STORE_ADDR_MISALIGNED,
            tval: addr,
        });
    }
    if addr < gpa_base || addr.wrapping_add(4) > mem_end {
        return Err(StepError::Trap {
            cause: EXC_STORE_ACCESS_FAULT,
            tval: addr,
        });
    }
    mem.write_u32(addr as u64, val).map_err(|_| StepError::Trap {
        cause: EXC_STORE_ACCESS_FAULT,
        tval: addr,
    })
}

// ── MMIO devices ───────────────────────────────────────────────────────

fn uart_read(rx_queue: &Arc<Mutex<VecDeque<u8>>>, off: u32) -> u8 {
    match off {
        UART_RBR_THR => rx_queue.lock().unwrap().pop_front().unwrap_or(0),
        UART_LSR => {
            let has = !rx_queue.lock().unwrap().is_empty();
            let mut v = UART_LSR_THRE | UART_LSR_TEMT;
            if has { v |= UART_LSR_DR; }
            v
        }
        UART_IIR_FCR => 0x01, // no interrupt pending
        UART_IER | UART_LCR | UART_MCR => 0,
        _ => 0,
    }
}

fn uart_write(off: u32, val: u8) {
    if off == UART_RBR_THR {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(&[val]);
        let _ = out.flush();
    }
    // Other registers (IER, LCR, MCR, FCR) are accepted and ignored.
}

fn clint_read(clint: &Clint, off: u32) -> u32 {
    match off {
        CLINT_MSIP => clint.msip,
        CLINT_MTIMECMP => clint.mtimecmp as u32,
        x if x == CLINT_MTIMECMP + 4 => (clint.mtimecmp >> 32) as u32,
        CLINT_MTIME => clint.mtime() as u32,
        x if x == CLINT_MTIME + 4 => (clint.mtime() >> 32) as u32,
        _ => 0,
    }
}

fn clint_write(clint: &mut Clint, off: u32, val: u32) {
    match off {
        CLINT_MSIP => { clint.msip = val & 1; }
        CLINT_MTIMECMP => {
            clint.mtimecmp = (clint.mtimecmp & !0xFFFF_FFFFu64) | (val as u64);
        }
        x if x == CLINT_MTIMECMP + 4 => {
            clint.mtimecmp = (clint.mtimecmp & 0xFFFF_FFFFu64) | ((val as u64) << 32);
        }
        _ => {}
    }
}

// ── CSR read / write ───────────────────────────────────────────────────

fn csr_read(cpu: &Cpu, clint: &Clint, csr: u16) -> Option<u32> {
    Some(match csr {
        CSR_MSTATUS  => cpu.mstatus,
        CSR_MISA     => 0x4000_0000 // MXL=1 (32-bit)
                       | (1 << 0)   // A? no → skip
                       | (1 << 8)   // I
                       | (1 << 12)  // M
                       | (1 << 2),  // C
        CSR_MIE      => cpu.mie,
        CSR_MTVEC    => cpu.mtvec,
        CSR_MSCRATCH => cpu.mscratch,
        CSR_MEPC     => cpu.mepc,
        CSR_MCAUSE   => cpu.mcause,
        CSR_MTVAL    => cpu.mtval,
        CSR_MIP => {
            let mut v = 0u32;
            if clint.timer_pending()   { v |= 1 << 7; }
            if clint.msip != 0 || cpu.mip_software { v |= 1 << 3; }
            if cpu.mip_external         { v |= 1 << 11; }
            v
        }
        CSR_MHARTID  => 0,
        CSR_MVENDORID| CSR_MARCHID | CSR_MIMPID => 0,
        CSR_CYCLE | CSR_INSTRET => cpu.retired as u32,
        CSR_CYCLEH | CSR_INSTRETH => (cpu.retired >> 32) as u32,
        CSR_TIME  => clint.mtime() as u32,
        CSR_TIMEH => (clint.mtime() >> 32) as u32,
        // PMP and other unimplemented — treat as 0 to keep kernel init happy.
        0x3A0..=0x3EF => 0,
        _ => return None,
    })
}

fn csr_write(cpu: &mut Cpu, _clint: &mut Clint, csr: u16, val: u32) -> Option<()> {
    match csr {
        CSR_MSTATUS  => { cpu.mstatus = val; }
        CSR_MISA     => { /* ignored */ }
        CSR_MIE      => { cpu.mie = val; }
        CSR_MTVEC    => { cpu.mtvec = val; }
        CSR_MSCRATCH => { cpu.mscratch = val; }
        CSR_MEPC     => { cpu.mepc = val & !1; }
        CSR_MCAUSE   => { cpu.mcause = val; }
        CSR_MTVAL    => { cpu.mtval = val; }
        CSR_MIP => {
            cpu.mip_software = (val & (1 << 3)) != 0;
            cpu.mip_external = (val & (1 << 11)) != 0;
        }
        CSR_MHARTID | CSR_MVENDORID | CSR_MARCHID | CSR_MIMPID => { /* RO */ }
        CSR_CYCLE | CSR_CYCLEH | CSR_TIME | CSR_TIMEH | CSR_INSTRET | CSR_INSTRETH => { /* RO */ }
        0x3A0..=0x3EF => { /* PMP stubs */ }
        _ => return None,
    }
    Some(())
}

// ── Trap / interrupt delivery ──────────────────────────────────────────

fn take_trap(cpu: &mut Cpu, cause: u32, tval: u32) {
    cpu.mepc = cpu.pc;
    cpu.mcause = cause;
    cpu.mtval = tval;

    // mstatus: MPIE = MIE; MIE = 0; MPP = 11 (machine)
    let mie = (cpu.mstatus >> 3) & 1;
    cpu.mstatus &= !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP_MASK);
    cpu.mstatus |= mie << 7;
    cpu.mstatus |= 0b11 << 11;

    let base = cpu.mtvec & !3;
    let mode = cpu.mtvec & 3;
    if mode == 1 && (cause & MCAUSE_INT) != 0 {
        // Vectored mode, interrupt
        cpu.pc = base.wrapping_add((cause & 0x7FFF_FFFF) * 4);
    } else {
        cpu.pc = base;
    }
}

fn maybe_take_interrupt(cpu: &mut Cpu, clint: &Clint) -> bool {
    if (cpu.mstatus & MSTATUS_MIE) == 0 {
        return false;
    }
    // Priority: MEI > MSI > MTI
    if cpu.mip_external && (cpu.mie & MIE_MEIE) != 0 {
        take_trap(cpu, MCAUSE_INT | IRQ_MEI, 0);
        return true;
    }
    if (cpu.mip_software || clint.msip != 0) && (cpu.mie & MIE_MSIE) != 0 {
        take_trap(cpu, MCAUSE_INT | IRQ_MSI, 0);
        return true;
    }
    if clint.timer_pending() && (cpu.mie & MIE_MTIE) != 0 {
        take_trap(cpu, MCAUSE_INT | IRQ_MTI, 0);
        return true;
    }
    false
}

// ── Stdin reader thread ────────────────────────────────────────────────

// ── Sensor feed thread ──────────────────────────────────────────────────
//
// Reads from a named pipe (FIFO) created by EdgeFabric and forwards bytes
// into the guest UART RX queue.  Lines are expected to be VeerOS shell
// commands such as `sensor set temperature 2500\n`.
//
// The thread stays alive across FIFO re-opens: when the writer closes the
// FIFO the blocking `open()` will succeed with the next writer.  We reopen
// in a loop so EdgeFabric can inject multiple times without recreating
// the process.

fn sensor_feed_thread(path: std::path::PathBuf, rx_queue: Arc<Mutex<VecDeque<u8>>>) {
    use std::io::Read;
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) { return; }

        // Blocking open: waits until a writer opens the FIFO.
        let f = match std::fs::OpenOptions::new().read(true).open(&path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[veer-vm] sensor-feed: cannot open {:?}: {e}", path);
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };

        let mut reader = std::io::BufReader::new(f);
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break, // EOF/writer closed — reopen
                Ok(n) => {
                    let mut q = rx_queue.lock().unwrap();
                    for &b in &buf[..n] {
                        q.push_back(b);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }
}

fn reader_thread(
    rx_queue: Arc<Mutex<VecDeque<u8>>>,
    main_tid: libc::pthread_t,
    interactive: bool,
) {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut buf = [0u8; 1];
    let mut escape = false;

    loop {
        match stdin.read(&mut buf) {
            Ok(0) => return,
            Ok(_) => {
                let b = buf[0];
                if interactive && escape {
                    escape = false;
                    match b {
                        b'x' | b'X' => {
                            vm::request_shutdown(main_tid);
                            return;
                        }
                        0x01 => {
                            rx_queue.lock().unwrap().push_back(0x01);
                        }
                        _ => {}
                    }
                } else if interactive && b == 0x01 {
                    escape = true;
                } else {
                    rx_queue.lock().unwrap().push_back(b);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

//! KVM VM + vCPU orchestration.
//!
//! Firecracker-style direct entry into 32-bit protected mode, matching the
//! Multiboot v1 boot state VeerOS expects (see `crates/kernel/qemu_pc/src/main.rs`
//! `_start:`). We set up segment registers for flat 32-bit PM via
//! `KVM_SET_SREGS`, load `EAX`/`EBX`/`EIP` via `KVM_SET_REGS`, then loop on
//! `KVM_RUN` dispatching PIO/MMIO exits to emulated devices.
//!
//! Phase 2 additions:
//!   * `KVM_CREATE_IRQCHIP` — in-kernel PIC + IOAPIC + LAPIC. Handles all
//!     `0xFEE0_xxxx` / `0xFEC0_xxxx` MMIO and CPU HLT waits internally.
//!   * `KVM_CREATE_PIT2` — in-kernel 8254 PIT (the scheduler timer).
//!   * UART RX — raw-mode stdin on a reader thread feeds the 16550 RBR and
//!     asserts IRQ 4 via `KVM_IRQ_LINE`. `Ctrl-A x` escape quits the VMM
//!     cleanly (like QEMU's monitor escape).

use anyhow::{bail, Context, Result};
use kvm_bindings::{
    kvm_pit_config, kvm_segment, kvm_userspace_memory_region, KVM_MAX_CPUID_ENTRIES,
};
use kvm_ioctls::{Kvm, VcpuExit, VcpuFd, VmFd};
use nix::sys::signal::{self, SaFlags, SigAction, SigHandler, SigSet, Signal};
use std::collections::VecDeque;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::config::{BootSource, GuestArch, VmConfig};
use crate::elf;
use crate::irq::IrqLine;
use crate::iso;
use crate::memory::GuestMem;
use crate::multiboot;
use crate::pci::{self, PciHost};
use crate::serial::{Serial16550, SerialShared};
use crate::snapshot;
use crate::termios_guard::RawMode;
use crate::virtio::{
    blk::VirtioBlk, net::VirtioNet, VirtioDevice, REG_DEVICE_CONFIG, REG_QUEUE_NOTIFY,
};

/// Global shutdown flag. Flipped by:
///   * SIGTERM / SIGHUP handlers
///   * The stdin reader thread on `Ctrl-A x` or read error
/// The run loop checks this at the top of each iteration.
pub(crate) static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Guest physical address where the Multiboot info struct is placed
/// (just below the 1 MiB kernel load address; outside any PT_LOAD).
const MBINFO_GPA: u64 = 0x9_F000;
const RISCV32_GPA_BASE: u64 = 0x8000_0000;
/// KVM AArch64 guest RAM base (QEMU `virt` machine convention).
const AARCH64_GPA_BASE: u64 = 0x4000_0000;

struct KvmIrqLine {
    vm: Arc<VmFd>,
}

impl KvmIrqLine {
    fn new(vm: Arc<VmFd>) -> Self {
        Self { vm }
    }
}

impl IrqLine for KvmIrqLine {
    fn set_irq_line(&self, line: u32, level: bool) {
        let _ = self.vm.set_irq_line(line, level);
    }
}

pub fn run(cfg: VmConfig) -> Result<()> {
    SHUTDOWN.store(false, Ordering::SeqCst);
    if cfg.guest_arch == GuestArch::Riscv32 {
        return run_riscv32(cfg);
    }

    let snapshot_meta = match &cfg.boot {
        BootSource::Snapshot(path) => {
            let meta = snapshot::load_meta(path)?;
            if meta.backend != "kvm" {
                bail!(
                    "snapshot backend '{}' is not compatible with KVM backend",
                    meta.backend
                );
            }
            Some(meta)
        }
        BootSource::Kernel(_) => None,
    };
    // ── 1. Open /dev/kvm ─────────────────────────────────────
    let kvm = Kvm::new().context("open /dev/kvm (is it present and are you in the kvm group?)")?;
    let api = kvm.get_api_version();
    if api != 12 {
        bail!("unexpected KVM API version {api} (expected 12)");
    }

    // ── 2. Create VM ─────────────────────────────────────────
    let vm: VmFd = kvm.create_vm().context("KVM_CREATE_VM")?;

    // On x86, the TSS address must be set before creating the irqchip
    // / any vCPU when running on Intel hosts without unrestricted_guest.
    vm.set_tss_address(0xfffb_d000)
        .context("KVM_SET_TSS_ADDR")?;

    // In-kernel IRQ chip: PIC + IOAPIC + LAPIC. Must come BEFORE any vCPU
    // creation so the vCPU gets an in-kernel LAPIC. After this call, the
    // kernel fully handles MMIO to LAPIC (0xFEE0_xxxx) and IOAPIC
    // (0xFEC0_xxxx) as well as CPU HLT waits.
    vm.create_irq_chip().context("KVM_CREATE_IRQCHIP")?;

    // In-kernel 8254 PIT (legacy interval timer).
    let pit_cfg = kvm_pit_config {
        flags: 0,
        pad: [0; 15],
    };
    vm.create_pit2(pit_cfg).context("KVM_CREATE_PIT2")?;

    // ── 3. Guest memory region ───────────────────────────────
    // Wrapped in Arc so the RX reader thread (for virtio-net) can copy
    // frames directly into guest physical memory without coordinating
    // with the vCPU thread. `GuestMem::slice_mut` already takes `&self`
    // and is safe for concurrent use (no aliasing by construction).
    let memory_bytes = snapshot_meta
        .as_ref()
        .map(|meta| meta.memory_bytes)
        .unwrap_or(cfg.memory_bytes);
    let guest_base = match cfg.guest_arch {
        GuestArch::X86_64 => 0,
        GuestArch::Riscv32 => RISCV32_GPA_BASE,
        GuestArch::Aarch64 => AARCH64_GPA_BASE,
    };
    let guest = Arc::new(GuestMem::new_with_base(guest_base, memory_bytes)?);
    let region = kvm_userspace_memory_region {
        slot: 0,
        flags: 0,
        guest_phys_addr: guest.gpa_base(),
        memory_size: guest.size() as u64,
        userspace_addr: guest.host_addr(),
    };
    // SAFETY: region lifetime is tied to `guest`, which outlives the VM.
    unsafe { vm.set_user_memory_region(region) }.context("KVM_SET_USER_MEMORY_REGION")?;

    // Share the VM fd so the UART can inject IRQ 4.
    let vm = Arc::new(vm);
    let irq_line: Arc<dyn IrqLine + Send + Sync> = Arc::new(KvmIrqLine::new(vm.clone()));

    let serial_shared = SerialShared::new();
    let uart = Arc::new(Mutex::new(Serial16550::stdout(
        serial_shared.clone(),
        irq_line,
    )));

    // ── 4. vCPU + CPUID ──────────────────────────────────────
    let mut vcpu: VcpuFd = vm.create_vcpu(0).context("KVM_CREATE_VCPU")?;
    let cpuid = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .context("KVM_GET_SUPPORTED_CPUID")?;
    vcpu.set_cpuid2(&cpuid).context("KVM_SET_CPUID2")?;

    let mut guest_entry = match &cfg.boot {
        BootSource::Kernel(kernel_path) => {
            if cfg.guest_arch == GuestArch::Riscv32 && is_iso_path(kernel_path) {
                bail!("--arch riscv32 requires a flat ELF kernel image; ISO boot is x86_64-only");
            }
            let loaded = if is_iso_path(kernel_path) {
                let kernel = iso::extract_boot_kernel(kernel_path)
                    .with_context(|| format!("extracting kernel from {}", kernel_path.display()))?;
                let image_name = format!("{}:/boot/kernel.elf", kernel_path.display());
                elf::load_bytes(&kernel, &image_name, guest.as_ref())
                    .with_context(|| format!("loading {image_name}"))?
            } else {
                elf::load(kernel_path, guest.as_ref())
                    .with_context(|| format!("loading {}", kernel_path.display()))?
            };
            eprintln!(
                "[veer-vm] loaded kernel {}: entry={:#x} end={:#x} memory={} MiB",
                kernel_path.display(),
                loaded.entry,
                loaded.end,
                memory_bytes / (1024 * 1024),
            );

            if cfg.guest_arch == GuestArch::Riscv32 {
                bail!(
                    "riscv32 kernel image loaded successfully, but the riscv32 KVM backend is not implemented yet"
                );
            }

            let mb_info = multiboot::write_info(guest.as_ref(), MBINFO_GPA, memory_bytes as u64)?;
            setup_sregs(&mut vcpu)?;
            setup_regs(&mut vcpu, loaded.entry, mb_info)?;
            loaded.entry
        }
        BootSource::Snapshot(snapshot_path) => {
            // Restore is applied after optional virtio devices are created.
            eprintln!(
                "[veer-vm] preparing snapshot restore {} (memory={} MiB)",
                snapshot_path.display(),
                memory_bytes / (1024 * 1024),
            );
            0
        }
    };

    // ── 5. Device model ──────────────────────────────────────

    // PCI host bridge + optional virtio-blk + optional virtio-net.
    let pci = Arc::new(Mutex::new(PciHost::new()));
    let blk: Option<Arc<Mutex<VirtioBlk>>> = if let Some(path) = cfg.disk_path.as_deref() {
        let dev = VirtioBlk::open(path, cfg.disk_read_only)
            .with_context(|| format!("opening disk image {}", path.display()))?;
        eprintln!(
            "[veer-vm] virtio-blk-pci: disk={} capacity={} sectors ({} MiB){}",
            path.display(),
            dev.capacity_sectors(),
            dev.capacity_sectors() * 512 / (1024 * 1024),
            if cfg.disk_read_only { " read-only" } else { "" },
        );
        let dev = Arc::new(Mutex::new(dev));
        pci.lock().unwrap().set_blk(dev.clone());
        Some(dev)
    } else {
        None
    };
    let net: Option<Arc<Mutex<VirtioNet>>> = if let Some(name) = cfg.tap_name.as_deref() {
        let dev = VirtioNet::open_tap(name, cfg.mac)
            .with_context(|| format!("attaching TAP interface {name}"))?;
        let tap_rx_fd = dev.dup_tap_fd()?;
        eprintln!(
            "[veer-vm] virtio-net-pci: tap={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            dev.tap_name(),
            cfg.mac[0],
            cfg.mac[1],
            cfg.mac[2],
            cfg.mac[3],
            cfg.mac[4],
            cfg.mac[5],
        );
        let dev = Arc::new(Mutex::new(dev));
        pci.lock().unwrap().set_net(dev.clone());

        // Spawn the RX reader thread: blocks on TAP `read(2)` and hands
        // each frame to the net device, which places it into the guest's
        // receiveq. Frames that arrive while no RX descriptor is posted
        // are dropped.
        let guest_rx = guest.clone();
        let dev_rx = dev.clone();
        thread::Builder::new()
            .name("veer-vm-net-rx".into())
            .spawn(move || net_rx_thread(tap_rx_fd, dev_rx, guest_rx))
            .context("spawn virtio-net rx thread")?;

        Some(dev)
    } else {
        None
    };

    if let BootSource::Snapshot(snapshot_path) = &cfg.boot {
        let mut blk_guard = match &blk {
            Some(device) => Some(device.lock().unwrap()),
            None => None,
        };
        let mut net_guard = match &net {
            Some(device) => Some(device.lock().unwrap()),
            None => None,
        };
        snapshot::restore(
            snapshot_path,
            guest.as_ref(),
            &mut vcpu,
            vm.as_ref(),
            &mut uart.lock().unwrap(),
            blk_guard.as_deref_mut(),
            cfg.disk_path.as_deref(),
            cfg.disk_read_only,
            net_guard.as_deref_mut(),
            cfg.tap_name.as_deref(),
            cfg.mac,
        )
        .with_context(|| format!("restoring snapshot {}", snapshot_path.display()))?;
        let regs = vcpu.get_regs().context("KVM_GET_REGS after restore")?;
        eprintln!(
            "[veer-vm] restored snapshot {}: rip={:#x} memory={} MiB",
            snapshot_path.display(),
            regs.rip,
            memory_bytes / (1024 * 1024),
        );
        guest_entry = regs.rip;
    }

    // ── 6. Install SIGUSR1 as a no-op so the reader thread can
    //       interrupt KVM_RUN cleanly. Also handle SIGTERM/SIGHUP.
    install_signal_handlers()?;

    // ── 7. Terminal raw mode + stdin reader thread ───────────
    let raw_guard = RawMode::enter()?;
    let interactive = raw_guard.is_active();
    if interactive {
        eprintln!("[veer-vm] raw mode engaged — press Ctrl-A x to quit");
    }
    // SAFETY: pthread_self always returns the calling thread's id; valid
    // until the thread exits, which for main means process exit.
    let main_tid: libc::pthread_t = unsafe { libc::pthread_self() };
    {
        let shared = serial_shared.clone();
        let uart = uart.clone();
        thread::Builder::new()
            .name("veer-vm-rx".into())
            .spawn(move || reader_thread(shared, uart, main_tid, interactive))
            .context("spawn stdin reader thread")?;
    }
    // Keep the raw-mode guard alive until function returns.
    let _raw_guard = raw_guard;

    eprintln!("[veer-vm] vcpu entering guest at {:#x}", guest_entry);

    // ── 8. Run loop ─────────────────────────────────────────
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            if let Some(path) = cfg.snapshot_save.as_deref() {
                let blk_guard = match &blk {
                    Some(device) => Some(device.lock().unwrap()),
                    None => None,
                };
                let net_guard = match &net {
                    Some(device) => Some(device.lock().unwrap()),
                    None => None,
                };
                snapshot::save(
                    path,
                    guest.as_ref(),
                    &vcpu,
                    vm.as_ref(),
                    &uart.lock().unwrap(),
                    blk_guard.as_deref(),
                    cfg.disk_path.as_deref(),
                    cfg.disk_read_only,
                    net_guard.as_deref(),
                    cfg.tap_name.as_deref(),
                    cfg.mac,
                )
                .with_context(|| format!("saving snapshot {}", path.display()))?;
                eprintln!("\r\n[veer-vm] snapshot saved to {}", path.display());
            }
            eprintln!("\r\n[veer-vm] shutdown requested — stopping guest");
            return Ok(());
        }
        match vcpu.run() {
            Ok(VcpuExit::IoIn(port, data)) => {
                if (0x3F8..=0x3FF).contains(&port) {
                    uart.lock().unwrap().io_in(port, data);
                } else if pio_is_pci_config(port) {
                    handle_pci_in(&pci, port, data);
                } else if let Some(off) = pio_blk_bar0_offset(&pci, port) {
                    handle_blk_bar0_in(off, data, blk.as_ref());
                } else if let Some(off) = pio_net_bar0_offset(&pci, port) {
                    handle_net_bar0_in(off, data, net.as_ref());
                } else {
                    for b in data.iter_mut() {
                        *b = 0xFF;
                    }
                }
            }
            Ok(VcpuExit::IoOut(port, data)) => {
                if (0x3F8..=0x3FF).contains(&port) {
                    uart.lock().unwrap().io_out(port, data);
                } else if pio_is_pci_config(port) {
                    handle_pci_out(&pci, port, data);
                } else if let Some(off) = pio_blk_bar0_offset(&pci, port) {
                    handle_blk_bar0_out(off, data, blk.as_ref(), guest.as_ref())?;
                } else if let Some(off) = pio_net_bar0_offset(&pci, port) {
                    handle_net_bar0_out(off, data, net.as_ref(), guest.as_ref())?;
                }
                // else: silently drop writes to unmapped PIO.
            }
            Ok(VcpuExit::MmioRead(addr, data)) => {
                eprintln!(
                    "[veer-vm] unhandled MMIO read {:#x} len={}",
                    addr,
                    data.len()
                );
                for b in data.iter_mut() {
                    *b = 0;
                }
            }
            Ok(VcpuExit::MmioWrite(addr, data)) => {
                eprintln!(
                    "[veer-vm] unhandled MMIO write {:#x} len={}",
                    addr,
                    data.len()
                );
            }
            Ok(VcpuExit::Hlt) => {
                // Should not happen with in-kernel LAPIC (KVM handles HLT
                // internally and waits for an interrupt). If we get here,
                // treat it as a graceful stop.
                eprintln!("\r\n[veer-vm] guest executed HLT without in-kernel LAPIC wait");
                return Ok(());
            }
            Ok(VcpuExit::Shutdown) => {
                eprintln!("\r\n[veer-vm] guest triple-faulted / shutdown");
                return Ok(());
            }
            Ok(VcpuExit::SystemEvent(ev, _)) => {
                eprintln!("\r\n[veer-vm] guest system event {ev} — stopping");
                return Ok(());
            }
            Ok(VcpuExit::IoapicEoi(_)) | Ok(VcpuExit::Intr) | Ok(VcpuExit::IrqWindowOpen) => {
                // These are informational / resumable — loop back.
                continue;
            }
            Ok(VcpuExit::InternalError) => {
                bail!("KVM internal error");
            }
            Ok(other) => {
                eprintln!("[veer-vm] unhandled vcpu exit: {:?}", other);
                return Ok(());
            }
            Err(e) => {
                if e.errno() == libc::EINTR {
                    // Woken by signal (reader thread asking us to quit,
                    // or SIGTERM/SIGHUP). Re-check shutdown flag at loop top.
                    continue;
                }
                bail!("KVM_RUN failed: {e}");
            }
        }
    }
}

#[cfg(not(target_arch = "riscv64"))]
fn run_riscv32(cfg: VmConfig) -> Result<()> {
    crate::rv32_soft::run(cfg)
}

#[cfg(target_arch = "riscv64")]
fn run_riscv32(cfg: VmConfig) -> Result<()> {
    use kvm_bindings::kvm_vcpu_init;

    if matches!(cfg.boot, BootSource::Snapshot(_)) {
        bail!("snapshot/restore is currently x86_64-only");
    }
    if cfg.disk_path.is_some() || cfg.tap_name.is_some() {
        bail!("riscv32 backend currently supports kernel-only boot (no --disk/--tap yet)");
    }

    let kernel_path = match &cfg.boot {
        BootSource::Kernel(path) => path,
        BootSource::Snapshot(_) => unreachable!(),
    };
    if is_iso_path(kernel_path) {
        bail!("--arch riscv32 requires a flat ELF kernel image; ISO boot is x86_64-only");
    }

    let kvm = Kvm::new().context("open /dev/kvm (is it present and are you in the kvm group?)")?;
    let vm: VmFd = kvm.create_vm().context("KVM_CREATE_VM")?;

    let guest = Arc::new(GuestMem::new_with_base(RISCV32_GPA_BASE, cfg.memory_bytes)?);
    let region = kvm_userspace_memory_region {
        slot: 0,
        flags: 0,
        guest_phys_addr: guest.gpa_base(),
        memory_size: guest.size() as u64,
        userspace_addr: guest.host_addr(),
    };
    unsafe { vm.set_user_memory_region(region) }.context("KVM_SET_USER_MEMORY_REGION")?;

    let mut vcpu: VcpuFd = vm.create_vcpu(0).context("KVM_CREATE_VCPU")?;
    let mut vcpu_init = kvm_vcpu_init::default();
    vm.get_preferred_target(&mut vcpu_init)
        .context("KVM_ARM_PREFERRED_TARGET / riscv preferred target")?;
    vcpu.vcpu_init(&vcpu_init)
        .context("KVM_ARM_VCPU_INIT / riscv vcpu_init")?;

    let loaded = elf::load(kernel_path, guest.as_ref())
        .with_context(|| format!("loading {}", kernel_path.display()))?;
    eprintln!(
        "[veer-vm] loaded riscv32 kernel {}: entry={:#x} end={:#x} memory={} MiB",
        kernel_path.display(),
        loaded.entry,
        loaded.end,
        cfg.memory_bytes / (1024 * 1024),
    );

    const RISCV_CORE_REG_BASE: u64 = 0x8030_0000_0200_0000;
    const RISCV_REG_PC: u64 = RISCV_CORE_REG_BASE;
    const RISCV_REG_A0: u64 = RISCV_CORE_REG_BASE + 10;
    const RISCV_REG_A1: u64 = RISCV_CORE_REG_BASE + 11;

    vcpu.set_one_reg(RISCV_REG_PC, &(loaded.entry as u64).to_le_bytes())
        .context("set riscv pc")?;
    vcpu.set_one_reg(RISCV_REG_A0, &(0u64).to_le_bytes())
        .context("set riscv a0 (hartid)")?;
    vcpu.set_one_reg(RISCV_REG_A1, &(0u64).to_le_bytes())
        .context("set riscv a1 (dtb ptr)")?;

    install_signal_handlers()?;
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
            .name("veer-vm-riscv-rx".into())
            .spawn(move || riscv_reader_thread(q, main_tid, interactive))
            .context("spawn riscv stdin reader thread")?;
    }

    let clint = ClintModel::new();
    eprintln!(
        "[veer-vm] vcpu entering riscv32 guest at {:#x}",
        loaded.entry
    );

    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            eprintln!("\r\n[veer-vm] shutdown requested — stopping guest");
            return Ok(());
        }

        match vcpu.run() {
            Ok(VcpuExit::MmioRead(addr, data)) => {
                if !handle_riscv_mmio_read(addr, data, &rx_queue, &clint) {
                    eprintln!(
                        "[veer-vm] unhandled RISC-V MMIO read {:#x} len={}",
                        addr,
                        data.len()
                    );
                    for b in data.iter_mut() {
                        *b = 0;
                    }
                }
            }
            Ok(VcpuExit::MmioWrite(addr, data)) => {
                if !handle_riscv_mmio_write(addr, data, &clint)? {
                    eprintln!(
                        "[veer-vm] unhandled RISC-V MMIO write {:#x} len={}",
                        addr,
                        data.len()
                    );
                }
            }
            Ok(VcpuExit::Hlt) | Ok(VcpuExit::Shutdown) => return Ok(()),
            Ok(VcpuExit::Intr) | Ok(VcpuExit::IrqWindowOpen) | Ok(VcpuExit::IoapicEoi(_)) => {
                continue
            }
            Ok(other) => {
                eprintln!("[veer-vm] unhandled riscv vcpu exit: {:?}", other);
                return Ok(());
            }
            Err(e) => {
                if e.errno() == libc::EINTR {
                    continue;
                }
                bail!("KVM_RUN failed: {e}");
            }
        }
    }
}

#[cfg(target_arch = "riscv64")]
const RISCV_UART_BASE: u64 = 0x1000_0000;
#[cfg(target_arch = "riscv64")]
const RISCV_UART_RBR_THR: u64 = 0x00;
#[cfg(target_arch = "riscv64")]
const RISCV_UART_LSR: u64 = 0x05;
#[cfg(target_arch = "riscv64")]
const RISCV_UART_LSR_DR: u8 = 1 << 0;
#[cfg(target_arch = "riscv64")]
const RISCV_UART_LSR_THRE: u8 = 1 << 5;

#[cfg(target_arch = "riscv64")]
const RISCV_CLINT_BASE: u64 = 0x0200_0000;
#[cfg(target_arch = "riscv64")]
const RISCV_CLINT_MTIMECMP: u64 = RISCV_CLINT_BASE + 0x4000;
#[cfg(target_arch = "riscv64")]
const RISCV_CLINT_MTIME: u64 = RISCV_CLINT_BASE + 0xBFF8;
#[cfg(target_arch = "riscv64")]
const RISCV_CLINT_FREQ_HZ: u64 = 10_000_000;

#[cfg(target_arch = "riscv64")]
struct ClintModel {
    start: Instant,
    mtimecmp: Mutex<u64>,
}

#[cfg(target_arch = "riscv64")]
impl ClintModel {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            mtimecmp: Mutex::new(u64::MAX),
        }
    }

    fn mtime(&self) -> u64 {
        let ns = self.start.elapsed().as_nanos() as u128;
        ((ns * RISCV_CLINT_FREQ_HZ as u128) / 1_000_000_000u128) as u64
    }
}

#[cfg(target_arch = "riscv64")]
fn handle_riscv_mmio_read(
    addr: u64,
    data: &mut [u8],
    rx_queue: &Arc<Mutex<VecDeque<u8>>>,
    clint: &ClintModel,
) -> bool {
    if addr >= RISCV_UART_BASE && addr < RISCV_UART_BASE + 0x100 {
        let off = addr - RISCV_UART_BASE;
        match off {
            RISCV_UART_RBR_THR => {
                let b = rx_queue.lock().unwrap().pop_front().unwrap_or(0);
                for v in data.iter_mut() {
                    *v = 0;
                }
                if !data.is_empty() {
                    data[0] = b;
                }
                return true;
            }
            RISCV_UART_LSR => {
                let has_data = !rx_queue.lock().unwrap().is_empty();
                let mut lsr = RISCV_UART_LSR_THRE;
                if has_data {
                    lsr |= RISCV_UART_LSR_DR;
                }
                for v in data.iter_mut() {
                    *v = 0;
                }
                if !data.is_empty() {
                    data[0] = lsr;
                }
                return true;
            }
            _ => {
                for v in data.iter_mut() {
                    *v = 0;
                }
                return true;
            }
        }
    }

    if addr >= RISCV_CLINT_BASE && addr < RISCV_CLINT_BASE + 0x10000 {
        let value = if addr == RISCV_CLINT_MTIME {
            clint.mtime() as u32
        } else if addr == RISCV_CLINT_MTIME + 4 {
            (clint.mtime() >> 32) as u32
        } else if addr == RISCV_CLINT_MTIMECMP {
            *clint.mtimecmp.lock().unwrap() as u32
        } else if addr == RISCV_CLINT_MTIMECMP + 4 {
            (*clint.mtimecmp.lock().unwrap() >> 32) as u32
        } else {
            0
        };
        let le = value.to_le_bytes();
        let n = data.len().min(4);
        data[..n].copy_from_slice(&le[..n]);
        for v in &mut data[n..] {
            *v = 0;
        }
        return true;
    }

    false
}

#[cfg(target_arch = "riscv64")]
fn handle_riscv_mmio_write(addr: u64, data: &[u8], clint: &ClintModel) -> Result<bool> {
    if addr >= RISCV_UART_BASE && addr < RISCV_UART_BASE + 0x100 {
        let off = addr - RISCV_UART_BASE;
        if off == RISCV_UART_RBR_THR && !data.is_empty() {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            out.write_all(&data[..1]).context("uart stdout write")?;
            out.flush().context("uart stdout flush")?;
        }
        return Ok(true);
    }

    if addr >= RISCV_CLINT_BASE && addr < RISCV_CLINT_BASE + 0x10000 {
        let mut bytes = [0u8; 4];
        let n = data.len().min(4);
        bytes[..n].copy_from_slice(&data[..n]);
        let v = u32::from_le_bytes(bytes) as u64;
        if addr == RISCV_CLINT_MTIMECMP {
            let mut cur = clint.mtimecmp.lock().unwrap();
            *cur = (*cur & !0xFFFF_FFFFu64) | v;
        } else if addr == RISCV_CLINT_MTIMECMP + 4 {
            let mut cur = clint.mtimecmp.lock().unwrap();
            *cur = (*cur & 0xFFFF_FFFFu64) | (v << 32);
        }
        return Ok(true);
    }

    Ok(false)
}

#[cfg(target_arch = "riscv64")]
fn riscv_reader_thread(
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
                            request_shutdown(main_tid);
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

fn is_iso_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("iso"))
        .unwrap_or(false)
}

/// Configure segment / system registers for a flat 32-bit PM entry.
fn setup_sregs(vcpu: &mut VcpuFd) -> Result<()> {
    let mut sregs = vcpu.get_sregs().context("KVM_GET_SREGS")?;

    // Code: base=0, limit=4GiB (G=1), DPL=0, P=1, S=1, type=0xB, DB=1.
    let code = kvm_segment {
        base: 0,
        limit: 0xFFFF_FFFF,
        selector: 0x08,
        type_: 0xB,
        present: 1,
        dpl: 0,
        db: 1,
        s: 1,
        l: 0,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    };
    // Data: type=0x3 (read/write/accessed).
    let data = kvm_segment {
        base: 0,
        limit: 0xFFFF_FFFF,
        selector: 0x10,
        type_: 0x3,
        present: 1,
        dpl: 0,
        db: 1,
        s: 1,
        l: 0,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    };

    sregs.cs = code;
    sregs.ds = data;
    sregs.es = data;
    sregs.fs = data;
    sregs.gs = data;
    sregs.ss = data;

    // Multiboot requires PE=1, PG=0. CR0 bit 0 = PE, bit 4 = ET (reserved,
    // must be 1 on P6+).
    sregs.cr0 = 0x0000_0011;
    sregs.cr2 = 0;
    sregs.cr3 = 0;
    sregs.cr4 = 0;
    sregs.efer = 0;

    vcpu.set_sregs(&sregs).context("KVM_SET_SREGS")?;
    Ok(())
}

fn setup_regs(vcpu: &mut VcpuFd, entry: u64, mb_info: u64) -> Result<()> {
    let mut regs = vcpu.get_regs().context("KVM_GET_REGS")?;
    regs.rip = entry;
    regs.rax = multiboot::MULTIBOOT1_BOOTLOADER_MAGIC as u64;
    regs.rbx = mb_info;
    regs.rflags = 0x2; // bit 1 reserved, must be 1
    regs.rsp = 0;
    vcpu.set_regs(&regs).context("KVM_SET_REGS")?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════
// Signal handling
// ═══════════════════════════════════════════════════════════════════════

extern "C" fn sigusr1_handler(_: libc::c_int) {
    // Intentionally empty — we only want to interrupt blocking syscalls
    // (specifically KVM_RUN's ioctl) with EINTR.
}

extern "C" fn sigterm_handler(_: libc::c_int) {
    // Set the global flag so the run loop exits on the next iteration.
    // The EINTR from KVM_RUN (signals are not SA_RESTART) takes us back
    // to the top of the loop which re-checks SHUTDOWN.
    SHUTDOWN.store(true, Ordering::SeqCst);
}

pub(crate) fn install_signal_handlers() -> Result<()> {
    // SIGUSR1 — no-op, wakes KVM_RUN.
    let usr1 = SigAction::new(
        SigHandler::Handler(sigusr1_handler),
        SaFlags::empty(), // deliberately no SA_RESTART
        SigSet::empty(),
    );
    unsafe {
        signal::sigaction(Signal::SIGUSR1, &usr1).context("sigaction(SIGUSR1)")?;
    }
    // SIGTERM / SIGHUP — flag + wake.
    let term = SigAction::new(
        SigHandler::Handler(sigterm_handler),
        SaFlags::empty(),
        SigSet::empty(),
    );
    unsafe {
        signal::sigaction(Signal::SIGTERM, &term).context("sigaction(SIGTERM)")?;
        signal::sigaction(Signal::SIGHUP, &term).context("sigaction(SIGHUP)")?;
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════
// Stdin reader thread
// ═══════════════════════════════════════════════════════════════════════
//
// Reads raw bytes from stdin one at a time, pushes them into the UART's
// RX queue and kicks the IRQ line. Handles a Ctrl-A x escape sequence
// (like QEMU) to quit the VMM.

fn reader_thread(
    shared: Arc<SerialShared>,
    uart: Arc<Mutex<Serial16550>>,
    main_tid: libc::pthread_t,
    interactive: bool,
) {
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut buf = [0u8; 1];
    let mut escape = false;

    loop {
        match stdin.read(&mut buf) {
            Ok(0) => {
                // stdin EOF — stop reading but DO NOT shut the guest
                // down. Without a TTY there's nothing more to deliver;
                // the guest should run until it halts or we get SIGTERM.
                return;
            }
            Ok(_) => {
                let b = buf[0];
                if interactive && escape {
                    escape = false;
                    match b {
                        b'x' | b'X' => {
                            request_shutdown(main_tid);
                            return;
                        }
                        0x01 => {
                            // Literal Ctrl-A — forward to guest.
                            push_byte(&shared, &uart, 0x01);
                        }
                        _ => { /* swallow unknown escape */ }
                    }
                } else if interactive && b == 0x01 {
                    // Enter escape mode (Ctrl-A prefix).
                    escape = true;
                } else {
                    push_byte(&shared, &uart, b);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

fn push_byte(shared: &Arc<SerialShared>, uart: &Arc<Mutex<Serial16550>>, b: u8) {
    shared.rx.lock().unwrap().push_back(b);
    uart.lock().unwrap().kick_rx();
}

pub(crate) fn request_shutdown(main_tid: libc::pthread_t) {
    SHUTDOWN.store(true, Ordering::SeqCst);
    // SAFETY: `main_tid` is a valid pthread_t captured before the thread
    // was spawned; the main thread is alive until we exit below.
    unsafe {
        libc::pthread_kill(main_tid, libc::SIGUSR1);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// PIO dispatch — PCI config mechanism #1 and virtio-blk BAR0
// ═══════════════════════════════════════════════════════════════════════
//
// The guest reaches the emulated PCI host bridge through the classic
// pair of I/O ports (0xCF8 / 0xCFC), and reaches the virtio-blk
// transport through its I/O BAR0 (base programmed by the guest during
// enumeration, defaulting to `VIRTIO_BLK_PIO_BASE`). Reads/writes to any
// other unmapped port are silently dropped (writes) or return all-ones
// (reads) to match bare-metal PCI behaviour.

/// True if `port` falls anywhere inside the CONFIG_ADDRESS or
/// CONFIG_DATA dwords (0xCF8..=0xCFF).
fn pio_is_pci_config(port: u16) -> bool {
    (pci::PCI_CONFIG_ADDR..pci::PCI_CONFIG_ADDR + 4).contains(&port)
        || (pci::PCI_CONFIG_DATA..pci::PCI_CONFIG_DATA + 4).contains(&port)
}

/// If `port` falls inside the currently-programmed virtio-blk BAR0
/// window, return the offset within that BAR; else `None`.
fn pio_blk_bar0_offset(pci: &Arc<Mutex<PciHost>>, port: u16) -> Option<u16> {
    let base = pci.lock().unwrap().blk_bar0_base();
    if base == 0 {
        return None;
    }
    let end = base.saturating_add(pci::VIRTIO_BLK_PIO_SIZE);
    if port >= base && port < end {
        Some(port - base)
    } else {
        None
    }
}

/// If `port` falls inside the currently-programmed virtio-net BAR0
/// window, return the offset within that BAR; else `None`.
fn pio_net_bar0_offset(pci: &Arc<Mutex<PciHost>>, port: u16) -> Option<u16> {
    let base = pci.lock().unwrap().net_bar0_base();
    if base == 0 {
        return None;
    }
    let end = base.saturating_add(pci::VIRTIO_NET_PIO_SIZE);
    if port >= base && port < end {
        Some(port - base)
    } else {
        None
    }
}

fn handle_pci_in(pci: &Arc<Mutex<PciHost>>, port: u16, data: &mut [u8]) {
    let host = pci.lock().unwrap();
    if (pci::PCI_CONFIG_ADDR..pci::PCI_CONFIG_ADDR + 4).contains(&port) {
        let val = host.read_addr();
        let bytes = val.to_le_bytes();
        let byte_off = (port - pci::PCI_CONFIG_ADDR) as usize;
        let n = data.len().min(4 - byte_off);
        data[..n].copy_from_slice(&bytes[byte_off..byte_off + n]);
        for b in &mut data[n..] {
            *b = 0xFF;
        }
    } else {
        host.read_data(data.len(), data);
    }
}

fn handle_pci_out(pci: &Arc<Mutex<PciHost>>, port: u16, data: &[u8]) {
    let mut host = pci.lock().unwrap();
    if (pci::PCI_CONFIG_ADDR..pci::PCI_CONFIG_ADDR + 4).contains(&port) {
        let mut bytes = host.read_addr().to_le_bytes();
        let byte_off = (port - pci::PCI_CONFIG_ADDR) as usize;
        let n = data.len().min(4 - byte_off);
        bytes[byte_off..byte_off + n].copy_from_slice(&data[..n]);
        host.write_addr(u32::from_le_bytes(bytes));
    } else {
        host.write_data(data.len(), data);
    }
}

fn handle_blk_bar0_in(offset: u16, data: &mut [u8], blk: Option<&Arc<Mutex<VirtioBlk>>>) {
    let Some(dev) = blk else {
        for b in data.iter_mut() {
            *b = 0xFF;
        }
        return;
    };
    let mut dev = dev.lock().unwrap();
    if offset < REG_DEVICE_CONFIG {
        dev.transport_mut().read_reg(offset, data);
    } else {
        dev.config_read(offset - REG_DEVICE_CONFIG, data);
    }
}

fn handle_blk_bar0_out(
    offset: u16,
    data: &[u8],
    blk: Option<&Arc<Mutex<VirtioBlk>>>,
    mem: &GuestMem,
) -> Result<()> {
    let Some(dev) = blk else {
        return Ok(());
    };
    if offset < REG_DEVICE_CONFIG {
        if offset == REG_QUEUE_NOTIFY {
            let mut bytes = [0u8; 2];
            let n = data.len().min(2);
            bytes[..n].copy_from_slice(&data[..n]);
            let qidx = u16::from_le_bytes(bytes);
            dev.lock().unwrap().notify(qidx, mem)?;
        } else {
            dev.lock().unwrap().transport_mut().write_reg(offset, data);
        }
    }
    Ok(())
}

fn handle_net_bar0_in(offset: u16, data: &mut [u8], net: Option<&Arc<Mutex<VirtioNet>>>) {
    let Some(dev) = net else {
        for b in data.iter_mut() {
            *b = 0xFF;
        }
        return;
    };
    let mut dev = dev.lock().unwrap();
    if offset < REG_DEVICE_CONFIG {
        dev.transport_mut().read_reg(offset, data);
    } else {
        dev.config_read(offset - REG_DEVICE_CONFIG, data);
    }
}

fn handle_net_bar0_out(
    offset: u16,
    data: &[u8],
    net: Option<&Arc<Mutex<VirtioNet>>>,
    mem: &GuestMem,
) -> Result<()> {
    let Some(dev) = net else {
        return Ok(());
    };
    if offset < REG_DEVICE_CONFIG {
        if offset == REG_QUEUE_NOTIFY {
            let mut bytes = [0u8; 2];
            let n = data.len().min(2);
            bytes[..n].copy_from_slice(&data[..n]);
            let qidx = u16::from_le_bytes(bytes);
            dev.lock().unwrap().notify(qidx, mem)?;
        } else {
            dev.lock().unwrap().transport_mut().write_reg(offset, data);
        }
    }
    Ok(())
}

/// RX reader thread for virtio-net. Blocks on the TAP fd (dup'd from the
/// device) and hands each Ethernet frame to `VirtioNet::deliver_rx_frame`,
/// which places it into the guest's receiveq. Frames that arrive with no
/// posted RX descriptor are dropped.
fn net_rx_thread(tap_fd: libc::c_int, dev: Arc<Mutex<VirtioNet>>, mem: Arc<GuestMem>) {
    // TAP was opened O_NONBLOCK in the VMM; use poll(2) to block cheaply.
    let mut buf = vec![0u8; 2048];
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            unsafe {
                libc::close(tap_fd);
            }
            return;
        }
        // Wait up to 200ms for TAP readability, then re-check SHUTDOWN.
        let mut pfd = libc::pollfd {
            fd: tap_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd as *mut _, 1, 200) };
        if rc < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            eprintln!("[veer-vm] virtio-net rx: poll: {e}");
            unsafe {
                libc::close(tap_fd);
            }
            return;
        }
        if rc == 0 || (pfd.revents & libc::POLLIN) == 0 {
            continue;
        }

        loop {
            let n = unsafe { libc::read(tap_fd, buf.as_mut_ptr() as _, buf.len()) };
            if n < 0 {
                let e = std::io::Error::last_os_error();
                if matches!(
                    e.raw_os_error(),
                    Some(libc::EAGAIN) | Some(libc::EWOULDBLOCK)
                ) {
                    break;
                }
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                eprintln!("[veer-vm] virtio-net rx: read: {e}");
                unsafe {
                    libc::close(tap_fd);
                }
                return;
            }
            if n == 0 {
                break;
            }
            if std::env::var_os("VEER_VM_NET_TRACE").is_some() {
                eprintln!("[veer-vm/net] rx-read: {n} bytes from tap");
            }
            let frame = &buf[..n as usize];
            let mut dev = dev.lock().unwrap();
            if let Err(e) = dev.deliver_rx_frame(mem.as_ref(), frame) {
                eprintln!("[veer-vm] virtio-net rx: deliver: {e:#}");
            }
        }
    }
}

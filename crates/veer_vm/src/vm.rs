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
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::elf;
use crate::memory::GuestMem;
use crate::multiboot;
use crate::pci::{self, PciHost};
use crate::serial::{Serial16550, SerialShared};
use crate::termios_guard::RawMode;
use crate::virtio::{blk::VirtioBlk, VirtioDevice, REG_DEVICE_CONFIG, REG_QUEUE_NOTIFY};

/// Global shutdown flag. Flipped by:
///   * SIGTERM / SIGHUP handlers
///   * The stdin reader thread on `Ctrl-A x` or read error
/// The run loop checks this at the top of each iteration.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Guest physical address where the Multiboot info struct is placed
/// (just below the 1 MiB kernel load address; outside any PT_LOAD).
const MBINFO_GPA: u64 = 0x9_F000;

pub struct VmConfig {
    pub kernel_path: std::path::PathBuf,
    pub memory_bytes: usize,
    pub disk_path: Option<std::path::PathBuf>,
    pub disk_read_only: bool,
}

pub fn run(cfg: VmConfig) -> Result<()> {
    // ── 1. Open /dev/kvm ─────────────────────────────────────
    let kvm = Kvm::new()
        .context("open /dev/kvm (is it present and are you in the kvm group?)")?;
    let api = kvm.get_api_version();
    if api != 12 {
        bail!("unexpected KVM API version {api} (expected 12)");
    }

    // ── 2. Create VM ─────────────────────────────────────────
    let vm: VmFd = kvm.create_vm().context("KVM_CREATE_VM")?;

    // On x86, the TSS address must be set before creating the irqchip
    // / any vCPU when running on Intel hosts without unrestricted_guest.
    vm.set_tss_address(0xfffb_d000).context("KVM_SET_TSS_ADDR")?;

    // In-kernel IRQ chip: PIC + IOAPIC + LAPIC. Must come BEFORE any vCPU
    // creation so the vCPU gets an in-kernel LAPIC. After this call, the
    // kernel fully handles MMIO to LAPIC (0xFEE0_xxxx) and IOAPIC
    // (0xFEC0_xxxx) as well as CPU HLT waits.
    vm.create_irq_chip().context("KVM_CREATE_IRQCHIP")?;

    // In-kernel 8254 PIT (legacy interval timer).
    let pit_cfg = kvm_pit_config { flags: 0, pad: [0; 15] };
    vm.create_pit2(pit_cfg).context("KVM_CREATE_PIT2")?;

    // ── 3. Guest memory region ───────────────────────────────
    let guest = GuestMem::new(cfg.memory_bytes)?;
    let region = kvm_userspace_memory_region {
        slot: 0,
        flags: 0,
        guest_phys_addr: 0,
        memory_size: guest.size() as u64,
        userspace_addr: guest.host_addr(),
    };
    // SAFETY: region lifetime is tied to `guest`, which outlives the VM.
    unsafe { vm.set_user_memory_region(region) }
        .context("KVM_SET_USER_MEMORY_REGION")?;

    // ── 4. Load kernel ELF and place Multiboot info ──────────
    let loaded = elf::load(&cfg.kernel_path, &guest)
        .with_context(|| format!("loading {}", cfg.kernel_path.display()))?;
    eprintln!(
        "[veer-vm] loaded kernel {}: entry={:#x} end={:#x} memory={} MiB",
        cfg.kernel_path.display(),
        loaded.entry,
        loaded.end,
        cfg.memory_bytes / (1024 * 1024),
    );
    let mb_info = multiboot::write_info(&guest, MBINFO_GPA, cfg.memory_bytes as u64)?;

    // ── 5. vCPU + CPUID ──────────────────────────────────────
    let mut vcpu: VcpuFd = vm.create_vcpu(0).context("KVM_CREATE_VCPU")?;
    let cpuid = kvm.get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .context("KVM_GET_SUPPORTED_CPUID")?;
    vcpu.set_cpuid2(&cpuid).context("KVM_SET_CPUID2")?;

    setup_sregs(&mut vcpu)?;
    setup_regs(&mut vcpu, loaded.entry, mb_info)?;

    // ── 6. Share vm handle (for IRQ injection from reader thread) ──
    let vm = Arc::new(vm);

    // ── 7. Device model ──────────────────────────────────────
    let serial_shared = SerialShared::new();
    let uart = Arc::new(Mutex::new(Serial16550::stdout(
        serial_shared.clone(),
        vm.clone(),
    )));

    // PCI host bridge + optional virtio-blk.
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

    // ── 8. Install SIGUSR1 as a no-op so the reader thread can
    //       interrupt KVM_RUN cleanly. Also handle SIGTERM/SIGHUP.
    install_signal_handlers()?;

    // ── 9. Terminal raw mode + stdin reader thread ───────────
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

    eprintln!("[veer-vm] vcpu entering guest at {:#x}", loaded.entry);

    // ── 10. Run loop ─────────────────────────────────────────
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            eprintln!("\r\n[veer-vm] shutdown requested — stopping guest");
            return Ok(());
        }
        match vcpu.run() {
            Ok(VcpuExit::IoIn(port, data)) => {
                if (0x3F8..=0x3FF).contains(&port) {
                    uart.lock().unwrap().io_in(port, data);
                } else if pio_is_pci_config(port) {
                    handle_pci_in(&pci, port, data);
                } else if let Some(off) = pio_bar0_offset(&pci, port) {
                    handle_bar0_in(off, data, blk.as_ref());
                } else {
                    for b in data.iter_mut() { *b = 0xFF; }
                }
            }
            Ok(VcpuExit::IoOut(port, data)) => {
                if (0x3F8..=0x3FF).contains(&port) {
                    uart.lock().unwrap().io_out(port, data);
                } else if pio_is_pci_config(port) {
                    handle_pci_out(&pci, port, data);
                } else if let Some(off) = pio_bar0_offset(&pci, port) {
                    handle_bar0_out(off, data, blk.as_ref(), &guest)?;
                }
                // else: silently drop writes to unmapped PIO.
            }
            Ok(VcpuExit::MmioRead(addr, data)) => {
                eprintln!("[veer-vm] unhandled MMIO read {:#x} len={}", addr, data.len());
                for b in data.iter_mut() { *b = 0; }
            }
            Ok(VcpuExit::MmioWrite(addr, data)) => {
                eprintln!("[veer-vm] unhandled MMIO write {:#x} len={}", addr, data.len());
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

/// Configure segment / system registers for a flat 32-bit PM entry.
fn setup_sregs(vcpu: &mut VcpuFd) -> Result<()> {
    let mut sregs = vcpu.get_sregs().context("KVM_GET_SREGS")?;

    // Code: base=0, limit=4GiB (G=1), DPL=0, P=1, S=1, type=0xB, DB=1.
    let code = kvm_segment {
        base: 0, limit: 0xFFFF_FFFF, selector: 0x08,
        type_: 0xB, present: 1, dpl: 0, db: 1, s: 1, l: 0, g: 1,
        avl: 0, unusable: 0, padding: 0,
    };
    // Data: type=0x3 (read/write/accessed).
    let data = kvm_segment {
        base: 0, limit: 0xFFFF_FFFF, selector: 0x10,
        type_: 0x3, present: 1, dpl: 0, db: 1, s: 1, l: 0, g: 1,
        avl: 0, unusable: 0, padding: 0,
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

fn install_signal_handlers() -> Result<()> {
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

fn request_shutdown(main_tid: libc::pthread_t) {
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
fn pio_bar0_offset(pci: &Arc<Mutex<PciHost>>, port: u16) -> Option<u16> {
    let base = pci.lock().unwrap().bar0_base();
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

fn handle_pci_in(pci: &Arc<Mutex<PciHost>>, port: u16, data: &mut [u8]) {
    let host = pci.lock().unwrap();
    if (pci::PCI_CONFIG_ADDR..pci::PCI_CONFIG_ADDR + 4).contains(&port) {
        // Only 32-bit aligned reads of CONFIG_ADDRESS are meaningful;
        // anything else returns 0xFF..FF. Guest driver only does 4-byte.
        let val = host.read_addr();
        let bytes = val.to_le_bytes();
        let byte_off = (port - pci::PCI_CONFIG_ADDR) as usize;
        let n = data.len().min(4 - byte_off);
        data[..n].copy_from_slice(&bytes[byte_off..byte_off + n]);
        for b in &mut data[n..] { *b = 0xFF; }
    } else {
        // CONFIG_DATA — width is the access width; PciHost honours the
        // low two bits of the latched address to pick the right byte.
        host.read_data(data.len(), data);
    }
}

fn handle_pci_out(pci: &Arc<Mutex<PciHost>>, port: u16, data: &[u8]) {
    let mut host = pci.lock().unwrap();
    if (pci::PCI_CONFIG_ADDR..pci::PCI_CONFIG_ADDR + 4).contains(&port) {
        // Accept a full-dword write at the base, or patch sub-dword
        // fragments by read-modify-write.
        let mut bytes = host.read_addr().to_le_bytes();
        let byte_off = (port - pci::PCI_CONFIG_ADDR) as usize;
        let n = data.len().min(4 - byte_off);
        bytes[byte_off..byte_off + n].copy_from_slice(&data[..n]);
        host.write_addr(u32::from_le_bytes(bytes));
    } else {
        host.write_data(data.len(), data);
    }
}

fn handle_bar0_in(
    offset: u16,
    data: &mut [u8],
    blk: Option<&Arc<Mutex<VirtioBlk>>>,
) {
    let Some(dev) = blk else {
        for b in data.iter_mut() { *b = 0xFF; }
        return;
    };
    let mut dev = dev.lock().unwrap();
    if offset < REG_DEVICE_CONFIG {
        dev.transport_mut().read_reg(offset, data);
    } else {
        let cfg_off = offset - REG_DEVICE_CONFIG;
        dev.config_read(cfg_off, data);
    }
}

fn handle_bar0_out(
    offset: u16,
    data: &[u8],
    blk: Option<&Arc<Mutex<VirtioBlk>>>,
    mem: &GuestMem,
) -> Result<()> {
    let Some(dev) = blk else { return Ok(()); };
    if offset < REG_DEVICE_CONFIG {
        // QUEUE_NOTIFY is special: the write value is the queue index
        // to kick. All other common-header writes are straight register
        // updates handled by the transport.
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
    // Device config area is read-only for virtio-blk; ignore writes.
    Ok(())
}

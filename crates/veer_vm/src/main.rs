//! `veer-vm` — VeerOS microVMM.
//!
//! A lightweight Firecracker-class VMM that boots VeerOS kernels directly
//! via KVM, without QEMU. Phase 1: Multiboot v1 kernel + 16550 serial to
//! host stdout.
//!
//! Usage:
//!   veer-vm --kernel target/x86_64-unknown-none/debug/kernel-qemu-pc \
//!           --memory 128

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

mod backend;
mod config;
mod irq;

#[cfg(target_os = "linux")]
mod elf;
#[cfg(target_os = "linux")]
mod iso;
#[cfg(target_os = "linux")]
mod memory;
#[cfg(target_os = "linux")]
mod multiboot;
#[cfg(target_os = "linux")]
mod pci;
#[cfg(target_os = "linux")]
mod serial;
#[cfg(target_os = "linux")]
mod snapshot;
#[cfg(target_os = "linux")]
mod termios_guard;
#[cfg(target_os = "linux")]
mod virtio;
#[cfg(target_os = "linux")]
mod rv32_soft;
#[cfg(target_os = "linux")]
mod vm;

#[derive(Parser, Debug)]
#[command(name = "veer-vm", about = "VeerOS microVMM (KVM-based)")]
struct Cli {
    /// Path to the boot image to load.
    ///
    /// Accepts either:
    /// - a Multiboot v1-compatible kernel ELF, or
    /// - a VeerOS ISO containing `/boot/kernel.elf`.
    #[arg(long)]
    kernel: Option<PathBuf>,

    /// Restore from a snapshot directory created by `--snapshot-save`.
    #[arg(long, conflicts_with = "kernel")]
    restore: Option<PathBuf>,

    /// Save a snapshot directory when the VM shuts down cleanly.
    #[arg(long)]
    snapshot_save: Option<PathBuf>,

    /// Guest memory size, in MiB.
    #[arg(long, default_value_t = 128)]
    memory: usize,

    /// Number of virtual CPUs.
    #[arg(long, default_value_t = 1)]
    cpus: usize,

    /// Raw disk image to expose as virtio-blk-pci. Size must be a multiple
    /// of 512 bytes.
    #[arg(long)]
    disk: Option<PathBuf>,

    /// Mount the disk read-only (sets VIRTIO_BLK_F_RO).
    #[arg(long, default_value_t = false)]
    disk_ro: bool,

    /// Attach a virtio-net-pci device backed by an existing TAP interface.
    /// The TAP must be created out-of-band: e.g.
    ///   `sudo ip tuntap add dev tap0 mode tap user $USER && sudo ip link set tap0 up`.
    /// Pass the interface name (e.g. `--tap tap0`).
    #[arg(long)]
    tap: Option<String>,

    /// macOS-only host networking mode via vmnet.framework.
    ///
    /// `shared` uses host NAT; `host` creates host-only networking.
    /// Mutually exclusive with `--tap`.
    #[arg(long, value_enum)]
    vmnet: Option<VmnetModeArg>,

    /// MAC address to advertise to the guest (format `aa:bb:cc:dd:ee:ff`).
    /// Defaults to a locally-administered, randomly-seeded address.
    /// When restoring with `--tap` and no `--mac`, the MAC saved in the
    /// snapshot is reused automatically.
    #[arg(long)]
    mac: Option<String>,

    /// Guest architecture.
    #[arg(long, value_enum, default_value_t = ArchArg::X8664)]
    arch: ArchArg,

    /// Busy-loop throttle sleep (milliseconds) for riscv32 software mode.
    ///
    /// Applies only to `--arch riscv32` on non-riscv64 hosts (rv32-soft).
    /// When the guest stays runnable and does not block in WFI, veer-vm
    /// sleeps this long per throttle cycle to reduce host CPU usage.
    ///
    /// `0` disables the busy-loop throttle.
    #[arg(long, default_value_t = 4)]
    cpu_throttle_ms: u64,

    /// Path to a named pipe (FIFO) for sensor injection.
    ///
    /// When provided, a background thread reads lines from this FIFO and
    /// forwards them to the guest UART RX queue (as if typed at the console).
    /// This lets EdgeFabric inject `sensor set <name> <value>\n` commands
    /// into the running VeerOS guest without a network connection.
    #[arg(long)]
    sensor_feed: Option<PathBuf>,

    /// Run macOS Hypervisor.framework readiness checks and exit.
    #[arg(long, default_value_t = false)]
    hvf_preflight: bool,

    /// Run a minimal Hypervisor.framework VM create/destroy probe and exit.
    ///
    /// macOS-only diagnostic aid to distinguish host-level HVF contention
    /// from veer-vm runtime wiring.
    #[arg(long, default_value_t = false)]
    hvf_probe: bool,

    /// Alias for `--hvf-probe`.
    ///
    /// Kept for quick diagnostics and backwards compatibility.
    #[arg(long, hide = true, default_value_t = false)]
    probe: bool,

    /// Run a minimal Hypervisor.framework VM + vCPU create/destroy probe and exit.
    ///
    /// macOS-only diagnostic aid to isolate hv_vcpu_create compatibility and
    /// host support issues from guest boot plumbing.
    #[arg(long, default_value_t = false)]
    hvf_vcpu_probe: bool,

    /// Limit HVF execution to a short bounded bring-up loop.
    ///
    /// macOS-only diagnostic aid. By default, HVF runs continuously.
    #[arg(long, default_value_t = false)]
    hvf_run_once: bool,

    /// Enable experimental LAPIC timer interrupt injection in HVF run-once mode.
    ///
    /// macOS-only bring-up knob; has no effect unless `--hvf-run-once` is set.
    #[arg(long, default_value_t = false)]
    hvf_inject_timer: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ArchArg {
    #[value(name = "x86_64")]
    X8664,
    #[value(name = "riscv32")]
    Riscv32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum VmnetModeArg {
    #[value(name = "shared")]
    Shared,
    #[value(name = "host")]
    Host,
}

impl ArchArg {
    fn to_guest_arch(self) -> config::GuestArch {
        match self {
            ArchArg::X8664 => config::GuestArch::X86_64,
            ArchArg::Riscv32 => config::GuestArch::Riscv32,
        }
    }
}

impl VmnetModeArg {
    fn to_vmnet_mode(self) -> config::VmnetMode {
        match self {
            VmnetModeArg::Shared => config::VmnetMode::Shared,
            VmnetModeArg::Host => config::VmnetMode::Host,
        }
    }
}

fn parse_mac(s: &str) -> Result<[u8; 6]> {
    let mut out = [0u8; 6];
    let parts: Vec<&str> = s.split(|c| c == ':' || c == '-').collect();
    if parts.len() != 6 {
        anyhow::bail!("MAC '{s}' must have 6 hex bytes separated by ':' or '-'");
    }
    for (i, p) in parts.iter().enumerate() {
        out[i] = u8::from_str_radix(p, 16)
            .with_context(|| format!("invalid MAC byte '{p}'"))?;
    }
    Ok(out)
}

fn default_mac() -> [u8; 6] {
    // Locally-administered, unicast; low 4 bytes from process PID + clock.
    let pid = std::process::id() as u32;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let mix = pid.wrapping_mul(0x9E37_79B1).wrapping_add(nanos);
    [
        0x02,
        0x00,
        ((mix >> 24) & 0xFF) as u8,
        ((mix >> 16) & 0xFF) as u8,
        ((mix >> 8) & 0xFF) as u8,
        (mix & 0xFF) as u8,
    ]
}

fn validate_cpus(cpus: usize) -> Result<()> {
    anyhow::ensure!((1..=64).contains(&cpus), "--cpus must be between 1 and 64");
    Ok(())
}

#[cfg(target_os = "linux")]
fn main() -> Result<()> {
    use anyhow::Context;
    let cli = Cli::parse();
    validate_cpus(cli.cpus)?;
    if cli.hvf_preflight {
        anyhow::bail!("--hvf-preflight is macOS-only");
    }
    if cli.hvf_probe || cli.probe {
        anyhow::bail!("--hvf-probe is macOS-only");
    }
    if cli.hvf_run_once {
        anyhow::bail!("--hvf-run-once is macOS-only");
    }
    if cli.hvf_vcpu_probe {
        anyhow::bail!("--hvf-vcpu-probe is macOS-only");
    }
    if cli.hvf_inject_timer {
        anyhow::bail!("--hvf-inject-timer is macOS-only");
    }
    if cli.vmnet.is_some() {
        anyhow::bail!("--vmnet is macOS-only");
    }
    let restore_path = cli.restore.clone();
    let boot = match (cli.kernel, cli.restore) {
        (Some(kernel), None) => config::BootSource::Kernel(kernel),
        (None, Some(snapshot)) => config::BootSource::Snapshot(snapshot),
        (Some(_), Some(_)) => anyhow::bail!("pass either --kernel or --restore, not both"),
        (None, None) => anyhow::bail!("one of --kernel or --restore is required"),
    };
    let mac = match &cli.mac {
        Some(s) => parse_mac(s).context("parsing --mac")?,
        None => {
            if cli.tap.is_some() {
                if let Some(path) = restore_path.as_deref() {
                    if let Some(snapshot_mac) = snapshot::load_net_mac(path)
                        .with_context(|| format!("reading snapshot net MAC from {}", path.display()))?
                    {
                        snapshot_mac
                    } else {
                        default_mac()
                    }
                } else {
                    default_mac()
                }
            } else {
                default_mac()
            }
        }
    };
    let cfg = config::VmConfig {
        boot,
        guest_arch: cli.arch.to_guest_arch(),
        cpus: cli.cpus,
        memory_bytes: cli.memory * 1024 * 1024,
        disk_path: cli.disk,
        disk_read_only: cli.disk_ro,
        tap_name: cli.tap,
        vmnet_mode: None,
        mac,
        snapshot_save: cli.snapshot_save,
        cpu_throttle_ms: cli.cpu_throttle_ms,
        sensor_feed: cli.sensor_feed,
    };
    vm::run(cfg)
}

#[cfg(target_os = "macos")]
fn main() -> Result<()> {
    let cli = Cli::parse();
    validate_cpus(cli.cpus)?;
    if cli.tap.is_some() && cli.vmnet.is_some() {
        anyhow::bail!("--tap and --vmnet are mutually exclusive");
    }
    if cli.hvf_preflight {
        return backend::hvf::preflight();
    }
    if cli.hvf_probe || cli.probe {
        return backend::hvf::probe();
    }
    if cli.hvf_vcpu_probe {
        return backend::hvf::vcpu_probe();
    }
    if cli.hvf_run_once {
        std::env::set_var("VEER_VM_HVF_RUN_ONCE", "1");
    }
    if cli.hvf_inject_timer {
        std::env::set_var("VEER_VM_HVF_INJECT_TIMER", "1");
    }
    if let Some(path) = &cli.sensor_feed {
        std::env::set_var("VEER_VM_HVF_UART_RX_FILE", path);
    }
    let boot = match (cli.kernel, cli.restore) {
        (Some(kernel), None) => config::BootSource::Kernel(kernel),
        (None, Some(_)) => anyhow::bail!("--restore is not implemented for the macOS HVF skeleton"),
        (Some(_), Some(_)) => anyhow::bail!("pass either --kernel or --restore, not both"),
        (None, None) => anyhow::bail!("one of --kernel or --restore is required"),
    };
    let mac = match &cli.mac {
        Some(s) => parse_mac(s).context("parsing --mac")?,
        None => default_mac(),
    };

    let cfg = config::VmConfig {
        boot,
        guest_arch: cli.arch.to_guest_arch(),
        cpus: cli.cpus,
        memory_bytes: cli.memory * 1024 * 1024,
        disk_path: cli.disk,
        disk_read_only: cli.disk_ro,
        tap_name: cli.tap,
        vmnet_mode: cli.vmnet.map(|m| m.to_vmnet_mode()),
        mac,
        snapshot_save: cli.snapshot_save,
        cpu_throttle_ms: cli.cpu_throttle_ms,
        sensor_feed: cli.sensor_feed,
    };
    backend::hvf::run(cfg)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() -> Result<()> {
    let _ = Cli::parse();
    anyhow::bail!("veer-vm currently supports Linux (KVM) and macOS (HVF skeleton)");
}

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
use clap::Parser;
use std::path::PathBuf;

#[cfg(target_os = "linux")]
mod elf;
#[cfg(target_os = "linux")]
mod memory;
#[cfg(target_os = "linux")]
mod multiboot;
#[cfg(target_os = "linux")]
mod pci;
#[cfg(target_os = "linux")]
mod serial;
#[cfg(target_os = "linux")]
mod termios_guard;
#[cfg(target_os = "linux")]
mod virtio;
#[cfg(target_os = "linux")]
mod vm;

#[derive(Parser, Debug)]
#[command(name = "veer-vm", about = "VeerOS microVMM (KVM-based)")]
struct Cli {
    /// Path to the kernel ELF to boot (Multiboot v1 compatible).
    #[arg(long)]
    kernel: PathBuf,

    /// Guest memory size, in MiB.
    #[arg(long, default_value_t = 128)]
    memory: usize,

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

    /// MAC address to advertise to the guest (format `aa:bb:cc:dd:ee:ff`).
    /// Defaults to a locally-administered, randomly-seeded address.
    #[arg(long)]
    mac: Option<String>,
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

#[cfg(target_os = "linux")]
fn main() -> Result<()> {
    use anyhow::Context;
    let cli = Cli::parse();
    let mac = match &cli.mac {
        Some(s) => parse_mac(s).context("parsing --mac")?,
        None => default_mac(),
    };
    let cfg = vm::VmConfig {
        kernel_path: cli.kernel,
        memory_bytes: cli.memory * 1024 * 1024,
        disk_path: cli.disk,
        disk_read_only: cli.disk_ro,
        tap_name: cli.tap,
        mac,
    };
    vm::run(cfg)
}

#[cfg(not(target_os = "linux"))]
fn main() -> Result<()> {
    let _ = Cli::parse();
    anyhow::bail!("veer-vm currently requires Linux + KVM");
}

//! `veer-vm` — VeerOS microVMM.
//!
//! A lightweight Firecracker-class VMM that boots VeerOS kernels directly
//! via KVM, without QEMU. Phase 1: Multiboot v1 kernel + 16550 serial to
//! host stdout.
//!
//! Usage:
//!   veer-vm --kernel target/x86_64-unknown-none/debug/kernel-qemu-pc \
//!           --memory 128

use anyhow::Result;
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
}

#[cfg(target_os = "linux")]
fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = vm::VmConfig {
        kernel_path: cli.kernel,
        memory_bytes: cli.memory * 1024 * 1024,
        disk_path: cli.disk,
        disk_read_only: cli.disk_ro,
    };
    vm::run(cfg)
}

#[cfg(not(target_os = "linux"))]
fn main() -> Result<()> {
    let _ = Cli::parse();
    anyhow::bail!("veer-vm currently requires Linux + KVM");
}

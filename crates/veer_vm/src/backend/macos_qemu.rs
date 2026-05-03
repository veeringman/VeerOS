use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

use crate::config::{BootSource, GuestArch, VmConfig, VmnetMode};

pub fn run_riscv32(cfg: VmConfig) -> Result<()> {
    if cfg.guest_arch != GuestArch::Riscv32 {
        bail!(
            "macOS QEMU fallback supports only --arch riscv32 (got {})",
            cfg.guest_arch.as_str()
        );
    }

    if cfg.snapshot_save.is_some() {
        bail!("--snapshot-save is not implemented on the macOS riscv32 QEMU path");
    }

    if cfg.sensor_feed.is_some() {
        eprintln!(
            "[veer-vm] warning: --sensor-feed is ignored on macOS riscv32 QEMU fallback"
        );
    }

    let kernel = match &cfg.boot {
        BootSource::Kernel(path) => path,
        BootSource::Snapshot(path) => {
            bail!(
                "--restore is not implemented on the macOS riscv32 QEMU path (requested {})",
                path.display()
            )
        }
    };

    let ext = kernel
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ext == "iso" {
        bail!("--arch riscv32 requires a flat ELF kernel image; ISO boot is x86_64-only");
    }

    let qemu = resolve_qemu_riscv32().context("locating qemu-system-riscv32")?;
    let memory_mib = cfg.memory_bytes / (1024 * 1024);

    let mut cmd = Command::new(&qemu);
    cmd.arg("-M")
        .arg("virt")
        .arg("-cpu")
        .arg("rv32")
        .arg("-smp")
        .arg(cfg.cpus.to_string())
        .arg("-m")
        .arg(memory_mib.to_string())
        .arg("-nographic")
        .arg("-serial")
        .arg("mon:stdio")
        .arg("-bios")
        .arg("none")
        .arg("-kernel")
        .arg(kernel);

    if let Some(path) = &cfg.disk_path {
        let disk_arg = format!(
            "file={},if=none,id=d0,format=raw,readonly={}",
            qemu_path(path),
            if cfg.disk_read_only { "on" } else { "off" }
        );
        cmd.arg("-drive")
            .arg(disk_arg)
            .arg("-device")
            .arg("virtio-blk-device,drive=d0");
    }

    if cfg.tap_name.is_some() && cfg.vmnet_mode.is_some() {
        bail!("--tap and --vmnet are mutually exclusive");
    }

    if let Some(tap) = &cfg.tap_name {
        cmd.arg("-netdev")
            .arg(format!("tap,id=n0,ifname={tap},script=no,downscript=no"))
            .arg("-device")
            .arg(format!(
                "virtio-net-device,netdev=n0,mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                cfg.mac[0], cfg.mac[1], cfg.mac[2], cfg.mac[3], cfg.mac[4], cfg.mac[5]
            ));
    } else {
        let user_net = match cfg.vmnet_mode {
            Some(VmnetMode::Shared) => "user,id=n0,hostfwd=tcp::2323-:2323".to_string(),
            Some(VmnetMode::Host) => {
                eprintln!(
                    "[veer-vm] warning: --vmnet host is not available for riscv32 fallback; using user-mode networking"
                );
                "user,id=n0".to_string()
            }
            Some(VmnetMode::Bridged(_)) => {
                eprintln!(
                    "[veer-vm] warning: --vmnet bridged is not available for riscv32 qemu fallback; using user-mode networking"
                );
                "user,id=n0".to_string()
            }
            None => "user,id=n0".to_string(),
        };
        cmd.arg("-netdev")
            .arg(user_net)
            .arg("-device")
            .arg(format!(
                "virtio-net-device,netdev=n0,mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                cfg.mac[0], cfg.mac[1], cfg.mac[2], cfg.mac[3], cfg.mac[4], cfg.mac[5]
            ));
    }

    eprintln!("[veer-vm] backend: macos-qemu-riscv32");
    eprintln!("[veer-vm] qemu   : {}", qemu.display());
    eprintln!("[veer-vm] kernel : {}", kernel.display());
    eprintln!("[veer-vm] memory : {} MiB", memory_mib);
    if let Some(tap) = &cfg.tap_name {
        eprintln!("[veer-vm] net    : tap {}", tap);
    } else {
        eprintln!("[veer-vm] net    : user-mode");
    }

    let status = cmd.status().context("starting qemu-system-riscv32")?;
    if !status.success() {
        bail!("qemu-system-riscv32 exited with status {status}");
    }

    Ok(())
}

fn resolve_qemu_riscv32() -> Result<std::path::PathBuf> {
    if let Ok(override_path) = std::env::var("VEER_VM_QEMU") {
        let p = std::path::PathBuf::from(override_path);
        if p.exists() {
            return Ok(p);
        }
        bail!("VEER_VM_QEMU points to a missing path: {}", p.display());
    }

    for c in ["qemu-system-riscv32"] {
        if Command::new(c)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(std::path::PathBuf::from(c));
        }
    }

    for candidate in [
        "/opt/homebrew/bin/qemu-system-riscv32",
        "/usr/local/bin/qemu-system-riscv32",
    ] {
        let p = std::path::PathBuf::from(candidate);
        if p.exists() {
            return Ok(p);
        }
    }

    bail!(
        "qemu-system-riscv32 not found. Install QEMU (`brew install qemu`) or set VEER_VM_QEMU=<path to qemu-system-riscv32>"
    )
}

fn qemu_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

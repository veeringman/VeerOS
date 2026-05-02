use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{BootSource, GuestArch, VmConfig};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowsBackendKind {
    Qemu,
    Hyperv,
    Custom,
}

#[derive(Clone, Debug)]
pub struct WindowsBackendOptions {
    pub kind: WindowsBackendKind,
    pub custom_runner: Option<std::path::PathBuf>,
    pub custom_args: Vec<String>,
    pub hyperv_switch: Option<String>,
    pub hyperv_vm_name: Option<String>,
    pub hyperv_keep_vm: bool,
    pub hyperv_health_check: bool,
    pub hyperv_create_disk_if_missing: bool,
    pub hyperv_disk_size_gib: u64,
}

pub fn run(cfg: VmConfig, opts: WindowsBackendOptions) -> Result<()> {
    match opts.kind {
        WindowsBackendKind::Qemu => run_qemu(cfg),
        WindowsBackendKind::Hyperv => run_hyperv(cfg, opts),
        WindowsBackendKind::Custom => run_custom(cfg, opts),
    }
}

fn run_hyperv(cfg: VmConfig, opts: WindowsBackendOptions) -> Result<()> {
    if cfg.guest_arch != GuestArch::X86_64 {
        bail!(
            "Hyper-V backend currently supports only --arch x86_64 (got {})",
            cfg.guest_arch.as_str()
        );
    }

    if cfg.tap_name.is_some() {
        bail!("--tap is not supported by the Hyper-V backend");
    }
    if cfg.vmnet_mode.is_some() {
        bail!("--vmnet is macOS-only");
    }
    if cfg.snapshot_save.is_some() {
        bail!("--snapshot-save is not implemented on the Hyper-V backend");
    }

    let boot_iso = match &cfg.boot {
        BootSource::Kernel(path) => {
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if ext != "iso" {
                bail!(
                    "Hyper-V backend expects an ISO boot image. Build one with scripts/build-qemu-pc.sh and pass --kernel build/veeros.iso"
                );
            }
            path
        }
        BootSource::Snapshot(path) => {
            bail!(
                "--restore is not implemented on the Hyper-V backend (requested {})",
                path.display()
            )
        }
    };

    let memory_mib = cfg.memory_bytes / (1024 * 1024);
    let vm_name = opts
        .hyperv_vm_name
        .unwrap_or_else(|| format!("VeerOS-{}", std::process::id()));
    let runner = resolve_hyperv_runner().context("locating Hyper-V runner script")?;
    let shell = resolve_powershell();

    let mut cmd = Command::new(&shell);
    cmd.arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&runner)
        .arg("-Kernel")
        .arg(boot_iso)
        .arg("-MemoryMiB")
        .arg(memory_mib.to_string())
        .arg("-Cpus")
        .arg(cfg.cpus.to_string())
        .arg("-VmName")
        .arg(&vm_name);

    if let Some(switch_name) = opts.hyperv_switch {
        cmd.arg("-SwitchName").arg(switch_name);
    }

    if opts.hyperv_keep_vm {
        cmd.arg("-KeepVm");
    }

    if opts.hyperv_health_check {
        cmd.arg("-HealthCheck");
    }

    if opts.hyperv_create_disk_if_missing {
        cmd.arg("-CreateDiskIfMissing")
            .arg("-DiskSizeGiB")
            .arg(opts.hyperv_disk_size_gib.to_string());
    }

    if let Some(disk) = &cfg.disk_path {
        let ext = disk
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ext != "vhd" && ext != "vhdx" {
            bail!(
                "Hyper-V backend requires --disk to be .vhd/.vhdx (got {})",
                disk.display()
            );
        }
        cmd.arg("-Disk").arg(disk);
        if cfg.disk_read_only {
            cmd.arg("-DiskReadOnly");
        }
    }

    eprintln!("[veer-vm] backend: windows-hyperv");
    eprintln!("[veer-vm] shell  : {}", shell.display());
    eprintln!("[veer-vm] runner : {}", runner.display());
    eprintln!("[veer-vm] vm     : {}", vm_name);
    eprintln!("[veer-vm] boot   : {}", boot_iso.display());
    eprintln!("[veer-vm] memory : {} MiB", memory_mib);

    let status = cmd.status().context("starting Hyper-V runner")?;
    if !status.success() {
        bail!("Hyper-V runner exited with status {status}");
    }

    Ok(())
}

fn run_qemu(cfg: VmConfig) -> Result<()> {
    if cfg.guest_arch != GuestArch::X86_64 {
        bail!(
            "Windows backend currently supports only --arch x86_64 (got {})",
            cfg.guest_arch.as_str()
        );
    }

    if cfg.tap_name.is_some() {
        bail!("--tap is not supported by the Windows backend (use user-net)");
    }
    if cfg.vmnet_mode.is_some() {
        bail!("--vmnet is macOS-only");
    }
    if cfg.snapshot_save.is_some() {
        bail!("--snapshot-save is not implemented on the Windows backend");
    }

    let boot_iso = match &cfg.boot {
        BootSource::Kernel(path) => {
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if ext != "iso" {
                bail!(
                    "Windows backend expects an ISO boot image. Build one with scripts/build-qemu-pc.sh and pass --kernel build/veeros.iso"
                );
            }
            path
        }
        BootSource::Snapshot(path) => {
            bail!(
                "--restore is not implemented on the Windows backend (requested {})",
                path.display()
            )
        }
    };

    let qemu = resolve_qemu_x86_64().context("locating qemu-system-x86_64")?;
    let memory_mib = cfg.memory_bytes / (1024 * 1024);
    let cpu_model = std::env::var("VEER_VM_QEMU_CPU").unwrap_or_else(|_| "qemu64".to_string());

    let mut cmd = Command::new(&qemu);
    cmd.arg("-machine")
        .arg("q35,accel=whpx:tcg")
        .arg("-cpu")
        .arg(&cpu_model)
        .arg("-smp")
        .arg(cfg.cpus.to_string())
        .arg("-m")
        .arg(memory_mib.to_string())
        .arg("-serial")
        .arg("mon:stdio")
        .arg("-display")
        .arg("none")
        .arg("-cdrom")
        .arg(boot_iso)
        .arg("-boot")
        .arg("d")
        .arg("-netdev")
        .arg("user,id=net0,hostfwd=tcp::2222-:22")
        .arg("-device")
        .arg(format!(
            "virtio-net-pci,netdev=net0,mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            cfg.mac[0], cfg.mac[1], cfg.mac[2], cfg.mac[3], cfg.mac[4], cfg.mac[5]
        ));

    if let Some(disk) = &cfg.disk_path {
        let disk_arg = format!(
            "file={},if=virtio,format=raw,readonly={}",
            qemu_path(disk),
            if cfg.disk_read_only { "on" } else { "off" }
        );
        cmd.arg("-drive").arg(disk_arg);
    }

    eprintln!("[veer-vm] backend: windows-qemu-whpx");
    eprintln!("[veer-vm] qemu   : {}", qemu.display());
    eprintln!("[veer-vm] boot   : {}", boot_iso.display());
    eprintln!("[veer-vm] cpu    : {}", cpu_model);
    eprintln!("[veer-vm] memory : {} MiB", memory_mib);
    eprintln!("[veer-vm] ssh    : localhost:2222 -> guest:22");

    let status = cmd.status().context("starting QEMU/WHPX")?;
    if !status.success() {
        bail!("QEMU exited with status {status}");
    }

    Ok(())
}

fn run_custom(cfg: VmConfig, opts: WindowsBackendOptions) -> Result<()> {
    let runner = opts
        .custom_runner
        .context("custom backend requires --custom-runner <path>")?;

    let kernel = match &cfg.boot {
        BootSource::Kernel(path) => path,
        BootSource::Snapshot(path) => {
            bail!(
                "--restore is not implemented for custom Windows backend (requested {})",
                path.display()
            )
        }
    };

    let memory_mib = cfg.memory_bytes / (1024 * 1024);
    let mac = format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        cfg.mac[0], cfg.mac[1], cfg.mac[2], cfg.mac[3], cfg.mac[4], cfg.mac[5]
    );
    let disk = cfg
        .disk_path
        .as_ref()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();

    let template_args = if opts.custom_args.is_empty() {
        let mut args = vec![
            "--kernel".to_string(),
            kernel.to_string_lossy().to_string(),
            "--memory".to_string(),
            memory_mib.to_string(),
            "--cpus".to_string(),
            cfg.cpus.to_string(),
            "--arch".to_string(),
            cfg.guest_arch.as_str().to_string(),
            "--mac".to_string(),
            mac.clone(),
        ];
        if let Some(d) = &cfg.disk_path {
            args.push("--disk".to_string());
            args.push(d.to_string_lossy().to_string());
            if cfg.disk_read_only {
                args.push("--disk-ro".to_string());
            }
        }
        args
    } else {
        opts.custom_args
    };

    let args = template_args
        .into_iter()
        .map(|a| {
            a.replace("{kernel}", &kernel.to_string_lossy())
                .replace("{memory_mib}", &memory_mib.to_string())
                .replace("{cpus}", &cfg.cpus.to_string())
                .replace("{arch}", cfg.guest_arch.as_str())
                .replace("{disk}", &disk)
                .replace("{disk_ro}", if cfg.disk_read_only { "true" } else { "false" })
                .replace("{mac}", &mac)
        })
        .collect::<Vec<_>>();

    eprintln!("[veer-vm] backend: windows-custom");
    eprintln!("[veer-vm] runner : {}", runner.display());

    let status = Command::new(&runner)
        .args(&args)
        .status()
        .with_context(|| format!("starting custom runner {}", runner.display()))?;

    if !status.success() {
        bail!("custom runner exited with status {status}");
    }
    Ok(())
}

fn resolve_qemu_x86_64() -> Result<std::path::PathBuf> {
    if let Ok(override_path) = std::env::var("VEER_VM_QEMU") {
        let p = std::path::PathBuf::from(override_path);
        if p.exists() {
            return Ok(p);
        }
        bail!("VEER_VM_QEMU points to a missing path: {}", p.display());
    }

    // Check PATH first
    let path_candidates = ["qemu-system-x86_64.exe", "qemu-system-x86_64"];
    for c in path_candidates {
        if Command::new(c)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(std::path::PathBuf::from(c));
        }
    }

    // Check well-known Windows installation directories
    let known_dirs = [
        r"C:\Program Files\qemu\qemu-system-x86_64.exe",
        r"C:\Program Files (x86)\qemu\qemu-system-x86_64.exe",
        r"C:\tools\qemu\qemu-system-x86_64.exe",
    ];
    for p in known_dirs {
        let path = std::path::PathBuf::from(p);
        if path.exists() {
            return Ok(path);
        }
    }

    // Check Scoop
    if let Ok(home) = std::env::var("USERPROFILE") {
        let scoop = std::path::PathBuf::from(&home)
            .join("scoop/apps/qemu/current/qemu-system-x86_64.exe");
        if scoop.exists() {
            return Ok(scoop);
        }
    }

    bail!(
        "qemu-system-x86_64 not found. Install QEMU for Windows from https://qemu.weilnetz.de/w64/ \
         or set VEER_VM_QEMU=<path to qemu-system-x86_64.exe>"
    )
}

fn resolve_powershell() -> PathBuf {
    let pwsh = PathBuf::from("pwsh");
    if Command::new(&pwsh)
        .arg("-NoProfile")
        .arg("-Command")
        .arg("$PSVersionTable.PSVersion.Major")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return pwsh;
    }
    PathBuf::from("powershell")
}

fn resolve_hyperv_runner() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("VEER_VM_HYPERV_RUNNER") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
        bail!(
            "VEER_VM_HYPERV_RUNNER points to a missing path: {}",
            p.display()
        );
    }

    let cwd = std::env::current_dir().context("reading current directory")?;
    let candidate = cwd.join("scripts").join("run-hyperv-veer-vm.ps1");
    if candidate.exists() {
        return Ok(candidate);
    }

    bail!(
        "Hyper-V runner script not found. Expected scripts/run-hyperv-veer-vm.ps1 in current directory, \
         or set VEER_VM_HYPERV_RUNNER=<absolute path>"
    )
}

fn qemu_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

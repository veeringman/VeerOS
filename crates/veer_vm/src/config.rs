use std::path::PathBuf;

pub enum BootSource {
    Kernel(PathBuf),
    Snapshot(PathBuf),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestArch {
    X86_64,
    Riscv32,
    Aarch64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VmnetMode {
    Shared,
    Host,
    /// Bridged to a specific physical interface (e.g. "en0").
    Bridged(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UserNetMode {
    User { host_port: u16, guest_port: u16 },
}

impl VmnetMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            VmnetMode::Shared => "shared",
            VmnetMode::Host => "host",
            VmnetMode::Bridged(_) => "bridged",
        }
    }
}

impl GuestArch {
    pub fn as_str(self) -> &'static str {
        match self {
            GuestArch::X86_64 => "x86_64",
            GuestArch::Riscv32 => "riscv32",
            GuestArch::Aarch64 => "aarch64",
        }
    }
}

pub struct VmConfig {
    pub boot: BootSource,
    pub guest_arch: GuestArch,
    /// Number of virtual CPUs presented to the guest.
    pub cpus: usize,
    pub memory_bytes: usize,
    pub disk_path: Option<PathBuf>,
    pub disk_read_only: bool,
    /// Name of a pre-created TAP interface to attach as virtio-net-pci.
    pub tap_name: Option<String>,
    /// macOS vmnet mode request.
    pub vmnet_mode: Option<VmnetMode>,
    /// Entitlement-free userspace networking request.
    pub user_net: Option<UserNetMode>,
    /// MAC address advertised to the guest.
    pub mac: [u8; 6],
    /// Save a snapshot directory when the VM shuts down cleanly.
    pub snapshot_save: Option<PathBuf>,
    /// Busy-loop throttle sleep (ms) for rv32-soft mode.
    pub cpu_throttle_ms: u64,
    /// Optional path to a FIFO for EdgeFabric sensor injection.
    pub sensor_feed: Option<PathBuf>,
}

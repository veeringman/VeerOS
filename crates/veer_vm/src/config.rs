use std::path::PathBuf;

pub enum BootSource {
    Kernel(PathBuf),
    Snapshot(PathBuf),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestArch {
    X86_64,
    Riscv32,
}

impl GuestArch {
    pub fn as_str(self) -> &'static str {
        match self {
            GuestArch::X86_64 => "x86_64",
            GuestArch::Riscv32 => "riscv32",
        }
    }
}

pub struct VmConfig {
    pub boot: BootSource,
    pub guest_arch: GuestArch,
    pub memory_bytes: usize,
    pub disk_path: Option<PathBuf>,
    pub disk_read_only: bool,
    /// Name of a pre-created TAP interface to attach as virtio-net-pci.
    pub tap_name: Option<String>,
    /// MAC address advertised to the guest.
    pub mac: [u8; 6],
    /// Save a snapshot directory when the VM shuts down cleanly.
    pub snapshot_save: Option<PathBuf>,
    /// Busy-loop throttle sleep (ms) for rv32-soft mode.
    pub cpu_throttle_ms: u64,
    /// Optional path to a FIFO for EdgeFabric sensor injection.
    pub sensor_feed: Option<PathBuf>,
}
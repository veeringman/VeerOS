//! Memory map for the generic AArch64 virtual-machine target.

/// Guest RAM base used by the VeerOS AArch64 virtual target.
/// Matches the standard QEMU `virt` board — RAM above all device MMIO.
pub const DRAM_BASE: usize = 0x4000_0000;

/// Kernel load address within DRAM.
pub const KERNEL_LOAD_ADDR: usize = 0x4008_0000;

/// Default guest RAM size used by development scripts.
pub const DRAM_SIZE: usize = 512 * 1024 * 1024;

/// PL011 UART base. Matches QEMU `virt` and the initial VeerHV-A64 device model.
pub const UART0_BASE: usize = 0x0900_0000;

/// Generic Interrupt Controller distributor base for the virtual platform.
pub const GIC_DIST_BASE: usize = 0x0800_0000;

/// Generic Interrupt Controller redistributor base for the virtual platform.
pub const GIC_REDIST_BASE: usize = 0x080A_0000;

/// ARM virtual timer PPI.
pub const TIMER_PPI: u16 = 27;

/// Virtio-mmio v2 net device MMIO base.
/// Must match `VIRTIO_MMIO_NET_BASE` in `veer_vm::backend::hvf_aarch64`.
pub const VIRTIO_NET_BASE: usize = 0x0A00_0000;

/// Virtio-mmio v2 block device MMIO base.
/// Must match `VIRTIO_MMIO_BLK_BASE` in `veer_vm::backend::hvf_aarch64`.
pub const VIRTIO_BLK_BASE: usize = 0x0A00_1000;

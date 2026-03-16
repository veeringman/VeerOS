//! Memory map constants for Raspberry Pi 5 (BCM2712).
//!
//! The RPi 5 uses the RP1 southbridge (PCIe-attached) for most I/O
//! peripherals (UART, SPI, I2C, GPIO).  The firmware maps the RP1
//! BAR into the CPU's physical address space.
//!
//! **Important**: If these addresses don't match your firmware,
//! update the constants and rebuild.  You can extract the real
//! values from the device-tree blob passed at boot (x0 register).

// ─── RP1 Southbridge (PCIe BAR, firmware-mapped) ─────────────────
/// RP1 BAR base address (mapped by RPi firmware via PCIe).
pub const RP1_BAR_BASE: usize = 0x1F_0000_0000;

/// RP1 UART0 offset within BAR (PL011, GPIO 14/15 serial console).
pub const RP1_UART0_OFF: usize = 0x3_0000;

/// RP1 UART1 offset within BAR (PL011).
pub const RP1_UART1_OFF: usize = 0x3_4000;

// ─── GIC-400 (GICv2) ────────────────────────────────────────────
/// GIC distributor base address.
pub const GIC_DIST_BASE: usize = 0x107FFF9000;

/// GIC CPU interface base address.
pub const GIC_CPU_BASE: usize = 0x107FFFA000;

// ─── DRAM ────────────────────────────────────────────────────────
/// Physical RAM starts at address 0.
pub const DRAM_BASE: usize = 0x0;

/// The RPi firmware loads kernel8.img at this address.
pub const KERNEL_LOAD_ADDR: usize = 0x80000;

/// Conservative DRAM size for linker script (1 GB).
/// Actual boards have 4 or 8 GB; kernel uses a tiny fraction.
pub const DRAM_SIZE: usize = 1024 * 1024 * 1024;

// ─── ARM Generic Timer ──────────────────────────────────────────
/// PPI number for the ARM physical timer interrupt.
pub const TIMER_PPI: u16 = 30;

/// Timer frequency on Pi 5 (set by firmware, typically 54 MHz).
pub const TIMER_FREQ_HZ: u64 = 54_000_000;

// ─── VideoCore Mailbox ──────────────────────────────────────
/// BCM2712 VideoCore mailbox base address (ARM physical).
/// Derived from device tree: `soc@107c000000 / mailbox@7c013880`.
pub const MBOX_BASE: usize = 0x10_7C01_3880;

/// Offset to convert ARM physical address to VC bus address (uncached alias).
/// On BCM2835/2711/2712, the VC sees ARM physical address 0 at bus 0xC0000000.
pub const ARM_TO_VC_BUS: usize = 0xC000_0000;

/// Convert a VC bus address to an ARM physical address.
#[inline]
pub const fn vc_bus_to_arm(vc_addr: u32) -> usize {
    (vc_addr & 0x3FFF_FFFF) as usize
}

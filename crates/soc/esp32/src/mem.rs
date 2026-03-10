//! Memory map constants for ESP32 RISC-V variants.
//!
//! Each variant has a different SRAM layout. The constants here are used
//! by linker scripts and kernel code to configure memory regions correctly.
//!
//! # Variant summary
//!
//! | Variant | HP SRAM | SRAM origin  | Address space  |
//! |---------|---------|--------------|----------------|
//! | C3      | 400 KB  | 0x3FC8_0000  | Split IRAM/DRAM |
//! | C6      | 512 KB  | 0x4080_0000  | Unified         |
//! | H2      | 320 KB  | 0x4080_0000  | Unified         |

// ═══════════════════════════════════════════════════════════════════════════
// ESP32-C3
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "c3")]
pub mod sram {
    /// IRAM (instruction) base address.
    pub const IRAM_BASE: usize = 0x4038_0000;
    /// IRAM size in bytes (384 KB).
    pub const IRAM_SIZE: usize = 384 * 1024;

    /// DRAM (data) base address.
    pub const DRAM_BASE: usize = 0x3FC8_0000;
    /// DRAM size in bytes (400 KB).
    pub const DRAM_SIZE: usize = 400 * 1024;

    /// Total SRAM available.
    pub const TOTAL_SRAM: usize = 400 * 1024;
}

// ═══════════════════════════════════════════════════════════════════════════
// ESP32-C6
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "c6")]
pub mod sram {
    /// HP SRAM base (unified instruction + data address space).
    pub const SRAM_BASE: usize = 0x4080_0000;
    /// HP SRAM size in bytes (512 KB).
    pub const SRAM_SIZE: usize = 512 * 1024;

    /// Total SRAM available.
    pub const TOTAL_SRAM: usize = 512 * 1024;
}

// ═══════════════════════════════════════════════════════════════════════════
// ESP32-H2
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(feature = "h2")]
pub mod sram {
    /// HP SRAM base (unified instruction + data address space).
    pub const SRAM_BASE: usize = 0x4080_0000;
    /// HP SRAM size in bytes (320 KB).
    pub const SRAM_SIZE: usize = 320 * 1024;

    /// Total SRAM available.
    pub const TOTAL_SRAM: usize = 320 * 1024;
}

// ═══════════════════════════════════════════════════════════════════════════
// Fallback (no feature selected)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(all(not(feature = "c3"), not(feature = "c6"), not(feature = "h2")))]
pub mod sram {
    pub const SRAM_BASE: usize = 0x4080_0000;
    pub const SRAM_SIZE: usize = 512 * 1024;
    pub const TOTAL_SRAM: usize = 512 * 1024;
}

// ═══════════════════════════════════════════════════════════════════════════
// Shared peripheral base addresses (same across C3/C6/H2)
// ═══════════════════════════════════════════════════════════════════════════

/// UART0 base address.
pub const UART0_BASE: usize = 0x6000_0000;

/// SYSTIMER base address.
pub const SYSTIMER_BASE: usize = 0x6002_3000;

/// Interrupt matrix (INTERRUPT_CORE0) base address.
pub const INTC_BASE: usize = 0x600C_2000;

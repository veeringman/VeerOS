#![no_std]

//! SoC-level drivers for the ESP32 RISC-V family (C3, C6, H2).
//!
//! Variant-specific differences (memory map, watchdog registers) are
//! handled via Cargo features: `c3`, `c6`, `h2`.
//!
//! ```text
//! ┌──────────────────────────────────────────────────────┐
//! │  arch  (traits: Platform, Serial, InterruptController) │
//! └────────────────────┬─────────────────────────────────┘
//!                      │ implements
//! ┌────────────────────▼─────────────────────────────────┐
//! │  soc-esp32  (this crate)                              │
//! │  ┌──────┐ ┌──────┐ ┌────────┐ ┌─────┐ ┌──────┐      │
//! │  │ UART │ │ INTC │ │SYSTIMER│ │ WDT │ │ WiFi │      │
//! │  └──────┘ └──────┘ └────────┘ └─────┘ └──────┘      │
//! │  ┌─────────────────────────────────────────┐         │
//! │  │ mem  (per-variant memory map constants)  │         │
//! │  └─────────────────────────────────────────┘         │
//! └────────────────────┬─────────────────────────────────┘
//!                      │ used by
//! ┌────────────────────▼─────────────────────────────────┐
//! │  kernel-xiao-esp32c6 / kernel-generic-esp32c3 / ...   │
//! └──────────────────────────────────────────────────────┘
//! ```

use arch::Platform;

pub mod ble;
pub mod ble_hid;
pub mod ieee802154;
pub mod intc;
pub mod mem;
pub mod sdspi;
pub mod systimer;
pub mod uart;
pub mod wdt;
pub mod wifi;

// ═══════════════════════════════════════════════════════════════════════════
// Platform implementation
// ═══════════════════════════════════════════════════════════════════════════

pub struct Esp32Riscv;

impl Esp32Riscv {
    pub const fn new() -> Self {
        Self
    }
}

impl Platform for Esp32Riscv {
    fn name(&self) -> &'static str {
        #[cfg(feature = "c6")]
        { "ESP32-C6 (RISC-V)" }
        #[cfg(all(not(feature = "c6"), feature = "h2"))]
        { "ESP32-H2 (RISC-V)" }
        #[cfg(all(not(feature = "c6"), not(feature = "h2"), feature = "c3"))]
        { "ESP32-C3 (RISC-V)" }
        #[cfg(all(not(feature = "c6"), not(feature = "h2"), not(feature = "c3")))]
        { "ESP32 RISC-V" }
    }

    fn init_cpu(&self) {}
    fn init_interrupts(&self) {}
    fn init_timer(&self) {}
}

// ═══════════════════════════════════════════════════════════════════════════
// Convenience accessors
// ═══════════════════════════════════════════════════════════════════════════

pub fn default_serial() -> uart::Uart0 {
    uart::Uart0::new()
}

pub fn interrupt_controller() -> intc::Esp32Intc {
    intc::Esp32Intc::new()
}

pub fn system_timer() -> systimer::SysTimer {
    systimer::SysTimer::new()
}
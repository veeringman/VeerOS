#![no_std]

use arch::Platform;

pub mod intc;
pub mod systimer;
pub mod uart;

pub struct Esp32Riscv;

impl Esp32Riscv {
    pub const fn new() -> Self {
        Self
    }
}

impl Platform for Esp32Riscv {
    fn name(&self) -> &'static str {
        #[cfg(feature = "esp32c6")]
        {
            "ESP32-C6 (RISC-V)"
        }
        #[cfg(all(not(feature = "esp32c6"), feature = "esp32h2"))]
        {
            "ESP32-H2 (RISC-V)"
        }
        #[cfg(all(not(feature = "esp32c6"), not(feature = "esp32h2"), feature = "esp32c3"))]
        {
            "ESP32-C3 (RISC-V)"
        }
        #[cfg(all(not(feature = "esp32c6"), not(feature = "esp32h2"), not(feature = "esp32c3")))]
        {
            "ESP32 RISC-V"
        }
    }

    fn init_cpu(&self) {}

    fn init_interrupts(&self) {}

    fn init_timer(&self) {}
}

// ---------------------------------------------------------------------------
// Re-export board peripherals as convenient accessors
// ---------------------------------------------------------------------------
pub fn default_serial() -> uart::Uart0 {
    uart::Uart0::new()
}

pub fn interrupt_controller() -> intc::Esp32Intc {
    intc::Esp32Intc::new()
}

pub fn system_timer() -> systimer::SysTimer {
    systimer::SysTimer::new()
}
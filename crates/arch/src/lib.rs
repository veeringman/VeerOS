#![no_std]

use core::fmt;

// ═══════════════════════════════════════════════════════════════════════════
// Platform
// ═══════════════════════════════════════════════════════════════════════════

/// Core platform abstraction — every BSP implements this.
pub trait Platform {
    fn name(&self) -> &'static str;
    fn init_cpu(&self);
    fn init_interrupts(&self);
    fn init_timer(&self);
}

// ═══════════════════════════════════════════════════════════════════════════
// Serial / Console
// ═══════════════════════════════════════════════════════════════════════════

/// Byte-level serial I/O (UART / USB-serial / SWO / ...).
pub trait Serial {
    /// Send a single byte. Blocks until the TX FIFO has room.
    fn write_byte(&self, byte: u8);

    /// Read a single byte. Blocks until a byte is available.
    fn read_byte(&self) -> u8;

    /// Returns `true` if at least one byte is available to read without blocking.
    fn has_data(&self) -> bool { false }

    /// Convenience: write an entire byte slice.
    fn write_bytes(&self, bytes: &[u8]) {
        for &b in bytes {
            self.write_byte(b);
        }
    }
}

/// Higher-level console that bridges `Serial` to `core::fmt::Write`.
pub struct Console<S: Serial> {
    serial: S,
}

impl<S: Serial> Console<S> {
    pub const fn new(serial: S) -> Self {
        Self { serial }
    }

    pub fn write_str_raw(&mut self, s: &str) {
        self.serial.write_bytes(s.as_bytes());
    }

    /// Read a single byte from the underlying serial device.
    pub fn read_byte(&self) -> u8 {
        self.serial.read_byte()
    }

    /// Check if data is available.
    pub fn has_data(&self) -> bool {
        self.serial.has_data()
    }
}

impl<S: Serial> fmt::Write for Console<S> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        // Translate bare LF → CR+LF for serial terminals.
        for &b in s.as_bytes() {
            if b == b'\n' {
                self.serial.write_byte(b'\r');
            }
            self.serial.write_byte(b);
        }
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Interrupt controller
// ═══════════════════════════════════════════════════════════════════════════

/// Architecture-neutral interrupt controller interface.
pub trait InterruptController {
    /// Enable a specific interrupt line.
    fn enable_interrupt(&self, irq: u16);
    /// Disable a specific interrupt line.
    fn disable_interrupt(&self, irq: u16);
    /// Set interrupt priority (0 = disabled on most controllers).
    fn set_priority(&self, irq: u16, priority: u8);
    /// Enable global interrupts at the CPU level.
    fn enable_global(&self);
    /// Disable global interrupts at the CPU level.
    fn disable_global(&self);
}

// ═══════════════════════════════════════════════════════════════════════════
// Timer
// ═══════════════════════════════════════════════════════════════════════════

/// Architecture-neutral periodic tick timer.
pub trait TickTimer {
    /// Configure the timer for the given period in microseconds and start it.
    fn configure_tick(&self, period_us: u32);
    /// Acknowledge / clear the pending interrupt so the next tick can fire.
    fn clear_pending(&self);
    /// Read the current free-running counter value (µs resolution or better).
    fn counter_us(&self) -> u64;
}

// ═══════════════════════════════════════════════════════════════════════════
// Task context (saved/restored on context switch)
// ═══════════════════════════════════════════════════════════════════════════

/// Minimal saved CPU context for preemptive context switching.
/// Each architecture defines the concrete register set.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TaskContext {
    /// General-purpose registers (RISC-V: x1–x31; ARM: r0–r15; etc.).
    pub gpr: [usize; 32],
    /// Program counter / return address for resumption.
    pub pc: usize,
    /// Machine / thread status register.
    pub status: usize,
}

impl TaskContext {
    pub const fn zero() -> Self {
        Self {
            gpr: [0; 32],
            pc: 0,
            status: 0,
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Memory model (unchanged)
// ═══════════════════════════════════════════════════════════════════════════

pub trait MemoryModel {
    fn page_size_bytes(&self) -> usize;
}

// ═══════════════════════════════════════════════════════════════════════════
// Network device
// ═══════════════════════════════════════════════════════════════════════════

/// Architecture-neutral network device interface.
///
/// A BSP implements this for its NIC (VIRTIO-NET, Wi-Fi radio, etc.).
/// The network stack (`smoltcp`) consumes these methods to send/receive
/// Ethernet frames.
pub trait NetworkDevice {
    /// Maximum transmission unit (bytes of payload the device can carry).
    fn mtu(&self) -> usize { 1514 }

    /// Returns `true` when at least one received frame is pending.
    fn has_rx(&self) -> bool;

    /// Receive a single Ethernet frame into `buf`.
    /// Returns the number of bytes written, or 0 if nothing was available.
    fn recv(&self, buf: &mut [u8]) -> usize;

    /// Transmit an Ethernet frame from `buf[..len]`.
    fn send(&self, buf: &[u8]);

    /// The device's MAC address.
    fn mac_address(&self) -> [u8; 6];
}
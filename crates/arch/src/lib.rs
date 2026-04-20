#![no_std]

use core::fmt;

// ═══════════════════════════════════════════════════════════════════════════
// Per-architecture modules
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
pub mod riscv32;

#[cfg(target_arch = "aarch64")]
pub mod aarch64;

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

// Future: pub mod riscv64;
// Future: pub mod x86_64;
// Future: pub mod xtensa;

// ═══════════════════════════════════════════════════════════════════════════
// TaskContext — cfg-selected concrete type
// ═══════════════════════════════════════════════════════════════════════════

/// The concrete saved-context type for the current target architecture.
///
/// Assembly trap code and kernel binaries use this directly.
/// The `SavedContext` trait provides architecture-neutral access.
#[cfg(any(target_arch = "riscv32", not(any(target_arch = "aarch64", target_arch = "x86_64"))))]
pub type TaskContext = riscv32::Riscv32Context;

#[cfg(target_arch = "aarch64")]
pub type TaskContext = aarch64::Aarch64Context;

#[cfg(target_arch = "x86_64")]
pub type TaskContext = x86_64::X86_64Context;

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

    /// Flush any buffered TX data to the wire.  Default is no-op (for
    /// byte-at-a-time UART drivers); USB-CDC drivers should override.
    fn flush(&self) {}

    /// Convenience: write an entire byte slice, then flush.
    fn write_bytes(&self, bytes: &[u8]) {
        for &b in bytes {
            self.write_byte(b);
        }
        self.flush();
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

    /// Return a reference to the underlying serial device.
    pub fn serial(&self) -> &S {
        &self.serial
    }

    /// Consume the console and return the underlying serial device.
    pub fn into_inner(self) -> S {
        self.serial
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
        self.serial.flush();
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
// SavedContext trait (architecture-neutral context switch interface)
// ═══════════════════════════════════════════════════════════════════════════

/// Architecture-neutral interface for saved CPU register state.
///
/// Each architecture provides a concrete `#[repr(C)]` struct whose layout
/// matches its trap entry/exit assembly.  This trait provides portable
/// accessor methods so the microkernel (scheduler, syscall dispatcher, IPC)
/// can manipulate contexts without knowing register file details.
///
/// The concrete type is selected at compile time via `arch::TaskContext`
/// (a `#[cfg(target_arch)]` type alias), so all calls are monomorphised
/// with zero dynamic dispatch overhead.
pub trait SavedContext: Copy + Clone + core::fmt::Debug + Sized {
    /// Size of the trap instruction that triggers a syscall (e.g., 4 for
    /// RISC-V `ecall`, 4 for ARM `svc`, variable for x86 `syscall`).
    const INSTRUCTION_SIZE: usize;

    /// Create a zeroed context (all registers = 0).
    fn zero() -> Self;

    /// Set the program counter (entry point / resume address).
    fn set_pc(&mut self, pc: usize);
    /// Get the program counter.
    fn get_pc(&self) -> usize;
    /// Advance the PC past the current trap instruction (e.g., +4 for ecall).
    fn advance_pc(&mut self);

    /// Set the stack pointer register.
    fn set_sp(&mut self, sp: usize);
    /// Get the stack pointer register.
    fn get_sp(&self) -> usize;

    /// Set the status/flags register (mstatus, CPSR, RFLAGS, etc.).
    fn set_status(&mut self, status: usize);
    /// Get the status/flags register.
    fn get_status(&self) -> usize;

    /// Set a syscall/function argument by index (0 = first argument).
    fn set_arg(&mut self, index: usize, val: usize);
    /// Get a syscall/function argument by index.
    fn get_arg(&self, index: usize) -> usize;

    /// Set a return value register by index (0 = primary return value).
    fn set_ret(&mut self, index: usize, val: usize);
    /// Get a return value register by index.
    fn get_ret(&self, index: usize) -> usize;

    /// Read the syscall number register (a7 on RISC-V, x8 on ARM64, rax on x86).
    fn get_syscall_nr(&self) -> usize;

    /// Read the kernel bookkeeping word (stored in an otherwise-unused
    /// register slot — e.g., x0 on RISC-V, which is hardwired to zero
    /// in hardware but free in the saved frame).
    fn get_kernel_word(&self) -> usize;
    /// Write the kernel bookkeeping word.
    fn set_kernel_word(&mut self, val: usize);
}

// ═══════════════════════════════════════════════════════════════════════════
// Memory model (unchanged)
// ═══════════════════════════════════════════════════════════════════════════

pub trait MemoryModel {
    fn page_size_bytes(&self) -> usize;
}

// ═══════════════════════════════════════════════════════════════════════════
// Memory region + permission types (used by PMP, MPU, and page-table drivers)
// ═══════════════════════════════════════════════════════════════════════════

/// Access permission flags for a memory region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemPerms(pub u8);

impl MemPerms {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const EXECUTE: Self = Self(4);
    pub const RW: Self = Self(1 | 2);
    pub const RX: Self = Self(1 | 4);
    pub const RWX: Self = Self(1 | 2 | 4);

    #[inline]
    pub const fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }
}

/// Maximum number of memory regions per task.
/// 8 regions: stack, guard, code, rodata, + up to 4 MMIO grants for userspace drivers.
pub const MAX_TASK_REGIONS: usize = 8;

/// A memory region descriptor associated with a task.
#[derive(Debug, Clone, Copy)]
pub struct TaskMemRegion {
    pub base: usize,
    pub size: usize,
    pub perms: MemPerms,
}

impl TaskMemRegion {
    pub const fn empty() -> Self {
        Self { base: 0, size: 0, perms: MemPerms::NONE }
    }

    /// Returns `true` if the range `[addr, addr+len)` is fully inside this
    /// region and the region grants at least the requested permissions.
    pub fn allows(&self, addr: usize, len: usize, required: MemPerms) -> bool {
        self.size > 0
            && addr >= self.base
            && len <= self.size
            && addr - self.base <= self.size - len
            && self.perms.contains(required)
    }
}

/// Per-task memory region set.
pub type TaskRegions = [TaskMemRegion; MAX_TASK_REGIONS];

/// Validate that a pointer range is allowed by the task's region set.
pub fn validate_user_ptr(
    regions: &TaskRegions,
    addr: usize,
    len: usize,
    required: MemPerms,
) -> bool {
    if len == 0 {
        return true;
    }
    for r in regions {
        if r.allows(addr, len, required) {
            return true;
        }
    }
    false
}

// ═══════════════════════════════════════════════════════════════════════════
// Network device
// ═══════════════════════════════════════════════════════════════════════════

/// Network medium type (maps to smoltcp Medium).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetMedium {
    Ethernet,
    Ieee802154,
}

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

    /// The network medium. Defaults to Ethernet; 802.15.4 radios override.
    fn medium(&self) -> NetMedium { NetMedium::Ethernet }

    /// Extended MAC address (EUI-64) for 802.15.4 radios.
    /// Returns zeros by default (Ethernet devices don't need this).
    fn mac_address_ext(&self) -> [u8; 8] { [0u8; 8] }
}

// ═══════════════════════════════════════════════════════════════════════════
// Block device
// ═══════════════════════════════════════════════════════════════════════════

/// Generic block device interface for SD cards, USB mass storage, NVMe, etc.
pub trait BlockDevice {
    /// Read one block at the given logical block address into `buf`.
    /// `buf` must be at least `block_size()` bytes. Returns `true` on success.
    fn read_block(&self, lba: u64, buf: &mut [u8]) -> bool;

    /// Write one block at the given logical block address from `buf`.
    /// `buf` must be at least `block_size()` bytes. Returns `true` on success.
    fn write_block(&self, lba: u64, buf: &[u8]) -> bool;

    /// Block size in bytes (typically 512).
    fn block_size(&self) -> usize;

    /// Total number of blocks on the device.
    fn block_count(&self) -> u64;
}

// ═══════════════════════════════════════════════════════════════════════════
// Display device
// ═══════════════════════════════════════════════════════════════════════════

/// Pixel-level display device interface (framebuffer, SPI LCD, etc.).
pub trait DisplayDevice {
    /// Display width in pixels.
    fn width(&self) -> u32;
    /// Display height in pixels.
    fn height(&self) -> u32;
    /// Bytes per row (may include padding beyond width × bpp/8).
    fn pitch(&self) -> u32;
    /// Bits per pixel (16, 24, or 32).
    fn bpp(&self) -> u32;
    /// Write a single pixel at (x, y) with the given color value.
    fn set_pixel(&self, x: u32, y: u32, color: u32);
    /// Fill a rectangle with a solid color.
    fn fill_rect(&self, x: u32, y: u32, w: u32, h: u32, color: u32);
    /// Clear the entire display to a color.
    fn clear(&self, color: u32);
    /// Flush pending writes (for double-buffered or SPI displays).
    fn flush(&self) {}
}

// ═══════════════════════════════════════════════════════════════════════════
// Input device
// ═══════════════════════════════════════════════════════════════════════════

/// An input event from a keyboard or mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    /// Key pressed (ASCII code or scan code).
    KeyPress(u8),
    /// Key released.
    KeyRelease(u8),
    /// Relative mouse movement.
    MouseMove { dx: i16, dy: i16 },
    /// Mouse button press/release (button index, true = pressed).
    MouseButton { button: u8, pressed: bool },
    /// No event available.
    None,
}

/// Generic input device (keyboard, mouse, touchpad, etc.).
pub trait InputDevice {
    /// Poll for the next input event. Returns `InputEvent::None` if empty.
    fn poll_event(&self) -> InputEvent;
    /// Returns `true` if at least one event is queued.
    fn has_event(&self) -> bool;
}

// ═══════════════════════════════════════════════════════════════════════════
// GPIO / SPI / I2C bus traits
// ═══════════════════════════════════════════════════════════════════════════

/// GPIO pin operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpioMode {
    Input,
    Output,
    AltFn(u8),
}

/// GPIO internal pull resistor setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpioPull {
    None,
    Up,
    Down,
}

/// Single GPIO pin interface.
pub trait GpioPin {
    fn set_mode(&mut self, mode: GpioMode);
    fn set_pull(&mut self, pull: GpioPull);
    fn read(&self) -> bool;
    fn write(&mut self, high: bool);
}

/// SPI master bus interface.
pub trait SpiBus {
    fn configure(&mut self, clock_hz: u32, mode: u8);
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]);
    fn write(&mut self, data: &[u8]);
}

/// I²C master bus interface.
pub trait I2cBus {
    fn configure(&mut self, clock_hz: u32);
    fn write_read(&mut self, addr: u8, tx: &[u8], rx: &mut [u8]) -> bool;
    fn write_to(&mut self, addr: u8, data: &[u8]) -> bool;
    fn read_from(&mut self, addr: u8, buf: &mut [u8]) -> bool;
}

// ═══════════════════════════════════════════════════════════════════════════
// USB Host Controller
// ═══════════════════════════════════════════════════════════════════════════

/// USB transfer direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDirection {
    In,
    Out,
}

/// USB device speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbSpeed {
    Low,   // 1.5 Mbit/s — mice, keyboards
    Full,  // 12 Mbit/s
    High,  // 480 Mbit/s (USB 2.0)
    Super, // 5 Gbit/s (USB 3.x)
}

/// USB endpoint type (transfer type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbEpType {
    Control,
    Interrupt,
    Bulk,
    Isochronous,
}

/// Descriptor for an attached USB device.
#[derive(Debug, Clone, Copy)]
pub struct UsbDeviceInfo {
    /// Device address assigned by the host (1–127).
    pub address: u8,
    /// USB speed of this device.
    pub speed: UsbSpeed,
    /// Vendor ID.
    pub vendor_id: u16,
    /// Product ID.
    pub product_id: u16,
    /// Device class (from device or interface descriptor).
    pub class: u8,
    /// Device subclass.
    pub subclass: u8,
    /// Device protocol.
    pub protocol: u8,
}

/// USB host controller trait — abstracts xHCI / EHCI / DWC2 / OHCI.
///
/// The HCD owns the root hub ports and manages device enumeration.
/// Higher-level class drivers (HID, mass storage, etc.) interact
/// through control/interrupt/bulk transfer primitives.
pub trait UsbHostController {
    /// Initialise the host controller hardware. Returns `true` on success.
    fn init(&mut self) -> bool;

    /// Reset the host controller.
    fn reset(&mut self) -> bool;

    /// Number of root hub ports.
    fn port_count(&self) -> usize;

    /// Returns `true` if a device is connected to `port` (0-indexed).
    fn port_connected(&self, port: usize) -> bool;

    /// Reset a port and enable it. Returns the detected speed.
    fn port_reset(&mut self, port: usize) -> Option<UsbSpeed>;

    /// Perform a control transfer (SETUP + optional DATA + STATUS).
    ///
    /// - `addr`: target device address (0 during enumeration)
    /// - `setup`: 8-byte SETUP packet
    /// - `data`: optional data phase buffer (direction inferred from `setup`)
    ///
    /// Returns the number of bytes actually transferred in the data phase.
    fn control_transfer(
        &mut self,
        addr: u8,
        setup: &[u8; 8],
        data: Option<&mut [u8]>,
    ) -> Option<usize>;

    /// Submit an interrupt IN transfer (for HID polling).
    ///
    /// - `addr`: device address
    /// - `ep`: endpoint number (1–15)
    /// - `buf`: buffer to receive data
    ///
    /// Returns number of bytes received, or `None` if no data / NAK.
    fn interrupt_in(
        &mut self,
        addr: u8,
        ep: u8,
        buf: &mut [u8],
    ) -> Option<usize>;

    /// Get info about an enumerated device. Returns `None` if no device.
    fn device_info(&self, addr: u8) -> Option<UsbDeviceInfo>;
}
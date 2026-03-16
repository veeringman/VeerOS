#![allow(dead_code)]
//! RP1 GPIO driver for Raspberry Pi 5.
//!
//! The RP1 southbridge provides 28 GPIOs (GPIO0–GPIO27) accessible via
//! PCIe-mapped MMIO.  Each GPIO has a status register and a control register
//! at 8-byte stride.  Pin function selection, pull-up/down, output drive,
//! input read, and interrupt configuration are all in the control register.
//!
//! Reference: RP1 datasheet §3 (GPIO), Raspberry Pi 5 device tree.

use crate::mem;

// ── RP1 GPIO base offset within the BAR ──────────────────────────────────
const GPIO_BASE: usize = mem::RP1_BAR_BASE + 0xD_0000;

// ── Per-pin registers (8-byte stride) ────────────────────────────────────
// Each GPIO has:
//   STATUS  at base + pin*8 + 0  (read-only)
//   CTRL    at base + pin*8 + 4  (read-write)
const PIN_STRIDE: usize = 8;

// STATUS register bits
const STATUS_OUTFROMPERI: u32  = 1 << 8;   // output level from peripheral
const STATUS_OUTTOPAD: u32     = 1 << 9;   // output level to pad
const STATUS_OEFROMPERI: u32   = 1 << 12;  // OE from peripheral
const STATUS_OETOPAD: u32      = 1 << 13;  // OE to pad
const STATUS_INFROMPAD: u32    = 1 << 17;  // input level from pad
const STATUS_INTOPERI: u32     = 1 << 19;  // input level to peripheral
const STATUS_IRQFROMPERI: u32  = 1 << 24;  // IRQ from peripheral
const STATUS_IRQTOPROC: u32    = 1 << 26;  // IRQ to processor

// CTRL register bits
const CTRL_FUNCSEL_MASK: u32   = 0x1F;       // bits [4:0]  — function select
const CTRL_OUTOVER_SHIFT: u32  = 12;         // bits [13:12] — output override
const CTRL_OEOVER_SHIFT: u32   = 14;         // bits [15:14] — OE override
const CTRL_INOVER_SHIFT: u32   = 16;         // bits [17:16] — input override
const CTRL_IRQOVER_SHIFT: u32  = 28;         // bits [29:28] — IRQ override

// ── Pad control registers ────────────────────────────────────────────────
// RP1 pad control starts at GPIO_BASE + 0x4 * (number_of_gpios * 2) + pad_offset
// In practice, at PADS_BASE offset within the RP1 BAR.
const PADS_BASE: usize = mem::RP1_BAR_BASE + 0xF_0000;
const PAD_STRIDE: usize = 4;

// Pad register bits
const PAD_SLEWFAST: u32  = 1 << 0;
const PAD_SCHMITT: u32   = 1 << 1;
const PAD_PDE: u32       = 1 << 2;  // pull-down enable
const PAD_PUE: u32       = 1 << 3;  // pull-up enable
const PAD_DRIVE_MASK: u32 = 0x3 << 4; // drive strength [5:4]
const PAD_IE: u32        = 1 << 6;  // input enable
const PAD_OD: u32        = 1 << 7;  // output disable

// ── GPIO set/clear registers (RIO — Register I/O) ───────────────────────
const RIO_BASE: usize = mem::RP1_BAR_BASE + 0xE_0000;
const RIO_OUT:      usize = 0x00;  // output value
const RIO_OE:       usize = 0x04;  // output enable
const RIO_IN:       usize = 0x08;  // input value
// Each has XOR/SET/CLR variants at +0x1000, +0x2000, +0x3000
const RIO_SET_OFF:  usize = 0x2000;
const RIO_CLR_OFF:  usize = 0x3000;

/// Maximum GPIO pin number (0-based).
pub const GPIO_COUNT: u8 = 28;

/// GPIO function select values (alt functions).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GpioFunc {
    Alt0  = 0,
    Alt1  = 1,
    Alt2  = 2,
    Alt3  = 3,
    Alt4  = 4,
    Alt5  = 5,  // SYS_RIO (software GPIO)
    Alt6  = 6,
    Alt7  = 7,
    Alt8  = 8,
    Null  = 31, // disconnect from peripheral
}

/// GPIO pull-up/down configuration.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GpioPull {
    None,
    Up,
    Down,
}

/// GPIO pin mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GpioMode {
    Input,
    Output,
    Alt(GpioFunc),
}

// ── Low-level MMIO ──────────────────────────────────────────────────────

#[inline]
unsafe fn mmio_read(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline]
unsafe fn mmio_write(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

// ── Public API ──────────────────────────────────────────────────────────

/// Set the function/mode for a GPIO pin.
pub fn set_function(pin: u8, func: GpioFunc) {
    if pin >= GPIO_COUNT { return; }
    let ctrl_addr = GPIO_BASE + (pin as usize) * PIN_STRIDE + 4;
    unsafe {
        let mut ctrl = mmio_read(ctrl_addr);
        ctrl &= !CTRL_FUNCSEL_MASK;
        ctrl |= func as u32;
        mmio_write(ctrl_addr, ctrl);
    }
}

/// Set pin mode (input, output, or alt function).
pub fn set_mode(pin: u8, mode: GpioMode) {
    if pin >= GPIO_COUNT { return; }
    match mode {
        GpioMode::Input => {
            set_function(pin, GpioFunc::Alt5); // SYS_RIO for software control
            // Enable input, disable output
            unsafe {
                // Set input enable on pad
                let pad = PADS_BASE + (pin as usize + 1) * PAD_STRIDE;
                let val = mmio_read(pad);
                mmio_write(pad, val | PAD_IE);
                // Clear output enable bit
                mmio_write(RIO_BASE + RIO_OE + RIO_CLR_OFF, 1 << pin);
            }
        }
        GpioMode::Output => {
            set_function(pin, GpioFunc::Alt5); // SYS_RIO for software control
            unsafe {
                // Enable output
                mmio_write(RIO_BASE + RIO_OE + RIO_SET_OFF, 1 << pin);
                // Disable input, clear output-disable on pad
                let pad = PADS_BASE + (pin as usize + 1) * PAD_STRIDE;
                let val = mmio_read(pad);
                mmio_write(pad, (val & !PAD_OD) | PAD_IE);
            }
        }
        GpioMode::Alt(func) => {
            set_function(pin, func);
        }
    }
}

/// Set pull-up/pull-down on a GPIO pin.
pub fn set_pull(pin: u8, pull: GpioPull) {
    if pin >= GPIO_COUNT { return; }
    let pad = PADS_BASE + (pin as usize + 1) * PAD_STRIDE;
    unsafe {
        let mut val = mmio_read(pad);
        val &= !(PAD_PUE | PAD_PDE);
        match pull {
            GpioPull::Up   => val |= PAD_PUE,
            GpioPull::Down => val |= PAD_PDE,
            GpioPull::None => {}
        }
        mmio_write(pad, val);
    }
}

/// Read the current input level of a GPIO pin (true = high).
pub fn read(pin: u8) -> bool {
    if pin >= GPIO_COUNT { return false; }
    unsafe {
        let val = mmio_read(RIO_BASE + RIO_IN);
        (val >> pin) & 1 != 0
    }
}

/// Set a GPIO output pin high or low.
pub fn write(pin: u8, high: bool) {
    if pin >= GPIO_COUNT { return; }
    let offset = if high { RIO_SET_OFF } else { RIO_CLR_OFF };
    unsafe {
        mmio_write(RIO_BASE + RIO_OUT + offset, 1 << pin);
    }
}

/// Toggle a GPIO output pin.
pub fn toggle(pin: u8) {
    if pin >= GPIO_COUNT { return; }
    unsafe {
        // XOR offset is at +0x1000
        mmio_write(RIO_BASE + RIO_OUT + 0x1000, 1 << pin);
    }
}

/// Read the status register for a GPIO pin (raw 32-bit value).
pub fn read_status(pin: u8) -> u32 {
    if pin >= GPIO_COUNT { return 0; }
    let addr = GPIO_BASE + (pin as usize) * PIN_STRIDE;
    unsafe { mmio_read(addr) }
}

/// Read the control register for a GPIO pin (raw 32-bit value).
pub fn read_ctrl(pin: u8) -> u32 {
    if pin >= GPIO_COUNT { return 0; }
    let addr = GPIO_BASE + (pin as usize) * PIN_STRIDE + 4;
    unsafe { mmio_read(addr) }
}

/// Set the drive strength for a GPIO pin (0=2mA, 1=4mA, 2=8mA, 3=12mA).
pub fn set_drive_strength(pin: u8, strength: u8) {
    if pin >= GPIO_COUNT { return; }
    let pad = PADS_BASE + (pin as usize + 1) * PAD_STRIDE;
    unsafe {
        let mut val = mmio_read(pad);
        val &= !PAD_DRIVE_MASK;
        val |= ((strength as u32) & 0x3) << 4;
        mmio_write(pad, val);
    }
}

/// Enable Schmitt trigger on a GPIO input pin.
pub fn set_schmitt(pin: u8, enable: bool) {
    if pin >= GPIO_COUNT { return; }
    let pad = PADS_BASE + (pin as usize + 1) * PAD_STRIDE;
    unsafe {
        let mut val = mmio_read(pad);
        if enable { val |= PAD_SCHMITT; } else { val &= !PAD_SCHMITT; }
        mmio_write(pad, val);
    }
}

/// Enable fast slew rate on a GPIO output pin.
pub fn set_slew_fast(pin: u8, fast: bool) {
    if pin >= GPIO_COUNT { return; }
    let pad = PADS_BASE + (pin as usize + 1) * PAD_STRIDE;
    unsafe {
        let mut val = mmio_read(pad);
        if fast { val |= PAD_SLEWFAST; } else { val &= !PAD_SLEWFAST; }
        mmio_write(pad, val);
    }
}

/// Write a short GPIO status summary for the shell `gpio` command.
pub fn write_pin_list(w: &mut dyn core::fmt::Write) {
    let _ = writeln!(w, "  Pin  Func  Mode   Pull  Level");
    for pin in 0..GPIO_COUNT {
        let ctrl = read_ctrl(pin);
        let funcsel = ctrl & CTRL_FUNCSEL_MASK;
        let pad_addr = PADS_BASE + (pin as usize + 1) * PAD_STRIDE;
        let pad = unsafe { mmio_read(pad_addr) };
        let pull = if pad & PAD_PUE != 0 { "up  " }
                   else if pad & PAD_PDE != 0 { "down" }
                   else { "none" };
        let level = if read(pin) { "HIGH" } else { "LOW " };
        let oe = unsafe { (mmio_read(RIO_BASE + RIO_OE) >> pin) & 1 };
        let dir = if oe != 0 { "OUT" } else { "IN " };
        let _ = writeln!(w, "  {:>2}   ALT{:<2} {dir}    {pull}  {level}", pin, funcsel);
    }
}


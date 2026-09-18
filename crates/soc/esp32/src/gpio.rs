//! ESP32-C6 GPIO driver.
//!
//! The ESP32-C6 has 31 GPIOs (GPIO0–GPIO30).  Each pin has:
//!   - An IO MUX register for function select and pull-up/down
//!   - GPIO matrix registers for input/output routing
//!   - GPIO output set/clear registers
//!
//! Register base addresses:
//!   GPIO matrix:  0x6009_1000
//!   IO MUX:       0x6000_9000
//!
//! XIAO ESP32-C6 pin mapping (active header pins):
//!   D0/A0  = GPIO0    D1/A1  = GPIO1    D2/A2  = GPIO2
//!   D3/SCL = GPIO21   D4/SDA = GPIO22   D5/SCK = GPIO19
//!   D6/MISO= GPIO20   D7/MOSI= GPIO18   D8     = GPIO23
//!   D9     = GPIO15   D10    = GPIO16   Built-in LED = GPIO15
//!
//! Reference: ESP32-C6 Technical Reference Manual, Chapter 6 (IO MUX / GPIO).

use core::fmt::Write;

// ══════════════════════════════════════════════════════════════════════════
// Register addresses
// ══════════════════════════════════════════════════════════════════════════

/// GPIO matrix base.
const GPIO_BASE: usize = 0x6009_1000;

/// GPIO output register — current output level of all pins.
const GPIO_OUT_REG: usize = GPIO_BASE + 0x04;
/// GPIO output set — write 1 to set pins high (W1TS).
const GPIO_OUT_W1TS: usize = GPIO_BASE + 0x08;
/// GPIO output clear — write 1 to set pins low (W1TC).
const GPIO_OUT_W1TC: usize = GPIO_BASE + 0x0C;
/// GPIO output enable register.
const GPIO_ENABLE_REG: usize = GPIO_BASE + 0x20;
/// GPIO output enable set (W1TS).
const GPIO_ENABLE_W1TS: usize = GPIO_BASE + 0x24;
/// GPIO output enable clear (W1TC).
const GPIO_ENABLE_W1TC: usize = GPIO_BASE + 0x28;
/// GPIO input register — current level of all pins.
const GPIO_IN_REG: usize = GPIO_BASE + 0x3C;

/// GPIO pin configuration registers (one per pin, 4-byte stride).
/// GPIO_PINn_REG = GPIO_PIN0_REG + n * 4
const GPIO_PIN0_REG: usize = GPIO_BASE + 0x74;
const GPIO_PIN_STRIDE: usize = 4;

/// GPIO function output selection registers.
/// GPIO_FUNCn_OUT_SEL_CFG_REG = FUNC_OUT_SEL_BASE + n * 4
const FUNC_OUT_SEL_BASE: usize = GPIO_BASE + 0x554;
/// OUT_SEL = 0x80 routes GPIO_OUT_REG; OEN_SEL (bit 10) uses GPIO_ENABLE_REG
/// for pad OE. Without OEN_SEL the pad OE stays with the peripheral and HP
/// GPIO writes never leave the chip — GPIO3 is the XIAO RF-switch enable.
const GPIO_FUNC_OUT_SEL_GPIO: u32 = 0x80;
const GPIO_FUNC_OEN_SEL: u32 = 1 << 10;

/// LP_AON: GPIO0–7 can be owned by LP_IO. SEL bit=1 routes the pad to LP_IO
/// (HP GPIO is ignored). HOLD freezes the pad regardless of HP writes.
const LP_AON_BASE: usize = 0x600B_1000;
const LP_AON_GPIO_MUX: usize = LP_AON_BASE + 0x28;
const LP_AON_GPIO_HOLD0: usize = LP_AON_BASE + 0x2C;
const LP_IO_BASE: usize = 0x600B_2000;
const LP_IO_OUT_W1TC: usize = LP_IO_BASE + 0x08;
const LP_IO_OE_W1TS: usize = LP_IO_BASE + 0x10;
const LP_IO_IN: usize = LP_IO_BASE + 0x24;
const LP_IO_GPIO0: usize = LP_IO_BASE + 0x48;
const LPPERI_CLK_EN: usize = 0x600B_2800;
const LPPERI_LP_IO_CK_EN: u32 = 1 << 27;
/// Write-1-to-unhold. Bit 29 = all LP pads, bit 31 = all HP pads.
const PMU_IMM_PAD_HOLD_ALL: usize = 0x600B_00E4;
const PMU_UNHOLD_LP: u32 = 1 << 29;
const PMU_UNHOLD_HP: u32 = 1 << 31;

/// IO MUX base address.
const IO_MUX_BASE: usize = 0x6000_9000;
/// IO MUX per-pin register: IO_MUX_GPIOn_REG = IO_MUX_BASE + 0x04 + n * 4
const IO_MUX_PIN_BASE: usize = IO_MUX_BASE + 0x04;
const IO_MUX_STRIDE: usize = 4;

// ── IO MUX register bit positions ────────────────────────────────────────
const IO_MUX_MCU_OE: u32 = 1 << 0;       // output enable in sleep
const IO_MUX_SLP_SEL: u32 = 1 << 1;      // sleep mode select
const IO_MUX_MCU_WPD: u32 = 1 << 2;      // pull-down in sleep
const IO_MUX_MCU_WPU: u32 = 1 << 3;      // pull-up in sleep
const _IO_MUX_MCU_IE: u32 = 1 << 4;      // input enable in sleep
const IO_MUX_FUN_WPD: u32 = 1 << 7;      // pull-down enable
const IO_MUX_FUN_WPU: u32 = 1 << 8;      // pull-up enable
const IO_MUX_FUN_IE: u32 = 1 << 9;       // input enable
const IO_MUX_FUN_DRV_SHIFT: u32 = 10;    // drive strength [11:10]
const IO_MUX_FUN_DRV_MASK: u32 = 0x3 << IO_MUX_FUN_DRV_SHIFT;
const IO_MUX_MCU_SEL_SHIFT: u32 = 12;    // function select [14:12]
const IO_MUX_MCU_SEL_MASK: u32 = 0x7 << IO_MUX_MCU_SEL_SHIFT;
const IO_MUX_FILTER_EN: u32 = 1 << 15;   // input filter enable

/// Function select value for GPIO (simple I/O via GPIO matrix).
const IO_MUX_GPIO_FUNC: u32 = 1;         // function 1 = GPIO

/// GPIO count on ESP32-C6.
pub const GPIO_COUNT: u8 = 31; // GPIO0..GPIO30

// ══════════════════════════════════════════════════════════════════════════
// Low-level MMIO
// ══════════════════════════════════════════════════════════════════════════

#[inline(always)]
unsafe fn read_reg(addr: usize) -> u32 {
    core::ptr::read_volatile(addr as *const u32)
}

#[inline(always)]
unsafe fn write_reg(addr: usize, val: u32) {
    core::ptr::write_volatile(addr as *mut u32, val);
}

#[inline(always)]
unsafe fn set_bits(addr: usize, mask: u32) {
    let v = read_reg(addr);
    write_reg(addr, v | mask);
}

#[inline(always)]
unsafe fn clear_bits(addr: usize, mask: u32) {
    let v = read_reg(addr);
    write_reg(addr, v & !mask);
}

// ══════════════════════════════════════════════════════════════════════════
// Public types
// ══════════════════════════════════════════════════════════════════════════

/// GPIO pull-up/pull-down configuration.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GpioPull {
    None,
    Up,
    Down,
}

/// GPIO direction.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum GpioMode {
    Input,
    Output,
}

// ══════════════════════════════════════════════════════════════════════════
// Public API
// ══════════════════════════════════════════════════════════════════════════

/// Configure a pin as simple GPIO (via IO MUX function select).
fn configure_as_gpio(pin: u8) {
    if pin >= GPIO_COUNT { return; }
    let mux_addr = IO_MUX_PIN_BASE + (pin as usize) * IO_MUX_STRIDE;
    unsafe {
        let mut val = read_reg(mux_addr);
        // Set function to GPIO (func 1)
        val &= !IO_MUX_MCU_SEL_MASK;
        val |= IO_MUX_GPIO_FUNC << IO_MUX_MCU_SEL_SHIFT;
        // GPIO function, no sleep, 20 mA, no pulls. FUN_IE on so GPIO_IN
        // readback is trustworthy (esp-hal leaves it off; we need the diag).
        val |= IO_MUX_FUN_IE;
        val &= !IO_MUX_SLP_SEL;
        val &= !(IO_MUX_FUN_WPU | IO_MUX_FUN_WPD);
        val &= !IO_MUX_FUN_DRV_MASK;
        val |= 2 << IO_MUX_FUN_DRV_SHIFT;
        write_reg(mux_addr, val);
    }

    // Route GPIO output through GPIO matrix: SIG_GPIO_OUT (0x80). Match
    // esp-hal init_gpio: only out_sel, leave oen_sel=0 so OE follows GPIO_ENABLE
    // via the GPIO peripheral signal.
    unsafe {
        let func_out = FUNC_OUT_SEL_BASE + (pin as usize) * 4;
        write_reg(func_out, GPIO_FUNC_OUT_SEL_GPIO);
    }
}

/// Attach GPIO0–7 to the HP IO_MUX and release pad-hold so HP writes reach the pin.
fn attach_hp_gpio(pin: u8) {
    if pin >= 8 {
        return;
    }
    let mask = 1u32 << pin;
    unsafe {
        clear_bits(LP_AON_GPIO_MUX, mask);
        clear_bits(LP_AON_GPIO_HOLD0, mask);
    }
}

/// Set the direction of a GPIO pin.
pub fn set_mode(pin: u8, mode: GpioMode) {
    if pin >= GPIO_COUNT { return; }
    attach_hp_gpio(pin);
    configure_as_gpio(pin);
    let mask = 1u32 << pin;
    match mode {
        GpioMode::Input => {
            // Disable output enable
            unsafe { write_reg(GPIO_ENABLE_W1TC, mask); }
        }
        GpioMode::Output => {
            // Enable output
            unsafe { write_reg(GPIO_ENABLE_W1TS, mask); }
        }
    }
}

/// Set pull-up/pull-down resistor on a pin.
pub fn set_pull(pin: u8, pull: GpioPull) {
    if pin >= GPIO_COUNT { return; }
    let mux_addr = IO_MUX_PIN_BASE + (pin as usize) * IO_MUX_STRIDE;
    unsafe {
        let mut val = read_reg(mux_addr);
        val &= !(IO_MUX_FUN_WPU | IO_MUX_FUN_WPD | IO_MUX_MCU_WPU | IO_MUX_MCU_WPD);
        match pull {
            GpioPull::Up => { val |= IO_MUX_FUN_WPU; }
            GpioPull::Down => { val |= IO_MUX_FUN_WPD; }
            GpioPull::None => {}
        }
        write_reg(mux_addr, val);
    }
}

/// Read the current input level of a GPIO pin.
pub fn read(pin: u8) -> bool {
    if pin >= GPIO_COUNT { return false; }
    unsafe {
        (read_reg(GPIO_IN_REG) >> pin) & 1 != 0
    }
}

/// Set a GPIO output pin high or low.
pub fn write(pin: u8, high: bool) {
    if pin >= GPIO_COUNT { return; }
    let mask = 1u32 << pin;
    if high {
        unsafe { write_reg(GPIO_OUT_W1TS, mask); }
    } else {
        unsafe { write_reg(GPIO_OUT_W1TC, mask); }
    }
}

/// XIAO ESP32-C6 FM8625H RF switch: GPIO3 powers the switch (active low),
/// GPIO14 selects onboard ceramic (low) vs U.FL (high). Arduino `initVariant()`
/// drives these as HP GPIO. GPIO3 is an LP pad — without PMU unhold, HP writes
/// leave GPIO_OUT low while the pad stays high and the radio is deaf.
pub fn enable_xiao_onboard_antenna() {
    unsafe {
        write_reg(PMU_IMM_PAD_HOLD_ALL, PMU_UNHOLD_LP | PMU_UNHOLD_HP);
        set_bits(LPPERI_CLK_EN, LPPERI_LP_IO_CK_EN);
        clear_bits(LP_AON_GPIO_HOLD0, 1 << 3);
        // Digital/HP GPIO owns the pad (Arduino gpio_config path).
        clear_bits(LP_AON_GPIO_MUX, 1 << 3);
    }

    for pin in [3u8, 14] {
        attach_hp_gpio(pin);
        configure_as_gpio(pin);
        write(pin, false);
        unsafe { write_reg(GPIO_ENABLE_W1TS, 1u32 << pin); }
    }
}

/// Snapshot used to prove the RF-switch pads actually moved.
/// (enable, out, hp_in, func_out[3], func_out[14], lp_mux, hold0, lp_in)
pub fn rf_switch_diag() -> (u32, u32, u32, u32, u32, u32, u32, u32) {
    unsafe {
        (
            read_reg(GPIO_ENABLE_REG),
            read_reg(GPIO_OUT_REG),
            read_reg(GPIO_IN_REG),
            read_reg(FUNC_OUT_SEL_BASE + 3 * 4),
            read_reg(FUNC_OUT_SEL_BASE + 14 * 4),
            read_reg(LP_AON_GPIO_MUX),
            read_reg(LP_AON_GPIO_HOLD0),
            read_reg(LP_IO_IN),
        )
    }
}

/// Toggle a GPIO output pin.
pub fn toggle(pin: u8) {
    if pin >= GPIO_COUNT { return; }
    let mask = 1u32 << pin;
    unsafe {
        let cur = read_reg(GPIO_OUT_REG);
        if cur & mask != 0 {
            write_reg(GPIO_OUT_W1TC, mask);
        } else {
            write_reg(GPIO_OUT_W1TS, mask);
        }
    }
}

/// Get the current pin state as a summary: (is_output, is_high, pull).
fn pin_info(pin: u8) -> (bool, bool, GpioPull) {
    if pin >= GPIO_COUNT { return (false, false, GpioPull::None); }
    unsafe {
        let oe = read_reg(GPIO_ENABLE_REG);
        let is_output = (oe >> pin) & 1 != 0;

        let level = if is_output {
            (read_reg(GPIO_OUT_REG) >> pin) & 1 != 0
        } else {
            (read_reg(GPIO_IN_REG) >> pin) & 1 != 0
        };

        let mux = read_reg(IO_MUX_PIN_BASE + (pin as usize) * IO_MUX_STRIDE);
        let pull = if mux & IO_MUX_FUN_WPU != 0 {
            GpioPull::Up
        } else if mux & IO_MUX_FUN_WPD != 0 {
            GpioPull::Down
        } else {
            GpioPull::None
        };

        (is_output, level, pull)
    }
}

/// Write a GPIO pin status listing for the shell.
pub fn write_pin_list(w: &mut dyn Write) {
    let _ = writeln!(w, "  Pin  Mode   Pull  Level");
    let _ = writeln!(w, "  ---  ----   ----  -----");
    for pin in 0..GPIO_COUNT {
        let (is_output, level, pull) = pin_info(pin);
        let mode_s = if is_output { "OUT " } else { "IN  " };
        let pull_s = match pull {
            GpioPull::Up => "UP  ",
            GpioPull::Down => "DOWN",
            GpioPull::None => "NONE",
        };
        let _ = writeln!(w, "  {:>2}   {}   {}  {}",
            pin, mode_s, pull_s, if level { 1 } else { 0 });
    }
}

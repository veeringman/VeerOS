//! GPIO control via syscalls.
//!
//! Provides safe wrappers around `SYS_GPIO_*` syscalls for pin I/O.
//!
//! # Example
//!
//! ```rust,ignore
//! use userlib::gpio;
//!
//! gpio::set_mode(17, gpio::OUTPUT);
//! gpio::write(17, true);           // LED on
//! let state = gpio::read(17);
//! ```

use crate::sys;

/// GPIO mode: input.
pub const INPUT: usize = 0;
/// GPIO mode: output.
pub const OUTPUT: usize = 1;

/// Pull-up/down: none.
pub const PULL_NONE: usize = 0;
/// Pull-up.
pub const PULL_UP: usize = 1;
/// Pull-down.
pub const PULL_DOWN: usize = 2;

const SYS_GPIO_SET_MODE: usize = 0xB0;
const SYS_GPIO_READ: usize = 0xB1;
const SYS_GPIO_WRITE: usize = 0xB2;
const SYS_GPIO_SET_PULL: usize = 0xB3;

/// Set a GPIO pin to input or output mode.
///
/// Returns `true` on success.
pub fn set_mode(pin: u8, mode: usize) -> bool {
    sys::syscall2(SYS_GPIO_SET_MODE, pin as usize, mode).0 == 0
}

/// Read the current level of a GPIO pin.
///
/// Returns `true` if high, `false` if low.
pub fn read(pin: u8) -> bool {
    sys::syscall1(SYS_GPIO_READ, pin as usize) != 0
}

/// Set a GPIO pin high or low.
///
/// Returns `true` on success.
pub fn write(pin: u8, high: bool) -> bool {
    sys::syscall2(SYS_GPIO_WRITE, pin as usize, high as usize).0 == 0
}

/// Configure the internal pull-up/down resistor.
///
/// Use [`PULL_NONE`], [`PULL_UP`], or [`PULL_DOWN`].
pub fn set_pull(pin: u8, pull: usize) -> bool {
    sys::syscall2(SYS_GPIO_SET_PULL, pin as usize, pull).0 == 0
}

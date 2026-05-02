//! Hardware information and board queries.
//!
//! Provides syscall wrappers for temperature and hardware info.
//!
//! # Example
//!
//! ```rust,ignore
//! use userlib::hw;
//!
//! let temp = hw::temperature_millic();
//! println!("CPU: {}.{}°C", temp / 1000, (temp % 1000).abs() / 100);
//! ```

use crate::sys;

const SYS_GET_TEMP: usize = 0xB7;
const SYS_I2C_WRITE: usize = 0xB4;
const SYS_I2C_READ: usize = 0xB5;
const SYS_SPI_TRANSFER: usize = 0xB6;

/// Read the SoC temperature in millidegrees Celsius.
///
/// Returns 0 if the platform doesn't support temperature readout.
pub fn temperature_millic() -> i32 {
    sys::syscall0(SYS_GET_TEMP) as i32
}

/// Perform an I2C write to `addr` on `bus`.
///
/// Returns `Ok(())` on success, `Err(code)` on failure.
pub fn i2c_write(bus: u8, addr: u8, data: &[u8]) -> Result<(), usize> {
    let ret = sys::syscall4(
        SYS_I2C_WRITE,
        bus as usize,
        addr as usize,
        data.as_ptr() as usize,
        data.len(),
    );
    if ret == 0 {
        Ok(())
    } else {
        Err(ret)
    }
}

/// Perform an I2C read from `addr` on `bus`.
///
/// Returns the number of bytes read, or `Err(code)`.
pub fn i2c_read(bus: u8, addr: u8, buf: &mut [u8]) -> Result<usize, usize> {
    let ret = sys::syscall4(
        SYS_I2C_READ,
        bus as usize,
        addr as usize,
        buf.as_mut_ptr() as usize,
        buf.len(),
    );
    if ret != usize::MAX {
        Ok(ret)
    } else {
        Err(ret)
    }
}

/// Perform a full-duplex SPI transfer on `bus`.
///
/// `tx` bytes are sent while `rx` receives clocked-in data.
/// Both slices must have the same length.
pub fn spi_transfer(bus: u8, tx: &[u8], rx: &mut [u8]) -> Result<(), usize> {
    let len = tx.len().min(rx.len());
    let ret = sys::syscall5(
        SYS_SPI_TRANSFER,
        bus as usize,
        tx.as_ptr() as usize,
        rx.as_mut_ptr() as usize,
        len,
        0,
    );
    if ret == 0 {
        Ok(())
    } else {
        Err(ret)
    }
}

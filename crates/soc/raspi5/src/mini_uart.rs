//! BCM2712 PL011 UART driver (RP1-independent).
//!
//! The BCM2712 has a PL011 UART at `0x10_7D00_1000` (`serial@7d001000`
//! in the DTB, compatible `arm,pl011-axi`) that is directly on the SoC —
//! it does NOT go through the RP1 southbridge.
//! When `enable_uart=1` is set in config.txt, the firmware initialises
//! this UART and routes it to GPIO 14/15.
//!
//! This gives us a serial debug channel that works even when RP1 PCIe
//! is completely broken.  Use `minicom -D /dev/ttyACM0 -b 115200` on
//! the host to see output.
//!
//! PL011 register layout:
//!   DR  (+0x00) — data register
//!   FR  (+0x18) — flag register (bit 5 = TXFF, bit 7 = TXFE)

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU8, Ordering};

// ─── BCM2712 PL011 UART base address ────────────────────────────
//
// From bcm2712-rpi-5-b.dtb:
//   serial@7d001000 { compatible = "arm,pl011-axi"; }
//   soc ranges: child 0x7D001000 → parent 0x10_7D00_1000
const UART_BASE: usize = 0x10_7D00_1000;

// PL011 register offsets
const PL011_DR:  usize = UART_BASE + 0x00;   // Data register
const PL011_FR:  usize = UART_BASE + 0x18;   // Flag register

// Flag register bit masks
const FR_TXFF: u32 = 1 << 5;  // TX FIFO full

// Availability cache: 0 = untested, 1 = available, 2 = unavailable
static AVAIL: AtomicU8 = AtomicU8::new(0);

/// Check once whether the mini UART looks initialised.
/// Result is cached so subsequent calls are free.
fn uart_ok() -> bool {
    match AVAIL.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let ok = probe_available();
            AVAIL.store(if ok { 1 } else { 2 }, Ordering::Relaxed);
            ok
        }
    }
}

fn probe_available() -> bool {
    let fr = unsafe { read_volatile(PL011_FR as *const u32) };
    // All-ones or zero means register not mapped / UART not initialised
    fr != 0xFFFF_FFFF && fr != 0
}

/// Write a single byte to the BCM2712 PL011 UART (blocking).
///
/// No-op if the UART is not initialised.
pub fn write_byte(byte: u8) {
    if !uart_ok() { return; }
    unsafe {
        // Wait for TX FIFO to have space (bounded: bail after ~64k spins)
        let mut timeout = 0u32;
        while read_volatile(PL011_FR as *const u32) & FR_TXFF != 0 {
            timeout += 1;
            if timeout > 65_536 {
                // UART stuck — mark unavailable so future calls are no-ops
                AVAIL.store(2, Ordering::Relaxed);
                return;
            }
            core::hint::spin_loop();
        }
        write_volatile(PL011_DR as *mut u32, byte as u32);
    }
}

/// Write a string to the mini UART. No-op if unavailable.
pub fn write_str(s: &str) {
    if !uart_ok() { return; }
    for b in s.bytes() {
        if b == b'\n' {
            write_byte(b'\r');
        }
        write_byte(b);
    }
}

/// Write a hex u32 to the mini UART. No-op if unavailable.
pub fn write_hex32(val: u32) {
    if !uart_ok() { return; }
    for i in (0..8).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        let ch = if nibble < 10 { b'0' + nibble } else { b'a' + nibble - 10 };
        write_byte(ch);
    }
}

/// Write a hex u64 to the mini UART. No-op if unavailable.
pub fn write_hex64(val: u64) {
    if !uart_ok() { return; }
    write_hex32((val >> 32) as u32);
    write_hex32(val as u32);
}

/// Check if the mini UART TX is available (not stuck).
pub fn is_available() -> bool {
    uart_ok()
}

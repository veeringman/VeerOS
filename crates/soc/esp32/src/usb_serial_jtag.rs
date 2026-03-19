//! USB Serial/JTAG controller driver for ESP32-C6 / C3 / H2.
//!
//! On the XIAO ESP32-C6, the USB-C port is wired to the internal
//! USB Serial/JTAG peripheral — **not** UART0.  This driver writes
//! and reads bytes through that peripheral so the VeerOS console
//! appears on the host's `/dev/ttyACM0`.
//!
//! Register reference: ESP32-C6 TRM §29 (USB Serial/JTAG Controller).
//! Register offsets verified against ESP-IDF v5.3 `usb_serial_jtag_reg.h`.

#![allow(dead_code)]

use arch::Serial;
use core::sync::atomic::{AtomicU8, Ordering};

// ---------------------------------------------------------------------------
// Base address (same for C6, C3, H2)
// ---------------------------------------------------------------------------

const USB_SERIAL_JTAG_BASE: usize = 0x6000_F000;

// ---------------------------------------------------------------------------
// Register offsets  (ESP-IDF v5.3 usb_serial_jtag_reg.h)
// ---------------------------------------------------------------------------

/// EP1 — shared TX/RX FIFO data register (64 bytes deep).
const EP1_REG: usize       = 0x00;
/// EP1_CONF — bit 0: WR_DONE (flush TX), bit 1: SERIAL_IN_EP_DATA_FREE (RO),
///            bit 2: SERIAL_OUT_EP_DATA_AVAIL (RO).
const EP1_CONF_REG: usize  = 0x04;
/// INT_RAW — raw (unmasked) interrupt status.
const INT_RAW_REG: usize   = 0x08;
/// INT_ENA — interrupt enable mask (do NOT confuse with CLR).
const INT_ENA_REG: usize   = 0x10;
/// INT_CLR — write-1-to-clear interrupt flags.
const INT_CLR_REG: usize   = 0x14;
/// OUT_EP1_ST — OUT endpoint 1 status; bits [22:16] = received byte count.
const OUT_EP1_ST_REG: usize = 0x3C;

// ---------------------------------------------------------------------------
// EP1_CONF bit masks
// ---------------------------------------------------------------------------

/// Write 1 to signal that we finished writing a packet into the TX FIFO.
const WR_DONE: u32                    = 1 << 0;
/// (Read-only) 1 = TX FIFO is free — previous packet was consumed by host.
const SERIAL_IN_EP_DATA_FREE: u32     = 1 << 1;
/// (Read-only) 1 = RX FIFO has data from host.
const SERIAL_OUT_EP_DATA_AVAIL: u32   = 1 << 2;

// ---------------------------------------------------------------------------
// INT_RAW / INT_CLR bit masks
// ---------------------------------------------------------------------------

/// Bit 1: SOF (Start-of-Frame) received — host sends one every 1 ms once
/// USB enumeration completes.
const SOF_INT: u32                    = 1 << 1;
/// Bit 2: a complete OUT packet was received from the host (RX ready).
const SERIAL_OUT_RECV_PKT_INT: u32    = 1 << 2;
/// Bit 3: the Serial-IN (TX) FIFO is empty after the host consumed the data.
const SERIAL_IN_EMPTY_INT: u32        = 1 << 3;

// ---------------------------------------------------------------------------
// TX FIFO depth & byte counter
// ---------------------------------------------------------------------------

const TX_FIFO_SIZE: u8 = 64;

/// Tracks how many bytes have been written into the TX FIFO since the last
/// flush.  Accessed only from the (single) boot/kernel core, so Relaxed is
/// sufficient.
static TX_FIFO_COUNT: AtomicU8 = AtomicU8::new(0);

// ---------------------------------------------------------------------------
// MMIO helpers
// ---------------------------------------------------------------------------

#[inline(always)]
unsafe fn mmio_read(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

#[inline(always)]
unsafe fn mmio_write(addr: usize, val: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, val) }
}

// ---------------------------------------------------------------------------
// Driver type
// ---------------------------------------------------------------------------

/// Handle to the USB Serial/JTAG peripheral.
pub struct UsbSerialJtag;

impl UsbSerialJtag {
    pub const fn new() -> Self {
        Self
    }

    /// Wait for the USB host to finish enumerating this device.
    ///
    /// Polls for SOF (Start-of-Frame) packets — the host sends one every
    /// 1 ms once the device is enumerated.  Times out after ~2 s.
    pub fn wait_for_usb_ready(&self) {
        // Clear any stale SOF flag.
        unsafe { mmio_write(USB_SERIAL_JTAG_BASE + INT_CLR_REG, SOF_INT) };

        let mut timeout = 2_000_000u32;
        while timeout > 0 {
            let raw = unsafe { mmio_read(USB_SERIAL_JTAG_BASE + INT_RAW_REG) };
            if raw & SOF_INT != 0 {
                unsafe { mmio_write(USB_SERIAL_JTAG_BASE + INT_CLR_REG, SOF_INT) };
                return;
            }
            timeout -= 1;
            core::hint::spin_loop();
        }
    }

    /// Wait until the TX FIFO is free (previous packet consumed by host).
    #[inline]
    fn wait_tx_free(&self) {
        let mut timeout = 500_000u32;
        while timeout > 0 {
            let conf = unsafe { mmio_read(USB_SERIAL_JTAG_BASE + EP1_CONF_REG) };
            if conf & SERIAL_IN_EP_DATA_FREE != 0 {
                return;
            }
            timeout -= 1;
            core::hint::spin_loop();
        }
    }

    /// Commit the TX FIFO contents to the USB controller and wait for the
    /// host to consume them.
    fn flush_and_wait(&self) {
        // Signal "write done" — the hardware will schedule the USB IN transfer.
        unsafe { mmio_write(USB_SERIAL_JTAG_BASE + EP1_CONF_REG, WR_DONE) };

        // Wait until the host reads the data and the FIFO becomes free again.
        self.wait_tx_free();

        TX_FIFO_COUNT.store(0, Ordering::Relaxed);
    }
}

impl Serial for UsbSerialJtag {
    fn write_byte(&self, byte: u8) {
        let count = TX_FIFO_COUNT.load(Ordering::Relaxed);

        // If we're starting a fresh packet, make sure the FIFO is actually free
        // (the previous packet may still be in flight).
        if count == 0 {
            self.wait_tx_free();
        }

        // Push byte into the 64-byte hardware FIFO.
        unsafe { mmio_write(USB_SERIAL_JTAG_BASE + EP1_REG, byte as u32) };
        let new_count = count + 1;

        // Commit the FIFO when it's full (64 bytes) or on newline (line-buffer).
        if new_count >= TX_FIFO_SIZE || byte == b'\n' {
            self.flush_and_wait();
        } else {
            TX_FIFO_COUNT.store(new_count, Ordering::Relaxed);
        }
    }

    fn flush(&self) {
        let count = TX_FIFO_COUNT.load(Ordering::Relaxed);
        if count > 0 {
            self.flush_and_wait();
        }
    }

    fn read_byte(&self) -> u8 {
        // Flush any pending TX so the prompt is visible before blocking.
        self.flush();

        // Wait until the host sends us data.
        loop {
            let conf = unsafe { mmio_read(USB_SERIAL_JTAG_BASE + EP1_CONF_REG) };
            if conf & SERIAL_OUT_EP_DATA_AVAIL != 0 {
                break;
            }
            core::hint::spin_loop();
        }
        let byte = unsafe { mmio_read(USB_SERIAL_JTAG_BASE + EP1_REG) } as u8;

        // If the FIFO is now empty, clear the RX interrupt so the hardware
        // can accept the next OUT packet from the host.
        let conf = unsafe { mmio_read(USB_SERIAL_JTAG_BASE + EP1_CONF_REG) };
        if conf & SERIAL_OUT_EP_DATA_AVAIL == 0 {
            unsafe { mmio_write(USB_SERIAL_JTAG_BASE + INT_CLR_REG, SERIAL_OUT_RECV_PKT_INT) };
        }
        byte
    }

    fn has_data(&self) -> bool {
        let conf = unsafe { mmio_read(USB_SERIAL_JTAG_BASE + EP1_CONF_REG) };
        (conf & SERIAL_OUT_EP_DATA_AVAIL) != 0
    }
}

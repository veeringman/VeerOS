//! 16550A-compatible UART emulation for PIO 0x3F8 (COM1).
//!
//! Minimal register semantics — just enough that a Rust `no_std` kernel
//! using a standard 16550 driver can print to stdout and receive typed
//! keystrokes from host stdin. Register layout:
//!
//! | Offset | Read          | Write         |
//! |--------|---------------|---------------|
//! | 0      | RBR (input)   | THR (output)  |
//! | 1      | IER           | IER           |
//! | 2      | IIR           | FCR           |
//! | 3      | LCR           | LCR           |
//! | 4      | MCR           | MCR           |
//! | 5      | LSR           | (ignored)     |
//! | 6      | MSR           | (ignored)     |
//! | 7      | SCR           | SCR           |
//!
//! When LCR.DLAB is set, offsets 0/1 become DLL/DLM (divisor latch).
//! Interrupts: when `IER.ERBFI` (bit 0) is set and `LSR.DR` is 1, we
//! assert IRQ 4 on the guest's in-kernel IRQ chip. When RBR is read and
//! the queue empties, the line is deasserted.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use crate::irq::IrqLine;

pub struct SerialSnapshot {
    pub ier: u8,
    pub lcr: u8,
    pub mcr: u8,
    pub scr: u8,
    pub dll: u8,
    pub dlm: u8,
    pub rx: Vec<u8>,
}

const LSR_DR: u8 = 1 << 0; // data ready
const LSR_THRE: u8 = 1 << 5; // transmit holding register empty
const LSR_TEMT: u8 = 1 << 6; // transmitter empty

const IER_ERBFI: u8 = 1 << 0; // enable received-data-available interrupt

/// Legacy COM1 IRQ line.
pub const COM1_IRQ: u32 = 4;

/// Shared state between the RX reader thread and the vCPU PIO handler.
pub struct SerialShared {
    pub rx: Mutex<VecDeque<u8>>,
}

impl SerialShared {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            rx: Mutex::new(VecDeque::with_capacity(256)),
        })
    }
}

pub struct Serial16550 {
    out: Box<dyn Write + Send>,
    ier: u8,
    lcr: u8,
    mcr: u8,
    scr: u8,
    dll: u8,
    dlm: u8,

    shared: Arc<SerialShared>,
    irq: Arc<dyn IrqLine + Send + Sync>,
    irq_asserted: bool,
}

impl Serial16550 {
    pub fn new(
        out: Box<dyn Write + Send>,
        shared: Arc<SerialShared>,
        irq: Arc<dyn IrqLine + Send + Sync>,
    ) -> Self {
        Self {
            out,
            ier: 0,
            lcr: 0,
            mcr: 0,
            scr: 0,
            dll: 0,
            dlm: 0,
            shared,
            irq,
            irq_asserted: false,
        }
    }

    pub fn stdout(shared: Arc<SerialShared>, irq: Arc<dyn IrqLine + Send + Sync>) -> Self {
        Self::new(Box::new(io::stdout()), shared, irq)
    }

    fn dlab(&self) -> bool {
        (self.lcr & 0x80) != 0
    }

    fn has_rx(&self) -> bool {
        !self.shared.rx.lock().unwrap().is_empty()
    }

    /// Drive IRQ 4 based on `IER.ERBFI & LSR.DR`. Safe to call repeatedly;
    /// only issues an ioctl on edge transitions.
    fn update_irq(&mut self) {
        let want = (self.ier & IER_ERBFI) != 0 && self.has_rx();
        if want != self.irq_asserted {
            self.irq.set_irq_line(COM1_IRQ, want);
            self.irq_asserted = want;
        }
    }

    pub fn io_in(&mut self, port: u16, data: &mut [u8]) {
        if data.is_empty() {
            return;
        }
        let offset = (port & 0x7) as u8;
        let v = match offset {
            0 if self.dlab() => self.dll,
            0 => {
                // RBR — pop one byte (or 0 if empty).
                let mut q = self.shared.rx.lock().unwrap();
                q.pop_front().unwrap_or(0)
            }
            1 if self.dlab() => self.dlm,
            1 => self.ier,
            2 => {
                // IIR: bit 0 clear = interrupt pending; bits 3:1 = cause.
                // 0b0100 = received data available.
                if (self.ier & IER_ERBFI) != 0 && self.has_rx() {
                    0b0000_0100
                } else {
                    0b0000_0001
                }
            }
            3 => self.lcr,
            4 => self.mcr,
            5 => {
                let mut lsr = LSR_THRE | LSR_TEMT;
                if self.has_rx() {
                    lsr |= LSR_DR;
                }
                lsr
            }
            6 => 0, // MSR — report nothing
            7 => self.scr,
            _ => 0,
        };
        data[0] = v;
        for b in &mut data[1..] {
            *b = 0;
        }

        // Reading RBR / IIR may clear the pending IRQ condition.
        if offset == 0 || offset == 2 {
            self.update_irq();
        }
    }

    pub fn io_out(&mut self, port: u16, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        let offset = (port & 0x7) as u8;
        let v = data[0];
        match offset {
            0 if self.dlab() => self.dll = v,
            0 => {
                // THR — transmit byte to host.
                let _ = self.out.write_all(&[v]);
                let _ = self.out.flush();
            }
            1 if self.dlab() => self.dlm = v,
            1 => {
                self.ier = v;
                self.update_irq();
            }
            2 => {} // FCR — ignore
            3 => self.lcr = v,
            4 => self.mcr = v,
            5 | 6 => {} // read-only
            7 => self.scr = v,
            _ => {}
        }
    }

    /// Re-evaluate IRQ after external RX queue change.
    pub fn kick_rx(&mut self) {
        self.update_irq();
    }

    pub fn snapshot(&self) -> SerialSnapshot {
        let rx = self.shared.rx.lock().unwrap().iter().copied().collect();
        SerialSnapshot {
            ier: self.ier,
            lcr: self.lcr,
            mcr: self.mcr,
            scr: self.scr,
            dll: self.dll,
            dlm: self.dlm,
            rx,
        }
    }

    pub fn restore(&mut self, state: &SerialSnapshot) {
        self.ier = state.ier;
        self.lcr = state.lcr;
        self.mcr = state.mcr;
        self.scr = state.scr;
        self.dll = state.dll;
        self.dlm = state.dlm;
        let mut rx = self.shared.rx.lock().unwrap();
        *rx = state.rx.iter().copied().collect::<VecDeque<_>>();
        drop(rx);
        self.irq_asserted = false;
        self.update_irq();
    }
}

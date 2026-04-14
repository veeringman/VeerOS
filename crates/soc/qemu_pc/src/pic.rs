//! Intel 8259 PIC (Programmable Interrupt Controller) driver.
//!
//! On x86 PCs, there are two cascaded PICs (master + slave). At boot the
//! BIOS maps IRQ 0–7 to vectors 8–15, which overlaps CPU exceptions 8–15.
//! We remap them to vectors 32–47 so timer (IRQ0) becomes vector 32, etc.
//!
//! After APIC-based timer is implemented, the PIC can be fully masked.

use crate::{outb, io_wait};

// Master PIC (IRQ 0–7).
const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;

// Slave PIC (IRQ 8–15).
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

/// ICW1: initialisation + ICW4 needed.
const ICW1_INIT: u8 = 0x10;
const ICW1_ICW4: u8 = 0x01;
/// ICW4: 8086 mode.
const ICW4_8086: u8 = 0x01;

/// PIC End-Of-Interrupt command.
pub const EOI: u8 = 0x20;

/// Vector offset for master PIC (IRQ 0 → vector 32).
pub const PIC1_OFFSET: u8 = 32;
/// Vector offset for slave PIC (IRQ 8 → vector 40).
pub const PIC2_OFFSET: u8 = 40;

/// Remap both PICs to vectors 32–47 and mask all IRQs except
/// IRQ 0 (timer) and IRQ 4 (COM1).
pub fn init() {
    unsafe {
        // Save current masks.
        // let mask1 = inb(PIC1_DATA);
        // let mask2 = inb(PIC2_DATA);

        // Start initialisation sequence (cascade mode).
        outb(PIC1_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();
        outb(PIC2_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();

        // ICW2: vector offsets.
        outb(PIC1_DATA, PIC1_OFFSET);
        io_wait();
        outb(PIC2_DATA, PIC2_OFFSET);
        io_wait();

        // ICW3: master has slave on IRQ2, slave ID is 2.
        outb(PIC1_DATA, 0x04); // bit 2 = IRQ2 has slave
        io_wait();
        outb(PIC2_DATA, 0x02); // slave ID = 2
        io_wait();

        // ICW4: 8086 mode.
        outb(PIC1_DATA, ICW4_8086);
        io_wait();
        outb(PIC2_DATA, ICW4_8086);
        io_wait();

        // Mask all IRQs except IRQ 0 (timer), IRQ 1 (keyboard),
        // IRQ 2 (cascade), and IRQ 4 (COM1).
        // Unmask bits: 0=IRQ0, 1=IRQ1, 2=IRQ2, 4=IRQ4 → mask = ~0x17 = 0xE8
        outb(PIC1_DATA, 0xE8);   // unmask IRQ0 + IRQ1 + IRQ2 + IRQ4
        outb(PIC2_DATA, 0xFF);    // mask all slave IRQs
    }
}

/// Send End-Of-Interrupt to the appropriate PIC(s).
pub fn send_eoi(irq: u8) {
    unsafe {
        if irq >= 8 {
            outb(PIC2_CMD, EOI);
        }
        outb(PIC1_CMD, EOI);
    }
}

/// Unmask a specific IRQ line (0–15).
pub fn unmask(irq: u8) {
    unsafe {
        if irq < 8 {
            let mask = crate::inb(PIC1_DATA) & !(1 << irq);
            outb(PIC1_DATA, mask);
        } else {
            let mask = crate::inb(PIC2_DATA) & !(1 << (irq - 8));
            outb(PIC2_DATA, mask);
        }
    }
}

/// Minimal IRQ injection interface used by emulated devices.
pub trait IrqLine {
    fn set_irq_line(&self, line: u32, level: bool);
}
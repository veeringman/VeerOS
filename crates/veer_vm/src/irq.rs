/// Minimal IRQ injection interface used by emulated devices.
#[allow(dead_code)]
pub trait IrqLine {
    fn set_irq_line(&self, line: u32, level: bool);
}

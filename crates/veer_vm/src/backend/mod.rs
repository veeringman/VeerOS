/// Backend-agnostic VM exit reasons used by orchestrator code.
#[derive(Clone, Debug)]
pub enum ExitReason {
    IoIn { port: u16, len: usize },
    IoOut { port: u16, data: Vec<u8> },
    MmioRead { addr: u64, len: usize },
    MmioWrite { addr: u64, data: Vec<u8> },
    Hlt,
    Shutdown,
    Intr,
    Other(String),
}

/// Minimal backend contract. KVM/HVF implementations will fill this in.
pub trait Backend {
    fn name(&self) -> &'static str;
}

#[cfg(target_os = "linux")]
pub mod kvm;
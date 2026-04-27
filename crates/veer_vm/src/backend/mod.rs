//! VeerOS Multi-Backend Virtualization Architecture
//!
//! This module provides a pluggable backend abstraction for VeerOS guest execution.
//! Veer-VM supports multiple host platforms through platform-specific backends.
//!
//! # Backend Architecture
//!
//! Each backend implements host-specific virtualization APIs while maintaining
//! a common interface for VM orchestration. The Veer hypervisor core is portable;
//! backends handle platform differences.
//!
//! | Host OS      | Backend Framework          | Codename       |
//! |--------------|----------------------------|----------------|
//! | macOS (Intel)| Apple Hypervisor.framework | **VeerHV-Mac** |
//! | Linux (x64)  | KVM (/dev/kvm)             | **VeerKVM-Linux**|
//! | Windows      | WHPX (Windows Hypervisor)  | **VeerWHPX-Win** |
//!
//! # Compilation
//!
//! Backend selection is automatic via conditional compilation:
//! - `target_os = "linux"` → KVM backend
//! - `target_os = "macos"` → Hypervisor.framework backend
//!
//! # macOS Critical Requirement
//!
//! On macOS, executables invoking `hv_vm_create()` MUST be code-signed with:
//! ```xml
//! <key>com.apple.security.hypervisor</key>
//! <true/>
//! ```
//! See [veer-vm.entitlements](../veer-vm.entitlements) and [build-mac.sh](../../scripts/build-mac.sh).

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

#[cfg(target_os = "macos")]
pub mod hvf;
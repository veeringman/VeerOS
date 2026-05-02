//! Compatibility shim for legacy `crate::vm::*` references.
//!
//! The Linux/KVM implementation now lives in `backend::kvm`.

#[cfg(target_os = "linux")]
pub use crate::backend::kvm::run;
#[cfg(target_os = "linux")]
pub(crate) use crate::backend::kvm::{install_signal_handlers, request_shutdown, SHUTDOWN};

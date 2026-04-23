//! Engine abstraction — platform-specific fold lifecycle primitives.
//!
//! The host CLI talks to one of these backends selected at compile time:
//!   - Linux: namespaces + (future) cgroups + seccomp
//!   - macOS / Windows: currently stubs returning `Unsupported`

use anyhow::Result;

use crate::manifest::Manifest;
use crate::state::FoldRecord;

pub trait Engine {
    /// Launch a fold, return the recorded state on success.
    fn spawn(&self, manifest: Manifest) -> Result<FoldRecord>;

    /// Send SIGTERM (or platform equivalent) to the fold's init process.
    fn stop(&self, rec: &FoldRecord) -> Result<()>;

    /// True if the fold's init PID is still alive.
    fn is_alive(&self, rec: &FoldRecord) -> bool;

    /// Tear down any engine-specific state (cgroups, etc.) attached to a
    /// fold. Called by `fold rm`. Missing/stale state is not an error.
    fn cleanup(&self, _rec: &FoldRecord) -> Result<()> { Ok(()) }
}

#[cfg(target_os = "linux")]
mod cgroup;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod seccomp;
#[cfg(target_os = "linux")]
pub use linux::LinuxEngine as PlatformEngine;

#[cfg(not(target_os = "linux"))]
mod stub;
#[cfg(not(target_os = "linux"))]
pub use stub::StubEngine as PlatformEngine;

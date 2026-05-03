//! Compatibility shim for legacy `crate::vm::*` references.
//!
//! The Linux/KVM implementation now lives in `backend::kvm`.

#[cfg(target_os = "linux")]
pub use crate::backend::kvm::run;
#[cfg(target_os = "linux")]
pub(crate) use crate::backend::kvm::{install_signal_handlers, request_shutdown, SHUTDOWN};

#[cfg(target_os = "macos")]
use anyhow::{bail, Result};
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "macos")]
pub(crate) static SHUTDOWN: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
extern "C" fn sigusr1_handler(_: libc::c_int) {
	// No-op: only used to interrupt blocking syscalls in the main thread.
}

#[cfg(target_os = "macos")]
extern "C" fn sigterm_handler(_: libc::c_int) {
	SHUTDOWN.store(true, Ordering::SeqCst);
}

#[cfg(target_os = "macos")]
pub(crate) fn install_signal_handlers() -> Result<()> {
	unsafe {
		if libc::signal(
			libc::SIGUSR1,
			sigusr1_handler as *const () as libc::sighandler_t,
		) == libc::SIG_ERR
		{
			bail!("signal(SIGUSR1) failed");
		}
		if libc::signal(
			libc::SIGTERM,
			sigterm_handler as *const () as libc::sighandler_t,
		) == libc::SIG_ERR
		{
			bail!("signal(SIGTERM) failed");
		}
		if libc::signal(
			libc::SIGHUP,
			sigterm_handler as *const () as libc::sighandler_t,
		) == libc::SIG_ERR
		{
			bail!("signal(SIGHUP) failed");
		}
	}
	Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn request_shutdown(main_tid: libc::pthread_t) {
	SHUTDOWN.store(true, Ordering::SeqCst);
	unsafe {
		libc::pthread_kill(main_tid, libc::SIGUSR1);
	}
}

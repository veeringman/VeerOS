//! VeerOS user-space runtime library.
//!
//! This crate is the **only** interface a user task needs to interact with
//! the VeerOS microkernel. It provides:
//!
//! - [`sys`] — raw syscall ABI (`ecall` on RISC-V)
//! - [`ipc`] — typed message-passing wrappers
//! - [`task`] — task control (yield, sleep, exit, spawn info)
//! - [`io`] — formatted printing via the kernel console
//! - [`time`] — tick counter and delay helpers
//!
//! # Design principles
//!
//! 1. **`no_std`, no heap** — everything works with fixed-size buffers.
//! 2. **Thin wrappers** — syscalls compile to a single `ecall` instruction
//!    plus argument setup in registers.
//! 3. **Safe API** — the public surface is safe Rust; the one `unsafe`
//!    boundary is the raw `ecall` in [`sys`].
//!
//! # Example
//!
//! ```rust,ignore
//! #![no_std]
//! #![no_main]
//!
//! use userlib::{println, task, ipc};
//!
//! #[no_mangle]
//! pub extern "C" fn main() -> ! {
//!     println!("Hello from user task!");
//!     let ticks = task::uptime();
//!     println!("Uptime: {} ticks", ticks);
//!     task::exit(0);
//! }
//! ```

#![no_std]

pub mod sys;
pub mod ipc;
pub mod task;
pub mod io;
pub mod time;
pub mod sync;
pub mod channel;
pub mod poll;
pub mod async_rt;
pub mod socket;
pub mod user;
pub mod fs;

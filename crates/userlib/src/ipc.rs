//! IPC (inter-process communication) syscall wrappers.
//!
//! VeerOS IPC is synchronous, single-slot mailbox messaging. Each task
//! has one mailbox; `send()` writes into the destination's mailbox and
//! `recv()` blocks until a message arrives.

use crate::sys;

// Syscall numbers (must match microkernel::syscall)
const SYS_IPC_SEND: usize = 0x10;
const SYS_IPC_RECV: usize = 0x11;
const SYS_IPC_POLL: usize = 0x12;

/// A decoded IPC message received from another task.
#[derive(Debug, Clone, Copy)]
pub struct Message {
    /// Task ID of the sender.
    pub sender: u8,
    /// Opcode / service number (meaning defined by the receiver).
    pub opcode: u8,
    /// First payload word.
    pub arg0: usize,
    /// Second payload word.
    pub arg1: usize,
}

/// Send a message to `dest`.
///
/// - `dest` — target task ID
/// - `opcode` — service number (meaning agreed upon by both tasks)
/// - `arg0`, `arg1` — payload words
///
/// Returns `true` if the kernel accepted the message, `false` if the
/// destination slot is invalid or free.
#[inline]
pub fn send(dest: u8, opcode: u8, arg0: usize, arg1: usize) -> bool {
    let ret = sys::syscall4(SYS_IPC_SEND, dest as usize, opcode as usize, arg0, arg1);
    ret != 0
}

/// Receive a message (blocking).
///
/// If the mailbox is empty, the calling task is blocked until another
/// task sends it a message. On return, the message is consumed.
#[inline]
pub fn recv() -> Message {
    let (r0, r1) = sys::syscall2(SYS_IPC_RECV, 0, 0);
    Message {
        sender: (r0 & 0xFF) as u8,
        opcode: ((r0 >> 8) & 0xFF) as u8,
        arg0: r1,
        arg1: 0, // second word not returned in current ABI
    }
}

/// Non-blocking check: is there a message waiting in the mailbox?
#[inline]
pub fn poll() -> bool {
    sys::syscall0(SYS_IPC_POLL) != 0
}

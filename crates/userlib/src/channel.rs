//! Bounded channel API for VeerOS user tasks.
//!
//! Wraps the kernel channel syscalls (`SYS_CHAN_CREATE` / `SYS_CHAN_SEND` /
//! `SYS_CHAN_RECV` / `SYS_CHAN_CLOSE`) into a safe Rust API.
//!
//! # Example
//!
//! ```rust,ignore
//! let ch = channel::create().unwrap();
//! channel::send(ch, 42, 0, 0, 0);   // blocks if channel full
//! let (w0, w1, w2, w3) = channel::recv(ch); // blocks if channel empty
//! channel::close(ch);
//! ```

use crate::sys;

const SYS_CHAN_CREATE: usize = 0x58;
const SYS_CHAN_SEND: usize   = 0x59;
const SYS_CHAN_RECV: usize   = 0x5A;
const SYS_CHAN_CLOSE: usize  = 0x5B;
const SYS_CHAN_POLL: usize   = 0x5C;

/// Create a new bounded channel.
///
/// Returns the channel ID, or `None` if the channel pool is exhausted.
#[inline]
pub fn create() -> Option<usize> {
    let id = sys::syscall0(SYS_CHAN_CREATE);
    if id == usize::MAX { None } else { Some(id) }
}

/// Send four words on a channel. Blocks if the channel is full.
///
/// Returns `true` on success, `false` if the channel was closed.
#[inline]
pub fn send(chan_id: usize, word0: usize, word1: usize, word2: usize, word3: usize) -> bool {
    sys::syscall5(SYS_CHAN_SEND, chan_id, word0, word1, word2, word3) == 1
}

/// Receive four words from a channel. Blocks if the channel is empty.
///
/// Returns `(word0, word1, word2, word3)`. On a closed channel,
/// returns `(usize::MAX, 0, 0, 0)`.
#[inline]
pub fn recv(chan_id: usize) -> (usize, usize, usize, usize) {
    sys::syscall_ret4(SYS_CHAN_RECV, chan_id)
}

/// Close a channel, waking all blocked senders and receivers.
///
/// Returns `true` on success, `false` if the channel ID was invalid.
#[inline]
pub fn close(chan_id: usize) -> bool {
    sys::syscall1(SYS_CHAN_CLOSE, chan_id) == 1
}

/// Non-blocking poll: returns the number of messages in the channel.
///
/// Returns 0 if the channel is closed or invalid.
#[inline]
pub fn poll(chan_id: usize) -> usize {
    sys::syscall1(SYS_CHAN_POLL, chan_id)
}

// ---------------------------------------------------------------------------
// Typed Channel<T> wrapper
// ---------------------------------------------------------------------------

/// A typed bounded channel that transports values of type `T`.
///
/// `T` must fit in two `usize` words (i.e. `size_of::<T>() <= 2 * size_of::<usize>()`).
/// Values are bitwise-copied through the kernel channel.
///
/// # Example
///
/// ```rust,ignore
/// let ch: Channel<u32> = Channel::new().unwrap();
/// ch.send(42);
/// let val = ch.recv().unwrap();
/// assert_eq!(val, 42);
/// ch.close();
/// ```
pub struct Channel<T> {
    id: usize,
    _marker: core::marker::PhantomData<T>,
}

impl<T: Copy> Channel<T> {
    /// Create a new typed channel. Returns `None` if the pool is exhausted.
    pub fn new() -> Option<Self> {
        // Compile-time check: T must fit in four words.
        const { assert!(core::mem::size_of::<T>() <= 4 * core::mem::size_of::<usize>()) };
        create().map(|id| Channel { id, _marker: core::marker::PhantomData })
    }

    /// Send a value. Blocks if the channel is full.
    pub fn send(&self, val: T) -> bool {
        let mut buf = [0usize; 4];
        // Safety: T fits in 4 words (checked at compile time), copy bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(
                &val as *const T as *const u8,
                buf.as_mut_ptr() as *mut u8,
                core::mem::size_of::<T>(),
            );
        }
        send(self.id, buf[0], buf[1], buf[2], buf[3])
    }

    /// Receive a value. Blocks if the channel is empty.
    /// Returns `None` if the channel was closed.
    pub fn recv(&self) -> Option<T> {
        let (w0, w1, w2, w3) = recv(self.id);
        if w0 == usize::MAX && w1 == 0 && w2 == 0 && w3 == 0 {
            return None; // channel closed
        }
        let buf = [w0, w1, w2, w3];
        let mut val = unsafe { core::mem::zeroed::<T>() };
        unsafe {
            core::ptr::copy_nonoverlapping(
                buf.as_ptr() as *const u8,
                &mut val as *mut T as *mut u8,
                core::mem::size_of::<T>(),
            );
        }
        Some(val)
    }

    /// Non-blocking poll: number of pending messages.
    pub fn pending(&self) -> usize {
        poll(self.id)
    }

    /// Close the channel.
    pub fn close(self) -> bool {
        close(self.id)
    }

    /// Return the underlying channel ID.
    pub fn id(&self) -> usize {
        self.id
    }
}

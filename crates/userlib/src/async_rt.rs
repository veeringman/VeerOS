//! Lightweight async runtime for VeerOS user tasks.
//!
//! Provides a minimal `no_std`, no-alloc executor and ready-made
//! [`Future`] types built on the kernel poll subsystem.
//!
//! # Usage
//!
//! ```rust,ignore
//! use userlib::async_rt::{block_on, AsyncTimer};
//!
//! block_on(async {
//!     AsyncTimer::new(100).await;  // sleep 100 ticks
//!     // ...
//! });
//! ```

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use crate::poll as kpoll;

// ───────────────────────────────────────────────────────────────────
// Waker — no-op, because we use SYS_POLL_WAIT to block
// ───────────────────────────────────────────────────────────────────

fn noop_raw_waker() -> RawWaker {
    fn no_op(_: *const ()) {}
    fn clone_fn(_: *const ()) -> RawWaker { noop_raw_waker() }
    static VTABLE: RawWakerVTable =
        RawWakerVTable::new(clone_fn, no_op, no_op, no_op);
    RawWaker::new(core::ptr::null(), &VTABLE)
}

fn noop_waker() -> Waker {
    unsafe { Waker::from_raw(noop_raw_waker()) }
}

// ───────────────────────────────────────────────────────────────────
// block_on — drive a Future to completion
// ───────────────────────────────────────────────────────────────────

/// Run a future to completion on the current task.
///
/// Each time the future returns [`Poll::Pending`] we issue a
/// `SYS_POLL_WAIT` to let the kernel block us until a registered
/// event fires, avoiding busy-spinning.
pub fn block_on<F: Future>(mut f: F) -> F::Output {
    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    // SAFETY: we never move `f` after pinning.
    let mut f = unsafe { Pin::new_unchecked(&mut f) };

    loop {
        match f.as_mut().poll(&mut cx) {
            Poll::Ready(val) => return val,
            Poll::Pending => {
                // Block until the kernel delivers any registered event.
                // The futures below call `poll_set` before returning
                // Pending, so the kernel knows what to watch for.
                kpoll::poll_wait(usize::MAX);
            }
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// AsyncTimer — await a tick-based delay
// ───────────────────────────────────────────────────────────────────

/// A [`Future`] that completes after `ticks` kernel ticks have elapsed.
///
/// On first poll it records the deadline and registers a `POLL_TIMER`
/// event; subsequent polls check whether the deadline has passed.
pub struct AsyncTimer {
    ticks: u32,
    deadline: u64,
}

impl AsyncTimer {
    /// Create a timer that fires after `ticks` kernel ticks.
    pub fn new(ticks: u32) -> Self {
        Self { ticks, deadline: 0 }
    }
}

impl Future for AsyncTimer {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        let this = self.get_mut();
        let now = crate::time::ticks();

        if this.deadline == 0 {
            // First poll — record deadline.
            this.deadline = now + this.ticks as u64;
        }

        if now >= this.deadline {
            return Poll::Ready(());
        }

        // Tell the kernel to wake us when the timer fires.
        kpoll::poll_set(kpoll::POLL_TIMER, this.ticks as usize);
        Poll::Pending
    }
}

// ───────────────────────────────────────────────────────────────────
// AsyncRecv — await data on a channel
// ───────────────────────────────────────────────────────────────────

/// A [`Future`] that completes when a channel has data to read.
///
/// Resolves with the two-word message `(word0, word1)`.
pub struct AsyncRecv {
    chan_id: usize,
}

impl AsyncRecv {
    /// Create a future that resolves when `chan_id` is readable.
    pub fn new(chan_id: usize) -> Self {
        Self { chan_id }
    }
}

impl Future for AsyncRecv {
    type Output = (usize, usize, usize, usize);

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<(usize, usize, usize, usize)> {
        let this = self.get_mut();

        // Attempt a non-blocking receive by polling the channel.
        // We first check readability via a non-blocking poll_wait.
        kpoll::poll_set(kpoll::POLL_CHAN_READABLE, this.chan_id);
        let fired = kpoll::poll_wait(0); // non-blocking check

        if fired & kpoll::POLL_CHAN_READABLE != 0 {
            // Channel has data — do the actual receive (which should
            // return immediately since there's data).
            let msg = crate::channel::recv(this.chan_id);
            Poll::Ready(msg)
        } else {
            // Re-register interest and return Pending.
            // The block_on executor will call SYS_POLL_WAIT(MAX) to
            // block until the kernel sees data on the channel.
            kpoll::poll_set(kpoll::POLL_CHAN_READABLE, this.chan_id);
            Poll::Pending
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// AsyncSend — await space on a channel
// ───────────────────────────────────────────────────────────────────

/// A [`Future`] that completes when a channel has space for a write.
///
/// On completion it sends the four-word message and returns `true` if
/// successful, `false` if the channel was closed.
pub struct AsyncSend {
    chan_id: usize,
    word0: usize,
    word1: usize,
    word2: usize,
    word3: usize,
}

impl AsyncSend {
    /// Create a future that sends `(word0, word1, word2, word3)` to `chan_id`
    /// once the channel has space.
    pub fn new(chan_id: usize, word0: usize, word1: usize, word2: usize, word3: usize) -> Self {
        Self { chan_id, word0, word1, word2, word3 }
    }
}

impl Future for AsyncSend {
    type Output = bool;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<bool> {
        let this = self.get_mut();

        kpoll::poll_set(kpoll::POLL_CHAN_WRITABLE, this.chan_id);
        let fired = kpoll::poll_wait(0);

        if fired & kpoll::POLL_CHAN_WRITABLE != 0 {
            let ok = crate::channel::send(this.chan_id, this.word0, this.word1, this.word2, this.word3);
            Poll::Ready(ok)
        } else {
            kpoll::poll_set(kpoll::POLL_CHAN_WRITABLE, this.chan_id);
            Poll::Pending
        }
    }
}

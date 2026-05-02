//! Bounded message channels for VeerOS.
//!
//! Replaces the single-slot mailbox IPC with proper ring-buffer channels
//! that support multiple messages in flight.
//!
//! # Design
//!
//! - Fixed pool of channels (no heap allocation).
//! - Each channel has a configurable-depth ring buffer (default 8 slots).
//! - `send()` copies a message into the ring buffer; blocks if full.
//! - `recv()` reads from the ring buffer; blocks if empty.
//! - Channels are created by `SYS_CHAN_CREATE` and identified by a small
//!   integer handle (`ChanId`).

use crate::task::{BlockReason, Scheduler, TaskState};

/// Maximum number of concurrently open channels.
pub const MAX_CHANNELS: usize = 8;

/// Depth of each channel's ring buffer (number of messages).
pub const CHAN_DEPTH: usize = 8;

/// A channel message — four machine words payload.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ChanMsg {
    pub word0: usize,
    pub word1: usize,
    pub word2: usize,
    pub word3: usize,
}

impl ChanMsg {
    pub const fn empty() -> Self {
        Self {
            word0: 0,
            word1: 0,
            word2: 0,
            word3: 0,
        }
    }
}

/// A single bounded channel with a ring buffer.
#[derive(Debug)]
pub struct Channel {
    /// Ring buffer storage.
    buf: [ChanMsg; CHAN_DEPTH],
    /// Read index (next slot to consume).
    head: usize,
    /// Write index (next slot to produce).
    tail: usize,
    /// Number of messages currently in the buffer.
    count: usize,
    /// Whether this channel slot is allocated.
    pub open: bool,
}

impl Channel {
    pub const fn empty() -> Self {
        Self {
            buf: [ChanMsg::empty(); CHAN_DEPTH],
            head: 0,
            tail: 0,
            count: 0,
            open: false,
        }
    }

    pub fn is_full(&self) -> bool {
        self.count >= CHAN_DEPTH
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Push a message. Returns `false` if the buffer is full.
    pub fn push(&mut self, msg: ChanMsg) -> bool {
        if self.is_full() {
            return false;
        }
        self.buf[self.tail] = msg;
        self.tail = (self.tail + 1) % CHAN_DEPTH;
        self.count += 1;
        true
    }

    /// Pop a message. Returns `None` if the buffer is empty.
    pub fn pop(&mut self) -> Option<ChanMsg> {
        if self.is_empty() {
            return None;
        }
        let msg = self.buf[self.head];
        self.head = (self.head + 1) % CHAN_DEPTH;
        self.count -= 1;
        Some(msg)
    }
}

/// Channel subsystem — manages the channel pool and per-channel wait lists.
pub struct Channels {
    pub chans: [Channel; MAX_CHANNELS],
}

impl Channels {
    pub const fn new() -> Self {
        Self {
            chans: [
                Channel::empty(),
                Channel::empty(),
                Channel::empty(),
                Channel::empty(),
                Channel::empty(),
                Channel::empty(),
                Channel::empty(),
                Channel::empty(),
            ],
        }
    }

    /// Check if a channel has data to read (non-mutating).
    pub fn has_data(&self, chan_id: usize) -> bool {
        chan_id < MAX_CHANNELS && self.chans[chan_id].open && !self.chans[chan_id].is_empty()
    }

    /// Check if a channel has space to write (non-mutating).
    pub fn has_space(&self, chan_id: usize) -> bool {
        chan_id < MAX_CHANNELS && self.chans[chan_id].open && !self.chans[chan_id].is_full()
    }

    /// Return the number of messages currently in the channel (0 if invalid/closed).
    pub fn message_count(&self, chan_id: usize) -> usize {
        if chan_id < MAX_CHANNELS && self.chans[chan_id].open {
            self.chans[chan_id].count
        } else {
            0
        }
    }

    /// Allocate a new channel. Returns the channel ID or `None`.
    pub fn create(&mut self) -> Option<usize> {
        for (i, ch) in self.chans.iter_mut().enumerate() {
            if !ch.open {
                *ch = Channel::empty();
                ch.open = true;
                return Some(i);
            }
        }
        None
    }

    /// Close a channel and wake all waiters.
    pub fn close(&mut self, sched: &mut Scheduler, chan_id: usize) -> bool {
        if chan_id >= MAX_CHANNELS || !self.chans[chan_id].open {
            return false;
        }
        self.chans[chan_id].open = false;
        // Wake any tasks blocked on this channel.
        for task in sched.tasks.iter_mut() {
            if task.state == TaskState::Blocked {
                match task.block_reason {
                    BlockReason::ChanSend(id) | BlockReason::ChanRecv(id) if id == chan_id => {
                        task.state = TaskState::Ready;
                        task.block_reason = BlockReason::None;
                    }
                    _ => {}
                }
            }
        }
        true
    }

    /// Send a message on a channel.
    ///
    /// Returns:
    /// - `Ok(true)` — message sent, wake a receiver if any was blocked
    /// - `Ok(false)` — channel full, caller should be blocked
    /// - `Err(())` — invalid channel or channel closed
    pub fn send(
        &mut self,
        sched: &mut Scheduler,
        chan_id: usize,
        msg: ChanMsg,
    ) -> Result<bool, ()> {
        if chan_id >= MAX_CHANNELS || !self.chans[chan_id].open {
            return Err(());
        }
        if self.chans[chan_id].push(msg) {
            // Wake one blocked receiver, if any.
            for task in sched.tasks.iter_mut() {
                if task.state == TaskState::Blocked
                    && task.block_reason == BlockReason::ChanRecv(chan_id)
                {
                    task.state = TaskState::Ready;
                    task.block_reason = BlockReason::None;
                    break; // wake just one
                }
            }
            Ok(true)
        } else {
            Ok(false) // full — caller should block
        }
    }

    /// Receive a message from a channel.
    ///
    /// Returns:
    /// - `Ok(Some(msg))` — message received, wake a sender if any was blocked
    /// - `Ok(None)` — channel empty, caller should be blocked
    /// - `Err(())` — invalid channel or channel closed
    pub fn recv(&mut self, sched: &mut Scheduler, chan_id: usize) -> Result<Option<ChanMsg>, ()> {
        if chan_id >= MAX_CHANNELS || !self.chans[chan_id].open {
            return Err(());
        }
        match self.chans[chan_id].pop() {
            Some(msg) => {
                // Wake one blocked sender, if any.
                for task in sched.tasks.iter_mut() {
                    if task.state == TaskState::Blocked
                        && task.block_reason == BlockReason::ChanSend(chan_id)
                    {
                        task.state = TaskState::Ready;
                        task.block_reason = BlockReason::None;
                        break; // wake just one
                    }
                }
                Ok(Some(msg))
            }
            None => Ok(None), // empty — caller should block
        }
    }
}

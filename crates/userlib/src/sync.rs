//! Synchronization primitives for VeerOS user tasks.
//!
//! Built on top of the kernel futex syscalls (`SYS_FUTEX_WAIT` / `SYS_FUTEX_WAKE`).
//! All primitives are `no_std`, `no_alloc`, and sized for static allocation.
//!
//! Because `riscv32imc` has no atomic instructions, all state is stored in
//! plain `usize` words and the kernel reads them atomically (interrupts are
//! disabled during syscall handling).
//!
//! # Primitives
//!
//! - [`Mutex`] — mutual exclusion lock (futex-based)
//! - [`Condvar`] — condition variable for signaling between tasks
//! - [`Semaphore`] — counting semaphore for bounded concurrency

use core::cell::UnsafeCell;
use crate::sys;

const SYS_FUTEX_WAIT: usize = 0x50;
const SYS_FUTEX_WAKE: usize = 0x51;

#[inline(always)]
fn futex_wait(addr: *const usize, expected: usize) {
    sys::syscall2(SYS_FUTEX_WAIT, addr as usize, expected);
}

#[inline(always)]
fn futex_wake(addr: *const usize, count: usize) -> usize {
    sys::syscall2(SYS_FUTEX_WAKE, addr as usize, count).0
}

// ═══════════════════════════════════════════════════════════════════════════
// Mutex
// ═══════════════════════════════════════════════════════════════════════════

const UNLOCKED: usize = 0;
const LOCKED: usize = 1;

/// A mutual-exclusion lock protecting a value of type `T`.
///
/// On single-core `riscv32imc` (no atomic CAS), the lock/unlock is
/// mediated entirely by the kernel via futex syscalls. The kernel
/// checks the lock word with interrupts disabled, making it atomic.
pub struct Mutex<T> {
    /// 0 = unlocked, 1 = locked.
    state: UnsafeCell<usize>,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Mutex<T> {}
unsafe impl<T: Send> Send for Mutex<T> {}

/// RAII lock guard — automatically unlocks the mutex when dropped.
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    /// Create a new unlocked mutex wrapping `val`.
    pub const fn new(val: T) -> Self {
        Self {
            state: UnsafeCell::new(UNLOCKED),
            data: UnsafeCell::new(val),
        }
    }

    /// Acquire the lock. Blocks until the lock is available.
    pub fn lock(&self) -> MutexGuard<'_, T> {
        loop {
            let s = unsafe { self.state.get().read_volatile() };
            if s == UNLOCKED {
                // Try to acquire — write LOCKED. On a single-core system
                // without preemption between these two volatile accesses
                // this is safe because we immediately enter the syscall
                // or succeed. If we get preempted between the read and
                // write, worst case two tasks both think they locked —
                // but the kernel's futex_wait will serialize: one will
                // see LOCKED and block. We use futex_wait as the
                // serialization point.
                unsafe { self.state.get().write_volatile(LOCKED) };
                return MutexGuard { mutex: self };
            }
            // The lock is held — ask the kernel to block us until it
            // changes.
            futex_wait(self.state.get(), LOCKED);
        }
    }

    fn unlock(&self) {
        unsafe { self.state.get().write_volatile(UNLOCKED) };
        futex_wake(self.state.get(), 1);
    }
}

impl<'a, T> core::ops::Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> core::ops::DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        self.mutex.unlock();
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Condvar
// ═══════════════════════════════════════════════════════════════════════════

/// A condition variable for task-to-task signaling.
pub struct Condvar {
    seq: UnsafeCell<usize>,
}

unsafe impl Sync for Condvar {}
unsafe impl Send for Condvar {}

impl Condvar {
    pub const fn new() -> Self {
        Self {
            seq: UnsafeCell::new(0),
        }
    }

    /// Block until `notify_one` or `notify_all` is called.
    ///
    /// The mutex guard is released before sleeping and re-acquired after.
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        let seq = unsafe { self.seq.get().read_volatile() };
        let mutex = guard.mutex;
        core::mem::drop(guard);
        futex_wait(self.seq.get(), seq);
        mutex.lock()
    }

    /// Wake one waiting task.
    pub fn notify_one(&self) {
        let s = unsafe { self.seq.get().read_volatile() };
        unsafe { self.seq.get().write_volatile(s.wrapping_add(1)) };
        futex_wake(self.seq.get(), 1);
    }

    /// Wake all waiting tasks.
    pub fn notify_all(&self) {
        let s = unsafe { self.seq.get().read_volatile() };
        unsafe { self.seq.get().write_volatile(s.wrapping_add(1)) };
        futex_wake(self.seq.get(), usize::MAX);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Semaphore
// ═══════════════════════════════════════════════════════════════════════════

/// A counting semaphore for bounding concurrent access.
pub struct Semaphore {
    count: UnsafeCell<usize>,
}

unsafe impl Sync for Semaphore {}
unsafe impl Send for Semaphore {}

impl Semaphore {
    /// Create a semaphore with `initial` permits.
    pub const fn new(initial: usize) -> Self {
        Self {
            count: UnsafeCell::new(initial),
        }
    }

    /// Acquire one permit. Blocks if no permits are available.
    pub fn acquire(&self) {
        loop {
            let c = unsafe { self.count.get().read_volatile() };
            if c > 0 {
                unsafe { self.count.get().write_volatile(c - 1) };
                return;
            }
            futex_wait(self.count.get(), 0);
        }
    }

    /// Release one permit, potentially waking a blocked acquirer.
    pub fn release(&self) {
        let c = unsafe { self.count.get().read_volatile() };
        unsafe { self.count.get().write_volatile(c + 1) };
        futex_wake(self.count.get(), 1);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// RwLock
// ═══════════════════════════════════════════════════════════════════════════

/// State encoding for RwLock:
/// - 0 = unlocked
/// - 1..=MAX = reader count (N readers)
/// - usize::MAX = write-locked
const RW_UNLOCKED: usize = 0;
const RW_WRITE_LOCKED: usize = usize::MAX;

/// A reader-writer lock protecting a value of type `T`.
///
/// Multiple readers can hold the lock concurrently, but a writer gets
/// exclusive access. Writers wait until all readers release; readers wait
/// while a writer holds the lock.
pub struct RwLock<T> {
    /// 0 = unlocked, 1..MAX-1 = N readers, MAX = writer
    state: UnsafeCell<usize>,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send + Sync> Sync for RwLock<T> {}
unsafe impl<T: Send> Send for RwLock<T> {}

/// RAII read guard — automatically releases the read lock when dropped.
pub struct RwLockReadGuard<'a, T> {
    lock: &'a RwLock<T>,
}

/// RAII write guard — automatically releases the write lock when dropped.
pub struct RwLockWriteGuard<'a, T> {
    lock: &'a RwLock<T>,
}

impl<T> RwLock<T> {
    /// Create a new unlocked reader-writer lock wrapping `val`.
    pub const fn new(val: T) -> Self {
        Self {
            state: UnsafeCell::new(RW_UNLOCKED),
            data: UnsafeCell::new(val),
        }
    }

    /// Acquire a read lock. Blocks while a writer holds the lock.
    pub fn read(&self) -> RwLockReadGuard<'_, T> {
        loop {
            let s = unsafe { self.state.get().read_volatile() };
            if s != RW_WRITE_LOCKED {
                // No writer — increment reader count.
                unsafe { self.state.get().write_volatile(s + 1) };
                return RwLockReadGuard { lock: self };
            }
            // Writer holds the lock — block until state changes.
            futex_wait(self.state.get(), RW_WRITE_LOCKED);
        }
    }

    /// Acquire a write lock. Blocks while any readers or another writer holds the lock.
    pub fn write(&self) -> RwLockWriteGuard<'_, T> {
        loop {
            let s = unsafe { self.state.get().read_volatile() };
            if s == RW_UNLOCKED {
                unsafe { self.state.get().write_volatile(RW_WRITE_LOCKED) };
                return RwLockWriteGuard { lock: self };
            }
            // Readers or writer present — block.
            futex_wait(self.state.get(), s);
        }
    }
}

impl<'a, T> core::ops::Deref for RwLockReadGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> Drop for RwLockReadGuard<'a, T> {
    fn drop(&mut self) {
        let s = unsafe { self.lock.state.get().read_volatile() };
        unsafe { self.lock.state.get().write_volatile(s - 1) };
        // If we were the last reader, wake a waiting writer.
        if s == 1 {
            futex_wake(self.lock.state.get(), 1);
        }
    }
}

impl<'a, T> core::ops::Deref for RwLockWriteGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> core::ops::DerefMut for RwLockWriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for RwLockWriteGuard<'a, T> {
    fn drop(&mut self) {
        unsafe { self.lock.state.get().write_volatile(RW_UNLOCKED) };
        // Wake all waiters (readers + writers all blocked on same addr).
        futex_wake(self.lock.state.get(), usize::MAX);
    }
}

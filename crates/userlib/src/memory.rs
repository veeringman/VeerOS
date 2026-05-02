//! Memory engine — read/write the kernel's persistent knowledge store
//! and query execution fabric status.
//!
//! # Persistent Memory
//!
//! The kernel maintains a global key-value store that survives across
//! agent lifecycles. Agents and applications can store learned facts,
//! preferences, and cached results.
//!
//! # Fabric
//!
//! The execution fabric tracks available compute nodes. Userspace can
//! query how many nodes are in the cluster and how many are healthy.
//!
//! # Example
//!
//! ```rust,no_run
//! use userlib::memory;
//!
//! // Store a fact
//! memory::store(b"deploy.last_host", b"node-3");
//!
//! // Query it later
//! let mut buf = [0u8; 64];
//! if let Some(len) = memory::query(b"deploy.last_host", &mut buf) {
//!     // buf[..len] contains "node-3"
//! }
//!
//! // Check fabric
//! let (total, healthy) = memory::fabric_status();
//! ```

use crate::sys;

const SYS_MEMORY_STORE: usize = 0xF8;
const SYS_MEMORY_QUERY: usize = 0xF9;
const SYS_FABRIC_STATUS: usize = 0xFA;

/// Store a key-value pair in the kernel's persistent memory.
///
/// - `key`: identifier (max 32 bytes).
/// - `value`: data to store (max 64 bytes).
///
/// Returns `true` on success.
#[inline]
pub fn store(key: &[u8], value: &[u8]) -> bool {
    sys::syscall4(
        SYS_MEMORY_STORE,
        key.as_ptr() as usize,
        key.len(),
        value.as_ptr() as usize,
        value.len(),
    ) == 0
}

/// Query a value from the kernel's persistent memory by key.
///
/// On success, writes the value into `buf` and returns the total
/// value length (may exceed `buf.len()`).
/// Returns `None` if the key is not found.
#[inline]
pub fn query(key: &[u8], buf: &mut [u8]) -> Option<usize> {
    let ret = sys::syscall4(
        SYS_MEMORY_QUERY,
        key.as_ptr() as usize,
        key.len(),
        buf.as_mut_ptr() as usize,
        buf.len(),
    );
    if ret == usize::MAX {
        None
    } else {
        Some(ret)
    }
}

/// Query execution fabric status.
///
/// Returns `(total_nodes, healthy_nodes)`.
#[inline]
pub fn fabric_status() -> (usize, usize) {
    sys::syscall2(SYS_FABRIC_STATUS, 0, 0)
}

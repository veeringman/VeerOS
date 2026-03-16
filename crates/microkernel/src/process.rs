//! Process model — groups threads that share an address space.
//!
//! A **Process** owns memory regions, capability tokens (future), and
//! resource quotas.  One or more **Threads** (TCBs in [`crate::task`])
//! execute within a process, sharing its memory.
//!
//! On targets without an MMU (riscv32imc), the "address space" is simply
//! the set of PMP regions granted to the process.  On MMU targets the
//! process will additionally own a page table root pointer / ASID.

use arch::{TaskMemRegion, MemPerms};
use crate::user::{UserId, GroupId, ROOT_UID, ROOT_GID};
use crate::vfs::{FileDescriptor, MAX_FDS, ROOT_INODE};

/// Maximum number of simultaneous processes.
pub const MAX_PROCESSES: usize = 8;

/// Maximum memory regions held at the process level.
///
/// Threads add their own stack regions on top of these when programming
/// PMP on context switch.
pub const MAX_PROCESS_REGIONS: usize = 4;

// ─── Process state ──────────────────────────────────────────────────────

/// Lifecycle state of a process slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Slot is unused.
    Free,
    /// Process is active (has at least one living thread).
    Active,
    /// All threads have exited; exit code available for collection.
    Zombie,
}

/// Process Control Block.
#[derive(Debug, Clone, Copy)]
pub struct Process {
    pub state: ProcessState,
    /// Human-readable name (for debug / `ps`).
    pub name: &'static str,
    /// Parent process index (`usize::MAX` for the root/init process).
    pub parent_pid: usize,
    /// Exit code of the last exiting thread.
    pub exit_code: usize,
    /// Number of living threads in this process.
    pub thread_count: usize,
    /// Cumulative heap bytes allocated by this process.
    pub heap_usage: usize,
    /// Number of open handles (channels, IPC ports, file descriptors).
    pub open_handles: usize,
    /// Process-level memory regions (code, data, heap).
    /// Thread-level stack regions are kept in the TCB.
    pub regions: [TaskMemRegion; MAX_PROCESS_REGIONS],
    /// Number of valid entries in `regions`.
    pub region_count: usize,
    /// Owner user ID.
    pub uid: UserId,
    /// Owner group ID.
    pub gid: GroupId,
    /// Per-process file descriptor table.
    pub fds: [Option<FileDescriptor>; MAX_FDS],
    /// Current working directory (inode ID).
    pub cwd: u16,
}

impl Process {
    pub const fn empty() -> Self {
        Self {
            state: ProcessState::Free,
            name: "",
            parent_pid: usize::MAX,
            exit_code: 0,
            thread_count: 0,
            heap_usage: 0,
            open_handles: 0,
            regions: [TaskMemRegion::empty(); MAX_PROCESS_REGIONS],
            region_count: 0,
            uid: ROOT_UID,
            gid: ROOT_GID,
            fds: [None; MAX_FDS],
            cwd: ROOT_INODE,
        }
    }
}

// ─── Process table ──────────────────────────────────────────────────────

/// Fixed-size table of all processes.
pub struct ProcessTable {
    pub processes: [Process; MAX_PROCESSES],
}

impl ProcessTable {
    pub const fn new() -> Self {
        Self {
            processes: [Process::empty(); MAX_PROCESSES],
        }
    }

    /// Create a new process.  Returns the process index (pid), or `None`
    /// if the table is full.
    pub fn create(
        &mut self,
        name: &'static str,
        parent_pid: usize,
        uid: UserId,
        gid: GroupId,
    ) -> Option<usize> {
        for (i, slot) in self.processes.iter_mut().enumerate() {
            if slot.state == ProcessState::Free {
                *slot = Process {
                    state: ProcessState::Active,
                    name,
                    parent_pid,
                    exit_code: 0,
                    thread_count: 0,
                    heap_usage: 0,
                    open_handles: 0,
                    regions: [TaskMemRegion::empty(); MAX_PROCESS_REGIONS],
                    region_count: 0,
                    uid,
                    gid,
                    fds: [None; MAX_FDS],
                    cwd: ROOT_INODE,
                };
                return Some(i);
            }
        }
        None
    }

    /// Grant a memory region to a process.
    /// Returns `true` on success, `false` if the region table is full.
    pub fn grant_region(&mut self, pid: usize, region: TaskMemRegion) -> bool {
        if pid >= MAX_PROCESSES {
            return false;
        }
        let p = &mut self.processes[pid];
        if p.state == ProcessState::Free || p.region_count >= MAX_PROCESS_REGIONS {
            return false;
        }
        p.regions[p.region_count] = region;
        p.region_count += 1;
        true
    }

    /// Check whether a pointer range is allowed by a process's regions.
    ///
    /// Returns `true` if the process has no regions configured (legacy
    /// mode) or if the range falls within a region with the required
    /// permissions.
    pub fn validate_ptr(
        &self,
        pid: usize,
        addr: usize,
        len: usize,
        required: MemPerms,
    ) -> bool {
        if pid >= MAX_PROCESSES {
            return false;
        }
        let p = &self.processes[pid];
        // Legacy: no regions → allow all.
        if p.region_count == 0 {
            return true;
        }
        if len == 0 {
            return true;
        }
        for i in 0..p.region_count {
            if p.regions[i].allows(addr, len, required) {
                return true;
            }
        }
        false
    }

    /// Notify the process table that a thread within `pid` has exited.
    ///
    /// Decrements the thread count; if it reaches zero, transitions the
    /// process to `Zombie` with the given exit code.
    pub fn thread_exited(&mut self, pid: usize, exit_code: usize) {
        if pid >= MAX_PROCESSES {
            return;
        }
        let p = &mut self.processes[pid];
        if p.state != ProcessState::Active {
            return;
        }
        p.thread_count = p.thread_count.saturating_sub(1);
        p.exit_code = exit_code;
        if p.thread_count == 0 {
            p.state = ProcessState::Zombie;
        }
    }

    /// Reclaim a zombie process slot (called after exit code is collected).
    pub fn reap(&mut self, pid: usize) {
        if pid < MAX_PROCESSES && self.processes[pid].state == ProcessState::Zombie {
            self.processes[pid] = Process::empty();
        }
    }
}

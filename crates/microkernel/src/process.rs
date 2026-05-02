//! Process model — groups threads that share an address space.
//!
//! A **Process** owns memory regions, capability tokens, and resource
//! quotas.  One or more **Threads** (TCBs in [`crate::task`]) execute
//! within a process, sharing its memory.
//!
//! On targets without an MMU (riscv32imc), the "address space" is simply
//! the set of PMP regions granted to the process.  On MMU targets the
//! process will additionally own a page table root pointer / ASID.
//!
//! # Capability Model
//!
//! Each process holds a [`ProcessCaps`] bitfield controlling which syscall
//! groups it may invoke. Capabilities follow these rules:
//!
//! - **Root (pid 0)** starts with all capabilities.
//! - **Child processes** inherit the parent's caps (never gain more).
//! - **Capabilities can be dropped** (voluntarily, before exec), never added.
//! - The dispatcher checks `process.caps` before executing privileged syscalls.

use crate::user::{GroupId, UserId, ROOT_GID, ROOT_UID};
use crate::vfs::{FileDescriptor, MAX_FDS, ROOT_INODE};
use arch::{MemPerms, TaskMemRegion};
use bitflags::bitflags;

/// Maximum number of simultaneous processes.
pub const MAX_PROCESSES: usize = 8;

/// Maximum memory regions held at the process level.
///
/// Threads add their own stack regions on top of these when programming
/// PMP on context switch.
pub const MAX_PROCESS_REGIONS: usize = 4;

// ─── Per-process capabilities ───────────────────────────────────────────

bitflags! {
    /// Per-process capability bits controlling which syscall groups are allowed.
    ///
    /// Caps are enforced by the dispatcher before executing any privileged
    /// syscall. A process may voluntarily drop caps (e.g. a sandbox) but
    /// can never gain caps it wasn't born with.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ProcessCaps: u32 {
        // ── Basic (granted to all processes) ──────────────────────
        /// Task control: yield, exit, task_id, tls_get/set.
        const TASK_BASIC      = 1 << 0;
        /// Memory: alloc, free, region info.
        const MEM             = 1 << 1;
        /// Time: tick, sleep.
        const TIME            = 1 << 2;
        /// Synchronization: futex_wait, futex_wake.
        const SYNC            = 1 << 3;

        // ── IPC / Channels ────────────────────────────────────────
        /// IPC: send, recv, poll.
        const IPC             = 1 << 4;
        /// Channels: create, send, recv, close, poll.
        const CHANNEL         = 1 << 5;
        /// Poll: poll_set, poll_wait.
        const POLL            = 1 << 6;

        // ── I/O ───────────────────────────────────────────────────
        /// Console I/O: write_byte, write_buf, read_byte.
        const CONSOLE_IO      = 1 << 7;
        /// Filesystem: open, close, read, write, seek, stat, readdir, etc.
        const FS              = 1 << 8;

        // ── Networking ────────────────────────────────────────────
        /// Sockets: socket, bind, listen, accept, connect, send, recv, close.
        const NET             = 1 << 9;

        // ── Process management ────────────────────────────────────
        /// Spawn new threads within own process.
        const SPAWN_THREAD    = 1 << 10;
        /// Spawn new processes (fork-like).
        const SPAWN_PROCESS   = 1 << 11;

        // ── Privileged / Admin ────────────────────────────────────
        /// User management: setuid, login, logout.
        const USER_ADMIN      = 1 << 12;
        /// Driver MMIO/IRQ access: mmio_read, mmio_write, irq_wait, irq_ack.
        const DRIVER          = 1 << 13;
        /// Mount / unmount filesystems.
        const MOUNT           = 1 << 14;
        /// Hardware GPIO / I2C / SPI / sensors.
        const HW              = 1 << 15;

        // ── Future security extensions ────────────────────────────
        /// Crypto syscalls (future: 0xE0–0xEF).
        const CRYPTO          = 1 << 16;
        /// Capability management syscalls (future: 0xD0–0xDF).
        /// Required to drop caps on child, query own caps, etc.
        const CAP_ADMIN       = 1 << 17;

        /// Unified accelerator interface: coprocessor / FPGA / quantum jobs.
        /// Only available when the `accel` feature is enabled (desktop/server).
        #[cfg(feature = "accel")]
        const ACCEL           = 1 << 18;

        // ── AI-Native Execution ───────────────────────────────────
        /// Agent lifecycle: spawn, complete, fail, context read/write.
        const AGENT           = 1 << 19;
        /// Intent submission and management.
        const INTENT          = 1 << 20;
        /// Memory engine: persistent/episodic read/write.
        const MEMORY_ENGINE   = 1 << 21;
        /// Fabric: query node topology and placement.
        const FABRIC          = 1 << 22;

        // ── Distributed Fabric ────────────────────────────────────
        /// Fabric admin: peer registration, trust management, session setup.
        const FABRIC_ADMIN    = 1 << 23;
        /// Agent/intent migration across nodes.
        const FABRIC_MIGRATE  = 1 << 24;
        /// ZKP capability proofs: prove and verify.
        const ZKP             = 1 << 25;
    }
}

impl ProcessCaps {
    /// All capabilities — granted to the root/init process.
    pub const fn all_caps() -> Self {
        Self::all()
    }

    /// Default capability set for unprivileged user processes.
    ///
    /// Includes: task basics, memory, time, sync, IPC, channels, poll,
    /// console I/O, filesystem, networking, thread spawning.
    /// Excludes: spawn_process, user_admin, driver, mount, hw, crypto, cap_admin.
    pub const fn user_default() -> Self {
        Self::TASK_BASIC
            .union(Self::MEM)
            .union(Self::TIME)
            .union(Self::SYNC)
            .union(Self::IPC)
            .union(Self::CHANNEL)
            .union(Self::POLL)
            .union(Self::CONSOLE_IO)
            .union(Self::FS)
            .union(Self::NET)
            .union(Self::SPAWN_THREAD)
    }

    /// Minimal sandbox: only compute + basic IPC, no I/O at all.
    pub const fn sandbox() -> Self {
        Self::TASK_BASIC
            .union(Self::MEM)
            .union(Self::TIME)
            .union(Self::SYNC)
            .union(Self::IPC)
            .union(Self::CHANNEL)
    }
}

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
    /// Per-process capability bits (controls which syscall groups are allowed).
    pub caps: ProcessCaps,
    /// Per-process file descriptor table.
    pub fds: [Option<FileDescriptor>; MAX_FDS],
    /// Current working directory (inode ID).
    pub cwd: u16,
    /// Physical address of PML4 page table root (0 = shares kernel address space).
    pub cr3: usize,
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
            caps: ProcessCaps::all(),
            fds: [None; MAX_FDS],
            cwd: ROOT_INODE,
            cr3: 0,
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
        // Child inherits parent's caps (can never exceed them).
        let inherited_caps = if parent_pid < MAX_PROCESSES {
            self.processes[parent_pid].caps
        } else {
            // Root / init process — full capabilities.
            ProcessCaps::all()
        };

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
                    caps: inherited_caps,
                    fds: [None; MAX_FDS],
                    cwd: ROOT_INODE,
                    cr3: 0,
                };
                return Some(i);
            }
        }
        None
    }

    /// Drop capabilities from a process. Caps can only be removed, never added.
    /// Returns `true` if successful.
    pub fn drop_caps(&mut self, pid: usize, to_drop: ProcessCaps) -> bool {
        if pid >= MAX_PROCESSES {
            return false;
        }
        let p = &mut self.processes[pid];
        if p.state == ProcessState::Free {
            return false;
        }
        p.caps.remove(to_drop);
        true
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
    pub fn validate_ptr(&self, pid: usize, addr: usize, len: usize, required: MemPerms) -> bool {
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

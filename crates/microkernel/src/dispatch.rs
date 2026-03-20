//! Kernel-side syscall dispatcher.
//!
//! Called from the trap handler when `mcause` indicates an environment call
//! (ecall). Reads the syscall number from `a7` (x17) in the saved context,
//! dispatches to the appropriate handler, and writes return values back
//! into `a0`/`a1` of the saved context.
//!
//! The dispatcher receives mutable references to all kernel subsystems so
//! it can service any syscall without global state lookups.

use arch::{SavedContext, TaskContext, MemPerms, validate_user_ptr};
use crate::channel::{Channels, ChanMsg};
use crate::driver::DriverRegistry;
use crate::fat32::Fat32;
use crate::futex::FutexTable;
use crate::input::InputSubsystem;
use crate::ipc::{Ipc, Message};
use crate::poll::PollTable;
use crate::process::ProcessTable;
use crate::ramfs::RamFs;
use crate::socket::SocketTable;
use crate::task::{BlockReason, Scheduler, TaskState};
use crate::user::UserTable;
use crate::vfs::{InodeTable, InodeKind, FileDescriptor, OpenFlags, DirEntry, StatBuf,
                 MountTable, FsType,
                 MAX_FDS, NO_INODE, ROOT_INODE, SEEK_SET, SEEK_CUR, SEEK_END};
use crate::alloc::Heap;
use crate::syscall::*;

/// Result of a syscall dispatch — tells the trap handler what to do next.
pub enum SyscallAction {
    /// Return to the same task (normal case).
    Resume,
    /// The task yielded or was blocked — a context switch is needed.
    Reschedule,
    /// The task exited — pick a new task.
    TaskExited,
}

/// Check that a user-provided pointer range is valid for the current task.
///
/// Checks both the process-level regions and thread-level regions.
/// Returns `true` if:
/// - neither the process nor the thread has regions configured (legacy), or
/// - the pointer range falls within a region with the required permissions.
fn check_user_ptr(
    sched: &Scheduler,
    procs: &ProcessTable,
    ptr: usize,
    len: usize,
    required: MemPerms,
) -> bool {
    let cur = sched.current;
    if cur >= sched.tasks.len() {
        return false;
    }
    let tcb = &sched.tasks[cur];
    let pid = tcb.process_id;

    // Check thread-level regions (stack + guard).
    if tcb.region_count > 0 && validate_user_ptr(&tcb.regions, ptr, len, required) {
        return true;
    }
    // Check process-level regions.
    if pid < crate::process::MAX_PROCESSES {
        if procs.validate_ptr(pid, ptr, len, required) {
            return true;
        }
    }
    // Legacy mode: no regions at all → allow.
    if tcb.region_count == 0
        && (pid >= crate::process::MAX_PROCESSES
            || procs.processes[pid].region_count == 0)
    {
        return true;
    }
    false
}

/// Dispatch a syscall from the saved trap context.
///
/// # Arguments
/// - `ctx` — pointer to the saved `TaskContext` on the trap stack
/// - `sched` — the kernel scheduler
/// - `ipc` — the IPC mailbox array
/// - `heap` — the kernel heap (for SYS_ALLOC / SYS_FREE)
/// - `futex` — the futex wait-queue table
/// - `channels` — the bounded channel pool
/// - `poll` — the per-task event poll table
/// - `processes` — the process table
/// - `sockets` — the socket table
/// - `console_write` — callback to write a byte to the console
/// - `console_read` — callback to read a byte from the console
///
/// # Safety
/// `ctx` must be a valid pointer to a `TaskContext`. The caller must hold
/// exclusive access to `sched`, `ipc`, `heap`, `futex`, `channels`, `poll`,
/// `processes`, and `sockets`.
pub unsafe fn dispatch(
    ctx: *mut TaskContext,
    sched: &mut Scheduler,
    ipc: &mut Ipc,
    heap: &mut Heap,
    futex: &mut FutexTable,
    channels: &mut Channels,
    poll: &mut PollTable,
    processes: &mut ProcessTable,
    sockets: &mut SocketTable,
    users: &mut UserTable,
    inodes: &mut InodeTable,
    ramfs: &mut RamFs,
    fat32: &mut Fat32,
    mounts: &mut MountTable,
    input: &mut InputSubsystem,
    drivers: &mut DriverRegistry,
    console_write: fn(u8),
    console_read: fn() -> u8,
) -> SyscallAction {
    let c = unsafe { &mut *ctx };

    // Skip the trap instruction so we return to the next one.
    c.advance_pc();

    // Read syscall arguments via architecture-neutral accessors.
    let nr = c.get_syscall_nr();
    let a0 = c.get_arg(0);
    let a1 = c.get_arg(1);
    let a2 = c.get_arg(2);
    let a3 = c.get_arg(3);
    let a4 = c.get_arg(4);

    match nr {
        // ── Task control ────────────────────────────────────────
        SYS_YIELD => {
            // Mark current task Ready so the scheduler picks something else.
            let cur = sched.current;
            if cur < sched.tasks.len() {
                use crate::task::TaskState;
                if sched.tasks[cur].state == TaskState::Running {
                    sched.tasks[cur].state = TaskState::Ready;
                }
            }
            SyscallAction::Reschedule
        }

        SYS_EXIT => {
            let cur = sched.current;
            // Diagnostic: confirm SYS_EXIT is reached.
            console_write(b'[');
            console_write(b'X');
            console_write(b'I');
            console_write(b'T');
            console_write(b':');
            console_write(b'0' + (cur as u8 % 10));
            console_write(b']');
            console_write(b'\n');
            if cur < sched.tasks.len() {
                use crate::task::TaskState;
                let pid = sched.tasks[cur].process_id;
                sched.tasks[cur].exit_code = a0;
                sched.tasks[cur].state = TaskState::Free;
                sched.tasks[cur].block_reason = BlockReason::None;
                sched.tasks[cur].wakeup_tick = 0;
                sched.tasks[cur].join_target = usize::MAX;
                // Zero the PC so any accidental resumption is detectable.
                sched.tasks[cur].context.set_pc(0);
                // Notify the process table that a thread exited.
                processes.thread_exited(pid, a0);
                // Wake any task that was joining on us.
                for i in 0..sched.tasks.len() {
                    if sched.tasks[i].state == TaskState::Blocked
                        && sched.tasks[i].block_reason == BlockReason::Join
                        && sched.tasks[i].join_target == cur
                    {
                        sched.tasks[i].state = TaskState::Ready;
                        sched.tasks[i].block_reason = BlockReason::None;
                        sched.tasks[i].join_target = usize::MAX;
                        // Write the exit code into the joiner's return register.
                        sched.tasks[i].context.set_ret(0, a0);
                    }
                }
            }
            SyscallAction::TaskExited
        }

        SYS_TASK_ID => {
            c.set_ret(0, sched.current);
            SyscallAction::Resume
        }

        SYS_TASK_PRIORITY => {
            let cur = sched.current;
            c.set_ret(0, if cur < sched.tasks.len() {
                sched.tasks[cur].priority as usize
            } else {
                0
            });
            SyscallAction::Resume
        }

        SYS_TASK_COUNT => {
            use crate::task::TaskState;
            let count = sched.tasks.iter()
                .filter(|t| t.state != TaskState::Free)
                .count();
            c.set_ret(0, count);
            SyscallAction::Resume
        }

        SYS_SPAWN => {
            let entry = a0;
            let stack_top = a1;
            let stack_bottom = a2;
            let priority = a3 as u8;
            let parent = sched.current;
            let parent_pid = if parent < sched.tasks.len() {
                sched.tasks[parent].process_id
            } else {
                0
            };
            match sched.create_task("spawned", entry, stack_top, stack_bottom, priority, parent_pid) {
                Some(child_id) => {
                    // Inherit caller execution status (M/U mode and interrupt bits).
                    // Required for kernel-spawned helper threads (e.g., WiFi blob tasks)
                    // that execute from kernel-managed IROM without per-process mappings.
                    let parent_status = c.get_status();
                    sched.tasks[child_id].context.set_status(parent_status);
                    sched.tasks[child_id].parent = parent;
                    // Bump parent process's thread count.
                    if parent_pid < crate::process::MAX_PROCESSES {
                        processes.processes[parent_pid].thread_count += 1;
                    }
                    c.set_ret(0, child_id);
                }
                None => {
                    c.set_ret(0, usize::MAX);
                }
            }
            SyscallAction::Resume
        }

        SYS_JOIN => {
            use crate::task::TaskState;
            let target = a0;
            if target >= sched.tasks.len() {
                // Invalid task ID — return immediately with error.
                c.set_ret(0, usize::MAX);
                SyscallAction::Resume
            } else if sched.tasks[target].state == TaskState::Free {
                // Target already exited — return its saved exit code.
                c.set_ret(0, sched.tasks[target].exit_code);
                SyscallAction::Resume
            } else {
                // Target still running — block caller until it exits.
                let cur = sched.current;
                if cur < sched.tasks.len() {
                    sched.tasks[cur].state = TaskState::Blocked;
                    sched.tasks[cur].block_reason = BlockReason::Join;
                    sched.tasks[cur].join_target = target;
                }
                SyscallAction::Reschedule
            }
        }

        SYS_SPAWN_PROCESS => {
            let entry = a0;
            let stack_top = a1;
            let stack_bottom = a2;
            let priority = a3 as u8;
            let caller = sched.current;
            let parent_pid = if caller < sched.tasks.len() {
                sched.tasks[caller].process_id
            } else {
                usize::MAX
            };
            // Inherit UID/GID from parent process.
            let (uid, gid) = if parent_pid < crate::process::MAX_PROCESSES {
                let pp = &processes.processes[parent_pid];
                (pp.uid, pp.gid)
            } else {
                (crate::user::ROOT_UID, crate::user::ROOT_GID)
            };
            // Create a new process.
            match processes.create("process", parent_pid, uid, gid) {
                Some(new_pid) => {
                    // Create the initial thread in the new process.
                    match sched.create_task("main", entry, stack_top, stack_bottom, priority, new_pid) {
                        Some(tid) => {
                            sched.tasks[tid].parent = caller;
                            processes.processes[new_pid].thread_count = 1;
                            c.set_ret(0, new_pid);
                        }
                        None => {
                            // Thread table full — tear down the process.
                            processes.reap(new_pid);
                            c.set_ret(0, usize::MAX);
                        }
                    }
                }
                None => {
                    c.set_ret(0, usize::MAX);
                }
            }
            SyscallAction::Resume
        }

        SYS_PROCESS_ID => {
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() {
                sched.tasks[cur].process_id
            } else {
                usize::MAX
            };
            c.set_ret(0, pid);
            SyscallAction::Resume
        }

        SYS_THREAD_COUNT => {
            let pid = a0;
            let count = if pid < crate::process::MAX_PROCESSES {
                processes.processes[pid].thread_count
            } else {
                0
            };
            c.set_ret(0, count);
            SyscallAction::Resume
        }

        SYS_TLS_GET => {
            let cur = sched.current;
            let base = if cur < sched.tasks.len() {
                sched.tasks[cur].tls_base
            } else {
                0
            };
            c.set_ret(0, base);
            SyscallAction::Resume
        }

        SYS_TLS_SET => {
            let cur = sched.current;
            if cur < sched.tasks.len() {
                sched.tasks[cur].tls_base = a0;
            }
            c.set_ret(0, 0);
            SyscallAction::Resume
        }

        // ── IPC ─────────────────────────────────────────────────
        SYS_IPC_SEND => {
            let dest = a0 as u8;
            let opcode = a1 as u8;
            let msg = Message {
                sender: 0, // filled by ipc.send()
                opcode,
                arg0: a2,
                arg1: a3,
            };
            let from = sched.current as u8;
            let ok = ipc.send(sched, from, dest, msg);
            c.set_ret(0, ok as usize);
            SyscallAction::Resume
        }

        SYS_IPC_RECV => {
            let tid = sched.current as u8;
            match ipc.recv(sched, tid) {
                Some(msg) => {
                    // Pack sender + opcode into ret0, arg0 into ret1.
                    c.set_ret(0, (msg.sender as usize) | ((msg.opcode as usize) << 8));
                    c.set_ret(1, msg.arg0);
                    SyscallAction::Resume
                }
                None => {
                    // Task was blocked by ipc.recv() — need to reschedule.
                    // When the task is unblocked by a send(), the scheduler
                    // will resume it. We need to re-execute the ecall so
                    // the recv sees the message, so rewind pc.
                    let pc = c.get_pc();
                    c.set_pc(pc - TaskContext::INSTRUCTION_SIZE);
                    let cur = sched.current;
                    if cur < sched.tasks.len() {
                        sched.tasks[cur].state = TaskState::Blocked;
                        sched.tasks[cur].block_reason = BlockReason::IpcRecv;
                    }
                    SyscallAction::Reschedule
                }
            }
        }

        SYS_IPC_POLL => {
            let tid = sched.current as u8;
            c.set_ret(0, ipc.has_message(tid) as usize);
            SyscallAction::Resume
        }

        // ── I/O / Console ───────────────────────────────────────
        SYS_WRITE_BYTE => {
            console_write(a0 as u8);
            SyscallAction::Resume
        }

        SYS_WRITE_BUF => {
            let ptr = a0 as *const u8;
            let len = a1;
            if !check_user_ptr(sched, processes, a0, len, MemPerms::READ) {
                // Invalid pointer — return error, don't access memory.
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            // Safety: validated that the pointer is within the task's
            // granted read regions (or legacy mode with no regions).
            for i in 0..len {
                let byte = unsafe { ptr.add(i).read() };
                console_write(byte);
            }
            SyscallAction::Resume
        }

        SYS_READ_BYTE => {
            let byte = console_read();
            c.set_ret(0, byte as usize);
            SyscallAction::Resume
        }

        // ── Time ────────────────────────────────────────────────
        SYS_TICK => {
            let ticks = sched.ticks;
            c.set_ret(0, ticks as usize);           // low word
            c.set_ret(1, (ticks >> 32) as usize);   // high word (32-bit targets)
            SyscallAction::Resume
        }

        SYS_SLEEP => {
            let cur = sched.current;
            if cur < sched.tasks.len() {
                use crate::task::TaskState;
                sched.tasks[cur].state = TaskState::Blocked;
                sched.tasks[cur].block_reason = BlockReason::Sleep;
                sched.tasks[cur].wakeup_tick = sched.ticks.saturating_add(a0 as u64);
            }
            SyscallAction::Reschedule
        }

        // ── Memory ──────────────────────────────────────────────
        SYS_ALLOC => {
            let size = a0;
            match heap.alloc(size) {
                Ok(ptr) => c.set_ret(0, ptr as usize),
                Err(_) => c.set_ret(0, 0),
            }
            SyscallAction::Resume
        }

        SYS_FREE => {
            let ptr = a0 as *mut u8;
            let size = a1;
            if !ptr.is_null() {
                unsafe { heap.free(ptr, size) };
            }
            SyscallAction::Resume
        }

        SYS_MEM_REGION_COUNT => {
            let cur = sched.current;
            let mut count = 0usize;
            if cur < sched.tasks.len() {
                count += sched.tasks[cur].region_count;
                let pid = sched.tasks[cur].process_id;
                if pid < crate::process::MAX_PROCESSES {
                    count += processes.processes[pid].region_count;
                }
            }
            c.set_ret(0, count);
            SyscallAction::Resume
        }

        SYS_MEM_REGION_INFO => {
            let idx = a0;
            let cur = sched.current;
            if cur < sched.tasks.len() {
                let tcb = &sched.tasks[cur];
                let pid = tcb.process_id;
                let thread_n = tcb.region_count;
                let proc_n = if pid < crate::process::MAX_PROCESSES {
                    processes.processes[pid].region_count
                } else {
                    0
                };
                // Thread regions first, then process regions.
                if idx < thread_n {
                    let r = &tcb.regions[idx];
                    c.set_ret(0, r.base);
                    c.set_ret(1, r.size);
                } else if idx < thread_n + proc_n {
                    let r = &processes.processes[pid].regions[idx - thread_n];
                    c.set_ret(0, r.base);
                    c.set_ret(1, r.size);
                } else {
                    c.set_ret(0, 0);
                    c.set_ret(1, 0);
                }
            } else {
                c.set_ret(0, 0);
                c.set_ret(1, 0);
            }
            SyscallAction::Resume
        }

        // ── Debug ───────────────────────────────────────────────
        SYS_PANIC => {
            let ptr = a0 as *const u8;
            let len = a1;
            // Print the panic message to console.
            for b in b"PANIC(user): " {
                console_write(*b);
            }
            if check_user_ptr(sched, processes, a0, len, MemPerms::READ) {
                for i in 0..len {
                    let byte = unsafe { ptr.add(i).read() };
                    console_write(byte);
                }
            } else {
                for b in b"<invalid pointer>" {
                    console_write(*b);
                }
            }
            console_write(b'\r');
            console_write(b'\n');
            // Kill the task.
            let cur = sched.current;
            if cur < sched.tasks.len() {
                use crate::task::TaskState;
                sched.tasks[cur].state = TaskState::Free;
            }
            SyscallAction::TaskExited
        }

        SYS_PLATFORM_NAME => {
            // Returns a pointer + length. The string lives in .rodata
            // so it's safe for the user to read.
            c.set_ret(0, 0); // not easily available without platform ref
            c.set_ret(1, 0);
            SyscallAction::Resume
        }

        // ── Synchronization ─────────────────────────────────────
        SYS_FUTEX_WAIT => {
            let addr = a0;
            let expected = a1;
            let tid = sched.current;
            // Validate futex address (4-byte read).
            if !check_user_ptr(sched, processes, addr, core::mem::size_of::<usize>(), MemPerms::READ) {
                c.set_ret(0, usize::MAX); // invalid address
                return SyscallAction::Resume;
            }
            let blocked = unsafe { futex.wait(sched, tid, addr, expected) };
            if blocked {
                c.set_ret(0, 0); // will be seen on wakeup
                SyscallAction::Reschedule
            } else {
                c.set_ret(0, 1); // value didn't match — no block
                SyscallAction::Resume
            }
        }

        SYS_FUTEX_WAKE => {
            let addr = a0;
            let count = a1;
            let woken = futex.wake(sched, addr, count);
            c.set_ret(0, woken);
            SyscallAction::Resume
        }

        // ── Channels ────────────────────────────────────────────
        SYS_CHAN_CREATE => {
            match channels.create() {
                Some(id) => c.set_ret(0, id),
                None => c.set_ret(0, usize::MAX),
            }
            SyscallAction::Resume
        }

        SYS_CHAN_SEND => {
            let chan_id = a0;
            let msg = ChanMsg { word0: a1, word1: a2, word2: a3, word3: a4 };
            match channels.send(sched, chan_id, msg) {
                Ok(true) => {
                    c.set_ret(0, 1); // success
                    SyscallAction::Resume
                }
                Ok(false) => {
                    // Channel full — block sender.
                    let cur = sched.current;
                    if cur < sched.tasks.len() {
                        use crate::task::TaskState;
                        sched.tasks[cur].state = TaskState::Blocked;
                        sched.tasks[cur].block_reason = BlockReason::ChanSend(chan_id);
                    }
                    // Rewind PC so the send is retried on wakeup.
                    let pc = c.get_pc();
                    c.set_pc(pc - TaskContext::INSTRUCTION_SIZE);
                    SyscallAction::Reschedule
                }
                Err(()) => {
                    c.set_ret(0, 0); // closed / invalid
                    SyscallAction::Resume
                }
            }
        }

        SYS_CHAN_RECV => {
            let chan_id = a0;
            match channels.recv(sched, chan_id) {
                Ok(Some(msg)) => {
                    c.set_ret(0, msg.word0);
                    c.set_ret(1, msg.word1);
                    c.set_ret(2, msg.word2);
                    c.set_ret(3, msg.word3);
                    SyscallAction::Resume
                }
                Ok(None) => {
                    // Channel empty — block receiver.
                    let cur = sched.current;
                    if cur < sched.tasks.len() {
                        use crate::task::TaskState;
                        sched.tasks[cur].state = TaskState::Blocked;
                        sched.tasks[cur].block_reason = BlockReason::ChanRecv(chan_id);
                    }
                    // Rewind PC so the recv is retried on wakeup.
                    let pc = c.get_pc();
                    c.set_pc(pc - TaskContext::INSTRUCTION_SIZE);
                    SyscallAction::Reschedule
                }
                Err(()) => {
                    c.set_ret(0, usize::MAX); // closed / invalid
                    c.set_ret(1, 0);
                    c.set_ret(2, 0);
                    c.set_ret(3, 0);
                    SyscallAction::Resume
                }
            }
        }

        SYS_CHAN_CLOSE => {
            let chan_id = a0;
            let ok = channels.close(sched, chan_id);
            c.set_ret(0, ok as usize);
            SyscallAction::Resume
        }

        SYS_CHAN_POLL => {
            let chan_id = a0;
            let count = channels.message_count(chan_id);
            c.set_ret(0, count);
            SyscallAction::Resume
        }

        // ── Poll / Async ────────────────────────────────────────
        SYS_POLL_SET => {
            let mask = a0;
            let param = a1;
            let tid = sched.current;
            poll.set(tid, mask, param, sched.ticks);
            c.set_ret(0, 0);
            SyscallAction::Resume
        }

        SYS_POLL_WAIT => {
            let timeout = a0;
            let tid = sched.current;

            // Check if any events already fired.
            let fired = poll.check(tid, sched, ipc, channels);
            if fired != 0 {
                c.set_ret(0, fired);
                poll.clear(tid);
                SyscallAction::Resume
            } else if timeout == 0 {
                // Non-blocking poll — nothing ready.
                c.set_ret(0, 0);
                SyscallAction::Resume
            } else {
                // Block until events fire or timeout.
                if tid < sched.tasks.len() {
                    use crate::task::TaskState;
                    sched.tasks[tid].state = TaskState::Blocked;
                    sched.tasks[tid].block_reason = BlockReason::PollWait;
                    // If timeout != MAX, also set a wakeup tick as a deadline.
                    if timeout != usize::MAX {
                        sched.tasks[tid].wakeup_tick = sched.ticks.saturating_add(timeout as u64);
                    }
                }
                // Rewind PC so poll_wait is re-executed on wakeup.
                let pc = c.get_pc();
                c.set_pc(pc - TaskContext::INSTRUCTION_SIZE);
                SyscallAction::Reschedule
            }
        }

        // ── Sockets (0x70–0x7F) ─────────────────────────────────

        SYS_SOCKET => {
            let handle = sockets.create(a0, a1);
            c.set_ret(0, handle);
            SyscallAction::Resume
        }

        SYS_BIND => {
            let ret = sockets.bind(a0, a1);
            c.set_ret(0, ret);
            SyscallAction::Resume
        }

        SYS_LISTEN => {
            let ret = sockets.listen(a0, a1);
            c.set_ret(0, ret);
            SyscallAction::Resume
        }

        SYS_ACCEPT => {
            match sockets.accept(a0) {
                Some(new_handle) => {
                    c.set_ret(0, new_handle);
                    SyscallAction::Resume
                }
                None => {
                    // No pending connections — block.
                    let cur = sched.current;
                    if cur < sched.tasks.len() {
                        sched.tasks[cur].state = TaskState::Blocked;
                        sched.tasks[cur].block_reason = BlockReason::SockAccept(a0);
                    }
                    let pc = c.get_pc();
                    c.set_pc(pc - TaskContext::INSTRUCTION_SIZE);
                    SyscallAction::Reschedule
                }
            }
        }

        SYS_CONNECT => {
            let ret = sockets.connect(sched, a0, a1);
            c.set_ret(0, ret);
            SyscallAction::Resume
        }

        SYS_SOCK_SEND => {
            let ptr = a1;
            let len = a2;
            if !check_user_ptr(sched, processes, ptr, len, MemPerms::READ) {
                c.set_ret(0, usize::MAX);
                SyscallAction::Resume
            } else {
                let buf = unsafe { core::slice::from_raw_parts(ptr as *const u8, len) };
                match sockets.send_bytes(sched, a0, buf) {
                    Some(n) => {
                        c.set_ret(0, n);
                        SyscallAction::Resume
                    }
                    None => {
                        // Peer buffer full — block sender.
                        let cur = sched.current;
                        if cur < sched.tasks.len() {
                            sched.tasks[cur].state = TaskState::Blocked;
                            sched.tasks[cur].block_reason = BlockReason::SockSend(a0);
                        }
                        let pc = c.get_pc();
                        c.set_pc(pc - TaskContext::INSTRUCTION_SIZE);
                        SyscallAction::Reschedule
                    }
                }
            }
        }

        SYS_SOCK_RECV => {
            let ptr = a1;
            let len = a2;
            if !check_user_ptr(sched, processes, ptr, len, MemPerms::WRITE) {
                c.set_ret(0, usize::MAX);
                SyscallAction::Resume
            } else {
                let buf = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) };
                match sockets.recv_bytes(a0, buf) {
                    Some(n) => {
                        c.set_ret(0, n);
                        SyscallAction::Resume
                    }
                    None => {
                        // No data — block receiver.
                        let cur = sched.current;
                        if cur < sched.tasks.len() {
                            sched.tasks[cur].state = TaskState::Blocked;
                            sched.tasks[cur].block_reason = BlockReason::SockRecv(a0);
                        }
                        let pc = c.get_pc();
                        c.set_pc(pc - TaskContext::INSTRUCTION_SIZE);
                        SyscallAction::Reschedule
                    }
                }
            }
        }

        SYS_SOCK_CLOSE => {
            let ret = sockets.close(sched, a0);
            c.set_ret(0, ret);
            SyscallAction::Resume
        }

        // ── User identity ───────────────────────────────────────
        SYS_GETUID => {
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() {
                sched.tasks[cur].process_id
            } else {
                usize::MAX
            };
            let uid = if pid < crate::process::MAX_PROCESSES {
                processes.processes[pid].uid as usize
            } else {
                0
            };
            c.set_ret(0, uid);
            SyscallAction::Resume
        }

        SYS_GETGID => {
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() {
                sched.tasks[cur].process_id
            } else {
                usize::MAX
            };
            let gid = if pid < crate::process::MAX_PROCESSES {
                processes.processes[pid].gid as usize
            } else {
                0
            };
            c.set_ret(0, gid);
            SyscallAction::Resume
        }

        SYS_SETUID => {
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() {
                sched.tasks[cur].process_id
            } else {
                usize::MAX
            };
            // Only root (uid 0) may change UID.
            if pid < crate::process::MAX_PROCESSES
                && processes.processes[pid].uid == crate::user::ROOT_UID
            {
                let new_uid = a0 as u16;
                processes.processes[pid].uid = new_uid;
                c.set_ret(0, 0);
            } else {
                c.set_ret(0, usize::MAX);
            }
            SyscallAction::Resume
        }

        SYS_LOGIN => {
            // a0 = username ptr, a1 = username len, a2 = password ptr, a3 = password len
            let name_ok = check_user_ptr(sched, processes, a0, a1, MemPerms::READ);
            let pass_ok = check_user_ptr(sched, processes, a2, a3, MemPerms::READ);
            if name_ok && pass_ok && a1 > 0 && a3 > 0 && a1 < 64 && a3 < 64 {
                let name_bytes = unsafe { core::slice::from_raw_parts(a0 as *const u8, a1) };
                let pass_bytes = unsafe { core::slice::from_raw_parts(a2 as *const u8, a3) };
                if let Ok(name_str) = core::str::from_utf8(name_bytes) {
                    match users.login(name_str, pass_bytes) {
                        Ok(token) => {
                            // Set the process UID/GID to the logged-in user.
                            let cur = sched.current;
                            let pid = if cur < sched.tasks.len() {
                                sched.tasks[cur].process_id
                            } else {
                                usize::MAX
                            };
                            if let Some((uid, gid)) = users.session_info(token) {
                                if pid < crate::process::MAX_PROCESSES {
                                    processes.processes[pid].uid = uid;
                                    processes.processes[pid].gid = gid;
                                }
                            }
                            c.set_ret(0, token as usize);
                        }
                        Err(_) => {
                            c.set_ret(0, 0);
                        }
                    }
                } else {
                    c.set_ret(0, 0);
                }
            } else {
                c.set_ret(0, 0);
            }
            SyscallAction::Resume
        }

        SYS_LOGOUT => {
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() {
                sched.tasks[cur].process_id
            } else {
                usize::MAX
            };
            let uid = if pid < crate::process::MAX_PROCESSES {
                processes.processes[pid].uid
            } else {
                crate::user::NOBODY_UID
            };
            // Find and invalidate the session for this user.
            let mut found = false;
            for s in users.sessions.iter_mut() {
                if s.active && s.uid == uid {
                    *s = crate::user::Session::empty();
                    found = true;
                    break;
                }
            }
            // Reset process to root in single-user / after logout.
            if pid < crate::process::MAX_PROCESSES {
                processes.processes[pid].uid = crate::user::ROOT_UID;
                processes.processes[pid].gid = crate::user::ROOT_GID;
            }
            c.set_ret(0, found as usize);
            SyscallAction::Resume
        }

        // ── Filesystem (0xA0–0xAF) ─────────────────────────────

        SYS_OPEN => {
            let path_ptr = a0;
            let path_len = a1;
            let flags = OpenFlags::from_raw(a2 as u8);
            if !check_user_ptr(sched, processes, path_ptr, path_len, MemPerms::READ) || path_len == 0 || path_len > 256 {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let path_bytes = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len) };
            let path = match core::str::from_utf8(path_bytes) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };

            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let cwd = processes.processes[pid].cwd;

            // Resolve path.
            let inode_id = if flags.contains(OpenFlags::O_CREAT) {
                match inodes.resolve(cwd, path) {
                    Some(id) => {
                        // File exists — if O_TRUNC, truncate it.
                        if flags.contains(OpenFlags::O_TRUNC) && inodes.inodes[id as usize].kind == InodeKind::File {
                            let mount_id = inodes.inodes[id as usize].dev_major;
                            if mount_id > 0 && fat32.mounted {
                                fat32.truncate(inodes, id);
                            } else {
                                ramfs.truncate(inodes, id, 0);
                            }
                        }
                        id
                    }
                    None => {
                        // Create the file. Find parent directory and file name.
                        let (parent_path, file_name) = split_parent_name(path);
                        let parent_id = if parent_path.is_empty() {
                            cwd
                        } else {
                            match inodes.resolve(cwd, parent_path) {
                                Some(p) => p,
                                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
                            }
                        };
                        // Check if the parent is under a FAT32 mount.
                        let parent_mount = inodes.inodes[parent_id as usize].dev_major;
                        if parent_mount > 0 && fat32.mounted {
                            match fat32.create_file(inodes, parent_id, file_name, parent_mount) {
                                Some(id) => id,
                                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
                            }
                        } else {
                            match inodes.create_file_in(parent_id, file_name) {
                                Some(id) => id,
                                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
                            }
                        }
                    }
                }
            } else {
                match inodes.resolve(cwd, path) {
                    Some(id) => {
                        if flags.contains(OpenFlags::O_TRUNC) && inodes.inodes[id as usize].kind == InodeKind::File {
                            let mount_id = inodes.inodes[id as usize].dev_major;
                            if mount_id > 0 && fat32.mounted {
                                fat32.truncate(inodes, id);
                            } else {
                                ramfs.truncate(inodes, id, 0);
                            }
                        }
                        id
                    }
                    None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
                }
            };

            // Find a free FD slot.
            let proc = &mut processes.processes[pid];
            let mut fd_idx = usize::MAX;
            for i in 0..MAX_FDS {
                if proc.fds[i].is_none() {
                    fd_idx = i;
                    break;
                }
            }
            if fd_idx == usize::MAX {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }

            proc.fds[fd_idx] = Some(FileDescriptor {
                inode_id,
                cursor: if flags.contains(OpenFlags::O_APPEND) {
                    inodes.inodes[inode_id as usize].size
                } else {
                    0
                },
                flags,
            });
            proc.open_handles += 1;
            c.set_ret(0, fd_idx);
            SyscallAction::Resume
        }

        SYS_CLOSE => {
            let fd = a0;
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES || fd >= MAX_FDS {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let proc = &mut processes.processes[pid];
            if proc.fds[fd].is_some() {
                proc.fds[fd] = None;
                proc.open_handles = proc.open_handles.saturating_sub(1);
                c.set_ret(0, 0);
            } else {
                c.set_ret(0, usize::MAX);
            }
            SyscallAction::Resume
        }

        SYS_READ => {
            let fd = a0;
            let buf_ptr = a1;
            let count = a2;
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES || fd >= MAX_FDS {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            if !check_user_ptr(sched, processes, buf_ptr, count, MemPerms::WRITE) {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let proc = &mut processes.processes[pid];
            let file_desc = match proc.fds[fd] {
                Some(ref f) => *f,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            if !file_desc.flags.readable() {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }

            let inode_id = file_desc.inode_id;
            let idx = inode_id as usize;
            if idx >= crate::vfs::MAX_INODES {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }

            let kind = inodes.inodes[idx].kind;
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, count) };

            let bytes = match kind {
                InodeKind::Device => {
                    // Device dispatch.
                    let major = inodes.inodes[idx].dev_major;
                    let minor = inodes.inodes[idx].dev_minor;
                    dev_read(major, minor, buf, console_read, input)
                }
                InodeKind::File => {
                    let mount_id = inodes.inodes[idx].dev_major;
                    if mount_id > 0 && fat32.mounted {
                        fat32.read(inodes, inode_id, file_desc.cursor, buf)
                    } else {
                        ramfs.read(inodes, inode_id, file_desc.cursor, buf)
                    }
                }
                _ => 0,
            };

            // Update cursor.
            if let Some(ref mut f) = processes.processes[pid].fds[fd] {
                f.cursor += bytes as u32;
            }

            c.set_ret(0, bytes);
            SyscallAction::Resume
        }

        SYS_WRITE => {
            let fd = a0;
            let buf_ptr = a1;
            let count = a2;
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES || fd >= MAX_FDS {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            if !check_user_ptr(sched, processes, buf_ptr, count, MemPerms::READ) {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let proc = &mut processes.processes[pid];
            let file_desc = match proc.fds[fd] {
                Some(ref f) => *f,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            if !file_desc.flags.writable() {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }

            let inode_id = file_desc.inode_id;
            let idx = inode_id as usize;
            if idx >= crate::vfs::MAX_INODES {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }

            let kind = inodes.inodes[idx].kind;
            let data = unsafe { core::slice::from_raw_parts(buf_ptr as *const u8, count) };

            let cursor = if file_desc.flags.contains(OpenFlags::O_APPEND) {
                inodes.inodes[idx].size
            } else {
                file_desc.cursor
            };

            let bytes = match kind {
                InodeKind::Device => {
                    let major = inodes.inodes[idx].dev_major;
                    let minor = inodes.inodes[idx].dev_minor;
                    dev_write(major, minor, data, console_write)
                }
                InodeKind::File => {
                    let mount_id = inodes.inodes[idx].dev_major;
                    if mount_id > 0 && fat32.mounted {
                        fat32.write(inodes, inode_id, cursor, data)
                    } else {
                        ramfs.write(inodes, inode_id, cursor, data)
                    }
                }
                _ => 0,
            };

            // Update cursor.
            if let Some(ref mut f) = processes.processes[pid].fds[fd] {
                f.cursor = cursor + bytes as u32;
            }

            c.set_ret(0, bytes);
            SyscallAction::Resume
        }

        SYS_SEEK => {
            let fd = a0;
            let offset = a1;
            let whence = a2;
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES || fd >= MAX_FDS {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let f = match processes.processes[pid].fds[fd] {
                Some(ref f) => *f,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let inode_id = f.inode_id;
            let idx = inode_id as usize;
            if idx >= crate::vfs::MAX_INODES {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let size = inodes.inodes[idx].size;
            let new_pos = match whence {
                SEEK_SET => offset as u32,
                SEEK_CUR => {
                    let cur_pos = f.cursor;
                    cur_pos.wrapping_add(offset as u32)
                }
                SEEK_END => {
                    if offset as u32 > size {
                        0
                    } else {
                        size - offset as u32
                    }
                }
                _ => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            if let Some(ref mut fd_ent) = processes.processes[pid].fds[fd] {
                fd_ent.cursor = new_pos;
            }
            c.set_ret(0, new_pos as usize);
            SyscallAction::Resume
        }

        SYS_STAT => {
            let path_ptr = a0;
            let path_len = a1;
            let stat_ptr = a2;
            if !check_user_ptr(sched, processes, path_ptr, path_len, MemPerms::READ)
                || !check_user_ptr(sched, processes, stat_ptr, core::mem::size_of::<StatBuf>(), MemPerms::WRITE)
                || path_len == 0 || path_len > 256
            {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let path_bytes = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len) };
            let path = match core::str::from_utf8(path_bytes) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            let cwd = if pid < crate::process::MAX_PROCESSES { processes.processes[pid].cwd } else { ROOT_INODE };
            let inode_id = match inodes.resolve(cwd, path) {
                Some(id) => id,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let inode = &inodes.inodes[inode_id as usize];
            let stat = StatBuf {
                kind: inode.kind as u8,
                dev_major: inode.dev_major,
                dev_minor: inode.dev_minor,
                _pad: 0,
                size: inode.size,
                inode_id,
                parent: inode.parent,
            };
            let dst = stat_ptr as *mut StatBuf;
            unsafe { dst.write(stat); }
            c.set_ret(0, 0);
            SyscallAction::Resume
        }

        SYS_FSTAT => {
            let fd = a0;
            let stat_ptr = a1;
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES || fd >= MAX_FDS
                || !check_user_ptr(sched, processes, stat_ptr, core::mem::size_of::<StatBuf>(), MemPerms::WRITE)
            {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let f = match processes.processes[pid].fds[fd] {
                Some(ref f) => *f,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let inode_id = f.inode_id;
            let idx = inode_id as usize;
            if idx >= crate::vfs::MAX_INODES || inodes.inodes[idx].kind == InodeKind::Free {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let inode = &inodes.inodes[idx];
            let stat = StatBuf {
                kind: inode.kind as u8,
                dev_major: inode.dev_major,
                dev_minor: inode.dev_minor,
                _pad: 0,
                size: inode.size,
                inode_id,
                parent: inode.parent,
            };
            let dst = stat_ptr as *mut StatBuf;
            unsafe { dst.write(stat); }
            c.set_ret(0, 0);
            SyscallAction::Resume
        }

        SYS_MKDIR => {
            let path_ptr = a0;
            let path_len = a1;
            if !check_user_ptr(sched, processes, path_ptr, path_len, MemPerms::READ) || path_len == 0 || path_len > 256 {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let path_bytes = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len) };
            let path = match core::str::from_utf8(path_bytes) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            let cwd = if pid < crate::process::MAX_PROCESSES { processes.processes[pid].cwd } else { ROOT_INODE };
            let (parent_path, dir_name) = split_parent_name(path);
            let parent_id = if parent_path.is_empty() {
                cwd
            } else {
                match inodes.resolve(cwd, parent_path) {
                    Some(p) => p,
                    None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
                }
            };
            // Check parent is a directory.
            let pi = parent_id as usize;
            if pi >= crate::vfs::MAX_INODES || inodes.inodes[pi].kind != InodeKind::Directory {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            // Check name doesn't already exist.
            if inodes.find_child(parent_id, dir_name).is_some() {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            match inodes.mkdir_in(parent_id, dir_name) {
                Some(_) => c.set_ret(0, 0),
                None => c.set_ret(0, usize::MAX),
            }
            SyscallAction::Resume
        }

        SYS_UNLINK => {
            let path_ptr = a0;
            let path_len = a1;
            if !check_user_ptr(sched, processes, path_ptr, path_len, MemPerms::READ) || path_len == 0 || path_len > 256 {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let path_bytes = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len) };
            let path = match core::str::from_utf8(path_bytes) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            let cwd = if pid < crate::process::MAX_PROCESSES { processes.processes[pid].cwd } else { ROOT_INODE };
            let inode_id = match inodes.resolve(cwd, path) {
                Some(id) => id,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            if inodes.unlink(inode_id) {
                c.set_ret(0, 0);
            } else {
                c.set_ret(0, usize::MAX);
            }
            SyscallAction::Resume
        }

        SYS_READDIR => {
            let fd = a0;
            let entry_ptr = a1;
            let max_entries = a2;
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES || fd >= MAX_FDS {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let entry_size = core::mem::size_of::<DirEntry>();
            if !check_user_ptr(sched, processes, entry_ptr, max_entries * entry_size, MemPerms::WRITE) {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let f = match processes.processes[pid].fds[fd] {
                Some(ref f) => *f,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let inode_id = f.inode_id;
            let idx = inode_id as usize;
            if idx >= crate::vfs::MAX_INODES || inodes.inodes[idx].kind != InodeKind::Directory {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }

            // Walk children, skip `cursor` entries, return up to max_entries.
            let cursor = f.cursor as usize;
            let mut child = inodes.inodes[idx].children_head;
            let mut skip = cursor;
            let mut count = 0usize;
            while child != NO_INODE && count < max_entries {
                let ci = child as usize;
                if ci >= crate::vfs::MAX_INODES { break; }
                if skip > 0 {
                    skip -= 1;
                    child = inodes.inodes[ci].next_sibling;
                    continue;
                }
                let ch_inode = &inodes.inodes[ci];
                let mut entry = DirEntry {
                    kind: ch_inode.kind as u8,
                    name_len: 0,
                    size_lo: ch_inode.size as u16,
                    size_hi: (ch_inode.size >> 16) as u16,
                    _pad: 0,
                    name: [0u8; 28],
                };
                let name = ch_inode.name_str();
                let nlen = if name.len() > 27 { 27 } else { name.len() };
                entry.name[..nlen].copy_from_slice(&name.as_bytes()[..nlen]);
                entry.name_len = nlen as u8;
                let dst = (entry_ptr + count * entry_size) as *mut DirEntry;
                unsafe { dst.write(entry); }
                count += 1;
                child = inodes.inodes[ci].next_sibling;
            }

            // Advance cursor.
            if let Some(ref mut fd_ent) = processes.processes[pid].fds[fd] {
                fd_ent.cursor += count as u32;
            }

            c.set_ret(0, count);
            SyscallAction::Resume
        }

        SYS_TRUNCATE => {
            let fd = a0;
            let new_size = a1 as u32;
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES || fd >= MAX_FDS {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let f = match processes.processes[pid].fds[fd] {
                Some(ref f) => *f,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            if !f.flags.writable() {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let inode_idx = f.inode_id as usize;
            let mount_id = if inode_idx < crate::vfs::MAX_INODES {
                inodes.inodes[inode_idx].dev_major
            } else {
                0
            };
            let ok = if mount_id > 0 && fat32.mounted {
                fat32.truncate(inodes, f.inode_id)
            } else {
                ramfs.truncate(inodes, f.inode_id, new_size)
            };
            if ok {
                c.set_ret(0, 0);
            } else {
                c.set_ret(0, usize::MAX);
            }
            SyscallAction::Resume
        }

        SYS_RENAME => {
            let old_ptr = a0;
            let old_len = a1;
            let new_ptr = a2;
            let new_len = a3;
            if !check_user_ptr(sched, processes, old_ptr, old_len, MemPerms::READ)
                || !check_user_ptr(sched, processes, new_ptr, new_len, MemPerms::READ)
                || old_len == 0 || old_len > 256 || new_len == 0 || new_len > 256
            {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let old_bytes = unsafe { core::slice::from_raw_parts(old_ptr as *const u8, old_len) };
            let new_bytes = unsafe { core::slice::from_raw_parts(new_ptr as *const u8, new_len) };
            let old_path = match core::str::from_utf8(old_bytes) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let new_path = match core::str::from_utf8(new_bytes) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            let cwd = if pid < crate::process::MAX_PROCESSES { processes.processes[pid].cwd } else { ROOT_INODE };
            let src_id = match inodes.resolve(cwd, old_path) {
                Some(id) => id,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let (parent_path, new_name) = split_parent_name(new_path);
            let new_parent = if parent_path.is_empty() {
                cwd
            } else {
                match inodes.resolve(cwd, parent_path) {
                    Some(p) => p,
                    None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
                }
            };
            if inodes.rename(src_id, new_parent, new_name) {
                c.set_ret(0, 0);
            } else {
                c.set_ret(0, usize::MAX);
            }
            SyscallAction::Resume
        }

        SYS_GETCWD => {
            let buf_ptr = a0;
            let buf_len = a1;
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            if !check_user_ptr(sched, processes, buf_ptr, buf_len, MemPerms::WRITE) || buf_len == 0 {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let cwd = processes.processes[pid].cwd;
            let buf = unsafe { core::slice::from_raw_parts_mut(buf_ptr as *mut u8, buf_len) };
            let written = inodes.build_path(cwd, buf);
            c.set_ret(0, written);
            SyscallAction::Resume
        }

        SYS_CHDIR => {
            let path_ptr = a0;
            let path_len = a1;
            if !check_user_ptr(sched, processes, path_ptr, path_len, MemPerms::READ) || path_len == 0 || path_len > 256 {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let path_bytes = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len) };
            let path = match core::str::from_utf8(path_bytes) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let cwd = processes.processes[pid].cwd;
            let inode_id = match inodes.resolve(cwd, path) {
                Some(id) => id,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            // Must be a directory.
            let idx = inode_id as usize;
            if idx >= crate::vfs::MAX_INODES || inodes.inodes[idx].kind != InodeKind::Directory {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            processes.processes[pid].cwd = inode_id;
            c.set_ret(0, 0);
            SyscallAction::Resume
        }

        SYS_MOUNT => {
            // a0 = device label ptr, a1 = label len
            // a2 = mount path ptr,   a3 = mount path len
            let label_ptr = a0;
            let label_len = a1;
            let path_ptr = a2;
            let path_len = a3;
            if !check_user_ptr(sched, processes, label_ptr, label_len, MemPerms::READ)
                || !check_user_ptr(sched, processes, path_ptr, path_len, MemPerms::READ)
                || label_len == 0 || label_len > 8 || path_len == 0 || path_len > 256
            {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let _label = unsafe { core::slice::from_raw_parts(label_ptr as *const u8, label_len) };
            let path_bytes = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len) };
            let path = match core::str::from_utf8(path_bytes) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let _label_str = match core::str::from_utf8(_label) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let cwd = processes.processes[pid].cwd;
            // Resolve mount target — must be an existing directory.
            let dir_id = match inodes.resolve(cwd, path) {
                Some(id) => id,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let didx = dir_id as usize;
            if didx >= crate::vfs::MAX_INODES || inodes.inodes[didx].kind != InodeKind::Directory {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            // Register in mount table. FAT32 init is handled by the kernel
            // (it calls fat32.mount() with the appropriate block-device fn
            // pointers before issuing this syscall, or the kernel wires it
            // up on boot). Here we just record the mount in the table.
            match mounts.mount(dir_id, FsType::Fat32, _label_str) {
                Some(_mid) => {
                    c.set_ret(0, 0);
                }
                None => {
                    c.set_ret(0, usize::MAX);
                }
            }
            SyscallAction::Resume
        }

        SYS_UMOUNT => {
            let path_ptr = a0;
            let path_len = a1;
            if !check_user_ptr(sched, processes, path_ptr, path_len, MemPerms::READ) || path_len == 0 || path_len > 256 {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let path_bytes = unsafe { core::slice::from_raw_parts(path_ptr as *const u8, path_len) };
            let path = match core::str::from_utf8(path_bytes) {
                Ok(s) => s,
                Err(_) => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            let cur = sched.current;
            let pid = if cur < sched.tasks.len() { sched.tasks[cur].process_id } else { usize::MAX };
            if pid >= crate::process::MAX_PROCESSES {
                c.set_ret(0, usize::MAX);
                return SyscallAction::Resume;
            }
            let cwd = processes.processes[pid].cwd;
            let dir_id = match inodes.resolve(cwd, path) {
                Some(id) => id,
                None => { c.set_ret(0, usize::MAX); return SyscallAction::Resume; }
            };
            if mounts.unmount(dir_id) {
                fat32.unmount();
                c.set_ret(0, 0);
            } else {
                c.set_ret(0, usize::MAX);
            }
            SyscallAction::Resume
        }

        // ── Driver / userspace I/O ───────────────────────────────
        SYS_DRV_MMIO_READ32 => {
            let addr = a0;
            // Validate that the calling task has a PMP-granted region covering this address.
            let cur = sched.current;
            let allowed = if cur < sched.tasks.len() {
                let tcb = &sched.tasks[cur];
                // Check thread regions for an MMIO grant (RW permission).
                tcb.regions[..tcb.region_count].iter().any(|r| r.allows(addr, 4, MemPerms::READ))
            } else {
                false
            };
            if allowed {
                let val = unsafe { core::ptr::read_volatile(addr as *const u32) };
                c.set_ret(0, val as usize);
            } else {
                c.set_ret(0, usize::MAX);
            }
            SyscallAction::Resume
        }

        SYS_DRV_MMIO_WRITE32 => {
            let addr = a0;
            let val = a1 as u32;
            let cur = sched.current;
            let allowed = if cur < sched.tasks.len() {
                let tcb = &sched.tasks[cur];
                tcb.regions[..tcb.region_count].iter().any(|r| r.allows(addr, 4, MemPerms::RW))
            } else {
                false
            };
            if allowed {
                unsafe { core::ptr::write_volatile(addr as *mut u32, val) };
                c.set_ret(0, 0);
            } else {
                c.set_ret(0, usize::MAX);
            }
            SyscallAction::Resume
        }

        SYS_DRV_IRQ_WAIT => {
            let irq_line = a0;
            let cur = sched.current;
            if cur < sched.tasks.len() {
                sched.tasks[cur].state = TaskState::Blocked;
                sched.tasks[cur].block_reason = BlockReason::IrqWait(irq_line);
            }
            c.set_ret(0, 0);
            SyscallAction::Reschedule
        }

        SYS_DRV_IRQ_ACK => {
            // Re-enable the interrupt line after handling.
            // The actual ack is done by the kernel in the ISR; this is
            // a notification from the driver that it finished handling.
            c.set_ret(0, 0);
            SyscallAction::Resume
        }

        SYS_DRV_REGISTER => {
            let chan_id = a0;
            let drv_handle = a1;
            // Validate: driver handle exists and channel is open.
            let ok = drv_handle < drivers.count()
                && chan_id < crate::channel::MAX_CHANNELS
                && channels.chans[chan_id].open;
            c.set_ret(0, if ok { 0 } else { usize::MAX });
            SyscallAction::Resume
        }

        SYS_DRV_LOG => {
            let ptr = a0 as *const u8;
            let len = a1;
            if len > 0 && len <= 256 && check_user_ptr(sched, processes, a0, len, MemPerms::READ) {
                let buf = unsafe { core::slice::from_raw_parts(ptr, len) };
                for &b in buf {
                    console_write(b);
                }
            }
            c.set_ret(0, 0);
            SyscallAction::Resume
        }

        // ── Unknown ─────────────────────────────────────────────
        _ => {
            // Unknown syscall — return -1 (usize::MAX) in ret0.
            c.set_ret(0, usize::MAX);
            SyscallAction::Resume
        }
    }
}

/// Check sleeping tasks and wake any whose target tick has been reached.
///
/// Call this from the timer tick handler, after incrementing `sched.ticks`.
pub fn wake_sleepers(sched: &mut Scheduler) {
    use crate::task::TaskState;
    let now = sched.ticks;
    for task in sched.tasks.iter_mut() {
        if task.state == TaskState::Blocked && task.block_reason == BlockReason::Sleep {
            if task.wakeup_tick > 0 && now >= task.wakeup_tick {
                task.wakeup_tick = 0;
                task.block_reason = BlockReason::None;
                task.state = TaskState::Ready;
            }
        }
    }
}

/// Wake any driver tasks blocked on `IrqWait` for the given IRQ line.
///
/// Called from the hardware ISR after acknowledging the interrupt.
pub fn wake_irq_waiters(sched: &mut Scheduler, irq_line: usize) {
    for task in sched.tasks.iter_mut() {
        if task.state == TaskState::Blocked && task.block_reason == BlockReason::IrqWait(irq_line) {
            task.block_reason = BlockReason::None;
            task.state = TaskState::Ready;
        }
    }
}

// ─── Filesystem helpers ─────────────────────────────────────────────────

/// Split a path into (parent_path, final_component).
/// e.g., "/tmp/foo/bar" → ("/tmp/foo", "bar"), "hello" → ("", "hello").
fn split_parent_name(path: &str) -> (&str, &str) {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(pos) => {
            let parent = if pos == 0 { "/" } else { &path[..pos] };
            (&parent, &path[pos + 1..])
        }
        None => ("", path),
    }
}

/// Device read dispatcher (devfs).
/// Major 0: null/zero/console/random.
/// Major 1: input devices (keyboard/mouse).
fn dev_read(major: u8, minor: u8, buf: &mut [u8], console_read: fn() -> u8, input: &mut InputSubsystem) -> usize {
    match (major, minor) {
        // /dev/null — EOF
        (0, 0) => 0,
        // /dev/zero — fill with zeroes
        (0, 1) => {
            for b in buf.iter_mut() {
                *b = 0;
            }
            buf.len()
        }
        // /dev/console — read from serial
        (0, 2) => {
            if !buf.is_empty() {
                buf[0] = console_read();
                1
            } else {
                0
            }
        }
        // /dev/random — simple PRNG
        (0, 3) => {
            // Very simple xorshift32 PRNG — not cryptographic.
            static mut PRNG_STATE: u32 = 0xDEAD_BEEF;
            for b in buf.iter_mut() {
                unsafe {
                    PRNG_STATE ^= PRNG_STATE << 13;
                    PRNG_STATE ^= PRNG_STATE >> 17;
                    PRNG_STATE ^= PRNG_STATE << 5;
                    *b = PRNG_STATE as u8;
                }
            }
            buf.len()
        }
        // /dev/keyboard — read ASCII bytes from keyboard queue
        (1, 0) => input.kbd_read(buf),
        // /dev/mouse — read encoded mouse events
        (1, 1) => input.mouse_read(buf),
        _ => 0,
    }
}

/// Device write dispatcher (devfs).
fn dev_write(major: u8, minor: u8, data: &[u8], console_write: fn(u8)) -> usize {
    match (major, minor) {
        // /dev/null — discard
        (0, 0) => data.len(),
        // /dev/zero — discard
        (0, 1) => data.len(),
        // /dev/console — write to serial
        (0, 2) => {
            for &b in data {
                console_write(b);
            }
            data.len()
        }
        // /dev/random — discard
        (0, 3) => data.len(),
        // /dev/keyboard, /dev/mouse — input devices, writes discarded
        (1, 0) | (1, 1) => data.len(),
        _ => 0,
    }
}

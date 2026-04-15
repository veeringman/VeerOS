//! Sample user tasks that exercise the VeerOS userlib syscall interface.
//!
//! Each task is a `fn() -> !` that can be registered with the scheduler.
//! They use **only** the userlib API (no direct kernel or hardware access)
//! to prove the syscall path works end-to-end.

use userlib::{print, println};

// ─────────────────────────────────────────────────────────────────────────
// Sample 1: Hello — basic console I/O via syscalls
// ─────────────────────────────────────────────────────────────────────────

pub fn hello_task() -> ! {
    println!("╔════════════════════════════════════╗");
    println!("║  hello_task: userlib console test   ║");
    println!("╚════════════════════════════════════╝");

    let tid = userlib::task::id();
    let pri = userlib::task::priority();
    let cnt = userlib::task::task_count();
    println!("[hello] my task id  = {}", tid);
    println!("[hello] my priority = {}", pri);
    println!("[hello] total tasks = {}", cnt);

    print!("[hello] byte-by-byte: ");
    for b in b"VeerOS\r\n" {
        userlib::io::write_byte(*b);
    }

    println!("[hello] done — exiting with code 0");
    userlib::task::exit(0);
}

// ─────────────────────────────────────────────────────────────────────────
// Sample 2: Timer — tick counter and sleep syscalls
// ─────────────────────────────────────────────────────────────────────────

pub fn timer_task() -> ! {
    println!("[timer] starting timer test");

    let t0 = userlib::time::ticks();
    println!("[timer] initial ticks = {}", t0);

    println!("[timer] sleeping 100 ticks...");
    userlib::time::sleep(100);
    let t1 = userlib::time::ticks();
    println!("[timer] after sleep: ticks = {} (delta = {})", t1, t1 - t0);

    println!("[timer] sleeping 50 ticks...");
    userlib::time::sleep(50);
    let t2 = userlib::time::ticks();
    println!("[timer] after sleep: ticks = {} (delta = {})", t2, t2 - t1);

    let t3 = userlib::time::ticks();
    for _ in 0..5 {
        userlib::task::yield_now();
    }
    let t4 = userlib::time::ticks();
    println!("[timer] 5 yields took {} ticks", t4 - t3);

    println!("[timer] done — exiting");
    userlib::task::exit(0);
}

// ─────────────────────────────────────────────────────────────────────────
// Sample 3: IPC — send a message to the ping-pong partner
// ─────────────────────────────────────────────────────────────────────────

pub fn ipc_sender_task() -> ! {
    println!("[ipc-tx] sender starting (task {})", userlib::task::id());

    userlib::time::sleep(10);

    let receiver_id = (userlib::task::id() + 1) as u8;

    for i in 0u32..3 {
        let opcode = 0x42u8;
        let ok = userlib::ipc::send(receiver_id, opcode, i as usize, 0xDEAD);
        println!(
            "[ipc-tx] sent msg #{} to task {} — ok={}",
            i, receiver_id, ok
        );
        userlib::time::sleep(20);
    }

    println!("[ipc-tx] done — exiting");
    userlib::task::exit(0);
}

pub fn ipc_receiver_task() -> ! {
    println!("[ipc-rx] receiver starting (task {})", userlib::task::id());

    for i in 0..3 {
        let pending = userlib::ipc::poll();
        println!("[ipc-rx] poll before recv #{}: pending={}", i, pending);

        let msg = userlib::ipc::recv();
        println!(
            "[ipc-rx] got msg #{}: sender={}, opcode=0x{:02X}, arg0={}, arg1=0x{:X}",
            i, msg.sender, msg.opcode, msg.arg0, msg.arg1
        );
    }

    println!("[ipc-rx] done — exiting");
    userlib::task::exit(0);
}

// ─────────────────────────────────────────────────────────────────────────
// Sample 5: Ring 3 user-mode task — proves true user-space syscalls work
// ─────────────────────────────────────────────────────────────────────────

/// A minimal user-mode task that runs at Ring 3 (CPL=3).
///
/// It has its own per-process page tables and uses `int 0x80` to make
/// syscalls into the kernel.
pub fn ring3_task() -> ! {
    println!("╔════════════════════════════════════╗");
    println!("║  ring3_task: TRUE USERSPACE (CPL3) ║");
    println!("╚════════════════════════════════════╝");

    let tid = userlib::task::id();
    println!("[ring3] task id = {}", tid);

    let ticks = userlib::time::ticks();
    println!("[ring3] ticks = {}", ticks);

    println!("[ring3] sleeping 50 ticks...");
    userlib::time::sleep(50);
    let t1 = userlib::time::ticks();
    println!("[ring3] after sleep: ticks = {}", t1);

    println!("[ring3] yielding 3 times...");
    for _ in 0..3 {
        userlib::task::yield_now();
    }

    println!("[ring3] done — exiting from userspace!");
    userlib::task::exit(0);
}

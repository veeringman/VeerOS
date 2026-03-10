//! Sample user tasks that exercise the VeerOS userlib syscall interface.
//!
//! Each task is a `fn() -> !` that can be registered with the scheduler.
//! They use **only** the userlib API (no direct kernel or hardware access)
//! to prove the syscall path works end-to-end.

use userlib::{print, println};

// ─────────────────────────────────────────────────────────────────────────
// Sample 1: Hello — basic console I/O via syscalls
// ─────────────────────────────────────────────────────────────────────────

/// Prints a greeting, queries its own task info, then exits cleanly.
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

    // Test raw byte I/O
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

/// Reads the tick counter, sleeps a few times, and prints elapsed ticks.
pub fn timer_task() -> ! {
    println!("[timer] starting timer test");

    let t0 = userlib::time::ticks();
    println!("[timer] initial ticks = {}", t0);

    // Sleep for 100 ms (100 ticks at 1 kHz)
    println!("[timer] sleeping 100 ticks...");
    userlib::time::sleep(100);
    let t1 = userlib::time::ticks();
    println!("[timer] after sleep: ticks = {} (delta = {})", t1, t1 - t0);

    // Sleep again
    println!("[timer] sleeping 50 ticks...");
    userlib::time::sleep(50);
    let t2 = userlib::time::ticks();
    println!("[timer] after sleep: ticks = {} (delta = {})", t2, t2 - t1);

    // Yield a few times and measure
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

/// Sender side of the IPC test. Sends 3 messages to the receiver task
/// (whose task ID is passed in via a known slot — task 4 sends to task 5).
pub fn ipc_sender_task() -> ! {
    println!("[ipc-tx] sender starting (task {})", userlib::task::id());

    // Wait a moment for the receiver to start first
    userlib::time::sleep(10);

    let receiver_id = (userlib::task::id() + 1) as u8; // convention: receiver is next slot

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

/// Receiver side of the IPC test. Waits for 3 messages, prints them.
pub fn ipc_receiver_task() -> ! {
    println!("[ipc-rx] receiver starting (task {})", userlib::task::id());

    for i in 0..3 {
        // Check if there's a message waiting (non-blocking poll)
        let pending = userlib::ipc::poll();
        println!("[ipc-rx] poll before recv #{}: pending={}", i, pending);

        // Blocking receive
        let msg = userlib::ipc::recv();
        println!(
            "[ipc-rx] got msg #{}: sender={}, opcode=0x{:02X}, arg0={}, arg1=0x{:X}",
            i, msg.sender, msg.opcode, msg.arg0, msg.arg1
        );
    }

    println!("[ipc-rx] done — exiting");
    userlib::task::exit(0);
}

//! Sample user tasks that exercise the VeerOS userlib syscall interface.
//!
//! Each task is a `fn() -> !` that can be registered with the scheduler.
//! They use **only** the userlib API (no direct kernel or hardware access)
//! to prove the syscall path works end-to-end.

#![allow(dead_code)]

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
        // Minimal output: just a dot per send to avoid flooding screen.
        userlib::io::write_byte(b'.');
        if !ok {
            userlib::io::write_byte(b'!');
        }
        userlib::time::sleep(20);
    }
    userlib::io::write_byte(b'\n');

    println!("[ipc-tx] done — exiting");
    // Diagnostic: unmistakable marker before exit
    userlib::io::write_byte(b'[');
    userlib::io::write_byte(b'T');
    userlib::io::write_byte(b'X');
    userlib::io::write_byte(b'-');
    userlib::io::write_byte(b'B');
    userlib::io::write_byte(b'Y');
    userlib::io::write_byte(b'E');
    userlib::io::write_byte(b']');
    userlib::io::write_byte(b'\n');
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
    userlib::io::write_byte(b'[');
    userlib::io::write_byte(b'R');
    userlib::io::write_byte(b'X');
    userlib::io::write_byte(b'-');
    userlib::io::write_byte(b'B');
    userlib::io::write_byte(b'Y');
    userlib::io::write_byte(b'E');
    userlib::io::write_byte(b']');
    userlib::io::write_byte(b'\n');
    userlib::task::exit(0);
}

// ─────────────────────────────────────────────────────────────────────────
// Sample 4: GPIO — blink an LED on GPIO 17 via syscalls
// ─────────────────────────────────────────────────────────────────────────

/// Configures GPIO 17 as output and toggles it 10 times (classic LED blink).
/// This exercises the SYS_GPIO_SET_MODE and SYS_GPIO_WRITE syscall path.
pub fn gpio_blink_task() -> ! {
    println!("[gpio] GPIO blink sample starting");

    let pin = 17u8; // RPi header pin 11
    userlib::gpio::set_mode(pin, userlib::gpio::OUTPUT);
    println!("[gpio] pin {} set to OUTPUT", pin);

    for i in 0..10 {
        let on = (i % 2) == 0;
        userlib::gpio::write(pin, on);
        println!("[gpio] pin {} = {}", pin, if on { "HIGH" } else { "LOW" });
        userlib::time::sleep(500); // 500 ms
    }

    println!("[gpio] blink done — exiting");
    userlib::task::exit(0);
}

// ─────────────────────────────────────────────────────────────────────────
// Sample 5: I2C scan — probe all addresses on bus 1
// ─────────────────────────────────────────────────────────────────────────

/// Scans I2C bus 1 (GPIO 2/3, header pins 3 & 5) for connected devices.
/// Each address that ACKs is printed. This is the `i2cdetect` equivalent.
pub fn i2c_scan_task() -> ! {
    println!("[i2c] I2C bus scan sample starting (bus 1)");

    let mut found = 0u32;
    for addr in 0x03u8..=0x77 {
        let mut buf = [0u8; 1];
        match userlib::hw::i2c_read(1, addr, &mut buf) {
            Ok(_) => {
                println!("[i2c]   found device at 0x{:02X}", addr);
                found += 1;
            }
            Err(_) => {}
        }
    }

    println!("[i2c] scan complete — {} device(s) found", found);
    userlib::task::exit(0);
}

// ─────────────────────────────────────────────────────────────────────────
// Sample 6: SPI loopback — send bytes and check MISO
// ─────────────────────────────────────────────────────────────────────────

/// Sends a short byte pattern on SPI bus 0 and prints what comes back.
/// With MOSI wired to MISO, all bytes should echo; otherwise RX is 0x00/0xFF.
pub fn spi_loopback_task() -> ! {
    println!("[spi] SPI loopback sample starting (bus 0)");

    let tx = [0xA5u8, 0x5A, 0xFF, 0x00, 0xDE, 0xAD, 0xBE, 0xEF];
    let mut rx = [0u8; 8];

    match userlib::hw::spi_transfer(0, &tx, &mut rx) {
        Ok(()) => {
            print!("[spi] TX:");
            for b in &tx {
                print!(" {:02X}", b);
            }
            println!();
            print!("[spi] RX:");
            for b in &rx {
                print!(" {:02X}", b);
            }
            println!();
            if tx == rx {
                println!("[spi] loopback PASS — MOSI=MISO verified");
            } else {
                println!("[spi] loopback data differs (no loopback wire?)");
            }
        }
        Err(e) => {
            println!("[spi] transfer error: {}", e);
        }
    }

    println!("[spi] done — exiting");
    userlib::task::exit(0);
}

// ─────────────────────────────────────────────────────────────────────────
// Sample 7: Temperature — read SoC temperature via syscall
// ─────────────────────────────────────────────────────────────────────────

/// Reads the SoC temperature 5 times with 1-second intervals.
pub fn temp_monitor_task() -> ! {
    println!("[temp] temperature monitor sample starting");

    for i in 0..5 {
        let mc = userlib::hw::temperature_millic();
        let whole = mc / 1000;
        let frac = (mc % 1000).unsigned_abs() / 100;
        println!("[temp] reading {}: {}.{}°C", i, whole, frac);
        userlib::time::sleep(1000);
    }

    println!("[temp] done — exiting");
    userlib::task::exit(0);
}

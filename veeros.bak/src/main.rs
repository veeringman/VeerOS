#![no_std]
#![no_main]

mod bootloader;
mod kernel;
mod net;
mod shell;

use core::panic::PanicInfo;
use panic_halt as _;
use riscv_rt::entry; // brings in a panic handler that halts the CPU
                     // #[panic_handler] fn panic(_info: &PanicInfo) -> ! { loop {} }

#[entry]
fn main() -> ! {
    // Initialize bootloader
    bootloader::init();

    // Start kernel
    kernel::start();

    // Loop forever
    loop {
        // Kernel scheduler or idle loop
    }
}

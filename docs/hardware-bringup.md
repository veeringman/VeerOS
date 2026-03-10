# ESP32-C3 Hardware Bring-up Checklist

Use this checklist when testing VeerOS on real ESP32-C3 hardware (or QEMU/Wokwi).

## Prerequisites
- ESP32-C3 dev board (or ESP32-C6 / H2 — adjust feature flag)
- USB-C cable + serial monitor @ 115200 baud (or ROM default)
- `espflash` or `esptool.py` for flashing

## Boot path overview
```
ROM bootloader
  └─▶ loads from flash at 0x0000
        └─▶ _start()                          // crates/kernel-esp32/src/main.rs
              ├─ UART0 early console init      // crates/bsp-esp32-riscv/src/uart.rs
              ├─ Kernel::boot()                // init_cpu → init_interrupts → init_timer
              ├─ Boot banner over UART0
              ├─ Install trap vector (csrw mtvec)
              ├─ Create tasks: idle + shell
              └─ Start scheduler → shell prompt
```

## Step-by-step verification

### 1. Build the kernel binary
```bash
cargo build -p kernel-esp32 --target riscv32imc-unknown-none-elf --release
```

### 2. Flash and monitor
```bash
espflash flash --chip esp32c3 --monitor \
    target/riscv32imc-unknown-none-elf/release/kernel-esp32
```

### 3. Expected UART output
```
================================================
  VeerOS v0.1.0
  Platform : ESP32-C3 (RISC-V)
  Scheduler: round-robin preemptive
================================================

VeerOS> 
```

### 4. Verification milestones

| # | Milestone | How to verify | Status |
|---|-----------|---------------|--------|
| 1 | UART TX works | Banner appears in serial monitor | ✅ |
| 2 | Platform name correct | Matches chip variant | ✅ |
| 3 | Scheduler label correct | Matches distribution feature flag | ✅ |
| 4 | No crash / watchdog reset | Board stays in idle loop | ✅ |
| 5 | Interrupt controller init | INTERRUPT_CORE0 configured | ✅ |
| 6 | SYSTIMER tick (1 ms) | Timer ISR fires, `uptime` increments | ✅ |
| 7 | Preemptive context switch | `tasks` shows idle + shell | ✅ |
| 8 | Shell interactive | Commands respond over UART | ✅ |

## Memory map (ESP32-C3)
| Region | Start | Length | Usage |
|--------|-------|--------|-------|
| IROM | 0x4038_0000 | 384 KiB | .text + .rodata |
| DRAM | 0x3FC8_0000 | 400 KiB | .data + .bss + stack |
| UART0 | 0x6000_0000 | — | MMIO registers |
| SYSTIMER | 0x6001_C000 | — | Timer counter + comparators |
| INTERRUPT_CORE0 | 0x600C_2000 | — | Interrupt matrix |

## QEMU `virt` reference (development)

| Region | Start | Length | Usage |
|--------|-------|--------|-------|
| RAM | 0x8000_0000 | 16 MiB | .text + .rodata + .data + .bss + stacks |
| NS16550a UART | 0x1000_0000 | — | Console I/O |
| CLINT | 0x0200_0000 | — | mtime / mtimecmp (10 MHz) |
| SiFive Test | 0x0010_0000 | — | Poweroff (write 0x5555) |

## Troubleshooting
- **No output**: verify baud rate, check if ROM bootloader output is visible first.
- **Garbled text**: ROM default baud is usually 115200; mismatch causes garble.
- **Watchdog reset loop**: disable RTC WDT in `init_cpu()` before banner.
- **QEMU hangs on exit**: ensure `qemu_poweroff()` writes `0x5555u32` to `0x100000`.

## QEMU Networking — Remote Shell

VeerOS includes a TCP remote shell (port 2323). To launch QEMU with
networking enabled:

### Build
```bash
cargo build -p kernel-qemu-virt --target riscv32imc-unknown-none-elf --release
```

### Run with user-net (port 2323 forwarded to host 12323)
```bash
qemu-system-riscv32 -nographic -machine virt \
    -bios none \
    -kernel target/riscv32imc-unknown-none-elf/release/kernel-qemu-virt \
    -device virtio-net-device,netdev=net0 \
    -netdev user,id=net0,hostfwd=tcp::12323-:2323
```

### Connect from host
```bash
# From another terminal:
nc localhost 12323
# or
telnet localhost 12323
```

You should see the VeerOS shell prompt over the network. The UART
console remains available as usual for local access.

### Memory map (QEMU virt, with networking)
| Region | Start | Usage |
|--------|-------|-------|
| VIRTIO-NET MMIO | 0x1000_1000–0x1000_8000 | Network device (probed) |

### Troubleshooting
- **`[net] no VIRTIO-NET device found`**: ensure `-device virtio-net-device` is
  passed to QEMU.
- **Connection refused on port 12323**: verify the `hostfwd` argument in `-netdev`.
- **Shell hangs after connect**: the net task must get CPU time; ensure the
  scheduler has ≥ 3 tasks (idle + shell + net).

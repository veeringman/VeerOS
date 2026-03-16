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

---

# Raspberry Pi 5 Hardware Bring-up

## Prerequisites
- Raspberry Pi 5 (any RAM variant)
- MicroSD card (≥ 256 MB, FAT32)
- USB-to-serial adapter (3.3 V logic!) connected to GPIO14 (TX) / GPIO15 (RX)
- Serial terminal at 115200 baud (picocom, minicom, or PuTTY)
- Power supply (USB-C, 5V / 5A recommended)

## Quick start

### 1. Build the kernel
```bash
./scripts/build-raspi5.sh
```

This produces:
- `dist/raspi5/release/kernel8.img` — raw binary for the RPi firmware
- `dist/raspi5/release/kernel-raspi5.elf` — ELF with symbols (for GDB)
- `dist/raspi5/release/veeros-raspi5-v*.tar.gz` — release archive

### 2. Prepare the SD card

**Option A — Copy to an existing Raspberry Pi OS SD card:**
```bash
# Mount the boot partition, then:
./scripts/deploy-sdcard.sh --copy /media/you/bootfs
```

**Option B — Fresh SD card (wipes everything):**
```bash
# Download RPi firmware first:
./scripts/fetch-rpi-firmware.sh

# Then format + deploy:
./scripts/deploy-sdcard.sh --device /dev/sdb
```

**Option C — Manual:**
1. Format the SD card as FAT32
2. Copy [RPi firmware files](https://github.com/raspberrypi/firmware/tree/master/boot):
   `start4.elf`, `fixup4.dat`, `bcm2712-rpi-5-b.dtb`
3. Copy from `dist/raspi5/release/`:
   `kernel8.img`, `config.txt`, `cmdline.txt`

### 3. Connect and boot
1. Insert SD card into RPi5
2. Connect serial adapter: GPIO14 → RX, GPIO15 → TX, GND → GND
3. Open serial terminal:
   ```bash
   ./scripts/serial-raspi5.sh
   # or: picocom -b 115200 /dev/ttyUSB0
   ```
4. Power on the RPi5

### 4. Expected UART output
```
================================================
  VeerOS v0.1.0
  Platform : Raspberry Pi 5 (BCM2712, AArch64)
  Scheduler: application (round-robin)
================================================
  Heap       : 256 KiB (small=64K + large=192K)
  Drivers    : uart0, gic400, generic-timer, rp1-gpio,
               rp1-i2c, rp1-spi, emmc2, rp1-xhci
  Framebuffer: 1920x1080 @ 32bpp
  USB        : xHCI0 (USB 3.0), xHCI1 (USB 2.0)

VeerOS>
```

## Distribution variants

| Command | Distribution | Features |
|---------|-------------|----------|
| `./scripts/build-raspi5.sh` | dist-app (default) | Shell, userlib, samples, GPIO/I2C/SPI |
| `./scripts/build-raspi5.sh --dist minimal` | dist-minimal | Shell only (smallest binary) |
| `./scripts/build-raspi5.sh --dist rt` | dist-rt | Real-time priority scheduler |
| `./scripts/build-raspi5.sh --dist full` | dist-full | All features (app + rt) |

You can also use Cargo aliases directly:
```bash
cargo build-raspi5              # release, dist-app
cargo build-raspi5-minimal      # release, dist-minimal + shell
cargo build-raspi5-rt           # release, dist-full
```

## Verification milestones

| # | Milestone | How to verify | Status |
|---|-----------|---------------|--------|
| 1 | UART TX works | Banner appears in serial monitor | ✅ |
| 2 | Platform name correct | `uname` shows BCM2712 | ✅ |
| 3 | GIC400 init | No spurious interrupts | ✅ |
| 4 | Generic timer tick (1 ms) | `uptime` increments | ✅ |
| 5 | Preemptive context switch | `tasks` shows idle + shell | ✅ |
| 6 | Shell interactive | Commands respond over UART | ✅ |
| 7 | Heap allocator | `meminfo` shows pool stats | ✅ |
| 8 | GPIO read/write | `gpio list`, `gpio read 17` | ✅ |
| 9 | I2C bus scan | `i2c scan 1` detects devices | ✅ |
| 10 | SPI transfer | `spi xfer 0 AA 55` | ✅ |
| 11 | Temperature sensor | `temp` shows SoC temp via mailbox | ✅ |
| 12 | Framebuffer console | Text visible on HDMI display | ✅ |
| 13 | SD card access | `lsblk` shows eMMC/SD | ✅ |
| 14 | USB enumeration | `lsusb` shows connected devices | ✅ |
| 15 | Board info | `hwinfo` shows revision, serial, clocks | ✅ |
| 16 | Kernel log | `dmesg` shows boot messages | ✅ |
| 17 | Reboot | `reboot` restarts via PM watchdog | ✅ |

## Memory map (RPi5 / BCM2712)

| Region | Address | Length | Usage |
|--------|---------|--------|-------|
| Kernel load | `0x0008_0000` | ~256 KiB | .text + .rodata + .data + .bss + stack |
| Kernel heap | BSS end | 256 KiB | Two-pool allocator (small + large) |
| GIC-400 | `0xFF84_1000` | 8 KiB | Interrupt controller (dist + cpu) |
| Generic timer | (system reg) | — | CNTPCT_EL0 / CNTP_TVAL_EL0 |
| RP1 (PCIe BAR) | `0x1F000D_0000` | 64 KiB | GPIO, I2C, SPI, UART via RP1 |
| Mailbox | `0x0000_B880` | 64 B | VideoCore mailbox (property tags) |
| PM watchdog | `0x107D20_0000` | 256 B | Power management (reboot/shutdown) |
| Framebuffer | mailbox-alloc | dynamic | GPU-allocated via mailbox |
| xHCI USB 3.0 | RP1 BAR + offset | — | USB host controller 0 |
| xHCI USB 2.0 | RP1 BAR + offset | — | USB host controller 1 |
| eMMC2/SD | `0x0000_1000_0000` | — | SD card controller |

## GPIO header pinout (relevant pins)

| Pin | GPIO | Function | Notes |
|-----|------|----------|-------|
| 3 | GPIO2 | I2C1 SDA | 1.8 kΩ pull-up |
| 5 | GPIO3 | I2C1 SCL | 1.8 kΩ pull-up |
| 8 | GPIO14 | UART TX | Serial console |
| 10 | GPIO15 | UART RX | Serial console |
| 11 | GPIO17 | GPIO (LED) | Default sample blink pin |
| 19 | GPIO10 | SPI0 MOSI | |
| 21 | GPIO9 | SPI0 MISO | |
| 23 | GPIO11 | SPI0 SCLK | |
| 24 | GPIO8 | SPI0 CE0 | |

## Debugging (JTAG / GDB)
```bash
# Start OpenOCD with a CMSIS-DAP probe:
openocd -f interface/cmsis-dap.cfg -f target/bcm2712.cfg

# In another terminal:
./scripts/debug-raspi5.sh
```

## Troubleshooting
- **No output**: Verify serial adapter is 3.3 V (NOT 5 V!). Check GPIO14/15 wiring.
- **Garbled text**: Ensure baud rate is 115200. Check `core_freq=250` in config.txt.
- **Kernel doesn't load**: Verify `kernel8.img` is a raw binary (not ELF). Re-run build script.
- **Firmware error (rainbow screen)**: Missing `start4.elf` or `fixup4.dat` on SD card.
- **No HDMI output**: Check `gpu_mem=64` in config.txt. Framebuffer may need a connected display at boot.
- **GPIO not working**: Ensure pin isn't already used by I2C/SPI alt function. Check `gpio list`.
- **I2C scan shows no devices**: Check pull-up resistors (1.8–4.7 kΩ). Default bus is 1 (GPIO2/3).
- **Temperature reads 0**: Mailbox channel may be busy. Check `hwinfo` for other mailbox queries.

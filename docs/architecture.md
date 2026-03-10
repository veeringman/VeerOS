# VeerOS Architecture

VeerOS is a **Rust-native, bare-metal operating system** targeting RISC-V
microcontrollers.  It cleanly separates **SoC drivers from kernel logic**
using a three-layer crate hierarchy, making it straightforward to add new
boards without rewriting the OS.

---

## Crate Hierarchy

```
┌─────────────────────────────────────────────────────────────────────┐
│  Layer 0 — arch                                                     │
│  Trait definitions: Platform, Serial, InterruptController,          │
│  TickTimer, NetworkDevice, TaskContext, Console                     │
└──────────────────────────┬──────────────────────────────────────────┘
                           │ implements
┌──────────────────────────▼──────────────────────────────────────────┐
│  Layer 1 — SoC drivers                                              │
│  ┌────────────────────┐  ┌──────────────────────┐                   │
│  │  soc-esp32          │  │  soc-qemu-virt        │                  │
│  │  • uart  (UART0)   │  │  • uart  (NS16550)    │                  │
│  │  • intc  (INTC)    │  │  • clint (mtime)      │                  │
│  │  • systimer        │  │  • virtio_net          │                  │
│  │  • wdt  (C3/C6/H2) │  │                       │                  │
│  │  • wifi             │  │                       │                  │
│  │  • mem  (map const) │  │                       │                  │
│  └────────────────────┘  └──────────────────────┘                   │
└──────────────────────────┬──────────────────────────────────────────┘
                           │ used by
┌──────────────────────────▼──────────────────────────────────────────┐
│  Layer 2 — Kernel binaries (one per board)                          │
│  ┌─────────────────────────┐  ┌─────────────────────────┐          │
│  │ kernel-xiao-esp32c6     │  │ kernel-qemu-virt         │          │
│  │ • _start assembly       │  │ • _start assembly        │          │
│  │ • watchdog disable      │  │ • CLINT timer setup      │          │
│  │ • scheduler + tasks     │  │ • scheduler + tasks      │          │
│  │ • shell (UART)          │  │ • shell (UART + TCP)     │          │
│  │ • linker: xiao-esp32c6.x│  │ • linker: qemu-virt.x   │          │
│  └─────────────────────────┘  └─────────────────────────┘          │
└─────────────────────────────────────────────────────────────────────┘

  Shared OS components (used by all kernels):
    microkernel  — Scheduler, PoolAllocator, Heap, DriverRegistry
    shell        — Interactive command shell (ShellEnv callbacks)
    net          — smoltcp integration, TcpSerial, auth
```

### Adding a new board

1. **Same SoC family** — add a new kernel crate under `crates/kernel/`,
   depend on the existing SoC crate, provide a board-specific linker script.

2. **New SoC** — create a SoC crate under `crates/soc/` implementing the
   `arch` traits, then add the kernel crate.

---

## Workspace layout

```
crates/
  arch/                     # Layer 0: architecture-neutral traits
  soc/
    esp32/                  # Layer 1: ESP32 RISC-V family (C3/C6/H2 via features)
    qemu_virt/              # Layer 1: QEMU virt RISC-V 32-bit machine
  kernel/
    xiao_esp32c6/           # Layer 2: kernel for Seeed XIAO ESP32-C6
    qemu_virt/              # Layer 2: kernel for QEMU virt
  microkernel/              # Shared: scheduler, allocator, driver registry
  shell/                    # Shared: interactive shell
  net/                      # Shared: smoltcp networking, TCP serial, auth
  distributions/            # Build profiles (minimal, app, rt, full)
```

---

## ESP32 variant support

The `soc-esp32` crate uses Cargo features to handle variant differences:

| Feature | SoC       | SRAM   | Address space            | Target triple              |
|---------|-----------|--------|--------------------------|----------------------------|
| `c3`    | ESP32-C3  | 400 KB | Split IRAM/DRAM          | riscv32imc-unknown-none-elf |
| `c6`    | ESP32-C6  | 512 KB | Unified @ 0x4080_0000    | riscv32imc-unknown-none-elf |
| `h2`    | ESP32-H2  | 320 KB | Unified @ 0x4080_0000    | riscv32imc-unknown-none-elf |

Shared peripherals (UART0, SYSTIMER, INTC) use the same base addresses.
Variant-specific code (WDT registers, memory map constants) is `#[cfg]`-gated.

---

## Boot sequence (ESP32-C6)

1. ROM bootloader loads ELF from SPI flash into HP SRAM at `0x4080_0000`
2. `_start` (assembly): disable interrupts, set stack pointer, zero BSS
3. `_rust_start`: disable watchdogs, init console, init heap
4. Register drivers (UART, INTC, SYSTIMER)
5. Install trap vector, configure interrupt controller
6. Start SYSTIMER tick (1 ms)
7. Create tasks: idle (priority 0), shell (priority 1)
8. `_veer_start_first_task` → `mret` into first runnable task

## Boot sequence (QEMU virt)

Same flow, but uses CLINT timer instead of SYSTIMER, and adds a network
listener task for remote TCP shell over VIRTIO-NET.

---

## Key Design Principles

- **Portability** — kernel logic depends only on `arch` traits, never on
  register addresses.
- **Extensibility** — new SoC = new crate implementing traits; new board =
  new kernel crate with linker script.
- **Minimalism** — each layer does one thing; no unnecessary abstractions.

- **Networking-First**  
  `smoltcp` integrated directly into the kernel.

- **Security**  
  Strong memory isolation using PMP or MPU.

- **Future-Proof**  
  Designed to scale from microcontrollers to multicore systems.

---

## Summary

VeerOS provides a **clean, modern OS architecture** that combines:
- Rust safety
- Strong abstraction boundaries
- First-class networking
- Long-term scalability

This makes VeerOS suitable for **embedded devices, IoT platforms, and future edge systems**.

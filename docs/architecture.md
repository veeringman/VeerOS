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

## Process Model (Current → Planned)

### Current (v0.1): Flat Task Model

```
┌───────────────────────────────────────────────────────┐
│  Single address space (M-mode, no MMU/PMP)            │
│                                                       │
│  ┌────────┐ ┌────────┐ ┌────────┐ ┌────────┐         │
│  │ idle   │ │ shell  │ │ net    │ │ user   │ ...×16  │
│  │ task 0 │ │ task 1 │ │ task 2 │ │ task 3 │         │
│  └────────┘ └────────┘ └────────┘ └────────┘         │
│  Fixed TCB table [Tcb; 16], round-robin / priority    │
│  Single-slot IPC mailbox, no locks, no processes      │
└───────────────────────────────────────────────────────┘
```

- Tasks are flat: no parent-child, no address space isolation
- `TaskContext` is RISC-V 32-GPR specific
- Sleep uses `gpr[0]` overload instead of a proper field
- IPC is fire-and-forget single-slot (overwrites on full)

### Planned (v0.2+): Process + Thread + Multi-Arch

```
┌──────────────────────────────────────────────────────────────────┐
│  Kernel (privileged: M-mode / EL1 / Ring 0)                     │
│  ┌──────────────────────────────────────────────────────────┐    │
│  │ Scheduler │ IPC queues │ Futex table │ Socket layer      │    │
│  └──────────────────────────────────────────────────────────┘    │
├──────────────────────────────────────────────────────────────────┤
│  Process A (ASID 1)              │  Process B (ASID 2)          │
│  ┌────────┐ ┌────────┐          │  ┌────────┐                  │
│  │ Thread 0│ │ Thread 1│          │  │ Thread 0│                  │
│  │ (main) │ │ (worker)│          │  │ (main) │                  │
│  └────────┘ └────────┘          │  └────────┘                  │
│  Shared: heap, .text, .rodata   │  Own: heap, .text, .rodata   │
│  PMP/MPU/page-table isolated    │  PMP/MPU/page-table isolated │
└──────────────────────────────────────────────────────────────────┘
```

Key changes:
- **`SavedContext` trait** — per-arch context type (riscv32, riscv64, aarch64, x86-64)
- **Process owns address space** — ASID, memory regions, handle table, child list
- **Thread runs within process** — own stack + context, shares process memory
- **BlockReason enum** — typed blocking: `Sleep`, `IpcRecv`, `MutexWait`, `Join`, `IoWait`
- **Message queues** — bounded ring buffers replace single-slot mailboxes
- **Futex-based synchronization** — Mutex, Condvar, Semaphore, RwLock in userlib
- **Socket API** — unified local + network sockets with handle table
- **Async executor** — `no_std` poll-based runtime in userlib using `SYS_POLL_WAIT`

### Multi-Architecture `SavedContext`

```rust
// In arch crate — trait that each arch implements:
pub trait SavedContext: Copy + Sized {
    fn zero() -> Self;
    fn set_pc(&mut self, pc: usize);
    fn set_sp(&mut self, sp: usize);
    fn set_arg(&mut self, n: usize, val: usize);  // syscall args
    fn get_ret(&self, n: usize) -> usize;          // return values
    fn pc(&self) -> usize;
    fn sp(&self) -> usize;
    fn syscall_nr(&self) -> usize;
}

// Per-arch implementations:
//   riscv32: 32 GPRs (x0-x31) + pc + mstatus      = 136 bytes
//   riscv64: 32 GPRs (x0-x31) + pc + sstatus       = 272 bytes
//   aarch64: 31 GPRs (x0-x30) + SP + PC + PSTATE   = 272 bytes
//   x86-64:  16 GPRs + RIP + RFLAGS + segments      = 176 bytes
```

## Summary

VeerOS provides a **clean, modern OS architecture** that combines:
- Rust safety
- Strong abstraction boundaries
- First-class networking
- Long-term scalability

This makes VeerOS suitable for **embedded devices, IoT platforms, and future edge systems**.

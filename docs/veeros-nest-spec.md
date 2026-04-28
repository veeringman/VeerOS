# veeros-nest — Lightweight Sandbox Runtime for VeerOS

**Status:** Draft Specification  
**Date:** April 2026  
**Author:** VeerOS Team

> Note: `veeros-nest` is an experimental design track.
> Production runtime path remains `fold_engine` + `veer-vm`.

---

## 1. Problem

QEMU system emulation of VeerOS (riscv32 target) consumes 100% CPU even
when the kernel is idle. This makes development painful: slow startup,
fans spinning, laptop battery drain, and inability to run more than a
couple of instances for mesh/fabric testing.

We need a lightweight, state-of-the-art sandbox runtime that can replace
QEMU for most development workflows while preserving the ability to test
real riscv32 binaries when needed.

---

## 2. Design Principles

- **Zero-overhead idle.** When VeerOS has no work, the host thread sleeps.
  CPU usage should be <1% at idle.
- **Instant startup.** Target <100ms from command to shell-ready (native)
  or <1s (interpreted).
- **Fleet-scale.** Spin up 10–100 instances on a single developer machine
  for distributed fabric and mesh testing.
- **Pluggable backends.** One CLI, multiple isolation strategies, selected
  per workload.
- **Reuse everything.** Shell, crypto, net, microkernel, and secure channel
  code must run unmodified across all backends.

---

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│  veeros-nest CLI                                             │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌─────────────┐ │
│  │  native   │  │  rv32    │  │  qemu    │  │  (future)   │ │
│  │  backend  │  │  backend │  │  backend │  │  wasm / sfi │ │
│  └─────┬────┘  └─────┬────┘  └─────┬────┘  └──────┬──────┘ │
│        │             │             │               │        │
│  ┌─────▼─────────────▼─────────────▼───────────────▼──────┐ │
│  │  NestBackend trait                                      │ │
│  │  fn start(config) → Instance                            │ │
│  │  fn stop(instance)                                      │ │
│  │  fn status(instance) → State                            │ │
│  └────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Backend Specifications

### 4.1 Tier 1 — `native` (Default, highest priority)

**Concept:** Compile VeerOS as a native Linux/macOS process. Replace the
hardware abstraction layer (HAL) with host OS primitives.

**HAL mapping:**

| VeerOS subsystem | Hardware target | native backend |
|---|---|---|
| UART / Serial | MMIO registers | PTY pair or stdio |
| TCP/IP (smoltcp) | virtio-net NIC | `std::net::TcpListener` |
| Timers / ticks | Hardware timer IRQ | `timerfd` / `timer_create` |
| GPIO / I2C / SPI | MMIO peripherals | Simulated / logged to stderr |
| Allocator | Bare-metal pool | `mmap` / system allocator |
| Interrupts | Hardware IRQ | `epoll` + `eventfd` |
| Scheduler | Cooperative (riscv32) | Native threads or `async` |

**Implementation plan:**

1. Add a new kernel target: `crates/kernel/native/`
2. Add a new SoC crate: `crates/soc/native/` implementing `arch` traits
3. `Serial` trait → wraps `std::io::Stdin`/`Stdout` or PTY fd
4. `TcpSerial` → wraps `std::net::TcpStream` directly (no smoltcp)
5. Network listener → `std::net::TcpListener::bind("0.0.0.0:2323")`
6. Timer → `std::time::Instant` for uptime, `timerfd` for periodic ticks
7. VFS → pass-through to a host directory (e.g. `~/.veeros-nest/<name>/fs/`)

**Advantages:**
- Native CPU speed, no emulation overhead
- Instant startup (<10ms)
- Full debugger support (gdb, lldb, valgrind, ASAN, miri)
- Runs 100+ instances per laptop
- Easy CI/CD integration

**Limitations:**
- Does not catch riscv32-specific compiler bugs (e.g. the LTO/crypto issue)
- No real interrupt or peripheral testing
- Architecture-dependent code paths skipped

**Estimated effort:** ~500 lines of platform glue.

---

### 4.2 Tier 2 — `rv32` (Binary-fidelity testing)

**Concept:** A minimal, custom riscv32imc instruction interpreter written
in Rust. Not a full system emulator — just enough to run the kernel ELF.

**Core components:**

| Component | Description | Est. lines |
|---|---|---|
| Decoder | RV32IMC instruction decode | ~400 |
| Executor | Register file + ALU + branch | ~600 |
| Memory | Flat address space + MMIO traps | ~200 |
| MMIO handlers | UART, timer, virtio-net stubs | ~400 |
| ELF loader | Parse ELF, load segments | ~200 |
| Host bridge | Map MMIO to host I/O (PTY, sockets) | ~300 |
| **Total** | | **~2100** |

**Key design decisions:**

- **WFI-aware:** When the kernel executes `wfi`, the interpreter thread
  calls `epoll_wait()` with a timeout. CPU drops to zero at idle. This is
  the single most important feature vs QEMU.
- **MMIO trap regions:** Writes to known MMIO addresses (UART TX, timer
  reload, virtio registers) are intercepted and routed to host handlers.
  All other memory is a flat `Vec<u8>`.
- **No MMU:** riscv32imc has no virtual memory. Flat physical addressing.
- **No device models:** Peripherals like virtio-net are stubbed at the
  MMIO level, not emulated as real devices. Network packets go to a host
  TAP/socket bridge.
- **Cycle-approximate (optional):** Can count instructions for profiling,
  but doesn't model pipeline/cache.

**Instruction set coverage:**

```
RV32I base:  LUI AUIPC JAL JALR BEQ BNE BLT BGE BLTU BGEU
             LB LH LW LBU LHU SB SH SW
             ADDI SLTI SLTIU XORI ORI ANDI SLLI SRLI SRAI
             ADD SUB SLL SLT SLTU XOR SRL SRA OR AND
             FENCE ECALL EBREAK
             CSR: CSRRW CSRRS CSRRC CSRRWI CSRRSI CSRRCI

RV32M:       MUL MULH MULHSU MULHU DIV DIVU REM REMU

RV32C:       All 16-bit compressed forms (C.LW, C.SW, C.ADDI, C.JAL,
             C.BEQZ, C.BNEZ, C.LI, C.LUI, C.MV, C.ADD, C.NOP, etc.)
```

**CSR support (minimal):**

| CSR | Address | Behavior |
|---|---|---|
| `mstatus` | 0x300 | Read/write, MIE bit controls interrupt gate |
| `mie` | 0x304 | Interrupt enable mask |
| `mtvec` | 0x305 | Trap vector base |
| `mepc` | 0x341 | Exception PC |
| `mcause` | 0x342 | Trap cause |
| `mtval` | 0x343 | Trap value |
| `mip` | 0x344 | Pending interrupts (read-only, set by host) |
| `cycle` | 0xC00 | Instruction counter (lo) |
| `cycleh` | 0xC80 | Instruction counter (hi) |

**MMIO map (matching QEMU virt):**

| Address | Size | Device |
|---|---|---|
| `0x1000_0000` | 8 bytes | UART (NS16550-like, TX/RX only) |
| `0x0200_0000` | 16K | CLINT (mtime, mtimecmp, msip) |
| `0x1000_1000` | 4K | virtio-net (stubbed) |

**Host I/O bridge:**

```
UART TX byte → write(pty_fd, &byte, 1)
UART RX      → non-blocking read(pty_fd, ...) via epoll
Timer IRQ    → host timerfd fires → set mip.MTIP → resume from wfi
Network      → host TUN/TAP or socket bridge
```

**Advantages:**
- Runs the *actual* riscv32 ELF (catches real compiler bugs)
- WFI → host sleep (zero CPU at idle)
- Sub-2ms startup
- Full control: fault injection, coverage, breakpoints
- ~2K lines — easy to maintain and audit

**Limitations:**
- ~10-50x slower than native (interpreted)
- No real peripheral fidelity
- Must be updated if new CSRs or MMIO regions are added

**Estimated effort:** ~2000-2500 lines of Rust.

---

### 4.3 Tier 3 — `qemu` (Full-fidelity, existing)

**Concept:** Keep QEMU for full system emulation when needed. Improve the
existing integration.

**Improvements:**

1. **Fix idle CPU burn.** Add `wfi` instruction to the kernel's idle loop
   so QEMU's TCG can halt the vCPU:
   ```rust
   // In scheduler idle path:
   #[cfg(target_arch = "riscv32")]
   unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
   ```

2. **Snapshot support.** Use QEMU savevm/loadvm to create checkpoints
   after boot+DHCP, reducing startup from 20s to <1s for repeated tests.

3. **Headless mode.** Suppress QEMU GUI, redirect serial to stdio/PTY.

4. **Memory tuning.** Reduce default RAM from 128M to 32M (VeerOS uses <1M).

**When to use:** Full peripheral testing, hardware-accurate timing, testing
interrupt handling, validating linker scripts and boot sequence.

---

### 4.4 Tier 4 — Future: WASM / Rust SFI (Research)

**Concept:** Explore post-MVP isolation strategies for specific workloads.

**4.4.1 WASM sandbox**

For running *user-space* VeerOS applications or plugins (not the kernel
itself) in a WebAssembly sandbox. Requires defining a stable syscall ABI
that can be compiled to both riscv32 (real) and wasm32 (sandbox).

**4.4.2 Rust SFI (Software Fault Isolation)**

Compile user code with bounds-checking instrumentation for memory safety
without hardware MMU. Could run natively on the host with near-zero
overhead. Research area — no immediate implementation planned.

---

## 5. CLI Interface

```
veeros-nest — VeerOS Sandbox Runtime

USAGE:
    veeros-nest <COMMAND> [OPTIONS]

COMMANDS:
    run         Run a VeerOS instance
    start       Start a VeerOS instance in the background (daemon)
    stop        Stop a running instance
    kill        Force-stop an instance
    status      Show instance status
    list        List all instances
    fleet       Start multiple instances for mesh testing
    log         Show instance serial output
    attach      Attach to instance console (interactive)
    snapshot    Save/restore instance state (qemu backend only)

OPTIONS:
    --backend <native|rv32|qemu>    Isolation backend [default: native]
    --name <NAME>                   Instance name [default: auto]
    --port <PORT>                   VSC shell port [default: 2323]
    --elf <PATH>                    Kernel ELF (rv32/qemu backends)
    --memory <MB>                   RAM size in MB (qemu backend)
    --fs <PATH>                     Host directory for VFS passthrough
    --verbose                       Verbose logging
```

**Examples:**

```bash
# Quick development — instant start, native speed
veeros-nest run

# Test the real riscv32 binary
veeros-nest run --backend rv32 --elf target/riscv32imc-unknown-none-elf/release/kernel-qemu-esp32c6

# Full-fidelity QEMU test
veeros-nest run --backend qemu --elf target/riscv32imc-unknown-none-elf/release/kernel-qemu-esp32c6

# Spin up 10 instances for distributed testing
veeros-nest fleet --count 10 --backend native --port-base 3000

# Attach to a running instance
veeros-nest attach myiot1

# Background daemon mode
veeros-nest start --name myiot1 --backend native
veeros-nest stop myiot1
```

---

## 6. Implementation Roadmap

| Phase | Backend | Deliverable | Priority |
|---|---|---|---|
| 1 | `native` | Kernel target + SoC crate + CLI wrapper | **High** |
| 2 | `qemu` fix | Add `wfi` to idle loop, reduce CPU burn | **High** |
| 3 | `rv32` | Custom RV32IMC interpreter + MMIO bridge | Medium |
| 4 | CLI | Unified `veeros-nest` binary | Medium |
| 5 | Fleet | Multi-instance orchestration | Low |
| 6 | WASM/SFI | Research / prototype | Low |

**Phase 1 delivers the highest ROI:** native-speed development with
instant startup and zero idle CPU, using 100% of the existing VeerOS
shell/crypto/net/microkernel code.

**Phase 2 is a quick win:** a single `wfi` instruction in the scheduler
idle path eliminates the QEMU CPU burn immediately, even before the
native backend is ready.

---

## 7. Crate Structure

```
crates/
  veeros-nest/              # CLI binary + backend trait
    Cargo.toml
    src/
      main.rs               # CLI entry point (clap)
      backend.rs            # NestBackend trait
      native.rs             # native backend
      rv32/
        mod.rs              # interpreter entry
        decode.rs           # instruction decoder
        exec.rs             # executor
        memory.rs           # address space + MMIO
        csr.rs              # CSR file
      qemu.rs               # QEMU backend (wraps scripts/veeros-vm)
  soc/
    native/                 # Host-native SoC implementing arch traits
      Cargo.toml
      src/
        lib.rs
        serial.rs           # PTY / stdio Serial impl
        timer.rs            # timerfd tick source
        net.rs              # std::net NetworkDevice impl
  kernel/
    native/                 # Native kernel binary (links soc-native)
      Cargo.toml
      src/
        main.rs             # fn main() — no _start, no linker script
```

---

## 8. NestBackend Trait

```rust
pub trait NestBackend {
    type Instance;
    type Error: std::fmt::Display;

    /// Start a new VeerOS instance with the given configuration.
    fn start(&self, config: &NestConfig) -> Result<Self::Instance, Self::Error>;

    /// Stop a running instance gracefully.
    fn stop(&self, instance: &mut Self::Instance) -> Result<(), Self::Error>;

    /// Force-kill an instance.
    fn kill(&self, instance: &mut Self::Instance) -> Result<(), Self::Error>;

    /// Check instance status.
    fn status(&self, instance: &Self::Instance) -> InstanceState;

    /// Attach to instance console (returns a bidirectional stream).
    fn attach(&self, instance: &Self::Instance) -> Result<Box<dyn Console>, Self::Error>;
}

pub struct NestConfig {
    pub name: String,
    pub backend: BackendType,
    pub port: u16,
    pub elf_path: Option<PathBuf>,
    pub fs_root: Option<PathBuf>,
    pub memory_mb: u32,
    pub verbose: bool,
}

pub enum BackendType {
    Native,
    Rv32,
    Qemu,
}

pub enum InstanceState {
    Starting,
    Running,
    Idle,
    Stopped,
    Error(String),
}
```

---

## 9. Security Considerations

- **Native backend:** Runs as a regular user process. No elevated
  privileges required. VFS passthrough is scoped to a single directory.
- **RV32 backend:** Memory is a flat `Vec<u8>` — no host memory access
  possible from interpreted code. MMIO writes are validated before
  dispatch.
- **QEMU backend:** Existing QEMU isolation applies.
- **Fleet mode:** Each instance gets a unique port and isolated filesystem
  directory. No shared mutable state between instances.

---

## 10. Testing Strategy

- **Unit tests:** Instruction decoder and executor for rv32 backend.
- **Integration tests:** Boot VeerOS, connect via veer-connect, run shell
  commands, verify output.
- **Parity tests:** Run the same test suite across all three backends and
  compare output. Ensures native and rv32 match QEMU behavior.
- **Performance benchmarks:** Startup time, idle CPU, throughput for each
  backend.

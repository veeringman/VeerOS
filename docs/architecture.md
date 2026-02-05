# VeerOS Architecture

VeerOS is a **Rust-native, multi-architecture operating system** designed for portability, security, and developer ergonomics.  
It cleanly separates **policy from mechanism**, allowing the same kernel core to run across RISC-V, ARM, and future architectures.

---

## High-Level Architecture Overview

The diagram below illustrates the layered design of VeerOS, from user programs down to physical hardware.

![VeerOS Architecture Diagram](architecture.png)

> 📌 The kernel core is fully portable Rust code.  
> All architecture- and chip-specific logic is isolated behind well-defined abstraction layers.

---

## User Space

### User Programs

VeerOS applications are written as **normal Rust programs**, without exposing low-level OS details.

- Entry point defined using `#[veer_entry]`
- Familiar APIs: `println!`, `TcpListener`, `spawn_thread`
- Fully portable across CPU architectures

Developers never interact with registers, traps, or linker scripts.

---

### Userlib Layer (`veeros_userlib`)

The user library provides **ergonomic Rust APIs** built on top of syscalls.

- Syscall wrappers:
  - `write()`, `exit()`, `yield()`, `socket()`
- Idiomatic Rust interfaces:
  - `println!`, `TcpStream`, `TcpListener`
- Hides:
  - `no_std`, `no_main`
  - `extern "C"` boilerplate

This layer delivers a clean developer experience without sacrificing control.

---

## Kernel Boundary

### Syscall Interface

The syscall layer defines a **portable ABI** between user space and the kernel.

- Trap handler dispatches by syscall number
- ABI convention:
  - Syscall number → `a7`
  - Arguments → `a0..a6`
  - Return value → `a0`
- Unified syscall table:
  - `write`, `exit`, `yield`, `socket`, etc.

This is the **only layer aware of calling conventions**.

---

## Kernel Space

### Kernel Core

The kernel core contains **pure Rust logic**, independent of architecture or hardware.

- Scheduler:
  - Round-robin
  - Priority-based
  - Preemptive
- Process & thread model:
  - TCBs
  - Kernel/user stacks
  - Execution states
- Memory management:
  - Allocators
  - PMP (RISC-V) / MPU (ARM) isolation
- Networking:
  - `smoltcp` integrated at kernel level
- Filesystem (planned):
  - Virtual filesystem abstraction

The kernel core depends **only on traits**, never on registers or peripherals.

---

### Architecture Abstraction Layer (AAL)

The AAL isolates **CPU-specific behavior** behind Rust traits.

- Defines interfaces such as:
  - `Arch::init_traps()`
  - `Arch::context_switch()`
- Architecture implementations:
  - **RISC-V**: CSRs, `mtvec`, PMP
  - **ARM**: exception vectors, MPU
- Future targets:
  - x86
  - MIPS
  - SMP architectures

Adding a new architecture means implementing the AAL — no kernel rewrite required.

---

### Hardware Abstraction Layer (HAL)

HAL abstracts **peripherals and SoC functionality**.

- Peripheral traits:
  - `Timer::start()`
  - `Uart::write_byte()`
  - `Wifi::send_frame()`
- Chip-specific HAL crates:
  - `hal_esp32c3`
  - `hal_stm32`
  - `hal_rp2040`
- Kernel interacts only with HAL traits

This guarantees clean separation between kernel logic and hardware details.

---

### Hardware / SoC Drivers

Driver implementations live in chip-specific crates.

- ESP32-C3:
  - Wi-Fi radio
  - Packet buffers
- STM32:
  - Ethernet MAC
- RP2040:
  - USB controller

Each driver implements HAL traits and remains isolated from the kernel core.

---

## Physical Hardware

VeerOS currently targets and scales across:

- **ESP32-C3** (RISC-V, Wi-Fi)
- **STM32** (ARM Cortex-M, Ethernet)
- **RP2040** (ARM Cortex-M0+, USB)
- Future platforms:
  - ARMv8
  - RISC-V SMP
  - Advanced SoCs

---

## Key Design Principles

- **Portability**  
  Kernel core is pure Rust and depends only on HAL/AAL traits.

- **Extensibility**  
  New chips via HAL crates, new CPUs via AAL crates.

- **Developer Experience**  
  Write normal Rust programs with `#[veer_entry]`.

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

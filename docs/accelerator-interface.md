# VeerOS Unified Accelerator Interface (UAI)

This document defines a single kernel-facing contract for:

- Co-processors
- Hardware accelerators
- GPUs (compute / GPGPU)
- FPGAs
- Quantum processors (QPU / qubits)

The goal is to make heterogeneous compute look like one scheduling and security model, independent of vendor protocol or board.

## Design Goals

1. One control-plane ABI for all accelerator classes
2. Works on ARM64 and x86_64 hosts
3. No dynamic allocation required in kernel core
4. Capability-gated access per process
5. Async submission model with completion polling/cancel

## Core Model

The microkernel module [crates/microkernel/src/accelerator.rs](../crates/microkernel/src/accelerator.rs) defines:

- `AcceleratorClass`: `Coprocessor`, `Accelerator`, `Fpga`, `Quantum`, `Gpu`
- `AcceleratorBus`: MMIO / PCIe / CXL / Virtio / SPI / I2C / shared memory
- `AcceleratorCapabilities`: DMA, queue count, max transfer, preemption, bitstream, quantum limits, GPU compute flag
- `QuantumInfo`: model + fidelity/lifetime metrics
- `GpuInfo`: compute units, VRAM, workgroup size, unified memory, hw queues
- `WorkDescriptor`: generic job descriptor (`input`, `output`, `opcode`, `queue`, `requested_qubits`)
- `WorkloadClass`: Vector, Matrix, Signal, Crypto, BitstreamProgram, QuantumCircuit, QuantumSampling, `GpuComputeShader`, `GpuRenderCompute`, Vendor
- `CompletionRecord`: async completion state and result metadata
- `AcceleratorRuntime` trait: submit / poll / cancel / fence
- `AcceleratorRegistry`: fixed-size endpoint table

## Architecture Targeting

### x86_64

- Baseline context type is provided in [crates/arch/src/x86_64.rs](../crates/arch/src/x86_64.rs)
- Intended transport profile: PCIe, CXL, and virtio accelerators
- QPU endpoints are expected to be local cards or remote proxies exposed through kernel network abstractions

### ARM64

- Existing ARM64 context remains in [crates/arch/src/aarch64.rs](../crates/arch/src/aarch64.rs)
- Intended transport profile: MMIO/NOC accelerators, PCIe endpoints on SBC/server SoCs, FPGA fabrics

## Syscall ABI Reservation

The following syscall range is reserved in [crates/microkernel/src/syscall.rs](../crates/microkernel/src/syscall.rs):

- `0xB0` `SYS_ACCEL_COUNT`
- `0xB1` `SYS_ACCEL_INFO`
- `0xB2` `SYS_ACCEL_SUBMIT`
- `0xB3` `SYS_ACCEL_POLL`
- `0xB4` `SYS_ACCEL_CANCEL`
- `0xB5` `SYS_FPGA_PROGRAM`
- `0xB6` `SYS_QPU_SUBMIT`

Dispatcher stubs are already capability-gated and ABI-stable in [crates/microkernel/src/dispatch.rs](../crates/microkernel/src/dispatch.rs).

## GPU Dual-Role Model

GPUs serve two fundamentally different roles:

- **Compute (GPGPU)** — Shader dispatch, matrix ops, AI inference, signal processing.
  Modelled as `AcceleratorClass::Gpu` within the UAI.  Drivers submit compute
  workloads through the same `submit / poll / cancel` path used by every other
  accelerator class.  GPU-specific metadata lives in the `GpuInfo` struct
  (compute units, VRAM, workgroup size, unified-memory flag, hardware queues).
  Two dedicated workload types — `GpuComputeShader` and `GpuRenderCompute` —
  distinguish pure compute from render-pipeline compute.

- **Display output** — Framebuffer, scanout, pixel operations.
  Remains on the separate `arch::DisplayDevice` trait (`set_pixel`, `fill_rect`,
  `flush`).  Display has nothing to do with job submission; it is a continuous
  output device with its own refresh-rate and vsync semantics.

A single physical GPU chip can register both:
1. An `AcceleratorDevice` with `class: Gpu` for compute.
2. A `DisplayDevice` implementation for scanout.

This keeps the UAI focused on **scheduled work** while display stays in the
platform's continuous-output abstraction.

## Security and Isolation

- Process capability bit `ACCEL` controls access to this interface
- Device MMIO/queue regions are explicitly registered
- DMA coherency expectation is device-declared (`Coherent` vs `NonCoherent`)
- Driver implementations should fence and flush before/after shared-buffer ownership changes

## Next Integration Steps

1. Instantiate `AcceleratorRegistry` in each kernel BSP
2. Register concrete drivers (GPU/NPU/FPGA/QPU)
3. Wire syscall handlers to active `AcceleratorRuntime` instances
4. Add userlib wrappers and typed command descriptors
5. Add scheduler accounting for accelerator queue pressure and timeouts

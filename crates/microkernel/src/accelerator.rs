//! Unified co-processor / accelerator / GPU / FPGA / quantum interface layer.
//!
//! This module provides a hardware-neutral contract that lets VeerOS expose
//! heterogeneous compute devices through one kernel API surface.
//! Drivers can map vendor-specific protocols to these descriptors while
//! scheduler, capability checks, and future syscall handlers remain generic.
//!
//! GPUs register as `AcceleratorClass::Gpu` and use the same submit/poll/cancel
//! model for compute workloads (shaders, matrix ops, AI inference). Display and
//! framebuffer output remain on the separate `arch::DisplayDevice` trait.

use crate::driver::MemRegion;

/// Maximum number of accelerator-class devices tracked by the kernel.
pub const MAX_ACCEL_DEVICES: usize = 16;

/// Maximum number of queue pairs exposed per device.
pub const MAX_ACCEL_QUEUES: usize = 8;

/// Device class covered by the unified accelerator model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AcceleratorClass {
    Coprocessor = 0,
    Accelerator = 1,
    Fpga = 2,
    Quantum = 3,
    Gpu = 4,
}

/// Interconnect or control-path used to reach a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AcceleratorBus {
    Mmio = 0,
    Pcie = 1,
    Cxl = 2,
    Virtio = 3,
    Spi = 4,
    I2c = 5,
    SharedMemory = 6,
    Vendor = 255,
}

/// Device execution transport model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AcceleratorTransport {
    DoorbellQueue = 0,
    Mailbox = 1,
    RingBuffer = 2,
    RegisterCommand = 3,
}

/// Coherence semantics for host/device shared buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DmaCoherency {
    Coherent = 0,
    NonCoherent = 1,
}

/// Quantum execution model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum QuantumModel {
    Gate = 0,
    Annealing = 1,
    Analog = 2,
    Simulator = 3,
}

/// Generic accelerator capabilities, including quantum and GPU limits.
#[derive(Debug, Clone, Copy)]
pub struct AcceleratorCapabilities {
    pub max_queues: u8,
    pub max_transfer_bytes: usize,
    pub dma_coherency: DmaCoherency,
    pub supports_preemption: bool,
    pub supports_sriov: bool,
    pub supports_bitstream_reconfig: bool,
    pub supports_quantum: bool,
    pub max_physical_qubits: u16,
    pub max_logical_qubits: u16,
    pub supports_gpu_compute: bool,
}

impl AcceleratorCapabilities {
    pub const fn minimal() -> Self {
        Self {
            max_queues: 1,
            max_transfer_bytes: 4096,
            dma_coherency: DmaCoherency::NonCoherent,
            supports_preemption: false,
            supports_sriov: false,
            supports_bitstream_reconfig: false,
            supports_quantum: false,
            max_physical_qubits: 0,
            max_logical_qubits: 0,
            supports_gpu_compute: false,
        }
    }
}

/// Optional quantum-specific information.
#[derive(Debug, Clone, Copy)]
pub struct QuantumInfo {
    pub model: QuantumModel,
    pub t1_ns: u32,
    pub t2_ns: u32,
    pub gate_error_ppm: u32,
    pub readout_error_ppm: u32,
}

impl QuantumInfo {
    pub const fn none() -> Self {
        Self {
            model: QuantumModel::Simulator,
            t1_ns: 0,
            t2_ns: 0,
            gate_error_ppm: 0,
            readout_error_ppm: 0,
        }
    }
}

/// Optional GPU-specific information.
#[derive(Debug, Clone, Copy)]
pub struct GpuInfo {
    /// Number of compute units / streaming multiprocessors.
    pub compute_units: u16,
    /// Dedicated VRAM in bytes (0 if unified memory).
    pub vram_bytes: usize,
    /// Maximum workgroup / thread-block size.
    pub max_workgroup_size: u32,
    /// Maximum concurrent workgroups.
    pub max_dispatch_x: u32,
    /// Whether the GPU shares host memory (UMA / iGPU).
    pub unified_memory: bool,
    /// Number of hardware queues (compute, transfer, etc.).
    pub hw_queue_count: u8,
}

impl GpuInfo {
    pub const fn none() -> Self {
        Self {
            compute_units: 0,
            vram_bytes: 0,
            max_workgroup_size: 0,
            max_dispatch_x: 0,
            unified_memory: false,
            hw_queue_count: 0,
        }
    }
}

/// Kernel-visible descriptor for one device endpoint.
#[derive(Debug, Clone, Copy)]
pub struct AcceleratorDevice {
    pub id: u16,
    pub class: AcceleratorClass,
    pub bus: AcceleratorBus,
    pub transport: AcceleratorTransport,
    pub irq_line: Option<u16>,
    pub control_region: Option<MemRegion>,
    pub queue_region: Option<MemRegion>,
    pub caps: AcceleratorCapabilities,
    pub quantum: Option<QuantumInfo>,
    pub gpu: Option<GpuInfo>,
}

impl AcceleratorDevice {
    pub const fn empty() -> Self {
        Self {
            id: u16::MAX,
            class: AcceleratorClass::Accelerator,
            bus: AcceleratorBus::Vendor,
            transport: AcceleratorTransport::RegisterCommand,
            irq_line: None,
            control_region: None,
            queue_region: None,
            caps: AcceleratorCapabilities::minimal(),
            quantum: None,
            gpu: None,
        }
    }
}

/// High-level work class requested by userland or in-kernel clients.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WorkloadClass {
    Vector = 0,
    Matrix = 1,
    Signal = 2,
    Crypto = 3,
    BitstreamProgram = 4,
    QuantumCircuit = 5,
    QuantumSampling = 6,
    GpuComputeShader = 7,
    GpuRenderCompute = 8,
    Vendor = 255,
}

/// Generic command descriptor passed to accelerator drivers.
#[derive(Debug, Clone, Copy)]
pub struct WorkDescriptor {
    pub queue: u8,
    pub workload: WorkloadClass,
    pub opcode: u16,
    pub flags: u16,
    pub in_addr: usize,
    pub in_len: usize,
    pub out_addr: usize,
    pub out_len: usize,
    pub requested_qubits: u16,
    pub deadline_tick: u64,
}

/// Completion state for asynchronous accelerator work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CompletionState {
    Pending = 0,
    Running = 1,
    Done = 2,
    Failed = 3,
    Cancelled = 4,
    Timeout = 5,
}

/// Completion record returned by polling interfaces.
#[derive(Debug, Clone, Copy)]
pub struct CompletionRecord {
    pub token: u64,
    pub state: CompletionState,
    pub status_code: i32,
    pub bytes_written: usize,
}

/// Errors for the accelerator control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcceleratorError {
    RegistryFull,
    InvalidDevice,
    QueueOutOfRange,
    Unsupported,
    Busy,
    InvalidDescriptor,
}

/// Trait implemented by concrete accelerator drivers.
pub trait AcceleratorRuntime {
    /// Return static metadata for this accelerator endpoint.
    fn device(&self) -> AcceleratorDevice;

    /// Submit a work item and return an opaque completion token.
    fn submit(&mut self, work: &WorkDescriptor) -> Result<u64, AcceleratorError>;

    /// Poll completion status by token.
    fn poll(&mut self, token: u64) -> CompletionRecord;

    /// Best-effort cancellation for long-running jobs.
    fn cancel(&mut self, _token: u64) -> Result<(), AcceleratorError> {
        Err(AcceleratorError::Unsupported)
    }

    /// Driver hook for cache / ordering synchronization.
    fn fence(&self) {}
}

/// Fixed-size kernel registry for accelerator endpoints.
pub struct AcceleratorRegistry {
    devices: [Option<AcceleratorDevice>; MAX_ACCEL_DEVICES],
    len: usize,
}

impl AcceleratorRegistry {
    pub const fn new() -> Self {
        Self {
            devices: [None; MAX_ACCEL_DEVICES],
            len: 0,
        }
    }

    pub fn register(&mut self, dev: AcceleratorDevice) -> Result<(), AcceleratorError> {
        if self.len >= MAX_ACCEL_DEVICES {
            return Err(AcceleratorError::RegistryFull);
        }
        self.devices[self.len] = Some(dev);
        self.len += 1;
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.len
    }

    pub fn get(&self, idx: usize) -> Option<AcceleratorDevice> {
        if idx >= self.len {
            return None;
        }
        self.devices[idx]
    }

    pub fn find_by_id(&self, id: u16) -> Option<AcceleratorDevice> {
        for i in 0..self.len {
            if let Some(dev) = self.devices[i] {
                if dev.id == id {
                    return Some(dev);
                }
            }
        }
        None
    }

    pub fn count_class(&self, class: AcceleratorClass) -> usize {
        let mut n = 0;
        for i in 0..self.len {
            if let Some(dev) = self.devices[i] {
                if dev.class == class {
                    n += 1;
                }
            }
        }
        n
    }
}

// ── In-kernel quantum circuit slot table ────────────────────────────────────

/// Maximum number of concurrently held in-kernel quantum circuit slots.
#[cfg(feature = "accel")]
pub const MAX_KERNEL_CIRCUITS: usize = 8;

/// State of one in-kernel quantum circuit slot.
#[cfg(feature = "accel")]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KernelCircuitState {
    Free,
    Building,
}

/// One in-kernel circuit slot holding a `quantum::Circuit`.
#[cfg(feature = "accel")]
pub struct KernelCircuitSlot {
    pub state: KernelCircuitState,
    pub circuit: quantum::circuit::Circuit,
}

#[cfg(feature = "accel")]
impl KernelCircuitSlot {
    pub const fn empty() -> Self {
        Self {
            state: KernelCircuitState::Free,
            circuit: quantum::circuit::Circuit::new(0),
        }
    }
}

/// Fixed-size table of in-kernel quantum circuit slots (one per process max).
#[cfg(feature = "accel")]
pub struct QuantumCircuitTable {
    slots: [KernelCircuitSlot; MAX_KERNEL_CIRCUITS],
}

#[cfg(feature = "accel")]
impl QuantumCircuitTable {
    pub const fn new() -> Self {
        // SAFETY: Circuit::new() is const and safe.
        Self {
            slots: [
                KernelCircuitSlot::empty(), KernelCircuitSlot::empty(),
                KernelCircuitSlot::empty(), KernelCircuitSlot::empty(),
                KernelCircuitSlot::empty(), KernelCircuitSlot::empty(),
                KernelCircuitSlot::empty(), KernelCircuitSlot::empty(),
            ],
        }
    }

    /// Allocate a free slot, returning its index.
    pub fn alloc(&mut self, max_qubits: u16) -> Option<usize> {
        for i in 0..MAX_KERNEL_CIRCUITS {
            if self.slots[i].state == KernelCircuitState::Free {
                self.slots[i].state = KernelCircuitState::Building;
                self.slots[i].circuit = quantum::circuit::Circuit::new(max_qubits);
                return Some(i);
            }
        }
        None
    }

    /// Get an active slot by index.
    pub fn get_mut(&mut self, idx: usize) -> Option<&mut KernelCircuitSlot> {
        if idx < MAX_KERNEL_CIRCUITS && self.slots[idx].state == KernelCircuitState::Building {
            Some(&mut self.slots[idx])
        } else {
            None
        }
    }

    /// Free a slot.
    pub fn free(&mut self, idx: usize) {
        if idx < MAX_KERNEL_CIRCUITS {
            self.slots[idx].state = KernelCircuitState::Free;
        }
    }
}

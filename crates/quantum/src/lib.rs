#![no_std]

pub mod backend;
pub mod circuit;
#[cfg(feature = "quantum-cloud")]
pub mod cloud;
#[cfg(feature = "quantum-cloud")]
pub mod ibm;
#[cfg(feature = "simulator")]
pub mod simulator;

pub use backend::{
    BackendInfo, BackendRegistry, BackendType, CalibrationWindow, ConnectivityMap, GateFidelity,
    GateFidelityTable, NativeGateSet, QuantumBackend, QuantumError,
};
pub use circuit::{
    Circuit, CircuitOp, CircuitOpKind, ClassicalRegister, FixedQ16, Gate, Measurement, Qubit,
    QubitLive, QubitMeasured, QubitRange,
};
#[cfg(feature = "quantum-cloud")]
pub use cloud::{
    CloudExecutionMetadata, CloudJobId, CloudJobStatus, CloudProvider, CloudQpu,
    CostEstimateMilliQpuSeconds,
};

pub const QUANTUM_IR_VERSION_MAJOR: u16 = 1;
pub const QUANTUM_IR_VERSION_MINOR: u16 = 0;
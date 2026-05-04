use crate::circuit::{Circuit, ClassicalRegister, Gate, Qubit, QubitLive, QubitRange};

pub const MAX_NATIVE_GATES: usize = 32;
pub const MAX_CONNECTIVITY_EDGES: usize = 256;
pub const MAX_GATE_FIDELITIES: usize = 64;
pub const MAX_BACKENDS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BackendType {
    Simulator = 0,
    Hardware = 1,
    Cloud = 2,
    Fpga = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantumError {
    NotEnoughQubits,
    InvalidQubit,
    GateNotSupported,
    CircuitTooLarge,
    DecoherenceTimeout,
    HardwareError,
    CalibrationExpired,
    CloudTimeout,
    TranspileError,
    BackendBusy,
    InvalidBackend,
}

#[derive(Debug, Clone, Copy)]
pub struct CalibrationWindow {
    pub t1_ns: u32,
    pub t2_ns: u32,
    pub last_calibrated_tick: u64,
    pub expires_after_ticks: u64,
}

impl CalibrationWindow {
    /// Ideal (never-expiring) calibration for simulated backends.
    pub const fn ideal() -> Self {
        Self {
            t1_ns: u32::MAX,
            t2_ns: u32::MAX,
            last_calibrated_tick: 0,
            expires_after_ticks: u64::MAX,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GateFidelity {
    pub gate: Gate,
    pub fidelity_ppm: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct GateFidelityTable {
    entries: [Option<GateFidelity>; MAX_GATE_FIDELITIES],
    len: u8,
}

impl GateFidelityTable {
    pub const fn empty() -> Self {
        Self {
            entries: [None; MAX_GATE_FIDELITIES],
            len: 0,
        }
    }

    pub fn push(&mut self, entry: GateFidelity) -> Result<(), QuantumError> {
        if (self.len as usize) >= MAX_GATE_FIDELITIES {
            return Err(QuantumError::CircuitTooLarge);
        }
        self.entries[self.len as usize] = Some(entry);
        self.len += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct NativeGateSet {
    gates: [Option<Gate>; MAX_NATIVE_GATES],
    len: u8,
}

impl NativeGateSet {
    pub const fn empty() -> Self {
        Self {
            gates: [None; MAX_NATIVE_GATES],
            len: 0,
        }
    }

    pub fn push(&mut self, gate: Gate) -> Result<(), QuantumError> {
        if (self.len as usize) >= MAX_NATIVE_GATES {
            return Err(QuantumError::CircuitTooLarge);
        }
        self.gates[self.len as usize] = Some(gate);
        self.len += 1;
        Ok(())
    }

    pub fn contains(&self, gate: Gate) -> bool {
        let mut i = 0;
        while i < self.len as usize {
            if self.gates[i] == Some(gate) {
                return true;
            }
            i += 1;
        }
        false
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConnectivityMap {
    edges: [Option<(u16, u16)>; MAX_CONNECTIVITY_EDGES],
    len: u16,
}

impl ConnectivityMap {
    pub const fn empty() -> Self {
        Self {
            edges: [None; MAX_CONNECTIVITY_EDGES],
            len: 0,
        }
    }

    pub fn connect(&mut self, q0: u16, q1: u16) -> Result<(), QuantumError> {
        if (self.len as usize) >= MAX_CONNECTIVITY_EDGES {
            return Err(QuantumError::CircuitTooLarge);
        }
        self.edges[self.len as usize] = Some((q0, q1));
        self.len += 1;
        Ok(())
    }

    pub fn allows_pair(&self, q0: u16, q1: u16) -> bool {
        let mut i = 0;
        while i < self.len as usize {
            if let Some((a, b)) = self.edges[i] {
                if (a == q0 && b == q1) || (a == q1 && b == q0) {
                    return true;
                }
            }
            i += 1;
        }
        false
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BackendInfo {
    pub id: u16,
    pub name: &'static str,
    pub backend_type: BackendType,
    pub max_qubits: u16,
    pub queue_depth: u16,
    pub native_gates: NativeGateSet,
    pub connectivity_map: ConnectivityMap,
    pub gate_fidelities: GateFidelityTable,
    pub calibration: CalibrationWindow,
}

pub trait QuantumBackend {
    fn allocate(&mut self, n: u16) -> Result<QubitRange, QuantumError>;
    fn apply(&mut self, gate: Gate, qubits: &[Qubit<QubitLive>]) -> Result<(), QuantumError>;
    fn measure(&mut self, qubit: Qubit<QubitLive>) -> Result<crate::circuit::Measurement, QuantumError>;
    fn execute_circuit(&mut self, circuit: &Circuit) -> Result<ClassicalRegister, QuantumError>;
    fn reset(&mut self) -> Result<(), QuantumError>;
    fn backend_info(&self) -> BackendInfo;
}

pub struct BackendRegistration<'a> {
    pub capability_token: u32,
    pub backend: &'a mut dyn QuantumBackend,
}

pub struct BackendRegistry<'a> {
    entries: [Option<BackendRegistration<'a>>; MAX_BACKENDS],
    len: usize,
}

impl<'a> BackendRegistry<'a> {
    pub const fn new() -> Self {
        Self {
            entries: [None, None, None, None, None, None, None, None],
            len: 0,
        }
    }

    pub fn register(&mut self, entry: BackendRegistration<'a>) -> Result<(), QuantumError> {
        if self.len >= MAX_BACKENDS {
            return Err(QuantumError::BackendBusy);
        }
        self.entries[self.len] = Some(entry);
        self.len += 1;
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.len
    }

    pub fn select_by_capability(
        &mut self,
        capability_token: u32,
    ) -> Result<&mut dyn QuantumBackend, QuantumError> {
        let mut found: Option<usize> = None;
        let mut i = 0;
        while i < self.len {
            if let Some(slot) = self.entries[i].as_ref() {
                if slot.capability_token == capability_token {
                    found = Some(i);
                    break;
                }
            }
            i += 1;
        }

        if let Some(idx) = found {
            if let Some(slot) = self.entries[idx].as_mut() {
                return Ok(slot.backend);
            }
        }

        Err(QuantumError::InvalidBackend)
    }
}
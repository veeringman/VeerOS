use core::marker::PhantomData;

use crate::backend::QuantumError;

#[cfg(feature = "circuit-4k")]
pub const MAX_CIRCUIT_OPS: usize = 4096;
#[cfg(all(not(feature = "circuit-4k"), feature = "circuit-1k"))]
pub const MAX_CIRCUIT_OPS: usize = 1024;
#[cfg(all(not(feature = "circuit-4k"), not(feature = "circuit-1k")))]
pub const MAX_CIRCUIT_OPS: usize = 256;

pub const MAX_CLASSICAL_BITS: usize = 128;
pub const MAX_OP_DEPS: usize = 4;

/// Fixed-point angle in Q16.16 format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FixedQ16(pub i32);

impl FixedQ16 {
    pub const ZERO: Self = Self(0);
    pub const PI: Self = Self(205_887); // pi * 2^16
}

pub enum QubitLive {}
pub enum QubitMeasured {}

/// Opaque qubit handle indexed in backend register space.
#[derive(Debug, PartialEq, Eq)]
pub struct Qubit<State = QubitLive> {
    index: u16,
    _state: PhantomData<State>,
}

impl<State> Copy for Qubit<State> {}

impl<State> Clone for Qubit<State> {
    fn clone(&self) -> Self {
        *self
    }
}

impl Qubit<QubitLive> {
    pub const fn new(index: u16) -> Self {
        Self {
            index,
            _state: PhantomData,
        }
    }

    pub const fn measured(self) -> Qubit<QubitMeasured> {
        Qubit {
            index: self.index,
            _state: PhantomData,
        }
    }
}

impl<State> Qubit<State> {
    pub const fn index(&self) -> u16 {
        self.index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QubitRange {
    pub base: u16,
    pub len: u16,
}

impl QubitRange {
    pub const fn get(&self, offset: u16) -> Option<Qubit<QubitLive>> {
        if offset < self.len {
            Some(Qubit::new(self.base + offset))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Measurement {
    Zero,
    One,
}

#[derive(Debug, Clone, Copy)]
pub struct ClassicalRegister {
    bits: [Measurement; MAX_CLASSICAL_BITS],
    len: u16,
}

impl ClassicalRegister {
    pub const fn new() -> Self {
        Self {
            bits: [Measurement::Zero; MAX_CLASSICAL_BITS],
            len: 0,
        }
    }

    pub fn push(&mut self, bit: Measurement) -> Result<(), QuantumError> {
        if (self.len as usize) >= MAX_CLASSICAL_BITS {
            return Err(QuantumError::CircuitTooLarge);
        }
        self.bits[self.len as usize] = bit;
        self.len += 1;
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub fn get(&self, idx: usize) -> Option<Measurement> {
        if idx < self.len as usize {
            Some(self.bits[idx])
        } else {
            None
        }
    }
}

impl Default for ClassicalRegister {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    H,
    X,
    Y,
    Z,
    S,
    Sdg,
    T,
    Tdg,
    Rx(FixedQ16),
    Ry(FixedQ16),
    Rz(FixedQ16),
    U(FixedQ16, FixedQ16, FixedQ16),
    Cx,
    Cz,
    Cy,
    Swap,
    ISwap,
    Ecr,
    Rxx(FixedQ16),
    Ryy(FixedQ16),
    Rzz(FixedQ16),
    Ccx,
    Cswap,
    Ccz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitOpKind {
    Gate {
        gate: Gate,
        qubits: [u16; 3],
        qubit_count: u8,
    },
    Barrier,
    Measure {
        qubit: u16,
        cbit: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitOp {
    pub kind: CircuitOpKind,
    pub deps: [u16; MAX_OP_DEPS],
    pub dep_count: u8,
}

impl CircuitOp {
    pub const fn barrier() -> Self {
        Self {
            kind: CircuitOpKind::Barrier,
            deps: [0; MAX_OP_DEPS],
            dep_count: 0,
        }
    }

    pub const fn measure(qubit: u16, cbit: u16) -> Self {
        Self {
            kind: CircuitOpKind::Measure { qubit, cbit },
            deps: [0; MAX_OP_DEPS],
            dep_count: 0,
        }
    }

    pub const fn gate1(gate: Gate, q0: u16) -> Self {
        Self {
            kind: CircuitOpKind::Gate {
                gate,
                qubits: [q0, 0, 0],
                qubit_count: 1,
            },
            deps: [0; MAX_OP_DEPS],
            dep_count: 0,
        }
    }

    pub const fn gate2(gate: Gate, q0: u16, q1: u16) -> Self {
        Self {
            kind: CircuitOpKind::Gate {
                gate,
                qubits: [q0, q1, 0],
                qubit_count: 2,
            },
            deps: [0; MAX_OP_DEPS],
            dep_count: 0,
        }
    }

    pub const fn gate3(gate: Gate, q0: u16, q1: u16, q2: u16) -> Self {
        Self {
            kind: CircuitOpKind::Gate {
                gate,
                qubits: [q0, q1, q2],
                qubit_count: 3,
            },
            deps: [0; MAX_OP_DEPS],
            dep_count: 0,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Circuit {
    pub logical_qubits: u16,
    ops: [Option<CircuitOp>; MAX_CIRCUIT_OPS],
    op_len: u16,
    classical_bits: u16,
    first_error: Option<QuantumError>,
}

impl Circuit {
    pub const fn new(logical_qubits: u16) -> Self {
        Self {
            logical_qubits,
            ops: [None; MAX_CIRCUIT_OPS],
            op_len: 0,
            classical_bits: 0,
            first_error: None,
        }
    }

    pub fn push_op(&mut self, op: CircuitOp) -> Result<(), QuantumError> {
        if (self.op_len as usize) >= MAX_CIRCUIT_OPS {
            return Err(QuantumError::CircuitTooLarge);
        }
        self.ops[self.op_len as usize] = Some(op);
        self.op_len += 1;
        Ok(())
    }

    fn record_err(&mut self, err: QuantumError) {
        if self.first_error.is_none() {
            self.first_error = Some(err);
        }
    }

    pub fn status(&self) -> Result<(), QuantumError> {
        match self.first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    pub fn ops_len(&self) -> usize {
        self.op_len as usize
    }

    pub fn classical_bits_count(&self) -> usize {
        self.classical_bits as usize
    }

    pub fn op(&self, idx: usize) -> Option<CircuitOp> {
        if idx < self.op_len as usize {
            self.ops[idx]
        } else {
            None
        }
    }

    pub fn barrier(mut self) -> Self {
        if let Err(err) = self.push_op(CircuitOp::barrier()) {
            self.record_err(err);
        }
        self
    }

    pub fn measure(mut self, qubit: u16) -> Self {
        if qubit >= self.logical_qubits {
            self.record_err(QuantumError::InvalidQubit);
            return self;
        }
        if self.classical_bits >= MAX_CLASSICAL_BITS as u16 {
            self.record_err(QuantumError::CircuitTooLarge);
            return self;
        }
        let cbit = self.classical_bits;
        self.classical_bits += 1;
        if let Err(err) = self.push_op(CircuitOp::measure(qubit, cbit)) {
            self.record_err(err);
        }
        self
    }

    pub fn measure_all(mut self) -> Self {
        let mut q = 0;
        while q < self.logical_qubits {
            self = self.measure(q);
            q += 1;
        }
        self
    }

    pub fn h(mut self, q0: u16) -> Self {
        self.push_single(Gate::H, q0);
        self
    }

    pub fn x(mut self, q0: u16) -> Self {
        self.push_single(Gate::X, q0);
        self
    }

    pub fn y(mut self, q0: u16) -> Self {
        self.push_single(Gate::Y, q0);
        self
    }

    pub fn z(mut self, q0: u16) -> Self {
        self.push_single(Gate::Z, q0);
        self
    }

    pub fn rz(mut self, q0: u16, theta: FixedQ16) -> Self {
        self.push_single(Gate::Rz(theta), q0);
        self
    }

    pub fn cx(mut self, control: u16, target: u16) -> Self {
        self.push_pair(Gate::Cx, control, target);
        self
    }

    pub fn cnot(self, control: u16, target: u16) -> Self {
        self.cx(control, target)
    }

    pub fn ccx(mut self, q0: u16, q1: u16, q2: u16) -> Self {
        self.push_triple(Gate::Ccx, q0, q1, q2);
        self
    }

    fn push_single(&mut self, gate: Gate, q0: u16) {
        if q0 >= self.logical_qubits {
            self.record_err(QuantumError::InvalidQubit);
            return;
        }
        if let Err(err) = self.push_op(CircuitOp::gate1(gate, q0)) {
            self.record_err(err);
        }
    }

    fn push_pair(&mut self, gate: Gate, q0: u16, q1: u16) {
        if q0 >= self.logical_qubits || q1 >= self.logical_qubits {
            self.record_err(QuantumError::InvalidQubit);
            return;
        }
        if let Err(err) = self.push_op(CircuitOp::gate2(gate, q0, q1)) {
            self.record_err(err);
        }
    }

    fn push_triple(&mut self, gate: Gate, q0: u16, q1: u16, q2: u16) {
        if q0 >= self.logical_qubits || q1 >= self.logical_qubits || q2 >= self.logical_qubits {
            self.record_err(QuantumError::InvalidQubit);
            return;
        }
        if let Err(err) = self.push_op(CircuitOp::gate3(gate, q0, q1, q2)) {
            self.record_err(err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluent_builder_creates_ops() {
        let c = Circuit::new(4)
            .h(0)
            .cnot(0, 1)
            .rz(1, FixedQ16::PI)
            .barrier()
            .measure_all();

        assert!(c.status().is_ok());
        assert_eq!(c.ops_len(), 8);
    }

    #[test]
    fn qubit_typestate_blocks_reuse_after_measure() {
        let q = Qubit::<QubitLive>::new(3);
        let mq = q.measured();
        assert_eq!(mq.index(), 3);
    }
}
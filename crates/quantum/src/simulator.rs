use crate::backend::{
    BackendInfo, BackendType, CalibrationWindow, ConnectivityMap, GateFidelity, GateFidelityTable,
    NativeGateSet, QuantumBackend, QuantumError,
};
use crate::circuit::{
    Circuit, CircuitOpKind, ClassicalRegister, FixedQ16, Gate, Measurement, Qubit, QubitLive,
    QubitRange,
};

pub const MAX_SIM_QUBITS: usize = 12;
const MAX_STATE_LEN: usize = 1 << MAX_SIM_QUBITS;
const INV_SQRT2: f32 = 0.70710677;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Complex32 {
    pub re: f32,
    pub im: f32,
}

impl Complex32 {
    pub const fn new(re: f32, im: f32) -> Self {
        Self { re, im }
    }

    pub const fn zero() -> Self {
        Self { re: 0.0, im: 0.0 }
    }

    pub const fn one() -> Self {
        Self { re: 1.0, im: 0.0 }
    }

    pub fn norm_sq(self) -> f32 {
        self.re * self.re + self.im * self.im
    }

    pub fn scale(self, s: f32) -> Self {
        Self::new(self.re * s, self.im * s)
    }

    pub fn add(self, other: Self) -> Self {
        Self::new(self.re + other.re, self.im + other.im)
    }

    pub fn mul(self, other: Self) -> Self {
        Self::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }
}

const HIST_CAPACITY: usize = 64;
const PI_F32: f32 = 3.14159265;
const TWO_PI_F32: f32 = 6.2831853;

fn sqrt_approx(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut g = if x > 1.0 { x } else { 1.0 };
    let mut i = 0;
    while i < 6 {
        g = 0.5 * (g + x / g);
        i += 1;
    }
    g
}

fn wrap_pi(mut x: f32) -> f32 {
    while x > PI_F32 {
        x -= TWO_PI_F32;
    }
    while x < -PI_F32 {
        x += TWO_PI_F32;
    }
    x
}

fn sin_approx(x: f32) -> f32 {
    let xr = wrap_pi(x);
    let x2 = xr * xr;
    xr * (1.0 - (x2 / 6.0) + (x2 * x2 / 120.0) - (x2 * x2 * x2 / 5040.0))
}

fn cos_approx(x: f32) -> f32 {
    let xr = wrap_pi(x);
    let x2 = xr * xr;
    1.0 - (x2 / 2.0) + (x2 * x2 / 24.0) - (x2 * x2 * x2 / 720.0)
}

#[derive(Debug, Clone, Copy)]
pub struct ShotHistogram {
    pub total_shots: u32,
    bins: [Option<(u16, u32)>; HIST_CAPACITY],
}

impl ShotHistogram {
    pub const fn new() -> Self {
        Self {
            total_shots: 0,
            bins: [None; HIST_CAPACITY],
        }
    }

    pub fn increment(&mut self, key: u16) {
        self.total_shots = self.total_shots.saturating_add(1);

        let mut i = 0;
        while i < HIST_CAPACITY {
            if let Some((k, v)) = self.bins[i] {
                if k == key {
                    self.bins[i] = Some((k, v.saturating_add(1)));
                    return;
                }
            }
            i += 1;
        }

        let mut j = 0;
        while j < HIST_CAPACITY {
            if self.bins[j].is_none() {
                self.bins[j] = Some((key, 1));
                return;
            }
            j += 1;
        }
    }

    pub fn count(&self, key: u16) -> u32 {
        let mut i = 0;
        while i < HIST_CAPACITY {
            if let Some((k, v)) = self.bins[i] {
                if k == key {
                    return v;
                }
            }
            i += 1;
        }
        0
    }

    /// Return the i-th occupied bin as `(bitstring, count)`, skipping empty slots.
    pub fn bin(&self, slot: usize) -> Option<(u16, u32)> {
        if slot < HIST_CAPACITY {
            self.bins[slot]
        } else {
            None
        }
    }
}

/// Fixed-size state-vector backend for no_std environments.
pub struct SimulatorBackend {
    allocated: u16,
    amplitudes: [Complex32; MAX_STATE_LEN],
    seed: u64,
    queue_depth: u16,
}

impl SimulatorBackend {
    pub const fn new() -> Self {
        Self {
            allocated: 0,
            amplitudes: [Complex32::zero(); MAX_STATE_LEN],
            seed: 0x5EED_5EED_1234_5678,
            queue_depth: 0,
        }
    }

    pub const fn with_seed(seed: u64) -> Self {
        Self {
            allocated: 0,
            amplitudes: [Complex32::zero(); MAX_STATE_LEN],
            seed,
            queue_depth: 0,
        }
    }

    #[inline]
    fn next_u64(&mut self) -> u64 {
        // xorshift64* PRNG for deterministic pseudo-random sampling.
        let mut x = self.seed;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.seed = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn next_unit_f32(&mut self) -> f32 {
        let n = self.next_u64() >> 40;
        (n as f32) / ((1u32 << 24) as f32)
    }

    fn check_qubit(&self, q: u16) -> Result<usize, QuantumError> {
        let idx = q as usize;
        if idx >= self.allocated as usize || idx >= MAX_SIM_QUBITS {
            return Err(QuantumError::InvalidQubit);
        }
        Ok(idx)
    }

    fn active_len(&self) -> usize {
        1usize << (self.allocated as usize)
    }

    fn reset_statevector(&mut self) {
        self.amplitudes = [Complex32::zero(); MAX_STATE_LEN];
        self.amplitudes[0] = Complex32::one();
    }

    fn apply_single_matrix(
        &mut self,
        q: usize,
        m00: Complex32,
        m01: Complex32,
        m10: Complex32,
        m11: Complex32,
    ) {
        let n = self.active_len();
        let stride = 1usize << q;
        let step = stride << 1;
        let mut base = 0;
        while base < n {
            let mut off = 0;
            while off < stride {
                let i0 = base + off;
                let i1 = i0 + stride;
                let a0 = self.amplitudes[i0];
                let a1 = self.amplitudes[i1];
                self.amplitudes[i0] = m00.mul(a0).add(m01.mul(a1));
                self.amplitudes[i1] = m10.mul(a0).add(m11.mul(a1));
                off += 1;
            }
            base += step;
        }
    }

    fn apply_cx(&mut self, control: usize, target: usize) {
        let n = self.active_len();
        let c_mask = 1usize << control;
        let t_mask = 1usize << target;
        let mut i = 0;
        while i < n {
            if (i & c_mask) != 0 && (i & t_mask) == 0 {
                let j = i | t_mask;
                let tmp = self.amplitudes[i];
                self.amplitudes[i] = self.amplitudes[j];
                self.amplitudes[j] = tmp;
            }
            i += 1;
        }
    }

    fn measure_qubit(&mut self, q: usize) -> Measurement {
        let n = self.active_len();
        let q_mask = 1usize << q;
        let mut p0 = 0.0f32;

        let mut i = 0;
        while i < n {
            if (i & q_mask) == 0 {
                p0 += self.amplitudes[i].norm_sq();
            }
            i += 1;
        }

        let r = self.next_unit_f32();
        let outcome_one = r >= p0;

        let norm = if outcome_one {
            sqrt_approx((1.0 - p0).max(1e-12))
        } else {
            sqrt_approx(p0.max(1e-12))
        };
        let inv_norm = 1.0 / norm;

        let mut j = 0;
        while j < n {
            let is_one = (j & q_mask) != 0;
            if is_one == outcome_one {
                self.amplitudes[j] = self.amplitudes[j].scale(inv_norm);
            } else {
                self.amplitudes[j] = Complex32::zero();
            }
            j += 1;
        }

        if outcome_one {
            Measurement::One
        } else {
            Measurement::Zero
        }
    }

    fn phase(theta: FixedQ16) -> (f32, f32) {
        let t = (theta.0 as f32) / 65536.0;
        (cos_approx(t), sin_approx(t))
    }

    pub fn execute_shots(
        &mut self,
        circuit: &Circuit,
        shots: u32,
    ) -> Result<ShotHistogram, QuantumError> {
        if circuit.logical_qubits as usize > MAX_SIM_QUBITS {
            return Err(QuantumError::NotEnoughQubits);
        }

        let mut hist = ShotHistogram::new();
        let mut s = 0;
        while s < shots {
            self.reset()?;
            self.allocate(circuit.logical_qubits)?;
            let reg = self.execute_circuit(circuit)?;

            if reg.len() > 16 {
                return Err(QuantumError::CircuitTooLarge);
            }

            let mut key: u16 = 0;
            let mut i = 0;
            while i < reg.len() {
                if matches!(reg.get(i), Some(Measurement::One)) {
                    key |= 1u16 << i;
                }
                i += 1;
            }
            hist.increment(key);
            s += 1;
        }

        Ok(hist)
    }
}

impl Default for SimulatorBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl QuantumBackend for SimulatorBackend {
    fn allocate(&mut self, n: u16) -> Result<QubitRange, QuantumError> {
        if n == 0 {
            return Ok(QubitRange { base: 0, len: 0 });
        }
        if n as usize > MAX_SIM_QUBITS {
            return Err(QuantumError::NotEnoughQubits);
        }
        if self.allocated == 0 {
            self.reset_statevector();
        }
        if (self.allocated as usize) + (n as usize) > MAX_SIM_QUBITS {
            return Err(QuantumError::NotEnoughQubits);
        }
        let base = self.allocated;
        self.allocated += n;
        Ok(QubitRange { base, len: n })
    }

    fn apply(&mut self, gate: Gate, qubits: &[Qubit<QubitLive>]) -> Result<(), QuantumError> {
        match gate {
            Gate::H => {
                if qubits.len() != 1 {
                    return Err(QuantumError::InvalidQubit);
                }
                let q = self.check_qubit(qubits[0].index())?;
                self.apply_single_matrix(
                    q,
                    Complex32::new(INV_SQRT2, 0.0),
                    Complex32::new(INV_SQRT2, 0.0),
                    Complex32::new(INV_SQRT2, 0.0),
                    Complex32::new(-INV_SQRT2, 0.0),
                );
                Ok(())
            }
            Gate::X => {
                if qubits.len() != 1 {
                    return Err(QuantumError::InvalidQubit);
                }
                let q = self.check_qubit(qubits[0].index())?;
                self.apply_single_matrix(
                    q,
                    Complex32::zero(),
                    Complex32::one(),
                    Complex32::one(),
                    Complex32::zero(),
                );
                Ok(())
            }
            Gate::Z => {
                if qubits.len() != 1 {
                    return Err(QuantumError::InvalidQubit);
                }
                let q = self.check_qubit(qubits[0].index())?;
                self.apply_single_matrix(
                    q,
                    Complex32::one(),
                    Complex32::zero(),
                    Complex32::zero(),
                    Complex32::new(-1.0, 0.0),
                );
                Ok(())
            }
            Gate::S => {
                if qubits.len() != 1 {
                    return Err(QuantumError::InvalidQubit);
                }
                let q = self.check_qubit(qubits[0].index())?;
                self.apply_single_matrix(
                    q,
                    Complex32::one(),
                    Complex32::zero(),
                    Complex32::zero(),
                    Complex32::new(0.0, 1.0),
                );
                Ok(())
            }
            Gate::Sdg => {
                if qubits.len() != 1 {
                    return Err(QuantumError::InvalidQubit);
                }
                let q = self.check_qubit(qubits[0].index())?;
                self.apply_single_matrix(
                    q,
                    Complex32::one(),
                    Complex32::zero(),
                    Complex32::zero(),
                    Complex32::new(0.0, -1.0),
                );
                Ok(())
            }
            Gate::T => {
                if qubits.len() != 1 {
                    return Err(QuantumError::InvalidQubit);
                }
                let q = self.check_qubit(qubits[0].index())?;
                let c = 0.70710677;
                let s = 0.70710677;
                self.apply_single_matrix(
                    q,
                    Complex32::one(),
                    Complex32::zero(),
                    Complex32::zero(),
                    Complex32::new(c, s),
                );
                Ok(())
            }
            Gate::Tdg => {
                if qubits.len() != 1 {
                    return Err(QuantumError::InvalidQubit);
                }
                let q = self.check_qubit(qubits[0].index())?;
                let c = 0.70710677;
                let s = -0.70710677;
                self.apply_single_matrix(
                    q,
                    Complex32::one(),
                    Complex32::zero(),
                    Complex32::zero(),
                    Complex32::new(c, s),
                );
                Ok(())
            }
            Gate::Rz(theta) => {
                if qubits.len() != 1 {
                    return Err(QuantumError::InvalidQubit);
                }
                let q = self.check_qubit(qubits[0].index())?;
                let half = FixedQ16(theta.0 / 2);
                let (c1, s1) = Self::phase(FixedQ16(-half.0));
                let (c2, s2) = Self::phase(half);
                self.apply_single_matrix(
                    q,
                    Complex32::new(c1, s1),
                    Complex32::zero(),
                    Complex32::zero(),
                    Complex32::new(c2, s2),
                );
                Ok(())
            }
            Gate::Cx => {
                if qubits.len() != 2 {
                    return Err(QuantumError::InvalidQubit);
                }
                let c = self.check_qubit(qubits[0].index())?;
                let t = self.check_qubit(qubits[1].index())?;
                self.apply_cx(c, t);
                Ok(())
            }
            _ => Err(QuantumError::GateNotSupported),
        }
    }

    fn measure(&mut self, qubit: Qubit<QubitLive>) -> Result<Measurement, QuantumError> {
        let idx = self.check_qubit(qubit.index())?;
        Ok(self.measure_qubit(idx))
    }

    fn execute_circuit(&mut self, circuit: &Circuit) -> Result<ClassicalRegister, QuantumError> {
        circuit.status()?;
        self.queue_depth = self.queue_depth.saturating_add(1);
        let mut out = ClassicalRegister::new();

        let mut i = 0;
        while i < circuit.ops_len() {
            let op = match circuit.op(i) {
                Some(op) => op,
                None => break,
            };
            match op.kind {
                CircuitOpKind::Gate {
                    gate,
                    qubits,
                    qubit_count,
                } => {
                    let q0 = Qubit::new(qubits[0]);
                    let q1 = Qubit::new(qubits[1]);
                    let q2 = Qubit::new(qubits[2]);
                    match qubit_count {
                        1 => self.apply(gate, &[q0])?,
                        2 => self.apply(gate, &[q0, q1])?,
                        3 => self.apply(gate, &[q0, q1, q2])?,
                        _ => return Err(QuantumError::InvalidQubit),
                    }
                }
                CircuitOpKind::Barrier => {}
                CircuitOpKind::Measure { qubit, .. } => {
                    out.push(self.measure(Qubit::new(qubit))?)?;
                }
            }
            i += 1;
        }

        self.queue_depth = self.queue_depth.saturating_sub(1);
        Ok(out)
    }

    fn reset(&mut self) -> Result<(), QuantumError> {
        self.allocated = 0;
        self.reset_statevector();
        self.queue_depth = 0;
        Ok(())
    }

    fn backend_info(&self) -> BackendInfo {
        let mut native = NativeGateSet::empty();
        let _ = native.push(Gate::H);
        let _ = native.push(Gate::X);
        let _ = native.push(Gate::Z);
        let _ = native.push(Gate::S);
        let _ = native.push(Gate::T);
        let _ = native.push(Gate::Rz(crate::circuit::FixedQ16::ZERO));
        let _ = native.push(Gate::Cx);

        let mut map = ConnectivityMap::empty();
        let max_q = self.allocated.max(1);
        let mut i = 0;
        while i + 1 < max_q {
            let _ = map.connect(i, i + 1);
            i += 1;
        }

        let mut fidelities = GateFidelityTable::empty();
        let _ = fidelities.push(GateFidelity {
            gate: Gate::H,
            fidelity_ppm: 1_000_000,
        });
        let _ = fidelities.push(GateFidelity {
            gate: Gate::X,
            fidelity_ppm: 1_000_000,
        });
        let _ = fidelities.push(GateFidelity {
            gate: Gate::Cx,
            fidelity_ppm: 1_000_000,
        });

        BackendInfo {
            id: 0,
            name: "veeros-local-sim",
            backend_type: BackendType::Simulator,
            max_qubits: MAX_SIM_QUBITS as u16,
            queue_depth: self.queue_depth,
            native_gates: native,
            connectivity_map: map,
            gate_fidelities: fidelities,
            calibration: CalibrationWindow {
                t1_ns: 0,
                t2_ns: 0,
                last_calibrated_tick: 0,
                expires_after_ticks: u64::MAX,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bell_like_flow_executes() {
        let mut sim = SimulatorBackend::with_seed(1234);
        let _ = sim.allocate(2).expect("alloc");
        let c = Circuit::new(2).h(0).cnot(0, 1).measure_all();
        let out = sim.execute_circuit(&c).expect("execute");
        assert_eq!(out.len(), 2);
        assert_eq!(out.get(0), out.get(1));
    }

    #[test]
    fn hadamard_shots_are_reasonable() {
        let mut sim = SimulatorBackend::with_seed(1);
        let c = Circuit::new(1).h(0).measure_all();
        let hist = sim.execute_shots(&c, 1000).expect("shots");
        let zeros = hist.count(0);
        let ones = hist.count(1);
        assert!(zeros > 300 && zeros < 700);
        assert!(ones > 300 && ones < 700);
    }
}
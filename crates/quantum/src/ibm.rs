//! IBM Quantum cloud backend adapter.
//!
//! This module keeps a strict `no_std` profile and uses a caller-provided
//! transport callback for HTTPS requests. The transport can be implemented in
//! host tooling (`std`) or in-kernel TLS stacks.

use crate::backend::{
    BackendInfo, BackendType, CalibrationWindow, ConnectivityMap, GateFidelityTable,
    NativeGateSet, QuantumBackend, QuantumError,
};
use crate::circuit::{
    Circuit, CircuitOpKind, ClassicalRegister, Gate, Measurement, Qubit, QubitLive,
    QubitRange,
};
use crate::cloud::{
    CloudExecutionMetadata, CloudJobId, CloudJobStatus, CloudProvider, CloudQpu,
    CostEstimateMilliQpuSeconds,
};

pub const MAX_QASM_LEN: usize = 8192;
pub const MAX_TOKEN_LEN: usize = 256;
pub const MAX_BACKEND_NAME_LEN: usize = 64;
pub const MAX_TRACKED_JOBS: usize = 4;
pub const MAX_REMOTE_JOB_ID_LEN: usize = 96;
pub const MAX_API_HOST_LEN: usize = 128;

pub const IBM_API_HOST: &[u8] = b"api.quantum-computing.ibm.com";

pub struct HttpRequest<'a> {
    pub method: HttpMethod,
    pub host: &'a [u8],
    pub path: &'a [u8],
    pub headers: &'a [(&'static [u8], &'a [u8])],
    pub body: &'a [u8],
    pub out_buf: &'a mut [u8],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    NotConnected,
    Timeout,
    TlsError,
    HttpError(u16),
    BufferTooSmall,
}

pub type TransportFn = fn(req: HttpRequest<'_>) -> Result<usize, TransportError>;

pub fn default_transport(_req: HttpRequest<'_>) -> Result<usize, TransportError> {
    Err(TransportError::NotConnected)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JobState {
    Empty,
    Submitted,
    Completed,
    Failed,
    Cancelled,
}

struct JobSlot {
    state: JobState,
    id: u64,
    shots: u32,
    remote_job_id: [u8; MAX_REMOTE_JOB_ID_LEN],
    remote_job_id_len: usize,
    result: ClassicalRegister,
}

impl JobSlot {
    const fn empty() -> Self {
        Self {
            state: JobState::Empty,
            id: 0,
            shots: 0,
            remote_job_id: [0; MAX_REMOTE_JOB_ID_LEN],
            remote_job_id_len: 0,
            result: ClassicalRegister::new(),
        }
    }
}

pub struct IbmQuantumBackend {
    api_token: [u8; MAX_TOKEN_LEN],
    api_token_len: usize,
    instance: [u8; MAX_TOKEN_LEN],
    instance_len: usize,
    backend_name: [u8; MAX_BACKEND_NAME_LEN],
    backend_len: usize,
    api_host: [u8; MAX_API_HOST_LEN],
    api_host_len: usize,
    max_qubits: u16,
    default_shots: u32,
    transport: TransportFn,
    jobs: [JobSlot; MAX_TRACKED_JOBS],
    job_counter: u64,
    qasm_buf: [u8; MAX_QASM_LEN],
    resp_buf: [u8; 4096],
    last_histogram_json: [u8; 2048],
    last_histogram_len: usize,
}

impl IbmQuantumBackend {
    pub fn new(
        api_token: &[u8],
        instance: &[u8],
        backend_name: &[u8],
        max_qubits: u16,
        transport: TransportFn,
    ) -> Self {
        let mut s = Self {
            api_token: [0; MAX_TOKEN_LEN],
            api_token_len: 0,
            instance: [0; MAX_TOKEN_LEN],
            instance_len: 0,
            backend_name: [0; MAX_BACKEND_NAME_LEN],
            backend_len: 0,
            api_host: [0; MAX_API_HOST_LEN],
            api_host_len: 0,
            max_qubits,
            default_shots: 1024,
            transport,
            jobs: [
                JobSlot::empty(),
                JobSlot::empty(),
                JobSlot::empty(),
                JobSlot::empty(),
            ],
            job_counter: 1,
            qasm_buf: [0; MAX_QASM_LEN],
            resp_buf: [0; 4096],
            last_histogram_json: [0; 2048],
            last_histogram_len: 0,
        };

        let tlen = api_token.len().min(MAX_TOKEN_LEN);
        s.api_token[..tlen].copy_from_slice(&api_token[..tlen]);
        s.api_token_len = tlen;

        let ilen = instance.len().min(MAX_TOKEN_LEN);
        s.instance[..ilen].copy_from_slice(&instance[..ilen]);
        s.instance_len = ilen;

        let blen = backend_name.len().min(MAX_BACKEND_NAME_LEN);
        s.backend_name[..blen].copy_from_slice(&backend_name[..blen]);
        s.backend_len = blen;

        // Initialize with default IBM API host
        let hlen = IBM_API_HOST.len().min(MAX_API_HOST_LEN);
        s.api_host[..hlen].copy_from_slice(&IBM_API_HOST[..hlen]);
        s.api_host_len = hlen;

        s
    }

    pub fn set_default_shots(&mut self, shots: u32) {
        self.default_shots = if shots == 0 { 1 } else { shots };
    }

    pub fn set_api_host(&mut self, host: &[u8]) {
        let hlen = host.len().min(MAX_API_HOST_LEN);
        self.api_host[..hlen].copy_from_slice(&host[..hlen]);
        self.api_host_len = hlen;
    }

    pub fn last_histogram(&self) -> &[u8] {
        &self.last_histogram_json[..self.last_histogram_len]
    }

    pub fn serialize_qasm(&mut self, circuit: &Circuit) -> Option<usize> {
        let mut pos = 0usize;
        let b = &mut self.qasm_buf;

        macro_rules! emit {
            ($bytes:expr) => {{
                let s: &[u8] = $bytes;
                if pos + s.len() > b.len() {
                    return None;
                }
                b[pos..pos + s.len()].copy_from_slice(s);
                pos += s.len();
            }};
        }

        macro_rules! emit_u16 {
            ($v:expr) => {{
                let mut n: u16 = $v;
                if n == 0 {
                    emit!(b"0");
                } else {
                    let mut tmp = [0u8; 5];
                    let mut tlen = 0usize;
                    while n > 0 {
                        tmp[tlen] = b'0' + (n % 10) as u8;
                        n /= 10;
                        tlen += 1;
                    }
                    let mut i = 0;
                    let mut j = tlen - 1;
                    while i < j {
                        tmp.swap(i, j);
                        i += 1;
                        j -= 1;
                    }
                    emit!(&tmp[..tlen]);
                }
            }};
        }

        macro_rules! emit_angle {
            ($fq:expr) => {{
                let raw: i32 = ($fq).0;
                if raw < 0 {
                    emit!(b"-");
                }
                let abs = (raw as i64).unsigned_abs();
                let int_part = (abs >> 16) as u32;
                let frac_raw = (abs & 0xFFFF) as u32;
                let frac = (frac_raw * 100000 + 32767) / 65536;
                emit_u16!(int_part as u16);
                emit!(b".");
                let mut tmp = [0u8; 5];
                let mut fv = frac;
                let mut i = 4usize;
                loop {
                    tmp[i] = b'0' + (fv % 10) as u8;
                    fv /= 10;
                    if i == 0 {
                        break;
                    }
                    i -= 1;
                }
                emit!(&tmp);
            }};
        }

        emit!(b"OPENQASM 3.0;\n");
        emit!(b"include \"stdgates.inc\";\n");

        emit!(b"qubit[");
        emit_u16!(circuit.logical_qubits);
        emit!(b"] q;\n");

        let nc = circuit.classical_bits_count() as u16;
        if nc > 0 {
            emit!(b"bit[");
            emit_u16!(nc);
            emit!(b"] c;\n");
        }

        let mut op_idx = 0usize;
        while op_idx < circuit.ops_len() {
            if let Some(op) = circuit.op(op_idx) {
                match op.kind {
                    CircuitOpKind::Gate {
                        gate,
                        qubits,
                        qubit_count,
                    } => {
                        match gate {
                            Gate::H => emit!(b"h "),
                            Gate::X => emit!(b"x "),
                            Gate::Y => emit!(b"y "),
                            Gate::Z => emit!(b"z "),
                            Gate::S => emit!(b"s "),
                            Gate::Sdg => emit!(b"sdg "),
                            Gate::T => emit!(b"t "),
                            Gate::Tdg => emit!(b"tdg "),
                            Gate::Rx(a) => {
                                emit!(b"rx(");
                                emit_angle!(a);
                                emit!(b") ");
                            }
                            Gate::Ry(a) => {
                                emit!(b"ry(");
                                emit_angle!(a);
                                emit!(b") ");
                            }
                            Gate::Rz(a) => {
                                emit!(b"rz(");
                                emit_angle!(a);
                                emit!(b") ");
                            }
                            Gate::U(t, p, l) => {
                                emit!(b"U(");
                                emit_angle!(t);
                                emit!(b",");
                                emit_angle!(p);
                                emit!(b",");
                                emit_angle!(l);
                                emit!(b") ");
                            }
                            Gate::Cx => emit!(b"cx "),
                            Gate::Cz => emit!(b"cz "),
                            Gate::Cy => emit!(b"cy "),
                            Gate::Swap => emit!(b"swap "),
                            Gate::ISwap => emit!(b"iswap "),
                            Gate::Ecr => emit!(b"ecr "),
                            Gate::Rxx(a) => {
                                emit!(b"rxx(");
                                emit_angle!(a);
                                emit!(b") ");
                            }
                            Gate::Ryy(a) => {
                                emit!(b"ryy(");
                                emit_angle!(a);
                                emit!(b") ");
                            }
                            Gate::Rzz(a) => {
                                emit!(b"rzz(");
                                emit_angle!(a);
                                emit!(b") ");
                            }
                            Gate::Ccx => emit!(b"ccx "),
                            Gate::Cswap => emit!(b"cswap "),
                            Gate::Ccz => emit!(b"ccz "),
                        }

                        let mut qi = 0u8;
                        while qi < qubit_count {
                            if qi > 0 {
                                emit!(b", ");
                            }
                            emit!(b"q[");
                            emit_u16!(qubits[qi as usize]);
                            emit!(b"]");
                            qi += 1;
                        }
                        emit!(b";\n");
                    }
                    CircuitOpKind::Measure { qubit, cbit } => {
                        emit!(b"c[");
                        emit_u16!(cbit);
                        emit!(b"] = measure q[");
                        emit_u16!(qubit);
                        emit!(b"];\n");
                    }
                    CircuitOpKind::Barrier => emit!(b"barrier q;\n"),
                }
            }
            op_idx += 1;
        }

        Some(pos)
    }

    fn alloc_job_slot(&mut self) -> Option<usize> {
        let mut i = 0;
        while i < MAX_TRACKED_JOBS {
            if self.jobs[i].state == JobState::Empty {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    fn find_slot_by_local_id(&self, id: u64) -> Option<usize> {
        let mut i = 0;
        while i < MAX_TRACKED_JOBS {
            if self.jobs[i].state != JobState::Empty && self.jobs[i].id == id {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    fn map_transport_error(err: TransportError) -> QuantumError {
        match err {
            TransportError::Timeout => QuantumError::CloudTimeout,
            TransportError::HttpError(429) => QuantumError::BackendBusy,
            TransportError::HttpError(_) => QuantumError::HardwareError,
            TransportError::BufferTooSmall => QuantumError::CircuitTooLarge,
            TransportError::NotConnected | TransportError::TlsError => QuantumError::CloudTimeout,
        }
    }

    fn make_auth_header(&self, out: &mut [u8; MAX_TOKEN_LEN + 8]) -> usize {
        let prefix = b"Bearer ";
        out[..prefix.len()].copy_from_slice(prefix);
        out[prefix.len()..prefix.len() + self.api_token_len]
            .copy_from_slice(&self.api_token[..self.api_token_len]);
        prefix.len() + self.api_token_len
    }

    fn build_job_path<'a>(&self, remote_id: &'a [u8], out: &'a mut [u8; 160]) -> Option<&'a [u8]> {
        let prefix = b"/api/jobs/";
        let total = prefix.len() + remote_id.len();
        if total > out.len() {
            return None;
        }
        out[..prefix.len()].copy_from_slice(prefix);
        out[prefix.len()..total].copy_from_slice(remote_id);
        Some(&out[..total])
    }

    fn build_result_path<'a>(
        &self,
        remote_id: &'a [u8],
        out: &'a mut [u8; 192],
    ) -> Option<&'a [u8]> {
        let prefix = b"/api/jobs/";
        let suffix = b"/result";
        let total = prefix.len() + remote_id.len() + suffix.len();
        if total > out.len() {
            return None;
        }
        let mut pos = 0;
        out[pos..pos + prefix.len()].copy_from_slice(prefix);
        pos += prefix.len();
        out[pos..pos + remote_id.len()].copy_from_slice(remote_id);
        pos += remote_id.len();
        out[pos..pos + suffix.len()].copy_from_slice(suffix);
        pos += suffix.len();
        Some(&out[..pos])
    }

    fn parse_json_string_field<'a>(body: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
        let mut i = 0usize;
        while i + key.len() + 4 < body.len() {
            if body[i] == b'"' && body.get(i + 1..i + 1 + key.len()) == Some(key) {
                let mut j = i + 1 + key.len();
                if body.get(j) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                j += 1;
                while j < body.len() && (body[j] == b' ' || body[j] == b'\t' || body[j] == b'\n') {
                    j += 1;
                }
                if body.get(j) != Some(&b':') {
                    i += 1;
                    continue;
                }
                j += 1;
                while j < body.len() && (body[j] == b' ' || body[j] == b'\t' || body[j] == b'\n') {
                    j += 1;
                }
                if body.get(j) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                j += 1;
                let start = j;
                while j < body.len() {
                    if body[j] == b'"' && body[j.saturating_sub(1)] != b'\\' {
                        return Some(&body[start..j]);
                    }
                    j += 1;
                }
                return None;
            }
            i += 1;
        }
        None
    }

    fn parse_status(body: &[u8]) -> Option<CloudJobStatus> {
        let s = Self::parse_json_string_field(body, b"status")?;
        match s {
            b"QUEUED" | b"PENDING" => Some(CloudJobStatus::Queued),
            b"RUNNING" | b"IN_PROGRESS" => Some(CloudJobStatus::Running),
            b"COMPLETED" | b"DONE" => Some(CloudJobStatus::Completed),
            b"FAILED" | b"ERROR" => Some(CloudJobStatus::Failed),
            b"CANCELLED" | b"CANCELED" => Some(CloudJobStatus::Cancelled),
            _ => None,
        }
    }

    fn parse_remote_job_id<'a>(body: &'a [u8]) -> Option<&'a [u8]> {
        if let Some(id) = Self::parse_json_string_field(body, b"id") {
            return Some(id);
        }
        Self::parse_json_string_field(body, b"job_id")
    }

    fn parse_best_bitstring(body: &[u8]) -> Option<[u8; 128]> {
        let mut best_count = 0u64;
        let mut best = [0u8; 128];
        let mut best_len = 0usize;

        let mut i = 0usize;
        while i < body.len() {
            if body[i] != b'"' {
                i += 1;
                continue;
            }

            let key_start = i + 1;
            let mut key_end = key_start;
            while key_end < body.len() && body[key_end] != b'"' {
                key_end += 1;
            }
            if key_end >= body.len() {
                break;
            }

            let key = &body[key_start..key_end];
            let key_is_binary = !key.is_empty()
                && key.len() <= 128
                && key.iter().all(|&c| c == b'0' || c == b'1');
            if !key_is_binary {
                i = key_end + 1;
                continue;
            }

            let mut p = key_end + 1;
            while p < body.len() && (body[p] == b' ' || body[p] == b'\t' || body[p] == b'\n') {
                p += 1;
            }
            if p >= body.len() || body[p] != b':' {
                i = key_end + 1;
                continue;
            }
            p += 1;
            while p < body.len() && (body[p] == b' ' || body[p] == b'\t' || body[p] == b'\n') {
                p += 1;
            }

            let mut count = 0u64;
            let mut digits = 0usize;
            while p < body.len() && body[p].is_ascii_digit() {
                count = count
                    .saturating_mul(10)
                    .saturating_add((body[p] - b'0') as u64);
                digits += 1;
                p += 1;
            }

            if digits > 0 && count >= best_count {
                best_count = count;
                best_len = key.len();
                best[..best_len].copy_from_slice(key);
            }

            i = key_end + 1;
        }

        if best_len == 0 {
            None
        } else {
            best[best_len..].fill(0);
            Some(best)
        }
    }

    fn classical_from_best_bitstring(body: &[u8]) -> Option<ClassicalRegister> {
        let best = Self::parse_best_bitstring(body)?;
        let mut len = 0usize;
        while len < best.len() && (best[len] == b'0' || best[len] == b'1') {
            len += 1;
        }
        if len == 0 {
            return None;
        }

        let mut reg = ClassicalRegister::new();
        let mut i = len;
        while i > 0 {
            i -= 1;
            let bit = if best[i] == b'1' {
                Measurement::One
            } else {
                Measurement::Zero
            };
            if reg.push(bit).is_err() {
                break;
            }
        }
        Some(reg)
    }

    pub fn extract_histogram_json(body: &[u8]) -> Option<&[u8]> {
        // Find the "counts" field in the result JSON
        let mut i = 0usize;
        while i + 10 < body.len() {
            if body[i] == b'"' && body.get(i + 1..i + 7) == Some(b"counts") {
                if body.get(i + 7) == Some(&b'"') {
                    // Found "counts" field, now find the opening brace
                    let mut j = i + 8;
                    while j < body.len() && (body[j] == b' ' || body[j] == b'\t' || body[j] == b'\n' || body[j] == b':') {
                        j += 1;
                    }
                    // Extract the entire histogram object/map until we find the closing brace at the right nesting level
                    if body.get(j) == Some(&b'{') || body.get(j) == Some(&b'[') {
                        let start = j;
                        let mut depth = 1;
                        let mut k = j + 1;
                        let open_char = body[j];
                        let close_char = if open_char == b'{' { b'}' } else { b']' };
                        while k < body.len() && depth > 0 {
                            if body[k] == open_char {
                                depth += 1;
                            } else if body[k] == close_char {
                                depth -= 1;
                            }
                            if depth == 0 {
                                return Some(&body[start..k + 1]);
                            }
                            k += 1;
                        }
                    }
                }
            }
            i += 1;
        }
        None
    }

    fn submit_qasm(&mut self, qasm: &[u8], shots: u32, slot: usize) -> Result<CloudJobId, QuantumError> {
        let mut body = [0u8; MAX_QASM_LEN + 1024];
        let mut bpos = 0usize;

        let mut bpush = |s: &[u8]| -> Result<(), QuantumError> {
            if bpos + s.len() > body.len() {
                return Err(QuantumError::CircuitTooLarge);
            }
            body[bpos..bpos + s.len()].copy_from_slice(s);
            bpos += s.len();
            Ok(())
        };

        bpush(b"{\"program_id\":\"sampler\",\"params\":{\"circuits\":[\"")?;
        let mut qi = 0usize;
        while qi < qasm.len() {
            match qasm[qi] {
                b'"' => bpush(b"\\\"")?,
                b'\\' => bpush(b"\\\\")?,
                b'\n' => bpush(b"\\n")?,
                other => bpush(&[other])?,
            }
            qi += 1;
        }
        bpush(b"\"],\"backend\":\"")?;
        bpush(&self.backend_name[..self.backend_len])?;
        bpush(b"\",\"shots\":")?;

        let mut shot_digits = [0u8; 12];
        let mut n = if shots == 0 { 1 } else { shots };
        let mut dlen = 0usize;
        while n > 0 {
            shot_digits[dlen] = b'0' + (n % 10) as u8;
            n /= 10;
            dlen += 1;
        }
        if dlen == 0 {
            shot_digits[0] = b'1';
            dlen = 1;
        }
        let mut i = dlen;
        while i > 0 {
            i -= 1;
            bpush(&[shot_digits[i]])?;
        }

        bpush(b"}}")?;

        let mut auth_hdr = [0u8; MAX_TOKEN_LEN + 8];
        let auth_len = self.make_auth_header(&mut auth_hdr);
        let headers: [(&[u8], &[u8]); 3] = [
            (b"Content-Type", b"application/json"),
            (b"Authorization", &auth_hdr[..auth_len]),
            (b"IBM-Q-Instance", &self.instance[..self.instance_len]),
        ];

        let req = HttpRequest {
            method: HttpMethod::Post,
            host: &self.api_host[..self.api_host_len],
            path: b"/api/jobs",
            headers: &headers,
            body: &body[..bpos],
            out_buf: &mut self.resp_buf,
        };

        let resp_len = (self.transport)(req).map_err(Self::map_transport_error)?;
        let remote_id = Self::parse_remote_job_id(&self.resp_buf[..resp_len])
            .ok_or(QuantumError::HardwareError)?;
        if remote_id.len() > MAX_REMOTE_JOB_ID_LEN {
            return Err(QuantumError::CircuitTooLarge);
        }

        let local_id = self.job_counter;
        self.job_counter = self.job_counter.wrapping_add(1);

        self.jobs[slot].state = JobState::Submitted;
        self.jobs[slot].id = local_id;
        self.jobs[slot].shots = shots;
        self.jobs[slot].remote_job_id_len = remote_id.len();
        self.jobs[slot].remote_job_id[..remote_id.len()].copy_from_slice(remote_id);
        self.jobs[slot].result = ClassicalRegister::new();

        Ok(CloudJobId(local_id))
    }

    fn poll_remote_status(&mut self, slot: usize) -> Result<CloudJobStatus, QuantumError> {
        let remote_id_len = self.jobs[slot].remote_job_id_len;
        if remote_id_len == 0 {
            return Err(QuantumError::InvalidBackend);
        }
        let remote_id = &self.jobs[slot].remote_job_id[..remote_id_len];

        let mut path_buf = [0u8; 160];
        let path = self
            .build_job_path(remote_id, &mut path_buf)
            .ok_or(QuantumError::CircuitTooLarge)?;

        let mut auth_hdr = [0u8; MAX_TOKEN_LEN + 8];
        let auth_len = self.make_auth_header(&mut auth_hdr);
        let headers: [(&[u8], &[u8]); 2] = [
            (b"Authorization", &auth_hdr[..auth_len]),
            (b"IBM-Q-Instance", &self.instance[..self.instance_len]),
        ];

        let req = HttpRequest {
            method: HttpMethod::Get,
            host: &self.api_host[..self.api_host_len],
            path,
            headers: &headers,
            body: &[],
            out_buf: &mut self.resp_buf,
        };

        let resp_len = (self.transport)(req).map_err(Self::map_transport_error)?;
        let status =
            Self::parse_status(&self.resp_buf[..resp_len]).ok_or(QuantumError::HardwareError)?;
        match status {
            CloudJobStatus::Completed => self.jobs[slot].state = JobState::Completed,
            CloudJobStatus::Failed => self.jobs[slot].state = JobState::Failed,
            CloudJobStatus::Cancelled => self.jobs[slot].state = JobState::Cancelled,
            CloudJobStatus::Queued | CloudJobStatus::Running => {}
        }
        Ok(status)
    }
}

impl QuantumBackend for IbmQuantumBackend {
    fn allocate(&mut self, n: u16) -> Result<QubitRange, QuantumError> {
        if n > self.max_qubits {
            return Err(QuantumError::NotEnoughQubits);
        }
        Ok(QubitRange { base: 0, len: n })
    }

    fn apply(&mut self, _gate: Gate, _qubits: &[Qubit<QubitLive>]) -> Result<(), QuantumError> {
        Err(QuantumError::GateNotSupported)
    }

    fn measure(&mut self, _qubit: Qubit<QubitLive>) -> Result<Measurement, QuantumError> {
        Err(QuantumError::GateNotSupported)
    }

    fn execute_circuit(&mut self, circuit: &Circuit) -> Result<ClassicalRegister, QuantumError> {
        let job_id = self.submit_cloud_job(circuit)?;
        let mut attempts = 0u32;
        while attempts < 1500 {
            let (status, _) = self.poll_cloud_job(job_id)?;
            match status {
                CloudJobStatus::Completed => return self.fetch_cloud_result(job_id),
                CloudJobStatus::Failed | CloudJobStatus::Cancelled => {
                    return Err(QuantumError::HardwareError);
                }
                CloudJobStatus::Queued | CloudJobStatus::Running => {}
            }
            attempts += 1;
        }
        Err(QuantumError::CloudTimeout)
    }

    fn reset(&mut self) -> Result<(), QuantumError> {
        Ok(())
    }

    fn backend_info(&self) -> BackendInfo {
        BackendInfo {
            id: 0xC100,
            name: "ibm-quantum-cloud",
            backend_type: BackendType::Cloud,
            max_qubits: self.max_qubits,
            queue_depth: MAX_TRACKED_JOBS as u16,
            native_gates: NativeGateSet::empty(),
            connectivity_map: ConnectivityMap::empty(),
            gate_fidelities: GateFidelityTable::empty(),
            calibration: CalibrationWindow::ideal(),
        }
    }
}

impl CloudQpu for IbmQuantumBackend {
    fn provider(&self) -> CloudProvider {
        CloudProvider::IbmQuantum
    }

    fn estimate_cost(&self, circuit: &Circuit) -> Result<CostEstimateMilliQpuSeconds, QuantumError> {
        Ok((circuit.ops_len() as u64).saturating_mul(self.default_shots as u64))
    }

    fn submit_cloud_job(&mut self, circuit: &Circuit) -> Result<CloudJobId, QuantumError> {
        let qasm_len = self
            .serialize_qasm(circuit)
            .ok_or(QuantumError::CircuitTooLarge)?;
        let slot = self.alloc_job_slot().ok_or(QuantumError::BackendBusy)?;
        let mut qasm = [0u8; MAX_QASM_LEN];
        qasm[..qasm_len].copy_from_slice(&self.qasm_buf[..qasm_len]);
        self.submit_qasm(&qasm[..qasm_len], self.default_shots, slot)
    }

    fn poll_cloud_job(
        &mut self,
        id: CloudJobId,
    ) -> Result<(CloudJobStatus, CloudExecutionMetadata), QuantumError> {
        let slot = self
            .find_slot_by_local_id(id.0)
            .ok_or(QuantumError::InvalidBackend)?;
        let status = match self.jobs[slot].state {
            JobState::Completed => CloudJobStatus::Completed,
            JobState::Failed => CloudJobStatus::Failed,
            JobState::Cancelled => CloudJobStatus::Cancelled,
            JobState::Submitted => self.poll_remote_status(slot)?,
            JobState::Empty => return Err(QuantumError::InvalidBackend),
        };

        let meta = CloudExecutionMetadata {
            provider: CloudProvider::IbmQuantum,
            queue_position: 0,
            retries: 0,
            estimated_cost_milli_qpu_seconds: self.jobs[slot].shots as u64,
        };
        Ok((status, meta))
    }

    fn fetch_cloud_result(&mut self, id: CloudJobId) -> Result<ClassicalRegister, QuantumError> {
        let slot = self
            .find_slot_by_local_id(id.0)
            .ok_or(QuantumError::InvalidBackend)?;
        let remote_id_len = self.jobs[slot].remote_job_id_len;
        if remote_id_len == 0 {
            return Err(QuantumError::InvalidBackend);
        }
        let remote_id = &self.jobs[slot].remote_job_id[..remote_id_len];

        let mut path_buf = [0u8; 192];
        let path = self
            .build_result_path(remote_id, &mut path_buf)
            .ok_or(QuantumError::CircuitTooLarge)?;

        let mut auth_hdr = [0u8; MAX_TOKEN_LEN + 8];
        let auth_len = self.make_auth_header(&mut auth_hdr);
        let headers: [(&[u8], &[u8]); 2] = [
            (b"Authorization", &auth_hdr[..auth_len]),
            (b"IBM-Q-Instance", &self.instance[..self.instance_len]),
        ];

        let req = HttpRequest {
            method: HttpMethod::Get,
            host: &self.api_host[..self.api_host_len],
            path,
            headers: &headers,
            body: &[],
            out_buf: &mut self.resp_buf,
        };

        let resp_len = (self.transport)(req).map_err(Self::map_transport_error)?;
        let reg =
            Self::classical_from_best_bitstring(&self.resp_buf[..resp_len])
                .ok_or(QuantumError::HardwareError)?;

        // Extract and store histogram JSON for later access
        if let Some(histogram_json) = Self::extract_histogram_json(&self.resp_buf[..resp_len]) {
            let hlen = histogram_json.len().min(2048);
            self.last_histogram_json[..hlen].copy_from_slice(&histogram_json[..hlen]);
            self.last_histogram_len = hlen;
        }

        self.jobs[slot].result = reg;
        self.jobs[slot].state = JobState::Empty;
        Ok(reg)
    }

    fn cancel_cloud_job(&mut self, id: CloudJobId) -> Result<(), QuantumError> {
        let slot = self
            .find_slot_by_local_id(id.0)
            .ok_or(QuantumError::InvalidBackend)?;
        self.jobs[slot].state = JobState::Cancelled;
        Ok(())
    }
}

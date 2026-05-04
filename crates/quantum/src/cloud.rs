use crate::backend::{QuantumBackend, QuantumError};
use crate::circuit::{Circuit, ClassicalRegister};

pub type CostEstimateMilliQpuSeconds = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CloudProvider {
    IbmQuantum = 0,
    AmazonBraket = 1,
    AzureQuantum = 2,
    GoogleQuantum = 3,
    Custom = 255,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudJobId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudJobStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudExecutionMetadata {
    pub provider: CloudProvider,
    pub queue_position: u32,
    pub retries: u8,
    pub estimated_cost_milli_qpu_seconds: CostEstimateMilliQpuSeconds,
}

/// Cloud backend contract layered on top of the core backend trait.
///
/// Transport details (TLS, REST, auth tokens, retries) live in provider
/// adapters; this trait keeps the kernel-facing scheduling contract uniform.
pub trait CloudQpu: QuantumBackend {
    fn provider(&self) -> CloudProvider;

    fn estimate_cost(
        &self,
        circuit: &Circuit,
    ) -> Result<CostEstimateMilliQpuSeconds, QuantumError>;

    fn submit_cloud_job(&mut self, circuit: &Circuit) -> Result<CloudJobId, QuantumError>;

    fn poll_cloud_job(
        &mut self,
        id: CloudJobId,
    ) -> Result<(CloudJobStatus, CloudExecutionMetadata), QuantumError>;

    fn fetch_cloud_result(&mut self, id: CloudJobId) -> Result<ClassicalRegister, QuantumError>;

    fn cancel_cloud_job(&mut self, id: CloudJobId) -> Result<(), QuantumError>;
}
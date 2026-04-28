//! VeerOS Graph Fabric core (Phase B, in-process).
//!
//! Provides a mutable graph store G = (V, E, W):
//! - vertices: typed graph entities (usr/dev/fld/svc/...)
//! - edges: directed relationships (trust/membership/reachability/...)
//! - weights: latency, trust, cost, affinity, load

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use vas::{canonicalize, AddressType, VasAddress};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VertexKind {
    User,
    Device,
    Fold,
    Aura,
    Service,
    Vault,
    Agent,
    Zone,
    Node,
    Event,
}

impl VertexKind {
    fn from_address_type(t: AddressType) -> Self {
        match t {
            AddressType::User => VertexKind::User,
            AddressType::Device => VertexKind::Device,
            AddressType::Fold => VertexKind::Fold,
            AddressType::Aura => VertexKind::Aura,
            AddressType::Service => VertexKind::Service,
            AddressType::Vault => VertexKind::Vault,
            AddressType::Agent => VertexKind::Agent,
            AddressType::Zone => VertexKind::Zone,
            AddressType::Node => VertexKind::Node,
            AddressType::Event => VertexKind::Event,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Trust,
    Membership,
    Reachability,
    Capability,
    Replication,
    Affinity,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeWeights {
    pub latency: f32,
    pub trust: f32,
    pub cost: f32,
    pub affinity: f32,
    pub load: f32,
}

impl Default for EdgeWeights {
    fn default() -> Self {
        Self {
            latency: 1.0,
            trust: 0.0,
            cost: 0.0,
            affinity: 0.0,
            load: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WeightPatch {
    pub latency: Option<f32>,
    pub trust: Option<f32>,
    pub cost: Option<f32>,
    pub affinity: Option<f32>,
    pub load: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vertex {
    pub id: String,
    pub kind: VertexKind,
    #[serde(default)]
    pub attrs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub weights: EdgeWeights,
    #[serde(default)]
    pub attrs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EdgeKey {
    from: String,
    to: String,
    kind: EdgeKind,
}

impl EdgeKey {
    fn new(from: &str, to: &str, kind: EdgeKind) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            kind,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    InvalidVertexId,
    VertexKindMismatch,
    MissingVertex,
    InvalidWeight,
    InvalidAddressType,
    QuotaNotFound,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetrySignal {
    /// Live metric update for a directed relationship edge.
    EdgeMetrics {
        from: String,
        to: String,
        kind: EdgeKind,
        latency: Option<f32>,
        trust: Option<f32>,
        cost: Option<f32>,
        affinity: Option<f32>,
        load: Option<f32>,
    },
    /// Live capability/locality update for a node vertex.
    NodeCapacity {
        node: String,
        cpu_available: Option<f32>,
        gpu_available: Option<f32>,
        locality: Option<String>,
    },
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TelemetryStats {
    pub applied: u64,
    pub rejected: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SolverWeights {
    pub alpha_latency: f32,
    pub beta_cost: f32,
    pub gamma_trust: f32,
    pub delta_affinity: f32,
}

impl Default for SolverWeights {
    fn default() -> Self {
        Self {
            alpha_latency: 1.0,
            beta_cost: 1.0,
            gamma_trust: 1.0,
            delta_affinity: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SolverChoice {
    pub target: String,
    pub edge_kind: EdgeKind,
    pub score: f32,
    pub weights: EdgeWeights,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacementRequest {
    pub source: String,
    pub edge_kind: EdgeKind,
    pub target_kind: Option<VertexKind>,
    #[serde(default)]
    pub coefficients: SolverWeights,
    /// Minimum objective improvement required to switch away from
    /// `current_target` during re-optimization.
    #[serde(default)]
    pub hysteresis_margin: f32,
    /// Existing placement target to evaluate for re-optimization.
    #[serde(default)]
    pub current_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacementDecision {
    pub source: String,
    pub selected_target: Option<String>,
    pub selected_score: Option<f32>,
    pub previous_target: Option<String>,
    pub previous_score: Option<f32>,
    pub switched: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuraResourceQuota {
    pub aura: String,
    pub gpu_slices: u32,
    pub storage_mb: u64,
    pub bandwidth_mbps: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRequest {
    pub aura: String,
    pub fold: String,
    pub gpu_slices: u32,
    pub storage_mb: u64,
    pub bandwidth_mbps: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub gpu_slices: u32,
    pub storage_mb: u64,
    pub bandwidth_mbps: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceAllocationDecision {
    pub aura: String,
    pub fold: String,
    pub granted: bool,
    pub reason: String,
    pub usage: ResourceUsage,
    pub remaining: ResourceUsage,
}

#[derive(Debug, Default)]
pub struct AuraResourceScheduler {
    quotas: HashMap<String, AuraResourceQuota>,
    usage: HashMap<String, ResourceUsage>,
    allocations: HashMap<String, ResourceUsage>,
}

impl AuraResourceScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_quota(&mut self, quota: AuraResourceQuota) -> Result<(), GraphError> {
        let aura = canonical_typed_addr(&quota.aura, AddressType::Aura)?;
        self.quotas.insert(
            aura.clone(),
            AuraResourceQuota {
                aura,
                gpu_slices: quota.gpu_slices,
                storage_mb: quota.storage_mb,
                bandwidth_mbps: quota.bandwidth_mbps,
            },
        );
        self.usage.entry(canonical_typed_addr(&quota.aura, AddressType::Aura)?).or_default();
        Ok(())
    }

    pub fn usage_for_aura(&self, aura_raw: &str) -> Result<ResourceUsage, GraphError> {
        let aura = canonical_typed_addr(aura_raw, AddressType::Aura)?;
        Ok(self.usage.get(&aura).cloned().unwrap_or_default())
    }

    pub fn allocate(&mut self, request: ResourceRequest) -> Result<ResourceAllocationDecision, GraphError> {
        let aura = canonical_typed_addr(&request.aura, AddressType::Aura)?;
        let fold = canonical_typed_addr(&request.fold, AddressType::Fold)?;

        let Some(quota) = self.quotas.get(&aura).cloned() else {
            return Err(GraphError::QuotaNotFound);
        };

        let key = allocation_key(&aura, &fold);
        let previous = self.allocations.get(&key).cloned().unwrap_or_default();

        let used_before = self.usage.get(&aura).cloned().unwrap_or_default();
        let used_after = ResourceUsage {
            gpu_slices: used_before
                .gpu_slices
                .saturating_sub(previous.gpu_slices)
                .saturating_add(request.gpu_slices),
            storage_mb: used_before
                .storage_mb
                .saturating_sub(previous.storage_mb)
                .saturating_add(request.storage_mb),
            bandwidth_mbps: used_before
                .bandwidth_mbps
                .saturating_sub(previous.bandwidth_mbps)
                .saturating_add(request.bandwidth_mbps),
        };

        let within_quota = used_after.gpu_slices <= quota.gpu_slices
            && used_after.storage_mb <= quota.storage_mb
            && used_after.bandwidth_mbps <= quota.bandwidth_mbps;

        if within_quota {
            self.allocations.insert(
                key,
                ResourceUsage {
                    gpu_slices: request.gpu_slices,
                    storage_mb: request.storage_mb,
                    bandwidth_mbps: request.bandwidth_mbps,
                },
            );
            self.usage.insert(aura.clone(), used_after.clone());

            Ok(ResourceAllocationDecision {
                aura,
                fold,
                granted: true,
                reason: "allocated".to_string(),
                usage: used_after.clone(),
                remaining: remaining_for(&quota, &used_after),
            })
        } else {
            Ok(ResourceAllocationDecision {
                aura,
                fold,
                granted: false,
                reason: "quota_exceeded".to_string(),
                usage: used_before.clone(),
                remaining: remaining_for(&quota, &used_before),
            })
        }
    }

    pub fn release(&mut self, aura_raw: &str, fold_raw: &str) -> Result<bool, GraphError> {
        let aura = canonical_typed_addr(aura_raw, AddressType::Aura)?;
        let fold = canonical_typed_addr(fold_raw, AddressType::Fold)?;
        let key = allocation_key(&aura, &fold);

        let Some(existing) = self.allocations.remove(&key) else {
            return Ok(false);
        };

        let mut used = self.usage.get(&aura).cloned().unwrap_or_default();
        used.gpu_slices = used.gpu_slices.saturating_sub(existing.gpu_slices);
        used.storage_mb = used.storage_mb.saturating_sub(existing.storage_mb);
        used.bandwidth_mbps = used.bandwidth_mbps.saturating_sub(existing.bandwidth_mbps);
        self.usage.insert(aura, used);
        Ok(true)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyClause {
    RequireTargetKind(VertexKind),
    RequireVertexAttrEq { key: String, value: String },
    RequireEdgeAttrEq { key: String, value: String },
    MinTrust(f32),
    MaxLatency(f32),
    MaxLoad(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    All,
    Any,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicySet {
    pub mode: PolicyMode,
    #[serde(default)]
    pub clauses: Vec<PolicyClause>,
}

impl Default for PolicySet {
    fn default() -> Self {
        Self {
            mode: PolicyMode::All,
            clauses: Vec::new(),
        }
    }
}

impl PolicySet {
    pub fn allows(&self, target_vertex: &Vertex, edge: &Edge) -> bool {
        if self.clauses.is_empty() {
            return true;
        }

        match self.mode {
            PolicyMode::All => self
                .clauses
                .iter()
                .all(|c| clause_allows(c, target_vertex, edge)),
            PolicyMode::Any => self
                .clauses
                .iter()
                .any(|c| clause_allows(c, target_vertex, edge)),
        }
    }
}

#[derive(Debug, Default)]
pub struct GraphCore {
    vertices: HashMap<String, Vertex>,
    edges: HashMap<EdgeKey, Edge>,
    telemetry_stats: TelemetryStats,
}

impl GraphCore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_vertex(
        &mut self,
        raw_id: &str,
        kind: Option<VertexKind>,
        attrs: BTreeMap<String, String>,
    ) -> Result<String, GraphError> {
        let id = canonicalize(raw_id).map_err(|_| GraphError::InvalidVertexId)?;
        let parsed = VasAddress::parse(&id).map_err(|_| GraphError::InvalidVertexId)?;
        let inferred_kind = VertexKind::from_address_type(parsed.kind);
        if let Some(k) = kind {
            if k != inferred_kind {
                return Err(GraphError::VertexKindMismatch);
            }
        }

        self.vertices.insert(
            id.clone(),
            Vertex {
                id: id.clone(),
                kind: inferred_kind,
                attrs,
            },
        );

        Ok(id)
    }

    pub fn get_vertex(&self, id: &str) -> Option<&Vertex> {
        self.vertices.get(id)
    }

    pub fn upsert_edge(
        &mut self,
        from_raw: &str,
        to_raw: &str,
        kind: EdgeKind,
        weights: EdgeWeights,
        attrs: BTreeMap<String, String>,
    ) -> Result<(), GraphError> {
        validate_weights(weights)?;

        let from = canonicalize(from_raw).map_err(|_| GraphError::InvalidVertexId)?;
        let to = canonicalize(to_raw).map_err(|_| GraphError::InvalidVertexId)?;
        if !self.vertices.contains_key(&from) || !self.vertices.contains_key(&to) {
            return Err(GraphError::MissingVertex);
        }

        let key = EdgeKey::new(&from, &to, kind);
        self.edges.insert(
            key,
            Edge {
                from,
                to,
                kind,
                weights,
                attrs,
            },
        );
        Ok(())
    }

    pub fn update_edge_weights(
        &mut self,
        from_raw: &str,
        to_raw: &str,
        kind: EdgeKind,
        patch: WeightPatch,
    ) -> Result<(), GraphError> {
        let from = canonicalize(from_raw).map_err(|_| GraphError::InvalidVertexId)?;
        let to = canonicalize(to_raw).map_err(|_| GraphError::InvalidVertexId)?;
        let key = EdgeKey::new(&from, &to, kind);
        let edge = self.edges.get_mut(&key).ok_or(GraphError::MissingVertex)?;

        if let Some(v) = patch.latency {
            edge.weights.latency = v;
        }
        if let Some(v) = patch.trust {
            edge.weights.trust = v;
        }
        if let Some(v) = patch.cost {
            edge.weights.cost = v;
        }
        if let Some(v) = patch.affinity {
            edge.weights.affinity = v;
        }
        if let Some(v) = patch.load {
            edge.weights.load = v;
        }

        validate_weights(edge.weights)?;
        Ok(())
    }

    pub fn neighbors(
        &self,
        id_raw: &str,
        direction: Direction,
        kind_filter: Option<EdgeKind>,
    ) -> Result<Vec<&Edge>, GraphError> {
        let id = canonicalize(id_raw).map_err(|_| GraphError::InvalidVertexId)?;
        if !self.vertices.contains_key(&id) {
            return Err(GraphError::MissingVertex);
        }

        let mut out = Vec::new();
        for edge in self.edges.values() {
            if let Some(k) = kind_filter {
                if edge.kind != k {
                    continue;
                }
            }

            match direction {
                Direction::Out if edge.from == id => out.push(edge),
                Direction::In if edge.to == id => out.push(edge),
                Direction::Both if edge.from == id || edge.to == id => out.push(edge),
                _ => {}
            }
        }

        Ok(out)
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Solve `argmin_x (alpha*L + beta*(C+load) - gamma*T - delta*A)` over
    /// outgoing edges from `source_raw` filtered by `edge_kind`, optionally
    /// constrained to destination vertex kind.
    pub fn solve_best_target(
        &self,
        source_raw: &str,
        edge_kind: EdgeKind,
        target_kind: Option<VertexKind>,
        coefficients: SolverWeights,
    ) -> Result<Option<SolverChoice>, GraphError> {
        let source = canonicalize(source_raw).map_err(|_| GraphError::InvalidVertexId)?;
        if !self.vertices.contains_key(&source) {
            return Err(GraphError::MissingVertex);
        }

        let mut best: Option<SolverChoice> = None;

        for edge in self.edges.values() {
            if edge.from != source || edge.kind != edge_kind {
                continue;
            }

            if let Some(expected_kind) = target_kind {
                let Some(v) = self.vertices.get(&edge.to) else {
                    continue;
                };
                if v.kind != expected_kind {
                    continue;
                }
            }

            let score = coefficients.alpha_latency * edge.weights.latency
                + coefficients.beta_cost * (edge.weights.cost + edge.weights.load)
                - coefficients.gamma_trust * edge.weights.trust
                - coefficients.delta_affinity * edge.weights.affinity;

            let candidate = SolverChoice {
                target: edge.to.clone(),
                edge_kind,
                score,
                weights: edge.weights,
            };

            match &best {
                None => best = Some(candidate),
                Some(cur) => {
                    if candidate.score < cur.score
                        || ((candidate.score - cur.score).abs() <= f32::EPSILON
                            && candidate.target < cur.target)
                    {
                        best = Some(candidate);
                    }
                }
            }
        }

        Ok(best)
    }

    /// Policy-aware solver path. Policy clauses are evaluated as hard
    /// constraints before objective scoring.
    pub fn solve_best_target_with_policy(
        &self,
        source_raw: &str,
        edge_kind: EdgeKind,
        target_kind: Option<VertexKind>,
        coefficients: SolverWeights,
        policy: &PolicySet,
    ) -> Result<Option<SolverChoice>, GraphError> {
        let source = canonicalize(source_raw).map_err(|_| GraphError::InvalidVertexId)?;
        if !self.vertices.contains_key(&source) {
            return Err(GraphError::MissingVertex);
        }

        let mut best: Option<SolverChoice> = None;

        for edge in self.edges.values() {
            if edge.from != source || edge.kind != edge_kind {
                continue;
            }

            let Some(v) = self.vertices.get(&edge.to) else {
                continue;
            };
            if let Some(expected_kind) = target_kind {
                if v.kind != expected_kind {
                    continue;
                }
            }
            if !policy.allows(v, edge) {
                continue;
            }

            let score = coefficients.alpha_latency * edge.weights.latency
                + coefficients.beta_cost * (edge.weights.cost + edge.weights.load)
                - coefficients.gamma_trust * edge.weights.trust
                - coefficients.delta_affinity * edge.weights.affinity;

            let candidate = SolverChoice {
                target: edge.to.clone(),
                edge_kind,
                score,
                weights: edge.weights,
            };

            match &best {
                None => best = Some(candidate),
                Some(cur) => {
                    if candidate.score < cur.score
                        || ((candidate.score - cur.score).abs() <= f32::EPSILON
                            && candidate.target < cur.target)
                    {
                        best = Some(candidate);
                    }
                }
            }
        }

        Ok(best)
    }

    pub fn telemetry_stats(&self) -> TelemetryStats {
        self.telemetry_stats
    }

    /// Graph-driven workload placement with continuous re-optimization and
    /// hysteresis to avoid target flapping.
    pub fn optimize_placement_with_hysteresis(
        &self,
        request: &PlacementRequest,
    ) -> Result<PlacementDecision, GraphError> {
        if request.hysteresis_margin < 0.0 {
            return Err(GraphError::InvalidWeight);
        }

        let source = canonicalize(&request.source).map_err(|_| GraphError::InvalidVertexId)?;
        if !self.vertices.contains_key(&source) {
            return Err(GraphError::MissingVertex);
        }

        let best = self.solve_best_target(
            &source,
            request.edge_kind,
            request.target_kind,
            request.coefficients,
        )?;

        let current_target = request
            .current_target
            .as_deref()
            .map(canonicalize)
            .transpose()
            .map_err(|_| GraphError::InvalidVertexId)?;
        let previous_score = current_target
            .as_deref()
            .and_then(|t| self.score_for_target(&source, t, request));

        let decision = match (current_target.clone(), previous_score, best) {
            (None, _, Some(best)) => PlacementDecision {
                source,
                selected_target: Some(best.target),
                selected_score: Some(best.score),
                previous_target: None,
                previous_score: None,
                switched: true,
                reason: "initial_placement".to_string(),
            },
            (None, _, None) => PlacementDecision {
                source,
                selected_target: None,
                selected_score: None,
                previous_target: None,
                previous_score: None,
                switched: false,
                reason: "no_candidate".to_string(),
            },
            (Some(current), Some(prev_score), Some(best)) if current == best.target => PlacementDecision {
                source,
                selected_target: Some(current.clone()),
                selected_score: Some(prev_score),
                previous_target: Some(current),
                previous_score: Some(prev_score),
                switched: false,
                reason: "current_is_optimal".to_string(),
            },
            (Some(current), Some(prev_score), Some(best)) => {
                let improvement = prev_score - best.score;
                if improvement > request.hysteresis_margin {
                    PlacementDecision {
                        source,
                        selected_target: Some(best.target),
                        selected_score: Some(best.score),
                        previous_target: Some(current),
                        previous_score: Some(prev_score),
                        switched: true,
                        reason: "switch_better_than_hysteresis".to_string(),
                    }
                } else {
                    PlacementDecision {
                        source,
                        selected_target: Some(current.clone()),
                        selected_score: Some(prev_score),
                        previous_target: Some(current),
                        previous_score: Some(prev_score),
                        switched: false,
                        reason: "stay_due_to_hysteresis".to_string(),
                    }
                }
            }
            (Some(current), None, Some(best)) => PlacementDecision {
                source,
                selected_target: Some(best.target),
                selected_score: Some(best.score),
                previous_target: Some(current),
                previous_score: None,
                switched: true,
                reason: "previous_target_unavailable".to_string(),
            },
            (Some(current), Some(prev_score), None) => PlacementDecision {
                source,
                selected_target: Some(current.clone()),
                selected_score: Some(prev_score),
                previous_target: Some(current),
                previous_score: Some(prev_score),
                switched: false,
                reason: "no_better_candidate".to_string(),
            },
            (Some(current), None, None) => PlacementDecision {
                source,
                selected_target: Some(current.clone()),
                selected_score: None,
                previous_target: Some(current),
                previous_score: None,
                switched: false,
                reason: "no_candidate_keep_current".to_string(),
            },
        };

        Ok(decision)
    }

    pub fn ingest_signal(&mut self, signal: TelemetrySignal) -> Result<(), GraphError> {
        let result = match signal {
            TelemetrySignal::EdgeMetrics {
                from,
                to,
                kind,
                latency,
                trust,
                cost,
                affinity,
                load,
            } => self.update_edge_weights(
                &from,
                &to,
                kind,
                WeightPatch {
                    latency,
                    trust,
                    cost,
                    affinity,
                    load,
                },
            ),
            TelemetrySignal::NodeCapacity {
                node,
                cpu_available,
                gpu_available,
                locality,
            } => self.apply_node_capacity(&node, cpu_available, gpu_available, locality),
        };

        match result {
            Ok(()) => {
                self.telemetry_stats.applied += 1;
                Ok(())
            }
            Err(e) => {
                self.telemetry_stats.rejected += 1;
                Err(e)
            }
        }
    }

    pub fn ingest_batch<I>(&mut self, signals: I) -> (u64, u64)
    where
        I: IntoIterator<Item = TelemetrySignal>,
    {
        let mut applied = 0u64;
        let mut rejected = 0u64;

        for signal in signals {
            if self.ingest_signal(signal).is_ok() {
                applied += 1;
            } else {
                rejected += 1;
            }
        }

        (applied, rejected)
    }

    fn apply_node_capacity(
        &mut self,
        node_raw: &str,
        cpu_available: Option<f32>,
        gpu_available: Option<f32>,
        locality: Option<String>,
    ) -> Result<(), GraphError> {
        let id = canonicalize(node_raw).map_err(|_| GraphError::InvalidVertexId)?;
        let vertex = self.vertices.get_mut(&id).ok_or(GraphError::MissingVertex)?;

        if vertex.kind != VertexKind::Node {
            return Err(GraphError::VertexKindMismatch);
        }

        if let Some(cpu) = cpu_available {
            if cpu < 0.0 {
                return Err(GraphError::InvalidWeight);
            }
            vertex.attrs.insert("cpu_available".to_string(), format!("{cpu:.4}"));
        }

        if let Some(gpu) = gpu_available {
            if gpu < 0.0 {
                return Err(GraphError::InvalidWeight);
            }
            vertex.attrs.insert("gpu_available".to_string(), format!("{gpu:.4}"));
        }

        if let Some(loc) = locality {
            let trimmed = loc.trim();
            if !trimmed.is_empty() {
                vertex
                    .attrs
                    .insert("locality".to_string(), trimmed.to_string());
            }
        }

        Ok(())
    }

    fn score_for_target(
        &self,
        source: &str,
        target: &str,
        request: &PlacementRequest,
    ) -> Option<f32> {
        let edge = self
            .edges
            .get(&EdgeKey::new(source, target, request.edge_kind))?;
        if let Some(expected) = request.target_kind {
            let v = self.vertices.get(target)?;
            if v.kind != expected {
                return None;
            }
        }

        Some(
            request.coefficients.alpha_latency * edge.weights.latency
                + request.coefficients.beta_cost * (edge.weights.cost + edge.weights.load)
                - request.coefficients.gamma_trust * edge.weights.trust
                - request.coefficients.delta_affinity * edge.weights.affinity,
        )
    }
}

fn validate_weights(weights: EdgeWeights) -> Result<(), GraphError> {
    if !(weights.latency >= 0.0
        && weights.cost >= 0.0
        && weights.load >= 0.0
        && weights.trust >= 0.0
        && weights.affinity >= 0.0)
    {
        return Err(GraphError::InvalidWeight);
    }
    Ok(())
}

fn canonical_typed_addr(input: &str, expected: AddressType) -> Result<String, GraphError> {
    let canonical = canonicalize(input).map_err(|_| GraphError::InvalidVertexId)?;
    let addr = VasAddress::parse(&canonical).map_err(|_| GraphError::InvalidVertexId)?;
    if addr.kind != expected {
        return Err(GraphError::InvalidAddressType);
    }
    Ok(canonical)
}

fn allocation_key(aura: &str, fold: &str) -> String {
    format!("{}|{}", aura, fold)
}

fn remaining_for(quota: &AuraResourceQuota, usage: &ResourceUsage) -> ResourceUsage {
    ResourceUsage {
        gpu_slices: quota.gpu_slices.saturating_sub(usage.gpu_slices),
        storage_mb: quota.storage_mb.saturating_sub(usage.storage_mb),
        bandwidth_mbps: quota.bandwidth_mbps.saturating_sub(usage.bandwidth_mbps),
    }
}

fn clause_allows(clause: &PolicyClause, target_vertex: &Vertex, edge: &Edge) -> bool {
    match clause {
        PolicyClause::RequireTargetKind(kind) => target_vertex.kind == *kind,
        PolicyClause::RequireVertexAttrEq { key, value } => {
            target_vertex.attrs.get(key).map(|v| v == value).unwrap_or(false)
        }
        PolicyClause::RequireEdgeAttrEq { key, value } => {
            edge.attrs.get(key).map(|v| v == value).unwrap_or(false)
        }
        PolicyClause::MinTrust(v) => edge.weights.trust >= *v,
        PolicyClause::MaxLatency(v) => edge.weights.latency <= *v,
        PolicyClause::MaxLoad(v) => edge.weights.load <= *v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_vertex_canonicalizes_and_infers_kind() {
        let mut g = GraphCore::new();
        let id = g
            .upsert_vertex(" SVC{Render,Company,Live}", None, BTreeMap::new())
            .unwrap();

        assert_eq!(id, "svc{render,company,live}");
        let v = g.get_vertex(&id).unwrap();
        assert_eq!(v.kind, VertexKind::Service);
    }

    #[test]
    fn kind_mismatch_is_rejected() {
        let mut g = GraphCore::new();
        let err = g
            .upsert_vertex(
                "svc{render,company,live}",
                Some(VertexKind::Aura),
                BTreeMap::new(),
            )
            .unwrap_err();
        assert_eq!(err, GraphError::VertexKindMismatch);
    }

    #[test]
    fn edge_requires_existing_vertices() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();

        let err = g
            .upsert_edge(
                "svc{render,company,live}",
                "nod{edge-a,zone-1,ready}",
                EdgeKind::Reachability,
                EdgeWeights::default(),
                BTreeMap::new(),
            )
            .unwrap_err();
        assert_eq!(err, GraphError::MissingVertex);
    }

    #[test]
    fn upsert_and_query_neighbors() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();

        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 8.0,
                trust: 0.9,
                cost: 0.2,
                affinity: 0.6,
                load: 0.4,
            },
            BTreeMap::new(),
        )
        .unwrap();

        let out = g
            .neighbors(
                "svc{render,company,live}",
                Direction::Out,
                Some(EdgeKind::Reachability),
            )
            .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to, "nod{edge-a,zone-1,ready}");
    }

    #[test]
    fn update_edge_weights_rewrites_values() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();

        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights::default(),
            BTreeMap::new(),
        )
        .unwrap();

        g.update_edge_weights(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            WeightPatch {
                latency: Some(3.0),
                trust: Some(0.95),
                cost: None,
                affinity: None,
                load: Some(0.1),
            },
        )
        .unwrap();

        let edge = g
            .neighbors(
                "svc{render,company,live}",
                Direction::Out,
                Some(EdgeKind::Reachability),
            )
            .unwrap()
            .pop()
            .unwrap()
            .clone();
        assert_eq!(edge.weights.latency, 3.0);
        assert_eq!(edge.weights.trust, 0.95);
        assert_eq!(edge.weights.load, 0.1);
    }

    #[test]
    fn negative_weight_is_rejected() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();

        let err = g
            .upsert_edge(
                "svc{render,company,live}",
                "nod{edge-a,zone-1,ready}",
                EdgeKind::Reachability,
                EdgeWeights {
                    latency: -1.0,
                    ..EdgeWeights::default()
                },
                BTreeMap::new(),
            )
            .unwrap_err();
        assert_eq!(err, GraphError::InvalidWeight);
    }

    #[test]
    fn telemetry_edge_metrics_updates_existing_edge() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights::default(),
            BTreeMap::new(),
        )
        .unwrap();

        g.ingest_signal(TelemetrySignal::EdgeMetrics {
            from: "svc{render,company,live}".into(),
            to: "nod{edge-a,zone-1,ready}".into(),
            kind: EdgeKind::Reachability,
            latency: Some(5.0),
            trust: Some(0.8),
            cost: None,
            affinity: None,
            load: Some(0.2),
        })
        .unwrap();

        let edge = g
            .neighbors(
                "svc{render,company,live}",
                Direction::Out,
                Some(EdgeKind::Reachability),
            )
            .unwrap()[0]
            .clone();

        assert_eq!(edge.weights.latency, 5.0);
        assert_eq!(edge.weights.trust, 0.8);
        assert_eq!(edge.weights.load, 0.2);
        assert_eq!(g.telemetry_stats().applied, 1);
        assert_eq!(g.telemetry_stats().rejected, 0);
    }

    #[test]
    fn telemetry_node_capacity_updates_attrs() {
        let mut g = GraphCore::new();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();

        g.ingest_signal(TelemetrySignal::NodeCapacity {
            node: "nod{edge-a,zone-1,ready}".into(),
            cpu_available: Some(0.42),
            gpu_available: Some(0.75),
            locality: Some("zone-1/rack-2".into()),
        })
        .unwrap();

        let v = g.get_vertex("nod{edge-a,zone-1,ready}").unwrap();
        assert_eq!(v.attrs.get("cpu_available").map(String::as_str), Some("0.4200"));
        assert_eq!(v.attrs.get("gpu_available").map(String::as_str), Some("0.7500"));
        assert_eq!(v.attrs.get("locality").map(String::as_str), Some("zone-1/rack-2"));
    }

    #[test]
    fn telemetry_batch_reports_applied_and_rejected() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights::default(),
            BTreeMap::new(),
        )
        .unwrap();

        let (applied, rejected) = g.ingest_batch(vec![
            TelemetrySignal::EdgeMetrics {
                from: "svc{render,company,live}".into(),
                to: "nod{edge-a,zone-1,ready}".into(),
                kind: EdgeKind::Reachability,
                latency: Some(7.0),
                trust: None,
                cost: None,
                affinity: None,
                load: None,
            },
            TelemetrySignal::EdgeMetrics {
                from: "svc{render,company,live}".into(),
                to: "nod{missing,zone-1,ready}".into(),
                kind: EdgeKind::Reachability,
                latency: Some(7.0),
                trust: None,
                cost: None,
                affinity: None,
                load: None,
            },
        ]);

        assert_eq!(applied, 1);
        assert_eq!(rejected, 1);
        assert_eq!(g.telemetry_stats().applied, 1);
        assert_eq!(g.telemetry_stats().rejected, 1);
    }

    #[test]
    fn solver_picks_lowest_objective_score() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-b,zone-1,ready}", None, BTreeMap::new())
            .unwrap();

        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 9.0,
                trust: 0.9,
                cost: 0.2,
                affinity: 0.9,
                load: 0.4,
            },
            BTreeMap::new(),
        )
        .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-b,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 6.0,
                trust: 0.2,
                cost: 0.1,
                affinity: 0.1,
                load: 0.3,
            },
            BTreeMap::new(),
        )
        .unwrap();

        let choice = g
            .solve_best_target(
                "svc{render,company,live}",
                EdgeKind::Reachability,
                Some(VertexKind::Node),
                SolverWeights {
                    alpha_latency: 1.0,
                    beta_cost: 1.0,
                    gamma_trust: 1.0,
                    delta_affinity: 1.0,
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(choice.target, "nod{edge-b,zone-1,ready}");
    }

    #[test]
    fn solver_honors_target_kind_filter() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("fld{worker,gpu,warm}", None, BTreeMap::new())
            .unwrap();

        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights::default(),
            BTreeMap::new(),
        )
        .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "fld{worker,gpu,warm}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 0.1,
                ..EdgeWeights::default()
            },
            BTreeMap::new(),
        )
        .unwrap();

        let node_choice = g
            .solve_best_target(
                "svc{render,company,live}",
                EdgeKind::Reachability,
                Some(VertexKind::Node),
                SolverWeights::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(node_choice.target, "nod{edge-a,zone-1,ready}");

        let fold_choice = g
            .solve_best_target(
                "svc{render,company,live}",
                EdgeKind::Reachability,
                Some(VertexKind::Fold),
                SolverWeights::default(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(fold_choice.target, "fld{worker,gpu,warm}");
    }

    #[test]
    fn solver_returns_none_when_no_candidate_matches() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();

        let out = g
            .solve_best_target(
                "svc{render,company,live}",
                EdgeKind::Reachability,
                Some(VertexKind::Node),
                SolverWeights::default(),
            )
            .unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn policy_all_mode_filters_candidates_before_scoring() {
        let mut g = GraphCore::new();
        let mut zone1 = BTreeMap::new();
        zone1.insert("zone".into(), "z1".into());
        let mut zone2 = BTreeMap::new();
        zone2.insert("zone".into(), "z2".into());

        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, zone1)
            .unwrap();
        g.upsert_vertex("nod{edge-b,zone-2,ready}", None, zone2)
            .unwrap();

        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 10.0,
                trust: 0.9,
                cost: 0.3,
                affinity: 0.4,
                load: 0.2,
            },
            BTreeMap::new(),
        )
        .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-b,zone-2,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 3.0,
                trust: 0.2,
                cost: 0.1,
                affinity: 0.1,
                load: 0.1,
            },
            BTreeMap::new(),
        )
        .unwrap();

        let policy = PolicySet {
            mode: PolicyMode::All,
            clauses: vec![
                PolicyClause::RequireTargetKind(VertexKind::Node),
                PolicyClause::RequireVertexAttrEq {
                    key: "zone".into(),
                    value: "z1".into(),
                },
                PolicyClause::MinTrust(0.5),
            ],
        };

        let choice = g
            .solve_best_target_with_policy(
                "svc{render,company,live}",
                EdgeKind::Reachability,
                Some(VertexKind::Node),
                SolverWeights::default(),
                &policy,
            )
            .unwrap()
            .unwrap();

        assert_eq!(choice.target, "nod{edge-a,zone-1,ready}");
    }

    #[test]
    fn policy_any_mode_accepts_if_any_clause_matches() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();

        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 12.0,
                trust: 0.4,
                cost: 0.2,
                affinity: 0.3,
                load: 0.2,
            },
            BTreeMap::new(),
        )
        .unwrap();

        let policy = PolicySet {
            mode: PolicyMode::Any,
            clauses: vec![
                PolicyClause::MaxLatency(5.0),
                PolicyClause::MinTrust(0.3),
            ],
        };

        let choice = g
            .solve_best_target_with_policy(
                "svc{render,company,live}",
                EdgeKind::Reachability,
                Some(VertexKind::Node),
                SolverWeights::default(),
                &policy,
            )
            .unwrap();
        assert!(choice.is_some());
    }

    #[test]
    fn policy_can_reject_all_candidates() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights::default(),
            BTreeMap::new(),
        )
        .unwrap();

        let policy = PolicySet {
            mode: PolicyMode::All,
            clauses: vec![PolicyClause::RequireVertexAttrEq {
                key: "nonexistent".into(),
                value: "x".into(),
            }],
        };

        let out = g
            .solve_best_target_with_policy(
                "svc{render,company,live}",
                EdgeKind::Reachability,
                Some(VertexKind::Node),
                SolverWeights::default(),
                &policy,
            )
            .unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn placement_initial_pick_selects_best_target() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-b,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 10.0,
                trust: 0.4,
                cost: 0.2,
                affinity: 0.2,
                load: 0.2,
            },
            BTreeMap::new(),
        )
        .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-b,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 4.0,
                trust: 0.2,
                cost: 0.1,
                affinity: 0.1,
                load: 0.1,
            },
            BTreeMap::new(),
        )
        .unwrap();

        let decision = g
            .optimize_placement_with_hysteresis(&PlacementRequest {
                source: "svc{render,company,live}".to_string(),
                edge_kind: EdgeKind::Reachability,
                target_kind: Some(VertexKind::Node),
                coefficients: SolverWeights::default(),
                hysteresis_margin: 0.5,
                current_target: None,
            })
            .unwrap();

        assert_eq!(decision.selected_target.as_deref(), Some("nod{edge-b,zone-1,ready}"));
        assert!(decision.switched);
        assert_eq!(decision.reason, "initial_placement");
    }

    #[test]
    fn placement_reoptimization_respects_hysteresis_margin() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-b,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 5.0,
                trust: 0.4,
                cost: 0.2,
                affinity: 0.1,
                load: 0.2,
            },
            BTreeMap::new(),
        )
        .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-b,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 4.8,
                trust: 0.41,
                cost: 0.2,
                affinity: 0.1,
                load: 0.2,
            },
            BTreeMap::new(),
        )
        .unwrap();

        let decision = g
            .optimize_placement_with_hysteresis(&PlacementRequest {
                source: "svc{render,company,live}".to_string(),
                edge_kind: EdgeKind::Reachability,
                target_kind: Some(VertexKind::Node),
                coefficients: SolverWeights::default(),
                hysteresis_margin: 0.3,
                current_target: Some("nod{edge-a,zone-1,ready}".to_string()),
            })
            .unwrap();

        assert_eq!(decision.selected_target.as_deref(), Some("nod{edge-a,zone-1,ready}"));
        assert!(!decision.switched);
        assert_eq!(decision.reason, "stay_due_to_hysteresis");
    }

    #[test]
    fn placement_reoptimization_switches_when_improvement_exceeds_margin() {
        let mut g = GraphCore::new();
        g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_vertex("nod{edge-b,zone-1,ready}", None, BTreeMap::new())
            .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-a,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 9.0,
                trust: 0.2,
                cost: 0.3,
                affinity: 0.1,
                load: 0.3,
            },
            BTreeMap::new(),
        )
        .unwrap();
        g.upsert_edge(
            "svc{render,company,live}",
            "nod{edge-b,zone-1,ready}",
            EdgeKind::Reachability,
            EdgeWeights {
                latency: 3.0,
                trust: 0.8,
                cost: 0.1,
                affinity: 0.4,
                load: 0.1,
            },
            BTreeMap::new(),
        )
        .unwrap();

        let decision = g
            .optimize_placement_with_hysteresis(&PlacementRequest {
                source: "svc{render,company,live}".to_string(),
                edge_kind: EdgeKind::Reachability,
                target_kind: Some(VertexKind::Node),
                coefficients: SolverWeights::default(),
                hysteresis_margin: 0.5,
                current_target: Some("nod{edge-a,zone-1,ready}".to_string()),
            })
            .unwrap();

        assert_eq!(decision.selected_target.as_deref(), Some("nod{edge-b,zone-1,ready}"));
        assert!(decision.switched);
        assert_eq!(decision.reason, "switch_better_than_hysteresis");
    }

    #[test]
    fn aura_resource_scheduler_allocates_within_quota() {
        let mut s = AuraResourceScheduler::new();
        s.set_quota(AuraResourceQuota {
            aura: "aur{ops,private,open}".to_string(),
            gpu_slices: 8,
            storage_mb: 2048,
            bandwidth_mbps: 1000,
        })
        .unwrap();

        let d = s
            .allocate(ResourceRequest {
                aura: "aur{ops,private,open}".to_string(),
                fold: "fld{worker-a,gpu,warm}".to_string(),
                gpu_slices: 4,
                storage_mb: 512,
                bandwidth_mbps: 200,
            })
            .unwrap();

        assert!(d.granted);
        assert_eq!(d.remaining.gpu_slices, 4);
        assert_eq!(d.remaining.storage_mb, 1536);
        assert_eq!(d.remaining.bandwidth_mbps, 800);
    }

    #[test]
    fn aura_resource_scheduler_rejects_over_quota_request() {
        let mut s = AuraResourceScheduler::new();
        s.set_quota(AuraResourceQuota {
            aura: "aur{ops,private,open}".to_string(),
            gpu_slices: 2,
            storage_mb: 256,
            bandwidth_mbps: 100,
        })
        .unwrap();

        let d = s
            .allocate(ResourceRequest {
                aura: "aur{ops,private,open}".to_string(),
                fold: "fld{worker-a,gpu,warm}".to_string(),
                gpu_slices: 3,
                storage_mb: 300,
                bandwidth_mbps: 101,
            })
            .unwrap();

        assert!(!d.granted);
        assert_eq!(d.reason, "quota_exceeded");
    }

    #[test]
    fn aura_resource_scheduler_reallocation_and_release_update_usage() {
        let mut s = AuraResourceScheduler::new();
        s.set_quota(AuraResourceQuota {
            aura: "aur{ops,private,open}".to_string(),
            gpu_slices: 8,
            storage_mb: 2048,
            bandwidth_mbps: 1000,
        })
        .unwrap();

        s.allocate(ResourceRequest {
            aura: "aur{ops,private,open}".to_string(),
            fold: "fld{worker-a,gpu,warm}".to_string(),
            gpu_slices: 4,
            storage_mb: 400,
            bandwidth_mbps: 100,
        })
        .unwrap();

        let d = s
            .allocate(ResourceRequest {
                aura: "aur{ops,private,open}".to_string(),
                fold: "fld{worker-a,gpu,warm}".to_string(),
                gpu_slices: 2,
                storage_mb: 200,
                bandwidth_mbps: 50,
            })
            .unwrap();
        assert!(d.granted);

        let usage = s.usage_for_aura("aur{ops,private,open}").unwrap();
        assert_eq!(usage.gpu_slices, 2);
        assert_eq!(usage.storage_mb, 200);
        assert_eq!(usage.bandwidth_mbps, 50);

        let released = s
            .release("aur{ops,private,open}", "fld{worker-a,gpu,warm}")
            .unwrap();
        assert!(released);

        let usage_after = s.usage_for_aura("aur{ops,private,open}").unwrap();
        assert_eq!(usage_after, ResourceUsage::default());
    }
}

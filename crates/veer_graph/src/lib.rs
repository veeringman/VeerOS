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
}

#[derive(Debug, Default)]
pub struct GraphCore {
    vertices: HashMap<String, Vertex>,
    edges: HashMap<EdgeKey, Edge>,
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
}

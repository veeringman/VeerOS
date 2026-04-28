# veer_graph

In-process Graph Fabric core for VeerOS (Phase B foundation).

## Scope

- Typed vertices from VAS addresses (`usr/dev/fld/aur/svc/vlt/agt/zon/nod/evt`)
- Directed edges for trust, membership, reachability, capability, replication, and affinity
- Edge weights for latency, trust, cost, affinity, and load
- Mutable graph APIs for upsert, update, and neighborhood queries

## Build and test

```bash
cargo check -p veer_graph
cargo test -p veer_graph
```

## Example

```rust
use std::collections::BTreeMap;
use veer_graph::{Direction, EdgeKind, EdgeWeights, GraphCore};

let mut g = GraphCore::new();
g.upsert_vertex("svc{render,company,live}", None, BTreeMap::new()).unwrap();
g.upsert_vertex("nod{edge-a,zone-1,ready}", None, BTreeMap::new()).unwrap();

g.upsert_edge(
    "svc{render,company,live}",
    "nod{edge-a,zone-1,ready}",
    EdgeKind::Reachability,
    EdgeWeights { latency: 8.0, trust: 0.9, cost: 0.2, affinity: 0.6, load: 0.4 },
    BTreeMap::new(),
).unwrap();

let out = g.neighbors("svc{render,company,live}", Direction::Out, Some(EdgeKind::Reachability)).unwrap();
assert_eq!(out.len(), 1);
```

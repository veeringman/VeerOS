//! Execution Fabric — unified compute layer spanning heterogeneous nodes.
//!
//! The Execution Fabric abstracts the physical topology of VeerOS nodes
//! (microcontrollers, edge devices, cloud VMs) into a single logical
//! compute surface.  The Intent Scheduler uses fabric metadata to place
//! agents on the most appropriate node based on capability, latency,
//! cost, and locality constraints.
//!
//! # Node model
//!
//! Every VeerOS instance registers itself as a **FabricNode** with a
//! capability vector describing what it can do (inference, storage, I/O,
//! network, etc.) and what resources it has (CPU cores, RAM, accelerators).
//!
//! On a standalone device (single-node), the fabric table contains
//! exactly one entry — the local node.  When cluster membership is
//! active (`dist-cluster`), remote nodes are discovered and registered
//! via the cluster protocol.
//!
//! # Placement
//!
//! The fabric provides `select_node()` which scores all healthy nodes
//! against a set of `PlacementConstraint`s and returns the best match.
//! This is called by the Intent Scheduler when spawning agents.

// ─── Configuration ──────────────────────────────────────────────────────

/// Maximum nodes in the fabric (local + discovered remote nodes).
#[cfg(feature = "dist-minimal")]
pub const MAX_FABRIC_NODES: usize = 1;

#[cfg(not(any(feature = "dist-minimal", feature = "dist-full")))]
pub const MAX_FABRIC_NODES: usize = 8;

#[cfg(feature = "dist-full")]
pub const MAX_FABRIC_NODES: usize = 64;

/// Maximum capabilities a node can declare.
pub const MAX_NODE_CAPS: usize = 8;

/// Maximum length of a node name.
pub const MAX_NODE_NAME_LEN: usize = 24;

// ─── Node capability taxonomy ───────────────────────────────────────────

/// Hardware / software capability that a fabric node offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeCapability {
    /// General-purpose CPU computation.
    Compute         = 0,
    /// GPU / accelerator for parallel workloads.
    GpuCompute      = 1,
    /// NPU / TPU for neural network inference.
    NpuInference    = 2,
    /// FPGA for custom hardware logic.
    FpgaLogic       = 3,
    /// Persistent block storage (SD, NVMe, etc.).
    BlockStorage    = 4,
    /// Network connectivity (Ethernet, WiFi).
    Network         = 5,
    /// Wireless radio (BLE, 802.15.4, LoRa).
    Radio           = 6,
    /// Sensor input (temperature, motion, camera).
    Sensors         = 7,
    /// Display output (HDMI, LCD, e-ink).
    Display         = 8,
    /// USB host (keyboards, drives, etc.).
    UsbHost         = 9,
    /// Cryptographic accelerator (AES, SHA, RSA).
    CryptoAccel     = 10,
    /// Quantum processing unit (gate or annealing).
    Quantum         = 11,
    /// Real-time capable (deterministic scheduling).
    Realtime        = 12,
    /// Low-power / battery optimized.
    LowPower        = 13,
    /// Cloud API access (HTTP, gRPC egress).
    CloudApi        = 14,
    /// Model inference (can run ML models locally).
    ModelInference  = 15,
}

/// Health status of a fabric node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeHealth {
    /// Node is healthy and accepting work.
    Healthy     = 0,
    /// Node is overloaded (accept only critical work).
    Overloaded  = 1,
    /// Node is degraded (some capabilities unavailable).
    Degraded    = 2,
    /// Node is unreachable / offline.
    Offline     = 3,
    /// Slot is not in use.
    Unused      = 255,
}

/// Architecture class of a fabric node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NodeArch {
    Riscv32     = 0,
    Riscv64     = 1,
    Aarch64     = 2,
    X86_64      = 3,
    Xtensa      = 4,
    Unknown     = 255,
}

/// Locality zone — coarse geographic or topological grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LocalityZone {
    /// Same physical device (intra-node).
    Local       = 0,
    /// Same rack / room (< 1ms RTT).
    Rack        = 1,
    /// Same data center / building (< 5ms RTT).
    DataCenter  = 2,
    /// Same region (< 20ms RTT).
    Region      = 3,
    /// Cross-region (> 20ms RTT).
    Global      = 4,
}

// ─── Fabric Node descriptor ────────────────────────────────────────────

/// Resource snapshot of a fabric node.
#[derive(Debug, Clone, Copy)]
pub struct NodeResources {
    /// Number of CPU cores.
    pub cpu_cores: u8,
    /// CPU frequency in MHz.
    pub cpu_mhz: u16,
    /// Total RAM in KiB.
    pub ram_kib: u32,
    /// Available (free) RAM in KiB.
    pub ram_free_kib: u32,
    /// Number of accelerator devices.
    pub accel_count: u8,
    /// Current CPU load (0–255, where 255 = 100%).
    pub cpu_load: u8,
    /// Number of active agents on this node.
    pub active_agents: u16,
}

impl NodeResources {
    pub const fn empty() -> Self {
        Self {
            cpu_cores: 1,
            cpu_mhz: 0,
            ram_kib: 0,
            ram_free_kib: 0,
            accel_count: 0,
            cpu_load: 0,
            active_agents: 0,
        }
    }
}

/// Kernel-visible descriptor for one fabric node.
#[derive(Debug, Clone, Copy)]
pub struct FabricNode {
    /// Health status.
    pub health: NodeHealth,
    /// Human-readable name (e.g. "rpi5-kitchen", "qemu-dev").
    pub name: [u8; MAX_NODE_NAME_LEN],
    pub name_len: usize,
    /// Architecture.
    pub arch: NodeArch,
    /// Locality zone relative to the local node.
    pub zone: LocalityZone,
    /// Capability bitmask (each bit maps to a NodeCapability).
    pub capabilities: u32,
    /// Resource snapshot.
    pub resources: NodeResources,
    /// Last heartbeat tick (0 for local node, updated by cluster protocol).
    pub last_heartbeat: u64,
    /// Estimated round-trip latency to this node in microseconds.
    pub rtt_us: u32,
    /// Whether this is the local node.
    pub is_local: bool,
}

impl FabricNode {
    pub const fn empty() -> Self {
        Self {
            health: NodeHealth::Unused,
            name: [0u8; MAX_NODE_NAME_LEN],
            name_len: 0,
            arch: NodeArch::Unknown,
            zone: LocalityZone::Local,
            capabilities: 0,
            resources: NodeResources::empty(),
            last_heartbeat: 0,
            rtt_us: 0,
            is_local: false,
        }
    }

    /// Check if this node has a specific capability.
    pub fn has_capability(&self, cap: NodeCapability) -> bool {
        self.capabilities & (1u32 << (cap as u8)) != 0
    }

    /// Set a capability bit.
    pub fn add_capability(&mut self, cap: NodeCapability) {
        self.capabilities |= 1u32 << (cap as u8);
    }
}

// ─── Placement constraints ─────────────────────────────────────────────

/// A constraint used by the placement algorithm to select the best node.
#[derive(Debug, Clone, Copy)]
pub struct PlacementConstraint {
    /// Required capability (node must have this).
    pub required_cap: Option<NodeCapability>,
    /// Maximum acceptable latency (microseconds RTT).
    pub max_rtt_us: u32,
    /// Preferred locality zone (or tighter).
    pub preferred_zone: LocalityZone,
    /// Minimum free RAM (KiB).
    pub min_ram_kib: u32,
    /// Prefer nodes with low load (weight in scoring).
    pub prefer_low_load: bool,
}

impl PlacementConstraint {
    pub const fn any() -> Self {
        Self {
            required_cap: None,
            max_rtt_us: u32::MAX,
            preferred_zone: LocalityZone::Global,
            min_ram_kib: 0,
            prefer_low_load: true,
        }
    }
}

// ─── Execution Fabric ──────────────────────────────────────────────────

/// The Execution Fabric — manages the topology of compute nodes.
pub struct ExecutionFabric {
    pub nodes: [FabricNode; MAX_FABRIC_NODES],
    /// Index of the local node in the table.
    pub local_idx: usize,
}

impl ExecutionFabric {
    pub const fn new() -> Self {
        Self {
            nodes: [FabricNode::empty(); MAX_FABRIC_NODES],
            local_idx: 0,
        }
    }

    /// Register the local node with its capabilities and resources.
    ///
    /// Called once at kernel boot.
    pub fn register_local(
        &mut self,
        name: &[u8],
        arch: NodeArch,
        resources: NodeResources,
    ) -> usize {
        let node = &mut self.nodes[0];
        node.health = NodeHealth::Healthy;
        let nlen = name.len().min(MAX_NODE_NAME_LEN);
        node.name[..nlen].copy_from_slice(&name[..nlen]);
        node.name_len = nlen;
        node.arch = arch;
        node.zone = LocalityZone::Local;
        node.resources = resources;
        node.is_local = true;
        node.rtt_us = 0;
        self.local_idx = 0;
        0
    }

    /// Register a remote node (discovered via cluster protocol).
    ///
    /// Returns the node index, or `None` if the table is full.
    pub fn register_remote(
        &mut self,
        name: &[u8],
        arch: NodeArch,
        zone: LocalityZone,
        capabilities: u32,
        resources: NodeResources,
        rtt_us: u32,
        tick: u64,
    ) -> Option<usize> {
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if node.health == NodeHealth::Unused {
                node.health = NodeHealth::Healthy;
                let nlen = name.len().min(MAX_NODE_NAME_LEN);
                node.name = [0u8; MAX_NODE_NAME_LEN];
                node.name[..nlen].copy_from_slice(&name[..nlen]);
                node.name_len = nlen;
                node.arch = arch;
                node.zone = zone;
                node.capabilities = capabilities;
                node.resources = resources;
                node.rtt_us = rtt_us;
                node.last_heartbeat = tick;
                node.is_local = false;
                return Some(i);
            }
        }
        None
    }

    /// Update a remote node's heartbeat and resource snapshot.
    pub fn heartbeat(&mut self, node_idx: usize, resources: NodeResources, tick: u64) {
        if node_idx < MAX_FABRIC_NODES && self.nodes[node_idx].health != NodeHealth::Unused {
            self.nodes[node_idx].resources = resources;
            self.nodes[node_idx].last_heartbeat = tick;
            // Auto-recover from Offline if heartbeat arrives.
            if self.nodes[node_idx].health == NodeHealth::Offline {
                self.nodes[node_idx].health = NodeHealth::Healthy;
            }
        }
    }

    /// Mark nodes as offline if their heartbeat is stale.
    pub fn check_health(&mut self, current_tick: u64, timeout_ticks: u64) {
        for node in self.nodes.iter_mut() {
            if node.health != NodeHealth::Unused
                && !node.is_local
                && node.last_heartbeat > 0
                && current_tick.saturating_sub(node.last_heartbeat) > timeout_ticks
            {
                node.health = NodeHealth::Offline;
            }
        }
    }

    /// Select the best node for a workload given constraints.
    ///
    /// Scoring: each healthy node is scored 0–1000; highest score wins.
    /// Factors: capability match (mandatory), latency, locality, load,
    /// available RAM.
    pub fn select_node(&self, constraint: &PlacementConstraint) -> Option<usize> {
        let mut best_idx = None;
        let mut best_score: i32 = -1;

        for (i, node) in self.nodes.iter().enumerate() {
            if !matches!(node.health, NodeHealth::Healthy | NodeHealth::Overloaded) {
                continue;
            }
            // Hard constraint: required capability.
            if let Some(cap) = constraint.required_cap {
                if !node.has_capability(cap) {
                    continue;
                }
            }
            // Hard constraint: maximum latency.
            if node.rtt_us > constraint.max_rtt_us {
                continue;
            }
            // Hard constraint: minimum RAM.
            if node.resources.ram_free_kib < constraint.min_ram_kib {
                continue;
            }

            // Scoring.
            let mut score: i32 = 500; // base

            // Locality bonus (closer = better).
            let zone_score = match node.zone {
                LocalityZone::Local => 200,
                LocalityZone::Rack => 150,
                LocalityZone::DataCenter => 100,
                LocalityZone::Region => 50,
                LocalityZone::Global => 0,
            };
            score += zone_score;

            // Zone preference match.
            if (node.zone as u8) <= (constraint.preferred_zone as u8) {
                score += 100;
            }

            // Load penalty.
            if constraint.prefer_low_load {
                score -= (node.resources.cpu_load as i32) * 2; // 0–510 penalty
            }

            // Overloaded penalty.
            if node.health == NodeHealth::Overloaded {
                score -= 200;
            }

            // Latency bonus (lower = better).
            if node.rtt_us == 0 {
                score += 100; // local node bonus
            } else if node.rtt_us < 1000 {
                score += 50;
            } else if node.rtt_us < 5000 {
                score += 20;
            }

            if score > best_score {
                best_score = score;
                best_idx = Some(i);
            }
        }

        best_idx
    }

    /// Count healthy (non-Unused, non-Offline) nodes.
    pub fn healthy_count(&self) -> usize {
        self.nodes.iter()
            .filter(|n| matches!(n.health, NodeHealth::Healthy | NodeHealth::Overloaded | NodeHealth::Degraded))
            .count()
    }

    /// Count all registered nodes (including offline).
    pub fn total_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.health != NodeHealth::Unused).count()
    }

    /// Remove (unregister) a node by index.
    pub fn remove(&mut self, idx: usize) {
        if idx < MAX_FABRIC_NODES && idx != self.local_idx {
            self.nodes[idx] = FabricNode::empty();
        }
    }

    /// Returns (total, healthy) node counts.
    pub fn node_counts(&self) -> (usize, usize) {
        (self.total_count(), self.healthy_count())
    }
}

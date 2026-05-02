//! Mesh Transport — decentralized message routing across the fabric.
//!
//! The mesh layer handles node discovery, peer management, and message
//! delivery between fabric nodes.  It is transport-agnostic: the same
//! mesh logic works over TCP, UDP, BLE, UART, or SPI.
//!
//! # Architecture
//!
//! ```text
//!  ┌──────────────────────────────────────────────────────────┐
//!  │  Application (Intent Scheduler, Agent Migration, etc.)   │
//!  ├──────────────────────────────────────────────────────────┤
//!  │  Mesh Transport                                          │
//!  │  ┌──────────┐ ┌──────────┐ ┌──────────────────────────┐│
//!  │  │ PeerTable│ │ Outbox   │ │ Routing (greedy geo)     ││
//!  │  └──────────┘ └──────────┘ └──────────────────────────┘│
//!  ├──────────────────────────────────────────────────────────┤
//!  │  Node Identity (Zero Trust auth)                         │
//!  ├──────────────────────────────────────────────────────────┤
//!  │  Fabric Crypto (PQC-hybrid encryption)                   │
//!  ├──────────────────────────────────────────────────────────┤
//!  │  Physical Transport (TCP/UDP/BLE/UART/SPI)               │
//!  └──────────────────────────────────────────────────────────┘
//! ```
//!
//! # No central broker
//!
//! Every node is equal.  Discovery uses periodic `NodeAnnounce`
//! broadcasts.  Gossip propagates the peer table.  Routing uses
//! greedy forwarding via the lowest-RTT path.
//!
//! # Partition tolerance
//!
//! Nodes continue operating locally during network partition.
//! When connectivity resumes, gossip re-syncs the peer table and
//! distributed memory sync resolves conflicts.

use crate::fabric_proto::{MsgType, NodeId, ProtoError, WireMsg, MAX_MSG_LEN, NODE_ID_LEN};
use crate::node_identity::{TrustLevel, MAX_PEERS};

// ─── Configuration ──────────────────────────────────────────────────────

/// Maximum messages in the per-peer outbound queue.
pub const OUTBOX_DEPTH: usize = 8;

/// Maximum total queued messages across all peers.
#[cfg(feature = "dist-minimal")]
pub const MAX_QUEUED_MSGS: usize = 8;

#[cfg(not(any(feature = "dist-minimal", feature = "dist-full")))]
pub const MAX_QUEUED_MSGS: usize = 32;

#[cfg(feature = "dist-full")]
pub const MAX_QUEUED_MSGS: usize = 128;

/// How often (in ticks) to send heartbeats to each peer.
pub const HEARTBEAT_INTERVAL: u64 = 50;

/// How often (in ticks) to broadcast node announcements.
pub const ANNOUNCE_INTERVAL: u64 = 200;

/// Ticks before a peer is considered stale.
pub const PEER_TIMEOUT: u64 = 500;

/// Maximum hops for message forwarding (prevents routing loops).
pub const MAX_HOPS: u8 = 8;

// ─── Queued message ─────────────────────────────────────────────────────

/// A message waiting to be transmitted.
#[derive(Clone, Copy)]
pub struct QueuedMsg {
    /// Destination node ID.
    pub dest: NodeId,
    /// Message data.
    pub data: [u8; MAX_MSG_LEN],
    /// Actual message length.
    pub len: usize,
    /// Priority (higher = send first).
    pub priority: u8,
    /// Remaining hop count.
    pub hops: u8,
    /// Whether this slot is in use.
    pub active: bool,
    /// Tick at which this message was queued.
    pub queued_at: u64,
}

impl QueuedMsg {
    pub const fn empty() -> Self {
        Self {
            dest: [0u8; NODE_ID_LEN],
            data: [0u8; MAX_MSG_LEN],
            len: 0,
            priority: 0,
            hops: MAX_HOPS,
            active: false,
            queued_at: 0,
        }
    }
}

// ─── Peer routing entry ─────────────────────────────────────────────────

/// Routing information for a known peer.
///
/// This is a lightweight entry used by the mesh layer for routing
/// decisions.  Full identity/crypto state lives in `NodeIdentityManager`.
#[derive(Clone, Copy)]
pub struct PeerRoute {
    /// Peer's node ID.
    pub node_id: NodeId,
    /// Whether this route is active.
    pub active: bool,
    /// Direct peer (true) or reachable via forwarding (false).
    pub direct: bool,
    /// If not direct, the next-hop node ID for forwarding.
    pub next_hop: NodeId,
    /// Estimated RTT in microseconds.
    pub rtt_us: u32,
    /// Last time we heard from this peer (tick).
    pub last_seen: u64,
    /// Number of messages sent to this peer.
    pub tx_count: u32,
    /// Number of messages received from this peer.
    pub rx_count: u32,
    /// Trust level (mirrors node_identity, cached for fast routing decisions).
    pub trust: TrustLevel,
}

impl PeerRoute {
    pub const fn empty() -> Self {
        Self {
            node_id: [0u8; NODE_ID_LEN],
            active: false,
            direct: false,
            next_hop: [0u8; NODE_ID_LEN],
            rtt_us: u32::MAX,
            last_seen: 0,
            tx_count: 0,
            rx_count: 0,
            trust: TrustLevel::Untrusted,
        }
    }
}

// ─── Mesh statistics ────────────────────────────────────────────────────

/// Runtime statistics for the mesh transport.
#[derive(Debug, Clone, Copy)]
pub struct MeshStats {
    /// Total messages sent.
    pub msgs_sent: u64,
    /// Total messages received.
    pub msgs_received: u64,
    /// Total messages forwarded (relay).
    pub msgs_forwarded: u64,
    /// Total messages dropped (queue full, TTL expired, untrusted).
    pub msgs_dropped: u64,
    /// Total bytes sent.
    pub bytes_sent: u64,
    /// Total bytes received.
    pub bytes_received: u64,
    /// Current number of direct peers.
    pub direct_peers: u16,
    /// Current number of reachable (indirect) peers.
    pub indirect_peers: u16,
    /// Last announce tick.
    pub last_announce: u64,
    /// Last heartbeat tick.
    pub last_heartbeat: u64,
}

impl MeshStats {
    pub const fn new() -> Self {
        Self {
            msgs_sent: 0,
            msgs_received: 0,
            msgs_forwarded: 0,
            msgs_dropped: 0,
            bytes_sent: 0,
            bytes_received: 0,
            direct_peers: 0,
            indirect_peers: 0,
            last_announce: 0,
            last_heartbeat: 0,
        }
    }
}

// ─── Mesh Transport ─────────────────────────────────────────────────────

/// The Mesh Transport — owns the routing table, outbox, and statistics.
///
/// The mesh does NOT own the physical transport.  Instead, the kernel
/// passes received bytes in and pulls outbound bytes out.  This makes
/// the mesh transport-agnostic (TCP, UDP, BLE, UART, SPI).
pub struct MeshTransport {
    /// Our local node ID (copied from NodeIdentityManager).
    pub local_id: NodeId,
    /// Routing table.
    pub routes: [PeerRoute; MAX_PEERS],
    /// Outbound message queue.
    pub outbox: [QueuedMsg; MAX_QUEUED_MSGS],
    /// Statistics.
    pub stats: MeshStats,
    /// Whether the mesh is active (set after identity init).
    pub active: bool,
}

impl MeshTransport {
    pub const fn new() -> Self {
        Self {
            local_id: [0u8; NODE_ID_LEN],
            routes: [PeerRoute::empty(); MAX_PEERS],
            outbox: [QueuedMsg::empty(); MAX_QUEUED_MSGS],
            stats: MeshStats::new(),
            active: false,
        }
    }

    /// Initialize the mesh with our local node ID.
    pub fn init(&mut self, local_id: &NodeId) {
        self.local_id = *local_id;
        self.active = true;
    }

    /// Register or update a direct peer route.
    pub fn add_direct_peer(
        &mut self,
        node_id: &NodeId,
        rtt_us: u32,
        trust: TrustLevel,
        tick: u64,
    ) -> Option<usize> {
        // Check if already known.
        for (i, route) in self.routes.iter_mut().enumerate() {
            if route.active && route.node_id == *node_id {
                route.rtt_us = rtt_us;
                route.trust = trust;
                route.last_seen = tick;
                route.direct = true;
                return Some(i);
            }
        }
        // Find empty slot.
        for (i, route) in self.routes.iter_mut().enumerate() {
            if !route.active {
                *route = PeerRoute::empty();
                route.node_id = *node_id;
                route.active = true;
                route.direct = true;
                route.rtt_us = rtt_us;
                route.trust = trust;
                route.last_seen = tick;
                self.stats.direct_peers += 1;
                return Some(i);
            }
        }
        None // table full
    }

    /// Register or update an indirect (forwarded) peer route.
    pub fn add_indirect_peer(
        &mut self,
        node_id: &NodeId,
        next_hop: &NodeId,
        rtt_us: u32,
        trust: TrustLevel,
        tick: u64,
    ) -> Option<usize> {
        // Don't add route to ourselves.
        if *node_id == self.local_id {
            return None;
        }
        // Check if already known with a better route.
        for (i, route) in self.routes.iter_mut().enumerate() {
            if route.active && route.node_id == *node_id {
                // Only update if new route is better (lower RTT).
                if rtt_us < route.rtt_us {
                    route.next_hop = *next_hop;
                    route.rtt_us = rtt_us;
                    route.direct = false;
                    route.last_seen = tick;
                }
                return Some(i);
            }
        }
        // Find empty slot.
        for (i, route) in self.routes.iter_mut().enumerate() {
            if !route.active {
                *route = PeerRoute::empty();
                route.node_id = *node_id;
                route.active = true;
                route.direct = false;
                route.next_hop = *next_hop;
                route.rtt_us = rtt_us;
                route.trust = trust;
                route.last_seen = tick;
                self.stats.indirect_peers += 1;
                return Some(i);
            }
        }
        None
    }

    /// Find the best route to a destination node.
    ///
    /// Returns the route index, or None if destination is unreachable.
    pub fn find_route(&self, dest: &NodeId) -> Option<usize> {
        let mut best_idx = None;
        let mut best_rtt = u32::MAX;

        for (i, route) in self.routes.iter().enumerate() {
            if route.active
                && route.node_id == *dest
                && route.trust != TrustLevel::Revoked
                && route.rtt_us < best_rtt
            {
                best_idx = Some(i);
                best_rtt = route.rtt_us;
            }
        }
        best_idx
    }

    /// Enqueue a message for transmission.
    ///
    /// If the destination is a direct peer, the message goes directly.
    /// If indirect, the message is addressed to the next-hop.
    /// Returns true if successfully queued.
    pub fn enqueue(&mut self, dest: &NodeId, msg: &WireMsg, priority: u8, tick: u64) -> bool {
        if !self.active {
            return false;
        }

        // Find a free outbox slot.
        let slot = self.outbox.iter_mut().find(|s| !s.active);
        let slot = match slot {
            Some(s) => s,
            None => {
                // Queue full — try to evict lowest-priority message.
                let mut min_pri = u8::MAX;
                let mut min_idx = None;
                for (i, s) in self.outbox.iter().enumerate() {
                    if s.active && s.priority < min_pri {
                        min_pri = s.priority;
                        min_idx = Some(i);
                    }
                }
                match min_idx {
                    Some(idx) if priority > min_pri => {
                        self.stats.msgs_dropped += 1;
                        &mut self.outbox[idx]
                    }
                    _ => {
                        self.stats.msgs_dropped += 1;
                        return false;
                    }
                }
            }
        };

        *slot = QueuedMsg::empty();
        slot.dest = *dest;
        let copy_len = msg.wire_len().min(MAX_MSG_LEN);
        slot.data[..copy_len].copy_from_slice(&msg.as_bytes()[..copy_len]);
        slot.len = copy_len;
        slot.priority = priority;
        slot.hops = MAX_HOPS;
        slot.active = true;
        slot.queued_at = tick;
        true
    }

    /// Dequeue the next message to transmit.
    ///
    /// Returns (next_hop_node_id, message_bytes, length) or None.
    /// The caller is responsible for actually sending the bytes over
    /// the physical transport.
    pub fn dequeue(&mut self) -> Option<(NodeId, usize)> {
        // Find highest-priority active message.
        let mut best_idx = None;
        let mut best_pri = 0u8;

        for (i, slot) in self.outbox.iter().enumerate() {
            if slot.active && slot.priority >= best_pri {
                best_pri = slot.priority;
                best_idx = Some(i);
            }
        }

        let idx = best_idx?;

        // Copy dest before mutating.
        let dest = self.outbox[idx].dest;
        let len = self.outbox[idx].len;

        // Determine next hop.
        let next_hop = if let Some(route_idx) = self.find_route(&dest) {
            let route = &self.routes[route_idx];
            if route.direct {
                route.node_id
            } else {
                route.next_hop
            }
        } else {
            // No route — drop the message.
            self.outbox[idx].active = false;
            self.stats.msgs_dropped += 1;
            return None;
        };

        self.outbox[idx].active = false;
        self.stats.msgs_sent += 1;
        self.stats.bytes_sent += len as u64;

        Some((next_hop, idx))
    }

    /// Get the outbox slot data by index (after dequeue).
    pub fn outbox_data(&self, idx: usize) -> &[u8] {
        &self.outbox[idx].data[..self.outbox[idx].len]
    }

    /// Handle a received message.
    ///
    /// If addressed to us, returns the message for processing.
    /// If addressed to another node, forwards it (decrements hop count).
    ///
    /// Returns `Some(msg_bytes)` if the message is for us.
    pub fn receive<'a>(&mut self, from: &NodeId, data: &'a [u8], tick: u64) -> Option<&'a [u8]> {
        self.stats.msgs_received += 1;
        self.stats.bytes_received += data.len() as u64;

        // Update last_seen for the sender.
        for route in self.routes.iter_mut() {
            if route.active && route.node_id == *from {
                route.last_seen = tick;
                route.rx_count += 1;
                break;
            }
        }

        // Check if message is for us by examining the payload.
        // For now, assume all received messages are for us.
        // Forwarding logic will be added when we implement
        // the destination field in the wire format.
        Some(data)
    }

    /// Periodic tick — expire stale routes.
    pub fn tick(&mut self, current_tick: u64) {
        for route in self.routes.iter_mut() {
            if route.active && current_tick.saturating_sub(route.last_seen) > PEER_TIMEOUT {
                route.active = false;
                if route.direct {
                    self.stats.direct_peers = self.stats.direct_peers.saturating_sub(1);
                } else {
                    self.stats.indirect_peers = self.stats.indirect_peers.saturating_sub(1);
                }
            }
        }

        // Expire old queued messages (older than 2x peer timeout).
        for slot in self.outbox.iter_mut() {
            if slot.active && current_tick.saturating_sub(slot.queued_at) > PEER_TIMEOUT * 2 {
                slot.active = false;
                self.stats.msgs_dropped += 1;
            }
        }
    }

    /// Count of active routes.
    pub fn route_count(&self) -> usize {
        self.routes.iter().filter(|r| r.active).count()
    }

    /// Count of queued outbound messages.
    pub fn queue_depth(&self) -> usize {
        self.outbox.iter().filter(|s| s.active).count()
    }
}

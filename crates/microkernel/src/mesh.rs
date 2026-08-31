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

use crate::fabric_proto::{
    AnnouncePayload, HeartbeatPayload, MsgType, NodeId, ProtoError, WireMsg, MAX_MSG_LEN,
    MAX_PAYLOAD_LEN, NODE_ID_LEN,
};
use crate::fabric_crypto::SESSION_KEY_LEN;
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

/// Maximum routes included in a single gossip snapshot.
pub const MAX_GOSSIP_ROUTES: usize = MAX_PEERS;

/// Fixed size of one encoded gossip route entry.
pub const GOSSIP_ROUTE_LEN: usize = NODE_ID_LEN + NODE_ID_LEN + 4 + 1 + 1;

/// Maximum encoded gossip payload length.
pub const MAX_GOSSIP_PAYLOAD_LEN: usize = 1 + (MAX_GOSSIP_ROUTES * GOSSIP_ROUTE_LEN);

/// Maximum cached transport address bytes per peer.
pub const MAX_PEER_ADDR_LEN: usize = 32;

/// Fixed mesh routing header: destination node ID + remaining hop budget.
pub const MESH_HEADER_LEN: usize = NODE_ID_LEN + 1;

/// Maximum bytes in one mesh frame including the routing header.
pub const MAX_MESH_MSG_LEN: usize = MAX_MSG_LEN + MESH_HEADER_LEN;

/// Fixed-buffer message carried by transport adapters.
#[derive(Clone, Copy)]
pub struct TransportMsg {
    pub data: [u8; MAX_MESH_MSG_LEN],
    pub len: usize,
}

impl TransportMsg {
    pub const fn empty() -> Self {
        Self {
            data: [0u8; MAX_MESH_MSG_LEN],
            len: 0,
        }
    }

    pub fn from_slice(data: &[u8]) -> Result<Self, ProtoError> {
        if data.len() > MAX_MESH_MSG_LEN {
            return Err(ProtoError::PayloadTooLarge);
        }

        let mut msg = Self::empty();
        msg.data[..data.len()].copy_from_slice(data);
        msg.len = data.len();
        Ok(msg)
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

/// Abstract physical transport used by the mesh routing core.
pub trait MeshTransport {
    fn send(&mut self, node_id: &NodeId, msg: &TransportMsg) -> Result<(), ProtoError>;
    fn recv(&mut self) -> Option<(NodeId, TransportMsg)>;
}

// ─── Queued message ─────────────────────────────────────────────────────

/// A message waiting to be transmitted.
#[derive(Clone, Copy)]
pub struct QueuedMsg {
    /// Destination node ID.
    pub dest: NodeId,
    /// Message data.
    pub data: [u8; MAX_MESH_MSG_LEN],
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
            data: [0u8; MAX_MESH_MSG_LEN],
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
    /// Cached symmetric session key for this peer when established.
    pub session_key: [u8; SESSION_KEY_LEN],
    /// Whether the cached session key is usable.
    pub session_active: bool,
    /// Opaque transport address bytes for the direct peer transport.
    pub address: [u8; MAX_PEER_ADDR_LEN],
    /// Actual length of the cached address bytes.
    pub address_len: u8,
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
            session_key: [0u8; SESSION_KEY_LEN],
            session_active: false,
            address: [0u8; MAX_PEER_ADDR_LEN],
            address_len: 0,
        }
    }
}

/// Bounded peer table carrying routing and cached transport/crypto state.
#[derive(Clone, Copy)]
pub struct PeerTable {
    pub entries: [PeerRoute; MAX_PEERS],
}

impl PeerTable {
    pub const fn new() -> Self {
        Self {
            entries: [PeerRoute::empty(); MAX_PEERS],
        }
    }

    pub fn find_route(&self, dest: &NodeId) -> Option<usize> {
        let mut best_idx = None;
        let mut best_rtt = u32::MAX;

        for (i, route) in self.entries.iter().enumerate() {
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

    pub fn active_count(&self) -> usize {
        self.entries.iter().filter(|route| route.active).count()
    }

    pub fn set_session_key(&mut self, node_id: &NodeId, key: &[u8; SESSION_KEY_LEN]) -> bool {
        for route in self.entries.iter_mut() {
            if route.active && route.node_id == *node_id {
                route.session_key = *key;
                route.session_active = true;
                return true;
            }
        }
        false
    }

    pub fn set_address(&mut self, node_id: &NodeId, address: &[u8]) -> bool {
        if address.len() > MAX_PEER_ADDR_LEN {
            return false;
        }

        for route in self.entries.iter_mut() {
            if route.active && route.node_id == *node_id {
                route.address = [0u8; MAX_PEER_ADDR_LEN];
                route.address[..address.len()].copy_from_slice(address);
                route.address_len = address.len() as u8;
                return true;
            }
        }
        false
    }
}

/// One route entry carried inside a gossip snapshot.
#[derive(Clone, Copy)]
pub struct GossipRoute {
    pub node_id: NodeId,
    pub next_hop: NodeId,
    pub rtt_us: u32,
    pub trust: TrustLevel,
    pub direct: bool,
}

impl GossipRoute {
    pub const fn empty() -> Self {
        Self {
            node_id: [0u8; NODE_ID_LEN],
            next_hop: [0u8; NODE_ID_LEN],
            rtt_us: u32::MAX,
            trust: TrustLevel::Untrusted,
            direct: false,
        }
    }
}

/// No-alloc mesh gossip snapshot carrying a bounded route table sample.
#[derive(Clone, Copy)]
pub struct GossipSnapshot {
    pub routes: [GossipRoute; MAX_GOSSIP_ROUTES],
    pub count: usize,
}

impl GossipSnapshot {
    pub const fn empty() -> Self {
        Self {
            routes: [GossipRoute::empty(); MAX_GOSSIP_ROUTES],
            count: 0,
        }
    }

    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, ProtoError> {
        let total = 1 + (self.count * GOSSIP_ROUTE_LEN);
        if buf.len() < total || self.count > MAX_GOSSIP_ROUTES {
            return Err(ProtoError::BufferTooSmall);
        }

        buf[0] = self.count as u8;
        let mut offset = 1;
        for route in self.routes.iter().take(self.count) {
            buf[offset..offset + NODE_ID_LEN].copy_from_slice(&route.node_id);
            offset += NODE_ID_LEN;
            buf[offset..offset + NODE_ID_LEN].copy_from_slice(&route.next_hop);
            offset += NODE_ID_LEN;
            buf[offset..offset + 4].copy_from_slice(&route.rtt_us.to_le_bytes());
            offset += 4;
            buf[offset] = route.trust as u8;
            offset += 1;
            buf[offset] = route.direct as u8;
            offset += 1;
        }
        Ok(total)
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        if buf.is_empty() {
            return Err(ProtoError::BufferTooSmall);
        }
        let count = buf[0] as usize;
        if count > MAX_GOSSIP_ROUTES {
            return Err(ProtoError::PayloadTooLarge);
        }
        let total = 1 + (count * GOSSIP_ROUTE_LEN);
        if buf.len() < total {
            return Err(ProtoError::BufferTooSmall);
        }

        let mut snapshot = Self::empty();
        snapshot.count = count;
        let mut offset = 1;
        for idx in 0..count {
            let mut route = GossipRoute::empty();
            route.node_id.copy_from_slice(&buf[offset..offset + NODE_ID_LEN]);
            offset += NODE_ID_LEN;
            route.next_hop.copy_from_slice(&buf[offset..offset + NODE_ID_LEN]);
            offset += NODE_ID_LEN;
            route.rtt_us = u32::from_le_bytes([
                buf[offset],
                buf[offset + 1],
                buf[offset + 2],
                buf[offset + 3],
            ]);
            offset += 4;
            route.trust = match buf[offset] {
                1 => TrustLevel::Challenged,
                2 => TrustLevel::Verified,
                3 => TrustLevel::Attested,
                255 => TrustLevel::Revoked,
                _ => TrustLevel::Untrusted,
            };
            offset += 1;
            route.direct = buf[offset] != 0;
            offset += 1;
            snapshot.routes[idx] = route;
        }
        Ok(snapshot)
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

/// The Mesh Router — owns the routing table, outbox, and statistics.
///
/// The mesh does NOT own the physical transport.  Instead, the kernel
/// passes received bytes in and pulls outbound bytes out.  This makes
/// the mesh transport-agnostic (TCP, UDP, BLE, UART, SPI).
pub struct MeshRouter {
    /// Our local node ID (copied from NodeIdentityManager).
    pub local_id: NodeId,
    /// Peer table containing routing and cached transport/crypto state.
    pub peers: PeerTable,
    /// Outbound message queue.
    pub outbox: [QueuedMsg; MAX_QUEUED_MSGS],
    /// Statistics.
    pub stats: MeshStats,
    /// Whether the mesh is active (set after identity init).
    pub active: bool,
}

impl MeshRouter {
    pub const fn new() -> Self {
        Self {
            local_id: [0u8; NODE_ID_LEN],
            peers: PeerTable::new(),
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

    fn encode_mesh_frame(dest: &NodeId, hops: u8, payload: &[u8], buf: &mut [u8]) -> Option<usize> {
        let total = MESH_HEADER_LEN + payload.len();
        if payload.len() > MAX_MSG_LEN || buf.len() < total {
            return None;
        }

        buf[..NODE_ID_LEN].copy_from_slice(dest);
        buf[NODE_ID_LEN] = hops;
        buf[MESH_HEADER_LEN..total].copy_from_slice(payload);
        Some(total)
    }

    fn decode_mesh_frame(data: &[u8]) -> Option<(NodeId, u8, &[u8])> {
        if data.len() < MESH_HEADER_LEN {
            return None;
        }

        let mut dest = [0u8; NODE_ID_LEN];
        dest.copy_from_slice(&data[..NODE_ID_LEN]);
        let hops = data[NODE_ID_LEN];
        let payload = &data[MESH_HEADER_LEN..];
        if payload.len() > MAX_MSG_LEN {
            return None;
        }

        Some((dest, hops, payload))
    }

    fn queue_depth_for(&self, dest: &NodeId) -> usize {
        self.outbox
            .iter()
            .filter(|slot| slot.active && slot.dest == *dest)
            .count()
    }

    fn reserve_outbox_slot(&mut self, dest: &NodeId, priority: u8) -> Option<&mut QueuedMsg> {
        if self.queue_depth_for(dest) >= OUTBOX_DEPTH {
            let mut min_pri = u8::MAX;
            let mut min_idx = None;
            for (idx, slot) in self.outbox.iter().enumerate() {
                if slot.active && slot.dest == *dest && slot.priority <= min_pri {
                    min_pri = slot.priority;
                    min_idx = Some(idx);
                }
            }

            return match min_idx {
                Some(idx) if priority > min_pri => {
                    self.stats.msgs_dropped += 1;
                    Some(&mut self.outbox[idx])
                }
                _ => {
                    self.stats.msgs_dropped += 1;
                    None
                }
            };
        }

        if let Some(idx) = self.outbox.iter().position(|slot| !slot.active) {
            return Some(&mut self.outbox[idx]);
        }

        let mut min_pri = u8::MAX;
        let mut min_idx = None;
        for (idx, slot) in self.outbox.iter().enumerate() {
            if slot.active && slot.priority < min_pri {
                min_pri = slot.priority;
                min_idx = Some(idx);
            }
        }

        match min_idx {
            Some(idx) if priority > min_pri => {
                self.stats.msgs_dropped += 1;
                Some(&mut self.outbox[idx])
            }
            _ => {
                self.stats.msgs_dropped += 1;
                None
            }
        }
    }

    /// Whether it is time to send a fresh `NodeAnnounce` broadcast.
    pub fn should_send_announce(&self, current_tick: u64) -> bool {
        self.active
            && (self.stats.last_announce == 0
                || current_tick.saturating_sub(self.stats.last_announce) >= ANNOUNCE_INTERVAL)
    }

    /// Whether it is time to send per-peer heartbeats.
    pub fn should_send_heartbeat(&self, current_tick: u64) -> bool {
        self.active
            && (self.stats.last_heartbeat == 0
                || current_tick.saturating_sub(self.stats.last_heartbeat) >= HEARTBEAT_INTERVAL)
    }

    /// Build a `NodeAnnounce` broadcast frame and stamp announce time.
    pub fn build_announce(
        &mut self,
        arch: u8,
        zone: u8,
        capabilities: u32,
        cpu_cores: u8,
        cpu_mhz: u16,
        ram_kib: u32,
        name: &[u8],
        tick: u64,
    ) -> Result<WireMsg, ProtoError> {
        let mut payload = [0u8; MAX_PAYLOAD_LEN];
        let payload_len = AnnouncePayload::encode(
            &self.local_id,
            arch,
            zone,
            capabilities,
            cpu_cores,
            cpu_mhz,
            ram_kib,
            name,
            &mut payload,
        );
        if payload_len == 0 {
            return Err(ProtoError::BufferTooSmall);
        }

        self.stats.last_announce = tick;
        WireMsg::build(MsgType::NodeAnnounce, &payload[..payload_len])
    }

    /// Build a `NodeHeartbeat` frame and stamp heartbeat time.
    pub fn build_heartbeat(
        &mut self,
        seq: u32,
        cpu_load: u8,
        active_agents: u16,
        ram_free_kib: u32,
        health: u8,
        tick: u64,
    ) -> Result<WireMsg, ProtoError> {
        let mut payload = [0u8; MAX_PAYLOAD_LEN];
        let payload_len = HeartbeatPayload::encode(
            &self.local_id,
            seq,
            cpu_load,
            active_agents,
            ram_free_kib,
            health,
            &mut payload,
        );
        if payload_len == 0 {
            return Err(ProtoError::BufferTooSmall);
        }

        self.stats.last_heartbeat = tick;
        WireMsg::build(MsgType::NodeHeartbeat, &payload[..payload_len])
    }

    /// Export the currently active route table as a bounded gossip snapshot.
    pub fn export_gossip_snapshot(&self) -> GossipSnapshot {
        let mut snapshot = GossipSnapshot::empty();
        for route in self.peers.entries.iter().filter(|route| route.active) {
            if snapshot.count >= MAX_GOSSIP_ROUTES {
                break;
            }
            snapshot.routes[snapshot.count] = GossipRoute {
                node_id: route.node_id,
                next_hop: if route.direct { route.node_id } else { route.next_hop },
                rtt_us: route.rtt_us,
                trust: route.trust,
                direct: route.direct,
            };
            snapshot.count += 1;
        }
        snapshot
    }

    /// Merge a received gossip snapshot into the local routing table.
    pub fn apply_gossip_snapshot(
        &mut self,
        from: &NodeId,
        snapshot: &GossipSnapshot,
        tick: u64,
    ) {
        for route in snapshot.routes.iter().take(snapshot.count) {
            if route.node_id == self.local_id || route.trust == TrustLevel::Revoked {
                continue;
            }

            let next_hop = if route.direct { *from } else { route.next_hop };
            let _ = self.add_indirect_peer(
                &route.node_id,
                &next_hop,
                route.rtt_us,
                route.trust,
                tick,
            );
        }
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
        for (i, route) in self.peers.entries.iter_mut().enumerate() {
            if route.active && route.node_id == *node_id {
                route.rtt_us = rtt_us;
                route.trust = trust;
                route.last_seen = tick;
                route.direct = true;
                route.next_hop = *node_id;
                return Some(i);
            }
        }
        // Find empty slot.
        for (i, route) in self.peers.entries.iter_mut().enumerate() {
            if !route.active {
                *route = PeerRoute::empty();
                route.node_id = *node_id;
                route.active = true;
                route.direct = true;
                route.next_hop = *node_id;
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
        for (i, route) in self.peers.entries.iter_mut().enumerate() {
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
        for (i, route) in self.peers.entries.iter_mut().enumerate() {
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
        self.peers.find_route(dest)
    }

    /// Cache an established session key for a peer in the local peer table.
    pub fn set_peer_session_key(&mut self, node_id: &NodeId, key: &[u8; SESSION_KEY_LEN]) -> bool {
        self.peers.set_session_key(node_id, key)
    }

    /// Cache an opaque transport address for a peer in the local peer table.
    pub fn set_peer_address(&mut self, node_id: &NodeId, address: &[u8]) -> bool {
        self.peers.set_address(node_id, address)
    }

    fn next_hop_for_dest(&self, dest: &NodeId) -> Option<NodeId> {
        self.find_route(dest).map(|route_idx| {
            let route = &self.peers.entries[route_idx];
            if route.direct {
                route.node_id
            } else {
                route.next_hop
            }
        })
    }

    fn next_routable_slot(&self) -> Option<(usize, NodeId)> {
        let mut best_idx = None;
        let mut best_pri = 0u8;
        let mut best_next_hop = [0u8; NODE_ID_LEN];

        for (idx, slot) in self.outbox.iter().enumerate() {
            if !slot.active {
                continue;
            }
            let Some(next_hop) = self.next_hop_for_dest(&slot.dest) else {
                continue;
            };
            if slot.priority >= best_pri {
                best_pri = slot.priority;
                best_idx = Some(idx);
                best_next_hop = next_hop;
            }
        }

        best_idx.map(|idx| (idx, best_next_hop))
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

        let slot = match self.reserve_outbox_slot(dest, priority) {
            Some(slot) => slot,
            None => return false,
        };

        *slot = QueuedMsg::empty();
        slot.dest = *dest;
        let msg_len = msg.wire_len();
        let encoded_len = match Self::encode_mesh_frame(dest, MAX_HOPS, &msg.as_bytes()[..msg_len], &mut slot.data) {
            Some(len) => len,
            None => {
                self.stats.msgs_dropped += 1;
                return false;
            }
        };
        slot.len = encoded_len;
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
        let (idx, best_next_hop) = self.next_routable_slot()?;

        let len = self.outbox[idx].len;

        self.outbox[idx].active = false;
        self.stats.msgs_sent += 1;
        self.stats.bytes_sent += len as u64;

        Some((best_next_hop, idx))
    }

    /// Flush the next routable queued frame through the physical transport.
    pub fn flush_next<T: MeshTransport>(&mut self, transport: &mut T) -> Result<bool, ProtoError> {
        let Some((idx, next_hop)) = self.next_routable_slot() else {
            return Ok(false);
        };

        let frame = TransportMsg::from_slice(self.outbox_data(idx))?;
        transport.send(&next_hop, &frame)?;

        let len = self.outbox[idx].len;
        self.outbox[idx].active = false;
        self.stats.msgs_sent += 1;
        self.stats.bytes_sent += len as u64;
        Ok(true)
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
        for route in self.peers.entries.iter_mut() {
            if route.active && route.node_id == *from {
                route.last_seen = tick;
                route.rx_count += 1;
                break;
            }
        }

        let (dest, hops, payload) = match Self::decode_mesh_frame(data) {
            Some(frame) => frame,
            None => {
                self.stats.msgs_dropped += 1;
                return None;
            }
        };

        if dest == self.local_id {
            return Some(payload);
        }

        if hops == 0 {
            self.stats.msgs_dropped += 1;
            return None;
        }

        let route_idx = self
            .peers
            .entries
            .iter()
            .enumerate()
            .filter(|(_, route)| {
                route.active
                    && route.node_id == dest
                    && route.trust != TrustLevel::Revoked
                    && ((route.direct && route.node_id != *from) || (!route.direct && route.next_hop != *from))
            })
            .min_by_key(|(_, route)| route.rtt_us)
            .map(|(idx, _)| idx);

        let route_idx = match route_idx {
            Some(idx) => idx,
            None => {
                self.stats.msgs_dropped += 1;
                return None;
            }
        };

        let slot = match self.reserve_outbox_slot(&dest, 0) {
            Some(slot) => slot,
            None => return None,
        };

        *slot = QueuedMsg::empty();
        slot.dest = dest;
        slot.hops = hops - 1;
        slot.priority = 0;
        slot.len = match Self::encode_mesh_frame(&dest, slot.hops, payload, &mut slot.data) {
            Some(len) => len,
            None => {
                self.stats.msgs_dropped += 1;
                return None;
            }
        };
        slot.active = true;
        slot.queued_at = tick;

        self.peers.entries[route_idx].tx_count = self.peers.entries[route_idx]
            .tx_count
            .saturating_add(1);
        self.stats.msgs_forwarded = self.stats.msgs_forwarded.saturating_add(1);
        None
    }

    /// Poll the physical transport once and deliver any frame addressed to us.
    pub fn recv_from<T: MeshTransport>(
        &mut self,
        transport: &mut T,
        tick: u64,
    ) -> Option<(NodeId, TransportMsg)> {
        let (from, frame) = transport.recv()?;
        let delivered = self.receive(&from, frame.as_slice(), tick)?;
        let delivered = TransportMsg::from_slice(delivered).ok()?;
        Some((from, delivered))
    }

    /// Periodic tick — expire stale routes.
    pub fn tick(&mut self, current_tick: u64) {
        for route in self.peers.entries.iter_mut() {
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
        self.peers.active_count()
    }

    /// Count of queued outbound messages.
    pub fn queue_depth(&self) -> usize {
        self.outbox.iter().filter(|s| s.active).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct InMemoryFrame {
        node_id: NodeId,
        msg: TransportMsg,
        active: bool,
    }

    impl InMemoryFrame {
        const fn empty() -> Self {
            Self {
                node_id: [0u8; NODE_ID_LEN],
                msg: TransportMsg::empty(),
                active: false,
            }
        }
    }

    struct InMemoryMeshTransport {
        inbox: [InMemoryFrame; MAX_QUEUED_MSGS],
    }

    impl InMemoryMeshTransport {
        const fn new() -> Self {
            Self {
                inbox: [InMemoryFrame::empty(); MAX_QUEUED_MSGS],
            }
        }
    }

    impl MeshTransport for InMemoryMeshTransport {
        fn send(&mut self, node_id: &NodeId, msg: &TransportMsg) -> Result<(), ProtoError> {
            let Some(slot) = self.inbox.iter_mut().find(|slot| !slot.active) else {
                return Err(ProtoError::BufferTooSmall);
            };
            *slot = InMemoryFrame {
                node_id: *node_id,
                msg: *msg,
                active: true,
            };
            Ok(())
        }

        fn recv(&mut self) -> Option<(NodeId, TransportMsg)> {
            let idx = self.inbox.iter().position(|slot| slot.active)?;
            let frame = self.inbox[idx];
            self.inbox[idx].active = false;
            Some((frame.node_id, frame.msg))
        }
    }

    fn node_id(byte: u8) -> NodeId {
        [byte; NODE_ID_LEN]
    }

    #[test]
    fn announce_scheduler_respects_interval() {
        let mut mesh = MeshRouter::new();
        mesh.init(&node_id(1));

        assert!(mesh.should_send_announce(10));
        let wire = mesh
            .build_announce(2, 1, 0x55, 4, 2400, 512 * 1024, b"edge-a", 10)
            .unwrap();
        assert_eq!(wire.header().unwrap().msg_type, MsgType::NodeAnnounce);
        assert!(!mesh.should_send_announce(10 + ANNOUNCE_INTERVAL - 1));
        assert!(mesh.should_send_announce(10 + ANNOUNCE_INTERVAL));
    }

    #[test]
    fn heartbeat_scheduler_respects_interval() {
        let mut mesh = MeshRouter::new();
        mesh.init(&node_id(2));

        assert!(mesh.should_send_heartbeat(20));
        let wire = mesh.build_heartbeat(7, 12, 3, 128 * 1024, 0, 20).unwrap();
        assert_eq!(wire.header().unwrap().msg_type, MsgType::NodeHeartbeat);
        assert!(!mesh.should_send_heartbeat(20 + HEARTBEAT_INTERVAL - 1));
        assert!(mesh.should_send_heartbeat(20 + HEARTBEAT_INTERVAL));
    }

    #[test]
    fn announce_payload_contains_local_node_id() {
        let mut mesh = MeshRouter::new();
        let local = node_id(3);
        mesh.init(&local);

        let wire = mesh
            .build_announce(2, 1, 0xAA, 2, 1600, 64 * 1024, b"esp32", 30)
            .unwrap();
        let payload = wire.payload();

        assert_eq!(AnnouncePayload::sender_id(payload).unwrap(), &local);
    }

    #[test]
    fn gossip_snapshot_roundtrip_preserves_routes() {
        let mut mesh = MeshRouter::new();
        mesh.init(&node_id(4));
        let direct = node_id(5);
        let indirect = node_id(6);
        let next_hop = node_id(7);
        mesh.add_direct_peer(&direct, 100, TrustLevel::Verified, 10);
        mesh.add_indirect_peer(&indirect, &next_hop, 250, TrustLevel::Attested, 11);

        let snapshot = mesh.export_gossip_snapshot();
        let mut buf = [0u8; MAX_GOSSIP_PAYLOAD_LEN];
        let len = snapshot.encode(&mut buf).unwrap();
        let decoded = GossipSnapshot::decode(&buf[..len]).unwrap();

        assert_eq!(decoded.count, 2);
        assert_eq!(decoded.routes[0].node_id, direct);
        assert_eq!(decoded.routes[1].node_id, indirect);
    }

    #[test]
    fn gossip_merge_adds_indirect_route_via_sender() {
        let mut mesh = MeshRouter::new();
        let local = node_id(8);
        let sender = node_id(9);
        let target = node_id(10);
        mesh.init(&local);
        mesh.add_direct_peer(&sender, 90, TrustLevel::Verified, 20);

        let mut snapshot = GossipSnapshot::empty();
        snapshot.routes[0] = GossipRoute {
            node_id: target,
            next_hop: sender,
            rtt_us: 300,
            trust: TrustLevel::Verified,
            direct: true,
        };
        snapshot.count = 1;

        mesh.apply_gossip_snapshot(&sender, &snapshot, 21);
        let idx = mesh.find_route(&target).unwrap();
        assert!(!mesh.peers.entries[idx].direct);
        assert_eq!(mesh.peers.entries[idx].next_hop, sender);
        assert_eq!(mesh.peers.entries[idx].trust, TrustLevel::Verified);
    }

    #[test]
    fn receive_forwards_remote_message_via_best_next_hop() {
        let mut sender = MeshRouter::new();
        let mut relay = MeshRouter::new();
        let sender_id = node_id(11);
        let relay_id = node_id(12);
        let target_id = node_id(13);
        sender.init(&sender_id);
        relay.init(&relay_id);

        sender.add_direct_peer(&relay_id, 50, TrustLevel::Verified, 1);
        relay.add_direct_peer(&sender_id, 50, TrustLevel::Verified, 1);
        relay.add_direct_peer(&target_id, 80, TrustLevel::Verified, 1);

        let wire = WireMsg::build(MsgType::Ack, &[]).unwrap();
        assert!(sender.enqueue(&target_id, &wire, 3, 2));
        let (_, slot_idx) = sender.dequeue().unwrap();
        let outbound = sender.outbox_data(slot_idx).to_vec();

        assert!(relay.receive(&sender_id, &outbound, 3).is_none());
        assert_eq!(relay.queue_depth(), 1);

        let (next_hop, forwarded_idx) = relay.dequeue().unwrap();
        assert_eq!(next_hop, target_id);

        let (dest, hops, payload) = MeshRouter::decode_mesh_frame(relay.outbox_data(forwarded_idx)).unwrap();
        assert_eq!(dest, target_id);
        assert_eq!(hops, MAX_HOPS - 1);
        assert_eq!(payload, wire.as_bytes());
    }

    #[test]
    fn receive_delivers_inner_wire_message_for_local_destination() {
        let mut sender = MeshRouter::new();
        let mut local = MeshRouter::new();
        let sender_id = node_id(14);
        let local_id = node_id(15);
        sender.init(&sender_id);
        local.init(&local_id);

        sender.add_direct_peer(&local_id, 40, TrustLevel::Verified, 1);
        local.add_direct_peer(&sender_id, 40, TrustLevel::Verified, 1);

        let wire = WireMsg::build(MsgType::Ack, &[]).unwrap();
        assert!(sender.enqueue(&local_id, &wire, 1, 2));
        let (_, slot_idx) = sender.dequeue().unwrap();
        let outbound = sender.outbox_data(slot_idx).to_vec();

        let delivered = local.receive(&sender_id, &outbound, 3).unwrap();
        assert_eq!(delivered, wire.as_bytes());
    }

    #[test]
    fn peer_table_caches_session_key_and_address() {
        let mut mesh = MeshRouter::new();
        let local = node_id(16);
        let peer = node_id(17);
        mesh.init(&local);
        mesh.add_direct_peer(&peer, 55, TrustLevel::Verified, 5);

        let session_key = [0xAB; SESSION_KEY_LEN];
        assert!(mesh.set_peer_session_key(&peer, &session_key));
        assert!(mesh.set_peer_address(&peer, b"tcp://peer-17"));

        let idx = mesh.find_route(&peer).unwrap();
        assert!(mesh.peers.entries[idx].session_active);
        assert_eq!(mesh.peers.entries[idx].session_key, session_key);
        assert_eq!(mesh.peers.entries[idx].address_len as usize, b"tcp://peer-17".len());
        assert_eq!(
            &mesh.peers.entries[idx].address[..mesh.peers.entries[idx].address_len as usize],
            b"tcp://peer-17"
        );
    }

    #[test]
    fn enqueue_enforces_per_peer_outbox_limit() {
        let mut mesh = MeshRouter::new();
        let local = node_id(18);
        let peer = node_id(19);
        mesh.init(&local);
        mesh.add_direct_peer(&peer, 60, TrustLevel::Verified, 1);

        for _ in 0..OUTBOX_DEPTH {
            let wire = WireMsg::build(MsgType::Ack, &[]).unwrap();
            assert!(mesh.enqueue(&peer, &wire, 1, 2));
        }

        let wire = WireMsg::build(MsgType::Ack, &[]).unwrap();
        assert!(!mesh.enqueue(&peer, &wire, 1, 3));
        assert_eq!(mesh.queue_depth_for(&peer), OUTBOX_DEPTH);
    }

    #[test]
    fn enqueue_evicts_lowest_priority_within_same_peer_queue() {
        let mut mesh = MeshRouter::new();
        let local = node_id(20);
        let peer = node_id(21);
        mesh.init(&local);
        mesh.add_direct_peer(&peer, 60, TrustLevel::Verified, 1);

        for _ in 0..OUTBOX_DEPTH {
            let wire = WireMsg::build(MsgType::Ack, &[]).unwrap();
            assert!(mesh.enqueue(&peer, &wire, 1, 2));
        }

        let high = WireMsg::build(MsgType::Nack, &[]).unwrap();
        assert!(mesh.enqueue(&peer, &high, 5, 3));
        assert_eq!(mesh.queue_depth_for(&peer), OUTBOX_DEPTH);
        assert_eq!(
            mesh.outbox
                .iter()
                .filter(|slot| slot.active && slot.dest == peer && slot.priority == 5)
                .count(),
            1
        );
    }

    #[test]
    fn dequeue_retains_unroutable_message_until_route_returns() {
        let mut mesh = MeshRouter::new();
        let local = node_id(22);
        let peer = node_id(23);
        mesh.init(&local);

        let wire = WireMsg::build(MsgType::Ack, &[]).unwrap();
        assert!(mesh.enqueue(&peer, &wire, 2, 1));
        assert!(mesh.dequeue().is_none());
        assert_eq!(mesh.queue_depth(), 1);
        assert_eq!(mesh.stats.msgs_dropped, 0);

        mesh.add_direct_peer(&peer, 70, TrustLevel::Verified, 2);
        let (next_hop, idx) = mesh.dequeue().unwrap();
        assert_eq!(next_hop, peer);
        assert_eq!(mesh.outbox_data(idx), &mesh.outbox[idx].data[..mesh.outbox[idx].len]);
        assert_eq!(mesh.queue_depth(), 0);
    }

    #[test]
    fn dequeue_skips_partitioned_peer_and_sends_routable_message() {
        let mut mesh = MeshRouter::new();
        let local = node_id(24);
        let blocked = node_id(25);
        let reachable = node_id(26);
        mesh.init(&local);
        mesh.add_direct_peer(&reachable, 30, TrustLevel::Verified, 1);

        let high = WireMsg::build(MsgType::Nack, &[]).unwrap();
        let low = WireMsg::build(MsgType::Ack, &[]).unwrap();
        assert!(mesh.enqueue(&blocked, &high, 5, 2));
        assert!(mesh.enqueue(&reachable, &low, 1, 2));

        let (next_hop, _) = mesh.dequeue().unwrap();
        assert_eq!(next_hop, reachable);
        assert_eq!(mesh.queue_depth_for(&blocked), 1);
        assert_eq!(mesh.queue_depth_for(&reachable), 0);
    }

    #[test]
    fn flush_next_sends_queued_frame_through_transport_trait() {
        let mut mesh = MeshRouter::new();
        let mut transport = InMemoryMeshTransport::new();
        let local = node_id(27);
        let peer = node_id(28);
        mesh.init(&local);
        mesh.add_direct_peer(&peer, 25, TrustLevel::Verified, 1);

        let wire = WireMsg::build(MsgType::Ack, &[]).unwrap();
        assert!(mesh.enqueue(&peer, &wire, 2, 2));
        assert!(mesh.flush_next(&mut transport).unwrap());

        let (next_hop, frame) = transport.recv().unwrap();
        assert_eq!(next_hop, peer);
        let (dest, _, payload) = MeshRouter::decode_mesh_frame(frame.as_slice()).unwrap();
        assert_eq!(dest, peer);
        assert_eq!(payload, wire.as_bytes());
    }

    #[test]
    fn recv_from_delivers_local_frame_from_transport_trait() {
        let mut mesh = MeshRouter::new();
        let mut transport = InMemoryMeshTransport::new();
        let local = node_id(29);
        let sender = node_id(30);
        mesh.init(&local);
        mesh.add_direct_peer(&sender, 25, TrustLevel::Verified, 1);

        let wire = WireMsg::build(MsgType::Ack, &[]).unwrap();
        let mut framed = [0u8; MAX_MESH_MSG_LEN];
        let len = MeshRouter::encode_mesh_frame(&local, MAX_HOPS, wire.as_bytes(), &mut framed).unwrap();
        transport
            .send(&sender, &TransportMsg::from_slice(&framed[..len]).unwrap())
            .unwrap();

        let (from, delivered) = mesh.recv_from(&mut transport, 2).unwrap();
        assert_eq!(from, sender);
        assert_eq!(delivered.as_slice(), wire.as_bytes());
    }
}

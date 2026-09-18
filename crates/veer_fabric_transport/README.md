# veer_fabric_transport

VeerOS Fabric transport runtime with secure framing, multiplexed logical streams,
and resumable session state.

## Current components

- `FabricSession`: encrypted frame engine using ChaCha20-Poly1305
- Stream multiplexing over logical `stream_id`
- Snapshot/restore for resumable sequence state
- Optional QUIC adapter (`quic-backend` feature) for real wire transport

## Transport layering

`veer_fabric_transport` is the std-capable Fabric runtime. It can sit on top of
plain sockets, tunnels, or QUIC when WAN behavior justifies the extra transport
stack.

QUIC is intentionally optional here rather than foundational for all Fabric
links:

- `FabricSession` already provides Veer-specific framing, sequencing, logical
	streams, and resumable session snapshots.
- The microkernel mesh stays `no_std` and transport-agnostic, so it cannot host
	a Quinn/Tokio QUIC stack directly.
- Running Fabric over QUIC duplicates some transport features, especially
	encryption and multiplexing, but can still be worthwhile on lossy or
	internet-facing links.

Current rule of thumb:

- Kernel / embedded mesh: custom Fabric transport only.
- User-space / host-side runtime: optional QUIC backend when needed.

## Build and test

```bash
cargo test -p veer_fabric_transport
cargo check -p veer_fabric_transport
```

With QUIC backend enabled:

```bash
cargo check -p veer_fabric_transport --features quic-backend
```

## QUIC adapter usage

`quic_backend::QuicFabricRuntime` wraps a `quinn::Connection` and maps payloads to
`FabricSession` framed packets.

This adapter is best treated as a std-side wire backend for VeerOS Fabric, not
as a replacement for the mesh routing core.

```rust
use veer_fabric_transport::quic_backend::QuicFabricRuntime;

# async fn f(conn: quinn::Connection, key: [u8; 32]) -> Result<(), veer_fabric_transport::TransportError> {
let mut rt = QuicFabricRuntime::new(conn, &key)?;
let sid = rt.open_logical_stream();
let (mut send, mut recv) = rt.open_quic_bi().await?;
rt.send_frame(sid, b"hello", &mut send).await?;
let _frame = rt.recv_one_frame(&mut recv, 64 * 1024).await?;
# Ok(()) }
```

The QUIC adapter uses a 4-byte little-endian length prefix per Fabric packet,
so one QUIC stream can carry multiple incremental Fabric frames without relying
on stream-close (`read_to_end`) semantics.

## Host-side mesh bridge

`quic_backend::QuicMeshBridge` pins one QUIC bi-stream and exposes node-addressed
operations that map naturally to mesh transport semantics:

- `send_to(node_id, payload)`
- `recv_from() -> Option<MeshEnvelope { node_id, payload }>`

This is intended for std-capable host/user-space integration where QUIC is used
as the wire backend under Veer-specific Fabric framing and routing.

`quic_backend::HostMeshAdapter<T>` wraps any backend that implements
`AsyncMeshBridgeTransport`, giving a backend-agnostic host runtime surface with:

- `send(node_id, payload)`
- `recv_for_local()` (filters envelopes not addressed to local node id)
- `finish()`

`QuicMeshBridge` is the concrete QUIC implementation of that trait today.

## Host UDP / TCP mesh adapters

`mesh_io::UdpMeshTransport` and `mesh_io::TcpMeshTransport` implement the
microkernel `MeshTransport` trait on real sockets so mesh frames leave the
process. Persistent Ed25519 node identity lives in `identity_fs::FileIdentityStore`
(`node_id = SHA-256(public_key)`).

```bash
# in-process socket ping
cargo test -p veer_fabric_transport mesh_io -- --nocapture

# two terminals
cargo run -p veer_fabric_transport --example mesh_udp_ping -- \
  --listen 127.0.0.1:7001 --peer 127.0.0.1:7002 --name a
cargo run -p veer_fabric_transport --example mesh_udp_ping -- \
  --listen 127.0.0.1:7002 --peer 127.0.0.1:7001 --name b

# single-process loopback
cargo run -p veer_fabric_transport --example mesh_udp_ping -- --self-test
```

Runnable local QUIC loopback example:

```bash
cargo run -p veer_fabric_transport --features quic-backend --example quic_mesh_bridge_demo
```

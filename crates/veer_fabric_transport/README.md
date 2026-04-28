# veer_fabric_transport

VeerOS Fabric transport runtime with secure framing, multiplexed logical streams,
and resumable session state.

## Current components

- `FabricSession`: encrypted frame engine using ChaCha20-Poly1305
- Stream multiplexing over logical `stream_id`
- Snapshot/restore for resumable sequence state
- Optional QUIC adapter (`quic-backend` feature) for real wire transport

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

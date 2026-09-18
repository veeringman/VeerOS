//! Two-process UDP mesh ping.
//!
//! Terminal A:
//! ```bash
//! cargo run -p veer_fabric_transport --example mesh_udp_ping -- \
//!   --listen 127.0.0.1:7001 --peer 127.0.0.1:7002 --name a
//! ```
//!
//! Terminal B:
//! ```bash
//! cargo run -p veer_fabric_transport --example mesh_udp_ping -- \
//!   --listen 127.0.0.1:7002 --peer 127.0.0.1:7001 --name b
//! ```
//!
//! Single-process loopback:
//! ```bash
//! cargo run -p veer_fabric_transport --example mesh_udp_ping -- --self-test
//! ```

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use microkernel::fabric_proto::{MsgType, WireMsg};
use microkernel::mesh::{MeshRouter, MeshTransport, MAX_HOPS, MESH_BROADCAST_ID};
use microkernel::node_identity::{NodeIdentityManager, TrustLevel};
use veer_fabric_transport::identity_fs::{host_entropy, FileIdentityStore};
use veer_fabric_transport::mesh_io::UdpMeshTransport;

struct Args {
    listen: Option<SocketAddr>,
    peer: Option<SocketAddr>,
    identity: PathBuf,
    name: String,
    timeout: Duration,
    self_test: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut listen = None;
    let mut peer = None;
    let mut identity = PathBuf::from(format!(
        "/tmp/veeros-mesh-{}.bin",
        std::process::id()
    ));
    let mut name = "mesh-ping".to_string();
    let mut timeout = Duration::from_secs(3);
    let mut self_test = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => {
                let value = args.next().ok_or("--listen requires HOST:PORT")?;
                listen = Some(value.parse().map_err(|_| "invalid --listen address")?);
            }
            "--peer" => {
                let value = args.next().ok_or("--peer requires HOST:PORT")?;
                peer = Some(value.parse().map_err(|_| "invalid --peer address")?);
            }
            "--identity" => {
                identity = PathBuf::from(args.next().ok_or("--identity requires PATH")?);
            }
            "--name" => {
                name = args.next().ok_or("--name requires VALUE")?;
            }
            "--timeout-ms" => {
                let value = args.next().ok_or("--timeout-ms requires integer")?;
                timeout = Duration::from_millis(
                    value.parse().map_err(|_| "invalid --timeout-ms")?,
                );
            }
            "--self-test" => self_test = true,
            "--help" | "-h" => {
                println!(
                    "mesh_udp_ping [--listen HOST:PORT] [--peer HOST:PORT] [--identity PATH]\n\
                     [--name NAME] [--timeout-ms N] [--self-test]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    Ok(Args {
        listen,
        peer,
        identity,
        name,
        timeout,
        self_test,
    })
}

fn init_identity(path: &PathBuf, name: &str) -> NodeIdentityManager {
    let mut store = FileIdentityStore::new(path);
    let mut id = NodeIdentityManager::new();
    assert!(
        id.init_persistent(&mut store, host_entropy(), 2, 0x1, 1, 1),
        "failed to persist node identity at {}",
        path.display()
    );
    println!(
        "[{name}] node_id={:02x}{:02x}{:02x}{:02x} identity={}",
        id.local_id[0], id.local_id[1], id.local_id[2], id.local_id[3], path.display()
    );
    id
}

fn wait_recv<T: MeshTransport>(
    mesh: &mut MeshRouter,
    transport: &mut T,
    timeout: Duration,
    tick: u64,
) -> Option<(microkernel::fabric_proto::NodeId, WireMsg)> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Some((from, payload)) = mesh.recv_from(transport, tick) {
            if let Ok(wire) = WireMsg::from_bytes(payload.as_slice()) {
                return Some((from, wire));
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

fn run_ping(
    listen: SocketAddr,
    peer: SocketAddr,
    identity: PathBuf,
    name: &str,
    timeout: Duration,
) -> Result<(), String> {
    let mut ident = init_identity(&identity, name);
    let mut transport = UdpMeshTransport::bind(listen, ident.local_id)
        .map_err(|_| "failed to bind UDP socket")?;
    let bound = transport.local_addr().map_err(|_| "missing local addr")?;
    println!("[{name}] listening on {bound}, peering {peer}");

    let mut mesh = MeshRouter::new();
    mesh.init(&ident.local_id);

    let announce = mesh
        .build_announce_with_pubkey(
            2,
            1,
            0x1,
            1,
            1000,
            64 * 1024,
            name.as_bytes(),
            &ident.local_keypair.ed25519_pk,
            1,
        )
        .map_err(|_| "announce encode failed")?;
    let frame = mesh
        .encode_outbound(&MESH_BROADCAST_ID, &announce, MAX_HOPS)
        .map_err(|_| "announce wrap failed")?;
    transport
        .send_to_addr(peer, &frame)
        .map_err(|_| "announce send failed")?;

    let (from, wire) = wait_recv(&mut mesh, &mut transport, timeout, 2)
        .ok_or("timed out waiting for peer announce/ping")?;
    let header = wire.header().map_err(|_| "bad peer header")?;
    println!(
        "[{name}] received {:?} from {:02x}{:02x}{:02x}{:02x}",
        header.msg_type, from[0], from[1], from[2], from[3]
    );

    if header.msg_type == MsgType::NodeAnnounce {
        if let Some(pk) = microkernel::fabric_proto::AnnouncePayload::public_key(wire.payload()) {
            if let Some(idx) = ident.register_peer(&from, 3) {
                ident.set_peer_public_key(idx, &pk);
            }
        }
        mesh.add_direct_peer(&from, 20, TrustLevel::Untrusted, 3);
        transport.map_peer(from, peer);

        let ping = WireMsg::build(MsgType::Ack, b"ping").map_err(|_| "ping build failed")?;
        mesh.enqueue(&from, &ping, 2, 4);
        mesh.flush_next(&mut transport)
            .map_err(|_| "ping send failed")?;

        let (pong_from, pong) = wait_recv(&mut mesh, &mut transport, timeout, 5)
            .ok_or("timed out waiting for pong")?;
        let pong_ty = pong.header().map_err(|_| "bad pong")?.msg_type;
        if pong_from != from || pong_ty != MsgType::Ack {
            return Err("unexpected pong".into());
        }
        println!("[{name}] ping ok ({} bytes)", pong.payload().len());
        return Ok(());
    }

    if header.msg_type == MsgType::Ack {
        mesh.add_direct_peer(&from, 20, TrustLevel::Untrusted, 3);
        transport.map_peer(from, peer);
        let pong = WireMsg::build(MsgType::Ack, b"pong").map_err(|_| "pong build failed")?;
        mesh.enqueue(&from, &pong, 2, 4);
        mesh.flush_next(&mut transport)
            .map_err(|_| "pong send failed")?;
        println!("[{name}] pong sent");
        // Keep the socket alive briefly so the peer can receive.
        std::thread::sleep(Duration::from_millis(200));
        return Ok(());
    }

    Err(format!("unexpected message {:?}", header.msg_type))
}

fn self_test() -> Result<(), String> {
    let mut left = UdpMeshTransport::bind("127.0.0.1:0".parse().unwrap(), [0xAAu8; 32])
        .map_err(|_| "left bind")?;
    let mut right = UdpMeshTransport::bind("127.0.0.1:0".parse().unwrap(), [0xBBu8; 32])
        .map_err(|_| "right bind")?;
    let left_addr = left.local_addr().map_err(|_| "left addr")?;
    let right_addr = right.local_addr().map_err(|_| "right addr")?;
    left.map_peer([0xBBu8; 32], right_addr);
    right.map_peer([0xAAu8; 32], left_addr);

    let mut sender = MeshRouter::new();
    let mut receiver = MeshRouter::new();
    sender.init(&[0xAAu8; 32]);
    receiver.init(&[0xBBu8; 32]);
    sender.add_direct_peer(&[0xBBu8; 32], 10, TrustLevel::Verified, 1);
    receiver.add_direct_peer(&[0xAAu8; 32], 10, TrustLevel::Verified, 1);

    let ping = WireMsg::build(MsgType::Ack, b"self-test").unwrap();
    sender.enqueue(&[0xBBu8; 32], &ping, 1, 2);
    sender.flush_next(&mut left).map_err(|_| "self-test send")?;

    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if let Some((from, payload)) = receiver.recv_from(&mut right, 3) {
            assert_eq!(from, [0xAAu8; 32]);
            let wire = WireMsg::from_bytes(payload.as_slice()).unwrap();
            assert_eq!(wire.payload(), b"self-test");
            println!(
                "self-test ok: {} -> {} ({right_addr} / {left_addr})",
                hex4(&from),
                hex4(&[0xBBu8; 32])
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Err("self-test timeout".into())
}

fn hex4(id: &[u8]) -> String {
    format!("{:02x}{:02x}{:02x}{:02x}", id[0], id[1], id[2], id[3])
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(2);
        }
    };

    let result = if args.self_test {
        self_test()
    } else {
        let listen = args.listen.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
        let peer = match args.peer {
            Some(peer) => peer,
            None => {
                eprintln!("--peer HOST:PORT is required (or pass --self-test)");
                std::process::exit(2);
            }
        };
        run_ping(listen, peer, args.identity, &args.name, args.timeout)
    };

    if let Err(err) = result {
        eprintln!("mesh ping failed: {err}");
        std::process::exit(1);
    }
}

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use veer_fabric_transport::quic_backend::{
    HostMeshAdapter, QuicFabricRuntime, QuicMeshBridge, MESH_NODE_ID_LEN,
};

fn transport_err(e: veer_fabric_transport::TransportError) -> std::io::Error {
    std::io::Error::other(format!("{e:?}"))
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let key = [0x66u8; 32];
    let server_node = [0xB1u8; MESH_NODE_ID_LEN];
    let client_node = [0x12u8; MESH_NODE_ID_LEN];

    let cert = generate_simple_self_signed(vec!["localhost".into()])?;
    let cert_der: CertificateDer<'static> = CertificateDer::from(cert.serialize_der()?);
    let key_der = PrivatePkcs8KeyDer::from(cert.serialize_private_key_der());

    let mut server_config = ServerConfig::with_single_cert(vec![cert_der.clone()], key_der.into())?;
    server_config.transport_config(Arc::new(TransportConfig::default()));

    let server_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
    let server_endpoint = Endpoint::server(server_config, server_addr)?;
    let bound_addr = server_endpoint.local_addr()?;

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der)?;
    let client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    let mut client_endpoint = Endpoint::client(SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::LOCALHOST,
        0,
    )))?;
    let client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(client_crypto)?,
    ));
    client_endpoint.set_default_client_config(client_config);

    let server_task = tokio::spawn(async move {
        let incoming = server_endpoint.accept().await.expect("server accept");
        let connection = incoming.await.expect("server handshake");
        let runtime = QuicFabricRuntime::new(connection, &key).expect("runtime");
        let stream = runtime.accept_quic_bi().await.expect("server bi stream");
        let bridge = runtime.into_bridge(stream, 64 * 1024);
        let mut adapter = HostMeshAdapter::new(bridge, server_node);

        let env = adapter
            .recv_for_local()
            .await
            .expect("server recv")
            .expect("server eof");
        println!(
            "server received from envelope for node {:02x?}: {}",
            &env.node_id[..4],
            String::from_utf8_lossy(&env.payload)
        );
        adapter.finish().await.expect("server finish");
    });

    let client_conn = client_endpoint
        .connect(bound_addr, "localhost")?
        .await?;
    let bridge = QuicMeshBridge::from_connection(client_conn, &key, 64 * 1024)
        .await
        .map_err(transport_err)?;
    let mut adapter = HostMeshAdapter::new(bridge, client_node);

    adapter
        .send(&server_node, b"hello from veeros quic demo")
        .await
        .map_err(transport_err)?;
    adapter.finish().await.map_err(transport_err)?;

    server_task.await?;
    Ok(())
}

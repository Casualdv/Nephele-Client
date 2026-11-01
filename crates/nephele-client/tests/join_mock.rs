//! End-to-end: real TCP, real framing/compression, real chunk decoding, against
//! the in-process mock server.

use std::time::Duration;

use nephele_client::GameClient;
use nephele_mock::{MockConfig, MockServer};
use nephele_protocol::{ping, ServerAddress};

fn address_of(server: &MockServer) -> ServerAddress {
    ServerAddress::parse(&server.address()).unwrap()
}

#[tokio::test]
async fn joins_and_loads_the_mock_world() {
    let server = MockServer::start(MockConfig::default()).await.unwrap();
    let address = address_of(&server);

    let mut client = GameClient::join(&address, "Tester").await.unwrap();
    assert_eq!(client.info.username, "Tester");
    assert_eq!(client.info.dimension, 0);
    assert_eq!(client.info.compression, Some(64));

    client
        .pump(Duration::from_millis(1500), Duration::from_millis(50))
        .await
        .unwrap();

    assert!(client.spawned, "server position should set spawned");
    assert_eq!(client.world.loaded_chunks(), 25, "5x5 chunk region");
    assert!(client.world.total_non_air() > 0);
    assert!(client.chunks_loaded >= 25);

    // The mock spawns us one block above the surface.
    let feet_y = client.position.y.floor() as i32 - 1;
    let under = client.world.get_block(
        client.position.x.floor() as i32,
        feet_y,
        client.position.z.floor() as i32,
    );
    assert!(
        !under.is_air(),
        "expected ground under spawn, got {under:?}"
    );

    // Column mask and biome data survived the round trip.
    let column = client
        .world
        .get(nephele_world::ChunkCoord::new(0, 0))
        .expect("origin chunk");
    assert_eq!(column.mask().count_ones(), 5, "sections 0..=4");
    assert!(column.biomes().is_some());

    client.disconnect().await;
}

#[tokio::test]
async fn chat_is_echoed_back() {
    let server = MockServer::start(MockConfig {
        radius: 0,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    let address = address_of(&server);

    let mut client = GameClient::join(&address, "Chatty").await.unwrap();
    client
        .pump(Duration::from_millis(600), Duration::from_millis(50))
        .await
        .unwrap();
    assert!(
        client.chat.iter().any(|line| line.contains("Welcome")),
        "greeting: {:?}",
        client.chat
    );

    client.send_chat("hello nephele").await.unwrap();
    client
        .pump(Duration::from_millis(800), Duration::from_millis(50))
        .await
        .unwrap();
    assert!(
        client
            .chat
            .iter()
            .any(|line| line.contains("<Chatty> hello nephele")),
        "chat log: {:?}",
        client.chat
    );

    client.disconnect().await;
}

#[tokio::test]
async fn works_without_compression() {
    let server = MockServer::start(MockConfig {
        compression_threshold: None,
        radius: 0,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    let address = address_of(&server);

    let mut client = GameClient::join(&address, "Plain").await.unwrap();
    assert_eq!(client.info.compression, None);
    client
        .pump(Duration::from_millis(600), Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(client.world.loaded_chunks(), 1);
    assert!(client.world.total_non_air() > 0);

    client.disconnect().await;
}

#[tokio::test]
async fn status_ping_reports_protocol_47() {
    let server = MockServer::start(MockConfig::default()).await.unwrap();
    let address = address_of(&server);

    let status = ping(&address, Duration::from_secs(5)).await.unwrap();
    assert_eq!(status.protocol, Some(47));
    assert_eq!(status.version_name.as_deref(), Some("1.8.9"));
    assert!(status.description.contains("Nephele mock server"));
}

//! End-to-end: real TCP, real framing/compression, real chunk decoding, against
//! the in-process mock server.

use std::time::Duration;

use nephele_client::{Aim, EntityKind, GameClient};
use nephele_mock::{MockConfig, MockServer};
use nephele_protocol::packets::play;
use nephele_protocol::packets::play::ItemStack;
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

/// The lobby demo: bots with names/skins and a framed map with real pixels.
#[tokio::test]
async fn the_lobby_has_players_and_a_framed_map() {
    let server = MockServer::start(MockConfig {
        radius: 1,
        greeting: false,
        bots: true,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    let mut client = GameClient::join(&address_of(&server), "Visitor")
        .await
        .unwrap();
    client
        .pump(Duration::from_millis(1400), Duration::from_millis(50))
        .await
        .unwrap();

    let players: Vec<_> = client
        .entities
        .iter()
        .filter(|entity| matches!(entity.kind, EntityKind::Player { .. }))
        .collect();
    assert!(
        players.len() >= nephele_mock::BOT_COUNT,
        "expected the lobby bots, got {} players",
        players.len()
    );
    assert!(players.iter().any(|entity| entity.shows_name()));
    // Half the bots are slim, so both arm layouts are exercised.
    assert!(client.skins.iter().any(|(_, info)| info.slim));
    assert!(client.skins.iter().any(|(_, info)| !info.slim));

    let frame = client
        .entities
        .iter()
        .find(|entity| matches!(entity.kind, EntityKind::ItemFrame { .. }))
        .expect("the mock spawns a framed map");
    let state = frame.item_frame.as_ref().expect("item frame state");
    let item = state.item.expect("the frame contains an item");
    assert_eq!(item.id, 358, "a filled map item");
    let map_id = item.map_id.expect("the map item carries its id");
    // N's parser has not supplied pixels here; the stub fallback must still be
    // a real picture rather than a blank square.
    let map = client.maps.ensure(map_id);
    let distinct: std::collections::HashSet<_> = map.pixels.iter().collect();
    assert!(
        distinct.len() > 4,
        "map {map_id} placeholder should be a picture, not one flat colour"
    );

    client.disconnect().await;
}

/// The whole survival drop chain against a real socket: break a block, watch
/// the item entity arrive, then watch it be collected into the hotbar.
#[tokio::test]
async fn a_broken_block_drops_an_item_that_is_picked_up() {
    let server = MockServer::start(MockConfig {
        radius: 1,
        greeting: false,
        survival: true,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    let address = address_of(&server);

    let mut client = GameClient::join(&address, "Digger").await.unwrap();
    client
        .pump(Duration::from_millis(800), Duration::from_millis(50))
        .await
        .unwrap();
    assert!(client.spawned, "the server positions us on join");

    // Break the block we are standing on. A single digging packet is all the
    // mock needs; it is the same packet the survival miner sends when a block
    // finishes.
    let x = client.position.x.floor() as i32;
    let z = client.position.z.floor() as i32;
    let y = client.position.y.floor() as i32 - 1;
    let block = client.world.get_block(x, y, z);
    assert!(!block.is_air(), "expected ground under the spawn");
    let aim = Aim {
        x,
        y,
        z,
        face: 1,
        cursor: [0.5, 1.0, 0.5],
    };
    client
        .dig(
            nephele_protocol::packets::play::digging::STOP_DESTROY_BLOCK,
            &aim,
        )
        .await
        .unwrap();

    // The drop is an item entity at the broken block.
    let mut saw_item = false;
    for _ in 0..30 {
        client
            .pump(Duration::from_millis(100), Duration::from_millis(20))
            .await
            .unwrap();
        if client.entities.iter().any(|entity| {
            matches!(
                entity.kind,
                EntityKind::Item { id, .. } if id == block.id() as i16
            )
        }) {
            saw_item = true;
            break;
        }
    }
    assert!(saw_item, "no item entity was spawned for {block:?}");

    // A second later the mock collects it and puts it in the first hotbar slot.
    for _ in 0..40 {
        client
            .pump(Duration::from_millis(100), Duration::from_millis(20))
            .await
            .unwrap();
        if client.inventory[nephele_protocol::packets::play::HOTBAR_FIRST_SLOT].is_some() {
            break;
        }
    }
    assert_eq!(
        client.inventory[nephele_protocol::packets::play::HOTBAR_FIRST_SLOT].map(|item| item.id),
        Some(block.id() as i16),
        "the pickup should land in the held hotbar slot"
    );
    assert!(
        !client
            .entities
            .iter()
            .any(|entity| matches!(entity.kind, EntityKind::Item { .. })),
        "the collected entity should have been removed"
    );

    client.disconnect().await;
}

/// The survival inventory loop over a real socket: a stack moves from the
/// hotbar into the 2x2 grid through `C0E`, and the server reports planks in the
/// output slot.
#[tokio::test]
async fn a_log_in_the_crafting_grid_shows_planks_in_the_output_slot() {
    let server = MockServer::start(MockConfig {
        radius: 0,
        greeting: false,
        ..MockConfig::default()
    })
    .await
    .unwrap();
    let address = address_of(&server);

    let mut client = GameClient::join(&address, "Crafter").await.unwrap();
    client
        .pump(Duration::from_millis(600), Duration::from_millis(50))
        .await
        .unwrap();

    let hotbar = play::HOTBAR_FIRST_SLOT;
    let log = ItemStack::new(17, 3, 0);
    client
        .creative_set_slot(hotbar as i16, Some(log))
        .await
        .unwrap();
    client
        .pump(Duration::from_millis(300), Duration::from_millis(20))
        .await
        .unwrap();
    assert_eq!(client.inventory[hotbar], Some(log), "setup stack");

    // Pick the stack up, then drop it all into the top-left grid cell.
    client
        .click_slot(hotbar as i16, 0, play::click_mode::CLICK)
        .await
        .unwrap();
    client
        .pump(Duration::from_millis(300), Duration::from_millis(20))
        .await
        .unwrap();
    assert_eq!(client.inventory[hotbar], None, "the stack left the hotbar");

    client
        .click_slot(1, 0, play::click_mode::CLICK)
        .await
        .unwrap();
    client
        .pump(Duration::from_millis(300), Duration::from_millis(20))
        .await
        .unwrap();
    assert_eq!(client.inventory[1], Some(log));
    assert_eq!(
        client.inventory[0],
        Some(ItemStack::new(5, 4, 0)),
        "one log crafts four planks"
    );

    // Taking the result spends exactly one log.
    client
        .click_slot(0, 0, play::click_mode::CLICK)
        .await
        .unwrap();
    client
        .pump(Duration::from_millis(300), Duration::from_millis(20))
        .await
        .unwrap();
    assert_eq!(client.inventory[1], Some(ItemStack::new(17, 2, 0)));
    assert_eq!(
        client.inventory[0],
        Some(ItemStack::new(5, 4, 0)),
        "the output preview is refreshed for the next craft"
    );

    client.disconnect().await;
}

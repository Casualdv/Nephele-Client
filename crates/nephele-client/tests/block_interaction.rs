//! End-to-end block interaction against the mock server: dig a block, place a
//! block, and confirm the client's world follows the server's `S23` updates.

use std::time::Duration;

use nephele_client::mining::Aim;
use nephele_client::GameClient;
use nephele_mock::{MockConfig, MockServer};
use nephele_protocol::packets::play::{digging, ItemStack};
use nephele_protocol::ServerAddress;
use nephele_world::Face;

fn address_of(server: &MockServer) -> ServerAddress {
    ServerAddress::parse(&server.address()).unwrap()
}

async fn join(server: &MockServer) -> GameClient {
    let mut client = GameClient::join(&address_of(server), "Digger")
        .await
        .unwrap();
    client
        .pump(Duration::from_millis(1200), Duration::from_millis(50))
        .await
        .unwrap();
    client
}

/// Aim at the surface block under the spawn point.
fn surface_aim(client: &GameClient) -> Aim {
    let x = client.position.x.floor() as i32;
    let z = client.position.z.floor() as i32;
    let mut y = client.position.y.floor() as i32 + 1;
    while y > 0 && client.world.get_block(x, y, z).is_air() {
        y -= 1;
    }
    Aim {
        x,
        y,
        z,
        face: Face::Up.protocol_id(),
        cursor: [0.5, 1.0, 0.5],
    }
}

#[tokio::test]
async fn digging_removes_the_block_in_the_clients_world() {
    let server = MockServer::start(MockConfig::default()).await.unwrap();
    let mut client = join(&server).await;

    let aim = surface_aim(&client);
    let before = client.world.get_block(aim.x, aim.y, aim.z);
    assert!(!before.is_air(), "expected a solid block at {aim:?}");

    client
        .dig(digging::START_DESTROY_BLOCK, &aim)
        .await
        .unwrap();
    client.dig(digging::STOP_DESTROY_BLOCK, &aim).await.unwrap();
    client
        .pump(Duration::from_millis(600), Duration::from_millis(50))
        .await
        .unwrap();

    assert!(
        client.world.get_block(aim.x, aim.y, aim.z).is_air(),
        "server did not confirm the break"
    );
    client.disconnect().await;
}

#[tokio::test]
async fn creative_placement_lands_on_the_clicked_face() {
    let server = MockServer::start(MockConfig::default()).await.unwrap();
    let mut client = join(&server).await;
    assert!(client.is_creative(), "mock server runs in creative");

    // Give ourselves cobblestone in the first hotbar slot.
    let cobblestone = ItemStack::new(4, 64, 0);
    client.give_starter_hotbar().await.unwrap();
    assert_eq!(client.held_item(), Some(cobblestone));
    assert_eq!(
        client.inventory[nephele_protocol::packets::play::HOTBAR_FIRST_SLOT],
        Some(cobblestone)
    );

    let aim = surface_aim(&client);
    let target = (aim.x, aim.y + 1, aim.z);
    assert!(
        client
            .world
            .get_block(target.0, target.1, target.2)
            .is_air(),
        "the block above the surface should be empty"
    );

    let placed = client.place(&aim).await.unwrap();
    assert_eq!(placed, Some(target));
    client
        .pump(Duration::from_millis(600), Duration::from_millis(50))
        .await
        .unwrap();

    assert_eq!(
        client.world.get_block(target.0, target.1, target.2).raw(),
        (4 << 4),
        "expected cobblestone where we clicked"
    );
    client.disconnect().await;
}

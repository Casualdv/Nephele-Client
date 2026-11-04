//! Minimized protocol-47 samples and a synthetic multi-byte-length chat spacer.

use std::time::Duration;

use nephele_protocol::packets::play;
use nephele_protocol::{Connection, Packet, State};
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

/// Play `0x01` Join Game: entity 1, survival, overworld, difficulty 3,
/// 60 max players, `flat`, no reduced debug info.
const JOIN_GAME: &str = "000000010000033c04666c617400";
/// Play `0x05` Spawn Position: block (-38, 72, 0).
const SPAWN_POSITION: &str = "fffff68120000000";
/// Play `0x03` Time Update: age 148765325, time of day -22500.
const TIME_UPDATE: &str = "0000000008ddfa8dffffffffffffa81c";
/// Play `0x06` Update Health: 20.0 health, 20 food, 5.0 saturation.
const UPDATE_HEALTH: &str = "41a000001440a00000";
/// Play `0x09` Held Item Change: slot 0, a single byte.
const HELD_ITEM_CHANGE: &str = "00";
/// Play `0x3F` Plugin Message: channel `REGISTER`, payload `BungeeCord`.
const PLUGIN_REGISTER: &str = "08524547495354455242756e676565436f7264";
fn spacer() -> Packet {
    let mut w = nephele_protocol::PacketWriter::new();
    w.write_string(&format!(r#"{{"text":"{}"}}"#, " ".repeat(256)))
        .write_u8(0);
    Packet::new(play::clientbound::CHAT_MESSAGE, w.into_bytes())
}

fn packet(id: i32, hex_bytes: &str) -> Packet {
    Packet::new(
        id,
        hex::decode(hex_bytes.trim()).expect("captured fixture is valid hex"),
    )
}

#[test]
fn captured_join_game_parses() {
    let join = play::parse_join_game(&packet(play::clientbound::JOIN_GAME, JOIN_GAME))
        .expect("captured join game");
    assert_eq!(join.entity_id, 1);
    assert_eq!(join.gamemode, 0);
    assert_eq!(join.dimension, 0);
    assert_eq!(join.difficulty, 3);
    assert_eq!(join.max_players, 60);
    assert_eq!(join.level_type, "flat");
    assert!(!join.reduced_debug_info);
}

#[test]
fn captured_spawn_position_parses() {
    let position =
        play::parse_spawn_position(&packet(play::clientbound::SPAWN_POSITION, SPAWN_POSITION))
            .expect("captured spawn position");
    assert_eq!(position, (-38, 72, 0));
}

#[test]
fn captured_time_update_parses() {
    let time = play::parse_time_update(&packet(play::clientbound::TIME_UPDATE, TIME_UPDATE))
        .expect("captured time update");
    assert_eq!(time.world_age, 148_765_325);
    assert_eq!(time.time_of_day, -22_500);
}

#[test]
fn captured_update_health_parses() {
    let health =
        play::parse_update_health(&packet(play::clientbound::UPDATE_HEALTH, UPDATE_HEALTH))
            .expect("captured health");
    assert_eq!(health.health, 20.0);
    assert_eq!(health.food, 20);
    assert_eq!(health.saturation, 5.0);
}

#[test]
fn captured_held_item_change_is_one_byte() {
    assert_eq!(
        play::parse_held_item_change(&packet(
            play::clientbound::HELD_ITEM_CHANGE,
            HELD_ITEM_CHANGE,
        ))
        .expect("captured held slot"),
        0
    );
}

#[test]
fn captured_plugin_register_parses() {
    let message =
        play::parse_plugin_message(&packet(play::clientbound::PLUGIN_MESSAGE, PLUGIN_REGISTER))
            .expect("captured plugin message");
    assert_eq!(message.channel, "REGISTER");
    assert_eq!(message.data, b"BungeeCord");
}

#[test]
fn synthetic_chat_consumes_its_whole_payload() {
    let chat = spacer();
    let (text, position) = play::parse_chat_message(&chat).expect("captured chat");

    assert!(text.chars().all(|ch| ch == ' '), "unexpected text {text:?}");
    assert_eq!(position, 0);

    // The 1.8 packet is exactly string + byte. An extra byte must be rejected,
    // not silently ignored.
    let mut extra = chat.data.clone();
    extra.push(0);
    assert!(
        play::parse_chat_message(&Packet::new(play::clientbound::CHAT_MESSAGE, extra)).is_err()
    );
}

/// The chat handler sees an isolated payload; the socket still owns the next header.
#[tokio::test]
async fn a_synthetic_chat_frame_does_not_desync_the_next_frame() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socket = listener.local_addr().unwrap();
    let (client_stream, accepted) = tokio::join!(TcpStream::connect(socket), listener.accept());
    let client_stream = client_stream.unwrap();
    let (mut server, _) = accepted.unwrap();

    let frames: Vec<Vec<u8>> = [
        spacer(),
        packet(play::clientbound::SPAWN_POSITION, SPAWN_POSITION),
    ]
    .iter()
    .map(|packet| {
        nephele_protocol::framing::encode_frame(&packet.to_payload(), None).expect("frame")
    })
    .collect();
    let stream_bytes = frames.concat();

    let writer = tokio::spawn(async move {
        server.write_all(&stream_bytes).await.unwrap();
        server.flush().await.unwrap();
    });

    let mut client = Connection::from_stream(client_stream, State::Play);
    let first = tokio::time::timeout(Duration::from_secs(5), client.read_packet())
        .await
        .expect("first frame timed out")
        .expect("first frame");
    assert_eq!(first.id, play::clientbound::CHAT_MESSAGE);
    let (text, position) = play::parse_chat_message(&first).expect("captured chat");
    assert!(text.chars().all(|ch| ch == ' '));
    assert_eq!(position, 0);

    let second = tokio::time::timeout(Duration::from_secs(5), client.read_packet())
        .await
        .expect("second frame timed out")
        .expect("second frame");
    assert_eq!(second.id, play::clientbound::SPAWN_POSITION);
    assert_eq!(
        play::parse_spawn_position(&second).expect("spawn position"),
        (-38, 72, 0)
    );

    writer.await.unwrap();
}

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
/// Play `0x3B` Scoreboard Objective: create `Prototype`, display `§e§lBED WARS`,
/// type `integer` — straight out of a Hypixel lobby capture.
const SCOREBOARD_OBJECTIVE: &str =
    "0950726F746F74797065000EC2A765C2A76C424544205741525307696E7465676572";
/// Play `0x3D` Display Scoreboard: sidebar (`1`) shows `Prototype`.
const DISPLAY_SCOREBOARD: &str = "010950726F746F74797065";
/// Play `0x3C` Update Score: entry `🍫`, action 0, objective `Prototype`, 14.
const UPDATE_SCORE_TOP: &str = "04F09F8DAB000950726F746F747970650E";
/// Play `0x3E` Teams: create `team_14`, display `§fteam_14`, white (15).
const TEAM_CREATE: &str = "077465616D5F3134000AC2A7667465616D5F313400000306616C776179730F00";
/// Play `0x3E` Teams: re-format `team_14` with prefix `§711/03/26  §8L2`
/// and suffix `§85F`.
const TEAM_UPDATE: &str = "077465616D5F3134020AC2A7667465616D5F313412C2A73731312F30332F32362020C2A7384C3205C2A73835460306616C776179730F";
/// Play `0x3E` Teams: add `🍫` to `team_14`.
const TEAM_ADD_MEMBER: &str = "077465616D5F3134030104F09F8DAB";
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

#[test]
fn captured_scoreboard_objective_parses() {
    let update = play::parse_scoreboard_objective(&packet(
        play::clientbound::SCOREBOARD_OBJECTIVE,
        SCOREBOARD_OBJECTIVE,
    ))
    .expect("captured objective");

    assert_eq!(
        update,
        play::ObjectiveUpdate::Create {
            name: "Prototype".into(),
            display_name: "§e§lBED WARS".into(),
            render_type: "integer".into(),
        }
    );
}

#[test]
fn captured_display_scoreboard_parses() {
    let display = play::parse_display_scoreboard(&packet(
        play::clientbound::DISPLAY_SCOREBOARD,
        DISPLAY_SCOREBOARD,
    ))
    .expect("captured display slot");

    assert_eq!(display.position, play::scoreboard_slot::SIDEBAR);
    assert_eq!(display.objective, "Prototype");
}

#[test]
fn captured_update_score_parses_a_utf8_entry() {
    let update =
        play::parse_update_score(&packet(play::clientbound::UPDATE_SCORE, UPDATE_SCORE_TOP))
            .expect("captured score");

    assert_eq!(update.entry, "🍫");
    assert_eq!(update.objective, "Prototype");
    assert_eq!(update.value, Some(14));
}

#[test]
fn captured_team_create_parses_exactly() {
    let update =
        play::parse_teams(&packet(play::clientbound::TEAMS, TEAM_CREATE)).expect("captured team");

    assert_eq!(
        update,
        play::TeamUpdate::Create {
            name: "team_14".into(),
            info: play::TeamInfo {
                display_name: "§fteam_14".into(),
                prefix: String::new(),
                suffix: String::new(),
                friendly_flags: 3,
                name_tag_visibility: "always".into(),
                color: 15,
            },
            players: Vec::new(),
        }
    );
}

#[test]
fn captured_team_update_and_membership_parse() {
    let update =
        play::parse_teams(&packet(play::clientbound::TEAMS, TEAM_UPDATE)).expect("team update");
    let play::TeamUpdate::Update { name, info } = update else {
        panic!("expected a team update, got {update:?}");
    };
    assert_eq!(name, "team_14");
    assert_eq!(info.prefix, "§711/03/26  §8L2");
    assert_eq!(info.suffix, "§85F");
    assert_eq!(info.color, 15);

    let added = play::parse_teams(&packet(play::clientbound::TEAMS, TEAM_ADD_MEMBER))
        .expect("team membership");
    assert_eq!(
        added,
        play::TeamUpdate::AddPlayers {
            name: "team_14".into(),
            players: vec!["🍫".into()],
        }
    );
}

#[test]
fn attack_uses_the_vanilla_action_ordinal() {
    let packet = play::attack_entity(300);
    assert_eq!(packet.id, play::serverbound::USE_ENTITY);
    // VarInt 300 = 0xAC 0x02, then ATTACK = 1, and nothing else: an attack has
    // no trailing vector.
    assert_eq!(packet.data, vec![0xAC, 0x02, 0x01]);
    assert_eq!(
        play::parse_use_entity(&packet).expect("parse attack"),
        (300, play::use_entity::ATTACK, None)
    );
}

#[test]
fn interact_at_carries_the_relative_hit_vector() {
    let packet = play::interact_entity_at(42, [0.25, 1.5, -0.75]);
    assert_eq!(packet.id, play::serverbound::USE_ENTITY);
    assert_eq!(packet.data.len(), 14, "id + action + three f32s");
    assert_eq!(packet.data[1], play::use_entity::INTERACT_AT as u8);

    let (id, action, target) = play::parse_use_entity(&packet).expect("parse interact-at");
    assert_eq!((id, action), (42, play::use_entity::INTERACT_AT));
    assert_eq!(target, Some([0.25, 1.5, -0.75]));
    // A well-formed packet leaves no payload bytes unread.
    assert_eq!(packet.consumed(), packet.data.len());
}

#[test]
fn interact_without_a_target_has_no_vector() {
    let packet = play::interact_entity(7);
    assert_eq!(packet.id, play::serverbound::USE_ENTITY);
    assert_eq!(packet.data, vec![7, play::use_entity::INTERACT as u8]);
    assert_eq!(
        play::parse_use_entity(&packet).expect("parse interact"),
        (7, play::use_entity::INTERACT, None)
    );
}

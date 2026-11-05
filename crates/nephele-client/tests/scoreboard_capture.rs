//! Replay the captured Hypixel lobby session through the scoreboard state.
//!
//! `fixtures/hypixel_play_frames.bin` is a raw play-state capture: every frame
//! is a VarInt length followed by a payload whose first field is the packet id.
//! The session is the acceptance case for `S3B`/`S3C`/`S3D`/`S3E`: a real
//! server's sidebar with real team formatting.

use nephele_client::Scoreboard;
use nephele_protocol::packets::play;
use nephele_protocol::Packet;

const FRAMES: &[u8] = include_bytes!("fixtures/hypixel_play_frames.bin");

fn read_varint(bytes: &[u8], at: &mut usize) -> Option<i32> {
    let mut value = 0u32;
    for shift in 0..5 {
        let byte = *bytes.get(*at)?;
        *at += 1;
        value |= ((byte & 0x7F) as u32) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some(value as i32);
        }
    }
    None
}

#[test]
fn captured_lobby_sidebar_has_rows_in_order_with_colours() {
    let mut board = Scoreboard::new();
    let mut at = 0;
    let mut applied = 0usize;
    while at < FRAMES.len() {
        let length = read_varint(FRAMES, &mut at).expect("frame length") as usize;
        let end = at + length;
        let packet = Packet::from_payload(&FRAMES[at..end]).expect("frame payload");
        at = end;
        match packet.id {
            play::clientbound::SCOREBOARD_OBJECTIVE => {
                board.apply_objective(
                    play::parse_scoreboard_objective(&packet).expect("objective packet"),
                );
                applied += 1;
            }
            play::clientbound::UPDATE_SCORE => {
                board.apply_score(play::parse_update_score(&packet).expect("score packet"));
                applied += 1;
            }
            play::clientbound::DISPLAY_SCOREBOARD => {
                board.apply_display(
                    play::parse_display_scoreboard(&packet).expect("display packet"),
                );
                applied += 1;
            }
            play::clientbound::TEAMS => {
                board.apply_team(play::parse_teams(&packet).expect("team packet"));
                applied += 1;
            }
            _ => {}
        }
    }
    assert!(
        applied > 300,
        "the capture holds scoreboard traffic ({applied} packets)"
    );

    let objective = board.sidebar().expect("a sidebar objective");
    assert_eq!(objective.name, "Prototype");
    assert_eq!(objective.display_name, "§e§lBED WARS");
    assert_eq!(objective.render_type, "integer");

    let rows = board.sidebar_rows();
    let entries: Vec<&str> = rows.iter().map(|row| row.entry.as_str()).collect();
    assert_eq!(
        entries,
        [
            "🍫", "💣", "👽", "🔮", "🐍", "👾", "🌠", "🍭", "⚽", "🏀", "👹", "🎁", "🎉", "🎂",
        ],
        "the sidebar is ordered by descending score"
    );
    let scores: Vec<i32> = rows.iter().map(|row| row.score).collect();
    assert_eq!(scores, (1..=14).rev().collect::<Vec<_>>());

    for row in &rows {
        assert!(row.team.is_some(), "{} has no team", row.entry);
        // Every `team_N` in this lobby is created with colour id 15 (white) and
        // carries its colour codes inside the prefix/suffix instead.
        assert_eq!(row.color, Some(15), "{} colour", row.entry);
        assert_eq!(
            row.text(),
            format!("{}{}{}", row.prefix, row.entry, row.suffix)
        );
    }

    // A name resolves to its team prefix, and the prefix is the lobby's line
    // formatting (the date and the row's column).
    let top = &rows[0];
    assert_eq!(board.team_name_for("🍫"), Some("team_14"));
    assert_eq!(board.prefix_for("🍫"), Some("§711/03/26  §8L2"));
    assert_eq!(board.format_entry("🍫"), "§711/03/26  §8L2🍫§85F");
    assert_eq!(top.text(), board.format_entry("🍫"));

    // The captured teams also include real player-name teams whose members
    // carry formatting codes; a plain name still resolves.
    board.apply_team(play::TeamUpdate::Create {
        name: "mvp".into(),
        info: play::TeamInfo {
            display_name: "mvp".into(),
            prefix: "§b[MVP§c++§b] ".into(),
            suffix: String::new(),
            friendly_flags: 3,
            name_tag_visibility: "always".into(),
            color: 11,
        },
        players: vec!["§eFelix Nevada".into()],
    });
    assert_eq!(board.prefix_for("Felix Nevada"), Some("§b[MVP§c++§b] "));
}

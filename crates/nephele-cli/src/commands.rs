//! Client-side chat commands.
//!
//! Vanilla runs `/commands` on the server. Nephele needs a few that work with
//! no server at all — the mock and singleplayer have nothing to run them — so
//! these are parsed and executed locally and are never sent. A command this
//! module does not recognise is forwarded to the server instead, so real
//! server commands keep working.

use nephele_world::blocks;

/// A `/command` this client knows how to run itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Help,
    Gamemode(GameMode),
    Teleport {
        x: f64,
        y: f64,
        z: f64,
    },
    SetBlock {
        x: i32,
        y: i32,
        z: i32,
        id: u16,
        meta: u8,
    },
    Give {
        id: u16,
        meta: u8,
        count: i8,
    },
    Seed,
    Time(i64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Survival,
    Creative,
}

/// What a typed line means.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    /// One of ours, ready to run.
    Command(Command),
    /// One of ours with bad arguments: show this to the player.
    Error(String),
    /// Not one of ours: send it to the server unchanged.
    Unknown,
}

pub const HELP: [&str; 6] = [
    "/help - this list",
    "/gamemode <creative|survival> - change the local game mode",
    "/tp <x> <y> <z> - teleport (accepts ~)",
    "/setblock <x> <y> <z> <block> - change a block (accepts ~)",
    "/time set <day|noon|night|midnight|ticks> - local time",
    "/seed - the world seed (singleplayer)",
];

/// Parse `line` with the leading `/` already removed.
///
/// `position` resolves `~` offsets; `seed` is what `/seed` reports.
pub fn parse(line: &str, position: [f64; 3]) -> Parsed {
    let mut words = line.split_whitespace();
    let Some(name) = words.next() else {
        return Parsed::Error("empty command".to_string());
    };

    match name {
        "help" => Parsed::Command(Command::Help),
        "seed" => Parsed::Command(Command::Seed),
        "gamemode" => match words.next() {
            Some("creative" | "c" | "1") => Parsed::Command(Command::Gamemode(GameMode::Creative)),
            Some("survival" | "s" | "0") => Parsed::Command(Command::Gamemode(GameMode::Survival)),
            Some(other) => Parsed::Error(format!("unknown game mode: {other}")),
            None => Parsed::Error("usage: /gamemode <creative|survival>".to_string()),
        },
        "tp" | "teleport" => {
            let Some(x) = words.next() else {
                return Parsed::Error("usage: /tp <x> <y> <z>".to_string());
            };
            let Some(y) = words.next() else {
                return Parsed::Error("usage: /tp <x> <y> <z>".to_string());
            };
            let Some(z) = words.next() else {
                return Parsed::Error("usage: /tp <x> <y> <z>".to_string());
            };
            match (
                coordinate(x, position[0]),
                coordinate(y, position[1]),
                coordinate(z, position[2]),
            ) {
                (Ok(x), Ok(y), Ok(z)) => Parsed::Command(Command::Teleport { x, y, z }),
                (Err(message), _, _) | (_, Err(message), _) | (_, _, Err(message)) => {
                    Parsed::Error(message)
                }
            }
        }
        "setblock" => {
            let mut next = || words.next().map(str::to_string);
            let (Some(x), Some(y), Some(z), Some(block)) = (next(), next(), next(), next()) else {
                return Parsed::Error("usage: /setblock <x> <y> <z> <block>".to_string());
            };
            let (x, y, z) = match (
                block_coordinate(&x, position[0]),
                block_coordinate(&y, position[1]),
                block_coordinate(&z, position[2]),
            ) {
                (Ok(x), Ok(y), Ok(z)) => (x, y, z),
                (Err(message), _, _) | (_, Err(message), _) | (_, _, Err(message)) => {
                    return Parsed::Error(message);
                }
            };
            match blocks::by_name(&block) {
                Some((id, meta)) => Parsed::Command(Command::SetBlock { x, y, z, id, meta }),
                None => Parsed::Error(format!("unknown block: {block}")),
            }
        }
        "time" => {
            let Some("set") = words.next() else {
                return Parsed::Error(
                    "usage: /time set <day|noon|night|midnight|ticks>".to_string(),
                );
            };
            let Some(value) = words.next() else {
                return Parsed::Error(
                    "usage: /time set <day|noon|night|midnight|ticks>".to_string(),
                );
            };
            let time = match value {
                "day" | "morning" => 1_000,
                "noon" => 6_000,
                "night" | "sunset" => 13_000,
                "midnight" => 18_000,
                other => match other.parse::<i64>() {
                    Ok(time) => time,
                    Err(_) => return Parsed::Error(format!("bad time: {other}")),
                },
            };
            Parsed::Command(Command::Time(time.rem_euclid(24_000)))
        }
        "give" => {
            let Some(block) = words.next() else {
                return Parsed::Error("usage: /give <block> [count]".to_string());
            };
            let Some((id, meta)) = blocks::by_name(block) else {
                return Parsed::Error(format!("unknown block: {block}"));
            };
            let count = match words.next() {
                Some(text) => match text.parse::<i32>() {
                    Ok(count) => count.clamp(1, 64) as i8,
                    Err(_) => return Parsed::Error(format!("bad count: {text}")),
                },
                None => 1,
            };
            Parsed::Command(Command::Give { id, meta, count })
        }
        _ => Parsed::Unknown,
    }
}

/// `~` means "relative to here"; `~5` is five blocks along. Everything else is
/// an absolute coordinate.
fn coordinate(token: &str, origin: f64) -> Result<f64, String> {
    match token.strip_prefix('~') {
        Some("") => Ok(origin),
        Some(offset) => offset
            .parse::<f64>()
            .map(|delta| origin + delta)
            .map_err(|_| format!("bad coordinate: {token}")),
        None => token
            .parse::<f64>()
            .map_err(|_| format!("bad coordinate: {token}")),
    }
}

fn block_coordinate(token: &str, origin: f64) -> Result<i32, String> {
    Ok(coordinate(token, origin)?.floor() as i32)
}

/// One line of feedback, ready for the chat log.
pub fn help_lines() -> Vec<String> {
    HELP.iter().map(|line| line.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HERE: [f64; 3] = [10.5, 64.0, -3.25];

    fn parsed(line: &str) -> Parsed {
        parse(line, HERE)
    }

    #[test]
    fn help_and_seed_need_no_arguments() {
        assert_eq!(parsed("help"), Parsed::Command(Command::Help));
        assert_eq!(parsed("seed"), Parsed::Command(Command::Seed));
    }

    #[test]
    fn gamemode_accepts_the_vanilla_spellings() {
        for text in ["creative", "c", "1"] {
            assert_eq!(
                parsed(&format!("gamemode {text}")),
                Parsed::Command(Command::Gamemode(GameMode::Creative))
            );
        }
        for text in ["survival", "s", "0"] {
            assert_eq!(
                parsed(&format!("gamemode {text}")),
                Parsed::Command(Command::Gamemode(GameMode::Survival))
            );
        }
        assert!(matches!(parsed("gamemode spectator"), Parsed::Error(_)));
        assert!(matches!(parsed("gamemode"), Parsed::Error(_)));
    }

    #[test]
    fn teleport_resolves_absolute_and_relative_coordinates() {
        assert_eq!(
            parsed("tp 1 2 3"),
            Parsed::Command(Command::Teleport {
                x: 1.0,
                y: 2.0,
                z: 3.0
            })
        );
        assert_eq!(
            parsed("tp ~ ~5 ~-2"),
            Parsed::Command(Command::Teleport {
                x: 10.5,
                y: 69.0,
                z: -5.25
            })
        );
        assert!(matches!(parsed("tp ~ nope ~"), Parsed::Error(_)));
        assert!(matches!(parsed("tp 1 2"), Parsed::Error(_)));
    }

    #[test]
    fn setblock_takes_a_block_name_and_relative_positions() {
        assert_eq!(
            parsed("setblock 0 64 0 stone"),
            Parsed::Command(Command::SetBlock {
                x: 0,
                y: 64,
                z: 0,
                id: 1,
                meta: 0
            })
        );
        // Block coordinates floor, so ~0.7 stays in the block you stand on.
        assert_eq!(
            parsed("setblock ~ ~0.7 ~ minecraft:red_wool"),
            Parsed::Command(Command::SetBlock {
                x: 10,
                y: 64,
                z: -4,
                id: 35,
                meta: 14
            })
        );
        assert!(matches!(
            parsed("setblock 0 64 0 not_a_block"),
            Parsed::Error(_)
        ));
        assert!(matches!(parsed("setblock 0 64"), Parsed::Error(_)));
    }

    #[test]
    fn give_clamps_the_count() {
        assert_eq!(
            parsed("give stone 5"),
            Parsed::Command(Command::Give {
                id: 1,
                meta: 0,
                count: 5
            })
        );
        assert_eq!(
            parsed("give dirt"),
            Parsed::Command(Command::Give {
                id: 3,
                meta: 0,
                count: 1
            })
        );
        assert!(matches!(
            parsed("give stone 999"),
            Parsed::Command(Command::Give { count: 64, .. })
        ));
    }

    #[test]
    fn time_set_accepts_names_and_ticks() {
        assert_eq!(
            parsed("time set night"),
            Parsed::Command(Command::Time(13_000))
        );
        assert_eq!(
            parsed("time set 25000"),
            Parsed::Command(Command::Time(1_000))
        );
        assert!(matches!(parsed("time set nope"), Parsed::Error(_)));
        assert!(matches!(parsed("time"), Parsed::Error(_)));
    }

    /// Anything that is not ours must reach the server untouched.
    #[test]
    fn unknown_commands_are_forwarded() {
        assert_eq!(parsed("spawn"), Parsed::Unknown);
        assert_eq!(parsed("tpaccept Steve"), Parsed::Unknown);
        assert_eq!(parsed("warp shop"), Parsed::Unknown);
    }
}

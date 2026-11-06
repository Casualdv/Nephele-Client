//! The scoreboard sidebar: model, ordering and right-aligned layout.
//!
//! Protocol 47 carries a scoreboard in four packets (`S3B` objective, `S3C`
//! update score, `S3D` display objective, `S3E` team). None of those are parsed
//! yet — that is the `net` lane's half — so this module owns the *model* and the
//! *renderer*, and [`Scoreboard::demo`] is a hand-built model that exercises the
//! layout without a server. Vanilla shows at most fifteen entries on the
//! sidebar; a sixteenth is dropped, not squeezed in.

use crate::chat::{self, Spans};

/// Vanilla's sidebar cap (`GuiIngame` draws at most 15 `ScoreboardObjective`
/// entries).
pub const MAX_ROWS: usize = 15;

/// One objective entry: the display name and its score.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoreEntry {
    pub name: String,
    pub score: i32,
}

/// One `S3B` objective. Only the display title is needed for the sidebar; the
/// criterion is kept so a future objectives screen has something to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Objective {
    pub name: String,
    pub title: Spans,
    pub entries: Vec<ScoreEntry>,
}

impl Objective {
    pub fn new(name: impl Into<String>, title: Spans) -> Self {
        Self {
            name: name.into(),
            title,
            entries: Vec::new(),
        }
    }

    /// `S3C` inserts or updates the score for one entry.
    pub fn set_score(&mut self, name: &str, score: i32) {
        match self.entries.iter_mut().find(|entry| entry.name == name) {
            Some(entry) => entry.score = score,
            None => self.entries.push(ScoreEntry {
                name: name.to_string(),
                score,
            }),
        }
    }

    /// `S3C` with an empty score name removes that entry.
    ///
    /// Nothing calls this yet: the packets that would are the net lane's half.
    /// It sits next to [`set_score`](Self::set_score) so that half has the whole
    /// mutating API ready.
    #[allow(dead_code)]
    pub fn remove(&mut self, name: &str) {
        self.entries.retain(|entry| entry.name != name);
    }
}

/// One `S3E` team. A team's prefix and suffix are chat components in vanilla;
/// the sidebar uses them as a name decoration, which is the only place this
/// client has a consumer for them today.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Team {
    pub name: String,
    pub display_name: String,
    pub prefix: Spans,
    pub suffix: Spans,
    pub players: Vec<String>,
}

impl Team {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            display_name: name.clone(),
            name,
            ..Self::default()
        }
    }

    pub fn add_player(&mut self, player: &str) {
        if !self.players.iter().any(|known| known == player) {
            self.players.push(player.to_string());
        }
    }

    pub fn has_player(&self, player: &str) -> bool {
        self.players.iter().any(|known| known == player)
    }
}

/// A row ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub spans: Spans,
    /// Score as it appears on the right, already formatted.
    pub score: String,
    pub value: i32,
}

/// The scoreboard the client knows about.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scoreboard {
    /// The objective currently shown on the sidebar, if any.
    pub sidebar: Option<Objective>,
    pub teams: Vec<Team>,
}

impl Scoreboard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.sidebar.is_none()
    }

    pub fn team_of(&self, player: &str) -> Option<&Team> {
        self.teams.iter().find(|team| team.has_player(player))
    }

    /// The sidebar's rows: highest score first (ties keep insertion order, as
    /// `ScoreboardObjective` does), capped at [`MAX_ROWS`], with the owning
    /// team's prefix/suffix folded into the name.
    pub fn sidebar_rows(&self) -> Vec<Row> {
        let Some(objective) = self.sidebar.as_ref() else {
            return Vec::new();
        };
        let mut entries: Vec<&ScoreEntry> = objective.entries.iter().collect();
        // `sort_by_key` is stable, so equal scores keep the order the server
        // sent.
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.score));

        entries
            .into_iter()
            .take(MAX_ROWS)
            .map(|entry| {
                let mut spans: Spans = Vec::new();
                if let Some(team) = self.team_of(&entry.name) {
                    spans.extend(team.prefix.iter().cloned());
                }
                spans.extend(chat::parse(&entry.name));
                if let Some(team) = self.team_of(&entry.name) {
                    spans.extend(team.suffix.iter().cloned());
                }
                Row {
                    spans,
                    score: entry.score.to_string(),
                    value: entry.score,
                }
            })
            .collect()
    }

    /// A hand-built model for layout work and for the acceptance screenshot.
    ///
    /// It is *not* server data: it is only shown when
    /// `NEPHELE_SCOREBOARD_DEMO=1` is set, so a real match never displays
    /// invented numbers.
    pub fn demo() -> Self {
        let title = chat::parse("§e§lHYPIXEL NETWORK");
        let mut objective = Objective::new("lobby", title);
        let rows: [(&str, i32); 16] = [
            ("§7§m                        ", 15),
            ("§a§lSERVER", 14),
            ("§fLobby: §a#1", 13),
            ("§fPlayers: §a412", 12),
            ("§7§m                       ", 11),
            ("§e§lYOUR STATS", 10),
            ("§7Notch", 9),
            ("§fLevel: §a74", 8),
            ("§fCoins: §6542,910", 7),
            ("§7§m                      ", 6),
            ("§e§lCLICK TO PLAY", 5),
            ("§f  §e§lSKYBLOCK", 4),
            ("§f  §e§lBEDWARS", 3),
            ("§f  §e§lDUELS", 2),
            ("§7store.hypixel.net", 1),
            // A sixteenth row: the renderer must drop it, exactly like vanilla.
            ("§cnever shown", 0),
        ];
        for (name, score) in rows {
            objective.set_score(name, score);
        }

        let mut admin = Team::new("admin");
        admin.prefix = chat::parse("§c§l[ADMIN] ");
        admin.suffix = chat::parse("§r");
        admin.add_player("§7Notch");

        Self {
            sidebar: Some(objective),
            teams: vec![admin],
        }
    }
}

/// Where the sidebar goes, in GUI pixels.
///
/// The pane hugs the right edge and is centred vertically. `screen` is the GUI
/// space size (`window size / gui scale`), so callers pass the unscaled width
/// and height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// Left edge of the text column.
    pub text_x: f32,
    /// Right edge the score numbers are aligned to.
    pub score_right: f32,
    /// Top of the first row.
    pub first_row_y: f32,
    pub row_h: f32,
    /// Top of the title, if there is one.
    pub title_y: f32,
    pub has_title: bool,
}

/// Pixels between the pane and the screen edge.
const EDGE_PAD: f32 = 2.0;
/// Pixels of inner padding on each side.
const TEXT_PAD: f32 = 2.0;
/// One row of text, matching the bitmap font's 9 px line.
pub const ROW_H: f32 = 9.0;
/// Title strip above the first row.
pub const TITLE_H: f32 = 11.0;
/// Minimum space between a name and its score.
const SCORE_GAP: f32 = 4.0;

/// Lay the sidebar out for rows measured as `(name width, score width)`.
pub fn layout(rows: &[(f32, f32)], title_width: Option<f32>, screen: (f32, f32)) -> Layout {
    let rows = &rows[..rows.len().min(MAX_ROWS)];
    let content_width = rows
        .iter()
        .map(|(name, score)| name + SCORE_GAP + score)
        .fold(0.0f32, f32::max);
    let has_title = title_width.is_some();
    let width = content_width.max(title_width.unwrap_or(0.0)) + TEXT_PAD * 2.0;
    let height = rows.len() as f32 * ROW_H + if has_title { TITLE_H } else { 0.0 };

    let x = screen.0 - EDGE_PAD - width;
    let y = ((screen.1 - height) / 2.0).max(EDGE_PAD);
    Layout {
        x,
        y,
        width,
        height,
        text_x: x + TEXT_PAD,
        score_right: x + width - TEXT_PAD,
        first_row_y: y + if has_title { TITLE_H } else { 0.0 },
        row_h: ROW_H,
        title_y: y,
        has_title,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoreboard_with(scores: &[(&str, i32)]) -> Scoreboard {
        let mut objective = Objective::new("sidebar", chat::parse("title"));
        for (name, score) in scores {
            objective.set_score(name, *score);
        }
        Scoreboard {
            sidebar: Some(objective),
            teams: Vec::new(),
        }
    }

    #[test]
    fn entries_sort_by_score_descending() {
        let board = scoreboard_with(&[("low", 1), ("high", 9), ("mid", 5)]);
        let names: Vec<i32> = board.sidebar_rows().iter().map(|row| row.value).collect();
        assert_eq!(names, vec![9, 5, 1]);
    }

    #[test]
    fn equal_scores_keep_insertion_order() {
        let board = scoreboard_with(&[("first", 3), ("second", 3)]);
        let rows = board.sidebar_rows();
        assert_eq!(chat::plain(&rows[0].spans), "first");
        assert_eq!(chat::plain(&rows[1].spans), "second");
    }

    #[test]
    fn a_sixteenth_row_is_dropped() {
        let entries: Vec<(String, i32)> = (0..16)
            .map(|index| (format!("row{index}"), index))
            .collect();
        let scores: Vec<(&str, i32)> = entries
            .iter()
            .map(|(name, score)| (name.as_str(), *score))
            .collect();
        let board = scoreboard_with(&scores);
        let rows = board.sidebar_rows();
        assert_eq!(rows.len(), MAX_ROWS);
        assert_eq!(
            chat::plain(&rows[0].spans),
            "row15",
            "the highest score sorts first"
        );
    }

    #[test]
    fn set_score_updates_in_place_instead_of_duplicating() {
        let mut objective = Objective::new("o", chat::parse("t"));
        objective.set_score("a", 1);
        objective.set_score("a", 7);
        assert_eq!(objective.entries.len(), 1);
        assert_eq!(objective.entries[0].score, 7);

        // `S3C` with an empty name removes the entry.
        objective.remove("a");
        assert!(objective.entries.is_empty());
    }

    #[test]
    fn team_prefix_and_suffix_wrap_the_name() {
        let mut board = scoreboard_with(&[("Steve", 4)]);
        let mut team = Team::new("red");
        team.prefix = chat::parse("§c[R] ");
        team.suffix = chat::parse("§r!");
        team.add_player("Steve");
        board.teams.push(team);

        let rows = board.sidebar_rows();
        assert_eq!(chat::plain(&rows[0].spans), "[R] Steve!");
        assert_eq!(rows[0].spans[0].color, chat::PALETTE[12]);
    }

    #[test]
    fn an_empty_scoreboard_draws_nothing() {
        assert!(Scoreboard::new().sidebar_rows().is_empty());
    }

    #[test]
    fn layout_hugs_the_right_edge_and_centres_vertically() {
        let rows = vec![(100.0, 10.0), (80.0, 5.0)];
        let layout = layout(&rows, Some(60.0), (320.0, 240.0));

        assert_eq!(layout.width, 100.0 + SCORE_GAP + 10.0 + TEXT_PAD * 2.0);
        assert_eq!(layout.x + layout.width, 320.0 - EDGE_PAD);
        assert_eq!(layout.height, TITLE_H + 2.0 * ROW_H);
        assert!((layout.y + layout.height / 2.0 - 120.0).abs() < 0.001);
        assert_eq!(layout.first_row_y, layout.y + TITLE_H);
        assert_eq!(layout.score_right, layout.x + layout.width - TEXT_PAD);
    }

    #[test]
    fn the_title_widens_the_pane_but_not_the_past_screen_edge() {
        let layout = layout(&[(10.0, 1.0)], Some(200.0), (320.0, 240.0));
        assert_eq!(layout.width, 200.0 + TEXT_PAD * 2.0);
        assert_eq!(layout.x + layout.width, 318.0);
    }

    #[test]
    fn layout_ignores_rows_past_the_cap() {
        let rows = vec![(50.0, 5.0); 20];
        let layout = layout(&rows, None, (320.0, 240.0));
        assert_eq!(layout.height, 15.0 * ROW_H);
        assert!(!layout.has_title);
    }

    #[test]
    fn the_demo_model_fits_the_sidebar() {
        let rows = Scoreboard::demo().sidebar_rows();
        assert_eq!(rows.len(), MAX_ROWS, "16 rows declared, 15 shown");
        assert!(rows.iter().all(|row| !row.score.is_empty()));
    }
}

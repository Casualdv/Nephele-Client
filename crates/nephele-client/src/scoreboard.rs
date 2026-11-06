//! Scoreboard, team and sidebar state (`S3B`, `S3C`, `S3D`, `S3E`).
//!
//! This is a faithful port of the parts of `net.minecraft.scoreboard.Scoreboard`
//! and `ScorePlayerTeam` that a client actually reads: objectives, the scores
//! attached to them, the display-slot map, teams with their prefix/suffix
//! formatting, and the resolver that turns a player name into its team's
//! formatted name.
//!
//! Nothing here renders. The renderer can ask for [`Scoreboard::sidebar_rows`]
//! and draw the result; the CLI can ask [`Scoreboard::format_entry`] to show a
//! player's team prefix.

use std::collections::{BTreeMap, HashMap};

use nephele_protocol::packets::play::{
    DisplayScoreboard, ObjectiveUpdate, ScoreUpdate, TeamInfo, TeamUpdate,
};

/// Vanilla only draws the fifteen highest scores in the sidebar.
pub const SIDEBAR_ROWS: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Objective {
    pub name: String,
    /// The title drawn above the sidebar.
    pub display_name: String,
    /// `integer` or `hearts`; only integer scores are exposed as numbers.
    pub render_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Team {
    pub name: String,
    pub display_name: String,
    pub prefix: String,
    pub suffix: String,
    pub friendly_flags: u8,
    pub name_tag_visibility: String,
    /// Vanilla colour id 0..=15, or `None` when the team has no colour.
    pub color: Option<u8>,
    pub players: Vec<String>,
}

/// One row of the sidebar, already resolved through its team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarRow {
    /// The raw score-holder string, as the server sent it.
    pub entry: String,
    pub score: i32,
    /// The team the entry belongs to, if any.
    pub team: Option<String>,
    /// `formatPlayerName`: team prefix, then the entry, then the suffix.
    pub prefix: String,
    pub suffix: String,
    /// The team's colour id, when it has one.
    pub color: Option<u8>,
}

impl SidebarRow {
    /// The row as the sidebar renders it, colour codes included.
    pub fn text(&self) -> String {
        format!("{}{}{}", self.prefix, self.entry, self.suffix)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scoreboard {
    objectives: BTreeMap<String, Objective>,
    /// `(objective, entry) -> value`.
    scores: BTreeMap<(String, String), i32>,
    /// Display slot -> objective name. `S3D` also addresses 3..=18 for
    /// team-coloured sidebars, so this is not limited to the three standard
    /// slots.
    displays: BTreeMap<u8, String>,
    teams: BTreeMap<String, Team>,
    /// Entry -> team name, kept in step with the teams above.
    team_of: HashMap<String, String>,
}

impl Scoreboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop everything, as a respawn does.
    pub fn clear(&mut self) {
        self.objectives.clear();
        self.scores.clear();
        self.displays.clear();
        self.teams.clear();
        self.team_of.clear();
    }

    pub fn objective(&self, name: &str) -> Option<&Objective> {
        self.objectives.get(name)
    }

    pub fn objectives(&self) -> impl Iterator<Item = &Objective> {
        self.objectives.values()
    }

    /// The objective displayed in a slot, if that objective still exists.
    pub fn display(&self, slot: u8) -> Option<&Objective> {
        self.displays
            .get(&slot)
            .and_then(|name| self.objectives.get(name))
    }

    /// The right-hand sidebar objective.
    pub fn sidebar(&self) -> Option<&Objective> {
        self.display(nephele_protocol::packets::play::scoreboard_slot::SIDEBAR)
    }

    pub fn teams(&self) -> impl Iterator<Item = &Team> {
        self.teams.values()
    }

    pub fn team(&self, name: &str) -> Option<&Team> {
        self.teams.get(name)
    }

    /// The team a scoreboard entry belongs to.
    ///
    /// Servers are inconsistent about whether team members carry colour codes
    /// (`§eFelix Nevada`) while callers ask about the plain name
    /// (`Felix Nevada`), so an exact match is tried first and a match ignoring
    /// formatting codes second.
    pub fn team_for(&self, entry: &str) -> Option<&Team> {
        if let Some(name) = self.team_of.get(entry) {
            return self.teams.get(name);
        }
        let plain = strip_formatting(entry);
        self.teams.values().find(|team| {
            team.players
                .iter()
                .any(|member| strip_formatting(member) == plain)
        })
    }

    /// The team name for an entry, if it has one.
    pub fn team_name_for(&self, entry: &str) -> Option<&str> {
        self.team_for(entry).map(|team| team.name.as_str())
    }

    /// A player's team prefix, the usual thing a chat or tab-list renderer
    /// wants. `None` when the name has no team.
    pub fn prefix_for(&self, entry: &str) -> Option<&str> {
        self.team_for(entry).map(|team| team.prefix.as_str())
    }

    pub fn suffix_for(&self, entry: &str) -> Option<&str> {
        self.team_for(entry).map(|team| team.suffix.as_str())
    }

    /// Vanilla `ScorePlayerTeam.formatPlayerName`: prefix, name, suffix.
    pub fn format_entry(&self, entry: &str) -> String {
        match self.team_for(entry) {
            Some(team) => format!("{}{}{}", team.prefix, entry, team.suffix),
            None => entry.to_owned(),
        }
    }

    pub fn apply_objective(&mut self, update: ObjectiveUpdate) {
        match update {
            ObjectiveUpdate::Create {
                name,
                display_name,
                render_type,
            }
            | ObjectiveUpdate::UpdateDisplayName {
                name,
                display_name,
                render_type,
            } => {
                self.objectives.insert(
                    name.clone(),
                    Objective {
                        name,
                        display_name,
                        render_type,
                    },
                );
            }
            ObjectiveUpdate::Remove { name } => {
                self.objectives.remove(&name);
                self.scores.retain(|(objective, _), _| objective != &name);
                self.displays.retain(|_, objective| objective != &name);
            }
        }
    }

    pub fn apply_score(&mut self, update: ScoreUpdate) {
        match update.value {
            Some(value) => {
                self.scores.insert((update.objective, update.entry), value);
            }
            None => {
                if update.objective.is_empty() {
                    // Vanilla: an empty objective removes the entry everywhere.
                    let entry = update.entry;
                    self.scores.retain(|(_, holder), _| holder != &entry);
                } else {
                    self.scores.remove(&(update.objective, update.entry));
                }
            }
        }
    }

    pub fn apply_display(&mut self, display: DisplayScoreboard) {
        if display.objective.is_empty() {
            self.displays.remove(&display.position);
        } else {
            self.displays.insert(display.position, display.objective);
        }
    }

    pub fn apply_team(&mut self, update: TeamUpdate) {
        match update {
            TeamUpdate::Create {
                name,
                info,
                players,
            } => {
                self.forget_members_of(&name);
                let mut team = team_from_info(name.clone(), info);
                team.players = Vec::new();
                self.teams.insert(name.clone(), team);
                for player in players {
                    self.add_member(&name, player);
                }
            }
            TeamUpdate::Remove { name } => {
                self.forget_members_of(&name);
                self.teams.remove(&name);
            }
            TeamUpdate::Update { name, info } => {
                let team = self
                    .teams
                    .entry(name.clone())
                    .or_insert_with(|| team_from_info(name.clone(), info.clone()));
                let players = std::mem::take(&mut team.players);
                *team = team_from_info(name, info);
                team.players = players;
            }
            TeamUpdate::AddPlayers { name, players } => {
                for player in players {
                    self.add_member(&name, player);
                }
            }
            TeamUpdate::RemovePlayers { name, players } => {
                if let Some(team) = self.teams.get_mut(&name) {
                    team.players.retain(|member| !players.contains(member));
                }
                for player in players {
                    if self.team_of.get(&player).is_some_and(|team| team == &name) {
                        self.team_of.remove(&player);
                    }
                }
            }
        }
    }

    /// The sidebar rows in the order the client draws them: highest score at
    /// the top, ties by name ascending (vanilla reverses its ascending list).
    pub fn sidebar_rows(&self) -> Vec<SidebarRow> {
        let Some(objective) = self.sidebar() else {
            return Vec::new();
        };
        let mut scores: Vec<(&String, i32)> = self
            .scores
            .iter()
            .filter(|((objective_name, entry), _)| {
                objective_name == &objective.name && !entry.starts_with('#')
            })
            .map(|((_, entry), value)| (entry, *value))
            .collect();
        scores.sort_by(|(name_a, score_a), (name_b, score_b)| {
            score_b
                .cmp(score_a)
                .then_with(|| name_a.to_lowercase().cmp(&name_b.to_lowercase()))
        });
        scores.truncate(SIDEBAR_ROWS);
        scores
            .into_iter()
            .map(|(entry, score)| {
                let team = self.team_for(entry);
                SidebarRow {
                    entry: entry.clone(),
                    score,
                    team: team.map(|team| team.name.clone()),
                    prefix: team.map(|team| team.prefix.clone()).unwrap_or_default(),
                    suffix: team.map(|team| team.suffix.clone()).unwrap_or_default(),
                    color: team.and_then(|team| team.color),
                }
            })
            .collect()
    }

    /// Every score on one objective, unsorted. Mostly for diagnostics.
    pub fn scores_for(&self, objective: &str) -> Vec<(String, i32)> {
        self.scores
            .iter()
            .filter(|((name, _), _)| name == objective)
            .map(|((_, entry), value)| (entry.clone(), *value))
            .collect()
    }

    fn add_member(&mut self, team_name: &str, player: String) {
        // Vanilla moves a player between teams rather than allowing two.
        if let Some(previous) = self.team_of.get(&player).cloned() {
            if previous == team_name {
                return;
            }
            if let Some(team) = self.teams.get_mut(&previous) {
                team.players.retain(|member| member != &player);
            }
        }
        let Some(team) = self.teams.get_mut(team_name) else {
            return;
        };
        if !team.players.contains(&player) {
            team.players.push(player.clone());
        }
        self.team_of.insert(player, team_name.to_owned());
    }

    fn forget_members_of(&mut self, team_name: &str) {
        self.team_of.retain(|_, team| team != team_name);
        if let Some(team) = self.teams.get_mut(team_name) {
            team.players.clear();
        }
    }
}

fn team_from_info(name: String, info: TeamInfo) -> Team {
    Team {
        name,
        display_name: info.display_name,
        prefix: info.prefix,
        suffix: info.suffix,
        friendly_flags: info.friendly_flags,
        name_tag_visibility: info.name_tag_visibility,
        color: u8::try_from(info.color).ok().filter(|color| *color <= 15),
        players: Vec::new(),
    }
}

/// Remove `§`-style formatting codes, the way `StringUtils.stripControlCodes`
/// does, so a formatted team member can be found by its plain name.
pub fn strip_formatting(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '§' {
            chars.next();
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use nephele_protocol::packets::play::scoreboard_slot;

    fn objective(name: &str, display: &str) -> ObjectiveUpdate {
        ObjectiveUpdate::Create {
            name: name.into(),
            display_name: display.into(),
            render_type: "integer".into(),
        }
    }

    fn score(entry: &str, objective: &str, value: i32) -> ScoreUpdate {
        ScoreUpdate {
            entry: entry.into(),
            objective: objective.into(),
            value: Some(value),
        }
    }

    fn team(name: &str, prefix: &str, suffix: &str, color: i8, players: &[&str]) -> TeamUpdate {
        TeamUpdate::Create {
            name: name.into(),
            info: TeamInfo {
                display_name: name.into(),
                prefix: prefix.into(),
                suffix: suffix.into(),
                friendly_flags: 3,
                name_tag_visibility: "always".into(),
                color,
            },
            players: players.iter().map(|name| name.to_string()).collect(),
        }
    }

    #[test]
    fn sidebar_rows_sort_by_score_then_name_and_carry_team_colour() {
        let mut board = Scoreboard::new();
        board.apply_objective(objective("board", "§e§lTEST"));
        board.apply_display(DisplayScoreboard {
            position: scoreboard_slot::SIDEBAR,
            objective: "board".into(),
        });
        board.apply_team(team("red", "§c[RED] ", "", 12, &["Steve"]));
        board.apply_team(team("blue", "§9[BLU] ", "", 11, &["Alex"]));
        board.apply_score(score("Steve", "board", 3));
        board.apply_score(score("Alex", "board", 7));
        board.apply_score(score("Zed", "board", 3));
        board.apply_score(score("#hidden", "board", 99));

        let rows = board.sidebar_rows();
        let entries: Vec<&str> = rows.iter().map(|row| row.entry.as_str()).collect();
        assert_eq!(entries, ["Alex", "Steve", "Zed"]);
        assert_eq!(rows[0].text(), "§9[BLU] Alex");
        assert_eq!(rows[0].color, Some(11));
        assert_eq!(rows[1].text(), "§c[RED] Steve");
        assert_eq!(rows[1].color, Some(12));
        assert_eq!(rows[2].color, None);
    }

    #[test]
    fn a_formatted_team_member_is_found_by_its_plain_name() {
        let mut board = Scoreboard::new();
        board.apply_team(team(
            "mvp",
            "§b[MVP§c++§b] ",
            " §7",
            11,
            &["§eFelix Nevada"],
        ));

        assert_eq!(board.prefix_for("Felix Nevada"), Some("§b[MVP§c++§b] "));
        assert_eq!(
            board.format_entry("Felix Nevada"),
            "§b[MVP§c++§b] Felix Nevada §7"
        );
        assert_eq!(board.team_name_for("§eFelix Nevada"), Some("mvp"));
        assert_eq!(board.prefix_for("someone else"), None);
    }

    #[test]
    fn players_move_between_teams_and_removal_unindexes_them() {
        let mut board = Scoreboard::new();
        board.apply_team(team("red", "§c", "", 12, &["Steve"]));
        board.apply_team(team("blue", "§9", "", 11, &[]));
        board.apply_team(TeamUpdate::AddPlayers {
            name: "blue".into(),
            players: vec!["Steve".into()],
        });

        assert_eq!(board.team_name_for("Steve"), Some("blue"));
        assert_eq!(board.team("red").unwrap().players.len(), 0);

        board.apply_team(TeamUpdate::Remove {
            name: "blue".into(),
        });
        assert_eq!(board.team_name_for("Steve"), None);
        assert_eq!(board.format_entry("Steve"), "Steve");
    }

    #[test]
    fn removing_an_objective_clears_its_scores_and_display() {
        let mut board = Scoreboard::new();
        board.apply_objective(objective("board", "TEST"));
        board.apply_display(DisplayScoreboard {
            position: scoreboard_slot::SIDEBAR,
            objective: "board".into(),
        });
        board.apply_score(score("Steve", "board", 1));

        board.apply_objective(ObjectiveUpdate::Remove {
            name: "board".into(),
        });

        assert!(board.sidebar().is_none());
        assert!(board.scores_for("board").is_empty());
    }

    #[test]
    fn a_score_remove_with_no_objective_drops_the_entry_everywhere() {
        let mut board = Scoreboard::new();
        board.apply_objective(objective("a", "A"));
        board.apply_objective(objective("b", "B"));
        board.apply_score(score("Steve", "a", 1));
        board.apply_score(score("Steve", "b", 2));

        board.apply_score(ScoreUpdate {
            entry: "Steve".into(),
            objective: String::new(),
            value: None,
        });

        assert!(board.scores_for("a").is_empty());
        assert!(board.scores_for("b").is_empty());
    }
}

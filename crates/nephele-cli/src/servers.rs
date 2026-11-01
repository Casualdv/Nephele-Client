//! The saved server list behind the multiplayer screen.
//!
//! Kept deliberately separate from the UI: what a list *is* (add, remove,
//! reorder, deduplicate, persist) and what a ping found out about each entry
//! are testable without a window, and the screen only has to draw them.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One saved server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEntry {
    /// What the player called it. Falls back to the address when blank.
    pub name: String,
    /// `host` or `host:port`, as typed.
    pub address: String,
    #[serde(default)]
    pub favourite: bool,
}

impl ServerEntry {
    pub fn new(name: &str, address: &str) -> Self {
        let name = name.trim();
        let address = address.trim();
        Self {
            name: if name.is_empty() {
                address.to_owned()
            } else {
                name.to_owned()
            },
            address: address.to_owned(),
            favourite: false,
        }
    }

    /// The label to show in the list.
    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.address
        } else {
            &self.name
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerList {
    #[serde(default)]
    entries: Vec<ServerEntry>,
}

impl ServerList {
    pub fn entries(&self) -> &[ServerEntry] {
        &self.entries
    }

    /// Add a server, replacing any existing entry with the same address.
    ///
    /// Re-adding is the common case (the player types the address again rather
    /// than hunting for it), so it updates in place instead of duplicating.
    pub fn add(&mut self, entry: ServerEntry) {
        if entry.address.is_empty() {
            return;
        }
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|saved| saved.address == entry.address)
        {
            let favourite = existing.favourite;
            *existing = ServerEntry { favourite, ..entry };
            return;
        }
        self.entries.push(entry);
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }

    /// Storage indices in display order: favourites first, otherwise the
    /// saved order is kept.
    ///
    /// Indices rather than references because the screen acts on an entry with
    /// [`ServerList::toggle_favourite`] / [`ServerList::remove`], which take one.
    pub fn ordered_indices(&self) -> Vec<usize> {
        let mut out: Vec<usize> = (0..self.entries.len()).collect();
        out.sort_by_key(|&index| !self.entries[index].favourite);
        out
    }

    pub fn toggle_favourite(&mut self, index: usize) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.favourite = !entry.favourite;
        }
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string());
        std::fs::write(path, json)
    }
}

/// What a server-list ping learned, in the columns the screen shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// The MOTD, formatting codes already stripped. May be empty or two lines.
    pub motd: String,
    /// The version string the server advertises, e.g. `1.8.9`.
    pub version: String,
    /// Round-trip time of the status exchange.
    pub ping_ms: u32,
}

/// One row's ping state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Ping {
    /// Not asked yet.
    #[default]
    Unknown,
    /// A request is in flight.
    Pending,
    Online(Status),
    /// The ping failed or timed out.
    Offline,
}

static UNKNOWN: Ping = Ping::Unknown;

/// Ping results, keyed by the address a row shows.
#[derive(Debug, Clone, Default)]
pub struct PingBoard {
    states: HashMap<String, Ping>,
}

impl PingBoard {
    pub fn of(&self, address: &str) -> &Ping {
        self.states.get(address).unwrap_or(&UNKNOWN)
    }

    /// Mark `address` as in flight. Returns whether a request should be
    /// started, so opening the screen twice cannot double up on one server.
    pub fn begin(&mut self, address: &str) -> bool {
        if self.of(address) == &Ping::Pending {
            return false;
        }
        self.states.insert(address.to_owned(), Ping::Pending);
        true
    }

    pub fn finish(&mut self, address: &str, ping: Ping) {
        self.states.insert(address.to_owned(), ping);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_name_falls_back_to_the_address() {
        let entry = ServerEntry::new("   ", "  play.example.net ");
        assert_eq!(entry.name, "play.example.net");
        assert_eq!(entry.address, "play.example.net");
        assert_eq!(entry.label(), "play.example.net");
    }

    #[test]
    fn adding_the_same_address_twice_updates_it() {
        let mut list = ServerList::default();
        list.add(ServerEntry::new("Old name", "a.example.net"));
        list.add(ServerEntry::new("New name", "a.example.net"));
        assert_eq!(list.entries().len(), 1);
        assert_eq!(list.entries()[0].name, "New name");
    }

    #[test]
    fn re_adding_keeps_the_favourite_flag() {
        let mut list = ServerList::default();
        list.add(ServerEntry::new("A", "a.example.net"));
        list.toggle_favourite(0);
        list.add(ServerEntry::new("A renamed", "a.example.net"));
        assert!(list.entries()[0].favourite);
    }

    #[test]
    fn an_empty_address_is_ignored() {
        let mut list = ServerList::default();
        list.add(ServerEntry::new("nothing", "   "));
        assert!(list.entries().is_empty());
    }

    #[test]
    fn favourites_sort_first_without_disturbing_the_rest() {
        let mut list = ServerList::default();
        list.add(ServerEntry::new("one", "1.example.net"));
        list.add(ServerEntry::new("two", "2.example.net"));
        list.add(ServerEntry::new("three", "3.example.net"));
        list.toggle_favourite(2);
        let ordered: Vec<&str> = list
            .ordered_indices()
            .into_iter()
            .map(|index| list.entries()[index].name.as_str())
            .collect();
        assert_eq!(ordered, vec!["three", "one", "two"]);
    }

    /// The screen acts on entries by index, so ordering must hand back the
    /// storage slot rather than a position in the sorted view.
    #[test]
    fn display_order_reports_storage_indices() {
        let mut list = ServerList::default();
        list.add(ServerEntry::new("one", "1.example.net"));
        list.add(ServerEntry::new("two", "2.example.net"));
        list.add(ServerEntry::new("three", "3.example.net"));
        list.toggle_favourite(2);
        assert_eq!(list.ordered_indices(), vec![2, 0, 1]);
    }

    #[test]
    fn removing_out_of_range_is_harmless() {
        let mut list = ServerList::default();
        list.add(ServerEntry::new("one", "1.example.net"));
        list.remove(9);
        assert_eq!(list.entries().len(), 1);
    }

    #[test]
    fn the_list_round_trips_through_disk() {
        let dir = std::env::temp_dir().join("nephele-server-test");
        let path = dir.join("servers.json");
        let _ = std::fs::remove_file(&path);

        let mut list = ServerList::default();
        list.add(ServerEntry::new("Home", "127.0.0.1:25565"));
        list.toggle_favourite(0);
        list.save(&path).expect("save");

        let loaded = ServerList::load(&path);
        assert_eq!(loaded.entries().len(), 1);
        assert_eq!(loaded.entries()[0].name, "Home");
        assert!(loaded.entries()[0].favourite);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_or_broken_file_loads_an_empty_list() {
        assert!(ServerList::load(Path::new("does-not-exist.json"))
            .entries()
            .is_empty());
    }

    #[test]
    fn a_fresh_board_knows_nothing() {
        let board = PingBoard::default();
        assert_eq!(board.of("a.example.net"), &Ping::Unknown);
    }

    #[test]
    fn starting_a_ping_marks_the_row_and_does_not_duplicate_it() {
        let mut board = PingBoard::default();
        assert!(board.begin("a.example.net"), "first ask starts a request");
        assert_eq!(board.of("a.example.net"), &Ping::Pending);
        assert!(!board.begin("a.example.net"), "already in flight");
        assert_eq!(board.of("a.example.net"), &Ping::Pending);
    }

    #[test]
    fn a_finished_ping_replaces_the_pending_state() {
        let mut board = PingBoard::default();
        board.begin("a.example.net");
        board.finish(
            "a.example.net",
            Ping::Online(Status {
                motd: "Hello".to_string(),
                version: "1.8.9".to_string(),
                ping_ms: 42,
            }),
        );
        assert_eq!(
            board.of("a.example.net"),
            &Ping::Online(Status {
                motd: "Hello".to_string(),
                version: "1.8.9".to_string(),
                ping_ms: 42,
            })
        );
        // A finished server can be asked again.
        assert!(board.begin("a.example.net"));
    }

    #[test]
    fn addresses_are_tracked_independently() {
        let mut board = PingBoard::default();
        board.begin("a.example.net");
        board.finish("b.example.net", Ping::Offline);
        assert_eq!(board.of("a.example.net"), &Ping::Pending);
        assert_eq!(board.of("b.example.net"), &Ping::Offline);
        assert_eq!(board.of("c.example.net"), &Ping::Unknown);
    }
}

//! The dimensions of Minecraft 1.8.9 and the few rules that vary between them.
//!
//! Vanilla keys a dimension by the `i8` in `Join Game` / `Respawn`:
//! `0` overworld, `-1` nether, `1` end. The id decides more than the generator:
//! the nether and the end have no sky, so their chunk sections carry **no
//! sky-light array at all** (`S21PacketChunkData` only writes it when
//! `!provider.getHasNoSky()`), and their generation height is 128 rather than
//! 256 (`World.getActualHeight`).

/// A dimension the client can generate or receive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dimension {
    #[default]
    Overworld,
    Nether,
    End,
}

impl Dimension {
    /// The `i8` the protocol uses.
    pub const fn id(self) -> i8 {
        match self {
            Dimension::Overworld => 0,
            Dimension::Nether => -1,
            Dimension::End => 1,
        }
    }

    /// The dimension for a protocol id, or `None` for one 1.8.9 does not have.
    pub const fn from_id(id: i8) -> Option<Self> {
        match id {
            0 => Some(Dimension::Overworld),
            -1 => Some(Dimension::Nether),
            1 => Some(Dimension::End),
            _ => None,
        }
    }

    /// `WorldProvider.getHasNoSky()` — false only for the overworld.
    pub const fn has_skylight(self) -> bool {
        matches!(self, Dimension::Overworld)
    }

    /// `World.getActualHeight()`: the height weather and clouds stop at.
    ///
    /// Generation always fills `0..256`; this is the cap vanilla uses for
    /// dimensions without a sky (`hasNoSky ? 128 : 256`).
    pub const fn actual_height(self) -> i32 {
        if self.has_skylight() {
            256
        } else {
            128
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Dimension::Overworld => "Overworld",
            Dimension::Nether => "Nether",
            Dimension::End => "The End",
        }
    }

    /// The lowercase name used on the command line and in logs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Dimension::Overworld => "overworld",
            Dimension::Nether => "nether",
            Dimension::End => "end",
        }
    }

    /// Parse a dimension name or protocol id (`overworld`/`0`, `nether`/`-1`,
    /// `end`/`1`).
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "overworld" | "0" => Some(Dimension::Overworld),
            "nether" | "hell" | "-1" => Some(Dimension::Nether),
            "end" | "the_end" | "1" => Some(Dimension::End),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_ids_round_trip() {
        for dimension in [Dimension::Overworld, Dimension::Nether, Dimension::End] {
            assert_eq!(Dimension::from_id(dimension.id()), Some(dimension));
        }
        assert_eq!(Dimension::from_id(2), None);
    }

    #[test]
    fn only_the_overworld_has_a_sky() {
        assert!(Dimension::Overworld.has_skylight());
        assert!(!Dimension::Nether.has_skylight());
        assert!(!Dimension::End.has_skylight());
        assert_eq!(Dimension::Nether.actual_height(), 128);
        assert_eq!(Dimension::Overworld.actual_height(), 256);
    }
}

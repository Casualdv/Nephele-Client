//! Map data (`S34`), stored per map id.
//!
//! A map is a 128x128 grid of colour indexes that the server patches
//! incrementally: each packet carries only the rectangle that changed. Framed
//! maps and held maps share the same ids, so the store is keyed by the id in
//! the packet and a renderer can look a framed map's picture up by the item's
//! damage value (in 1.8 a filled map's damage *is* its map id).

use std::collections::HashMap;

use nephele_protocol::packets::play::{MapIcon, MapUpdate};

/// One map's picture and markers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MapState {
    pub scale: u8,
    pub icons: Vec<MapIcon>,
    /// 128x128 colour indexes, row-major. 0 is "no colour"/transparent.
    pub colors: Vec<u8>,
    /// How many patches have been applied, which is the only way to tell a map
    /// that has data from one that has only had its markers sent.
    pub patches: u32,
}

impl MapState {
    pub const SIZE: usize = MapUpdate::SIZE;
    pub const AREA: usize = Self::SIZE * Self::SIZE;

    /// Apply one packet's patch.
    ///
    /// A packet with no columns only updates the markers. Out-of-range patches
    /// are clipped rather than trusted: a server is free to send a rectangle
    /// that runs off the grid, but writing past the end of the buffer would
    /// panic.
    pub fn apply(&mut self, update: &MapUpdate) {
        if self.colors.len() != Self::AREA {
            self.colors = vec![0; Self::AREA];
        }
        self.scale = update.scale;
        self.icons = update.icons.clone();
        if update.columns == 0 || update.rows == 0 {
            return;
        }
        for row in 0..update.rows as usize {
            let y = update.start_y as usize + row;
            if y >= Self::SIZE {
                break;
            }
            for column in 0..update.columns as usize {
                let x = update.start_x as usize + column;
                if x >= Self::SIZE {
                    break;
                }
                let source = row * update.columns as usize + column;
                if let Some(color) = update.colors.get(source) {
                    self.colors[y * Self::SIZE + x] = *color;
                }
            }
        }
        self.patches += 1;
    }

    /// Colour index at `(x, y)`, if the map has data.
    pub fn color_at(&self, x: usize, y: usize) -> Option<u8> {
        if x >= Self::SIZE || y >= Self::SIZE {
            return None;
        }
        self.colors.get(y * Self::SIZE + x).copied()
    }
}

/// Every map the client has received data for, keyed by map id.
#[derive(Debug, Clone, Default)]
pub struct Maps {
    map: HashMap<i32, MapState>,
}

impl Maps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&self, id: i32) -> Option<&MapState> {
        self.map.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&i32, &MapState)> {
        self.map.iter()
    }

    /// Apply a packet, creating the map on first sight.
    pub fn apply(&mut self, update: &MapUpdate) -> &mut MapState {
        let state = self.map.entry(update.map_id).or_default();
        state.apply(update);
        state
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(map_id: i32, columns: u8, rows: u8, start_x: u8, start_y: u8) -> MapUpdate {
        MapUpdate {
            map_id,
            scale: 1,
            icons: Vec::new(),
            columns,
            rows,
            start_x,
            start_y,
            colors: (0..columns as usize * rows as usize)
                .map(|index| (index + 1) as u8)
                .collect(),
        }
    }

    #[test]
    fn a_patch_lands_where_the_packet_says() {
        let mut maps = Maps::new();
        maps.apply(&update(3, 2, 2, 4, 5));

        let map = maps.get(3).expect("map");
        assert_eq!(map.colors.len(), MapState::AREA);
        assert_eq!(map.color_at(4, 5), Some(1));
        assert_eq!(map.color_at(5, 5), Some(2));
        assert_eq!(map.color_at(4, 6), Some(3));
        assert_eq!(map.color_at(5, 6), Some(4));
        // Everything off the patch is still untouched.
        assert_eq!(map.color_at(0, 0), Some(0));
        assert_eq!(map.patches, 1);
    }

    #[test]
    fn later_patches_overwrite_only_their_rectangle() {
        let mut maps = Maps::new();
        maps.apply(&update(1, 2, 2, 0, 0));
        maps.apply(&update(1, 1, 1, 1, 1));

        let map = maps.get(1).unwrap();
        assert_eq!(map.color_at(0, 0), Some(1));
        assert_eq!(map.color_at(1, 1), Some(1));
        assert_eq!(map.patches, 2);
    }

    #[test]
    fn a_marker_only_packet_keeps_the_picture() {
        let mut maps = Maps::new();
        maps.apply(&update(2, 1, 1, 0, 0));
        let mut markers = update(2, 0, 0, 0, 0);
        markers.icons.push(MapIcon {
            kind: 1,
            direction: 4,
            x: 10,
            z: -2,
        });
        maps.apply(&markers);

        let map = maps.get(2).unwrap();
        assert_eq!(map.color_at(0, 0), Some(1));
        assert_eq!(map.icons.len(), 1);
        assert_eq!(map.patches, 1);
    }

    #[test]
    fn patches_running_off_the_grid_are_clipped() {
        let mut maps = Maps::new();
        maps.apply(&update(9, 4, 4, 126, 127));
        // No panic, and the pixels that fit were written.
        let map = maps.get(9).unwrap();
        assert_eq!(map.color_at(126, 127), Some(1));
        assert_eq!(map.color_at(127, 127), Some(2));
    }
}

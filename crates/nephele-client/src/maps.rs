//! Map data (`S34`), stored per map id.
//!
//! A map is a 128x128 grid of colour indexes that the server patches
//! incrementally: each packet carries only the rectangle that changed. Framed
//! maps and held maps share the same ids, so the store is keyed by the id in
//! the packet and a renderer can look a framed map's picture up by the item's
//! damage value (in 1.8 a filled map's damage *is* its map id).
//!
//! **Wire parsing belongs to N.** This module is the pinned seam: N's parser
//! fills a [`nephele_protocol::packets::play::MapUpdate`] and calls
//! [`Maps::apply`]. Until that lands, [`MapState::placeholder`] supplies a
//! deterministic picture so a framed map is never blank.

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

    /// A deterministic picture for a map whose pixels N has not supplied yet.
    /// The indices go through [`map_color`], so this is a real sky/sun/ground
    /// image rather than a flat square.
    pub fn placeholder(id: i32) -> Self {
        let mut colors = vec![0u8; Self::AREA];
        let seed = id as u32;
        for y in 0..Self::SIZE {
            for x in 0..Self::SIZE {
                let border = x < 3 || y < 3 || x >= Self::SIZE - 3 || y >= Self::SIZE - 3;
                let sun = (x as i32 - 96).pow(2) + (y as i32 - 24).pow(2) < 11 * 11;
                let index = if border {
                    10 * 4 + 1 // brown frame edge
                } else if sun {
                    14 * 4 + 3 // white/yellow
                } else if y < 64 {
                    let cloud = ((x / 9 + y / 7 + seed as usize) % 11) == 0 && y > 12 && y < 48;
                    if cloud {
                        8 * 4 + 2 // white
                    } else {
                        12 * 4 + 2 // blue sky
                    }
                } else {
                    let n = (x as u32 * 31 + y as u32 * 17 + seed) % 13;
                    4 + (n % 3) as u8 // green ground shades
                };
                colors[y * Self::SIZE + x] = index;
            }
        }
        Self {
            scale: 0,
            icons: Vec::new(),
            colors,
            patches: 1,
        }
    }

    /// Flatten to the RGBA byte order the atlas upload expects.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::AREA * 4);
        for y in 0..Self::SIZE {
            for x in 0..Self::SIZE {
                let index = self.colors.get(y * Self::SIZE + x).copied().unwrap_or(0);
                out.extend_from_slice(&map_color(index));
            }
        }
        out
    }
}

/// The 1.8 map palette is 64 colours: 16 base colours × 4 brightness shades.
/// The exact table is Mojang data, so this derives an equivalent palette from
/// the base hues instead of copying it.
pub fn map_color(index: u8) -> [u8; 4] {
    let index = index & 0x3F;
    if index == 0 {
        return [0, 0, 0, 0];
    }
    const BASE: [[u8; 3]; 16] = [
        [0, 0, 0],
        [127, 178, 56],
        [247, 233, 163],
        [199, 199, 199],
        [255, 0, 0],
        [160, 160, 255],
        [167, 167, 167],
        [0, 124, 0],
        [255, 255, 255],
        [164, 168, 184],
        [151, 109, 77],
        [112, 112, 112],
        [64, 64, 255],
        [143, 119, 72],
        [255, 252, 245],
        [216, 127, 51],
    ];
    let base = BASE[(index >> 2) as usize];
    let shade = [0.45, 0.70, 1.0, 1.25][(index & 3) as usize];
    [
        (base[0] as f32 * shade).min(255.0) as u8,
        (base[1] as f32 * shade).min(255.0) as u8,
        (base[2] as f32 * shade).min(255.0) as u8,
        255,
    ]
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

    /// Get a map, creating the placeholder if N has not sent pixels yet.
    pub fn ensure(&mut self, id: i32) -> &mut MapState {
        self.map
            .entry(id)
            .or_insert_with(|| MapState::placeholder(id))
    }

    /// Replace one map with a state already merged on the network thread.
    pub fn insert(&mut self, id: i32, state: MapState) {
        self.map.insert(id, state);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&i32, &MapState)> {
        self.map.iter()
    }

    /// Apply a packet, creating the map on first sight. This is the seam N's
    /// `S34` parser calls.
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
        assert_eq!(map.patches, 1);
    }

    #[test]
    fn out_of_range_patches_are_clipped() {
        let mut maps = Maps::new();
        maps.apply(&update(4, 4, 4, 126, 126));
        let map = maps.get(4).expect("map");
        assert_eq!(map.color_at(127, 127), Some(6));
        // Nothing was written past the grid.
        assert_eq!(map.colors.len(), MapState::AREA);
    }

    #[test]
    fn a_marker_only_packet_keeps_the_picture() {
        let mut maps = Maps::new();
        maps.apply(&update(5, 1, 1, 0, 0));
        let mut marker_only = update(5, 0, 0, 0, 0);
        marker_only.icons.push(MapIcon {
            kind: 1,
            direction: 0,
            x: 2,
            z: 3,
        });
        maps.apply(&marker_only);
        let map = maps.get(5).expect("map");
        assert_eq!(map.color_at(0, 0), Some(1));
        assert_eq!(map.icons.len(), 1);
        assert_eq!(map.patches, 1);
    }

    #[test]
    fn a_placeholder_is_a_real_picture() {
        let map = MapState::placeholder(3);
        assert_eq!(map.colors.len(), MapState::AREA);
        let distinct: std::collections::HashSet<_> = map.colors.iter().collect();
        assert!(distinct.len() > 4, "placeholder should not be flat");
        assert_eq!(map.to_rgba().len(), MapState::AREA * 4);
    }

    #[test]
    fn palette_zero_is_transparent_and_others_are_opaque() {
        assert_eq!(map_color(0)[3], 0);
        assert_eq!(map_color(1)[3], 255);
        assert_eq!(map_color(63)[3], 255);
    }
}

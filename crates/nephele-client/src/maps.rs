//! Map item pixels.
//!
//! The wire format (`S34` / `S33`) carries 128×128 colour indices. This module
//! is the interface between M3's packet parsing and the renderer: packets land
//! in [`Maps::apply_chunk`] / [`Maps::apply_columns`], and the CLI asks for a
//! [`MapData`] to upload into the entity atlas. Until a server sends real data,
//! [`MapData::placeholder`] makes a framed map show a picture instead of a
//! blank square.

use std::collections::HashMap;

use nephele_protocol::packets::play;

/// Edge length of a map in pixels.
pub const MAP_SIZE: usize = 128;

/// RGBA pixels for one map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapData {
    pub id: i32,
    pub scale: i8,
    /// `MAP_SIZE * MAP_SIZE` pixels, row-major.
    pub pixels: Vec<[u8; 4]>,
}

impl MapData {
    pub fn empty(id: i32) -> Self {
        Self {
            id,
            scale: 0,
            pixels: vec![[0, 0, 0, 0]; MAP_SIZE * MAP_SIZE],
        }
    }

    /// A deterministic, recognisable picture so an unparsed map is not blank.
    pub fn placeholder(id: i32) -> Self {
        let mut data = Self::empty(id);
        let seed = id as u32;
        for y in 0..MAP_SIZE {
            for x in 0..MAP_SIZE {
                let border = x < 3 || y < 3 || x >= MAP_SIZE - 3 || y >= MAP_SIZE - 3;
                let sun = (x as i32 - 96).pow(2) + (y as i32 - 24).pow(2) < 11 * 11;
                let pixel = if border {
                    [74, 49, 27, 255]
                } else if sun {
                    [255, 220, 72, 255]
                } else if y < 64 {
                    // Sky with a couple of clouds.
                    let cloud = ((x / 9 + y / 7 + seed as usize) % 11) == 0 && y > 12 && y < 48;
                    if cloud {
                        [238, 238, 238, 255]
                    } else {
                        [92, 148, 214, 255]
                    }
                } else {
                    // Ground with a little deterministic texture.
                    let n = (x as u32 * 31 + y as u32 * 17 + seed) % 13;
                    [64 + (n * 3) as u8, 132 + (n * 2) as u8, 52, 255]
                };
                data.pixels[y * MAP_SIZE + x] = pixel;
            }
        }
        data
    }

    /// Copy a `S34 Map Chunk` rectangle into this map.
    pub fn apply_chunk(&mut self, chunk: &play::MapChunk) {
        self.scale = chunk.scale;
        self.blit(chunk.x, chunk.z, chunk.columns, chunk.rows, &chunk.colors);
    }

    /// Copy a `S33 Map Columns` rectangle into this map.
    pub fn apply_columns(&mut self, columns: &play::MapColumns) {
        self.blit(
            columns.x as i32,
            columns.z as i32,
            columns.columns,
            columns.rows,
            &columns.colors,
        );
    }

    fn blit(&mut self, origin_x: i32, origin_z: i32, columns: u8, rows: u8, colors: &[u8]) {
        for row in 0..rows as i32 {
            for column in 0..columns as i32 {
                let x = origin_x + column;
                let z = origin_z + row;
                if !(0..MAP_SIZE as i32).contains(&x) || !(0..MAP_SIZE as i32).contains(&z) {
                    continue;
                }
                let source = (row as usize * columns as usize) + column as usize;
                let Some(color) = colors.get(source) else {
                    continue;
                };
                self.pixels[z as usize * MAP_SIZE + x as usize] = map_color(*color);
            }
        }
    }

    /// Flatten to the RGBA byte order the atlas upload expects.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAP_SIZE * MAP_SIZE * 4);
        for pixel in &self.pixels {
            out.extend_from_slice(pixel);
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

/// Every map the server has described, keyed by map id.
#[derive(Debug, Clone, Default)]
pub struct Maps {
    maps: HashMap<i32, MapData>,
}

impl Maps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.maps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }

    pub fn get(&self, id: i32) -> Option<&MapData> {
        self.maps.get(&id)
    }

    /// Get a map, creating the placeholder if the server has not sent pixels.
    pub fn ensure(&mut self, id: i32) -> &mut MapData {
        self.maps
            .entry(id)
            .or_insert_with(|| MapData::placeholder(id))
    }

    /// Replace one map wholesale (the event path already carries the merged
    /// pixels, so the CLI does not need to replay the packet).
    pub fn insert(&mut self, data: MapData) {
        self.maps.insert(data.id, data);
    }

    pub fn apply_chunk(&mut self, chunk: &play::MapChunk) -> &MapData {
        let map = self
            .maps
            .entry(chunk.map_id)
            .or_insert_with(|| MapData::empty(chunk.map_id));
        map.apply_chunk(chunk);
        map
    }

    pub fn apply_columns(&mut self, columns: &play::MapColumns) -> &MapData {
        let map = self
            .maps
            .entry(columns.map_id)
            .or_insert_with(|| MapData::empty(columns.map_id));
        map.apply_columns(columns);
        map
    }

    pub fn clear(&mut self) {
        self.maps.clear();
    }

    pub fn iter(&self) -> impl Iterator<Item = (&i32, &MapData)> {
        self.maps.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_lands_at_the_requested_offset() {
        let mut maps = Maps::new();
        let chunk = play::MapChunk {
            map_id: 7,
            scale: 1,
            columns: 2,
            rows: 2,
            x: 10,
            z: 20,
            colors: vec![1, 2, 3, 4],
        };
        let map = maps.apply_chunk(&chunk);
        assert_eq!(map.scale, 1);
        assert_eq!(map.pixels[20 * MAP_SIZE + 10], map_color(1));
        assert_eq!(map.pixels[21 * MAP_SIZE + 11], map_color(4));
    }

    #[test]
    fn a_placeholder_is_a_real_picture() {
        let map = MapData::placeholder(3);
        assert_eq!(map.pixels.len(), MAP_SIZE * MAP_SIZE);
        let distinct: std::collections::HashSet<_> = map.pixels.iter().collect();
        assert!(distinct.len() > 4, "placeholder should not be flat");
        assert_eq!(map.to_rgba().len(), MAP_SIZE * MAP_SIZE * 4);
    }

    #[test]
    fn map_columns_share_the_blit_path() {
        let mut maps = Maps::new();
        let columns = play::MapColumns {
            map_id: 9,
            columns: 1,
            rows: 1,
            x: 0,
            z: 0,
            colors: vec![5],
        };
        assert_eq!(maps.apply_columns(&columns).pixels[0], map_color(5));
    }

    #[test]
    fn palette_zero_is_transparent_and_others_are_opaque() {
        assert_eq!(map_color(0)[3], 0);
        assert_eq!(map_color(1)[3], 255);
        assert_eq!(map_color(63)[3], 255);
    }
}

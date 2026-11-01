use crate::block::BlockState;

pub const SECTION_COUNT: usize = 16;
pub const SECTION_DIM: usize = 16;
pub const SECTION_VOLUME: usize = SECTION_DIM * SECTION_DIM * SECTION_DIM;
/// Bytes of nibble-packed light data for one section (4096 / 2).
pub const SECTION_LIGHT_BYTES: usize = SECTION_VOLUME / 2;
/// Bytes of block-state data for one section (4096 × 2).
pub const SECTION_BLOCK_BYTES: usize = SECTION_VOLUME * 2;

/// Local index of `(x, y, z)` inside a section — vanilla's `y << 8 | z << 4 | x`.
#[inline]
pub const fn section_index(x: usize, y: usize, z: usize) -> usize {
    (y << 8) | (z << 4) | x
}

#[derive(Debug, Clone)]
pub struct Section {
    blocks: Vec<u16>,
    block_light: Vec<u8>,
    sky_light: Option<Vec<u8>>,
    non_air: u16,
}

impl Section {
    pub fn filled(block: BlockState) -> Self {
        let raw = block.raw();
        let non_air = if raw == 0 { 0 } else { SECTION_VOLUME as u16 };
        Self {
            blocks: vec![raw; SECTION_VOLUME],
            block_light: vec![0; SECTION_LIGHT_BYTES],
            sky_light: None,
            non_air,
        }
    }

    pub fn from_blocks(blocks: Vec<u16>, non_air: u32) -> Self {
        debug_assert_eq!(blocks.len(), SECTION_VOLUME);
        Self {
            blocks,
            block_light: vec![0; SECTION_LIGHT_BYTES],
            sky_light: None,
            non_air: non_air.min(u16::MAX as u32) as u16,
        }
    }

    pub fn blocks(&self) -> &[u16] {
        &self.blocks
    }

    pub fn block_light(&self) -> &[u8] {
        &self.block_light
    }

    pub fn sky_light(&self) -> Option<&[u8]> {
        self.sky_light.as_deref()
    }

    pub fn non_air_count(&self) -> u16 {
        self.non_air
    }

    pub fn set_block_light(&mut self, data: &[u8]) {
        debug_assert_eq!(data.len(), SECTION_LIGHT_BYTES);
        self.block_light
            .copy_from_slice(&data[..SECTION_LIGHT_BYTES]);
    }

    pub fn set_sky_light(&mut self, data: &[u8]) {
        debug_assert_eq!(data.len(), SECTION_LIGHT_BYTES);
        match &mut self.sky_light {
            Some(buf) => buf.copy_from_slice(&data[..SECTION_LIGHT_BYTES]),
            None => self.sky_light = Some(data[..SECTION_LIGHT_BYTES].to_vec()),
        }
    }

    pub fn ensure_sky_light(&mut self) {
        if self.sky_light.is_none() {
            self.sky_light = Some(vec![0; SECTION_LIGHT_BYTES]);
        }
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize, z: usize) -> BlockState {
        BlockState::from_raw(self.blocks[section_index(x, y, z)])
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, state: BlockState) {
        let index = section_index(x, y, z);
        let old = self.blocks[index];
        let new = state.raw();
        if old == new {
            return;
        }
        if old == 0 && new != 0 {
            self.non_air += 1;
        } else if old != 0 && new == 0 {
            self.non_air = self.non_air.saturating_sub(1);
        }
        self.blocks[index] = new;
    }

    /// Nibble-packed light lookup (even xyz → low nibble, odd → high nibble).
    pub fn light_at(data: &[u8], x: usize, y: usize, z: usize) -> u8 {
        let index = section_index(x, y, z);
        let byte = data[index / 2];
        if index & 1 == 0 {
            byte & 0x0F
        } else {
            (byte >> 4) & 0x0F
        }
    }

    pub fn block_light_at(&self, x: usize, y: usize, z: usize) -> u8 {
        Self::light_at(&self.block_light, x, y, z)
    }

    pub fn sky_light_at(&self, x: usize, y: usize, z: usize) -> Option<u8> {
        self.sky_light
            .as_ref()
            .map(|data| Self::light_at(data, x, y, z))
    }
}

#[derive(Debug, Clone)]
pub struct ChunkColumn {
    pub x: i32,
    pub z: i32,
    sections: Vec<Option<Section>>,
    biomes: Option<Vec<u8>>,
    non_air: u32,
}

impl ChunkColumn {
    pub fn empty(x: i32, z: i32) -> Self {
        Self {
            x,
            z,
            sections: vec![None; SECTION_COUNT],
            biomes: None,
            non_air: 0,
        }
    }

    pub fn from_sections(x: i32, z: i32, sections: Vec<Option<Section>>) -> Self {
        debug_assert_eq!(sections.len(), SECTION_COUNT);
        let non_air = sections
            .iter()
            .flatten()
            .map(|s| s.non_air_count() as u32)
            .sum();
        Self {
            x,
            z,
            sections,
            biomes: None,
            non_air,
        }
    }

    pub fn section(&self, index: usize) -> Option<&Section> {
        self.sections.get(index).and_then(Option::as_ref)
    }

    pub fn section_mut(&mut self, index: usize) -> Option<&mut Section> {
        self.sections.get_mut(index).and_then(Option::as_mut)
    }

    pub fn sections(&self) -> &[Option<Section>] {
        &self.sections
    }

    pub fn iter_sections(&self) -> impl Iterator<Item = (usize, &Section)> {
        self.sections
            .iter()
            .enumerate()
            .filter_map(|(index, section)| section.as_ref().map(|s| (index, s)))
    }

    pub fn biomes(&self) -> Option<&[u8]> {
        self.biomes.as_deref()
    }

    pub fn set_biomes(&mut self, data: &[u8]) {
        debug_assert_eq!(data.len(), 256);
        self.biomes = Some(data[..256].to_vec());
    }

    /// Section bitmask in wire order (bit 0 = lowest section).
    pub fn mask(&self) -> u16 {
        let mut mask = 0u16;
        for (index, section) in self.sections.iter().enumerate() {
            if section.is_some() {
                mask |= 1 << index;
            }
        }
        mask
    }

    pub fn non_air_count(&self) -> u32 {
        self.non_air
    }

    /// `y` is the absolute world height (0..256); out-of-range is air.
    pub fn get_block(&self, x: usize, y: usize, z: usize) -> BlockState {
        if y >= SECTION_COUNT * SECTION_DIM {
            return BlockState::AIR;
        }
        match &self.sections[y >> 4] {
            Some(section) => section.get(x & 15, y & 15, z & 15),
            None => BlockState::AIR,
        }
    }

    pub fn set_block(&mut self, x: usize, y: usize, z: usize, state: BlockState) {
        if y >= SECTION_COUNT * SECTION_DIM {
            return;
        }
        let index = y >> 4;
        if self.sections[index].is_none() {
            if state.is_air() {
                return;
            }
            self.sections[index] = Some(Section::filled(BlockState::AIR));
        }
        let section = self.sections[index].as_mut().expect("just created");
        let before = section.non_air_count();
        section.set(x & 15, y & 15, z & 15, state);
        let after = section.non_air_count();
        self.non_air = self.non_air.saturating_sub(before as u32) + after as u32;
    }

    /// Highest non-air block in a column, if any.
    pub fn height_at(&self, x: usize, z: usize) -> Option<usize> {
        (0..SECTION_COUNT * SECTION_DIM)
            .rev()
            .find(|&y| !self.get_block(x, y, z).is_air())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_index_matches_vanilla() {
        assert_eq!(section_index(0, 0, 0), 0);
        assert_eq!(section_index(15, 0, 0), 15);
        assert_eq!(section_index(0, 0, 1), 16);
        assert_eq!(section_index(0, 1, 0), 256);
        assert_eq!(section_index(15, 15, 15), 4095);
    }

    #[test]
    fn section_tracks_non_air() {
        let mut section = Section::filled(BlockState::AIR);
        assert_eq!(section.non_air_count(), 0);
        section.set(1, 2, 3, BlockState::STONE);
        assert_eq!(section.non_air_count(), 1);
        assert_eq!(section.get(1, 2, 3), BlockState::STONE);
        section.set(1, 2, 3, BlockState::AIR);
        assert_eq!(section.non_air_count(), 0);
    }

    #[test]
    fn light_nibbles_are_packed() {
        let mut data = vec![0u8; SECTION_LIGHT_BYTES];
        data[0] = 0xAB;
        assert_eq!(Section::light_at(&data, 0, 0, 0), 0x0B);
        assert_eq!(Section::light_at(&data, 1, 0, 0), 0x0A);
    }

    #[test]
    fn column_creates_sections_on_demand() {
        let mut column = ChunkColumn::empty(4, -2);
        assert_eq!(column.mask(), 0);
        column.set_block(0, 70, 0, BlockState::GRASS);
        assert_eq!(column.mask(), 1 << 4);
        assert_eq!(column.non_air_count(), 1);
        assert_eq!(column.get_block(0, 70, 0), BlockState::GRASS);
        assert_eq!(column.height_at(0, 0), Some(70));
    }

    #[test]
    fn out_of_range_height_is_air() {
        let column = ChunkColumn::empty(0, 0);
        assert!(column.get_block(0, 500, 0).is_air());
        assert_eq!(column.height_at(0, 0), None);
    }
}

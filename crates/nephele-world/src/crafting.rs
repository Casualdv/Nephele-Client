//! Shaped crafting recipes.
//!
//! The table lives here rather than in the mock so singleplayer and the mock
//! agree by construction: the mock's copy was the only implementation, and it
//! meant crafting worked against the fake server and nowhere else.
//!
//! Matching follows vanilla's two useful rules and ignores the rest:
//!
//! - shapes are trimmed of empty rows and columns, so a recipe can sit anywhere
//!   in the grid;
//! - one log anywhere makes four planks, keeping the log's variant.
//!
//! Stack sizes are ignored. Mirrored shapes, tags and container-based recipes
//! (fireworks, banners) are not modelled.

/// What a grid produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipe {
    pub id: u16,
    /// Metadata/damage of the result.
    pub damage: u16,
    pub count: u8,
}

pub const LOG: u16 = 17;
pub const PLANKS: u16 = 5;
pub const STICK: u16 = 280;
pub const CRAFTING_TABLE: u16 = 58;

const COBBLESTONE: u16 = 4;
const IRON_INGOT: u16 = 265;
const COAL: u16 = 263;

type Shape = &'static [&'static [u16]];

/// Every recipe the client knows, as `(result, damage, count, shape)`.
///
/// A `0` in a shape is an empty cell.
const RECIPES: &[(u16, u16, u8, Shape)] = &[
    (STICK, 0, 4, &[&[PLANKS], &[PLANKS]]),
    (
        CRAFTING_TABLE,
        0,
        1,
        &[&[PLANKS, PLANKS], &[PLANKS, PLANKS]],
    ),
    (50, 0, 4, &[&[COAL], &[STICK]]),
    (
        61,
        0,
        1,
        &[
            &[COBBLESTONE, COBBLESTONE, COBBLESTONE],
            &[COBBLESTONE, 0, COBBLESTONE],
            &[COBBLESTONE, COBBLESTONE, COBBLESTONE],
        ],
    ),
    (
        54,
        0,
        1,
        &[
            &[PLANKS, PLANKS, PLANKS],
            &[PLANKS, 0, PLANKS],
            &[PLANKS, PLANKS, PLANKS],
        ],
    ),
    // Pickaxes.
    (
        270,
        0,
        1,
        &[&[PLANKS, PLANKS, PLANKS], &[0, STICK, 0], &[0, STICK, 0]],
    ),
    (
        274,
        0,
        1,
        &[
            &[COBBLESTONE, COBBLESTONE, COBBLESTONE],
            &[0, STICK, 0],
            &[0, STICK, 0],
        ],
    ),
    (
        257,
        0,
        1,
        &[
            &[IRON_INGOT, IRON_INGOT, IRON_INGOT],
            &[0, STICK, 0],
            &[0, STICK, 0],
        ],
    ),
    // Axes.
    (
        271,
        0,
        1,
        &[&[PLANKS, PLANKS], &[PLANKS, STICK], &[0, STICK]],
    ),
    (
        275,
        0,
        1,
        &[
            &[COBBLESTONE, COBBLESTONE],
            &[COBBLESTONE, STICK],
            &[0, STICK],
        ],
    ),
    (
        258,
        0,
        1,
        &[&[IRON_INGOT, IRON_INGOT], &[IRON_INGOT, STICK], &[0, STICK]],
    ),
    // Shovels.
    (269, 0, 1, &[&[PLANKS], &[STICK], &[STICK]]),
    (273, 0, 1, &[&[COBBLESTONE], &[STICK], &[STICK]]),
    (256, 0, 1, &[&[IRON_INGOT], &[STICK], &[STICK]]),
    // Swords.
    (268, 0, 1, &[&[PLANKS], &[PLANKS], &[STICK]]),
    (272, 0, 1, &[&[COBBLESTONE], &[COBBLESTONE], &[STICK]]),
    (267, 0, 1, &[&[IRON_INGOT], &[IRON_INGOT], &[STICK]]),
    // Iron armour.
    (
        306,
        0,
        1,
        &[
            &[IRON_INGOT, IRON_INGOT, IRON_INGOT],
            &[IRON_INGOT, 0, IRON_INGOT],
        ],
    ),
    (
        307,
        0,
        1,
        &[
            &[IRON_INGOT, 0, IRON_INGOT],
            &[IRON_INGOT, IRON_INGOT, IRON_INGOT],
            &[IRON_INGOT, IRON_INGOT, IRON_INGOT],
        ],
    ),
    (
        308,
        0,
        1,
        &[
            &[IRON_INGOT, IRON_INGOT, IRON_INGOT],
            &[IRON_INGOT, 0, IRON_INGOT],
            &[IRON_INGOT, 0, IRON_INGOT],
        ],
    ),
    (
        309,
        0,
        1,
        &[&[IRON_INGOT, 0, IRON_INGOT], &[IRON_INGOT, 0, IRON_INGOT]],
    ),
];

/// Every recipe result as `(id, damage)`, for icon-coverage checks.
pub fn recipe_outputs() -> impl Iterator<Item = (u16, u16)> {
    RECIPES.iter().map(|(id, damage, _, _)| (*id, *damage))
}

/// Match a 3x3 grid of `(id, damage)` cells, `None` being empty.
pub fn craft(grid: &[Option<(u16, u16)>; 9]) -> Option<Recipe> {
    let filled: Vec<(u16, u16)> = grid.iter().flatten().copied().collect();
    if filled.len() == 1 && filled[0].0 == LOG {
        return Some(Recipe {
            id: PLANKS,
            damage: filled[0].1.clamp(0, 3),
            count: 4,
        });
    }
    if filled.is_empty() {
        return None;
    }

    let rows: Vec<Vec<u16>> = (0..3)
        .map(|r| {
            (0..3)
                .map(|c| grid[r * 3 + c].map_or(0, |cell| cell.0))
                .collect()
        })
        .collect();
    let used_rows: Vec<usize> = (0..3)
        .filter(|&r| rows[r].iter().any(|&id| id != 0))
        .collect();
    let used_cols: Vec<usize> = (0..3)
        .filter(|&c| (0..3).any(|r| rows[r][c] != 0))
        .collect();
    let shape: Vec<Vec<u16>> = used_rows
        .iter()
        .map(|&r| used_cols.iter().map(|&c| rows[r][c]).collect())
        .collect();

    RECIPES.iter().find_map(|(id, damage, count, pattern)| {
        let matches = pattern.len() == shape.len()
            && pattern
                .iter()
                .zip(shape.iter())
                .all(|(a, b)| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y));
        matches.then_some(Recipe {
            id: *id,
            damage: *damage,
            count: *count,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cells: &[(usize, u16)]) -> [Option<(u16, u16)>; 9] {
        let mut out = [None; 9];
        for (slot, id) in cells {
            out[*slot] = Some((*id, 0));
        }
        out
    }

    #[test]
    fn one_log_becomes_four_planks_of_the_same_variant() {
        let mut cells = [None; 9];
        cells[4] = Some((LOG, 2));
        assert_eq!(
            craft(&cells),
            Some(Recipe {
                id: PLANKS,
                damage: 2,
                count: 4
            })
        );
    }

    #[test]
    fn shapes_match_anywhere_in_the_grid() {
        let top_left = grid(&[(0, PLANKS), (3, PLANKS)]);
        let offset = grid(&[(1, PLANKS), (4, PLANKS)]);
        assert_eq!(craft(&top_left), craft(&offset));
        assert_eq!(craft(&top_left).map(|r| r.id), Some(STICK));
    }

    #[test]
    fn the_survival_chain_reaches_a_pickaxe() {
        assert_eq!(craft(&grid(&[(0, LOG)])).map(|r| r.id), Some(PLANKS));

        let table = craft(&grid(&[(0, PLANKS), (1, PLANKS), (3, PLANKS), (4, PLANKS)]));
        assert_eq!(table.map(|r| r.id), Some(CRAFTING_TABLE));

        let pickaxe = craft(&grid(&[
            (0, PLANKS),
            (1, PLANKS),
            (2, PLANKS),
            (4, STICK),
            (7, STICK),
        ]));
        assert_eq!(pickaxe.map(|r| r.id), Some(270));
    }

    #[test]
    fn a_malformed_shape_does_not_craft() {
        assert_eq!(craft(&grid(&[(0, STICK), (1, STICK)])), None);
        assert_eq!(craft(&[None; 9]), None);
    }

    #[test]
    fn every_recipe_result_is_a_known_item() {
        // A guard against a typo'd id silently crafting nothing.
        for (id, _, count, _) in RECIPES {
            assert!(*id != 0, "recipe result has no id");
            assert!(*count > 0, "recipe {id} produces nothing");
            assert!(
                !crate::blocks::name(*id).is_empty(),
                "recipe {id} is not a known block"
            );
        }
    }
}

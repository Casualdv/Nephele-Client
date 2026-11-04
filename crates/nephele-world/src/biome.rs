//! The 1.8.9 biome table, as the layer stack sees it.
//!
//! Layers only need a few things from a biome: its Java class (that is how
//! `isEqualTo` decides two ids are "the same" biome), its temperature (for
//! `getTempCategory`), and whether it is snowy. Heights and surface blocks come
//! later, when terrain is generated.

/// Which Java class a biome id belongs to. `isEqualTo` compares these, not ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiomeClass {
    Ocean,
    Plains,
    Desert,
    Hills,
    Forest,
    Taiga,
    Swamp,
    River,
    Hell,
    End,
    Snow,
    MushroomIsland,
    Beach,
    Jungle,
    StoneBeach,
    Savanna,
    SavannaMutated,
    Mesa,
    /// The default mutation: `BiomeGenMutated`.
    Mutated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempCategory {
    Cold,
    Medium,
    Warm,
}

#[derive(Debug, Clone, Copy)]
pub struct Biome {
    pub id: i32,
    pub name: &'static str,
    pub class: BiomeClass,
    pub temperature: f32,
    pub rainfall: f32,
    pub snowy: bool,
}

const fn b(
    id: i32,
    name: &'static str,
    class: BiomeClass,
    temperature: f32,
    rainfall: f32,
    snowy: bool,
) -> Biome {
    Biome {
        id,
        name,
        class,
        temperature,
        rainfall,
        snowy,
    }
}

/// The registered biomes, indexed by id.
pub static BIOMES: [Biome; 40] = [
    b(0, "Ocean", BiomeClass::Ocean, 0.5, 0.5, false),
    b(1, "Plains", BiomeClass::Plains, 0.8, 0.4, false),
    b(2, "Desert", BiomeClass::Desert, 2.0, 0.0, false),
    b(3, "Extreme Hills", BiomeClass::Hills, 0.2, 0.3, false),
    b(4, "Forest", BiomeClass::Forest, 0.7, 0.8, false),
    b(5, "Taiga", BiomeClass::Taiga, 0.25, 0.8, false),
    b(6, "Swampland", BiomeClass::Swamp, 0.8, 0.9, false),
    b(7, "River", BiomeClass::River, 0.5, 0.5, false),
    b(8, "Hell", BiomeClass::Hell, 2.0, 0.0, false),
    b(9, "The End", BiomeClass::End, 0.5, 0.5, false),
    b(10, "FrozenOcean", BiomeClass::Ocean, 0.0, 0.5, true),
    b(11, "FrozenRiver", BiomeClass::River, 0.0, 0.5, true),
    b(12, "Ice Plains", BiomeClass::Snow, 0.0, 0.5, true),
    b(13, "Ice Mountains", BiomeClass::Snow, 0.0, 0.5, true),
    b(
        14,
        "MushroomIsland",
        BiomeClass::MushroomIsland,
        0.9,
        1.0,
        false,
    ),
    b(
        15,
        "MushroomIslandShore",
        BiomeClass::MushroomIsland,
        0.9,
        1.0,
        false,
    ),
    b(16, "Beach", BiomeClass::Beach, 0.8, 0.4, false),
    b(17, "DesertHills", BiomeClass::Desert, 2.0, 0.0, false),
    b(18, "ForestHills", BiomeClass::Forest, 0.7, 0.8, false),
    b(19, "TaigaHills", BiomeClass::Taiga, 0.25, 0.8, false),
    b(20, "Extreme Hills Edge", BiomeClass::Hills, 0.2, 0.3, false),
    b(21, "Jungle", BiomeClass::Jungle, 0.95, 0.9, false),
    b(22, "JungleHills", BiomeClass::Jungle, 0.95, 0.9, false),
    b(23, "JungleEdge", BiomeClass::Jungle, 0.95, 0.8, false),
    b(24, "Deep Ocean", BiomeClass::Ocean, 0.5, 0.5, false),
    b(25, "Stone Beach", BiomeClass::StoneBeach, 0.2, 0.3, false),
    b(26, "Cold Beach", BiomeClass::Beach, 0.05, 0.3, true),
    b(27, "Birch Forest", BiomeClass::Forest, 0.6, 0.6, false),
    b(
        28,
        "Birch Forest Hills",
        BiomeClass::Forest,
        0.6,
        0.6,
        false,
    ),
    b(29, "Roofed Forest", BiomeClass::Forest, 0.7, 0.8, false),
    b(30, "Cold Taiga", BiomeClass::Taiga, -0.5, 0.4, true),
    b(31, "Cold Taiga Hills", BiomeClass::Taiga, -0.5, 0.4, true),
    b(32, "Mega Taiga", BiomeClass::Taiga, 0.3, 0.8, false),
    b(33, "Mega Taiga Hills", BiomeClass::Taiga, 0.3, 0.8, false),
    b(34, "Extreme Hills+", BiomeClass::Hills, 0.2, 0.3, false),
    b(35, "Savanna", BiomeClass::Savanna, 1.2, 0.0, false),
    b(36, "Savanna Plateau", BiomeClass::Savanna, 1.0, 0.0, false),
    b(37, "Mesa", BiomeClass::Mesa, 2.0, 0.0, false),
    b(38, "Mesa Plateau F", BiomeClass::Mesa, 2.0, 0.0, false),
    b(39, "Mesa Plateau", BiomeClass::Mesa, 2.0, 0.0, false),
];

pub const MUTATED: i32 = 128;

/// The base ids for which `BiomeGenBase`'s static block registers a mutated
/// biome. Every other `base + 128` id is null in vanilla's `biomeList`.
pub const MUTATION_BASES: [i32; 21] = [
    1, 2, 3, 4, 5, 6, 12, 21, 23, 27, 28, 29, 30, 32, 33, 34, 35, 36, 37, 38, 39,
];

pub const fn has_mutation(base_id: i32) -> bool {
    matches!(
        base_id,
        1 | 2
            | 3
            | 4
            | 5
            | 6
            | 12
            | 21
            | 23
            | 27
            | 28
            | 29
            | 30
            | 32
            | 33
            | 34
            | 35
            | 36
            | 37
            | 38
            | 39
    )
}

pub fn base(id: i32) -> Option<&'static Biome> {
    if (0..BIOMES.len() as i32).contains(&id) {
        Some(&BIOMES[id as usize])
    } else {
        None
    }
}

/// A biome, including mutations, for the layer stack's purposes.
#[derive(Debug, Clone, Copy)]
pub struct AnyBiome {
    pub id: i32,
    pub base: Biome,
    pub class: BiomeClass,
    pub mutated: bool,
}

pub fn any(id: i32) -> Option<AnyBiome> {
    let (base_id, mutated) = if id >= MUTATED {
        (id - MUTATED, true)
    } else {
        (id, false)
    };
    if mutated && !has_mutation(base_id) {
        return None;
    }
    let base = *base(base_id)?;
    let class = if mutated {
        mutated_class(base_id)
    } else {
        base.class
    };
    Some(AnyBiome {
        id,
        base,
        class,
        mutated,
    })
}

/// Which class the mutated variant of `base_id` is built as.
///
/// `BiomeGenMutated.getBiomeClass` returns its base biome's class, so a
/// mutation matches its base in `isEqualTo`. The few biomes that build a
/// concrete subclass for their mutation keep that subclass instead.
fn mutated_class(base_id: i32) -> BiomeClass {
    match base_id {
        1 => BiomeClass::Plains,
        2 => BiomeClass::Desert,
        4 | 27 | 28 | 29 => BiomeClass::Forest,
        5 | 30 | 32 | 33 => BiomeClass::Taiga,
        6 => BiomeClass::Swamp,
        12 | 13 => BiomeClass::Snow,
        21 | 23 => BiomeClass::Jungle,
        3 | 20 | 34 => BiomeClass::Hills,
        35 | 36 => BiomeClass::SavannaMutated,
        37..=39 => BiomeClass::Mesa,
        _ => BiomeClass::Mutated,
    }
}

/// `GenLayer.isBiomeOceanic`.
pub fn is_oceanic(id: i32) -> bool {
    matches!(id, 0 | 10 | 24)
}

/// `BiomeGenBase.isEqualTo`: the same class, not the same id.
pub fn is_equal_to(a: i32, b: i32) -> bool {
    match (any(a), any(b)) {
        (Some(a), Some(b)) => a.class == b.class,
        _ => false,
    }
}

/// `GenLayer.biomesEqualOrMesaPlateau`.
pub fn equal_or_mesa_plateau(a: i32, b: i32) -> bool {
    if a == b {
        return true;
    }
    if a != 38 && a != 39 {
        is_equal_to(a, b)
    } else {
        b == 38 || b == 39
    }
}

pub fn temp_category(id: i32) -> Option<TempCategory> {
    let biome = any(id)?;
    Some(if biome.base.temperature < 0.2 {
        TempCategory::Cold
    } else if biome.base.temperature < 1.0 {
        TempCategory::Medium
    } else {
        TempCategory::Warm
    })
}

pub fn is_snowy(id: i32) -> bool {
    any(id).map(|biome| biome.base.snowy).unwrap_or(false)
}

pub fn is_jungle_class(id: i32) -> bool {
    any(id)
        .map(|biome| biome.class == BiomeClass::Jungle)
        .unwrap_or(false)
}

/// `BiomeGenSwamp`, including its mutation.
pub fn is_swamp_class(id: i32) -> bool {
    any(id)
        .map(|biome| biome.class == BiomeClass::Swamp)
        .unwrap_or(false)
}

pub fn is_mesa(id: i32) -> bool {
    any(id)
        .map(|biome| biome.class == BiomeClass::Mesa)
        .unwrap_or(false)
}

/// `GenLayerBiomeEdge.canBiomesBeNeighbors`.
pub fn can_be_neighbors(a: i32, b: i32) -> bool {
    if equal_or_mesa_plateau(a, b) {
        return true;
    }
    match (temp_category(a), temp_category(b)) {
        (Some(a), Some(b)) => a == b || a == TempCategory::Medium || b == TempCategory::Medium,
        _ => false,
    }
}

/// `GenLayerShore.func_151631_c`.
pub fn shore_accepts(id: i32) -> bool {
    is_jungle_class(id) || matches!(id, 21 | 22 | 23 | 4 | 5) || is_oceanic(id)
}

/// `BiomeGenBase.Height` presets, as `(rootHeight, variation)`.
const HEIGHT_DEFAULT: (f32, f32) = (0.1, 0.2);
const HEIGHT_SHALLOW_WATERS: (f32, f32) = (-0.5, 0.0);
const HEIGHT_OCEANS: (f32, f32) = (-1.0, 0.1);
const HEIGHT_DEEP_OCEANS: (f32, f32) = (-1.8, 0.1);
const HEIGHT_LOW_PLAINS: (f32, f32) = (0.125, 0.05);
const HEIGHT_MID_PLAINS: (f32, f32) = (0.2, 0.2);
const HEIGHT_LOW_HILLS: (f32, f32) = (0.45, 0.3);
const HEIGHT_HIGH_PLATEAUS: (f32, f32) = (1.5, 0.025);
const HEIGHT_MID_HILLS: (f32, f32) = (1.0, 0.5);
const HEIGHT_SHORES: (f32, f32) = (0.0, 0.025);
const HEIGHT_ROCKY_WATERS: (f32, f32) = (0.1, 0.8);
const HEIGHT_LOW_ISLANDS: (f32, f32) = (0.2, 0.3);
const HEIGHT_PARTIALLY_SUBMERGED: (f32, f32) = (-0.2, 0.1);

/// The `(minHeight, maxHeight)` a biome contributes to the terrain height
/// field. Mutated biomes are their base biome plus `(0.1, 0.2)`, exactly as
/// `BiomeGenMutated` builds them.
pub fn height_params(id: i32) -> (f32, f32) {
    let (root, variation) = match if id >= 128 { id - 128 } else { id } {
        0 | 10 => HEIGHT_OCEANS,
        1 | 4 | 8 | 9 | 20 | 21 | 23 | 27 | 29 | 37 => HEIGHT_DEFAULT,
        2 | 12 | 35 => HEIGHT_LOW_PLAINS,
        3 | 34 => HEIGHT_MID_HILLS,
        5 | 30 | 32 => HEIGHT_MID_PLAINS,
        6 => HEIGHT_PARTIALLY_SUBMERGED,
        7 | 11 => HEIGHT_SHALLOW_WATERS,
        13 | 17 | 18 | 19 | 22 | 28 | 31 | 33 => HEIGHT_LOW_HILLS,
        14 => HEIGHT_LOW_ISLANDS,
        15 | 16 | 26 => HEIGHT_SHORES,
        24 => HEIGHT_DEEP_OCEANS,
        25 => HEIGHT_ROCKY_WATERS,
        36 | 38 | 39 => HEIGHT_HIGH_PLATEAUS,
        _ => HEIGHT_DEFAULT,
    };

    if id >= 128 {
        (root + 0.1, variation + 0.2)
    } else {
        (root, variation)
    }
}

pub fn name_of(id: i32) -> &'static str {
    any(id).map(|biome| biome.base.name).unwrap_or("?")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_indexed_by_id() {
        for biome in BIOMES {
            assert_eq!(base(biome.id).unwrap().id, biome.id);
        }
        assert!(base(-1).is_none());
        assert!(base(40).is_none());
    }

    #[test]
    fn oceanics_are_the_three_oceans() {
        assert!(is_oceanic(0));
        assert!(is_oceanic(10));
        assert!(is_oceanic(24));
        assert!(!is_oceanic(7));
        assert!(!is_oceanic(1));
    }

    #[test]
    fn equality_is_by_class_not_id() {
        // Forest, ForestHills, Birch Forest and Roofed Forest are all
        // BiomeGenForest in 1.8, so the layers treat them as equal.
        assert!(is_equal_to(4, 18));
        assert!(is_equal_to(4, 27));
        assert!(!is_equal_to(4, 1));
        // `BiomeGenMutated` delegates its class to the base biome, so every
        // mutation matches its base; the concrete-subclass mutations match too.
        assert!(is_equal_to(4, 4 + MUTATED));
        assert!(is_equal_to(3, 3 + MUTATED));
        assert!(is_equal_to(5, 5 + MUTATED));
        assert!(is_equal_to(2, 2 + MUTATED));
        assert!(!is_equal_to(5, 4));
    }

    #[test]
    fn only_registered_mutations_resolve() {
        for base_id in MUTATION_BASES {
            assert!(any(base_id + MUTATED).is_some(), "base {base_id}");
        }
        assert!(
            any(141).is_none(),
            "Ice Mountains has no registered mutation"
        );
        assert!(
            any(148).is_none(),
            "Extreme Hills Edge has no registered mutation"
        );
    }

    #[test]
    fn mutated_classes_follow_the_base_biome() {
        assert_eq!(any(129).unwrap().class, BiomeClass::Plains);
        assert_eq!(any(130).unwrap().class, BiomeClass::Desert);
        assert_eq!(any(133).unwrap().class, BiomeClass::Taiga);
        assert_eq!(any(149).unwrap().class, BiomeClass::Jungle);
        assert_eq!(any(158).unwrap().class, BiomeClass::Taiga);
        assert_eq!(any(161).unwrap().class, BiomeClass::Taiga);
    }

    #[test]
    fn mesa_plateaus_compare_against_each_other_only() {
        assert!(equal_or_mesa_plateau(38, 39));
        assert!(equal_or_mesa_plateau(39, 38));
        // All three mesas share a class, so 37 matches 38 too.
        assert!(equal_or_mesa_plateau(37, 38));
        assert!(equal_or_mesa_plateau(3, 20));
        assert!(!equal_or_mesa_plateau(3, 5));
    }

    #[test]
    fn temperature_categories_follow_the_thresholds() {
        assert_eq!(temp_category(12), Some(TempCategory::Cold)); // 0.0
        assert_eq!(temp_category(3), Some(TempCategory::Medium)); // 0.2
        assert_eq!(temp_category(5), Some(TempCategory::Medium)); // 0.25
        assert_eq!(temp_category(2), Some(TempCategory::Warm)); // 2.0
        assert_eq!(temp_category(35), Some(TempCategory::Warm)); // 1.2
    }

    #[test]
    fn neighbours_need_a_shared_or_medium_temperature() {
        assert!(can_be_neighbors(12, 30)); // both cold
        assert!(can_be_neighbors(12, 4)); // cold vs medium
        assert!(!can_be_neighbors(12, 2)); // cold vs warm
    }

    #[test]
    fn snowy_biomes_are_the_ones_vanilla_flags() {
        for id in [10, 11, 12, 13, 26, 30, 31] {
            assert!(is_snowy(id), "{id} should be snowy");
        }
        for id in [0, 1, 2, 4, 5, 16, 25, 35] {
            assert!(!is_snowy(id), "{id} should not be snowy");
        }
    }
}

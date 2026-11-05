//! Block id -> vanilla `SoundType`, and the volume/pitch rules that turn a
//! sound type into a `playSound` call.
//!
//! Reimplemented from the 1.8.9 reference source, not copied:
//!
//! - `Block.SoundType` (`net/minecraft/block/Block$SoundType.java`) holds
//!   `(name, volume, frequency)` and derives `break`/`step` by prefixing
//!   `"dig."` / `"step."`; `getPlaceSound` is `getBreakSound`, except where a
//!   type overrides it (glass, anvil, slime).
//! - `ItemBlock.onItemUse` plays the place sound at
//!   `(volume + 1) / 2`, `frequency * 0.8`.
//! - `Block.playStepSound` plays the step sound at `volume * 0.15`, `frequency`.
//! - `World.playAuxSFX(2001)` uses the same `(volume + 1) / 2`,
//!   `frequency * 0.8` pair for a broken block.
//!
//! Only the *event names and numbers* are reimplemented here; no Mojang asset
//! or data file is reproduced. The id lists below come from the
//! `registerBlock(...)` calls in `Block.java` plus each block class's own
//! `setStepSound` in its constructor.

/// Vanilla's `Block.SoundType`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundType {
    pub name: &'static str,
    pub volume: f32,
    pub frequency: f32,
    pub break_sound: &'static str,
    pub place_sound: &'static str,
    pub step_sound: &'static str,
}

const STONE: SoundType = SoundType {
    name: "stone",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "dig.stone",
    place_sound: "dig.stone",
    step_sound: "step.stone",
};

const WOOD: SoundType = SoundType {
    name: "wood",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "dig.wood",
    place_sound: "dig.wood",
    step_sound: "step.wood",
};

const GRAVEL: SoundType = SoundType {
    name: "gravel",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "dig.gravel",
    place_sound: "dig.gravel",
    step_sound: "step.gravel",
};

const GRASS: SoundType = SoundType {
    name: "grass",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "dig.grass",
    place_sound: "dig.grass",
    step_sound: "step.grass",
};

const SAND: SoundType = SoundType {
    name: "sand",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "dig.sand",
    place_sound: "dig.sand",
    step_sound: "step.sand",
};

const SNOW: SoundType = SoundType {
    name: "snow",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "dig.snow",
    place_sound: "dig.snow",
    step_sound: "step.snow",
};

const CLOTH: SoundType = SoundType {
    name: "cloth",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "dig.cloth",
    place_sound: "dig.cloth",
    step_sound: "step.cloth",
};

/// `soundTypeGlass`: named `stone`, but breaks and places with `dig.glass` and
/// places with `step.stone`.
const GLASS: SoundType = SoundType {
    name: "stone",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "dig.glass",
    place_sound: "step.stone",
    step_sound: "step.stone",
};

/// `soundTypeMetal`: stone sounds, a fifth higher.
const METAL: SoundType = SoundType {
    name: "stone",
    volume: 1.0,
    frequency: 1.5,
    break_sound: "dig.stone",
    place_sound: "dig.stone",
    step_sound: "step.stone",
};

/// `soundTypeLadder`: breaks with wood, steps with `step.ladder`.
const LADDER: SoundType = SoundType {
    name: "ladder",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "dig.wood",
    place_sound: "dig.wood",
    step_sound: "step.ladder",
};

/// `soundTypeAnvil`: quiet, breaks like stone, lands with its own sound.
const ANVIL: SoundType = SoundType {
    name: "anvil",
    volume: 0.3,
    frequency: 1.0,
    break_sound: "dig.stone",
    place_sound: "random.anvil_land",
    step_sound: "step.anvil",
};

/// `SLIME_SOUND`.
const SLIME: SoundType = SoundType {
    name: "slime",
    volume: 1.0,
    frequency: 1.0,
    break_sound: "mob.slime.big",
    place_sound: "mob.slime.big",
    step_sound: "mob.slime.small",
};

/// The sound type of block `id`, or `None` where vanilla plays nothing.
pub fn sound_type(id: u16) -> Option<SoundType> {
    let kind = match id {
        // Air, the four liquids, the end portal, and the technical barrier.
        0 | 8..=11 | 119 | 166 => return None,

        // Stone, ores, bricks, slabs, stairs, and every other rock-like block.
        1
        | 4
        | 7
        | 14..=16
        | 21
        | 23
        | 24
        | 29..=30
        | 34
        | 36
        | 43..=45
        | 48..=49
        | 55
        | 61..=62
        | 70
        | 87
        | 90
        | 97..=98
        | 108..=109
        | 112..=113
        | 116..=117
        | 121..=122
        | 130
        | 137
        | 139..=140
        | 144
        | 153
        | 155..=156
        | 159
        | 168
        | 172..=173
        | 179..=182 => STONE,

        // Wood: planks, logs, doors, fences, signs, chests, workstations.
        5
        | 17
        | 25..=26
        | 47
        | 50
        | 53..=54
        | 58
        | 63..=64
        | 68..=69
        | 72
        | 75..=76
        | 84..=85
        | 91
        | 96
        | 99..=100
        | 103..=105
        | 107
        | 125..=127
        | 134..=136
        | 143
        | 146..=150
        | 162..=164
        | 176..=177
        | 183..=197 => WOOD,

        // Grass and plants: violet-green, leaves, crops, flowers, hay.
        2
        | 6
        | 18..=19
        | 31..=32
        | 37..=40
        | 46
        | 59
        | 83
        | 106
        | 110..=111
        | 115
        | 141..=142
        | 161
        | 170
        | 175 => GRASS,

        // Dirt is *gravel* in vanilla, not grass - the most surprising entry here.
        3 | 13 | 60 | 82 => GRAVEL,

        12 | 88 => SAND,
        78 | 80 => SNOW,
        35 | 51 | 81 | 171 => CLOTH,

        20 | 79 | 89 | 95 | 102 | 120 | 123..=124 | 138 | 160 | 169 | 174 => GLASS,

        // Metal blocks, bars, rails, hoppers and the cauldron.
        27..=28 | 41..=42 | 52 | 57 | 66 | 71 | 101 | 118 | 133 | 152 | 154 | 157 | 167 => METAL,

        65 => LADDER,
        145 => ANVIL,
        165 => SLIME,

        // Unknown ids behave like the base `Block` constructor: stone.
        _ => STONE,
    };
    Some(kind)
}

/// Volume vanilla uses when a block breaks (`playAuxSFX(2001)`).
pub fn break_volume(type_: SoundType) -> f32 {
    (type_.volume + 1.0) / 2.0
}

/// Pitch vanilla uses when a block breaks.
pub fn break_pitch(type_: SoundType) -> f32 {
    type_.frequency * 0.8
}

/// Volume vanilla uses when a block is placed (`ItemBlock.onItemUse`).
pub fn place_volume(type_: SoundType) -> f32 {
    (type_.volume + 1.0) / 2.0
}

/// Pitch vanilla uses when a block is placed.
pub fn place_pitch(type_: SoundType) -> f32 {
    type_.frequency * 0.8
}

/// Step sounds are 15% volume, which is why footsteps never drown out blocks.
pub fn step_volume(type_: SoundType) -> f32 {
    type_.volume * 0.15
}

pub fn step_pitch(type_: SoundType) -> f32 {
    type_.frequency
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_obvious_blocks_map_to_their_vanilla_types() {
        assert_eq!(sound_type(1), Some(STONE)); // stone
        assert_eq!(sound_type(2), Some(GRASS)); // grass block
        assert_eq!(sound_type(12), Some(SAND)); // sand
        assert_eq!(sound_type(20), Some(GLASS)); // glass
        assert_eq!(sound_type(35), Some(CLOTH)); // wool
        assert_eq!(sound_type(78), Some(SNOW)); // snow layer
        assert_eq!(sound_type(165), Some(SLIME)); // slime block
        assert_eq!(sound_type(145), Some(ANVIL)); // anvil
        assert_eq!(sound_type(65), Some(LADDER)); // ladder
    }

    #[test]
    fn dirt_is_gravel_and_logs_are_wood() {
        // Both are easy to get wrong from the texture names.
        assert_eq!(sound_type(3), Some(GRAVEL));
        assert_eq!(sound_type(13), Some(GRAVEL));
        assert_eq!(sound_type(17), Some(WOOD));
        assert_eq!(sound_type(5), Some(WOOD));
    }

    #[test]
    fn air_and_liquids_are_silent() {
        for id in [0, 8, 9, 10, 11, 119, 166] {
            assert_eq!(sound_type(id), None, "id {id} should be silent");
        }
    }

    #[test]
    fn glass_breaks_and_places_differently() {
        assert_eq!(GLASS.break_sound, "dig.glass");
        assert_eq!(GLASS.place_sound, "step.stone");
        assert_eq!(GLASS.step_sound, "step.stone");
    }

    #[test]
    fn metal_is_stone_a_fifth_higher() {
        assert_eq!(METAL.break_sound, "dig.stone");
        assert!((place_pitch(METAL) - 1.2).abs() < 1e-6);
        assert!((break_pitch(STONE) - 0.8).abs() < 1e-6);
    }

    #[test]
    fn volume_and_pitch_match_the_vanilla_formulas() {
        assert!((break_volume(STONE) - 1.0).abs() < 1e-6);
        assert!((place_volume(ANVIL) - 0.65).abs() < 1e-6);
        assert!((step_volume(STONE) - 0.15).abs() < 1e-6);
        assert_eq!(step_pitch(METAL), 1.5);
    }
}

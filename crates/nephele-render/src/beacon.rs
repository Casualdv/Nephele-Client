//! Beacon beams, computed the way vanilla's `TileEntityBeacon` does.
//!
//! The client has no beacon tile-entity NBT (the block-entity packet's NBT is
//! skipped), so the beam is derived from the world instead: scan up from the
//! beacon, tint through stained glass and panes, and stop at anything opaque.
//! The result feeds the entity vertex buffer and the additive beam pass.

use nephele_assets::is_transparent;
use nephele_world::block::blocks_movement;
use nephele_world::{ChunkColumn, World};

use crate::entity::{EntityMesh, EntityVertex};

/// The beacon block itself.
pub const BEACON: u16 = 138;
/// Stained glass and stained glass panes colour the beam.
pub const STAINED_GLASS: u16 = 95;
pub const STAINED_GLASS_PANE: u16 = 160;
/// World height; a beam runs to the top of the build limit.
const WORLD_TOP: i32 = 256;
/// Half-width of the rotating core, in blocks (vanilla's 0.2 radius).
const CORE_RADIUS: f32 = 0.2;
/// Vanilla draws the core with a semi-transparent beam texture; without that
/// texture, overlapping additive quads saturate to white unless the per-vertex
/// alpha stays low.
const CORE_ALPHA: f32 = 0.35;
/// Vanilla draws a faint 0.2..0.8 halo around the core at alpha 0.125.
const GLOW_ALPHA: f32 = 0.08;
/// Blocks over which the beam fades out at the top instead of stopping flat.
const TOP_FADE: f32 = 24.0;

/// `EntitySheep.getDyeRgb`, in `EnumDyeColor` metadata order (0 = white).
pub const DYE_COLORS: [[f32; 3]; 16] = [
    [1.0, 1.0, 1.0],
    [0.85, 0.5, 0.2],
    [0.7, 0.3, 0.85],
    [0.4, 0.6, 0.85],
    [0.9, 0.9, 0.2],
    [0.5, 0.8, 0.1],
    [0.95, 0.5, 0.65],
    [0.3, 0.3, 0.3],
    [0.6, 0.6, 0.6],
    [0.3, 0.5, 0.6],
    [0.5, 0.25, 0.7],
    [0.2, 0.3, 0.7],
    [0.4, 0.3, 0.2],
    [0.4, 0.5, 0.2],
    [0.6, 0.2, 0.2],
    [0.1, 0.1, 0.1],
];

/// One coloured run of the beam.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BeaconSegment {
    pub color: [f32; 3],
    /// Height in blocks.
    pub height: i32,
}

/// A beacon and the coloured segments rising from it.
#[derive(Debug, Clone, PartialEq)]
pub struct BeaconBeam {
    pub position: [i32; 3],
    pub segments: Vec<BeaconSegment>,
}

impl BeaconBeam {
    /// Total height in blocks.
    pub fn height(&self) -> i32 {
        self.segments.iter().map(|segment| segment.height).sum()
    }
}

/// Stained-glass metadata is the dye metadata, so `meta & 15` indexes directly.
pub fn dye_color(meta: u8) -> [f32; 3] {
    DYE_COLORS[(meta & 0x0F) as usize]
}

/// The beam a beacon at `(x, y, z)` would draw, or `None` if it is obstructed.
pub fn beam_at(world: &World, x: i32, y: i32, z: i32) -> Option<BeaconBeam> {
    if world.get_block(x, y, z).id() != BEACON {
        return None;
    }

    let mut segments = vec![BeaconSegment {
        color: DYE_COLORS[0],
        height: 0,
    }];
    let mut tinted = false;
    for height_y in (y + 1)..WORLD_TOP {
        let state = world.get_block(x, height_y, z);
        let id = state.id();
        let color = if id == STAINED_GLASS || id == STAINED_GLASS_PANE {
            dye_color(state.meta())
        } else {
            if blocks_movement(state) && !is_transparent(id) {
                // An opaque block above the beacon kills the beam entirely.
                return None;
            }
            segments
                .last_mut()
                .expect("a segment is seeded above")
                .height += 1;
            continue;
        };
        let mut color = color;
        if tinted {
            let previous = segments.last().expect("a segment is seeded above").color;
            color = [
                (previous[0] + color[0]) / 2.0,
                (previous[1] + color[1]) / 2.0,
                (previous[2] + color[2]) / 2.0,
            ];
        }
        let last = segments.last_mut().expect("a segment is seeded above");
        if last.color == color {
            last.height += 1;
        } else {
            segments.push(BeaconSegment { color, height: 0 });
        }
        tinted = true;
    }

    Some(BeaconBeam {
        position: [x, y, z],
        segments,
    })
}

/// Every beacon in one loaded column, for the incremental chunk path.
pub fn beams_in_column(world: &World, column: &ChunkColumn) -> Vec<BeaconBeam> {
    let mut beams = Vec::new();
    for (section_index, section) in column.iter_sections() {
        for (index, raw) in section.blocks().iter().enumerate() {
            if raw >> 4 != BEACON {
                continue;
            }
            let x = column.x * 16 + (index & 15) as i32;
            let y = (section_index * 16 + ((index >> 8) & 15)) as i32;
            let z = column.z * 16 + ((index >> 4) & 15) as i32;
            if let Some(beam) = beam_at(world, x, y, z) {
                beams.push(beam);
            }
        }
    }
    beams
}

/// Scan the whole loaded world. Only for the offline viewer, where the world is
/// built once; the live client updates per loaded column instead.
pub fn find_beams(world: &World) -> Vec<BeaconBeam> {
    let mut beams = Vec::new();
    for (_, column) in world.iter() {
        beams.extend(beams_in_column(world, column));
    }
    beams
}

/// Build the beam geometry for this frame.
///
/// `time` is seconds; vanilla rotates the core by -0.75 radians per second.
pub fn build_beams(beams: &[BeaconBeam], time: f32) -> EntityMesh {
    let mut mesh = EntityMesh::default();
    for beam in beams {
        let base_y = beam.position[1] as f32;
        let total = beam.height() as f32;
        let top_y = base_y + total;
        let angle = time * -0.75;
        let mut y = base_y;
        for segment in &beam.segments {
            if segment.height <= 0 {
                continue;
            }
            let next_y = y + segment.height as f32;
            append_segment(
                &mut mesh,
                beam.position[0],
                beam.position[2],
                y,
                next_y,
                top_y,
                angle,
                segment.color,
            );
            y = next_y;
        }
    }
    mesh
}

#[allow(clippy::too_many_arguments)]
fn append_segment(
    mesh: &mut EntityMesh,
    x: i32,
    z: i32,
    bottom: f32,
    top: f32,
    beam_top: f32,
    angle: f32,
    color: [f32; 3],
) {
    let base_x = x as f32;
    let base_z = z as f32;

    // The core is a rotating square of four quads, as vanilla draws it: the
    // corners sit at 45, 135, 225 and 315 degrees around the beacon axis.
    use std::f32::consts::PI;
    let corner = |offset: f32| {
        [
            base_x + 0.5 + (angle + offset).cos() * CORE_RADIUS,
            base_z + 0.5 + (angle + offset).sin() * CORE_RADIUS,
        ]
    };
    let corners = [
        corner(PI * 0.75),
        corner(PI * 0.25),
        corner(PI * 1.75),
        corner(PI * 1.25),
    ];
    for index in 0..4 {
        let a = corners[index];
        let b = corners[(index + 1) % 4];
        append_vertical_quad(mesh, a, b, bottom, top, beam_top, color, CORE_ALPHA);
    }

    // The faint halo, axis-aligned in vanilla's 0.2..0.8 square.
    let halo = [
        [base_x + 0.2, base_z + 0.2],
        [base_x + 0.8, base_z + 0.2],
        [base_x + 0.8, base_z + 0.8],
        [base_x + 0.2, base_z + 0.8],
    ];
    for index in 0..4 {
        let a = halo[index];
        let b = halo[(index + 1) % 4];
        append_vertical_quad(mesh, a, b, bottom, top, beam_top, color, GLOW_ALPHA);
    }
}

#[allow(clippy::too_many_arguments)]
fn append_vertical_quad(
    mesh: &mut EntityMesh,
    a: [f32; 2],
    b: [f32; 2],
    bottom: f32,
    top: f32,
    beam_top: f32,
    color: [f32; 3],
    alpha: f32,
) {
    let base = mesh.vertices.len() as u32;
    let vertices = [
        [a[0], bottom, a[1]],
        [a[0], top, a[1]],
        [b[0], top, b[1]],
        [b[0], bottom, b[1]],
    ];
    for position in vertices {
        mesh.vertices.push(EntityVertex {
            position,
            // Negative U means "untextured, unfogged": beams carry their whole
            // colour per vertex and must stay visible at distance.
            uv: [-1.0, -1.0],
            shade: 1.0,
            color: [
                color[0],
                color[1],
                color[2],
                alpha * top_fade(position[1], beam_top),
            ],
        });
    }
    mesh.indices
        .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// Fade the last few blocks so the beam does not end in a hard line against the
/// sky.
fn top_fade(y: f32, beam_top: f32) -> f32 {
    let distance = (beam_top - y).max(0.0);
    0.15 + (distance / TOP_FADE).min(1.0) * 0.85
}

#[cfg(test)]
mod tests {
    use super::*;
    use nephele_world::{BlockState, ChunkColumn};

    fn world_with_blocks(blocks: &[((i32, i32, i32), BlockState)]) -> World {
        let mut world = World::new(0);
        world.insert(ChunkColumn::empty(0, 0));
        for &((x, y, z), state) in blocks {
            world.set_block(x, y, z, state);
        }
        world
    }

    #[test]
    fn a_coloured_pane_tints_the_beam() {
        let world = world_with_blocks(&[
            ((4, 64, 4), BlockState::new(BEACON, 0)),
            ((4, 65, 4), BlockState::new(STAINED_GLASS_PANE, 14)), // red
        ]);
        let beam = beam_at(&world, 4, 64, 4).expect("beacon beams under open sky");
        assert_eq!(beam.segments.last().unwrap().color, dye_color(14));
        assert_eq!(dye_color(14), [0.6, 0.2, 0.2]);
        assert!(beam.height() > 0);
    }

    #[test]
    fn plain_glass_keeps_the_beam_white_and_opaque_blocks_kill_it() {
        let glass = world_with_blocks(&[
            ((0, 64, 0), BlockState::new(BEACON, 0)),
            ((0, 65, 0), BlockState::new(20, 0)),
        ]);
        let beam = beam_at(&glass, 0, 64, 0).expect("glass is transparent");
        assert_eq!(beam.segments[0].color, [1.0, 1.0, 1.0]);

        let roofed = world_with_blocks(&[
            ((0, 64, 0), BlockState::new(BEACON, 0)),
            ((0, 65, 0), BlockState::new(1, 0)),
        ]);
        assert!(beam_at(&roofed, 0, 64, 0).is_none());
    }

    #[test]
    fn beams_are_crossed_quads_with_untinted_uvs() {
        let beam = BeaconBeam {
            position: [0, 64, 0],
            segments: vec![BeaconSegment {
                color: [0.2, 0.3, 0.7],
                height: 40,
            }],
        };
        let mesh = build_beams(&[beam], 0.0);
        // Four core quads and four halo quads, six indices each.
        assert_eq!(mesh.indices.len(), 8 * 6);
        assert!(mesh.vertices.iter().all(|vertex| vertex.uv == [-1.0, -1.0]));
        // Glow vertices are the fainter half of the mesh.
        assert!(mesh
            .vertices
            .iter()
            .any(|vertex| (vertex.color[3] - GLOW_ALPHA).abs() < 1e-6));
        // Every vertex carries the beam colour.
        assert!(mesh
            .vertices
            .iter()
            .all(|vertex| (vertex.color[0] - 0.2).abs() < 1e-6));
    }

    #[test]
    fn scanning_a_column_finds_the_beacon() {
        let world = world_with_blocks(&[((5, 70, 6), BlockState::new(BEACON, 0))]);
        let beam = &find_beams(&world)[0];
        assert_eq!(beam.position, [5, 70, 6]);
    }
}

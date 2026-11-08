//! Client-side player movement and collision.
//!
//! This is vanilla 1.8's `EntityLivingBase.travel`, reimplemented in blocks per
//! tick: input adds `aiMoveSpeed * 0.16277136 / friction^3` of velocity on the
//! ground (0.02 in the air), the move happens, then friction and gravity are
//! applied. That acceleration curve is what makes walking feel like Minecraft
//! rather than a teleport.
//!
//! Velocities are stored in blocks per second (`vx`, `vy`, `vz`) so callers can
//! reason in real units; the tick maths converts at the boundary.

use nephele_world::block::ids as block_ids;
use nephele_world::shapes::{block_shape, has_collision};
use nephele_world::World;

pub const WIDTH: f64 = 0.6;
pub const HEIGHT: f64 = 1.8;
pub const EYE_HEIGHT: f64 = 1.62;

/// Steady-state speeds the vanilla constants below produce, for reference.
pub const WALK_SPEED: f64 = 4.405;
pub const SPRINT_SPEED: f64 = 5.726;
pub const SNEAK_SPEED: f64 = 1.322;
pub const FLY_SPEED: f64 = 10.9;
pub const FLY_SPRINT_SPEED: f64 = 21.8;

/// Vanilla's player movement-speed attribute.
const MOVEMENT_SPEED: f64 = 0.1;
/// Default block slipperiness; friction is `slipperiness * 0.91`.
const SLIPPERINESS: f64 = 0.6;
/// `BlockSlime.slipperiness` (1.8): 0.8, so you keep sliding on a pad.
const SLIME_SLIPPERINESS: f64 = 0.8;
/// `BlockIce.slipperiness` (1.8), shared by packed ice.
const ICE_SLIPPERINESS: f64 = 0.98;
/// `jumpMovementFactor`: the much smaller acceleration available in the air.
const AIR_ACCEL: f64 = 0.02;
/// 0.08 blocks/tick of gravity.
const GRAVITY_TICK: f64 = 0.08;
/// `0.42` blocks/tick launch velocity: a ~1.25 block jump.
pub const JUMP_VELOCITY_TICK: f64 = 0.42;
/// Vanilla's `stepHeight`: the tallest ledge a walking player climbs without
/// jumping (slabs, single stair steps, pressure plates).
pub const STEP_HEIGHT: f64 = 0.6;
/// `3.92` blocks/tick terminal velocity.
pub const TERMINAL_VELOCITY_TICK: f64 = 3.92;

const EPS: f64 = 1e-7;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Input {
    /// -1 (back) .. 1 (forward)
    pub forward: f64,
    /// -1 (left) .. 1 (right)
    pub strafe: f64,
    pub jump: bool,
    pub sneak: bool,
    pub sprint: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerState {
    /// Feet position in world coordinates.
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub vx: f64,
    pub vy: f64,
    pub vz: f64,
    pub on_ground: bool,
    pub flying: bool,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            x: 0.5,
            y: 80.0,
            z: 0.5,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            on_ground: false,
            flying: false,
        }
    }
}

impl PlayerState {
    pub fn at(x: f64, y: f64, z: f64) -> Self {
        Self {
            x,
            y,
            z,
            ..Self::default()
        }
    }

    pub fn eye(&self) -> [f64; 3] {
        [self.x, self.y + EYE_HEIGHT, self.z]
    }

    pub fn teleport(&mut self, x: f64, y: f64, z: f64) {
        self.x = x;
        self.y = y;
        self.z = z;
        self.vx = 0.0;
        self.vy = 0.0;
        self.vz = 0.0;
    }

    /// An S08 correction cancels motion only on axes with absolute coordinates.
    pub fn correct_position(&mut self, pos: crate::Vec3, relative_flags: u8) {
        self.x = pos.x;
        self.y = pos.y;
        self.z = pos.z;
        if relative_flags & 1 == 0 {
            self.vx = 0.0;
        }
        if relative_flags & 2 == 0 {
            self.vy = 0.0;
        }
        if relative_flags & 4 == 0 {
            self.vz = 0.0;
        }
    }

    /// Replace the current motion, in blocks per second.
    ///
    /// The server owns launches, knockback and falls, and expresses them as a
    /// velocity on our own entity. Setting it here (rather than waiting for the
    /// next physics tick) is what makes a launch pad feel instant.
    pub fn set_velocity(&mut self, vx: f64, vy: f64, vz: f64) {
        self.vx = vx;
        self.vy = vy;
        self.vz = vz;
        self.on_ground = false;
    }

    /// Advance by `dt` seconds. `yaw` uses the camera convention (0 = -Z).
    pub fn step(&mut self, dt: f64, input: &Input, yaw: f32, world: &World) {
        let dt = dt.clamp(0.0, 0.1);
        if dt == 0.0 {
            return;
        }

        // A teleport can land us before the target chunk has arrived, leaving
        // the box inside terrain once it does. Lift out of any overlap.
        if self.collides(world) {
            for step in 1..=16 {
                self.y += 0.25;
                if !self.collides(world) {
                    self.vy = 0.0;
                    break;
                }
                if step == 16 {
                    // Give up and let gravity sort it out next frame.
                    self.y -= 4.0;
                }
            }
        }

        let (sin_yaw, cos_yaw) = (yaw as f64).sin_cos();
        let forward = (sin_yaw, -cos_yaw);
        let right = (cos_yaw, sin_yaw);

        if self.flying {
            let mut dx = forward.0 * input.forward + right.0 * input.strafe;
            let mut dz = forward.1 * input.forward + right.1 * input.strafe;
            let length = (dx * dx + dz * dz).sqrt();
            if length > 1.0 {
                dx /= length;
                dz /= length;
            }
            let speed = if input.sprint {
                FLY_SPRINT_SPEED
            } else {
                FLY_SPEED
            };
            self.vx = dx * speed;
            self.vz = dz * speed;
            self.vy = if input.jump {
                speed
            } else if input.sneak {
                -speed
            } else {
                0.0
            };
            self.on_ground = false;
            self.move_axis(world, 1, self.vy * dt);
            self.move_axis(world, 0, self.vx * dt);
            self.move_axis(world, 2, self.vz * dt);
            return;
        }

        let mut vx = self.vx * dt;
        let mut vy = self.vy * dt;
        let mut vz = self.vz * dt;

        let ground_friction = self.ground_slipperiness(world) * 0.91;
        let ai_speed = MOVEMENT_SPEED
            * if input.sneak {
                0.3
            } else if input.sprint {
                1.3
            } else {
                1.0
            };
        let accel = if self.on_ground {
            ai_speed * 0.16277136 / (ground_friction * ground_friction * ground_friction)
        } else {
            AIR_ACCEL
        };

        let mut fwd = input.forward;
        let mut strafe = input.strafe;
        let length_sq = fwd * fwd + strafe * strafe;
        if length_sq >= 1.0e-4 {
            // Vanilla's `moveFlying`: normalise, then scale by the acceleration.
            let mut length = length_sq.sqrt();
            if length < 1.0 {
                length = 1.0;
            }
            let scale = accel / length;
            fwd *= scale;
            strafe *= scale;
            vx += forward.0 * fwd + right.0 * strafe;
            vz += forward.1 * fwd + right.1 * strafe;
        }

        if input.jump && self.on_ground {
            vy = JUMP_VELOCITY_TICK;
        }

        self.on_ground = false;
        let falling = vy < 0.0;
        if self.move_axis(world, 1, vy) {
            if falling && !input.sneak && self.standing_on_slime(world) {
                // `BlockSlime.onLanded` (1.8): motion is reflected with the
                // speed it arrived at, so a pad throws you as high as you fell
                // instead of stopping you dead. Sneaking opts out, which is how
                // vanilla lets you get off a pad without bouncing.
                vy = -vy;
                self.on_ground = false;
            } else {
                vy = 0.0;
            }
        }
        let grounded = self.on_ground;

        // Vanilla's sneak edge protection: a crouched player on the ground
        // refuses a step that would put them over a drop, rather than walking
        // off the edge of the block.
        if input.sneak && grounded {
            vx = self.edge_clamped(world, 0, vx);
            vz = self.edge_clamped(world, 2, vz);
        }

        if self.move_axis(world, 0, vx) && (!grounded || !self.try_step(world, 0, vx)) {
            vx = 0.0;
        }
        if self.move_axis(world, 2, vz) && (!grounded || !self.try_step(world, 2, vz)) {
            vz = 0.0;
        }

        let friction = if self.on_ground {
            ground_friction
        } else {
            0.91
        };
        vx *= friction;
        vz *= friction;
        vy = ((vy - GRAVITY_TICK) * 0.98).max(-TERMINAL_VELOCITY_TICK);

        self.vx = vx / dt;
        self.vy = vy / dt;
        self.vz = vz / dt;
    }

    /// Move along one axis, stopping against blocks. Returns `true` if blocked.
    fn move_axis(&mut self, world: &World, axis: usize, delta: f64) -> bool {
        if delta == 0.0 {
            return false;
        }

        // Sub-step so fast movement can't tunnel, then bisect against a blocked
        // step so the player rests exactly on the surface.
        let steps = (delta.abs() / 0.5).ceil().max(1.0) as usize;
        let step = delta / steps as f64;

        for _ in 0..steps {
            let start = self.axis(axis);
            self.set_axis(axis, start + step);
            if !self.collides(world) {
                continue;
            }

            let mut free = start;
            let mut blocked = start + step;
            for _ in 0..10 {
                let mid = 0.5 * (free + blocked);
                self.set_axis(axis, mid);
                if self.collides(world) {
                    blocked = mid;
                } else {
                    free = mid;
                }
            }
            self.set_axis(axis, free);

            if axis == 1 && delta < 0.0 {
                self.on_ground = true;
            }
            return true;
        }
        false
    }

    /// Try the same horizontal move from [`STEP_HEIGHT`] higher up, then settle
    /// back down onto the ledge.
    ///
    /// This is vanilla's auto-step: walking into a slab or a single stair step climbs it instead of stopping dead.
    fn try_step(&mut self, world: &World, axis: usize, delta: f64) -> bool {
        let saved = *self;
        self.y += STEP_HEIGHT;
        if self.collides(world) || self.move_axis(world, axis, delta) {
            *self = saved;
            return false;
        }

        if !self.settle_down(world, STEP_HEIGHT) {
            // Nothing to land on: keep the lift and let gravity do the rest.
            self.y = saved.y + STEP_HEIGHT;
        }
        true
    }

    /// Drop the player onto the first surface within `max_drop`.
    ///
    /// Returns `false` when the air below is clear all the way down, which
    /// means there is no ledge to stand on at this height.
    fn settle_down(&mut self, world: &World, max_drop: f64) -> bool {
        let top = self.y;
        let bottom = self.y - max_drop;
        self.y = bottom;
        if !self.collides(world) {
            self.y = top;
            return false;
        }

        let mut free = top;
        let mut blocked = bottom;
        for _ in 0..10 {
            let mid = 0.5 * (free + blocked);
            self.y = mid;
            if self.collides(world) {
                blocked = mid;
            } else {
                free = mid;
            }
        }
        self.y = free;
        self.on_ground = true;
        true
    }

    /// `Block.slipperiness` for the block the player is standing on.
    ///
    /// Vanilla 1.8 makes slime and ice slippery, which is what turns a slime
    /// pad into a launch instead of a wall of friction. Anything else uses the
    /// default 0.6.
    fn ground_slipperiness(&self, world: &World) -> f64 {
        let layer = (self.y - 0.05).floor() as i32;
        let min_x = self.x - WIDTH / 2.0 + EPS;
        let max_x = self.x + WIDTH / 2.0 - EPS;
        let min_z = self.z - WIDTH / 2.0 + EPS;
        let max_z = self.z + WIDTH / 2.0 - EPS;
        let mut slipperiness: f64 = SLIPPERINESS;
        for block_x in (min_x.floor() as i32)..=(max_x.floor() as i32) {
            for block_z in (min_z.floor() as i32)..=(max_z.floor() as i32) {
                let candidate = match world.get_block(block_x, layer, block_z).id() {
                    block_ids::SLIME => SLIME_SLIPPERINESS,
                    block_ids::ICE | block_ids::PACKED_ICE => ICE_SLIPPERINESS,
                    _ => SLIPPERINESS,
                };
                slipperiness = slipperiness.max(candidate);
            }
        }
        slipperiness
    }

    /// Whether any block directly under the player's feet is slime.
    fn standing_on_slime(&self, world: &World) -> bool {
        let layer = (self.y - 0.05).floor() as i32;
        let min_x = self.x - WIDTH / 2.0 + EPS;
        let max_x = self.x + WIDTH / 2.0 - EPS;
        let min_z = self.z - WIDTH / 2.0 + EPS;
        let max_z = self.z + WIDTH / 2.0 - EPS;
        for block_x in (min_x.floor() as i32)..=(max_x.floor() as i32) {
            for block_z in (min_z.floor() as i32)..=(max_z.floor() as i32) {
                if world.get_block(block_x, layer, block_z).id() == block_ids::SLIME {
                    return true;
                }
            }
        }
        false
    }

    /// The largest part of `delta` that still leaves ground under the player.
    ///
    /// Failing every step means the player is already at the edge, so nothing
    /// of the movement is allowed — which is the point of sneaking.
    fn edge_clamped(&self, world: &World, axis: usize, delta: f64) -> f64 {
        if delta.abs() < 1e-9 {
            return 0.0;
        }
        let start = self.axis(axis);
        for fraction in [1.0, 0.5, 0.25, 0.125, 0.0625] {
            let mut probe = *self;
            probe.set_axis(axis, start + delta * fraction);
            if probe.has_ground(world) {
                return delta * fraction;
            }
        }
        0.0
    }

    /// True when a solid block sits directly beneath the feet.
    ///
    /// Only the block layer the feet rest on is checked, and the footprint is
    /// inset slightly, so standing flush against a wall is not mistaken for
    /// standing on a floor.
    fn has_ground(&self, world: &World) -> bool {
        let foot = self.y - 0.05;
        let layer = foot.floor() as i32;
        let min_x = self.x - WIDTH / 2.0 + EPS;
        let max_x = self.x + WIDTH / 2.0 - EPS;
        let min_z = self.z - WIDTH / 2.0 + EPS;
        let max_z = self.z + WIDTH / 2.0 - EPS;

        for block_x in (min_x.floor() as i32)..=(max_x.floor() as i32) {
            for block_z in (min_z.floor() as i32)..=(max_z.floor() as i32) {
                if has_collision(world.get_block(block_x, layer, block_z)) {
                    return true;
                }
            }
        }
        false
    }

    fn axis(&self, axis: usize) -> f64 {
        match axis {
            0 => self.x,
            1 => self.y,
            _ => self.z,
        }
    }

    fn set_axis(&mut self, axis: usize, value: f64) {
        match axis {
            0 => self.x = value,
            1 => self.y = value,
            _ => self.z = value,
        }
    }

    fn collides(&self, world: &World) -> bool {
        let min_x = self.x - WIDTH / 2.0;
        let max_x = self.x + WIDTH / 2.0;
        let min_y = self.y;
        let max_y = self.y + HEIGHT;
        let min_z = self.z - WIDTH / 2.0;
        let max_z = self.z + WIDTH / 2.0;

        let x0 = min_x.floor() as i32;
        let x1 = (max_x - EPS).floor() as i32;
        let y0 = min_y.floor() as i32;
        let y1 = (max_y - EPS).floor() as i32;
        let z0 = min_z.floor() as i32;
        let z1 = (max_z - EPS).floor() as i32;

        for bx in x0..=x1 {
            for by in y0..=y1 {
                for bz in z0..=z1 {
                    let state = world.get_block(bx, by, bz);
                    if !has_collision(state) {
                        continue;
                    }
                    // Partial blocks (slabs, stairs, doors, fences) collide as
                    // their real boxes rather than as a full cube. Vanilla
                    // tests these with strict inequalities, so resting exactly
                    // on a surface is not an overlap.
                    for shape in block_shape(state.id(), state.meta()) {
                        let min = [
                            bx as f64 + shape.min[0] as f64 / 16.0,
                            by as f64 + shape.min[1] as f64 / 16.0,
                            bz as f64 + shape.min[2] as f64 / 16.0,
                        ];
                        let max = [
                            bx as f64 + shape.max[0] as f64 / 16.0,
                            by as f64 + shape.max[1] as f64 / 16.0,
                            bz as f64 + shape.max[2] as f64 / 16.0,
                        ];
                        if min_x < max[0]
                            && max_x > min[0]
                            && min_y < max[1]
                            && max_y > min[1]
                            && min_z < max[2]
                            && max_z > min[2]
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nephele_world::{BlockState, ChunkColumn};

    /// A 5x5-chunk stone plane, so long walks don't run off the edge.
    fn flat_world(floor_y: usize) -> World {
        floor_of(block_ids::STONE, floor_y)
    }

    /// A 5x5-chunk plane of one block id, for surface-friction and bounce tests.
    fn floor_of(id: u16, floor_y: usize) -> World {
        let mut world = World::new(0);
        for cx in -2..=2 {
            for cz in -2..=2 {
                let mut column = ChunkColumn::empty(cx, cz);
                for x in 0..16 {
                    for z in 0..16 {
                        column.set_block(x, floor_y, z, BlockState::new(id, 0));
                    }
                }
                world.insert(column);
            }
        }
        world
    }

    fn run_ticks(player: &mut PlayerState, input: &Input, ticks: usize, world: &World) {
        for _ in 0..ticks {
            player.step(0.05, input, 0.0, world);
        }
    }

    /// One stone block at (0, 9, 0) with nothing around it, so its top face is
    /// the only place to stand.
    fn pillar() -> World {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(0, 9, 0, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(column);
        world
    }

    /// A player resting on the floor must not creep upward. The unstick
    /// routine that lifts a player out of terrain must not mistake "standing
    /// exactly on a block" for "buried in one".
    #[test]
    fn resting_on_the_floor_does_not_drift_upward() {
        let world = flat_world(9);
        let mut player = PlayerState::at(0.5, 12.0, 0.5);
        let idle = Input::default();
        run_ticks(&mut player, &idle, 120, &world);

        let settled = player.y;
        let mut highest = settled;
        for _ in 0..200 {
            player.step(0.05, &idle, 0.0, &world);
            highest = highest.max(player.y);
        }
        assert!(
            highest - settled < 1e-6,
            "the player drifted upward from {settled} to {highest}"
        );
        assert!((settled - 10.0).abs() < 0.01, "settled at {settled}");
    }
    #[test]
    fn sneaking_keeps_you_on_the_edge() {
        let world = pillar();
        let mut player = PlayerState::at(0.5, 10.0, 0.5);
        player.on_ground = true;

        let input = Input {
            forward: 1.0,
            sneak: true,
            ..Input::default()
        };
        run_ticks(&mut player, &input, 60, &world);

        assert!(
            (player.y - 10.0).abs() < 0.05,
            "sneaking walked off the block and fell to y = {}",
            player.y
        );
        assert!(
            player.z > -0.35,
            "sneaking should stop at the edge, not at z = {}",
            player.z
        );
    }

    #[test]
    fn walking_off_the_edge_still_falls_without_sneaking() {
        let world = pillar();
        let mut player = PlayerState::at(0.5, 10.0, 0.5);
        player.on_ground = true;

        let input = Input {
            forward: 1.0,
            ..Input::default()
        };
        run_ticks(&mut player, &input, 60, &world);

        assert!(
            player.y < 9.0,
            "without sneaking the player should walk off and fall, y = {}",
            player.y
        );
    }

    #[test]
    fn sneaking_still_lets_you_walk_along_a_wide_floor() {
        let world = flat_world(9);
        let mut player = PlayerState::at(0.5, 10.0, 0.5);
        player.on_ground = true;

        let input = Input {
            forward: 1.0,
            sneak: true,
            ..Input::default()
        };
        run_ticks(&mut player, &input, 40, &world);

        assert!(
            player.z < -1.0,
            "sneaking must not freeze movement, z = {}",
            player.z
        );
        assert!((player.y - 10.0).abs() < 0.05, "y = {}", player.y);
    }

    #[test]
    fn falls_until_it_rests_on_the_floor() {
        let world = flat_world(64);
        let mut player = PlayerState::at(8.5, 70.0, 8.5);
        run_ticks(&mut player, &Input::default(), 200, &world);
        assert!(player.on_ground, "should land");
        assert!((player.y - 65.0).abs() < 1e-3, "y = {}", player.y);
        // Vanilla keeps pulling down every tick even when standing.
    }

    #[test]
    fn walking_forward_moves_towards_minus_z() {
        let world = flat_world(64);
        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let input = Input {
            forward: 1.0,
            ..Input::default()
        };
        run_ticks(&mut player, &input, 20, &world);
        assert!(player.z < 8.5 - 1.0, "z = {}", player.z);
        assert!((player.x - 8.5).abs() < 1e-6);
    }

    #[test]
    fn acceleration_ramps_up_instead_of_snapping() {
        let world = flat_world(64);
        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let input = Input {
            forward: 1.0,
            ..Input::default()
        };
        player.step(0.05, &input, 0.0, &world);
        // A vanilla walk starts at ~0.1 blocks/tick, nowhere near top speed.
        let first = 8.5 - player.z;
        assert!(first > 0.05 && first < 0.15, "first tick moved {first}");
    }

    #[test]
    fn walk_sprint_and_sneak_match_vanilla_speeds() {
        let world = flat_world(64);
        let measure = |input: Input| -> f64 {
            let mut player = PlayerState::at(8.5, 65.0, 8.5);
            player.on_ground = true;
            // Settle at the steady state first.
            run_ticks(&mut player, &input, 60, &world);
            let start = player.z;
            run_ticks(&mut player, &input, 20, &world);
            (start - player.z) / (20.0 * 0.05)
        };

        let walk = measure(Input {
            forward: 1.0,
            ..Input::default()
        });
        let sprint = measure(Input {
            forward: 1.0,
            sprint: true,
            ..Input::default()
        });
        let sneak = measure(Input {
            forward: 1.0,
            sneak: true,
            ..Input::default()
        });

        assert!((walk - 4.4).abs() < 0.3, "walk = {walk} b/s");
        assert!((sprint - 5.7).abs() < 0.4, "sprint = {sprint} b/s");
        assert!((sneak - 1.3).abs() < 0.2, "sneak = {sneak} b/s");
        assert!(sprint > walk && walk > sneak);
    }

    #[test]
    fn releasing_input_slides_to_a_stop() {
        let world = flat_world(64);
        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let input = Input {
            forward: 1.0,
            ..Input::default()
        };
        run_ticks(&mut player, &input, 40, &world);
        // Vanilla stores the *post-friction* motion, so measure displacement.
        let before = player.z;
        player.step(0.05, &input, 0.0, &world);
        let per_tick = (before - player.z).abs();
        assert!(per_tick > 0.15, "should be at speed, moved {per_tick}");

        run_ticks(&mut player, &Input::default(), 30, &world);
        let before = player.z;
        player.step(0.05, &Input::default(), 0.0, &world);
        let per_tick = (before - player.z).abs();
        assert!(per_tick < 0.01, "still sliding {per_tick}");
    }

    #[test]
    fn walls_block_movement() {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                column.set_block(x, 64, z, BlockState::STONE);
            }
            // Wall two blocks in front of the player (towards -Z).
            for y in 65..67 {
                column.set_block(x, y, 6, BlockState::STONE);
            }
        }
        let mut world = World::new(0);
        world.insert(column);

        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let input = Input {
            forward: 1.0,
            ..Input::default()
        };
        run_ticks(&mut player, &input, 100, &world);
        assert!(
            player.z > 7.0 - 1e-6,
            "walked through the wall: z = {}",
            player.z
        );
    }

    #[test]
    fn a_jump_clears_a_full_block() {
        let world = flat_world(64);
        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        let mut peak: f64 = player.y;
        for tick in 0..40 {
            let input = if tick == 0 { jump } else { Input::default() };
            player.step(0.05, &input, 0.0, &world);
            peak = peak.max(player.y);
        }
        assert!(
            peak - 65.0 >= 1.05,
            "apex was {:.2} blocks, a full block needs 1.0+",
            peak - 65.0
        );
    }

    #[test]
    fn you_can_jump_onto_a_one_block_step() {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                column.set_block(x, 64, z, BlockState::STONE);
            }
            // A one-block step two blocks in front of the player.
            column.set_block(x, 65, 6, BlockState::STONE);
        }
        let mut world = World::new(0);
        world.insert(column);

        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let mut landed_on_step = false;
        for tick in 0..80 {
            let input = Input {
                forward: 1.0,
                sprint: true,
                jump: tick % 10 == 0,
                ..Input::default()
            };
            player.step(0.05, &input, 0.0, &world);
            if player.on_ground && player.y > 65.5 {
                landed_on_step = true;
                break;
            }
        }
        assert!(landed_on_step, "never climbed the step, y = {}", player.y);
    }

    #[test]
    fn jumping_leaves_the_ground_and_lands_again() {
        let world = flat_world(64);
        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        player.step(0.05, &jump, 0.0, &world);
        assert!(!player.on_ground);
        assert!(player.y > 65.2, "peak {}", player.y);

        run_ticks(&mut player, &Input::default(), 100, &world);
        assert!(player.on_ground);
        assert!((player.y - 65.0).abs() < 1e-3);
    }

    #[test]
    fn flying_ignores_gravity() {
        let world = flat_world(64);
        let mut player = PlayerState::at(8.5, 90.0, 8.5);
        player.flying = true;
        run_ticks(&mut player, &Input::default(), 40, &world);
        assert!((player.y - 90.0).abs() < 1e-6, "y = {}", player.y);
    }

    #[test]
    fn stuck_players_are_lifted_out_of_blocks() {
        let world = flat_world(64);
        // Start with the feet inside the floor block.
        let mut player = PlayerState::at(8.5, 64.4, 8.5);
        player.step(0.05, &Input::default(), 0.0, &world);
        assert!(player.y >= 65.0 - 1e-3, "still stuck at {}", player.y);
        assert!(!player.collides(&world));
    }

    #[test]
    fn eye_height_is_above_the_feet() {
        let player = PlayerState::at(1.0, 2.0, 3.0);
        assert_eq!(player.eye(), [1.0, 2.0 + EYE_HEIGHT, 3.0]);
    }

    #[test]
    fn landing_on_a_slime_pad_bounces_back_up() {
        // A 3x3 pad is what Hypixel's lobby actually has; one block is enough
        // for the physics, but the model is the same.
        let world = floor_of(block_ids::SLIME, 64);
        let mut player = PlayerState::at(8.5, 69.5, 8.5);
        player.on_ground = false;

        let mut bounced = false;
        let mut apex: f64 = 0.0;
        for _ in 0..80 {
            player.step(0.05, &Input::default(), 0.0, &world);
            if player.vy > 1.0 {
                bounced = true;
            }
            if bounced {
                apex = apex.max(player.y);
            }
        }

        assert!(bounced, "the pad did not reflect the fall");
        // 1.8 reflects the landing speed, so the ballpark is the fall height.
        // The per-tick 0.98 damping in this integrator bites hardest on the
        // way up, which is why it is not an exact return.
        assert!(apex > 67.0, "bounce only reached {apex}");
    }

    #[test]
    fn a_stone_floor_stops_a_fall_instead_of_bouncing() {
        let world = flat_world(64);
        let mut player = PlayerState::at(8.5, 69.5, 8.5);
        player.on_ground = false;

        run_ticks(&mut player, &Input::default(), 40, &world);

        assert!(player.on_ground);
        assert!((player.y - 65.0).abs() < 1e-3, "y = {}", player.y);
        // A resting player still reports one tick of gravity on the wire
        // (vanilla sends the same), so the check is "not moving upward".
        assert!(player.vy <= 0.0);
    }

    #[test]
    fn slime_and_ice_have_their_own_slipperiness() {
        let slime = PlayerState::at(8.5, 65.0, 8.5);
        assert_eq!(
            slime.ground_slipperiness(&floor_of(block_ids::SLIME, 64)),
            SLIME_SLIPPERINESS
        );

        let ice = PlayerState::at(8.5, 65.0, 8.5);
        assert_eq!(
            ice.ground_slipperiness(&floor_of(block_ids::ICE, 64)),
            ICE_SLIPPERINESS
        );

        let stone = PlayerState::at(8.5, 65.0, 8.5);
        assert_eq!(stone.ground_slipperiness(&flat_world(64)), SLIPPERINESS);
    }

    #[test]
    fn a_server_velocity_launches_the_player() {
        let world = flat_world(64);
        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        // 20 blocks/second is 1 block/tick upward: a launch pad, not a hop.
        player.set_velocity(0.0, 20.0, 0.0);

        player.step(0.05, &Input::default(), 0.0, &world);

        assert!(!player.on_ground);
        assert!(player.y > 65.5, "launch height was {}", player.y);
        assert!(
            player.vy > 0.0,
            "gravity ate the launch: vy = {}",
            player.vy
        );
    }

    #[test]
    fn walking_into_a_slab_steps_up_onto_it() {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                column.set_block(x, 64, z, BlockState::STONE);
            }
            // A four-deep half-slab platform in front of the player.
            for z in 3..=6 {
                column.set_block(x, 65, z, BlockState::new(44, 0));
            }
        }
        let mut world = World::new(0);
        world.insert(column);

        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let input = Input {
            forward: 1.0,
            ..Input::default()
        };
        // 20 ticks keeps the player on the four-deep platform; 30 would walk
        // off its far edge.
        run_ticks(&mut player, &input, 20, &world);

        assert!(player.z < 7.0, "stopped at the slab edge, z = {}", player.z);
        assert!(
            (player.y - 65.5).abs() < 1e-2,
            "should stand on the slab at 65.5, y = {}",
            player.y
        );
    }

    #[test]
    fn pressure_plates_are_walked_over() {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                column.set_block(x, 64, z, BlockState::STONE);
            }
            column.set_block(x, 65, 4, BlockState::new(72, 0));
        }
        let mut world = World::new(0);
        world.insert(column);

        let mut player = PlayerState::at(8.5, 65.0, 8.5);
        player.on_ground = true;
        let input = Input {
            forward: 1.0,
            ..Input::default()
        };
        for _ in 0..40 {
            player.step(0.05, &input, 0.0, &world);
            assert!(
                (player.y - 65.0).abs() < 1e-3,
                "plate raised feet at z={}",
                player.z
            );
        }

        assert!(
            player.z < 4.0,
            "the plate blocked the player at z = {}",
            player.z
        );
        assert!(
            (player.y - 65.0).abs() < 1e-3,
            "walking over a plate lifted y to {}",
            player.y
        );
    }

    #[test]
    fn a_bottom_slab_is_half_a_block_high() {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                column.set_block(x, 64, z, BlockState::STONE);
            }
        }
        column.set_block(8, 65, 8, BlockState::new(44, 0));
        let mut world = World::new(0);
        world.insert(column);

        let mut player = PlayerState::at(8.5, 66.5, 8.5);
        run_ticks(&mut player, &Input::default(), 120, &world);

        assert!(player.on_ground);
        assert!(
            (player.y - 65.5).abs() < 1e-3,
            "slab floor is at 65.5, player rests at {}",
            player.y
        );
    }
}

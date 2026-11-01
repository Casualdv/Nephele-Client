//! The block-breaking state machine, following vanilla `PlayerControllerMP`.
//!
//! Vanilla accumulates `curBlockDamageMP` every tick by
//! `toolEfficiency / hardness / (canHarvest ? 30 : 100)` and destroys the block
//! once it reaches 1.0, then ignores clicks for 5 ticks (`blockHitDelay`).
//! Nephele has no tools yet, so efficiency is always the bare-hand 1.0.

use nephele_world::block::{hardness, requires_tool, UNBREAKABLE};
use nephele_world::{BlockState, World};

/// Where the crosshair is pointing, as both the wire and the renderer need it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aim {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `Face` discriminant, 0..=5.
    pub face: u8,
    /// Hit point inside the block, 0..1 per axis (C08's cursor).
    pub cursor: [f32; 3],
}

impl Aim {
    pub const fn pos(&self) -> (i32, i32, i32) {
        (self.x, self.y, self.z)
    }
}

/// What the mining tick wants the caller to send, and for which block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MiningEvent {
    /// `C07 START_DESTROY_BLOCK` (plus an arm swing).
    Start(Aim),
    /// `C07 STOP_DESTROY_BLOCK` — the block is gone.
    Stop(Aim),
    /// `C07 ABORT_DESTROY_BLOCK` — stopped aiming or released the button.
    Abort(Aim),
}

/// Vanilla's post-break cooldown, in ticks.
pub const HIT_DELAY_TICKS: u8 = 5;

#[derive(Debug, Default, Clone)]
pub struct Miner {
    current: Option<Aim>,
    progress: f32,
    hit_delay: u8,
}

impl Miner {
    pub fn target(&self) -> Option<(i32, i32, i32)> {
        self.current.map(|aim| aim.pos())
    }

    pub fn progress(&self) -> f32 {
        self.progress
    }

    /// Crack overlay stage, 0..=9, or `None` when nothing is being mined.
    pub fn stage(&self) -> Option<u8> {
        self.current?;
        Some(((self.progress * 10.0) as u8).min(9))
    }

    /// Advance one tick (1/20 s) and report what to send.
    pub fn tick(
        &mut self,
        world: &World,
        aim: Option<Aim>,
        digging: bool,
        creative: bool,
    ) -> Vec<MiningEvent> {
        let mut events = Vec::new();

        if !digging {
            self.stop(&mut events);
            return events;
        }

        // Vanilla only honours the hit delay in creative, but it also keeps
        // rapid clicks from starting a second block immediately.
        if self.hit_delay > 0 {
            self.hit_delay -= 1;
            return events;
        }

        let Some(aim) = aim else {
            self.stop(&mut events);
            return events;
        };

        let state = world.get_block(aim.x, aim.y, aim.z);
        if state.is_air() || hardness(state.id()) == UNBREAKABLE {
            self.stop(&mut events);
            return events;
        }

        if self.current.map(|current| current.pos()) != Some(aim.pos()) {
            self.stop(&mut events);
            self.current = Some(aim);
            self.progress = 0.0;
            events.push(MiningEvent::Start(aim));

            if creative {
                // Creative destroys on the first click; the server applies it
                // itself, so this is just START followed by STOP.
                events.push(MiningEvent::Stop(aim));
                self.current = None;
                self.hit_delay = HIT_DELAY_TICKS;
                return events;
            }
        }

        self.progress += tick_damage(state);
        if self.progress >= 1.0 {
            events.push(MiningEvent::Stop(aim));
            self.current = None;
            self.progress = 0.0;
            self.hit_delay = HIT_DELAY_TICKS;
        }
        events
    }

    /// Abort whatever is in progress (un-aim, release, match end).
    pub fn abort(&mut self) -> Option<Aim> {
        let aim = self.current.take()?;
        self.progress = 0.0;
        Some(aim)
    }

    fn stop(&mut self, events: &mut Vec<MiningEvent>) {
        if let Some(aim) = self.current.take() {
            events.push(MiningEvent::Abort(aim));
        }
        self.progress = 0.0;
    }
}

/// Fraction of the block destroyed by one tick of bare-handed digging.
pub fn tick_damage(state: BlockState) -> f32 {
    let hardness = hardness(state.id());
    if hardness <= 0.0 {
        // Instantly breakable (flowers, torches, ...).
        return 1.0;
    }
    let divisor = if requires_tool(state.id()) {
        100.0
    } else {
        30.0
    };
    1.0 / (hardness * divisor)
}

/// How many ticks a bare-handed break takes, rounded up.
pub fn break_ticks(state: BlockState) -> u32 {
    let damage = tick_damage(state);
    if damage >= 1.0 {
        return 1;
    }
    (1.0 / damage).ceil() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use nephele_world::block::ids;
    use nephele_world::ChunkColumn;

    fn world_with(block: BlockState) -> World {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(1, 10, 1, block);
        let mut world = World::new(0);
        world.insert(column);
        world
    }

    fn aim() -> Aim {
        Aim {
            x: 1,
            y: 10,
            z: 1,
            face: 1,
            cursor: [0.5, 1.0, 0.5],
        }
    }

    #[test]
    fn bare_hand_stone_takes_the_vanilla_duration() {
        // 1.5 hardness, pickaxe required: 1.5 * 100 = 150 ticks = 7.5 s.
        assert_eq!(break_ticks(BlockState::STONE), 150);
    }

    #[test]
    fn dirt_breaks_in_half_the_time() {
        // 0.5 hardness, no tool required: 0.5 * 30 = 15 ticks.
        assert_eq!(break_ticks(BlockState::new(ids::DIRT, 0)), 15);
    }

    #[test]
    fn mining_a_block_emits_start_then_stop() {
        let world = world_with(BlockState::new(ids::DIRT, 0));
        let mut miner = Miner::default();
        let first = miner.tick(&world, Some(aim()), true, false);
        assert_eq!(first, vec![MiningEvent::Start(aim())]);
        assert_eq!(miner.stage(), Some(0));

        let mut stops = 0;
        for _ in 0..40 {
            let events = miner.tick(&world, Some(aim()), true, false);
            if events
                .iter()
                .any(|event| matches!(event, MiningEvent::Stop(_)))
            {
                stops += 1;
                // The block is gone from the server's point of view; stop here
                // rather than let the never-cleared test world be mined twice.
                break;
            }
        }
        assert_eq!(stops, 1);
        assert!(miner.target().is_none());
    }

    #[test]
    fn releasing_the_button_aborts() {
        let world = world_with(BlockState::STONE);
        let mut miner = Miner::default();
        miner.tick(&world, Some(aim()), true, false);
        let events = miner.tick(&world, Some(aim()), false, false);
        assert_eq!(events, vec![MiningEvent::Abort(aim())]);
        assert!(miner.target().is_none());
    }

    #[test]
    fn switching_target_aborts_the_old_one() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(1, 10, 1, BlockState::STONE);
        column.set_block(2, 10, 1, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(column);

        let mut miner = Miner::default();
        miner.tick(&world, Some(aim()), true, false);
        let other = Aim {
            x: 2,
            y: 10,
            z: 1,
            face: 1,
            cursor: [0.5, 1.0, 0.5],
        };
        let events = miner.tick(&world, Some(other), true, false);
        assert_eq!(
            events,
            vec![MiningEvent::Abort(aim()), MiningEvent::Start(other)]
        );
        assert_eq!(miner.target(), Some((2, 10, 1)));
    }

    #[test]
    fn creative_breaks_on_the_first_tick() {
        let world = world_with(BlockState::STONE);
        let mut miner = Miner::default();
        let events = miner.tick(&world, Some(aim()), true, true);
        assert_eq!(
            events,
            vec![MiningEvent::Start(aim()), MiningEvent::Stop(aim())]
        );
    }

    #[test]
    fn unbreakable_blocks_never_start() {
        let world = world_with(BlockState::BEDROCK);
        let mut miner = Miner::default();
        assert!(miner.tick(&world, Some(aim()), true, false).is_empty());
        assert!(miner.target().is_none());
    }

    #[test]
    fn crack_stage_walks_up_to_nine() {
        let world = world_with(BlockState::new(ids::DIRT, 0));
        let mut miner = Miner::default();
        miner.tick(&world, Some(aim()), true, false);
        let mut seen = vec![];
        for tick in 0..14 {
            if tick > 0 {
                miner.tick(&world, Some(aim()), true, false);
            }
            if let Some(stage) = miner.stage() {
                if seen.last() != Some(&stage) {
                    seen.push(stage);
                }
            }
        }
        assert!(seen.contains(&0), "stages seen: {seen:?}");
        assert!(seen.iter().all(|stage| *stage <= 9));
    }
}

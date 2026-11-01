//! The block-breaking state machine, following vanilla `PlayerControllerMP`.
//!
//! Vanilla accumulates `curBlockDamageMP` every tick by
//! `toolEfficiency / hardness / (canHarvest ? 30 : 100)` and destroys the block
//! once it reaches 1.0, then ignores clicks for 5 ticks (`blockHitDelay`).
//! Nephele has no tools yet, so efficiency is always the bare-hand 1.0.
//!
//! Breaking is instantaneous for now: clicking a block yields it on the same
//! tick and there is no post-break cooldown, so holding the button clears
//! blocks as fast as the crosshair moves. [`tick_damage`] still carries
//! vanilla's hardness formula, ready for when tools make it meaningful again.

use nephele_world::block::{hardness, requires_tool, UNBREAKABLE};
use nephele_world::{BlockState, World};

/// Destroy a block on the tick it is first aimed at, with no cooldown.
pub const INSTANT_MINING: bool = true;

/// How long a destroyed block is remembered while waiting for the server's
/// block update, in ticks. Without this, holding the button would re-send
/// START/STOP for the same block every tick until the update arrives.
const PENDING_TICKS: u8 = 20;

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
    /// A block we just destroyed that the server has not confirmed yet.
    pending: Option<(i32, i32, i32)>,
    pending_ticks: u8,
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
            self.pending = None;
            self.stop(&mut events);
            return events;
        }

        // Wait out a block we have already destroyed. The server sends its
        // block update a round trip later, and re-sending START/STOP for the
        // same block in the meantime would just be noise.
        if let Some(pos) = self.pending {
            let gone = world.get_block(pos.0, pos.1, pos.2).is_air();
            if gone || self.pending_ticks == 0 {
                self.pending = None;
            } else if aim.map(|aim| aim.pos()) == Some(pos) {
                self.pending_ticks -= 1;
                return events;
            } else {
                self.pending = None;
            }
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

        self.progress += if INSTANT_MINING {
            1.0
        } else {
            tick_damage(state)
        };
        if self.progress >= 1.0 {
            events.push(MiningEvent::Stop(aim));
            self.current = None;
            self.progress = 0.0;
            self.hit_delay = if INSTANT_MINING { 0 } else { HIT_DELAY_TICKS };
            if INSTANT_MINING {
                self.pending = Some(aim.pos());
                self.pending_ticks = PENDING_TICKS;
            }
        }
        events
    }

    /// Abort whatever is in progress (un-aim, release, match end).
    pub fn abort(&mut self) -> Option<Aim> {
        self.pending = None;
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
        assert_eq!(
            first,
            vec![MiningEvent::Start(aim()), MiningEvent::Stop(aim())]
        );
        assert!(miner.target().is_none());
    }

    #[test]
    fn holding_the_button_waits_for_the_server_to_remove_the_block() {
        let world = world_with(BlockState::new(ids::DIRT, 0));
        let mut miner = Miner::default();
        assert_eq!(miner.tick(&world, Some(aim()), true, false).len(), 2);
        for _ in 0..5 {
            assert!(
                miner.tick(&world, Some(aim()), true, false).is_empty(),
                "the same block must not be re-sent while it is still there"
            );
        }
    }

    #[test]
    fn the_next_block_breaks_on_the_very_next_tick() {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(1, 10, 1, BlockState::STONE);
        column.set_block(2, 10, 1, BlockState::STONE);
        let mut world = World::new(0);
        world.insert(column);

        let mut miner = Miner::default();
        assert_eq!(miner.tick(&world, Some(aim()), true, false).len(), 2);

        let other = Aim {
            x: 2,
            y: 10,
            z: 1,
            face: 1,
            cursor: [0.5, 1.0, 0.5],
        };
        assert_eq!(
            miner.tick(&world, Some(other), true, false),
            vec![MiningEvent::Start(other), MiningEvent::Stop(other)]
        );
    }

    #[test]
    fn releasing_the_button_sends_nothing_further() {
        let world = world_with(BlockState::STONE);
        let mut miner = Miner::default();
        miner.tick(&world, Some(aim()), true, false);
        assert!(miner.tick(&world, Some(aim()), false, false).is_empty());
        assert!(miner.target().is_none());
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
}

//! The block-breaking state machine, following vanilla `PlayerControllerMP`.
//!
//! The server is the authority and treats the two modes differently
//! (`ItemInWorldManager`), so the client has to follow suit:
//!
//! - **Creative**: `onBlockClicked` harvests on `START_DESTROY_BLOCK`, so the
//!   block is gone the moment it is clicked and the crosshair can move on to
//!   the next one immediately.
//! - **Survival**: the server only harvests once its own timer reaches the
//!   block's hardness (`blockRemoving` checks `>= 0.7`, then `>= 1.0` of the
//!   full time). Nothing a client sends can shorten that, so the client
//!   accumulates exactly like vanilla and sends `STOP` when the block pops.
//!
//! Nephele has no tools yet, so efficiency is always the bare-hand 1.0.

use nephele_world::block::{hardness, requires_tool, UNBREAKABLE};
use nephele_world::{BlockState, World};

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
    /// The block the hit delay belongs to. Vanilla keeps `blockHitDelay` as a
    /// global timer, which throttles a whole row of blocks to one break per
    /// five ticks while the crosshair sweeps along it. Keying the delay on the
    /// block instead means it only throttles repeats of *that* block, so
    /// dragging the crosshair across a row breaks each block at its own speed.
    hit_delay_target: Option<(i32, i32, i32)>,
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
        // rapid clicks from starting a second block immediately. The delay is
        // scoped to the block it was set on: sweeping onto a different block
        // clears it so the row breaks at normal speed.
        if self.hit_delay > 0 {
            let same_block = self.hit_delay_target.is_some()
                && aim.map(|aim| aim.pos()) == self.hit_delay_target;
            if same_block {
                self.hit_delay -= 1;
                if self.hit_delay == 0 {
                    self.hit_delay_target = None;
                }
                return events;
            }
            self.hit_delay = 0;
            self.hit_delay_target = None;
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
        }

        self.progress += if creative { 1.0 } else { tick_damage(state) };
        if self.progress >= 1.0 {
            events.push(MiningEvent::Stop(aim));
            self.current = None;
            self.progress = 0.0;
            // Vanilla's `blockHitDelay`: without it a single click that spans
            // two ticks clears two blocks, and holding the button would mine
            // twenty blocks a second. Keyed on this block so it cannot throttle
            // the next one in a row.
            self.hit_delay = HIT_DELAY_TICKS;
            self.hit_delay_target = Some(aim.pos());
            if creative {
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

    fn two_blocks(block: BlockState) -> World {
        let mut column = ChunkColumn::empty(0, 0);
        column.set_block(1, 10, 1, block);
        column.set_block(2, 10, 1, block);
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

    fn next_to_aim() -> Aim {
        Aim {
            x: 2,
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
    fn survival_starts_then_accumulates_until_the_block_pops() {
        let world = world_with(BlockState::new(ids::DIRT, 0));
        let mut miner = Miner::default();

        assert_eq!(
            miner.tick(&world, Some(aim()), true, false),
            vec![MiningEvent::Start(aim())]
        );
        assert_eq!(miner.stage(), Some(0));

        let mut ticks = 1;
        loop {
            ticks += 1;
            let events = miner.tick(&world, Some(aim()), true, false);
            if events
                .iter()
                .any(|event| matches!(event, MiningEvent::Stop(_)))
            {
                break;
            }
            assert!(ticks < 100, "never finished");
        }

        // Vanilla's 15 ticks for dirt: 1/15 of the block per tick.
        assert_eq!(ticks, 15);
        assert!(miner.target().is_none());
    }

    #[test]
    fn survival_aborts_when_the_button_is_released() {
        let world = world_with(BlockState::STONE);
        let mut miner = Miner::default();
        miner.tick(&world, Some(aim()), true, false);
        let events = miner.tick(&world, Some(aim()), false, false);
        assert_eq!(events, vec![MiningEvent::Abort(aim())]);
        assert!(miner.target().is_none());
    }

    #[test]
    fn survival_aborts_the_old_block_when_the_target_changes() {
        let world = two_blocks(BlockState::STONE);
        let mut miner = Miner::default();
        miner.tick(&world, Some(aim()), true, false);

        let events = miner.tick(&world, Some(next_to_aim()), true, false);
        assert_eq!(
            events,
            vec![MiningEvent::Abort(aim()), MiningEvent::Start(next_to_aim())]
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
        assert!(miner.target().is_none());
    }

    #[test]
    fn creative_sweeps_onto_the_next_block_at_normal_speed() {
        let world = two_blocks(BlockState::STONE);
        let mut miner = Miner::default();
        assert_eq!(miner.tick(&world, Some(aim()), true, true).len(), 2);

        // Dragging the crosshair onto the next block must not inherit the
        // previous block's cooldown: the row breaks at one block per tick.
        assert_eq!(
            miner.tick(&world, Some(next_to_aim()), true, true),
            vec![
                MiningEvent::Start(next_to_aim()),
                MiningEvent::Stop(next_to_aim())
            ]
        );
    }

    #[test]
    fn creative_breaks_a_five_block_row_at_one_per_tick() {
        let mut column = ChunkColumn::empty(0, 0);
        for x in 1..=5 {
            column.set_block(x, 10, 1, BlockState::STONE);
        }
        let mut world = World::new(0);
        world.insert(column);

        let mut miner = Miner::default();
        let mut broken = 0;
        for x in 1..=5 {
            let aim = Aim {
                x,
                y: 10,
                z: 1,
                face: 1,
                cursor: [0.5, 1.0, 0.5],
            };
            broken += miner
                .tick(&world, Some(aim), true, true)
                .iter()
                .filter(|event| matches!(event, MiningEvent::Stop(_)))
                .count();
        }
        assert_eq!(broken, 5, "a swept row must not wait out a shared cooldown");
    }

    #[test]
    fn creative_still_throttles_the_block_it_just_broke() {
        let world = world_with(BlockState::STONE);
        let mut miner = Miner::default();
        assert_eq!(miner.tick(&world, Some(aim()), true, true).len(), 2);

        // The same block (still there because the server has not confirmed it)
        // is never asked for twice in a row.
        for _ in 0..(HIT_DELAY_TICKS - 1) {
            assert!(miner.tick(&world, Some(aim()), true, true).is_empty());
        }
    }

    #[test]
    fn one_click_breaks_one_block_even_if_it_spans_several_ticks() {
        let world = two_blocks(BlockState::STONE);
        let mut miner = Miner::default();
        // The click is held for four ticks, as a normal one is.
        let mut broken = 0;
        for _ in 0..4 {
            broken += miner.tick(&world, Some(aim()), true, true).len() / 2;
        }
        assert_eq!(broken, 1, "a single click must not clear two blocks");
    }

    #[test]
    fn creative_does_not_repeat_a_block_the_server_has_not_removed() {
        let world = world_with(BlockState::STONE);
        let mut miner = Miner::default();
        assert_eq!(miner.tick(&world, Some(aim()), true, true).len(), 2);
        for _ in 0..5 {
            assert!(miner.tick(&world, Some(aim()), true, true).is_empty());
        }
    }

    #[test]
    fn creative_retries_after_the_pending_window() {
        let world = world_with(BlockState::STONE);
        let mut miner = Miner::default();
        assert_eq!(miner.tick(&world, Some(aim()), true, true).len(), 2);
        // The pending window and then the hit delay both have to run out.
        for _ in 0..(PENDING_TICKS + HIT_DELAY_TICKS) {
            miner.tick(&world, Some(aim()), true, true);
        }
        assert_eq!(
            miner.tick(&world, Some(aim()), true, true).len(),
            2,
            "a server that ignored the break should be asked again"
        );
    }

    #[test]
    fn unbreakable_blocks_never_start() {
        let world = world_with(BlockState::BEDROCK);
        let mut miner = Miner::default();
        assert!(miner.tick(&world, Some(aim()), true, true).is_empty());
        assert!(miner.tick(&world, Some(aim()), true, false).is_empty());
        assert!(miner.target().is_none());
    }
}

//! Entity targeting and click pacing.
//!
//! Vanilla gives a click on an entity priority over the block behind it
//! (`Minecraft.clickMouse` checks `objectMouseOver.typeOfHit == ENTITY` first),
//! and rate-limits use-clicks with `rightClickDelayTimer` (4 ticks). Both rules
//! live here so they are testable without a window.
//!
//! The packet itself is not built here: `C02 Use Entity` belongs to
//! `nephele-protocol` and the send path to `nephele-client`, which are the
//! `net` lane's files. This module decides *what* would be sent and *when*; the
//! caller collects the result from `Shared::entity_actions`.

use std::time::{Duration, Instant};

/// Half the width of a player's or a standard mob's hitbox (`0.6` blocks).
const HALF_WIDTH: f64 = 0.3;
/// A player's hitbox height (`EntityPlayer` is 1.8 tall).
const PLAYER_HEIGHT: f64 = 1.8;
/// A generic hostile/passive mob hitbox height. Mobs vary; 1.5 blocks covers
/// everything a 1.8 client can interact with except the tallest (iron golem,
/// which is 2.7) — close enough for a click target and never *larger* than the
/// real box, so we never click through a wall of hitbox.
const MOB_HEIGHT: f64 = 1.5;

/// The minimum gap between two left-click attacks.
///
/// 1.8 has no attack cooldown (that is 1.9's attack-strength meter), so this is
/// purely a debounce: it stops a held button or an OS key-repeat from turning
/// one swing into a stream of packets.
pub const ATTACK_COOLDOWN: Duration = Duration::from_millis(125);

/// Vanilla's `rightClickDelayTimer = 4` ticks.
pub const USE_COOLDOWN: Duration = Duration::from_millis(200);

/// A clickable entity, as the network thread knows it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub id: i32,
    /// Feet position (the entity's own Y is the bottom of its hitbox).
    pub feet: [f64; 3],
    pub player: bool,
}

impl Candidate {
    pub fn new(id: i32, feet: [f64; 3], player: bool) -> Self {
        Self { id, feet, player }
    }

    fn min(&self) -> [f64; 3] {
        [
            self.feet[0] - HALF_WIDTH,
            self.feet[1],
            self.feet[2] - HALF_WIDTH,
        ]
    }

    fn max(&self) -> [f64; 3] {
        let height = if self.player {
            PLAYER_HEIGHT
        } else {
            MOB_HEIGHT
        };
        [
            self.feet[0] + HALF_WIDTH,
            self.feet[1] + height,
            self.feet[2] + HALF_WIDTH,
        ]
    }
}

/// What a click would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityAction {
    /// `C02 Use Entity` with `INTERACT` (right click).
    Interact { entity: i32 },
    /// `C02 Use Entity` with `ATTACK` (left click).
    Attack { entity: i32 },
}

/// The nearest entity along the view ray, with the distance to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Target {
    pub id: i32,
    pub distance: f64,
}

/// Nearest entity whose hitbox the ray enters within `reach`.
pub fn pick(
    eye: [f64; 3],
    direction: [f64; 3],
    reach: f64,
    candidates: &[Candidate],
) -> Option<Target> {
    let mut best: Option<Target> = None;
    for candidate in candidates {
        let Some(distance) = ray_box(eye, direction, candidate.min(), candidate.max()) else {
            continue;
        };
        if distance > reach {
            continue;
        }
        if best.is_none_or(|current| distance < current.distance) {
            best = Some(Target {
                id: candidate.id,
                distance,
            });
        }
    }
    best
}

/// Slab test: the entry distance of `ray` into the axis-aligned box, if any.
fn ray_box(origin: [f64; 3], direction: [f64; 3], min: [f64; 3], max: [f64; 3]) -> Option<f64> {
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;
    for axis in 0..3 {
        let d = direction[axis];
        let o = origin[axis];
        if d.abs() < 1e-9 {
            // Parallel to this slab: a miss unless the origin is already inside.
            if o < min[axis] || o > max[axis] {
                return None;
            }
            continue;
        }
        let mut t0 = (min[axis] - o) / d;
        let mut t1 = (max[axis] - o) / d;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        near = near.max(t0);
        far = far.min(t1);
        if near > far {
            return None;
        }
    }
    if far < 0.0 {
        return None;
    }
    Some(near.max(0.0))
}

/// Two independent one-button timers.
#[derive(Debug, Clone, Default)]
pub struct ClickGate {
    last_attack: Option<Instant>,
    last_use: Option<Instant>,
}

impl ClickGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Would an attack be sent right now? Records the click when it is.
    pub fn attack(&mut self, now: Instant) -> bool {
        if !ready(self.last_attack, now, ATTACK_COOLDOWN) {
            return false;
        }
        self.last_attack = Some(now);
        true
    }

    /// Would a use-click be sent right now? Records the click when it is.
    pub fn use_item(&mut self, now: Instant) -> bool {
        if !ready(self.last_use, now, USE_COOLDOWN) {
            return false;
        }
        self.last_use = Some(now);
        true
    }
}

fn ready(last: Option<Instant>, now: Instant, cooldown: Duration) -> bool {
    match last {
        None => true,
        Some(last) => now.saturating_duration_since(last) >= cooldown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EYE: [f64; 3] = [0.5, 65.62, 0.5];

    fn candidate_at(distance: f64, player: bool) -> Candidate {
        Candidate::new(7, [0.5, EYE[1] - 1.62, 0.5 + distance], player)
    }

    #[test]
    fn a_ray_down_the_axis_hits_a_player_in_front() {
        let target = pick(EYE, [0.0, 0.0, 1.0], 4.5, &[candidate_at(2.0, true)]);
        let target = target.expect("straight ahead");
        assert_eq!(target.id, 7);
        assert!((target.distance - 2.0).abs() < 0.35);
    }

    #[test]
    fn the_nearest_entity_wins() {
        let far = Candidate::new(1, [0.5, 64.0, 4.5], true);
        let near = Candidate::new(2, [0.5, 64.0, 2.5], true);
        let target = pick(EYE, [0.0, 0.0, 1.0], 4.5, &[far, near]).unwrap();
        assert_eq!(target.id, 2);
    }

    #[test]
    fn a_mob_hitbox_is_shorter_than_a_players() {
        // Eye level clears a 1.5-block mob's box but not a 1.8-block player's.
        let at_eye = EYE[1] - 1.62;
        let mob = Candidate::new(5, [0.5, at_eye, 2.5], false);
        let player = Candidate::new(6, [0.5, at_eye, 2.5], true);
        assert!(pick(EYE, [0.0, 0.0, 1.0], 4.5, &[mob]).is_none());
        assert!(pick(EYE, [0.0, 0.0, 1.0], 4.5, &[player]).is_some());
    }

    #[test]
    fn nothing_beyond_reach_is_clickable() {
        assert!(pick(EYE, [0.0, 0.0, 1.0], 4.5, &[candidate_at(6.0, true)]).is_none());
    }

    #[test]
    fn looking_away_misses() {
        assert!(pick(EYE, [0.0, 0.0, -1.0], 4.5, &[candidate_at(2.0, true)]).is_none());
    }

    #[test]
    fn a_position_inside_the_hitbox_is_the_nearest_hit() {
        let inside = Candidate::new(3, [0.5, 64.0, 0.5], true);
        let target = pick(EYE, [0.0, 0.0, 1.0], 4.5, &[inside]).unwrap();
        assert_eq!(target.distance, 0.0);
    }

    #[test]
    fn attacks_are_debounced_but_use_clicks_wait_longer() {
        let mut gate = ClickGate::new();
        let start = Instant::now();
        assert!(gate.attack(start));
        assert!(!gate.attack(start + Duration::from_millis(50)));
        assert!(gate.attack(start + ATTACK_COOLDOWN));

        assert!(gate.use_item(start));
        assert!(!gate.use_item(start + ATTACK_COOLDOWN));
        assert!(gate.use_item(start + USE_COOLDOWN));
    }
}

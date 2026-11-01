//! Tracked remote entities.
//!
//! Positions arrive at whatever rate the server sends them; rendering runs far
//! faster. Like vanilla's `lastTickPos`, every entity keeps the position it had
//! at the start of the current client tick and the latest one received, and the
//! renderer blends between them by the fraction of a tick elapsed.

use std::collections::HashMap;

/// Blocks per second the server's "teleport" threshold uses. Vanilla treats a
/// move larger than 4 blocks as absolute and snaps.
const TELEPORT_DISTANCE: f64 = 4.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityKind {
    /// A real player: `S0C`, or a player-shaped mob (`MOB_TYPE_PLAYER`).
    Player { name: String, uuid: [u8; 16] },
    /// Any other mob, rendered as a plain box tinted by its type.
    Mob { kind: u8 },
    /// A dropped item stack (`S0E` with object type 2).
    Item { id: i16, meta: i16 },
}

#[derive(Debug, Clone)]
pub struct Entity {
    pub id: i32,
    pub kind: EntityKind,
    /// Interpolation start (last client tick) and latest server position.
    pub prev: [f64; 3],
    pub position: [f64; 3],
    /// Body yaw in Minecraft degrees (0 = +Z, increasing towards -X).
    pub yaw: f32,
    pub prev_yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub prev_head_yaw: f32,
    /// Blocks per tick, straight off `S12`. Nothing integrates it yet.
    pub velocity: [f64; 3],
    /// Client tick this entity spawned on.
    pub spawned_at: u64,
}

impl Entity {
    fn new(id: i32, kind: EntityKind, position: [f64; 3], yaw: f32, pitch: f32, tick: u64) -> Self {
        Self {
            id,
            kind,
            prev: position,
            position,
            yaw,
            prev_yaw: yaw,
            pitch,
            head_yaw: yaw,
            prev_head_yaw: yaw,
            velocity: [0.0; 3],
            spawned_at: tick,
        }
    }

    /// Position at `alpha` (0..1) through the current tick.
    pub fn interpolated(&self, alpha: f64) -> [f64; 3] {
        let alpha = alpha.clamp(0.0, 1.0);
        [
            self.prev[0] + (self.position[0] - self.prev[0]) * alpha,
            self.prev[1] + (self.position[1] - self.prev[1]) * alpha,
            self.prev[2] + (self.position[2] - self.prev[2]) * alpha,
        ]
    }

    /// Yaw at `alpha`, taking the short way round so a turn past ±180° doesn't
    /// spin the model the long way. Always normalised into `[0, 360)`.
    pub fn interpolated_yaw(&self, alpha: f64) -> f32 {
        let delta = wrap_degrees(self.yaw - self.prev_yaw);
        let angle = self.prev_yaw + delta * alpha.clamp(0.0, 1.0) as f32;
        angle.rem_euclid(360.0)
    }

    fn move_to(&mut self, dx: f64, dy: f64, dz: f64) {
        let next = [
            self.position[0] + dx,
            self.position[1] + dy,
            self.position[2] + dz,
        ];
        self.jump(next);
    }

    /// Absolute correction. A large jump snaps `prev` too, so the model doesn't
    /// slide across the world in one tick.
    fn jump(&mut self, position: [f64; 3]) {
        if distance(self.position, position) > TELEPORT_DISTANCE {
            self.prev = position;
        }
        self.position = position;
    }

    fn look(&mut self, yaw: f32, pitch: f32) {
        self.yaw = yaw;
        self.pitch = pitch;
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

pub fn wrap_degrees(degrees: f32) -> f32 {
    let mut wrapped = degrees % 360.0;
    if wrapped >= 180.0 {
        wrapped -= 360.0;
    }
    if wrapped < -180.0 {
        wrapped += 360.0;
    }
    wrapped
}

/// Every non-local entity the client knows about, keyed by entity id.
#[derive(Debug, Default, Clone)]
pub struct Entities {
    map: HashMap<i32, Entity>,
    /// Client tick counter, bumped by [`Entities::tick`].
    tick: u64,
}

impl Entities {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&self, id: i32) -> Option<&Entity> {
        self.map.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entity> {
        self.map.values()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// Snapshot every entity's current position as the interpolation start.
    /// Called once per client tick, before new movement packets are applied.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        for entity in self.map.values_mut() {
            entity.prev = entity.position;
            entity.prev_yaw = entity.yaw;
            entity.prev_head_yaw = entity.head_yaw;
        }
    }

    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    pub fn spawn_player(
        &mut self,
        id: i32,
        uuid: [u8; 16],
        name: String,
        position: [f64; 3],
        yaw: f32,
        pitch: f32,
    ) {
        self.map.insert(
            id,
            Entity::new(
                id,
                EntityKind::Player { name, uuid },
                position,
                yaw,
                pitch,
                self.tick,
            ),
        );
    }

    pub fn spawn_mob(
        &mut self,
        id: i32,
        kind: u8,
        player_shaped: bool,
        position: [f64; 3],
        yaw: f32,
        pitch: f32,
    ) {
        let kind = if player_shaped {
            EntityKind::Player {
                name: String::new(),
                uuid: [0; 16],
            }
        } else {
            EntityKind::Mob { kind }
        };
        self.map
            .insert(id, Entity::new(id, kind, position, yaw, pitch, self.tick));
    }

    /// Track a dropped item (`S0E`, object type 2).
    pub fn spawn_item(&mut self, id: i32, item: i16, meta: i16, position: [f64; 3]) {
        self.map.insert(
            id,
            Entity::new(
                id,
                EntityKind::Item { id: item, meta },
                position,
                0.0,
                0.0,
                self.tick,
            ),
        );
    }

    pub fn relative_move(&mut self, id: i32, dx: f64, dy: f64, dz: f64) {
        if let Some(entity) = self.map.get_mut(&id) {
            entity.move_to(dx, dy, dz);
        }
    }

    pub fn look(&mut self, id: i32, yaw: f32, pitch: f32) {
        if let Some(entity) = self.map.get_mut(&id) {
            entity.look(yaw, pitch);
            entity.head_yaw = yaw;
        }
    }

    pub fn relative_move_and_look(
        &mut self,
        id: i32,
        dx: f64,
        dy: f64,
        dz: f64,
        yaw: f32,
        pitch: f32,
    ) {
        if let Some(entity) = self.map.get_mut(&id) {
            entity.move_to(dx, dy, dz);
            entity.look(yaw, pitch);
            entity.head_yaw = yaw;
        }
    }

    pub fn teleport(&mut self, id: i32, position: [f64; 3], yaw: f32, pitch: f32) {
        if let Some(entity) = self.map.get_mut(&id) {
            entity.jump(position);
            entity.prev_yaw = yaw;
            entity.look(yaw, pitch);
            entity.head_yaw = yaw;
        }
    }

    pub fn head_look(&mut self, id: i32, yaw: f32) {
        if let Some(entity) = self.map.get_mut(&id) {
            entity.head_yaw = yaw;
        }
    }

    pub fn set_velocity(&mut self, id: i32, vx: f64, vy: f64, vz: f64) {
        if let Some(entity) = self.map.get_mut(&id) {
            entity.velocity = [vx, vy, vz];
        }
    }

    pub fn remove(&mut self, id: i32) {
        self.map.remove(&id);
    }

    pub fn despawn_all(&mut self, ids: &[i32]) {
        for id in ids {
            self.map.remove(id);
        }
    }

    /// Entity id whose position is closest to `point` within `max_distance`,
    /// used for attack target resolution later.
    pub fn nearest(&self, point: [f64; 3], max_distance: f64) -> Option<&Entity> {
        self.map
            .values()
            .map(|entity| (entity, distance(entity.position, point)))
            .filter(|(_, distance)| *distance <= max_distance)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(entity, _)| entity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_item_is_tracked_like_any_other_entity() {
        let mut entities = Entities::new();
        entities.spawn_item(9, 17, 2, [1.0, 64.0, 2.0]);

        let entity = entities.get(9).expect("item entity");
        assert_eq!(entity.kind, EntityKind::Item { id: 17, meta: 2 });
        assert_eq!(entity.position, [1.0, 64.0, 2.0]);

        entities.relative_move(9, 1.0, 0.0, 0.0);
        assert!((entities.get(9).unwrap().interpolated(0.5)[0] - 1.5).abs() < 1e-9);
    }

    #[test]
    fn relative_moves_accumulate_and_interpolate() {
        let mut entities = Entities::new();
        entities.spawn_player(1, [0; 16], "Steve".into(), [0.0, 64.0, 0.0], 0.0, 0.0);
        entities.relative_move(1, 0.5, 0.0, 0.0);
        entities.relative_move(1, 0.5, 0.0, 0.0);

        let entity = entities.get(1).unwrap();
        assert_eq!(entity.position[0], 1.0);
        // Not ticked yet, so prev is still the spawn point.
        assert_eq!(entity.interpolated(0.0)[0], 0.0);
        assert_eq!(entity.interpolated(1.0)[0], 1.0);
        assert!((entity.interpolated(0.5)[0] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn tick_moves_the_interpolation_start() {
        let mut entities = Entities::new();
        entities.spawn_player(1, [0; 16], "Steve".into(), [0.0, 64.0, 0.0], 0.0, 0.0);
        entities.relative_move(1, 1.0, 0.0, 0.0);
        entities.tick();
        // Now prev == position, so a second move blends from 1.0 to 2.0.
        entities.relative_move(1, 1.0, 0.0, 0.0);
        let entity = entities.get(1).unwrap();
        assert_eq!(entity.interpolated(0.0)[0], 1.0);
        assert_eq!(entity.interpolated(1.0)[0], 2.0);
    }

    #[test]
    fn long_teleports_snap_instead_of_sliding() {
        let mut entities = Entities::new();
        entities.spawn_player(1, [0; 16], "Steve".into(), [0.0, 64.0, 0.0], 0.0, 0.0);
        entities.teleport(1, [100.0, 64.0, 0.0], 90.0, 10.0);
        let entity = entities.get(1).unwrap();
        assert_eq!(entity.prev[0], 100.0);
        assert_eq!(entity.interpolated(0.0)[0], 100.0);
        // A short teleport still blends.
        entities.teleport(1, [101.0, 64.0, 0.0], 90.0, 10.0);
        assert_eq!(entities.get(1).unwrap().interpolated(0.0)[0], 100.0);
    }

    #[test]
    fn yaw_interpolates_the_short_way() {
        let mut entities = Entities::new();
        entities.spawn_player(1, [0; 16], "Steve".into(), [0.0; 3], 350.0, 0.0);
        entities.tick();
        entities.look(1, 10.0, 0.0);
        let entity = entities.get(1).unwrap();
        // 350 -> 10 is +20, not -340.
        assert!((entity.interpolated_yaw(0.5) - 0.0).abs() < 1e-4);
    }

    #[test]
    fn head_yaw_tracks_look_and_is_independently_settable() {
        let mut entities = Entities::new();
        entities.spawn_mob(4, 5, false, [0.0; 3], 0.0, 0.0);
        entities.head_look(4, 90.0);
        let entity = entities.get(4).unwrap();
        assert_eq!(entity.head_yaw, 90.0);
        // Body yaw untouched.
        assert_eq!(entity.yaw, 0.0);
        assert!(matches!(entity.kind, EntityKind::Mob { kind: 5 }));
    }

    #[test]
    fn player_shaped_mobs_are_tracked_as_players() {
        let mut entities = Entities::new();
        entities.spawn_mob(7, 30, true, [1.0, 2.0, 3.0], 0.0, 0.0);
        assert!(matches!(
            entities.get(7).unwrap().kind,
            EntityKind::Player { .. }
        ));
    }

    #[test]
    fn destroy_entities_removes_everything_listed() {
        let mut entities = Entities::new();
        for id in 1..=3 {
            entities.spawn_mob(id, 1, false, [0.0; 3], 0.0, 0.0);
        }
        entities.despawn_all(&[1, 3, 99]);
        assert_eq!(entities.len(), 1);
        assert!(entities.get(2).is_some());
    }

    #[test]
    fn moves_for_unknown_entities_are_ignored() {
        let mut entities = Entities::new();
        entities.relative_move(42, 1.0, 1.0, 1.0);
        entities.look(42, 10.0, 10.0);
        assert!(entities.is_empty());
    }

    #[test]
    fn nearest_finds_the_closest_within_range() {
        let mut entities = Entities::new();
        entities.spawn_player(1, [0; 16], "a".into(), [0.0; 3], 0.0, 0.0);
        entities.spawn_player(2, [0; 16], "b".into(), [2.0, 0.0, 0.0], 0.0, 0.0);
        assert_eq!(entities.nearest([1.9, 0.0, 0.0], 5.0).unwrap().id, 2);
        assert!(entities.nearest([40.0, 0.0, 0.0], 5.0).is_none());
    }
}

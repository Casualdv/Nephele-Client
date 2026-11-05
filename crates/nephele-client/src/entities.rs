//! Tracked remote entities.
//!
//! Positions arrive at whatever rate the server sends them; rendering runs far
//! faster. Like vanilla's `lastTickPos`, every entity keeps the position it had
//! at the start of the current client tick and the latest one received, and the
//! renderer blends between them by the fraction of a tick elapsed.

use std::collections::HashMap;

use nephele_protocol::packets::play::{
    equipment_slot, EntityMetadataUpdate, EntityMetadataValue, ItemStack,
};

/// Blocks per second the server's "teleport" threshold uses. Vanilla treats a
/// move larger than 4 blocks as absolute and snaps.
const TELEPORT_DISTANCE: f64 = 4.0;

/// The 1.8 `DataWatcher` fields shared by every living entity.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityMetadata {
    /// Entity flags: 0x01 fire, 0x02 crouched, 0x04 sprinting, 0x08 using,
    /// 0x20 invisible.
    pub flags: u8,
    pub air: i16,
    pub custom_name: String,
    pub name_visible: bool,
    pub silent: bool,
    pub health: f32,
}

impl Default for EntityMetadata {
    fn default() -> Self {
        Self {
            flags: 0,
            air: 300,
            custom_name: String::new(),
            name_visible: false,
            silent: false,
            health: 20.0,
        }
    }
}

impl EntityMetadata {
    pub fn on_fire(&self) -> bool {
        self.flags & 0x01 != 0
    }

    pub fn crouched(&self) -> bool {
        self.flags & 0x02 != 0
    }

    pub fn sprinting(&self) -> bool {
        self.flags & 0x08 != 0
    }

    pub fn invisible(&self) -> bool {
        self.flags & 0x20 != 0
    }
}

/// The plate drawn above an entity's head.
///
/// Only `name` is filled today: it is the server's `custom_name` when there is
/// one, and the player-list name otherwise. `prefix` (team colours) and `ping`
/// are the slots the `net` branch fills once it parses them; they live here so
/// the plate layout does not have to change when that lands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Nametag {
    /// Team prefix, e.g. `[RED] `. Empty until team data lands.
    pub prefix: String,
    /// `custom_name`, or the player-list name for a real player.
    pub name: String,
    /// Latency in milliseconds. `None` until the net branch supplies it.
    pub ping: Option<u16>,
}

impl Nametag {
    /// The plate as it is drawn. Multi-line custom names keep their line
    /// breaks, which is how ASCII-art holograms are authored. `§` formatting
    /// codes are dropped: the UI draws one colour per plate today. The net
    /// branch maps them to colours and this method grows a colour-aware
    /// variant then.
    pub fn plate(&self) -> String {
        let mut text = String::with_capacity(self.prefix.len() + self.name.len() + 8);
        text.push_str(&self.prefix);
        text.push_str(&strip_formatting(&self.name));
        if let Some(ping) = self.ping {
            text.push(' ');
            text.push_str(&ping.to_string());
            text.push_str("ms");
        }
        text
    }
}

/// Remove vanilla `§` formatting codes (`§6`, `§l`, ...), leaving the text.
fn strip_formatting(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == '§' {
            chars.next();
        } else {
            out.push(ch);
        }
    }
    out
}

/// The entity under the crosshair, as returned by [`Entities::pick`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityPick {
    pub id: i32,
    /// Distance from the eye, in blocks.
    pub distance: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityKind {
    /// A real player: `S0C`, or a player-shaped mob (`MOB_TYPE_PLAYER`).
    Player { name: String, uuid: [u8; 16] },
    /// Any other mob, rendered as a plain box tinted by its type.
    Mob { kind: u8 },
    /// A dropped item stack (`S0E` with object type 2).
    Item { id: i16, meta: i16 },
    /// An item frame (`S0E` with object type 71). `direction` is the face it is
    /// attached to: 0 down, 1 up, 2 north, 3 south, 4 west, 5 east. The framed
    /// stack and rotation arrive later as `S1C` metadata.
    ItemFrame {
        direction: u8,
        item: Option<ItemStack>,
        /// Rotation in 45-degree steps, 0..=7.
        rotation: u8,
    },
    /// An armour stand (`S0E` with object type 78). Status flags and the head
    /// pose arrive as metadata; an invisible marker stand is a hologram whose
    /// only visible part is its `custom_name`.
    ArmorStand {
        small: bool,
        /// Marker stands have no hitbox and are what holograms are built from.
        marker: bool,
        show_arms: bool,
    },
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
    /// DataWatcher state: flags, name, health, silence.
    pub metadata: EntityMetadata,
    /// Held item, boots, leggings, chestplate, helmet.
    pub equipment: [Option<ItemStack>; equipment_slot::COUNT],
    /// Last `S0B` animation id (0 = arm swing) and how many ticks it lasts.
    pub animation: u8,
    pub animation_ticks: u8,
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
            metadata: EntityMetadata::default(),
            equipment: [None; equipment_slot::COUNT],
            animation: 0,
            animation_ticks: 0,
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

    /// Yaw at `alpha`, taking the short way round so a turn past Â±180Â° doesn't
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

    /// Custom name, falling back to the player-list name for real players.
    pub fn name(&self) -> &str {
        if !self.metadata.custom_name.is_empty() {
            &self.metadata.custom_name
        } else if let EntityKind::Player { name, .. } = &self.kind {
            name
        } else {
            ""
        }
    }

    /// Whether a nametag should be drawn. Players always show their name;
    /// mobs only when the server marks the custom name visible.
    pub fn shows_name(&self) -> bool {
        self.nametag().is_some()
    }

    /// The plate to draw above this entity, or `None` when nothing should be
    /// shown.
    ///
    /// Vanilla hides an invisible entity's plate, but an armour stand is the
    /// exception: a hologram *is* its name, so an invisible marker stand still
    /// draws its label. That is the whole "CLICK TO PLAY" trick.
    pub fn nametag(&self) -> Option<Nametag> {
        if self.invisible() && !self.is_armor_stand() {
            return None;
        }
        if self.metadata.custom_name.is_empty() {
            let EntityKind::Player { name, .. } = &self.kind else {
                return None;
            };
            return (!name.is_empty()).then(|| Nametag {
                name: name.clone(),
                ..Nametag::default()
            });
        }
        self.metadata.name_visible.then(|| Nametag {
            name: self.metadata.custom_name.clone(),
            ..Nametag::default()
        })
    }

    /// Whether the plate should also carry a health line. Players read their
    /// own health off the HUD, so only other living entities get one.
    pub fn shows_health(&self) -> bool {
        !self.is_armor_stand() && !matches!(self.kind, EntityKind::Player { .. })
    }

    /// Height above the feet at which the plate is centred.
    pub fn label_height(&self) -> f32 {
        match &self.kind {
            EntityKind::Player { .. } => 2.05,
            EntityKind::ArmorStand { small, .. } => {
                if *small {
                    0.8
                } else {
                    1.55
                }
            }
            _ => 1.75,
        }
    }

    /// Axis-aligned bounds around the body, used for crosshair picking.
    /// Returns `(min, max)` in world space.
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let (half_width, height) = match &self.kind {
            EntityKind::Player { .. } => (0.32, 1.8),
            EntityKind::ArmorStand { small, .. } => {
                if *small {
                    (0.25, 0.99)
                } else {
                    (0.25, 1.99)
                }
            }
            EntityKind::Mob { kind } => match kind {
                50 => (0.32, 1.7),
                52 | 59 => (0.7, 0.9),
                55 | 62 => (0.51, 0.51),
                65 => (0.25, 0.45),
                90..=92 | 95 | 96 => (0.45, 0.85),
                _ => (0.32, 1.8),
            },
            EntityKind::Item { .. } => (0.13, 0.25),
            EntityKind::ItemFrame { .. } => (0.13, 0.13),
        };
        let [x, y, z] = self.position;
        (
            [x - half_width, y, z - half_width],
            [x + half_width, y + height, z + half_width],
        )
    }

    /// Whether this entity is an armour stand. Marker stands are invisible in
    /// vanilla but still pickable as holograms here.
    pub fn is_armor_stand(&self) -> bool {
        matches!(self.kind, EntityKind::ArmorStand { .. })
    }

    pub fn health(&self) -> f32 {
        self.metadata.health
    }

    pub fn invisible(&self) -> bool {
        self.metadata.invisible()
    }

    pub fn crouched(&self) -> bool {
        self.metadata.crouched()
    }

    pub fn apply_metadata_value(&mut self, index: u8, value: &EntityMetadataValue) {
        match (index, value) {
            (0, EntityMetadataValue::Byte(flags)) => self.metadata.flags = *flags as u8,
            (1, EntityMetadataValue::Short(air)) => self.metadata.air = *air,
            (2, EntityMetadataValue::String(name)) => self.metadata.custom_name.clone_from(name),
            (3, EntityMetadataValue::Byte(visible)) => self.metadata.name_visible = *visible != 0,
            (4, EntityMetadataValue::Byte(silent)) => self.metadata.silent = *silent != 0,
            // Index 6 is health for a living entity, but this server also uses
            // it for head yaw: a byte is a 1/256 turn, a `Rotation` is degrees.
            // A `Float` is health, as in vanilla.
            (6, EntityMetadataValue::Byte(turn)) => {
                self.head_yaw = *turn as f32 * (360.0 / 256.0);
            }
            (6, EntityMetadataValue::Rotation([_, yaw, _])) => self.head_yaw = *yaw,
            (6, EntityMetadataValue::Float(health)) => self.metadata.health = health.max(0.0),
            (8, EntityMetadataValue::Slot(item)) => {
                if let EntityKind::ItemFrame { item: framed, .. } = &mut self.kind {
                    *framed = *item;
                }
            }
            (9, EntityMetadataValue::Byte(rotation)) => {
                if let EntityKind::ItemFrame {
                    rotation: current, ..
                } = &mut self.kind
                {
                    *current = (*rotation).clamp(0, 7) as u8;
                }
            }
            // Armour-stand status: 0x01 small, 0x04 show arms, 0x08 no base
            // plate, 0x10 marker.
            (10, EntityMetadataValue::Byte(status)) => {
                if let EntityKind::ArmorStand {
                    small,
                    marker,
                    show_arms,
                } = &mut self.kind
                {
                    let status = *status as u8;
                    *small = status & 0x01 != 0;
                    *show_arms = status & 0x04 != 0;
                    *marker = status & 0x10 != 0;
                }
            }
            // The 1.8 armour-stand head pose, in degrees.
            (11, EntityMetadataValue::Rotation([_, yaw, _])) => self.head_yaw = *yaw,
            _ => {}
        }
    }

    pub fn set_equipment(&mut self, slot: i16, item: Option<ItemStack>) {
        if (0..equipment_slot::COUNT as i16).contains(&slot) {
            self.equipment[slot as usize] = item;
        }
    }

    pub fn play_animation(&mut self, animation: u8) {
        self.animation = animation;
        self.animation_ticks = 8;
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Distance along a unit `direction` from `origin` to the near face of an
/// axis-aligned box, using the slab method. `None` when the ray misses or the
/// box is entirely behind the origin.
fn ray_box(origin: [f64; 3], direction: [f64; 3], min: [f64; 3], max: [f64; 3]) -> Option<f64> {
    let mut near = f64::NEG_INFINITY;
    let mut far = f64::INFINITY;
    for axis in 0..3 {
        let o = origin[axis];
        let d = direction[axis];
        if d.abs() < 1e-9 {
            if o < min[axis] || o > max[axis] {
                return None;
            }
            continue;
        }
        let inverse = 1.0 / d;
        let mut t1 = (min[axis] - o) * inverse;
        let mut t2 = (max[axis] - o) * inverse;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        near = near.max(t1);
        far = far.min(t2);
        if near > far {
            return None;
        }
    }
    (far >= 0.0).then(|| near.max(0.0))
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
            entity.animation_ticks = entity.animation_ticks.saturating_sub(1);
            if entity.animation_ticks == 0 {
                entity.animation = 0;
            }
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

    /// Track an item frame (`S0E`, object type 71).
    pub fn spawn_item_frame(&mut self, id: i32, direction: u8, position: [f64; 3]) {
        self.map.insert(
            id,
            Entity::new(
                id,
                EntityKind::ItemFrame {
                    direction,
                    item: None,
                    rotation: 0,
                },
                position,
                0.0,
                0.0,
                self.tick,
            ),
        );
    }

    /// Apply the two metadata entries an item frame uses: index 8 is the framed
    /// stack and index 9 is its rotation. `Some(None)` empties the frame.
    pub fn set_item_frame(
        &mut self,
        id: i32,
        item: Option<Option<ItemStack>>,
        rotation: Option<u8>,
    ) {
        let Some(entity) = self.map.get_mut(&id) else {
            return;
        };
        let EntityKind::ItemFrame {
            item: framed,
            rotation: current,
            ..
        } = &mut entity.kind
        else {
            return;
        };
        if let Some(item) = item {
            *framed = item;
        }
        if let Some(rotation) = rotation {
            *current = rotation & 7;
        }
    }

    /// Track an armour stand (`S0E`, object type 78). The status flags and the
    /// head pose arrive as metadata afterwards, so it starts as a normal stand.
    pub fn spawn_armor_stand(&mut self, id: i32, position: [f64; 3]) {
        self.map.insert(
            id,
            Entity::new(
                id,
                EntityKind::ArmorStand {
                    small: false,
                    marker: false,
                    show_arms: false,
                },
                position,
                0.0,
                0.0,
                self.tick,
            ),
        );
    }

    /// The closest entity along `direction` from `eye`, within `max_distance`.
    ///
    /// This is the entity half of the crosshair: block raycasts answer "which
    /// block", and this answers "which player, mob or hologram". Invisible
    /// non-hologram entities are skipped, matching what the renderer draws.
    pub fn pick(
        &self,
        eye: [f64; 3],
        direction: [f64; 3],
        max_distance: f64,
    ) -> Option<EntityPick> {
        let mut best: Option<EntityPick> = None;
        for entity in self.map.values() {
            if entity.invisible() && !entity.is_armor_stand() {
                continue;
            }
            let (min, max) = entity.bounds();
            let Some(distance) = ray_box(eye, direction, min, max) else {
                continue;
            };
            if distance > max_distance {
                continue;
            }
            if best.is_none_or(|pick| distance < pick.distance as f64) {
                best = Some(EntityPick {
                    id: entity.id,
                    distance: distance as f32,
                });
            }
        }
        best
    }

    /// Apply one `S1C Entity Metadata` update.
    pub fn apply_metadata(&mut self, update: &EntityMetadataUpdate) {
        let Some(entity) = self.map.get_mut(&update.entity_id) else {
            return;
        };
        for (index, value) in &update.values {
            entity.apply_metadata_value(*index, value);
        }
    }

    /// Apply one `S04 Entity Equipment` update.
    pub fn apply_equipment(&mut self, id: i32, slot: i16, item: Option<ItemStack>) -> bool {
        let Some(entity) = self.map.get_mut(&id) else {
            return false;
        };
        entity.set_equipment(slot, item);
        true
    }

    /// Record an `S0B` animation on one entity.
    pub fn play_animation(&mut self, id: i32, animation: u8) -> bool {
        let Some(entity) = self.map.get_mut(&id) else {
            return false;
        };
        entity.play_animation(animation);
        true
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
    fn metadata_updates_flags_name_health_and_equipment() {
        let mut entities = Entities::new();
        entities.spawn_mob(9, 54, false, [0.0; 3], 0.0, 0.0);
        let update = EntityMetadataUpdate {
            entity_id: 9,
            values: vec![
                (0, EntityMetadataValue::Byte(0x22)),
                (2, EntityMetadataValue::String("Bob".into())),
                (3, EntityMetadataValue::Byte(1)),
                (6, EntityMetadataValue::Float(7.0)),
            ],
        };
        entities.apply_metadata(&update);
        entities.apply_equipment(9, equipment_slot::HELMET, Some(ItemStack::new(310, 1, 0)));
        let entity = entities.get(9).unwrap();
        assert!(entity.crouched());
        assert!(entity.invisible());
        assert_eq!(entity.name(), "Bob");
        // Vanilla hides an invisible mob's plate; only holograms keep theirs.
        assert!(!entity.shows_name());
        assert_eq!(entity.health(), 7.0);
        assert_eq!(
            entity.equipment[equipment_slot::HELMET as usize],
            Some(ItemStack::new(310, 1, 0))
        );
    }

    #[test]
    fn item_frame_metadata_sets_the_framed_stack_and_rotation() {
        let mut entities = Entities::new();
        entities.spawn_item_frame(42, 2, [8.0, 65.0, 5.0]);
        let update = EntityMetadataUpdate {
            entity_id: 42,
            values: vec![
                (
                    8,
                    EntityMetadataValue::Slot(Some(ItemStack::new(358, 1, 7))),
                ),
                (9, EntityMetadataValue::Byte(5)),
            ],
        };
        entities.apply_metadata(&update);
        let frame = entities.get(42).unwrap();
        let EntityKind::ItemFrame {
            direction,
            item,
            rotation,
        } = &frame.kind
        else {
            panic!("expected an item frame");
        };
        assert_eq!(*direction, 2);
        assert_eq!(item.unwrap().damage, 7);
        assert_eq!(*rotation, 5);
    }

    #[test]
    fn nearest_finds_the_closest_within_range() {
        let mut entities = Entities::new();
        entities.spawn_player(1, [0; 16], "a".into(), [0.0; 3], 0.0, 0.0);
        entities.spawn_player(2, [0; 16], "b".into(), [2.0, 0.0, 0.0], 0.0, 0.0);
        assert_eq!(entities.nearest([1.9, 0.0, 0.0], 5.0).unwrap().id, 2);
        assert!(entities.nearest([40.0, 0.0, 0.0], 5.0).is_none());
    }

    #[test]
    fn metadata_index_six_can_carry_head_yaw_without_moving_the_body() {
        let mut entities = Entities::new();
        entities.spawn_player(3, [0; 16], "Steve".into(), [0.0; 3], 0.0, 0.0);
        entities.apply_metadata(&EntityMetadataUpdate {
            entity_id: 3,
            values: vec![(6, EntityMetadataValue::Byte(64))],
        });
        let entity = entities.get(3).unwrap();
        // 64/256 of a turn is a right angle.
        assert!((entity.head_yaw - 90.0).abs() < 1e-4);
        assert_eq!(entity.yaw, 0.0);

        // A `Rotation` at the same index is already in degrees.
        entities.apply_metadata(&EntityMetadataUpdate {
            entity_id: 3,
            values: vec![(6, EntityMetadataValue::Rotation([0.0, 135.0, 0.0]))],
        });
        assert!((entities.get(3).unwrap().head_yaw - 135.0).abs() < 1e-4);

        // `Float` stays health, as in vanilla.
        entities.apply_metadata(&EntityMetadataUpdate {
            entity_id: 3,
            values: vec![(6, EntityMetadataValue::Float(12.0))],
        });
        assert_eq!(entities.get(3).unwrap().health(), 12.0);
    }

    #[test]
    fn an_invisible_marker_armour_stand_still_draws_its_plate() {
        let mut entities = Entities::new();
        entities.spawn_armor_stand(11, [0.0, 64.0, 0.0]);
        entities.apply_metadata(&EntityMetadataUpdate {
            entity_id: 11,
            values: vec![
                (0, EntityMetadataValue::Byte(0x20)),
                (
                    2,
                    EntityMetadataValue::String("§6§lCLICK TO PLAY\n§7§oJoin a game".into()),
                ),
                (3, EntityMetadataValue::Byte(1)),
                (10, EntityMetadataValue::Byte(0x10 | 0x01)),
                (11, EntityMetadataValue::Rotation([0.0, 90.0, 0.0])),
            ],
        });

        let entity = entities.get(11).unwrap();
        assert!(entity.invisible());
        assert!(entity.is_armor_stand());
        assert_eq!(entity.head_yaw, 90.0);
        let EntityKind::ArmorStand {
            small,
            marker,
            show_arms,
        } = entity.kind
        else {
            panic!("expected an armour stand");
        };
        assert!(small);
        assert!(marker);
        assert!(!show_arms);

        let tag = entity.nametag().expect("holograms keep their plate");
        assert_eq!(tag.plate(), "CLICK TO PLAY\nJoin a game");
    }

    #[test]
    fn an_invisible_player_hides_its_plate() {
        let mut entities = Entities::new();
        entities.spawn_player(12, [0; 16], "Steve".into(), [0.0; 3], 0.0, 0.0);
        assert!(entities.get(12).unwrap().nametag().is_some());
        entities.apply_metadata(&EntityMetadataUpdate {
            entity_id: 12,
            values: vec![(0, EntityMetadataValue::Byte(0x20))],
        });
        assert!(entities.get(12).unwrap().nametag().is_none());
    }

    #[test]
    fn the_plate_keeps_team_and_ping_slots_ready() {
        let tag = Nametag {
            prefix: "[RED] ".into(),
            name: "Steve".into(),
            ping: Some(42),
        };
        assert_eq!(tag.plate(), "[RED] Steve 42ms");
    }

    #[test]
    fn pick_returns_the_closest_visible_entity_along_the_crosshair() {
        let mut entities = Entities::new();
        entities.spawn_player(1, [0; 16], "a".into(), [0.0, 0.0, 3.0], 0.0, 0.0);
        entities.spawn_player(2, [0; 16], "b".into(), [0.0, 0.0, 6.0], 0.0, 0.0);
        let pick = entities
            .pick([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 8.0)
            .expect("something in front");
        assert_eq!(pick.id, 1);
        assert!((pick.distance - 2.68).abs() < 0.05);

        // Out of range misses entirely.
        assert!(entities
            .pick([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 2.0)
            .is_none());

        // An invisible player is not clickable, but an invisible hologram is.
        entities.apply_metadata(&EntityMetadataUpdate {
            entity_id: 1,
            values: vec![(0, EntityMetadataValue::Byte(0x20))],
        });
        assert_eq!(
            entities
                .pick([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 8.0)
                .unwrap()
                .id,
            2
        );
        entities.spawn_armor_stand(3, [0.0, 0.0, 1.0]);
        entities.apply_metadata(&EntityMetadataUpdate {
            entity_id: 3,
            values: vec![(0, EntityMetadataValue::Byte(0x20))],
        });
        assert_eq!(
            entities
                .pick([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], 8.0)
                .unwrap()
                .id,
            3
        );
    }
}

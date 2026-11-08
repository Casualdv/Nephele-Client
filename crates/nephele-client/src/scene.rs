//! The render-facing view of the game state.
//!
//! Rendering is not implemented here, but this is the shape the entity renderer
//! consumes: one borrowed [`SceneView`] per frame, with an [`EntityRender`] per
//! tracked entity and the [`MapState`]s an item frame can point at.
//!
//! Everything is borrowed from [`GameClient`], so a renderer can build its
//! draw list every frame without cloning entities, metadata or map colours.
//! Interpolation stays the renderer's job: [`EntityRender`] exposes both the
//! previous and current position/yaw plus the same helpers the physics loop
//! already uses.

use std::collections::HashMap;

use nephele_protocol::packets::play::{EntityMetadata, ItemStack};

use crate::entities::{Entities, Entity, EntityKind};
use crate::maps::{MapState, Maps};

/// Everything an entity renderer needs for one frame.
#[derive(Debug, Clone, Copy)]
pub struct SceneView<'a> {
    pub entities: &'a Entities,
    pub maps: &'a Maps,
    metadata: &'a HashMap<i32, EntityMetadata>,
}

impl<'a> SceneView<'a> {
    pub(crate) fn new(
        entities: &'a Entities,
        maps: &'a Maps,
        metadata: &'a HashMap<i32, EntityMetadata>,
    ) -> Self {
        Self {
            entities,
            maps,
            metadata,
        }
    }

    pub fn entities(&self) -> impl Iterator<Item = EntityRender<'a>> + 'a {
        // `SceneView` is `Copy`, so the iterator can own a copy and still hand
        // out borrows tied to the game state it came from.
        let view = *self;
        view.entities.iter().map(move |entity| view.render(entity))
    }

    pub fn entity(&self, id: i32) -> Option<EntityRender<'a>> {
        self.entities.get(id).map(|entity| self.render(entity))
    }

    pub fn map(&self, id: i32) -> Option<&'a MapState> {
        self.maps.get(id)
    }

    pub fn maps(&self) -> impl Iterator<Item = (&'a i32, &'a MapState)> {
        self.maps.iter()
    }

    /// The picture for a framed map.
    ///
    /// In 1.8 a filled map's damage value *is* its map id (`ItemMap` keys off
    /// `getMetadata()`), which is how a frame's item and its `S34` data are
    /// joined. Returns `None` for anything that is not a filled map.
    pub fn map_for_item(&self, item: ItemStack) -> Option<&'a MapState> {
        if item.id != nephele_protocol::packets::play::ITEM_FILLED_MAP {
            return None;
        }
        self.maps.get(item.damage as i32)
    }

    fn render(&self, entity: &'a Entity) -> EntityRender<'a> {
        let metadata = self.metadata.get(&entity.id);
        let kind = match &entity.kind {
            EntityKind::Player { name, uuid } => EntityRenderKind::Player {
                name,
                uuid: *uuid,
                skin_flags: metadata.and_then(EntityMetadata::skin_flags).unwrap_or(0),
            },
            EntityKind::Mob { kind } => EntityRenderKind::Mob { kind: *kind },
            EntityKind::Item { id, meta } => EntityRenderKind::Item {
                id: *id,
                meta: *meta,
            },
            EntityKind::ItemFrame {
                direction,
                item,
                rotation,
            } => EntityRenderKind::ItemFrame {
                direction: *direction,
                item: *item,
                rotation: *rotation,
            },
            EntityKind::ArmorStand {
                small,
                marker,
                show_arms,
            } => EntityRenderKind::ArmorStand {
                small: *small,
                marker: *marker,
                show_arms: *show_arms,
            },
        };
        EntityRender {
            id: entity.id,
            kind,
            position: entity.position,
            prev_position: entity.prev,
            yaw: entity.yaw,
            prev_yaw: entity.prev_yaw,
            pitch: entity.pitch,
            head_yaw: entity.head_yaw,
            prev_head_yaw: entity.prev_head_yaw,
            metadata,
            equipment: entity.equipment,
            animation: entity.animation,
            animation_ticks: entity.animation_ticks,
            health: entity.health,
        }
    }
}

/// One entity, ready to be drawn.
#[derive(Debug, Clone, Copy)]
pub struct EntityRender<'a> {
    pub id: i32,
    pub kind: EntityRenderKind<'a>,
    pub position: [f64; 3],
    pub prev_position: [f64; 3],
    pub yaw: f32,
    pub prev_yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub prev_head_yaw: f32,
    /// The last metadata seen for this entity, if the server sent any.
    pub metadata: Option<&'a EntityMetadata>,
    /// Held item, boots, leggings, chestplate, helmet (`S04`).
    pub equipment: [Option<ItemStack>; nephele_protocol::packets::play::equipment_slot::COUNT],
    /// Last `S0B` animation id (0 = arm swing) and its remaining ticks.
    pub animation: u8,
    pub animation_ticks: u8,
    /// Last health reported in metadata index 6.
    pub health: f32,
}

#[derive(Debug, Clone, Copy)]
pub enum EntityRenderKind<'a> {
    Player {
        name: &'a str,
        uuid: [u8; 16],
        /// Cape/jacket/sleeve/leg/hat visibility, from metadata index 10.
        skin_flags: u8,
    },
    Mob {
        kind: u8,
    },
    Item {
        id: i16,
        meta: i16,
    },
    ItemFrame {
        /// 0 = south, 1 = west, 2 = north, 3 = east.
        direction: u8,
        item: Option<ItemStack>,
        /// 45-degree steps, 0..=7.
        rotation: u8,
    },
    ArmorStand {
        small: bool,
        marker: bool,
        show_arms: bool,
    },
}

impl EntityRender<'_> {
    /// Position `alpha` (0..1) through the current tick.
    pub fn interpolated(&self, alpha: f64) -> [f64; 3] {
        let alpha = alpha.clamp(0.0, 1.0);
        [
            self.prev_position[0] + (self.position[0] - self.prev_position[0]) * alpha,
            self.prev_position[1] + (self.position[1] - self.prev_position[1]) * alpha,
            self.prev_position[2] + (self.position[2] - self.prev_position[2]) * alpha,
        ]
    }

    /// Yaw at `alpha`, the short way round, normalised into `[0, 360)`.
    pub fn interpolated_yaw(&self, alpha: f64) -> f32 {
        let delta = crate::entities::wrap_degrees(self.yaw - self.prev_yaw);
        (self.prev_yaw + delta * alpha.clamp(0.0, 1.0) as f32).rem_euclid(360.0)
    }

    /// A carried block state (`id << 4 | meta`), for mobs that hold one.
    pub fn block_state(&self) -> Option<u16> {
        self.metadata.and_then(EntityMetadata::block_state)
    }

    pub fn custom_name(&self) -> Option<&str> {
        self.metadata.and_then(|metadata| {
            metadata.string(nephele_protocol::packets::play::metadata_index::CUSTOM_NAME)
        })
    }

    /// The framed stack, for item frames.
    pub fn framed_item(&self) -> Option<ItemStack> {
        match self.kind {
            EntityRenderKind::ItemFrame { item, .. } => item,
            _ => None,
        }
    }

    /// Skin flags, for players.
    pub fn skin_flags(&self) -> Option<u8> {
        match self.kind {
            EntityRenderKind::Player { skin_flags, .. } => Some(skin_flags),
            _ => None,
        }
    }

    pub fn is_player(&self) -> bool {
        matches!(self.kind, EntityRenderKind::Player { .. })
    }

    pub fn is_armor_stand(&self) -> bool {
        matches!(self.kind, EntityRenderKind::ArmorStand { .. })
    }

    /// The display name before team decoration: a player's list name, or a
    /// mob/hologram's `custom_name`.
    pub fn name(&self) -> Option<&str> {
        match self.kind {
            EntityRenderKind::Player { name, .. } => Some(name),
            _ => self.custom_name(),
        }
    }

    fn flag(&self, mask: u8) -> bool {
        self.metadata
            .and_then(|metadata| metadata.byte(0))
            .is_some_and(|flags| flags as u8 & mask != 0)
    }

    pub fn invisible(&self) -> bool {
        self.flag(0x20)
    }

    pub fn crouched(&self) -> bool {
        self.flag(0x02)
    }

    /// Metadata index 3: whether a custom name is drawn. Players always show
    /// their list name; this only gates mobs and armour-stand holograms.
    pub fn custom_name_visible(&self) -> bool {
        self.metadata
            .and_then(|metadata| metadata.byte(3))
            .is_some_and(|visible| visible != 0)
    }

    /// Height above the feet at which the plate is centred.
    pub fn label_height(&self) -> f32 {
        match self.kind {
            EntityRenderKind::Player { .. } => 2.05,
            EntityRenderKind::ArmorStand { small, .. } => {
                if small {
                    0.8
                } else {
                    1.55
                }
            }
            _ => 1.75,
        }
    }

    /// Whether the plate carries a health line. Players read their own health
    /// off the HUD; holograms have no health.
    pub fn shows_health(&self) -> bool {
        !self.is_armor_stand() && !self.is_player()
    }

    pub fn held_item(&self) -> Option<ItemStack> {
        self.equipment[nephele_protocol::packets::play::equipment_slot::HELD as usize]
    }

    /// Boots, leggings, chestplate, helmet, in the order the renderer expects.
    pub fn armor(&self) -> [Option<ItemStack>; 4] {
        use nephele_protocol::packets::play::equipment_slot;
        [
            self.equipment[equipment_slot::BOOTS as usize],
            self.equipment[equipment_slot::LEGGINGS as usize],
            self.equipment[equipment_slot::CHESTPLATE as usize],
            self.equipment[equipment_slot::HELMET as usize],
        ]
    }

    pub fn item_frame(&self) -> Option<(u8, u8, Option<ItemStack>)> {
        match self.kind {
            EntityRenderKind::ItemFrame {
                direction,
                item,
                rotation,
            } => Some((direction, rotation, item)),
            _ => None,
        }
    }
}

/// The plate drawn above an entity's head.
///
/// `prefix`/`suffix` are team formatting and `ping` is the tab-list latency;
/// both are filled by `GameClient::nametag`. `raw_plate` keeps `§` codes for a
/// colour-aware renderer, while `plate` is the plain-text form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Nametag {
    pub prefix: String,
    pub name: String,
    pub suffix: String,
    pub ping: Option<u16>,
}

impl Nametag {
    pub fn raw_plate(&self) -> String {
        let mut text =
            String::with_capacity(self.prefix.len() + self.name.len() + self.suffix.len() + 16);
        text.push_str(&self.prefix);
        text.push_str(&self.name);
        text.push_str(&self.suffix);
        if let Some(ping) = self.ping {
            text.push_str(" §7");
            text.push_str(&ping.to_string());
            text.push_str("ms");
        }
        text
    }

    pub fn plate(&self) -> String {
        strip_formatting(&self.raw_plate())
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use nephele_protocol::packets::play::{MapIcon, MapUpdate, MetadataValue};

    fn scene<'a>(
        entities: &'a Entities,
        maps: &'a Maps,
        metadata: &'a HashMap<i32, EntityMetadata>,
    ) -> SceneView<'a> {
        SceneView::new(entities, maps, metadata)
    }

    #[test]
    fn a_player_carries_its_skin_flags() {
        let mut entities = Entities::new();
        entities.spawn_player(1, [7; 16], "Steve".into(), [0.0; 3], 90.0, 0.0);
        let mut metadata = HashMap::new();
        let mut entry = EntityMetadata::new();
        entry.insert(10, MetadataValue::Byte(0b0110_0011));
        metadata.insert(1, entry);

        let entities_ref = &entities;
        let maps = Maps::new();
        let view = scene(entities_ref, &maps, &metadata);
        let player = view.entity(1).expect("player");
        assert_eq!(player.skin_flags(), Some(0b0110_0011));
        match player.kind {
            EntityRenderKind::Player {
                name, skin_flags, ..
            } => {
                assert_eq!(name, "Steve");
                assert_eq!(skin_flags, 0b0110_0011);
            }
            other => panic!("not a player: {other:?}"),
        }
    }

    #[test]
    fn a_frame_item_resolves_to_its_map_picture() {
        let mut entities = Entities::new();
        entities.spawn_item_frame(9, 3, [10.0, 64.0, -2.0]);
        entities.set_item_frame(9, Some(Some(ItemStack::new(358, 1, 41))), Some(5));

        let mut maps = Maps::new();
        maps.apply(&MapUpdate {
            map_id: 41,
            scale: 1,
            icons: vec![MapIcon {
                kind: 1,
                direction: 2,
                x: 3,
                z: 4,
            }],
            columns: 1,
            rows: 1,
            start_x: 0,
            start_y: 0,
            colors: vec![0x2A],
        });

        let metadata = HashMap::new();
        let view = scene(&entities, &maps, &metadata);
        let frame = view.entity(9).expect("frame");
        let EntityRenderKind::ItemFrame {
            direction,
            item,
            rotation,
        } = frame.kind
        else {
            panic!("not a frame");
        };
        assert_eq!((direction, rotation), (3, 5));
        let item = item.expect("framed map");
        assert_eq!(
            view.map_for_item(item).expect("map picture").color_at(0, 0),
            Some(0x2A)
        );
        // A non-map item has no picture.
        assert!(view.map_for_item(ItemStack::new(345, 1, 0)).is_none());
    }

    #[test]
    fn mob_block_state_and_custom_name_are_reachable() {
        let mut entities = Entities::new();
        entities.spawn_mob(4, 58, false, [0.0; 3], 0.0, 0.0);
        let mut entry = EntityMetadata::new();
        entry.insert(2, MetadataValue::String("Bob".into()));
        entry.insert(16, MetadataValue::Int((4 << 4) | 2));
        let mut metadata = HashMap::new();
        metadata.insert(4, entry);

        let maps = Maps::new();
        let view = scene(&entities, &maps, &metadata);
        let mob = view.entity(4).expect("mob");
        assert_eq!(mob.custom_name(), Some("Bob"));
        assert_eq!(mob.block_state(), Some((4 << 4) | 2));
        assert_eq!(mob.framed_item(), None);
    }
}

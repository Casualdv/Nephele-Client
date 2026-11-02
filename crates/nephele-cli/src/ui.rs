//! HUD, hotbar, pause menu and creative inventory.
//!
//! Everything is laid out in vanilla GUI pixels (320x240-ish) and multiplied by
//! an integer [`gui_scale`] so the HUD keeps its proportions on any window size.
//!
//! Drawing is split into three painter's layers: `bg` (panels, bars, slot
//! backgrounds), `icons` (isometric block icons, which need the texture array)
//! and `fg` (text and highlights). The renderer draws them in that order.

use std::sync::OnceLock;

use crate::servers::{Ping, PingBoard, ServerList};
use crate::settings::{SettingId, Settings};
use nephele_assets::{face_textures, font, sprites, TextureArray, MISSING_TEXTURE};
use nephele_protocol::packets::play::{ItemStack, HOTBAR_FIRST_SLOT, HOTBAR_SLOTS};
use nephele_render::{Rect, UiBatch};
use nephele_world::BlockState;

const TEXT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const TEXT_DIM: [f32; 4] = [0.6, 0.6, 0.6, 1.0];
const SHADOW_PANEL: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const BUTTON_BORDER: [f32; 4] = [0.05, 0.05, 0.05, 1.0];
const PANEL: [f32; 4] = [0.16, 0.16, 0.16, 0.96];
const SLOT_BG: [f32; 4] = [0.35, 0.35, 0.35, 1.0];
const SLOT_HOVER: [f32; 4] = [0.55, 0.55, 0.55, 1.0];
const ROW: [f32; 4] = [0.18, 0.18, 0.18, 1.0];
const ROW_ALT: [f32; 4] = [0.15, 0.15, 0.15, 1.0];
const ROW_HOVER: [f32; 4] = [0.30, 0.30, 0.30, 1.0];
const ROW_SELECTED: [f32; 4] = [0.20, 0.30, 0.45, 1.0];
const SELECTION_BORDER: [f32; 4] = [0.55, 0.70, 0.95, 1.0];
const STAR: [f32; 4] = [1.0, 0.85, 0.25, 1.0];
const REMOVE_HOVER: [f32; 4] = [1.0, 0.4, 0.4, 1.0];
const MOTD: [f32; 4] = [0.75, 0.75, 0.75, 1.0];
const OFFLINE: [f32; 4] = [0.95, 0.42, 0.42, 1.0];
const PING_GOOD: [f32; 4] = [0.45, 0.95, 0.45, 1.0];
const PING_WARN: [f32; 4] = [0.95, 0.85, 0.30, 1.0];
const PING_BAD: [f32; 4] = [0.95, 0.42, 0.42, 1.0];

/// Which screen is on top of the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Title,
    /// The saved-server list and its address field.
    Servers,
    Connect,
    Game,
    Pause,
    Settings,
    Creative,
    /// The survival player inventory (2x2 crafting, armour, storage, hotbar).
    Inventory,
}

/// A clickable thing the UI drew this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Singleplayer,
    Multiplayer,
    TitleOptions,
    Quit,
    /// Join whichever saved entry is selected.
    ServersJoin,
    ServersDirectConnect,
    ServersRefresh,
    ServersBack,
    /// Select a saved entry, by storage index.
    ServerSelect(usize),
    ServerFavourite(usize),
    ServerRemove(usize),
    ConnectSubmit,
    ConnectCancel,
    Resume,
    OpenSettings,
    SettingsDone,
    Disconnect,
    SettingAdjust(SettingId, i32),
    CreativePage(i32),
    CreativeItem(usize),
    SelectSlot(usize),
    /// A click on a player-container slot (0..=44).
    InventorySlot(usize),
}

#[derive(Debug, Clone, Copy)]
pub struct Button {
    pub action: Action,
    pub rect: Rect,
}

/// Everything the HUD needs from the game.
pub struct HudState<'a> {
    pub inventory: &'a [Option<ItemStack>],
    pub held_slot: i16,
    pub health: f32,
    pub food: i32,
    pub xp_bar: f32,
    pub xp_level: i32,
    pub fps: f32,
    pub position: [f64; 3],
    pub chunks: usize,
    pub creative: bool,
    pub creative_page: usize,
    pub pointer: Option<[f32; 2]>,
    /// Whether the mouse is captured (playing) rather than free (in a menu).
    pub captured: bool,
    /// The chat line is open, so the "click to look" hint would be wrong.
    pub chatting: bool,
}

/// Vanilla's automatic GUI scale: integers only so the pixel font stays crisp.
pub fn gui_scale(width: f32, height: f32) -> f32 {
    ((height / 240.0).min(width / 320.0))
        .floor()
        .clamp(1.0, 4.0)
}

pub const CREATIVE_COLUMNS: usize = 9;
pub const CREATIVE_ROWS: usize = 5;
pub const CREATIVE_SLOTS: usize = CREATIVE_COLUMNS * CREATIVE_ROWS;

/// Number of creative pages for the item list.
pub fn creative_pages() -> usize {
    creative_items().len().div_ceil(CREATIVE_SLOTS)
}

/// Every block the creative inventory offers, in registration order.
pub fn creative_items() -> &'static [BlockState] {
    static ITEMS: OnceLock<Vec<BlockState>> = OnceLock::new();
    ITEMS.get_or_init(|| {
        let mut items = Vec::new();
        for id in 1..=197u16 {
            let variants: &[u8] = match id {
                1 => &[0, 1, 2, 3, 4, 5, 6],
                5 => &[0, 1, 2, 3, 4, 5],
                17 => &[0, 1, 2, 3],
                18 => &[0, 1, 2, 3],
                24 => &[0, 1, 2],
                35 | 95 | 159 => &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                44 | 126 | 182 => &[0, 1, 2, 3, 4, 5],
                98 => &[0, 1, 2, 3],
                155 => &[0, 1, 2],
                162 => &[0, 1],
                179 => &[0, 1, 2],
                _ => &[0],
            };
            // Skip things that cannot be seen once placed.
            if matches!(id, 8 | 9 | 10 | 11 | 34 | 51 | 90 | 119 | 166) {
                continue;
            }
            for meta in variants {
                let state = BlockState::new(id, *meta);
                // Only offer blocks we can actually draw; the rest have no
                // texture mapping yet and would show the missing-texture tile.
                if let Some(faces) = face_textures(state) {
                    if faces.side != MISSING_TEXTURE {
                        items.push(state);
                    }
                }
            }
        }
        items
    })
}

/// Textures used to draw a block icon.
fn icon_layers(state: BlockState, textures: &TextureArray) -> Option<(u32, u32)> {
    let faces = face_textures(state)?;
    Some((
        textures.layer_or_missing(faces.top),
        textures.layer_or_missing(faces.side),
    ))
}

fn hovered(pointer: Option<[f32; 2]>, rect: &Rect) -> bool {
    pointer.map(|[x, y]| rect.contains(x, y)).unwrap_or(false)
}

/// Draw the in-game HUD: crosshair, hotbar, health, food, XP and a debug line.
#[allow(clippy::too_many_arguments)]
pub fn draw_hud(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    icons: &mut UiBatch,
    hud: &HudState<'_>,
    textures: &TextureArray,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let s = |v: f32| v * scale;
    let cx = width / 2.0;
    let mut buttons = Vec::new();

    // --- hotbar ------------------------------------------------------------
    let hotbar = Rect::new(cx - s(91.0), height - s(22.0), s(182.0), s(22.0));
    bg.sprite(sprites::HOTBAR, hotbar, TEXT);

    for slot in 0..HOTBAR_SLOTS {
        let slot_rect = Rect::new(hotbar.x + slot as f32 * s(20.0), hotbar.y, s(20.0), s(22.0));
        buttons.push(Button {
            action: Action::SelectSlot(slot),
            rect: slot_rect,
        });

        let item_rect = Rect::new(slot_rect.x + s(3.0), hotbar.y + s(3.0), s(16.0), s(16.0));
        if let Some(item) = hud
            .inventory
            .get(HOTBAR_FIRST_SLOT + slot)
            .copied()
            .flatten()
        {
            if let Some(state) = item.block_state() {
                if let Some((top, side)) = icon_layers(BlockState::from_raw(state), textures) {
                    icons.block_icon(item_rect.x, item_rect.y, item_rect.w, top, side, 1.0);
                }
            }
            if item.count > 1 {
                let label = item.count.to_string();
                let text_w = UiBatch::text_width(&label, scale);
                fg.text(
                    &label,
                    item_rect.x + item_rect.w + s(1.0) - text_w,
                    item_rect.y + item_rect.h - s(7.0),
                    scale,
                    TEXT,
                );
            }
        }
    }

    // Selection frame goes on top of the items, like vanilla.
    let selected_x = hotbar.x - s(1.0) + hud.held_slot.max(0) as f32 * s(20.0);
    fg.sprite(
        sprites::HOTBAR_SELECTION,
        Rect::new(selected_x, hotbar.y - s(1.0), s(24.0), s(22.0)),
        TEXT,
    );

    // --- health ------------------------------------------------------------
    let left = cx - s(91.0);
    let health_y = height - s(39.0);
    for i in 0..10 {
        let x = left + i as f32 * s(8.0);
        bg.sprite(
            sprites::HEART_BACKGROUND,
            Rect::new(x, health_y, s(9.0), s(9.0)),
            TEXT,
        );
        let filled = hud.health - i as f32 * 2.0;
        let overlay = if filled >= 2.0 {
            sprites::HEART_FULL
        } else if filled > 0.0 {
            sprites::HEART_HALF
        } else {
            continue;
        };
        bg.sprite(overlay, Rect::new(x, health_y, s(9.0), s(9.0)), TEXT);
    }

    // --- food (hunger), mirrored to the right ------------------------------
    let right = cx + s(91.0);
    for i in 0..10 {
        let x = right - s(9.0) - i as f32 * s(8.0);
        bg.sprite(
            sprites::FOOD_BACKGROUND,
            Rect::new(x, health_y, s(9.0), s(9.0)),
            TEXT,
        );
        let filled = hud.food - i * 2;
        let overlay = if filled >= 2 {
            sprites::FOOD_FULL
        } else if filled > 0 {
            sprites::FOOD_HALF
        } else {
            continue;
        };
        bg.sprite(overlay, Rect::new(x, health_y, s(9.0), s(9.0)), TEXT);
    }

    // --- experience --------------------------------------------------------
    let xp = Rect::new(cx - s(91.0), height - s(29.0), s(182.0), s(5.0));
    bg.sprite(sprites::XP_BAR_BACKGROUND, xp, TEXT);
    bg.sprite_cropped(sprites::XP_BAR_FILL, xp, hud.xp_bar);
    if hud.xp_level > 0 {
        fg.text_centered(
            &hud.xp_level.to_string(),
            cx,
            height - s(34.0),
            scale,
            [0.55, 0.95, 0.3, 1.0],
        );
    }

    // --- debug line and crosshair -----------------------------------------
    let debug = format!(
        "Nephele 1.8.9  {} fps  xyz {:.1} {:.1} {:.1}  chunks {}{}",
        hud.fps as i32,
        hud.position[0],
        hud.position[1],
        hud.position[2],
        hud.chunks,
        if hud.creative { "  [creative]" } else { "" }
    );
    fg.text(&debug, s(4.0), s(4.0), scale, TEXT);
    fg.crosshair(cx, height / 2.0, scale);
    if !hud.captured && !hud.chatting {
        fg.text_centered(
            "Click to look around  (Esc for the menu)",
            cx,
            height * 0.62,
            scale,
            TEXT,
        );
    }

    buttons
}

/// Recent chat in the bottom-left, and the input line while it is open.
///
/// `lines` carries `(text, alpha)` pairs so the caller owns the fade timing;
/// the input line sits just above the hotbar so the two never overlap.
pub fn draw_chat(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    lines: &[(String, f32)],
    input: Option<&str>,
    scale: f32,
    width: f32,
    height: f32,
) {
    let s = |v: f32| v * scale;
    let line_h = s(9.0);

    if let Some(input) = input {
        let strip = Rect::new(0.0, height - s(34.0), width, s(12.0));
        bg.rect(strip, [0.0, 0.0, 0.0, 0.62]);
        let text = format!("> {input}");
        fg.text(&text, s(4.0), height - s(31.0), scale, TEXT);
        let caret_x = s(4.0) + UiBatch::text_width(&text, scale) + s(1.0);
        bg.rect(Rect::new(caret_x, height - s(31.0), s(5.0), s(8.0)), TEXT);
    }

    if lines.is_empty() {
        return;
    }
    let bottom = if input.is_some() {
        height - s(36.0)
    } else {
        height - s(24.0)
    };
    for (index, (text, alpha)) in lines.iter().rev().enumerate() {
        let y = bottom - (index as f32 + 1.0) * line_h;
        if y < 0.0 {
            break;
        }
        let text_w = UiBatch::text_width(text, scale);
        bg.rect(
            Rect::new(s(2.0), y - s(1.0), text_w + s(4.0), line_h),
            [0.0, 0.0, 0.0, 0.35 * alpha],
        );
        fg.text(text, s(4.0), y, scale, [TEXT[0], TEXT[1], TEXT[2], *alpha]);
    }
}

fn draw_button(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    rect: Rect,
    label: &str,
    scale: f32,
    hover: bool,
) {
    draw_button_state(bg, fg, rect, label, scale, hover, true);
}

/// A vanilla button, greyed out when `enabled` is false.
#[allow(clippy::too_many_arguments)]
fn draw_button_state(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    rect: Rect,
    label: &str,
    scale: f32,
    hover: bool,
    enabled: bool,
) {
    // Vanilla 1.8 has no disabled strip: a disabled button keeps the normal
    // face and greys its label. (`BUTTON_DISABLED` in this atlas overlaps the
    // hotbar-selection art, so drawing it would show stray pixels.)
    let sprite = if hover {
        sprites::BUTTON_HOVER
    } else {
        sprites::BUTTON
    };
    bg.sprite(sprite, rect, TEXT);
    fg.text_centered(
        label,
        rect.x + rect.w / 2.0,
        rect.y + (rect.h - 8.0 * scale) / 2.0,
        scale,
        if enabled { TEXT } else { TEXT_DIM },
    );
}

/// The pause menu: title, "Back to Game", "Disconnect".
pub fn draw_pause(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    hud: &HudState<'_>,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    bg.rect(Rect::new(0.0, 0.0, width, height), SHADOW_PANEL);
    let cx = width / 2.0;

    fg.text_centered("Game Menu", cx, height * 0.2, scale * 2.0, TEXT);

    let w = 200.0 * scale;
    let h = 20.0 * scale;
    let top = height * 0.38;
    let resume = Rect::new(cx - w / 2.0, top, w, h);
    let settings = Rect::new(cx - w / 2.0, top + 24.0 * scale, w, h);
    let disconnect = Rect::new(cx - w / 2.0, top + 48.0 * scale, w, h);

    draw_button(
        bg,
        fg,
        resume,
        "Back to Game",
        scale,
        hovered(hud.pointer, &resume),
    );
    draw_button(
        bg,
        fg,
        settings,
        "Settings...",
        scale,
        hovered(hud.pointer, &settings),
    );
    draw_button(
        bg,
        fg,
        disconnect,
        "Disconnect",
        scale,
        hovered(hud.pointer, &disconnect),
    );

    let hint = if hud.creative {
        "Esc: resume   E: creative inventory"
    } else {
        "Esc: resume"
    };
    fg.text_centered(hint, cx, height - 24.0 * scale, scale, TEXT_DIM);

    vec![
        Button {
            action: Action::Resume,
            rect: resume,
        },
        Button {
            action: Action::OpenSettings,
            rect: settings,
        },
        Button {
            action: Action::Disconnect,
            rect: disconnect,
        },
    ]
}

/// Settings screen: one row per setting with `<` / `>` nudge buttons.
pub fn draw_settings(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    hud: &HudState<'_>,
    settings: &Settings,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let s = |v: f32| v * scale;
    bg.rect(Rect::new(0.0, 0.0, width, height), SHADOW_PANEL);
    let cx = width / 2.0;

    let rows = SettingId::ALL.len() as f32;
    let panel_w = s(320.0);
    let panel_h = s(30.0 + rows * 22.0 + 30.0);
    let panel = Rect::new(
        cx - panel_w / 2.0,
        height / 2.0 - panel_h / 2.0,
        panel_w,
        panel_h,
    );
    bg.rect(panel, PANEL);
    bg.rect_outline(panel, scale.max(1.0), BUTTON_BORDER);

    fg.text_centered("Settings", cx, panel.y + s(8.0), scale, TEXT);

    let label_x = panel.x + s(12.0);
    let mut buttons = Vec::new();
    for (index, id) in SettingId::ALL.iter().enumerate() {
        let y = panel.y + s(28.0) + index as f32 * s(22.0);
        fg.text(id.label(), label_x, y + s(3.0), scale, TEXT);

        let value = settings.value_label(*id);
        fg.text_centered(
            &value,
            panel.x + panel_w - s(60.0),
            y + s(3.0),
            scale,
            [1.0, 0.86, 0.35, 1.0],
        );

        let prev = Rect::new(panel.x + panel_w - s(112.0), y, s(18.0), s(16.0));
        let next = Rect::new(panel.x + panel_w - s(26.0), y, s(18.0), s(16.0));
        draw_button(
            bg,
            fg,
            prev,
            "<",
            scale,
            hovered(hud.pointer, &prev) && id.can_adjust(settings, -1),
        );
        draw_button(
            bg,
            fg,
            next,
            ">",
            scale,
            hovered(hud.pointer, &next) && id.can_adjust(settings, 1),
        );
        buttons.push(Button {
            action: Action::SettingAdjust(*id, -1),
            rect: prev,
        });
        buttons.push(Button {
            action: Action::SettingAdjust(*id, 1),
            rect: next,
        });
    }

    let done = Rect::new(cx - s(60.0), panel.y + panel_h - s(26.0), s(120.0), s(20.0));
    draw_button(bg, fg, done, "Done", scale, hovered(hud.pointer, &done));
    buttons.push(Button {
        action: Action::SettingsDone,
        rect: done,
    });

    buttons
}

/// The creative inventory: a paged grid of every block.
#[allow(clippy::too_many_arguments)]
pub fn draw_creative(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    icons: &mut UiBatch,
    hud: &HudState<'_>,
    textures: &TextureArray,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let s = |v: f32| v * scale;
    let items = creative_items();
    let pages = creative_pages().max(1);
    let page = hud.creative_page.min(pages - 1);
    let cx = width / 2.0;

    bg.rect(Rect::new(0.0, 0.0, width, height), SHADOW_PANEL);

    let panel_w = s(CREATIVE_COLUMNS as f32 * 20.0 + 8.0);
    let panel_h = s(CREATIVE_ROWS as f32 * 20.0 + 30.0);
    let panel = Rect::new(
        cx - panel_w / 2.0,
        height / 2.0 - panel_h / 2.0,
        panel_w,
        panel_h,
    );
    bg.rect(panel, PANEL);
    bg.rect_outline(panel, scale.max(1.0), BUTTON_BORDER);

    let grid_x = panel.x + s(4.0);
    let grid_y = panel.y + s(20.0);
    let mut buttons = Vec::new();

    for slot in 0..CREATIVE_SLOTS {
        let index = page * CREATIVE_SLOTS + slot;
        let col = slot % CREATIVE_COLUMNS;
        let row = slot / CREATIVE_COLUMNS;
        let rect = Rect::new(
            grid_x + col as f32 * s(20.0),
            grid_y + row as f32 * s(20.0),
            s(20.0),
            s(20.0),
        );
        let is_hovered = hovered(hud.pointer, &rect);
        bg.rect(rect, if is_hovered { SLOT_HOVER } else { SLOT_BG });

        let Some(state) = items.get(index).copied() else {
            continue;
        };
        let inner = Rect::new(rect.x + s(2.0), rect.y + s(2.0), s(16.0), s(16.0));
        if let Some((top, side)) = icon_layers(state, textures) {
            icons.block_icon(inner.x, inner.y, inner.w, top, side, 1.0);
        }
        buttons.push(Button {
            action: Action::CreativeItem(index),
            rect,
        });
    }

    // Pager and labels, drawn above the icons.
    let pager_y = panel.y + panel_h - s(22.0);
    let prev = Rect::new(panel.x + s(4.0), pager_y, s(24.0), s(18.0));
    let next = Rect::new(panel.x + panel_w - s(28.0), pager_y, s(24.0), s(18.0));
    draw_button(
        bg,
        fg,
        prev,
        "<",
        scale,
        hovered(hud.pointer, &prev) && page > 0,
    );
    draw_button(
        bg,
        fg,
        next,
        ">",
        scale,
        hovered(hud.pointer, &next) && page + 1 < pages,
    );
    fg.text_centered(
        &format!("{} / {}", page + 1, pages),
        cx,
        pager_y + s(5.0),
        scale,
        TEXT,
    );
    fg.text_centered("Creative Inventory", cx, panel.y - s(14.0), scale, TEXT);
    fg.text_centered(
        "Click a block to fill the selected slot  -  E / Esc to close",
        cx,
        panel.y + panel_h + s(6.0),
        scale,
        TEXT_DIM,
    );

    if page > 0 {
        buttons.push(Button {
            action: Action::CreativePage(-1),
            rect: prev,
        });
    }
    if page + 1 < pages {
        buttons.push(Button {
            action: Action::CreativePage(1),
            rect: next,
        });
    }
    buttons
}

/// Vanilla player-inventory panel size, in GUI pixels.
pub const INVENTORY_PANEL_W: f32 = 176.0;
pub const INVENTORY_PANEL_H: f32 = 166.0;
const SLOT_SIZE: f32 = 18.0;

fn inventory_panel(scale: f32, width: f32, height: f32) -> Rect {
    Rect::new(
        width / 2.0 - INVENTORY_PANEL_W * scale / 2.0,
        height / 2.0 - INVENTORY_PANEL_H * scale / 2.0,
        INVENTORY_PANEL_W * scale,
        INVENTORY_PANEL_H * scale,
    )
}

/// Top-left of a player-container slot inside the panel, in GUI pixels.
///
/// Vanilla 1.8 layout: output at (144,36), 2x2 grid at (98/116, 18/36), armour
/// down the left, three storage rows from y=84 and the hotbar at y=142.
pub fn inventory_slot_origin(slot: usize) -> Option<[f32; 2]> {
    Some(match slot {
        0 => [144.0, 36.0],
        1 => [98.0, 18.0],
        2 => [116.0, 18.0],
        3 => [98.0, 36.0],
        4 => [116.0, 36.0],
        5..=8 => [8.0, 8.0 + (slot - 5) as f32 * 18.0],
        9..=35 => {
            let index = slot - 9;
            [
                8.0 + (index % 9) as f32 * 18.0,
                84.0 + (index / 9) as f32 * 18.0,
            ]
        }
        36..=44 => [8.0 + (slot - 36) as f32 * 18.0, 142.0],
        _ => return None,
    })
}

/// Screen-space rectangle of a container slot, for hit-testing and drawing.
pub fn inventory_slot_rect(slot: usize, scale: f32, width: f32, height: f32) -> Option<Rect> {
    let [x, y] = inventory_slot_origin(slot)?;
    let panel = inventory_panel(scale, width, height);
    Some(Rect::new(
        panel.x + x * scale,
        panel.y + y * scale,
        SLOT_SIZE * scale,
        SLOT_SIZE * scale,
    ))
}

/// One stack: block icon (or the missing-texture tile) plus a count.
fn draw_stack(
    fg: &mut UiBatch,
    icons: &mut UiBatch,
    inner: Rect,
    item: ItemStack,
    textures: &TextureArray,
    scale: f32,
) {
    let s = |v: f32| v * scale;
    match item.block_state() {
        Some(state) => {
            if let Some((top, side)) = icon_layers(BlockState::from_raw(state), textures) {
                icons.block_icon(inner.x, inner.y, inner.w, top, side, 1.0);
            }
        }
        None => {
            // Tools, sticks and food have no uploaded item texture yet; the
            // missing tile is the project's convention for "no art".
            let layer = textures.layer_or_missing(MISSING_TEXTURE);
            icons.block_icon(inner.x, inner.y, inner.w, layer, layer, 1.0);
        }
    }
    if item.count > 1 {
        let label = item.count.to_string();
        let text_w = UiBatch::text_width(&label, scale);
        fg.text(
            &label,
            inner.x + inner.w + s(1.0) - text_w,
            inner.y + inner.h - s(7.0),
            scale,
            TEXT,
        );
    }
}

/// The survival inventory: 2x2 crafting, armour, storage and the hotbar, with
/// the stack under the cursor drawn last.
#[allow(clippy::too_many_arguments)]
pub fn draw_inventory(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    icons: &mut UiBatch,
    hud: &HudState<'_>,
    cursor: Option<ItemStack>,
    textures: &TextureArray,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let s = |v: f32| v * scale;
    bg.rect(Rect::new(0.0, 0.0, width, height), SHADOW_PANEL);

    let panel = inventory_panel(scale, width, height);
    bg.rect(panel, PANEL);
    bg.rect_outline(panel, scale.max(1.0), BUTTON_BORDER);
    fg.text_centered("Inventory", width / 2.0, panel.y - s(13.0), scale, TEXT);
    fg.text(
        "Crafting",
        panel.x + s(97.0),
        panel.y + s(6.0),
        scale,
        TEXT_DIM,
    );

    let mut buttons = Vec::new();
    for slot in 0..nephele_protocol::packets::play::PLAYER_CONTAINER_SLOTS {
        let Some(rect) = inventory_slot_rect(slot, scale, width, height) else {
            continue;
        };
        bg.rect(
            rect,
            if hovered(hud.pointer, &rect) {
                SLOT_HOVER
            } else {
                SLOT_BG
            },
        );
        if let Some(item) = hud.inventory.get(slot).copied().flatten() {
            let inner = Rect::new(rect.x + s(1.0), rect.y + s(1.0), s(16.0), s(16.0));
            draw_stack(fg, icons, inner, item, textures, scale);
        }
        buttons.push(Button {
            action: Action::InventorySlot(slot),
            rect,
        });
    }

    if let (Some(item), Some([x, y])) = (cursor, hud.pointer) {
        let rect = Rect::new(x - s(8.0), y - s(8.0), s(16.0), s(16.0));
        draw_stack(fg, icons, rect, item, textures, scale);
    }

    fg.text_centered(
        "Click to move stacks  -  E / Esc to close",
        width / 2.0,
        panel.y + panel.h + s(6.0),
        scale,
        TEXT_DIM,
    );

    buttons
}

/// The title screen: panorama behind (drawn by the renderer), logo, splash and
/// the four main buttons.
#[allow(clippy::too_many_arguments)]
pub fn draw_title(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    splash: &str,
    time: f32,
    pointer: Option<[f32; 2]>,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let cx = width / 2.0;

    // Logo: the wordmark occupies the top-left of a 256x256 image.
    let logo_w = (199.0 * scale * 0.95).min(width * 0.46);
    let logo_h = logo_w * (89.0 / 199.0);
    let logo = Rect::new(cx - logo_w / 2.0, height * 0.07, logo_w, logo_h);
    bg.sprite(sprites::LOGO_INK, logo, TEXT);

    if !splash.is_empty() {
        let pulse = 1.0 + 0.07 * (time * 4.0).sin();
        splash_text(
            bg,
            splash,
            cx + logo_w * 0.33,
            logo.y + logo_h * 0.74,
            scale * 1.5 * pulse,
        );
    }

    let w = 200.0 * scale;
    let h = 20.0 * scale;
    let gap = 4.0 * scale;
    let top = height * 0.60;
    let items = [
        ("Singleplayer", Action::Singleplayer),
        ("Multiplayer", Action::Multiplayer),
        ("Options...", Action::TitleOptions),
        ("Quit Game", Action::Quit),
    ];

    let mut buttons = Vec::new();
    for (index, (label, action)) in items.iter().enumerate() {
        let rect = Rect::new(cx - w / 2.0, top + index as f32 * (h + gap), w, h);
        draw_button(bg, fg, rect, label, scale, hovered(pointer, &rect));
        buttons.push(Button {
            action: *action,
            rect,
        });
    }

    fg.text(
        "Nephele 1.8.9",
        4.0 * scale,
        height - 14.0 * scale,
        scale,
        TEXT,
    );
    let right = "protocol 47";
    let right_width = UiBatch::text_width(right, scale);
    fg.text(
        right,
        width - right_width - 4.0 * scale,
        height - 14.0 * scale,
        scale,
        TEXT_DIM,
    );
    buttons
}

/// A splash line, rotated like vanilla's and drawn glyph by glyph.
fn splash_text(bg: &mut UiBatch, text: &str, cx: f32, cy: f32, scale: f32) {
    let angle: f32 = -20f32.to_radians();
    let (sin, cos) = angle.sin_cos();
    let width = UiBatch::text_width(text, scale);
    let height = 8.0 * scale;

    let mut cursor = -width / 2.0;
    for ch in text.chars() {
        let advance = font::advance(ch) * scale;
        let Some(glyph) = font::glyph(ch) else {
            cursor += advance;
            continue;
        };
        let gw = glyph.w as f32 * scale;
        let local = [
            [cursor, -height / 2.0],
            [cursor + gw, -height / 2.0],
            [cursor + gw, height / 2.0],
            [cursor, height / 2.0],
        ];
        let rotate = |p: [f32; 2], dx: f32, dy: f32| -> [f32; 2] {
            let (x, y) = (p[0] + dx, p[1] + dy);
            [cx + x * cos - y * sin, cy + x * sin + y * cos]
        };
        // Shadow first, then the yellow glyph.
        let shadow: [[f32; 2]; 4] = [
            rotate(local[0], 1.0, 1.0),
            rotate(local[1], 1.0, 1.0),
            rotate(local[2], 1.0, 1.0),
            rotate(local[3], 1.0, 1.0),
        ];
        bg.quad(shadow, glyph.uv(), [0.18, 0.18, 0.0, 1.0], 0);
        let corners: [[f32; 2]; 4] = [
            rotate(local[0], 0.0, 0.0),
            rotate(local[1], 0.0, 0.0),
            rotate(local[2], 0.0, 0.0),
            rotate(local[3], 0.0, 0.0),
        ];
        bg.quad(corners, glyph.uv(), [1.0, 0.85, 0.1, 1.0], 0);
        cursor += advance;
    }
}

/// Direct-connect screen: type `host[:port]`, then Connect.
pub fn draw_connect(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    address: &str,
    pointer: Option<[f32; 2]>,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let cx = width / 2.0;
    bg.rect(Rect::new(0.0, 0.0, width, height), [0.08, 0.08, 0.08, 1.0]);

    fg.text_centered("Connect to a Server", cx, height * 0.18, scale * 1.5, TEXT);

    let field = Rect::new(
        cx - 150.0 * scale,
        height * 0.38,
        300.0 * scale,
        20.0 * scale,
    );
    bg.rect(field, [0.0, 0.0, 0.0, 0.75]);
    bg.rect_outline(field, scale.max(1.0), BUTTON_BORDER);
    let text = if address.is_empty() {
        "Server address".to_string()
    } else {
        address.to_string()
    };
    let colour = if address.is_empty() { TEXT_DIM } else { TEXT };
    fg.text(
        &text,
        field.x + 4.0 * scale,
        field.y + 6.0 * scale,
        scale,
        colour,
    );

    let connect = Rect::new(
        cx - 100.0 * scale,
        height * 0.54,
        200.0 * scale,
        20.0 * scale,
    );
    let cancel = Rect::new(
        cx - 100.0 * scale,
        height * 0.54 + 24.0 * scale,
        200.0 * scale,
        20.0 * scale,
    );
    draw_button(
        bg,
        fg,
        connect,
        "Connect",
        scale,
        hovered(pointer, &connect),
    );
    draw_button(bg, fg, cancel, "Cancel", scale, hovered(pointer, &cancel));

    fg.text_centered(
        "Type an address (for example 127.0.0.1:25565), then Enter",
        cx,
        height * 0.72,
        scale,
        TEXT_DIM,
    );

    vec![
        Button {
            action: Action::ConnectSubmit,
            rect: connect,
        },
        Button {
            action: Action::ConnectCancel,
            rect: cancel,
        },
    ]
}

/// How many saved servers fit in the list before it needs scrolling.
pub const SERVER_ROWS: usize = 6;

/// The multiplayer screen: the saved list with its ping columns, and the
/// Join / Direct Connect / Refresh / Back buttons.
///
/// The four columns are name+address, MOTD, version and ping. Everything but
/// the name is a placeholder until the screen's pings come back — see
/// [`crate::servers::PingBoard`].
#[allow(clippy::too_many_arguments)]
pub fn draw_servers(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    list: &ServerList,
    pings: &PingBoard,
    selected: Option<&str>,
    scroll: usize,
    pointer: Option<[f32; 2]>,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let s = |v: f32| v * scale;
    let cx = width / 2.0;
    bg.rect(Rect::new(0.0, 0.0, width, height), [0.08, 0.08, 0.08, 1.0]);

    let row_h = s(22.0);
    let list_h = s(4.0) + row_h * SERVER_ROWS as f32 + s(4.0);
    // Heading, list, two button rows and hint, stacked and centred as one block
    // so the screen still fits a 240 px-tall GUI.
    let block_h =
        s(12.0) + s(8.0) + list_h + s(12.0) + s(20.0) + s(8.0) + s(20.0) + s(8.0) + s(8.0);
    let top = ((height - block_h) / 2.0).max(s(2.0));
    // Wide enough that a truncated MOTD still says something: at GUI scale 3
    // the columns get ~30 characters.
    let panel_w = s(400.0).min(width - s(8.0));
    let panel_x = cx - panel_w / 2.0;

    fg.text_centered("Play Multiplayer", cx, top, scale * 1.5, TEXT);

    let order = list.ordered_indices();
    if order.len() > SERVER_ROWS {
        let label = format!("1-{} of {}", scroll + SERVER_ROWS, order.len());
        let label_w = UiBatch::text_width(&label, scale);
        fg.text(
            &label,
            panel_x + panel_w - label_w,
            top + s(4.0),
            scale,
            TEXT_DIM,
        );
    }

    let list_rect = Rect::new(panel_x, top + s(20.0), panel_w, list_h);
    bg.rect(list_rect, PANEL);
    bg.rect_outline(list_rect, scale.max(1.0), BUTTON_BORDER);

    let visible = order.len().saturating_sub(scroll).min(SERVER_ROWS);
    if order.is_empty() {
        fg.text_centered(
            "No servers saved yet - use Direct Connect",
            cx,
            list_rect.y + list_h / 2.0 - s(4.0),
            scale,
            TEXT_DIM,
        );
    }

    let name_w = s(112.0);
    let mut buttons = Vec::new();
    for slot in 0..visible {
        let Some(&index) = order.get(scroll + slot) else {
            continue;
        };
        let Some(entry) = list.entries().get(index) else {
            continue;
        };

        let row = Rect::new(
            list_rect.x + s(2.0),
            list_rect.y + s(4.0) + slot as f32 * row_h,
            panel_w - s(4.0),
            row_h - s(1.0),
        );
        let is_selected = selected == Some(entry.address.as_str());
        let is_hovered = hovered(pointer, &row);
        bg.rect(
            row,
            if is_selected {
                ROW_SELECTED
            } else if is_hovered {
                ROW_HOVER
            } else if slot % 2 == 0 {
                ROW
            } else {
                ROW_ALT
            },
        );
        if is_selected {
            bg.rect_outline(row, scale.max(1.0), SELECTION_BORDER);
        }
        // Selecting is the row's job; the small buttons below win the hit test
        // because `hit_test` keeps the last match.
        buttons.push(Button {
            action: Action::ServerSelect(index),
            rect: row,
        });

        let star = Rect::new(row.x + s(2.0), row.y + s(3.0), s(10.0), s(16.0));
        fg.text_centered(
            "*",
            star.x + star.w / 2.0,
            star.y + s(4.0),
            scale,
            if entry.favourite { STAR } else { TEXT_DIM },
        );
        buttons.push(Button {
            action: Action::ServerFavourite(index),
            rect: star,
        });

        let remove = Rect::new(row.x + row.w - s(13.0), row.y + s(3.0), s(11.0), s(16.0));
        fg.text_centered(
            "x",
            remove.x + remove.w / 2.0,
            remove.y + s(4.0),
            scale,
            if hovered(pointer, &remove) {
                REMOVE_HOVER
            } else {
                TEXT_DIM
            },
        );
        buttons.push(Button {
            action: Action::ServerRemove(index),
            rect: remove,
        });

        // Column positions, right to left: remove, ping, version, MOTD.
        let name_x = row.x + s(15.0);
        let motd_x = row.x + s(130.0);
        let motd_w = (row.w - s(239.0)).max(s(24.0));
        let version_x = row.x + row.w - s(105.0);
        let ping_x = row.x + row.w - s(51.0);

        fg.text(
            &fit_text(entry.label(), name_w, scale),
            name_x,
            row.y + s(2.0),
            scale,
            TEXT,
        );
        fg.text(
            &fit_text(&entry.address, name_w, scale),
            name_x,
            row.y + s(11.0),
            scale,
            TEXT_DIM,
        );

        match pings.of(&entry.address) {
            Ping::Unknown => {}
            Ping::Pending => {
                fg.text(
                    &fit_text("Pinging...", motd_w, scale),
                    motd_x,
                    row.y + s(2.0),
                    scale,
                    TEXT_DIM,
                );
            }
            Ping::Offline => {
                fg.text(
                    &fit_text("Can't reach server", motd_w, scale),
                    motd_x,
                    row.y + s(2.0),
                    scale,
                    OFFLINE,
                );
            }
            Ping::Online(status) => {
                for (line_index, line) in status.motd.lines().take(2).enumerate() {
                    fg.text(
                        &fit_text(line, motd_w, scale),
                        motd_x,
                        row.y + s(2.0) + line_index as f32 * s(9.0),
                        scale,
                        MOTD,
                    );
                }
                fg.text(
                    &fit_text(&status.version, s(50.0), scale),
                    version_x,
                    row.y + s(2.0),
                    scale,
                    TEXT_DIM,
                );
                fg.text(
                    &fit_text(&format!("{} ms", status.ping_ms), s(36.0), scale),
                    ping_x,
                    row.y + s(2.0),
                    scale,
                    ping_colour(status.ping_ms),
                );
            }
        }
    }

    let button_w = (panel_w - s(6.0)) / 2.0;
    let first_row = list_rect.y + list_h + s(12.0);
    let second_row = first_row + s(28.0);
    let join = Rect::new(panel_x, first_row, button_w, s(20.0));
    let direct = Rect::new(panel_x + button_w + s(6.0), first_row, button_w, s(20.0));
    let refresh = Rect::new(panel_x, second_row, button_w, s(20.0));
    let back = Rect::new(panel_x + button_w + s(6.0), second_row, button_w, s(20.0));

    let can_join = selected.is_some();
    draw_button_state(
        bg,
        fg,
        join,
        "Join Server",
        scale,
        can_join && hovered(pointer, &join),
        can_join,
    );
    draw_button(
        bg,
        fg,
        direct,
        "Direct Connect",
        scale,
        hovered(pointer, &direct),
    );
    draw_button(
        bg,
        fg,
        refresh,
        "Refresh",
        scale,
        hovered(pointer, &refresh),
    );
    draw_button(bg, fg, back, "Back", scale, hovered(pointer, &back));

    if can_join {
        buttons.push(Button {
            action: Action::ServersJoin,
            rect: join,
        });
    }
    buttons.push(Button {
        action: Action::ServersDirectConnect,
        rect: direct,
    });
    buttons.push(Button {
        action: Action::ServersRefresh,
        rect: refresh,
    });
    buttons.push(Button {
        action: Action::ServersBack,
        rect: back,
    });

    fg.text_centered(
        "Click to select, double-click or Enter to join.  * favourite, x delete.",
        cx,
        second_row + s(28.0),
        scale,
        TEXT_DIM,
    );

    buttons
}

/// Green / amber / red for a round-trip time, in the PvP-client convention.
fn ping_colour(ms: u32) -> [f32; 4] {
    if ms < 80 {
        PING_GOOD
    } else if ms < 160 {
        PING_WARN
    } else {
        PING_BAD
    }
}

/// Trim `text` with a trailing `...` until it fits `max_width`.
fn fit_text(text: &str, max_width: f32, scale: f32) -> String {
    if UiBatch::text_width(text, scale) <= max_width {
        return text.to_owned();
    }
    const ELLIPSIS: &str = "...";
    let budget = (max_width - UiBatch::text_width(ELLIPSIS, scale)).max(0.0);
    let mut out = String::new();
    let mut width = 0.0;
    for ch in text.chars() {
        let advance = font::advance(ch) * scale;
        if width + advance > budget {
            break;
        }
        out.push(ch);
        width += advance;
    }
    out.push_str(ELLIPSIS);
    out
}

/// Topmost button under the pointer.
pub fn hit_test(buttons: &[Button], pointer: [f32; 2]) -> Option<Action> {
    buttons
        .iter()
        .rev()
        .find(|button| button.rect.contains(pointer[0], pointer[1]))
        .map(|button| button.action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::servers::ServerEntry;

    #[test]
    fn gui_scale_is_integral_and_bounded() {
        assert_eq!(gui_scale(1280.0, 720.0), 3.0);
        assert_eq!(gui_scale(320.0, 240.0), 1.0);
        assert_eq!(gui_scale(3840.0, 2160.0), 4.0);
        assert_eq!(gui_scale(100.0, 100.0), 1.0);
    }

    #[test]
    fn creative_list_covers_the_block_range() {
        let items = creative_items();
        assert!(items.len() > 100, "{} items", items.len());
        assert!(items.iter().any(|s| s.id() == 1));
        assert!(items.iter().any(|s| s.id() == 35 && s.meta() == 14));
        assert!(!items.iter().any(|s| s.id() == 0));
        assert!(!items.iter().any(|s| matches!(s.id(), 8..=11)));
    }

    #[test]
    fn creative_pages_cover_every_item() {
        assert!(creative_pages() >= 1);
        assert!(creative_pages() * CREATIVE_SLOTS >= creative_items().len());
    }

    fn hud_state<'a>(inventory: &'a [Option<ItemStack>]) -> HudState<'a> {
        HudState {
            inventory,
            held_slot: 0,
            health: 20.0,
            food: 20,
            xp_bar: 0.0,
            xp_level: 0,
            fps: 60.0,
            position: [0.0, 0.0, 0.0],
            chunks: 0,
            creative: true,
            creative_page: 0,
            pointer: None,
            captured: false,
            chatting: false,
        }
    }

    #[test]
    fn settings_screen_has_a_control_per_setting() {
        let inventory = vec![None; 45];
        let hud = hud_state(&inventory);
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        let buttons = draw_settings(
            &mut bg,
            &mut fg,
            &hud,
            &Settings::default(),
            2.0,
            1280.0,
            720.0,
        );
        assert_eq!(
            buttons.len(),
            SettingId::ALL.len() * 2 + 1,
            "two arrows + Done"
        );
        assert!(buttons.iter().any(|b| b.action == Action::SettingsDone));
        assert!(!bg.is_empty());
        assert!(!fg.is_empty());
    }

    #[test]
    fn pause_menu_offers_settings_and_disconnect() {
        let inventory = vec![None; 45];
        let hud = hud_state(&inventory);
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        let buttons = draw_pause(&mut bg, &mut fg, &hud, 2.0, 1280.0, 720.0);
        assert_eq!(buttons.len(), 3);
        assert!(buttons.iter().any(|b| b.action == Action::OpenSettings));
        assert!(buttons.iter().any(|b| b.action == Action::Resume));
        assert!(buttons.iter().any(|b| b.action == Action::Disconnect));
        // The three buttons must not overlap.
        assert!(buttons[0].rect.y + buttons[0].rect.h <= buttons[1].rect.y);
        assert!(buttons[1].rect.y + buttons[1].rect.h <= buttons[2].rect.y);
    }

    #[test]
    fn inventory_slots_sit_where_vanilla_puts_them() {
        assert_eq!(inventory_slot_origin(0), Some([144.0, 36.0]));
        assert_eq!(inventory_slot_origin(1), Some([98.0, 18.0]));
        assert_eq!(inventory_slot_origin(4), Some([116.0, 36.0]));
        assert_eq!(inventory_slot_origin(5), Some([8.0, 8.0]));
        assert_eq!(inventory_slot_origin(8), Some([8.0, 62.0]));
        assert_eq!(inventory_slot_origin(9), Some([8.0, 84.0]));
        assert_eq!(inventory_slot_origin(35), Some([152.0, 120.0]));
        assert_eq!(inventory_slot_origin(36), Some([8.0, 142.0]));
        assert_eq!(inventory_slot_origin(44), Some([152.0, 142.0]));
        assert_eq!(inventory_slot_origin(45), None);
    }

    #[test]
    fn inventory_slot_rects_are_inside_the_panel_and_do_not_overlap() {
        let (width, height, scale) = (1280.0, 720.0, 3.0);
        let panel = inventory_panel(scale, width, height);
        let mut rects: Vec<Rect> = Vec::new();
        for slot in 0..45 {
            let rect = inventory_slot_rect(slot, scale, width, height).expect("slot exists");
            assert!(rect.x >= panel.x && rect.y >= panel.y);
            assert!(rect.x + rect.w <= panel.x + panel.w);
            assert!(rect.y + rect.h <= panel.y + panel.h);
            for other in &rects {
                let overlap = rect.x < other.x + other.w
                    && other.x < rect.x + rect.w
                    && rect.y < other.y + other.h
                    && other.y < rect.y + rect.h;
                assert!(!overlap, "slots overlap");
            }
            rects.push(rect);
        }
    }

    #[test]
    fn hit_test_picks_the_last_matching_button() {
        let buttons = vec![
            Button {
                action: Action::Resume,
                rect: Rect::new(0.0, 0.0, 50.0, 50.0),
            },
            Button {
                action: Action::Disconnect,
                rect: Rect::new(10.0, 10.0, 20.0, 20.0),
            },
        ];
        assert_eq!(hit_test(&buttons, [15.0, 15.0]), Some(Action::Disconnect));
        assert_eq!(hit_test(&buttons, [45.0, 45.0]), Some(Action::Resume));
        assert_eq!(hit_test(&buttons, [200.0, 200.0]), None);
    }

    fn server_screen(
        list: &ServerList,
        pings: &PingBoard,
        selected: Option<&str>,
        scroll: usize,
    ) -> Vec<Button> {
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        draw_servers(
            &mut bg, &mut fg, list, pings, selected, scroll, None, 2.0, 1280.0, 720.0,
        )
    }

    fn three_servers() -> ServerList {
        let mut list = ServerList::default();
        list.add(ServerEntry::new("one", "1.example.net"));
        list.add(ServerEntry::new("two", "2.example.net"));
        list.add(ServerEntry::new("three", "3.example.net"));
        list
    }

    fn rect_of(buttons: &[Button], action: Action) -> Rect {
        buttons
            .iter()
            .find(|button| button.action == action)
            .map(|button| button.rect)
            .unwrap_or_else(|| panic!("{action:?} was not drawn"))
    }

    #[test]
    fn joining_needs_a_selection_but_the_other_buttons_are_always_there() {
        let list = three_servers();
        let pings = PingBoard::default();

        let buttons = server_screen(&list, &pings, None, 0);
        assert!(!buttons.iter().any(|b| b.action == Action::ServersJoin));
        for action in [
            Action::ServersDirectConnect,
            Action::ServersRefresh,
            Action::ServersBack,
        ] {
            rect_of(&buttons, action);
        }

        let buttons = server_screen(&list, &pings, Some("1.example.net"), 0);
        rect_of(&buttons, Action::ServersJoin);
    }

    #[test]
    fn every_saved_server_gets_a_row_that_acts_on_its_storage_index() {
        let list = three_servers();
        let buttons = server_screen(&list, &PingBoard::default(), None, 0);
        for index in 0..3 {
            rect_of(&buttons, Action::ServerSelect(index));
            rect_of(&buttons, Action::ServerFavourite(index));
            rect_of(&buttons, Action::ServerRemove(index));
        }
        assert!(!buttons.iter().any(|b| b.action == Action::ServerSelect(3)));
    }

    #[test]
    fn favourites_are_drawn_above_the_other_servers() {
        let mut list = three_servers();
        list.toggle_favourite(2);
        let buttons = server_screen(&list, &PingBoard::default(), None, 0);
        assert!(
            rect_of(&buttons, Action::ServerSelect(2)).y
                < rect_of(&buttons, Action::ServerSelect(0)).y
        );
    }

    /// The small controls must win the hit test against the row behind them.
    #[test]
    fn the_star_and_remove_buttons_beat_the_row_itself() {
        let list = three_servers();
        let buttons = server_screen(&list, &PingBoard::default(), None, 0);
        let star = rect_of(&buttons, Action::ServerFavourite(0));
        let remove = rect_of(&buttons, Action::ServerRemove(0));
        assert_eq!(
            hit_test(&buttons, [star.x + 1.0, star.y + 1.0]),
            Some(Action::ServerFavourite(0))
        );
        assert_eq!(
            hit_test(&buttons, [remove.x + 1.0, remove.y + 1.0]),
            Some(Action::ServerRemove(0))
        );
        let row = rect_of(&buttons, Action::ServerSelect(0));
        assert_eq!(
            hit_test(&buttons, [row.x + row.w / 2.0, row.y + row.h / 2.0]),
            Some(Action::ServerSelect(0))
        );
    }

    #[test]
    fn the_list_shows_one_page_and_scrolls_to_the_rest() {
        let mut list = ServerList::default();
        for index in 0..SERVER_ROWS + 2 {
            list.add(ServerEntry::new(
                &format!("server {index}"),
                &format!("{index}.example.net"),
            ));
        }

        let first = server_screen(&list, &PingBoard::default(), None, 0);
        let rows: Vec<Rect> = (0..SERVER_ROWS)
            .map(|index| rect_of(&first, Action::ServerSelect(index)))
            .collect();
        assert_eq!(rows.len(), SERVER_ROWS);
        assert!(!first
            .iter()
            .any(|b| b.action == Action::ServerSelect(SERVER_ROWS)));

        let last = server_screen(&list, &PingBoard::default(), None, 2);
        rect_of(&last, Action::ServerSelect(SERVER_ROWS + 1));
        assert!(!last.iter().any(|b| b.action == Action::ServerSelect(1)));
    }

    #[test]
    fn rows_and_bottom_controls_do_not_overlap() {
        let list = three_servers();
        let buttons = server_screen(&list, &PingBoard::default(), Some("1.example.net"), 0);
        let mut rows: Vec<Rect> = (0..3)
            .map(|index| rect_of(&buttons, Action::ServerSelect(index)))
            .collect();
        rows.sort_by(|a, b| a.y.total_cmp(&b.y));
        for pair in rows.windows(2) {
            assert!(pair[0].y + pair[0].h <= pair[1].y);
        }
        let join = rect_of(&buttons, Action::ServersJoin);
        let direct = rect_of(&buttons, Action::ServersDirectConnect);
        let refresh = rect_of(&buttons, Action::ServersRefresh);
        let back = rect_of(&buttons, Action::ServersBack);
        assert!(rows.last().unwrap().y + rows.last().unwrap().h <= join.y);
        assert!(join.x + join.w <= direct.x);
        assert!(join.y + join.h <= refresh.y);
        assert!(refresh.x + refresh.w <= back.x);
    }

    #[test]
    fn every_ping_state_draws_without_losing_the_row() {
        let list = three_servers();
        let mut pings = PingBoard::default();
        pings.finish(
            "1.example.net",
            Ping::Online(crate::servers::Status {
                motd: "A Nephele Test\nsecond line".to_string(),
                version: "1.8.9".to_string(),
                ping_ms: 42,
            }),
        );
        pings.begin("2.example.net");
        pings.finish("3.example.net", Ping::Offline);

        let buttons = server_screen(&list, &pings, None, 0);
        for index in 0..3 {
            rect_of(&buttons, Action::ServerSelect(index));
        }
    }

    #[test]
    fn ping_colours_follow_the_latency_bands() {
        assert_eq!(ping_colour(12), PING_GOOD);
        assert_eq!(ping_colour(80), PING_WARN);
        assert_eq!(ping_colour(159), PING_WARN);
        assert_eq!(ping_colour(160), PING_BAD);
    }

    #[test]
    fn long_labels_are_trimmed_to_fit() {
        let label = "a-very-long-server-name-that-will-not-fit-in-the-row";
        let fitted = fit_text(label, 100.0, 2.0);
        assert!(fitted.ends_with("..."));
        assert!(UiBatch::text_width(&fitted, 2.0) <= 100.0);
        assert_eq!(fit_text("short", 100.0, 2.0), "short");
    }

    #[test]
    fn chat_draws_its_lines_and_the_input_strip() {
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        let lines = vec![("hello".to_string(), 1.0), ("fading".to_string(), 0.4)];
        draw_chat(
            &mut bg,
            &mut fg,
            &lines,
            Some("/gamemode creative"),
            2.0,
            1280.0,
            720.0,
        );
        assert!(!bg.is_empty());
        assert!(!fg.is_empty());
        // Each line gets a backdrop plus the strip gets one; the caret is a
        // fourth rectangle on the background layer.
        assert_eq!(bg.indices.len() / 6, 4);

        let mut quiet = UiBatch::default();
        let mut quiet_fg = UiBatch::default();
        draw_chat(&mut quiet, &mut quiet_fg, &[], None, 2.0, 1280.0, 720.0);
        assert!(
            quiet.is_empty(),
            "nothing to draw with no lines and no input"
        );
    }
}

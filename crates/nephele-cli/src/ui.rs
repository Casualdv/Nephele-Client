//! HUD, hotbar, pause menu and creative inventory.
//!
//! Everything is laid out in vanilla GUI pixels (320x240-ish) and multiplied by
//! an integer [`gui_scale`] so the HUD keeps its proportions on any window size.
//!
//! Drawing is split into three painter's layers: `bg` (panels, bars, slot
//! backgrounds), `icons` (isometric block icons, which need the texture array)
//! and `fg` (text and highlights). The renderer draws them in that order.

use std::sync::OnceLock;

use crate::servers::ServerList;
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
const STAR: [f32; 4] = [1.0, 0.85, 0.25, 1.0];
const REMOVE_HOVER: [f32; 4] = [1.0, 0.4, 0.4, 1.0];

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
}

/// A clickable thing the UI drew this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Singleplayer,
    Multiplayer,
    TitleOptions,
    Quit,
    /// Join the address typed into the multiplayer screen's field.
    ServersSubmit,
    ServersBack,
    /// Join a saved entry, by storage index.
    ServerJoin(usize),
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
    if !hud.captured {
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

fn draw_button(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    rect: Rect,
    label: &str,
    scale: f32,
    hover: bool,
) {
    // Vanilla's button face: three stacked 200x20 strips in widgets.png.
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
        TEXT,
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
pub const SERVER_ROWS: usize = 5;

/// The multiplayer screen: the saved server list, an address field and the
/// Join / Back buttons.
///
/// The address field is the same widget `draw_connect` draws and feeds the same
/// buffer, so both screens share one piece of input handling.
#[allow(clippy::too_many_arguments)]
pub fn draw_servers(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    list: &ServerList,
    address: &str,
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
    // Heading, list, field, buttons and hint, stacked and centred as one block
    // so a 5-row list still fits a 240 px-tall GUI.
    let block_h =
        s(12.0) + s(8.0) + list_h + s(12.0) + s(20.0) + s(8.0) + s(20.0) + s(8.0) + s(8.0);
    let top = ((height - block_h) / 2.0).max(s(2.0));
    let panel_w = s(320.0).min(width - s(8.0));
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
            "No servers saved yet",
            cx,
            list_rect.y + list_h / 2.0 - s(4.0),
            scale,
            TEXT_DIM,
        );
    }

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
        let is_hovered = hovered(pointer, &row);
        bg.rect(
            row,
            if is_hovered {
                ROW_HOVER
            } else if slot % 2 == 0 {
                ROW
            } else {
                ROW_ALT
            },
        );
        // The whole row joins; the small buttons below are hit-tested first
        // because `hit_test` keeps the last match.
        buttons.push(Button {
            action: Action::ServerJoin(index),
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

        let text_x = row.x + s(15.0);
        let text_w = row.w - s(31.0);
        fg.text(
            &fit_text(entry.label(), text_w, scale),
            text_x,
            row.y + s(2.0),
            scale,
            TEXT,
        );
        fg.text(
            &fit_text(&entry.address, text_w, scale),
            text_x,
            row.y + s(11.0),
            scale,
            TEXT_DIM,
        );
    }

    let field = Rect::new(panel_x, list_rect.y + list_h + s(12.0), panel_w, s(20.0));
    bg.rect(field, [0.0, 0.0, 0.0, 0.75]);
    bg.rect_outline(field, scale.max(1.0), BUTTON_BORDER);
    let text = if address.is_empty() {
        "Server address".to_string()
    } else {
        address.to_string()
    };
    let colour = if address.is_empty() { TEXT_DIM } else { TEXT };
    fg.text(
        &fit_text(&text, field.w - s(8.0), scale),
        field.x + s(4.0),
        field.y + s(6.0),
        scale,
        colour,
    );

    let button_w = (panel_w - s(6.0)) / 2.0;
    let button_y = field.y + field.h + s(8.0);
    let join = Rect::new(panel_x, button_y, button_w, s(20.0));
    let back = Rect::new(panel_x + button_w + s(6.0), button_y, button_w, s(20.0));
    let can_join = !address.trim().is_empty();
    draw_button(
        bg,
        fg,
        join,
        "Join Server",
        scale,
        can_join && hovered(pointer, &join),
    );
    draw_button(bg, fg, back, "Back", scale, hovered(pointer, &back));
    if can_join {
        buttons.push(Button {
            action: Action::ServersSubmit,
            rect: join,
        });
    }
    buttons.push(Button {
        action: Action::ServersBack,
        rect: back,
    });

    fg.text_centered(
        "Click a server to join it.  * favourite, x remove.  Esc goes back.",
        cx,
        button_y + s(28.0),
        scale,
        TEXT_DIM,
    );

    buttons
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

    fn server_screen(list: &ServerList, address: &str, scroll: usize) -> Vec<Button> {
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        draw_servers(
            &mut bg, &mut fg, list, address, scroll, None, 2.0, 1280.0, 720.0,
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
    fn the_server_screen_needs_an_address_before_it_can_join() {
        let list = three_servers();
        let buttons = server_screen(&list, "  ", 0);
        assert!(!buttons.iter().any(|b| b.action == Action::ServersSubmit));
        assert!(buttons.iter().any(|b| b.action == Action::ServersBack));

        let buttons = server_screen(&list, "play.example.net", 0);
        assert!(buttons.iter().any(|b| b.action == Action::ServersSubmit));
    }

    #[test]
    fn every_saved_server_gets_a_row_that_acts_on_its_storage_index() {
        let list = three_servers();
        let buttons = server_screen(&list, "", 0);
        for index in 0..3 {
            rect_of(&buttons, Action::ServerJoin(index));
            rect_of(&buttons, Action::ServerFavourite(index));
            rect_of(&buttons, Action::ServerRemove(index));
        }
        assert!(!buttons.iter().any(|b| b.action == Action::ServerJoin(3)));
    }

    #[test]
    fn favourites_are_drawn_above_the_other_servers() {
        let mut list = three_servers();
        list.toggle_favourite(2);
        let buttons = server_screen(&list, "", 0);
        assert!(
            rect_of(&buttons, Action::ServerJoin(2)).y < rect_of(&buttons, Action::ServerJoin(0)).y
        );
    }

    /// The small controls must win the hit test against the row behind them.
    #[test]
    fn the_star_and_remove_buttons_beat_the_row_itself() {
        let list = three_servers();
        let buttons = server_screen(&list, "", 0);
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
        let row = rect_of(&buttons, Action::ServerJoin(0));
        assert_eq!(
            hit_test(&buttons, [row.x + row.w / 2.0, row.y + row.h / 2.0]),
            Some(Action::ServerJoin(0))
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

        let first = server_screen(&list, "", 0);
        let rows: Vec<Rect> = (0..SERVER_ROWS)
            .map(|index| rect_of(&first, Action::ServerJoin(index)))
            .collect();
        assert_eq!(rows.len(), SERVER_ROWS);
        assert!(!first
            .iter()
            .any(|b| b.action == Action::ServerJoin(SERVER_ROWS)));

        let last = server_screen(&list, "", 2);
        rect_of(&last, Action::ServerJoin(SERVER_ROWS + 1));
        assert!(!last.iter().any(|b| b.action == Action::ServerJoin(1)));
    }

    #[test]
    fn rows_and_bottom_controls_do_not_overlap() {
        let list = three_servers();
        let buttons = server_screen(&list, "address", 0);
        let mut rows: Vec<Rect> = (0..3)
            .map(|index| rect_of(&buttons, Action::ServerJoin(index)))
            .collect();
        rows.sort_by(|a, b| a.y.total_cmp(&b.y));
        for pair in rows.windows(2) {
            assert!(pair[0].y + pair[0].h <= pair[1].y);
        }
        let join = rect_of(&buttons, Action::ServersSubmit);
        let back = rect_of(&buttons, Action::ServersBack);
        assert!(rows.last().unwrap().y + rows.last().unwrap().h <= join.y);
        assert!(join.x + join.w <= back.x);
    }

    #[test]
    fn long_labels_are_trimmed_to_fit() {
        let label = "a-very-long-server-name-that-will-not-fit-in-the-row";
        let fitted = fit_text(label, 100.0, 2.0);
        assert!(fitted.ends_with("..."));
        assert!(UiBatch::text_width(&fitted, 2.0) <= 100.0);
        assert_eq!(fit_text("short", 100.0, 2.0), "short");
    }
}

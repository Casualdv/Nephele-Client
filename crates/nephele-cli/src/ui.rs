//! HUD, hotbar, pause menu and creative inventory.
//!
//! Everything is laid out in vanilla GUI pixels (320x240-ish) and multiplied by
//! an integer [`gui_scale`] so the HUD keeps its proportions on any window size.
//!
//! Drawing is split into three painter's layers: `bg` (panels, bars, slot
//! backgrounds), `icons` (isometric block icons, which need the texture array)
//! and `fg` (text and highlights). The renderer draws them in that order.

use std::sync::OnceLock;

use crate::chat::Span;
use crate::hud_modules::{KeyBox, ModuleConfig, ModuleId, Values as ModuleValues};
use crate::scoreboard::{self, Row as ScoreboardRow};
use crate::servers::{Ping, PingBoard, ServerList};
use crate::settings::{SettingId, Settings};
use nephele_assets::{
    face_textures, font, item_texture_name, sprites, TextureArray, UiAtlas, MISSING_TEXTURE,
};
use nephele_protocol::packets::play::{ItemStack, HOTBAR_FIRST_SLOT, HOTBAR_SLOTS};
use nephele_render::{Rect, UiBatch};
use nephele_world::save::{WorldMetadata, WorldMode};
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
    /// The saved single-player worlds.
    Worlds,
    /// World name, seed and game-mode form.
    WorldCreate,
    /// Rename the selected single-player world.
    WorldRename,
    /// Explicit confirmation before deleting a single-player world.
    WorldDeleteConfirm,
    /// The saved-server list and its address field.
    Servers,
    Connect,
    Game,
    Pause,
    Settings,
    Creative,
    /// The survival player inventory (2x2 crafting, armour, storage, hotbar).
    Inventory,
    /// Vanilla's "You Died!" screen, shown until the player respawns.
    Death,
    /// The HUD module editor: toggle rows plus the modules drawn in place, where
    /// dragging moves them.
    Modules,
}

/// A clickable thing the UI drew this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Singleplayer,
    Multiplayer,
    WorldSelect(usize),
    WorldOpenSelected,
    WorldNew,
    WorldRenameSelected,
    WorldDeleteSelected,
    WorldsBack,
    WorldNameFocus,
    WorldSeedFocus,
    WorldModeToggle,
    WorldCreateSubmit,
    WorldCreateCancel,
    WorldRenameFocus,
    WorldRenameSubmit,
    WorldRenameCancel,
    WorldDeleteConfirm,
    WorldDeleteCancel,
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
    /// Perform respawn from the death screen.
    Respawn,
    /// Open the HUD module editor from the settings screen.
    OpenModules,
    /// Leave the HUD module editor, back to settings.
    ModulesDone,
    /// Turn one HUD module on or off.
    ModuleToggle(ModuleId),
    /// Put every module back where it started.
    ModulesReset,
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
    /// Slots of the open container window (a crafting table), if any.
    pub window_slots: Option<&'a [Option<ItemStack>]>,
    /// Inventory type of the open window (`minecraft:chest`, ...), for its title.
    pub window_kind: Option<&'a str>,
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
        textures.slot_or_missing(faces.top),
        textures.slot_or_missing(faces.side),
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
    // The one-line debug readout is a development aid; the modules carry FPS,
    // coordinates and speed in play, so this would only be screen noise.
    if debug_line_enabled() {
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
    }
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

/// Red edge vignette shown briefly after the player takes damage.
pub fn draw_damage_vignette(bg: &mut UiBatch, width: f32, height: f32, strength: f32) {
    let strength = strength.clamp(0.0, 1.0);
    if strength <= 0.0 {
        return;
    }
    let uv = UiAtlas::white_uv();
    let red = [0.62, 0.02, 0.02, 0.48 * strength];
    let clear = [0.62, 0.02, 0.02, 0.0];
    let side = width * 0.16;
    let band = height * 0.20;

    bg.quad_colors(
        [[0.0, 0.0], [width, 0.0], [width, band], [0.0, band]],
        uv,
        [red, red, clear, clear],
        0,
    );
    bg.quad_colors(
        [
            [0.0, height - band],
            [width, height - band],
            [width, height],
            [0.0, height],
        ],
        uv,
        [clear, clear, red, red],
        0,
    );
    bg.quad_colors(
        [[0.0, 0.0], [side, 0.0], [side, height], [0.0, height]],
        uv,
        [red, clear, clear, red],
        0,
    );
    bg.quad_colors(
        [
            [width - side, 0.0],
            [width, 0.0],
            [width, height],
            [width - side, height],
        ],
        uv,
        [clear, red, red, clear],
        0,
    );
}

/// Draw coloured runs left to right. Returns the width drawn.
pub fn draw_spans(fg: &mut UiBatch, spans: &[Span], x: f32, y: f32, scale: f32, alpha: f32) -> f32 {
    let mut cursor = x;
    for span in spans {
        let color = [
            span.color[0],
            span.color[1],
            span.color[2],
            span.color[3] * alpha,
        ];
        fg.text(&span.text, cursor, y, scale, color);
        cursor += UiBatch::text_width(&span.text, scale);
    }
    cursor - x
}

/// Width of a run list in pixels.
pub fn spans_width(spans: &[Span], scale: f32) -> f32 {
    spans
        .iter()
        .map(|span| UiBatch::text_width(&span.text, scale))
        .sum()
}

/// Recent chat in the bottom-left, and the input line while it is open.
///
/// `lines` carries `(spans, alpha)` pairs so the caller owns the fade timing;
/// the input line sits just above the hotbar so the two never overlap. Chat
/// arrives coloured (§ codes and JSON components, see [`crate::chat`]), so each
/// line is drawn run by run.
pub fn draw_chat(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    lines: &[(&[Span], f32)],
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
    for (index, (spans, alpha)) in lines.iter().rev().enumerate() {
        let y = bottom - (index as f32 + 1.0) * line_h;
        if y < 0.0 {
            break;
        }
        let text_w = spans_width(spans, scale);
        bg.rect(
            Rect::new(s(2.0), y - s(1.0), text_w + s(4.0), line_h),
            [0.0, 0.0, 0.0, 0.35 * alpha],
        );
        draw_spans(fg, spans, s(4.0), y, scale, *alpha);
    }
}

/// The scoreboard sidebar: a translucent column against the right edge, its
/// title across the top and one right-aligned row per entry.
pub fn draw_sidebar(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    sidebar: &ScoreboardRowList,
    scale: f32,
    width: f32,
    height: f32,
) -> Option<scoreboard::Layout> {
    if sidebar.rows.is_empty() {
        return None;
    }
    let s = |v: f32| v * scale;
    // Measure in GUI pixels, lay out there too, then scale into the window.
    let measured: Vec<(f32, f32)> = sidebar
        .rows
        .iter()
        .map(|row| {
            (
                spans_width(&row.spans, 1.0),
                UiBatch::text_width(&row.score, 1.0),
            )
        })
        .collect();
    let title_width = sidebar.title.map(|title| spans_width(title, 1.0));
    let layout = scoreboard::layout(&measured, title_width, (width / scale, height / scale));

    let panel = Rect::new(s(layout.x), s(layout.y), s(layout.width), s(layout.height));
    // Vanilla 1.8 paints the pane at 0x50000000; anything more hides the world.
    bg.rect(panel, [0.0, 0.0, 0.0, 0.31]);

    if let Some(title) = sidebar.title {
        let title_w = spans_width(title, scale);
        let x = panel.x + (panel.w - title_w) / 2.0;
        draw_spans(fg, title, x, s(layout.title_y) + s(1.0), scale, 1.0);
    }

    for (index, row) in sidebar.rows.iter().enumerate() {
        let y = s(layout.first_row_y) + index as f32 * s(layout.row_h);
        draw_spans(fg, &row.spans, s(layout.text_x), y, scale, 1.0);
        // Scores are right-aligned and red, as `GuiIngame` draws them.
        let score_w = UiBatch::text_width(&row.score, scale);
        fg.text(
            &row.score,
            s(layout.score_right) - score_w,
            y,
            scale,
            [1.0, 0.25, 0.25, 1.0],
        );
    }
    Some(layout)
}

/// What [`draw_sidebar`] needs: a title (optional) and the rows.
pub struct ScoreboardRowList<'a> {
    pub title: Option<&'a [Span]>,
    pub rows: &'a [ScoreboardRow],
}

const MODULE_LABEL: [f32; 4] = [0.72, 0.72, 0.72, 1.0];
const MODULE_PANEL: [f32; 4] = [0.0, 0.0, 0.0, 0.6];
const KEY_BG: [f32; 4] = [0.08, 0.08, 0.08, 0.55];
const KEY_BORDER: [f32; 4] = [0.85, 0.85, 0.85, 0.55];
const KEY_PRESSED: [f32; 4] = [1.0, 1.0, 1.0, 0.92];
const KEY_PRESSED_TEXT: [f32; 4] = [0.05, 0.05, 0.05, 1.0];

/// One information module's text.
fn module_text(id: ModuleId, values: &ModuleValues) -> (String, String, [f32; 4]) {
    match id {
        ModuleId::Fps => (
            "FPS".to_string(),
            (values.fps.round() as i32).to_string(),
            TEXT,
        ),
        ModuleId::Ping => {
            let (value, color) = match values.ping {
                None => ("--".to_string(), TEXT_DIM),
                Some(ping) => (
                    format!("{ping}ms"),
                    match ping {
                        0..=80 => [0.45, 0.95, 0.45, 1.0],
                        81..=200 => [0.95, 0.85, 0.30, 1.0],
                        _ => [0.95, 0.42, 0.42, 1.0],
                    },
                ),
            };
            ("PING".to_string(), value, color)
        }
        ModuleId::Cps => (
            "CPS".to_string(),
            format!("{} | {}", values.cps.0, values.cps.1),
            TEXT,
        ),
        ModuleId::Coords => (
            "XYZ".to_string(),
            format!(
                "{:.0} {:.0} {:.0}",
                values.position[0], values.position[1], values.position[2]
            ),
            [0.55, 0.95, 0.55, 1.0],
        ),
        ModuleId::Speed => (
            "SPEED".to_string(),
            format!("{:.1}", values.speed),
            [0.80, 0.90, 1.0, 1.0],
        ),
        // Drawn as a key box, not a label/value row.
        ModuleId::Keystrokes => (String::new(), String::new(), TEXT),
    }
}

/// Width and height of one module in GUI pixels, without drawing it.
fn module_size(id: ModuleId, values: &ModuleValues) -> (f32, f32) {
    if id == ModuleId::Keystrokes {
        return (
            KEY_UNIT * 3.0 + KEY_GAP * 2.0 + MODULE_PAD * 2.0,
            key_layout_height(),
        );
    }
    let (label, value, _) = module_text(id, values);
    let label_w = UiBatch::text_width(&label, 1.0);
    let value_w = UiBatch::text_width(&value, 1.0);
    (
        label_w + MODULE_GAP + value_w + MODULE_PAD * 2.0,
        FONT_LINE + MODULE_PAD * 2.0,
    )
}

/// Keystroke key size and spacing, in GUI pixels.
const KEY_UNIT: f32 = 13.0;
const KEY_GAP: f32 = 1.0;
const FONT_LINE: f32 = 9.0;
const MODULE_GAP: f32 = 4.0;
const MODULE_PAD: f32 = 2.0;

fn key_layout_height() -> f32 {
    // W / A S D / space / shift / LMB RMB.
    5.0 * KEY_UNIT + 4.0 * KEY_GAP + MODULE_PAD * 2.0
}

fn draw_key(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    rect: Rect,
    label: &str,
    pressed: bool,
    scale: f32,
) {
    bg.rect(rect, if pressed { KEY_PRESSED } else { KEY_BG });
    bg.rect_outline(rect, scale.max(1.0), KEY_BORDER);
    let text_w = UiBatch::text_width(label, scale);
    fg.text(
        label,
        rect.x + (rect.w - text_w) / 2.0,
        rect.y + (rect.h - FONT_LINE * scale) / 2.0,
        scale,
        if pressed { KEY_PRESSED_TEXT } else { TEXT },
    );
}

/// The keystrokes module: W / A S D / space / shift, lit while held.
fn draw_keystrokes(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    origin: (f32, f32),
    values: &ModuleValues,
    scale: f32,
) {
    let s = |v: f32| v * scale;
    let unit = s(KEY_UNIT);
    let gap = s(KEY_GAP);
    let (x, y) = origin;

    bg.rect(
        Rect::new(
            x,
            y,
            s(KEY_UNIT * 3.0 + KEY_GAP * 2.0 + MODULE_PAD * 2.0),
            s(key_layout_height()),
        ),
        MODULE_PANEL,
    );
    let left = x + s(MODULE_PAD);
    let mut top = y + s(MODULE_PAD);
    let step = unit + gap;

    // Row 1: W, centred over S.
    draw_key(
        bg,
        fg,
        Rect::new(left + step, top, unit, unit),
        "W",
        values.keys.pressed(KeyBox::Forward),
        scale,
    );
    top += step;
    for (index, (label, key)) in [
        ("A", KeyBox::Left),
        ("S", KeyBox::Back),
        ("D", KeyBox::Right),
    ]
    .into_iter()
    .enumerate()
    {
        draw_key(
            bg,
            fg,
            Rect::new(left + index as f32 * step, top, unit, unit),
            label,
            values.keys.pressed(key),
            scale,
        );
    }
    top += step;
    draw_key(
        bg,
        fg,
        Rect::new(left, top, s(KEY_UNIT * 3.0 + KEY_GAP * 2.0), unit),
        "SPACE",
        values.keys.pressed(KeyBox::Jump),
        scale,
    );
    top += step;
    draw_key(
        bg,
        fg,
        Rect::new(left, top, s(KEY_UNIT * 3.0 + KEY_GAP * 2.0), unit),
        "SHIFT",
        values.keys.pressed(KeyBox::Sneak),
        scale,
    );
    // Mouse buttons last, split across the same width as the space bar.
    top += step;
    let half = s((KEY_UNIT * 3.0 + KEY_GAP) / 2.0);
    draw_key(
        bg,
        fg,
        Rect::new(left, top, half, unit),
        "LMB",
        values.keys.pressed(KeyBox::Attack),
        scale,
    );
    draw_key(
        bg,
        fg,
        Rect::new(left + half + gap, top, half, unit),
        "RMB",
        values.keys.pressed(KeyBox::UseItem),
        scale,
    );
}

/// Draw every enabled HUD module. Returns each module's rectangle so the editor
/// can hit-test a drag.
pub fn draw_modules(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    config: &ModuleConfig,
    values: &ModuleValues,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<(ModuleId, Rect)> {
    let s = |v: f32| v * scale;
    let mut rects = Vec::new();
    for id in ModuleId::ALL {
        let entry = config.get(id);
        if !entry.enabled {
            continue;
        }
        let (gui_w, gui_h) = module_size(id, values);
        let (w, h) = (s(gui_w), s(gui_h));
        // The anchor is the top-left; clamp so a dragged module never leaves
        // the window even if the resolution changed underneath it.
        let x = (entry.anchor[0] * width).clamp(0.0, (width - w).max(0.0));
        let y = (entry.anchor[1] * height).clamp(0.0, (height - h).max(0.0));
        let rect = Rect::new(x, y, w, h);
        if id == ModuleId::Keystrokes {
            draw_keystrokes(bg, fg, (rect.x, rect.y), values, scale);
            rects.push((id, rect));
            continue;
        }

        let (label, value, color) = module_text(id, values);
        bg.rect(rect, MODULE_PANEL);
        fg.text(
            &label,
            rect.x + s(MODULE_PAD),
            rect.y + s(MODULE_PAD),
            scale,
            MODULE_LABEL,
        );
        let value_w = UiBatch::text_width(&value, scale);
        fg.text(
            &value,
            rect.x + rect.w - s(MODULE_PAD) - value_w,
            rect.y + s(MODULE_PAD),
            scale,
            color,
        );
        rects.push((id, rect));
    }
    rects
}

/// The HUD module editor: a left-hand list of toggles over the live module
/// layout, so dragging a module moves the real thing.
pub fn draw_modules_editor(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    config: &ModuleConfig,
    values: &ModuleValues,
    scale: f32,
    width: f32,
    height: f32,
) -> (Vec<Button>, Vec<(ModuleId, Rect)>) {
    let s = |v: f32| v * scale;
    // A dim wash, light enough to keep the world (and the modules) readable.
    bg.rect(Rect::new(0.0, 0.0, width, height), [0.0, 0.0, 0.0, 0.35]);

    let mut buttons = Vec::new();
    let rows = ModuleId::ALL.len() as f32;
    let panel_w = s(210.0);
    let panel_h = s(30.0 + rows * 20.0 + 56.0);
    let panel = Rect::new(s(10.0), height / 2.0 - panel_h / 2.0, panel_w, panel_h);

    // Modules first, panel second: the panel must stay readable even when a
    // module is dragged underneath it, and the module rects are what the drag
    // hit-test uses.
    let rects = draw_modules(bg, fg, config, values, scale, width, height);
    for (_, rect) in &rects {
        bg.rect_outline(*rect, scale.max(1.0), SELECTION_BORDER);
    }

    bg.rect(panel, PANEL);
    bg.rect_outline(panel, scale.max(1.0), BUTTON_BORDER);
    fg.text_centered(
        "HUD Modules",
        panel.x + panel_w / 2.0,
        panel.y + s(7.0),
        scale,
        TEXT,
    );
    fg.text(
        "drag a module to move it",
        panel.x + s(10.0),
        panel.y + s(19.0),
        scale,
        TEXT_DIM,
    );

    for (index, id) in ModuleId::ALL.iter().enumerate() {
        let y = panel.y + s(32.0) + index as f32 * s(20.0);
        fg.text(id.label(), panel.x + s(10.0), y + s(4.0), scale, TEXT);
        let toggle = Rect::new(panel.x + panel_w - s(56.0), y, s(46.0), s(16.0));
        let enabled = config.is_enabled(*id);
        draw_button(
            bg,
            fg,
            toggle,
            if enabled { "ON" } else { "OFF" },
            scale,
            false,
        );
        buttons.push(Button {
            action: Action::ModuleToggle(*id),
            rect: toggle,
        });
    }

    let reset = Rect::new(
        panel.x + s(10.0),
        panel.y + panel_h - s(44.0),
        s(90.0),
        s(18.0),
    );
    draw_button(bg, fg, reset, "Reset", scale, false);
    buttons.push(Button {
        action: Action::ModulesReset,
        rect: reset,
    });
    let done = Rect::new(
        panel.x + panel_w - s(100.0),
        panel.y + panel_h - s(44.0),
        s(90.0),
        s(18.0),
    );
    draw_button(bg, fg, done, "Done", scale, false);
    buttons.push(Button {
        action: Action::ModulesDone,
        rect: done,
    });

    (buttons, rects)
}

/// `NEPHELE_DEBUG=1` restores the one-line footer readout that the modules
/// replaced.
fn debug_line_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("NEPHELE_DEBUG").is_some())
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
/// Vanilla's death screen: a red wash over the frozen game.
pub fn draw_death(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    pointer: Option<[f32; 2]>,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    bg.rect(Rect::new(0.0, 0.0, width, height), [0.35, 0.02, 0.02, 0.75]);
    let cx = width / 2.0;

    fg.text_centered("You Died!", cx, height * 0.3, scale * 2.5, TEXT);

    let w = 200.0 * scale;
    let h = 20.0 * scale;
    let respawn = Rect::new(cx - w / 2.0, height * 0.55, w, h);
    draw_button(
        bg,
        fg,
        respawn,
        "Respawn",
        scale,
        hovered(pointer, &respawn),
    );
    vec![Button {
        action: Action::Respawn,
        rect: respawn,
    }]
}
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
    let panel_h = s(30.0 + rows * 22.0 + 52.0);
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

    let modules = Rect::new(cx - s(90.0), panel.y + panel_h - s(52.0), s(180.0), s(20.0));
    draw_button(
        bg,
        fg,
        modules,
        "HUD Modules...",
        scale,
        hovered(hud.pointer, &modules),
    );
    buttons.push(Button {
        action: Action::OpenModules,
        rect: modules,
    });

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

/// One stack: an isometric block icon or flat item sprite, plus a count.
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
            let texture = item_texture_name(item.id, item.damage).unwrap_or(MISSING_TEXTURE);
            let layer = textures.slot_or_missing(texture);
            icons.quad(
                [
                    [inner.x, inner.y],
                    [inner.x + inner.w, inner.y],
                    [inner.x + inner.w, inner.y + inner.h],
                    [inner.x, inner.y + inner.h],
                ],
                [0.0, 0.0, 1.0, 1.0],
                TEXT,
                layer,
            );
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

/// Top-left of a crafting-table window slot (46 slots: output, 9 grid cells,
/// 27 storage, 9 hotbar) inside the same 176x166 panel.
pub fn window_slot_origin(slot: usize) -> Option<[f32; 2]> {
    Some(match slot {
        0 => [124.0, 35.0],
        1..=9 => {
            let index = slot - 1;
            [
                30.0 + (index % 3) as f32 * 18.0,
                17.0 + (index / 3) as f32 * 18.0,
            ]
        }
        10..=36 => {
            let index = slot - 10;
            [
                8.0 + (index % 9) as f32 * 18.0,
                84.0 + (index / 9) as f32 * 18.0,
            ]
        }
        37..=45 => [8.0 + (slot - 37) as f32 * 18.0, 142.0],
        _ => return None,
    })
}

pub fn window_slot_rect(slot: usize, scale: f32, width: f32, height: f32) -> Option<Rect> {
    let [x, y] = window_slot_origin(slot)?;
    let panel = inventory_panel(scale, width, height);
    Some(Rect::new(
        panel.x + x * scale,
        panel.y + y * scale,
        SLOT_SIZE * scale,
        SLOT_SIZE * scale,
    ))
}

/// Slots in a crafting-table window: output, a 3x3 grid, 27 storage, 9 hotbar.
pub const CRAFTING_WINDOW_SLOTS: usize = 46;

/// Every window has the player's own 36 slots (27 storage + 9 hotbar) at the
/// bottom; a container window is anything above them.
const PLAYER_WINDOW_SLOTS: usize = 36;
const WINDOW_PANEL_WIDTH: f32 = 176.0;
const WINDOW_GRID_TOP: f32 = 18.0;
const WINDOW_GRID_LEFT: f32 = 8.0;
const WINDOW_SLOT_PITCH: f32 = 18.0;
/// Gap between the container's grid and the player's storage rows.
const WINDOW_PLAYER_GAP: f32 = 14.0;

/// How many rows of nine the container part of a window needs.
///
/// A hopper (5 container slots) is one row; a double chest (54) is six. Slots
/// that are not a multiple of nine still get a row, so nothing is dropped.
pub fn window_rows(slots: usize) -> usize {
    slots.saturating_sub(PLAYER_WINDOW_SLOTS).div_ceil(9).max(1)
}

/// Panel size of a generic container window, in GUI pixels.
pub fn container_panel_size(slots: usize) -> [f32; 2] {
    let rows = window_rows(slots) as f32;
    let player_top = WINDOW_GRID_TOP + rows * WINDOW_SLOT_PITCH + WINDOW_PLAYER_GAP;
    [
        WINDOW_PANEL_WIDTH,
        player_top + WINDOW_SLOT_PITCH * 4.0 + WINDOW_GRID_LEFT,
    ]
}

fn container_panel(slots: usize, scale: f32, width: f32, height: f32) -> Rect {
    let [panel_w, panel_h] = container_panel_size(slots);
    Rect::new(
        width / 2.0 - panel_w * scale / 2.0,
        height / 2.0 - panel_h * scale / 2.0,
        panel_w * scale,
        panel_h * scale,
    )
}

/// Top-left of a slot in a generic server window of any row count, in GUI
/// pixels. Slots `0..slots-36` are the container's own, laid out nine to a row;
/// the last 36 are the player's storage and hotbar, exactly as in their own
/// inventory screen.
pub fn server_window_origin(slot: usize, slots: usize) -> Option<[f32; 2]> {
    if slot >= slots {
        return None;
    }
    let container = slots.saturating_sub(PLAYER_WINDOW_SLOTS);
    if slot < container {
        return Some([
            WINDOW_GRID_LEFT + (slot % 9) as f32 * WINDOW_SLOT_PITCH,
            WINDOW_GRID_TOP + (slot / 9) as f32 * WINDOW_SLOT_PITCH,
        ]);
    }
    let rows = window_rows(slots) as f32;
    let player_top = WINDOW_GRID_TOP + rows * WINDOW_SLOT_PITCH + WINDOW_PLAYER_GAP;
    let index = slot - container;
    Some(if index < 27 {
        [
            WINDOW_GRID_LEFT + (index % 9) as f32 * WINDOW_SLOT_PITCH,
            player_top + (index / 9) as f32 * WINDOW_SLOT_PITCH,
        ]
    } else {
        [
            WINDOW_GRID_LEFT + (index - 27) as f32 * WINDOW_SLOT_PITCH,
            player_top + WINDOW_SLOT_PITCH * 3.0 + 4.0,
        ]
    })
}

pub fn server_window_rect(
    slot: usize,
    slots: usize,
    scale: f32,
    width: f32,
    height: f32,
) -> Option<Rect> {
    let [x, y] = server_window_origin(slot, slots)?;
    let panel = container_panel(slots, scale, width, height);
    Some(Rect::new(
        panel.x + x * scale,
        panel.y + y * scale,
        SLOT_SIZE * scale,
        SLOT_SIZE * scale,
    ))
}

/// The window title to show for an `S2D` inventory type.
pub fn window_kind_label(kind: &str) -> &'static str {
    match kind {
        "minecraft:chest" => "Chest",
        "minecraft:crafting_table" => "Crafting Table",
        "minecraft:furnace" => "Furnace",
        "minecraft:dispenser" => "Dispenser",
        "minecraft:hopper" => "Hopper",
        "minecraft:enchanting_table" => "Enchanting",
        "minecraft:beacon" => "Beacon",
        "minecraft:anvil" => "Repair",
        "minecraft:brewing_stand" => "Brewing Stand",
        "minecraft:villager" => "Trading",
        _ => "Container",
    }
}

/// The survival inventory: 2x2 crafting, armour, storage and the hotbar, with
/// the stack under the cursor drawn last.
///
/// An open server window replaces the crafting half of the panel. A 46-slot
/// window is a crafting table and keeps the vanilla 3x3+output layout; anything
/// else is laid out generically, so a chest of any row count — or a hopper, or
/// a furnace — draws without a bespoke screen.
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

    let mut buttons = Vec::new();
    let draw_slots = |bg: &mut UiBatch,
                      fg: &mut UiBatch,
                      icons: &mut UiBatch,
                      slots: &[Option<ItemStack>],
                      rects: &[Option<Rect>],
                      buttons: &mut Vec<Button>| {
        for (slot, item) in slots.iter().enumerate() {
            let Some(rect) = rects.get(slot).copied().flatten() else {
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
            if let Some(item) = item {
                let inner = Rect::new(rect.x + s(1.0), rect.y + s(1.0), s(16.0), s(16.0));
                draw_stack(fg, icons, inner, *item, textures, scale);
            }
            buttons.push(Button {
                action: Action::InventorySlot(slot),
                rect,
            });
        }
    };

    match hud.window_slots {
        Some(slots) if slots.len() == CRAFTING_WINDOW_SLOTS => {
            let panel = inventory_panel(scale, width, height);
            bg.rect(panel, PANEL);
            bg.rect_outline(panel, scale.max(1.0), BUTTON_BORDER);
            fg.text_centered(
                "Crafting Table",
                width / 2.0,
                panel.y - s(13.0),
                scale,
                TEXT,
            );
            fg.text(
                "Crafting",
                panel.x + s(30.0),
                panel.y + s(6.0),
                scale,
                TEXT_DIM,
            );

            let rects: Vec<Option<Rect>> = (0..slots.len())
                .map(|slot| window_slot_rect(slot, scale, width, height))
                .collect();
            draw_slots(bg, fg, icons, slots, &rects, &mut buttons);
            fg.text_centered(
                "Click to move stacks  -  E / Esc to close",
                width / 2.0,
                panel.y + panel.h + s(6.0),
                scale,
                TEXT_DIM,
            );
        }
        Some(slots) => {
            let panel = container_panel(slots.len(), scale, width, height);
            bg.rect(panel, PANEL);
            bg.rect_outline(panel, scale.max(1.0), BUTTON_BORDER);
            let title = hud
                .window_kind
                .map(window_kind_label)
                .unwrap_or("Container");
            fg.text_centered(title, width / 2.0, panel.y - s(13.0), scale, TEXT);
            fg.text(
                "Inventory",
                panel.x + s(WINDOW_GRID_LEFT),
                panel.y + panel.h - s(76.0),
                scale,
                TEXT_DIM,
            );

            let rects: Vec<Option<Rect>> = (0..slots.len())
                .map(|slot| server_window_rect(slot, slots.len(), scale, width, height))
                .collect();
            draw_slots(bg, fg, icons, slots, &rects, &mut buttons);
            fg.text_centered(
                "Click to move stacks  -  E / Esc to close",
                width / 2.0,
                panel.y + panel.h + s(6.0),
                scale,
                TEXT_DIM,
            );
        }
        None => {
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

            let rects: Vec<Option<Rect>> = (0..hud.inventory.len())
                .map(|slot| inventory_slot_rect(slot, scale, width, height))
                .collect();
            draw_slots(bg, fg, icons, hud.inventory, &rects, &mut buttons);
            fg.text_centered(
                "Click to move stacks  -  E / Esc to close",
                width / 2.0,
                panel.y + panel.h + s(6.0),
                scale,
                TEXT_DIM,
            );
        }
    }

    if let (Some(item), Some([x, y])) = (cursor, hud.pointer) {
        let rect = Rect::new(x - s(8.0), y - s(8.0), s(16.0), s(16.0));
        draw_stack(fg, icons, rect, item, textures, scale);
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

/// How many saved worlds fit on the selection page.
pub const WORLD_ROWS: usize = 6;

/// The single-player save list. Clicking a row opens that world directly.
#[allow(clippy::too_many_arguments)]
pub fn draw_worlds(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    worlds: &[WorldMetadata],
    selected_directory: Option<&str>,
    scroll: usize,
    error: Option<&str>,
    pointer: Option<[f32; 2]>,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let s = |value: f32| value * scale;
    let cx = width / 2.0;
    bg.rect(Rect::new(0.0, 0.0, width, height), [0.08, 0.08, 0.08, 1.0]);

    let row_h = s(20.0);
    let list_h = s(4.0) + row_h * WORLD_ROWS as f32 + s(4.0);
    let block_h = s(12.0) + s(8.0) + list_h + s(8.0) + s(20.0) + s(4.0) + s(20.0) + s(8.0) + s(8.0);
    let top = ((height - block_h) / 2.0).max(s(2.0));
    let panel_w = s(360.0).min(width - s(8.0));
    let panel_x = cx - panel_w / 2.0;
    fg.text_centered("Select World", cx, top, scale * 1.5, TEXT);

    let list_rect = Rect::new(panel_x, top + s(20.0), panel_w, list_h);
    bg.rect(list_rect, PANEL);
    bg.rect_outline(list_rect, scale.max(1.0), BUTTON_BORDER);

    let visible = worlds.len().saturating_sub(scroll).min(WORLD_ROWS);
    if worlds.is_empty() {
        fg.text_centered(
            "No worlds saved yet - create one",
            cx,
            list_rect.y + list_h / 2.0 - s(4.0),
            scale,
            TEXT_DIM,
        );
    }

    let mut buttons = Vec::new();
    for slot in 0..visible {
        let index = scroll + slot;
        let world = &worlds[index];
        let row = Rect::new(
            list_rect.x + s(2.0),
            list_rect.y + s(4.0) + slot as f32 * row_h,
            panel_w - s(4.0),
            row_h - s(1.0),
        );
        let selected = selected_directory == Some(world.directory.as_str());
        bg.rect(
            row,
            if selected {
                ROW_SELECTED
            } else if hovered(pointer, &row) {
                ROW_HOVER
            } else if slot % 2 == 0 {
                ROW
            } else {
                ROW_ALT
            },
        );
        if selected {
            bg.rect_outline(row, scale.max(1.0), SELECTION_BORDER);
        }
        buttons.push(Button {
            action: Action::WorldSelect(index),
            rect: row,
        });

        let text_w = row.w - s(10.0);
        fg.text(
            &fit_text(&world.name, text_w, scale),
            row.x + s(5.0),
            row.y + s(2.0),
            scale,
            TEXT,
        );
        fg.text(
            &format!("Seed: {}  Mode: {}", world.seed, world.mode.as_str()),
            row.x + s(5.0),
            row.y + s(11.0),
            scale,
            TEXT_DIM,
        );
    }

    let first_button_y = list_rect.y + list_h + s(8.0);
    let button_w = (panel_w - s(6.0)) / 2.0;
    let play = Rect::new(panel_x, first_button_y, button_w, s(20.0));
    let new_world = Rect::new(
        panel_x + button_w + s(6.0),
        first_button_y,
        button_w,
        s(20.0),
    );
    let can_manage = selected_directory.is_some();
    draw_button_state(
        bg,
        fg,
        play,
        "Play Selected",
        scale,
        can_manage && hovered(pointer, &play),
        can_manage,
    );
    draw_button(
        bg,
        fg,
        new_world,
        "Create New World",
        scale,
        hovered(pointer, &new_world),
    );
    if can_manage {
        buttons.push(Button {
            action: Action::WorldOpenSelected,
            rect: play,
        });
    }
    buttons.push(Button {
        action: Action::WorldNew,
        rect: new_world,
    });

    let second_button_y = first_button_y + s(24.0);
    let small_button_w = (panel_w - s(12.0)) / 3.0;
    let rename = Rect::new(panel_x, second_button_y, small_button_w, s(20.0));
    let delete = Rect::new(
        panel_x + small_button_w + s(6.0),
        second_button_y,
        small_button_w,
        s(20.0),
    );
    let back = Rect::new(
        panel_x + (small_button_w + s(6.0)) * 2.0,
        second_button_y,
        small_button_w,
        s(20.0),
    );
    draw_button_state(
        bg,
        fg,
        rename,
        "Rename",
        scale,
        can_manage && hovered(pointer, &rename),
        can_manage,
    );
    draw_button_state(
        bg,
        fg,
        delete,
        "Delete",
        scale,
        can_manage && hovered(pointer, &delete),
        can_manage,
    );
    draw_button(bg, fg, back, "Back", scale, hovered(pointer, &back));
    if can_manage {
        buttons.push(Button {
            action: Action::WorldRenameSelected,
            rect: rename,
        });
        buttons.push(Button {
            action: Action::WorldDeleteSelected,
            rect: delete,
        });
    }
    buttons.push(Button {
        action: Action::WorldsBack,
        rect: back,
    });

    if let Some(error) = error {
        fg.text_centered(error, cx, second_button_y + s(28.0), scale, OFFLINE);
    } else {
        fg.text_centered(
            "Select a world, then choose an action",
            cx,
            second_button_y + s(28.0),
            scale,
            TEXT_DIM,
        );
    }
    buttons
}

/// World creation form. Text input itself is handled by `LiveApp`.
#[allow(clippy::too_many_arguments)]
pub fn draw_world_create(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    name: &str,
    seed: &str,
    mode: WorldMode,
    name_focused: bool,
    seed_focused: bool,
    error: Option<&str>,
    pointer: Option<[f32; 2]>,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let s = |value: f32| value * scale;
    let cx = width / 2.0;
    bg.rect(Rect::new(0.0, 0.0, width, height), [0.08, 0.08, 0.08, 1.0]);
    fg.text_centered("Create New World", cx, height * 0.16, scale * 1.5, TEXT);

    let field_w = s(300.0).min(width - s(16.0));
    let field_h = s(22.0);
    let field_x = cx - field_w / 2.0;
    let name_label_y = height * 0.28;
    let name_rect = Rect::new(field_x, name_label_y + s(10.0), field_w, field_h);
    let seed_label_y = height * 0.43;
    let seed_rect = Rect::new(field_x, seed_label_y + s(10.0), field_w, field_h);

    fg.text("World Name", field_x, name_label_y, scale, TEXT_DIM);
    fg.text(
        "Seed (blank for random)",
        field_x,
        seed_label_y,
        scale,
        TEXT_DIM,
    );
    for (rect, value, placeholder, focused) in [
        (name_rect, name, "World name", name_focused),
        (seed_rect, seed, "Random", seed_focused),
    ] {
        bg.rect(rect, [0.0, 0.0, 0.0, 0.78]);
        bg.rect_outline(
            rect,
            if focused { s(2.0) } else { scale.max(1.0) },
            if focused {
                SELECTION_BORDER
            } else {
                BUTTON_BORDER
            },
        );
        let text = if value.is_empty() { placeholder } else { value };
        let color = if value.is_empty() { TEXT_DIM } else { TEXT };
        fg.text(
            &fit_text(text, field_w - s(8.0), scale),
            rect.x + s(4.0),
            rect.y + s(7.0),
            scale,
            color,
        );
    }

    let mode_rect = Rect::new(cx - s(150.0), height * 0.61, s(300.0), field_h);
    draw_button(
        bg,
        fg,
        mode_rect,
        &format!("Game Mode: {}  (click to switch)", mode.as_str()),
        scale,
        hovered(pointer, &mode_rect),
    );

    let button_y = height * 0.75;
    let button_w = s(145.0);
    let create = Rect::new(cx - button_w - s(5.0), button_y, button_w, s(20.0));
    let cancel = Rect::new(cx + s(5.0), button_y, button_w, s(20.0));
    draw_button(bg, fg, create, "Create", scale, hovered(pointer, &create));
    draw_button(bg, fg, cancel, "Cancel", scale, hovered(pointer, &cancel));

    if let Some(error) = error {
        fg.text_centered(error, cx, button_y - s(14.0), scale, OFFLINE);
    }

    vec![
        Button {
            action: Action::WorldNameFocus,
            rect: name_rect,
        },
        Button {
            action: Action::WorldSeedFocus,
            rect: seed_rect,
        },
        Button {
            action: Action::WorldModeToggle,
            rect: mode_rect,
        },
        Button {
            action: Action::WorldCreateSubmit,
            rect: create,
        },
        Button {
            action: Action::WorldCreateCancel,
            rect: cancel,
        },
    ]
}

/// Rename form; the save service applies the same validation as world creation.
#[allow(clippy::too_many_arguments)]
pub fn draw_world_rename(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    name: &str,
    focused: bool,
    error: Option<&str>,
    pointer: Option<[f32; 2]>,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let s = |value: f32| value * scale;
    let cx = width / 2.0;
    bg.rect(Rect::new(0.0, 0.0, width, height), [0.08, 0.08, 0.08, 1.0]);
    fg.text_centered("Rename World", cx, height * 0.22, scale * 1.5, TEXT);

    let field_w = s(300.0).min(width - s(16.0));
    let field = Rect::new(cx - field_w / 2.0, height * 0.42, field_w, s(22.0));
    fg.text("World Name", field.x, field.y - s(12.0), scale, TEXT_DIM);
    bg.rect(field, [0.0, 0.0, 0.0, 0.78]);
    bg.rect_outline(
        field,
        if focused { s(2.0) } else { scale.max(1.0) },
        if focused {
            SELECTION_BORDER
        } else {
            BUTTON_BORDER
        },
    );
    let value = if name.is_empty() { "World name" } else { name };
    fg.text(
        &fit_text(value, field_w - s(8.0), scale),
        field.x + s(4.0),
        field.y + s(7.0),
        scale,
        if name.is_empty() { TEXT_DIM } else { TEXT },
    );

    let button_y = height * 0.62;
    let button_w = s(145.0);
    let save = Rect::new(cx - button_w - s(5.0), button_y, button_w, s(20.0));
    let cancel = Rect::new(cx + s(5.0), button_y, button_w, s(20.0));
    draw_button(bg, fg, save, "Save Name", scale, hovered(pointer, &save));
    draw_button(bg, fg, cancel, "Cancel", scale, hovered(pointer, &cancel));
    if let Some(error) = error {
        fg.text_centered(error, cx, button_y + s(30.0), scale, OFFLINE);
    }

    vec![
        Button {
            action: Action::WorldRenameFocus,
            rect: field,
        },
        Button {
            action: Action::WorldRenameSubmit,
            rect: save,
        },
        Button {
            action: Action::WorldRenameCancel,
            rect: cancel,
        },
    ]
}

/// Delete is a separate, explicit confirmation step.
#[allow(clippy::too_many_arguments)]
pub fn draw_world_delete_confirm(
    bg: &mut UiBatch,
    fg: &mut UiBatch,
    name: &str,
    error: Option<&str>,
    pointer: Option<[f32; 2]>,
    scale: f32,
    width: f32,
    height: f32,
) -> Vec<Button> {
    let s = |value: f32| value * scale;
    let cx = width / 2.0;
    let panel_w = s(300.0).min(width - s(16.0));
    let panel_h = s(104.0);
    let panel = Rect::new(
        cx - panel_w / 2.0,
        (height - panel_h) / 2.0,
        panel_w,
        panel_h,
    );
    bg.rect(Rect::new(0.0, 0.0, width, height), [0.0, 0.0, 0.0, 0.82]);
    bg.rect(panel, PANEL);
    bg.rect_outline(panel, scale.max(1.0), BUTTON_BORDER);
    fg.text_centered("Delete World?", cx, panel.y + s(12.0), scale * 1.4, TEXT);
    fg.text_centered(
        &fit_text(
            &format!("Delete \"{name}\"? This cannot be undone."),
            panel_w - s(16.0),
            scale,
        ),
        cx,
        panel.y + s(38.0),
        scale,
        TEXT_DIM,
    );

    let button_w = (panel_w - s(18.0)) / 2.0;
    let button_y = panel.y + panel_h - s(26.0);
    let delete = Rect::new(panel.x + s(4.0), button_y, button_w, s(20.0));
    let cancel = Rect::new(panel.x + s(14.0) + button_w, button_y, button_w, s(20.0));
    draw_button(bg, fg, delete, "Delete", scale, hovered(pointer, &delete));
    draw_button(bg, fg, cancel, "Cancel", scale, hovered(pointer, &cancel));
    if let Some(error) = error {
        fg.text_centered(error, cx, panel.y + panel_h + s(8.0), scale, OFFLINE);
    }
    vec![
        Button {
            action: Action::WorldDeleteConfirm,
            rect: delete,
        },
        Button {
            action: Action::WorldDeleteCancel,
            rect: cancel,
        },
    ]
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
    fn damage_vignette_is_four_fading_edge_quads() {
        let mut batch = UiBatch::default();
        draw_damage_vignette(&mut batch, 100.0, 80.0, 0.0);
        assert!(batch.is_empty());

        draw_damage_vignette(&mut batch, 100.0, 80.0, 1.0);
        assert_eq!(batch.vertices.len(), 16);
        assert_eq!(batch.indices.len(), 24);
        assert!(batch.vertices.iter().any(|vertex| vertex.color[3] > 0.0));
        assert!(batch.vertices.iter().any(|vertex| vertex.color[3] == 0.0));
    }

    #[test]
    fn gui_scale_is_integral_and_bounded() {
        assert_eq!(gui_scale(1280.0, 720.0), 3.0);
        assert_eq!(gui_scale(320.0, 240.0), 1.0);
        assert_eq!(gui_scale(3840.0, 2160.0), 4.0);
        assert_eq!(gui_scale(100.0, 100.0), 1.0);
    }

    #[test]
    fn world_list_pages_rows_and_selects_the_clicked_world() {
        let worlds: Vec<WorldMetadata> = (0..WORLD_ROWS + 2)
            .map(|index| WorldMetadata {
                directory: format!("World {index}"),
                name: format!("World {index}"),
                seed: index as u64,
                mode: if index % 2 == 0 {
                    WorldMode::Creative
                } else {
                    WorldMode::Survival
                },
            })
            .collect();
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        let first = draw_worlds(
            &mut bg, &mut fg, &worlds, None, 0, None, None, 2.0, 1280.0, 720.0,
        );
        assert!(first
            .iter()
            .any(|button| button.action == Action::WorldSelect(0)));
        assert!(first
            .iter()
            .any(|button| button.action == Action::WorldSelect(WORLD_ROWS - 1)));
        assert!(!first
            .iter()
            .any(|button| button.action == Action::WorldSelect(WORLD_ROWS)));
        assert!(!first
            .iter()
            .any(|button| button.action == Action::WorldOpenSelected));
        let first_row = rect_of(&first, Action::WorldSelect(0));
        assert_eq!(
            hit_test(
                &first,
                [
                    first_row.x + first_row.w / 2.0,
                    first_row.y + first_row.h / 2.0
                ]
            ),
            Some(Action::WorldSelect(0))
        );

        let last = draw_worlds(
            &mut bg,
            &mut fg,
            &worlds,
            Some("World 3"),
            2,
            None,
            None,
            2.0,
            1280.0,
            720.0,
        );
        assert!(last
            .iter()
            .any(|button| button.action == Action::WorldSelect(WORLD_ROWS + 1)));
        assert!(!last
            .iter()
            .any(|button| button.action == Action::WorldSelect(1)));
        assert!(last
            .iter()
            .any(|button| button.action == Action::WorldOpenSelected));
        assert!(last.iter().any(|button| button.action == Action::WorldNew));
        assert!(last
            .iter()
            .any(|button| button.action == Action::WorldRenameSelected));
        assert!(last
            .iter()
            .any(|button| button.action == Action::WorldDeleteSelected));
        assert!(last
            .iter()
            .any(|button| button.action == Action::WorldsBack));
    }

    #[test]
    fn world_creation_form_exposes_both_fields_mode_and_submit_actions() {
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        let buttons = draw_world_create(
            &mut bg,
            &mut fg,
            "Build",
            "12345",
            WorldMode::Survival,
            true,
            false,
            None,
            None,
            2.0,
            1280.0,
            720.0,
        );
        for action in [
            Action::WorldNameFocus,
            Action::WorldSeedFocus,
            Action::WorldModeToggle,
            Action::WorldCreateSubmit,
            Action::WorldCreateCancel,
        ] {
            rect_of(&buttons, action);
        }
        let name = rect_of(&buttons, Action::WorldNameFocus);
        let seed = rect_of(&buttons, Action::WorldSeedFocus);
        assert!(name.y + name.h <= seed.y);
        assert!(!bg.is_empty());
        assert!(!fg.is_empty());
    }

    #[test]
    fn rename_and_delete_screens_have_separate_submit_and_cancel_actions() {
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        let rename = draw_world_rename(
            &mut bg, &mut fg, "World", true, None, None, 2.0, 1280.0, 720.0,
        );
        for action in [
            Action::WorldRenameFocus,
            Action::WorldRenameSubmit,
            Action::WorldRenameCancel,
        ] {
            rect_of(&rename, action);
        }

        let confirm =
            draw_world_delete_confirm(&mut bg, &mut fg, "World", None, None, 2.0, 1280.0, 720.0);
        let delete = rect_of(&confirm, Action::WorldDeleteConfirm);
        let cancel = rect_of(&confirm, Action::WorldDeleteCancel);
        assert!(delete.x + delete.w <= cancel.x);
        assert!(!bg.is_empty());
        assert!(!fg.is_empty());
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

    #[test]
    fn non_block_inventory_items_use_flat_item_texture_layers() {
        let textures = TextureArray::from_tiles(vec![(
            "item:stick".to_string(),
            image::RgbaImage::from_pixel(16, 16, image::Rgba([180, 130, 80, 255])),
        )]);
        let mut fg = UiBatch::default();
        let mut icons = UiBatch::default();
        draw_stack(
            &mut fg,
            &mut icons,
            Rect::new(4.0, 8.0, 16.0, 16.0),
            ItemStack::new(280, 1, 0),
            &textures,
            1.0,
        );
        let layer = textures.slot("item:stick").unwrap();
        assert_eq!(icons.vertices.len(), 4);
        assert!(icons.vertices.iter().all(|vertex| vertex.layer == layer));
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
            window_slots: None,
            window_kind: None,
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
            SettingId::ALL.len() * 2 + 2,
            "two arrows per setting, then HUD Modules and Done"
        );
        assert!(buttons.iter().any(|b| b.action == Action::SettingsDone));
        assert!(buttons.iter().any(|b| b.action == Action::OpenModules));
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
    fn crafting_table_slots_cover_the_grid_and_the_player_half() {
        assert_eq!(window_slot_origin(0), Some([124.0, 35.0]));
        assert_eq!(window_slot_origin(1), Some([30.0, 17.0]));
        assert_eq!(window_slot_origin(9), Some([66.0, 53.0]));
        assert_eq!(window_slot_origin(10), Some([8.0, 84.0]));
        assert_eq!(window_slot_origin(36), Some([152.0, 120.0]));
        assert_eq!(window_slot_origin(37), Some([8.0, 142.0]));
        assert_eq!(window_slot_origin(45), Some([152.0, 142.0]));
        assert_eq!(window_slot_origin(46), None);

        let (width, height, scale) = (1280.0, 720.0, 3.0);
        let panel = inventory_panel(scale, width, height);
        let mut rects: Vec<Rect> = Vec::new();
        for slot in 0..46 {
            let rect = window_slot_rect(slot, scale, width, height).expect("slot exists");
            assert!(rect.x >= panel.x && rect.y >= panel.y);
            assert!(rect.x + rect.w <= panel.x + panel.w);
            assert!(rect.y + rect.h <= panel.y + panel.h);
            for other in &rects {
                let overlap = rect.x < other.x + other.w
                    && other.x < rect.x + rect.w
                    && rect.y < other.y + other.h
                    && other.y < rect.y + rect.h;
                assert!(!overlap, "crafting table slots overlap");
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
        let hello = crate::chat::parse("§ahello");
        let fading = crate::chat::parse("fading");
        let lines: Vec<(&[crate::chat::Span], f32)> =
            vec![(hello.as_slice(), 1.0), (fading.as_slice(), 0.4)];
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

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    fn module_values() -> ModuleValues {
        ModuleValues {
            fps: 143.0,
            ping: Some(42),
            cps: (5, 2),
            position: [-52.5, 96.0, 0.5],
            speed: 4.3,
            keys: crate::hud_modules::KeyState::default(),
        }
    }

    #[test]
    fn a_generic_window_lays_out_any_row_count() {
        for slots in [41, 45, 63, 90] {
            // 5 container slots (hopper) up to 54 (double chest), plus the 36
            // player slots.
            let rects: Vec<Rect> = (0..slots)
                .map(|slot| server_window_rect(slot, slots, 2.0, 1280.0, 720.0).unwrap())
                .collect();
            assert_eq!(rects.len(), slots);
            for (index, rect) in rects.iter().enumerate() {
                for other in &rects[index + 1..] {
                    assert!(
                        !overlaps(*rect, *other),
                        "{slots} slots: {index} overlaps a neighbour"
                    );
                }
            }
            // Inside the panel it was placed from.
            let [panel_w, panel_h] = container_panel_size(slots);
            let panel = container_panel(slots, 2.0, 1280.0, 720.0);
            for rect in &rects {
                assert!(rect.x >= panel.x && rect.y >= panel.y);
                assert!(rect.x + rect.w <= panel.x + panel_w * 2.0 + 0.01);
                assert!(rect.y + rect.h <= panel.y + panel_h * 2.0 + 0.01);
            }
        }
    }

    #[test]
    fn a_six_row_chest_needs_more_panel_than_a_hopper() {
        assert_eq!(window_rows(41), 1, "a hopper: 5 container slots");
        assert_eq!(window_rows(45), 1, "nine container slots");
        assert_eq!(window_rows(63), 3, "a single chest");
        assert_eq!(window_rows(90), 6, "a double chest");
        assert!(container_panel_size(90)[1] > container_panel_size(45)[1]);
    }

    #[test]
    fn the_sidebar_draws_a_panel_and_every_row() {
        let board = crate::scoreboard::Scoreboard::demo();
        let objective = board.sidebar.as_ref().unwrap();
        let rows = board.sidebar_rows();
        let list = ScoreboardRowList {
            title: Some(objective.title.as_slice()),
            rows: &rows,
        };
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        let layout = draw_sidebar(&mut bg, &mut fg, &list, 2.0, 1280.0, 720.0).unwrap();

        assert_eq!(bg.indices.len() / 6, 1, "one pane behind the column");
        assert!(!fg.is_empty());
        // Right-aligned against the screen edge, inside the window.
        assert!((layout.x + layout.width - (1280.0 / 2.0 - 2.0)).abs() < 0.01);
        assert!(layout.y > 0.0 && layout.y + layout.height < 720.0 / 2.0);
    }

    #[test]
    fn only_enabled_modules_are_drawn_and_they_do_not_overlap() {
        let config = ModuleConfig::default();
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        let rects = draw_modules(
            &mut bg,
            &mut fg,
            &config,
            &module_values(),
            2.0,
            1280.0,
            720.0,
        );
        assert_eq!(rects.len(), ModuleId::ALL.len());
        for (index, (_, rect)) in rects.iter().enumerate() {
            assert!(rect.x >= 0.0 && rect.y >= 0.0);
            assert!(rect.x + rect.w <= 1280.0 && rect.y + rect.h <= 720.0);
            for (_, other) in &rects[index + 1..] {
                assert!(
                    !overlaps(*rect, *other),
                    "two default module positions overlap"
                );
            }
        }

        let mut hidden = ModuleConfig::default();
        hidden.toggle(ModuleId::Cps);
        let rects = draw_modules(
            &mut UiBatch::default(),
            &mut UiBatch::default(),
            &hidden,
            &module_values(),
            2.0,
            1280.0,
            720.0,
        );
        assert_eq!(rects.len(), ModuleId::ALL.len() - 1);
        assert!(!rects.iter().any(|(id, _)| *id == ModuleId::Cps));
    }

    #[test]
    fn the_default_layout_clears_the_sidebar_and_the_hotbar() {
        let board = crate::scoreboard::Scoreboard::demo();
        let objective = board.sidebar.as_ref().unwrap();
        let rows = board.sidebar_rows();
        let list = ScoreboardRowList {
            title: Some(objective.title.as_slice()),
            rows: &rows,
        };
        // 1024x768 is left out on purpose: at GUI scale 3 a fifteen-row sidebar
        // takes 438 of 1024 px, so nothing fits beside it. Dragging a module
        // there is the user's call, not a default worth shipping.
        for (width, height) in [
            (1280.0, 720.0),
            (1366.0, 768.0),
            (1600.0, 900.0),
            (1920.0, 1080.0),
        ] {
            let scale = gui_scale(width, height);
            let mut bg = UiBatch::default();
            let mut fg = UiBatch::default();
            let layout = draw_sidebar(&mut bg, &mut fg, &list, scale, width, height).unwrap();
            let sidebar = Rect::new(
                layout.x * scale,
                layout.y * scale,
                layout.width * scale,
                layout.height * scale,
            );
            let modules = draw_modules(
                &mut UiBatch::default(),
                &mut UiBatch::default(),
                &ModuleConfig::default(),
                &module_values(),
                scale,
                width,
                height,
            );
            for (id, rect) in &modules {
                assert!(
                    !overlaps(*rect, sidebar),
                    "{id:?} covers the scoreboard at {width}x{height}: module {rect:?} vs sidebar {sidebar:?} scale {scale}"
                );
                // The hotbar is the bottom 22 GUI pixels, centred.
                let hotbar = Rect::new(
                    width / 2.0 - 91.0 * scale,
                    height - 22.0 * scale,
                    182.0 * scale,
                    22.0 * scale,
                );
                assert!(
                    !overlaps(*rect, hotbar),
                    "{id:?} covers the hotbar at {width}x{height}"
                );
            }
        }
    }

    #[test]
    fn the_module_editor_lists_every_module_and_shows_the_live_positions() {
        let mut bg = UiBatch::default();
        let mut fg = UiBatch::default();
        let (buttons, rects) = draw_modules_editor(
            &mut bg,
            &mut fg,
            &ModuleConfig::default(),
            &module_values(),
            2.0,
            1280.0,
            720.0,
        );
        assert_eq!(
            buttons.len(),
            ModuleId::ALL.len() + 2,
            "toggles, Reset, Done"
        );
        assert_eq!(rects.len(), ModuleId::ALL.len());
        assert!(buttons.iter().any(|b| b.action == Action::ModulesDone));
        assert!(buttons
            .iter()
            .any(|b| b.action == Action::ModuleToggle(ModuleId::Ping)));

        // The Reset/Done row must clear the toggle rows above it.
        let reset = buttons
            .iter()
            .find(|b| b.action == Action::ModulesReset)
            .unwrap()
            .rect;
        let done = buttons
            .iter()
            .find(|b| b.action == Action::ModulesDone)
            .unwrap()
            .rect;
        assert!(!overlaps(reset, done));
        for button in &buttons {
            if matches!(button.action, Action::ModuleToggle(_)) {
                assert!(!overlaps(button.rect, reset), "{:?}", button.action);
                assert!(!overlaps(button.rect, done), "{:?}", button.action);
            }
        }
    }
}

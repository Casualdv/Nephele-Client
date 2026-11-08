//! Chat text colours: vanilla's legacy `§` dialect and JSON chat components.
//!
//! `S02 Chat Message` carries a JSON chat component, but `flatten_chat`
//! (`nephele-protocol`) reduces it to plain text before the client sees it, and
//! many servers — Hypixel among them — embed `§` codes inside the `text`
//! fields anyway. This module is the display half: it turns either dialect into
//! coloured runs the bitmap font can draw. It deliberately does not parse
//! packets.
//!
//! Formatting codes (`§l` bold, `§o` italic, `§n` underline, `§m` strike,
//! `§k` obfuscated) are consumed and dropped: the 1.8 bitmap font has a single
//! weight and no underline pass, so drawing them would be a lie.

use serde_json::Value;

/// One run of chat text sharing a single colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub text: String,
    pub color: [f32; 4],
}

impl Span {
    pub fn new(text: impl Into<String>, color: [f32; 4]) -> Self {
        Self {
            text: text.into(),
            color,
        }
    }
}

/// A chat line: the runs, in drawing order.
pub type Spans = Vec<Span>;

/// The vanilla chat palette, `§0`..`§f`.
pub const PALETTE: [[f32; 4]; 16] = [
    [0.0, 0.0, 0.0, 1.0],       // 0 black
    [0.0, 0.0, 0.667, 1.0],     // 1 dark blue
    [0.0, 0.667, 0.0, 1.0],     // 2 dark green
    [0.0, 0.667, 0.667, 1.0],   // 3 dark aqua
    [0.667, 0.0, 0.0, 1.0],     // 4 dark red
    [0.667, 0.0, 0.667, 1.0],   // 5 dark purple
    [1.0, 0.667, 0.0, 1.0],     // 6 gold
    [0.667, 0.667, 0.667, 1.0], // 7 grey
    [0.333, 0.333, 0.333, 1.0], // 8 dark grey
    [0.333, 0.333, 1.0, 1.0],   // 9 blue
    [0.333, 1.0, 0.333, 1.0],   // a green
    [0.333, 1.0, 1.0, 1.0],     // b aqua
    [1.0, 0.333, 0.333, 1.0],   // c red
    [1.0, 0.333, 1.0, 1.0],     // d light purple
    [1.0, 1.0, 0.333, 1.0],     // e yellow
    [1.0, 1.0, 1.0, 1.0],       // f white
];

pub const WHITE: [f32; 4] = PALETTE[15];

/// Colour named by a `§` code `0`..`f`, or `None` for anything else.
pub fn palette(index: u8) -> Option<[f32; 4]> {
    PALETTE.get(index as usize).copied()
}

/// Colour named by a JSON component (`{"color":"dark_aqua"}`).
pub fn named(name: &str) -> Option<[f32; 4]> {
    Some(match name {
        "black" => PALETTE[0],
        "dark_blue" => PALETTE[1],
        "dark_green" => PALETTE[2],
        "dark_aqua" => PALETTE[3],
        "dark_red" => PALETTE[4],
        "dark_purple" => PALETTE[5],
        "gold" => PALETTE[6],
        "gray" | "grey" => PALETTE[7],
        "dark_gray" | "dark_grey" => PALETTE[8],
        "blue" => PALETTE[9],
        "green" => PALETTE[10],
        "aqua" => PALETTE[11],
        "red" => PALETTE[12],
        "light_purple" => PALETTE[13],
        "yellow" => PALETTE[14],
        "white" => PALETTE[15],
        _ => return hex_color(name),
    })
}

/// `"#rrggbb"`, as a JSON component may name an arbitrary colour.
fn hex_color(text: &str) -> Option<[f32; 4]> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&digits[range], 16)
            .ok()
            .map(|v| v as f32 / 255.0)
    };
    Some([channel(0..2)?, channel(2..4)?, channel(4..6)?, 1.0])
}

/// Parse whatever a chat line actually contains.
///
/// A line that is a JSON component (starts with `{` or `[`) is parsed as one;
/// anything else is treated as the legacy `§` dialect. Being tolerant here is
/// what lets this module work today (flattened `§` text) and keep working when
/// the net lane starts forwarding the raw component.
pub fn parse(raw: &str) -> Spans {
    let trimmed = raw.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            return parse_component(&value);
        }
    }
    parse_legacy(raw, WHITE)
}

/// Parse a JSON chat component, inheriting colour down the tree exactly as
/// `IChatComponent` styles do.
pub fn parse_component(value: &Value) -> Spans {
    parse_component_with(value, WHITE)
}

fn parse_component_with(value: &Value, inherited: [f32; 4]) -> Spans {
    let mut out = Spans::new();
    walk(value, inherited, &mut out);
    out
}

fn walk(value: &Value, inherited: [f32; 4], out: &mut Spans) {
    match value {
        Value::Null => {}
        Value::String(text) => parse_legacy_into(text, inherited, out),
        Value::Number(number) => push(out, &number.to_string(), inherited),
        Value::Bool(flag) => push(out, &flag.to_string(), inherited),
        Value::Array(items) => {
            for item in items {
                walk(item, inherited, out);
            }
        }
        Value::Object(map) => {
            // A child without its own colour inherits the parent's, which is
            // how `{"color":"red","extra":[{"text":"x"}]}` renders red.
            let mut color = inherited;
            if let Some(name) = map.get("color").and_then(Value::as_str) {
                color = named(name).unwrap_or(inherited);
            }
            // `translate` renders its key plus the `with` arguments in vanilla;
            // showing the key keeps the same information without a lang table.
            if let Some(Value::String(text)) = map.get("text") {
                parse_legacy_into(text, color, out);
            } else if let Some(Value::String(key)) = map.get("translate") {
                push(out, key, color);
            } else if let Some(Value::String(key)) = map.get("keybind") {
                push(out, key, color);
            } else if let Some(Value::String(selector)) = map.get("selector") {
                push(out, selector, color);
            }
            for field in ["with", "extra"] {
                if let Some(Value::Array(items)) = map.get(field) {
                    for item in items {
                        walk(item, color, out);
                    }
                }
            }
        }
    }
}

/// Expand `§` codes into coloured runs starting from `base`.
pub fn parse_legacy(text: &str, base: [f32; 4]) -> Spans {
    let mut out = Spans::new();
    parse_legacy_into(text, base, &mut out);
    out
}

fn parse_legacy_into(text: &str, base: [f32; 4], out: &mut Spans) {
    let mut color = base;
    let mut buffer = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '§' {
            buffer.push(ch);
            continue;
        }
        // `§` takes the next character with it, valid or not, exactly like the
        // vanilla font renderer. A trailing `§` is simply dropped.
        let Some(code) = chars.next() else { break };
        let flushed = buffer.len();
        if flushed > 0 {
            push(out, &buffer, color);
            buffer.clear();
        }
        match code.to_ascii_lowercase() {
            digit @ '0'..='9' => {
                if let Some(value) = palette(digit as u8 - b'0') {
                    color = value;
                }
            }
            letter @ 'a'..='f' => {
                if let Some(value) = palette(letter as u8 - b'a' + 10) {
                    color = value;
                }
            }
            'r' => color = WHITE,
            // Bold/italic/underline/strike/obfuscated: consumed, not drawn.
            'k' | 'l' | 'm' | 'n' | 'o' => {}
            _ => {}
        }
    }
    if !buffer.is_empty() {
        push(out, &buffer, color);
    }
}

/// Append `text`, merging into the previous run when the colour matches so a
/// line stays a handful of spans instead of one per component.
fn push(out: &mut Spans, text: &str, color: [f32; 4]) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.color == color {
            last.text.push_str(text);
            return;
        }
    }
    out.push(Span::new(text, color));
}

/// The plain text of a line, for the console and for tests.
pub fn plain(spans: &Spans) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(spans: &Spans) -> Vec<[f32; 4]> {
        spans.iter().map(|span| span.color).collect()
    }

    #[test]
    fn hypixel_style_codes_split_into_coloured_runs() {
        // The exact line that rendered as raw text at Hypixel spawn.
        let spans = parse("§b[MVP§c+§b] Affieeyy§f §6joined the lobby!");
        assert_eq!(
            plain(&spans),
            "[MVP+] Affieeyy joined the lobby!",
            "codes are consumed, text is kept"
        );
        assert_eq!(
            colors(&spans),
            vec![
                PALETTE[11],
                PALETTE[12],
                PALETTE[11],
                PALETTE[15],
                PALETTE[6]
            ]
        );
    }

    #[test]
    fn adjacent_runs_of_one_colour_merge() {
        let spans = parse_legacy("§aone§atwo", WHITE);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].text, "onetwo");
    }

    #[test]
    fn reset_returns_to_white() {
        let spans = parse_legacy("§cred§rwhite", WHITE);
        assert_eq!(colors(&spans), vec![PALETTE[12], WHITE]);
    }

    #[test]
    fn formatting_codes_are_consumed_not_drawn() {
        let spans = parse_legacy("§lBOLD§r plain", WHITE);
        assert_eq!(plain(&spans), "BOLD plain");
        assert_eq!(colors(&spans), vec![WHITE]);
    }

    #[test]
    fn a_dangling_code_character_does_not_panic() {
        assert_eq!(plain(&parse_legacy("trailing§", WHITE)), "trailing");
        assert_eq!(plain(&parse_legacy("§", WHITE)), "");
    }

    #[test]
    fn json_components_inherit_colour() {
        let value: Value = serde_json::from_str(
            r#"{"text":"a","color":"red","extra":[{"text":"b"},{"text":"c","color":"aqua"}]}"#,
        )
        .unwrap();
        let spans = parse_component(&value);
        assert_eq!(plain(&spans), "abc");
        assert_eq!(colors(&spans), vec![PALETTE[12], PALETTE[11]]);
    }

    #[test]
    fn json_arrays_and_nested_extra_flatten_in_order() {
        let value: Value =
            serde_json::from_str(r#"[{"text":"one "},{"text":"two","extra":[{"text":" three"}]}]"#)
                .unwrap();
        assert_eq!(plain(&parse_component(&value)), "one two three");
    }

    #[test]
    fn json_hex_colours_and_legacy_codes_inside_text() {
        let value: Value =
            serde_json::from_str(r##"{"text":"§aGreen","color":"#102030"}"##).unwrap();
        let spans = parse_component(&value);
        assert_eq!(plain(&spans), "Green");
        // The `§a` inside `text` wins over the component's own colour, which is
        // what the server that wrote it intended.
        assert_eq!(colors(&spans), vec![PALETTE[10]]);
    }

    #[test]
    fn parse_accepts_both_dialects() {
        assert_eq!(plain(&parse("[{\"text\":\"json\"}]")), "json");
        assert_eq!(plain(&parse("§aplain")), "plain");
        // A line that only looks like JSON falls back to legacy parsing.
        assert_eq!(plain(&parse("{not json")), "{not json");
    }
}

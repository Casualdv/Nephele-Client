//! A built-in 3x5 bitmap font, for the launcher's first run.
//!
//! The launcher normally draws with the vanilla bitmap font from the prepared
//! resources. Before those exist there is nothing to sample, so the setup
//! screen — the one screen that has to work when preparation failed — uses
//! this small block font instead. It is deliberately not a copy of any game
//! asset: every glyph is defined here as three columns of five bits.

use nephele_render::{Rect, UiBatch};

const GLYPH_HEIGHT: usize = 5;
const GLYPH_WIDTH: usize = 3;
/// One blank column between glyphs.
const ADVANCE: usize = GLYPH_WIDTH + 1;
const LINE_ADVANCE: usize = GLYPH_HEIGHT + 1;

/// Rows for `ch`, uppercased. `None` means the caller should draw `?`.
fn glyph(ch: char) -> Option<[&'static str; GLYPH_HEIGHT]> {
    let rows = match ch.to_ascii_uppercase() {
        ' ' => ["000", "000", "000", "000", "000"],
        'A' => ["010", "101", "111", "101", "101"],
        'B' => ["110", "101", "110", "101", "110"],
        'C' => ["011", "100", "100", "100", "011"],
        'D' => ["110", "101", "101", "101", "110"],
        'E' => ["111", "100", "110", "100", "111"],
        'F' => ["111", "100", "110", "100", "100"],
        'G' => ["011", "100", "101", "101", "011"],
        'H' => ["101", "101", "111", "101", "101"],
        'I' => ["111", "010", "010", "010", "111"],
        'J' => ["001", "001", "001", "101", "010"],
        'K' => ["101", "101", "110", "101", "101"],
        'L' => ["100", "100", "100", "100", "111"],
        'M' => ["101", "111", "111", "101", "101"],
        'N' => ["110", "101", "101", "101", "101"],
        'O' => ["010", "101", "101", "101", "010"],
        'P' => ["110", "101", "110", "100", "100"],
        'Q' => ["010", "101", "101", "111", "011"],
        'R' => ["110", "101", "110", "101", "101"],
        'S' => ["011", "100", "010", "001", "110"],
        'T' => ["111", "010", "010", "010", "010"],
        'U' => ["101", "101", "101", "101", "111"],
        'V' => ["101", "101", "101", "101", "010"],
        'W' => ["101", "101", "111", "111", "101"],
        'X' => ["101", "101", "010", "101", "101"],
        'Y' => ["101", "101", "010", "010", "010"],
        'Z' => ["111", "001", "010", "100", "111"],
        '0' => ["111", "101", "101", "101", "111"],
        '1' => ["010", "110", "010", "010", "111"],
        '2' => ["110", "001", "010", "100", "111"],
        '3' => ["110", "001", "010", "001", "110"],
        '4' => ["101", "101", "111", "001", "001"],
        '5' => ["111", "100", "110", "001", "110"],
        '6' => ["011", "100", "111", "101", "111"],
        '7' => ["111", "001", "010", "010", "010"],
        '8' => ["111", "101", "111", "101", "111"],
        '9' => ["111", "101", "111", "001", "110"],
        '.' => ["000", "000", "000", "000", "010"],
        ',' => ["000", "000", "000", "010", "100"],
        ':' => ["000", "010", "000", "010", "000"],
        ';' => ["000", "010", "000", "010", "100"],
        '-' => ["000", "000", "111", "000", "000"],
        '_' => ["000", "000", "000", "000", "111"],
        '+' => ["000", "010", "111", "010", "000"],
        '=' => ["000", "111", "000", "111", "000"],
        '/' => ["001", "001", "010", "100", "100"],
        '\\' => ["100", "100", "010", "001", "001"],
        '(' => ["001", "010", "010", "010", "001"],
        ')' => ["100", "010", "010", "010", "100"],
        '[' => ["011", "010", "010", "010", "011"],
        ']' => ["110", "010", "010", "010", "110"],
        '<' => ["001", "010", "100", "010", "001"],
        '>' => ["100", "010", "001", "010", "100"],
        '!' => ["010", "010", "010", "000", "010"],
        '?' => ["110", "001", "010", "000", "010"],
        '\'' => ["010", "010", "000", "000", "000"],
        '"' => ["101", "101", "000", "000", "000"],
        '*' => ["101", "010", "111", "010", "101"],
        '#' => ["101", "111", "101", "111", "101"],
        '%' => ["101", "001", "010", "100", "101"],
        '&' => ["010", "101", "010", "101", "011"],
        '@' => ["011", "101", "111", "100", "011"],
        '|' => ["010", "010", "010", "010", "010"],
        '~' => ["000", "011", "111", "110", "000"],
        '^' => ["010", "101", "000", "000", "000"],
        _ => return None,
    };
    Some(rows)
}

/// Pixel size of one font dot for a caller-facing `scale` (same meaning as
/// [`UiBatch::text`]'s scale): 1.5 px keeps the 5-row glyph close to the
/// vanilla 8 px cell.
fn unit(scale: f32) -> f32 {
    scale * 1.8
}

pub fn text(ui: &mut UiBatch, content: &str, x: f32, y: f32, scale: f32, color: [f32; 4]) {
    let dot = unit(scale);
    let mut cursor_y = y;
    for line in content.split('\n') {
        let mut cursor_x = x;
        for ch in line.chars() {
            let rows = glyph(ch).or_else(|| glyph('?')).expect("'?' is defined");
            for (row, bits) in rows.iter().enumerate() {
                for (column, bit) in bits.chars().enumerate() {
                    if bit == '1' {
                        ui.rect(
                            Rect::new(
                                cursor_x + column as f32 * dot,
                                cursor_y + row as f32 * dot,
                                dot,
                                dot,
                            ),
                            color,
                        );
                    }
                }
            }
            cursor_x += ADVANCE as f32 * dot;
        }
        cursor_y += LINE_ADVANCE as f32 * dot;
    }
}

/// Width of the longest line, in pixels.
pub fn text_width(content: &str, scale: f32) -> f32 {
    let dot = unit(scale);
    content
        .split('\n')
        .map(|line| {
            let columns = line.chars().count() * ADVANCE;
            if columns == 0 {
                0.0
            } else {
                (columns - 1) as f32 * dot
            }
        })
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_is_three_columns_by_five_rows() {
        for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,:;-+_=/\\()[]<>!?'\"*#%&@|~^ ".chars() {
            let rows = glyph(ch).unwrap_or_else(|| panic!("missing glyph for {ch:?}"));
            assert_eq!(rows.len(), GLYPH_HEIGHT);
            for row in rows {
                assert_eq!(row.len(), GLYPH_WIDTH, "{ch:?} row {row:?}");
                assert!(row.chars().all(|bit| bit == '0' || bit == '1'));
            }
        }
        assert!(glyph('\u{4e00}').is_none());
    }

    #[test]
    fn unknown_characters_fall_back_to_a_question_mark() {
        let mut ui = UiBatch::default();
        text(&mut ui, "\u{4e00}", 0.0, 0.0, 2.0, [1.0; 4]);
        let mut expected = UiBatch::default();
        text(&mut expected, "?", 0.0, 0.0, 2.0, [1.0; 4]);
        assert_eq!(ui.indices, expected.indices);
        assert_eq!(ui.vertices.len(), expected.vertices.len());
    }

    #[test]
    fn width_tracks_the_longest_line() {
        let one = text_width("AB", 2.0);
        let two = text_width("ABC", 2.0);
        assert!(two > one);
        assert_eq!(text_width("A\nABCDEF", 2.0), text_width("ABCDEF", 2.0));
        assert_eq!(text_width("", 2.0), 0.0);
    }

    #[test]
    fn drawing_emits_quads_and_newlines_move_down() {
        let mut ui = UiBatch::default();
        text(&mut ui, "A\nB", 0.0, 10.0, 1.0, [1.0; 4]);
        assert!(!ui.is_empty());
        let lowest = ui
            .vertices
            .iter()
            .map(|v| v.position[1])
            .fold(0.0, f32::max);
        assert!(lowest > 10.0 + GLYPH_HEIGHT as f32 * unit(1.0));
    }
}

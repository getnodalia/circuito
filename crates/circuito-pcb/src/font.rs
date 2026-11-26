// SPDX-License-Identifier: MIT OR Apache-2.0
//! A small original single-stroke font for silkscreen and fabrication text.
//!
//! The glyphs were drawn for Circuito on a grid 4 units wide and 6 units tall (cap height 6, units
//! of `height / 6`), with an advance of 6 units. They cover A-Z (lower case is drawn as upper case),
//! 0-9 and `- + . , : / \ _ ( ) = ! ? * < > [ ] | ' "`. Any other character is drawn as an empty box
//! and counted in [`StrokeText::unsupported`] so callers can warn.

use crate::board::Justify;
use crate::error::{PcbError, Result};
use crate::geom::point::Point;
use crate::transform::rotate_point;
use crate::units::{check_coord, check_positive, Nm};

/// Longest text accepted.
pub const MAX_TEXT_CHARS: usize = 256;

type Stroke = &'static [(i8, i8)];

const BOX: &[Stroke] = &[&[(0, 0), (4, 0), (4, 6), (0, 6), (0, 0)]];

fn glyph(c: char) -> Option<&'static [Stroke]> {
    let g: &'static [Stroke] = match c.to_ascii_uppercase() {
        ' ' => &[],
        'A' => &[&[(0, 0), (0, 4), (2, 6), (4, 4), (4, 0)], &[(0, 3), (4, 3)]],
        'B' => &[
            &[(0, 0), (0, 6), (3, 6), (4, 5), (4, 4), (3, 3), (0, 3)],
            &[(3, 3), (4, 2), (4, 1), (3, 0), (0, 0)],
        ],
        'C' => &[&[
            (4, 5),
            (3, 6),
            (1, 6),
            (0, 5),
            (0, 1),
            (1, 0),
            (3, 0),
            (4, 1),
        ]],
        'D' => &[&[(0, 0), (0, 6), (2, 6), (4, 4), (4, 2), (2, 0), (0, 0)]],
        'E' => &[&[(4, 6), (0, 6), (0, 0), (4, 0)], &[(0, 3), (3, 3)]],
        'F' => &[&[(4, 6), (0, 6), (0, 0)], &[(0, 3), (3, 3)]],
        'G' => &[&[
            (4, 5),
            (3, 6),
            (1, 6),
            (0, 5),
            (0, 1),
            (1, 0),
            (3, 0),
            (4, 1),
            (4, 3),
            (2, 3),
        ]],
        'H' => &[&[(0, 0), (0, 6)], &[(4, 0), (4, 6)], &[(0, 3), (4, 3)]],
        'I' => &[&[(1, 6), (3, 6)], &[(2, 6), (2, 0)], &[(1, 0), (3, 0)]],
        'J' => &[&[(4, 6), (4, 1), (3, 0), (1, 0), (0, 1)]],
        'K' => &[&[(0, 0), (0, 6)], &[(4, 6), (0, 3), (4, 0)]],
        'L' => &[&[(0, 6), (0, 0), (4, 0)]],
        'M' => &[&[(0, 0), (0, 6), (2, 3), (4, 6), (4, 0)]],
        'N' => &[&[(0, 0), (0, 6), (4, 0), (4, 6)]],
        'O' => &[&[
            (1, 0),
            (0, 1),
            (0, 5),
            (1, 6),
            (3, 6),
            (4, 5),
            (4, 1),
            (3, 0),
            (1, 0),
        ]],
        'P' => &[&[(0, 0), (0, 6), (3, 6), (4, 5), (4, 4), (3, 3), (0, 3)]],
        'Q' => &[
            &[
                (1, 0),
                (0, 1),
                (0, 5),
                (1, 6),
                (3, 6),
                (4, 5),
                (4, 1),
                (3, 0),
                (1, 0),
            ],
            &[(2, 2), (4, 0)],
        ],
        'R' => &[
            &[(0, 0), (0, 6), (3, 6), (4, 5), (4, 4), (3, 3), (0, 3)],
            &[(2, 3), (4, 0)],
        ],
        'S' => &[&[
            (4, 5),
            (3, 6),
            (1, 6),
            (0, 5),
            (0, 4),
            (1, 3),
            (3, 3),
            (4, 2),
            (4, 1),
            (3, 0),
            (1, 0),
            (0, 1),
        ]],
        'T' => &[&[(0, 6), (4, 6)], &[(2, 6), (2, 0)]],
        'U' => &[&[(0, 6), (0, 1), (1, 0), (3, 0), (4, 1), (4, 6)]],
        'V' => &[&[(0, 6), (2, 0), (4, 6)]],
        'W' => &[&[(0, 6), (1, 0), (2, 3), (3, 0), (4, 6)]],
        'X' => &[&[(0, 6), (4, 0)], &[(4, 6), (0, 0)]],
        'Y' => &[&[(0, 6), (2, 3), (4, 6)], &[(2, 3), (2, 0)]],
        'Z' => &[&[(0, 6), (4, 6), (0, 0), (4, 0)]],
        '0' => &[
            &[
                (1, 0),
                (0, 1),
                (0, 5),
                (1, 6),
                (3, 6),
                (4, 5),
                (4, 1),
                (3, 0),
                (1, 0),
            ],
            &[(0, 1), (4, 5)],
        ],
        '1' => &[&[(1, 5), (2, 6), (2, 0)], &[(1, 0), (3, 0)]],
        '2' => &[&[(0, 5), (1, 6), (3, 6), (4, 5), (4, 4), (0, 0), (4, 0)]],
        '3' => &[
            &[(0, 5), (1, 6), (3, 6), (4, 5), (4, 4), (3, 3), (1, 3)],
            &[(3, 3), (4, 2), (4, 1), (3, 0), (1, 0), (0, 1)],
        ],
        '4' => &[&[(3, 0), (3, 6), (0, 2), (4, 2)]],
        '5' => &[&[
            (4, 6),
            (0, 6),
            (0, 3),
            (3, 3),
            (4, 2),
            (4, 1),
            (3, 0),
            (1, 0),
            (0, 1),
        ]],
        '6' => &[&[
            (4, 5),
            (3, 6),
            (1, 6),
            (0, 5),
            (0, 1),
            (1, 0),
            (3, 0),
            (4, 1),
            (4, 2),
            (3, 3),
            (0, 3),
        ]],
        '7' => &[&[(0, 6), (4, 6), (1, 0)]],
        '8' => &[&[
            (1, 3),
            (0, 4),
            (0, 5),
            (1, 6),
            (3, 6),
            (4, 5),
            (4, 4),
            (3, 3),
            (1, 3),
            (0, 2),
            (0, 1),
            (1, 0),
            (3, 0),
            (4, 1),
            (4, 2),
            (3, 3),
        ]],
        '9' => &[&[
            (0, 1),
            (1, 0),
            (3, 0),
            (4, 1),
            (4, 5),
            (3, 6),
            (1, 6),
            (0, 5),
            (0, 4),
            (1, 3),
            (4, 3),
        ]],
        '-' => &[&[(0, 3), (4, 3)]],
        '+' => &[&[(0, 3), (4, 3)], &[(2, 1), (2, 5)]],
        '.' => &[&[(2, 0), (2, 1)]],
        ',' => &[&[(2, 1), (2, 0), (1, -1)]],
        ':' => &[&[(2, 0), (2, 1)], &[(2, 4), (2, 5)]],
        '/' => &[&[(0, 0), (4, 6)]],
        '\\' => &[&[(0, 6), (4, 0)]],
        '_' => &[&[(0, 0), (4, 0)]],
        '(' => &[&[(3, 6), (2, 5), (2, 1), (3, 0)]],
        ')' => &[&[(1, 6), (2, 5), (2, 1), (1, 0)]],
        '[' => &[&[(3, 6), (1, 6), (1, 0), (3, 0)]],
        ']' => &[&[(1, 6), (3, 6), (3, 0), (1, 0)]],
        '=' => &[&[(0, 2), (4, 2)], &[(0, 4), (4, 4)]],
        '!' => &[&[(2, 6), (2, 2)], &[(2, 0), (2, 1)]],
        '?' => &[
            &[(0, 5), (1, 6), (3, 6), (4, 5), (4, 4), (2, 3), (2, 2)],
            &[(2, 0), (2, 1)],
        ],
        '*' => &[&[(0, 1), (4, 5)], &[(0, 5), (4, 1)], &[(2, 1), (2, 5)]],
        '<' => &[&[(4, 5), (0, 3), (4, 1)]],
        '>' => &[&[(0, 5), (4, 3), (0, 1)]],
        '|' => &[&[(2, 0), (2, 6)]],
        '\'' => &[&[(2, 6), (2, 4)]],
        '"' => &[&[(1, 6), (1, 4)], &[(3, 6), (3, 4)]],
        _ => return None,
    };
    Some(g)
}

/// Text as a set of straight strokes in board coordinates; draw them with a round aperture of the
/// text's stroke thickness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrokeText {
    pub strokes: Vec<(Point, Point)>,
    /// Characters drawn as a box because the font has no glyph for them.
    pub unsupported: usize,
}

fn scaled(units: i128, height: Nm) -> i128 {
    // round half away from zero of units * height / 6
    let n = units * height as i128;
    let q = n / 6;
    let r = n % 6;
    if 2 * r.abs() >= 6 {
        q + if n < 0 { -1 } else { 1 }
    } else {
        q
    }
}

/// Lays out `text` with its anchor at `pos`.
pub fn render_text(
    text: &str,
    pos: Point,
    height: Nm,
    rotation: i32,
    mirrored: bool,
    justify: Justify,
) -> Result<StrokeText> {
    check_positive(height, "text height")?;
    check_coord(pos.x, "text position")?;
    check_coord(pos.y, "text position")?;
    let count = text.chars().count();
    if count > MAX_TEXT_CHARS {
        return Err(PcbError::range(
            "text",
            format!("{count} characters exceed the limit of {MAX_TEXT_CHARS}"),
        ));
    }
    let width_units = if count == 0 { 0 } else { count as i128 * 6 - 2 };
    let shift = match justify {
        Justify::Left => 0,
        Justify::Center => -width_units * height as i128 / 12,
        Justify::Right => -width_units * height as i128 / 6,
    };
    let mut strokes = Vec::new();
    let mut unsupported = 0usize;
    let place = |gx: i128, gy: i128, ci: usize| -> Result<Point> {
        let x = scaled(ci as i128 * 6 + gx, height) + shift;
        let y = scaled(gy, height);
        let x = if mirrored { -x } else { x };
        let q = Point::new(
            i64::try_from(x).map_err(|_| PcbError::Overflow("text"))?,
            i64::try_from(y).map_err(|_| PcbError::Overflow("text"))?,
        );
        let r = rotate_point(q, rotation).ok_or(PcbError::Overflow("text rotation"))?;
        let p = r
            .checked_add(pos.x, pos.y)
            .ok_or(PcbError::Overflow("text position"))?;
        check_coord(p.x, "text")?;
        check_coord(p.y, "text")?;
        Ok(p)
    };
    for (ci, c) in text.chars().enumerate() {
        let g = match glyph(c) {
            Some(g) => g,
            None => {
                unsupported += 1;
                BOX
            }
        };
        for stroke in g {
            for w in stroke.windows(2) {
                let a = place(w[0].0 as i128, w[0].1 as i128, ci)?;
                let b = place(w[1].0 as i128, w[1].1 as i128, ci)?;
                strokes.push((a, b));
            }
        }
    }
    Ok(StrokeText {
        strokes,
        unsupported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_char_has_a_glyph_inside_its_cell() {
        for c in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-+.,:/\\_()[]=!?*<>|'\" abc".chars() {
            let g = glyph(c).unwrap_or_else(|| panic!("no glyph for {c}"));
            for s in g {
                for &(x, y) in *s {
                    assert!(
                        (0..=4).contains(&x) && (-1..=6).contains(&y),
                        "{c}: ({x},{y})"
                    );
                }
            }
        }
        assert!(glyph('é').is_none());
    }

    #[test]
    fn layout_is_exact_and_deterministic() {
        let t = render_text("I", Point::new(1000, 2000), 6000, 0, false, Justify::Left).unwrap();
        // unit = 1000 nm; first stroke (1,6)-(3,6)
        assert_eq!(
            t.strokes[0],
            (Point::new(2000, 8000), Point::new(4000, 8000))
        );
        assert_eq!(t.unsupported, 0);
        let m = render_text("I", Point::new(0, 0), 6000, 0, true, Justify::Left).unwrap();
        assert_eq!(
            m.strokes[0],
            (Point::new(-1000, 6000), Point::new(-3000, 6000))
        );
        let r = render_text("I", Point::new(0, 0), 6000, 90_000, false, Justify::Left).unwrap();
        assert_eq!(
            r.strokes[0],
            (Point::new(-6000, 1000), Point::new(-6000, 3000))
        );
        let c = render_text("II", Point::new(0, 0), 6000, 0, false, Justify::Center).unwrap();
        // width = 10 units, shifted by -5 units
        assert_eq!(c.strokes[0].0, Point::new(1000 - 5000, 6000));
    }

    #[test]
    fn unsupported_and_oversize() {
        let t = render_text("é", Point::ORIGIN, 6000, 0, false, Justify::Left).unwrap();
        assert_eq!(t.unsupported, 1);
        assert_eq!(t.strokes.len(), 4);
        assert!(render_text(
            &"A".repeat(MAX_TEXT_CHARS + 1),
            Point::ORIGIN,
            6000,
            0,
            false,
            Justify::Left
        )
        .is_err());
        assert!(render_text("A", Point::ORIGIN, 0, 0, false, Justify::Left).is_err());
    }
}

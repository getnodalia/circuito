// SPDX-License-Identifier: MIT OR Apache-2.0
//! The app icon, drawn here from shapes: a resistor between two terminals in near-black on
//! Circuito's gold, `#C99A06`. Nothing is read from a file; the picture is original work, flat,
//! with no gradients or glow, and the same pixels are the window icon, the logo in the title bar
//! and the mark on the About page.

/// Circuito's accent, the icon's background.
pub const ACCENT: [u8; 3] = [0xC9, 0x9A, 0x06];
/// The drawing's ink.
pub const INK: [u8; 3] = [0x20, 0x18, 0x04];

/// A rounded rectangle's signed distance: negative inside.
fn rounded_rect(p: (f32, f32), centre: (f32, f32), half: (f32, f32), radius: f32) -> f32 {
    let qx = (p.0 - centre.0).abs() - (half.0 - radius);
    let qy = (p.1 - centre.1).abs() - (half.1 - radius);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - radius
}

/// Distance from `p` to the segment `a`-`b`.
fn segment(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// The resistor's path: a lead, three peaks of the zigzag, a lead.
const PATH: [(f32, f32); 7] = [
    (0.17, 0.5),
    (0.30, 0.5),
    (0.375, 0.30),
    (0.50, 0.70),
    (0.625, 0.30),
    (0.70, 0.5),
    (0.83, 0.5),
];
const STROKE: f32 = 0.028;
const TERMINAL: f32 = 0.055;

/// Whether `(u, v)` in the unit square is on the drawing.
fn ink(p: (f32, f32)) -> bool {
    let on_path = PATH.windows(2).any(|w| segment(p, w[0], w[1]) <= STROKE);
    let terminal = [PATH[0], PATH[PATH.len() - 1]]
        .iter()
        .any(|c| ((p.0 - c.0).powi(2) + (p.1 - c.1).powi(2)).sqrt() <= TERMINAL);
    on_path || terminal
}

/// The icon as `size * size * 4` bytes of straight (not premultiplied) RGBA.
pub fn rgba(size: usize) -> Vec<u8> {
    const SAMPLES: usize = 3;
    let mut out = Vec::with_capacity(size * size * 4);
    let n = SAMPLES as f32;
    let channel = |c: u8| f32::from(c) / 255.0;
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0.0f32; 3];
            let mut alpha = 0.0f32;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let u = (x as f32 + (sx as f32 + 0.5) / n) / size as f32;
                    let v = (y as f32 + (sy as f32 + 0.5) / n) / size as f32;
                    if rounded_rect((u, v), (0.5, 0.5), (0.5, 0.5), 0.22) > 0.0 {
                        continue;
                    }
                    let c = if ink((u, v)) { INK } else { ACCENT };
                    for (s, v) in sum.iter_mut().zip(c) {
                        *s += channel(v);
                    }
                    alpha += 1.0;
                }
            }
            for s in sum {
                let c = if alpha > 0.0 { s / alpha } else { 0.0 };
                out.push((c * 255.0).round().clamp(0.0, 255.0) as u8);
            }
            out.push((alpha / (n * n) * 255.0).round().clamp(0.0, 255.0) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_is_a_rounded_square_of_the_accent_with_a_dark_resistor() {
        let size = 64;
        let px = rgba(size);
        assert_eq!(px.len(), size * size * 4);
        let at = |x: usize, y: usize| &px[(y * size + x) * 4..(y * size + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0, "the corner is transparent");
        assert_eq!(at(size / 2, 3), &[ACCENT[0], ACCENT[1], ACCENT[2], 255]);
        // The left lead crosses the middle row; the zigzag's low point is under the centre.
        assert_eq!(at((0.23 * size as f32) as usize, size / 2)[..3], INK);
        assert_eq!(at(size / 2, (0.70 * size as f32) as usize)[..3], INK);
        assert_eq!(at(size / 2, (0.30 * size as f32) as usize)[..3], ACCENT);
    }

    #[test]
    fn it_draws_at_every_size_the_app_uses() {
        for size in [1, 16, 24, 128, 256] {
            assert_eq!(rgba(size).len(), size * size * 4);
        }
    }
}

// SPDX-License-Identifier: MIT OR Apache-2.0
//! Placement transforms and the geometry of pads and holes in board coordinates.
//!
//! A footprint is placed by mirroring (back side: `x -> -x`), then rotating about its origin, then
//! translating. Rotations that are multiples of 90 000 millidegrees are exact; other angles round
//! each transformed vertex to the nearest nm (round half away from zero) using `f64` sin/cos, so
//! results for odd angles could differ in the last nm between platforms with different libm.
//! Sizes with an odd nm count (half-nm centres) are placed with the extra nm on the positive side.

use crate::board::{Footprint, Pad, PadKind, PadShape};
use crate::error::{PcbError, Result};
use crate::geom::point::Point;
use crate::geom::region::Region;
use crate::geom::shape::Shape;
use crate::layer::Side;
use crate::units::{check_coord, Nm};

/// Rotates `p` about the origin by `mdeg` millidegrees counter-clockwise. `None` on overflow.
pub fn rotate_point(p: Point, mdeg: i32) -> Option<Point> {
    let m = mdeg.rem_euclid(360_000);
    match m {
        0 => Some(p),
        90_000 => Some(Point::new(p.y.checked_neg()?, p.x)),
        180_000 => Some(Point::new(p.x.checked_neg()?, p.y.checked_neg()?)),
        270_000 => Some(Point::new(p.y, p.x.checked_neg()?)),
        _ => {
            let a = (m as f64) * std::f64::consts::PI / 180_000.0;
            let (s, c) = a.sin_cos();
            let x = (p.x as f64 * c - p.y as f64 * s).round();
            let y = (p.x as f64 * s + p.y as f64 * c).round();
            let lim = (1i64 << 62) as f64;
            if !(x.abs() < lim && y.abs() < lim) {
                return None;
            }
            Some(Point::new(x as i64, y as i64))
        }
    }
}

/// A footprint placement.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub position: Point,
    pub rotation: i32,
    pub mirror: bool,
}

impl Placement {
    pub fn of(fp: &Footprint) -> Placement {
        Placement {
            position: fp.position,
            rotation: fp.rotation,
            mirror: fp.side == Side::Back,
        }
    }

    /// Footprint-local point to board point.
    pub fn apply(&self, p: Point) -> Result<Point> {
        let m = if self.mirror {
            Point::new(p.x.checked_neg().ok_or(PcbError::Overflow("mirror"))?, p.y)
        } else {
            p
        };
        let r = rotate_point(m, self.rotation).ok_or(PcbError::Overflow("rotation"))?;
        let t = r
            .checked_add(self.position.x, self.position.y)
            .ok_or(PcbError::Overflow("placement"))?;
        check_coord(t.x, "placed coordinate")?;
        check_coord(t.y, "placed coordinate")?;
        Ok(t)
    }

    /// The footprint rotation as seen on the board (mirrored footprints turn the other way).
    pub fn effective_rotation(&self) -> i32 {
        self.rotation
    }
}

/// Pad-local offset (relative to the pad centre, before the pad rotation) to footprint-local point.
fn pad_local(pad: &Pad, v: Point) -> Result<Point> {
    let r = rotate_point(v, pad.rotation).ok_or(PcbError::Overflow("pad rotation"))?;
    r.checked_add(pad.position.x, pad.position.y)
        .ok_or(PcbError::Overflow("pad position"))
}

/// Board-coordinate centre of a pad.
pub fn pad_center(fp: &Footprint, pad: &Pad) -> Result<Point> {
    Placement::of(fp).apply(pad.position)
}

fn rect_corners(lo_x: Nm, hi_x: Nm, lo_y: Nm, hi_y: Nm) -> [Point; 4] {
    [
        Point::new(lo_x, lo_y),
        Point::new(hi_x, lo_y),
        Point::new(hi_x, hi_y),
        Point::new(lo_x, hi_y),
    ]
}

fn half_lo_hi(size: Nm) -> (Nm, Nm) {
    let lo = -(size / 2);
    (lo, lo + size)
}

/// The copper shape of a pad in board coordinates, grown by `grow` nm on every side (negative
/// shrinks). `None` when shrinking leaves nothing. Custom polygons cannot shrink: a negative `grow`
/// leaves them unchanged.
pub fn pad_shape(fp: &Footprint, pad: &Pad, grow: Nm) -> Result<Option<Shape>> {
    let pl = Placement::of(fp);
    let center = pl.apply(pad.position)?;
    let grow2 = grow
        .checked_mul(2)
        .ok_or(PcbError::Overflow("pad margin"))?;
    let to_board = |v: Point| -> Result<Point> { pl.apply(pad_local(pad, v)?) };
    let shape = match &pad.shape {
        PadShape::Circle => {
            let d = pad.size.w + grow2;
            if d <= 0 {
                return Ok(None);
            }
            Shape::point(center, d)
        }
        PadShape::Oval => {
            let w = pad.size.w + grow2;
            let h = pad.size.h + grow2;
            if w <= 0 || h <= 0 {
                return Ok(None);
            }
            let (long, short, horizontal) = if w >= h { (w, h, true) } else { (h, w, false) };
            let half = (long - short) / 2;
            let (a, b) = if horizontal {
                (Point::new(-half, 0), Point::new(half, 0))
            } else {
                (Point::new(0, -half), Point::new(0, half))
            };
            Shape::segment(to_board(a)?, to_board(b)?, short)
        }
        PadShape::Rect => {
            let w = pad.size.w + grow2;
            let h = pad.size.h + grow2;
            if w <= 0 || h <= 0 {
                return Ok(None);
            }
            let (lx, hx) = half_lo_hi(w);
            let (ly, hy) = half_lo_hi(h);
            let pts = rect_corners(lx, hx, ly, hy)
                .iter()
                .map(|c| to_board(*c))
                .collect::<Result<Vec<_>>>()?;
            Shape::region(Region::polygon(pts)?, 0)
        }
        PadShape::RoundRect { corner_radius } => {
            let w = pad.size.w + grow2;
            let h = pad.size.h + grow2;
            let r = corner_radius + grow;
            if w <= 0 || h <= 0 || r < 0 {
                return Ok(None);
            }
            let r = r.min(w.min(h) / 2);
            let iw = w - 2 * r;
            let ih = h - 2 * r;
            match (iw > 0, ih > 0) {
                (false, false) => Shape::point(center, 2 * r),
                (true, false) => {
                    let half = iw / 2;
                    Shape::segment(
                        to_board(Point::new(-half, 0))?,
                        to_board(Point::new(iw - half, 0))?,
                        2 * r,
                    )
                }
                (false, true) => {
                    let half = ih / 2;
                    Shape::segment(
                        to_board(Point::new(0, -half))?,
                        to_board(Point::new(0, ih - half))?,
                        2 * r,
                    )
                }
                (true, true) => {
                    let (lx, hx) = half_lo_hi(iw);
                    let (ly, hy) = half_lo_hi(ih);
                    let pts = rect_corners(lx, hx, ly, hy)
                        .iter()
                        .map(|c| to_board(*c))
                        .collect::<Result<Vec<_>>>()?;
                    Shape::region(Region::polygon(pts)?, 2 * r)
                }
            }
        }
        PadShape::Custom { polygon } => {
            let pts = polygon
                .iter()
                .map(|c| to_board(*c))
                .collect::<Result<Vec<_>>>()?;
            // Rotation by an odd angle and mirroring may flip orientation; Region accepts either.
            Shape::region(Region::polygon(pts)?, if grow > 0 { grow2 } else { 0 })
        }
    };
    Ok(Some(shape))
}

/// The hole of a pad as a shape (a point for a round hole, a segment for a slot), if it has one.
pub fn pad_hole(fp: &Footprint, pad: &Pad) -> Result<Option<Shape>> {
    let Some(d) = &pad.drill else { return Ok(None) };
    let pl = Placement::of(fp);
    let to_board = |v: Point| -> Result<Point> { pl.apply(pad_local(pad, v)?) };
    Ok(Some(match d.slot_length {
        Some(l) if l > d.diameter => {
            let span = l - d.diameter;
            let half = span / 2;
            Shape::segment(
                to_board(Point::new(-half, 0))?,
                to_board(Point::new(span - half, 0))?,
                d.diameter,
            )
        }
        _ => Shape::point(pl.apply(pad.position)?, d.diameter),
    }))
}

/// A footprint-local polygon placed on the board.
pub fn place_region(fp: &Footprint, r: &Region) -> Result<Region> {
    let pl = Placement::of(fp);
    let map = |ring: &Vec<Point>| {
        ring.iter()
            .map(|p| pl.apply(*p))
            .collect::<Result<Vec<_>>>()
    };
    Region::new(
        map(&r.outer)?,
        r.holes.iter().map(map).collect::<Result<Vec<_>>>()?,
    )
}

/// Which copper layers a pad has copper on.
pub fn pad_copper_layers(fp: &Footprint, pad: &Pad, copper_layers: u8) -> Vec<crate::layer::Layer> {
    match pad.kind {
        PadKind::Smd => vec![fp.side.copper()],
        PadKind::ThroughHole => crate::layer::Layer::copper_stack(copper_layers),
        PadKind::Npth => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_turns_are_exact() {
        let p = Point::new(3, 5);
        assert_eq!(rotate_point(p, 90_000), Some(Point::new(-5, 3)));
        assert_eq!(rotate_point(p, 180_000), Some(Point::new(-3, -5)));
        assert_eq!(rotate_point(p, 270_000), Some(Point::new(5, -3)));
        assert_eq!(rotate_point(p, -90_000), Some(Point::new(5, -3)));
        assert_eq!(rotate_point(p, 360_000), Some(p));
        assert_eq!(
            rotate_point(Point::new(1_000_000, 0), 45_000),
            Some(Point::new(707_107, 707_107))
        );
    }

    #[test]
    fn back_side_mirrors_x() {
        let pl = Placement {
            position: Point::new(100, 100),
            rotation: 0,
            mirror: true,
        };
        assert_eq!(pl.apply(Point::new(10, 5)).unwrap(), Point::new(90, 105));
    }
}

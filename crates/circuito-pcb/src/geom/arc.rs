// SPDX-License-Identifier: MIT OR Apache-2.0
//! Circular arcs: exact definition by integer start, end and centre, and a polyline approximation
//! with a stated tolerance for the geometry checks.
//!
//! An arc stores `start`, `end`, `center` and a direction. `start` and `end` must be at the same
//! distance from `center` within [`ARC_RADIUS_TOLERANCE`] nm (integer rounding of a centre makes an
//! exact match impossible in general). When `start == end` the arc is a full circle.
//! Gerber output uses the arc as given; only DRC and bounding boxes use [`Arc::approximate`], whose
//! vertices lie on the circle of mean radius (rounded to nm), so a chord is inside the true arc by
//! at most the requested tolerance (plus 1 nm of rounding).

use serde::{Deserialize, Serialize};

use super::point::{cross, Point, Rect};
use crate::error::{PcbError, Result};
use crate::units::{check_coord, isqrt, Nm, MAX_COORD};

/// Largest allowed difference between the start and end radii, in nm.
pub const ARC_RADIUS_TOLERANCE: Nm = 2;
/// Default approximation tolerance (maximum sagitta) in nm.
pub const DEFAULT_ARC_TOLERANCE: Nm = 1_000;
/// Most segments an approximated arc may have.
pub const MAX_ARC_SEGMENTS: usize = 20_000;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Arc {
    pub start: Point,
    pub end: Point,
    pub center: Point,
    /// True when the arc runs counter-clockwise from `start` to `end`.
    pub ccw: bool,
}

impl Arc {
    /// Builds and validates an arc.
    pub fn new(start: Point, end: Point, center: Point, ccw: bool) -> Result<Arc> {
        let a = Arc { start, end, center, ccw };
        a.validate("arc")?;
        Ok(a)
    }

    /// Checks coordinates, a non-zero radius and matching start and end radii.
    pub fn validate(&self, what: &str) -> Result<()> {
        for p in [self.start, self.end, self.center] {
            check_coord(p.x, what)?;
            check_coord(p.y, what)?;
        }
        let rs = isqrt(self.start.dist2(self.center)) as i128;
        let re = isqrt(self.end.dist2(self.center)) as i128;
        if rs == 0 || re == 0 {
            return Err(PcbError::geometry(what, "start or end coincides with the centre"));
        }
        if (rs - re).abs() > ARC_RADIUS_TOLERANCE as i128 {
            return Err(PcbError::geometry(what, format!("start radius {rs} nm and end radius {re} nm differ by more than {ARC_RADIUS_TOLERANCE} nm")));
        }
        Ok(())
    }

    /// The arc through three points (`mid` lies on the arc between `start` and `end`).
    /// The centre is computed exactly and rounded to the nearest nm.
    pub fn through(start: Point, mid: Point, end: Point) -> Result<Arc> {
        for p in [start, mid, end] {
            check_coord(p.x, "arc point")?;
            check_coord(p.y, "arc point")?;
        }
        let d = cross(start, mid, end);
        if d == 0 {
            return Err(PcbError::geometry("arc", "the three points are collinear"));
        }
        let bx = mid.x as i128 - start.x as i128;
        let by = mid.y as i128 - start.y as i128;
        let cx = end.x as i128 - start.x as i128;
        let cy = end.y as i128 - start.y as i128;
        let b2 = bx * bx + by * by;
        let c2 = cx * cx + cy * cy;
        let dd = 2 * (bx * cy - by * cx);
        let ux = div_round(cy * b2 - by * c2, dd);
        let uy = div_round(bx * c2 - cx * b2, dd);
        let center = Point::new(
            i64::try_from(start.x as i128 + ux).map_err(|_| PcbError::Overflow("arc centre"))?,
            i64::try_from(start.y as i128 + uy).map_err(|_| PcbError::Overflow("arc centre"))?,
        );
        // start -> mid -> end turning left means counter-clockwise travel.
        Arc::new(start, end, center, d > 0, ).map_err(|e| match e {
            PcbError::Geometry { detail, .. } => PcbError::geometry("arc", format!("{detail}; the centre is too far away or the points too close to collinear")),
            o => o,
        })
    }

    /// Mean radius in nm (floating point; used only for approximation).
    pub fn radius(&self) -> f64 {
        ((isqrt(self.start.dist2(self.center)) + isqrt(self.end.dist2(self.center))) as f64) / 2.0
    }

    /// Signed sweep in radians: positive counter-clockwise, in `(-2pi, 2pi]`; a full circle when
    /// `start == end`.
    pub fn sweep(&self) -> f64 {
        let a0 = ((self.start.y - self.center.y) as f64).atan2((self.start.x - self.center.x) as f64);
        let a1 = ((self.end.y - self.center.y) as f64).atan2((self.end.x - self.center.x) as f64);
        let tau = std::f64::consts::TAU;
        if self.start == self.end {
            return if self.ccw { tau } else { -tau };
        }
        let mut d = a1 - a0;
        if self.ccw {
            while d <= 0.0 {
                d += tau;
            }
            while d > tau {
                d -= tau;
            }
            d
        } else {
            while d >= 0.0 {
                d -= tau;
            }
            while d < -tau {
                d += tau;
            }
            d
        }
    }

    /// Polyline from `start` to `end` (both exact) whose chords deviate from the arc by at most
    /// `tolerance` nm (plus rounding to the nm grid). Errors if more than [`MAX_ARC_SEGMENTS`]
    /// segments would be needed.
    pub fn approximate(&self, tolerance: Nm) -> Result<Vec<Point>> {
        let r = self.radius();
        let sweep = self.sweep();
        let tol = tolerance.max(1) as f64;
        let step = if tol >= r { std::f64::consts::FRAC_PI_2 } else { (2.0 * (1.0 - tol / r).acos()).min(std::f64::consts::FRAC_PI_2) };
        let n_f = (sweep.abs() / step).ceil().max(1.0);
        if !n_f.is_finite() || n_f > MAX_ARC_SEGMENTS as f64 {
            return Err(PcbError::range("arc", format!("needs more than {MAX_ARC_SEGMENTS} segments at {tolerance} nm tolerance")));
        }
        let n = n_f as usize;
        let a0 = ((self.start.y - self.center.y) as f64).atan2((self.start.x - self.center.x) as f64);
        let mut pts = Vec::with_capacity(n + 1);
        pts.push(self.start);
        for i in 1..n {
            let a = a0 + sweep * (i as f64) / (n as f64);
            let x = (self.center.x as f64 + r * a.cos()).round();
            let y = (self.center.y as f64 + r * a.sin()).round();
            let lim = MAX_COORD as f64 * 2.0;
            if x.abs() > lim || y.abs() > lim {
                return Err(PcbError::range("arc", "approximation leaves the coordinate range"));
            }
            pts.push(Point::new(x as i64, y as i64));
        }
        pts.push(self.end);
        Ok(pts)
    }

    /// Bounding box of the true arc (including its extreme points on the axes), computed from the
    /// angles in floating point and widened by 2 nm for rounding.
    pub fn bbox(&self) -> Rect {
        let mut r = Rect::from_point(self.start).include(self.end);
        let sweep = self.sweep();
        let a0 = ((self.start.y - self.center.y) as f64).atan2((self.start.x - self.center.x) as f64);
        let rad = self.radius();
        for k in 0..4 {
            let ang = k as f64 * std::f64::consts::FRAC_PI_2;
            // angle of the axis extreme relative to the start, in the arc's direction
            let tau = std::f64::consts::TAU;
            let rel = if sweep >= 0.0 { (ang - a0).rem_euclid(tau) } else { -((a0 - ang).rem_euclid(tau)) };
            if rel.abs() <= sweep.abs() {
                let x = (self.center.x as f64 + rad * ang.cos()).round() as i64;
                let y = (self.center.y as f64 + rad * ang.sin()).round() as i64;
                r = r.include(Point::new(x, y));
            }
        }
        r.inflate(2)
    }
}

/// Division rounding half away from zero.
fn div_round(n: i128, d: i128) -> i128 {
    let q = n / d;
    let r = n % d;
    if 2 * r.abs() >= d.abs() {
        q + if (n < 0) != (d < 0) { -1 } else { 1 }
    } else {
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn through_three_points_quarter_circle() {
        // quarter circle of radius 1 mm about the origin, counter-clockwise
        let a = Arc::through(Point::new(1_000_000, 0), Point::new(707_107, 707_107), Point::new(0, 1_000_000)).unwrap();
        assert!(a.center.x.abs() <= 2 && a.center.y.abs() <= 2, "{:?}", a.center);
        assert!(a.ccw);
        let cw = Arc::through(Point::new(0, 1_000_000), Point::new(707_107, 707_107), Point::new(1_000_000, 0)).unwrap();
        assert!(!cw.ccw);
        assert!((a.sweep() - std::f64::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((cw.sweep() + std::f64::consts::FRAC_PI_2).abs() < 1e-5);
    }

    #[test]
    fn collinear_and_bad_arcs_rejected() {
        assert!(Arc::through(Point::new(0, 0), Point::new(5, 0), Point::new(10, 0)).is_err());
        assert!(Arc::new(Point::new(10, 0), Point::new(0, 20), Point::ORIGIN, true).is_err());
        assert!(Arc::new(Point::ORIGIN, Point::new(0, 20), Point::ORIGIN, true).is_err());
    }

    #[test]
    fn approximation_stays_within_tolerance() {
        let a = Arc::new(Point::new(5_000_000, 0), Point::new(0, 5_000_000), Point::ORIGIN, true).unwrap();
        let pts = a.approximate(500).unwrap();
        assert_eq!(pts.first().copied(), Some(a.start));
        assert_eq!(pts.last().copied(), Some(a.end));
        for w in pts.windows(2) {
            // midpoint of the chord is at most tol (+rounding) inside the circle
            let mx = (w[0].x as f64 + w[1].x as f64) / 2.0;
            let my = (w[0].y as f64 + w[1].y as f64) / 2.0;
            let d = 5_000_000.0 - (mx * mx + my * my).sqrt();
            assert!((-2.0..=502.0).contains(&d), "sagitta {d}");
        }
    }

    #[test]
    fn full_circle_and_bbox() {
        let c = Arc::new(Point::new(1000, 0), Point::new(1000, 0), Point::ORIGIN, false).unwrap();
        assert!((c.sweep() + std::f64::consts::TAU).abs() < 1e-9);
        let b = c.bbox();
        assert!(b.min.x <= -1000 && b.max.x >= 1000 && b.min.y <= -1000 && b.max.y >= 1000);
        // half circle above the axis only
        let h = Arc::new(Point::new(1000, 0), Point::new(-1000, 0), Point::ORIGIN, true).unwrap();
        let b = h.bbox();
        assert!(b.max.y >= 1000 && b.min.y >= -2);
    }

    #[test]
    fn huge_arc_segment_count_is_bounded() {
        let a = Arc::new(Point::new(1_000_000_000, 0), Point::new(1_000_000_000, 0), Point::ORIGIN, true).unwrap();
        assert!(a.approximate(1).is_err());
        assert!(a.approximate(1_000).is_ok());
    }
}

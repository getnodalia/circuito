// SPDX-License-Identifier: MIT OR Apache-2.0
//! Shapes for clearance work: a core (point, segment or polygon region) grown by a disc.
//!
//! A circle is a point with a diameter, a stadium (track, oval pad) a segment with a diameter, a
//! rounded rectangle a rectangle region with a diameter, a plain polygon a region with diameter 0.
//! The gap between two shapes is `dist(core_a, core_b) - (diam_a + diam_b) / 2`, evaluated exactly:
//! the comparison with a required clearance never leaves integer arithmetic.

use std::cmp::Ordering;

use super::dist::{point_segment_dist2, segment_segment_dist2};
use super::point::{Point, Rect};
use super::region::{Loc, Region};
use super::wide::Ratio;
use crate::units::Nm;

/// The core of a shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Core {
    Point(Point),
    Segment(Point, Point),
    Region(Region),
}

/// A core grown by a disc of diameter `diam` (>= 0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub core: Core,
    pub diam: Nm,
}

/// Stand-in for "farther than anything we looked at" when edge pruning removes every edge.
const FAR: u128 = 1 << 120;

impl Shape {
    pub fn point(p: Point, diam: Nm) -> Shape {
        Shape { core: Core::Point(p), diam }
    }
    pub fn segment(a: Point, b: Point, diam: Nm) -> Shape {
        if a == b {
            Shape::point(a, diam)
        } else {
            Shape { core: Core::Segment(a, b), diam }
        }
    }
    pub fn region(r: Region, diam: Nm) -> Shape {
        Shape { core: Core::Region(r), diam }
    }

    /// Bounding box including the grown disc (rounded up).
    pub fn bbox(&self) -> Rect {
        let base = match &self.core {
            Core::Point(p) => Rect::from_point(*p),
            Core::Segment(a, b) => Rect::from_point(*a).include(*b),
            Core::Region(r) => r.bbox(),
        };
        base.inflate((self.diam + 1) / 2)
    }

    /// A point of the core, used for containment shortcuts.
    pub fn any_point(&self) -> Point {
        match &self.core {
            Core::Point(p) => *p,
            Core::Segment(a, _) => *a,
            Core::Region(r) => r.outer.first().copied().unwrap_or(Point::ORIGIN),
        }
    }

    /// Compares the true gap between the two shapes with `required`: `Less` when the gap is smaller
    /// than `required` (so with `required == 0`, `Less` means the shapes overlap, `Equal` that they
    /// touch exactly). Exact for every input within the coordinate limits.
    pub fn gap_cmp(&self, other: &Shape, required: Nm) -> Ordering {
        let r2 = (self.diam as i128 + other.diam as i128 + 2 * required as i128).max(0) as u128;
        let reach = ((self.diam as i128 + other.diam as i128) / 2 + required as i128 + 1).clamp(0, i64::MAX as i128) as i64;
        core_dist2(&self.core, &other.core, Some(reach)).cmp_double_dist_sq(r2)
    }

    /// `floor(gap)` in nm, clamped to zero for overlapping shapes. Exact.
    pub fn gap_floor(&self, other: &Shape) -> Nm {
        let d = core_dist2(&self.core, &other.core, None);
        let two_d = d.floor_double_dist().min(i64::MAX as u128 / 4) as i128;
        let g2 = two_d - self.diam as i128 - other.diam as i128;
        g2.div_euclid(2).clamp(0, i64::MAX as i128) as i64
    }

    /// True when the shapes overlap or touch (gap <= 0).
    pub fn touches(&self, other: &Shape) -> bool {
        self.gap_cmp(other, 0) != Ordering::Greater
    }
}

fn region_near(r: &Region, b: Rect, reach: Option<Nm>) -> impl Iterator<Item = (Point, Point)> + '_ {
    let window = reach.map(|d| b.inflate(d));
    r.edges().filter(move |(p, q)| match window {
        None => true,
        Some(w) => Rect::from_point(*p).include(*q).intersects(w),
    })
}

/// Exact squared distance between two cores. `reach` (when given) prunes polygon edges whose box is
/// farther than `reach` from the other core's box: the result is then exact whenever the true
/// distance is at most `reach`, and larger than `reach` otherwise.
pub fn core_dist2(a: &Core, b: &Core, reach: Option<Nm>) -> Ratio {
    use Core::*;
    match (a, b) {
        (Point(p), Point(q)) => Ratio::int(p.dist2(*q)),
        (Point(p), Segment(c, d)) | (Segment(c, d), Point(p)) => point_segment_dist2(*p, *c, *d),
        (Segment(a1, a2), Segment(b1, b2)) => segment_segment_dist2(*a1, *a2, *b1, *b2),
        (Point(p), Region(r)) | (Region(r), Point(p)) => {
            if r.locate(*p) != Loc::Outside {
                return Ratio::ZERO;
            }
            let bb = Rect::from_point(*p);
            region_near(r, bb, reach).map(|(c, d)| point_segment_dist2(*p, c, d)).fold(Ratio::int(FAR), Ratio::min)
        }
        (Segment(p, q), Region(r)) | (Region(r), Segment(p, q)) => {
            if r.locate(*p) != Loc::Outside || r.locate(*q) != Loc::Outside {
                return Ratio::ZERO;
            }
            let bb = Rect::from_point(*p).include(*q);
            region_near(r, bb, reach).map(|(c, d)| segment_segment_dist2(*p, *q, c, d)).fold(Ratio::int(FAR), Ratio::min)
        }
        (Region(r1), Region(r2)) => {
            let b1 = r1.bbox();
            let b2 = r2.bbox();
            let mut best = Ratio::int(FAR);
            let win1 = reach.map(|d| b2.inflate(d));
            let win2 = reach.map(|d| b1.inflate(d));
            let e2: Vec<(super::point::Point, super::point::Point)> = r2
                .edges()
                .filter(|(p, q)| match win2 {
                    None => true,
                    Some(w) => Rect::from_point(*p).include(*q).intersects(w),
                })
                .collect();
            for (p, q) in r1.edges() {
                let eb = Rect::from_point(p).include(q);
                if let Some(w) = win1 {
                    if !eb.intersects(w) {
                        continue;
                    }
                }
                for &(c, d) in &e2 {
                    if let Some(w) = reach {
                        if !eb.inflate(w).intersects(Rect::from_point(c).include(d)) {
                            continue;
                        }
                    }
                    let r = segment_segment_dist2(p, q, c, d);
                    if r.is_zero() {
                        return Ratio::ZERO;
                    }
                    best = best.min(r);
                }
            }
            let v1 = r1.outer.first().copied();
            let v2 = r2.outer.first().copied();
            if let Some(v) = v1 {
                if r2.locate(v) != Loc::Outside {
                    return Ratio::ZERO;
                }
            }
            if let Some(v) = v2 {
                if r1.locate(v) != Loc::Outside {
                    return Ratio::ZERO;
                }
            }
            best
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i64, y: i64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn stadium_gap_boundary_is_exact() {
        // two horizontal 100 nm wide tracks 300 nm apart centre to centre: gap 200 nm
        let a = Shape::segment(p(0, 0), p(1000, 0), 100);
        let b = Shape::segment(p(0, 300), p(1000, 300), 100);
        assert_eq!(a.gap_cmp(&b, 200), Ordering::Equal);
        assert_eq!(a.gap_cmp(&b, 201), Ordering::Less);
        assert_eq!(a.gap_cmp(&b, 199), Ordering::Greater);
        assert_eq!(a.gap_floor(&b), 200);
    }

    #[test]
    fn diagonal_gap_is_irrational_but_exact() {
        // two points at distance sqrt(2)*1000 = 1414.2135..; diameters 0 -> gap floor 1414
        let a = Shape::point(p(0, 0), 0);
        let b = Shape::point(p(1000, 1000), 0);
        assert_eq!(a.gap_floor(&b), 1414);
        assert_eq!(a.gap_cmp(&b, 1414), Ordering::Greater);
        assert_eq!(a.gap_cmp(&b, 1415), Ordering::Less);
    }

    #[test]
    fn odd_diameter_half_nm_boundary() {
        // circles of diameter 101 at distance 301: gap = 301 - 101 = 200
        let a = Shape::point(p(0, 0), 101);
        let b = Shape::point(p(301, 0), 101);
        assert_eq!(a.gap_cmp(&b, 200), Ordering::Equal);
        // one diameter odd: gap 301 - 50.5 - 50 = 200.5
        let c = Shape::point(p(0, 0), 101);
        let d = Shape::point(p(301, 0), 100);
        assert_eq!(c.gap_cmp(&d, 200), Ordering::Greater);
        assert_eq!(c.gap_cmp(&d, 201), Ordering::Less);
        assert_eq!(c.gap_floor(&d), 200);
    }

    #[test]
    fn region_distance_and_containment() {
        let sq = Region::polygon(vec![p(0, 0), p(1000, 0), p(1000, 1000), p(0, 1000)]).unwrap();
        let a = Shape::region(sq.clone(), 0);
        assert_eq!(a.gap_floor(&Shape::point(p(1500, 500), 0)), 500);
        assert!(a.touches(&Shape::point(p(500, 500), 0)));
        assert!(a.touches(&Shape::point(p(1000, 500), 0)));
        // segment fully inside
        assert!(a.touches(&Shape::segment(p(100, 100), p(900, 900), 0)));
        // region inside a region with a hole: disjoint when sitting in the hole
        let holey = Region::new(vec![p(-5000, -5000), p(5000, -5000), p(5000, 5000), p(-5000, 5000)], vec![vec![p(-2000, -2000), p(2000, -2000), p(2000, 2000), p(-2000, 2000)]]).unwrap();
        let h = Shape::region(holey, 0);
        assert!(!a.touches(&h));
        assert_eq!(a.gap_floor(&h), 1000);
        // pruned comparison agrees
        assert_eq!(a.gap_cmp(&h, 1000), Ordering::Equal);
        assert_eq!(a.gap_cmp(&h, 1001), Ordering::Less);
    }
}

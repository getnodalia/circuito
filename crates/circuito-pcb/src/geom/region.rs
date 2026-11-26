// SPDX-License-Identifier: MIT OR Apache-2.0
//! Polygon rings and regions (an outer ring with holes): area, containment, validation.

use serde::{Deserialize, Serialize};

use super::point::{cross, on_segment, segments_cross_properly, segments_intersect, Point, Rect};
use crate::error::{PcbError, Result};
use crate::units::check_coord;

/// Most vertices accepted in one ring.
pub const MAX_RING_VERTS: usize = 50_000;
/// Most holes accepted in one region.
pub const MAX_HOLES: usize = 10_000;

/// Where a point is relative to a polygon.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Loc {
    Inside,
    Boundary,
    Outside,
}

/// Twice the signed area of a ring (positive when counter-clockwise), exact.
pub fn ring_area2(ring: &[Point]) -> i128 {
    let n = ring.len();
    let mut s: i128 = 0;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        s += a.x as i128 * b.y as i128 - b.x as i128 * a.y as i128;
    }
    s
}

/// Exact point-in-ring test (crossing number with boundary detection).
pub fn locate_in_ring(ring: &[Point], p: Point) -> Loc {
    let n = ring.len();
    let mut inside = false;
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        if on_segment(a, b, p) {
            return Loc::Boundary;
        }
        if (a.y > p.y) != (b.y > p.y) {
            let c = cross(a, b, p);
            let upward = b.y > a.y;
            if (upward && c > 0) || (!upward && c < 0) {
                inside = !inside;
            }
        }
    }
    if inside {
        Loc::Inside
    } else {
        Loc::Outside
    }
}

/// True when no two non-adjacent edges of the ring touch and adjacent edges only share their vertex.
/// Sweeps by x so ordinary rings are checked in roughly `O(n log n)`.
pub fn ring_is_simple(ring: &[Point]) -> bool {
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut edges: Vec<(i64, i64, usize)> = (0..n)
        .map(|i| {
            let a = ring[i];
            let b = ring[(i + 1) % n];
            (a.x.min(b.x), a.x.max(b.x), i)
        })
        .collect();
    edges.sort_unstable();
    for (k, &(_, xmax, i)) in edges.iter().enumerate() {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        if a == b {
            return false;
        }
        for &(xmin2, _, j) in &edges[k + 1..] {
            if xmin2 > xmax {
                break;
            }
            let (c, d) = (ring[j], ring[(j + 1) % n]);
            let adjacent = j == (i + 1) % n || i == (j + 1) % n;
            if adjacent {
                // Adjacent edges may share only their common vertex: reject folding back on each other.
                let (shared, p, q) = if j == (i + 1) % n {
                    (b, a, d)
                } else {
                    (a, b, c)
                };
                if cross(shared, p, q) == 0 && super::point::dot(shared, p, q) > 0 {
                    return false;
                }
            } else if segments_intersect(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

/// A polygon with holes. The outer ring and the holes may have any orientation; the fields are
/// public for reading but go through [`Region::new`] to be validated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub outer: Vec<Point>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<Point>>,
}

impl Region {
    /// Validates sizes, coordinate ranges, simplicity, non-zero area, and that holes lie inside the
    /// outer ring without touching it or each other's interiors (holes are only checked against the
    /// outer ring by a vertex test and edge crossing test).
    pub fn new(outer: Vec<Point>, holes: Vec<Vec<Point>>) -> Result<Region> {
        let r = Region { outer, holes };
        r.validate("region")?;
        Ok(r)
    }

    /// A region without holes.
    pub fn polygon(outer: Vec<Point>) -> Result<Region> {
        Region::new(outer, Vec::new())
    }

    /// Validates a deserialised region.
    pub fn validate(&self, what: &str) -> Result<()> {
        if self.holes.len() > MAX_HOLES {
            return Err(PcbError::range(
                what,
                format!("{} holes exceed the limit of {MAX_HOLES}", self.holes.len()),
            ));
        }
        validate_ring(&self.outer, what)?;
        for (i, h) in self.holes.iter().enumerate() {
            let name = format!("{what} hole {i}");
            validate_ring(h, &name)?;
            for &v in h {
                if locate_in_ring(&self.outer, v) == Loc::Outside {
                    return Err(PcbError::geometry(
                        &name,
                        format!("vertex ({}, {}) is outside the outer ring", v.x, v.y),
                    ));
                }
            }
            let n = h.len();
            let m = self.outer.len();
            for i in 0..n {
                for j in 0..m {
                    if segments_cross_properly(
                        h[i],
                        h[(i + 1) % n],
                        self.outer[j],
                        self.outer[(j + 1) % m],
                    ) {
                        return Err(PcbError::geometry(&name, "crosses the outer ring"));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn bbox(&self) -> Rect {
        // `outer` is validated non-empty; fall back to the origin for an unvalidated empty ring.
        Rect::of_points(&self.outer).unwrap_or(Rect::from_point(Point::ORIGIN))
    }

    /// Where `p` is relative to the material of the region (holes are not material).
    pub fn locate(&self, p: Point) -> Loc {
        match locate_in_ring(&self.outer, p) {
            Loc::Outside => Loc::Outside,
            Loc::Boundary => Loc::Boundary,
            Loc::Inside => {
                for h in &self.holes {
                    match locate_in_ring(h, p) {
                        Loc::Inside => return Loc::Outside,
                        Loc::Boundary => return Loc::Boundary,
                        Loc::Outside => {}
                    }
                }
                Loc::Inside
            }
        }
    }

    /// Twice the area of the material (outer minus holes), exact.
    pub fn area2(&self) -> i128 {
        let mut a = ring_area2(&self.outer).abs();
        for h in &self.holes {
            a -= ring_area2(h).abs();
        }
        a
    }

    /// Every edge of every ring.
    pub fn edges(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        std::iter::once(&self.outer)
            .chain(self.holes.iter())
            .flat_map(|r| {
                let n = r.len();
                (0..n).map(move |i| (r[i], r[(i + 1) % n]))
            })
    }

    pub fn vertex_count(&self) -> usize {
        self.outer.len() + self.holes.iter().map(Vec::len).sum::<usize>()
    }

    /// Translated copy; `None` on overflow.
    pub fn translated(&self, dx: i64, dy: i64) -> Option<Region> {
        let tr = |r: &Vec<Point>| {
            r.iter()
                .map(|p| p.checked_add(dx, dy))
                .collect::<Option<Vec<_>>>()
        };
        Some(Region {
            outer: tr(&self.outer)?,
            holes: self.holes.iter().map(tr).collect::<Option<Vec<_>>>()?,
        })
    }
}

fn validate_ring(ring: &[Point], what: &str) -> Result<()> {
    if ring.len() < 3 {
        return Err(PcbError::geometry(what, "needs at least 3 vertices"));
    }
    if ring.len() > MAX_RING_VERTS {
        return Err(PcbError::range(
            what,
            format!(
                "{} vertices exceed the limit of {MAX_RING_VERTS}",
                ring.len()
            ),
        ));
    }
    for p in ring {
        check_coord(p.x, what)?;
        check_coord(p.y, what)?;
    }
    if ring_area2(ring) == 0 {
        return Err(PcbError::geometry(what, "has zero area"));
    }
    if !ring_is_simple(ring) {
        return Err(PcbError::geometry(
            what,
            "is self-intersecting or has repeated vertices",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<Point> {
        vec![
            Point::new(x0, y0),
            Point::new(x1, y0),
            Point::new(x1, y1),
            Point::new(x0, y1),
        ]
    }

    #[test]
    fn locate_cases() {
        let r = sq(0, 0, 10, 10);
        assert_eq!(locate_in_ring(&r, Point::new(5, 5)), Loc::Inside);
        assert_eq!(locate_in_ring(&r, Point::new(0, 5)), Loc::Boundary);
        assert_eq!(locate_in_ring(&r, Point::new(10, 10)), Loc::Boundary);
        assert_eq!(locate_in_ring(&r, Point::new(11, 5)), Loc::Outside);
        assert_eq!(locate_in_ring(&r, Point::new(-1, -1)), Loc::Outside);
    }

    #[test]
    fn area_and_orientation() {
        let mut r = sq(0, 0, 10, 10);
        assert_eq!(ring_area2(&r), 200);
        r.reverse();
        assert_eq!(ring_area2(&r), -200);
    }

    #[test]
    fn simplicity() {
        assert!(ring_is_simple(&sq(0, 0, 4, 4)));
        // bow-tie
        let bow = vec![
            Point::new(0, 0),
            Point::new(4, 4),
            Point::new(4, 0),
            Point::new(0, 4),
        ];
        assert!(!ring_is_simple(&bow));
        // repeated vertex
        let rep = vec![
            Point::new(0, 0),
            Point::new(4, 0),
            Point::new(4, 0),
            Point::new(4, 4),
            Point::new(0, 4),
        ];
        assert!(!ring_is_simple(&rep));
        // spike (fold back)
        let spike = vec![
            Point::new(0, 0),
            Point::new(4, 0),
            Point::new(2, 0),
            Point::new(2, 4),
        ];
        assert!(!ring_is_simple(&spike));
    }

    #[test]
    fn region_with_hole() {
        let reg = Region::new(sq(0, 0, 100, 100), vec![sq(40, 40, 60, 60)]).unwrap();
        assert_eq!(reg.locate(Point::new(10, 10)), Loc::Inside);
        assert_eq!(reg.locate(Point::new(50, 50)), Loc::Outside);
        assert_eq!(reg.locate(Point::new(40, 50)), Loc::Boundary);
        assert_eq!(reg.area2(), 2 * (10_000 - 400));
        assert!(Region::new(sq(0, 0, 100, 100), vec![sq(90, 90, 110, 110)]).is_err());
        assert!(Region::new(
            vec![Point::new(0, 0), Point::new(1, 1), Point::new(2, 2)],
            vec![]
        )
        .is_err());
        assert!(Region::new(sq(0, 0, 2 << 30, 5), vec![]).is_err());
    }
}

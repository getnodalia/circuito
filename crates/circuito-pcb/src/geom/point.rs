// SPDX-License-Identifier: MIT OR Apache-2.0
//! Integer points and the exact predicates built on them.

use serde::{Deserialize, Serialize};

use crate::units::Nm;

/// A point in nanometres (Y up).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(from = "[i64; 2]", into = "[i64; 2]")]
pub struct Point {
    pub x: Nm,
    pub y: Nm,
}

impl From<[i64; 2]> for Point {
    fn from(a: [i64; 2]) -> Self {
        Point { x: a[0], y: a[1] }
    }
}
impl From<Point> for [i64; 2] {
    fn from(p: Point) -> Self {
        [p.x, p.y]
    }
}

impl Point {
    pub const ORIGIN: Point = Point { x: 0, y: 0 };

    pub const fn new(x: Nm, y: Nm) -> Self {
        Point { x, y }
    }

    /// Translation by `(dx, dy)`, `None` on overflow.
    pub fn checked_add(self, dx: Nm, dy: Nm) -> Option<Point> {
        Some(Point { x: self.x.checked_add(dx)?, y: self.y.checked_add(dy)? })
    }

    /// Squared distance, exact (no overflow for any two `i64` points).
    pub fn dist2(self, o: Point) -> u128 {
        let dx = (self.x as i128 - o.x as i128).unsigned_abs();
        let dy = (self.y as i128 - o.y as i128).unsigned_abs();
        dx * dx + dy * dy
    }
}

/// Cross product `(a - o) x (b - o)`, exact for coordinates within `±2^62`.
pub fn cross(o: Point, a: Point, b: Point) -> i128 {
    let ax = a.x as i128 - o.x as i128;
    let ay = a.y as i128 - o.y as i128;
    let bx = b.x as i128 - o.x as i128;
    let by = b.y as i128 - o.y as i128;
    ax * by - ay * bx
}

/// Dot product `(a - o) . (b - o)`.
pub fn dot(o: Point, a: Point, b: Point) -> i128 {
    let ax = a.x as i128 - o.x as i128;
    let ay = a.y as i128 - o.y as i128;
    let bx = b.x as i128 - o.x as i128;
    let by = b.y as i128 - o.y as i128;
    ax * bx + ay * by
}

/// Sign of [`cross`]: +1 counter-clockwise, -1 clockwise, 0 collinear.
pub fn orient(o: Point, a: Point, b: Point) -> i32 {
    match cross(o, a, b) {
        c if c > 0 => 1,
        c if c < 0 => -1,
        _ => 0,
    }
}

/// True when `p` lies on the closed segment `a`-`b` (exact).
pub fn on_segment(a: Point, b: Point, p: Point) -> bool {
    cross(a, b, p) == 0 && p.x >= a.x.min(b.x) && p.x <= a.x.max(b.x) && p.y >= a.y.min(b.y) && p.y <= a.y.max(b.y)
}

/// True when the closed segments `a`-`b` and `c`-`d` share at least one point (exact).
pub fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let o1 = orient(a, b, c);
    let o2 = orient(a, b, d);
    let o3 = orient(c, d, a);
    let o4 = orient(c, d, b);
    if o1 != o2 && o3 != o4 {
        return true;
    }
    (o1 == 0 && on_segment(a, b, c)) || (o2 == 0 && on_segment(a, b, d)) || (o3 == 0 && on_segment(c, d, a)) || (o4 == 0 && on_segment(c, d, b))
}

/// True when the open segments cross at a single interior point of both (proper crossing).
pub fn segments_cross_properly(a: Point, b: Point, c: Point, d: Point) -> bool {
    let o1 = orient(a, b, c);
    let o2 = orient(a, b, d);
    let o3 = orient(c, d, a);
    let o4 = orient(c, d, b);
    o1 * o2 < 0 && o3 * o4 < 0
}

/// Axis-aligned bounding box, inclusive.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Rect {
    pub fn from_point(p: Point) -> Rect {
        Rect { min: p, max: p }
    }
    pub fn union(self, o: Rect) -> Rect {
        Rect {
            min: Point::new(self.min.x.min(o.min.x), self.min.y.min(o.min.y)),
            max: Point::new(self.max.x.max(o.max.x), self.max.y.max(o.max.y)),
        }
    }
    pub fn include(self, p: Point) -> Rect {
        self.union(Rect::from_point(p))
    }
    /// Grows by `d` on every side (saturating).
    pub fn inflate(self, d: Nm) -> Rect {
        Rect {
            min: Point::new(self.min.x.saturating_sub(d), self.min.y.saturating_sub(d)),
            max: Point::new(self.max.x.saturating_add(d), self.max.y.saturating_add(d)),
        }
    }
    pub fn intersects(self, o: Rect) -> bool {
        self.min.x <= o.max.x && o.min.x <= self.max.x && self.min.y <= o.max.y && o.min.y <= self.max.y
    }
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
    pub fn of_points(pts: &[Point]) -> Option<Rect> {
        let mut it = pts.iter();
        let first = *it.next()?;
        Some(it.fold(Rect::from_point(first), |r, p| r.include(*p)))
    }
    pub fn center(self) -> Point {
        Point::new(((self.min.x as i128 + self.max.x as i128) / 2) as i64, ((self.min.y as i128 + self.max.y as i128) / 2) as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientation_and_intersection() {
        let a = Point::new(0, 0);
        let b = Point::new(10, 0);
        assert_eq!(orient(a, b, Point::new(5, 1)), 1);
        assert_eq!(orient(a, b, Point::new(5, -1)), -1);
        assert_eq!(orient(a, b, Point::new(5, 0)), 0);
        assert!(segments_intersect(a, b, Point::new(5, -1), Point::new(5, 1)));
        assert!(segments_intersect(a, b, Point::new(10, 0), Point::new(20, 5)));
        assert!(!segments_intersect(a, b, Point::new(11, 0), Point::new(20, 0)));
        assert!(!segments_cross_properly(a, b, Point::new(10, 0), Point::new(20, 5)));
        assert!(segments_cross_properly(a, b, Point::new(5, -1), Point::new(5, 1)));
    }

    #[test]
    fn serde_is_a_pair() {
        let s = serde_json::to_string(&Point::new(3, -4)).unwrap();
        assert_eq!(s, "[3,-4]");
        assert_eq!(serde_json::from_str::<Point>(&s).unwrap(), Point::new(3, -4));
    }
}

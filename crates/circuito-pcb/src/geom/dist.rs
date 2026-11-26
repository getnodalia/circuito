// SPDX-License-Identifier: MIT OR Apache-2.0
//! Exact squared distances between points and segments, as rationals.

use super::point::{cross, dot, segments_intersect, Point};
use super::wide::Ratio;

/// Squared distance from `p` to the closed segment `a`-`b`, exactly.
///
/// In the interior of the segment it is `cross^2 / |ab|^2`; at the ends it is an integer.
pub fn point_segment_dist2(p: Point, a: Point, b: Point) -> Ratio {
    if a == b {
        return Ratio::int(p.dist2(a));
    }
    let len2 = a.dist2(b);
    let t = dot(a, p, b);
    if t <= 0 {
        return Ratio::int(p.dist2(a));
    }
    if (t as u128) >= len2 {
        return Ratio::int(p.dist2(b));
    }
    let c = cross(a, b, p).unsigned_abs();
    Ratio { num: c.saturating_mul(c), den: len2 }
}

/// Squared distance between two closed segments, exactly. Zero when they touch or cross.
pub fn segment_segment_dist2(a: Point, b: Point, c: Point, d: Point) -> Ratio {
    if segments_intersect(a, b, c, d) {
        return Ratio::ZERO;
    }
    point_segment_dist2(a, c, d)
        .min(point_segment_dist2(b, c, d))
        .min(point_segment_dist2(c, a, b))
        .min(point_segment_dist2(d, a, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    fn p(x: i64, y: i64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn point_segment_cases() {
        // interior: distance 3 -> 9/1 after reduction (num/den = 9*100/100)
        let r = point_segment_dist2(p(5, 3), p(0, 0), p(10, 0));
        assert_eq!(r.cmp_ratio(Ratio::int(9)), Ordering::Equal);
        // beyond an end
        let r = point_segment_dist2(p(13, 4), p(0, 0), p(10, 0));
        assert_eq!(r.cmp_ratio(Ratio::int(25)), Ordering::Equal);
        // on the segment
        assert!(point_segment_dist2(p(4, 0), p(0, 0), p(10, 0)).is_zero());
        // degenerate segment
        assert_eq!(point_segment_dist2(p(3, 4), p(0, 0), p(0, 0)).cmp_ratio(Ratio::int(25)), Ordering::Equal);
        // diagonal: distance from (0,2) to line y=x is sqrt(2), squared 2
        let r = point_segment_dist2(p(0, 2), p(0, 0), p(10, 10));
        assert_eq!(r.cmp_ratio(Ratio::int(2)), Ordering::Equal);
    }

    #[test]
    fn segment_segment_cases() {
        assert!(segment_segment_dist2(p(0, 0), p(10, 10), p(0, 10), p(10, 0)).is_zero());
        let r = segment_segment_dist2(p(0, 0), p(10, 0), p(0, 5), p(10, 5));
        assert_eq!(r.cmp_ratio(Ratio::int(25)), Ordering::Equal);
        let r = segment_segment_dist2(p(0, 0), p(10, 0), p(13, 4), p(20, 9));
        assert_eq!(r.cmp_ratio(Ratio::int(25)), Ordering::Equal);
    }
}

// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Property tests of the exact geometry against brute-force floating point sampling.

use std::cmp::Ordering;

use circuito_pcb::geom::{Core, Point, Region, Shape};
use proptest::prelude::*;

fn pt() -> impl Strategy<Value = Point> {
    (-2000i64..2000, -2000i64..2000).prop_map(|(x, y)| Point::new(x, y))
}

fn rect_region() -> impl Strategy<Value = Region> {
    (pt(), 1i64..800, 1i64..800).prop_map(|(o, w, h)| {
        Region::polygon(vec![
            o,
            Point::new(o.x + w, o.y),
            Point::new(o.x + w, o.y + h),
            Point::new(o.x, o.y + h),
        ])
        .unwrap()
    })
}

fn tri_region() -> impl Strategy<Value = Region> {
    (pt(), pt(), pt()).prop_filter_map("degenerate", |(a, b, c)| {
        Region::polygon(vec![a, b, c]).ok()
    })
}

fn shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        (pt(), 0i64..300).prop_map(|(p, d)| Shape::point(p, d)),
        (pt(), pt(), 0i64..300).prop_map(|(a, b, d)| Shape::segment(a, b, d)),
        (rect_region(), 0i64..300).prop_map(|(r, d)| Shape::region(r, d)),
        (tri_region(), 0i64..100).prop_map(|(r, d)| Shape::region(r, d)),
    ]
}

fn translate(s: &Shape, dx: i64, dy: i64) -> Shape {
    let core = match &s.core {
        Core::Point(p) => Core::Point(p.checked_add(dx, dy).unwrap()),
        Core::Segment(a, b) => Core::Segment(
            a.checked_add(dx, dy).unwrap(),
            b.checked_add(dx, dy).unwrap(),
        ),
        Core::Region(r) => Core::Region(r.translated(dx, dy).unwrap()),
    };
    Shape { core, diam: s.diam }
}

/// Dense sample of the boundary and interior of a core, for brute-force distance estimates.
fn samples(c: &Core, n: usize) -> Vec<(f64, f64)> {
    match c {
        Core::Point(p) => vec![(p.x as f64, p.y as f64)],
        Core::Segment(a, b) => (0..=n).map(|i| lerp(*a, *b, i as f64 / n as f64)).collect(),
        Core::Region(r) => {
            let mut v = Vec::new();
            for (a, b) in r.edges() {
                v.extend((0..=n).map(|i| lerp(a, b, i as f64 / n as f64)));
            }
            v
        }
    }
}

fn lerp(a: Point, b: Point, t: f64) -> (f64, f64) {
    (
        a.x as f64 + (b.x - a.x) as f64 * t,
        a.y as f64 + (b.y - a.y) as f64 * t,
    )
}

fn inside_region(r: &Region, x: f64, y: f64) -> bool {
    // even-odd over every ring (outer and holes), floating point
    let mut inside = false;
    for (a, b) in r.edges() {
        let (ax, ay, bx, by) = (a.x as f64, a.y as f64, b.x as f64, b.y as f64);
        if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
            inside = !inside;
        }
    }
    inside
}

/// Brute-force core distance: zero when a sampled point of one lies inside a region of the other,
/// else the minimum over boundary samples (an upper bound within the sampling step).
fn brute_dist(a: &Core, b: &Core) -> f64 {
    let sa = samples(a, 400);
    let sb = samples(b, 400);
    if let Core::Region(r) = b {
        if sa.iter().any(|&(x, y)| inside_region(r, x, y)) {
            return 0.0;
        }
    }
    if let Core::Region(r) = a {
        if sb.iter().any(|&(x, y)| inside_region(r, x, y)) {
            return 0.0;
        }
    }
    let mut best = f64::MAX;
    for &(x1, y1) in &sa {
        for &(x2, y2) in &sb {
            best = best.min(((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt());
        }
    }
    best
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn gap_is_symmetric(a in shape(), b in shape(), req in 0i64..400) {
        prop_assert_eq!(a.gap_cmp(&b, req), b.gap_cmp(&a, req));
        prop_assert_eq!(a.gap_floor(&b), b.gap_floor(&a));
    }

    #[test]
    fn gap_is_translation_invariant(a in shape(), b in shape(), dx in -3000i64..3000, dy in -3000i64..3000, req in 0i64..400) {
        let (ta, tb) = (translate(&a, dx, dy), translate(&b, dx, dy));
        prop_assert_eq!(a.gap_floor(&b), ta.gap_floor(&tb));
        prop_assert_eq!(a.gap_cmp(&b, req), ta.gap_cmp(&tb, req));
    }

    #[test]
    fn cmp_and_floor_agree(a in shape(), b in shape()) {
        let g = a.gap_floor(&b);
        // the true gap lies in [g, g+1) (or is <= 0 when g == 0)
        for c in (g + 1)..=(g + 3) {
            prop_assert_eq!(a.gap_cmp(&b, c), Ordering::Less);
        }
        for c in 0..=g.min(5) {
            if c > 0 || g > 0 {
                prop_assert_ne!(a.gap_cmp(&b, c), Ordering::Less);
            }
        }
        if g > 5 {
            prop_assert_ne!(a.gap_cmp(&b, g), Ordering::Less);
        }
        prop_assert_eq!(a.touches(&b), a.gap_cmp(&b, 0) != Ordering::Greater);
    }

    #[test]
    fn exact_distance_matches_brute_force(a in shape(), b in shape()) {
        // gap_floor clamps at zero, so compare the core distance recovered from a zero-diameter copy
        let a0 = Shape { core: a.core.clone(), diam: 0 };
        let b0 = Shape { core: b.core.clone(), diam: 0 };
        let exact = a0.gap_floor(&b0) as f64;
        let brute = brute_dist(&a.core, &b.core);
        // floor(d) <= d <= brute (sampling can only overestimate); brute overshoots by at most the
        // sample spacing of the longest edge (<= 5700/400) on each side
        prop_assert!(exact <= brute + 1e-6, "exact {exact} brute {brute}");
        prop_assert!(brute <= exact + 1.0 + 2.0 * 5700.0 / 400.0, "exact {exact} brute {brute}");
    }

    #[test]
    fn core_distances_obey_the_triangle_inequality(a in pt(), b in pt(), c in pt()) {
        let d = |x: Point, y: Point| Shape::point(x, 0).gap_floor(&Shape::point(y, 0)) as i64;
        // floors lose at most 1 per term
        prop_assert!(d(a, c) <= d(a, b) + d(b, c) + 2);
    }

    #[test]
    fn point_in_polygon_matches_axis_comparison(r in rect_region(), p in pt()) {
        let Core::Region(_) = Shape::region(r.clone(), 0).core else { unreachable!() };
        let (lo, hi) = (r.outer[0], r.outer[2]);
        let loc = r.locate(p);
        let strictly = p.x > lo.x && p.x < hi.x && p.y > lo.y && p.y < hi.y;
        let on = (p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y) && !strictly;
        match loc {
            circuito_pcb::geom::Loc::Inside => prop_assert!(strictly),
            circuito_pcb::geom::Loc::Boundary => prop_assert!(on),
            circuito_pcb::geom::Loc::Outside => prop_assert!(!strictly && !on),
        }
    }

    #[test]
    fn region_area_is_orientation_independent(r in tri_region()) {
        let mut rev = r.outer.clone();
        rev.reverse();
        let r2 = Region::polygon(rev).unwrap();
        prop_assert_eq!(r.area2(), r2.area2());
        prop_assert!(r.area2() > 0);
    }
}

#[test]
fn sampled_segment_pair_agrees_on_a_known_skew_case() {
    // distance between (0,0)-(1000,0) and (500,300)-(500,900) is 300
    let a = Shape::segment(Point::new(0, 0), Point::new(1000, 0), 0);
    let b = Shape::segment(Point::new(500, 300), Point::new(500, 900), 0);
    assert_eq!(a.gap_floor(&b), 300);
}

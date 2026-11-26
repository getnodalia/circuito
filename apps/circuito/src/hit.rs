// SPDX-License-Identifier: MIT OR Apache-2.0
//! What is under the pointer while editing: snapping to the grid, the pin or the wire at a point,
//! the item a click picks and the items inside a rubber band. All in grid units, with no window.

use circuito_core::edit::{on_segment, pin_points, Item, Items};
use circuito_core::schematic::{Point, Sheet};

use crate::geometry::{hit_part, part_bounds, Bounds};

/// How far (in grid units) from a pin the pointer still means the pin.
pub const PIN_SLACK: f32 = 0.7;
/// How far from a wire the pointer still means the wire.
pub const WIRE_SLACK: f32 = 0.45;
/// How far from a part's outline a click still picks the part.
pub const PART_SLACK: f32 = 0.4;

/// The nearest grid point.
#[must_use]
pub fn snap(p: (f32, f32)) -> Point {
    let q = |v: f32| {
        if v.is_finite() {
            v.round().clamp(-1.0e6, 1.0e6) as i32
        } else {
            0
        }
    };
    Point::new(q(p.0), q(p.1))
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Distance from `p` to the segment `a`–`b`.
#[must_use]
pub fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 <= f32::EPSILON {
        return dist(p, a);
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    dist(p, (a.0 + t * dx, a.1 + t * dy))
}

fn pf(p: Point) -> (f32, f32) {
    (p.x as f32, p.y as f32)
}

/// The pin of a part nearest to `at`, when one is within `slack`.
#[must_use]
pub fn pin_at(sheet: &Sheet, at: (f32, f32), slack: f32) -> Option<Point> {
    let mut best: Option<(f32, Point)> = None;
    for part in &sheet.parts {
        for pin in pin_points(part) {
            let d = dist(at, pf(pin));
            if d <= slack && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, pin));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// How wide a label's text is, in grid units (the canvas draws it to the right of the anchor).
#[must_use]
pub fn label_width(name: &str) -> f32 {
    1.0 + name.chars().count() as f32 * 1.4
}

/// The bounds of a power port's symbol.
#[must_use]
pub fn port_bounds(at: Point) -> Bounds {
    let a = pf(at);
    Bounds {
        min: (a.0 - 1.2, a.1 - 2.2),
        max: (a.0 + 1.2, a.1 + 2.2),
    }
}

/// The box a label occupies, which is also where a click picks it.
fn label_bounds(l: &circuito_core::schematic::Label) -> Bounds {
    let a = pf(l.position);
    Bounds {
        min: (a.0 - 0.4, a.1 - 1.4),
        max: (a.0 + label_width(&l.name), a.1 + 0.4),
    }
}

/// The box that stands for an item: what is drawn around it when it is selected. `None` for a
/// wire (which is highlighted along its length) and for an index that is out of range.
#[must_use]
pub fn item_bounds(sheet: &Sheet, item: Item) -> Option<Bounds> {
    let around = |p: Point, r: f32| {
        let a = pf(p);
        Bounds {
            min: (a.0 - r, a.1 - r),
            max: (a.0 + r, a.1 + r),
        }
    };
    match item {
        Item::Part(i) => part_bounds(sheet.parts.get(i)?),
        Item::Label(i) => Some(label_bounds(sheet.labels.get(i)?)),
        Item::Port(i) => Some(port_bounds(sheet.power_ports.get(i)?.position)),
        Item::NoConnect(i) => Some(around(sheet.no_connects.get(i)?.position, 0.7)),
        Item::Junction(i) => Some(around(sheet.junctions.get(i)?.position, 0.6)),
        Item::Wire(_) => None,
    }
}

/// Whether `p` is a place where a wire can end: a pin, the end or the middle of a wire, a label
/// or a power port.
#[must_use]
pub fn is_connection(sheet: &Sheet, p: Point) -> bool {
    sheet.parts.iter().any(|part| pin_points(part).contains(&p))
        || sheet.wires.iter().any(|w| {
            w.points
                .windows(2)
                .any(|s| matches!(s, [a, b] if on_segment(p, *a, *b)))
        })
        || sheet.labels.iter().any(|l| l.position == p)
        || sheet.power_ports.iter().any(|q| q.position == p)
}

/// The item a click at `at` picks: labels, ports and flags first, then parts (the smallest of
/// those that overlap), then wires.
#[must_use]
pub fn item_at(sheet: &Sheet, at: (f32, f32)) -> Option<Item> {
    for (i, l) in sheet.labels.iter().enumerate() {
        if label_bounds(l).contains(at) {
            return Some(Item::Label(i));
        }
    }
    for (i, q) in sheet.power_ports.iter().enumerate() {
        if port_bounds(q.position).contains(at) {
            return Some(Item::Port(i));
        }
    }
    for (i, n) in sheet.no_connects.iter().enumerate() {
        if dist(at, pf(n.position)) <= 0.7 {
            return Some(Item::NoConnect(i));
        }
    }
    if let Some(i) = hit_part(sheet, at, PART_SLACK) {
        return Some(Item::Part(i));
    }
    let mut best: Option<(f32, usize)> = None;
    for (i, w) in sheet.wires.iter().enumerate() {
        for s in w.points.windows(2) {
            if let [a, b] = s {
                let d = segment_distance(at, pf(*a), pf(*b));
                if d <= WIRE_SLACK && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, i));
                }
            }
        }
    }
    if let Some((_, i)) = best {
        return Some(Item::Wire(i));
    }
    for (i, j) in sheet.junctions.iter().enumerate() {
        if dist(at, pf(j.position)) <= 0.6 {
            return Some(Item::Junction(i));
        }
    }
    None
}

/// The items inside the rectangle between two corners: parts whose symbol lies inside it, wires
/// with every point inside, and labels, ports, flags and junctions whose anchor is inside.
#[must_use]
pub fn items_in(sheet: &Sheet, a: (f32, f32), b: (f32, f32)) -> Items {
    let r = Bounds {
        min: (a.0.min(b.0), a.1.min(b.1)),
        max: (a.0.max(b.0), a.1.max(b.1)),
    };
    let mut out = Items::new();
    for (i, p) in sheet.parts.iter().enumerate() {
        if part_bounds(p).is_some_and(|pb| r.contains(pb.min) && r.contains(pb.max)) {
            out.insert(Item::Part(i));
        }
    }
    for (i, w) in sheet.wires.iter().enumerate() {
        if !w.points.is_empty() && w.points.iter().all(|p| r.contains(pf(*p))) {
            out.insert(Item::Wire(i));
        }
    }
    for (i, l) in sheet.labels.iter().enumerate() {
        if r.contains(pf(l.position)) {
            out.insert(Item::Label(i));
        }
    }
    for (i, q) in sheet.power_ports.iter().enumerate() {
        if r.contains(pf(q.position)) {
            out.insert(Item::Port(i));
        }
    }
    for (i, n) in sheet.no_connects.iter().enumerate() {
        if r.contains(pf(n.position)) {
            out.insert(Item::NoConnect(i));
        }
    }
    for (i, j) in sheet.junctions.iter().enumerate() {
        if r.contains(pf(j.position)) {
            out.insert(Item::Junction(i));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuito_core::schematic::builder::Builder;

    fn sheet() -> Sheet {
        let mut b = Builder::new("t");
        let r1 = b.place("R", "R1", "1k", 0, 0);
        let r2 = b.place("R", "R2", "1k", 20, 0);
        b.connect((r1, "2"), (r2, "1"));
        b.label_pin(r1, "1", "in");
        b.power_pin(r2, "2", "GND");
        b.build().sheets.remove(0)
    }

    #[test]
    fn points_snap_to_the_grid_and_junk_does_not_panic() {
        assert_eq!(snap((2.4, -3.6)), Point::new(2, -4));
        assert_eq!(snap((f32::NAN, f32::INFINITY)), Point::new(0, 0));
    }

    #[test]
    fn a_pin_is_found_near_its_position_only() {
        let s = sheet();
        assert_eq!(pin_at(&s, (0.3, 4.2), PIN_SLACK), Some(Point::new(0, 4)));
        assert_eq!(pin_at(&s, (5.0, 5.0), PIN_SLACK), None);
    }

    #[test]
    fn clicks_pick_parts_wires_and_labels() {
        let s = sheet();
        assert_eq!(item_at(&s, (0.0, 0.0)), Some(Item::Part(0)));
        assert_eq!(item_at(&s, (10.0, 4.0)), Some(Item::Wire(0)));
        assert_eq!(item_at(&s, (1.0, -4.5)), Some(Item::Label(0)));
        assert_eq!(item_at(&s, (20.0, 6.0)), Some(Item::Port(0)));
        assert_eq!(item_at(&s, (50.0, 50.0)), None);
    }

    #[test]
    fn a_rubber_band_takes_what_is_inside_it() {
        let s = sheet();
        let first = items_in(&s, (-4.0, -8.0), (4.0, 10.0));
        assert!(first.contains(&Item::Part(0)));
        assert!(!first.contains(&Item::Part(1)));
        let all = items_in(&s, (-50.0, -50.0), (50.0, 50.0));
        assert!(all.contains(&Item::Part(1)) && all.contains(&Item::Wire(0)));
    }

    #[test]
    fn wires_end_on_pins_and_on_other_wires() {
        let s = sheet();
        assert!(is_connection(&s, Point::new(0, 4)));
        assert!(is_connection(&s, Point::new(10, 4)));
        assert!(!is_connection(&s, Point::new(10, 9)));
    }

    #[test]
    fn selected_items_have_a_box_except_wires() {
        let s = sheet();
        assert!(item_bounds(&s, Item::Part(0)).is_some());
        assert!(item_bounds(&s, Item::Label(0)).is_some());
        assert!(item_bounds(&s, Item::Port(0)).is_some());
        assert!(item_bounds(&s, Item::Wire(0)).is_none());
        assert!(item_bounds(&s, Item::Part(9)).is_none());
    }

    #[test]
    fn distance_to_a_degenerate_segment_is_to_its_point() {
        assert!((segment_distance((3.0, 4.0), (0.0, 0.0), (0.0, 0.0)) - 5.0).abs() < 1e-4);
    }
}

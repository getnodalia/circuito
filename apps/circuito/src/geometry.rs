// SPDX-License-Identifier: MIT OR Apache-2.0
//! Where things are on the drawing: the view (pan and zoom), the extent of parts, which part is
//! under a point, and where a rules-check finding points. All in grid units, with no window.

use circuito_core::library::{transform, Library, Shape};
use circuito_core::schematic::{Part, Point, Rotation, Schematic, Sheet};
use circuito_erc::Violation;

use crate::findings::part_of;

/// A rectangle in grid units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: (f32, f32),
    pub max: (f32, f32),
}

impl Bounds {
    fn around(p: (f32, f32)) -> Bounds {
        Bounds { min: p, max: p }
    }

    fn include(&mut self, p: (f32, f32)) {
        self.min = (self.min.0.min(p.0), self.min.1.min(p.1));
        self.max = (self.max.0.max(p.0), self.max.1.max(p.1));
    }

    pub fn union(self, other: Bounds) -> Bounds {
        let mut b = self;
        b.include(other.min);
        b.include(other.max);
        b
    }

    pub fn width(&self) -> f32 {
        self.max.0 - self.min.0
    }

    pub fn height(&self) -> f32 {
        self.max.1 - self.min.1
    }

    pub fn centre(&self) -> (f32, f32) {
        (
            (self.min.0 + self.max.0) / 2.0,
            (self.min.1 + self.max.1) / 2.0,
        )
    }

    pub fn expanded(self, by: f32) -> Bounds {
        Bounds {
            min: (self.min.0 - by, self.min.1 - by),
            max: (self.max.0 + by, self.max.1 + by),
        }
    }

    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.min.0 && p.0 <= self.max.0 && p.1 >= self.min.1 && p.1 <= self.max.1
    }

    fn area(&self) -> f32 {
        self.width() * self.height()
    }
}

fn pf(p: Point) -> (f32, f32) {
    (p.x as f32, p.y as f32)
}

/// A point of a part's symbol, placed on the sheet.
pub fn place(part: &Part, at: Point) -> (f32, f32) {
    let o = transform(at, part.rotation, part.mirror);
    (
        part.position.x as f32 + o.x as f32,
        part.position.y as f32 + o.y as f32,
    )
}

/// A point of a part's symbol given in fractions of a grid unit (arcs are drawn from these),
/// placed on the sheet: the same mirror and rotation as [`place`].
pub fn place_f(part: &Part, at: (f32, f32)) -> (f32, f32) {
    let (x, y) = if part.mirror { (-at.0, at.1) } else { at };
    let (rx, ry) = match part.rotation {
        Rotation::R0 => (x, y),
        Rotation::R90 => (y, -x),
        Rotation::R180 => (-x, -y),
        Rotation::R270 => (-y, x),
    };
    (part.position.x as f32 + rx, part.position.y as f32 + ry)
}

/// The extent of a part's symbol and pins, in sheet coordinates. `None` for a symbol the library
/// does not have.
pub fn part_bounds(part: &Part) -> Option<Bounds> {
    let def = Library::builtin().get(&part.lib_id)?;
    let mut bounds: Option<Bounds> = None;
    let mut add = |p: (f32, f32)| match bounds.as_mut() {
        Some(b) => b.include(p),
        None => bounds = Some(Bounds::around(p)),
    };
    for pin in &def.pins {
        add(place(part, pin.position));
    }
    for shape in &def.shapes {
        match shape {
            Shape::Polyline { points, .. } => {
                for p in points {
                    add(place(part, *p));
                }
            }
            Shape::Rect { min, max } => {
                add(place(part, *min));
                add(place(part, *max));
                add(place(part, Point::new(min.x, max.y)));
                add(place(part, Point::new(max.x, min.y)));
            }
            Shape::Circle { center, radius } | Shape::Arc { center, radius, .. } => {
                for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    add(place(
                        part,
                        Point::new(
                            center.x.saturating_add(dx * radius),
                            center.y.saturating_add(dy * radius),
                        ),
                    ));
                }
            }
        }
    }
    bounds
}

/// The extent of everything on a sheet.
pub fn sheet_bounds(sheet: &Sheet) -> Option<Bounds> {
    let mut all: Option<Bounds> = None;
    let mut add = |b: Bounds| {
        all = Some(match all {
            Some(a) => a.union(b),
            None => b,
        });
    };
    for part in &sheet.parts {
        if let Some(b) = part_bounds(part) {
            add(b);
        }
    }
    for wire in &sheet.wires {
        for p in &wire.points {
            add(Bounds::around(pf(*p)));
        }
    }
    for l in &sheet.labels {
        // A label's text reaches to the right of its anchor.
        let at = pf(l.position);
        add(Bounds {
            min: at,
            max: (at.0 + 1.0 + l.name.chars().count() as f32 * 1.4, at.1 + 1.0),
        });
    }
    for p in &sheet.power_ports {
        let at = pf(p.position);
        add(Bounds {
            min: (at.0 - 2.0, at.1 - 2.0),
            max: (at.0 + 2.0, at.1 + 3.0),
        });
    }
    for j in &sheet.junctions {
        add(Bounds::around(pf(j.position)));
    }
    for n in &sheet.no_connects {
        add(Bounds::around(pf(n.position)));
    }
    all
}

/// The part under `at` (grid units), the smallest one when several overlap.
pub fn hit_part(sheet: &Sheet, at: (f32, f32), slack: f32) -> Option<usize> {
    sheet
        .parts
        .iter()
        .enumerate()
        .filter_map(|(i, p)| part_bounds(p).map(|b| (i, b.expanded(slack))))
        .filter(|(_, b)| b.contains(at))
        .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
        .map(|(i, _)| i)
}

/// What the person is looking at on the drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Part {
        sheet: usize,
        part: usize,
    },
    /// A spot on a sheet (where a finding points when no part is named).
    Spot {
        sheet: usize,
        x: i32,
        y: i32,
    },
}

impl Selection {
    pub fn sheet(&self) -> usize {
        match self {
            Selection::Part { sheet, .. } | Selection::Spot { sheet, .. } => *sheet,
        }
    }
}

/// Where a finding points: the first part it names, else its position.
pub fn target_of(schematic: &Schematic, v: &Violation) -> Option<Selection> {
    let find = |refdes: &str| -> Option<Selection> {
        let order = v
            .sheet
            .into_iter()
            .chain(0..schematic.sheets.len())
            .filter(|s| *s < schematic.sheets.len());
        for sheet in order {
            let found = schematic
                .sheets
                .get(sheet)?
                .parts
                .iter()
                .position(|p| p.refdes == refdes);
            if let Some(part) = found {
                return Some(Selection::Part { sheet, part });
            }
        }
        None
    };
    v.items
        .iter()
        .filter_map(|item| part_of(item))
        .find_map(find)
        .or_else(|| {
            let sheet = v.sheet?;
            let at = v.position?;
            Some(Selection::Spot {
                sheet,
                x: at.x,
                y: at.y,
            })
        })
}

/// The centre of a selection, in grid units.
pub fn centre_of(schematic: &Schematic, sel: Selection) -> Option<(f32, f32)> {
    match sel {
        Selection::Part { sheet, part } => schematic
            .sheets
            .get(sheet)?
            .parts
            .get(part)
            .and_then(part_bounds)
            .map(|b| b.centre()),
        Selection::Spot { x, y, .. } => Some((x as f32, y as f32)),
    }
}

/// Pan and zoom: `centre` is the grid point in the middle of the canvas, `zoom` is pixels per grid
/// unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub centre: (f32, f32),
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        View {
            centre: (0.0, 0.0),
            zoom: 12.0,
        }
    }
}

impl View {
    pub const MIN_ZOOM: f32 = 1.0;
    pub const MAX_ZOOM: f32 = 160.0;

    fn clamped(mut self) -> View {
        if !self.zoom.is_finite() {
            self.zoom = View::default().zoom;
        }
        self.zoom = self.zoom.clamp(View::MIN_ZOOM, View::MAX_ZOOM);
        if !(self.centre.0.is_finite() && self.centre.1.is_finite()) {
            self.centre = (0.0, 0.0);
        }
        self
    }

    /// The view that shows `bounds` in a canvas of `size` pixels with `margin` pixels around.
    pub fn fit(bounds: Bounds, size: (f32, f32), margin: f32) -> View {
        let w = bounds.width().max(1.0);
        let h = bounds.height().max(1.0);
        let zoom_x = (size.0 - 2.0 * margin).max(10.0) / w;
        let zoom_y = (size.1 - 2.0 * margin).max(10.0) / h;
        View {
            centre: bounds.centre(),
            zoom: zoom_x.min(zoom_y),
        }
        .clamped()
    }

    /// Where a grid point is on the canvas (pixels from the canvas's top-left corner).
    pub fn to_screen(&self, p: (f32, f32), size: (f32, f32)) -> (f32, f32) {
        (
            size.0 / 2.0 + (p.0 - self.centre.0) * self.zoom,
            size.1 / 2.0 + (p.1 - self.centre.1) * self.zoom,
        )
    }

    /// The grid point under a canvas pixel.
    pub fn to_world(&self, s: (f32, f32), size: (f32, f32)) -> (f32, f32) {
        (
            self.centre.0 + (s.0 - size.0 / 2.0) / self.zoom,
            self.centre.1 + (s.1 - size.1 / 2.0) / self.zoom,
        )
    }

    /// Zoomed by `factor`, keeping the grid point under `anchor` where it is.
    pub fn zoomed_at(&self, factor: f32, anchor: (f32, f32), size: (f32, f32)) -> View {
        if !factor.is_finite() || factor <= 0.0 {
            return *self;
        }
        let before = self.to_world(anchor, size);
        let zoom = (self.zoom * factor).clamp(View::MIN_ZOOM, View::MAX_ZOOM);
        let mut next = View { zoom, ..*self };
        let after = next.to_world(anchor, size);
        next.centre = (
            next.centre.0 + before.0 - after.0,
            next.centre.1 + before.1 - after.1,
        );
        next.clamped()
    }

    /// Moved by `delta` pixels.
    pub fn panned(&self, delta: (f32, f32)) -> View {
        View {
            centre: (
                self.centre.0 - delta.0 / self.zoom,
                self.centre.1 - delta.1 / self.zoom,
            ),
            ..*self
        }
        .clamped()
    }

    pub fn centred_on(&self, p: (f32, f32)) -> View {
        View { centre: p, ..*self }.clamped()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Circuit, Sample};

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn a_fractional_point_is_placed_like_a_whole_one() {
        use circuito_core::library::Library;
        let _ = Library::builtin();
        for rotation in [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270] {
            for mirror in [false, true] {
                let part = Part {
                    refdes: "R1".into(),
                    lib_id: "R".into(),
                    value: "1k".into(),
                    position: Point::new(10, -4),
                    rotation,
                    mirror,
                    model: None,
                    fields: Default::default(),
                };
                let at = Point::new(3, -7);
                assert!(close(place_f(&part, (3.0, -7.0)), place(&part, at)));
            }
        }
    }

    #[test]
    fn screen_and_world_are_inverses() {
        let v = View {
            centre: (3.0, -2.0),
            zoom: 17.5,
        };
        let size = (800.0, 600.0);
        for p in [(0.0, 0.0), (-40.0, 22.5), (100.0, 100.0)] {
            assert!(close(v.to_world(v.to_screen(p, size), size), p));
        }
        assert!(close(v.to_screen(v.centre, size), (400.0, 300.0)));
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let v = View::default();
        let size = (800.0, 600.0);
        let anchor = (123.0, 456.0);
        let at = v.to_world(anchor, size);
        let z = v.zoomed_at(1.7, anchor, size);
        assert!(z.zoom > v.zoom);
        assert!(close(z.to_world(anchor, size), at));
        // Zoom is limited, and junk factors change nothing.
        let far = v.zoomed_at(1e9, anchor, size);
        assert_eq!(far.zoom, View::MAX_ZOOM);
        assert_eq!(v.zoomed_at(f32::NAN, anchor, size), v);
        assert_eq!(v.zoomed_at(-2.0, anchor, size), v);
        assert_eq!(v.zoomed_at(0.0, anchor, size), v);
    }

    #[test]
    fn panning_moves_the_drawing_with_the_pointer() {
        let v = View::default();
        let size = (400.0, 300.0);
        let p = v.to_screen((5.0, 5.0), size);
        let moved = v.panned((30.0, -10.0));
        let q = moved.to_screen((5.0, 5.0), size);
        assert!(close((q.0 - p.0, q.1 - p.1), (30.0, -10.0)));
    }

    #[test]
    fn fit_shows_everything_with_a_margin() {
        let c = Circuit::sample(Sample::RcLowPass);
        let sheet = &c.schematic.as_ref().unwrap().sheets[0];
        let b = sheet_bounds(sheet).unwrap();
        let size = (900.0, 500.0);
        let v = View::fit(b, size, 40.0);
        let lo = v.to_screen(b.min, size);
        let hi = v.to_screen(b.max, size);
        assert!(lo.0 >= 39.0 && lo.1 >= 39.0, "{lo:?}");
        assert!(hi.0 <= 861.0 && hi.1 <= 461.0, "{hi:?}");
        // Degenerate input stays finite.
        let tiny = View::fit(Bounds::around((0.0, 0.0)), (0.0, 0.0), 1000.0);
        assert!(tiny.zoom.is_finite() && tiny.zoom >= View::MIN_ZOOM);
    }

    #[test]
    fn a_part_is_found_by_a_point_on_it_and_not_by_one_far_away() {
        let c = Circuit::sample(Sample::RcLowPass);
        let sheet = &c.schematic.as_ref().unwrap().sheets[0];
        let c1 = sheet.parts.iter().position(|p| p.refdes == "C1").unwrap();
        assert_eq!(hit_part(sheet, (26.0, 0.0), 0.5), Some(c1));
        assert_eq!(hit_part(sheet, (26.0, 40.0), 0.5), None);
        // A rotated part: R1 is horizontal.
        let r1 = sheet.parts.iter().position(|p| p.refdes == "R1").unwrap();
        assert_eq!(hit_part(sheet, (14.0, -4.0), 0.5), Some(r1));
        assert_eq!(hit_part(sheet, (14.0, 0.0), 0.2), None);
    }

    #[test]
    fn a_finding_points_at_the_part_it_names() {
        let c = Circuit::sample(Sample::RcLowPass);
        let s = c.schematic.as_ref().unwrap();
        let v1 = s.sheets[0]
            .parts
            .iter()
            .position(|p| p.refdes == "V1")
            .unwrap();
        let mut v = Violation {
            code: "E001",
            rule: circuito_erc::Rule::PinUnconnected,
            severity: circuito_erc::Severity::Warning,
            sheet: Some(0),
            sheet_name: None,
            position: Some(Point::new(7, 7)),
            message: String::new(),
            items: vec!["net out".into(), "V1.2".into()],
        };
        assert_eq!(
            target_of(s, &v),
            Some(Selection::Part { sheet: 0, part: v1 })
        );
        v.items = vec!["net out".into(), "Q9".into()];
        assert_eq!(
            target_of(s, &v),
            Some(Selection::Spot {
                sheet: 0,
                x: 7,
                y: 7
            })
        );
        v.position = None;
        assert_eq!(target_of(s, &v), None);
        v.sheet = Some(99);
        v.items = vec!["R1".into()];
        // A sheet that does not exist is skipped, not trusted.
        assert!(matches!(
            target_of(s, &v),
            Some(Selection::Part { sheet: 0, .. })
        ));
    }

    #[test]
    fn centres_come_from_the_part_or_the_spot() {
        let c = Circuit::sample(Sample::Divider);
        let s = c.schematic.as_ref().unwrap();
        let centre = centre_of(s, Selection::Part { sheet: 0, part: 0 }).unwrap();
        assert!(centre.0.abs() < 4.0);
        assert_eq!(
            centre_of(
                s,
                Selection::Spot {
                    sheet: 0,
                    x: 3,
                    y: 4
                }
            ),
            Some((3.0, 4.0))
        );
        assert_eq!(centre_of(s, Selection::Part { sheet: 5, part: 0 }), None);
    }

    #[test]
    fn an_unknown_symbol_has_no_bounds_and_an_empty_sheet_none() {
        let mut b = circuito_core::schematic::builder::Builder::new("x");
        b.place("NOPE", "U1", "", 0, 0);
        let s = b.build();
        assert!(part_bounds(&s.sheets[0].parts[0]).is_none());
        assert!(sheet_bounds(&Sheet::default()).is_none());
    }
}

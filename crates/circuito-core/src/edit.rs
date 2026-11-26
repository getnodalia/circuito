// SPDX-License-Identifier: MIT OR Apache-2.0
//! Editing a schematic: moving, turning, flipping, deleting, copying and pasting what is selected,
//! drawing wires with their junctions, and the undo history.
//!
//! Everything here works on plain [`Sheet`] data and knows nothing about a window. The interactive
//! editor, the script synchroniser and the tests all call these functions, so a wire drawn with the
//! mouse and a wire written in a script end up as the same data.
//!
//! # What an edit keeps true
//! * **Wires follow the parts they are attached to.** Moving, turning or flipping a part drags the
//!   wire ends, labels, power ports, junctions and no-connect flags that sit on its pins, and keeps
//!   those wires at right angles.
//! * **Junctions are not left to the person.** [`normalize`] adds the junction a wire end or a pin
//!   needs where it lands on the middle of another wire (the connectivity rules would otherwise
//!   leave it unconnected), and removes junctions that no longer join anything.
//! * **Nothing panics on hostile data.** Coordinates saturate, indexes are looked up with `get`, and
//!   a symbol the library does not have simply has no pins.

use crate::library::{transform, Library, PartDef};
use crate::schematic::{
    Junction, Label, NoConnect, Part, Point, PowerPort, Rotation, Schematic, Sheet, Wire, MAX_COORD,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// One thing on a sheet, by its index in the sheet's list for that kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    /// A part.
    Part(usize),
    /// A wire.
    Wire(usize),
    /// A net label.
    Label(usize),
    /// A power port.
    Port(usize),
    /// A junction.
    Junction(usize),
    /// A no-connect flag.
    NoConnect(usize),
}

/// A set of items: a selection.
pub type Items = BTreeSet<Item>;

/// Where indexes went after [`normalize`] removed or added wires and junctions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remap {
    /// The new index of each old wire, `None` when it was removed.
    pub wires: Vec<Option<usize>>,
    /// The new index of each old junction, `None` when it was removed.
    pub junctions: Vec<Option<usize>>,
}

impl Remap {
    /// A selection with wires and junctions renumbered; what was removed drops out.
    #[must_use]
    pub fn apply(&self, items: &Items) -> Items {
        items
            .iter()
            .filter_map(|it| match *it {
                Item::Wire(i) => self.wires.get(i).copied().flatten().map(Item::Wire),
                Item::Junction(i) => self.junctions.get(i).copied().flatten().map(Item::Junction),
                other => Some(other),
            })
            .collect()
    }
}

/// The absolute position of each pin of a part, in the symbol's pin order. Empty for a symbol the
/// library does not have.
#[must_use]
pub fn pin_points(part: &Part) -> Vec<Point> {
    let Some(def) = Library::builtin().get(&part.lib_id) else {
        return Vec::new();
    };
    def.pins
        .iter()
        .map(|pd| {
            let o = transform(pd.position, part.rotation, part.mirror);
            Point::new(
                part.position.x.saturating_add(o.x),
                part.position.y.saturating_add(o.y),
            )
        })
        .collect()
}

/// `p` moved by `(dx, dy)`, kept inside the coordinate range.
#[must_use]
pub fn shifted(p: Point, dx: i32, dy: i32) -> Point {
    Point::new(
        p.x.saturating_add(dx).clamp(-MAX_COORD, MAX_COORD),
        p.y.saturating_add(dy).clamp(-MAX_COORD, MAX_COORD),
    )
}

// --- Naming -------------------------------------------------------------------------------------

/// The letters a reference designator starts with (`R` for `R12`).
#[must_use]
pub fn refdes_prefix(refdes: &str) -> &str {
    let end = refdes
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphabetic())
        .map_or(refdes.len(), |(i, _)| i);
    refdes.get(..end).unwrap_or("")
}

/// The first free reference designator with this prefix: `R1`, then `R2`, and so on, across every
/// sheet and ignoring case.
#[must_use]
pub fn next_refdes(s: &Schematic, prefix: &str) -> String {
    let prefix = if prefix.is_empty() { "X" } else { prefix };
    let used: BTreeSet<String> = s
        .sheets
        .iter()
        .flat_map(|sh| sh.parts.iter())
        .map(|p| p.refdes.to_ascii_uppercase())
        .collect();
    let mut n = 1u32;
    loop {
        let candidate = format!("{prefix}{n}");
        if !used.contains(&candidate.to_ascii_uppercase()) || n == u32::MAX {
            return candidate;
        }
        n += 1;
    }
}

/// A new part of the library symbol `def` at `at`, with its default value and no rotation.
#[must_use]
pub fn new_part(def: &PartDef, refdes: &str, at: Point) -> Part {
    Part {
        refdes: refdes.to_string(),
        lib_id: def.id.to_string(),
        value: def.default_value.to_string(),
        position: at,
        rotation: Rotation::R0,
        mirror: false,
        model: None,
        fields: BTreeMap::new(),
    }
}

/// Why a name could not be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    /// The name is empty or has characters a reference designator cannot have.
    Invalid,
    /// Another part already has it.
    Taken,
}

/// Give a part a new reference designator.
///
/// # Errors
/// The name is not usable ([`RenameError::Invalid`]) or another part has it
/// ([`RenameError::Taken`]); the part keeps its name.
pub fn rename_part(
    s: &mut Schematic,
    sheet: usize,
    part: usize,
    new: &str,
) -> Result<(), RenameError> {
    let new = new.trim();
    if new.is_empty() || new.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(RenameError::Invalid);
    }
    let taken = s.sheets.iter().enumerate().any(|(si, sh)| {
        sh.parts
            .iter()
            .enumerate()
            .any(|(pi, p)| !(si == sheet && pi == part) && p.refdes.eq_ignore_ascii_case(new))
    });
    if taken {
        return Err(RenameError::Taken);
    }
    if let Some(p) = s
        .sheets
        .get_mut(sheet)
        .and_then(|sh| sh.parts.get_mut(part))
    {
        p.refdes = new.to_string();
    }
    Ok(())
}

// --- Geometry helpers ---------------------------------------------------------------------------

/// Whether `q` lies on the segment `a`–`b` (ends included).
#[must_use]
pub fn on_segment(q: Point, a: Point, b: Point) -> bool {
    let (qx, qy) = (i64::from(q.x), i64::from(q.y));
    let (ax, ay) = (i64::from(a.x), i64::from(a.y));
    let (bx, by) = (i64::from(b.x), i64::from(b.y));
    if (bx - ax) * (qy - ay) - (by - ay) * (qx - ax) != 0 {
        return false;
    }
    qx >= ax.min(bx) && qx <= ax.max(bx) && qy >= ay.min(by) && qy <= ay.max(by)
}

/// A wire from `a` to `b` at right angles: horizontal first, or vertical first. Straight when the
/// two share a row or a column.
#[must_use]
pub fn route(a: Point, b: Point, horizontal_first: bool) -> Vec<Point> {
    if a == b {
        return vec![a];
    }
    if a.x == b.x || a.y == b.y {
        return vec![a, b];
    }
    let corner = if horizontal_first {
        Point::new(b.x, a.y)
    } else {
        Point::new(a.x, b.y)
    };
    vec![a, corner, b]
}

fn simplify(points: &mut Vec<Point>) {
    points.dedup();
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for &p in points.iter() {
        while out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let same_line = (b.x - a.x).signum() * (p.y - b.y).signum()
                == (b.y - a.y).signum() * (p.x - b.x).signum()
                && (b.x == a.x) == (p.x == b.x)
                && (b.y == a.y) == (p.y == b.y);
            // Collinear and still heading the same way: the middle vertex is redundant.
            let same_way = (b.x - a.x).signum() == (p.x - b.x).signum()
                && (b.y - a.y).signum() == (p.y - b.y).signum();
            if same_line && same_way && (a.x == b.x || a.y == b.y) {
                out.pop();
            } else {
                break;
            }
        }
        out.push(p);
    }
    *points = out;
}

// --- Normalising --------------------------------------------------------------------------------

/// Tidy a sheet after an edit: drop zero-length and duplicate wires and redundant corners, add the
/// junctions that wire ends and pins landing on the middle of another wire need, and remove
/// junctions that join nothing.
///
/// Returns where wires and junctions went, so a selection can follow them.
pub fn normalize(sheet: &mut Sheet) -> Remap {
    let mut remap = Remap::default();
    // Wires.
    let mut seen: Vec<Vec<Point>> = Vec::new();
    let mut kept: Vec<Wire> = Vec::with_capacity(sheet.wires.len());
    for w in std::mem::take(&mut sheet.wires) {
        let mut w = w;
        simplify(&mut w.points);
        if w.points.len() < 2 {
            remap.wires.push(None);
            continue;
        }
        let mut rev = w.points.clone();
        rev.reverse();
        if seen.iter().any(|s| *s == w.points || *s == rev) {
            remap.wires.push(None);
            continue;
        }
        seen.push(w.points.clone());
        remap.wires.push(Some(kept.len()));
        kept.push(w);
    }
    sheet.wires = kept;
    // Junctions.
    let needed = junction_points(sheet);
    let mut kept_j: Vec<Junction> = Vec::new();
    let mut have: BTreeSet<Point> = BTreeSet::new();
    for j in std::mem::take(&mut sheet.junctions) {
        let useful = needed.get(&j.position).is_some_and(|c| c.keep_existing());
        if useful && have.insert(j.position) {
            remap.junctions.push(Some(kept_j.len()));
            kept_j.push(j);
        } else {
            remap.junctions.push(None);
        }
    }
    for (p, c) in &needed {
        if c.needs_junction() && have.insert(*p) {
            kept_j.push(Junction { position: *p });
        }
    }
    sheet.junctions = kept_j;
    remap
}

#[derive(Default, Clone, Copy)]
struct Contacts {
    /// Wire ends and pins at the point.
    ends_and_pins: u32,
    /// Wires whose middle passes through the point.
    interior: u32,
}

impl Contacts {
    fn needs_junction(self) -> bool {
        (self.interior >= 1 && self.ends_and_pins >= 1) || self.ends_and_pins >= 3
    }
    fn keep_existing(self) -> bool {
        self.needs_junction() || self.interior >= 2
    }
}

fn junction_points(sheet: &Sheet) -> BTreeMap<Point, Contacts> {
    let mut at: BTreeMap<Point, Contacts> = BTreeMap::new();
    for w in &sheet.wires {
        if let (Some(f), Some(l)) = (w.points.first(), w.points.last()) {
            at.entry(*f).or_default().ends_and_pins += 1;
            if l != f {
                at.entry(*l).or_default().ends_and_pins += 1;
            }
        }
    }
    for part in &sheet.parts {
        for p in pin_points(part) {
            at.entry(p).or_default().ends_and_pins += 1;
        }
    }
    // Points where a wire's middle can touch something: only points that already have contacts,
    // or an existing junction, matter.
    let mut interest: BTreeSet<(i32, i32)> = at.keys().map(|p| (p.x, p.y)).collect();
    let mut interest_yx: BTreeSet<(i32, i32)> = at.keys().map(|p| (p.y, p.x)).collect();
    for j in &sheet.junctions {
        interest.insert((j.position.x, j.position.y));
        interest_yx.insert((j.position.y, j.position.x));
    }
    for w in &sheet.wires {
        let (Some(first), Some(last)) = (w.points.first().copied(), w.points.last().copied())
        else {
            continue;
        };
        let mut touched: BTreeSet<Point> = BTreeSet::new();
        for seg in w.points.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let candidates: Vec<Point> = if a.x == b.x {
                interest
                    .range((a.x, a.y.min(b.y))..=(a.x, a.y.max(b.y)))
                    .map(|&(x, y)| Point::new(x, y))
                    .collect()
            } else if a.y == b.y {
                interest_yx
                    .range((a.y, a.x.min(b.x))..=(a.y, a.x.max(b.x)))
                    .map(|&(y, x)| Point::new(x, y))
                    .collect()
            } else {
                interest
                    .range((a.x.min(b.x), i32::MIN)..=(a.x.max(b.x), i32::MAX))
                    .map(|&(x, y)| Point::new(x, y))
                    .filter(|q| on_segment(*q, a, b))
                    .collect()
            };
            for q in candidates {
                if q != first && q != last {
                    touched.insert(q);
                }
            }
        }
        for q in touched {
            at.entry(q).or_default().interior += 1;
        }
    }
    for j in &sheet.junctions {
        at.entry(j.position).or_default();
    }
    at
}

// --- Moving, turning, flipping ------------------------------------------------------------------

/// The part indexes in a selection.
fn selected_parts(items: &Items) -> BTreeSet<usize> {
    items
        .iter()
        .filter_map(|i| {
            if let Item::Part(k) = i {
                Some(*k)
            } else {
                None
            }
        })
        .collect()
}

/// Apply `f` to every point of the selected items, and drag along what is attached to them.
fn transform_items(
    sheet: &mut Sheet,
    items: &Items,
    f: &dyn Fn(Point) -> Point,
    part_op: &dyn Fn(&mut Part),
) {
    let mut map: HashMap<Point, Point> = HashMap::new();
    let selected_wires: BTreeSet<usize> = items
        .iter()
        .filter_map(|i| {
            if let Item::Wire(k) = i {
                Some(*k)
            } else {
                None
            }
        })
        .collect();
    for it in items {
        match *it {
            Item::Part(k) => {
                if let Some(p) = sheet.parts.get_mut(k) {
                    let old = pin_points(p);
                    p.position = f(p.position);
                    part_op(p);
                    for (o, n) in old.into_iter().zip(pin_points(p)) {
                        map.entry(o).or_insert(n);
                    }
                }
            }
            Item::Wire(k) => {
                if let Some(w) = sheet.wires.get_mut(k) {
                    for q in &mut w.points {
                        let n = f(*q);
                        map.entry(*q).or_insert(n);
                        *q = n;
                    }
                }
            }
            Item::Label(k) => {
                if let Some(l) = sheet.labels.get_mut(k) {
                    let n = f(l.position);
                    map.entry(l.position).or_insert(n);
                    l.position = n;
                }
            }
            Item::Port(k) => {
                if let Some(l) = sheet.power_ports.get_mut(k) {
                    let n = f(l.position);
                    map.entry(l.position).or_insert(n);
                    l.position = n;
                }
            }
            Item::Junction(k) => {
                if let Some(l) = sheet.junctions.get_mut(k) {
                    let n = f(l.position);
                    map.entry(l.position).or_insert(n);
                    l.position = n;
                }
            }
            Item::NoConnect(k) => {
                if let Some(l) = sheet.no_connects.get_mut(k) {
                    let n = f(l.position);
                    map.entry(l.position).or_insert(n);
                    l.position = n;
                }
            }
        }
    }
    // Moved wires leave their old ends in the map keyed by the old points; the loop above stored
    // them before overwriting. Now stretch what is attached.
    for (k, w) in sheet.wires.iter_mut().enumerate() {
        if !selected_wires.contains(&k) {
            stretch(w, &map);
        }
    }
    let moved = |it: Item| items.contains(&it);
    for (k, l) in sheet.labels.iter_mut().enumerate() {
        if !moved(Item::Label(k)) {
            if let Some(n) = map.get(&l.position) {
                l.position = *n;
            }
        }
    }
    for (k, l) in sheet.power_ports.iter_mut().enumerate() {
        if !moved(Item::Port(k)) {
            if let Some(n) = map.get(&l.position) {
                l.position = *n;
            }
        }
    }
    for (k, l) in sheet.junctions.iter_mut().enumerate() {
        if !moved(Item::Junction(k)) {
            if let Some(n) = map.get(&l.position) {
                l.position = *n;
            }
        }
    }
    for (k, l) in sheet.no_connects.iter_mut().enumerate() {
        if !moved(Item::NoConnect(k)) {
            if let Some(n) = map.get(&l.position) {
                l.position = *n;
            }
        }
    }
}

fn stretch(w: &mut Wire, map: &HashMap<Point, Point>) {
    let n = w.points.len();
    if n < 2 {
        return;
    }
    let (first, last) = (w.points[0], w.points[n - 1]);
    if let (Some(a), Some(b)) = (map.get(&first), map.get(&last)) {
        let d1 = (a.x - first.x, a.y - first.y);
        let d2 = (b.x - last.x, b.y - last.y);
        if d1 == d2 {
            for q in &mut w.points {
                *q = shifted(*q, d1.0, d1.1);
            }
            return;
        }
    }
    if let Some(&new_last) = map.get(&last) {
        move_end(&mut w.points, true, new_last);
    }
    if let Some(&new_first) = map.get(&first) {
        move_end(&mut w.points, false, new_first);
    }
}

/// Move one end of a polyline, adding a corner when the last segment would stop being square.
fn move_end(points: &mut Vec<Point>, at_end: bool, new: Point) {
    if !at_end {
        points.reverse();
    }
    let n = points.len();
    if let Some(&e) = points.last() {
        if new != e {
            let q = if n >= 2 { points[n - 2] } else { e };
            if let Some(slot) = points.last_mut() {
                *slot = new;
            }
            if q.x != new.x && q.y != new.y {
                let corner = if q.y == e.y && q.x != e.x {
                    Point::new(new.x, q.y)
                } else if q.x == e.x && q.y != e.y {
                    Point::new(q.x, new.y)
                } else {
                    Point::new(new.x, q.y)
                };
                points.insert(n - 1, corner);
            }
        }
    }
    if !at_end {
        points.reverse();
    }
}

/// Move the selected items by `(dx, dy)`; wires, labels and ports attached to the parts follow.
pub fn translate(sheet: &mut Sheet, items: &Items, dx: i32, dy: i32) {
    if items.is_empty() || (dx == 0 && dy == 0) {
        return;
    }
    transform_items(sheet, items, &|p| shifted(p, dx, dy), &|_| {});
}

/// The middle of the selected items, rounded to the grid: what turning and flipping pivot on.
/// A single part pivots on its own origin.
#[must_use]
pub fn pivot(sheet: &Sheet, items: &Items) -> Point {
    let parts = selected_parts(items);
    if items.len() == 1 && parts.len() == 1 {
        if let Some(p) = parts.iter().next().and_then(|k| sheet.parts.get(*k)) {
            return p.position;
        }
    }
    let mut pts: Vec<Point> = Vec::new();
    for it in items {
        match *it {
            Item::Part(k) => pts.extend(sheet.parts.get(k).map(|p| p.position)),
            Item::Wire(k) => pts.extend(
                sheet
                    .wires
                    .get(k)
                    .into_iter()
                    .flat_map(|w| w.points.clone()),
            ),
            Item::Label(k) => pts.extend(sheet.labels.get(k).map(|p| p.position)),
            Item::Port(k) => pts.extend(sheet.power_ports.get(k).map(|p| p.position)),
            Item::Junction(k) => pts.extend(sheet.junctions.get(k).map(|p| p.position)),
            Item::NoConnect(k) => pts.extend(sheet.no_connects.get(k).map(|p| p.position)),
        }
    }
    let (Some(minx), Some(maxx), Some(miny), Some(maxy)) = (
        pts.iter().map(|p| p.x).min(),
        pts.iter().map(|p| p.x).max(),
        pts.iter().map(|p| p.y).min(),
        pts.iter().map(|p| p.y).max(),
    ) else {
        return Point::new(0, 0);
    };
    Point::new(
        ((i64::from(minx) + i64::from(maxx)) / 2) as i32,
        ((i64::from(miny) + i64::from(maxy)) / 2) as i32,
    )
}

fn turn_rotation(r: Rotation, counter_clockwise: bool) -> Rotation {
    match (r, counter_clockwise) {
        (Rotation::R0, true) | (Rotation::R180, false) => Rotation::R90,
        (Rotation::R90, true) | (Rotation::R270, false) => Rotation::R180,
        (Rotation::R180, true) | (Rotation::R0, false) => Rotation::R270,
        (Rotation::R270, true) | (Rotation::R90, false) => Rotation::R0,
    }
}

/// Turn the selection a quarter turn about its middle. Counter-clockwise on screen when
/// `counter_clockwise` is set.
pub fn rotate(sheet: &mut Sheet, items: &Items, counter_clockwise: bool) {
    if items.is_empty() {
        return;
    }
    let c = pivot(sheet, items);
    let f = move |p: Point| {
        let (dx, dy) = (p.x - c.x, p.y - c.y);
        // The screen's y axis points down, so counter-clockwise is (x, y) -> (y, -x).
        let (rx, ry) = if counter_clockwise {
            (dy, -dx)
        } else {
            (-dy, dx)
        };
        shifted(c, rx, ry)
    };
    transform_items(sheet, items, &f, &|p| {
        p.rotation = turn_rotation(p.rotation, counter_clockwise);
    });
}

/// Flip the selection left to right (`horizontal`) or top to bottom about its middle.
pub fn flip(sheet: &mut Sheet, items: &Items, horizontal: bool) {
    if items.is_empty() {
        return;
    }
    let c = pivot(sheet, items);
    let f = move |p: Point| {
        if horizontal {
            Point::new(2 * c.x - p.x, p.y)
        } else {
            Point::new(p.x, 2 * c.y - p.y)
        }
    };
    transform_items(sheet, items, &f, &|p| {
        // A flip on screen is a mirror of the symbol with the turn undone: see the module tests.
        p.mirror = !p.mirror;
        p.rotation = match (p.rotation, horizontal) {
            (Rotation::R90, true) => Rotation::R270,
            (Rotation::R270, true) => Rotation::R90,
            (Rotation::R0, false) => Rotation::R180,
            (Rotation::R180, false) => Rotation::R0,
            (r, _) => r,
        };
    });
}

// --- Adding and removing ------------------------------------------------------------------------

/// Draw a wire through `points` (made square by the caller) and tidy the sheet. Returns the new
/// wire's index after tidying, or `None` when it was too short to keep.
pub fn add_wire(sheet: &mut Sheet, points: &[Point]) -> (Option<usize>, Remap) {
    sheet.wires.push(Wire {
        points: points.to_vec(),
    });
    let new = sheet.wires.len() - 1;
    let remap = normalize(sheet);
    (remap.wires.get(new).copied().flatten(), remap)
}

/// Remove the selected items.
pub fn delete(sheet: &mut Sheet, items: &Items) -> Remap {
    for kind in 0..6 {
        let mut idx: Vec<usize> = items
            .iter()
            .filter_map(|i| match (kind, *i) {
                (0, Item::Part(k))
                | (1, Item::Wire(k))
                | (2, Item::Label(k))
                | (3, Item::Port(k))
                | (4, Item::Junction(k))
                | (5, Item::NoConnect(k)) => Some(k),
                _ => None,
            })
            .collect();
        idx.sort_unstable_by(|a, b| b.cmp(a));
        for k in idx {
            match kind {
                0 if k < sheet.parts.len() => {
                    sheet.parts.remove(k);
                }
                1 if k < sheet.wires.len() => {
                    sheet.wires.remove(k);
                }
                2 if k < sheet.labels.len() => {
                    sheet.labels.remove(k);
                }
                3 if k < sheet.power_ports.len() => {
                    sheet.power_ports.remove(k);
                }
                4 if k < sheet.junctions.len() => {
                    sheet.junctions.remove(k);
                }
                5 if k < sheet.no_connects.len() => {
                    sheet.no_connects.remove(k);
                }
                _ => {}
            }
        }
    }
    normalize(sheet)
}

/// A copy of some items, for pasting somewhere else.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clip {
    /// Parts.
    pub parts: Vec<Part>,
    /// Wires.
    pub wires: Vec<Wire>,
    /// Labels.
    pub labels: Vec<Label>,
    /// Power ports.
    pub ports: Vec<PowerPort>,
    /// Junctions.
    pub junctions: Vec<Junction>,
    /// No-connect flags.
    pub no_connects: Vec<NoConnect>,
}

impl Clip {
    /// Whether nothing was copied.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
            && self.wires.is_empty()
            && self.labels.is_empty()
            && self.ports.is_empty()
            && self.junctions.is_empty()
            && self.no_connects.is_empty()
    }
}

/// Copy the selected items.
#[must_use]
pub fn copy(sheet: &Sheet, items: &Items) -> Clip {
    let mut clip = Clip::default();
    for it in items {
        match *it {
            Item::Part(k) => clip.parts.extend(sheet.parts.get(k).cloned()),
            Item::Wire(k) => clip.wires.extend(sheet.wires.get(k).cloned()),
            Item::Label(k) => clip.labels.extend(sheet.labels.get(k).cloned()),
            Item::Port(k) => clip.ports.extend(sheet.power_ports.get(k).cloned()),
            Item::Junction(k) => clip.junctions.extend(sheet.junctions.get(k).cloned()),
            Item::NoConnect(k) => clip.no_connects.extend(sheet.no_connects.get(k).cloned()),
        }
    }
    clip
}

/// Paste a copy of `clip` moved by `(dx, dy)` onto sheet `sheet_index`. Pasted parts get fresh
/// reference designators. Returns the pasted items, to select them.
pub fn paste(s: &mut Schematic, sheet_index: usize, clip: &Clip, dx: i32, dy: i32) -> Items {
    let mut out = Items::new();
    if sheet_index >= s.sheets.len() {
        return out;
    }
    for part in &clip.parts {
        let refdes = next_refdes(s, refdes_prefix(&part.refdes));
        let mut p = part.clone();
        p.refdes = refdes;
        p.position = shifted(p.position, dx, dy);
        let Some(sheet) = s.sheets.get_mut(sheet_index) else {
            return out;
        };
        sheet.parts.push(p);
        out.insert(Item::Part(sheet.parts.len() - 1));
    }
    let Some(sheet) = s.sheets.get_mut(sheet_index) else {
        return out;
    };
    for w in &clip.wires {
        let mut w = w.clone();
        for q in &mut w.points {
            *q = shifted(*q, dx, dy);
        }
        sheet.wires.push(w);
        out.insert(Item::Wire(sheet.wires.len() - 1));
    }
    for l in &clip.labels {
        let mut l = l.clone();
        l.position = shifted(l.position, dx, dy);
        sheet.labels.push(l);
        out.insert(Item::Label(sheet.labels.len() - 1));
    }
    for l in &clip.ports {
        let mut l = l.clone();
        l.position = shifted(l.position, dx, dy);
        sheet.power_ports.push(l);
        out.insert(Item::Port(sheet.power_ports.len() - 1));
    }
    for l in &clip.junctions {
        let mut l = *l;
        l.position = shifted(l.position, dx, dy);
        sheet.junctions.push(l);
        out.insert(Item::Junction(sheet.junctions.len() - 1));
    }
    for l in &clip.no_connects {
        let mut l = *l;
        l.position = shifted(l.position, dx, dy);
        sheet.no_connects.push(l);
        out.insert(Item::NoConnect(sheet.no_connects.len() - 1));
    }
    // Pasting changes wire counts only by appending, so the new indexes stay valid unless tidying
    // removed some; map them through.
    let remap = normalize(sheet);
    remap.apply(&out)
}

/// A power port at `at` for the net `name`: ground for `0` and `GND`, a supply bar otherwise.
#[must_use]
pub fn new_power_port(name: &str, at: Point) -> PowerPort {
    use crate::schematic::PowerStyle;
    let style = if matches!(name.trim().to_ascii_lowercase().as_str(), "0" | "gnd") {
        PowerStyle::Ground
    } else {
        PowerStyle::Supply
    };
    PowerPort {
        name: name.to_string(),
        position: at,
        style,
    }
}

// --- Undo ---------------------------------------------------------------------------------------

/// The most steps an [`History`] keeps.
pub const HISTORY_LIMIT: usize = 200;

/// Snapshots to go back to, and forward to again.
///
/// A step is the whole state before an edit. Schematics are small, so a snapshot is the simplest
/// thing that cannot get out of step with the edits.
#[derive(Debug, Clone)]
pub struct History<T: Clone> {
    undo: Vec<(T, String)>,
    redo: Vec<(T, String)>,
    /// The key of the last recorded step while it can still absorb more of the same kind.
    open: Option<String>,
}

impl<T: Clone> Default for History<T> {
    fn default() -> Self {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
            open: None,
        }
    }
}

impl<T: Clone> History<T> {
    /// Remember `before` as the state to return to, under `label`. A new edit forgets what was
    /// undone.
    pub fn record(&mut self, before: T, label: &str) {
        self.open = None;
        self.redo.clear();
        self.undo.push((before, label.to_string()));
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
    }

    /// Like [`record`](Self::record), but a run of edits with the same `key` (typing in a field,
    /// nudging with the arrow keys) is one step: only the first remembers its `before`.
    pub fn record_merging(&mut self, key: &str, before: T, label: &str) {
        if self.open.as_deref() == Some(key) && !self.undo.is_empty() {
            self.redo.clear();
            return;
        }
        self.record(before, label);
        self.open = Some(key.to_string());
    }

    /// End the run [`record_merging`](Self::record_merging) is absorbing.
    pub fn close_run(&mut self) {
        self.open = None;
    }

    /// Step back: returns the state to show and remembers `current` to redo.
    pub fn undo(&mut self, current: T) -> Option<(T, String)> {
        self.open = None;
        let (state, label) = self.undo.pop()?;
        self.redo.push((current, label.clone()));
        Some((state, label))
    }

    /// Step forward again.
    pub fn redo(&mut self, current: T) -> Option<(T, String)> {
        self.open = None;
        let (state, label) = self.redo.pop()?;
        self.undo.push((current, label.clone()));
        Some((state, label))
    }

    /// Whether there is something to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether there is something to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The label of the step an undo would take back.
    #[must_use]
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|(_, l)| l.as_str())
    }

    /// The label of the step a redo would bring back.
    #[must_use]
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|(_, l)| l.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::builder::Builder;
    use crate::schematic::resolve_nets;

    fn nets_of(s: &Schematic) -> Vec<Vec<(usize, usize)>> {
        let r = resolve_nets(s, Library::builtin()).unwrap();
        let mut v: Vec<Vec<(usize, usize)>> = r
            .nets
            .iter()
            .map(|n| n.pins.iter().map(|p| (p.part, p.pin)).collect())
            .collect();
        v.sort();
        v
    }

    fn divider() -> Schematic {
        let mut b = Builder::new("t");
        let r1 = b.place("R", "R1", "1k", 0, 0);
        let r2 = b.place("R", "R2", "1k", 0, 12);
        b.connect((r1, "2"), (r2, "1"));
        b.build()
    }

    fn all(s: &Sheet) -> Items {
        let mut i = Items::new();
        (0..s.parts.len()).for_each(|k| {
            i.insert(Item::Part(k));
        });
        (0..s.wires.len()).for_each(|k| {
            i.insert(Item::Wire(k));
        });
        i
    }

    #[test]
    fn refdes_are_allocated_in_order_and_gaps_are_reused() {
        let mut s = divider();
        assert_eq!(next_refdes(&s, "R"), "R3");
        s.sheets[0].parts.remove(0);
        assert_eq!(next_refdes(&s, "R"), "R1");
        assert_eq!(next_refdes(&s, "C"), "C1");
        assert_eq!(next_refdes(&s, ""), "X1");
        assert_eq!(refdes_prefix("RV12"), "RV");
        assert_eq!(refdes_prefix("12"), "");
    }

    #[test]
    fn renaming_refuses_duplicates_and_junk() {
        let mut s = divider();
        assert_eq!(rename_part(&mut s, 0, 0, "r2"), Err(RenameError::Taken));
        assert_eq!(rename_part(&mut s, 0, 0, " "), Err(RenameError::Invalid));
        assert_eq!(rename_part(&mut s, 0, 0, "a b"), Err(RenameError::Invalid));
        assert_eq!(rename_part(&mut s, 0, 0, "R1"), Ok(()));
        assert_eq!(rename_part(&mut s, 0, 0, "RA"), Ok(()));
        assert_eq!(s.sheets[0].parts[0].refdes, "RA");
        assert_eq!(rename_part(&mut s, 0, 9, "Z"), Ok(()));
    }

    #[test]
    fn moving_a_part_drags_its_wires_and_keeps_the_net() {
        let mut s = divider();
        let before = nets_of(&s);
        let sheet = &mut s.sheets[0];
        let mut sel = Items::new();
        sel.insert(Item::Part(1));
        translate(sheet, &sel, 6, 3);
        normalize(sheet);
        assert_eq!(nets_of(&s), before);
        // Every wire is still made of horizontal and vertical segments.
        for w in &s.sheets[0].wires {
            for seg in w.points.windows(2) {
                assert!(seg[0].x == seg[1].x || seg[0].y == seg[1].y, "{seg:?}");
            }
        }
        assert_eq!(s.sheets[0].parts[1].position, Point::new(6, 15));
    }

    #[test]
    fn moving_everything_moves_wires_as_one() {
        let mut s = divider();
        let sel = all(&s.sheets[0]);
        let wires = s.sheets[0].wires.clone();
        translate(&mut s.sheets[0], &sel, 5, -2);
        for (a, b) in wires.iter().zip(&s.sheets[0].wires) {
            assert_eq!(a.points.len(), b.points.len());
            for (p, q) in a.points.iter().zip(&b.points) {
                assert_eq!(*q, Point::new(p.x + 5, p.y - 2));
            }
        }
    }

    #[test]
    fn labels_and_ports_on_pins_follow_the_part() {
        let mut b = Builder::new("t");
        let r1 = b.place("R", "R1", "1k", 0, 0);
        b.label_pin(r1, "1", "in");
        b.power_pin(r1, "2", "GND");
        let mut s = b.build();
        let mut sel = Items::new();
        sel.insert(Item::Part(0));
        translate(&mut s.sheets[0], &sel, 4, 4);
        assert_eq!(s.sheets[0].labels[0].position, Point::new(4, 0));
        assert_eq!(s.sheets[0].power_ports[0].position, Point::new(4, 8));
    }

    #[test]
    fn turning_a_part_keeps_wires_attached_and_square() {
        let mut s = divider();
        let before = nets_of(&s);
        let mut sel = Items::new();
        sel.insert(Item::Part(1));
        rotate(&mut s.sheets[0], &sel, true);
        normalize(&mut s.sheets[0]);
        assert_eq!(s.sheets[0].parts[1].rotation, Rotation::R90);
        assert_eq!(nets_of(&s), before);
        for w in &s.sheets[0].wires {
            for seg in w.points.windows(2) {
                assert!(seg[0].x == seg[1].x || seg[0].y == seg[1].y);
            }
        }
        // Four quarter turns come back to where it started.
        for _ in 0..3 {
            rotate(&mut s.sheets[0], &sel, true);
        }
        assert_eq!(s.sheets[0].parts[1].rotation, Rotation::R0);
        assert_eq!(s.sheets[0].parts[1].position, Point::new(0, 12));
        assert_eq!(nets_of(&s), before);
    }

    #[test]
    fn turning_the_other_way_undoes_it() {
        let mut s = divider();
        let mut sel = Items::new();
        sel.insert(Item::Part(0));
        sel.insert(Item::Part(1));
        let start = s.clone();
        rotate(&mut s.sheets[0], &sel, true);
        rotate(&mut s.sheets[0], &sel, false);
        assert_eq!(s.sheets[0].parts, start.sheets[0].parts);
    }

    #[test]
    fn a_flip_moves_pins_like_a_mirror_on_screen() {
        for rotation in [Rotation::R0, Rotation::R90, Rotation::R180, Rotation::R270] {
            for horizontal in [true, false] {
                let mut b = Builder::new("t");
                let q = b.place("NPN", "Q1", "", 10, 10);
                let mut s = b.build();
                s.sheets[0].parts[q.0].rotation = rotation;
                let before = pin_points(&s.sheets[0].parts[0]);
                let mut sel = Items::new();
                sel.insert(Item::Part(0));
                flip(&mut s.sheets[0], &sel, horizontal);
                let after = pin_points(&s.sheets[0].parts[0]);
                for (a, b) in before.iter().zip(&after) {
                    let expect = if horizontal {
                        Point::new(20 - a.x, a.y)
                    } else {
                        Point::new(a.x, 20 - a.y)
                    };
                    assert_eq!(*b, expect, "{rotation:?} horizontal={horizontal}");
                }
            }
        }
    }

    #[test]
    fn a_wire_ending_on_another_wires_middle_gets_a_junction() {
        let mut s = divider();
        let sheet = &mut s.sheets[0];
        let n = sheet.wires.len();
        // The divider's wire runs (0,4) to (0,8). End a new one at (0,6).
        let (idx, _) = add_wire(sheet, &[Point::new(6, 6), Point::new(0, 6)]);
        assert!(idx.is_some());
        assert_eq!(sheet.wires.len(), n + 1);
        assert_eq!(sheet.junctions.len(), 1);
        assert_eq!(sheet.junctions[0].position, Point::new(0, 6));
    }

    #[test]
    fn junctions_that_join_nothing_are_removed() {
        let mut s = divider();
        s.sheets[0].junctions.push(Junction {
            position: Point::new(40, 40),
        });
        let remap = normalize(&mut s.sheets[0]);
        assert!(s.sheets[0].junctions.is_empty());
        assert_eq!(remap.junctions, vec![None]);
    }

    #[test]
    fn square_routes_and_redundant_corners() {
        let (a, b) = (Point::new(0, 0), Point::new(4, 6));
        assert_eq!(route(a, b, true), vec![a, Point::new(4, 0), b]);
        assert_eq!(route(a, b, false), vec![a, Point::new(0, 6), b]);
        assert_eq!(route(a, Point::new(0, 6), true), vec![a, Point::new(0, 6)]);
        assert_eq!(route(a, a, true), vec![a]);
        let mut pts = vec![
            Point::new(0, 0),
            Point::new(0, 3),
            Point::new(0, 6),
            Point::new(4, 6),
        ];
        simplify(&mut pts);
        assert_eq!(
            pts,
            vec![Point::new(0, 0), Point::new(0, 6), Point::new(4, 6)]
        );
        // A wire that doubles back keeps its turn.
        let mut back = vec![Point::new(0, 0), Point::new(0, 6), Point::new(0, 2)];
        simplify(&mut back);
        assert_eq!(back.len(), 3);
    }

    #[test]
    fn duplicate_and_zero_length_wires_are_dropped_and_indexes_follow() {
        let mut sheet = Sheet::default();
        let w = |a: (i32, i32), b: (i32, i32)| Wire {
            points: vec![Point::new(a.0, a.1), Point::new(b.0, b.1)],
        };
        sheet.wires = vec![
            w((0, 0), (5, 0)),
            w((3, 3), (3, 3)),
            w((5, 0), (0, 0)),
            w((0, 1), (5, 1)),
        ];
        let remap = normalize(&mut sheet);
        assert_eq!(sheet.wires.len(), 2);
        assert_eq!(remap.wires, vec![Some(0), None, None, Some(1)]);
        let mut sel = Items::new();
        sel.insert(Item::Wire(3));
        sel.insert(Item::Wire(1));
        let moved = remap.apply(&sel);
        assert_eq!(moved.into_iter().collect::<Vec<_>>(), vec![Item::Wire(1)]);
    }

    #[test]
    fn deleting_removes_the_selected_items_only() {
        let mut s = divider();
        let mut sel = Items::new();
        sel.insert(Item::Part(0));
        delete(&mut s.sheets[0], &sel);
        assert_eq!(s.sheets[0].parts.len(), 1);
        assert_eq!(s.sheets[0].parts[0].refdes, "R2");
        // An index that does not exist is ignored.
        let mut bad = Items::new();
        bad.insert(Item::Part(40));
        bad.insert(Item::Wire(40));
        delete(&mut s.sheets[0], &bad);
        assert_eq!(s.sheets[0].parts.len(), 1);
    }

    #[test]
    fn copy_and_paste_give_new_names_and_keep_the_connections_between_the_copies() {
        let mut s = divider();
        let sel = all(&s.sheets[0]);
        let clip = copy(&s.sheets[0], &sel);
        assert!(!clip.is_empty());
        let pasted = paste(&mut s, 0, &clip, 20, 0);
        assert_eq!(s.sheets[0].parts.len(), 4);
        let names: Vec<&str> = s.sheets[0]
            .parts
            .iter()
            .map(|p| p.refdes.as_str())
            .collect();
        assert_eq!(names, ["R1", "R2", "R3", "R4"]);
        assert_eq!(pasted.len(), 3);
        // The copies are connected to each other, and not to the originals.
        let nets = nets_of(&s);
        assert!(
            nets.contains(&vec![(2, 1), (3, 0)])
                || nets
                    .iter()
                    .any(|n| n.len() == 2 && n.iter().all(|p| p.0 >= 2))
        );
        assert!(Clip::default().is_empty());
        assert!(paste(&mut s, 9, &clip, 0, 0).is_empty());
    }

    #[test]
    fn hostile_coordinates_do_not_overflow() {
        let mut s = divider();
        s.sheets[0].parts[0].position = Point::new(i32::MAX, i32::MIN);
        let sel = all(&s.sheets[0]);
        translate(&mut s.sheets[0], &sel, i32::MAX, i32::MIN);
        rotate(&mut s.sheets[0], &sel, true);
        flip(&mut s.sheets[0], &sel, true);
        flip(&mut s.sheets[0], &sel, false);
        normalize(&mut s.sheets[0]);
        // And an empty sheet and an empty selection are fine.
        let mut empty = Sheet::default();
        translate(&mut empty, &Items::new(), 1, 1);
        rotate(&mut empty, &Items::new(), true);
        assert_eq!(pivot(&empty, &Items::new()), Point::new(0, 0));
    }

    #[test]
    fn unknown_symbols_have_no_pins_and_do_not_break_edits() {
        let mut s = divider();
        s.sheets[0].parts[0].lib_id = "NOPE".into();
        assert!(pin_points(&s.sheets[0].parts[0]).is_empty());
        let sel = all(&s.sheets[0]);
        translate(&mut s.sheets[0], &sel, 1, 1);
        normalize(&mut s.sheets[0]);
    }

    #[test]
    fn history_steps_back_and_forward_and_merges_runs() {
        let mut h: History<i32> = History::default();
        assert!(!h.can_undo() && !h.can_redo());
        h.record(1, "one");
        h.record(2, "two");
        assert_eq!(h.undo_label(), Some("two"));
        assert_eq!(h.undo(3), Some((2, "two".to_string())));
        assert_eq!(h.redo_label(), Some("two"));
        assert_eq!(h.redo(2), Some((3, "two".to_string())));
        assert_eq!(h.undo(3).map(|x| x.0), Some(2));
        // A new edit forgets the redo stack.
        h.record(2, "three");
        assert!(!h.can_redo());
        // A run of nudges is one step.
        let mut h: History<i32> = History::default();
        h.record_merging("nudge", 10, "Move");
        h.record_merging("nudge", 11, "Move");
        h.record_merging("nudge", 12, "Move");
        assert_eq!(h.undo(13).map(|x| x.0), Some(10));
        assert!(!h.can_undo());
        // Closing the run starts a new one.
        let mut h: History<i32> = History::default();
        h.record_merging("k", 1, "a");
        h.close_run();
        h.record_merging("k", 2, "a");
        assert_eq!(h.undo(3).map(|x| x.0), Some(2));
        assert_eq!(h.undo(2).map(|x| x.0), Some(1));
    }

    #[test]
    fn history_is_bounded() {
        let mut h: History<usize> = History::default();
        for i in 0..HISTORY_LIMIT + 50 {
            h.record(i, "x");
        }
        let mut n = 0;
        let mut cur = 0;
        while let Some((s, _)) = h.undo(cur) {
            cur = s;
            n += 1;
        }
        assert_eq!(n, HISTORY_LIMIT);
    }

    #[test]
    fn power_ports_take_their_style_from_the_name() {
        use crate::schematic::PowerStyle;
        assert_eq!(
            new_power_port("GND", Point::new(0, 0)).style,
            PowerStyle::Ground
        );
        assert_eq!(
            new_power_port("0", Point::new(0, 0)).style,
            PowerStyle::Ground
        );
        assert_eq!(
            new_power_port("VCC", Point::new(0, 0)).style,
            PowerStyle::Supply
        );
    }

    #[test]
    fn a_new_part_has_the_defaults_of_its_symbol() {
        let def = Library::builtin().get("R").unwrap();
        let p = new_part(def, "R9", Point::new(3, 4));
        assert_eq!((p.value.as_str(), p.refdes.as_str()), ("1k", "R9"));
        assert_eq!(p.position, Point::new(3, 4));
    }
}

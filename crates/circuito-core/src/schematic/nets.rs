// SPDX-License-Identifier: MIT OR Apache-2.0
//! Net resolution: which pins, wires and names are the same electrical node.
//!
//! # Rules (each is covered by a test)
//! 1. **One wire is one conductor.** Every vertex of a polyline is connected to every other.
//! 2. **Endpoints connect.** Wire endpoints at the same grid point are connected; a pin at a wire's
//!    endpoint is connected to that wire; two pins at the same point are connected.
//! 3. **Interior contact needs a junction.** An endpoint or pin that touches the *interior* of
//!    another wire (a segment or a corner vertex that is not that wire's first or last point) is
//!    connected only if a junction sits on that point. Without one it is reported as
//!    [`NetWarning::TouchWithoutJunction`] and not connected.
//! 4. **Crossing is not connecting.** Two wires that cross at a point that is an endpoint of
//!    neither never connect, with or without a warning; a junction on the crossing connects them.
//! 5. **Labels name a net.** A label attaches to the wire or pin at its point (endpoint, junction)
//!    or, failing that, to the single wire whose interior it sits on. Two or more interior wires
//!    without a junction is ambiguous ([`NetWarning::AmbiguousLabel`]) and attaches nothing.
//!    Labels of the same name connect within a sheet (local scope) or across all sheets (global).
//! 6. **Power ports are global labels.** Ports of the same name connect on every sheet.
//! 7. **Ground.** The names `0` and `GND` (any case) are the same net everywhere.
//! 8. **Names ignore case**, as in SPICE.
//! 9. **No-connect flags** mark the pin at their point; they never connect anything.

use super::{Point, Schematic, SchematicError};
use crate::library::{transform, Library, PartDef};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A pin of a part: sheet, part and pin index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PinRef {
    /// Sheet index.
    pub sheet: usize,
    /// Part index on the sheet.
    pub part: usize,
    /// Pin index in the symbol.
    pub pin: usize,
}

/// A label, by sheet and index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LabelRef {
    /// Sheet index.
    pub sheet: usize,
    /// Label index on the sheet.
    pub index: usize,
}

/// A power port, by sheet and index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortRef {
    /// Sheet index.
    pub sheet: usize,
    /// Port index on the sheet.
    pub index: usize,
}

/// A resolved net.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Net {
    /// Pins on the net, in sheet/part/pin order.
    pub pins: Vec<PinRef>,
    /// Labels on the net.
    pub labels: Vec<LabelRef>,
    /// Power ports on the net.
    pub power_ports: Vec<PortRef>,
    /// Wires on the net as (sheet, wire index).
    pub wires: Vec<(usize, usize)>,
}

/// Something suspicious about connectivity that is not an error by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetWarning {
    /// A wire end or pin touches another wire's interior with no junction; not connected.
    TouchWithoutJunction {
        /// Sheet index.
        sheet: usize,
        /// Where.
        point: Point,
    },
    /// A label touches nothing.
    FloatingLabel {
        /// Sheet index.
        sheet: usize,
        /// Label name.
        name: String,
        /// Where.
        point: Point,
    },
    /// A label sits on the interior of two or more wires with no junction.
    AmbiguousLabel {
        /// Sheet index.
        sheet: usize,
        /// Label name.
        name: String,
        /// Where.
        point: Point,
    },
    /// A power port touches nothing.
    FloatingPowerPort {
        /// Sheet index.
        sheet: usize,
        /// Port name.
        name: String,
        /// Where.
        point: Point,
    },
    /// A junction with nothing to connect.
    DanglingJunction {
        /// Sheet index.
        sheet: usize,
        /// Where.
        point: Point,
    },
    /// A no-connect flag that is not on a pin.
    DanglingNoConnect {
        /// Sheet index.
        sheet: usize,
        /// Where.
        point: Point,
    },
}

/// The result of resolving a schematic's connectivity.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetResolution {
    /// The nets, in a deterministic order.
    pub nets: Vec<Net>,
    /// Connectivity warnings, in sheet and position order.
    pub warnings: Vec<NetWarning>,
    /// Pins carrying a no-connect flag.
    pub no_connect_pins: BTreeSet<PinRef>,
    /// Net index of every pin.
    pub pin_net: BTreeMap<PinRef, usize>,
    /// Absolute position of every pin.
    pub pin_points: BTreeMap<PinRef, Point>,
}

#[derive(Clone, Copy)]
enum Item {
    Wire(usize, usize),
    Pin(PinRef),
    Label(LabelRef),
    Port(PortRef),
}

struct Dsu {
    parent: Vec<usize>,
}

impl Dsu {
    fn new(n: usize) -> Self {
        Dsu {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // Keep the smaller index as the root so ordering stays deterministic.
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.parent[hi] = lo;
        }
    }
}

fn on_segment(p: Point, a: Point, b: Point) -> bool {
    let (px, py, ax, ay, bx, by) = (
        i64::from(p.x),
        i64::from(p.y),
        i64::from(a.x),
        i64::from(a.y),
        i64::from(b.x),
        i64::from(b.y),
    );
    let cross = (bx - ax) * (py - ay) - (by - ay) * (px - ax);
    cross == 0 && px >= ax.min(bx) && px <= ax.max(bx) && py >= ay.min(by) && py <= ay.max(by)
}

fn canonical(name: &str) -> String {
    let l = name.trim().to_lowercase();
    if l == "0" || l == "gnd" {
        "0".to_string()
    } else {
        l
    }
}

/// Resolve the connectivity of `s` using the symbols in `lib`.
///
/// # Errors
/// [`SchematicError::UnknownPart`] when a part names a symbol that is not in the library, or a
/// limit error from [`Schematic::check_limits`].
pub fn resolve_nets(s: &Schematic, lib: &Library) -> Result<NetResolution, SchematicError> {
    s.check_limits()?;
    let mut items: Vec<Item> = Vec::new();
    let mut res = NetResolution::default();
    // Item ids: wires first per sheet, then pins, labels, ports, in sheet order.
    let mut sheet_items: Vec<SheetItems> = Vec::new();
    for (si, sh) in s.sheets.iter().enumerate() {
        let mut si_items = SheetItems::default();
        for wi in 0..sh.wires.len() {
            si_items.wires.push(items.len());
            items.push(Item::Wire(si, wi));
        }
        for (pi, part) in sh.parts.iter().enumerate() {
            let def: &PartDef =
                lib.get(&part.lib_id)
                    .ok_or_else(|| SchematicError::UnknownPart {
                        refdes: part.refdes.clone(),
                        lib_id: part.lib_id.clone(),
                    })?;
            for (k, pd) in def.pins.iter().enumerate() {
                let off = transform(pd.position, part.rotation, part.mirror);
                let at = Point::new(
                    part.position.x.saturating_add(off.x),
                    part.position.y.saturating_add(off.y),
                );
                let r = PinRef {
                    sheet: si,
                    part: pi,
                    pin: k,
                };
                res.pin_points.insert(r, at);
                si_items.pins.push((items.len(), at));
                items.push(Item::Pin(r));
            }
        }
        for (li, l) in sh.labels.iter().enumerate() {
            si_items.labels.push((items.len(), l.position));
            items.push(Item::Label(LabelRef {
                sheet: si,
                index: li,
            }));
        }
        for (pi, p) in sh.power_ports.iter().enumerate() {
            si_items.ports.push((items.len(), p.position));
            items.push(Item::Port(PortRef {
                sheet: si,
                index: pi,
            }));
        }
        sheet_items.push(si_items);
    }
    let mut dsu = Dsu::new(items.len());

    // Name groups: key -> first item.
    let mut local_names: HashMap<(usize, String), usize> = HashMap::new();
    let mut global_names: HashMap<String, usize> = HashMap::new();

    for (si, sh) in s.sheets.iter().enumerate() {
        let it = &sheet_items[si];
        // Points of interest: pins, junctions, labels, ports, wire endpoints.
        let mut ends: HashMap<Point, Vec<usize>> = HashMap::new(); // endpoint wires
        let mut pins_at: HashMap<Point, Vec<usize>> = HashMap::new();
        let mut interior: HashMap<Point, Vec<usize>> = HashMap::new(); // wire item ids
        let junctions: BTreeSet<Point> = sh.junctions.iter().map(|j| j.position).collect();
        for (wi, w) in sh.wires.iter().enumerate() {
            let id = it.wires[wi];
            if let (Some(f), Some(l)) = (w.points.first(), w.points.last()) {
                ends.entry(*f).or_default().push(id);
                if l != f {
                    ends.entry(*l).or_default().push(id);
                }
            }
            // Rule 1: all vertices of one polyline are one conductor, which is one item.
        }
        for (id, at) in &it.pins {
            pins_at.entry(*at).or_default().push(*id);
        }
        let mut poi_xy: BTreeSet<(i32, i32)> = BTreeSet::new();
        let mut poi_yx: BTreeSet<(i32, i32)> = BTreeSet::new();
        let mut add_poi = |p: Point| {
            poi_xy.insert((p.x, p.y));
            poi_yx.insert((p.y, p.x));
        };
        for p in ends.keys().chain(pins_at.keys()).chain(junctions.iter()) {
            add_poi(*p);
        }
        for (_, p) in it.labels.iter().chain(it.ports.iter()) {
            add_poi(*p);
        }
        // Interior touches.
        for (wi, w) in sh.wires.iter().enumerate() {
            let id = it.wires[wi];
            let (first, last) = match (w.points.first(), w.points.last()) {
                (Some(f), Some(l)) => (*f, *l),
                _ => continue,
            };
            let mut seen: BTreeSet<Point> = BTreeSet::new();
            for seg in w.points.windows(2) {
                let (a, b) = (seg[0], seg[1]);
                let candidates: Vec<Point> = if a.x == b.x {
                    poi_xy
                        .range((a.x, a.y.min(b.y))..=(a.x, a.y.max(b.y)))
                        .map(|&(x, y)| Point::new(x, y))
                        .collect()
                } else if a.y == b.y {
                    poi_yx
                        .range((a.y, a.x.min(b.x))..=(a.y, a.x.max(b.x)))
                        .map(|&(y, x)| Point::new(x, y))
                        .collect()
                } else {
                    poi_xy
                        .range((a.x.min(b.x), i32::MIN)..=(a.x.max(b.x), i32::MAX))
                        .map(|&(x, y)| Point::new(x, y))
                        .filter(|q| on_segment(*q, a, b))
                        .collect()
                };
                for q in candidates {
                    if q == first || q == last {
                        continue;
                    }
                    if seen.insert(q) {
                        interior.entry(q).or_default().push(id);
                    }
                }
            }
        }
        // Union per point.
        let mut points: BTreeSet<Point> = BTreeSet::new();
        points.extend(ends.keys().copied());
        points.extend(pins_at.keys().copied());
        points.extend(interior.keys().copied());
        points.extend(junctions.iter().copied());
        let empty: Vec<usize> = Vec::new();
        for q in &points {
            let e = ends.get(q).unwrap_or(&empty);
            let k = pins_at.get(q).unwrap_or(&empty);
            let i = interior.get(q).unwrap_or(&empty);
            let has_j = junctions.contains(q);
            let mut group: Vec<usize> = e.iter().chain(k.iter()).copied().collect();
            if has_j {
                group.extend(i.iter().copied());
            }
            for pair in group.windows(2) {
                dsu.union(pair[0], pair[1]);
            }
            if !has_j && !i.is_empty() && !(e.is_empty() && k.is_empty()) {
                res.warnings.push(NetWarning::TouchWithoutJunction {
                    sheet: si,
                    point: *q,
                });
            }
            if has_j && group.is_empty() {
                res.warnings.push(NetWarning::DanglingJunction {
                    sheet: si,
                    point: *q,
                });
            }
        }
        // Labels and power ports.
        let attach = |q: &Point, dsu: &mut Dsu, item: usize| -> Attach {
            let e = ends.get(q).unwrap_or(&empty);
            let k = pins_at.get(q).unwrap_or(&empty);
            let i = interior.get(q).unwrap_or(&empty);
            let target = e.first().or_else(|| k.first()).copied();
            if let Some(t) = target {
                dsu.union(item, t);
                return Attach::Done;
            }
            if junctions.contains(q) {
                if let Some(t) = i.first() {
                    dsu.union(item, *t);
                    return Attach::Done;
                }
            }
            match i.len() {
                0 => Attach::Floating,
                1 => {
                    dsu.union(item, i[0]);
                    Attach::Done
                }
                _ => Attach::Ambiguous,
            }
        };
        for (li, (item, q)) in it.labels.iter().enumerate() {
            let l = &sh.labels[li];
            match attach(q, &mut dsu, *item) {
                Attach::Done => {}
                Attach::Floating => res.warnings.push(NetWarning::FloatingLabel {
                    sheet: si,
                    name: l.name.clone(),
                    point: *q,
                }),
                Attach::Ambiguous => res.warnings.push(NetWarning::AmbiguousLabel {
                    sheet: si,
                    name: l.name.clone(),
                    point: *q,
                }),
            }
            let key = canonical(&l.name);
            let global = key == "0" || l.scope == super::LabelScope::Global;
            let first = if global {
                *global_names.entry(key).or_insert(*item)
            } else {
                *local_names.entry((si, key)).or_insert(*item)
            };
            dsu.union(*item, first);
        }
        for (pi, (item, q)) in it.ports.iter().enumerate() {
            let pp = &sh.power_ports[pi];
            match attach(q, &mut dsu, *item) {
                Attach::Done => {}
                Attach::Floating => res.warnings.push(NetWarning::FloatingPowerPort {
                    sheet: si,
                    name: pp.name.clone(),
                    point: *q,
                }),
                Attach::Ambiguous => res.warnings.push(NetWarning::AmbiguousLabel {
                    sheet: si,
                    name: pp.name.clone(),
                    point: *q,
                }),
            }
            let first = *global_names.entry(canonical(&pp.name)).or_insert(*item);
            dsu.union(*item, first);
        }
        // No-connect flags.
        let pin_lookup: HashMap<Point, Vec<PinRef>> = {
            let mut m: HashMap<Point, Vec<PinRef>> = HashMap::new();
            for (id, at) in &it.pins {
                if let Some(Item::Pin(r)) = items.get(*id) {
                    m.entry(*at).or_default().push(*r);
                }
            }
            m
        };
        for nc in &sh.no_connects {
            match pin_lookup.get(&nc.position) {
                Some(rs) => res.no_connect_pins.extend(rs.iter().copied()),
                None => res.warnings.push(NetWarning::DanglingNoConnect {
                    sheet: si,
                    point: nc.position,
                }),
            }
        }
    }

    // Collect nets in order of first item.
    let mut net_of_root: HashMap<usize, usize> = HashMap::new();
    for (id, item) in items.iter().enumerate() {
        let root = dsu.find(id);
        let n = *net_of_root.entry(root).or_insert_with(|| {
            res.nets.push(Net::default());
            res.nets.len() - 1
        });
        let net = &mut res.nets[n];
        match item {
            Item::Wire(s, w) => net.wires.push((*s, *w)),
            Item::Pin(r) => {
                net.pins.push(*r);
                res.pin_net.insert(*r, n);
            }
            Item::Label(l) => net.labels.push(*l),
            Item::Port(p) => net.power_ports.push(*p),
        }
    }
    Ok(res)
}

enum Attach {
    Done,
    Floating,
    Ambiguous,
}

#[derive(Default)]
struct SheetItems {
    wires: Vec<usize>,
    pins: Vec<(usize, Point)>,
    labels: Vec<(usize, Point)>,
    ports: Vec<(usize, Point)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::{
        Junction, Label, LabelScope, NoConnect, Part, PowerPort, PowerStyle, Rotation, Sheet, Wire,
    };

    fn pt(x: i32, y: i32) -> Point {
        Point::new(x, y)
    }
    fn wire(pts: &[(i32, i32)]) -> Wire {
        Wire {
            points: pts.iter().map(|&(x, y)| pt(x, y)).collect(),
        }
    }
    fn res_part(refdes: &str, x: i32, y: i32) -> Part {
        Part {
            refdes: refdes.into(),
            lib_id: "R".into(),
            value: "1k".into(),
            position: pt(x, y),
            rotation: Rotation::R0,
            mirror: false,
            model: None,
            fields: Default::default(),
        }
    }
    fn schem(sheet: Sheet) -> Schematic {
        let mut s = Schematic::new("t");
        s.sheets = vec![sheet];
        s
    }
    fn resolve(s: &Schematic) -> NetResolution {
        resolve_nets(s, Library::builtin()).unwrap()
    }
    /// Net index of pin `pin` of part `part` on sheet 0.
    fn net(r: &NetResolution, part: usize, pin: usize) -> usize {
        r.pin_net[&PinRef {
            sheet: 0,
            part,
            pin,
        }]
    }

    #[test]
    fn wire_endpoints_connect_pins() {
        // R1 pin2 at (0,4); R2 pin1 at (10,-4). A wire joins them.
        let sh = Sheet {
            parts: vec![res_part("R1", 0, 0), res_part("R2", 10, 8)],
            wires: vec![wire(&[(0, 4), (0, 6), (10, 6), (10, 4)])],
            ..Sheet::default()
        };
        let r = resolve(&schem(sh));
        assert_eq!(net(&r, 0, 1), net(&r, 1, 0));
        assert_ne!(net(&r, 0, 0), net(&r, 0, 1));
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }

    #[test]
    fn pins_that_touch_connect_without_wire() {
        let sh = Sheet {
            parts: vec![res_part("R1", 0, 0), res_part("R2", 0, 8)],
            ..Sheet::default()
        };
        let r = resolve(&schem(sh));
        assert_eq!(net(&r, 0, 1), net(&r, 1, 0));
    }

    #[test]
    fn crossing_wires_do_not_connect_but_a_junction_does() {
        let mk = |junction: bool| {
            let mut sh = Sheet {
                parts: vec![res_part("R1", -10, -10), res_part("R2", 10, -10)],
                wires: vec![
                    wire(&[(-10, -6), (-10, 0), (10, 0)]),
                    wire(&[(0, -10), (0, 10)]),
                    wire(&[(10, -6), (10, 0)]),
                ],
                ..Sheet::default()
            };
            // Make a clean crossing: horizontal wire passes (0,0), vertical wire passes (0,0).
            sh.wires[0] = wire(&[(-10, -6), (-10, 0), (10, 0)]);
            sh.wires[2] = wire(&[(10, -6), (10, 0)]);
            if junction {
                sh.junctions.push(Junction { position: pt(0, 0) });
            }
            sh.labels.push(Label {
                name: "V".into(),
                position: pt(0, 10),
                scope: LabelScope::Local,
            });
            sh
        };
        // R1.2 is at (-10,-6)... pins: R at (-10,-10): pin1 (-10,-14), pin2 (-10,-6).
        let r = resolve(&schem(mk(false)));
        // The horizontal run (R1.2 .. R2.2) is one net; the vertical wire (with label V) is another.
        assert_eq!(net(&r, 0, 1), net(&r, 1, 1));
        let vert_net = r
            .pin_net
            .get(&PinRef {
                sheet: 0,
                part: 0,
                pin: 0,
            })
            .copied();
        assert_ne!(vert_net, Some(net(&r, 0, 1)));
        let label_net = r.nets.iter().position(|n| !n.labels.is_empty()).unwrap();
        assert_ne!(label_net, net(&r, 0, 1), "crossing wires must not connect");
        assert!(
            r.warnings.is_empty(),
            "a plain crossing is not even a warning: {:?}",
            r.warnings
        );
        let r = resolve(&schem(mk(true)));
        let label_net = r.nets.iter().position(|n| !n.labels.is_empty()).unwrap();
        assert_eq!(label_net, net(&r, 0, 1), "a junction connects the crossing");
    }

    #[test]
    fn wire_end_on_interior_needs_junction() {
        // Vertical wire through (0,0); a horizontal wire ends at (0,0).
        let base = |junction: bool| {
            let mut sh = Sheet {
                parts: vec![res_part("R1", 8, 0)],
                wires: vec![wire(&[(0, -10), (0, 10)]), wire(&[(0, 0), (8, 0), (8, 4)])],
                ..Sheet::default()
            };
            // R1 pin1 at (8,-4), pin2 at (8,4): the second wire's end meets pin2.
            if junction {
                sh.junctions.push(Junction { position: pt(0, 0) });
            }
            sh.labels.push(Label {
                name: "TOP".into(),
                position: pt(0, -10),
                scope: LabelScope::Local,
            });
            sh
        };
        let r = resolve(&schem(base(false)));
        assert!(r.warnings.contains(&NetWarning::TouchWithoutJunction {
            sheet: 0,
            point: pt(0, 0)
        }));
        let label_net = r.nets.iter().position(|n| !n.labels.is_empty()).unwrap();
        assert_ne!(label_net, net(&r, 0, 1));
        let r = resolve(&schem(base(true)));
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let label_net = r.nets.iter().position(|n| !n.labels.is_empty()).unwrap();
        assert_eq!(label_net, net(&r, 0, 1));
    }

    #[test]
    fn pin_on_wire_interior_needs_junction() {
        // R at (0,0): pin1 (0,-4), pin2 (0,4). A horizontal wire runs through (0,4).
        let mk = |junction: bool| {
            let mut sh = Sheet {
                parts: vec![res_part("R1", 0, 0)],
                wires: vec![wire(&[(-5, 4), (5, 4)])],
                ..Sheet::default()
            };
            if junction {
                sh.junctions.push(Junction { position: pt(0, 4) });
            }
            sh
        };
        let r = resolve(&schem(mk(false)));
        assert!(r.warnings.contains(&NetWarning::TouchWithoutJunction {
            sheet: 0,
            point: pt(0, 4)
        }));
        assert!(r
            .nets
            .iter()
            .all(|n| !(n.pins.len() == 1 && !n.wires.is_empty())));
        let r = resolve(&schem(mk(true)));
        let n = &r.nets[net(&r, 0, 1)];
        assert_eq!(n.wires.len(), 1);
    }

    #[test]
    fn corner_vertex_counts_as_interior() {
        // A wire with a corner at (0,0); another wire ends exactly on that corner: not connected
        // without a junction.
        let sh = Sheet {
            wires: vec![wire(&[(-5, 0), (0, 0), (0, 5)]), wire(&[(0, 0), (5, 0)])],
            ..Sheet::default()
        };
        let r = resolve(&schem(sh));
        assert_eq!(r.nets.len(), 2);
        assert_eq!(
            r.warnings,
            vec![NetWarning::TouchWithoutJunction {
                sheet: 0,
                point: pt(0, 0)
            }]
        );
    }

    #[test]
    fn labels_connect_by_name_case_insensitively_and_per_sheet() {
        let mut a = Sheet::default();
        a.wires.push(wire(&[(0, 0), (5, 0)]));
        a.wires.push(wire(&[(0, 10), (5, 10)]));
        a.labels.push(Label {
            name: "Sig".into(),
            position: pt(0, 0),
            scope: LabelScope::Local,
        });
        a.labels.push(Label {
            name: "SIG".into(),
            position: pt(0, 10),
            scope: LabelScope::Local,
        });
        let mut b = Sheet {
            name: "B".into(),
            ..Sheet::default()
        };
        b.wires.push(wire(&[(0, 0), (5, 0)]));
        b.labels.push(Label {
            name: "Sig".into(),
            position: pt(0, 0),
            scope: LabelScope::Local,
        });
        let mut s = Schematic::new("t");
        s.sheets = vec![a, b];
        let r = resolve_nets(&s, Library::builtin()).unwrap();
        assert_eq!(
            r.nets.len(),
            2,
            "sheet A's two labels join; sheet B's local label stays apart"
        );
        // Global scope joins sheets.
        s.sheets[0].labels[0].scope = LabelScope::Global;
        s.sheets[1].labels[0].scope = LabelScope::Global;
        let r = resolve_nets(&s, Library::builtin()).unwrap();
        assert_eq!(
            r.nets.len(),
            2,
            "global labels join A's first wire to B's wire"
        );
        assert!(r
            .nets
            .iter()
            .any(|n| n.wires.iter().any(|w| w.0 == 0) && n.wires.iter().any(|w| w.0 == 1)));
    }

    #[test]
    fn power_ports_are_global_and_ground_aliases_merge() {
        let mut a = Sheet::default();
        a.wires.push(wire(&[(0, 0), (5, 0)]));
        a.power_ports.push(PowerPort {
            name: "VCC".into(),
            position: pt(0, 0),
            style: PowerStyle::Supply,
        });
        a.wires.push(wire(&[(0, 10), (5, 10)]));
        a.power_ports.push(PowerPort {
            name: "gnd".into(),
            position: pt(0, 10),
            style: PowerStyle::Ground,
        });
        let mut b = Sheet {
            name: "B".into(),
            ..Sheet::default()
        };
        b.wires.push(wire(&[(0, 0), (5, 0)]));
        b.power_ports.push(PowerPort {
            name: "vcc".into(),
            position: pt(5, 0),
            style: PowerStyle::Supply,
        });
        b.wires.push(wire(&[(0, 10), (5, 10)]));
        b.labels.push(Label {
            name: "0".into(),
            position: pt(0, 10),
            scope: LabelScope::Local,
        });
        let mut s = Schematic::new("t");
        s.sheets = vec![a, b];
        let r = resolve_nets(&s, Library::builtin()).unwrap();
        assert_eq!(
            r.nets.len(),
            2,
            "VCC merged across sheets, GND/0 merged across sheets"
        );
    }

    #[test]
    fn floating_things_are_warned_about() {
        let mut sh = Sheet::default();
        sh.labels.push(Label {
            name: "X".into(),
            position: pt(50, 50),
            scope: LabelScope::Local,
        });
        sh.power_ports.push(PowerPort {
            name: "VCC".into(),
            position: pt(60, 60),
            style: PowerStyle::Supply,
        });
        sh.junctions.push(Junction {
            position: pt(70, 70),
        });
        sh.no_connects.push(NoConnect {
            position: pt(80, 80),
        });
        let r = resolve(&schem(sh));
        assert_eq!(r.warnings.len(), 4, "{:?}", r.warnings);
    }

    #[test]
    fn ambiguous_label_on_crossing() {
        let mut sh = Sheet::default();
        sh.wires.push(wire(&[(-5, 0), (5, 0)]));
        sh.wires.push(wire(&[(0, -5), (0, 5)]));
        sh.labels.push(Label {
            name: "X".into(),
            position: pt(0, 0),
            scope: LabelScope::Local,
        });
        let r = resolve(&schem(sh));
        assert!(matches!(
            r.warnings.as_slice(),
            [NetWarning::AmbiguousLabel { .. }]
        ));
    }

    #[test]
    fn no_connect_marks_pin() {
        let mut sh = Sheet {
            parts: vec![res_part("R1", 0, 0)],
            ..Sheet::default()
        };
        sh.no_connects.push(NoConnect {
            position: pt(0, -4),
        });
        let r = resolve(&schem(sh));
        assert!(r.no_connect_pins.contains(&PinRef {
            sheet: 0,
            part: 0,
            pin: 0
        }));
        assert!(!r.no_connect_pins.contains(&PinRef {
            sheet: 0,
            part: 0,
            pin: 1
        }));
    }

    #[test]
    fn rotation_and_mirror_move_pins() {
        let mut p = res_part("R1", 0, 0);
        p.rotation = Rotation::R90;
        let r = resolve(&schem(Sheet {
            parts: vec![p],
            ..Sheet::default()
        }));
        let pts: Vec<Point> = r.pin_points.values().copied().collect();
        assert!(
            pts.contains(&pt(-4, 0)) && pts.contains(&pt(4, 0)),
            "{pts:?}"
        );
    }

    #[test]
    fn unknown_symbol_is_an_error() {
        let mut p = res_part("X1", 0, 0);
        p.lib_id = "NOPE".into();
        let e = resolve_nets(
            &schem(Sheet {
                parts: vec![p],
                ..Sheet::default()
            }),
            Library::builtin(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("NOPE"));
    }

    #[test]
    fn diagonal_wires_and_hostile_coordinates_are_safe() {
        let sh = Sheet {
            wires: vec![
                wire(&[(-10_000_000, -10_000_000), (10_000_000, 10_000_000)]),
                wire(&[(0, 0), (1, 1)]),
            ],
            junctions: vec![Junction { position: pt(3, 3) }],
            ..Sheet::default()
        };
        let r = resolve(&schem(sh));
        assert_eq!(r.nets.len(), 2);
    }
}

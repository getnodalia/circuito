// SPDX-License-Identifier: MIT OR Apache-2.0
//! Drawing a design: placing the parts and routing the nets.
//!
//! Parts are arranged in columns by how far they are from a source along the nets, so a signal
//! reads from left to right. Parts that connect to ground hang below a common rail, series
//! parts lie on it. A net becomes wires when a clean route exists and a short stub with a net
//! label when it does not; ground and supply nets always end in a power port or label. Every
//! drawing is checked: the connectivity read back from the drawing must be the design's.

use crate::design::{is_anonymous, Design, NetInfo, NetKind, PinNet};
use circuito_core::edit;
use circuito_core::library::{transform, Device, Library, PartDef, Shape};
use circuito_core::schematic::{
    resolve_nets, Label, LabelScope, NoConnect, Point, PowerPort, PowerStyle, Rotation, Schematic,
    Wire,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;

/// Most parts a design may have to be drawn.
pub const MAX_PARTS: usize = 2000;

/// Why a design could not be drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// More parts than [`MAX_PARTS`].
    TooBig,
    /// A part uses a library symbol that does not exist.
    UnknownPart(String),
    /// A part does not list one net per pin.
    BadPins(String),
    /// No drawing with the design's connectivity could be made.
    Layout,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::TooBig => write!(f, "the design has more than {MAX_PARTS} parts"),
            BuildError::UnknownPart(id) => write!(f, "there is no part kind `{id}`"),
            BuildError::BadPins(r) => write!(f, "{r} does not give one net per pin"),
            BuildError::Layout => write!(f, "the circuit could not be drawn"),
        }
    }
}

impl std::error::Error for BuildError {}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
            Dir::Right => (1, 0),
        }
    }
    fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
            Dir::Right => Dir::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Rect {
    fn empty() -> Rect {
        Rect {
            x0: i32::MAX,
            y0: i32::MAX,
            x1: i32::MIN,
            y1: i32::MIN,
        }
    }
    fn add(&mut self, x: i32, y: i32) {
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(x);
        self.y1 = self.y1.max(y);
    }
    fn is_empty(&self) -> bool {
        self.x0 > self.x1
    }
    fn shifted(&self, dx: i32, dy: i32) -> Rect {
        Rect {
            x0: self.x0 + dx,
            y0: self.y0 + dy,
            x1: self.x1 + dx,
            y1: self.y1 + dy,
        }
    }
    fn union(&mut self, o: &Rect) {
        if o.is_empty() {
            return;
        }
        self.add(o.x0, o.y0);
        self.add(o.x1, o.y1);
    }
}

fn local_rect(def: &PartDef) -> Rect {
    let mut r = Rect::empty();
    for p in &def.pins {
        r.add(p.position.x, p.position.y);
    }
    for s in &def.shapes {
        match s {
            Shape::Polyline { points, .. } => {
                for p in points {
                    r.add(p.x, p.y);
                }
            }
            Shape::Rect { min, max } => {
                r.add(min.x, min.y);
                r.add(max.x, max.y);
            }
            Shape::Circle { center, radius } | Shape::Arc { center, radius, .. } => {
                r.add(center.x - radius, center.y - radius);
                r.add(center.x + radius, center.y + radius);
            }
        }
    }
    if r.is_empty() {
        r.add(0, 0);
    }
    r
}

fn oriented_rect(def: &PartDef, rot: Rotation, mirror: bool) -> Rect {
    let l = local_rect(def);
    let mut r = Rect::empty();
    for (x, y) in [(l.x0, l.y0), (l.x1, l.y0), (l.x0, l.y1), (l.x1, l.y1)] {
        let p = transform(Point::new(x, y), rot, mirror);
        r.add(p.x, p.y);
    }
    r
}

/// Where one part ends up.
#[derive(Clone, Debug)]
struct Geo {
    pos: Point,
    rot: Rotation,
    mirror: bool,
    rect: Rect,
    pins: Vec<Point>,
    dirs: Vec<Dir>,
}

fn geo_of(def: &PartDef, pos: Point, rot: Rotation, mirror: bool) -> Geo {
    let rel = oriented_rect(def, rot, mirror);
    let mut pins = Vec::new();
    let mut dirs = Vec::new();
    for pd in &def.pins {
        let o = transform(pd.position, rot, mirror);
        pins.push(Point::new(pos.x + o.x, pos.y + o.y));
        let dx = 2 * o.x - (rel.x0 + rel.x1);
        let dy = 2 * o.y - (rel.y0 + rel.y1);
        dirs.push(if dx.abs() > dy.abs() {
            if dx < 0 {
                Dir::Left
            } else {
                Dir::Right
            }
        } else if dy > 0 {
            Dir::Down
        } else {
            Dir::Up
        });
    }
    Geo {
        pos,
        rot,
        mirror,
        rect: rel.shifted(pos.x, pos.y),
        pins,
        dirs,
    }
}

struct Ctx<'a> {
    design: &'a Design,
    defs: Vec<&'a PartDef>,
    nets: Vec<NetInfo>,
    /// Net id of every pin; open and no-connect pins get ids of their own, after the nets.
    pin_id: Vec<Vec<usize>>,
}

impl Ctx<'_> {
    fn kind_of_pin(&self, part: usize, pin: usize) -> Option<NetKind> {
        let id = *self.pin_id.get(part)?.get(pin)?;
        self.nets.get(id).map(|n| n.kind)
    }
}

fn context<'a>(design: &'a Design, lib: &'a Library) -> Result<Ctx<'a>, BuildError> {
    if design.parts.len() > MAX_PARTS {
        return Err(BuildError::TooBig);
    }
    let mut defs = Vec::new();
    for p in &design.parts {
        let def = lib
            .get(&p.lib_id)
            .ok_or_else(|| BuildError::UnknownPart(p.lib_id.clone()))?;
        if def.pins.len() != p.pins.len() {
            return Err(BuildError::BadPins(p.refdes.clone()));
        }
        defs.push(def);
    }
    let nets = design.nets();
    let index: HashMap<&str, usize> = nets
        .iter()
        .enumerate()
        .map(|(i, n)| (n.key.as_str(), i))
        .collect();
    let mut next = nets.len();
    let mut pin_id = Vec::new();
    for p in &design.parts {
        let mut ids = Vec::new();
        for pin in &p.pins {
            match pin {
                PinNet::Net(n) => {
                    ids.push(
                        index
                            .get(crate::design::net_key(n).as_str())
                            .copied()
                            .unwrap_or(0),
                    );
                }
                _ => {
                    ids.push(next);
                    next += 1;
                }
            }
        }
        pin_id.push(ids);
    }
    Ok(Ctx {
        design,
        defs,
        nets,
        pin_id,
    })
}

fn is_source(def: &PartDef) -> bool {
    matches!(def.device, Device::VSource | Device::ISource)
}

/// Pick the rotation of a part that has no position of its own.
fn auto_orient(cx: &Ctx<'_>, part: usize, depth: &[usize]) -> Rotation {
    let def = cx.defs[part];
    if def.pins.len() != 2 {
        return Rotation::R0;
    }
    let k0 = cx.kind_of_pin(part, 0);
    let k1 = cx.kind_of_pin(part, 1);
    let g = |k: Option<NetKind>| k == Some(NetKind::Ground);
    let s = |k: Option<NetKind>| k == Some(NetKind::Supply);
    if g(k1) || (g(k0) && g(k1)) {
        return Rotation::R0;
    }
    if g(k0) {
        return Rotation::R180;
    }
    if s(k0) && !s(k1) {
        return Rotation::R0;
    }
    if s(k1) {
        return Rotation::R180;
    }
    let avg = |pin: usize| -> Option<f64> {
        let id = *cx.pin_id[part].get(pin)?;
        let net = cx.nets.get(id)?;
        let ds: Vec<usize> = net
            .pins
            .iter()
            .filter(|(p, _)| *p != part)
            .filter_map(|(p, _)| depth.get(*p).copied())
            .collect();
        if ds.is_empty() {
            None
        } else {
            Some(ds.iter().sum::<usize>() as f64 / ds.len() as f64)
        }
    };
    let me = depth.get(part).copied().unwrap_or(0) as f64;
    let a0 = avg(0).unwrap_or(me - 0.5);
    let a1 = avg(1).unwrap_or(me + 0.5);
    if a0 <= a1 {
        Rotation::R90
    } else {
        Rotation::R270
    }
}

/// Arrange the parts in columns and rows. Returns the position and rotation of every part.
fn auto_place(cx: &Ctx<'_>) -> Vec<(Point, Rotation)> {
    let n = cx.design.parts.len();
    let mut adj: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    for net in &cx.nets {
        if net.kind == NetKind::Ground {
            continue;
        }
        for (a, _) in &net.pins {
            for (b, _) in &net.pins {
                if a != b {
                    adj[*a].insert(*b);
                }
            }
        }
    }
    let mut comp_of = vec![usize::MAX; n];
    let mut depth = vec![0usize; n];
    let mut comps: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if comp_of[start] != usize::MAX {
            continue;
        }
        let cid = comps.len();
        let mut members = Vec::new();
        let mut stack = vec![start];
        comp_of[start] = cid;
        while let Some(u) = stack.pop() {
            members.push(u);
            for &v in &adj[u] {
                if comp_of[v] == usize::MAX {
                    comp_of[v] = cid;
                    stack.push(v);
                }
            }
        }
        members.sort_unstable();
        let mut seeds: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&i| is_source(cx.defs[i]))
            .collect();
        if seeds.is_empty() {
            seeds.push(members[0]);
        }
        let mut seen: BTreeSet<usize> = seeds.iter().copied().collect();
        let mut queue: VecDeque<usize> = seeds.iter().copied().collect();
        while let Some(u) = queue.pop_front() {
            for &v in &adj[u] {
                if seen.insert(v) {
                    depth[v] = depth[u] + 1;
                    queue.push_back(v);
                }
            }
        }
        comps.push(members);
    }
    let rots: Vec<Rotation> = (0..n).map(|i| auto_orient(cx, i, &depth)).collect();
    struct Ext {
        rect: Rect,
        top: i32,
        above: i32,
        below: i32,
    }
    let exts: Vec<Ext> = (0..n)
        .map(|i| {
            let def = cx.defs[i];
            let rect = oriented_rect(def, rots[i], false);
            let top = def
                .pins
                .iter()
                .map(|p| transform(p.position, rots[i], false).y)
                .min()
                .unwrap_or(0);
            let has = |k: NetKind| (0..def.pins.len()).any(|p| cx.kind_of_pin(i, p) == Some(k));
            Ext {
                rect,
                top,
                above: top - rect.y0 + if has(NetKind::Supply) { 4 } else { 0 },
                below: rect.y1 - top + if has(NetKind::Ground) { 5 } else { 0 },
            }
        })
        .collect();
    let mut placed = vec![(Point::new(0, 0), Rotation::R0); n];
    let mut y_cursor = 0;
    for members in &comps {
        let mut cols: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &m in members {
            cols.entry(depth[m]).or_default().push(m);
        }
        let rail = y_cursor + members.iter().map(|&m| exts[m].above).max().unwrap_or(0);
        let mut x_cursor = 0;
        let mut bottom = rail;
        for col in cols.values() {
            let width = col
                .iter()
                .map(|&m| exts[m].rect.x1 - exts[m].rect.x0)
                .max()
                .unwrap_or(0);
            let cx_col = x_cursor + width.div_euclid(2);
            let mut slot = rail;
            for (k, &m) in col.iter().enumerate() {
                let e = &exts[m];
                if k > 0 {
                    slot += e.above + 4;
                }
                placed[m] = (
                    Point::new(cx_col - (e.rect.x0 + e.rect.x1).div_euclid(2), slot - e.top),
                    rots[m],
                );
                slot += e.below;
            }
            bottom = bottom.max(slot);
            x_cursor += width + 12;
        }
        y_cursor = bottom + 8;
    }
    placed
}

fn plan(cx: &Ctx<'_>, previous: Option<&Schematic>) -> Vec<Geo> {
    let n = cx.design.parts.len();
    let auto = auto_place(cx);
    let old: HashMap<String, (Point, Rotation, bool, &str)> = previous
        .and_then(|s| s.sheets.first())
        .map(|sh| {
            sh.parts
                .iter()
                .map(|p| {
                    (
                        p.refdes.to_lowercase(),
                        (p.position, p.rotation, p.mirror, p.lib_id.as_str()),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let mut fixed: Vec<Option<(Point, Rotation, bool)>> = Vec::new();
    for spec in &cx.design.parts {
        let f = spec
            .place
            .map(|p| (p.position, p.rotation, p.mirror))
            .or_else(|| {
                old.get(&spec.refdes.to_lowercase())
                    .filter(|o| o.3 == spec.lib_id)
                    .map(|o| (o.0, o.1, o.2))
            });
        fixed.push(f);
    }
    let mut geos: Vec<Geo> = (0..n)
        .map(|i| match fixed[i] {
            Some((p, r, m)) => geo_of(cx.defs[i], p, r, m),
            None => geo_of(cx.defs[i], auto[i].0, auto[i].1, false),
        })
        .collect();
    if fixed.iter().any(Option::is_some) && fixed.iter().any(Option::is_none) {
        let mut taken = Rect::empty();
        let mut fresh = Rect::empty();
        for (i, g) in geos.iter().enumerate() {
            if fixed[i].is_some() {
                taken.union(&g.rect);
            } else {
                fresh.union(&g.rect);
            }
        }
        let (dx, dy) = (taken.x1 + 12 - fresh.x0, taken.y0 - fresh.y0);
        for i in 0..n {
            if fixed[i].is_none() {
                let g = &geos[i];
                geos[i] = geo_of(
                    cx.defs[i],
                    Point::new(g.pos.x + dx, g.pos.y + dy),
                    g.rot,
                    g.mirror,
                );
            }
        }
    }
    geos
}

#[derive(Clone, Copy)]
struct Seg {
    a: Point,
    b: Point,
    net: usize,
}

impl Seg {
    fn horizontal(&self) -> bool {
        self.a.y == self.b.y
    }
    fn xr(&self) -> (i32, i32) {
        (self.a.x.min(self.b.x), self.a.x.max(self.b.x))
    }
    fn yr(&self) -> (i32, i32) {
        (self.a.y.min(self.b.y), self.a.y.max(self.b.y))
    }
    fn contains(&self, p: Point) -> bool {
        let (x0, x1) = self.xr();
        let (y0, y1) = self.yr();
        (x0..=x1).contains(&p.x) && (y0..=y1).contains(&p.y)
    }
    fn conflicts(&self, o: &Seg) -> bool {
        let (sx, sy, ox, oy) = (self.xr(), self.yr(), o.xr(), o.yr());
        match (self.horizontal(), o.horizontal()) {
            (true, true) => self.a.y == o.a.y && sx.0.max(ox.0) <= sx.1.min(ox.1),
            (false, false) => self.a.x == o.a.x && sy.0.max(oy.0) <= sy.1.min(oy.1),
            (true, false) => cross(self, o),
            (false, true) => cross(o, self),
        }
    }
    fn enters(&self, r: &Rect) -> bool {
        let (sx, sy) = (self.xr(), self.yr());
        if self.horizontal() {
            r.y0 < self.a.y && self.a.y < r.y1 && sx.0.max(r.x0) < sx.1.min(r.x1)
        } else {
            r.x0 < self.a.x && self.a.x < r.x1 && sy.0.max(r.y0) < sy.1.min(r.y1)
        }
    }
}

/// Whether horizontal `h` and vertical `v` touch other than by a clean crossing.
fn cross(h: &Seg, v: &Seg) -> bool {
    let (hx0, hx1) = h.xr();
    let (vy0, vy1) = v.yr();
    let meet = (hx0..=hx1).contains(&v.a.x) && (vy0..=vy1).contains(&h.a.y);
    let clean = hx0 < v.a.x && v.a.x < hx1 && vy0 < h.a.y && h.a.y < vy1;
    meet && !clean
}

struct Router {
    boxes: Vec<Rect>,
    segs: Vec<Seg>,
    points: Vec<(Point, usize)>,
    wires: Vec<Vec<Point>>,
    labels: Vec<Label>,
    ports: Vec<PowerPort>,
    dir_at: HashMap<Point, Dir>,
}

impl Router {
    fn path_ok(&self, path: &[Point], net: usize) -> bool {
        for w in path.windows(2) {
            let s = Seg {
                a: w[0],
                b: w[1],
                net,
            };
            if s.a == s.b {
                continue;
            }
            if self.boxes.iter().any(|r| s.enters(r)) {
                return false;
            }
            if self.points.iter().any(|(p, n)| *n != net && s.contains(*p)) {
                return false;
            }
            if self.segs.iter().any(|t| t.net != net && s.conflicts(t)) {
                return false;
            }
        }
        true
    }

    fn add_path(&mut self, path: &[Point], net: usize) {
        for w in path.windows(2) {
            if w[0] != w[1] {
                self.segs.push(Seg {
                    a: w[0],
                    b: w[1],
                    net,
                });
            }
        }
        self.wires.push(path.to_vec());
    }

    fn point_free(&self, p: Point, net: usize) -> bool {
        !self.segs.iter().any(|s| s.net != net && s.contains(p))
            && !self.points.iter().any(|(q, n)| *n != net && *q == p)
    }

    /// A short wire leaving a pin, clear of everything else, with its end point returned. It
    /// goes the way `dir` says when it can, then any other way; two grid units long when it
    /// can, longer when something is in the way.
    fn stub(&mut self, p: Point, dir: Dir, net: usize) -> (Point, Dir) {
        let others = [Dir::Up, Dir::Down, Dir::Left, Dir::Right]
            .into_iter()
            .filter(|d| *d != dir);
        for d in std::iter::once(dir).chain(others) {
            for len in [2, 3, 4, 6, 8, 12] {
                let (dx, dy) = d.delta();
                let end = Point::new(p.x + len * dx, p.y + len * dy);
                if self.path_ok(&[p, end], net) && self.point_free(end, net) {
                    self.add_path(&[p, end], net);
                    self.points.push((end, net));
                    return (end, d);
                }
            }
        }
        let (dx, dy) = dir.delta();
        let end = Point::new(p.x + 2 * dx, p.y + 2 * dy);
        self.add_path(&[p, end], net);
        self.points.push((end, net));
        (end, dir)
    }

    fn score(&self, path: &[Point]) -> i32 {
        let dir_of = |a: Point, b: Point| {
            if b.x > a.x {
                Dir::Right
            } else if b.x < a.x {
                Dir::Left
            } else if b.y > a.y {
                Dir::Down
            } else {
                Dir::Up
            }
        };
        let mut s = 0;
        if let (Some(&a), Some(&b)) = (path.first(), path.get(1)) {
            if self.dir_at.get(&a) == Some(&dir_of(a, b)) {
                s += 1;
            }
        }
        if path.len() >= 2 {
            let (a, b) = (path[path.len() - 2], path[path.len() - 1]);
            if self.dir_at.get(&b).map(|d| d.opposite()) == Some(dir_of(a, b)) {
                s += 1;
            }
        }
        s
    }

    fn candidates(a: Point, b: Point) -> Vec<Vec<Point>> {
        if a.x == b.x || a.y == b.y {
            vec![vec![a, b]]
        } else {
            vec![
                vec![a, Point::new(b.x, a.y), b],
                vec![a, Point::new(a.x, b.y), b],
            ]
        }
    }

    /// Join the pins of a net with wires; false (with nothing added) when a pair has no clean route.
    fn wire_net(&mut self, pins: &[Point], net: usize) -> bool {
        let mark = (self.segs.len(), self.wires.len());
        let mut connected = vec![pins[0]];
        let mut rest: Vec<Point> = pins[1..].to_vec();
        while !rest.is_empty() {
            let mut best: Option<(i32, usize)> = None;
            for (i, r) in rest.iter().enumerate() {
                let d = connected
                    .iter()
                    .map(|c| (c.x - r.x).abs() + (c.y - r.y).abs())
                    .min()
                    .unwrap_or(0);
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, i));
                }
            }
            let Some((_, i)) = best else { break };
            let r = rest.remove(i);
            let mut order: Vec<Point> = connected.clone();
            order.sort_by_key(|c| (c.x - r.x).abs() + (c.y - r.y).abs());
            let mut done = connected.contains(&r);
            for c in order {
                if done {
                    break;
                }
                let mut options: Vec<(i32, Vec<Point>)> = Router::candidates(c, r)
                    .into_iter()
                    .filter(|p| self.path_ok(p, net))
                    .map(|p| (self.score(&p), p))
                    .collect();
                options.sort_by_key(|(s, _)| -*s);
                if let Some((_, p)) = options.into_iter().next() {
                    self.add_path(&p, net);
                    done = true;
                }
            }
            if !done {
                self.segs.truncate(mark.0);
                self.wires.truncate(mark.1);
                return false;
            }
            connected.push(r);
        }
        true
    }

    /// A point on the net's new wires where its name can sit.
    fn label_spot(&self, from_seg: usize, net: usize) -> Option<Point> {
        let mut mine: Vec<Seg> = self
            .segs
            .get(from_seg..)?
            .iter()
            .filter(|s| s.net == net)
            .copied()
            .collect();
        mine.sort_by_key(|s| -((s.a.x - s.b.x).abs() + (s.a.y - s.b.y).abs()));
        for s in &mine {
            let len = (s.a.x - s.b.x).abs() + (s.a.y - s.b.y).abs();
            if len >= 2 {
                let m = Point::new((s.a.x + s.b.x).div_euclid(2), (s.a.y + s.b.y).div_euclid(2));
                if self.point_free(m, net) {
                    return Some(m);
                }
            }
        }
        mine.first().map(|s| s.a)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Wires,
    Labels,
}

fn assemble(cx: &Ctx<'_>, geos: &[Geo], mode: Mode, previous: Option<&Schematic>) -> Schematic {
    let design = cx.design;
    let mut r = Router {
        boxes: geos.iter().map(|g| g.rect).collect(),
        segs: Vec::new(),
        points: Vec::new(),
        wires: Vec::new(),
        labels: Vec::new(),
        ports: Vec::new(),
        dir_at: HashMap::new(),
    };
    let mut no_connects = Vec::new();
    for (pi, g) in geos.iter().enumerate() {
        for (k, p) in g.pins.iter().enumerate() {
            let id = cx.pin_id[pi][k];
            r.points.push((*p, id));
            r.dir_at.insert(*p, g.dirs[k]);
            if design.parts[pi].pins[k] == PinNet::NoConnect {
                no_connects.push(NoConnect { position: *p });
            }
        }
    }
    let name_label = |name: &str, at: Point| Label {
        name: name.to_string(),
        position: at,
        scope: LabelScope::Local,
    };
    // Ground and supply: a power port at the end of a stub, or a label when the pin points the
    // wrong way for the symbol.
    for (id, net) in cx.nets.iter().enumerate() {
        if net.kind == NetKind::Signal {
            continue;
        }
        let want = if net.kind == NetKind::Ground {
            Dir::Down
        } else {
            Dir::Up
        };
        // A label and a power port of one name do not join, so a supply rail is drawn all with
        // ports or all with labels; ground names join either way.
        let all_fit = net.pins.iter().all(|&(pi, k)| geos[pi].dirs[k] == want);
        for &(pi, k) in &net.pins {
            let (p, d) = (geos[pi].pins[k], geos[pi].dirs[k]);
            let (end, used) = r.stub(p, d, id);
            if used == want && (net.kind == NetKind::Ground || all_fit) {
                r.ports.push(PowerPort {
                    name: if net.kind == NetKind::Ground {
                        "GND".to_string()
                    } else {
                        net.name.clone()
                    },
                    position: end,
                    style: if net.kind == NetKind::Ground {
                        PowerStyle::Ground
                    } else {
                        PowerStyle::Supply
                    },
                });
            } else {
                r.labels.push(name_label(&net.name, end));
            }
        }
    }
    let mut order: Vec<usize> = (0..cx.nets.len())
        .filter(|&i| cx.nets[i].kind == NetKind::Signal)
        .collect();
    let span = |i: usize| -> i32 {
        let pts: Vec<Point> = cx.nets[i]
            .pins
            .iter()
            .map(|&(p, k)| geos[p].pins[k])
            .collect();
        let (x0, x1) = (
            pts.iter().map(|p| p.x).min().unwrap_or(0),
            pts.iter().map(|p| p.x).max().unwrap_or(0),
        );
        let (y0, y1) = (
            pts.iter().map(|p| p.y).min().unwrap_or(0),
            pts.iter().map(|p| p.y).max().unwrap_or(0),
        );
        (x1 - x0) + (y1 - y0)
    };
    order.sort_by_key(|&i| (span(i), i));
    for id in order {
        let net = &cx.nets[id];
        let mut pts: Vec<Point> = net.pins.iter().map(|&(p, k)| geos[p].pins[k]).collect();
        pts.sort();
        pts.dedup();
        let seg_mark = r.segs.len();
        let wired = mode == Mode::Wires && pts.len() >= 2 && r.wire_net(&pts, id);
        if wired {
            if !is_anonymous(&net.name) {
                if let Some(at) = r.label_spot(seg_mark, id) {
                    r.labels.push(name_label(&net.name, at));
                    r.points.push((at, id));
                }
            }
            continue;
        }
        if mode == Mode::Wires && is_anonymous(&net.name) && pts.len() < 2 {
            continue;
        }
        for &(pi, k) in &net.pins {
            let (end, _) = r.stub(geos[pi].pins[k], geos[pi].dirs[k], id);
            r.labels.push(name_label(&net.name, end));
        }
    }
    let mut s = Schematic::new(&design.title);
    s.models = design.models.clone();
    s.analyses = design.analyses.clone();
    if let Some(old) = previous {
        if s.title.is_empty() {
            s.title.clone_from(&old.title);
        }
    }
    if let Some(sheet) = s.sheets.first_mut() {
        for (i, spec) in design.parts.iter().enumerate() {
            let mut part = edit::new_part(cx.defs[i], &spec.refdes, geos[i].pos);
            part.value.clone_from(&spec.value);
            part.rotation = geos[i].rot;
            part.mirror = geos[i].mirror;
            part.model.clone_from(&spec.model);
            part.fields.clone_from(&spec.fields);
            sheet.parts.push(part);
        }
        sheet.wires = r.wires.into_iter().map(|points| Wire { points }).collect();
        sheet.labels = r.labels;
        sheet.power_ports = r.ports;
        sheet.no_connects = no_connects;
        edit::normalize(sheet);
    }
    s
}

/// Whether the connectivity of the drawing is the design's, with nothing left dangling; or why not.
fn check(cx: &Ctx<'_>, s: &Schematic) -> Result<(), String> {
    let res = resolve_nets(s, Library::builtin()).map_err(|e| e.to_string())?;
    if let Some(w) = res.warnings.first() {
        return Err(format!("{w:?}"));
    }
    let mut by_net: HashMap<usize, usize> = HashMap::new();
    let mut used: BTreeSet<usize> = BTreeSet::new();
    for (pi, ids) in cx.pin_id.iter().enumerate() {
        for (k, &id) in ids.iter().enumerate() {
            let pin = circuito_core::schematic::PinRef {
                sheet: 0,
                part: pi,
                pin: k,
            };
            let Some(&resolved) = res.pin_net.get(&pin) else {
                return Err(format!("pin {pi}/{k} is on no net"));
            };
            let name = cx.nets.get(id).map_or("(open)", |n| n.name.as_str());
            if id < cx.nets.len() {
                match by_net.get(&id) {
                    Some(&r) if r != resolved => {
                        return Err(format!("net {name} is split ({pi}/{k})"))
                    }
                    Some(_) => {}
                    None => {
                        if !used.insert(resolved) {
                            return Err(format!("net {name} is joined to another ({pi}/{k})"));
                        }
                        by_net.insert(id, resolved);
                    }
                }
            } else if !used.insert(resolved) {
                return Err(format!("open pin {pi}/{k} is joined to a net"));
            }
        }
    }
    Ok(())
}

fn verify(cx: &Ctx<'_>, s: &Schematic) -> bool {
    let r = check(cx, s);
    if let Err(e) = &r {
        if std::env::var_os("CIRCUITO_SCRIPT_DEBUG").is_some() {
            eprintln!("layout check: {e}");
        }
    }
    r.is_ok()
}

/// Draw a design. Parts with a position (in the text, or in `previous` under the same
/// reference designator and kind) stay where they are; the others are placed automatically.
///
/// # Errors
/// When the design is too big, names an unknown part or cannot be drawn with its connectivity.
pub fn build(design: &Design, previous: Option<&Schematic>) -> Result<Schematic, BuildError> {
    let lib = Library::builtin();
    let cx = context(design, lib)?;
    let geos = plan(&cx, previous);
    for mode in [Mode::Wires, Mode::Labels] {
        let s = assemble(&cx, &geos, mode, previous);
        if verify(&cx, &s) {
            return Ok(s);
        }
    }
    Err(BuildError::Layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn drawn(text: &str) -> Schematic {
        let p = parse(text);
        assert!(p.is_clean(), "{:?}", p.problems);
        build(&p.design, None).expect("drawable")
    }

    #[test]
    fn a_ladder_is_drawn_with_wires() {
        let s = drawn("V1 5 in gnd\nR1 1k in out\nC1 100n out gnd\n");
        let sh = &s.sheets[0];
        assert_eq!(sh.parts.len(), 3);
        assert!(!sh.wires.is_empty());
        assert_eq!(sh.power_ports.len(), 2);
        assert!(
            sh.labels.iter().any(|l| l.name == "in") && sh.labels.iter().any(|l| l.name == "out")
        );
        let r1 = sh.parts[1].position;
        let c1 = sh.parts[2].position;
        assert!(r1.x < c1.x, "signal flows left to right");
    }

    #[test]
    fn every_example_is_drawn_with_the_designs_connectivity() {
        for text in [
            "V1 5 vcc gnd\nR1 10k vcc out\nR2 10k out gnd\n",
            "V1 SIN(0 1 1k) in gnd\nRB 100k vcc b\nRC 2.2k vcc c\nQ1 c=c b=in e=gnd\nV2 12 vcc gnd\nC1 1u in b\n",
            "R1 1k a b\nR2 1k b c\nR3 1k c a\nR4 1k a _\nD1 b ?\n",
            "U1:OPAMP +=in -=fb out=out\nR1 1k out fb\nR2 1k fb gnd\nV1 1 in gnd\n",
            "V1 5 a b\nR1 1k c d\n",
            "R1 1k x x\nR2 1k gnd gnd\n",
        ] {
            let p = parse(text);
            assert!(p.is_clean(), "{text}: {:?}", p.problems);
            let s = build(&p.design, None).unwrap_or_else(|e| panic!("{text}: {e}"));
            let cx = context(&p.design, Library::builtin()).expect("context");
            assert!(verify(&cx, &s), "{text}");
        }
    }

    #[test]
    fn fixed_parts_stay_and_new_ones_go_beside_them() {
        let first = drawn("V1 5 in gnd\nR1 1k in out\n");
        let p = parse("V1 5 in gnd\nR1 1k in out\nC1 1u out gnd\n");
        let s = build(&p.design, Some(&first)).expect("drawable");
        assert_eq!(
            s.sheets[0].parts[0].position,
            first.sheets[0].parts[0].position
        );
        assert_eq!(
            s.sheets[0].parts[1].position,
            first.sheets[0].parts[1].position
        );
        assert!(s.sheets[0].parts[2].position.x > first.sheets[0].parts[1].position.x);
    }

    #[test]
    fn placement_hints_win() {
        let s = drawn("R1 1k a b @20,30 r90\n");
        let r = &s.sheets[0].parts[0];
        assert_eq!(
            (r.position, r.rotation),
            (Point::new(20, 30), Rotation::R90)
        );
    }

    #[test]
    fn unknown_parts_and_bad_pin_lists_are_errors() {
        let mut d = parse("R1 1k a b\n").design;
        d.parts[0].pins.pop();
        assert_eq!(build(&d, None), Err(BuildError::BadPins("R1".into())));
        d.parts[0].lib_id = "NOPE".into();
        assert!(matches!(build(&d, None), Err(BuildError::UnknownPart(_))));
    }

    #[test]
    fn an_empty_design_is_an_empty_schematic() {
        let s = build(&Design::default(), None).expect("drawable");
        assert!(s.sheets[0].parts.is_empty());
    }
}

// SPDX-License-Identifier: MIT OR Apache-2.0
//! Import of a `.kicad_pcb` board (KiCad 7 and 8 syntax) into the PCB engine's model.
//!
//! KiCad's Y axis points down and Circuito's points up, so every Y coordinate is negated; angles
//! keep their sign because a counter-clockwise turn stays counter-clockwise under that mirror.
//!
//! What is imported: copper layer count, nets, footprints on the top side with their pads and
//! graphics, tracks (segments and arcs), vias, copper zones (outline and the fills the file
//! stores), keepout areas, the `Edge.Cuts` outline, and board graphics and text. What is not, and
//! is reported in [`Import::warnings`] rather than dropped silently: footprints on the bottom side,
//! pad shapes other than circle, rect, oval and roundrect (they become rectangles), footprint arcs,
//! net classes, dimensions, groups, images and anything on a layer this model has no name for.

use std::collections::HashMap;

use circuito_pcb::board::{
    Board, Drill, Footprint, Graphic, GraphicKind, Id, Justify, Keepout, Net, NetId, Outline,
    OutlineSeg, Pad, PadKind, PadShape, Size, TextItem, Thermal, ThermalMode, Track, TrackGeom,
    Via, ViaKind, Zone, DEFAULT_CLASS,
};
use circuito_pcb::geom::arc::Arc;
use circuito_pcb::geom::point::Point;
use circuito_pcb::geom::region::{ring_area2, Region};
use circuito_pcb::layer::{Layer, Side};
use circuito_pcb::units::Nm;

use crate::error::{Error, Result};
use crate::sexpr::{self, Node};

/// What an import produced.
#[derive(Debug)]
pub struct Import {
    /// The board, already validated by the PCB engine.
    pub board: Board,
    /// Things in the file that were not imported exactly, one line each (at most [`MAX_WARNINGS`]).
    pub warnings: Vec<String>,
    /// The file's `(version N)`, when it has one.
    pub file_version: Option<u64>,
}

/// Most warnings kept; the rest are counted in a final line.
pub const MAX_WARNINGS: usize = 200;
/// Largest distance at which two outline ends are taken to be the same point, in nm.
const JOIN_TOLERANCE: f64 = 5_000.0;
/// Most copper layers KiCad (and this model) allow.
const MAX_COPPER: usize = 32;

struct Ctx {
    warnings: Vec<String>,
    dropped: usize,
    next_id: u64,
    nets: Vec<Net>,
    by_num: HashMap<i64, NetId>,
    by_name: HashMap<String, NetId>,
    skipped_layers: HashMap<String, usize>,
}

impl Ctx {
    fn warn(&mut self, msg: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(msg);
        } else {
            self.dropped += 1;
        }
    }
    fn id(&mut self) -> Id {
        self.next_id += 1;
        Id(self.next_id)
    }
    fn net_named(&mut self, name: &str) -> Option<NetId> {
        if name.is_empty() {
            return None;
        }
        if let Some(id) = self.by_name.get(name) {
            return Some(*id);
        }
        let id = NetId(u32::try_from(self.nets.len() + 1).ok()?);
        self.nets.push(Net {
            id,
            name: name.to_string(),
            class: DEFAULT_CLASS.to_string(),
        });
        self.by_name.insert(name.to_string(), id);
        Some(id)
    }
    /// The net a `(net 3 "GND")`, `(net 3)` or `(net "GND")` item names.
    fn net_of(&mut self, item: &Node) -> Option<NetId> {
        let args = item.args();
        let num = args
            .first()
            .and_then(Node::text)
            .and_then(|s| s.parse::<i64>().ok());
        let name = args.iter().find_map(|n| {
            if let Node::Str(s) = n {
                Some(s.as_str())
            } else {
                None
            }
        });
        if let Some(n) = num {
            if let Some(id) = self.by_num.get(&n) {
                return Some(*id);
            }
        }
        let id = self.net_named(name?)?;
        if let Some(n) = num {
            self.by_num.insert(n, id);
        }
        Some(id)
    }
    fn layer(&mut self, name: &str) -> Option<Layer> {
        let l = layer_named(name);
        if l.is_none() {
            *self.skipped_layers.entry(name.to_string()).or_insert(0) += 1;
        }
        l
    }
}

fn invalid(context: &str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        context: context.to_string(),
        detail: detail.into(),
    }
}

/// Millimetres to nanometres, refusing anything beyond ±10 m (the engine's limit is about 1 m).
fn nm(mm: f64, what: &str) -> Result<Nm> {
    if !mm.is_finite() || mm.abs() > 10_000.0 {
        return Err(invalid(what, format!("{mm} mm is out of range")));
    }
    Ok((mm * 1e6).round() as Nm)
}

fn num(n: Option<&Node>) -> Option<f64> {
    n?.text()?.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn point(x: f64, y: f64, what: &str) -> Result<Point> {
    Ok(Point::new(nm(x, what)?, -nm(y, what)?))
}

/// A child `(name X Y)` as a point.
fn xy(node: &Node, name: &str, what: &str) -> Result<Option<Point>> {
    let Some(c) = node.child(name) else {
        return Ok(None);
    };
    match (num(c.args().first()), num(c.args().get(1))) {
        (Some(x), Some(y)) => Ok(Some(point(x, y, what)?)),
        _ => Err(invalid(what, format!("`{name}` needs two numbers"))),
    }
}

/// A child `(at X Y [ANGLE])`: the position and the angle in degrees.
fn at(node: &Node, what: &str) -> Result<(Point, f64)> {
    let c = node
        .child("at")
        .ok_or_else(|| invalid(what, "missing `at`"))?;
    match (num(c.args().first()), num(c.args().get(1))) {
        (Some(x), Some(y)) => Ok((point(x, y, what)?, num(c.args().get(2)).unwrap_or(0.0))),
        _ => Err(invalid(what, "`at` needs two numbers")),
    }
}

/// Degrees to millidegrees, normalised to (-180000, 180000].
fn mdeg(deg: f64) -> i32 {
    let mut m = (deg * 1000.0).round() % 360_000.0;
    if m > 180_000.0 {
        m -= 360_000.0;
    } else if m <= -180_000.0 {
        m += 360_000.0;
    }
    m as i32
}

fn width_of(node: &Node) -> f64 {
    node.arg_f64("width")
        .or_else(|| node.child("stroke").and_then(|s| s.arg_f64("width")))
        .unwrap_or(0.0)
}

fn layer_named(name: &str) -> Option<Layer> {
    Some(match name {
        "F.Cu" => Layer::FCu,
        "B.Cu" => Layer::BCu,
        "F.Mask" => Layer::FMask,
        "B.Mask" => Layer::BMask,
        "F.Paste" => Layer::FPaste,
        "B.Paste" => Layer::BPaste,
        "F.SilkS" | "F.Silkscreen" => Layer::FSilk,
        "B.SilkS" | "B.Silkscreen" => Layer::BSilk,
        "F.CrtYd" | "F.Courtyard" => Layer::FCourtyard,
        "B.CrtYd" | "B.Courtyard" => Layer::BCourtyard,
        "F.Fab" => Layer::FFab,
        "B.Fab" => Layer::BFab,
        "Edge.Cuts" => Layer::EdgeCuts,
        other => {
            let n: u8 = other
                .strip_prefix("In")?
                .strip_suffix(".Cu")?
                .parse()
                .ok()?;
            if (1..=30).contains(&n) {
                Layer::InnerCu(n)
            } else {
                return None;
            }
        }
    })
}

/// Import a `.kicad_pcb` file. `name` becomes the board's name.
pub fn import(text: &str, name: &str) -> Result<Import> {
    let root = sexpr::parse(text)?;
    if root.head() != Some("kicad_pcb") {
        return Err(Error::WrongKind {
            expected: "KiCad board (.kicad_pcb)",
            detail: format!("it starts with `{}`", root.head().unwrap_or("?")),
        });
    }
    let file_version = root.arg_text("version").and_then(|v| v.parse().ok());
    let copper = root
        .child("layers")
        .map(|l| {
            l.items()
                .iter()
                .skip(1)
                .filter(|n| {
                    n.items()
                        .get(1)
                        .and_then(Node::text)
                        .is_some_and(|s| s.ends_with(".Cu"))
                })
                .count()
        })
        .filter(|n| *n > 0)
        .unwrap_or(2);
    if copper > MAX_COPPER {
        return Err(invalid(
            "layers",
            format!("{copper} copper layers exceed {MAX_COPPER}"),
        ));
    }
    let mut board = Board::new(name, copper as u8).map_err(|e| Error::Board(e.to_string()))?;
    let mut cx = Ctx {
        warnings: Vec::new(),
        dropped: 0,
        next_id: 0,
        nets: Vec::new(),
        by_num: HashMap::new(),
        by_name: HashMap::new(),
        skipped_layers: HashMap::new(),
    };
    // The net table first, so numbers resolve however the items are ordered.
    for n in root.children("net") {
        cx.net_of(n);
    }
    let mut edge: Vec<OutlineSeg> = Vec::new();
    let mut skipped_kinds: HashMap<String, usize> = HashMap::new();
    for item in root.items().iter().skip(1) {
        let Some(head) = item.head() else { continue };
        match head {
            "footprint" | "module" => {
                if let Some(fp) = footprint(&mut cx, item)? {
                    board.footprints.push(fp);
                }
            }
            "segment" => {
                let (Some(start), Some(end)) =
                    (xy(item, "start", "segment")?, xy(item, "end", "segment")?)
                else {
                    cx.warn("a segment without start or end was skipped".into());
                    continue;
                };
                let layer = item.arg_text("layer").and_then(|l| cx.layer(l));
                let net = item.child("net").and_then(|n| cx.net_of(n));
                if let Some(layer) = layer.filter(|l| l.is_copper()) {
                    if start == end {
                        cx.warn("a zero-length track was skipped".into());
                        continue;
                    }
                    let id = cx.id();
                    board.tracks.push(Track {
                        id,
                        net,
                        layer,
                        width: nm(width_of(item), "segment width")?,
                        geom: TrackGeom::Segment { start, end },
                    });
                }
            }
            "arc" => {
                let (Some(start), Some(mid), Some(end)) = (
                    xy(item, "start", "arc")?,
                    xy(item, "mid", "arc")?,
                    xy(item, "end", "arc")?,
                ) else {
                    cx.warn("an arc track without start, mid or end was skipped".into());
                    continue;
                };
                let layer = item.arg_text("layer").and_then(|l| cx.layer(l));
                let net = item.child("net").and_then(|n| cx.net_of(n));
                if let Some(layer) = layer.filter(|l| l.is_copper()) {
                    match Arc::through(start, mid, end) {
                        Ok(a) => {
                            let id = cx.id();
                            board.tracks.push(Track {
                                id,
                                net,
                                layer,
                                width: nm(width_of(item), "arc width")?,
                                geom: TrackGeom::Arc(a),
                            });
                        }
                        Err(e) => cx.warn(format!("an arc track was skipped: {e}")),
                    }
                }
            }
            "via" => {
                let v = via(&mut cx, item)?;
                board.vias.push(v);
            }
            "zone" => zone(&mut cx, item, &mut board)?,
            "gr_line" | "gr_arc" | "gr_circle" | "gr_rect" | "gr_poly" | "gr_curve" => {
                board_graphic(&mut cx, item, &mut board, &mut edge)?;
            }
            "gr_text" => {
                if let Some(t) = text_item(&mut cx, item, None)? {
                    board.texts.push(t);
                }
            }
            "version" | "generator" | "generator_version" | "general" | "paper" | "layers"
            | "setup" | "net" | "title_block" | "uuid" | "property" | "embedded_fonts" => {}
            "net_class" => *skipped_kinds.entry("net_class".into()).or_insert(0) += 1,
            other => *skipped_kinds.entry(other.to_string()).or_insert(0) += 1,
        }
    }
    for (kind, n) in sorted(skipped_kinds) {
        cx.warn(format!("{n} `{kind}` item(s) are not imported"));
    }
    for (layer, n) in sorted(std::mem::take(&mut cx.skipped_layers)) {
        cx.warn(format!(
            "{n} item(s) on layer `{layer}` were skipped: this model has no such layer"
        ));
    }
    board.outline = outline(&mut cx, edge);
    board.nets = std::mem::take(&mut cx.nets);
    board.validate().map_err(|e| Error::Board(e.to_string()))?;
    if cx.dropped > 0 {
        let msg = format!("{} more warning(s) not shown", cx.dropped);
        cx.warnings.push(msg);
    }
    Ok(Import {
        board,
        warnings: cx.warnings,
        file_version,
    })
}

fn sorted(m: HashMap<String, usize>) -> Vec<(String, usize)> {
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort();
    v
}

fn via(cx: &mut Ctx, item: &Node) -> Result<Via> {
    let (position, _) = at(item, "via")?;
    let diameter = nm(item.arg_f64("size").unwrap_or(0.8), "via size")?;
    let drill = nm(item.arg_f64("drill").unwrap_or(0.4), "via drill")?;
    let kind = match item.args().first().and_then(Node::text) {
        Some("blind") => ViaKind::Blind,
        Some("micro") => ViaKind::Micro,
        _ => ViaKind::Through,
    };
    let layers: Vec<&str> = item
        .child("layers")
        .map(|l| l.args().iter().filter_map(Node::text).collect())
        .unwrap_or_default();
    let from = layers
        .first()
        .and_then(|l| cx.layer(l))
        .unwrap_or(Layer::FCu);
    let to = layers
        .get(1)
        .and_then(|l| cx.layer(l))
        .unwrap_or(Layer::BCu);
    let net = item.child("net").and_then(|n| cx.net_of(n));
    let id = cx.id();
    Ok(Via {
        id,
        net,
        position,
        diameter,
        drill,
        kind,
        from,
        to,
        tented: true,
    })
}

fn footprint(cx: &mut Ctx, item: &Node) -> Result<Option<Footprint>> {
    let full = item
        .args()
        .first()
        .and_then(Node::text)
        .unwrap_or("")
        .to_string();
    let package = full.rsplit(':').next().unwrap_or("").to_string();
    let layer = item.arg_text("layer").unwrap_or("F.Cu");
    let (position, angle) = at(item, "footprint")?;
    let mut reference = String::new();
    let mut value = String::new();
    for p in item.children("property") {
        let (k, v) = (
            p.args().first().and_then(Node::text),
            p.args().get(1).and_then(Node::text),
        );
        match (k, v) {
            (Some("Reference"), Some(v)) => reference = v.to_string(),
            (Some("Value"), Some(v)) => value = v.to_string(),
            _ => {}
        }
    }
    for t in item.children("fp_text") {
        let (k, v) = (
            t.args().first().and_then(Node::text),
            t.args().get(1).and_then(Node::text),
        );
        match (k, v) {
            (Some("reference"), Some(v)) if reference.is_empty() => reference = v.to_string(),
            (Some("value"), Some(v)) if value.is_empty() => value = v.to_string(),
            _ => {}
        }
    }
    if layer == "B.Cu" {
        cx.warn(format!(
            "footprint {} is on the bottom side and was not imported (its placement is unverified)",
            label(&reference, &package)
        ));
        return Ok(None);
    }
    let attr: Vec<&str> = item
        .child("attr")
        .map(|a| a.args().iter().filter_map(Node::text).collect())
        .unwrap_or_default();
    let id = cx.id();
    let mut fp = Footprint {
        id,
        reference,
        value,
        package,
        position,
        rotation: mdeg(angle),
        side: Side::Front,
        pads: Vec::new(),
        courtyard: None,
        graphics: Vec::new(),
        texts: Vec::new(),
        exclude_from_bom: attr.contains(&"exclude_from_bom"),
        exclude_from_pos: attr.contains(&"exclude_from_pos_files"),
    };
    let mut arcs = 0usize;
    for c in item.items().iter().skip(1) {
        match c.head() {
            Some("pad") => {
                let p = pad(cx, c, angle, &fp.reference)?;
                fp.pads.push(p);
            }
            Some(h @ ("fp_line" | "fp_circle" | "fp_rect" | "fp_poly")) => {
                footprint_graphic(cx, h, c, &mut fp)?
            }
            Some("fp_arc") => arcs += 1,
            Some("fp_text") => {
                let kind = c.args().first().and_then(Node::text).unwrap_or("user");
                let text = match kind {
                    "reference" => Some("${REF}".to_string()),
                    "value" => Some("${VALUE}".to_string()),
                    _ => c.args().get(1).and_then(Node::text).map(str::to_string),
                };
                if let Some(mut t) = text_item(cx, c, text)? {
                    t.rotation = mdeg(f64::from(t.rotation) / 1000.0 - angle);
                    fp.texts.push(t);
                }
            }
            _ => {}
        }
    }
    if arcs > 0 {
        cx.warn(format!(
            "footprint {}: {arcs} arc(s) in its drawing were not imported",
            label(&fp.reference, &fp.package)
        ));
    }
    Ok(Some(fp))
}

fn label(reference: &str, package: &str) -> String {
    if reference.is_empty() {
        format!("`{package}`")
    } else {
        reference.to_string()
    }
}

fn pad(cx: &mut Ctx, item: &Node, fp_angle: f64, owner: &str) -> Result<Pad> {
    let args = item.args();
    let number = args.first().and_then(Node::text).unwrap_or("").to_string();
    let kind = match args.get(1).and_then(Node::text) {
        Some("smd") | Some("connect") => PadKind::Smd,
        Some("thru_hole") => PadKind::ThroughHole,
        Some("np_thru_hole") => PadKind::Npth,
        other => {
            return Err(invalid(
                "pad",
                format!("{owner}: unknown pad type `{}`", other.unwrap_or("")),
            ))
        }
    };
    let shape_name = args.get(2).and_then(Node::text).unwrap_or("rect");
    let (position, pad_angle) = at(item, "pad")?;
    let size_node = item
        .child("size")
        .ok_or_else(|| invalid("pad", format!("{owner}: pad {number} has no size")))?;
    let (w, h) = match (num(size_node.args().first()), num(size_node.args().get(1))) {
        (Some(w), Some(h)) => (nm(w, "pad size")?, nm(h, "pad size")?),
        _ => {
            return Err(invalid(
                "pad",
                format!("{owner}: pad {number} has a bad size"),
            ))
        }
    };
    let (shape, size) = match shape_name {
        "circle" => (PadShape::Circle, Size { w, h: w }),
        "rect" => (PadShape::Rect, Size { w, h }),
        "oval" => (PadShape::Oval, Size { w, h }),
        "roundrect" => {
            let ratio = item
                .arg_f64("roundrect_rratio")
                .unwrap_or(0.25)
                .clamp(0.0, 0.5);
            (
                PadShape::RoundRect {
                    corner_radius: (w.min(h) as f64 * ratio).round() as Nm,
                },
                Size { w, h },
            )
        }
        other => {
            cx.warn(format!(
                "{owner}: pad {number} has shape `{other}`, imported as a rectangle"
            ));
            (PadShape::Rect, Size { w, h })
        }
    };
    let drill = match item.child("drill") {
        None => None,
        Some(d) => {
            let a = d.args();
            let (dia, slot) = if a.first().and_then(Node::text) == Some("oval") {
                let (x, y) = (
                    nm(num(a.get(1)).unwrap_or(0.0), "drill")?,
                    nm(num(a.get(2)).unwrap_or(0.0), "drill")?,
                );
                (x.min(y), Some(x.max(y)))
            } else {
                (nm(num(a.first()).unwrap_or(0.0), "drill")?, None)
            };
            if d.child("offset").is_some() {
                cx.warn(format!(
                    "{owner}: pad {number} has a drill offset, which is not imported"
                ));
            }
            (dia > 0).then_some(Drill {
                diameter: dia,
                slot_length: slot,
            })
        }
    };
    let layers: Vec<&str> = item
        .child("layers")
        .map(|l| l.args().iter().filter_map(Node::text).collect())
        .unwrap_or_default();
    let has = |names: &[&str]| layers.iter().any(|l| names.contains(l));
    let net = item.child("net").and_then(|n| cx.net_of(n));
    let id = cx.id();
    Ok(Pad {
        id,
        number,
        kind,
        shape,
        position,
        // A board file stores the pad's angle with the footprint's included.
        rotation: mdeg(pad_angle - fp_angle),
        size,
        drill,
        net,
        mask: has(&["F.Mask", "*.Mask"]),
        paste: has(&["F.Paste", "*.Paste"]),
        mask_margin: item
            .arg_f64("solder_mask_margin")
            .map(|v| nm(v, "mask margin"))
            .transpose()?,
        paste_margin: item
            .arg_f64("solder_paste_margin")
            .map(|v| nm(v, "paste margin"))
            .transpose()?,
    })
}

/// A footprint's graphic in the footprint's own frame: positions are relative, Y is negated.
fn footprint_graphic(cx: &mut Ctx, head: &str, c: &Node, fp: &mut Footprint) -> Result<()> {
    let Some(layer) = c.arg_text("layer").and_then(|l| cx.layer(l)) else {
        return Ok(());
    };
    let width = nm(width_of(c), "line width")?;
    let filled = c
        .child("fill")
        .and_then(|f| f.args().first())
        .and_then(Node::text)
        .is_some_and(|f| f == "solid" || f == "yes");
    let mut kinds: Vec<GraphicKind> = Vec::new();
    match head {
        "fp_line" => {
            if let (Some(a), Some(b)) = (xy(c, "start", "fp_line")?, xy(c, "end", "fp_line")?) {
                if a != b {
                    kinds.push(GraphicKind::Line { a, b });
                }
            }
        }
        "fp_circle" => {
            if let (Some(center), Some(edge)) =
                (xy(c, "center", "fp_circle")?, xy(c, "end", "fp_circle")?)
            {
                let r =
                    (((edge.x - center.x) as f64).hypot((edge.y - center.y) as f64)).round() as Nm;
                if r > 0 {
                    kinds.push(GraphicKind::Circle { center, radius: r });
                }
            }
        }
        "fp_rect" => {
            if let (Some(a), Some(b)) = (xy(c, "start", "fp_rect")?, xy(c, "end", "fp_rect")?) {
                let corners = [a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)];
                if matches!(layer, Layer::FCourtyard | Layer::BCourtyard) {
                    if let Ok(r) = Region::polygon(corners.to_vec()) {
                        fp.courtyard = Some(r);
                        return Ok(());
                    }
                }
                for i in 0..4 {
                    let (p, q) = (corners[i], corners[(i + 1) % 4]);
                    if p != q {
                        kinds.push(GraphicKind::Line { a: p, b: q });
                    }
                }
            }
        }
        _ => {
            if let Some(pts) = polygon_points(c, "fp_poly")? {
                match Region::polygon(pts) {
                    Ok(r) => kinds.push(GraphicKind::Polygon(r)),
                    Err(e) => cx.warn(format!("a footprint polygon was skipped: {e}")),
                }
            }
        }
    }
    for kind in kinds {
        let id = cx.id();
        fp.graphics.push(Graphic {
            id,
            layer,
            kind,
            width,
            filled,
        });
    }
    Ok(())
}

fn polygon_points(node: &Node, what: &str) -> Result<Option<Vec<Point>>> {
    let Some(pts) = node.child("pts") else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for p in pts.children("xy") {
        if let (Some(x), Some(y)) = (num(p.args().first()), num(p.args().get(1))) {
            out.push(point(x, y, what)?);
        }
    }
    Ok((out.len() >= 3).then_some(out))
}

fn text_item(cx: &mut Ctx, item: &Node, text: Option<String>) -> Result<Option<TextItem>> {
    let text = match text {
        Some(t) => t,
        None => match item.args().first().and_then(Node::text) {
            Some(t) => t.to_string(),
            None => return Ok(None),
        },
    };
    let Some(layer) = item.arg_text("layer").and_then(|l| cx.layer(l)) else {
        return Ok(None);
    };
    let (position, angle) = at(item, "text")?;
    let font = item.child("effects").and_then(|e| e.child("font"));
    let size = font.and_then(|f| f.child("size"));
    let height = nm(
        num(size.and_then(|s| s.args().first())).unwrap_or(1.0),
        "text height",
    )?;
    let thickness = nm(
        font.and_then(|f| f.arg_f64("thickness")).unwrap_or(0.15),
        "text thickness",
    )?;
    let justify: Vec<&str> = item
        .child("effects")
        .and_then(|e| e.child("justify"))
        .map(|j| j.args().iter().filter_map(Node::text).collect())
        .unwrap_or_default();
    let justify = if justify.contains(&"left") {
        Justify::Left
    } else if justify.contains(&"right") {
        Justify::Right
    } else {
        Justify::Center
    };
    if height <= 0 || thickness <= 0 {
        return Ok(None);
    }
    let id = cx.id();
    Ok(Some(TextItem {
        id,
        layer,
        text,
        position,
        height,
        thickness,
        rotation: mdeg(angle),
        mirrored: layer.side() == Some(Side::Back),
        justify,
    }))
}

fn board_graphic(
    cx: &mut Ctx,
    item: &Node,
    board: &mut Board,
    edge: &mut Vec<OutlineSeg>,
) -> Result<()> {
    let head = item.head().unwrap_or("");
    let Some(layer) = item.arg_text("layer").and_then(|l| cx.layer(l)) else {
        return Ok(());
    };
    let width = nm(width_of(item), "line width")?;
    let filled = item
        .child("fill")
        .and_then(|f| f.args().first())
        .and_then(Node::text)
        .is_some_and(|f| f == "solid" || f == "yes");
    let mut kinds: Vec<GraphicKind> = Vec::new();
    match head {
        "gr_line" => {
            if let (Some(a), Some(b)) = (xy(item, "start", head)?, xy(item, "end", head)?) {
                if a != b {
                    kinds.push(GraphicKind::Line { a, b });
                }
            }
        }
        "gr_arc" => {
            if let (Some(s), Some(m), Some(e)) = (
                xy(item, "start", head)?,
                xy(item, "mid", head)?,
                xy(item, "end", head)?,
            ) {
                match Arc::through(s, m, e) {
                    Ok(a) => kinds.push(GraphicKind::Arc(a)),
                    Err(err) => cx.warn(format!("a board arc was skipped: {err}")),
                }
            } else {
                cx.warn("a board arc in an older syntax (start, end, angle) was skipped".into());
            }
        }
        "gr_circle" => {
            if let (Some(c), Some(e)) = (xy(item, "center", head)?, xy(item, "end", head)?) {
                let r = (((e.x - c.x) as f64).hypot((e.y - c.y) as f64)).round() as Nm;
                if r > 0 {
                    kinds.push(GraphicKind::Circle {
                        center: c,
                        radius: r,
                    });
                }
            }
        }
        "gr_rect" => {
            if let (Some(a), Some(b)) = (xy(item, "start", head)?, xy(item, "end", head)?) {
                let corners = [a, Point::new(b.x, a.y), b, Point::new(a.x, b.y)];
                for i in 0..4 {
                    let (p, q) = (corners[i], corners[(i + 1) % 4]);
                    if p != q {
                        kinds.push(GraphicKind::Line { a: p, b: q });
                    }
                }
            }
        }
        "gr_poly" => {
            if let Some(pts) = polygon_points(item, head)? {
                match Region::polygon(pts) {
                    Ok(r) => kinds.push(GraphicKind::Polygon(r)),
                    Err(e) => cx.warn(format!("a board polygon was skipped: {e}")),
                }
            }
        }
        _ => cx.warn(format!("a `{head}` was not imported")),
    }
    for kind in kinds {
        if layer == Layer::EdgeCuts {
            match kind {
                GraphicKind::Line { a, b } => edge.push(OutlineSeg::Line { start: a, end: b }),
                GraphicKind::Arc(a) => edge.push(OutlineSeg::Arc(a)),
                GraphicKind::Circle { center, radius } => {
                    let start = Point::new(center.x + radius, center.y);
                    match Arc::new(start, start, center, true) {
                        Ok(a) => edge.push(OutlineSeg::Arc(a)),
                        Err(e) => cx.warn(format!("a circle on Edge.Cuts was skipped: {e}")),
                    }
                }
                GraphicKind::Polygon(r) => {
                    let n = r.outer.len();
                    for i in 0..n {
                        let (Some(a), Some(b)) = (r.outer.get(i), r.outer.get((i + 1) % n)) else {
                            continue;
                        };
                        edge.push(OutlineSeg::Line { start: *a, end: *b });
                    }
                }
            }
        } else {
            let id = cx.id();
            board.graphics.push(Graphic {
                id,
                layer,
                kind,
                width,
                filled,
            });
        }
    }
    Ok(())
}

fn zone(cx: &mut Ctx, item: &Node, board: &mut Board) -> Result<()> {
    let net = item.child("net").and_then(|n| cx.net_of(n));
    let mut layers: Vec<Layer> = Vec::new();
    if let Some(l) = item.arg_text("layer").and_then(|l| cx.layer(l)) {
        layers.push(l);
    }
    if let Some(ls) = item.child("layers") {
        for l in ls.args().iter().filter_map(Node::text) {
            if l == "F&B.Cu" {
                layers.extend([Layer::FCu, Layer::BCu]);
            } else if l == "*.Cu" {
                layers.extend(board.copper_stack());
            } else if let Some(l) = cx.layer(l) {
                layers.push(l);
            }
        }
    }
    let mut unique: Vec<Layer> = Vec::new();
    for l in layers {
        if !unique.contains(&l) {
            unique.push(l);
        }
    }
    let layers = unique;
    let Some(pts) = item
        .child("polygon")
        .map(|p| polygon_points(p, "zone"))
        .transpose()?
        .flatten()
    else {
        cx.warn("a zone without a usable outline was skipped".into());
        return Ok(());
    };
    let outline = match Region::polygon(pts) {
        Ok(r) => r,
        Err(e) => {
            cx.warn(format!("a zone outline was skipped: {e}"));
            return Ok(());
        }
    };
    if let Some(k) = item.child("keepout") {
        let not_allowed = |name: &str| k.arg_text(name) == Some("not_allowed");
        let id = cx.id();
        board.keepouts.push(Keepout {
            id,
            layers: layers.into_iter().filter(|l| l.is_copper()).collect(),
            outline,
            no_tracks: not_allowed("tracks"),
            no_vias: not_allowed("vias"),
            no_pads: not_allowed("pads"),
            no_zones: not_allowed("copperpour"),
        });
        return Ok(());
    }
    let connect = item.child("connect_pads");
    let mode = match connect.and_then(|c| c.args().first()).and_then(Node::text) {
        Some("yes") => ThermalMode::Solid,
        Some("no") => ThermalMode::None,
        _ => ThermalMode::Relief,
    };
    let fill = item.child("fill");
    let thermal = Thermal {
        mode,
        gap: nm(
            fill.and_then(|f| f.arg_f64("thermal_gap")).unwrap_or(0.5),
            "thermal gap",
        )?,
        spoke_width: nm(
            fill.and_then(|f| f.arg_f64("thermal_bridge_width"))
                .unwrap_or(0.5),
            "thermal spoke",
        )?,
    };
    let clearance = nm(
        connect.and_then(|c| c.arg_f64("clearance")).unwrap_or(0.5),
        "zone clearance",
    )?;
    let min_width = nm(
        item.arg_f64("min_thickness").unwrap_or(0.25),
        "zone minimum width",
    )?;
    let priority = item
        .arg_text("priority")
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    let mut skipped_fills = 0usize;
    for layer in layers.into_iter().filter(|l| l.is_copper()) {
        let mut filled = Vec::new();
        for f in item.children("filled_polygon") {
            if f.arg_text("layer").and_then(layer_named) != Some(layer) {
                continue;
            }
            match polygon_points(f, "filled polygon")
                .ok()
                .flatten()
                .map(Region::polygon)
            {
                Some(Ok(r)) => filled.push(r),
                _ => skipped_fills += 1,
            }
        }
        let id = cx.id();
        board.zones.push(Zone {
            id,
            net,
            layer,
            outline: outline.clone(),
            clearance,
            min_width,
            thermal,
            priority,
            filled,
        });
    }
    if skipped_fills > 0 {
        cx.warn(format!(
            "{skipped_fills} stored zone fill polygon(s) could not be imported; refill the zone"
        ));
    }
    Ok(())
}

fn dist(a: Point, b: Point) -> f64 {
    ((a.x - b.x) as f64).hypot((a.y - b.y) as f64)
}

fn set_start(seg: &mut OutlineSeg, p: Point) -> bool {
    match seg {
        OutlineSeg::Line { start, .. } => {
            *start = p;
            true
        }
        OutlineSeg::Arc(a) => a.start == p,
    }
}

fn set_end(seg: &mut OutlineSeg, p: Point) -> bool {
    match seg {
        OutlineSeg::Line { end, .. } => {
            *end = p;
            true
        }
        OutlineSeg::Arc(a) => a.end == p,
    }
}

fn reversed(seg: &OutlineSeg) -> OutlineSeg {
    match seg {
        OutlineSeg::Line { start, end } => OutlineSeg::Line {
            start: *end,
            end: *start,
        },
        OutlineSeg::Arc(a) => OutlineSeg::Arc(Arc {
            start: a.end,
            end: a.start,
            center: a.center,
            ccw: !a.ccw,
        }),
    }
}

/// Chain the `Edge.Cuts` pieces into closed loops: the largest is the outline, the rest cut-outs.
fn outline(cx: &mut Ctx, mut pieces: Vec<OutlineSeg>) -> Option<Outline> {
    let mut loops: Vec<Vec<OutlineSeg>> = Vec::new();
    while !pieces.is_empty() {
        let mut chain = vec![pieces.remove(0)];
        loop {
            let (first, last) = (chain.first()?.start(), chain.last()?.end());
            if first == last || (chain.len() > 1 && dist(first, last) <= JOIN_TOLERANCE) {
                break;
            }
            let found = pieces.iter().position(|s| {
                dist(s.start(), last) <= JOIN_TOLERANCE || dist(s.end(), last) <= JOIN_TOLERANCE
            });
            let Some(i) = found else { break };
            let mut next = pieces.remove(i);
            if dist(next.start(), last) > JOIN_TOLERANCE {
                next = reversed(&next);
            }
            if next.start() != last && !set_start(&mut next, last) {
                if let Some(prev) = chain.last_mut() {
                    if !set_end(prev, next.start()) {
                        cx.warn("two arcs on Edge.Cuts meet with a small gap that cannot be closed exactly".into());
                        return None;
                    }
                }
            }
            chain.push(next);
        }
        let (first, last) = (chain.first()?.start(), chain.last()?.end());
        if first != last {
            if dist(first, last) <= JOIN_TOLERANCE {
                let fixed = chain.last_mut().is_some_and(|l| set_end(l, first))
                    || chain.first_mut().is_some_and(|f| set_start(f, last));
                if !fixed {
                    cx.warn("the board outline has a gap between two arcs".into());
                    return None;
                }
            } else {
                cx.warn(format!("the board outline is not closed: a chain of {} edge(s) ends {:.3} mm from where it starts", chain.len(), dist(first, last) / 1e6));
                return None;
            }
        }
        loops.push(chain);
    }
    let area = |l: &Vec<OutlineSeg>| -> i128 {
        let pts: Vec<Point> = l.iter().map(OutlineSeg::start).collect();
        ring_area2(&pts).abs()
    };
    let outer_at = loops
        .iter()
        .enumerate()
        .max_by_key(|(_, l)| area(l))
        .map(|(i, _)| i)?;
    let outer = loops.remove(outer_at);
    Some(Outline {
        outer,
        cutouts: loops,
    })
}

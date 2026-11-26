// SPDX-License-Identifier: MIT OR Apache-2.0
//! The board model: nets, footprints, pads, tracks, vias, zones, outline, text and keepouts.
//!
//! Lengths are integer nanometres, the Y axis points up. Angles are millidegrees (`i32`, counter-
//! clockwise); rotations by multiples of 90 000 are applied exactly, other angles round every
//! transformed vertex to the nearest nm. Every object has a board-unique [`Id`] that never changes.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::{PcbError, Result};
use crate::geom::arc::Arc;
use crate::geom::point::Point;
use crate::geom::region::{ring_area2, ring_is_simple, Region, MAX_RING_VERTS};
use crate::layer::{Layer, Side, MAX_COPPER_LAYERS};
use crate::stackup::Stackup;
use crate::units::{check_coord, check_dim, check_positive, Nm, MAX_DIM};

/// Most objects of each kind accepted in one board.
pub const MAX_OBJECTS: usize = 1_000_000;
/// Most pads in one footprint.
pub const MAX_PADS_PER_FOOTPRINT: usize = 20_000;
/// Longest accepted name, reference, value or text, in bytes.
pub const MAX_STRING: usize = 512;

/// Stable identifier of any board object (unique across the whole board).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Id(pub u64);

/// Identifier of a net.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NetId(pub u32);

/// A net: a name (from the schematic) and the name of its class.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Net {
    pub id: NetId,
    pub name: String,
    #[serde(default = "default_class_name")]
    pub class: String,
}

pub const DEFAULT_CLASS: &str = "Default";
fn default_class_name() -> String {
    DEFAULT_CLASS.to_string()
}

/// Rules and routing defaults attached to nets. Unset fields fall back to the board [`crate::drc::Rules`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct NetClass {
    pub name: String,
    /// Minimum copper clearance for nets of this class (applied as a floor to every pair involving them).
    #[serde(default)]
    pub clearance: Option<Nm>,
    /// Preferred routing width (not a check).
    #[serde(default)]
    pub track_width: Option<Nm>,
    /// Minimum track width enforced by DRC.
    #[serde(default)]
    pub min_track_width: Option<Nm>,
    /// Preferred via pad diameter (not a check).
    #[serde(default)]
    pub via_diameter: Option<Nm>,
    /// Preferred via drill (not a check).
    #[serde(default)]
    pub via_drill: Option<Nm>,
    /// Minimum via pad diameter enforced by DRC.
    #[serde(default)]
    pub min_via_diameter: Option<Nm>,
    /// Minimum via drill enforced by DRC.
    #[serde(default)]
    pub min_via_drill: Option<Nm>,
    /// Differential pair routing parameters. Stored for the router and the app; DRC does not yet
    /// check them.
    #[serde(default)]
    pub diff_pair: Option<DiffPair>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffPair {
    pub width: Nm,
    pub gap: Nm,
}

/// Pad construction kind.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadKind {
    /// Surface-mount: copper on the footprint's side only.
    Smd,
    /// Plated through hole: copper on every copper layer.
    ThroughHole,
    /// Non-plated hole: no copper.
    Npth,
}

/// Pad outline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PadShape {
    /// Diameter is `size.w` (`size.w == size.h` is required).
    Circle,
    Rect,
    /// Stadium; the longer axis is along the pad's x axis when `w > h`.
    Oval,
    /// Rectangle with corner radius `corner_radius` (at most half the shorter side).
    RoundRect {
        corner_radius: Nm,
    },
    /// Polygon in the pad's own frame (relative to `position`, before the pad rotation).
    Custom {
        polygon: Vec<Point>,
    },
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub w: Nm,
    pub h: Nm,
}

/// A drilled hole. When `slot_length` is set the hole is a slot: a stadium of width `diameter` and
/// overall length `slot_length` along the pad's x axis.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drill {
    pub diameter: Nm,
    #[serde(default)]
    pub slot_length: Option<Nm>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pad {
    pub id: Id,
    /// Pad number or name ("1", "A3", "EP"). Pads of one footprint sharing a number are one
    /// electrical terminal.
    pub number: String,
    pub kind: PadKind,
    pub shape: PadShape,
    /// Centre in the footprint's frame.
    pub position: Point,
    /// Rotation in the footprint's frame, millidegrees counter-clockwise.
    #[serde(default)]
    pub rotation: i32,
    #[serde(default = "zero_size")]
    pub size: Size,
    #[serde(default)]
    pub drill: Option<Drill>,
    #[serde(default)]
    pub net: Option<NetId>,
    /// Whether the solder mask is opened over this pad.
    #[serde(default = "yes")]
    pub mask: bool,
    /// Whether solder paste is applied (SMD pads).
    #[serde(default)]
    pub paste: bool,
    /// Mask expansion; the board default is used when `None`.
    #[serde(default)]
    pub mask_margin: Option<Nm>,
    /// Paste expansion (negative shrinks); 0 when `None`.
    #[serde(default)]
    pub paste_margin: Option<Nm>,
}

fn yes() -> bool {
    true
}
fn zero_size() -> Size {
    Size { w: 0, h: 0 }
}

/// Footprint graphic primitive (footprint-local coordinates when inside a footprint).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphicKind {
    Line {
        a: Point,
        b: Point,
    },
    Circle {
        center: Point,
        radius: Nm,
    },
    /// Board-level graphics only.
    Arc(Arc),
    Polygon(Region),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graphic {
    pub id: Id,
    pub layer: Layer,
    #[serde(flatten)]
    pub kind: GraphicKind,
    /// Stroke width (outline) in nm; 0 only for filled shapes.
    pub width: Nm,
    #[serde(default)]
    pub filled: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Justify {
    #[default]
    Left,
    Center,
    Right,
}

/// Text drawn with the built-in stroke font. In footprints `${REF}` and `${VALUE}` expand to the
/// footprint's reference and value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextItem {
    pub id: Id,
    pub layer: Layer,
    pub text: String,
    pub position: Point,
    /// Cap height in nm.
    pub height: Nm,
    /// Stroke thickness in nm.
    pub thickness: Nm,
    #[serde(default)]
    pub rotation: i32,
    #[serde(default)]
    pub mirrored: bool,
    #[serde(default)]
    pub justify: Justify,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Footprint {
    pub id: Id,
    /// Reference designator ("R12").
    pub reference: String,
    pub value: String,
    /// Package / footprint name used for the BOM ("R_0603").
    pub package: String,
    pub position: Point,
    #[serde(default)]
    pub rotation: i32,
    pub side: Side,
    pub pads: Vec<Pad>,
    /// Courtyard in the footprint's frame.
    #[serde(default)]
    pub courtyard: Option<Region>,
    #[serde(default)]
    pub graphics: Vec<Graphic>,
    #[serde(default)]
    pub texts: Vec<TextItem>,
    #[serde(default)]
    pub exclude_from_bom: bool,
    #[serde(default)]
    pub exclude_from_pos: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "geometry", rename_all = "snake_case")]
pub enum TrackGeom {
    Segment { start: Point, end: Point },
    Arc(Arc),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    #[serde(default)]
    pub net: Option<NetId>,
    pub layer: Layer,
    pub width: Nm,
    #[serde(flatten)]
    pub geom: TrackGeom,
}

impl Track {
    pub fn endpoints(&self) -> (Point, Point) {
        match &self.geom {
            TrackGeom::Segment { start, end } => (*start, *end),
            TrackGeom::Arc(a) => (a.start, a.end),
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViaKind {
    /// `F.Cu` to `B.Cu`.
    Through,
    /// An outer layer to an inner layer.
    Blind,
    /// Between inner layers.
    Buried,
    /// Laser via between an outer layer and the adjacent inner layer.
    Micro,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Via {
    pub id: Id,
    #[serde(default)]
    pub net: Option<NetId>,
    pub position: Point,
    pub diameter: Nm,
    pub drill: Nm,
    pub kind: ViaKind,
    pub from: Layer,
    pub to: Layer,
    /// Covered by solder mask (no mask opening).
    #[serde(default = "yes")]
    pub tented: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThermalMode {
    /// Pads of the zone's net connect directly.
    Solid,
    /// Pads connect through spokes.
    #[default]
    Relief,
    /// Pads do not connect to the zone.
    None,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thermal {
    pub mode: ThermalMode,
    pub gap: Nm,
    pub spoke_width: Nm,
}

impl Default for Thermal {
    fn default() -> Self {
        Thermal {
            mode: ThermalMode::Relief,
            gap: 500_000,
            spoke_width: 500_000,
        }
    }
}

/// A copper zone. `filled` holds the copper that remains after clearing (computed by a fill
/// engine or supplied); DRC, connectivity and Gerber output use `filled`. This crate does not
/// compute fills.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zone {
    pub id: Id,
    #[serde(default)]
    pub net: Option<NetId>,
    pub layer: Layer,
    pub outline: Region,
    pub clearance: Nm,
    pub min_width: Nm,
    #[serde(default)]
    pub thermal: Thermal,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub filled: Vec<Region>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keepout {
    pub id: Id,
    /// Copper layers the keepout applies to; empty means all.
    #[serde(default)]
    pub layers: Vec<Layer>,
    pub outline: Region,
    #[serde(default)]
    pub no_tracks: bool,
    #[serde(default)]
    pub no_vias: bool,
    #[serde(default)]
    pub no_pads: bool,
    #[serde(default)]
    pub no_zones: bool,
}

/// One edge of the board outline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutlineSeg {
    Line { start: Point, end: Point },
    Arc(Arc),
}

impl OutlineSeg {
    pub fn start(&self) -> Point {
        match self {
            OutlineSeg::Line { start, .. } => *start,
            OutlineSeg::Arc(a) => a.start,
        }
    }
    pub fn end(&self) -> Point {
        match self {
            OutlineSeg::Line { end, .. } => *end,
            OutlineSeg::Arc(a) => a.end,
        }
    }
}

/// The board edge: one closed outer loop and any number of closed cut-out loops.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outline {
    pub outer: Vec<OutlineSeg>,
    #[serde(default)]
    pub cutouts: Vec<Vec<OutlineSeg>>,
}

impl Outline {
    /// A rectangle `[x0,x1] x [y0,y1]`, counter-clockwise.
    pub fn rectangle(x0: Nm, y0: Nm, x1: Nm, y1: Nm) -> Outline {
        let p = [
            Point::new(x0, y0),
            Point::new(x1, y0),
            Point::new(x1, y1),
            Point::new(x0, y1),
        ];
        Outline {
            outer: (0..4)
                .map(|i| OutlineSeg::Line {
                    start: p[i],
                    end: p[(i + 1) % 4],
                })
                .collect(),
            cutouts: vec![],
        }
    }

    /// Approximates the outline as a [`Region`] (arcs become polylines within `tolerance` nm).
    pub fn to_region(&self, tolerance: Nm) -> Result<Region> {
        let outer = loop_points(&self.outer, tolerance)?;
        let holes = self
            .cutouts
            .iter()
            .map(|l| loop_points(l, tolerance))
            .collect::<Result<Vec<_>>>()?;
        Region::new(outer, holes)
    }

    fn validate(&self) -> Result<()> {
        check_loop(&self.outer, "board outline")?;
        for (i, c) in self.cutouts.iter().enumerate() {
            check_loop(c, &format!("board cut-out {i}"))?;
        }
        // The approximation is a simple region with holes inside the outer loop.
        self.to_region(crate::geom::arc::DEFAULT_ARC_TOLERANCE)
            .map(|_| ())
    }
}

fn check_loop(segs: &[OutlineSeg], what: &str) -> Result<()> {
    if segs.is_empty() {
        return Err(PcbError::geometry(what, "has no edges"));
    }
    if segs.len() > MAX_RING_VERTS {
        return Err(PcbError::range(
            what,
            format!("{} edges exceed the limit of {MAX_RING_VERTS}", segs.len()),
        ));
    }
    for (i, s) in segs.iter().enumerate() {
        match s {
            OutlineSeg::Line { start, end } => {
                for p in [start, end] {
                    check_coord(p.x, what)?;
                    check_coord(p.y, what)?;
                }
                if start == end {
                    return Err(PcbError::geometry(
                        what,
                        format!("edge {i} has zero length"),
                    ));
                }
            }
            OutlineSeg::Arc(a) => a.validate(what)?,
        }
        let next = &segs[(i + 1) % segs.len()];
        if s.end() != next.start() {
            let e = s.end();
            let n = next.start();
            return Err(PcbError::geometry(
                what,
                format!(
                    "is not closed: edge {i} ends at ({}, {}) but edge {} starts at ({}, {})",
                    e.x,
                    e.y,
                    (i + 1) % segs.len(),
                    n.x,
                    n.y
                ),
            ));
        }
    }
    Ok(())
}

fn loop_points(segs: &[OutlineSeg], tolerance: Nm) -> Result<Vec<Point>> {
    let mut pts = Vec::new();
    for s in segs {
        match s {
            OutlineSeg::Line { start, .. } => pts.push(*start),
            OutlineSeg::Arc(a) => {
                let poly = a.approximate(tolerance)?;
                // full-circle arcs have start == end; keep all interior vertices
                pts.extend(poly.iter().take(poly.len().saturating_sub(1)).copied());
            }
        }
        if pts.len() > MAX_RING_VERTS {
            return Err(PcbError::range(
                "board outline",
                "approximation has too many vertices",
            ));
        }
    }
    Ok(pts)
}

/// A complete board.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Board {
    pub name: String,
    /// Number of copper layers, 1 to 32.
    pub copper_layers: u8,
    pub stackup: Stackup,
    #[serde(default)]
    pub outline: Option<Outline>,
    #[serde(default)]
    pub nets: Vec<Net>,
    #[serde(default)]
    pub net_classes: Vec<NetClass>,
    #[serde(default)]
    pub footprints: Vec<Footprint>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub vias: Vec<Via>,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub keepouts: Vec<Keepout>,
    #[serde(default)]
    pub graphics: Vec<Graphic>,
    #[serde(default)]
    pub texts: Vec<TextItem>,
}

fn check_text(s: &str, what: &str) -> Result<()> {
    if s.len() > MAX_STRING {
        return Err(PcbError::range(
            what,
            format!("{} bytes exceed the limit of {MAX_STRING}", s.len()),
        ));
    }
    if s.chars().any(char::is_control) {
        return Err(PcbError::invalid(format!(
            "{what} contains a control character"
        )));
    }
    Ok(())
}

impl Board {
    /// An empty board with a standard stackup and no outline.
    pub fn new(name: &str, copper_layers: u8) -> Result<Board> {
        if !(1..=MAX_COPPER_LAYERS).contains(&copper_layers) {
            return Err(PcbError::range(
                "copper layer count",
                format!("{copper_layers} is outside 1..={MAX_COPPER_LAYERS}"),
            ));
        }
        Ok(Board {
            name: name.to_string(),
            copper_layers,
            stackup: Stackup::standard(copper_layers),
            outline: None,
            nets: vec![],
            net_classes: vec![],
            footprints: vec![],
            tracks: vec![],
            vias: vec![],
            zones: vec![],
            keepouts: vec![],
            graphics: vec![],
            texts: vec![],
        })
    }

    /// The copper layers of this board, top to bottom.
    pub fn copper_stack(&self) -> Vec<Layer> {
        Layer::copper_stack(self.copper_layers)
    }

    /// Smallest unused object id.
    pub fn next_id(&self) -> Id {
        let mut m = 0u64;
        self.for_each_id(|id| m = m.max(id.0));
        Id(m.saturating_add(1))
    }

    fn for_each_id(&self, mut f: impl FnMut(Id)) {
        for fp in &self.footprints {
            f(fp.id);
            fp.pads.iter().for_each(|p| f(p.id));
            fp.graphics.iter().for_each(|g| f(g.id));
            fp.texts.iter().for_each(|t| f(t.id));
        }
        self.tracks.iter().for_each(|t| f(t.id));
        self.vias.iter().for_each(|v| f(v.id));
        self.zones.iter().for_each(|z| f(z.id));
        self.keepouts.iter().for_each(|k| f(k.id));
        self.graphics.iter().for_each(|g| f(g.id));
        self.texts.iter().for_each(|t| f(t.id));
    }

    pub fn net(&self, id: NetId) -> Option<&Net> {
        self.nets.iter().find(|n| n.id == id)
    }

    pub fn net_name(&self, id: Option<NetId>) -> String {
        match id.and_then(|i| self.net(i)) {
            Some(n) => n.name.clone(),
            None => "(no net)".to_string(),
        }
    }

    pub fn net_class(&self, net: Option<NetId>) -> Option<&NetClass> {
        let n = self.net(net?)?;
        self.net_classes.iter().find(|c| c.name == n.class)
    }

    pub fn footprint(&self, id: Id) -> Option<&Footprint> {
        self.footprints.iter().find(|f| f.id == id)
    }

    /// Validates everything: limits, coordinate ranges, id uniqueness, references, layers, geometry.
    /// Every other entry point of the crate calls this before working on a board.
    pub fn validate(&self) -> Result<()> {
        check_text(&self.name, "board name")?;
        if !(1..=MAX_COPPER_LAYERS).contains(&self.copper_layers) {
            return Err(PcbError::range(
                "copper layer count",
                format!("{} is outside 1..={MAX_COPPER_LAYERS}", self.copper_layers),
            ));
        }
        self.stackup.validate(self.copper_layers)?;
        for (what, n) in [
            ("nets", self.nets.len()),
            ("net classes", self.net_classes.len()),
            ("footprints", self.footprints.len()),
            ("tracks", self.tracks.len()),
            ("vias", self.vias.len()),
            ("zones", self.zones.len()),
            ("keepouts", self.keepouts.len()),
            ("graphics", self.graphics.len()),
            ("texts", self.texts.len()),
        ] {
            if n > MAX_OBJECTS {
                return Err(PcbError::TooLarge {
                    what: "board",
                    detail: format!("{n} {what} exceed the limit of {MAX_OBJECTS}"),
                });
            }
        }
        if let Some(o) = &self.outline {
            o.validate()?;
        }

        // nets and classes
        let mut net_ids = HashSet::new();
        let mut net_names = HashSet::new();
        let class_names: HashSet<&str> = self.net_classes.iter().map(|c| c.name.as_str()).collect();
        if class_names.len() != self.net_classes.len() {
            return Err(PcbError::invalid("two net classes share a name"));
        }
        for c in &self.net_classes {
            check_text(&c.name, "net class name")?;
            if c.name.is_empty() {
                return Err(PcbError::invalid("a net class has an empty name"));
            }
            for (what, v) in [
                ("clearance", c.clearance),
                ("track width", c.track_width),
                ("minimum track width", c.min_track_width),
                ("via diameter", c.via_diameter),
                ("via drill", c.via_drill),
                ("minimum via diameter", c.min_via_diameter),
                ("minimum via drill", c.min_via_drill),
            ] {
                if let Some(v) = v {
                    check_dim(v, &format!("net class {} {what}", c.name))?;
                }
            }
            if let Some(d) = c.diff_pair {
                check_positive(d.width, "differential pair width")?;
                check_positive(d.gap, "differential pair gap")?;
            }
        }
        for n in &self.nets {
            check_text(&n.name, "net name")?;
            if n.name.is_empty() {
                return Err(PcbError::invalid(format!(
                    "net {} has an empty name",
                    n.id.0
                )));
            }
            if !net_ids.insert(n.id) {
                return Err(PcbError::invalid(format!(
                    "net id {} is used twice",
                    n.id.0
                )));
            }
            if !net_names.insert(n.name.as_str()) {
                return Err(PcbError::invalid(format!(
                    "net name `{}` is used twice",
                    n.name
                )));
            }
            if n.class != DEFAULT_CLASS && !class_names.contains(n.class.as_str()) {
                return Err(PcbError::invalid(format!(
                    "net `{}` uses unknown net class `{}`",
                    n.name, n.class
                )));
            }
        }
        let check_net = |net: Option<NetId>, what: &str| -> Result<()> {
            match net {
                Some(n) if !net_ids.contains(&n) => Err(PcbError::invalid(format!(
                    "{what} refers to unknown net id {}",
                    n.0
                ))),
                _ => Ok(()),
            }
        };

        let mut ids: HashSet<Id> = HashSet::new();
        let mut claim = |id: Id, what: &str| -> Result<()> {
            if ids.insert(id) {
                Ok(())
            } else {
                Err(PcbError::invalid(format!(
                    "object id {} ({what}) is used twice",
                    id.0
                )))
            }
        };
        let copper_ok = |l: Layer, what: &str| -> Result<usize> {
            l.copper_index(self.copper_layers).ok_or_else(|| {
                PcbError::invalid(format!(
                    "{what} is on {l}, which is not a copper layer of this {}-layer board",
                    self.copper_layers
                ))
            })
        };

        for fp in &self.footprints {
            claim(fp.id, "footprint")?;
            self.validate_footprint(fp, &mut claim, &check_net)?;
        }
        for t in &self.tracks {
            let what = format!("track {}", t.id.0);
            claim(t.id, "track")?;
            check_net(t.net, &what)?;
            copper_ok(t.layer, &what)?;
            check_positive(t.width, &format!("{what} width"))?;
            match &t.geom {
                TrackGeom::Segment { start, end } => {
                    for p in [start, end] {
                        check_coord(p.x, &what)?;
                        check_coord(p.y, &what)?;
                    }
                }
                TrackGeom::Arc(a) => a.validate(&what)?,
            }
        }
        for v in &self.vias {
            let what = format!("via {}", v.id.0);
            claim(v.id, "via")?;
            check_net(v.net, &what)?;
            check_coord(v.position.x, &what)?;
            check_coord(v.position.y, &what)?;
            check_positive(v.diameter, &format!("{what} diameter"))?;
            check_positive(v.drill, &format!("{what} drill"))?;
            if v.drill > v.diameter {
                return Err(PcbError::invalid(format!(
                    "{what}: drill {} nm is larger than the pad diameter {} nm",
                    v.drill, v.diameter
                )));
            }
            let a = copper_ok(v.from, &what)?;
            let b = copper_ok(v.to, &what)?;
            if a >= b {
                return Err(PcbError::invalid(format!(
                    "{what}: `from` ({}) must be above `to` ({})",
                    v.from, v.to
                )));
            }
            let last = self.copper_layers as usize - 1;
            let ok = match v.kind {
                ViaKind::Through => a == 0 && b == last,
                ViaKind::Blind => (a == 0) != (b == last),
                ViaKind::Buried => a > 0 && b < last,
                ViaKind::Micro => (a == 0 || b == last) && b == a + 1 && !(a == 0 && b == last),
            };
            if !ok {
                return Err(PcbError::invalid(format!(
                    "{what}: layers {}..{} do not fit a {:?} via on a {}-layer board",
                    v.from, v.to, v.kind, self.copper_layers
                )));
            }
        }
        for z in &self.zones {
            let what = format!("zone {}", z.id.0);
            claim(z.id, "zone")?;
            check_net(z.net, &what)?;
            copper_ok(z.layer, &what)?;
            z.outline.validate(&what)?;
            check_dim(z.clearance, &format!("{what} clearance"))?;
            check_dim(z.min_width, &format!("{what} minimum width"))?;
            check_dim(z.thermal.gap, &format!("{what} thermal gap"))?;
            check_dim(z.thermal.spoke_width, &format!("{what} spoke width"))?;
            if z.filled.len() > MAX_OBJECTS {
                return Err(PcbError::range(&what, "too many fill polygons"));
            }
            for (i, f) in z.filled.iter().enumerate() {
                f.validate(&format!("{what} fill {i}"))?;
            }
        }
        for k in &self.keepouts {
            let what = format!("keepout {}", k.id.0);
            claim(k.id, "keepout")?;
            k.outline.validate(&what)?;
            for l in &k.layers {
                copper_ok(*l, &what)?;
            }
        }
        for g in &self.graphics {
            claim(g.id, "graphic")?;
            self.validate_graphic(g, false)?;
        }
        for t in &self.texts {
            claim(t.id, "text")?;
            self.validate_text(t)?;
        }
        Ok(())
    }

    fn validate_footprint(
        &self,
        fp: &Footprint,
        claim: &mut impl FnMut(Id, &str) -> Result<()>,
        check_net: &impl Fn(Option<NetId>, &str) -> Result<()>,
    ) -> Result<()> {
        let what = format!("footprint {}", fp.id.0);
        check_text(&fp.reference, "footprint reference")?;
        check_text(&fp.value, "footprint value")?;
        check_text(&fp.package, "footprint package")?;
        check_coord(fp.position.x, &what)?;
        check_coord(fp.position.y, &what)?;
        check_rotation(fp.rotation, &what)?;
        if fp.pads.len() > MAX_PADS_PER_FOOTPRINT {
            return Err(PcbError::TooLarge {
                what: "footprint",
                detail: format!(
                    "{} pads exceed the limit of {MAX_PADS_PER_FOOTPRINT}",
                    fp.pads.len()
                ),
            });
        }
        if let Some(c) = &fp.courtyard {
            c.validate(&format!("{what} courtyard"))?;
        }
        for pad in &fp.pads {
            let pw = format!(
                "pad {} of {}",
                pad.number,
                if fp.reference.is_empty() {
                    what.clone()
                } else {
                    fp.reference.clone()
                }
            );
            claim(pad.id, "pad")?;
            check_text(&pad.number, "pad number")?;
            check_net(pad.net, &pw)?;
            check_coord(pad.position.x, &pw)?;
            check_coord(pad.position.y, &pw)?;
            check_rotation(pad.rotation, &pw)?;
            for (n, m) in [
                ("mask margin", pad.mask_margin),
                ("paste margin", pad.paste_margin),
            ] {
                if let Some(m) = m {
                    if m.abs() > MAX_DIM {
                        return Err(PcbError::range(
                            format!("{pw} {n}"),
                            format!("{m} nm is outside ±{MAX_DIM} nm"),
                        ));
                    }
                }
            }
            match &pad.shape {
                PadShape::Circle => {
                    check_positive(pad.size.w, &format!("{pw} diameter"))?;
                    if pad.size.w != pad.size.h {
                        return Err(PcbError::invalid(format!(
                            "{pw}: a circular pad needs size.w == size.h"
                        )));
                    }
                }
                PadShape::Rect | PadShape::Oval => {
                    check_positive(pad.size.w, &format!("{pw} width"))?;
                    check_positive(pad.size.h, &format!("{pw} height"))?;
                }
                PadShape::RoundRect { corner_radius } => {
                    check_positive(pad.size.w, &format!("{pw} width"))?;
                    check_positive(pad.size.h, &format!("{pw} height"))?;
                    check_dim(*corner_radius, &format!("{pw} corner radius"))?;
                    if corner_radius.saturating_mul(2) > pad.size.w.min(pad.size.h) {
                        return Err(PcbError::invalid(format!("{pw}: corner radius {corner_radius} nm is more than half the shorter side")));
                    }
                }
                PadShape::Custom { polygon } => {
                    Region::new(polygon.clone(), vec![])
                        .map_err(|e| PcbError::invalid(format!("{pw}: custom polygon: {e}")))?;
                }
            }
            match (pad.kind, &pad.drill) {
                (PadKind::Smd, Some(_)) => {
                    return Err(PcbError::invalid(format!(
                        "{pw}: a surface-mount pad cannot have a drill"
                    )))
                }
                (PadKind::ThroughHole | PadKind::Npth, None) => {
                    return Err(PcbError::invalid(format!(
                        "{pw}: a through-hole or non-plated pad needs a drill"
                    )))
                }
                _ => {}
            }
            if pad.kind == PadKind::Npth && pad.net.is_some() {
                return Err(PcbError::invalid(format!(
                    "{pw}: a non-plated hole cannot belong to a net"
                )));
            }
            if let Some(d) = &pad.drill {
                check_positive(d.diameter, &format!("{pw} drill"))?;
                if let Some(l) = d.slot_length {
                    check_positive(l, &format!("{pw} slot length"))?;
                    if l < d.diameter {
                        return Err(PcbError::invalid(format!(
                            "{pw}: slot length {l} nm is shorter than its width {} nm",
                            d.diameter
                        )));
                    }
                }
            }
        }
        for g in &fp.graphics {
            claim(g.id, "footprint graphic")?;
            self.validate_graphic(g, true)?;
        }
        for t in &fp.texts {
            claim(t.id, "footprint text")?;
            self.validate_text(t)?;
        }
        Ok(())
    }

    fn validate_graphic(&self, g: &Graphic, in_footprint: bool) -> Result<()> {
        let what = format!("graphic {}", g.id.0);
        if g.layer.is_copper() && g.layer.copper_index(self.copper_layers).is_none() {
            return Err(PcbError::invalid(format!(
                "{what} is on {}, which this board does not have",
                g.layer
            )));
        }
        check_dim(g.width, &format!("{what} width"))?;
        if g.width == 0 && !g.filled {
            return Err(PcbError::invalid(format!(
                "{what}: an outline graphic needs a width"
            )));
        }
        match &g.kind {
            GraphicKind::Line { a, b } => {
                for p in [a, b] {
                    check_coord(p.x, &what)?;
                    check_coord(p.y, &what)?;
                }
                if a == b {
                    return Err(PcbError::geometry(&what, "line has zero length"));
                }
            }
            GraphicKind::Circle { center, radius } => {
                check_coord(center.x, &what)?;
                check_coord(center.y, &what)?;
                check_positive(*radius, &format!("{what} radius"))?;
            }
            GraphicKind::Arc(a) => {
                if in_footprint {
                    return Err(PcbError::Unsupported(format!("{what}: arcs are not supported inside footprints; use a polygon or a board-level graphic")));
                }
                a.validate(&what)?;
            }
            GraphicKind::Polygon(r) => r.validate(&what)?,
        }
        Ok(())
    }

    fn validate_text(&self, t: &TextItem) -> Result<()> {
        let what = format!("text {}", t.id.0);
        check_text(&t.text, &what)?;
        if t.text.chars().count() > crate::font::MAX_TEXT_CHARS {
            return Err(PcbError::range(
                &what,
                format!("longer than {} characters", crate::font::MAX_TEXT_CHARS),
            ));
        }
        check_coord(t.position.x, &what)?;
        check_coord(t.position.y, &what)?;
        check_positive(t.height, &format!("{what} height"))?;
        check_positive(t.thickness, &format!("{what} thickness"))?;
        check_rotation(t.rotation, &what)?;
        if t.layer.is_copper() && t.layer.copper_index(self.copper_layers).is_none() {
            return Err(PcbError::invalid(format!(
                "{what} is on {}, which this board does not have",
                t.layer
            )));
        }
        Ok(())
    }

    /// Map from a pad number to its group index within one footprint, for same-terminal tests.
    pub(crate) fn pad_groups(&self) -> HashMap<(Id, &str), u32> {
        let mut m = HashMap::new();
        let mut next = 0u32;
        for fp in &self.footprints {
            for p in &fp.pads {
                m.entry((fp.id, p.number.as_str())).or_insert_with(|| {
                    next += 1;
                    next
                });
            }
        }
        m
    }
}

fn check_rotation(r: i32, what: &str) -> Result<()> {
    if (-3_600_000..=3_600_000).contains(&r) {
        Ok(())
    } else {
        Err(PcbError::range(
            format!("{what} rotation"),
            format!("{r} millidegrees is outside ±3 600 000"),
        ))
    }
}

/// Twice the signed area of a ring; exposed for callers that build polygons.
pub fn polygon_area2(ring: &[Point]) -> i128 {
    ring_area2(ring)
}

/// Whether a polygon ring is simple.
pub fn polygon_is_simple(ring: &[Point]) -> bool {
    ring_is_simple(ring)
}

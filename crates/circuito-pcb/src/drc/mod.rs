// SPDX-License-Identifier: MIT OR Apache-2.0
//! Design rules check.
//!
//! [`run_drc`] validates the board, builds exact shapes for every copper object, finds candidate
//! pairs with a uniform grid ([`index::Grid`], roughly linear in the number of objects for boards of
//! bounded local density) and decides every clearance with exact integer arithmetic: a pair passes
//! when its gap is at least the required clearance, so "required minus 1 nm" fails and "required"
//! passes. Arcs are approximated by polylines within `Rules::arc_tolerance` for the checks (Gerber
//! output keeps true arcs). Results are sorted, so the report is deterministic.

mod checks;
mod connectivity;
pub mod index;
pub mod items;
mod misc;
pub mod rules;
pub mod violation;

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::board::Board;
use crate::error::Result;
use crate::geom::point::{Point, Rect};
use crate::layer::Layer;
use crate::units::{Nm, MM};

pub use rules::{CapabilityProfile, ClearanceTable, Rules};
pub use violation::{ObjKind, ObjRef, Severity, Violation, ViolationCode};

use index::Grid;
use items::{build_items, Item};

/// Counters about a run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrcStats {
    pub copper_items: usize,
    pub candidate_pairs: usize,
    pub holes: usize,
}

/// The result of a design rules check.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrcReport {
    pub violations: Vec<Violation>,
    /// True when `Rules::max_violations` was reached and further findings were dropped.
    pub truncated: bool,
    pub stats: DrcStats,
}

impl DrcReport {
    pub fn count(&self, code: ViolationCode) -> usize {
        self.violations.iter().filter(|v| v.code == code).count()
    }
    pub fn has(&self, code: ViolationCode) -> bool {
        self.violations.iter().any(|v| v.code == code)
    }
    pub fn errors(&self) -> usize {
        self.violations
            .iter()
            .filter(|v| v.severity == Severity::Error)
            .count()
    }
    pub fn warnings(&self) -> usize {
        self.violations
            .iter()
            .filter(|v| v.severity == Severity::Warning)
            .count()
    }
    /// No findings at all.
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}

pub(crate) struct Ctx<'a> {
    pub board: &'a Board,
    pub rules: &'a Rules,
    pub items: Vec<Item>,
    pub grid: Grid,
    pub stack: Vec<Layer>,
}

pub(crate) struct Out<'a> {
    rules: &'a Rules,
    pub v: Vec<Violation>,
    pub truncated: bool,
}

impl Out<'_> {
    pub fn push(&mut self, v: Violation) {
        if !self.rules.enabled(v.code) {
            return;
        }
        if self.v.len() >= self.rules.max_violations {
            self.truncated = true;
            return;
        }
        self.v.push(v);
    }
    pub fn full(&self) -> bool {
        self.v.len() >= self.rules.max_violations
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn violation(
    code: ViolationCode,
    layer: Option<Layer>,
    locations: Vec<Point>,
    objects: Vec<ObjRef>,
    net: Option<String>,
    measured: Option<Nm>,
    required: Option<Nm>,
    message: String,
) -> Violation {
    Violation {
        code,
        severity: code.severity(),
        layer,
        locations,
        objects,
        net,
        measured,
        required,
        message,
    }
}

/// Millimetres for messages.
pub(crate) fn mm(nm: Nm) -> String {
    let s = format!("{:.4}", nm as f64 / MM as f64);
    let t = s.trim_end_matches('0').trim_end_matches('.');
    if t.is_empty() || t == "-" {
        "0".to_string()
    } else {
        t.to_string()
    }
}

/// Compares the gap of two items with `req`: the minimum over their shape pairs.
pub(crate) fn items_cmp(a: &Item, b: &Item, req: Nm) -> Ordering {
    let mut res = Ordering::Greater;
    for (sa, ba) in a.shapes.iter().zip(&a.boxes) {
        for (sb, bb) in b.shapes.iter().zip(&b.boxes) {
            if !ba.inflate(req.saturating_add(1)).intersects(*bb) {
                continue;
            }
            res = res.min(sa.gap_cmp(sb, req));
            if res == Ordering::Less {
                return res;
            }
        }
    }
    res
}

/// Smallest `floor(gap)` over the shape pairs of two items within `req` of each other.
pub(crate) fn items_gap_floor(a: &Item, b: &Item, req: Nm) -> Nm {
    let mut best = Nm::MAX;
    for (sa, ba) in a.shapes.iter().zip(&a.boxes) {
        for (sb, bb) in b.shapes.iter().zip(&b.boxes) {
            if !ba.inflate(req.saturating_add(1)).intersects(*bb) {
                continue;
            }
            best = best.min(sa.gap_floor(sb));
        }
    }
    if best == Nm::MAX {
        0
    } else {
        best
    }
}

/// A marker position for a pair of items: the centre of where their neighbourhoods overlap.
pub(crate) fn marker(a: Rect, b: Rect, req: Nm) -> Point {
    let (ia, ib) = (a.inflate(req), b.inflate(req));
    let lo = Point::new(ia.min.x.max(ib.min.x), ia.min.y.max(ib.min.y));
    let hi = Point::new(ia.max.x.min(ib.max.x), ia.max.y.min(ib.max.y));
    if lo.x <= hi.x && lo.y <= hi.y {
        Rect { min: lo, max: hi }.center()
    } else {
        a.center()
    }
}

/// Runs every check enabled in `rules` on `board`.
pub fn run_drc(board: &Board, rules: &Rules) -> Result<DrcReport> {
    board.validate()?;
    rules.validate()?;
    let items = build_items(board, rules)?;
    let max_req = max_requirement(board, rules);
    let margin = max_req / 2 + 1;
    let boxes: Vec<Rect> = items.iter().map(|i| i.bbox.inflate(margin)).collect();
    let cell = Grid::choose_cell(&boxes, MM.max(max_req.saturating_mul(4)));
    let grid = Grid::new(boxes, cell);
    let ctx = Ctx {
        board,
        rules,
        items,
        grid,
        stack: board.copper_stack(),
    };
    let mut out = Out {
        rules,
        v: Vec::new(),
        truncated: false,
    };
    let mut stats = DrcStats {
        copper_items: ctx.items.len(),
        ..DrcStats::default()
    };

    checks::check_widths_and_drills(&ctx, &mut out);
    checks::check_copper_pairs(&ctx, &mut out, &mut stats);
    connectivity::check_unconnected(&ctx, &mut out);
    checks::check_dangling(&ctx, &mut out);
    checks::check_acute(&ctx, &mut out);
    checks::check_holes(&ctx, &mut out, &mut stats)?;
    checks::check_edges(&ctx, &mut out)?;
    checks::check_keepouts(&ctx, &mut out)?;
    misc::check_silk(&ctx, &mut out)?;
    misc::check_mask(&ctx, &mut out)?;
    misc::check_courtyards(&ctx, &mut out)?;
    misc::check_zones(&ctx, &mut out);

    let mut violations = out.v;
    violations.sort_by_key(Violation::sort_key);
    Ok(DrcReport {
        violations,
        truncated: out.truncated,
        stats,
    })
}

/// Checks the board against a manufacturer capability profile: copper layer count and via types.
/// (The numeric limits of a profile are enforced by running DRC with `profile.rules()`.)
pub fn check_profile(board: &Board, profile: &CapabilityProfile) -> Result<Vec<Violation>> {
    board.validate()?;
    let mut v = Vec::new();
    if board.copper_layers > profile.max_copper_layers {
        v.push(violation(
            ViolationCode::ProfileLayers,
            None,
            vec![],
            vec![ObjRef {
                kind: ObjKind::Board,
                id: crate::board::Id(0),
            }],
            None,
            Some(board.copper_layers as Nm),
            Some(profile.max_copper_layers as Nm),
            format!(
                "the board has {} copper layers but profile `{}` supports at most {}",
                board.copper_layers, profile.name, profile.max_copper_layers
            ),
        ));
    }
    if !profile.allow_blind_buried_vias {
        for via in board
            .vias
            .iter()
            .filter(|x| x.kind != crate::board::ViaKind::Through)
        {
            v.push(violation(
                ViolationCode::ProfileViaType,
                None,
                vec![via.position],
                vec![ObjRef {
                    kind: ObjKind::Via,
                    id: via.id,
                }],
                Some(board.net_name(via.net)),
                None,
                None,
                format!(
                    "via {} is {:?} but profile `{}` only makes through vias",
                    via.id.0, via.kind, profile.name
                ),
            ));
        }
    }
    v.sort_by_key(Violation::sort_key);
    Ok(v)
}

fn max_requirement(board: &Board, rules: &Rules) -> Nm {
    let c = &rules.clearance;
    let mut m = [
        c.track_track,
        c.track_pad,
        c.track_via,
        c.pad_pad,
        c.pad_via,
        c.via_via,
        c.zone_copper,
        rules.copper_to_edge,
        rules.silk_to_pad,
        rules.min_mask_web,
        rules.hole_to_hole,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    for k in &board.net_classes {
        m = m.max(k.clearance.unwrap_or(0));
    }
    for z in &board.zones {
        m = m.max(z.clearance);
    }
    m
}

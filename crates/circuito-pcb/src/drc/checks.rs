// SPDX-License-Identifier: MIT OR Apache-2.0
//! Copper checks: widths, drills, annular rings, clearances, shorts, dangling and acute tracks,
//! hole spacing, board edge and keepouts.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use super::index::Grid;
use super::items::{CopperKind, Item};
use super::rules::ClearanceTable;
use super::violation::{ObjKind, ObjRef, ViolationCode};
use super::{items_cmp, items_gap_floor, marker, mm, violation, Ctx, DrcStats, Out};
use crate::board::{PadKind, TrackGeom, ViaKind};
use crate::error::Result;
use crate::geom::dist::point_segment_dist2;
use crate::geom::point::{dot, Point, Rect};
use crate::geom::region::Loc;
use crate::geom::shape::{Core, Shape};
use crate::geom::wide::Ratio;
use crate::layer::Layer;
use crate::transform::{pad_center, pad_hole, pad_shape};
use crate::units::{Nm, MM};

fn track_mid(a: Point, b: Point) -> Point {
    Point::new(
        ((a.x as i128 + b.x as i128) / 2) as i64,
        ((a.y as i128 + b.y as i128) / 2) as i64,
    )
}

fn ceil_double_dist(r: Ratio) -> u128 {
    let f = r.floor_double_dist();
    if r.cmp_double_dist_sq(f) == Ordering::Equal {
        f
    } else {
        f + 1
    }
}

fn hole_points(hole: &Shape) -> Vec<Point> {
    match &hole.core {
        Core::Point(p) => vec![*p],
        Core::Segment(a, b) => vec![*a, *b],
        Core::Region(_) => vec![],
    }
}

/// Exact annular ring test: `Some(measured_floor)` when the ring between `hole` and the outline of
/// `pad` is narrower than `req` (the hole is assumed to sit inside the pad). Slot holes are judged
/// at their two ends; for a pad with a concave custom outline the test is exact only at those points.
pub(crate) fn ring_violation(pad: &Shape, hole: &Shape, req: Nm) -> Option<Nm> {
    let dp = pad.diam as i128;
    let dh = hole.diam as i128;
    let mut worst: Option<i128> = None; // smallest ring, floor, over the hole's end points
    let mut violated = false;
    for q in hole_points(hole) {
        let (viol, ring_floor): (bool, i128) = match &pad.core {
            Core::Point(_) | Core::Segment(_, _) => {
                let d2 = match &pad.core {
                    Core::Point(c) => Ratio::int(q.dist2(*c)),
                    Core::Segment(a, b) => point_segment_dist2(q, *a, *b),
                    Core::Region(_) => Ratio::ZERO,
                };
                let u = dp - dh - 2 * req as i128;
                let viol = if u < 0 {
                    true
                } else {
                    d2.cmp_double_dist_sq(u as u128) == Ordering::Greater
                };
                (viol, (dp - dh - ceil_double_dist(d2) as i128).div_euclid(2))
            }
            Core::Region(r) => {
                let inside = r.locate(q) != Loc::Outside;
                let bd2 = r
                    .edges()
                    .map(|(a, b)| point_segment_dist2(q, a, b))
                    .fold(Ratio::int(1 << 120), Ratio::min);
                let t = 2 * req as i128 + dh - dp;
                if inside {
                    let viol = t > 0 && bd2.cmp_double_dist_sq(t as u128) == Ordering::Less;
                    (
                        viol,
                        (dp - dh + bd2.floor_double_dist() as i128).div_euclid(2),
                    )
                } else {
                    let viol =
                        t > 0 || bd2.cmp_double_dist_sq(t.unsigned_abs()) == Ordering::Greater;
                    (
                        viol,
                        (dp - dh - ceil_double_dist(bd2) as i128).div_euclid(2),
                    )
                }
            }
        };
        violated |= viol;
        worst = Some(worst.map_or(ring_floor, |w| w.min(ring_floor)));
    }
    if violated {
        Some(worst.unwrap_or(0).clamp(0, i64::MAX as i128) as i64)
    } else {
        None
    }
}

pub(crate) fn check_widths_and_drills(ctx: &Ctx, out: &mut Out) {
    let r = ctx.rules;
    let b = ctx.board;
    for t in &b.tracks {
        let req = r.min_track_width.max(
            b.net_class(t.net)
                .and_then(|c| c.min_track_width)
                .unwrap_or(0),
        );
        if t.width < req {
            let (s, e) = t.endpoints();
            out.push(violation(
                ViolationCode::TrackWidth,
                Some(t.layer),
                vec![track_mid(s, e)],
                vec![ObjRef {
                    kind: ObjKind::Track,
                    id: t.id,
                }],
                Some(b.net_name(t.net)),
                Some(t.width),
                Some(req),
                format!(
                    "track {} is {} mm wide, the minimum is {} mm",
                    t.id.0,
                    mm(t.width),
                    mm(req)
                ),
            ));
        }
    }
    for v in &b.vias {
        let cls = b.net_class(v.net);
        let micro = v.kind == ViaKind::Micro;
        let req_d = (if micro {
            r.min_microvia_diameter
        } else {
            r.min_via_diameter
        })
        .max(cls.and_then(|c| c.min_via_diameter).unwrap_or(0));
        let req_h = (if micro {
            r.min_microvia_drill
        } else {
            r.min_via_drill
        })
        .max(r.min_drill.min(if micro {
            r.min_microvia_drill
        } else {
            r.min_via_drill
        }))
        .max(cls.and_then(|c| c.min_via_drill).unwrap_or(0));
        let obj = vec![ObjRef {
            kind: ObjKind::Via,
            id: v.id,
        }];
        let net = Some(b.net_name(v.net));
        if v.diameter < req_d {
            out.push(violation(
                ViolationCode::ViaDiameter,
                None,
                vec![v.position],
                obj.clone(),
                net.clone(),
                Some(v.diameter),
                Some(req_d),
                format!(
                    "via {} pad is {} mm, the minimum is {} mm",
                    v.id.0,
                    mm(v.diameter),
                    mm(req_d)
                ),
            ));
        }
        if v.drill < req_h {
            out.push(violation(
                ViolationCode::DrillMin,
                None,
                vec![v.position],
                obj.clone(),
                net.clone(),
                Some(v.drill),
                Some(req_h),
                format!(
                    "via {} drill is {} mm, the minimum is {} mm",
                    v.id.0,
                    mm(v.drill),
                    mm(req_h)
                ),
            ));
        }
        if let Some(m) = ring_violation(
            &Shape::point(v.position, v.diameter),
            &Shape::point(v.position, v.drill),
            r.min_annular_ring,
        ) {
            out.push(violation(
                ViolationCode::ViaAnnularRing,
                None,
                vec![v.position],
                obj,
                net,
                Some(m),
                Some(r.min_annular_ring),
                format!(
                    "via {} annular ring is {} mm, the minimum is {} mm",
                    v.id.0,
                    mm(m),
                    mm(r.min_annular_ring)
                ),
            ));
        }
    }
    for fp in &b.footprints {
        for pad in &fp.pads {
            let Some(d) = &pad.drill else { continue };
            let Ok(c) = pad_center(fp, pad) else { continue };
            let label = super::items::pad_label(fp, pad);
            let obj = vec![ObjRef {
                kind: ObjKind::Pad,
                id: pad.id,
            }];
            if d.diameter < r.min_drill {
                out.push(violation(
                    ViolationCode::DrillMin,
                    None,
                    vec![c],
                    obj.clone(),
                    Some(b.net_name(pad.net)),
                    Some(d.diameter),
                    Some(r.min_drill),
                    format!(
                        "{label} drill is {} mm, the minimum is {} mm",
                        mm(d.diameter),
                        mm(r.min_drill)
                    ),
                ));
            }
            if pad.kind == PadKind::ThroughHole {
                if let (Ok(Some(ps)), Ok(Some(hs))) = (pad_shape(fp, pad, 0), pad_hole(fp, pad)) {
                    if let Some(m) = ring_violation(&ps, &hs, r.min_annular_ring) {
                        out.push(violation(
                            ViolationCode::PadAnnularRing,
                            None,
                            vec![c],
                            obj,
                            Some(b.net_name(pad.net)),
                            Some(m),
                            Some(r.min_annular_ring),
                            format!(
                                "{label} annular ring is {} mm, the minimum is {} mm",
                                mm(m),
                                mm(r.min_annular_ring)
                            ),
                        ));
                    }
                }
            }
        }
    }
}

fn pair_code(a: CopperKind, b: CopperKind, t: &ClearanceTable) -> (ViolationCode, Nm) {
    use CopperKind::*;
    match (a, b) {
        (Zone, _) | (_, Zone) => (ViolationCode::ClearanceZoneCopper, t.zone_copper),
        (Track, Track) => (ViolationCode::ClearanceTrackTrack, t.track_track),
        (Track, Pad) | (Pad, Track) => (ViolationCode::ClearanceTrackPad, t.track_pad),
        (Track, Via) | (Via, Track) => (ViolationCode::ClearanceTrackVia, t.track_via),
        (Pad, Pad) => (ViolationCode::ClearancePadPad, t.pad_pad),
        (Pad, Via) | (Via, Pad) => (ViolationCode::ClearancePadVia, t.pad_via),
        (Via, Via) => (ViolationCode::ClearanceViaVia, t.via_via),
    }
}

/// Required clearance between two items of different nets.
pub(crate) fn required_clearance(a: &Item, b: &Item, t: &ClearanceTable) -> (ViolationCode, Nm) {
    let (code, base) = pair_code(a.kind, b.kind, t);
    (code, base.max(a.own_clearance).max(b.own_clearance))
}

pub(crate) fn check_copper_pairs(ctx: &Ctx, out: &mut Out, stats: &mut DrcStats) {
    let mut pairs = 0usize;
    ctx.grid.for_each_pair(|i, j| {
        pairs += 1;
        if out.full() {
            return;
        }
        let (a, b) = (&ctx.items[i], &ctx.items[j]);
        if a.mask & b.mask == 0 || a.same_electrical(b) {
            return;
        }
        let (code, req) = required_clearance(a, b, &ctx.rules.clearance);
        if items_cmp(a, b, req) != Ordering::Less {
            return;
        }
        let layer = a.common_layer(b, &ctx.stack);
        let objects = vec![a.obj, b.obj];
        let loc = marker(a.bbox, b.bbox, req / 2);
        let (na, nb) = (ctx.board.net_name(a.net), ctx.board.net_name(b.net));
        if items_cmp(a, b, 0) != Ordering::Greater {
            out.push(violation(
                ViolationCode::ShortCircuit,
                Some(layer),
                vec![loc],
                objects,
                Some(format!("{na} / {nb}")),
                Some(0),
                Some(req),
                format!(
                    "{} (net {na}) touches {} (net {nb}) on {layer}",
                    a.label, b.label
                ),
            ));
        } else {
            let gap = items_gap_floor(a, b, req);
            out.push(violation(
                code,
                Some(layer),
                vec![loc],
                objects,
                Some(format!("{na} / {nb}")),
                Some(gap),
                Some(req),
                format!(
                    "{} and {} are {} mm apart on {layer}, the minimum is {} mm",
                    a.label,
                    b.label,
                    mm(gap),
                    mm(req)
                ),
            ));
        }
    });
    stats.candidate_pairs = pairs;
}

pub(crate) fn check_dangling(ctx: &Ctx, out: &mut Out) {
    if !ctx.rules.check_dangling {
        return;
    }
    for (ti, t) in ctx.board.tracks.iter().enumerate() {
        let Some(me) = ctx.items.get(ti) else {
            continue;
        };
        if t.net.is_none() {
            continue;
        }
        let (s, e) = t.endpoints();
        let mut loose = Vec::new();
        for end in [s, e] {
            let probe = Shape::point(end, t.width);
            let mut joined = false;
            ctx.grid.query(probe.bbox(), |k| {
                if joined || k == ti {
                    return;
                }
                let o = &ctx.items[k];
                if o.net != t.net || o.mask & me.mask == 0 {
                    return;
                }
                if o.shapes
                    .iter()
                    .any(|sh| probe.gap_cmp(sh, 0) != Ordering::Greater)
                {
                    joined = true;
                }
            });
            if !joined {
                loose.push(end);
            }
        }
        loose.dedup();
        if !loose.is_empty() {
            out.push(violation(
                ViolationCode::TrackDangling,
                Some(t.layer),
                loose.clone(),
                vec![ObjRef {
                    kind: ObjKind::Track,
                    id: t.id,
                }],
                Some(ctx.board.net_name(t.net)),
                None,
                None,
                format!(
                    "track {} has {} end(s) connected to nothing",
                    t.id.0,
                    loose.len()
                ),
            ));
        }
    }
}

pub(crate) fn check_acute(ctx: &Ctx, out: &mut Out) {
    if !ctx.rules.reject_acute_traces {
        return;
    }
    let mut at: HashMap<(Layer, crate::board::NetId, Point), Vec<(usize, Point)>> = HashMap::new();
    for (ti, t) in ctx.board.tracks.iter().enumerate() {
        let (Some(net), TrackGeom::Segment { start, end }) = (t.net, &t.geom) else {
            continue;
        };
        if start == end {
            continue;
        }
        at.entry((t.layer, net, *start))
            .or_default()
            .push((ti, *end));
        at.entry((t.layer, net, *end))
            .or_default()
            .push((ti, *start));
    }
    let mut found: BTreeMap<(usize, usize), (Layer, Point)> = BTreeMap::new();
    for ((layer, _net, joint), list) in &at {
        for (k, &(ta, pa)) in list.iter().enumerate() {
            for &(tb, pb) in &list[k + 1..] {
                if ta != tb && dot(*joint, pa, pb) > 0 {
                    found
                        .entry((ta.min(tb), ta.max(tb)))
                        .or_insert((*layer, *joint));
                }
            }
        }
    }
    for ((ta, tb), (layer, joint)) in found {
        let (a, b) = (&ctx.board.tracks[ta], &ctx.board.tracks[tb]);
        out.push(violation(
            ViolationCode::TrackAcuteAngle,
            Some(layer),
            vec![joint],
            vec![
                ObjRef {
                    kind: ObjKind::Track,
                    id: a.id,
                },
                ObjRef {
                    kind: ObjKind::Track,
                    id: b.id,
                },
            ],
            Some(ctx.board.net_name(a.net)),
            None,
            None,
            format!(
                "tracks {} and {} meet at less than 90 degrees",
                a.id.0, b.id.0
            ),
        ));
    }
}

pub(crate) fn check_holes(ctx: &Ctx, out: &mut Out, stats: &mut DrcStats) -> Result<()> {
    let req = ctx.rules.hole_to_hole;
    let mut holes: Vec<(Shape, ObjRef, String, crate::board::Id)> = Vec::new();
    for v in &ctx.board.vias {
        holes.push((
            Shape::point(v.position, v.drill),
            ObjRef {
                kind: ObjKind::Via,
                id: v.id,
            },
            format!("via {}", v.id.0),
            v.id,
        ));
    }
    for fp in &ctx.board.footprints {
        for pad in &fp.pads {
            if let Some(h) = pad_hole(fp, pad)? {
                holes.push((
                    h,
                    ObjRef {
                        kind: ObjKind::Pad,
                        id: pad.id,
                    },
                    super::items::pad_label(fp, pad),
                    fp.id,
                ));
            }
        }
    }
    stats.holes = holes.len();
    let boxes: Vec<Rect> = holes
        .iter()
        .map(|h| h.0.bbox().inflate(req / 2 + 1))
        .collect();
    let cell = Grid::choose_cell(&boxes, MM);
    let grid = Grid::new(boxes, cell);
    grid.for_each_pair(|i, j| {
        if out.full() {
            return;
        }
        let (a, b) = (&holes[i], &holes[j]);
        if a.0.gap_cmp(&b.0, req) == Ordering::Less {
            let gap = a.0.gap_floor(&b.0);
            out.push(violation(
                ViolationCode::HoleToHole,
                None,
                vec![marker(a.0.bbox(), b.0.bbox(), 0)],
                vec![a.1, b.1],
                None,
                Some(gap),
                Some(req),
                format!(
                    "holes of {} and {} are {} mm apart, the minimum is {} mm",
                    a.2,
                    b.2,
                    mm(gap),
                    mm(req)
                ),
            ));
        }
    });
    Ok(())
}

pub(crate) fn check_edges(ctx: &Ctx, out: &mut Out) -> Result<()> {
    let Some(outline) = &ctx.board.outline else {
        out.push(violation(
            ViolationCode::OutlineMissing,
            None,
            vec![],
            vec![ObjRef {
                kind: ObjKind::Board,
                id: crate::board::Id(0),
            }],
            None,
            None,
            None,
            "the board has no outline (Edge.Cuts)".to_string(),
        ));
        return Ok(());
    };
    let region = outline.to_region(ctx.rules.arc_tolerance)?;
    let req = ctx.rules.copper_to_edge;
    let edges: Vec<(Point, Point)> = region.edges().collect();
    let eboxes: Vec<Rect> = edges
        .iter()
        .map(|(a, b)| Rect::from_point(*a).include(*b).inflate(req + 1))
        .collect();
    let cell = Grid::choose_cell(&eboxes, MM);
    let egrid = Grid::new(eboxes, cell);
    for it in &ctx.items {
        if out.full() {
            break;
        }
        let mut best: Option<Nm> = None;
        let mut touching = false;
        for (sh, bx) in it.shapes.iter().zip(&it.boxes) {
            egrid.query(bx.inflate(req + 1), |k| {
                let (a, b) = edges[k];
                let es = Shape::segment(a, b, 0);
                if sh.gap_cmp(&es, req) == Ordering::Less {
                    let g = sh.gap_floor(&es);
                    best = Some(best.map_or(g, |x| x.min(g)));
                    if sh.gap_cmp(&es, 0) != Ordering::Greater {
                        touching = true;
                    }
                }
            });
        }
        let outside = it
            .shapes
            .first()
            .map(|s| region.locate(s.any_point()) == Loc::Outside)
            .unwrap_or(false);
        if outside && !touching {
            out.push(violation(
                ViolationCode::EdgeOutside,
                Some(it.layer(&ctx.stack)),
                vec![it.at],
                vec![it.obj],
                Some(ctx.board.net_name(it.net)),
                None,
                None,
                format!("{} lies outside the board outline", it.label),
            ));
        } else if let Some(g) = best {
            out.push(violation(
                ViolationCode::EdgeClearance,
                Some(it.layer(&ctx.stack)),
                vec![it.at],
                vec![it.obj],
                Some(ctx.board.net_name(it.net)),
                Some(g),
                Some(req),
                format!(
                    "{} is {} mm from the board edge, the minimum is {} mm",
                    it.label,
                    mm(g),
                    mm(req)
                ),
            ));
        }
    }
    Ok(())
}

pub(crate) fn check_keepouts(ctx: &Ctx, out: &mut Out) -> Result<()> {
    for k in &ctx.board.keepouts {
        let mask: u32 = if k.layers.is_empty() {
            u32::MAX
        } else {
            k.layers
                .iter()
                .filter_map(|l| l.copper_index(ctx.board.copper_layers))
                .fold(0, |m, i| m | (1 << i))
        };
        let ks = Shape::region(k.outline.clone(), 0);
        let mut hits: Vec<usize> = Vec::new();
        ctx.grid.query(ks.bbox(), |i| hits.push(i));
        hits.sort_unstable();
        for i in hits {
            let it = &ctx.items[i];
            if it.mask & mask == 0 {
                continue;
            }
            let (code, forbidden) = match it.kind {
                CopperKind::Track => (ViolationCode::KeepoutTrack, k.no_tracks),
                CopperKind::Via => (ViolationCode::KeepoutVia, k.no_vias),
                CopperKind::Pad => (ViolationCode::KeepoutPad, k.no_pads),
                CopperKind::Zone => (ViolationCode::KeepoutZone, k.no_zones),
            };
            if !forbidden {
                continue;
            }
            if it
                .shapes
                .iter()
                .any(|s| s.gap_cmp(&ks, 0) != Ordering::Greater)
            {
                out.push(violation(
                    code,
                    Some(it.layer(&ctx.stack)),
                    vec![marker(it.bbox, ks.bbox(), 0)],
                    vec![
                        it.obj,
                        ObjRef {
                            kind: ObjKind::Keepout,
                            id: k.id,
                        },
                    ],
                    Some(ctx.board.net_name(it.net)),
                    None,
                    None,
                    format!("{} is inside keepout {}", it.label, k.id.0),
                ));
            }
        }
    }
    Ok(())
}

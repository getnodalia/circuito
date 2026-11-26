// SPDX-License-Identifier: MIT OR Apache-2.0
//! Silkscreen, solder mask, courtyard and zone checks.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};

use super::index::Grid;
use super::violation::{ObjKind, ObjRef, ViolationCode};
use super::{marker, mm, violation, Ctx, Out};
use crate::artwork::{collect_artwork, fp_label};
use crate::board::{NetId, PadKind};
use crate::error::Result;
use crate::geom::point::{Point, Rect};
use crate::geom::region::rings_overlap_interior;
use crate::geom::shape::Shape;
use crate::layer::{Layer, Side};
use crate::transform::{pad_shape, place_region};
use crate::units::{Nm, MM};

fn pad_sides(kind: PadKind, side: Side) -> Vec<Side> {
    match kind {
        PadKind::Smd => vec![side],
        PadKind::ThroughHole => vec![Side::Front, Side::Back],
        PadKind::Npth => vec![],
    }
}

pub(crate) fn check_silk(ctx: &Ctx, out: &mut Out) -> Result<()> {
    let r = ctx.rules;
    let art = collect_artwork(ctx.board)?;
    // exposed pads per side
    struct PadEntry {
        shape: Shape,
        obj: ObjRef,
    }
    let mut pads: [Vec<PadEntry>; 2] = [Vec::new(), Vec::new()];
    for fp in &ctx.board.footprints {
        for pad in &fp.pads {
            if !pad.mask {
                continue;
            }
            let Some(shape) = pad_shape(fp, pad, 0)? else {
                continue;
            };
            for s in pad_sides(pad.kind, fp.side) {
                pads[s as usize].push(PadEntry {
                    shape: shape.clone(),
                    obj: ObjRef {
                        kind: ObjKind::Pad,
                        id: pad.id,
                    },
                });
            }
        }
    }
    let grids: Vec<Grid> = pads
        .iter()
        .map(|list| {
            let boxes: Vec<Rect> = list
                .iter()
                .map(|p| p.shape.bbox().inflate(r.silk_to_pad + 1))
                .collect();
            let cell = Grid::choose_cell(&boxes, MM);
            Grid::new(boxes, cell)
        })
        .collect();
    let mut hits: BTreeMap<(ObjRef, ObjRef), (Nm, Point, Layer)> = BTreeMap::new();
    let mut thin: HashSet<ObjRef> = HashSet::new();
    for prim in &art.prims {
        let side = match prim.layer {
            Layer::FSilk => Side::Front,
            Layer::BSilk => Side::Back,
            _ => continue,
        };
        if prim.width > 0 && prim.width < r.min_silk_width && thin.insert(prim.owner) {
            out.push(violation(
                ViolationCode::SilkWidth,
                Some(prim.layer),
                vec![prim.at()],
                vec![prim.owner],
                None,
                Some(prim.width),
                Some(r.min_silk_width),
                format!(
                    "silkscreen line width {} mm is below the minimum {} mm",
                    mm(prim.width),
                    mm(r.min_silk_width)
                ),
            ));
        }
        let idx = side as usize;
        for sh in prim.shapes(r.arc_tolerance)? {
            grids[idx].query(sh.bbox().inflate(r.silk_to_pad + 1), |k| {
                let pe = &pads[idx][k];
                if sh.gap_cmp(&pe.shape, r.silk_to_pad) == Ordering::Less {
                    let g = sh.gap_floor(&pe.shape);
                    let e = hits.entry((prim.owner, pe.obj)).or_insert((
                        g,
                        marker(sh.bbox(), pe.shape.bbox(), 0),
                        prim.layer,
                    ));
                    if g < e.0 {
                        e.0 = g;
                    }
                }
            });
        }
    }
    for ((owner, pad), (gap, at, layer)) in hits {
        out.push(violation(
            ViolationCode::SilkOverPad,
            Some(layer),
            vec![at],
            vec![owner, pad],
            None,
            Some(gap),
            Some(r.silk_to_pad),
            format!(
                "silkscreen is {} mm from an exposed pad, the minimum is {} mm",
                mm(gap),
                mm(r.silk_to_pad)
            ),
        ));
    }
    Ok(())
}

pub(crate) fn check_mask(ctx: &Ctx, out: &mut Out) -> Result<()> {
    let r = ctx.rules;
    struct Opening {
        shape: Shape,
        obj: ObjRef,
        net: Option<NetId>,
        group: Option<u32>,
        label: String,
    }
    let groups = ctx.board.pad_groups();
    let mut open: [Vec<Opening>; 2] = [Vec::new(), Vec::new()];
    for fp in &ctx.board.footprints {
        for pad in &fp.pads {
            if !pad.mask || pad.kind == PadKind::Npth {
                continue;
            }
            let margin = pad.mask_margin.unwrap_or(r.mask_margin);
            let Some(shape) = pad_shape(fp, pad, margin)? else {
                continue;
            };
            for s in pad_sides(pad.kind, fp.side) {
                open[s as usize].push(Opening {
                    shape: shape.clone(),
                    obj: ObjRef {
                        kind: ObjKind::Pad,
                        id: pad.id,
                    },
                    net: pad.net,
                    group: groups.get(&(fp.id, pad.number.as_str())).copied(),
                    label: super::items::pad_label(fp, pad),
                });
            }
        }
    }
    for v in &ctx.board.vias {
        if v.tented {
            continue;
        }
        let shape = Shape::point(v.position, v.diameter + 2 * r.mask_margin.max(0));
        for l in [v.from, v.to] {
            let s = match l {
                Layer::FCu => Side::Front,
                Layer::BCu => Side::Back,
                _ => continue,
            };
            open[s as usize].push(Opening {
                shape: shape.clone(),
                obj: ObjRef {
                    kind: ObjKind::Via,
                    id: v.id,
                },
                net: v.net,
                group: None,
                label: format!("via {}", v.id.0),
            });
        }
    }
    let mut seen: HashSet<(ObjRef, ObjRef, ViolationCode)> = HashSet::new();
    let web = r.min_mask_web;
    for (si, list) in open.iter().enumerate() {
        let layer = if si == 0 { Layer::FMask } else { Layer::BMask };
        let boxes: Vec<Rect> = list
            .iter()
            .map(|o| o.shape.bbox().inflate(web / 2 + 1))
            .collect();
        let cell = Grid::choose_cell(&boxes, MM);
        let grid = Grid::new(boxes, cell);
        grid.for_each_pair(|i, j| {
            let (a, b) = (&list[i], &list[j]);
            if out.full() || (a.group.is_some() && a.group == b.group) || a.obj == b.obj {
                return;
            }
            let (x, y) = if a.obj <= b.obj { (a.obj, b.obj) } else { (b.obj, a.obj) };
            let at = marker(a.shape.bbox(), b.shape.bbox(), 0);
            let same_net = a.net.is_some() && a.net == b.net;
            if a.shape.gap_cmp(&b.shape, 0) != Ordering::Greater {
                if !same_net && seen.insert((x, y, ViolationCode::MaskBridge)) {
                    out.push(violation(ViolationCode::MaskBridge, Some(layer), vec![at], vec![x, y], Some(format!("{} / {}", ctx.board.net_name(a.net), ctx.board.net_name(b.net))), Some(0), Some(web), format!("the solder mask openings of {} and {} merge, leaving no mask between different nets", a.label, b.label)));
                }
            } else if a.shape.gap_cmp(&b.shape, web) == Ordering::Less && seen.insert((x, y, ViolationCode::MaskSliver)) {
                let g = a.shape.gap_floor(&b.shape);
                out.push(violation(ViolationCode::MaskSliver, Some(layer), vec![at], vec![x, y], None, Some(g), Some(web), format!("solder mask between {} and {} is {} mm wide, the minimum is {} mm", a.label, b.label, mm(g), mm(web))));
            }
        });
    }
    Ok(())
}

pub(crate) fn check_courtyards(ctx: &Ctx, out: &mut Out) -> Result<()> {
    if !ctx.rules.check_courtyards {
        return Ok(());
    }
    let mut list = Vec::new();
    for fp in &ctx.board.footprints {
        if let Some(c) = &fp.courtyard {
            list.push((fp, place_region(fp, c)?));
        }
    }
    let boxes: Vec<Rect> = list.iter().map(|(_, r)| r.bbox()).collect();
    let cell = Grid::choose_cell(&boxes, MM);
    let grid = Grid::new(boxes, cell);
    grid.for_each_pair(|i, j| {
        let ((fa, ra), (fb, rb)) = (&list[i], &list[j]);
        if out.full() || fa.side != fb.side {
            return;
        }
        if rings_overlap_interior(&ra.outer, &rb.outer) {
            let layer = fa.side.courtyard();
            out.push(violation(
                ViolationCode::CourtyardOverlap,
                Some(layer),
                vec![marker(ra.bbox(), rb.bbox(), 0)],
                vec![
                    ObjRef {
                        kind: ObjKind::Footprint,
                        id: fa.id,
                    },
                    ObjRef {
                        kind: ObjKind::Footprint,
                        id: fb.id,
                    },
                ],
                None,
                None,
                None,
                format!(
                    "the courtyards of {} and {} overlap",
                    fp_label(fa),
                    fp_label(fb)
                ),
            ));
        }
    });
    Ok(())
}

pub(crate) fn check_zones(ctx: &Ctx, out: &mut Out) {
    for z in &ctx.board.zones {
        if z.filled.is_empty() {
            out.push(violation(
                ViolationCode::ZoneUnfilled,
                Some(z.layer),
                vec![z.outline.outer.first().copied().unwrap_or(Point::ORIGIN)],
                vec![ObjRef { kind: ObjKind::Zone, id: z.id }],
                Some(ctx.board.net_name(z.net)),
                None,
                None,
                format!("zone {} has no fill: it is ignored by connectivity, clearance checks and Gerber output until a fill is supplied", z.id.0),
            ));
        }
    }
}

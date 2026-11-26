// SPDX-License-Identifier: MIT OR Apache-2.0
//! Copper items for the checks: tracks, pads, vias and zone fills as shapes with layers and nets.

use crate::board::{Board, NetId, Pad, PadKind, TrackGeom};
use crate::drc::rules::Rules;
use crate::drc::violation::{ObjKind, ObjRef};
use crate::error::Result;
use crate::geom::point::{Point, Rect};
use crate::geom::shape::Shape;
use crate::layer::Layer;
use crate::transform::{pad_center, pad_shape};
use crate::units::Nm;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum CopperKind {
    Track,
    Pad,
    Via,
    Zone,
}

pub(crate) struct Item {
    pub kind: CopperKind,
    pub obj: ObjRef,
    /// Bit `i` set when the item has copper on the board's copper layer `i`.
    pub mask: u32,
    pub net: Option<NetId>,
    /// Pads of one footprint with the same number share a group.
    pub group: Option<u32>,
    pub shapes: Vec<Shape>,
    pub boxes: Vec<Rect>,
    pub bbox: Rect,
    pub at: Point,
    pub label: String,
    /// Largest clearance demanded by the item's own net class or zone.
    pub own_clearance: Nm,
    pub thermal_none: bool,
}

impl Item {
    pub fn same_electrical(&self, o: &Item) -> bool {
        (self.net.is_some() && self.net == o.net) || (self.group.is_some() && self.group == o.group)
    }

    pub fn layer(&self, stack: &[Layer]) -> Layer {
        stack
            .get(self.mask.trailing_zeros() as usize)
            .copied()
            .unwrap_or(Layer::FCu)
    }

    pub fn common_layer(&self, o: &Item, stack: &[Layer]) -> Layer {
        let m = self.mask & o.mask;
        stack
            .get(m.trailing_zeros() as usize)
            .copied()
            .unwrap_or(Layer::FCu)
    }
}

fn mask_of(board: &Board, layers: impl Iterator<Item = Layer>) -> u32 {
    layers
        .filter_map(|l| l.copper_index(board.copper_layers))
        .fold(0u32, |m, i| m | (1u32 << i))
}

fn finish(
    kind: CopperKind,
    obj: ObjRef,
    mask: u32,
    net: Option<NetId>,
    shapes: Vec<Shape>,
    at: Point,
    label: String,
) -> Item {
    let boxes: Vec<Rect> = shapes.iter().map(Shape::bbox).collect();
    let bbox = boxes
        .iter()
        .copied()
        .reduce(Rect::union)
        .unwrap_or(Rect::from_point(at));
    Item {
        kind,
        obj,
        mask,
        net,
        group: None,
        shapes,
        boxes,
        bbox,
        at,
        label,
        own_clearance: 0,
        thermal_none: false,
    }
}

pub(crate) fn pad_label(fp: &crate::board::Footprint, pad: &Pad) -> String {
    format!("pad {}.{}", crate::artwork::fp_label(fp), pad.number)
}

/// Builds the copper items of a validated board.
pub(crate) fn build_items(board: &Board, rules: &Rules) -> Result<Vec<Item>> {
    let groups = board.pad_groups();
    let mut items = Vec::new();
    let class_clear =
        |net: Option<NetId>| board.net_class(net).and_then(|c| c.clearance).unwrap_or(0);

    for t in &board.tracks {
        let shapes = match &t.geom {
            TrackGeom::Segment { start, end } => vec![Shape::segment(*start, *end, t.width)],
            TrackGeom::Arc(a) => {
                crate::artwork::polyline_shapes(&a.approximate(rules.arc_tolerance)?, t.width)
            }
        };
        let (a, b) = t.endpoints();
        let at = Point::new(
            ((a.x as i128 + b.x as i128) / 2) as i64,
            ((a.y as i128 + b.y as i128) / 2) as i64,
        );
        let mut it = finish(
            CopperKind::Track,
            ObjRef {
                kind: ObjKind::Track,
                id: t.id,
            },
            mask_of(board, std::iter::once(t.layer)),
            t.net,
            shapes,
            at,
            format!("track {}", t.id.0),
        );
        it.own_clearance = class_clear(t.net);
        items.push(it);
    }
    for v in &board.vias {
        let layers = board.copper_stack();
        let (Some(a), Some(b)) = (
            v.from.copper_index(board.copper_layers),
            v.to.copper_index(board.copper_layers),
        ) else {
            continue;
        };
        let mask = layers
            .iter()
            .enumerate()
            .filter(|(i, _)| *i >= a && *i <= b)
            .fold(0u32, |m, (i, _)| m | (1 << i));
        let mut it = finish(
            CopperKind::Via,
            ObjRef {
                kind: ObjKind::Via,
                id: v.id,
            },
            mask,
            v.net,
            vec![Shape::point(v.position, v.diameter)],
            v.position,
            format!("via {}", v.id.0),
        );
        it.own_clearance = class_clear(v.net);
        items.push(it);
    }
    for fp in &board.footprints {
        for pad in &fp.pads {
            if pad.kind == PadKind::Npth {
                continue;
            }
            let Some(shape) = pad_shape(fp, pad, 0)? else {
                continue;
            };
            let mask = mask_of(
                board,
                crate::transform::pad_copper_layers(fp, pad, board.copper_layers).into_iter(),
            );
            let mut it = finish(
                CopperKind::Pad,
                ObjRef {
                    kind: ObjKind::Pad,
                    id: pad.id,
                },
                mask,
                pad.net,
                vec![shape],
                pad_center(fp, pad)?,
                pad_label(fp, pad),
            );
            it.group = groups.get(&(fp.id, pad.number.as_str())).copied();
            it.own_clearance = class_clear(pad.net);
            items.push(it);
        }
    }
    for z in &board.zones {
        for (i, f) in z.filled.iter().enumerate() {
            let at = f.outer.first().copied().unwrap_or(Point::ORIGIN);
            let mut it = finish(
                CopperKind::Zone,
                ObjRef {
                    kind: ObjKind::Zone,
                    id: z.id,
                },
                mask_of(board, std::iter::once(z.layer)),
                z.net,
                vec![Shape::region(f.clone(), 0)],
                at,
                format!("zone {} fill {}", z.id.0, i),
            );
            it.own_clearance = class_clear(z.net).max(z.clearance);
            it.thermal_none = z.thermal.mode == crate::board::ThermalMode::None;
            items.push(it);
        }
    }
    Ok(items)
}

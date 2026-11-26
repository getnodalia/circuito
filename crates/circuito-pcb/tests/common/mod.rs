// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::field_reassign_with_default
)]
//! Helpers shared by the integration tests: a terse way to build small boards.

use circuito_pcb::board::*;
use circuito_pcb::geom::{Arc, Point, Region};
use circuito_pcb::layer::{Layer, Side};
use circuito_pcb::units::{Nm, MM};

static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A process-wide fresh object id (ids only need to be unique inside one board).
pub fn fresh() -> Id {
    Id(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

pub fn p(x: Nm, y: Nm) -> Point {
    Point::new(x, y)
}

/// A 2-layer board with a 50 x 50 mm outline from (0,0) to (50 mm, 50 mm) and nets A, B, C, GND.
pub fn base_board() -> Board {
    let mut b = Board::new("test", 2).unwrap();
    b.outline = Some(Outline::rectangle(0, 0, 50 * MM, 50 * MM));
    for (i, n) in ["A", "B", "C", "GND"].iter().enumerate() {
        b.nets.push(Net {
            id: NetId(i as u32 + 1),
            name: n.to_string(),
            class: DEFAULT_CLASS.to_string(),
        });
    }
    b
}

pub const NA: Option<NetId> = Some(NetId(1));
pub const NB: Option<NetId> = Some(NetId(2));
pub const NC: Option<NetId> = Some(NetId(3));
pub const GND: Option<NetId> = Some(NetId(4));

pub fn track(b: &mut Board, net: Option<NetId>, layer: Layer, width: Nm, a: Point, c: Point) -> Id {
    let id = fresh();
    b.tracks.push(Track {
        id,
        net,
        layer,
        width,
        geom: TrackGeom::Segment { start: a, end: c },
    });
    id
}

pub fn arc_track(b: &mut Board, net: Option<NetId>, layer: Layer, width: Nm, arc: Arc) -> Id {
    let id = fresh();
    b.tracks.push(Track {
        id,
        net,
        layer,
        width,
        geom: TrackGeom::Arc(arc),
    });
    id
}

pub fn via(b: &mut Board, net: Option<NetId>, at: Point, diameter: Nm, drill: Nm) -> Id {
    let id = fresh();
    b.vias.push(Via {
        id,
        net,
        position: at,
        diameter,
        drill,
        kind: ViaKind::Through,
        from: Layer::FCu,
        to: Layer::BCu,
        tented: true,
    });
    id
}

pub fn smd_pad(
    _b: &mut Board,
    number: &str,
    net: Option<NetId>,
    pos: Point,
    w: Nm,
    h: Nm,
    shape: PadShape,
) -> Pad {
    let id = fresh();
    Pad {
        id,
        number: number.into(),
        kind: PadKind::Smd,
        shape,
        position: pos,
        rotation: 0,
        size: Size { w, h },
        drill: None,
        net,
        mask: true,
        paste: true,
        mask_margin: None,
        paste_margin: None,
    }
}

pub fn th_pad(
    _b: &mut Board,
    number: &str,
    net: Option<NetId>,
    pos: Point,
    d: Nm,
    drill: Nm,
) -> Pad {
    let id = fresh();
    Pad {
        id,
        number: number.into(),
        kind: PadKind::ThroughHole,
        shape: PadShape::Circle,
        position: pos,
        rotation: 0,
        size: Size { w: d, h: d },
        drill: Some(Drill {
            diameter: drill,
            slot_length: None,
        }),
        net,
        mask: true,
        paste: false,
        mask_margin: None,
        paste_margin: None,
    }
}

/// Adds a footprint at `at` holding `pads`; ids of the pads must already be unique.
pub fn footprint(b: &mut Board, reference: &str, at: Point, side: Side, pads: Vec<Pad>) -> Id {
    let id = fresh();
    b.footprints.push(Footprint {
        id,
        reference: reference.into(),
        value: "10k".into(),
        package: "PKG".into(),
        position: at,
        rotation: 0,
        side,
        pads,
        courtyard: None,
        graphics: vec![],
        texts: vec![],
        exclude_from_bom: false,
        exclude_from_pos: false,
    });
    id
}

pub fn rect_region(x0: Nm, y0: Nm, x1: Nm, y1: Nm) -> Region {
    Region::polygon(vec![p(x0, y0), p(x1, y0), p(x1, y1), p(x0, y1)]).unwrap()
}

pub fn zone(b: &mut Board, net: Option<NetId>, layer: Layer, filled: Region) -> Id {
    let id = fresh();
    b.zones.push(Zone {
        id,
        net,
        layer,
        outline: filled.clone(),
        clearance: 200_000,
        min_width: 200_000,
        thermal: Thermal::default(),
        priority: 0,
        filled: vec![filled],
    });
    id
}

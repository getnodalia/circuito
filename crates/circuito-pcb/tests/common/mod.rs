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

pub fn pad_at(
    b: &mut Board,
    num: &str,
    net: Option<NetId>,
    pos: Point,
    w: i64,
    h: i64,
    shape: PadShape,
) -> Pad {
    smd_pad(b, num, net, pos, w, h, shape)
}

/// A board with every pad kind, tracks, an arc, vias and a zone with a hole.
pub fn rich_board() -> Board {
    let mut b = base_board();
    let pads = vec![
        pad_at(&mut b, "1", NA, p(-MM, 0), MM, 500_000, PadShape::Rect),
        pad_at(
            &mut b,
            "2",
            NB,
            p(MM, 0),
            800_000,
            800_000,
            PadShape::Circle,
        ),
        pad_at(
            &mut b,
            "3",
            NA,
            p(0, 1_500_000),
            1_200_000,
            600_000,
            PadShape::Oval,
        ),
    ];
    footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, pads);

    let pads = vec![
        pad_at(&mut b, "1", NA, p(0, 0), MM, 500_000, PadShape::Rect),
        pad_at(
            &mut b,
            "2",
            NB,
            p(3 * MM, 0),
            1_400_000,
            600_000,
            PadShape::Oval,
        ),
        pad_at(
            &mut b,
            "3",
            NC,
            p(0, 3 * MM),
            2 * MM,
            MM,
            PadShape::RoundRect {
                corner_radius: 250_000,
            },
        ),
    ];
    let u2 = footprint(&mut b, "U2", p(30 * MM, 20 * MM), Side::Front, pads);
    b.footprints
        .iter_mut()
        .find(|f| f.id == u2)
        .unwrap()
        .rotation = 30_000;

    let pads = vec![pad_at(
        &mut b,
        "1",
        NA,
        p(MM, 0),
        MM,
        400_000,
        PadShape::Rect,
    )];
    let u3 = footprint(&mut b, "U3", p(40 * MM, 40 * MM), Side::Back, pads);
    b.footprints
        .iter_mut()
        .find(|f| f.id == u3)
        .unwrap()
        .rotation = 90_000;

    let pads = vec![th_pad(&mut b, "1", NC, p(0, 0), 1_600_000, 800_000)];
    footprint(&mut b, "J1", p(20 * MM, 40 * MM), Side::Front, pads);

    let mut custom = pad_at(
        &mut b,
        "1",
        GND,
        p(0, 0),
        0,
        0,
        PadShape::Custom {
            polygon: vec![p(0, 0), p(MM, 0), p(0, MM)],
        },
    );
    custom.paste = false;
    footprint(&mut b, "U4", p(5 * MM, 45 * MM), Side::Front, vec![custom]);

    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(10 * MM, 10 * MM),
        p(20 * MM, 10 * MM),
    );
    arc_track(
        &mut b,
        NB,
        Layer::FCu,
        250_000,
        Arc::new(
            p(30 * MM, 10 * MM),
            p(35 * MM, 15 * MM),
            p(30 * MM, 15 * MM),
            true,
        )
        .unwrap(),
    );
    track(
        &mut b,
        NC,
        Layer::BCu,
        300_000,
        p(10 * MM, 30 * MM),
        p(12 * MM, 30 * MM),
    );
    via(&mut b, NA, p(25 * MM, 25 * MM), 600_000, 300_000);
    let outer = vec![
        p(2 * MM, 2 * MM),
        p(8 * MM, 2 * MM),
        p(8 * MM, 8 * MM),
        p(2 * MM, 8 * MM),
    ];
    let hole = vec![
        p(4 * MM, 4 * MM),
        p(5 * MM, 4 * MM),
        p(5 * MM, 5 * MM),
        p(4 * MM, 5 * MM),
    ];
    zone(
        &mut b,
        GND,
        Layer::FCu,
        Region::new(outer, vec![hole]).unwrap(),
    );
    b
}

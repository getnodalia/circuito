// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Board model validation and `.circuito-pcb` round trips.

mod common;
use circuito_pcb::board::*;
use circuito_pcb::error::PcbError;
use circuito_pcb::geom::{Arc, Region};
use circuito_pcb::io::{FORMAT_VERSION, MAX_FILE_BYTES};
use circuito_pcb::layer::{Layer, Side};
use circuito_pcb::units::MM;
use common::*;

fn rich_board() -> Board {
    let mut b = base_board();
    b.net_classes.push(NetClass {
        name: "Power".into(),
        clearance: Some(300_000),
        track_width: Some(500_000),
        min_track_width: Some(300_000),
        via_diameter: Some(800_000),
        via_drill: Some(400_000),
        min_via_diameter: Some(600_000),
        min_via_drill: Some(300_000),
        diff_pair: Some(DiffPair {
            width: 150_000,
            gap: 150_000,
        }),
    });
    b.nets[3].class = "Power".into();
    track(&mut b, NA, Layer::FCu, 200_000, p(MM, MM), p(10 * MM, MM));
    arc_track(
        &mut b,
        NB,
        Layer::BCu,
        200_000,
        Arc::new(
            p(10 * MM, 10 * MM),
            p(11 * MM, 11 * MM),
            p(10 * MM, 11 * MM),
            true,
        )
        .unwrap(),
    );
    via(&mut b, NA, p(10 * MM, MM), 800_000, 400_000);
    let pads = vec![
        smd_pad(&mut b, "1", NA, p(-MM, 0), MM, MM, PadShape::Rect),
        smd_pad(
            &mut b,
            "2",
            NB,
            p(MM, 0),
            MM,
            2 * MM,
            PadShape::RoundRect {
                corner_radius: 200_000,
            },
        ),
        th_pad(&mut b, "3", GND, p(0, 2 * MM), 1_600_000, 800_000),
    ];
    let fid = footprint(&mut b, "U1", p(20 * MM, 20 * MM), Side::Front, pads);
    if let Some(f) = b.footprints.iter_mut().find(|f| f.id == fid) {
        f.courtyard = Some(rect_region(-2 * MM, -2 * MM, 2 * MM, 3 * MM));
        f.graphics.push(Graphic {
            id: fresh(),
            layer: Layer::FSilk,
            kind: GraphicKind::Line {
                a: p(-2 * MM, -2 * MM),
                b: p(2 * MM, -2 * MM),
            },
            width: 150_000,
            filled: false,
        });
        f.texts.push(TextItem {
            id: fresh(),
            layer: Layer::FSilk,
            text: "${REF}".into(),
            position: p(0, -3 * MM),
            height: MM,
            thickness: 150_000,
            rotation: 0,
            mirrored: false,
            justify: Justify::Center,
        });
    }
    zone(
        &mut b,
        GND,
        Layer::BCu,
        rect_region(2 * MM, 2 * MM, 8 * MM, 8 * MM),
    );
    b.keepouts.push(Keepout {
        id: fresh(),
        layers: vec![Layer::FCu],
        outline: rect_region(30 * MM, 30 * MM, 35 * MM, 35 * MM),
        no_tracks: true,
        no_vias: true,
        no_pads: false,
        no_zones: true,
    });
    b.texts.push(TextItem {
        id: fresh(),
        layer: Layer::BSilk,
        text: "REV A".into(),
        position: p(MM, 45 * MM),
        height: MM,
        thickness: 150_000,
        rotation: 0,
        mirrored: true,
        justify: Justify::Left,
    });
    b
}

#[test]
fn round_trip_is_lossless_and_deterministic() {
    let b = rich_board();
    b.validate().unwrap();
    let j = b.to_json().unwrap();
    let back = Board::from_json(&j).unwrap();
    assert_eq!(back, b);
    assert_eq!(back.to_json().unwrap(), j);
    assert!(j.contains("\"format\": \"circuito-pcb\""));
    assert!(j.contains(&format!("\"format_version\": {FORMAT_VERSION}")));
    assert!(j.contains("\"F.Cu\""));
}

#[test]
fn unknown_versions_and_formats_are_refused_clearly() {
    let j = rich_board().to_json().unwrap();
    let v2 = j.replace("\"format_version\": 1", "\"format_version\": 2");
    match Board::from_json(&v2) {
        Err(PcbError::UnsupportedVersion {
            found: 2,
            supported: 1,
            ..
        }) => {}
        other => panic!("{other:?}"),
    }
    assert!(Board::from_json(&j.replace("\"circuito-pcb\"", "\"other\"")).is_err());
    assert!(Board::from_json(&j.replace("\"format_version\": 1,", "")).is_err());
    assert!(
        Board::from_json(&j.replace("\"format_version\": 1", "\"format_version\": \"1\"")).is_err()
    );
    assert!(
        Board::from_json(&j.replace("\"format_version\": 1", "\"format_version\": -1")).is_err()
    );
}

#[test]
fn malformed_inputs_are_errors_not_panics() {
    let j = rich_board().to_json().unwrap();
    // every truncation of the document must fail to parse, never panic
    for cut in (0..j.len()).step_by(37) {
        if j.is_char_boundary(cut) {
            assert!(Board::from_json(&j[..cut]).is_err(), "cut at {cut}");
        }
    }
    for bad in [
        "",
        "null",
        "[]",
        "{}",
        "{\"format\":\"circuito-pcb\"}",
        "\u{0}",
        "{\"format\":\"circuito-pcb\",\"format_version\":1}",
        "{\"format\":\"circuito-pcb\",\"format_version\":1,\"board\":42}",
    ] {
        assert!(Board::from_json(bad).is_err(), "{bad:?}");
    }
    // unknown field
    assert!(Board::from_json(
        &j.replace("\"copper_layers\": 2", "\"copper_layers\": 2, \"bogus\": 1")
    )
    .is_err());
    // coordinate beyond range
    let mut over = rich_board();
    over.tracks[0].geom = TrackGeom::Segment {
        start: p(MM, MM),
        end: p(i64::MAX / 2, 0),
    };
    let text = serde_json::to_string(
        &serde_json::json!({"format": "circuito-pcb", "format_version": 1, "board": over}),
    )
    .unwrap();
    assert!(Board::from_json(&text).is_err());
    // deep nesting
    let deep = format!("{}1{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(Board::from_json(&deep).is_err());
    // oversize
    let big = " ".repeat(MAX_FILE_BYTES + 1);
    assert!(matches!(
        Board::from_json(&big),
        Err(PcbError::TooLarge { .. })
    ));
}

#[test]
fn hostile_numbers_are_rejected() {
    let mut b = rich_board();
    b.tracks[0].width = i64::MAX;
    assert!(b.validate().is_err());
    let mut b = rich_board();
    b.tracks[0].width = 0;
    assert!(b.validate().is_err());
    let mut b = rich_board();
    b.tracks[0].geom = TrackGeom::Segment {
        start: p(i64::MIN, 0),
        end: p(0, 0),
    };
    assert!(b.validate().is_err());
    let mut b = rich_board();
    b.footprints[0].rotation = i32::MAX;
    assert!(b.validate().is_err());
    let mut b = rich_board();
    b.vias[0].drill = b.vias[0].diameter + 1;
    assert!(b.validate().is_err());
}

#[test]
fn reference_and_id_checks() {
    let mut b = rich_board();
    let dup = b.tracks[1].id;
    b.tracks[0].id = dup;
    assert!(matches!(b.validate(), Err(PcbError::Invalid(m)) if m.contains("used twice")));
    let mut b = rich_board();
    b.tracks[0].net = Some(NetId(99));
    assert!(matches!(b.validate(), Err(PcbError::Invalid(m)) if m.contains("unknown net")));
    let mut b = rich_board();
    b.tracks[0].layer = Layer::InnerCu(1);
    assert!(b.validate().is_err(), "inner layer on a 2-layer board");
    let mut b = rich_board();
    b.tracks[0].layer = Layer::FSilk;
    assert!(b.validate().is_err());
    let mut b = rich_board();
    b.nets[0].name = b.nets[1].name.clone();
    assert!(b.validate().is_err());
    let mut b = rich_board();
    b.nets[0].class = "Nope".into();
    assert!(b.validate().is_err());
}

#[test]
fn via_kinds_are_checked_against_the_stack() {
    let mut b = Board::new("six", 6).unwrap();
    b.nets.push(Net {
        id: NetId(1),
        name: "A".into(),
        class: DEFAULT_CLASS.into(),
    });
    let mk = |kind, from, to| Via {
        id: fresh(),
        net: None,
        position: p(0, 0),
        diameter: 600_000,
        drill: 300_000,
        kind,
        from,
        to,
        tented: true,
    };
    let cases = [
        (ViaKind::Through, Layer::FCu, Layer::BCu, true),
        (ViaKind::Through, Layer::FCu, Layer::InnerCu(2), false),
        (ViaKind::Blind, Layer::FCu, Layer::InnerCu(2), true),
        (ViaKind::Blind, Layer::InnerCu(3), Layer::BCu, true),
        (ViaKind::Blind, Layer::InnerCu(1), Layer::InnerCu(2), false),
        (ViaKind::Buried, Layer::InnerCu(1), Layer::InnerCu(3), true),
        (ViaKind::Buried, Layer::FCu, Layer::InnerCu(3), false),
        (ViaKind::Micro, Layer::FCu, Layer::InnerCu(1), true),
        (ViaKind::Micro, Layer::FCu, Layer::InnerCu(2), false),
        (ViaKind::Micro, Layer::InnerCu(4), Layer::BCu, true),
        (ViaKind::Through, Layer::BCu, Layer::FCu, false),
        (ViaKind::Blind, Layer::FCu, Layer::InnerCu(5), false),
    ];
    for (k, f, t, ok) in cases {
        let mut bb = b.clone();
        bb.vias.push(mk(k, f, t));
        assert_eq!(bb.validate().is_ok(), ok, "{k:?} {f}..{t}");
    }
}

#[test]
fn pad_rules() {
    let mut b = rich_board();
    b.footprints[0].pads[0].drill = Some(Drill {
        diameter: 300_000,
        slot_length: None,
    });
    assert!(b.validate().is_err(), "SMD with drill");
    let mut b = rich_board();
    b.footprints[0].pads[2].drill = None;
    assert!(b.validate().is_err(), "through hole without drill");
    let mut b = rich_board();
    b.footprints[0].pads[1].shape = PadShape::RoundRect {
        corner_radius: MM + 1,
    };
    assert!(b.validate().is_err(), "radius above half the shorter side");
    let mut b = rich_board();
    b.footprints[0].pads[1].shape = PadShape::Circle;
    assert!(b.validate().is_err(), "non-square circle");
    let mut b = rich_board();
    b.footprints[0].pads[0].shape = PadShape::Custom {
        polygon: vec![p(0, 0), p(10, 10), p(10, 0), p(0, 10)],
    };
    assert!(b.validate().is_err(), "bow-tie custom pad");
}

#[test]
fn outline_must_close() {
    let mut b = rich_board();
    if let Some(o) = &mut b.outline {
        o.outer[1] = OutlineSeg::Line {
            start: p(MM, 0),
            end: p(50 * MM, 50 * MM),
        };
    }
    assert!(b.validate().is_err());
    let mut b = rich_board();
    b.outline = Some(Outline {
        outer: Outline::rectangle(0, 0, 50 * MM, 50 * MM).outer,
        cutouts: vec![Outline::rectangle(10 * MM, 10 * MM, 20 * MM, 20 * MM).outer],
    });
    b.validate().unwrap();
    let reg = b.outline.as_ref().unwrap().to_region(1000).unwrap();
    assert_eq!(reg.area2(), 2 * (2500 - 100) * 1_000_000i128 * 1_000_000);
    // cut-out outside the board
    b.outline = Some(Outline {
        outer: Outline::rectangle(0, 0, 50 * MM, 50 * MM).outer,
        cutouts: vec![Outline::rectangle(60 * MM, 60 * MM, 70 * MM, 70 * MM).outer],
    });
    assert!(b.validate().is_err());
    // rounded rectangle with arcs
    let r = 2 * MM;
    let corner = |cx: i64, cy: i64, s: (i64, i64), e: (i64, i64)| {
        OutlineSeg::Arc(
            Arc::new(
                p(cx + s.0, cy + s.1),
                p(cx + e.0, cy + e.1),
                p(cx, cy),
                true,
            )
            .unwrap(),
        )
    };
    let outer = vec![
        OutlineSeg::Line {
            start: p(r, 0),
            end: p(50 * MM - r, 0),
        },
        corner(50 * MM - r, r, (0, -r), (r, 0)),
        OutlineSeg::Line {
            start: p(50 * MM, r),
            end: p(50 * MM, 50 * MM - r),
        },
        corner(50 * MM - r, 50 * MM - r, (r, 0), (0, r)),
        OutlineSeg::Line {
            start: p(50 * MM - r, 50 * MM),
            end: p(r, 50 * MM),
        },
        corner(r, 50 * MM - r, (0, r), (-r, 0)),
        OutlineSeg::Line {
            start: p(0, 50 * MM - r),
            end: p(0, r),
        },
        corner(r, r, (-r, 0), (0, -r)),
    ];
    b.outline = Some(Outline {
        outer,
        cutouts: vec![],
    });
    b.validate().unwrap();
}

#[test]
fn region_json_is_compact_pairs() {
    let r = Region::polygon(vec![p(0, 0), p(10, 0), p(10, 10)]).unwrap();
    let s = serde_json::to_string(&r).unwrap();
    assert_eq!(s, "{\"outer\":[[0,0],[10,0],[10,10]]}");
}

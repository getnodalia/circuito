// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::field_reassign_with_default
)]
//! Gerber output read back with our own reader: exact coordinates, apertures, regions, arcs,
//! attributes, and what the writer refuses.

mod common;
use circuito_pcb::board::*;
use circuito_pcb::geom::{Arc, Point};
use circuito_pcb::gerber::{
    read_gerber, write_all, write_layer, GKind, GerberImage, GerberOptions, Polarity,
};
use circuito_pcb::layer::{Layer, Side};
use circuito_pcb::units::{MAX_COORD, MM};
use common::*;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn read(b: &Board, l: Layer) -> GerberImage {
    let (f, _) = write_layer(b, l, &GerberOptions::default()).unwrap();
    read_gerber(&f.content).unwrap_or_else(|e| panic!("{e}\n{}", f.content))
}

fn apertures(g: &GerberImage) -> Vec<String> {
    let mut v: Vec<String> = g
        .apertures
        .iter()
        .map(|a| {
            format!(
                "{}|{}|{}",
                a.template,
                a.params.join("X"),
                a.function.clone().unwrap_or_default()
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn front_copper_round_trip_is_exact() {
    let b = rich_board();
    let g = read(&b, Layer::FCu);
    assert!(g.finished);
    assert!(!g.inch);
    assert_eq!((g.int_digits, g.dec_digits), (4, 6));
    assert_eq!(
        g.file_attr("FileFunction").unwrap(),
        ["Copper", "L1", "Top"]
    );
    assert_eq!(g.file_attr("FilePolarity").unwrap(), ["Positive"]);
    assert_eq!(g.file_attr("SameCoordinates").unwrap(), ["Original"]);
    assert_eq!(
        g.file_attr("GenerationSoftware").unwrap(),
        ["Nodalia", "Circuito", VERSION]
    );
    assert!(g.file_attr("CreationDate").is_none());

    let mut expect = vec![
        "C|0.200000|Conductor",
        "C|0.250000|Conductor",
        "C|0.600000|SMDPad,CuDef", // the oval at 30 degrees, as one stroke
        "C|0.600000|ViaPad",
        "C|0.800000|SMDPad,CuDef",
        "C|1.600000|ComponentPad",
        "O|1.200000X0.600000|SMDPad,CuDef",
        "R|1.000000X0.500000|SMDPad,CuDef",
        "RECTROT|1.000000X0.500000X30.000|SMDPad,CuDef",
        "RRECT|2.000000X1.000000X0.250000X30.000|SMDPad,CuDef",
    ];
    expect.sort();
    assert_eq!(apertures(&g), expect);
    assert_eq!(
        g.macros.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["RECTROT", "RRECT"]
    );

    // flashes: exact centres
    let mut got: Vec<(i64, i64, String)> = g
        .flashes()
        .iter()
        .map(|(at, a)| (at.x, at.y, a.template.clone()))
        .collect();
    got.sort();
    let mut want = vec![
        (9 * MM, 10 * MM, "R"),
        (11 * MM, 10 * MM, "C"),
        (10 * MM, 11_500_000, "O"),
        (30 * MM, 20 * MM, "RECTROT"),
        // the 2 x 1 mm round rectangle at (0, 3 mm) in a footprint turned by 30 degrees
        (30 * MM - 1_500_000, 20 * MM + 2_598_076, "RRECT"),
        (20 * MM, 40 * MM, "C"),
        (25 * MM, 25 * MM, "C"),
    ]
    .into_iter()
    .map(|(x, y, t)| (x, y, t.to_string()))
    .collect::<Vec<_>>();
    want.sort();
    assert_eq!(got, want);

    // draws: the straight track and the oval pad at 30 degrees (a 0.8 mm stroke of a 0.6 mm pen)
    let draws = g.draws();
    assert_eq!(draws.len(), 2);
    let track = draws
        .iter()
        .find(|(_, _, a)| a.params == ["0.200000"])
        .unwrap();
    assert_eq!(
        (track.0, track.1),
        (p(10 * MM, 10 * MM), p(20 * MM, 10 * MM))
    );
    let oval = draws
        .iter()
        .find(|(_, _, a)| a.params == ["0.600000"])
        .unwrap();
    // pad centre (30 mm + 3 mm * cos 30, 20 mm + 3 mm * sin 30); half stroke = 0.4 mm
    assert_eq!(
        oval.0,
        p(30 * MM + 2_598_076 - 346_410, 20 * MM + 1_500_000 - 200_000)
    );
    assert_eq!(
        oval.1,
        p(30 * MM + 2_598_076 + 346_410, 20 * MM + 1_500_000 + 200_000)
    );

    // the arc is a true G03 arc
    let arcs: Vec<_> = g
        .objects
        .iter()
        .filter(|o| matches!(o.kind, GKind::Arc { .. }))
        .collect();
    assert_eq!(arcs.len(), 1);
    assert_eq!(
        arcs[0].kind,
        GKind::Arc {
            from: p(30 * MM, 10 * MM),
            to: p(35 * MM, 15 * MM),
            center: p(30 * MM, 15 * MM),
            ccw: true
        }
    );

    // regions: the zone (dark), its hole (clear) and the custom pad (dark)
    let regions = g.regions();
    assert_eq!(regions.len(), 3);
    assert_eq!(regions[0].1, Polarity::Dark);
    let r = regions[0].0.bounds().unwrap();
    assert_eq!((r.min, r.max), (p(2 * MM, 2 * MM), p(8 * MM, 8 * MM)));
    assert_eq!(regions[1].1, Polarity::Clear);
    let r = regions[1].0.bounds().unwrap();
    assert_eq!((r.min, r.max), (p(4 * MM, 4 * MM), p(5 * MM, 5 * MM)));
    assert_eq!(regions[2].1, Polarity::Dark);
    let r = regions[2].0.bounds().unwrap();
    assert_eq!((r.min, r.max), (p(5 * MM, 45 * MM), p(6 * MM, 46 * MM)));
    assert_eq!(regions[2].0.contours[0].len(), 3);
}

#[test]
fn object_attributes_name_net_component_and_pin() {
    let b = rich_board();
    let g = read(&b, Layer::FCu);
    let rect = g
        .objects
        .iter()
        .find(|o| matches!(o.kind, GKind::Flash { at } if at == p(9 * MM, 10 * MM)))
        .unwrap();
    let get = |n: &str| {
        rect.attrs
            .iter()
            .find(|(k, _)| k == n)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(get("N"), Some(vec!["A".into()]));
    assert_eq!(get("C"), Some(vec!["U1".into()]));
    assert_eq!(get("P"), Some(vec!["U1".into(), "1".into()]));
    // tracks carry the net only
    let tr = g
        .objects
        .iter()
        .find(|o| matches!(o.kind, GKind::Draw { from, .. } if from == p(10 * MM, 10 * MM)))
        .unwrap();
    assert!(tr
        .attrs
        .iter()
        .any(|(k, v)| k == "N" && v == &["A".to_string()]));
    assert!(tr.attrs.iter().all(|(k, _)| k != "P" && k != "C"));
}

#[test]
fn net_and_reference_text_is_escaped_and_read_back() {
    let mut b = base_board();
    b.nets[0].name = "A,B*C%\u{e9}".into();
    let pads = vec![pad_at(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
    footprint(&mut b, "R,1", p(10 * MM, 10 * MM), Side::Front, pads);
    let (f, _) = write_layer(&b, Layer::FCu, &GerberOptions::default()).unwrap();
    assert!(!f.content.contains("A,B"), "{}", f.content);
    let g = read_gerber(&f.content).unwrap();
    let o = &g.objects[0];
    assert!(o
        .attrs
        .iter()
        .any(|(k, v)| k == "N" && v == &["A,B*C%\u{e9}".to_string()]));
    assert!(o
        .attrs
        .iter()
        .any(|(k, v)| k == "P" && v == &["R,1".to_string(), "1".to_string()]));
}

#[test]
fn coordinates_are_written_as_exact_nanometres() {
    let mut b = base_board();
    b.outline = Some(Outline::rectangle(
        -MAX_COORD, -MAX_COORD, MAX_COORD, MAX_COORD,
    ));
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(1, -2),
        p(1_234_567, -7_654_321),
    );
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(-MAX_COORD, MAX_COORD),
        p(MAX_COORD, -MAX_COORD),
    );
    let (f, _) = write_layer(&b, Layer::FCu, &GerberOptions::default()).unwrap();
    assert!(f.content.contains("X1Y-2D02*"));
    assert!(f.content.contains("X1234567Y-7654321D01*"));
    assert!(f
        .content
        .contains(&format!("X-{MAX_COORD}Y{MAX_COORD}D02*")));
    let g = read_gerber(&f.content).unwrap();
    let d = g.draws();
    assert!(d
        .iter()
        .any(|(a, c, _)| *a == p(1, -2) && *c == p(1_234_567, -7_654_321)));
    assert!(d
        .iter()
        .any(|(a, c, _)| *a == p(-MAX_COORD, MAX_COORD) && *c == p(MAX_COORD, -MAX_COORD)));
    // an aperture of 1 nm width is still exact
    let mut b = base_board();
    track(&mut b, NA, Layer::FCu, 1, p(0, 0), p(MM, 0));
    let g = read(&b, Layer::FCu);
    assert_eq!(g.apertures[0].params, ["0.000001"]);
}

#[test]
fn back_side_footprints_flash_on_the_back_mirrored() {
    let b = rich_board();
    let front = read(&b, Layer::FCu);
    let back = read(&b, Layer::BCu);
    assert_eq!(
        back.file_attr("FileFunction").unwrap(),
        ["Copper", "L2", "Bot"]
    );
    // U3 is mirrored: pad at x = +1 mm lands at 39 mm; footprint rotated 90 degrees, pad
    // rotation 0 on the back: a 1.0 x 0.4 mm rectangle stands up as 0.4 x 1.0
    let f = back.flashes();
    let u3 = f
        .iter()
        .find(|(at, _)| *at == p(40 * MM, 39 * MM))
        .unwrap_or_else(|| panic!("{f:?}"));
    assert_eq!(u3.1.template, "R");
    assert_eq!(u3.1.params, ["0.400000", "1.000000"]);
    assert!(front
        .flashes()
        .iter()
        .all(|(at, _)| *at != p(40 * MM, 39 * MM)));
    // through-hole pads and vias are on both sides, a front SMD pad only on the front
    for g in [&front, &back] {
        assert!(g
            .flashes()
            .iter()
            .any(|(at, a)| *at == p(20 * MM, 40 * MM) && a.params == ["1.600000"]));
        assert!(g.flashes().iter().any(|(at, _)| *at == p(25 * MM, 25 * MM)));
    }
    assert!(!back
        .flashes()
        .iter()
        .any(|(at, _)| *at == p(9 * MM, 10 * MM)));
    // the back track
    assert!(back
        .draws()
        .iter()
        .any(|(a, c, _)| *a == p(10 * MM, 30 * MM) && *c == p(12 * MM, 30 * MM)));
}

#[test]
fn inner_layers_carry_through_pads_and_buried_vias_only_where_they_go() {
    let mut b = Board::new("four", 4).unwrap();
    b.outline = Some(Outline::rectangle(0, 0, 50 * MM, 50 * MM));
    b.nets.push(Net {
        id: NetId(1),
        name: "A".into(),
        class: DEFAULT_CLASS.to_string(),
    });
    let pads = vec![th_pad(&mut b, "1", NA, p(0, 0), 1_600_000, 800_000)];
    footprint(&mut b, "J1", p(10 * MM, 10 * MM), Side::Front, pads);
    for (kind, from, to, at) in [
        (ViaKind::Blind, Layer::FCu, Layer::InnerCu(1), 20 * MM),
        (
            ViaKind::Buried,
            Layer::InnerCu(1),
            Layer::InnerCu(2),
            30 * MM,
        ),
        (ViaKind::Through, Layer::FCu, Layer::BCu, 40 * MM),
    ] {
        let id = via(&mut b, NA, p(at, 20 * MM), 600_000, 300_000);
        let v = b.vias.iter_mut().find(|v| v.id == id).unwrap();
        v.kind = kind;
        v.from = from;
        v.to = to;
    }
    let has = |g: &GerberImage, x: i64| g.flashes().iter().any(|(a, _)| a.x == x);
    let l: Vec<_> = [Layer::FCu, Layer::InnerCu(1), Layer::InnerCu(2), Layer::BCu]
        .iter()
        .map(|l| read(&b, *l))
        .collect();
    assert_eq!(
        l[1].file_attr("FileFunction").unwrap(),
        ["Copper", "L2", "Inr"]
    );
    assert_eq!(
        l[2].file_attr("FileFunction").unwrap(),
        ["Copper", "L3", "Inr"]
    );
    let present = |x: i64| l.iter().map(|g| has(g, x)).collect::<Vec<_>>();
    assert_eq!(present(10 * MM), [true, true, true, true]);
    assert_eq!(present(20 * MM), [true, true, false, false]);
    assert_eq!(present(30 * MM), [false, true, true, false]);
    assert_eq!(present(40 * MM), [true, true, true, true]);
}

#[test]
fn solder_mask_and_paste_follow_margins() {
    let mut b = rich_board();
    // a per-pad mask margin, a pad without mask, an un-tented via, a tented one
    b.footprints[0].pads[1].mask_margin = Some(0);
    b.footprints[0].pads[2].mask = false;
    let id = via(&mut b, NB, p(26 * MM, 26 * MM), 600_000, 300_000);
    b.vias.iter_mut().find(|v| v.id == id).unwrap().tented = false;
    b.footprints[0].pads[0].paste_margin = Some(-50_000);

    let m = read(&b, Layer::FMask);
    assert_eq!(m.file_attr("FileFunction").unwrap(), ["Soldermask", "Top"]);
    assert_eq!(m.file_attr("FilePolarity").unwrap(), ["Negative"]);
    let fl = m.flashes();
    let at = |x: i64, y: i64| {
        fl.iter()
            .find(|(a, _)| *a == p(x, y))
            .map(|(_, a)| format!("{}|{}", a.template, a.params.join("X")))
    };
    assert_eq!(at(9 * MM, 10 * MM).as_deref(), Some("R|1.100000X0.600000"));
    assert_eq!(at(11 * MM, 10 * MM).as_deref(), Some("C|0.800000")); // margin 0
    assert_eq!(at(10 * MM, 11_500_000), None); // mask disabled
    assert_eq!(at(26 * MM, 26 * MM).as_deref(), Some("C|0.700000"));
    assert_eq!(at(25 * MM, 25 * MM), None); // tented
                                            // through-hole pads are open on both sides, back SMD pads only on the back
    assert!(at(20 * MM, 40 * MM).is_some());
    assert!(read(&b, Layer::BMask)
        .flashes()
        .iter()
        .any(|(a, _)| *a == p(20 * MM, 40 * MM)));
    assert!(read(&b, Layer::BMask)
        .flashes()
        .iter()
        .any(|(a, _)| *a == p(40 * MM, 39 * MM)));
    assert!(read(&b, Layer::FMask)
        .flashes()
        .iter()
        .all(|(a, _)| *a != p(40 * MM, 39 * MM)));
    // the custom pad: a region plus a 0.1 mm stroke all round
    assert_eq!(m.regions().len(), 1);
    assert!(m.draws().iter().any(|(_, _, a)| a.params == ["0.100000"]));

    let pst = read(&b, Layer::FPaste);
    assert_eq!(pst.file_attr("FileFunction").unwrap(), ["Paste", "Top"]);
    assert_eq!(pst.file_attr("FilePolarity").unwrap(), ["Positive"]);
    let fl = pst.flashes();
    assert!(fl
        .iter()
        .any(|(a, ap)| *a == p(9 * MM, 10 * MM) && ap.params == ["0.900000", "0.400000"]));
    assert!(fl
        .iter()
        .any(|(a, ap)| *a == p(11 * MM, 10 * MM) && ap.params == ["0.800000"]));
    // through-hole and custom (paste disabled) pads get no paste
    assert!(fl.iter().all(|(a, _)| *a != p(20 * MM, 40 * MM)));
    assert!(pst.regions().is_empty());
}

#[test]
fn silkscreen_strokes_arcs_circles_and_text() {
    let mut b = base_board();
    let pads = vec![pad_at(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
    let fid = footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, pads);
    let f = b.footprints.iter_mut().find(|f| f.id == fid).unwrap();
    f.graphics.push(Graphic {
        id: fresh(),
        layer: Layer::FSilk,
        kind: GraphicKind::Line {
            a: p(-MM, 2 * MM),
            b: p(MM, 2 * MM),
        },
        width: 150_000,
        filled: false,
    });
    f.graphics.push(Graphic {
        id: fresh(),
        layer: Layer::FSilk,
        kind: GraphicKind::Circle {
            center: p(0, -3 * MM),
            radius: 500_000,
        },
        width: 120_000,
        filled: false,
    });
    f.graphics.push(Graphic {
        id: fresh(),
        layer: Layer::FSilk,
        kind: GraphicKind::Circle {
            center: p(3 * MM, 0),
            radius: 200_000,
        },
        width: 100_000,
        filled: true,
    });
    f.graphics.push(Graphic {
        id: fresh(),
        layer: Layer::BSilk,
        kind: GraphicKind::Line {
            a: p(0, 0),
            b: p(MM, 0),
        },
        width: 150_000,
        filled: false,
    });
    b.texts.push(TextItem {
        id: fresh(),
        layer: Layer::FSilk,
        text: "R1".into(),
        position: p(20 * MM, 20 * MM),
        height: MM,
        thickness: 150_000,
        rotation: 0,
        mirrored: false,
        justify: Justify::Left,
    });
    let (file, warnings) = write_layer(&b, Layer::FSilk, &GerberOptions::default()).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let g = read_gerber(&file.content).unwrap();
    assert_eq!(g.file_attr("FileFunction").unwrap(), ["Legend", "Top"]);
    assert!(g.draws().iter().any(|(a, c, ap)| *a == p(9 * MM, 12 * MM)
        && *c == p(11 * MM, 12 * MM)
        && ap.params == ["0.150000"]));
    // the circle is one full-circle arc
    let circles: Vec<_> = g
        .objects
        .iter()
        .filter_map(|o| match o.kind {
            GKind::Arc {
                from,
                to,
                center,
                ccw,
            } => Some((from, to, center, ccw)),
            _ => None,
        })
        .collect();
    assert_eq!(
        circles,
        vec![(
            p(10 * MM + 500_000, 7 * MM),
            p(10 * MM + 500_000, 7 * MM),
            p(10 * MM, 7 * MM),
            true
        )]
    );
    // the filled disc is one flash
    assert!(g
        .flashes()
        .iter()
        .any(|(a, ap)| *a == p(13 * MM, 10 * MM) && ap.params == ["0.500000"])); // 2 * 0.2 mm radius + 0.1 mm stroke
                                                                                 // text strokes: several draws near the text origin, all with the text's pen
    let strokes = g
        .draws()
        .into_iter()
        .filter(|(a, _, ap)| a.x >= 20 * MM && ap.params == ["0.150000"])
        .count();
    assert!(strokes >= 4, "{strokes}");
    // the back silk file holds only the back line
    let back = read(&b, Layer::BSilk);
    assert_eq!(back.draws().len(), 1);
    // characters the stroke font lacks are reported
    b.texts.push(TextItem {
        id: fresh(),
        layer: Layer::FSilk,
        text: "\u{20ac}".into(),
        position: p(30 * MM, 20 * MM),
        height: MM,
        thickness: 150_000,
        rotation: 0,
        mirrored: false,
        justify: Justify::Left,
    });
    let (_, warnings) = write_layer(&b, Layer::FSilk, &GerberOptions::default()).unwrap();
    assert!(
        warnings.iter().any(|w| w.contains("stroke font")),
        "{warnings:?}"
    );
}

#[test]
fn profile_has_lines_and_true_arcs() {
    let mut b = base_board();
    let c = p(25 * MM, 25 * MM);
    let a1 = Arc::new(p(27 * MM, 25 * MM), p(23 * MM, 25 * MM), c, true).unwrap();
    let a2 = Arc::new(p(23 * MM, 25 * MM), p(27 * MM, 25 * MM), c, true).unwrap();
    b.outline
        .as_mut()
        .unwrap()
        .cutouts
        .push(vec![OutlineSeg::Arc(a1), OutlineSeg::Arc(a2)]);
    let g = read(&b, Layer::EdgeCuts);
    assert_eq!(g.file_attr("FileFunction").unwrap(), ["Profile", "NP"]);
    assert_eq!(g.draws().len(), 4);
    assert!(g
        .draws()
        .iter()
        .all(|(_, _, a)| a.params == ["0.100000"] && a.function.as_deref() == Some("Profile")));
    let arcs: Vec<_> = g
        .objects
        .iter()
        .filter(|o| matches!(o.kind, GKind::Arc { .. }))
        .collect();
    assert_eq!(arcs.len(), 2);
    assert_eq!(
        arcs[0].kind,
        GKind::Arc {
            from: p(27 * MM, 25 * MM),
            to: p(23 * MM, 25 * MM),
            center: c,
            ccw: true
        }
    );
    // the profile is closed: every segment starts where another ends
    let mut ends: Vec<Point> = g.draws().iter().flat_map(|(a, b, _)| [*a, *b]).collect();
    ends.sort_by_key(|q| (q.x, q.y));
    for w in ends.chunks(2) {
        assert_eq!(w[0], w[1]);
    }
}

#[test]
fn write_all_covers_every_layer_and_is_deterministic() {
    let b = rich_board();
    let opts = GerberOptions::default();
    let a = write_all(&b, &opts).unwrap();
    let names: Vec<_> = a.files.iter().map(|f| f.function.as_str()).collect();
    assert_eq!(
        names,
        [
            "Copper,L1,Top",
            "Copper,L2,Bot",
            "Soldermask,Top",
            "Soldermask,Bot",
            "Paste,Top",
            "Paste,Bot",
            "Legend,Top",
            "Legend,Bot",
            "Profile,NP"
        ]
    );
    assert_eq!(a, write_all(&b, &opts).unwrap());
    let file_names: Vec<_> = a.files.iter().map(|f| f.file_name.as_str()).collect();
    assert_eq!(file_names[0], "test-F_Cu.gbr");
    assert_eq!(file_names[8], "test-Edge_Cuts.gbr");
    // every file parses and ends properly
    for f in &a.files {
        assert!(read_gerber(&f.content).unwrap().finished, "{}", f.file_name);
    }
    // no date unless asked for
    assert!(a.files.iter().all(|f| !f.content.contains("CreationDate")));
    let dated = write_all(
        &b,
        &GerberOptions {
            creation_date: Some("2026-11-26T10:00:00+00:00".into()),
            ..opts
        },
    )
    .unwrap();
    assert!(dated.files[0]
        .content
        .contains("%TF.CreationDate,2026-11-26T10:00:00+00:00*%"));
}

#[test]
fn unfilled_zones_and_missing_outline_are_reported() {
    let mut b = rich_board();
    b.zones[0].filled.clear();
    let (_, w) = write_layer(&b, Layer::FCu, &GerberOptions::default()).unwrap();
    assert!(w.iter().any(|m| m.contains("no fill data")), "{w:?}");
    b.outline = None;
    let set = write_all(&b, &GerberOptions::default()).unwrap();
    assert!(set.warnings.iter().any(|m| m.contains("no outline")));
    assert!(set.files.iter().all(|f| f.function != "Profile,NP"));
}

#[test]
fn bad_requests_are_errors() {
    let b = rich_board();
    assert!(write_layer(&b, Layer::Drill, &GerberOptions::default()).is_err());
    assert!(write_layer(&b, Layer::FCourtyard, &GerberOptions::default()).is_err());
    assert!(write_layer(&b, Layer::InnerCu(3), &GerberOptions::default()).is_err());
    assert!(write_layer(
        &b,
        Layer::FCu,
        &GerberOptions {
            profile_width: 0,
            ..Default::default()
        }
    )
    .is_err());
    assert!(write_layer(
        &b,
        Layer::FCu,
        &GerberOptions {
            mask_margin: i64::MAX,
            ..Default::default()
        }
    )
    .is_err());
    // an invalid board is refused, not half written
    let mut bad = rich_board();
    bad.tracks[0].width = -1;
    assert!(write_all(&bad, &GerberOptions::default()).is_err());
}

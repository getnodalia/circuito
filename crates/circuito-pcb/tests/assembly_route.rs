// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::field_reassign_with_default
)]
//! Pick-and-place, the bill of materials and the connector helper.

mod common;
use circuito_pcb::assembly::{
    bom_csv, bom_lines, natural_cmp, pick_and_place_csv, AssemblyOptions,
};
use circuito_pcb::board::*;
use circuito_pcb::drc::{ClearanceTable, Rules, ViolationCode as C};
use circuito_pcb::geom::Point;
use circuito_pcb::layer::{Layer, Side};
use circuito_pcb::route::{connect, connector_points, ConnectorShape, ConnectorSpec};
use circuito_pcb::units::MM;
use common::*;
use proptest::prelude::*;

fn part(b: &mut Board, r: &str, value: &str, pkg: &str, at: Point, side: Side, rot: i32) -> Id {
    let pads = vec![smd_pad(b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
    let id = footprint(b, r, at, side, pads);
    let f = b.footprints.iter_mut().find(|f| f.id == id).unwrap();
    f.value = value.into();
    f.package = pkg.into();
    f.rotation = rot;
    id
}

fn assembly_board() -> Board {
    let mut b = base_board();
    part(
        &mut b,
        "R10",
        "10k",
        "0603",
        p(30 * MM, 10 * MM),
        Side::Front,
        0,
    );
    part(
        &mut b,
        "R2",
        "10k",
        "0603",
        p(20 * MM, 10 * MM),
        Side::Front,
        90_000,
    );
    part(
        &mut b,
        "R1",
        "4.7k",
        "0603",
        p(10 * MM, 10 * MM),
        Side::Front,
        0,
    );
    part(
        &mut b,
        "C1",
        "100n",
        "0402",
        p(12_345_678, 23_456_789),
        Side::Back,
        -90_000,
    );
    let pads = vec![th_pad(&mut b, "1", NA, p(0, 0), 1_600_000, 800_000)];
    footprint(&mut b, "J1", p(40 * MM, 40 * MM), Side::Front, pads);
    let id = part(&mut b, "TP1", "TP", "TP", p(5 * MM, 5 * MM), Side::Front, 0);
    b.footprints
        .iter_mut()
        .find(|f| f.id == id)
        .unwrap()
        .exclude_from_pos = true;
    let id = part(
        &mut b,
        "FID1",
        "FID",
        "Fid",
        p(6 * MM, 6 * MM),
        Side::Front,
        0,
    );
    b.footprints
        .iter_mut()
        .find(|f| f.id == id)
        .unwrap()
        .exclude_from_bom = true;
    b
}

#[test]
fn pick_and_place_lists_smd_parts_exactly() {
    let b = assembly_board();
    let csv = pick_and_place_csv(&b, &AssemblyOptions::default()).unwrap();
    assert_eq!(
        csv,
        "Ref,Value,Package,PosX,PosY,Rot,Side\n\
         C1,100n,0402,12.345678,23.456789,270.000,bottom\n\
         FID1,FID,Fid,6.000000,6.000000,0.000,top\n\
         R1,4.7k,0603,10.000000,10.000000,0.000,top\n\
         R2,10k,0603,20.000000,10.000000,90.000,top\n\
         R10,10k,0603,30.000000,10.000000,0.000,top\n"
    );
    // through-hole parts on request, an origin shifts every position
    let all = pick_and_place_csv(
        &b,
        &AssemblyOptions {
            origin: p(MM, -2 * MM),
            smd_only: false,
        },
    )
    .unwrap();
    assert!(
        all.contains("J1,10k,PKG,39.000000,42.000000,0.000,top\n"),
        "{all}"
    );
    assert!(all.contains("R1,4.7k,0603,9.000000,12.000000,0.000,top\n"));
    assert!(!all.contains("TP1"));
}

#[test]
fn bom_groups_by_value_and_package() {
    let mut b = assembly_board();
    // a second 4.7k part in a different package is its own line
    part(
        &mut b,
        "R3",
        "4.7k",
        "0805",
        p(15 * MM, 15 * MM),
        Side::Front,
        0,
    );
    let lines = bom_lines(&b).unwrap();
    let got: Vec<_> = lines
        .iter()
        .map(|l| {
            (
                l.quantity,
                l.value.as_str(),
                l.package.as_str(),
                l.references.join(" "),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (1, "100n", "0402", "C1".to_string()),
            (1, "10k", "PKG", "J1".to_string()),
            (1, "4.7k", "0603", "R1".to_string()),
            (2, "10k", "0603", "R2 R10".to_string()),
            (1, "4.7k", "0805", "R3".to_string()),
            (1, "TP", "TP", "TP1".to_string()),
        ]
    );
    let csv = bom_csv(&b).unwrap();
    assert!(csv.starts_with("Qty,Value,Package,References\n"));
    assert!(csv.contains("2,10k,0603,R2 R10\n"));
    assert!(!csv.contains("FID1"));
}

#[test]
fn csv_is_safe_to_open_in_a_spreadsheet() {
    let mut b = base_board();
    part(
        &mut b,
        "R1",
        "=HYPERLINK(\"x\")",
        "a,b",
        p(10 * MM, 10 * MM),
        Side::Front,
        0,
    );
    let csv = pick_and_place_csv(&b, &AssemblyOptions::default()).unwrap();
    assert!(
        csv.contains("R1,\"'=HYPERLINK(\"\"x\"\")\",\"a,b\","),
        "{csv}"
    );
    // and parses back as exactly three comma-separated columns before the numbers
    assert_eq!(csv.lines().count(), 2);
}

#[test]
fn missing_or_repeated_references_are_refused() {
    let mut b = base_board();
    part(
        &mut b,
        "",
        "1k",
        "0603",
        p(10 * MM, 10 * MM),
        Side::Front,
        0,
    );
    let e = pick_and_place_csv(&b, &AssemblyOptions::default())
        .unwrap_err()
        .to_string();
    assert!(e.contains("no reference designator"), "{e}");
    assert!(bom_csv(&b).is_err());
    let mut b = base_board();
    part(
        &mut b,
        "R1",
        "1k",
        "0603",
        p(10 * MM, 10 * MM),
        Side::Front,
        0,
    );
    part(
        &mut b,
        "R1",
        "1k",
        "0603",
        p(20 * MM, 10 * MM),
        Side::Front,
        0,
    );
    let e = bom_csv(&b).unwrap_err().to_string();
    assert!(e.contains("reference R1 is used by footprints"), "{e}");
}

#[test]
fn natural_order_is_a_total_order() {
    let names = [
        "", "A", "A1", "A2", "A10", "A010", "B", "R1", "R1A", "R01", "10", "9", "R",
    ];
    for a in names {
        for b in names {
            let ab = natural_cmp(a, b);
            assert_eq!(ab, natural_cmp(b, a).reverse(), "{a} {b}");
            for c in names {
                if ab.is_le() && natural_cmp(b, c).is_le() {
                    assert!(natural_cmp(a, c).is_le(), "{a} {b} {c}");
                }
            }
        }
    }
}

fn angle_ok(a: Point, b: Point) -> bool {
    let (dx, dy) = ((b.x - a.x).abs(), (b.y - a.y).abs());
    dx == 0 || dy == 0 || dx == dy
}

#[test]
fn connector_shapes_are_exact() {
    let a = p(0, 0);
    let b = p(10 * MM, 4 * MM);
    let pts = |s| connector_points(a, b, s).unwrap();
    assert_eq!(
        pts(ConnectorShape::HorizontalVertical),
        [a, p(10 * MM, 0), b]
    );
    assert_eq!(
        pts(ConnectorShape::VerticalHorizontal),
        [a, p(0, 4 * MM), b]
    );
    assert_eq!(
        pts(ConnectorShape::StraightThenDiagonal),
        [a, p(6 * MM, 0), b]
    );
    assert_eq!(
        pts(ConnectorShape::DiagonalThenStraight),
        [a, p(4 * MM, 4 * MM), b]
    );
    // tall: the straight leg is vertical
    let c = p(-3 * MM, -9 * MM);
    assert_eq!(
        connector_points(a, c, ConnectorShape::StraightThenDiagonal).unwrap(),
        [a, p(0, -6 * MM), c]
    );
    // already aligned: one segment
    for t in [p(5, 0), p(0, -7), p(9, 9), p(-4, 4)] {
        assert_eq!(
            connector_points(a, t, ConnectorShape::HorizontalVertical).unwrap(),
            [a, t]
        );
    }
    assert!(connector_points(a, a, ConnectorShape::HorizontalVertical).is_err());
    assert!(connector_points(a, p(i64::MAX, 0), ConnectorShape::HorizontalVertical).is_err());
}

proptest! {
    #[test]
    fn connectors_use_only_0_45_90_degrees(
        ax in -1_000_000i64..1_000_000, ay in -1_000_000i64..1_000_000,
        bx in -1_000_000i64..1_000_000, by in -1_000_000i64..1_000_000,
        s in 0usize..4,
    ) {
        let shape = [ConnectorShape::HorizontalVertical, ConnectorShape::VerticalHorizontal,
                     ConnectorShape::StraightThenDiagonal, ConnectorShape::DiagonalThenStraight][s];
        let (a, b) = (p(ax, ay), p(bx, by));
        match connector_points(a, b, shape) {
            Err(_) => prop_assert_eq!(a, b),
            Ok(pts) => {
                prop_assert_eq!(pts[0], a);
                prop_assert_eq!(*pts.last().unwrap(), b);
                prop_assert!(pts.len() == 2 || pts.len() == 3);
                for w in pts.windows(2) {
                    prop_assert!(angle_ok(w[0], w[1]));
                    prop_assert!(w[0] != w[1]);
                }
            }
        }
    }
}

fn rules() -> Rules {
    let mut r = Rules::default();
    r.clearance = ClearanceTable::uniform(200_000);
    r.min_track_width = 100_000;
    r.copper_to_edge = 100_000;
    r.check_dangling = false;
    r
}

fn spec(net: Option<NetId>, a: Point, b: Point, shape: ConnectorShape) -> ConnectorSpec {
    ConnectorSpec {
        net,
        layer: Layer::FCu,
        width: 200_000,
        from: a,
        to: b,
        shape,
    }
}

#[test]
fn connector_clearance_is_checked_exactly() {
    // an existing track of net B along y = 12 mm; the connector (net A, 0.2 mm wide) runs along
    // y = 10 mm: copper gap = 2 mm - 0.2 mm = 1.8 mm, so push the other track to the boundary
    for (off, bad) in [(0i64, false), (-1, true), (1, false)] {
        let mut b = base_board();
        let y = 10 * MM + 200_000 + 200_000 + off; // widths 0.2 mm: gap = dy - 0.2 mm
        track(
            &mut b,
            NB,
            Layer::FCu,
            200_000,
            p(10 * MM, y),
            p(20 * MM, y),
        );
        let c = connect(
            &b,
            &spec(
                NA,
                p(10 * MM, 10 * MM),
                p(20 * MM, 10 * MM),
                ConnectorShape::HorizontalVertical,
            ),
            &rules(),
        )
        .unwrap();
        assert_eq!(c.tracks.len(), 1);
        assert_eq!(
            c.violations
                .iter()
                .filter(|v| v.code == C::ClearanceTrackTrack)
                .count(),
            usize::from(bad),
            "off {off}: {:?}",
            c.violations
        );
        assert_eq!(c.is_clean(), !bad);
    }
    // the board itself is untouched and new ids are unused
    let mut b = base_board();
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(10 * MM, 30 * MM),
        p(20 * MM, 30 * MM),
    );
    let before = b.clone();
    let c = connect(
        &b,
        &spec(
            NA,
            p(10 * MM, 10 * MM),
            p(20 * MM, 14 * MM),
            ConnectorShape::StraightThenDiagonal,
        ),
        &rules(),
    )
    .unwrap();
    assert_eq!(b, before);
    assert_eq!(c.tracks.len(), 2);
    assert!(c.is_clean(), "{:?}", c.violations);
    let mut grown = b.clone();
    grown.tracks.extend(c.tracks.clone());
    grown.validate().unwrap();
}

#[test]
fn connector_through_another_net_is_a_short() {
    let mut b = base_board();
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(15 * MM, 5 * MM),
        p(15 * MM, 20 * MM),
    );
    let c = connect(
        &b,
        &spec(
            NA,
            p(10 * MM, 10 * MM),
            p(20 * MM, 10 * MM),
            ConnectorShape::HorizontalVertical,
        ),
        &rules(),
    )
    .unwrap();
    assert!(
        c.violations.iter().any(|v| v.code == C::ShortCircuit),
        "{:?}",
        c.violations
    );
    // the same net is fine
    let c = connect(
        &b,
        &spec(
            NB,
            p(10 * MM, 10 * MM),
            p(20 * MM, 10 * MM),
            ConnectorShape::HorizontalVertical,
        ),
        &rules(),
    )
    .unwrap();
    assert!(c.is_clean(), "{:?}", c.violations);
}

#[test]
fn connector_requests_are_validated() {
    let b = base_board();
    let ok = spec(
        NA,
        p(10 * MM, 10 * MM),
        p(20 * MM, 10 * MM),
        ConnectorShape::HorizontalVertical,
    );
    assert!(connect(&b, &ConnectorSpec { width: 0, ..ok }, &rules()).is_err());
    assert!(connect(
        &b,
        &ConnectorSpec {
            layer: Layer::InnerCu(1),
            ..ok
        },
        &rules()
    )
    .is_err());
    assert!(connect(
        &b,
        &ConnectorSpec {
            net: Some(NetId(99)),
            ..ok
        },
        &rules()
    )
    .is_err());
    assert!(connect(&b, &ConnectorSpec { to: ok.from, ..ok }, &rules()).is_err());
    assert!(connect(
        &b,
        &ConnectorSpec {
            to: p(i64::MAX, 0),
            ..ok
        },
        &rules()
    )
    .is_err());
}

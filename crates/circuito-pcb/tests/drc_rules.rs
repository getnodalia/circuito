// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::field_reassign_with_default
)]
//! One tiny board per rule, failing at exactly the boundary: required - 1 nm fails, required passes.

mod common;
use circuito_pcb::board::*;
use circuito_pcb::drc::{
    check_profile, run_drc, CapabilityProfile, ClearanceTable, DrcReport, Rules, ViolationCode as C,
};
use circuito_pcb::geom::{Arc, Point};
use circuito_pcb::layer::{Layer, Side};
use circuito_pcb::units::MM;
use common::*;

const CLR: i64 = 200_000;

fn rules() -> Rules {
    let mut r = Rules::default();
    r.min_track_width = 150_000;
    r.clearance = ClearanceTable::uniform(CLR);
    r.copper_to_edge = 300_000;
    r.min_annular_ring = 150_000;
    r.min_drill = 300_000;
    r.min_via_diameter = 600_000;
    r.min_via_drill = 300_000;
    r.hole_to_hole = 250_000;
    r.silk_to_pad = 100_000;
    r.min_silk_width = 150_000;
    r.mask_margin = 50_000;
    r.min_mask_web = 100_000;
    r.check_dangling = false;
    r
}

fn drc(b: &Board) -> DrcReport {
    run_drc(b, &rules()).unwrap()
}

fn drc_with(b: &Board, f: impl FnOnce(&mut Rules)) -> DrcReport {
    let mut r = rules();
    f(&mut r);
    run_drc(b, &r).unwrap()
}

/// `build(offset)` makes a board in which the gap between the two objects is `CLR + offset`;
/// offset 0 must pass, offset -1 must fail with exactly `code` once, and +1 must pass.
fn boundary(code: C, build: impl Fn(i64) -> Board) {
    for off in [0, 1, 1000] {
        let r = drc(&build(off));
        assert!(!r.has(code), "{code:?} at offset {off}: {:?}", r.violations);
    }
    let r = drc(&build(-1));
    assert_eq!(
        r.count(code),
        1,
        "{code:?} at offset -1: {:?}",
        r.violations
    );
    let v = r.violations.iter().find(|v| v.code == code).unwrap();
    assert_eq!(v.required, Some(CLR));
    assert!(v.measured.unwrap() < CLR);
    assert!(!v.objects.is_empty() && !v.locations.is_empty() && !v.message.is_empty());
}

#[test]
fn clean_board_has_no_violations() {
    let r = drc_with(&clean(), |r| r.check_dangling = true);
    assert!(r.is_clean(), "{:#?}", r.violations);
    assert!(!r.truncated);
    assert!(r.stats.copper_items >= 6);
}

fn clean() -> Board {
    let mut b = base_board();
    for (refd, x) in [("R1", 10), ("R2", 30)] {
        let pads = vec![
            smd_pad(&mut b, "1", NA, p(0, MM), MM, MM, PadShape::Rect),
            smd_pad(&mut b, "2", NB, p(0, -MM), MM, MM, PadShape::Rect),
        ];
        footprint(&mut b, refd, p(x * MM, 10 * MM), Side::Front, pads);
    }
    track(
        &mut b,
        NA,
        Layer::FCu,
        250_000,
        p(10 * MM, 11 * MM),
        p(30 * MM, 11 * MM),
    );
    track(
        &mut b,
        NB,
        Layer::FCu,
        250_000,
        p(10 * MM, 9 * MM),
        p(30 * MM, 9 * MM),
    );
    b
}

#[test]
fn clearance_track_track() {
    boundary(C::ClearanceTrackTrack, |off| {
        let mut b = base_board();
        track(
            &mut b,
            NA,
            Layer::FCu,
            200_000,
            p(10 * MM, 10 * MM),
            p(20 * MM, 10 * MM),
        );
        track(
            &mut b,
            NB,
            Layer::FCu,
            200_000,
            p(10 * MM, 10 * MM + 200_000 + CLR + off),
            p(20 * MM, 10 * MM + 200_000 + CLR + off),
        );
        b
    });
}

#[test]
fn clearance_track_pad() {
    boundary(C::ClearanceTrackPad, |off| {
        let mut b = base_board();
        let pads = vec![smd_pad(&mut b, "1", NB, p(0, 0), MM, MM, PadShape::Rect)];
        footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, pads);
        // pad top edge at y = 10.5 mm; track half width 100 um
        track(
            &mut b,
            NA,
            Layer::FCu,
            200_000,
            p(5 * MM, 10 * MM + 500_000 + 100_000 + CLR + off),
            p(15 * MM, 10 * MM + 500_000 + 100_000 + CLR + off),
        );
        b
    });
}

#[test]
fn clearance_track_via() {
    boundary(C::ClearanceTrackVia, |off| {
        let mut b = base_board();
        via(&mut b, NB, p(10 * MM, 10 * MM), 800_000, 400_000);
        track(
            &mut b,
            NA,
            Layer::FCu,
            200_000,
            p(5 * MM, 10 * MM + 400_000 + 100_000 + CLR + off),
            p(15 * MM, 10 * MM + 400_000 + 100_000 + CLR + off),
        );
        b
    });
}

#[test]
fn clearance_pad_pad() {
    boundary(C::ClearancePadPad, |off| {
        let mut b = base_board();
        let p1 = vec![smd_pad(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
        footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, p1);
        let p2 = vec![smd_pad(&mut b, "1", NB, p(0, 0), MM, MM, PadShape::Rect)];
        footprint(
            &mut b,
            "U2",
            p(10 * MM + MM + CLR + off, 10 * MM),
            Side::Front,
            p2,
        );
        b
    });
}

#[test]
fn clearance_pad_via() {
    boundary(C::ClearancePadVia, |off| {
        let mut b = base_board();
        let p1 = vec![smd_pad(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
        footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, p1);
        via(
            &mut b,
            NB,
            p(10 * MM + 500_000 + 400_000 + CLR + off, 10 * MM),
            800_000,
            400_000,
        );
        b
    });
}

#[test]
fn clearance_via_via() {
    boundary(C::ClearanceViaVia, |off| {
        let mut b = base_board();
        via(&mut b, NA, p(10 * MM, 10 * MM), 800_000, 400_000);
        via(
            &mut b,
            NB,
            p(10 * MM + 800_000 + CLR + off, 10 * MM),
            800_000,
            400_000,
        );
        b
    });
}

#[test]
fn clearance_zone_copper() {
    boundary(C::ClearanceZoneCopper, |off| {
        let mut b = base_board();
        zone(
            &mut b,
            GND,
            Layer::FCu,
            rect_region(5 * MM, 5 * MM, 15 * MM, 15 * MM),
        );
        // vertical track right of the zone edge x = 15 mm
        track(
            &mut b,
            NA,
            Layer::FCu,
            200_000,
            p(15 * MM + 100_000 + CLR + off, 5 * MM),
            p(15 * MM + 100_000 + CLR + off, 15 * MM),
        );
        b
    });
}

#[test]
fn clearance_is_exact_for_diagonal_distances() {
    // vias 800 um pad, 200 um clearance: centre distance must be >= 1000 um. Along a diagonal
    // dx = dy = k the distance is k*sqrt(2); find the smallest passing k exactly.
    let need = 1_000_000_000_000i128; // (1000 um)^2 in nm^2
    let k = (1..2_000_000i64)
        .find(|k| 2 * (*k as i128) * (*k as i128) >= need)
        .unwrap();
    assert!(2 * ((k - 1) as i128).pow(2) < need);
    for (dk, expect_violation) in [(0, false), (-1, true), (1, false)] {
        let mut b = base_board();
        via(&mut b, NA, p(10 * MM, 10 * MM), 800_000, 400_000);
        via(
            &mut b,
            NB,
            p(10 * MM + k + dk, 10 * MM + k + dk),
            800_000,
            400_000,
        );
        let r = drc(&b);
        assert_eq!(r.has(C::ClearanceViaVia), expect_violation, "k={k} dk={dk}");
    }
}

#[test]
fn clearance_on_arc_tracks_and_different_layers() {
    // arc track on F.Cu 5 mm radius; a straight track crossing its swept area on B.Cu is fine
    let arc = Arc::new(
        p(15 * MM, 10 * MM),
        p(10 * MM, 15 * MM),
        p(10 * MM, 10 * MM),
        true,
    )
    .unwrap();
    let mut b = base_board();
    arc_track(&mut b, NA, Layer::FCu, 200_000, arc);
    track(
        &mut b,
        NB,
        Layer::BCu,
        200_000,
        p(8 * MM, 8 * MM),
        p(18 * MM, 18 * MM),
    );
    assert!(!drc(&b).has(C::ShortCircuit) && !drc(&b).has(C::ClearanceTrackTrack));
    // the same crossing on one layer is a short
    let mut b = base_board();
    arc_track(&mut b, NA, Layer::FCu, 200_000, arc);
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(8 * MM, 8 * MM),
        p(18 * MM, 18 * MM),
    );
    let r = drc(&b);
    assert_eq!(r.count(C::ShortCircuit), 1);
    assert_eq!(
        r.count(C::ClearanceTrackTrack),
        0,
        "a short is not also a clearance error"
    );
    // a straight track 1 mm outside the arc's radius: clear
    let mut b = base_board();
    arc_track(&mut b, NA, Layer::FCu, 200_000, arc);
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(16 * MM, 8 * MM),
        p(16 * MM, 18 * MM),
    );
    assert!(drc(&b).is_clean_of(&[C::ShortCircuit, C::ClearanceTrackTrack]));
}

trait CleanOf {
    fn is_clean_of(&self, codes: &[C]) -> bool;
}
impl CleanOf for DrcReport {
    fn is_clean_of(&self, codes: &[C]) -> bool {
        codes.iter().all(|c| !self.has(*c))
    }
}

#[test]
fn short_circuit_between_nets() {
    let mut b = base_board();
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(10 * MM, 10 * MM),
        p(20 * MM, 20 * MM),
    );
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(10 * MM, 20 * MM),
        p(20 * MM, 10 * MM),
    );
    let r = drc(&b);
    assert_eq!(r.count(C::ShortCircuit), 1);
    let v = &r
        .violations
        .iter()
        .find(|v| v.code == C::ShortCircuit)
        .unwrap();
    assert_eq!(v.net.as_deref(), Some("A / B"));
    // touching exactly (gap 0) is still a short
    let mut b = base_board();
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(10 * MM, 10 * MM),
        p(20 * MM, 10 * MM),
    );
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(10 * MM, 10 * MM + 200_000),
        p(20 * MM, 10 * MM + 200_000),
    );
    assert_eq!(drc(&b).count(C::ShortCircuit), 1);
    // same net overlapping is fine
    let mut b = base_board();
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(10 * MM, 10 * MM),
        p(20 * MM, 20 * MM),
    );
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(10 * MM, 20 * MM),
        p(20 * MM, 10 * MM),
    );
    assert!(drc(&b).is_clean_of(&[C::ShortCircuit, C::ClearanceTrackTrack]));
}

#[test]
fn net_class_clearance_raises_the_requirement() {
    let build = |off: i64| {
        let mut b = base_board();
        b.net_classes.push(NetClass {
            name: "HV".into(),
            clearance: Some(500_000),
            ..Default::default()
        });
        b.nets[0].class = "HV".into();
        track(
            &mut b,
            NA,
            Layer::FCu,
            200_000,
            p(10 * MM, 10 * MM),
            p(20 * MM, 10 * MM),
        );
        track(
            &mut b,
            NB,
            Layer::FCu,
            200_000,
            p(10 * MM, 10 * MM + 200_000 + 500_000 + off),
            p(20 * MM, 10 * MM + 200_000 + 500_000 + off),
        );
        b
    };
    assert_eq!(drc(&build(-1)).count(C::ClearanceTrackTrack), 1);
    assert_eq!(drc(&build(0)).count(C::ClearanceTrackTrack), 0);
    // class value below the board rule never lowers the requirement
    let mut b = base_board();
    b.net_classes.push(NetClass {
        name: "X".into(),
        clearance: Some(10_000),
        ..Default::default()
    });
    b.nets[0].class = "X".into();
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(10 * MM, 10 * MM),
        p(20 * MM, 10 * MM),
    );
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(10 * MM, 10 * MM + 200_000 + CLR - 1),
        p(20 * MM, 10 * MM + 200_000 + CLR - 1),
    );
    assert_eq!(drc(&b).count(C::ClearanceTrackTrack), 1);
}

#[test]
fn track_width_boundary_and_class() {
    for (w, bad) in [(150_000, false), (149_999, true), (150_001, false)] {
        let mut b = base_board();
        track(
            &mut b,
            NA,
            Layer::FCu,
            w,
            p(10 * MM, 10 * MM),
            p(20 * MM, 10 * MM),
        );
        assert_eq!(drc(&b).has(C::TrackWidth), bad, "width {w}");
    }
    let mut b = base_board();
    b.net_classes.push(NetClass {
        name: "P".into(),
        min_track_width: Some(400_000),
        ..Default::default()
    });
    b.nets[0].class = "P".into();
    track(
        &mut b,
        NA,
        Layer::FCu,
        399_999,
        p(10 * MM, 10 * MM),
        p(20 * MM, 10 * MM),
    );
    track(
        &mut b,
        NB,
        Layer::FCu,
        399_999,
        p(10 * MM, 20 * MM),
        p(20 * MM, 20 * MM),
    );
    let r = drc(&b);
    assert_eq!(
        r.count(C::TrackWidth),
        1,
        "only the net of the class is held to it"
    );
    let v = r
        .violations
        .iter()
        .find(|v| v.code == C::TrackWidth)
        .unwrap();
    assert_eq!((v.measured, v.required), (Some(399_999), Some(400_000)));
}

#[test]
fn via_size_drill_and_annular_ring() {
    for (d, bad) in [(600_000, false), (599_999, true)] {
        let mut b = base_board();
        via(&mut b, NA, p(10 * MM, 10 * MM), d, 300_000);
        assert_eq!(
            drc_with(&b, |r| r.min_annular_ring = 0).has(C::ViaDiameter),
            bad,
            "diameter {d}"
        );
    }
    for (dr, bad) in [(300_000, false), (299_999, true)] {
        let mut b = base_board();
        via(&mut b, NA, p(10 * MM, 10 * MM), 800_000, dr);
        assert_eq!(drc(&b).has(C::DrillMin), bad, "drill {dr}");
    }
    // ring = (D - d) / 2 >= 150 um  <=>  D - d >= 300 um, including half-nm cases
    for (d_big, drill, bad) in [
        (900_000, 600_000, false),
        (899_999, 600_000, true),
        (900_001, 600_000, false),
        (800_000, 500_001, true),
        (800_001, 500_001, false),
    ] {
        let mut b = base_board();
        via(&mut b, NA, p(10 * MM, 10 * MM), d_big, drill);
        let r = drc_with(&b, |r| {
            r.min_via_drill = 100_000;
            r.min_drill = 100_000;
            r.min_annular_ring = 150_000;
        });
        assert_eq!(r.has(C::ViaAnnularRing), bad, "{d_big}/{drill}");
    }
}

#[test]
fn pad_drill_and_annular_ring_for_round_rect_and_slot() {
    // round TH pad
    for (d, bad) in [(1_100_000, false), (1_099_999, true)] {
        let mut b = base_board();
        let pads = vec![th_pad(&mut b, "1", NA, p(0, 0), d, 800_000)];
        footprint(&mut b, "J1", p(10 * MM, 10 * MM), Side::Front, pads);
        let r = drc(&b);
        assert_eq!(r.has(C::PadAnnularRing), bad, "{d}");
    }
    // square pad 1.1 mm with a 0.8 mm hole, rotated by a quarter turn: ring 150 um
    for (w, bad) in [(1_100_000, false), (1_099_999, true)] {
        let mut b = base_board();
        let mut pad = th_pad(&mut b, "1", NA, p(0, 0), w, 800_000);
        pad.shape = PadShape::Rect;
        pad.rotation = 90_000;
        footprint(&mut b, "J1", p(10 * MM, 10 * MM), Side::Front, vec![pad]);
        assert_eq!(drc(&b).has(C::PadAnnularRing), bad, "rect {w}");
    }
    // oval pad 2.0 x 1.1 with a 0.8 mm round hole (ring on the short side)
    for (h, bad) in [(1_100_000, false), (1_099_999, true)] {
        let mut b = base_board();
        let mut pad = th_pad(&mut b, "1", NA, p(0, 0), 2_000_000, 800_000);
        pad.shape = PadShape::Oval;
        pad.size = Size { w: 2_000_000, h };
        footprint(&mut b, "J1", p(10 * MM, 10 * MM), Side::Front, vec![pad]);
        assert_eq!(drc(&b).has(C::PadAnnularRing), bad, "oval {h}");
    }
    // slot 1.5 x 0.8 in a 2.0 x 1.1 oval: ends are 0.25 mm from the pad's end caps... ring 150 um on the short side
    for (h, bad) in [(1_100_000, false), (1_099_999, true)] {
        let mut b = base_board();
        let mut pad = th_pad(&mut b, "1", NA, p(0, 0), 2_000_000, 800_000);
        pad.shape = PadShape::Oval;
        pad.size = Size { w: 2_400_000, h };
        pad.drill = Some(Drill {
            diameter: 800_000,
            slot_length: Some(1_500_000),
        });
        footprint(&mut b, "J1", p(10 * MM, 10 * MM), Side::Front, vec![pad]);
        assert_eq!(drc(&b).has(C::PadAnnularRing), bad, "slot {h}");
    }
    // minimum drill, also for NPTH
    let mut b = base_board();
    let mut npth = th_pad(&mut b, "", None, p(0, 0), 500_000, 299_999);
    npth.kind = PadKind::Npth;
    footprint(&mut b, "H1", p(10 * MM, 10 * MM), Side::Front, vec![npth]);
    assert_eq!(drc(&b).count(C::DrillMin), 1);
}

#[test]
fn hole_to_hole_boundary() {
    for (off, bad) in [(0, false), (-1, true), (1, false)] {
        let mut b = base_board();
        via(&mut b, NA, p(10 * MM, 10 * MM), 1_000_000, 300_000);
        via(
            &mut b,
            NA,
            p(10 * MM + 300_000 + 250_000 + off, 10 * MM),
            1_000_000,
            300_000,
        );
        let r = drc_with(&b, |r| r.clearance = ClearanceTable::uniform(0));
        assert_eq!(r.has(C::HoleToHole), bad, "offset {off}");
    }
}

#[test]
fn board_edge_clearance_outside_and_missing_outline() {
    for (off, bad) in [(0, false), (-1, true), (1, false)] {
        let mut b = base_board();
        // vertical track near the left edge x = 0
        track(
            &mut b,
            NA,
            Layer::FCu,
            200_000,
            p(300_000 + 100_000 + off, 10 * MM),
            p(300_000 + 100_000 + off, 20 * MM),
        );
        assert_eq!(drc(&b).has(C::EdgeClearance), bad, "offset {off}");
    }
    // a via beyond the edge is outside, not merely close
    let mut b = base_board();
    via(&mut b, NA, p(-5 * MM, 10 * MM), 800_000, 400_000);
    let r = drc(&b);
    assert_eq!(r.count(C::EdgeOutside), 1);
    assert_eq!(r.count(C::EdgeClearance), 0);
    // a track straddling the edge is a clearance error with gap 0
    let mut b = base_board();
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(-MM, 10 * MM),
        p(MM, 10 * MM),
    );
    let r = drc(&b);
    assert_eq!(r.count(C::EdgeClearance), 1);
    // no outline
    let mut b = base_board();
    b.outline = None;
    assert_eq!(drc(&b).count(C::OutlineMissing), 1);
    // inside a cut-out counts as outside the board
    let mut b = base_board();
    b.outline = Some(Outline {
        outer: Outline::rectangle(0, 0, 50 * MM, 50 * MM).outer,
        cutouts: vec![Outline::rectangle(20 * MM, 20 * MM, 30 * MM, 30 * MM).outer],
    });
    via(&mut b, NA, p(25 * MM, 25 * MM), 800_000, 400_000);
    assert_eq!(drc(&b).count(C::EdgeOutside), 1);
}

#[test]
fn connectivity_across_layers_vias_and_thru_hole_pads() {
    let two_pads = |b: &mut Board, layer_a: Side, layer_b: Side| {
        let p1 = vec![smd_pad(b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
        footprint(b, "U1", p(10 * MM, 10 * MM), layer_a, p1);
        let p2 = vec![smd_pad(b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
        footprint(b, "U2", p(30 * MM, 10 * MM), layer_b, p2);
    };
    // unrouted
    let mut b = base_board();
    two_pads(&mut b, Side::Front, Side::Front);
    let r = drc(&b);
    assert_eq!(r.count(C::Unconnected), 1);
    let v = &r.violations[0];
    assert_eq!(v.net.as_deref(), Some("A"));
    assert!(
        v.message.contains("U1.1") && v.message.contains("U2.1"),
        "{}",
        v.message
    );
    assert_eq!(v.objects.len(), 2);
    // routed on the same layer
    track(
        &mut b,
        NA,
        Layer::FCu,
        250_000,
        p(10 * MM, 10 * MM),
        p(30 * MM, 10 * MM),
    );
    assert_eq!(drc(&b).count(C::Unconnected), 0);
    // a back-side track under a front pad does not connect it
    let mut b = base_board();
    two_pads(&mut b, Side::Front, Side::Front);
    track(
        &mut b,
        NA,
        Layer::BCu,
        250_000,
        p(10 * MM, 10 * MM),
        p(30 * MM, 10 * MM),
    );
    assert_eq!(drc(&b).count(C::Unconnected), 1);
    // ... but with vias at both ends and a track on each side it does
    let mut b = base_board();
    two_pads(&mut b, Side::Front, Side::Front);
    via(&mut b, NA, p(10 * MM, 10 * MM), 800_000, 400_000);
    via(&mut b, NA, p(30 * MM, 10 * MM), 800_000, 400_000);
    track(
        &mut b,
        NA,
        Layer::BCu,
        250_000,
        p(10 * MM, 10 * MM),
        p(30 * MM, 10 * MM),
    );
    assert_eq!(drc(&b).count(C::Unconnected), 0);
    // a front pad and a back pad joined by a track on each side and one via
    let mut b = base_board();
    two_pads(&mut b, Side::Front, Side::Back);
    via(&mut b, NA, p(20 * MM, 10 * MM), 800_000, 400_000);
    track(
        &mut b,
        NA,
        Layer::FCu,
        250_000,
        p(10 * MM, 10 * MM),
        p(20 * MM, 10 * MM),
    );
    assert_eq!(
        drc(&b).count(C::Unconnected),
        1,
        "the back pad is not routed yet"
    );
    track(
        &mut b,
        NA,
        Layer::BCu,
        250_000,
        p(20 * MM, 10 * MM),
        p(30 * MM, 10 * MM),
    );
    assert_eq!(drc(&b).count(C::Unconnected), 0);
    // three islands: two violations, each against the main island
    let mut b = base_board();
    for (i, x) in [10, 20, 30].iter().enumerate() {
        let pads = vec![smd_pad(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
        footprint(
            &mut b,
            &format!("U{}", i + 1),
            p(x * MM, 10 * MM),
            Side::Front,
            pads,
        );
    }
    assert_eq!(drc(&b).count(C::Unconnected), 2);
    // pads of one footprint with the same number are one terminal
    let mut b = base_board();
    let pads = vec![
        smd_pad(&mut b, "EP", NA, p(-3 * MM, 0), MM, MM, PadShape::Rect),
        smd_pad(&mut b, "EP", NA, p(3 * MM, 0), MM, MM, PadShape::Rect),
    ];
    footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, pads);
    assert_eq!(drc(&b).count(C::Unconnected), 0);
    // a zone connects pads that lie in it
    let mut b = base_board();
    two_pads(&mut b, Side::Front, Side::Front);
    zone(
        &mut b,
        NA,
        Layer::FCu,
        rect_region(5 * MM, 5 * MM, 35 * MM, 15 * MM),
    );
    assert_eq!(drc(&b).count(C::Unconnected), 0);
    // a through-hole pad joins a top track with a bottom track
    let mut b = base_board();
    let j = vec![th_pad(&mut b, "1", NA, p(0, 0), 1_600_000, 800_000)];
    footprint(&mut b, "J1", p(20 * MM, 10 * MM), Side::Front, j);
    let p1 = vec![smd_pad(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
    footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, p1);
    let p2 = vec![smd_pad(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
    footprint(&mut b, "U2", p(30 * MM, 10 * MM), Side::Back, p2);
    track(
        &mut b,
        NA,
        Layer::FCu,
        250_000,
        p(10 * MM, 10 * MM),
        p(20 * MM, 10 * MM),
    );
    track(
        &mut b,
        NA,
        Layer::BCu,
        250_000,
        p(20 * MM, 10 * MM),
        p(30 * MM, 10 * MM),
    );
    assert_eq!(drc(&b).count(C::Unconnected), 0);
}

#[test]
fn dangling_tracks() {
    let mut b = base_board();
    let pads = vec![smd_pad(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
    footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, pads);
    track(
        &mut b,
        NA,
        Layer::FCu,
        250_000,
        p(10 * MM, 10 * MM),
        p(20 * MM, 10 * MM),
    );
    let r = drc_with(&b, |r| r.check_dangling = true);
    assert_eq!(r.count(C::TrackDangling), 1);
    let v = r
        .violations
        .iter()
        .find(|v| v.code == C::TrackDangling)
        .unwrap();
    assert_eq!(v.locations, vec![p(20 * MM, 10 * MM)]);
    // T-junction onto another track's body connects; the far end then dangles, the junction does not
    track(
        &mut b,
        NA,
        Layer::FCu,
        250_000,
        p(15 * MM, 10 * MM),
        p(15 * MM, 20 * MM),
    );
    let r = drc_with(&b, |r| r.check_dangling = true);
    assert_eq!(r.count(C::TrackDangling), 2);
    // a via at the end ties it
    via(&mut b, NA, p(20 * MM, 10 * MM), 800_000, 400_000);
    via(&mut b, NA, p(15 * MM, 20 * MM), 800_000, 400_000);
    assert_eq!(
        drc_with(&b, |r| r.check_dangling = true).count(C::TrackDangling),
        0
    );
    // not checked when switched off
    assert_eq!(drc(&b).count(C::TrackDangling), 0);
}

#[test]
fn acute_angles() {
    let mk = |end_b: Point| {
        let mut b = base_board();
        track(
            &mut b,
            NA,
            Layer::FCu,
            200_000,
            p(10 * MM, 10 * MM),
            p(20 * MM, 10 * MM),
        );
        track(&mut b, NA, Layer::FCu, 200_000, p(20 * MM, 10 * MM), end_b);
        b
    };
    // continuing straight / 45 degree bend (135 between legs) / right angle: fine
    for e in [
        p(30 * MM, 10 * MM),
        p(30 * MM, 20 * MM),
        p(20 * MM, 20 * MM),
        p(20 * MM, 0),
    ] {
        assert_eq!(drc(&mk(e)).count(C::TrackAcuteAngle), 0, "{e:?}");
    }
    // legs at 45 degrees (a hairpin) and doubling back are acute
    for e in [
        p(10 * MM, 20 * MM),
        p(10 * MM, 10 * MM + 1),
        p(15 * MM, 10 * MM),
    ] {
        assert_eq!(drc(&mk(e)).count(C::TrackAcuteAngle), 1, "{e:?}");
    }
    // other nets do not interact
    let mut b = base_board();
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(10 * MM, 10 * MM),
        p(20 * MM, 10 * MM),
    );
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(20 * MM, 10 * MM),
        p(15 * MM, 20 * MM),
    );
    assert_eq!(drc(&b).count(C::TrackAcuteAngle), 0);
    assert_eq!(
        drc_with(&mk(p(15 * MM, 15 * MM)), |r| r.reject_acute_traces = false)
            .count(C::TrackAcuteAngle),
        0
    );
}

#[test]
fn silkscreen_checks() {
    for (off, bad) in [(0, false), (-1, true), (1, false)] {
        let mut b = base_board();
        let pads = vec![smd_pad(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
        let fid = footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, pads);
        // silk line, 150 um wide, above the pad top (y = +0.5 mm); gap = 100 um at off = 0
        let y = 500_000 + 75_000 + 100_000 + off;
        if let Some(f) = b.footprints.iter_mut().find(|f| f.id == fid) {
            f.graphics.push(Graphic {
                id: fresh(),
                layer: Layer::FSilk,
                kind: GraphicKind::Line {
                    a: p(-MM, y),
                    b: p(MM, y),
                },
                width: 150_000,
                filled: false,
            });
        }
        assert_eq!(drc(&b).has(C::SilkOverPad), bad, "offset {off}");
    }
    // silk on the back is not checked against front-only pads
    let mut b = base_board();
    let pads = vec![smd_pad(&mut b, "1", NA, p(0, 0), MM, MM, PadShape::Rect)];
    let fid = footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, pads);
    if let Some(f) = b.footprints.iter_mut().find(|f| f.id == fid) {
        f.graphics.push(Graphic {
            id: fresh(),
            layer: Layer::BSilk,
            kind: GraphicKind::Line {
                a: p(-MM, 0),
                b: p(MM, 0),
            },
            width: 150_000,
            filled: false,
        });
    }
    assert!(!drc(&b).has(C::SilkOverPad));
    // text over a pad
    let mut b = base_board();
    let pads = vec![smd_pad(
        &mut b,
        "1",
        NA,
        p(0, 0),
        2 * MM,
        2 * MM,
        PadShape::Rect,
    )];
    footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, pads);
    b.texts.push(TextItem {
        id: fresh(),
        layer: Layer::FSilk,
        text: "R1".into(),
        position: p(10 * MM - 500_000, 10 * MM - 500_000),
        height: MM,
        thickness: 150_000,
        rotation: 0,
        mirrored: false,
        justify: Justify::Left,
    });
    assert_eq!(drc(&b).count(C::SilkOverPad), 1);
    // minimum silk width
    for (w, bad) in [(150_000, false), (149_999, true)] {
        let mut b = base_board();
        b.graphics.push(Graphic {
            id: fresh(),
            layer: Layer::FSilk,
            kind: GraphicKind::Line {
                a: p(10 * MM, 10 * MM),
                b: p(20 * MM, 10 * MM),
            },
            width: w,
            filled: false,
        });
        assert_eq!(drc(&b).has(C::SilkWidth), bad, "{w}");
    }
}

#[test]
fn solder_mask_bridge_and_sliver() {
    // two pads 0.5 x 1 mm, centres `pitch` apart; openings grow 50 um per side
    let mk = |gap: i64, n2: Option<NetId>| {
        let mut b = base_board();
        let a = vec![smd_pad(
            &mut b,
            "1",
            NA,
            p(0, 0),
            500_000,
            MM,
            PadShape::Rect,
        )];
        footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Front, a);
        let c = vec![smd_pad(
            &mut b,
            "1",
            n2,
            p(0, 0),
            500_000,
            MM,
            PadShape::Rect,
        )];
        footprint(
            &mut b,
            "U2",
            p(10 * MM + 500_000 + gap, 10 * MM),
            Side::Front,
            c,
        );
        b
    };
    let r = |b: &Board| drc_with(b, |r| r.clearance = ClearanceTable::uniform(0));
    // copper gap g -> mask web g - 100 um (openings grow 50 um each); web must be >= 100 um
    assert_eq!(r(&mk(200_000, NB)).count(C::MaskSliver), 0);
    assert_eq!(r(&mk(199_999, NB)).count(C::MaskSliver), 1);
    // webs of 0 or less merge the openings: bridge between different nets, nothing between one net
    assert_eq!(r(&mk(100_000, NB)).count(C::MaskBridge), 1);
    assert_eq!(r(&mk(100_000, NB)).count(C::MaskSliver), 0);
    assert_eq!(r(&mk(100_001, NB)).count(C::MaskBridge), 0);
    assert_eq!(r(&mk(100_001, NB)).count(C::MaskSliver), 1);
    assert_eq!(r(&mk(100_000, NA)).count(C::MaskBridge), 0);
    // a pad with mask disabled has no opening
    let mut b = mk(100_000, NB);
    b.footprints[1].pads[0].mask = false;
    assert!(r(&b).is_clean_of(&[C::MaskBridge, C::MaskSliver]));
    // per-pad margin override
    let mut b = mk(180_000, NB);
    assert_eq!(
        r(&b).count(C::MaskSliver),
        1,
        "web 80 um with the default margin"
    );
    b.footprints[0].pads[0].mask_margin = Some(0);
    b.footprints[1].pads[0].mask_margin = Some(0);
    assert_eq!(r(&b).count(C::MaskSliver), 0, "web 180 um with no margin");
}

#[test]
fn courtyard_overlap() {
    let mk = |dx: i64| {
        let mut b = base_board();
        for (i, x) in [10 * MM, 10 * MM + dx].iter().enumerate() {
            let pads = vec![smd_pad(
                &mut b,
                "1",
                None,
                p(0, 0),
                100_000,
                100_000,
                PadShape::Rect,
            )];
            let id = footprint(&mut b, &format!("U{i}"), p(*x, 20 * MM), Side::Front, pads);
            if let Some(f) = b.footprints.iter_mut().find(|f| f.id == id) {
                f.courtyard = Some(rect_region(-MM, -MM, MM, MM));
            }
        }
        b
    };
    assert_eq!(
        drc(&mk(2 * MM)).count(C::CourtyardOverlap),
        0,
        "courtyards that only share an edge"
    );
    assert_eq!(drc(&mk(2 * MM - 1)).count(C::CourtyardOverlap), 1);
    assert_eq!(drc(&mk(0)).count(C::CourtyardOverlap), 1, "identical");
    let mut b = mk(2 * MM - 1);
    b.footprints[1].side = Side::Back;
    assert_eq!(drc(&b).count(C::CourtyardOverlap), 0, "different sides");
    assert_eq!(
        drc_with(&mk(0), |r| r.check_courtyards = false).count(C::CourtyardOverlap),
        0
    );
}

#[test]
fn keepouts() {
    let mut b = base_board();
    b.keepouts.push(Keepout {
        id: fresh(),
        layers: vec![Layer::FCu],
        outline: rect_region(20 * MM, 20 * MM, 30 * MM, 30 * MM),
        no_tracks: true,
        no_vias: true,
        no_pads: true,
        no_zones: true,
    });
    track(
        &mut b,
        NA,
        Layer::FCu,
        200_000,
        p(10 * MM, 25 * MM),
        p(40 * MM, 25 * MM),
    ); // crosses
    track(
        &mut b,
        NB,
        Layer::BCu,
        200_000,
        p(10 * MM, 26 * MM),
        p(40 * MM, 26 * MM),
    ); // other layer: allowed
    track(
        &mut b,
        NC,
        Layer::FCu,
        200_000,
        p(10 * MM, 40 * MM),
        p(40 * MM, 40 * MM),
    ); // outside
    via(&mut b, GND, p(25 * MM, 22 * MM), 800_000, 400_000);
    let pads = vec![smd_pad(&mut b, "1", None, p(0, 0), MM, MM, PadShape::Rect)];
    footprint(&mut b, "U1", p(22 * MM, 28 * MM), Side::Front, pads);
    zone(
        &mut b,
        GND,
        Layer::FCu,
        rect_region(28 * MM, 28 * MM, 35 * MM, 35 * MM),
    );
    let r = drc_with(&b, |r| {
        r.clearance = ClearanceTable::uniform(0);
        r.min_annular_ring = 0;
    });
    assert_eq!(r.count(C::KeepoutTrack), 1);
    assert_eq!(r.count(C::KeepoutVia), 1);
    assert_eq!(r.count(C::KeepoutPad), 1);
    assert_eq!(r.count(C::KeepoutZone), 1);
    // permissions: a keepout that only forbids tracks leaves vias alone
    b.keepouts[0].no_vias = false;
    assert_eq!(
        drc_with(&b, |r| r.clearance = ClearanceTable::uniform(0)).count(C::KeepoutVia),
        0
    );
}

#[test]
fn unfilled_zone_is_reported() {
    let mut b = base_board();
    zone(
        &mut b,
        GND,
        Layer::FCu,
        rect_region(5 * MM, 5 * MM, 15 * MM, 15 * MM),
    );
    assert_eq!(drc(&b).count(C::ZoneUnfilled), 0);
    b.zones[0].filled.clear();
    let r = drc(&b);
    assert_eq!(r.count(C::ZoneUnfilled), 1);
    assert_eq!(r.warnings(), 1);
}

#[test]
fn profiles_and_layer_counts() {
    let two = CapabilityProfile::generic_2layer();
    let four = CapabilityProfile::generic_4layer();
    let b2 = base_board();
    assert!(check_profile(&b2, &two).unwrap().is_empty());
    let mut b4 = Board::new("four", 4).unwrap();
    b4.outline = Some(Outline::rectangle(0, 0, 10 * MM, 10 * MM));
    assert_eq!(check_profile(&b4, &two).unwrap().len(), 1);
    assert!(check_profile(&b4, &four).unwrap().is_empty());
    let mut b6 = Board::new("six", 6).unwrap();
    b6.vias.push(Via {
        id: fresh(),
        net: None,
        position: p(0, 0),
        diameter: 600_000,
        drill: 300_000,
        kind: ViaKind::Blind,
        from: Layer::FCu,
        to: Layer::InnerCu(2),
        tented: true,
    });
    let v = check_profile(
        &b6,
        &CapabilityProfile {
            max_copper_layers: 6,
            ..four.clone()
        },
    )
    .unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].code, C::ProfileViaType);
    // the numeric limits of the 5/5 profile are looser than the 6/6 profile's
    let mk = |gap: i64| {
        let mut b = base_board();
        track(
            &mut b,
            NA,
            Layer::FCu,
            152_400,
            p(10 * MM, 10 * MM),
            p(20 * MM, 10 * MM),
        );
        track(
            &mut b,
            NB,
            Layer::FCu,
            152_400,
            p(10 * MM, 10 * MM + 152_400 + gap),
            p(20 * MM, 10 * MM + 152_400 + gap),
        );
        b
    };
    let mut r2 = two.rules();
    r2.check_dangling = false;
    let mut r4 = four.rules();
    r4.check_dangling = false;
    assert_eq!(
        run_drc(&mk(140_000), &r2)
            .unwrap()
            .count(C::ClearanceTrackTrack),
        1
    );
    assert_eq!(
        run_drc(&mk(140_000), &r4)
            .unwrap()
            .count(C::ClearanceTrackTrack),
        0
    );
    assert_eq!(
        run_drc(&mk(127_000), &r4)
            .unwrap()
            .count(C::ClearanceTrackTrack),
        0
    );
    assert_eq!(
        run_drc(&mk(126_999), &r4)
            .unwrap()
            .count(C::ClearanceTrackTrack),
        1
    );
    assert_eq!(
        run_drc(&mk(152_400), &r2)
            .unwrap()
            .count(C::ClearanceTrackTrack),
        0
    );
    assert_eq!(
        run_drc(&mk(152_399), &r2)
            .unwrap()
            .count(C::ClearanceTrackTrack),
        1
    );
}

#[test]
fn disabled_rules_and_truncation() {
    let mut b = base_board();
    for i in 0..20 {
        track(
            &mut b,
            NA,
            Layer::FCu,
            100_000,
            p(10 * MM, (10 + 2 * i) * MM),
            p(20 * MM, (10 + 2 * i) * MM),
        );
    }
    assert_eq!(drc(&b).count(C::TrackWidth), 20);
    assert_eq!(
        drc_with(&b, |r| r.disabled = vec![C::TrackWidth]).count(C::TrackWidth),
        0
    );
    let r = drc_with(&b, |r| r.max_violations = 5);
    assert!(r.truncated);
    assert_eq!(r.violations.len(), 5);
}

#[test]
fn reports_are_deterministic_and_serialisable() {
    let mut b = clean();
    track(
        &mut b,
        NC,
        Layer::FCu,
        100_000,
        p(10 * MM, 30 * MM),
        p(20 * MM, 30 * MM),
    );
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(10 * MM, 30 * MM + 250_000),
        p(20 * MM, 30 * MM + 250_000),
    );
    let r1 = drc(&b);
    let r2 = drc(&b);
    assert_eq!(r1, r2);
    let j = serde_json::to_string(&r1).unwrap();
    assert!(j.contains("\"code\":\"track.width\"") || j.contains("track.width"));
    assert_eq!(serde_json::from_str::<DrcReport>(&j).unwrap(), r1);
}

#[test]
fn invalid_boards_and_rules_are_errors() {
    let mut b = base_board();
    b.tracks.push(Track {
        id: fresh(),
        net: Some(NetId(77)),
        layer: Layer::FCu,
        width: 200_000,
        geom: TrackGeom::Segment {
            start: p(0, 0),
            end: p(1, 1),
        },
    });
    assert!(run_drc(&b, &rules()).is_err());
    let mut r = rules();
    r.min_track_width = -1;
    assert!(run_drc(&base_board(), &r).is_err());
}

#[test]
fn back_side_footprints_and_mirrored_pads() {
    // pads of a back footprint sit on B.Cu with x mirrored
    let mut b = base_board();
    let pads = vec![smd_pad(
        &mut b,
        "1",
        NA,
        p(2 * MM, 0),
        MM,
        MM,
        PadShape::Rect,
    )];
    footprint(&mut b, "U1", p(10 * MM, 10 * MM), Side::Back, pads);
    track(
        &mut b,
        NB,
        Layer::FCu,
        200_000,
        p(8 * MM, 5 * MM),
        p(8 * MM, 15 * MM),
    ); // front track over the pad: no conflict
    track(
        &mut b,
        NB,
        Layer::BCu,
        200_000,
        p(8 * MM, 5 * MM),
        p(8 * MM, 15 * MM),
    ); // pad is at x = 8 mm on the back
    let r = drc(&b);
    assert_eq!(r.count(C::ShortCircuit), 1);
    assert_eq!(
        r.violations
            .iter()
            .find(|v| v.code == C::ShortCircuit)
            .unwrap()
            .layer,
        Some(Layer::BCu)
    );
    let _ = Point::ORIGIN;
}

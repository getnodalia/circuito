// SPDX-License-Identifier: MIT OR Apache-2.0
//! Importing a board written in KiCad 8 syntax. The fixture is written by hand from the public
//! file-format documentation: it has not been produced or checked by KiCad itself.
use circuito_kicad::pcb::import;
use circuito_kicad::Error;
use circuito_pcb::board::{
    GraphicKind, OutlineSeg, PadKind, PadShape, ThermalMode, TrackGeom, ViaKind,
};
use circuito_pcb::layer::Layer;

const BOARD: &str = r#"(kicad_pcb (version 20240108) (generator "pcbnew") (generator_version "8.0")
  (general (thickness 1.6) (legacy_teardrops no))
  (paper "A4")
  (layers
    (0 "F.Cu" signal) (31 "B.Cu" signal)
    (36 "B.SilkS" user "B.Silkscreen") (37 "F.SilkS" user "F.Silkscreen")
    (38 "B.Mask" user) (39 "F.Mask" user) (44 "Edge.Cuts" user) (46 "B.CrtYd" user) (47 "F.CrtYd" user))
  (setup (pad_to_mask_clearance 0))
  (net 0 "")
  (net 1 "GND")
  (net 2 "VCC")
  (footprint "Resistor_SMD:R_0603_1608Metric" (layer "F.Cu") (uuid "a")
    (at 20 10 90)
    (property "Reference" "R1" (at 0 -1.4 90) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
    (property "Value" "10k" (at 0 1.4 90) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
    (attr smd)
    (fp_line (start -0.2 -0.5) (end 0.2 -0.5) (stroke (width 0.12) (type solid)) (layer "F.SilkS"))
    (fp_rect (start -1.5 -0.8) (end 1.5 0.8) (stroke (width 0.05) (type solid)) (fill none) (layer "F.CrtYd"))
    (pad "1" smd roundrect (at -0.825 0 90) (size 0.8 0.95) (layers "F.Cu" "F.Paste" "F.Mask") (roundrect_rratio 0.25) (net 1 "GND"))
    (pad "2" smd roundrect (at 0.825 0 90) (size 0.8 0.95) (layers "F.Cu" "F.Paste" "F.Mask") (roundrect_rratio 0.25) (net 2 "VCC")))
  (footprint "Connector:Pin" (layer "F.Cu") (at 40 20)
    (property "Reference" "J1" (at 0 0) (layer "F.SilkS") (effects (font (size 1 1) (thickness 0.15))))
    (property "Value" "Pin" (at 0 0) (layer "F.Fab") (effects (font (size 1 1) (thickness 0.15))))
    (pad "1" thru_hole circle (at 0 0) (size 1.7 1.7) (drill 1) (layers "*.Cu" "*.Mask") (net 1 "GND"))
    (pad "2" thru_hole oval (at 2.54 0) (size 1.7 2.2) (drill oval 1 1.5) (layers "*.Cu" "*.Mask") (net 2 "VCC")))
  (footprint "Resistor_SMD:R_0603_1608Metric" (layer "B.Cu") (at 5 5)
    (property "Reference" "R9" (at 0 0) (layer "B.SilkS") (effects (font (size 1 1) (thickness 0.15))))
    (pad "1" smd rect (at 0 0) (size 1 1) (layers "B.Cu" "B.Paste" "B.Mask")))
  (segment (start 19.175 10) (end 30 10) (width 0.25) (layer "F.Cu") (net 1) (uuid "s1"))
  (arc (start 30 10) (mid 32.12132 10.87868) (end 33 13) (width 0.25) (layer "F.Cu") (net 1) (uuid "s2"))
  (via (at 33 13) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1) (uuid "v1"))
  (segment (start 33 13) (end 33 25) (width 0.25) (layer "B.Cu") (net 1) (uuid "s3"))
  (gr_line (start 0 0) (end 50 0) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (gr_line (start 50 0) (end 50 30) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (gr_line (start 50 30) (end 0 30) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (gr_line (start 0 30) (end 0 0) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
  (gr_circle (center 5 25) (end 6.6 25) (stroke (width 0.1) (type default)) (fill none) (layer "Edge.Cuts"))
  (gr_line (start 2 2) (end 8 2) (stroke (width 0.15) (type solid)) (layer "F.SilkS"))
  (gr_text "Rev A" (at 25 27) (layer "F.SilkS") (effects (font (size 1.5 1.5) (thickness 0.2)) (justify left)))
  (zone (net 1) (net_name "GND") (layer "B.Cu") (uuid "z1") (hatch edge 0.5)
    (priority 1)
    (connect_pads (clearance 0.4))
    (min_thickness 0.2)
    (fill yes (thermal_gap 0.45) (thermal_bridge_width 0.55))
    (polygon (pts (xy 1 1) (xy 49 1) (xy 49 29) (xy 1 29)))
    (filled_polygon (layer "B.Cu") (pts (xy 1 1) (xy 49 1) (xy 49 29) (xy 1 29))))
  (dimension (type aligned) (layer "Cmts.User")))
"#;

fn close(a: i64, b: i64) -> bool {
    (a - b).abs() <= 1
}

#[test]
fn a_kicad_8_board_is_imported() {
    let imp = import(BOARD, "demo").unwrap();
    let b = &imp.board;
    assert_eq!(imp.file_version, Some(20240108));
    assert_eq!(b.copper_layers, 2);
    let names: Vec<_> = b.nets.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names, ["GND", "VCC"]);

    // Y is mirrored: KiCad's (20, 10) is (20 mm, -10 mm) here.
    assert_eq!(
        b.footprints.len(),
        2,
        "the bottom-side footprint is skipped"
    );
    let r1 = &b.footprints[0];
    assert_eq!(
        (
            r1.reference.as_str(),
            r1.value.as_str(),
            r1.package.as_str()
        ),
        ("R1", "10k", "R_0603_1608Metric")
    );
    assert_eq!(
        (r1.position.x, r1.position.y, r1.rotation),
        (20_000_000, -10_000_000, 90_000)
    );
    assert_eq!(r1.pads.len(), 2);
    let p1 = &r1.pads[0];
    assert_eq!((p1.number.as_str(), p1.kind), ("1", PadKind::Smd));
    assert_eq!(
        p1.rotation, 0,
        "the file's pad angle includes the footprint's 90 degrees"
    );
    assert_eq!(
        (p1.size.w, p1.size.h, p1.position.x),
        (800_000, 950_000, -825_000)
    );
    assert!(matches!(p1.shape, PadShape::RoundRect { corner_radius } if corner_radius == 200_000));
    assert!(p1.mask && p1.paste);
    assert_eq!(b.net_name(p1.net), "GND");
    assert!(r1.courtyard.is_some() && !r1.graphics.is_empty());

    let j1 = &b.footprints[1];
    let (pin, slot) = (&j1.pads[0], &j1.pads[1]);
    assert_eq!(pin.kind, PadKind::ThroughHole);
    assert_eq!(pin.drill.unwrap().diameter, 1_000_000);
    assert_eq!(
        (
            slot.drill.unwrap().diameter,
            slot.drill.unwrap().slot_length
        ),
        (1_000_000, Some(1_500_000))
    );
    assert!(matches!(slot.shape, PadShape::Oval));

    // Tracks, the arc and the via.
    assert_eq!(b.tracks.len(), 3);
    assert_eq!(b.tracks[0].layer, Layer::FCu);
    assert_eq!(b.tracks[0].width, 250_000);
    let TrackGeom::Arc(arc) = &b.tracks[1].geom else {
        panic!("second track is an arc")
    };
    assert!(
        close(arc.center.x, 30_000_000) && close(arc.center.y, -13_000_000),
        "{arc:?}"
    );
    assert!((arc.radius() - 3.0e6).abs() < 10.0);
    assert_eq!(b.tracks[2].layer, Layer::BCu);
    assert_eq!(b.vias.len(), 1);
    assert_eq!(
        (b.vias[0].diameter, b.vias[0].drill, b.vias[0].kind),
        (800_000, 400_000, ViaKind::Through)
    );

    // The outline is the rectangle; the circle is a cut-out.
    let o = b.outline.as_ref().expect("outline");
    assert_eq!(o.outer.len(), 4);
    assert!(o.outer.iter().all(|s| matches!(s, OutlineSeg::Line { .. })));
    assert_eq!(o.cutouts.len(), 1);

    // A zone with its stored fill, thermal settings and priority.
    assert_eq!(b.zones.len(), 1);
    let z = &b.zones[0];
    assert_eq!(
        (z.layer, z.priority, z.clearance, z.min_width),
        (Layer::BCu, 1, 400_000, 200_000)
    );
    assert_eq!(
        (z.thermal.mode, z.thermal.gap, z.thermal.spoke_width),
        (ThermalMode::Relief, 450_000, 550_000)
    );
    assert_eq!(z.filled.len(), 1);

    // Board graphics and text.
    assert!(b
        .graphics
        .iter()
        .any(|g| g.layer == Layer::FSilk && matches!(g.kind, GraphicKind::Line { .. })));
    assert_eq!(b.texts.len(), 1);
    assert_eq!(
        (b.texts[0].text.as_str(), b.texts[0].height),
        ("Rev A", 1_500_000)
    );

    // What was not imported is said, not dropped silently.
    let w = imp.warnings.join("\n");
    assert!(w.contains("R9") && w.contains("bottom side"), "{w}");
    assert!(w.contains("dimension"), "{w}");
}

#[test]
fn the_imported_board_is_usable_by_the_pcb_engine() {
    let imp = import(BOARD, "demo").unwrap();
    imp.board.validate().unwrap();
    // It round-trips through the engine's own file format.
    let text = imp.board.to_json().unwrap();
    assert_eq!(
        circuito_pcb::board::Board::from_json(&text).unwrap(),
        imp.board
    );
}

#[test]
fn other_files_and_broken_files_are_errors() {
    assert!(matches!(
        import("(kicad_sch (version 1))", "x"),
        Err(Error::WrongKind { .. })
    ));
    assert!(matches!(
        import("(kicad_pcb", "x"),
        Err(Error::Syntax { .. })
    ));
    let bad_pad = BOARD.replace("(size 0.8 0.95) (layers \"F.Cu\" \"F.Paste\" \"F.Mask\") (roundrect_rratio 0.25) (net 1 \"GND\")", "(layers \"F.Cu\")");
    assert!(matches!(import(&bad_pad, "x"), Err(Error::Invalid { .. })));
    let huge = BOARD.replace("(at 20 10 90)", "(at 2e9 10 90)");
    assert!(import(&huge, "x").is_err());
}

#[test]
fn truncated_and_mutated_boards_never_panic() {
    for n in (0..BOARD.len()).step_by(37) {
        if let Some(prefix) = BOARD.get(..n) {
            let _ = import(prefix, "x");
        }
    }
    let bytes = BOARD.as_bytes();
    let mut seed = 0x1234_5678_9abc_def1u64;
    for _ in 0..3000 {
        let mut b = bytes.to_vec();
        for _ in 0..3 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let at = (seed as usize) % b.len();
            b[at] = b"()\" 0123456789.-eE"[(seed >> 32) as usize % 18];
        }
        let text = String::from_utf8_lossy(&b).into_owned();
        let _ =
            std::panic::catch_unwind(|| import(&text, "x")).expect("the importer must not panic");
    }
}

#[test]
fn net_names_work_without_a_net_table() {
    // KiCad 10 style: pads refer to nets by name only.
    let text = r#"(kicad_pcb (version 20250101)
      (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
      (footprint "Lib:X" (layer "F.Cu") (at 0 0)
        (property "Reference" "U1")
        (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu") (net "SIG")))
      (segment (start 0 0) (end 5 0) (width 0.2) (layer "F.Cu") (net "SIG")))"#;
    let imp = import(text, "x").unwrap();
    assert_eq!(imp.board.nets.len(), 1);
    assert_eq!(imp.board.tracks[0].net, imp.board.footprints[0].pads[0].net);
}

// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::field_reassign_with_default
)]
//! Drill output read back with our own Excellon reader.

mod common;
use circuito_pcb::board::*;
use circuito_pcb::excellon::{read_excellon, write_drills, DrillOptions, ExcellonImage};
use circuito_pcb::layer::{Layer, Side};
use circuito_pcb::units::MM;
use common::*;

fn files(b: &Board) -> Vec<(String, String, ExcellonImage)> {
    write_drills(b, &DrillOptions::default())
        .unwrap()
        .files
        .into_iter()
        .map(|f| {
            let img = read_excellon(&f.content).unwrap_or_else(|e| panic!("{e}\n{}", f.content));
            (f.file_name, f.function, img)
        })
        .collect()
}

#[test]
fn plated_and_non_plated_holes_with_slots() {
    let mut b = base_board();
    let mut slot = th_pad(&mut b, "2", NB, p(0, 3 * MM), 1_600_000, 800_000);
    slot.drill = Some(Drill {
        diameter: 800_000,
        slot_length: Some(2_400_000),
    });
    let mut mount = th_pad(&mut b, "M", None, p(8 * MM, 0), 3 * MM, 2_500_000);
    mount.kind = PadKind::Npth;
    mount.mask = true;
    let pads = vec![
        th_pad(&mut b, "1", NA, p(0, 0), 1_600_000, 800_000),
        slot,
        mount,
    ];
    let fid = footprint(&mut b, "J1", p(10 * MM, 10 * MM), Side::Front, pads);
    // turned 90 degrees: the slot lies along y
    b.footprints
        .iter_mut()
        .find(|f| f.id == fid)
        .unwrap()
        .rotation = 90_000;
    via(&mut b, NA, p(20 * MM, 20 * MM), 600_000, 300_000);
    via(&mut b, NB, p(21 * MM, 20 * MM), 600_000, 300_000);
    via(&mut b, NB, p(21 * MM, 20 * MM), 600_000, 300_000); // an exact duplicate

    let f = files(&b);
    assert_eq!(f.len(), 2);
    let (name, function, pth) = &f[0];
    assert_eq!(name, "test-PTH.drl");
    assert_eq!(function, "Plated,1,2,PTH");
    assert!(pth.finished);
    assert_eq!(
        pth.file_attr("FileFunction").unwrap(),
        ["Plated", "1", "2", "PTH"]
    );
    assert_eq!(pth.file_attr("FilePolarity").unwrap(), ["Positive"]);
    // tools sorted by diameter: 0.3 vias, then 0.8 component drills
    assert_eq!(pth.tools.len(), 2);
    assert_eq!(
        (pth.tools[0].diameter, pth.tools[0].function.as_deref()),
        (300_000, Some("Plated,PTH,ViaDrill"))
    );
    assert_eq!(
        (pth.tools[1].diameter, pth.tools[1].function.as_deref()),
        (800_000, Some("Plated,PTH,ComponentDrill"))
    );
    // vias: duplicates removed, sorted by position
    let vias: Vec<_> = pth
        .holes
        .iter()
        .filter(|h| h.tool == 1)
        .map(|h| h.at)
        .collect();
    assert_eq!(vias, [p(20 * MM, 20 * MM), p(21 * MM, 20 * MM)]);
    // the round pad hole
    let comp: Vec<_> = pth
        .holes
        .iter()
        .filter(|h| h.tool == 2)
        .map(|h| h.at)
        .collect();
    assert_eq!(comp, [p(10 * MM, 10 * MM)]);
    // the slot: pad at (0, 3 mm) turned 90 degrees is at (-3 mm, 0); 2.4 mm long, 0.8 wide:
    // the centres of the end arcs are 1.6 mm apart along y
    assert_eq!(pth.slots.len(), 1);
    assert_eq!(pth.slots[0].tool, 2);
    assert_eq!(pth.slots[0].from, p(7 * MM, 10 * MM - 800_000));
    assert_eq!(pth.slots[0].to, p(7 * MM, 10 * MM + 800_000));

    let (name, function, npth) = &f[1];
    assert_eq!(name, "test-NPTH.drl");
    assert_eq!(function, "NonPlated,1,2,NPTH");
    assert_eq!(npth.tools[0].diameter, 2_500_000);
    assert_eq!(
        npth.tools[0].function.as_deref(),
        Some("NonPlated,NPTH,MechanicalDrill")
    );
    assert_eq!(npth.holes[0].at, p(10 * MM, 18 * MM));
}

#[test]
fn coordinates_and_diameters_are_exact_to_the_nanometre() {
    let mut b = base_board();
    via(&mut b, NA, p(12_345_678, 9_876_543), 600_001, 300_007);
    let f = files(&b);
    assert_eq!(f[0].2.tools[0].diameter, 300_007);
    assert_eq!(f[0].2.holes[0].at, p(12_345_678, 9_876_543));
    let text = write_drills(&b, &DrillOptions::default()).unwrap().files[0]
        .content
        .clone();
    assert!(text.contains("T1C0.300007\n"), "{text}");
    assert!(text.contains("X12.345678Y9.876543\n"), "{text}");
    assert!(text.contains("METRIC,TZ\n"));
}

#[test]
fn blind_and_buried_vias_get_a_file_per_layer_span() {
    let mut b = Board::new("four", 4).unwrap();
    b.outline = Some(Outline::rectangle(0, 0, 50 * MM, 50 * MM));
    b.nets.push(Net {
        id: NetId(1),
        name: "A".into(),
        class: DEFAULT_CLASS.to_string(),
    });
    for (i, (kind, from, to)) in [
        (ViaKind::Through, Layer::FCu, Layer::BCu),
        (ViaKind::Blind, Layer::FCu, Layer::InnerCu(1)),
        (ViaKind::Micro, Layer::FCu, Layer::InnerCu(1)),
        (ViaKind::Buried, Layer::InnerCu(1), Layer::InnerCu(2)),
        (ViaKind::Blind, Layer::InnerCu(2), Layer::BCu),
    ]
    .into_iter()
    .enumerate()
    {
        let id = via(
            &mut b,
            NA,
            p((10 + i as i64) * MM, 10 * MM),
            400_000,
            200_000,
        );
        let v = b.vias.iter_mut().find(|v| v.id == id).unwrap();
        v.kind = kind;
        v.from = from;
        v.to = to;
    }
    let f = files(&b);
    let funcs: Vec<_> = f.iter().map(|(_, fun, _)| fun.as_str()).collect();
    assert_eq!(
        funcs,
        [
            "Plated,1,4,PTH",
            "Plated,1,2,Blind",
            "Plated,2,3,Buried",
            "Plated,3,4,Blind"
        ]
    );
    let names: Vec<_> = f.iter().map(|(n, _, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "four-PTH.drl",
            "four-1-2.drl",
            "four-2-3.drl",
            "four-3-4.drl"
        ]
    );
    // the micro via and the blind via share a span but not a tool
    let blind = &f[1].2;
    assert_eq!(blind.tools.len(), 2);
    assert!(blind
        .tools
        .iter()
        .any(|t| t.function.as_deref() == Some("Plated,PTH,MicroViaDrill")));
    assert!(blind
        .tools
        .iter()
        .any(|t| t.function.as_deref() == Some("Plated,PTH,ViaDrill")));
    assert_eq!(f[2].2.holes[0].at, p(13 * MM, 10 * MM));
}

#[test]
fn output_is_deterministic_and_dates_are_injected() {
    let b = rich_board();
    let a = write_drills(&b, &DrillOptions::default()).unwrap();
    assert_eq!(a, write_drills(&b, &DrillOptions::default()).unwrap());
    assert!(!a.files[0].content.contains("CreationDate"));
    let d = write_drills(
        &b,
        &DrillOptions {
            creation_date: Some("2026-11-26T10:00:00+00:00".into()),
        },
    )
    .unwrap();
    assert!(d.files[0]
        .content
        .contains("; #@! TF.CreationDate,2026-11-26T10:00:00+00:00\n"));
    assert!(a.files[0]
        .content
        .starts_with("M48\n; #@! TF.GenerationSoftware,Nodalia,Circuito,"));
}

#[test]
fn a_board_without_holes_says_so_and_a_bad_board_is_refused() {
    let b = base_board();
    let s = write_drills(&b, &DrillOptions::default()).unwrap();
    assert!(s.files.is_empty());
    assert!(s.warnings.iter().any(|w| w.contains("no holes")));
    let mut bad = rich_board();
    bad.vias.push(bad.vias[0].clone());
    assert!(write_drills(&bad, &DrillOptions::default()).is_err());
}

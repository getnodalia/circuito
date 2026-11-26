// SPDX-License-Identifier: MIT OR Apache-2.0
//! A circuit written as text, drawn, extracted and simulated gives the physics of the circuit.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use circuito_script::{apply, compile, design_of, parse, print, PrintOptions};
use circuito_sim::{simulate_all, Control};

fn op_voltage(s: &circuito_core::schematic::Schematic, node: &str) -> f64 {
    let ex = circuito_erc::extract(s).expect("extracts");
    assert!(ex.issues.is_empty(), "{:?}", ex.issues);
    let results = simulate_all(&ex.netlist, &Control::default()).expect("options");
    let res = results
        .into_iter()
        .next()
        .expect("an analysis")
        .expect("solves");
    res.table
        .signal(&format!("V({node})"))
        .expect("signal")
        .real(0)
        .expect("sample")
}

#[test]
fn a_divider_written_as_text_divides() {
    let s = compile("V1 10 in gnd\nR1 1k in out\nR2 3k out gnd\nop\n").expect("compiles");
    assert!((op_voltage(&s, "out") - 7.5).abs() < 1e-9);
}

#[test]
fn editing_the_text_changes_the_answer_and_keeps_the_drawing() {
    let s = compile("V1 10 in gnd\nR1 1k in out\nR2 3k out gnd\nop\n").expect("compiles");
    let edited = parse("V1 10 in gnd\nR1 1k in out\nR2 1k out gnd\nop\n");
    assert!(edited.is_clean());
    let t = apply(&s, &edited.design).expect("applies");
    assert_eq!(t.sheets[0].wires, s.sheets[0].wires);
    assert!((op_voltage(&t, "out") - 5.0).abs() < 1e-9);
}

#[test]
fn a_redrawn_circuit_simulates_the_same_as_before() {
    let s = compile("V1 10 in gnd\nR1 1k in out\nR2 3k out gnd\nop\n").expect("compiles");
    let edited = parse("V1 10 in gnd\nR1 1k in out\nR2 3k out gnd\nR3 3k out gnd\nop\n");
    let t = apply(&s, &edited.design).expect("applies");
    // 3k || 3k = 1.5k: 10 * 1.5 / 2.5 = 6 V.
    assert!((op_voltage(&t, "out") - 6.0).abs() < 1e-9);
}

#[test]
fn the_text_of_a_drawing_compiles_to_the_same_circuit() {
    let s = compile("V1 SIN(0 1 1k) in gnd\nR1 1k in out\nC1 159n out gnd\nac dec 10 10 100k\n")
        .expect("compiles");
    let text = print(&design_of(&s), PrintOptions::default());
    let again = compile(&text).expect("compiles again");
    let a = circuito_erc::extract(&s).expect("extracts");
    let b = circuito_erc::extract(&again).expect("extracts");
    assert_eq!(
        a.netlist.to_string().lines().count(),
        b.netlist.to_string().lines().count()
    );
    assert_eq!(a.netlist.analyses, b.netlist.analyses);
}

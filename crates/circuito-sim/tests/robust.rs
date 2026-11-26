// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Hostile input: random netlists with nasty values must come back as a result or an error and
//! never panic, hang or return non-finite numbers.

mod common;
use common::*;

use circuito_sim::{simulate, Control, Options, SimError, Values};
use proptest::prelude::*;

fn node() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec!["0", "1", "2", "3", "gnd"])
}

fn value() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "1", "1k", "100", "1meg", "10u", "1n", "1p", "1e-30", "1e30", "1e-12", "1e12", "0", "-1",
        "-1k", "2.2", "47k", "1e300",
    ])
}

fn element() -> impl Strategy<Value = String> {
    (
        0usize..16,
        node(),
        node(),
        node(),
        node(),
        value(),
        0usize..1000,
    )
        .prop_map(|(kind, a, b, c, d, v, id)| match kind {
            0 | 1 => format!("R{id} {a} {b} {v}"),
            2 => format!("C{id} {a} {b} {v}"),
            3 => format!("L{id} {a} {b} {v}"),
            4 => format!("V{id} {a} {b} DC {v}"),
            5 => format!("I{id} {a} {b} DC {v}"),
            6 => format!("D{id} {a} {b} DM"),
            7 => format!("Q{id} {a} {b} {c} QM"),
            8 => format!("M{id} {a} {b} {c} {d} NM"),
            9 => format!("E{id} {a} {b} {c} {d} {v}"),
            10 => format!("G{id} {a} {b} {c} {d} {v}"),
            11 => format!("S{id} {a} {b} {c} {d} SM"),
            12 => format!("U{id} {a} {b} {c} OA"),
            13 => format!("V{id} {a} {b} PULSE(0 {v} 0 1n 1n 1u 3u) AC 1"),
            14 => format!("V{id} {a} {b} SIN(0 {v} 1meg) AC 1"),
            _ => format!("R{id} {a} {b} 1k"),
        })
}

const MODELS: &str = ".model DM D(IS=1e-14 CJO=1p RS=1)\n.model QM NPN(IS=1e-14 BF=100 CJE=1p CJC=1p)\n.model NM NMOS(VTO=1 KP=2e-5 CGSO=1n)\n.model SM SW(VT=1 VH=0.1)\n.model OA OPAMP(GAIN=1e5 VMAX=10 VMIN=-10)\n";

fn quick() -> Options {
    Options {
        max_points: 400,
        itl1: 20,
        ..Options::default()
    }
}

fn assert_finite(r: &circuito_sim::AnalysisResult) {
    assert!(r.table.axis.iter().all(|x| x.is_finite()));
    for s in &r.table.signals {
        match &s.values {
            Values::Real(v) => assert!(
                v.iter().all(|x| x.is_finite()),
                "{} has a non-finite value",
                s.name
            ),
            Values::Complex(v) => assert!(
                v.iter().all(|z| z.re.is_finite() && z.im.is_finite()),
                "{} has a non-finite value",
                s.name
            ),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 400, ..ProptestConfig::default() })]

    #[test]
    fn random_netlists_never_panic_and_never_return_non_finite_numbers(els in prop::collection::vec(element(), 1..9)) {
        let mut text = String::from("fuzz\n");
        for e in &els {
            text.push_str(e);
            text.push('\n');
        }
        text.push_str(MODELS);
        text.push_str(".op\n.dc V1 0 1 0.5\n.ac dec 2 1k 1meg\n.tran 100n 3u\n");
        let Ok(parsed) = circuito_spice::parse(&text) else { return Ok(()); };
        let n = parsed.netlist;
        for a in &n.analyses {
            match simulate(&n, a, &quick(), &Control::new()) {
                Ok(r) => assert_finite(&r),
                Err(e) => {
                    // Every failure has a readable message.
                    prop_assert!(!e.to_string().is_empty());
                }
            }
        }
    }
}

#[test]
fn a_huge_circuit_is_refused_rather_than_attempted() {
    let mut text = String::from("huge\nV1 n0 0 DC 1\n");
    for i in 0..(circuito_sim::MAX_UNKNOWNS + 10) {
        text.push_str(&format!("R{i} n{i} n{} 1k\n", i + 1));
    }
    text.push_str(".op\n");
    let n = parse(&text);
    let e = simulate(&n, &n.analyses[0], &Options::default(), &Control::new()).unwrap_err();
    assert!(matches!(e, SimError::TooLarge(_)), "{e}");
}

#[test]
fn extreme_but_legal_values_do_not_break_the_solver() {
    // A 1e12 ohm resistor across a 1e-12 ohm one: huge dynamic range, still a clean answer.
    let r = run("range\nV1 a 0 DC 1\nR1 a b 1e12\nR2 b 0 1e-3\n.op\n");
    assert_close(
        op(&r.table, "V(b)"),
        1e-3 / (1e12 + 1e-3),
        1e-6,
        1e-18,
        "extreme divider",
    );
    // Nonsense options are refused up front.
    let n = parse("o\nV1 a 0 DC 1\nR1 a 0 1k\n.op\n");
    for bad in [
        Options {
            reltol: 0.0,
            ..Options::default()
        },
        Options {
            reltol: f64::NAN,
            ..Options::default()
        },
        Options {
            gmin: -1.0,
            ..Options::default()
        },
        Options {
            temp: -500.0,
            ..Options::default()
        },
        Options {
            itl1: 1,
            ..Options::default()
        },
    ] {
        assert!(matches!(
            simulate(&n, &n.analyses[0], &bad, &Control::new()),
            Err(SimError::Invalid(_))
        ));
    }
}

#[test]
fn simulate_all_runs_every_card_and_options_cards_apply() {
    let n = parse("all\nV1 in 0 DC 1 AC 1\nR1 in out 1k\nC1 out 0 1u\n.options reltol=1e-6 temp=50\n.op\n.ac dec 3 10 1k\n.tran 100u 1m\n");
    let rs = circuito_sim::simulate_all(&n, &Control::new()).unwrap();
    assert_eq!(rs.len(), 3);
    for r in &rs {
        let r = r.as_ref().unwrap();
        assert_eq!(r.info.reltol, 1e-6);
        assert_eq!(r.info.temp, 50.0);
        assert!(r.info.engine.starts_with("circuito-sim "));
    }
    let n = parse("badopt\nR1 a 0 1k\n.options reltol=0\n.op\n");
    assert!(circuito_sim::simulate_all(&n, &Control::new()).is_err());
}

#[test]
fn subcircuits_are_simulated_after_flattening() {
    let r = run_tight(
        "sub\n.subckt DIV a b o\nR1 a o 1k\nR2 o b 3k\n.ends\nV1 in 0 DC 8\nX1 in 0 mid DIV\n.op\n",
    );
    assert_close(
        op(&r.table, "V(mid)"),
        6.0,
        1e-12,
        0.0,
        "subcircuit divider",
    );
    assert!(
        r.table.signal("I(R1.X1)").is_some(),
        "flattened names follow <inner>.<instance>"
    );
}

#[test]
fn an_unknown_dc_source_is_reported_by_name() {
    let n = parse("x\nV1 a 0 DC 1\nR1 a 0 1k\n.op\n");
    let a = circuito_core::netlist::Analysis::Dc {
        source: "V9".into(),
        start: 0.0,
        stop: 1.0,
        step: 0.5,
    };
    let e = simulate(&n, &a, &Options::default(), &Control::new()).unwrap_err();
    assert!(e.to_string().contains("V9"), "{e}");
}

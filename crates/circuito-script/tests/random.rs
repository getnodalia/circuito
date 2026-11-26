// SPDX-License-Identifier: MIT OR Apache-2.0
//! Whatever circuit is written, it is drawn, and the drawing has the circuit's connections.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use circuito_script::{apply, build, design_of, parse, print, PrintOptions};
use proptest::prelude::*;
use std::collections::BTreeSet;

const KINDS: &[(&str, usize)] = &[
    ("R", 2),
    ("C", 2),
    ("L", 2),
    ("D", 2),
    ("V", 2),
    ("Q:NPN", 3),
    ("Q:PNP", 3),
    ("M:NMOS", 3),
    ("U:OPAMP", 3),
    ("S:SWITCH", 4),
    ("T:TRANSFORMER", 4),
    ("RV:POT", 3),
];

const NETS: &[&str] = &[
    "gnd", "in", "out", "vcc", "a", "b", "c", "n1", "n2", "+5V", "_", "?",
];

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        (0..KINDS.len(), prop::collection::vec(0..NETS.len(), 4)),
        1..14,
    )
    .prop_map(|parts| {
        let mut out = String::new();
        for (i, (k, nets)) in parts.into_iter().enumerate() {
            let (kind, pins) = KINDS[k];
            let (prefix, lib) = kind.split_once(':').unwrap_or((kind, ""));
            let head = if lib.is_empty() {
                format!("{prefix}{i}")
            } else {
                format!("{prefix}{i}:{lib}")
            };
            let list: Vec<&str> = nets.iter().take(pins).map(|&n| NETS[n]).collect();
            out.push_str(&format!("{head} {}\n", list.join(" ")));
        }
        out
    })
}

fn partition(d: &circuito_script::Design) -> BTreeSet<BTreeSet<(String, usize)>> {
    let mut groups: std::collections::BTreeMap<String, BTreeSet<(String, usize)>> =
        Default::default();
    let mut singles = BTreeSet::new();
    for p in &d.parts {
        for (k, pin) in p.pins.iter().enumerate() {
            match pin {
                circuito_script::PinNet::Net(n) => {
                    groups
                        .entry(circuito_script::net_key(n))
                        .or_default()
                        .insert((p.refdes.to_lowercase(), k));
                }
                _ => {
                    singles.insert(BTreeSet::from([(
                        format!("single:{}", p.refdes.to_lowercase()),
                        k,
                    )]));
                }
            }
        }
    }
    let mut all: BTreeSet<_> = BTreeSet::new();
    for g in groups.into_values() {
        if g.len() == 1 {
            // A net with one pin joins nothing, like an open pin.
            all.extend(
                g.into_iter()
                    .map(|(r, k)| BTreeSet::from([(format!("single:{r}"), k)])),
            );
        } else {
            all.insert(g);
        }
    }
    all.extend(singles);
    all
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn any_circuit_is_drawn_with_its_connections(t in text()) {
        let parsed = parse(&t);
        prop_assert!(parsed.problems.is_empty(), "{:?}\n{t}", parsed.problems);
        let s = build(&parsed.design, None).map_err(|e| TestCaseError::fail(format!("{e}\n{t}")))?;
        prop_assert_eq!(partition(&design_of(&s)), partition(&parsed.design), "{}", t);
    }

    #[test]
    fn printing_what_was_drawn_gives_the_same_circuit(t in text()) {
        let parsed = parse(&t);
        let s = build(&parsed.design, None).map_err(|e| TestCaseError::fail(format!("{e}\n{t}")))?;
        let back = design_of(&s);
        let again = parse(&print(&back, PrintOptions::default()));
        prop_assert!(again.problems.is_empty());
        prop_assert_eq!(partition(&again.design), partition(&parsed.design));
        let s2 = apply(&s, &again.design).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(&s2.sheets[0].wires, &s.sheets[0].wires);
    }

    #[test]
    fn redrawing_after_an_edit_keeps_the_connections(t in text(), extra in 0..NETS.len()) {
        let parsed = parse(&t);
        let s = build(&parsed.design, None).map_err(|e| TestCaseError::fail(format!("{e}\n{t}")))?;
        let edited = parse(&format!("{t}Rx 1k {} {}\n", NETS[extra], NETS[(extra + 1) % NETS.len()]));
        let u = apply(&s, &edited.design).map_err(|e| TestCaseError::fail(format!("{e}\n{t}")))?;
        prop_assert_eq!(partition(&design_of(&u)), partition(&edited.design));
    }
}

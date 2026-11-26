// SPDX-License-Identifier: MIT OR Apache-2.0
//! One schematic per rule that must fire, plus clean schematics that must stay silent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use circuito_core::schematic::builder::{Builder, PartId};
use circuito_core::schematic::{Junction, Label, LabelScope, NoConnect, Point, Schematic, Wire};
use circuito_erc::{check, ErcReport, Rule, Severity};

fn count(r: &ErcReport, rule: Rule) -> usize {
    r.of(rule).count()
}

fn only(r: &ErcReport, rule: Rule) -> &circuito_erc::Violation {
    let v: Vec<_> = r.of(rule).collect();
    assert_eq!(
        v.len(),
        1,
        "expected exactly one {rule:?}, report: {:#?}",
        r.violations
    );
    v[0]
}

fn assert_clean(s: &Schematic) {
    let r = check(s);
    assert!(
        r.violations.is_empty(),
        "expected a clean report, got {:#?}",
        r.violations
    );
}

/// V1 + R1 + R2 divider, fully labelled and grounded.
fn divider() -> Builder {
    let mut b = Builder::new("divider");
    let v = b.place("V", "V1", "10", 0, 0);
    let r1 = b.place("R", "R1", "1k", 20, 0);
    let r2 = b.place("R", "R2", "2.2k", 40, 0);
    b.power_pin(v, "2", "GND");
    b.power_pin(r2, "2", "GND");
    b.label_pin(v, "1", "IN");
    b.label_pin(r1, "1", "IN");
    b.label_pin(r1, "2", "OUT");
    b.label_pin(r2, "1", "OUT");
    b
}

// ---- clean schematics -------------------------------------------------------------------------

#[test]
fn clean_divider() {
    assert_clean(&divider().build());
}

#[test]
fn clean_rc_lowpass() {
    let mut b = Builder::new("rc");
    let v = b.place("V", "V1", "AC 1", 0, 0);
    let r = b.place("R", "R1", "1k", 20, 0);
    let c = b.place("C", "C1", "100n", 40, 0);
    b.power_pin(v, "2", "0");
    b.power_pin(c, "2", "0");
    b.label_pin(v, "1", "IN");
    b.label_pin(r, "1", "IN");
    b.label_pin(r, "2", "OUT");
    b.label_pin(c, "1", "OUT");
    assert_clean(&b.build());
}

#[test]
fn clean_inverting_amplifier_with_op_amp() {
    let mut b = Builder::new("inv");
    let v = b.place("V", "V1", "SIN(0 0.1 1k)", 0, 0);
    let rin = b.place("R", "R1", "1k", 20, 0);
    let rf = b.place("R", "R2", "10k", 40, 0);
    let u = b.place("OPAMP", "U1", "", 60, 0);
    let rl = b.place("R", "R3", "10k", 90, 0);
    b.power_pin(v, "2", "GND");
    b.power_pin(u, "+", "GND");
    b.power_pin(rl, "2", "GND");
    for (p, pin, n) in [
        (v, "1", "IN"),
        (rin, "1", "IN"),
        (rin, "2", "INV"),
        (rf, "1", "INV"),
        (u, "-", "INV"),
        (rf, "2", "OUT"),
        (u, "OUT", "OUT"),
        (rl, "1", "OUT"),
    ] {
        b.label_pin(p, pin, n);
    }
    assert_clean(&b.build());
}

#[test]
fn clean_led_circuit_with_wires() {
    let mut b = Builder::new("led");
    let v = b.place("V", "V1", "5", 0, 0);
    let r = b.place("R", "R1", "330", 30, 0);
    let d = b.place("LED", "D1", "", 60, 12);
    b.connect((v, "1"), (r, "1"));
    b.connect((r, "2"), (d, "A"));
    b.power_pin(d, "K", "GND");
    b.power_pin(v, "2", "GND");
    assert_clean(&b.build());
}

#[test]
fn clean_opamp_with_supply_pins() {
    let mut b = Builder::new("pwr");
    let vp = b.place("V", "V1", "15", 0, 0);
    let vn = b.place("V", "V2", "15", 20, 0);
    let u = b.place("OPAMP_PWR", "U1", "", 50, 0);
    let r = b.place("R", "R1", "1k", 80, 0);
    b.power_pin(vp, "2", "GND");
    b.power_pin(vn, "1", "GND");
    b.label_pin(vp, "1", "VCC");
    b.label_pin(u, "V+", "VCC");
    b.label_pin(vn, "2", "VEE");
    b.label_pin(u, "V-", "VEE");
    b.label_pin(u, "OUT", "OUT");
    b.label_pin(u, "-", "OUT");
    b.label_pin(u, "+", "IN");
    b.label_pin(r, "1", "IN");
    b.power_pin(r, "2", "GND");
    let rep = check(&b.build());
    // V2's "+" pin is on GND here by design (it is a negative rail): that is a PowerOut on ground.
    assert!(rep.is_clean_of_errors(), "{:#?}", rep.violations);
}

// ---- one rule per schematic -------------------------------------------------------------------

#[test]
fn e001_pin_unconnected_and_no_connect_flag_cures_it() {
    let mut b = divider();
    let r3 = b.place("R", "R3", "1k", 60, 0);
    b.label_pin(r3, "1", "OUT");
    let s = b.build();
    let r = check(&s);
    let v = only(&r, Rule::PinUnconnected);
    assert_eq!(v.code, "E001");
    assert_eq!(v.severity, Severity::Error);
    assert!(v.message.contains("R3.2"), "{}", v.message);
    assert_eq!(v.items, vec!["R3.2"]);
    assert_eq!(v.position, Some(Point::new(60, 4)));
    // The flag removes it.
    let mut s2 = s.clone();
    s2.sheets[0].no_connects.push(NoConnect {
        position: Point::new(60, 4),
    });
    assert_eq!(count(&check(&s2), Rule::PinUnconnected), 0);
}

#[test]
fn e002_net_with_a_single_pin() {
    let mut b = divider();
    let r3 = b.place("R", "R3", "1k", 60, 0);
    b.label_pin(r3, "1", "OUT");
    let p = b.pin(r3, "2").unwrap();
    b.wire(&[p, Point::new(p.x, p.y + 10)]); // a stub that leads nowhere
    let r = check(&b.build());
    let v = only(&r, Rule::NetSinglePin);
    assert_eq!(v.severity, Severity::Warning);
    assert!(v.message.contains("R3.2"));
    assert_eq!(count(&r, Rule::PinUnconnected), 0);
}

#[test]
fn e003_no_connect_pin_that_is_connected() {
    let mut b = divider();
    let r2 = PartId(2);
    b.no_connect_pin(r2, "1"); // R2.1 is on net OUT
    let r = check(&b.build());
    let v = only(&r, Rule::NoConnectConnected);
    assert_eq!(v.severity, Severity::Error);
    assert!(v.message.contains("R2.1"), "{}", v.message);
}

#[test]
fn e010_two_outputs_on_one_net() {
    let mut b = Builder::new("fight");
    let u1 = b.place("OPAMP", "U1", "", 0, 0);
    let u2 = b.place("OPAMP", "U2", "", 30, 0);
    b.label_pin(u1, "OUT", "BUS");
    b.label_pin(u2, "OUT", "BUS");
    for u in [u1, u2] {
        b.power_pin(u, "+", "GND");
        b.power_pin(u, "-", "GND");
    }
    let r = check(&b.build());
    let v = only(&r, Rule::OutputConflict);
    assert_eq!(v.severity, Severity::Error);
    assert!(
        v.message.contains("U1.1") && v.message.contains("U2.1") && v.message.contains("BUS"),
        "{}",
        v.message
    );
}

#[test]
fn e011_two_power_outputs_and_e050_parallel_sources() {
    let mut b = Builder::new("two supplies");
    let v1 = b.place("V", "V1", "5", 0, 0);
    let v2 = b.place("V", "V2", "3.3", 20, 0);
    for v in [v1, v2] {
        b.label_pin(v, "1", "VCC");
        b.power_pin(v, "2", "GND");
    }
    let r = check(&b.build());
    only(&r, Rule::PowerOutputConflict);
    let loop_v = only(&r, Rule::VoltageSourceLoop);
    assert!(
        loop_v.message.contains("V1")
            && loop_v.message.contains("V2")
            && loop_v.message.contains("parallel"),
        "{}",
        loop_v.message
    );
}

#[test]
fn e012_output_into_a_power_input_is_a_warning() {
    let mut b = Builder::new("odd");
    let u1 = b.place("OPAMP", "U1", "", 0, 0);
    let u2 = b.place("OPAMP_PWR", "U2", "", 30, 0);
    b.label_pin(u1, "OUT", "X");
    b.label_pin(u2, "V+", "X");
    let r = check(&b.build());
    let v = only(&r, Rule::PinTypeConflict);
    assert_eq!(v.severity, Severity::Warning);
}

#[test]
fn e020_power_input_without_a_source() {
    let mut b = Builder::new("unpowered");
    let u = b.place("OPAMP_PWR", "U1", "", 0, 0);
    b.label_pin(u, "V+", "VCC");
    b.label_pin(u, "V-", "VEE");
    let r = check(&b.build());
    assert_eq!(count(&r, Rule::PowerInputUndriven), 2);
    let v = r.of(Rule::PowerInputUndriven).next().unwrap();
    assert!(
        v.message.contains("U1.8") || v.message.contains("U1.4"),
        "{}",
        v.message
    );
}

#[test]
fn e021_no_ground() {
    let mut b = Builder::new("floating");
    let v = b.place("V", "V1", "5", 0, 0);
    let r = b.place("R", "R1", "1k", 20, 0);
    b.connect((v, "1"), (r, "1"));
    b.connect((v, "2"), (r, "2"));
    let rep = check(&b.build());
    let v = only(&rep, Rule::NoGround);
    assert!(v.message.contains("ground"));
    // Adding a ground clears it.
    let mut b = Builder::new("grounded");
    let v = b.place("V", "V1", "5", 0, 0);
    let r = b.place("R", "R1", "1k", 20, 0);
    b.label_pin(v, "1", "A");
    b.label_pin(r, "1", "A");
    b.power_pin(v, "2", "0");
    b.power_pin(r, "2", "0");
    assert_eq!(count(&check(&b.build()), Rule::NoGround), 0);
}

#[test]
fn e030_duplicate_reference() {
    let mut b = divider();
    let r = b.place("R", "R1", "4.7k", 60, 0);
    b.label_pin(r, "1", "OUT");
    b.power_pin(r, "2", "GND");
    let rep = check(&b.build());
    let v = only(&rep, Rule::DuplicateRefdes);
    assert!(v.message.contains("R1"));
    // Reported once, not also as a rejected part.
    assert_eq!(count(&rep, Rule::PartRejected), 0);
}

#[test]
fn e031_e032_e033_e034_part_problems() {
    let mut b = divider();
    let r = b.place("R", "R7", "", 60, 0);
    b.label_pin(r, "1", "OUT");
    b.power_pin(r, "2", "GND");
    let r = b.place("R", "R8", "10 ohmz", 80, 0);
    b.label_pin(r, "1", "OUT");
    b.power_pin(r, "2", "GND");
    let q = b.place("NPN", "Q1", "", 100, 0);
    b.model(q, "MYSTERY");
    for pin in ["C", "B", "E"] {
        b.label_pin(q, pin, "OUT");
    }
    let rep = check(&b.build());
    assert!(only(&rep, Rule::MissingValue).message.contains("R7"));
    assert!(only(&rep, Rule::InvalidValue).message.contains("R8"));
    assert!(only(&rep, Rule::UnknownModel).message.contains("MYSTERY"));
    let mut s = divider().build();
    s.sheets[0].parts[0].lib_id = "NOPE".into();
    let rep = check(&s);
    assert!(only(&rep, Rule::UnknownSymbol).message.contains("NOPE"));
    assert_eq!(
        rep.violations.len(),
        1,
        "an unknown symbol stops further checks"
    );
}

#[test]
fn e040_floating_input() {
    let mut b = Builder::new("float");
    let u1 = b.place("OPAMP", "U1", "", 0, 0);
    let u2 = b.place("OPAMP", "U2", "", 30, 0);
    b.label_pin(u1, "-", "FLOAT");
    b.label_pin(u2, "-", "FLOAT");
    for u in [u1, u2] {
        b.power_pin(u, "+", "GND");
        b.label_pin(u, "OUT", &format!("O{}", u.0));
    }
    let r = check(&b.build());
    let v = only(&r, Rule::FloatingInput);
    assert!(
        v.message.contains("U1.2") && v.message.contains("U2.2"),
        "{}",
        v.message
    );
}

#[test]
fn e050_loops_and_self_shorts() {
    // Three sources in a loop: V1 between A and B, V2 between B and C, V3 between C and A.
    let mut b = Builder::new("loop");
    let v1 = b.place("V", "V1", "1", 0, 0);
    let v2 = b.place("V", "V2", "1", 20, 0);
    let v3 = b.place("V", "V3", "1", 40, 0);
    b.label_pin(v1, "1", "A");
    b.label_pin(v1, "2", "B");
    b.label_pin(v2, "1", "B");
    b.label_pin(v2, "2", "C");
    b.label_pin(v3, "1", "C");
    b.label_pin(v3, "2", "A");
    b.power_pin(v1, "2", "0");
    let rep = check(&b.build());
    let v = rep.of(Rule::VoltageSourceLoop).next().expect("loop found");
    assert!(v.message.contains("loop"), "{}", v.message);
    assert!(v.items.len() == 3, "{:?}", v.items);
    // A source with both ends on one net.
    let mut b = Builder::new("self");
    let v = b.place("V", "V1", "1", 0, 0);
    b.label_pin(v, "1", "X");
    b.label_pin(v, "2", "X");
    b.power_pin(v, "2", "0");
    let rep = check(&b.build());
    assert!(only(&rep, Rule::VoltageSourceLoop).message.contains("V1"));
}

#[test]
fn e051_current_sources_in_series() {
    let mut b = Builder::new("series I");
    let i1 = b.place("I", "I1", "1m", 0, 0);
    let i2 = b.place("I", "I2", "1m", 20, 0);
    b.power_pin(i1, "2", "0");
    b.label_pin(i1, "1", "X");
    b.label_pin(i2, "2", "X");
    b.power_pin(i2, "1", "0");
    let rep = check(&b.build());
    let v = only(&rep, Rule::CurrentSourceCutset);
    assert_eq!(v.severity, Severity::Error);
    assert!(
        v.message.contains("I1") && v.message.contains("I2"),
        "{}",
        v.message
    );
    // With a resistor to ground the current has somewhere to go.
    let mut b = Builder::new("ok");
    let i1 = b.place("I", "I1", "1m", 0, 0);
    let r = b.place("R", "R1", "1k", 20, 0);
    b.power_pin(i1, "2", "0");
    b.label_pin(i1, "1", "X");
    b.label_pin(r, "1", "X");
    b.power_pin(r, "2", "0");
    assert_clean(&b.build());
}

#[test]
fn e052_no_dc_path_to_ground() {
    let mut b = Builder::new("cap divider");
    let v = b.place("V", "V1", "5", 0, 0);
    let c1 = b.place("C", "C1", "1u", 20, 0);
    let c2 = b.place("C", "C2", "1u", 40, 0);
    b.label_pin(v, "1", "IN");
    b.label_pin(c1, "1", "IN");
    b.label_pin(c1, "2", "MID");
    b.label_pin(c2, "1", "MID");
    b.power_pin(c2, "2", "0");
    b.power_pin(v, "2", "0");
    let rep = check(&b.build());
    let v = only(&rep, Rule::NoDcPath);
    assert_eq!(v.severity, Severity::Warning);
    assert!(v.message.contains("MID"), "{}", v.message);
}

#[test]
fn e060_a_net_with_two_names() {
    let mut b = divider();
    let r2 = PartId(2);
    b.label_pin(r2, "1", "ALIAS");
    let rep = check(&b.build());
    let v = only(&rep, Rule::NetNameConflict);
    assert!(
        v.message.contains("ALIAS") && v.message.contains("OUT"),
        "{}",
        v.message
    );
    // GND and 0 are the same name, so they do not conflict.
    let mut b = divider();
    b.power_pin(PartId(0), "2", "0");
    assert_eq!(count(&check(&b.build()), Rule::NetNameConflict), 0);
}

#[test]
fn e07x_connectivity_findings() {
    let mut s = Schematic::new("geo");
    let sh = &mut s.sheets[0];
    let w = |pts: &[(i32, i32)]| Wire {
        points: pts.iter().map(|&(x, y)| Point::new(x, y)).collect(),
    };
    // E070: a wire ends on the middle of another without a junction.
    sh.wires.push(w(&[(0, 0), (10, 0)]));
    sh.wires.push(w(&[(5, 0), (5, 10)]));
    // E071 floating label, E072 ambiguous label.
    sh.labels.push(Label {
        name: "LOST".into(),
        position: Point::new(100, 100),
        scope: LabelScope::Local,
    });
    sh.wires.push(w(&[(40, 0), (60, 0)]));
    sh.wires.push(w(&[(50, -10), (50, 10)]));
    sh.labels.push(Label {
        name: "AMB".into(),
        position: Point::new(50, 0),
        scope: LabelScope::Local,
    });
    // E073, E074, E075.
    sh.power_ports.push(circuito_core::schematic::PowerPort {
        name: "VCC".into(),
        position: Point::new(200, 200),
        style: Default::default(),
    });
    sh.junctions.push(Junction {
        position: Point::new(300, 300),
    });
    sh.no_connects.push(NoConnect {
        position: Point::new(400, 400),
    });
    let rep = check(&s);
    for (rule, sev) in [
        (Rule::TouchWithoutJunction, Severity::Warning),
        (Rule::FloatingLabel, Severity::Warning),
        (Rule::AmbiguousLabel, Severity::Warning),
        (Rule::FloatingPowerPort, Severity::Warning),
        (Rule::DanglingJunction, Severity::Info),
        (Rule::DanglingNoConnect, Severity::Info),
    ] {
        let v = only(&rep, rule);
        assert_eq!(v.severity, sev, "{rule:?}");
        assert!(v.position.is_some());
        assert!(v.sheet_name.as_deref() == Some("Sheet1"));
    }
}

// ---- report properties ------------------------------------------------------------------------

#[test]
fn reports_are_ordered_deterministic_and_serialise() {
    let mut b = divider();
    let r3 = b.place("R", "R3", "", 60, 0);
    b.label_pin(r3, "1", "OUT");
    let s = b.build();
    let a = check(&s);
    assert_eq!(a, check(&s));
    let sev: Vec<_> = a.violations.iter().map(|v| v.severity).collect();
    assert!(
        sev.windows(2).all(|w| w[0] >= w[1]),
        "errors first: {sev:?}"
    );
    let json: serde_json::Value = serde_json::from_str(&a.to_json()).unwrap();
    assert!(json["violations"][0]["code"]
        .as_str()
        .unwrap()
        .starts_with('E'));
    // Codes are unique per rule.
    let codes: std::collections::BTreeSet<_> = [
        Rule::PinUnconnected,
        Rule::NetSinglePin,
        Rule::NoConnectConnected,
        Rule::OutputConflict,
        Rule::PowerOutputConflict,
        Rule::PinTypeConflict,
        Rule::PowerInputUndriven,
        Rule::NoGround,
        Rule::DuplicateRefdes,
        Rule::MissingValue,
        Rule::InvalidValue,
        Rule::UnknownModel,
        Rule::UnknownSymbol,
        Rule::PartRejected,
        Rule::FloatingInput,
        Rule::VoltageSourceLoop,
        Rule::CurrentSourceCutset,
        Rule::NoDcPath,
        Rule::NetNameConflict,
        Rule::TouchWithoutJunction,
        Rule::FloatingLabel,
        Rule::AmbiguousLabel,
        Rule::FloatingPowerPort,
        Rule::DanglingJunction,
        Rule::DanglingNoConnect,
    ]
    .iter()
    .map(|r| r.code())
    .collect();
    assert_eq!(codes.len(), 25);
}

#[test]
fn random_schematics_never_panic() {
    // A tiny xorshift so the test is deterministic without a dependency.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move |m: i32| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % (m as u64)) as i32
    };
    let ids = [
        "R",
        "C",
        "C_POL",
        "L",
        "V",
        "I",
        "D",
        "LED",
        "ZENER",
        "NPN",
        "PNP",
        "NMOS",
        "PMOS",
        "OPAMP",
        "OPAMP_PWR",
        "SWITCH",
        "POT",
        "TRANSFORMER",
        "FUSE",
    ];
    let values = [
        "",
        "1k",
        "abc",
        "SIN(0 1 1k)",
        "5",
        "-1",
        "0",
        "1e999",
        "AC 1 90",
    ];
    for round in 0..60 {
        let mut b = Builder::new("fuzz");
        let n = 3 + next(10);
        let mut parts = Vec::new();
        for k in 0..n {
            let id = ids[next(ids.len() as i32) as usize];
            parts.push(b.place(
                id,
                &format!("X{}", k % 5),
                values[next(values.len() as i32) as usize],
                next(12) * 4,
                next(6) * 8,
            ));
        }
        for _ in 0..next(12) {
            let p = parts[next(parts.len() as i32) as usize];
            let name = ["A", "B", "GND", "0", "VCC"][next(5) as usize];
            let pin = ["1", "2", "3", "A", "K", "C", "B", "E", "D", "G", "S"][next(11) as usize];
            if next(2) == 0 {
                b.label_pin(p, pin, name)
            } else {
                b.power_pin(p, pin, name)
            }
        }
        for _ in 0..next(6) {
            b.wire(&[
                Point::new(next(40), next(40)),
                Point::new(next(40), next(40)),
            ]);
        }
        if next(3) == 0 {
            b.junction(Point::new(next(40), next(40)));
        }
        let s = b.build();
        let rep = check(&s);
        let _ = rep.to_json();
        let _ = circuito_erc::extract(&s).map(|x| x.netlist.to_string());
        let _ = round;
    }
}

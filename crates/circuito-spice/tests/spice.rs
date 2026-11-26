// SPDX-License-Identifier: MIT OR Apache-2.0
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use circuito_core::netlist::{Analysis, Element, ModelKind, Sweep};
use circuito_spice::{parse, write};

const FULL: &str = "\
Full featured netlist
* a comment line
.param rval=4.7k fc={1/(2*pi*rval*100n)}
.options reltol=1e-4 abstol=1p
.model DMOD D(IS=1e-14 RS=0.5 CJO=2p)
.model QN NPN (IS=1e-15 BF=150 VAF=80)
.model MN NMOS(VTO=0.8 KP=100u LAMBDA=0.02)
.model SW1 SW(VT=1 VH=0.1 RON=1 ROFF=1meg)
.model OA OPAMP(GAIN=1e5 VMAX=12 VMIN=-12)
.subckt DIV in out ; the divider
R1 in out {rval}
R2 out 0 {rval*2}
.ends DIV
V1 in 0 DC 0 AC 1 SIN(0 1 1k)   ; source
VP p 0 PULSE(0 5 0 1n 1n 5u
+ 10u)
I1 0 n1 PWL(0 0 1m 1m)
R1 in out {rval}
C1 out 0 100n IC=0
L1 out x 10m
L2 y 0 10m
K1 L1 L2 0.9
D1 out 0 DMOD AREA=2
E1 e 0 out 0 10
G1 g 0 out 0 1m
F1 f 0 V1 2
H1 h 0 V1 100
S1 out s in 0 SW1
Q1 c b 0 QN
M1 d g 0 0 MN W=10u L=1u
U1 b out o OA
X1 in mid DIV
.op
.dc V1 0 5 0.5
.ac dec 10 1 1meg
.tran 1u 1m 0 10n uic
.end
ignored after end
";

#[test]
fn parses_a_full_netlist() {
    let p = parse(FULL).unwrap();
    let n = &p.netlist;
    assert_eq!(n.title, "Full featured netlist");
    assert_eq!(n.elements.len(), 18);
    assert_eq!(n.models.len(), 5);
    assert_eq!(n.models["QN"].kind, ModelKind::Npn);
    assert_eq!(n.models["QN"].params["BF"], 150.0);
    assert_eq!(n.subckts["DIV"].elements.len(), 2);
    assert_eq!(n.analyses.len(), 4);
    assert_eq!(n.options["RELTOL"], 1e-4);
    let fc = n.params.iter().find(|(k, _)| k == "FC").unwrap().1;
    assert!((fc - 1.0 / (2.0 * std::f64::consts::PI * 4700.0 * 100e-9)).abs() < 1e-6);
    match n.element("R1").unwrap() {
        Element::Resistor { value, .. } => assert_eq!(*value, 4700.0),
        e => panic!("{e:?}"),
    }
    assert!(matches!(
        n.analyses[2],
        Analysis::Ac {
            sweep: Sweep::Dec,
            points: 10,
            ..
        }
    ));
    assert!(matches!(
        n.analyses[3],
        Analysis::Tran {
            uic: true,
            tmax: Some(_),
            ..
        }
    ));
    // Continuation line joined.
    assert!(
        matches!(n.element("VP").unwrap(), Element::VSource { source, .. } if source.wave.is_some())
    );
    n.validate().unwrap();
    let flat = n.flatten().unwrap();
    assert!(flat.elements.iter().any(|e| e.name() == "R1.X1"));
}

#[test]
fn write_then_parse_is_identity() {
    let a = parse(FULL).unwrap().netlist;
    let text = write(&a).unwrap();
    let b = parse(&text)
        .unwrap_or_else(|e| panic!("{e}\n{text}"))
        .netlist;
    assert_eq!(a, b, "\n{text}");
    // Stable: writing again gives the same text.
    assert_eq!(text, write(&b).unwrap());
}

#[test]
fn flattened_netlist_round_trips_too() {
    let a = parse(FULL).unwrap().netlist.flatten().unwrap();
    let text = write(&a).unwrap();
    let b = parse(&text).unwrap().netlist;
    assert_eq!(a, b);
}

#[test]
fn case_insensitive_and_suffixes() {
    let p = parse("t\nr1 A b 2K2\nR2 b 0 4k7\nC1 B 0 10N\nv1 a 0 5\n.END\n").unwrap();
    let n = p.netlist;
    assert_eq!(n.nodes.len(), 3); // 0, A, b
    assert!(
        matches!(n.element("R1").unwrap(), Element::Resistor { value, .. } if *value == 2200.0)
    );
    assert!(
        matches!(n.element("r2").unwrap(), Element::Resistor { value, .. } if *value == 4700.0)
    );
}

#[test]
fn comments_and_continuations() {
    let p =
        parse("title\n* c\nR1 a 0 1k ; trailing\nR2 a 0 $ dollar comment\n+ 2k\n\n  \nV1 a 0 1\n")
            .unwrap();
    assert!(
        matches!(p.netlist.element("R2").unwrap(), Element::Resistor { value, .. } if *value == 2000.0)
    );
    let p = parse("t\nR1 a 0 1k\r\nV1 a 0 1\r\n.end\r\n").unwrap();
    assert_eq!(p.netlist.elements.len(), 2);
}

#[test]
fn ignored_commands_warn() {
    let p = parse("t\nV1 a 0 1\nR1 a 0 1k\n.print tran v(a)\n.plot ac vdb(a)\n.options foo\n.op\n")
        .unwrap();
    assert_eq!(p.warnings.len(), 3);
    assert_eq!(p.warnings[0].line, 4);
    assert!(p.warnings[0].message.contains(".print"));
}

fn fail(text: &str) -> String {
    parse(text).unwrap_err().to_string()
}

#[test]
fn errors_carry_line_numbers_and_say_what_to_do() {
    let e = fail("t\nV1 a 0 1\nR1 a 0 abc\n");
    assert!(
        e.starts_with("line 3:") && e.contains("R1") && e.contains("resistance"),
        "{e}"
    );
    let e = fail("t\nR1 a 0\n");
    assert!(e.starts_with("line 2:") && e.contains("too few"), "{e}");
    let e = fail("t\nD1 a 0 NOPE\n");
    assert!(
        e.starts_with("line 2:") && e.contains("model `NOPE`"),
        "{e}"
    );
    let e = fail("t\nR1 a 0 1k\nR1 b 0 1k\n");
    assert!(e.starts_with("line 3:") && e.contains("duplicate"), "{e}");
    let e = fail("t\nR1 a 0 {nope*2}\n");
    assert!(e.starts_with("line 2:") && e.contains("nope"), "{e}");
    let e = fail("t\nR1 a 0 {1+}\n");
    assert!(e.contains("line 2"), "{e}");
    let e = fail("t\nR1 a 0 {1+2\n");
    assert!(e.contains("no matching"), "{e}");
    let e = fail("t\nR1 a 0 1k\n.tran 1u\n");
    assert!(e.starts_with("line 3:") && e.contains(".tran"), "{e}");
    let e = fail("t\nR1 a 0 1k\n.dc V9 0 1 0.1\n");
    assert!(e.contains("V9"), "{e}");
    let e = fail("t\nZ1 a 0 1\n");
    assert!(e.contains("unsupported element"), "{e}");
    let e = fail("t\n.subckt A p\nR1 p 0 1\n");
    assert!(e.contains("never closed"), "{e}");
    let e = fail("t\n.ends\n");
    assert!(e.contains("without"), "{e}");
    let e = fail("t\n+ stray\n");
    assert!(e.contains("continuation"), "{e}");
    let e = fail("t\nF1 a 0 R1 2\nR1 a 0 1\n");
    assert!(e.contains("voltage source"), "{e}");
    let e = fail("t\nK1 L1 L2 0.5\n");
    assert!(e.contains("L1"), "{e}");
    let e = fail("t\n.model M NMOS(LEVEL=3)\n");
    assert!(e.contains("level 1"), "{e}");
    let e = fail("t\nX1 a b SUB\n");
    assert!(e.contains("SUB"), "{e}");
    let e = fail("t\n.subckt S a b\n.subckt T c\n");
    assert!(e.contains("nested"), "{e}");
    let e = fail("t\n.subckt S a b params: w=1\n");
    assert!(e.contains("parameters"), "{e}");
    let e = fail("t\nR1 a 0 0\n");
    assert!(e.contains("zero"), "{e}");
}

#[test]
fn include_is_rejected_and_never_reads_files() {
    for cmd in [
        ".include /etc/passwd",
        ".inc other.cir",
        ".lib models.lib typ",
        ".INCLUDE \"x.cir\"",
    ] {
        let e = fail(&format!("t\n{cmd}\nR1 a 0 1\n"));
        assert!(
            e.contains("not supported") && e.contains("never reads other files"),
            "{e}"
        );
        assert!(e.starts_with("line 2:"));
    }
}

#[test]
fn hostile_inputs_are_errors_not_panics() {
    assert!(parse("").is_ok() || parse("").is_err());
    assert!(parse("\0").is_err());
    let long = format!("t\nR1 a 0 {}\n", "9".repeat(70_000));
    assert!(parse(&long).is_err());
    let many = format!("t\n{}", "R1 a 0 1\n".repeat(10));
    assert!(parse(&many).is_err()); // duplicates
    let deep = format!("t\n.param x={}1{}\n", "(".repeat(5000), ")".repeat(5000));
    assert!(parse(&deep).is_err());
    let braces = format!("t\nR1 a 0 {}\n", "{".repeat(3000));
    assert!(parse(&braces).is_err());
    let huge_tran = parse("t\nR1 a 0 1\n.ac dec 100000 1 1e300\n");
    assert!(huge_tran.is_err());
    assert!(parse("t\n.ac dec 1.5 1 1k\n").is_err());
    assert!(parse("t\nV1 a 0 PWL(0 0 1)\n").is_err());
    // A million-point sweep is capped, not allocated.
    assert!(parse("t\nV1 a 0 1\n.dc V1 0 1e9 1e-9\n").is_err());
    // Unicode and odd bytes.
    let _ = parse("t\nR1 é 0 1k\n");
    let _ = parse("t\nR1 a 0 1µ\n");
}

#[test]
fn truncations_and_mutations_never_panic() {
    for len in 0..FULL.len() {
        if FULL.is_char_boundary(len) {
            let _ = parse(&FULL[..len]);
        }
    }
    let bytes = FULL.as_bytes();
    for i in (0..bytes.len()).step_by(3) {
        for repl in [
            b'(', b')', b'{', b'}', b'=', b'+', b'*', b'.', b'0', b'\\', b'e', b'-',
        ] {
            let mut m = bytes.to_vec();
            m[i] = repl;
            if let Ok(s) = String::from_utf8(m) {
                let _ = parse(&s);
            }
        }
    }
    // Every written result of a successful mutation reads back identically.
    let mut ok = 0;
    for i in (0..bytes.len()).step_by(11) {
        let mut m = bytes.to_vec();
        m[i] = b'1';
        if let Ok(s) = String::from_utf8(m) {
            if let Ok(p) = parse(&s) {
                let text = write(&p.netlist).unwrap();
                assert_eq!(parse(&text).unwrap().netlist, p.netlist);
                ok += 1;
            }
        }
    }
    assert!(ok > 0);
}

proptest::proptest! {
    #[test]
    fn arbitrary_text_never_panics(s in "\\PC{0,300}") {
        let _ = parse(&s);
    }

    #[test]
    fn arbitrary_cards_never_panic(
        letter in "[RCLVIDEGFHKSQMUXrclvidegfhksqmuxZ.]",
        rest in "[ a-zA-Z0-9=(){}+.*/^,;$-]{0,60}",
    ) {
        let _ = parse(&format!("t\n{letter}{rest}\n"));
    }

    #[test]
    fn simple_netlists_round_trip(
        r in proptest::collection::vec((1u32..9, 1u32..9, 1.0f64..1e6), 1..8),
        v in -100.0f64..100.0,
    ) {
        let mut text = String::from("generated\n");
        text.push_str(&format!("V1 n1 0 DC {v:e}\n"));
        for (i, (a, b, val)) in r.iter().enumerate() {
            text.push_str(&format!("R{} n{a} n{b} {val:e}\n", i + 1));
        }
        text.push_str(".op\n");
        if let Ok(p) = parse(&text) {
            let out = write(&p.netlist).unwrap();
            let q = parse(&out).unwrap();
            proptest::prop_assert_eq!(p.netlist, q.netlist);
        }
    }
}

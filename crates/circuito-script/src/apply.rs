// SPDX-License-Identifier: MIT OR Apache-2.0
//! Going between a schematic and its text: [`design_of`] reads a drawing as a design, and
//! [`apply`] changes a drawing to match a design while keeping what the person drew.

use crate::build::{build, BuildError};
use crate::design::{
    is_anonymous, is_ground, kind_of, net_key, Design, NetKind, PartSpec, PinNet, Place,
};
use crate::print::token_name;
use circuito_core::library::Library;
use circuito_core::schematic::{resolve_nets, Label, LabelScope, PinRef, PowerStyle, Schematic};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Read the first sheet of a schematic as a design: its parts, the net each pin is on and its
/// models and analyses. Nets without a name in the drawing get `n1`, `n2`… (see
/// [`is_anonymous`]); ground nets are `gnd`.
#[must_use]
pub fn design_of(s: &Schematic) -> Design {
    let lib = Library::builtin();
    let mut d = Design {
        title: s.title.clone(),
        models: s.models.clone(),
        analyses: s.analyses.clone(),
        ..Design::default()
    };
    let Some(sheet) = s.sheets.first() else {
        return d;
    };
    let res = resolve_nets(s, lib).ok();
    let mut taken: HashSet<String> = HashSet::new();
    let mut names: BTreeMap<usize, String> = BTreeMap::new();
    if let Some(res) = &res {
        for (idx, net) in res.nets.iter().enumerate() {
            let mut found: Vec<&str> = Vec::new();
            for l in &net.labels {
                if let Some(label) = s.sheets.get(l.sheet).and_then(|sh| sh.labels.get(l.index)) {
                    found.push(&label.name);
                }
            }
            for p in &net.power_ports {
                if let Some(port) = s
                    .sheets
                    .get(p.sheet)
                    .and_then(|sh| sh.power_ports.get(p.index))
                {
                    found.push(&port.name);
                }
            }
            let name = if found.iter().any(|n| is_ground(n)) {
                Some("gnd".to_string())
            } else {
                found.first().map(|n| token_name(n))
            };
            if let Some(n) = name {
                taken.insert(net_key(&n));
                names.insert(idx, n);
            }
        }
    }
    let mut next = 1usize;
    for (pi, part) in sheet.parts.iter().enumerate() {
        let Some(def) = lib.get(&part.lib_id) else {
            continue;
        };
        let mut pins = Vec::new();
        for k in 0..def.pins.len() {
            let pin = PinRef {
                sheet: 0,
                part: pi,
                pin: k,
            };
            let pn = match &res {
                None => PinNet::Open,
                Some(res) => {
                    let idx = res.pin_net.get(&pin).copied();
                    let size = idx
                        .and_then(|i| res.nets.get(i))
                        .map_or(0, |n| n.pins.len());
                    if res.no_connect_pins.contains(&pin) {
                        PinNet::NoConnect
                    } else if let Some(i) = idx.filter(|i| names.contains_key(i) || size >= 2) {
                        let name = names
                            .entry(i)
                            .or_insert_with(|| loop {
                                let c = format!("n{next}");
                                next += 1;
                                if taken.insert(net_key(&c)) {
                                    break c;
                                }
                            })
                            .clone();
                        PinNet::Net(name)
                    } else {
                        PinNet::Open
                    }
                }
            };
            pins.push(pn);
        }
        d.parts.push(PartSpec {
            refdes: part.refdes.clone(),
            lib_id: part.lib_id.clone(),
            value: part.value.clone(),
            model: part.model.clone(),
            fields: part.fields.clone(),
            pins,
            place: Some(Place {
                position: part.position,
                rotation: part.rotation,
                mirror: part.mirror,
            }),
            line: 0,
        });
    }
    d
}

/// What two designs must share for a drawing to be changed in place: the same parts and the same
/// groups of pins joined together. Net names, values and everything else may differ.
fn structure(d: &Design) -> (BTreeSet<(String, String)>, BTreeSet<BTreeSet<String>>) {
    let parts = d
        .parts
        .iter()
        .map(|p| (p.refdes.to_lowercase(), p.lib_id.clone()))
        .collect();
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut singles: BTreeSet<BTreeSet<String>> = BTreeSet::new();
    for p in &d.parts {
        for (k, pin) in p.pins.iter().enumerate() {
            let id = format!("{}#{k}", p.refdes.to_lowercase());
            match pin {
                PinNet::Net(n) => {
                    groups.entry(net_key(n)).or_default().insert(id);
                }
                PinNet::NoConnect => {
                    singles.insert(BTreeSet::from([format!("nc:{id}")]));
                }
                PinNet::Open => {
                    singles.insert(BTreeSet::from([format!("open:{id}")]));
                }
            }
        }
    }
    let mut all: BTreeSet<BTreeSet<String>> = BTreeSet::new();
    for g in groups.into_values() {
        if g.len() == 1 {
            // A net with one pin is the same as an open pin: it joins nothing.
            all.extend(
                g.into_iter()
                    .map(|id| BTreeSet::from([format!("open:{id}")])),
            );
        } else {
            all.insert(g);
        }
    }
    all.extend(singles);
    (parts, all)
}

/// Change the drawing in place when the design only renames nets or edits values, models, fields,
/// the title or the analyses. `None` when the connections differ.
fn patch(current: &Schematic, design: &Design) -> Option<Schematic> {
    let lib = Library::builtin();
    let old = design_of(current);
    if structure(&old) != structure(design) {
        return None;
    }
    // Which new name each old net takes.
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    for spec in &design.parts {
        let before = old
            .part_index(&spec.refdes)
            .and_then(|i| old.parts.get(i))?;
        for (a, b) in before.pins.iter().zip(&spec.pins) {
            if let (PinNet::Net(from), PinNet::Net(to)) = (a, b) {
                let key = net_key(from);
                if key != net_key(to) {
                    if kind_of(from) != kind_of(to) {
                        return None;
                    }
                    renames.entry(key).or_insert_with(|| to.clone());
                }
            }
        }
    }
    let mut out = current.clone();
    out.title.clone_from(&design.title);
    out.models.clone_from(&design.models);
    out.analyses.clone_from(&design.analyses);
    let res = resolve_nets(current, lib).ok()?;
    let sh = out.sheets.first_mut()?;
    for part in &mut sh.parts {
        if let Some(spec) = design
            .part_index(&part.refdes)
            .and_then(|i| design.parts.get(i))
        {
            part.value.clone_from(&spec.value);
            part.model.clone_from(&spec.model);
            part.fields.clone_from(&spec.fields);
        }
    }
    let rename_of = |name: &str| renames.get(&net_key(&token_name(name))).cloned();
    for l in &mut sh.labels {
        if let Some(n) = rename_of(&l.name) {
            l.name = n;
        }
    }
    for p in &mut sh.power_ports {
        if let Some(n) = rename_of(&p.name) {
            p.style = if is_ground(&n) {
                PowerStyle::Ground
            } else {
                PowerStyle::Supply
            };
            p.name = n;
        }
    }
    // A net that was anonymous and now has a name needs a label to carry it.
    for (key, to) in &renames {
        if is_anonymous(to) || kind_of(to) != NetKind::Signal {
            continue;
        }
        let net = res.nets.iter().find(|n| {
            n.pins.first().is_some_and(|pr| {
                old.parts
                    .get(pr.part)
                    .and_then(|p| p.pins.get(pr.pin))
                    .is_some_and(|pn| matches!(pn, PinNet::Net(n) if net_key(n) == *key))
            })
        })?;
        if net.labels.is_empty() && net.power_ports.is_empty() {
            let at = *res.pin_points.get(net.pins.first()?)?;
            sh.labels.push(Label {
                name: to.clone(),
                position: at,
                scope: LabelScope::Local,
            });
        }
    }
    Some(out)
}

/// Change `current` to match `design`. Edits that keep the connections (values, models, renamed
/// nets, analyses) change the drawing in place, wires and all. Edits that change the connections
/// redraw it: parts keep their positions and rotations and new parts are placed beside them,
/// wires are routed again.
///
/// # Errors
/// When the design cannot be drawn.
pub fn apply(current: &Schematic, design: &Design) -> Result<Schematic, BuildError> {
    if let Some(s) = patch(current, design) {
        return Ok(s);
    }
    build(design, Some(current))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use circuito_core::schematic::Point;

    fn design(text: &str) -> Design {
        let p = parse(text);
        assert!(p.is_clean(), "{:?}", p.problems);
        p.design
    }

    fn partition(d: &Design) -> BTreeSet<BTreeSet<String>> {
        structure(d).1
    }

    #[test]
    fn a_drawing_reads_back_as_the_same_circuit() {
        for text in [
            "V1 5 in gnd\nR1 1k in out\nC1 100n out gnd\n",
            "V2 12 vcc gnd\nQ1 c=c b=b e=gnd\nRB 100k vcc b\nRC 2.2k vcc c\nD1 c _\n",
            "R1 1k a b\nR2 1k b c\nR3 1k c a\n",
        ] {
            let d = design(text);
            let s = build(&d, None).expect("drawable");
            let back = design_of(&s);
            assert_eq!(partition(&d), partition(&back), "{text}");
            let again = build(&back, None).expect("drawable again");
            assert_eq!(partition(&design_of(&again)), partition(&d), "{text}");
        }
    }

    #[test]
    fn names_survive_the_round_trip() {
        let d = design("V1 5 in gnd\nR1 1k in out\nC1 100n out gnd\n");
        let back = design_of(&build(&d, None).expect("drawable"));
        let names: BTreeSet<String> = back.nets().into_iter().map(|n| n.name).collect();
        assert_eq!(
            names,
            BTreeSet::from(["in".to_string(), "out".to_string(), "0".to_string()])
        );
    }

    #[test]
    fn unnamed_nets_get_anonymous_names() {
        let mut s = build(&design("R1 1k a b\nR2 1k b c\n"), None).expect("drawable");
        s.sheets[0].labels.clear();
        let back = design_of(&s);
        assert!(
            back.nets().iter().all(|n| is_anonymous(&n.name)),
            "{:?}",
            back.nets()
        );
    }

    #[test]
    fn value_edits_keep_the_drawing_exactly() {
        let s = build(
            &design("V1 5 in gnd\nR1 1k in out\nC1 100n out gnd\n"),
            None,
        )
        .expect("drawable");
        let mut moved = s.clone();
        moved.sheets[0]
            .no_connects
            .push(circuito_core::schematic::NoConnect {
                position: Point::new(99, 40),
            });
        let d = design("V1 9 in gnd\nR1 4.7k in out\nC1 100n out gnd\nop\n");
        let t = apply(&moved, &d).expect("applies");
        assert_eq!(t.sheets[0].wires, moved.sheets[0].wires);
        assert_eq!(t.sheets[0].no_connects, moved.sheets[0].no_connects);
        assert_eq!(t.sheets[0].parts[1].value, "4.7k");
        assert_eq!(t.sheets[0].parts[0].value, "9");
        assert_eq!(t.analyses.len(), 1);
    }

    #[test]
    fn renaming_a_net_renames_its_labels() {
        let s = build(
            &design("V1 5 in gnd\nR1 1k in out\nC1 100n out gnd\n"),
            None,
        )
        .expect("drawable");
        let t = apply(
            &s,
            &design("V1 5 in gnd\nR1 1k in vout\nC1 100n vout gnd\n"),
        )
        .expect("applies");
        assert_eq!(t.sheets[0].wires, s.sheets[0].wires);
        assert!(t.sheets[0].labels.iter().any(|l| l.name == "vout"));
        assert!(!t.sheets[0].labels.iter().any(|l| l.name == "out"));
    }

    #[test]
    fn new_connections_redraw_but_parts_stay_put() {
        let s = build(&design("V1 5 in gnd\nR1 1k in out\n"), None).expect("drawable");
        let t = apply(&s, &design("V1 5 in gnd\nR1 1k in out\nC1 1u out gnd\n")).expect("applies");
        assert_eq!(t.sheets[0].parts.len(), 3);
        assert_eq!(t.sheets[0].parts[0].position, s.sheets[0].parts[0].position);
        assert_eq!(t.sheets[0].parts[1].position, s.sheets[0].parts[1].position);
        assert_eq!(
            partition(&design_of(&t)),
            partition(&design("V1 5 in gnd\nR1 1k in out\nC1 1u out gnd\n"))
        );
    }

    #[test]
    fn a_name_that_becomes_ground_redraws() {
        let s = build(&design("V1 5 in gnd\nR1 1k in out\nR2 1k out x\n"), None).expect("drawable");
        let d = design("V1 5 in gnd\nR1 1k in out\nR2 1k out gnd\n");
        let t = apply(&s, &d).expect("applies");
        assert_eq!(partition(&design_of(&t)), partition(&d));
    }
}

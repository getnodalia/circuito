// SPDX-License-Identifier: MIT OR Apache-2.0
//! Schematic to netlist extraction.
//!
//! Net names are chosen deterministically: a user label wins over a power port, which wins over an
//! automatic `N$001`-style name; the ground net is always `0`. Two distinct nets that would get the
//! same name (a local label reused on two sheets) are told apart by appending `@<sheet name>`.
//! A part that cannot be converted (missing value, unknown model, bad source text) is skipped and
//! reported as an [`ExtractIssue`]; the rest of the schematic still extracts.

use circuito_core::library::{Device, Library, PartDef};
use circuito_core::netlist::{Element, Model, Netlist, NodeId};
use circuito_core::schematic::{resolve_nets, NetResolution, PinRef, Schematic, SchematicError};
use circuito_core::source::Source;
use circuito_core::value::parse_value;
use std::collections::BTreeSet;

/// Why a part did not become a netlist element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueKind {
    /// A part that needs a value has none.
    MissingValue,
    /// A value, field or source description could not be read.
    InvalidValue,
    /// The model is neither defined in the schematic nor a built-in default.
    UnknownModel,
    /// The element could not be added (for example a duplicate reference designator).
    Rejected,
}

/// A part that could not be extracted.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractIssue {
    /// What kind of problem.
    pub kind: IssueKind,
    /// Sheet index.
    pub sheet: usize,
    /// Part index on the sheet.
    pub part: usize,
    /// Reference designator.
    pub refdes: String,
    /// A message a person can act on.
    pub message: String,
}

/// The result of extraction.
#[derive(Debug, Clone)]
pub struct Extraction {
    /// The netlist, with models and analyses carried over from the schematic.
    pub netlist: Netlist,
    /// The resolved connectivity the netlist was built from.
    pub resolution: NetResolution,
    /// The name given to each net (same indices as `resolution.nets`).
    pub net_names: Vec<String>,
    /// Parts that were skipped, with the reason.
    pub issues: Vec<ExtractIssue>,
}

/// Name every net. Returns the names and, per net, whether it is ground.
pub(crate) fn name_nets(s: &Schematic, res: &NetResolution) -> Vec<String> {
    let mut names: Vec<Option<String>> = vec![None; res.nets.len()];
    for (i, n) in res.nets.iter().enumerate() {
        let is_gnd = |name: &str| matches!(name.trim().to_lowercase().as_str(), "0" | "gnd");
        let mut label_names = n.labels.iter().filter_map(|l| {
            s.sheets
                .get(l.sheet)?
                .labels
                .get(l.index)
                .map(|x| x.name.trim().to_string())
        });
        let mut port_names = n.power_ports.iter().filter_map(|p| {
            s.sheets
                .get(p.sheet)?
                .power_ports
                .get(p.index)
                .map(|x| x.name.trim().to_string())
        });
        let all: Vec<String> = label_names.clone().chain(port_names.clone()).collect();
        if all.iter().any(|x| is_gnd(x)) {
            names[i] = Some("0".to_string());
        } else if let Some(l) = label_names.find(|x| !x.is_empty()) {
            names[i] = Some(l);
        } else if let Some(p) = port_names.find(|x| !x.is_empty()) {
            names[i] = Some(p);
        }
    }
    // Disambiguate duplicates (distinct nets with the same name), keeping ground merged.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (i, n) in res.nets.iter().enumerate() {
        if let Some(name) = names[i].clone() {
            let key = name.to_lowercase();
            if key != "0" && !seen.insert(key.clone()) {
                let sheet = n
                    .labels
                    .first()
                    .map(|l| l.sheet)
                    .or_else(|| n.power_ports.first().map(|p| p.sheet))
                    .and_then(|si| s.sheets.get(si))
                    .map_or("sheet", |sh| sh.name.as_str());
                let mut candidate = format!("{name}@{sheet}");
                let mut k = 2;
                while !seen.insert(candidate.to_lowercase()) {
                    candidate = format!("{name}@{sheet}#{k}");
                    k += 1;
                }
                names[i] = Some(candidate);
            }
        }
    }
    let mut counter = 0u32;
    names
        .into_iter()
        .map(|n| {
            n.unwrap_or_else(|| loop {
                counter += 1;
                let cand = format!("N${counter:03}");
                if seen.insert(cand.to_lowercase()) {
                    break cand;
                }
            })
        })
        .collect()
}

fn element_name(letter: char, refdes: &str, suffix: &str) -> String {
    let base = if refdes.to_uppercase().starts_with(letter) {
        refdes.to_string()
    } else {
        format!("{letter}{refdes}")
    };
    if suffix.is_empty() {
        base
    } else {
        format!("{base}_{suffix}")
    }
}

struct Ctx<'a> {
    res: &'a NetResolution,
    names: &'a [String],
    nl: Netlist,
    issues: Vec<ExtractIssue>,
}

impl Ctx<'_> {
    fn node(&mut self, sheet: usize, part: usize, pin: usize) -> Result<NodeId, String> {
        let net = self
            .res
            .pin_net
            .get(&PinRef { sheet, part, pin })
            .ok_or_else(|| "pin has no net".to_string())?;
        let name = self
            .names
            .get(*net)
            .ok_or_else(|| "net has no name".to_string())?
            .clone();
        self.nl.node(&name).map_err(|e| e.to_string())
    }
}

/// Extract the netlist of `s`.
///
/// # Errors
/// A schematic that cannot be resolved at all (unknown library symbol, limits exceeded).
pub fn extract(s: &Schematic) -> Result<Extraction, SchematicError> {
    let lib = Library::builtin();
    let res = resolve_nets(s, lib)?;
    let names = name_nets(s, &res);
    let mut cx = Ctx {
        res: &res,
        names: &names,
        nl: Netlist::new(&s.title),
        issues: Vec::new(),
    };
    for m in &s.models {
        // Duplicate model names in a schematic: the later one is reported by the rules check, the
        // first wins here.
        let _ = cx.nl.add_model(m.clone());
    }
    for (si, sh) in s.sheets.iter().enumerate() {
        for (pi, part) in sh.parts.iter().enumerate() {
            let Some(def) = lib.get(&part.lib_id) else {
                continue;
            };
            let nodes: Result<Vec<NodeId>, String> =
                (0..def.pins.len()).map(|k| cx.node(si, pi, k)).collect();
            let issue = |kind, message: String| ExtractIssue {
                kind,
                sheet: si,
                part: pi,
                refdes: part.refdes.clone(),
                message,
            };
            let nodes = match nodes {
                Ok(n) => n,
                Err(e) => {
                    cx.issues
                        .push(issue(IssueKind::Rejected, format!("{}: {e}", part.refdes)));
                    continue;
                }
            };
            match convert(&mut cx, def, part, &nodes) {
                Ok(()) => {}
                Err((kind, msg)) => cx.issues.push(issue(kind, msg)),
            }
        }
    }
    cx.nl.analyses = s.analyses.clone();
    let Ctx { nl, issues, .. } = cx;
    Ok(Extraction {
        netlist: nl,
        resolution: res,
        net_names: names,
        issues,
    })
}

type Conv = Result<(), (IssueKind, String)>;

fn value_of(part: &circuito_core::schematic::Part, what: &str) -> Result<f64, (IssueKind, String)> {
    if part.value.trim().is_empty() {
        return Err((
            IssueKind::MissingValue,
            format!("{} has no value: set the {what}", part.refdes),
        ));
    }
    parse_value(&part.value).map_err(|e| {
        (
            IssueKind::InvalidValue,
            format!("{}: value `{}`: {e}", part.refdes, part.value),
        )
    })
}

fn field_value(
    part: &circuito_core::schematic::Part,
    key: &str,
) -> Result<Option<f64>, (IssueKind, String)> {
    match part.fields.get(key) {
        None => Ok(None),
        Some(t) => parse_value(t).map(Some).map_err(|e| {
            (
                IssueKind::InvalidValue,
                format!("{}: field {key} `{t}`: {e}", part.refdes),
            )
        }),
    }
}

fn add(cx: &mut Ctx<'_>, refdes: &str, e: Element) -> Conv {
    cx.nl
        .add(e)
        .map_err(|err| (IssueKind::Rejected, format!("{refdes}: {err}")))
}

fn ensure_model(
    cx: &mut Ctx<'_>,
    part: &circuito_core::schematic::Part,
    def: &PartDef,
) -> Result<String, (IssueKind, String)> {
    let name = part
        .model
        .clone()
        .or_else(|| def.default_model.map(str::to_string))
        .ok_or_else(|| {
            (
                IssueKind::UnknownModel,
                format!("{} needs a model name", part.refdes),
            )
        })?;
    let key = name.to_uppercase();
    if !cx.nl.models.contains_key(&key) {
        match Library::default_model(&key) {
            Some(m) => {
                let _ = cx.nl.add_model(m);
            }
            None => {
                return Err((
                    IssueKind::UnknownModel,
                    format!("{}: model `{name}` is not defined; add a model to the schematic or use a built-in one (D, LED, DZ, NPN, PNP, NMOS, PMOS, SW, OPAMP)", part.refdes),
                ))
            }
        }
    }
    Ok(key)
}

fn convert(
    cx: &mut Ctx<'_>,
    def: &PartDef,
    part: &circuito_core::schematic::Part,
    n: &[NodeId],
) -> Conv {
    let r = part.refdes.as_str();
    let node = |i: usize| n.get(i).copied().unwrap_or(NodeId::GROUND);
    match def.device {
        Device::Resistor => {
            let value = value_of(part, "resistance")?;
            add(
                cx,
                r,
                Element::Resistor {
                    name: element_name('R', r, ""),
                    a: node(0),
                    b: node(1),
                    value,
                },
            )
        }
        Device::Fuse => {
            let value = field_value(part, "rseries")?.unwrap_or(0.01);
            add(
                cx,
                r,
                Element::Resistor {
                    name: element_name('R', r, ""),
                    a: node(0),
                    b: node(1),
                    value,
                },
            )
        }
        Device::Capacitor => {
            let value = value_of(part, "capacitance")?;
            let ic = field_value(part, "ic")?;
            add(
                cx,
                r,
                Element::Capacitor {
                    name: element_name('C', r, ""),
                    a: node(0),
                    b: node(1),
                    value,
                    ic,
                },
            )
        }
        Device::Inductor => {
            let value = value_of(part, "inductance")?;
            let ic = field_value(part, "ic")?;
            add(
                cx,
                r,
                Element::Inductor {
                    name: element_name('L', r, ""),
                    a: node(0),
                    b: node(1),
                    value,
                    ic,
                },
            )
        }
        Device::VSource | Device::ISource => {
            if part.value.trim().is_empty() {
                return Err((
                    IssueKind::MissingValue,
                    format!("{r} has no value: give a source such as 5, AC 1 or SIN(0 1 1k)"),
                ));
            }
            let source = Source::parse(&part.value).map_err(|e| {
                (
                    IssueKind::InvalidValue,
                    format!("{r}: source `{}`: {e}", part.value),
                )
            })?;
            if def.device == Device::VSource {
                add(
                    cx,
                    r,
                    Element::VSource {
                        name: element_name('V', r, ""),
                        p: node(0),
                        n: node(1),
                        source,
                    },
                )
            } else {
                add(
                    cx,
                    r,
                    Element::ISource {
                        name: element_name('I', r, ""),
                        p: node(0),
                        n: node(1),
                        source,
                    },
                )
            }
        }
        Device::Diode => {
            let model = ensure_model(cx, part, def)?;
            let area = field_value(part, "area")?.unwrap_or(1.0);
            add(
                cx,
                r,
                Element::Diode {
                    name: element_name('D', r, ""),
                    anode: node(0),
                    cathode: node(1),
                    model,
                    area,
                },
            )
        }
        Device::Npn | Device::Pnp => {
            let model = ensure_model(cx, part, def)?;
            let area = field_value(part, "area")?.unwrap_or(1.0);
            add(
                cx,
                r,
                Element::Bjt {
                    name: element_name('Q', r, ""),
                    c: node(0),
                    b: node(1),
                    e: node(2),
                    model,
                    area,
                },
            )
        }
        Device::Nmos | Device::Pmos => {
            let model = ensure_model(cx, part, def)?;
            let w = field_value(part, "w")?;
            let l = field_value(part, "l")?;
            add(
                cx,
                r,
                Element::Mosfet {
                    name: element_name('M', r, ""),
                    d: node(0),
                    g: node(1),
                    s: node(2),
                    bulk: node(2),
                    model,
                    w,
                    l,
                },
            )
        }
        Device::OpAmp => {
            let model = ensure_model(cx, part, def)?;
            // Pin order in the symbol: +, -, OUT.
            add(
                cx,
                r,
                Element::OpAmp {
                    name: element_name('U', r, ""),
                    inp: node(0),
                    inn: node(1),
                    out: node(2),
                    model,
                },
            )
        }
        Device::Switch => {
            let model = ensure_model(cx, part, def)?;
            add(
                cx,
                r,
                Element::Switch {
                    name: element_name('S', r, ""),
                    p: node(0),
                    n: node(1),
                    cp: node(2),
                    cn: node(3),
                    model,
                },
            )
        }
        Device::Potentiometer => {
            let total = value_of(part, "total resistance")?;
            let w = field_value(part, "wiper")?.unwrap_or(0.5);
            if !(0.0..=1.0).contains(&w) {
                return Err((
                    IssueKind::InvalidValue,
                    format!("{r}: wiper must be between 0 and 1 (it is {w})"),
                ));
            }
            // Pins: A, B, W. The wiper is clamped away from the ends so neither half is 0 ohm.
            let w = w.clamp(1e-3, 1.0 - 1e-3);
            add(
                cx,
                r,
                Element::Resistor {
                    name: element_name('R', r, "A"),
                    a: node(0),
                    b: node(2),
                    value: total * w,
                },
            )?;
            add(
                cx,
                r,
                Element::Resistor {
                    name: element_name('R', r, "B"),
                    a: node(2),
                    b: node(1),
                    value: total * (1.0 - w),
                },
            )
        }
        Device::Transformer => {
            let lp = value_of(part, "primary inductance")?;
            let ratio = field_value(part, "ratio")?.unwrap_or(1.0);
            let k = field_value(part, "k")?.unwrap_or(0.999);
            if !(ratio.is_finite() && ratio > 0.0) {
                return Err((
                    IssueKind::InvalidValue,
                    format!("{r}: ratio must be positive"),
                ));
            }
            let (lp_name, ls_name) = (element_name('L', r, "P"), element_name('L', r, "S"));
            add(
                cx,
                r,
                Element::Inductor {
                    name: lp_name.clone(),
                    a: node(0),
                    b: node(1),
                    value: lp,
                    ic: None,
                },
            )?;
            add(
                cx,
                r,
                Element::Inductor {
                    name: ls_name.clone(),
                    a: node(2),
                    b: node(3),
                    value: lp / (ratio * ratio),
                    ic: None,
                },
            )?;
            add(
                cx,
                r,
                Element::Mutual {
                    name: element_name('K', r, ""),
                    l1: lp_name,
                    l2: ls_name,
                    k,
                },
            )
        }
    }
}

/// The model cards a netlist needs, as a convenience for callers that only have a schematic.
#[must_use]
pub fn default_models() -> Vec<Model> {
    [
        "D", "LED", "DZ", "NPN", "PNP", "NMOS", "PMOS", "SW", "OPAMP",
    ]
    .iter()
    .filter_map(|n| Library::default_model(n))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use circuito_core::schematic::builder::Builder;

    fn divider() -> Schematic {
        let mut b = Builder::new("divider");
        let v = b.place("V", "V1", "10", 0, 0);
        let r1 = b.place("R", "R1", "1k", 20, 0);
        let r2 = b.place("R", "R2", "2.2k", 40, 0);
        b.power_pin(v, "2", "GND");
        b.power_pin(r2, "2", "0");
        b.label_pin(v, "1", "IN");
        b.label_pin(r1, "1", "IN");
        b.label_pin(r1, "2", "OUT");
        b.label_pin(r2, "1", "OUT");
        b.build()
    }

    #[test]
    fn extracts_a_divider_with_label_names() {
        let x = extract(&divider()).unwrap();
        assert!(x.issues.is_empty(), "{:?}", x.issues);
        let text = x.netlist.to_string();
        assert!(text.contains("V1 IN 0 DC 10"), "{text}");
        assert!(text.contains("R1 IN OUT 1k"), "{text}");
        assert!(text.contains("R2 OUT 0 2.2k"), "{text}");
        x.netlist.validate().unwrap();
    }

    #[test]
    fn extraction_is_deterministic() {
        let a = extract(&divider()).unwrap().netlist.to_string();
        let b = extract(&divider()).unwrap().netlist.to_string();
        assert_eq!(a, b);
    }

    #[test]
    fn unlabelled_nets_get_auto_names() {
        let mut b = Builder::new("t");
        let r1 = b.place("R", "R1", "1k", 0, 0);
        let r2 = b.place("R", "R2", "1k", 20, 0);
        b.connect((r1, "2"), (r2, "1"));
        let x = extract(&b.build()).unwrap();
        let text = x.netlist.to_string();
        assert!(
            text.contains("N$001") && text.contains("N$002") && text.contains("N$003"),
            "{text}"
        );
        assert!(x.net_names.iter().all(|n| n.starts_with("N$")));
    }

    #[test]
    fn label_beats_power_port_beats_auto() {
        let mut b = Builder::new("t");
        let r1 = b.place("R", "R1", "1k", 0, 0);
        b.power_pin(r1, "1", "VCC");
        b.label_pin(r1, "1", "RAIL");
        b.power_pin(r1, "2", "5V");
        let x = extract(&b.build()).unwrap();
        assert!(
            x.netlist.to_string().contains("R1 RAIL 5V 1k"),
            "{}",
            x.netlist
        );
    }

    #[test]
    fn parts_with_problems_are_reported_not_fatal() {
        let mut b = Builder::new("t");
        b.place("R", "R1", "", 0, 0);
        b.place("R", "R2", "abc", 20, 0);
        b.place("C", "C1", "1u", 40, 0);
        let q = b.place("NPN", "Q1", "", 60, 0);
        b.model(q, "NOSUCH");
        b.place("V", "V1", "SIN(1)", 80, 0);
        b.place("R", "R3", "1k", 100, 0);
        b.place("R", "R3", "2k", 120, 0);
        let x = extract(&b.build()).unwrap();
        let kinds: Vec<_> = x.issues.iter().map(|i| i.kind).collect();
        assert_eq!(
            kinds,
            vec![
                IssueKind::MissingValue,
                IssueKind::InvalidValue,
                IssueKind::UnknownModel,
                IssueKind::InvalidValue,
                IssueKind::Rejected
            ]
        );
        assert_eq!(x.netlist.elements.len(), 2);
    }

    #[test]
    fn composite_parts_expand() {
        let mut b = Builder::new("t");
        let pot = b.place("POT", "RV1", "10k", 0, 0);
        b.field(pot, "wiper", "0.25");
        let t = b.place("TRANSFORMER", "T1", "10m", 40, 0);
        b.field(t, "ratio", "2");
        let d = b.place("LED", "D1", "", 80, 0);
        let _ = d;
        let x = extract(&b.build()).unwrap();
        assert!(x.issues.is_empty(), "{:?}", x.issues);
        let names: Vec<&str> = x.netlist.elements.iter().map(Element::name).collect();
        assert_eq!(names, ["RV1_A", "RV1_B", "LT1_P", "LT1_S", "KT1", "D1"]);
        match &x.netlist.elements[0] {
            Element::Resistor { value, .. } => assert!((value - 2500.0).abs() < 1e-9),
            e => panic!("{e:?}"),
        }
        match &x.netlist.elements[3] {
            Element::Inductor { value, .. } => assert!((value - 2.5e-3).abs() < 1e-12),
            e => panic!("{e:?}"),
        }
        assert!(x.netlist.models.contains_key("LED"));
        x.netlist.validate().unwrap();
    }

    #[test]
    fn same_local_label_on_two_sheets_stays_two_nets() {
        use circuito_core::schematic::{Label, LabelScope, Point, Sheet, Wire};
        let mut s = Schematic::new("t");
        let wire = |y| Wire {
            points: vec![Point::new(0, y), Point::new(5, y)],
        };
        let lab = |y| Label {
            name: "OUT".into(),
            position: Point::new(0, y),
            scope: LabelScope::Local,
        };
        s.sheets = vec![
            Sheet {
                name: "A".into(),
                wires: vec![wire(0)],
                labels: vec![lab(0)],
                ..Sheet::default()
            },
            Sheet {
                name: "B".into(),
                wires: vec![wire(0)],
                labels: vec![lab(0)],
                ..Sheet::default()
            },
        ];
        let x = extract(&s).unwrap();
        assert_eq!(x.net_names, vec!["OUT".to_string(), "OUT@B".to_string()]);
    }
}

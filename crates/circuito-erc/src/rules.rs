// SPDX-License-Identifier: MIT OR Apache-2.0
//! The rules. See [`Rule`] for the list and their stable codes.

use crate::extract::{extract, name_nets, IssueKind};
use crate::matrix::{conflict, Verdict};
use circuito_core::library::{Device, Library};
use circuito_core::netlist::{Element, Netlist, NodeId};
use circuito_core::schematic::{
    resolve_nets, NetResolution, NetWarning, PinRef, PinType, Point, Schematic,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// How serious a violation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Informational.
    Info,
    /// Probably a mistake.
    Warning,
    /// A mistake: the circuit cannot work or cannot be simulated.
    Error,
}

/// Every rule, with a stable code that never changes meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub enum Rule {
    /// E001: a pin that is not marked no-connect is connected to nothing.
    PinUnconnected,
    /// E002: a net with exactly one pin.
    NetSinglePin,
    /// E003: a pin marked no-connect is connected.
    NoConnectConnected,
    /// E010: two outputs drive one net.
    OutputConflict,
    /// E011: two power outputs on one net.
    PowerOutputConflict,
    /// E012: other pin-type conflict from the matrix.
    PinTypeConflict,
    /// E020: a power-input pin on a net no power output or voltage source drives.
    PowerInputUndriven,
    /// E021: the circuit has no ground reference.
    NoGround,
    /// E030: two parts share a reference designator.
    DuplicateRefdes,
    /// E031: a part has no value.
    MissingValue,
    /// E032: a value, field or source description is invalid.
    InvalidValue,
    /// E033: a part refers to an unknown model.
    UnknownModel,
    /// E034: a part uses a symbol that is not in the library.
    UnknownSymbol,
    /// E035: a part could not be added to the netlist.
    PartRejected,
    /// E040: an input pin on a net nothing drives.
    FloatingInput,
    /// E050: ideal voltage sources in parallel or in a loop.
    VoltageSourceLoop,
    /// E051: current sources in series, or a node a current source cannot return from.
    CurrentSourceCutset,
    /// E052: a part of the circuit has no DC path to ground.
    NoDcPath,
    /// E060: a net carries two different names.
    NetNameConflict,
    /// E070: a wire end or pin touches a wire without a junction.
    TouchWithoutJunction,
    /// E071: a label touches nothing.
    FloatingLabel,
    /// E072: a label sits on several wires.
    AmbiguousLabel,
    /// E073: a power port touches nothing.
    FloatingPowerPort,
    /// E074: a junction with nothing to join.
    DanglingJunction,
    /// E075: a no-connect flag not on a pin.
    DanglingNoConnect,
}

impl Rule {
    /// The stable code, e.g. `E010`.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Rule::PinUnconnected => "E001",
            Rule::NetSinglePin => "E002",
            Rule::NoConnectConnected => "E003",
            Rule::OutputConflict => "E010",
            Rule::PowerOutputConflict => "E011",
            Rule::PinTypeConflict => "E012",
            Rule::PowerInputUndriven => "E020",
            Rule::NoGround => "E021",
            Rule::DuplicateRefdes => "E030",
            Rule::MissingValue => "E031",
            Rule::InvalidValue => "E032",
            Rule::UnknownModel => "E033",
            Rule::UnknownSymbol => "E034",
            Rule::PartRejected => "E035",
            Rule::FloatingInput => "E040",
            Rule::VoltageSourceLoop => "E050",
            Rule::CurrentSourceCutset => "E051",
            Rule::NoDcPath => "E052",
            Rule::NetNameConflict => "E060",
            Rule::TouchWithoutJunction => "E070",
            Rule::FloatingLabel => "E071",
            Rule::AmbiguousLabel => "E072",
            Rule::FloatingPowerPort => "E073",
            Rule::DanglingJunction => "E074",
            Rule::DanglingNoConnect => "E075",
        }
    }
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Violation {
    /// Stable code such as `E010`.
    pub code: &'static str,
    /// The rule.
    pub rule: Rule,
    /// Severity.
    pub severity: Severity,
    /// Sheet index, when the finding has a place.
    pub sheet: Option<usize>,
    /// Sheet name.
    pub sheet_name: Option<String>,
    /// Grid position, when known.
    pub position: Option<Point>,
    /// A message naming the parts, pins and nets involved.
    pub message: String,
    /// The things involved, e.g. `R1.2`, `net OUT`, `V1`.
    pub items: Vec<String>,
}

/// The result of a rules check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ErcReport {
    /// Every violation, ordered by severity (errors first) then by sheet, position and code.
    pub violations: Vec<Violation>,
}

impl ErcReport {
    /// Number of violations of a severity.
    #[must_use]
    pub fn count(&self, severity: Severity) -> usize {
        self.violations
            .iter()
            .filter(|v| v.severity == severity)
            .count()
    }

    /// True if there is no error.
    #[must_use]
    pub fn is_clean_of_errors(&self) -> bool {
        self.count(Severity::Error) == 0
    }

    /// Violations of one rule.
    pub fn of(&self, rule: Rule) -> impl Iterator<Item = &Violation> {
        self.violations.iter().filter(move |v| v.rule == rule)
    }

    /// JSON text of the report.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }
}

struct Sink<'a> {
    s: &'a Schematic,
    out: Vec<Violation>,
}

impl Sink<'_> {
    fn push(
        &mut self,
        rule: Rule,
        severity: Severity,
        sheet: Option<usize>,
        position: Option<Point>,
        message: String,
        items: Vec<String>,
    ) {
        self.out.push(Violation {
            code: rule.code(),
            rule,
            severity,
            sheet,
            sheet_name: sheet
                .and_then(|i| self.s.sheets.get(i))
                .map(|sh| sh.name.clone()),
            position,
            message,
            items,
        });
    }
}

fn pin_label(s: &Schematic, lib: &Library, r: PinRef) -> String {
    let part = s.sheets.get(r.sheet).and_then(|sh| sh.parts.get(r.part));
    let pin = part
        .and_then(|p| lib.get(&p.lib_id))
        .and_then(|d| d.pins.get(r.pin));
    match (part, pin) {
        (Some(p), Some(d)) => format!("{}.{}", p.refdes, d.number),
        (Some(p), None) => p.refdes.clone(),
        _ => "?".into(),
    }
}

fn pin_type(s: &Schematic, lib: &Library, r: PinRef) -> PinType {
    s.sheets
        .get(r.sheet)
        .and_then(|sh| sh.parts.get(r.part))
        .and_then(|p| lib.get(&p.lib_id))
        .and_then(|d| d.pins.get(r.pin))
        .map_or(PinType::Passive, |d| d.pin_type)
}

/// Run every rule on `s`.
///
/// The check never fails: a schematic that cannot be resolved (an unknown symbol) yields a report
/// with that finding.
#[must_use]
pub fn check(s: &Schematic) -> ErcReport {
    let lib = Library::builtin();
    let mut sink = Sink { s, out: Vec::new() };
    // Unknown symbols stop connectivity: report them and stop.
    let mut unknown = false;
    for (si, sh) in s.sheets.iter().enumerate() {
        for p in &sh.parts {
            if lib.get(&p.lib_id).is_none() {
                unknown = true;
                sink.push(
                    Rule::UnknownSymbol,
                    Severity::Error,
                    Some(si),
                    Some(p.position),
                    format!(
                        "{} uses the symbol `{}`, which is not in the library",
                        p.refdes, p.lib_id
                    ),
                    vec![p.refdes.clone()],
                );
            }
        }
    }
    if unknown {
        return finish(sink.out);
    }
    let res = match resolve_nets(s, lib) {
        Ok(r) => r,
        Err(e) => {
            sink.push(
                Rule::PartRejected,
                Severity::Error,
                None,
                None,
                e.to_string(),
                vec![],
            );
            return finish(sink.out);
        }
    };
    let names = name_nets(s, &res);
    duplicates(&mut sink, lib);
    connectivity_warnings(&mut sink, &res);
    pins_and_nets(&mut sink, lib, &res, &names);
    if let Ok(x) = extract(s) {
        for i in &x.issues {
            let rule = match i.kind {
                IssueKind::MissingValue => Rule::MissingValue,
                IssueKind::InvalidValue => Rule::InvalidValue,
                IssueKind::UnknownModel => Rule::UnknownModel,
                IssueKind::Rejected => Rule::PartRejected,
            };
            // A duplicate reference designator is reported once by E030, not again as a rejection.
            if rule == Rule::PartRejected && i.message.contains("duplicate element name") {
                continue;
            }
            let pos = s
                .sheets
                .get(i.sheet)
                .and_then(|sh| sh.parts.get(i.part))
                .map(|p| p.position);
            sink.push(
                rule,
                Severity::Error,
                Some(i.sheet),
                pos,
                i.message.clone(),
                vec![i.refdes.clone()],
            );
        }
        circuit_rules(&mut sink, &x.netlist, &res, s);
    }
    finish(sink.out)
}

fn finish(mut v: Vec<Violation>) -> ErcReport {
    v.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(a.sheet.cmp(&b.sheet))
            .then(a.position.cmp(&b.position))
            .then(a.code.cmp(b.code))
            .then(a.message.cmp(&b.message))
    });
    ErcReport { violations: v }
}

fn duplicates(sink: &mut Sink<'_>, lib: &Library) {
    let _ = lib;
    let mut seen: BTreeMap<String, (usize, Point)> = BTreeMap::new();
    let s = sink.s;
    for (si, sh) in s.sheets.iter().enumerate() {
        for p in &sh.parts {
            let key = p.refdes.to_uppercase();
            if key.is_empty() {
                continue;
            }
            if let Some((fs, fp)) = seen.get(&key) {
                let first_sheet = s.sheets.get(*fs).map_or("?", |x| x.name.as_str());
                sink.push(
                    Rule::DuplicateRefdes,
                    Severity::Error,
                    Some(si),
                    Some(p.position),
                    format!(
                        "reference {} is used more than once (first on sheet `{}` at ({}, {}))",
                        p.refdes, first_sheet, fp.x, fp.y
                    ),
                    vec![p.refdes.clone()],
                );
            } else {
                seen.insert(key, (si, p.position));
            }
        }
    }
}

fn connectivity_warnings(sink: &mut Sink<'_>, res: &NetResolution) {
    for w in &res.warnings {
        match w {
            NetWarning::TouchWithoutJunction { sheet, point } => sink.push(Rule::TouchWithoutJunction, Severity::Warning, Some(*sheet), Some(*point),
                format!("a wire end or pin touches a wire at ({}, {}) but there is no junction, so they are not connected", point.x, point.y), vec![]),
            NetWarning::FloatingLabel { sheet, name, point } => sink.push(Rule::FloatingLabel, Severity::Warning, Some(*sheet), Some(*point),
                format!("label `{name}` is not on a wire or pin"), vec![format!("label {name}")]),
            NetWarning::AmbiguousLabel { sheet, name, point } => sink.push(Rule::AmbiguousLabel, Severity::Warning, Some(*sheet), Some(*point),
                format!("`{name}` sits where wires cross without a junction; move it onto one wire"), vec![format!("label {name}")]),
            NetWarning::FloatingPowerPort { sheet, name, point } => sink.push(Rule::FloatingPowerPort, Severity::Warning, Some(*sheet), Some(*point),
                format!("power port `{name}` is not on a wire or pin"), vec![format!("port {name}")]),
            NetWarning::DanglingJunction { sheet, point } => sink.push(Rule::DanglingJunction, Severity::Info, Some(*sheet), Some(*point),
                format!("junction at ({}, {}) joins nothing", point.x, point.y), vec![]),
            NetWarning::DanglingNoConnect { sheet, point } => sink.push(Rule::DanglingNoConnect, Severity::Info, Some(*sheet), Some(*point),
                format!("no-connect flag at ({}, {}) is not on a pin", point.x, point.y), vec![]),
        }
    }
}

fn pins_and_nets(sink: &mut Sink<'_>, lib: &Library, res: &NetResolution, names: &[String]) {
    let s = sink.s;
    let at = |r: PinRef| res.pin_points.get(&r).copied();
    for (ni, net) in res.nets.iter().enumerate() {
        let nname = names.get(ni).map_or("?", String::as_str);
        let alone = net.pins.len() == 1
            && net.wires.is_empty()
            && net.labels.is_empty()
            && net.power_ports.is_empty();
        let is_ground = nname == "0";
        // E001 / E003 / E002
        for &r in &net.pins {
            let t = pin_type(s, lib, r);
            let flagged = res.no_connect_pins.contains(&r);
            let connected_to_something = net.pins.len() > 1
                || !net.wires.is_empty()
                || !net.labels.is_empty()
                || !net.power_ports.is_empty();
            if (t == PinType::Nc || flagged) && connected_to_something {
                sink.push(
                    Rule::NoConnectConnected,
                    Severity::Error,
                    Some(r.sheet),
                    at(r),
                    format!(
                        "pin {} is marked not connected but is connected to net {nname}",
                        pin_label(s, lib, r)
                    ),
                    vec![pin_label(s, lib, r)],
                );
            } else if alone && !flagged && !matches!(t, PinType::Nc | PinType::Unconnected) {
                sink.push(
                    Rule::PinUnconnected,
                    Severity::Error,
                    Some(r.sheet),
                    at(r),
                    format!(
                        "pin {} is not connected; connect it or place a no-connect flag on it",
                        pin_label(s, lib, r)
                    ),
                    vec![pin_label(s, lib, r)],
                );
            }
        }
        if net.pins.len() == 1
            && !alone
            && net.power_ports.is_empty()
            && !res.no_connect_pins.contains(&net.pins[0])
        {
            let r = net.pins[0];
            sink.push(
                Rule::NetSinglePin,
                Severity::Warning,
                Some(r.sheet),
                at(r),
                format!(
                    "net {nname} has only one pin ({}); it leads nowhere",
                    pin_label(s, lib, r)
                ),
                vec![pin_label(s, lib, r), format!("net {nname}")],
            );
        }
        // Pin-type conflicts, once per pair, capped per net.
        let live: Vec<PinRef> = net
            .pins
            .iter()
            .copied()
            .filter(|r| !res.no_connect_pins.contains(r))
            .collect();
        let mut reported = 0usize;
        let mut suppressed = 0usize;
        for (i, &a) in live.iter().enumerate() {
            for &b in &live[i + 1..] {
                let (ta, tb) = (pin_type(s, lib, a), pin_type(s, lib, b));
                if ta == PinType::Nc || tb == PinType::Nc {
                    continue; // reported by E003
                }
                let v = conflict(ta, tb);
                if v == Verdict::Ok {
                    continue;
                }
                if reported >= 20 {
                    suppressed += 1;
                    continue;
                }
                reported += 1;
                let (rule, severity) = match (ta, tb) {
                    (PinType::Output, PinType::Output) => (Rule::OutputConflict, Severity::Error),
                    (PinType::PowerOut, PinType::PowerOut) => {
                        (Rule::PowerOutputConflict, Severity::Error)
                    }
                    _ => (
                        Rule::PinTypeConflict,
                        if v == Verdict::Error {
                            Severity::Error
                        } else {
                            Severity::Warning
                        },
                    ),
                };
                let (la, lb) = (pin_label(s, lib, a), pin_label(s, lib, b));
                sink.push(
                    rule,
                    severity,
                    Some(a.sheet),
                    at(a),
                    format!("{la} ({ta:?}) and {lb} ({tb:?}) are on the same net {nname}"),
                    vec![la, lb, format!("net {nname}")],
                );
            }
        }
        if suppressed > 0 {
            sink.push(
                Rule::PinTypeConflict,
                Severity::Warning,
                None,
                None,
                format!("net {nname} has {suppressed} more pin-type conflicts that are not listed"),
                vec![format!("net {nname}")],
            );
        }
        // E020 power input undriven
        let has_power_in = live
            .iter()
            .any(|&r| pin_type(s, lib, r) == PinType::PowerIn);
        if has_power_in && !is_ground {
            let driven = live.iter().any(|&r| {
                pin_type(s, lib, r) == PinType::PowerOut
                    || s.sheets
                        .get(r.sheet)
                        .and_then(|sh| sh.parts.get(r.part))
                        .and_then(|p| lib.get(&p.lib_id))
                        .is_some_and(|d| d.device == Device::VSource)
            });
            if !driven {
                let pins: Vec<String> = live
                    .iter()
                    .filter(|&&r| pin_type(s, lib, r) == PinType::PowerIn)
                    .map(|&r| pin_label(s, lib, r))
                    .collect();
                let first = live
                    .iter()
                    .copied()
                    .find(|&r| pin_type(s, lib, r) == PinType::PowerIn);
                sink.push(Rule::PowerInputUndriven, Severity::Error, first.map(|r| r.sheet), first.and_then(at),
                    format!("power pin {} on net {nname} has no power source: nothing on the net drives it (add a voltage source or a power output)", pins.join(", ")),
                    pins);
            }
        }
        // E040 floating input
        let inputs: Vec<PinRef> = live
            .iter()
            .copied()
            .filter(|&r| pin_type(s, lib, r) == PinType::Input)
            .collect();
        if !inputs.is_empty() && net.power_ports.is_empty() && !is_ground {
            let only_inputs = live
                .iter()
                .all(|&r| matches!(pin_type(s, lib, r), PinType::Input | PinType::PowerIn));
            if only_inputs {
                let items: Vec<String> = inputs.iter().map(|&r| pin_label(s, lib, r)).collect();
                sink.push(
                    Rule::FloatingInput,
                    Severity::Warning,
                    Some(inputs[0].sheet),
                    at(inputs[0]),
                    format!(
                        "input {} on net {nname} is not driven by anything",
                        items.join(", ")
                    ),
                    items,
                );
            }
        }
        // E060 name conflict
        let mut distinct: BTreeSet<String> = BTreeSet::new();
        let mut shown: BTreeSet<String> = BTreeSet::new();
        for l in &net.labels {
            if let Some(x) = s.sheets.get(l.sheet).and_then(|sh| sh.labels.get(l.index)) {
                let c = canon(&x.name);
                if distinct.insert(c) {
                    shown.insert(x.name.clone());
                }
            }
        }
        for p in &net.power_ports {
            if let Some(x) = s
                .sheets
                .get(p.sheet)
                .and_then(|sh| sh.power_ports.get(p.index))
            {
                let c = canon(&x.name);
                if distinct.insert(c) {
                    shown.insert(x.name.clone());
                }
            }
        }
        if distinct.len() > 1 {
            let list: Vec<String> = shown.into_iter().collect();
            let first = net
                .pins
                .first()
                .map(|r| r.sheet)
                .or_else(|| net.labels.first().map(|l| l.sheet));
            sink.push(
                Rule::NetNameConflict,
                Severity::Warning,
                first,
                None,
                format!(
                    "one net carries several names ({}); it is called `{nname}` in the netlist",
                    list.join(", ")
                ),
                list,
            );
        }
    }
}

fn canon(n: &str) -> String {
    let l = n.trim().to_lowercase();
    if l == "gnd" {
        "0".into()
    } else {
        l
    }
}

struct Forest {
    parent: Vec<usize>,
}

impl Forest {
    fn new(n: usize) -> Self {
        Forest {
            parent: (0..n).collect(),
        }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        self.parent[rb] = ra;
        true
    }
}

fn circuit_rules(sink: &mut Sink<'_>, nl: &Netlist, res: &NetResolution, s: &Schematic) {
    let n = nl.nodes.len();
    // E021: ground reference.
    let any_parts = s.sheets.iter().any(|sh| !sh.parts.is_empty());
    let has_ground = nl
        .elements
        .iter()
        .any(|e| e.nodes().iter().any(|x| x.is_ground()));
    if any_parts && !has_ground {
        sink.push(Rule::NoGround, Severity::Error, None, None,
            "the circuit has no ground: connect a ground symbol (power port named GND or 0); every node voltage is measured from it".into(), vec![]);
    }
    let _ = res;
    // E050: loops of ideal voltage sources (V, E, H outputs).
    let mut forest = Forest::new(n);
    let mut adj: Vec<Vec<(usize, String)>> = vec![Vec::new(); n];
    for e in &nl.elements {
        let (name, p, q) = match e {
            Element::VSource { name, p, n, .. }
            | Element::Vcvs { name, p, n, .. }
            | Element::Ccvs { name, p, n, .. } => (name.clone(), p.0 as usize, n.0 as usize),
            _ => continue,
        };
        if p == q {
            sink.push(Rule::VoltageSourceLoop, Severity::Error, None, None,
                format!("voltage source {name} has both terminals on node `{}`: it forces 0 V = its own value", nl.nodes.name(NodeId(p as u32))), vec![name]);
            continue;
        }
        if forest.union(p, q) {
            adj[p].push((q, name.clone()));
            adj[q].push((p, name));
        } else {
            let path = bfs_path(&adj, p, q);
            let mut items = path;
            items.push(name.clone());
            let how = if items.len() == 2 {
                "in parallel"
            } else {
                "in a loop"
            };
            sink.push(Rule::VoltageSourceLoop, Severity::Error, None, None,
                format!("ideal voltage sources {} are {how} between `{}` and `{}`: their voltages cannot all hold (add a series resistance)", items.join(", "), nl.nodes.name(NodeId(p as u32)), nl.nodes.name(NodeId(q as u32))),
                items);
            adj[p].push((q, name.clone()));
            adj[q].push((p, name));
        }
    }
    // E051 / E052: DC conduction graph.
    let mut dc = Forest::new(n);
    let mut touching_i: Vec<Vec<String>> = vec![Vec::new(); n];
    for e in &nl.elements {
        match e {
            Element::Resistor { a, b, .. } | Element::Inductor { a, b, .. } => {
                dc.union(a.0 as usize, b.0 as usize);
            }
            Element::VSource { p, n, .. }
            | Element::Vcvs { p, n, .. }
            | Element::Ccvs { p, n, .. }
            | Element::Switch { p, n, .. } => {
                dc.union(p.0 as usize, n.0 as usize);
            }
            Element::Diode { anode, cathode, .. } => {
                dc.union(anode.0 as usize, cathode.0 as usize);
            }
            Element::Bjt { c, b, e, .. } => {
                dc.union(c.0 as usize, e.0 as usize);
                dc.union(b.0 as usize, e.0 as usize);
            }
            Element::Mosfet { d, s, bulk, .. } => {
                dc.union(d.0 as usize, s.0 as usize);
                dc.union(bulk.0 as usize, s.0 as usize);
            }
            Element::OpAmp { out, .. } => {
                dc.union(out.0 as usize, 0);
            }
            Element::ISource { p, n, name, .. }
            | Element::Cccs { p, n, name, .. }
            | Element::Vccs { p, n, name, .. } => {
                for x in [p, n] {
                    if let Some(v) = touching_i.get_mut(x.0 as usize) {
                        v.push(name.clone());
                    }
                }
            }
            Element::Capacitor { .. } | Element::Mutual { .. } | Element::Instance { .. } => {}
        }
    }
    // Mutual inductance gives no DC path. Capacitors give none either.
    let ground_root = dc.find(0);
    let mut comps: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut used = vec![false; n];
    for e in &nl.elements {
        for x in e.nodes() {
            if let Some(u) = used.get_mut(x.0 as usize) {
                *u = true;
            }
        }
    }
    for (i, &is_used) in used.iter().enumerate().skip(1) {
        if !is_used {
            continue;
        }
        let r = dc.find(i);
        if r != ground_root {
            comps.entry(r).or_default().push(i);
        }
    }
    for nodes in comps.values() {
        let mut isrc: BTreeSet<String> = BTreeSet::new();
        for &i in nodes {
            isrc.extend(touching_i[i].iter().cloned());
        }
        let node_names: Vec<String> = nodes
            .iter()
            .map(|&i| nl.nodes.name(NodeId(i as u32)).to_string())
            .collect();
        if isrc.is_empty() {
            sink.push(Rule::NoDcPath, Severity::Warning, None, None,
                format!("node(s) {} have no DC path to ground (only capacitors, gates or sources of current reach them); the DC solution is undefined without a resistor to ground", node_names.join(", ")),
                node_names);
        } else {
            let list: Vec<String> = isrc.into_iter().collect();
            sink.push(Rule::CurrentSourceCutset, Severity::Error, None, None,
                format!("current source(s) {} meet node(s) {} which have no other DC path to ground: the current has nowhere to return (sources in series, or a cut set of current sources)", list.join(", "), node_names.join(", ")),
                list);
        }
    }
}

fn bfs_path(adj: &[Vec<(usize, String)>], from: usize, to: usize) -> Vec<String> {
    let mut prev: BTreeMap<usize, (usize, String)> = BTreeMap::new();
    let mut q = VecDeque::from([from]);
    let mut seen = BTreeSet::from([from]);
    while let Some(u) = q.pop_front() {
        if u == to {
            break;
        }
        for (v, name) in adj.get(u).map(Vec::as_slice).unwrap_or(&[]) {
            if seen.insert(*v) {
                prev.insert(*v, (u, name.clone()));
                q.push_back(*v);
            }
        }
    }
    let mut out = Vec::new();
    let mut cur = to;
    while let Some((p, name)) = prev.get(&cur) {
        out.push(name.clone());
        cur = *p;
    }
    out.reverse();
    out
}

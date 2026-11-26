// SPDX-License-Identifier: MIT OR Apache-2.0
//! The SPICE reader.
//!
//! Supported: the title line, comments (`*` at line start, `;` and ` $` inline), continuation lines
//! (`+`), case-insensitive input, SI suffixes, `.param` with `{expr}`, `.model`, `.subckt`/`.ends`
//! (no parameters, no nested definitions), `.options`, `.temp`, `.op`, `.dc`, `.ac`, `.tran`, `.end`,
//! and elements R C L V I D E G F H K S Q M U X.
//!
//! `.print`, `.plot`, `.probe`, `.save`, `.width`, `.nodeset`, `.meas` and `.measure` are ignored
//! with a warning. `.include`, `.lib`, `.global`, `.ic`, `.noise`, `.step`, `.tf`, `.four` and other
//! unsupported commands are errors, never silently skipped. A netlist can never make this reader
//! open a file.

use crate::expr;
use circuito_core::netlist::{
    Analysis, Element, Model, ModelKind, Netlist, NetlistError, Subckt, Sweep,
};
use circuito_core::source::Source;
use circuito_core::value::parse_value;
use std::collections::{BTreeMap, HashMap};
use thiserror::Error;

/// Largest input accepted, in bytes.
pub const MAX_INPUT_BYTES: usize = 16 * 1024 * 1024;
/// Longest logical line (after joining continuations), in bytes.
pub const MAX_LINE_BYTES: usize = 64 * 1024;
/// Most `.param` definitions.
pub const MAX_PARAMS: usize = 10_000;

/// A reading error with the line it was found on.
#[derive(Debug, Clone, PartialEq, Error)]
#[error("{}", self.render())]
pub struct ParseError {
    /// 1-based line number; 0 when the error concerns the whole file.
    pub line: usize,
    /// What is wrong and, where possible, how to fix it.
    pub message: String,
}

impl ParseError {
    fn render(&self) -> String {
        if self.line == 0 {
            self.message.clone()
        } else {
            format!("line {}: {}", self.line, self.message)
        }
    }
}

/// Something worth telling the person that did not stop the reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// 1-based line number.
    pub line: usize,
    /// The message.
    pub message: String,
}

/// A successfully read netlist with the warnings produced on the way.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// The netlist (validated).
    pub netlist: Netlist,
    /// Warnings in line order.
    pub warnings: Vec<Warning>,
}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError {
        line,
        message: message.into(),
    })
}

struct Logical {
    line: usize,
    text: String,
}

/// Join continuations, drop comments, keep line numbers.
fn logical_lines(text: &str) -> Result<(String, Vec<Logical>), ParseError> {
    let mut out: Vec<Logical> = Vec::new();
    let mut title = String::new();
    for (i, raw) in text.split('\n').enumerate() {
        let n = i + 1;
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if i == 0 {
            title = raw.trim().to_string();
            continue;
        }
        let trimmed = raw.trim_start();
        if trimmed.starts_with('*') || trimmed.is_empty() {
            continue;
        }
        // Inline comments: `;` anywhere, `$` at the start of a token.
        let mut cut = raw.len();
        let mut prev_ws = true;
        for (k, c) in raw.char_indices() {
            if c == ';' || (c == '$' && prev_ws) {
                cut = k;
                break;
            }
            prev_ws = c.is_whitespace();
        }
        let body = raw[..cut].trim();
        if body.is_empty() {
            continue;
        }
        if let Some(rest) = body.strip_prefix('+') {
            match out.last_mut() {
                Some(prev) => {
                    prev.text.push(' ');
                    prev.text.push_str(rest.trim());
                    if prev.text.len() > MAX_LINE_BYTES {
                        return err(prev.line, format!("the line (with its continuations) is longer than {MAX_LINE_BYTES} bytes"));
                    }
                }
                None => return err(n, "a continuation line `+` has nothing to continue"),
            }
            continue;
        }
        if body.len() > MAX_LINE_BYTES {
            return err(n, format!("the line is longer than {MAX_LINE_BYTES} bytes"));
        }
        out.push(Logical {
            line: n,
            text: body.to_string(),
        });
    }
    Ok((title, out))
}

/// Replace every `{expr}` by its value, using `params`.
fn substitute(
    line: usize,
    text: &str,
    params: &BTreeMap<String, f64>,
) -> Result<String, ParseError> {
    if !text.contains('{') && !text.contains('}') {
        return Ok(text.to_string());
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut count = 0;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let mut depth = 1;
        let mut end = None;
        for (k, c) in after.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(k);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            return err(line, "`{` has no matching `}`");
        };
        count += 1;
        if count > 1000 {
            return err(line, "more than 1000 {expressions} on one line");
        }
        let v =
            expr::evaluate(&after[..end], params).map_err(|m| ParseError { line, message: m })?;
        out.push_str(&format!("{v:e}"));
        rest = &after[end + 1..];
    }
    if rest.contains('}') {
        return err(line, "`}` has no matching `{`");
    }
    out.push_str(rest);
    Ok(out)
}

/// Split into tokens: `(`, `)` and `,` separate; `a = b` becomes `a=b`.
fn tokens(text: &str) -> Vec<String> {
    let spaced: String = text
        .chars()
        .map(|c| match c {
            '(' | ')' | ',' => ' ',
            _ => c,
        })
        .collect();
    let mut padded = String::with_capacity(spaced.len() + 8);
    for c in spaced.chars() {
        if c == '=' {
            padded.push_str(" = ");
        } else {
            padded.push(c);
        }
    }
    let raw: Vec<&str> = padded.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == "=" {
            // `a = b`, `a= b`, `a =b`: glue to the previous token and the next.
            let left = out.pop().unwrap_or_default();
            let right = raw.get(i + 1).copied().unwrap_or("");
            out.push(format!("{left}={right}"));
            i += 2;
        } else {
            out.push(raw[i].to_string());
            i += 1;
        }
    }
    out
}

fn split_kv(t: &str) -> Option<(String, &str)> {
    let (k, v) = t.split_once('=')?;
    if k.is_empty() {
        return None;
    }
    Some((k.to_uppercase(), v))
}

struct Reader {
    nl: Netlist,
    warnings: Vec<Warning>,
    params: BTreeMap<String, f64>,
    models: HashMap<String, ModelKind>,
    element_lines: HashMap<String, usize>,
    current: Option<(Subckt, usize)>,
    ended: bool,
}

impl Reader {
    fn warn(&mut self, line: usize, message: impl Into<String>) {
        self.warnings.push(Warning {
            line,
            message: message.into(),
        });
    }

    fn value(&self, line: usize, who: &str, what: &str, tok: &str) -> Result<f64, ParseError> {
        parse_value(tok).map_err(|e| ParseError {
            line,
            message: format!("{who}: {what}: {e}"),
        })
    }

    fn node(
        &mut self,
        line: usize,
        tok: &str,
    ) -> Result<circuito_core::netlist::NodeId, ParseError> {
        let r = match &mut self.current {
            Some((s, _)) => s.nodes.intern(tok),
            None => self.nl.node(tok),
        };
        r.map_err(|e| ParseError {
            line,
            message: e.to_string(),
        })
    }

    fn add(&mut self, line: usize, e: Element) -> Result<(), ParseError> {
        let name = e.name().to_uppercase();
        let r = match &mut self.current {
            Some((s, _)) => s.add(e),
            None => self.nl.add(e),
        };
        r.map_err(|er| ParseError {
            line,
            message: er.to_string(),
        })?;
        if self.current.is_none() {
            self.element_lines.insert(name, line);
        }
        Ok(())
    }

    fn element(&mut self, line: usize, toks: &[String]) -> Result<(), ParseError> {
        let name = toks[0].clone();
        let letter = name
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or(' ');
        let need = |n: usize, usage: &str| -> Result<(), ParseError> {
            if toks.len() < n {
                err(line, format!("{name}: too few fields; expected `{usage}`"))
            } else {
                Ok(())
            }
        };
        // Split trailing key=value fields from positional ones.
        let positional: Vec<&String> = toks[1..].iter().filter(|t| !t.contains('=')).collect();
        let kvs: Vec<(String, &str)> = toks[1..].iter().filter_map(|t| split_kv(t)).collect();
        let kv_unknown = |me: &mut Reader, allowed: &[&str]| {
            for (k, _) in &kvs {
                if !allowed.contains(&k.as_str()) {
                    me.warn(line, format!("{name}: parameter {k} is ignored"));
                }
            }
        };
        match letter {
            'R' => {
                need(4, "Rname n1 n2 value")?;
                let (a, b) = (self.node(line, &toks[1])?, self.node(line, &toks[2])?);
                let value = self.value(line, &name, "resistance", &toks[3])?;
                kv_unknown(self, &[]);
                self.add(line, Element::Resistor { name, a, b, value })
            }
            'C' | 'L' => {
                need(4, "name n1 n2 value [IC=x]")?;
                let (a, b) = (self.node(line, &toks[1])?, self.node(line, &toks[2])?);
                let what = if letter == 'C' { "capacitance" } else { "inductance" };
                let value = self.value(line, &name, what, &toks[3])?;
                let mut ic = None;
                for (k, v) in &kvs {
                    if k == "IC" {
                        ic = Some(self.value(line, &name, "IC", v)?);
                    }
                }
                kv_unknown(self, &["IC"]);
                if letter == 'C' {
                    self.add(line, Element::Capacitor { name, a, b, value, ic })
                } else {
                    self.add(line, Element::Inductor { name, a, b, value, ic })
                }
            }
            'V' | 'I' => {
                need(3, "name n+ n- value")?;
                let (p, n) = (self.node(line, &toks[1])?, self.node(line, &toks[2])?);
                if toks.len() < 4 {
                    return err(line, format!("{name}: a source needs a value, for example `{name} {} {} DC 5`", toks[1], toks[2]));
                }
                let spec = toks[3..].join(" ");
                let source = Source::parse(&spec).map_err(|e| ParseError { line, message: format!("{name}: {e}") })?;
                if letter == 'V' {
                    self.add(line, Element::VSource { name, p, n, source })
                } else {
                    self.add(line, Element::ISource { name, p, n, source })
                }
            }
            'D' => {
                need(4, "Dname anode cathode model [area]")?;
                let (an, ca) = (self.node(line, &toks[1])?, self.node(line, &toks[2])?);
                let model = toks[3].clone();
                let mut area = 1.0;
                if let Some(t) = positional.get(3) {
                    area = self.value(line, &name, "area", t)?;
                }
                for (k, v) in &kvs {
                    if k == "AREA" {
                        area = self.value(line, &name, "AREA", v)?;
                    }
                }
                kv_unknown(self, &["AREA"]);
                self.add(line, Element::Diode { name, anode: an, cathode: ca, model, area })
            }
            'E' | 'G' => {
                need(6, "name n+ n- nc+ nc- value")?;
                if toks[5].to_uppercase().starts_with("VALUE") || toks[5].to_uppercase().starts_with("POLY") {
                    return err(line, format!("{name}: behavioural/polynomial sources are not supported; give a constant gain"));
                }
                let n = [self.node(line, &toks[1])?, self.node(line, &toks[2])?, self.node(line, &toks[3])?, self.node(line, &toks[4])?];
                let v = self.value(line, &name, "gain", &toks[5])?;
                if letter == 'E' {
                    self.add(line, Element::Vcvs { name, p: n[0], n: n[1], cp: n[2], cn: n[3], gain: v })
                } else {
                    self.add(line, Element::Vccs { name, p: n[0], n: n[1], cp: n[2], cn: n[3], gm: v })
                }
            }
            'F' | 'H' => {
                need(5, "name n+ n- Vcontrol value")?;
                let (p, n) = (self.node(line, &toks[1])?, self.node(line, &toks[2])?);
                let control = toks[3].to_uppercase();
                let v = self.value(line, &name, "gain", &toks[4])?;
                if letter == 'F' {
                    self.add(line, Element::Cccs { name, p, n, control, gain: v })
                } else {
                    self.add(line, Element::Ccvs { name, p, n, control, rm: v })
                }
            }
            'K' => {
                need(4, "Kname L1 L2 k")?;
                let k = self.value(line, &name, "coupling", &toks[3])?;
                self.add(line, Element::Mutual { name, l1: toks[1].to_uppercase(), l2: toks[2].to_uppercase(), k })
            }
            'S' => {
                need(6, "Sname n+ n- nc+ nc- model")?;
                let n = [self.node(line, &toks[1])?, self.node(line, &toks[2])?, self.node(line, &toks[3])?, self.node(line, &toks[4])?];
                if toks.len() > 6 {
                    self.warn(line, format!("{name}: the initial state `{}` is ignored; the switch starts from its control voltage", toks[6]));
                }
                self.add(line, Element::Switch { name, p: n[0], n: n[1], cp: n[2], cn: n[3], model: toks[5].clone() })
            }
            'Q' => {
                need(5, "Qname c b e model [area]")?;
                // `c b e model`, `c b e model area`, `c b e substrate model [area]`.
                let pos: Vec<&String> = positional;
                let model_idx = if pos.len() >= 4 && self.models.get(&pos[3].to_uppercase()).is_some_and(|k| matches!(k, ModelKind::Npn | ModelKind::Pnp)) {
                    3
                } else if pos.len() >= 5 {
                    self.warn(line, format!("{name}: the substrate node `{}` is ignored", pos[3]));
                    4
                } else {
                    3
                };
                let (c, b, e) = (self.node(line, pos[0])?, self.node(line, pos[1])?, self.node(line, pos[2])?);
                let model = pos.get(model_idx).map(|s| (*s).clone()).ok_or_else(|| ParseError { line, message: format!("{name}: missing model name") })?;
                let mut area = 1.0;
                if let Some(t) = pos.get(model_idx + 1) {
                    area = self.value(line, &name, "area", t)?;
                }
                for (k, v) in &kvs {
                    if k == "AREA" {
                        area = self.value(line, &name, "AREA", v)?;
                    }
                }
                kv_unknown(self, &["AREA"]);
                self.add(line, Element::Bjt { name, c, b, e, model, area })
            }
            'M' => {
                need(6, "Mname d g s b model [W=..] [L=..]")?;
                let n = [self.node(line, &toks[1])?, self.node(line, &toks[2])?, self.node(line, &toks[3])?, self.node(line, &toks[4])?];
                let model = toks[5].clone();
                let (mut w, mut l) = (None, None);
                for (k, v) in &kvs {
                    match k.as_str() {
                        "W" => w = Some(self.value(line, &name, "W", v)?),
                        "L" => l = Some(self.value(line, &name, "L", v)?),
                        _ => {}
                    }
                }
                kv_unknown(self, &["W", "L"]);
                self.add(line, Element::Mosfet { name, d: n[0], g: n[1], s: n[2], bulk: n[3], model, w, l })
            }
            'U' => {
                need(5, "Uname in+ in- out model")?;
                let n = [self.node(line, &toks[1])?, self.node(line, &toks[2])?, self.node(line, &toks[3])?];
                self.add(line, Element::OpAmp { name, inp: n[0], inn: n[1], out: n[2], model: toks[4].clone() })
            }
            'X' => {
                if let Some((k, _)) = kvs.first() {
                    return err(line, format!("{name}: subcircuit parameter {k}= is not supported; give each instance its own .subckt or use .param"));
                }
                if positional.len() < 2 {
                    return err(line, format!("{name}: expected `Xname node... subcktname`"));
                }
                let sub = positional[positional.len() - 1].clone();
                let mut nodes = Vec::new();
                for t in &positional[..positional.len() - 1] {
                    nodes.push(self.node(line, t)?);
                }
                self.add(line, Element::Instance { name, nodes, subckt: sub })
            }
            _ => err(line, format!("`{name}`: unsupported element type `{letter}` (supported: R C L V I D E G F H K S Q M U X)")),
        }
    }

    fn model(&mut self, line: usize, toks: &[String]) -> Result<(), ParseError> {
        if toks.len() < 3 {
            return err(
                line,
                ".model needs a name and a type: `.model NAME TYPE(param=value ...)`",
            );
        }
        let name = toks[1].to_uppercase();
        let kind = ModelKind::from_keyword(&toks[2]).ok_or_else(|| ParseError {
            line,
            message: format!(
                ".model {name}: unsupported type `{}` (supported: D NPN PNP NMOS PMOS SW OPAMP)",
                toks[2]
            ),
        })?;
        let mut m = Model::new(&name, kind);
        for t in &toks[3..] {
            let Some((k, v)) = split_kv(t) else {
                return err(
                    line,
                    format!(".model {name}: expected `param=value`, found `{t}`"),
                );
            };
            let val = self.value(line, &format!(".model {name}"), &k, v)?;
            if m.params.insert(k.clone(), val).is_some() {
                return err(line, format!(".model {name}: parameter {k} is given twice"));
            }
        }
        if matches!(kind, ModelKind::Nmos | ModelKind::Pmos) {
            if let Some(level) = m.params.get("LEVEL") {
                if *level != 1.0 {
                    return err(
                        line,
                        format!(".model {name}: only MOSFET level 1 is supported (LEVEL={level})"),
                    );
                }
            }
        }
        for k in m.unknown_params() {
            self.warn(
                line,
                format!(".model {name}: parameter {k} is not used by the simulator"),
            );
        }
        self.nl.add_model(m).map_err(|e| ParseError {
            line,
            message: e.to_string(),
        })
    }

    /// `.param name=value ...` where a value is `{expr}` or a run without spaces. Handled on the
    /// raw text so a later assignment can use an earlier one on the same line.
    fn param(&mut self, line: usize, raw: &str) -> Result<(), ParseError> {
        let body = raw.trim_start().get(".param".len()..).unwrap_or("");
        let body = body.strip_prefix(['s', 'S']).unwrap_or(body);
        let mut rest = body.trim_start();
        if rest.is_empty() {
            return err(line, ".param needs `name=value`");
        }
        while !rest.is_empty() {
            let Some(eq) = rest.find('=') else {
                return err(
                    line,
                    format!(
                        ".param: expected `name=value`, found `{}`",
                        rest.split_whitespace().next().unwrap_or("")
                    ),
                );
            };
            let k = rest[..eq].trim().to_uppercase();
            if k.is_empty()
                || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                || k.chars().next().is_some_and(|c| c.is_ascii_digit())
            {
                return err(line, format!(".param: `{}` is not a valid parameter name (letters, digits and _, not starting with a digit)", rest[..eq].trim()));
            }
            let after = rest[eq + 1..].trim_start();
            let (expr_text, remaining) = if let Some(inner) = after.strip_prefix('{') {
                let mut depth = 1;
                let mut end = None;
                for (i, c) in inner.char_indices() {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(i);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let Some(end) = end else {
                    return err(line, "`{` has no matching `}`");
                };
                (&inner[..end], &inner[end + 1..])
            } else {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                (&after[..end], &after[end..])
            };
            if expr_text.is_empty() {
                return err(line, format!(".param {k}: missing value"));
            }
            let val = match parse_value(expr_text) {
                Ok(x) => x,
                Err(_) => expr::evaluate(expr_text, &self.params).map_err(|m| ParseError {
                    line,
                    message: format!(".param {k}: {m}"),
                })?,
            };
            if self.nl.params.len() >= MAX_PARAMS {
                return err(line, format!("more than {MAX_PARAMS} parameters"));
            }
            self.params.insert(k.clone(), val);
            self.nl.params.retain(|(n, _)| *n != k);
            self.nl.params.push((k, val));
            rest = remaining.trim_start();
        }
        Ok(())
    }

    fn analysis(&mut self, line: usize, cmd: &str, toks: &[String]) -> Result<(), ParseError> {
        let val = |s: &Reader, what: &str, t: &str| s.value(line, cmd, what, t);
        let a = match cmd {
            ".op" => Analysis::Op,
            ".dc" => {
                if toks.len() == 9 {
                    return err(line, ".dc: nested (two-source) sweeps are not supported");
                }
                if toks.len() != 5 {
                    return err(line, ".dc: expected `.dc source start stop step`");
                }
                Analysis::Dc {
                    source: toks[1].to_uppercase(),
                    start: val(self, "start", &toks[2])?,
                    stop: val(self, "stop", &toks[3])?,
                    step: val(self, "step", &toks[4])?,
                }
            }
            ".ac" => {
                if toks.len() != 5 {
                    return err(line, ".ac: expected `.ac dec|oct|lin points fstart fstop`");
                }
                let sweep = match toks[1].to_lowercase().as_str() {
                    "dec" => Sweep::Dec,
                    "oct" => Sweep::Oct,
                    "lin" => Sweep::Lin,
                    other => {
                        return err(
                            line,
                            format!(".ac: sweep type `{other}` is not dec, oct or lin"),
                        )
                    }
                };
                let p = val(self, "points", &toks[2])?;
                if p.fract() != 0.0 || !(1.0..=4.0e9).contains(&p) {
                    return err(line, format!(".ac: the number of points must be a whole number of at least 1 (got `{}`)", toks[2]));
                }
                Analysis::Ac {
                    sweep,
                    points: p as u32,
                    fstart: val(self, "fstart", &toks[3])?,
                    fstop: val(self, "fstop", &toks[4])?,
                }
            }
            ".tran" => {
                let mut pos: Vec<&String> = Vec::new();
                let mut uic = false;
                for t in &toks[1..] {
                    if t.eq_ignore_ascii_case("uic") {
                        uic = true;
                    } else {
                        pos.push(t);
                    }
                }
                if pos.len() < 2 || pos.len() > 4 {
                    return err(
                        line,
                        ".tran: expected `.tran tstep tstop [tstart [tmax]] [uic]`",
                    );
                }
                Analysis::Tran {
                    tstep: val(self, "tstep", pos[0])?,
                    tstop: val(self, "tstop", pos[1])?,
                    tstart: match pos.get(2) {
                        Some(t) => val(self, "tstart", t)?,
                        None => 0.0,
                    },
                    tmax: match pos.get(3) {
                        Some(t) => Some(val(self, "tmax", t)?),
                        None => None,
                    },
                    uic,
                }
            }
            _ => return err(line, format!("unknown analysis `{cmd}`")),
        };
        a.validate().map_err(|e| ParseError {
            line,
            message: e.to_string(),
        })?;
        self.nl.analyses.push(a);
        Ok(())
    }

    fn options(&mut self, line: usize, toks: &[String]) -> Result<(), ParseError> {
        for t in &toks[1..] {
            match split_kv(t) {
                Some((k, v)) => {
                    let x = self.value(line, ".options", &k, v)?;
                    self.nl.options.insert(k, x);
                }
                None => self.warn(line, format!(".options: the flag `{t}` is ignored")),
            }
        }
        Ok(())
    }

    fn dot(&mut self, line: usize, raw: &str, toks: &[String]) -> Result<(), ParseError> {
        let cmd = toks[0].to_lowercase();
        match cmd.as_str() {
            ".end" => {
                self.ended = true;
                Ok(())
            }
            ".model" => {
                if self.current.is_some() {
                    return err(line, ".model inside .subckt is not supported; move it to the top level");
                }
                self.model(line, toks)
            }
            ".param" | ".params" => {
                if self.current.is_some() {
                    return err(line, ".param inside .subckt is not supported; define it at the top level");
                }
                self.param(line, raw)
            }
            ".options" | ".option" | ".opt" => self.options(line, toks),
            ".temp" => {
                let t = toks.get(1).ok_or_else(|| ParseError { line, message: ".temp needs a temperature in degrees Celsius".into() })?;
                let x = self.value(line, ".temp", "temperature", t)?;
                self.nl.options.insert("TEMP".into(), x);
                Ok(())
            }
            ".op" | ".dc" | ".ac" | ".tran" => {
                if self.current.is_some() {
                    return err(line, format!("{cmd} is not allowed inside .subckt"));
                }
                self.analysis(line, &cmd, toks)
            }
            ".subckt" => {
                if self.current.is_some() {
                    return err(line, "nested .subckt definitions are not supported");
                }
                if toks.len() < 2 {
                    return err(line, ".subckt needs a name and its ports: `.subckt NAME port1 port2 ...`");
                }
                if let Some(t) = toks[2..].iter().find(|t| t.contains('=') || t.eq_ignore_ascii_case("params:")) {
                    return err(line, format!(".subckt {}: parameters (`{t}`) are not supported; use .param at the top level", toks[1]));
                }
                let ports: Vec<&str> = toks[2..].iter().map(String::as_str).collect();
                let s = Subckt::new(&toks[1], &ports).map_err(|e| ParseError { line, message: e.to_string() })?;
                self.current = Some((s, line));
                Ok(())
            }
            ".ends" => match self.current.take() {
                Some((s, start)) => {
                    if let Some(n) = toks.get(1) {
                        if !n.eq_ignore_ascii_case(&s.name) {
                            return err(line, format!(".ends {n} does not match the open .subckt {}", s.name));
                        }
                    }
                    self.nl.add_subckt(s).map_err(|e| ParseError { line: start, message: e.to_string() })
                }
                None => err(line, ".ends without a matching .subckt"),
            },
            ".print" | ".plot" | ".probe" | ".save" | ".width" | ".meas" | ".measure" | ".nodeset" | ".title" => {
                self.warn(line, format!("{cmd} is ignored: every node voltage and branch current is always returned"));
                Ok(())
            }
            ".include" | ".inc" | ".lib" | ".library" => err(
                line,
                format!("{cmd} is not supported: a netlist never reads other files. Paste the contents into this file"),
            ),
            ".global" | ".ic" | ".noise" | ".step" | ".tf" | ".four" | ".sens" | ".pz" | ".disto" | ".func" | ".if" | ".control" => err(
                line,
                format!("{cmd} is not supported by this version"),
            ),
            _ => err(line, format!("unknown command `{}`", toks[0])),
        }
    }
}

/// Read a SPICE netlist.
///
/// # Errors
/// A [`ParseError`] with the line number and a message that says what to change.
pub fn parse(text: &str) -> Result<Parsed, ParseError> {
    if text.len() > MAX_INPUT_BYTES {
        return err(
            0,
            format!(
                "the input is {} bytes; the limit is {MAX_INPUT_BYTES}",
                text.len()
            ),
        );
    }
    if text.contains('\0') {
        return err(0, "the input contains a NUL byte; it is not a text netlist");
    }
    let (title, lines) = logical_lines(text)?;
    let mut r = Reader {
        nl: Netlist::new(&title),
        warnings: Vec::new(),
        params: BTreeMap::new(),
        models: HashMap::new(),
        element_lines: HashMap::new(),
        current: None,
        ended: false,
    };
    // Pre-scan: model kinds (for the optional substrate field of Q cards).
    for l in &lines {
        let t: Vec<&str> = l.text.split_whitespace().collect();
        if t.len() >= 3 && t[0].eq_ignore_ascii_case(".model") {
            let ty: String = t[2]
                .chars()
                .take_while(|c| c.is_ascii_alphabetic())
                .collect();
            if let Some(k) = ModelKind::from_keyword(&ty) {
                r.models.insert(t[1].to_uppercase(), k);
            }
        }
    }
    for l in &lines {
        if r.ended {
            break;
        }
        let is_param = l
            .text
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case(".param"));
        // `.param` lines evaluate their own braces, in order; everything else is substituted first.
        let text = if is_param {
            l.text.clone()
        } else {
            substitute(l.line, &l.text, &r.params)?
        };
        let toks = tokens(&text);
        let Some(first) = toks.first() else { continue };
        if first.starts_with('.') {
            r.dot(l.line, &text, &toks)?;
        } else {
            r.element(l.line, &toks)?;
        }
    }
    if let Some((s, start)) = r.current {
        return err(
            start,
            format!(".subckt {} is never closed with .ends", s.name),
        );
    }
    if let Err(e) = r.nl.validate() {
        let line = element_of(&e)
            .and_then(|n| r.element_lines.get(&n.to_uppercase()))
            .copied()
            .unwrap_or(0);
        return Err(ParseError {
            line,
            message: e.to_string(),
        });
    }
    Ok(Parsed {
        netlist: r.nl,
        warnings: r.warnings,
    })
}

fn element_of(e: &NetlistError) -> Option<&str> {
    match e {
        NetlistError::UnknownModel { element, .. }
        | NetlistError::ModelKind { element, .. }
        | NetlistError::UnknownControl { element, .. }
        | NetlistError::WrongControlKind { element, .. }
        | NetlistError::UnknownSubckt { element, .. }
        | NetlistError::SubcktPorts { element, .. }
        | NetlistError::BadValue { element, .. } => Some(element),
        NetlistError::DuplicateElement(n) => Some(n),
        _ => None,
    }
}

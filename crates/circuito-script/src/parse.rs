// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading the text form.
//!
//! ```text
//! title RC low-pass filter
//!
//! V1  SIN(0 1 1k)  in gnd        # value, then one net per pin
//! R1  1k           in out
//! C1  100n         out gnd
//! Q1:PNP  c=out b=in e=vcc       # name the pins instead
//! U1:OPAMP  +=in -=fb out=y
//!
//! model QFAST npn bf=200 is=1e-14
//! op
//! tran 10u 5m
//! ```
//!
//! A line is a part, `title`, `model` or an analysis (`op`, `dc`, `ac`, `tran`). Comments start
//! with `#` or `//`. Net names are anything without spaces; `gnd` and `0` are ground; `_` marks a
//! pin as deliberately unconnected and `?` as open. After `@x,y` a part line may say where the
//! part sits in the drawing, then `r90`, `r180`, `r270` and `mirror`.

use crate::design::{Design, PartSpec, PinNet, Place};
use crate::problem::{Diagnostic, Problem};
use circuito_core::library::{Library, PartDef};
use circuito_core::netlist::{Analysis, Model, ModelKind, Sweep};
use circuito_core::schematic::{Point, Rotation};
use circuito_core::source::Source;
use circuito_core::value::parse_value;
use std::collections::BTreeMap;

/// Longest text accepted, in bytes.
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
/// Most lines read.
pub const MAX_LINES: usize = 50_000;
/// Longest line read, in characters.
pub const MAX_LINE_CHARS: usize = 4096;
/// Most parts in one design.
pub const MAX_PARTS: usize = 5000;
/// Largest absolute `@x,y` coordinate.
pub const MAX_COORD: i32 = 100_000;

/// The result of reading a text: the design from every line that was understood, and what was
/// wrong with the others. A text with problems should not replace a working schematic.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    /// What was understood.
    pub design: Design,
    /// What was not.
    pub problems: Vec<Diagnostic>,
}

impl Parsed {
    /// Whether every line was understood.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty()
    }
}

/// The part kind a reference designator suggests: `R1` is a resistor, `Q2` a transistor, `LED3`
/// an LED. `None` when the designator says nothing.
#[must_use]
pub fn infer_lib(refdes: &str) -> Option<&'static str> {
    const KINDS: &[(&str, &str)] = &[
        ("LED", "LED"),
        ("RV", "POT"),
        ("DZ", "ZENER"),
        ("R", "R"),
        ("C", "C"),
        ("L", "L"),
        ("V", "V"),
        ("I", "I"),
        ("D", "D"),
        ("Z", "ZENER"),
        ("Q", "NPN"),
        ("M", "NMOS"),
        ("U", "OPAMP"),
        ("S", "SWITCH"),
        ("T", "TRANSFORMER"),
        ("F", "FUSE"),
    ];
    let up = refdes.to_ascii_uppercase();
    KINDS
        .iter()
        .filter(|(k, _)| up.starts_with(k))
        .max_by_key(|(k, _)| k.len())
        .map(|(_, lib)| *lib)
}

/// Whether a part of this kind takes a value.
#[must_use]
pub fn takes_value(def: &PartDef) -> bool {
    !def.default_value.is_empty()
}

/// The fields a part line may set with `name=value`.
pub const FIELDS: &[&str] = &["ic", "k", "ratio", "rseries", "wiper"];

struct Tok {
    text: String,
    start: usize,
    end: usize,
}

/// Split a line into words. Parentheses and double quotes group; `#` and `//` start a comment.
fn tokenize(line: &[char]) -> Result<Vec<Tok>, (usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '#' || (c == '/' && line.get(i + 1) == Some(&'/')) {
            break;
        }
        let start = i;
        let mut text = String::new();
        let mut depth = 0usize;
        let mut quoted = false;
        while i < line.len() {
            let c = line[i];
            if quoted {
                if c == '"' {
                    quoted = false;
                } else {
                    text.push(c);
                }
            } else if c == '"' {
                quoted = true;
            } else if c == '(' {
                depth += 1;
                text.push(c);
            } else if c == ')' {
                depth = depth.saturating_sub(1);
                text.push(c);
            } else if depth == 0
                && (c.is_whitespace() || c == '#' || (c == '/' && line.get(i + 1) == Some(&'/')))
            {
                break;
            } else {
                text.push(c);
            }
            i += 1;
        }
        if quoted || depth > 0 {
            return Err((start, i));
        }
        out.push(Tok {
            text,
            start,
            end: i,
        });
    }
    Ok(out)
}

fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn pin_list(def: &PartDef) -> String {
    def.pins
        .iter()
        .map(|p| {
            if p.name == p.number || p.name == "~" {
                p.number.to_string()
            } else {
                format!("{} ({})", p.name, p.number)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn find_pin(def: &PartDef, key: &str) -> Option<usize> {
    def.pins
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(key))
        .or_else(|| {
            def.pins
                .iter()
                .position(|p| p.number.eq_ignore_ascii_case(key))
        })
}

/// Read a text into a design and a list of problems. Never fails and never panics: whatever
/// cannot be understood becomes a [`Diagnostic`] with its line and columns.
#[must_use]
pub fn parse(text: &str) -> Parsed {
    let mut out = Parsed::default();
    if text.len() > MAX_TEXT_BYTES {
        out.problems.push(Diagnostic {
            line: 1,
            start: 0,
            end: 0,
            problem: Problem::TooMuch,
        });
        return out;
    }
    let lib = Library::builtin();
    for (n, raw) in text.lines().enumerate() {
        let line_no = n + 1;
        if line_no > MAX_LINES {
            push(&mut out, line_no, 0, 0, Problem::TooMuch);
            break;
        }
        let chars: Vec<char> = raw.chars().collect();
        if chars.len() > MAX_LINE_CHARS {
            push(&mut out, line_no, 0, MAX_LINE_CHARS, Problem::TooMuch);
            continue;
        }
        let trimmed = raw.trim_start();
        if let Some(rest) = keyword_rest(trimmed, "title") {
            out.design.title = rest.trim().to_string();
            continue;
        }
        let toks = match tokenize(&chars) {
            Ok(t) => t,
            Err((s, e)) => {
                push(&mut out, line_no, s, e, Problem::Unbalanced);
                continue;
            }
        };
        let Some(first) = toks.first() else {
            continue;
        };
        let kw = first.text.trim_start_matches('.').to_ascii_lowercase();
        match kw.as_str() {
            "model" => parse_model(&mut out, line_no, &toks),
            "op" | "dc" | "ac" | "tran" if !first.text.is_empty() => {
                parse_analysis(&mut out, line_no, &kw, &toks);
            }
            _ => parse_part(&mut out, lib, line_no, &toks),
        }
    }
    out
}

fn push(out: &mut Parsed, line: usize, start: usize, end: usize, problem: Problem) {
    out.problems.push(Diagnostic {
        line,
        start,
        end,
        problem,
    });
}

fn at(out: &mut Parsed, line: usize, t: &Tok, problem: Problem) {
    push(out, line, t.start, t.end, problem);
}

/// The rest of a line after a leading keyword that is followed by a space or the end.
fn keyword_rest<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let head = line.get(..keyword.len())?;
    if !head.eq_ignore_ascii_case(keyword) {
        return None;
    }
    let rest = line.get(keyword.len()..)?;
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest)
    } else {
        None
    }
}

fn num(out: &mut Parsed, line: usize, t: &Tok) -> Option<f64> {
    match parse_value(&t.text) {
        Ok(v) => Some(v),
        Err(_) => {
            at(
                out,
                line,
                t,
                Problem::BadNumber {
                    text: t.text.clone(),
                },
            );
            None
        }
    }
}

fn parse_model(out: &mut Parsed, line: usize, toks: &[Tok]) {
    let (Some(name), Some(kind)) = (toks.get(1), toks.get(2)) else {
        let end = toks.last().map_or(0, |t| t.end);
        push(out, line, 0, end, Problem::ModelUsage);
        return;
    };
    let Some(kind_v) = ModelKind::from_keyword(&kind.text) else {
        at(
            out,
            line,
            kind,
            Problem::UnknownModelKind {
                kind: kind.text.clone(),
            },
        );
        return;
    };
    let mut model = Model::new(&name.text, kind_v);
    let mut ok = true;
    for t in toks.iter().skip(3) {
        let (k, v) = match t.text.split_once('=') {
            Some((k, v)) if !k.is_empty() => (k, v),
            _ => {
                at(out, line, t, Problem::ModelUsage);
                ok = false;
                continue;
            }
        };
        match parse_value(v) {
            Ok(x) => {
                model.params.insert(k.to_uppercase(), x);
            }
            Err(_) => {
                at(
                    out,
                    line,
                    t,
                    Problem::BadNumber {
                        text: v.to_string(),
                    },
                );
                ok = false;
            }
        }
    }
    if ok {
        out.design.models.retain(|m| m.name != model.name);
        out.design.models.push(model);
    }
}

fn parse_analysis(out: &mut Parsed, line: usize, kw: &str, toks: &[Tok]) {
    let args = toks.get(1..).unwrap_or(&[]);
    let whole = |out: &mut Parsed, usage: &'static str| {
        let end = toks.last().map_or(0, |t| t.end);
        push(out, line, 0, end, Problem::AnalysisUsage { usage });
    };
    let analysis = match kw {
        "op" => {
            if !args.is_empty() {
                whole(out, "op");
                return;
            }
            Analysis::Op
        }
        "dc" => {
            let usage = "dc SOURCE START STOP STEP";
            let [src, a, b, c] = args else {
                whole(out, usage);
                return;
            };
            let (Some(start), Some(stop), Some(step)) =
                (num(out, line, a), num(out, line, b), num(out, line, c))
            else {
                return;
            };
            Analysis::Dc {
                source: src.text.clone(),
                start,
                stop,
                step,
            }
        }
        "ac" => {
            let usage = "ac dec|oct|lin POINTS FSTART FSTOP";
            let [sw, pts, f0, f1] = args else {
                whole(out, usage);
                return;
            };
            let sweep = match sw.text.to_ascii_lowercase().as_str() {
                "dec" => Sweep::Dec,
                "oct" => Sweep::Oct,
                "lin" => Sweep::Lin,
                _ => {
                    whole(out, usage);
                    return;
                }
            };
            let (Some(p), Some(fstart), Some(fstop)) =
                (num(out, line, pts), num(out, line, f0), num(out, line, f1))
            else {
                return;
            };
            if p < 1.0 || p.fract() != 0.0 || p > f64::from(u32::MAX) {
                at(
                    out,
                    line,
                    pts,
                    Problem::BadNumber {
                        text: pts.text.clone(),
                    },
                );
                return;
            }
            Analysis::Ac {
                sweep,
                points: p as u32,
                fstart,
                fstop,
            }
        }
        _ => {
            let usage = "tran TSTEP TSTOP [TSTART] [TMAX] [uic]";
            let uic = args
                .last()
                .is_some_and(|t| t.text.eq_ignore_ascii_case("uic"));
            let nums = if uic {
                args.get(..args.len() - 1).unwrap_or(&[])
            } else {
                args
            };
            if !(2..=4).contains(&nums.len()) {
                whole(out, usage);
                return;
            }
            let mut v = Vec::new();
            for t in nums {
                let Some(x) = num(out, line, t) else { return };
                v.push(x);
            }
            Analysis::Tran {
                tstep: v[0],
                tstop: v[1],
                tstart: v.get(2).copied().unwrap_or(0.0),
                tmax: v.get(3).copied(),
                uic,
            }
        }
    };
    match analysis.validate() {
        Ok(()) => out.design.analyses.push(analysis),
        Err(e) => {
            let end = toks.last().map_or(0, |t| t.end);
            let why = match e {
                circuito_core::netlist::NetlistError::BadValue { message, .. } => message,
                other => other.to_string(),
            };
            push(out, line, 0, end, Problem::BadAnalysis { why });
        }
    }
}

fn parse_place(out: &mut Parsed, line: usize, toks: &[&Tok]) -> Option<Place> {
    let bad = |out: &mut Parsed, t: &Tok| {
        at(
            out,
            line,
            t,
            Problem::BadLayout {
                text: t.text.clone(),
            },
        );
    };
    let first = toks.first()?;
    let body = first.text.strip_prefix('@')?;
    let Some((xs, ys)) = body.split_once(',') else {
        bad(out, first);
        return None;
    };
    let coord = |s: &str| {
        s.trim()
            .parse::<i32>()
            .ok()
            .filter(|v| v.abs() <= MAX_COORD)
    };
    let (Some(x), Some(y)) = (coord(xs), coord(ys)) else {
        bad(out, first);
        return None;
    };
    let mut place = Place {
        position: Point::new(x, y),
        rotation: Rotation::R0,
        mirror: false,
    };
    let mut ok = true;
    for t in toks.iter().skip(1) {
        match t.text.to_ascii_lowercase().as_str() {
            "r0" => place.rotation = Rotation::R0,
            "r90" => place.rotation = Rotation::R90,
            "r180" => place.rotation = Rotation::R180,
            "r270" => place.rotation = Rotation::R270,
            "mirror" | "m" => place.mirror = true,
            _ => {
                bad(out, t);
                ok = false;
            }
        }
    }
    ok.then_some(place)
}

fn net_token(t: &str) -> PinNet {
    match t {
        "_" => PinNet::NoConnect,
        "?" => PinNet::Open,
        other if crate::design::is_ground(other) => PinNet::Net("gnd".to_string()),
        other => PinNet::Net(other.to_string()),
    }
}

#[allow(clippy::too_many_lines)]
fn parse_part(out: &mut Parsed, lib: &Library, line: usize, toks: &[Tok]) {
    let Some(head) = toks.first() else { return };
    let (refdes, kind) = match head.text.split_once(':') {
        Some((r, k)) => (r, Some(k)),
        None => (head.text.as_str(), None),
    };
    if !valid_name(refdes) {
        at(
            out,
            line,
            head,
            Problem::BadRefdes {
                text: refdes.to_string(),
            },
        );
        return;
    }
    let def = match kind {
        Some(k) => match lib.parts().iter().find(|d| d.id.eq_ignore_ascii_case(k)) {
            Some(d) => d,
            None => {
                at(
                    out,
                    line,
                    head,
                    Problem::UnknownPart {
                        name: k.to_string(),
                    },
                );
                return;
            }
        },
        None => match infer_lib(refdes).and_then(|id| lib.get(id)) {
            Some(d) => d,
            None => {
                at(
                    out,
                    line,
                    head,
                    Problem::UnknownKind {
                        refdes: refdes.to_string(),
                    },
                );
                return;
            }
        },
    };
    if out.design.part_index(refdes).is_some() {
        at(
            out,
            line,
            head,
            Problem::DuplicateRefdes {
                refdes: refdes.to_string(),
            },
        );
        return;
    }
    if out.design.parts.len() >= MAX_PARTS {
        at(out, line, head, Problem::TooMuch);
        return;
    }
    let np = def.pins.len();
    let mut pins: Vec<Option<PinNet>> = vec![None; np];
    let mut value: Option<String> = None;
    let mut model: Option<String> = None;
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    let mut positional: Vec<&Tok> = Vec::new();
    let mut layout: Vec<&Tok> = Vec::new();
    let mut named_pins = false;
    let mut ok = true;
    for t in toks.iter().skip(1) {
        if !layout.is_empty() || t.text.starts_with('@') {
            layout.push(t);
            continue;
        }
        let paren = t.text.find('(').unwrap_or(t.text.len());
        let named = t
            .text
            .find('=')
            .filter(|&eq| eq > 0 && eq < paren)
            .map(|eq| (t.text[..eq].to_lowercase(), &t.text[eq + 1..]));
        let Some((key, val)) = named else {
            positional.push(t);
            continue;
        };
        if key == "model" {
            model = Some(val.to_uppercase());
        } else if key == "value" {
            value = Some(val.to_string());
        } else if let Some(i) = find_pin(def, &key) {
            named_pins = true;
            if pins.get(i).is_some_and(Option::is_some) {
                at(
                    out,
                    line,
                    t,
                    Problem::PinTwice {
                        refdes: refdes.to_string(),
                        key,
                    },
                );
                ok = false;
            } else if let Some(slot) = pins.get_mut(i) {
                *slot = Some(net_token(val));
            }
        } else if FIELDS.contains(&key.as_str()) {
            if let Err(e) = parse_value(val) {
                at(
                    out,
                    line,
                    t,
                    Problem::BadValue {
                        refdes: refdes.to_string(),
                        text: val.to_string(),
                        why: e.to_string(),
                    },
                );
                ok = false;
            }
            fields.insert(key, val.to_string());
        } else {
            at(
                out,
                line,
                t,
                Problem::UnknownPin {
                    refdes: refdes.to_string(),
                    key,
                    pins: pin_list(def),
                },
            );
            ok = false;
        }
    }
    if !ok {
        return;
    }
    let wants_value = takes_value(def);
    let empties: Vec<usize> = pins
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_none())
        .map(|(i, _)| i)
        .collect();
    let count = positional.len();
    let can_value = wants_value && value.is_none();
    // How many leading words are the value (0 or 1) and how many nets follow.
    let plan = if count == empties.len() {
        Some(0)
    } else if can_value && (count == empties.len() + 1 || (named_pins && count == 1)) {
        Some(1)
    } else if named_pins && count == 0 {
        Some(0)
    } else {
        None
    };
    match plan {
        Some(skip) => {
            if skip == 1 {
                if let Some(v) = positional.first() {
                    value = Some(v.text.clone());
                }
            }
            for (i, t) in empties.iter().zip(positional.iter().skip(skip)) {
                if let Some(slot) = pins.get_mut(*i) {
                    *slot = Some(net_token(&t.text));
                }
            }
        }
        None => {
            let end = toks.last().map_or(head.end, |t| t.end);
            let problem = if count == empties.len() + 1 && !wants_value {
                Problem::NoValue {
                    refdes: refdes.to_string(),
                }
            } else {
                Problem::NetCount {
                    refdes: refdes.to_string(),
                    expected: empties.len(),
                    found: count,
                    pins: pin_list(def),
                }
            };
            push(out, line, head.end, end.max(head.end), problem);
            ok = false;
        }
    }
    let value = value.unwrap_or_else(|| def.default_value.to_string());
    if ok && !value.is_empty() {
        if let Some(why) = value_problem(def, &value) {
            let t = toks
                .iter()
                .find(|t| t.text == value || t.text.ends_with(&format!("={value}")));
            let (s, e) = t.map_or((head.start, head.end), |t| (t.start, t.end));
            push(
                out,
                line,
                s,
                e,
                Problem::BadValue {
                    refdes: refdes.to_string(),
                    text: value.clone(),
                    why,
                },
            );
            ok = false;
        }
    }
    let place = if layout.is_empty() {
        None
    } else {
        let p = parse_place(out, line, &layout);
        if p.is_none() {
            ok = false;
        }
        p
    };
    if !ok {
        return;
    }
    out.design.parts.push(PartSpec {
        refdes: refdes.to_string(),
        lib_id: def.id.to_string(),
        value,
        model,
        fields,
        pins: pins
            .into_iter()
            .map(|p| p.unwrap_or(PinNet::Open))
            .collect(),
        place,
        line,
    });
}

/// Why a value is not acceptable for this kind of part, if it is not.
fn value_problem(def: &PartDef, value: &str) -> Option<String> {
    use circuito_core::library::Device;
    match def.device {
        Device::Resistor
        | Device::Capacitor
        | Device::Inductor
        | Device::Potentiometer
        | Device::Transformer => parse_value(value).err().map(|e| e.to_string()),
        Device::VSource | Device::ISource => Source::parse(value).err().map(|e| e.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Design {
        let p = parse(text);
        assert!(p.is_clean(), "{:?}", p.problems);
        p.design
    }

    fn problems(text: &str) -> Vec<Problem> {
        parse(text)
            .problems
            .into_iter()
            .map(|d| d.problem)
            .collect()
    }

    #[test]
    fn a_small_circuit_reads() {
        let d = ok("title RC filter\nV1 SIN(0 1 1k) in gnd\nR1 1k in out  # series\nC1 100n out 0\nop\ntran 10u 5m\n");
        assert_eq!(d.title, "RC filter");
        assert_eq!(d.parts.len(), 3);
        assert_eq!(d.parts[0].lib_id, "V");
        assert_eq!(d.parts[0].value, "SIN(0 1 1k)");
        assert_eq!(
            d.parts[1].pins,
            vec![PinNet::Net("in".into()), PinNet::Net("out".into())]
        );
        assert_eq!(d.analyses.len(), 2);
    }

    #[test]
    fn named_pins_and_models_read() {
        let d = ok("Q1:PNP c=out b=in e=vcc model=QFAST\nmodel qfast pnp bf=200 is=1e-14\nU1:OPAMP +=a -=b out=y\n");
        assert_eq!(d.parts[0].lib_id, "PNP");
        assert_eq!(d.parts[0].model.as_deref(), Some("QFAST"));
        assert_eq!(d.parts[0].pins[0], PinNet::Net("out".into()));
        assert_eq!(d.parts[1].pins[0], PinNet::Net("a".into()));
        assert_eq!(d.models.len(), 1);
        assert_eq!(d.models[0].params.get("BF"), Some(&200.0));
    }

    #[test]
    fn the_value_is_optional() {
        let d = ok("R1 a b\nD1 a b\nD2 a _\n");
        assert_eq!(d.parts[0].value, "1k");
        assert_eq!(d.parts[2].pins[1], PinNet::NoConnect);
    }

    #[test]
    fn placement_hints_read() {
        let d = ok("R1 1k a b @12,-4 r90 mirror\n");
        let p = d.parts[0].place.expect("placed");
        assert_eq!(
            (p.position, p.rotation, p.mirror),
            (Point::new(12, -4), Rotation::R90, true)
        );
        assert!(!problems("R1 1k a b @1;2\n").is_empty());
        assert!(!problems("R1 1k a b @1,2 rr\n").is_empty());
    }

    #[test]
    fn problems_name_the_line_and_columns() {
        let p = parse(
            "R1 1k a b\nR2 a\nR3 1kk a b\nX1 a b\nR1 1k a b\nD1 1k a b\nQ1 q=a\nR4 (1k a b\n",
        );
        let lines: Vec<usize> = p.problems.iter().map(|d| d.line).collect();
        assert_eq!(lines, vec![2, 3, 4, 5, 6, 7, 8]);
        assert!(matches!(
            p.problems[0].problem,
            Problem::NetCount {
                expected: 2,
                found: 1,
                ..
            }
        ));
        assert!(matches!(p.problems[1].problem, Problem::BadValue { .. }));
        assert!(matches!(p.problems[2].problem, Problem::UnknownKind { .. }));
        assert!(matches!(
            p.problems[3].problem,
            Problem::DuplicateRefdes { .. }
        ));
        assert!(matches!(p.problems[4].problem, Problem::NoValue { .. }));
        assert!(matches!(p.problems[5].problem, Problem::UnknownPin { .. }));
        assert!(matches!(p.problems[6].problem, Problem::Unbalanced));
        assert_eq!((p.problems[1].start, p.problems[1].end), (3, 6));
    }

    #[test]
    fn analyses_are_checked() {
        assert!(matches!(
            &problems("tran 0 1\n")[0],
            Problem::BadAnalysis { .. }
        ));
        assert!(matches!(
            &problems("dc V1 0 5\n")[0],
            Problem::AnalysisUsage { .. }
        ));
        assert!(matches!(
            &problems("ac dec 10 1 x\n")[0],
            Problem::BadNumber { .. }
        ));
        let d = ok(".ac dec 10 1 1meg\ndc V1 0 5 0.1\ntran 1u 1m 0 10n uic\n");
        assert_eq!(d.analyses.len(), 3);
    }

    #[test]
    fn kinds_are_inferred_from_the_designator() {
        for (r, lib) in [
            ("R1", "R"),
            ("LED2", "LED"),
            ("RV1", "POT"),
            ("DZ1", "ZENER"),
            ("D3", "D"),
            ("Q1", "NPN"),
            ("R_load", "R"),
        ] {
            assert_eq!(infer_lib(r), Some(lib), "{r}");
        }
        assert_eq!(infer_lib("Rload"), Some("R"));
        assert_eq!(infer_lib("RB"), Some("R"));
        assert_eq!(infer_lib("X1"), None);
    }

    #[test]
    fn hostile_text_does_not_panic() {
        for t in [
            "",
            "\u{0}",
            "((((",
            "\"",
            "R1 =",
            "=",
            "R1 a=b=c d",
            "@@@",
            "model",
            "tran",
            "title",
            &"R1 ".repeat(5000),
        ] {
            let _ = parse(t);
        }
    }
}

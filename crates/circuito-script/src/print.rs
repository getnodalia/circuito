// SPDX-License-Identifier: MIT OR Apache-2.0
//! Writing a design as text. What [`print`] writes, [`crate::parse`] reads back to the same design.

use crate::design::{is_ground, Design, PartSpec, PinNet};
use crate::parse::{infer_lib, takes_value};
use circuito_core::format_value;
use circuito_core::library::{Library, PartDef};
use circuito_core::schematic::Rotation;
use std::fmt::Write;

/// What to write.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrintOptions {
    /// Add `@x,y` placement hints to parts that have a position (for files; the editor's
    /// code pane leaves them out).
    pub layout: bool,
}

/// A name made safe to write as one word.
#[must_use]
pub fn token_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_whitespace() || matches!(c, '#' | '=' | '(' | ')' | '"' | ':') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if s.is_empty() || s == "_" || s == "?" {
        s = "net".to_string();
    }
    if s.starts_with('@') {
        s.insert(0, 'n');
    }
    if s.contains("//") {
        s = s.replace("//", "_");
    }
    s
}

fn quote_if_needed(value: &str) -> String {
    let mut depth = 0usize;
    let mut spaced = false;
    for c in value.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => spaced = true,
            _ => {}
        }
    }
    if spaced
        || value.contains('#')
        || value.starts_with('@')
        || value.contains('=') && !value.contains('(')
    {
        format!("\"{value}\"")
    } else {
        value.to_string()
    }
}

fn net_text(pin: &PinNet) -> String {
    match pin {
        PinNet::Open => "?".to_string(),
        PinNet::NoConnect => "_".to_string(),
        PinNet::Net(n) if is_ground(n) => "gnd".to_string(),
        PinNet::Net(n) => token_name(n),
    }
}

fn pin_key(def: &PartDef, k: usize) -> String {
    def.pins
        .get(k)
        .map(|p| if p.name == "~" { p.number } else { p.name })
        .unwrap_or("?")
        .to_lowercase()
}

fn head(part: &PartSpec) -> String {
    if infer_lib(&part.refdes) == Some(part.lib_id.as_str()) {
        part.refdes.clone()
    } else {
        format!("{}:{}", part.refdes, part.lib_id)
    }
}

/// One part as a line, split into the word for the name and the rest, so that columns line up.
fn part_line(
    part: &PartSpec,
    def: Option<&PartDef>,
    opts: PrintOptions,
) -> (String, String, String) {
    let value = if def.is_some_and(takes_value) && !part.value.is_empty() {
        quote_if_needed(&part.value)
    } else {
        String::new()
    };
    let mut rest: Vec<String> = Vec::new();
    match def {
        Some(def) if part.pins.len() > 2 && part.pins.iter().any(|p| *p != PinNet::Open) => {
            for (k, pin) in part.pins.iter().enumerate() {
                // Open pins are written too: a pin left out could be mistaken for the value's slot.
                rest.push(format!("{}={}", pin_key(def, k), net_text(pin)));
            }
        }
        _ => rest.extend(part.pins.iter().map(net_text)),
    }
    if let Some(m) = &part.model {
        rest.push(format!("model={m}"));
    }
    for (k, v) in &part.fields {
        rest.push(format!("{k}={}", quote_if_needed(v)));
    }
    if opts.layout {
        if let Some(p) = part.place {
            let mut at = format!("@{},{}", p.position.x, p.position.y);
            match p.rotation {
                Rotation::R0 => {}
                Rotation::R90 => at.push_str(" r90"),
                Rotation::R180 => at.push_str(" r180"),
                Rotation::R270 => at.push_str(" r270"),
            }
            if p.mirror {
                at.push_str(" mirror");
            }
            rest.push(at);
        }
    }
    (head(part), value, rest.join(" "))
}

/// Write a design as text.
#[must_use]
pub fn print(design: &Design, opts: PrintOptions) -> String {
    let lib = Library::builtin();
    let mut out = String::new();
    if !design.title.trim().is_empty() {
        let _ = writeln!(out, "title {}", design.title.trim());
        out.push('\n');
    }
    let lines: Vec<(String, String, String)> = design
        .parts
        .iter()
        .map(|p| part_line(p, lib.get(&p.lib_id), opts))
        .collect();
    let hw = lines
        .iter()
        .map(|l| l.0.chars().count())
        .max()
        .unwrap_or(0)
        .max(3);
    let vw = lines.iter().map(|l| l.1.chars().count()).max().unwrap_or(0);
    for (h, v, rest) in &lines {
        let mut line = format!("{h:<hw$}");
        if !v.is_empty() {
            let _ = write!(line, "  {v:<vw$}");
        }
        if !rest.is_empty() {
            let _ = write!(line, "  {rest}");
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if !design.models.is_empty() {
        out.push('\n');
        for m in &design.models {
            let mut line = format!("model {} {}", m.name, m.kind);
            for (k, v) in &m.params {
                let _ = write!(line, " {}={}", k.to_lowercase(), format_value(*v));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    if !design.analyses.is_empty() {
        out.push('\n');
        for a in &design.analyses {
            let text = a.to_string();
            let _ = writeln!(out, "{}", text.strip_prefix('.').unwrap_or(&text));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn round_trip(text: &str) {
        let first = parse(text);
        assert!(first.is_clean(), "{:?}", first.problems);
        for layout in [false, true] {
            let printed = print(&first.design, PrintOptions { layout });
            let again = parse(&printed);
            assert!(again.is_clean(), "{printed}\n{:?}", again.problems);
            let mut a = first.design.clone();
            let mut b = again.design.clone();
            for p in a.parts.iter_mut().chain(b.parts.iter_mut()) {
                p.line = 0;
                if !layout {
                    p.place = None;
                }
            }
            for p in &mut a.parts {
                if !layout {
                    p.place = None;
                }
            }
            assert_eq!(a, b, "{printed}");
            assert_eq!(print(&again.design, PrintOptions { layout }), printed);
        }
    }

    #[test]
    fn printing_and_parsing_agree() {
        round_trip("title RC\nV1 SIN(0 1 1k) in gnd\nR1 1k in out\nC1 100n out 0\nop\ntran 10u 5m\nac dec 10 1 1meg\ndc V1 0 5 0.1\n");
        round_trip("Q1:PNP c=out b=in e=vcc model=QFAST\nU1:OPAMP +=a -=b out=y\nmodel qfast pnp bf=200 is=1e-14\nRV1 10k a b w=c\n");
        round_trip("R1 1k a b @12,-4 r90 mirror\nD1:LED a _\nV2 \"DC 5\" a gnd\nT1 10m a b c d ratio=2 k=0.99\n");
    }

    #[test]
    fn layout_hints_are_optional() {
        let d = parse("R1 1k a b @3,4 r270\n").design;
        assert!(!print(&d, PrintOptions { layout: false }).contains('@'));
        assert!(print(&d, PrintOptions { layout: true }).contains("@3,4 r270"));
    }

    #[test]
    fn awkward_net_names_become_words() {
        assert_eq!(token_name("a b"), "a_b");
        assert_eq!(token_name(""), "net");
        assert_eq!(token_name("@x"), "n@x");
        assert_eq!(token_name("x#y=z"), "x_y_z");
    }
}

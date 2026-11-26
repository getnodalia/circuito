// SPDX-License-Identifier: MIT OR Apache-2.0
//! The low-level Gerber RS-274X/X2 emitter: a list of graphical objects with attributes becomes
//! one deterministic file.
//!
//! Coordinates are written as `FSLAX46Y46` in millimetres: four integer and six decimal digits,
//! leading zeros omitted. One unit of the integer is exactly 1 nm, so board coordinates are
//! written as they are, with no rounding at all. Aperture dimensions are written as millimetres
//! with six decimals, which is also exact.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::error::{PcbError, Result};
use crate::geom::arc::Arc;
use crate::geom::point::Point;
use crate::units::check_coord;

/// Most distinct apertures one file may define.
pub const MAX_APERTURES: usize = 100_000;

/// An aperture: its definition text (`C,0.150000`, `R,1.000000X0.500000`, `RRECT,...`) and the
/// `.AperFunction` it carries, if any. Two apertures with the same shape but different functions
/// are different apertures.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ApKey {
    pub def: String,
    pub func: Option<String>,
}

impl ApKey {
    pub fn new(def: impl Into<String>, func: Option<&str>) -> ApKey {
        ApKey {
            def: def.into(),
            func: func.map(str::to_string),
        }
    }
}

/// Object attributes (`TO.N`, `TO.C`, `TO.P`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ObjAttrs {
    pub net: Option<String>,
    pub comp: Option<String>,
    pub pin: Option<(String, String)>,
}

#[derive(Clone, Debug)]
pub(crate) enum Prim {
    Flash {
        ap: ApKey,
        at: Point,
    },
    Line {
        ap: ApKey,
        a: Point,
        b: Point,
    },
    Arc {
        ap: ApKey,
        arc: Arc,
    },
    /// A filled region: the outer contour drawn dark, each hole cleared.
    Region {
        outer: Vec<Point>,
        holes: Vec<Vec<Point>>,
    },
}

/// What the file says about itself.
#[derive(Clone, Debug)]
pub(crate) struct Header {
    pub file_function: String,
    pub polarity: &'static str,
    pub creation_date: Option<String>,
}

#[derive(Default)]
pub(crate) struct Image {
    objs: Vec<(Prim, ObjAttrs)>,
}

/// Escapes an attribute field: `\`, `%`, `*`, `,` and everything outside printable ASCII become
/// `\uXXXX` (UTF-16 code units, so characters beyond the BMP use a surrogate pair).
pub(crate) fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() && !c.is_ascii_control() && !matches!(c, '\\' | '%' | '*' | ',') {
            out.push(c);
        } else {
            let mut buf = [0u16; 2];
            for u in c.encode_utf16(&mut buf) {
                let _ = write!(out, "\\u{u:04X}");
            }
        }
    }
    out
}

const MACRO_RECTROT: &str = "%AMRECTROT*\n21,1,$1,$2,0,0,$3*%\n";
const MACRO_RRECT: &str = "%AMRRECT*\n21,1,$1,$2-$3-$3,0,0,$4*\n21,1,$1-$3-$3,$2,0,0,$4*\n1,1,$3+$3,$1/2-$3,$2/2-$3,$4*\n1,1,$3+$3,-$1/2+$3,$2/2-$3,$4*\n1,1,$3+$3,-$1/2+$3,-$2/2+$3,$4*\n1,1,$3+$3,$1/2-$3,-$2/2+$3,$4*%\n";

fn macro_text(name: &str) -> Option<&'static str> {
    match name {
        "RECTROT" => Some(MACRO_RECTROT),
        "RRECT" => Some(MACRO_RRECT),
        _ => None,
    }
}

impl Image {
    pub fn new() -> Image {
        Image::default()
    }

    pub fn flash(&mut self, ap: ApKey, at: Point, attrs: ObjAttrs) {
        self.objs.push((Prim::Flash { ap, at }, attrs));
    }

    pub fn line(&mut self, ap: ApKey, a: Point, b: Point, attrs: ObjAttrs) {
        self.objs.push((Prim::Line { ap, a, b }, attrs));
    }

    pub fn arc(&mut self, ap: ApKey, arc: Arc, attrs: ObjAttrs) {
        self.objs.push((Prim::Arc { ap, arc }, attrs));
    }

    pub fn region(&mut self, outer: Vec<Point>, holes: Vec<Vec<Point>>, attrs: ObjAttrs) {
        self.objs.push((Prim::Region { outer, holes }, attrs));
    }

    /// Renders the file. Objects are ordered regions first, then draws, then flashes, each group
    /// by aperture (in aperture order) and otherwise in insertion order.
    pub fn render(self, hdr: &Header) -> Result<String> {
        let mut keys: BTreeSet<ApKey> = BTreeSet::new();
        for (p, _) in &self.objs {
            match p {
                Prim::Flash { ap, .. } | Prim::Line { ap, .. } | Prim::Arc { ap, .. } => {
                    keys.insert(ap.clone());
                }
                Prim::Region { .. } => {}
            }
        }
        if keys.len() > MAX_APERTURES {
            return Err(PcbError::TooLarge {
                what: "Gerber file",
                detail: format!(
                    "{} apertures exceed the limit of {MAX_APERTURES}",
                    keys.len()
                ),
            });
        }
        let codes: BTreeMap<ApKey, u32> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.clone(), 10 + i as u32))
            .collect();
        let code_of = |k: &ApKey| codes.get(k).copied().unwrap_or(0);

        let mut objs = self.objs;
        objs.sort_by_key(|(p, _)| match p {
            Prim::Region { .. } => (0u8, 0u32),
            Prim::Line { ap, .. } | Prim::Arc { ap, .. } => (1, code_of(ap)),
            Prim::Flash { ap, .. } => (2, code_of(ap)),
        });

        let mut out = String::new();
        let ver = env!("CARGO_PKG_VERSION");
        let _ = writeln!(out, "G04 Created by Nodalia Circuito {ver}*");
        let _ = writeln!(out, "%TF.GenerationSoftware,Nodalia,Circuito,{ver}*%");
        if let Some(d) = &hdr.creation_date {
            let _ = writeln!(out, "%TF.CreationDate,{}*%", escape_field(d));
        }
        let _ = writeln!(out, "%TF.SameCoordinates,Original*%");
        let _ = writeln!(out, "%TF.FileFunction,{}*%", hdr.file_function);
        let _ = writeln!(out, "%TF.FilePolarity,{}*%", hdr.polarity);
        let _ = writeln!(out, "%FSLAX46Y46*%");
        let _ = writeln!(
            out,
            "G04 Gerber Fmt 4.6, Leading zero omitted, Abs format (unit mm)*"
        );
        let _ = writeln!(out, "%MOMM*%");
        let _ = writeln!(out, "%LPD*%");

        let mut macros: BTreeSet<&str> = BTreeSet::new();
        for k in keys.iter() {
            let name = k.def.split(',').next().unwrap_or("");
            if macro_text(name).is_some() {
                macros.insert(name);
            }
        }
        for m in macros {
            if let Some(t) = macro_text(m) {
                out.push_str(t);
            }
        }
        for (k, code) in &codes {
            match &k.func {
                Some(f) => {
                    let _ = writeln!(out, "%TA.AperFunction,{f}*%");
                    let _ = writeln!(out, "%ADD{code}{}*%", k.def);
                    let _ = writeln!(out, "%TD.AperFunction*%");
                }
                None => {
                    let _ = writeln!(out, "%ADD{code}{}*%", k.def);
                }
            }
        }
        out.push_str("G75*\n");

        let mut st = State::default();
        for (p, attrs) in &objs {
            write_attrs(&mut out, &mut st, attrs);
            match p {
                Prim::Flash { ap, at } => {
                    select(&mut out, &mut st, code_of(ap));
                    write_xy(&mut out, *at, "D03")?;
                    st.at = Some(*at);
                }
                Prim::Line { ap, a, b } => {
                    select(&mut out, &mut st, code_of(ap));
                    mode(&mut out, &mut st, 1);
                    move_to(&mut out, &mut st, *a)?;
                    write_xy(&mut out, *b, "D01")?;
                    st.at = Some(*b);
                }
                Prim::Arc { ap, arc } => {
                    select(&mut out, &mut st, code_of(ap));
                    mode(&mut out, &mut st, if arc.ccw { 3 } else { 2 });
                    move_to(&mut out, &mut st, arc.start)?;
                    let i = arc.center.x.checked_sub(arc.start.x);
                    let j = arc.center.y.checked_sub(arc.start.y);
                    let (Some(i), Some(j)) = (i, j) else {
                        return Err(PcbError::Overflow("arc centre offset"));
                    };
                    check_coord(arc.end.x, "arc end")?;
                    check_coord(arc.end.y, "arc end")?;
                    let _ = writeln!(out, "X{}Y{}I{}J{}D01*", arc.end.x, arc.end.y, i, j);
                    st.at = Some(arc.end);
                }
                Prim::Region { outer, holes } => {
                    write_contour(&mut out, &mut st, outer)?;
                    if !holes.is_empty() {
                        out.push_str("%LPC*%\n");
                        for h in holes {
                            write_contour(&mut out, &mut st, h)?;
                        }
                        out.push_str("%LPD*%\n");
                    }
                }
            }
        }
        out.push_str("M02*\n");
        Ok(out)
    }
}

#[derive(Default)]
struct State {
    ap: Option<u32>,
    at: Option<Point>,
    mode: Option<u8>,
    attrs: ObjAttrs,
}

fn select(out: &mut String, st: &mut State, code: u32) {
    if st.ap != Some(code) {
        let _ = writeln!(out, "D{code}*");
        st.ap = Some(code);
    }
}

fn mode(out: &mut String, st: &mut State, m: u8) {
    if st.mode != Some(m) {
        let _ = writeln!(out, "G0{m}*");
        st.mode = Some(m);
    }
}

fn write_xy(out: &mut String, p: Point, op: &str) -> Result<()> {
    check_coord(p.x, "Gerber coordinate")?;
    check_coord(p.y, "Gerber coordinate")?;
    let _ = writeln!(out, "X{}Y{}{op}*", p.x, p.y);
    Ok(())
}

fn move_to(out: &mut String, st: &mut State, p: Point) -> Result<()> {
    if st.at != Some(p) {
        write_xy(out, p, "D02")?;
        st.at = Some(p);
    }
    Ok(())
}

fn write_contour(out: &mut String, st: &mut State, ring: &[Point]) -> Result<()> {
    let Some(first) = ring.first() else {
        return Ok(());
    };
    mode(out, st, 1);
    out.push_str("G36*\n");
    write_xy(out, *first, "D02")?;
    for p in ring.iter().skip(1) {
        write_xy(out, *p, "D01")?;
    }
    write_xy(out, *first, "D01")?;
    out.push_str("G37*\n");
    // the spec leaves the current point after G37 open: every later draw starts with a move
    st.at = None;
    Ok(())
}

fn write_attrs(out: &mut String, st: &mut State, a: &ObjAttrs) {
    if st.attrs.net != a.net {
        match &a.net {
            Some(n) => {
                let _ = writeln!(out, "%TO.N,{}*%", escape_field(n));
            }
            None => out.push_str("%TD.N*%\n"),
        }
    }
    if st.attrs.comp != a.comp {
        match &a.comp {
            Some(c) => {
                let _ = writeln!(out, "%TO.C,{}*%", escape_field(c));
            }
            None => out.push_str("%TD.C*%\n"),
        }
    }
    if st.attrs.pin != a.pin {
        match &a.pin {
            Some((r, p)) => {
                let _ = writeln!(out, "%TO.P,{},{}*%", escape_field(r), escape_field(p));
            }
            None => out.push_str("%TD.P*%\n"),
        }
    }
    st.attrs = a.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_escaped() {
        assert_eq!(escape_field("GND"), "GND");
        assert_eq!(
            escape_field("a,b*c%d\\e"),
            "a\\u002Cb\\u002Ac\\u0025d\\u005Ce"
        );
        assert_eq!(escape_field("é"), "\\u00E9");
        assert_eq!(escape_field("\u{1F600}"), "\\uD83D\\uDE00");
    }

    #[test]
    fn output_is_deterministic_and_ordered() {
        let hdr = Header {
            file_function: "Copper,L1,Top".into(),
            polarity: "Positive",
            creation_date: None,
        };
        let build = || {
            let mut im = Image::new();
            let c = ApKey::new("C,0.200000", Some("Conductor"));
            im.line(
                c.clone(),
                Point::new(0, 0),
                Point::new(1_000_000, 0),
                ObjAttrs::default(),
            );
            im.flash(
                ApKey::new("C,1.000000", None),
                Point::new(5, 6),
                ObjAttrs::default(),
            );
            im.region(
                vec![Point::new(0, 0), Point::new(10, 0), Point::new(10, 10)],
                vec![],
                ObjAttrs::default(),
            );
            im.render(&hdr).unwrap()
        };
        let a = build();
        assert_eq!(a, build());
        let r = a.find("G36*").unwrap();
        let d = a.find("X1000000Y0D01*").unwrap();
        let f = a.find("D03*").unwrap();
        assert!(r < d && d < f);
        assert!(a.ends_with("M02*\n"));
        assert!(!a.contains("TF.CreationDate"));
    }
}

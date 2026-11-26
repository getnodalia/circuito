// SPDX-License-Identifier: MIT OR Apache-2.0
//! A small Gerber RS-274X/X2 reader.
//!
//! It exists to prove that the files this crate writes say what the board says, and to read back
//! files of the same dialect: absolute coordinates with leading zeros omitted (`FSLA`), `MOMM` or
//! `MOIN`, `G01`/`G02`/`G03` with `G75`, `G36`/`G37` regions, `LPD`/`LPC`, aperture definitions
//! and macros (kept as text, not evaluated), file, aperture and object attributes.
//! Not supported, and reported as such: `G74` single-quadrant arcs, trailing-zero formats,
//! incremental coordinates, step-and-repeat, block apertures and the mirror, rotate and scale
//! commands.
//!
//! Input is hostile: the size, the number of objects and apertures and every coordinate are
//! bounded, and every error names the line.

use std::sync::Arc as Shared;

use crate::error::{PcbError, Result};
use crate::geom::point::{Point, Rect};
use crate::units::MAX_COORD;

/// Largest file the reader accepts.
pub const MAX_GERBER_BYTES: usize = 256 * 1024 * 1024;
const MAX_OBJECTS: usize = 5_000_000;
const MAX_APERTURES: usize = 100_000;

/// Attributes as `(name, values)` pairs, in the order they were set.
pub type AttrList = Vec<(String, Vec<String>)>;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Polarity {
    Dark,
    Clear,
}

/// A defined aperture. `template` is `C`, `R`, `O`, `P` or a macro name; `params` are the
/// `X`-separated values exactly as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Aperture {
    pub code: u32,
    pub template: String,
    pub params: Vec<String>,
    /// The `.AperFunction` in effect when it was defined, for instance `SMDPad,CuDef`.
    pub function: Option<String>,
}

/// An aperture macro as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MacroDef {
    pub name: String,
    pub body: String,
}

/// The vertices of a region. A closing vertex equal to the first is dropped. Arcs inside a region
/// are recorded by their end points only and flagged in `has_arcs`; this crate never writes them.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GRegion {
    pub contours: Vec<Vec<Point>>,
    pub has_arcs: bool,
}

impl GRegion {
    /// The box around every vertex.
    pub fn bounds(&self) -> Option<Rect> {
        let mut it = self.contours.iter().flatten();
        let first = *it.next()?;
        let mut r = Rect {
            min: first,
            max: first,
        };
        for p in it {
            r.min.x = r.min.x.min(p.x);
            r.min.y = r.min.y.min(p.y);
            r.max.x = r.max.x.max(p.x);
            r.max.y = r.max.y.max(p.y);
        }
        Some(r)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GKind {
    Flash {
        at: Point,
    },
    Draw {
        from: Point,
        to: Point,
    },
    Arc {
        from: Point,
        to: Point,
        center: Point,
        ccw: bool,
    },
    Region(GRegion),
}

/// One graphical object with the state it was created in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GObject {
    pub kind: GKind,
    pub polarity: Polarity,
    /// The aperture code; `None` for regions.
    pub aperture: Option<u32>,
    /// The object attributes in effect (`N`, `C`, `P`), as `(name, values)`.
    pub attrs: Shared<AttrList>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GerberImage {
    pub inch: bool,
    pub int_digits: u8,
    pub dec_digits: u8,
    /// File attributes without the leading `.`: `("FileFunction", ["Copper", "L1", "Top"])`.
    pub file_attrs: AttrList,
    pub macros: Vec<MacroDef>,
    pub apertures: Vec<Aperture>,
    pub objects: Vec<GObject>,
    /// True when the file ended with `M02`.
    pub finished: bool,
}

impl GerberImage {
    pub fn aperture(&self, code: u32) -> Option<&Aperture> {
        self.apertures.iter().find(|a| a.code == code)
    }

    pub fn file_attr(&self, name: &str) -> Option<&[String]> {
        self.file_attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
    }

    /// Flash positions with the aperture's template and parameters, in file order.
    pub fn flashes(&self) -> Vec<(Point, &Aperture)> {
        self.objects
            .iter()
            .filter_map(|o| match (&o.kind, o.aperture) {
                (GKind::Flash { at }, Some(c)) => self.aperture(c).map(|a| (*at, a)),
                _ => None,
            })
            .collect()
    }

    /// Draw segments with the aperture, in file order.
    pub fn draws(&self) -> Vec<(Point, Point, &Aperture)> {
        self.objects
            .iter()
            .filter_map(|o| match (&o.kind, o.aperture) {
                (GKind::Draw { from, to }, Some(c)) => self.aperture(c).map(|a| (*from, *to, a)),
                _ => None,
            })
            .collect()
    }

    pub fn regions(&self) -> Vec<(&GRegion, Polarity)> {
        self.objects
            .iter()
            .filter_map(|o| match &o.kind {
                GKind::Region(r) => Some((r, o.polarity)),
                _ => None,
            })
            .collect()
    }
}

fn err(line: usize, detail: impl Into<String>) -> PcbError {
    PcbError::Parse {
        what: "Gerber file",
        detail: format!("line {line}: {}", detail.into()),
    }
}

/// Reads `\uXXXX` escapes of an attribute value.
pub fn unescape_field(s: &str) -> String {
    let mut out = String::new();
    let mut units: Vec<u16> = Vec::new();
    let mut rest = s;
    let flush = |units: &mut Vec<u16>, out: &mut String| {
        if !units.is_empty() {
            out.extend(char::decode_utf16(units.drain(..)).map(|r| r.unwrap_or('\u{FFFD}')));
        }
    };
    while !rest.is_empty() {
        if let Some(h) = rest.strip_prefix("\\u") {
            if let Some(code) = h.get(..4).and_then(|x| u16::from_str_radix(x, 16).ok()) {
                units.push(code);
                rest = h.get(4..).unwrap_or("");
                continue;
            }
        }
        flush(&mut units, &mut out);
        let mut cs = rest.chars();
        if let Some(c) = cs.next() {
            out.push(c);
        }
        rest = cs.as_str();
    }
    flush(&mut units, &mut out);
    out
}

struct Parser {
    inch: bool,
    int_digits: u8,
    dec_digits: u8,
    have_format: bool,
    file_attrs: AttrList,
    ap_attrs: AttrList,
    obj_attrs: AttrList,
    obj_attrs_shared: Option<Shared<AttrList>>,
    macros: Vec<MacroDef>,
    apertures: Vec<Aperture>,
    objects: Vec<GObject>,
    finished: bool,
    dark: bool,
    mode: u8,
    cur_ap: Option<u32>,
    at: Point,
    in_region: bool,
    region: GRegion,
    contour: Vec<Point>,
}

fn int_of(s: &str, line: usize, what: &str) -> Result<i64> {
    if s.is_empty() || s.len() > 15 {
        return Err(err(line, format!("`{s}` is not a usable {what}")));
    }
    s.parse::<i64>()
        .map_err(|_| err(line, format!("`{s}` is not a number ({what})")))
}

impl Parser {
    fn coord(&self, v: i64, line: usize) -> Result<i64> {
        let d = u32::from(self.dec_digits);
        let nm = if self.inch {
            // value is in 10^-d inch; 1 inch = 25,400,000 nm
            let num = (v as i128) * 25_400_000;
            let den = 10i128.pow(d);
            let q = num / den;
            let r = num % den;
            let adj = if r.abs() * 2 >= den { r.signum() } else { 0 };
            i64::try_from(q + adj).map_err(|_| err(line, "coordinate out of range"))?
        } else {
            let scale = 10i64.pow(6 - d);
            v.checked_mul(scale)
                .ok_or_else(|| err(line, "coordinate out of range"))?
        };
        if nm.abs() > MAX_COORD {
            return Err(err(
                line,
                format!("coordinate {nm} nm is outside the supported range"),
            ));
        }
        Ok(nm)
    }

    fn attrs_now(&mut self) -> Shared<AttrList> {
        if let Some(s) = &self.obj_attrs_shared {
            return s.clone();
        }
        let s = Shared::new(self.obj_attrs.clone());
        self.obj_attrs_shared = Some(s.clone());
        s
    }

    fn push(&mut self, kind: GKind, ap: Option<u32>, line: usize) -> Result<()> {
        if self.objects.len() >= MAX_OBJECTS {
            return Err(err(line, format!("more than {MAX_OBJECTS} objects")));
        }
        let attrs = self.attrs_now();
        self.objects.push(GObject {
            kind,
            polarity: if self.dark {
                Polarity::Dark
            } else {
                Polarity::Clear
            },
            aperture: ap,
            attrs,
        });
        Ok(())
    }

    fn set_attr(list: &mut AttrList, name: &str, vals: Vec<String>) {
        if let Some(e) = list.iter_mut().find(|(n, _)| n == name) {
            e.1 = vals;
        } else {
            list.push((name.to_string(), vals));
        }
    }

    fn attr_command(&mut self, kind: &str, rest: &str, line: usize) -> Result<()> {
        // rest: ".Name,v1,v2" or "" (TD alone) or ".Name" (TD)
        let rest = rest.trim();
        if kind == "TD" {
            if rest.is_empty() {
                self.ap_attrs.clear();
                self.obj_attrs.clear();
            } else {
                let name = rest.trim_start_matches('.');
                self.ap_attrs.retain(|(n, _)| n != name);
                self.obj_attrs.retain(|(n, _)| n != name);
            }
            self.obj_attrs_shared = None;
            return Ok(());
        }
        let body = rest.strip_prefix('.').ok_or_else(|| {
            err(
                line,
                format!("{kind} needs a standard attribute name starting with a dot"),
            )
        })?;
        let mut parts = body.split(',');
        let name = parts.next().unwrap_or("").to_string();
        if name.is_empty() {
            return Err(err(line, format!("{kind} without a name")));
        }
        let vals: Vec<String> = parts.map(unescape_field).collect();
        match kind {
            "TF" => Self::set_attr(&mut self.file_attrs, &name, vals),
            "TA" => Self::set_attr(&mut self.ap_attrs, &name, vals),
            _ => {
                Self::set_attr(&mut self.obj_attrs, &name, vals);
                self.obj_attrs_shared = None;
            }
        }
        Ok(())
    }

    fn extended(&mut self, body: &str, line: usize) -> Result<()> {
        let body = body.trim();
        let body = body.strip_suffix('*').unwrap_or(body);
        let mut parts = body.split('*');
        let head = parts.next().unwrap_or("").trim();
        if let Some(r) = head.strip_prefix("FS") {
            let r = r.strip_prefix("LA").ok_or_else(|| {
                err(
                    line,
                    "only absolute coordinates with leading zeros omitted (FSLA) are supported",
                )
            })?;
            let r = r
                .strip_prefix('X')
                .ok_or_else(|| err(line, "malformed FS"))?;
            let (xf, yf) = r.split_once('Y').ok_or_else(|| err(line, "malformed FS"))?;
            if xf != yf || xf.len() != 2 {
                return Err(err(
                    line,
                    "the X and Y formats must be equal two-digit values such as 46",
                ));
            }
            let mut d = xf.bytes().map(|b| b.wrapping_sub(b'0'));
            let (i, f) = (d.next().unwrap_or(99), d.next().unwrap_or(99));
            if i > 9 || !(1..=6).contains(&f) {
                return Err(err(line, "coordinate format needs 1..=6 decimals"));
            }
            self.int_digits = i;
            self.dec_digits = f;
            self.have_format = true;
        } else if let Some(r) = head.strip_prefix("MO") {
            match r {
                "MM" => self.inch = false,
                "IN" => self.inch = true,
                _ => return Err(err(line, format!("unknown unit `{r}`"))),
            }
        } else if let Some(r) = head.strip_prefix("LP") {
            match r {
                "D" => self.dark = true,
                "C" => self.dark = false,
                _ => return Err(err(line, format!("unknown polarity `{r}`"))),
            }
        } else if let Some(r) = head.strip_prefix("TF") {
            self.attr_command("TF", r, line)?;
        } else if let Some(r) = head.strip_prefix("TA") {
            self.attr_command("TA", r, line)?;
        } else if let Some(r) = head.strip_prefix("TO") {
            self.attr_command("TO", r, line)?;
        } else if let Some(r) = head.strip_prefix("TD") {
            self.attr_command("TD", r, line)?;
        } else if let Some(r) = head.strip_prefix("AD") {
            self.aperture_def(r, line)?;
        } else if let Some(r) = head.strip_prefix("AM") {
            let mut body = String::new();
            for p in parts {
                let p = p.trim();
                if !p.is_empty() {
                    body.push_str(p);
                    body.push('*');
                }
            }
            if r.is_empty() {
                return Err(err(line, "AM without a name"));
            }
            if self.macros.len() >= MAX_APERTURES {
                return Err(err(line, "too many macros"));
            }
            self.macros.push(MacroDef {
                name: r.to_string(),
                body,
            });
        } else if head.starts_with("IP") {
            if head != "IPPOS" {
                return Err(err(
                    line,
                    "negative image polarity (IPNEG) is not supported",
                ));
            }
        } else {
            let known = ["SR", "LM", "LR", "LS", "AB"];
            if known.iter().any(|k| head.starts_with(k)) {
                return Err(err(
                    line,
                    format!("`{head}` is not supported by this reader"),
                ));
            }
            return Err(err(line, format!("unknown extended command `{head}`")));
        }
        Ok(())
    }

    fn aperture_def(&mut self, r: &str, line: usize) -> Result<()> {
        let r = r
            .strip_prefix('D')
            .ok_or_else(|| err(line, "AD needs a D code"))?;
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        let code: u32 = r
            .get(..digits)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| err(line, "bad aperture code"))?;
        if code < 10 {
            return Err(err(line, "aperture codes start at D10"));
        }
        let rest = r.get(digits..).unwrap_or("");
        let (template, params) = match rest.split_once(',') {
            Some((t, p)) => (t, p.split('X').map(str::to_string).collect()),
            None => (rest, Vec::new()),
        };
        if template.is_empty() {
            return Err(err(line, "aperture without a template"));
        }
        if self.apertures.iter().any(|a| a.code == code) {
            return Err(err(line, format!("aperture D{code} is defined twice")));
        }
        if self.apertures.len() >= MAX_APERTURES {
            return Err(err(line, "too many apertures"));
        }
        let function = self
            .ap_attrs
            .iter()
            .find(|(n, _)| n == "AperFunction")
            .map(|(_, v)| v.join(","));
        self.apertures.push(Aperture {
            code,
            template: template.to_string(),
            params,
            function,
        });
        Ok(())
    }

    fn word_block(&mut self, block: &str, line: usize) -> Result<()> {
        let block = block.trim();
        if block.is_empty() || block.starts_with("G04") {
            return Ok(());
        }
        let b = block.as_bytes();
        let mut i = 0;
        let (mut x, mut y, mut ii, mut jj): (Option<i64>, Option<i64>, Option<i64>, Option<i64>) =
            (None, None, None, None);
        let mut d: Option<i64> = None;
        let mut gs: Vec<i64> = Vec::new();
        let mut m: Option<i64> = None;
        while i < b.len() {
            let c = b.get(i).copied().unwrap_or(b' ');
            i += 1;
            if c.is_ascii_whitespace() {
                continue;
            }
            let start = i;
            if matches!(b.get(i), Some(b'-' | b'+')) {
                i += 1;
            }
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
            let num = block.get(start..i).unwrap_or("");
            let num = num.strip_prefix('+').unwrap_or(num);
            let v = int_of(num, line, "number")?;
            match c {
                b'G' => gs.push(v),
                b'D' => d = Some(v),
                b'M' => m = Some(v),
                b'X' => x = Some(v),
                b'Y' => y = Some(v),
                b'I' => ii = Some(v),
                b'J' => jj = Some(v),
                other => {
                    return Err(err(
                        line,
                        format!("unexpected `{}` in `{block}`", other as char),
                    ))
                }
            }
        }
        for g in gs {
            match g {
                1 => self.mode = 1,
                2 => self.mode = 2,
                3 => self.mode = 3,
                36 => {
                    self.in_region = true;
                    self.region = GRegion::default();
                    self.contour.clear();
                }
                37 => self.end_region(line)?,
                75 | 54 | 55 => {}
                74 => return Err(err(line, "G74 single-quadrant arcs are not supported")),
                70 | 71 | 90 | 91 => {
                    return Err(err(
                        line,
                        format!("deprecated command G{g} is not supported"),
                    ))
                }
                other => return Err(err(line, format!("unsupported G{other}"))),
            }
        }
        if let Some(mc) = m {
            match mc {
                2 => self.finished = true,
                0 | 1 => {}
                other => return Err(err(line, format!("unsupported M{other:02}"))),
            }
        }
        if !self.have_format && (x.is_some() || y.is_some()) {
            return Err(err(line, "coordinates before the FS format statement"));
        }
        let to = Point::new(
            match x {
                Some(v) => self.coord(v, line)?,
                None => self.at.x,
            },
            match y {
                Some(v) => self.coord(v, line)?,
                None => self.at.y,
            },
        );
        match d {
            Some(n) if n >= 10 => {
                let code = u32::try_from(n).map_err(|_| err(line, "bad aperture code"))?;
                if !self.apertures.iter().any(|a| a.code == code) {
                    return Err(err(
                        line,
                        format!("aperture D{code} is used before it is defined"),
                    ));
                }
                self.cur_ap = Some(code);
            }
            Some(1) => self.interpolate(to, ii, jj, line)?,
            Some(2) => {
                if self.in_region {
                    self.close_contour();
                    self.contour.push(to);
                }
                self.at = to;
            }
            Some(3) => {
                if self.in_region {
                    return Err(err(line, "a flash inside a region"));
                }
                let ap = self
                    .cur_ap
                    .ok_or_else(|| err(line, "flash with no aperture selected"))?;
                self.push(GKind::Flash { at: to }, Some(ap), line)?;
                self.at = to;
            }
            Some(other) => return Err(err(line, format!("unsupported D{other:02}"))),
            None => {
                if x.is_some() || y.is_some() {
                    // coordinates without an operation: the deprecated modal D code
                    return Err(err(
                        line,
                        "coordinates without a D operation are not supported",
                    ));
                }
            }
        }
        Ok(())
    }

    fn interpolate(
        &mut self,
        to: Point,
        i: Option<i64>,
        j: Option<i64>,
        line: usize,
    ) -> Result<()> {
        let from = self.at;
        if self.in_region {
            if self.mode != 1 {
                self.region.has_arcs = true;
            }
            if self.contour.is_empty() {
                self.contour.push(from);
            }
            self.contour.push(to);
            self.at = to;
            return Ok(());
        }
        let ap = self
            .cur_ap
            .ok_or_else(|| err(line, "draw with no aperture selected"))?;
        if self.mode == 1 {
            self.push(GKind::Draw { from, to }, Some(ap), line)?;
        } else {
            let (Some(i), Some(j)) = (i, j) else {
                return Err(err(line, "an arc needs I and J"));
            };
            let center = Point::new(
                from.x
                    .checked_add(self.coord(i, line)?)
                    .ok_or_else(|| err(line, "arc centre out of range"))?,
                from.y
                    .checked_add(self.coord(j, line)?)
                    .ok_or_else(|| err(line, "arc centre out of range"))?,
            );
            self.push(
                GKind::Arc {
                    from,
                    to,
                    center,
                    ccw: self.mode == 3,
                },
                Some(ap),
                line,
            )?;
        }
        self.at = to;
        Ok(())
    }

    fn close_contour(&mut self) {
        let mut c = std::mem::take(&mut self.contour);
        if c.len() > 1 && c.first() == c.last() {
            c.pop();
        }
        if !c.is_empty() {
            self.region.contours.push(c);
        }
    }

    fn end_region(&mut self, line: usize) -> Result<()> {
        if !self.in_region {
            return Err(err(line, "G37 without G36"));
        }
        self.close_contour();
        self.in_region = false;
        let r = std::mem::take(&mut self.region);
        self.push(GKind::Region(r), None, line)
    }
}

/// Reads a Gerber file.
pub fn read_gerber(text: &str) -> Result<GerberImage> {
    if text.len() > MAX_GERBER_BYTES {
        return Err(PcbError::TooLarge {
            what: "Gerber file",
            detail: format!(
                "{} bytes exceed the limit of {MAX_GERBER_BYTES}",
                text.len()
            ),
        });
    }
    let mut p = Parser {
        inch: false,
        int_digits: 4,
        dec_digits: 6,
        have_format: false,
        file_attrs: Vec::new(),
        ap_attrs: Vec::new(),
        obj_attrs: Vec::new(),
        obj_attrs_shared: None,
        macros: Vec::new(),
        apertures: Vec::new(),
        objects: Vec::new(),
        finished: false,
        dark: true,
        mode: 1,
        cur_ap: None,
        at: Point::new(0, 0),
        in_region: false,
        region: GRegion::default(),
        contour: Vec::new(),
    };
    let mut line = 1usize;
    let mut i = 0usize;
    while i < text.len() {
        let rest = text.get(i..).unwrap_or("");
        let Some(c) = rest.chars().next() else { break };
        if c.is_whitespace() {
            if c == '\n' {
                line += 1;
            }
            i += c.len_utf8();
            continue;
        }
        let (body, consumed, extended) = if c == '%' {
            let inner = rest.get(1..).unwrap_or("");
            let end = inner
                .find('%')
                .ok_or_else(|| err(line, "extended command without a closing %"))?;
            (inner.get(..end).unwrap_or(""), end + 2, true)
        } else {
            let end = rest
                .find('*')
                .ok_or_else(|| err(line, "command without a closing *"))?;
            (rest.get(..end).unwrap_or(""), end + 1, false)
        };
        let nl = body.bytes().filter(|b| *b == b'\n').count();
        if extended {
            let flat: String = body.chars().filter(|c| *c != '\n' && *c != '\r').collect();
            p.extended(&flat, line)?;
        } else {
            p.word_block(body, line)?;
        }
        line += nl;
        i += consumed;
    }
    if p.in_region {
        return Err(err(line, "the file ends inside a region (G36 without G37)"));
    }
    Ok(GerberImage {
        inch: p.inch,
        int_digits: p.int_digits,
        dec_digits: p.dec_digits,
        file_attrs: p.file_attrs,
        macros: p.macros,
        apertures: p.apertures,
        objects: p.objects,
        finished: p.finished,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_small_file() {
        let t = "%TF.FileFunction,Copper,L1,Top*%\n%FSLAX46Y46*%\n%MOMM*%\n%LPD*%\n%TA.AperFunction,SMDPad,CuDef*%\n%ADD10R,1.000000X0.500000*%\n%TD.AperFunction*%\nD10*\nX1000000Y-2000000D03*\nG01*\nX0Y0D02*\nX5D01*\nM02*\n";
        let g = read_gerber(t).unwrap();
        assert!(g.finished);
        assert_eq!(
            g.file_attr("FileFunction").unwrap(),
            ["Copper", "L1", "Top"]
        );
        let a = g.aperture(10).unwrap();
        assert_eq!(a.template, "R");
        assert_eq!(a.params, ["1.000000", "0.500000"]);
        assert_eq!(a.function.as_deref(), Some("SMDPad,CuDef"));
        assert_eq!(g.flashes()[0].0, Point::new(1_000_000, -2_000_000));
        assert_eq!(g.draws()[0].0, Point::new(0, 0));
        assert_eq!(g.draws()[0].1, Point::new(5, 0));
    }

    #[test]
    fn hostile_input_is_an_error() {
        for t in [
            "X1Y1D03*",
            "%FSLAX46Y46*%\nD10*\n",
            "%FSLAX46Y46*%\nX99999999999999D03*\n",
            "%FSLAX46Y46*%\n%ADD10C,1*%\nD10*\nG74*\n",
            "%FSLAX46Y46*%\nG36*\nM02*\n",
            "%MOMM",
            "%FSLTX46Y46*%",
            "%FSLAX46Y46*%\n%ADD10C,1*%\nD10*\nX1Y1D03*\nQ5*\n",
            "G37*",
        ] {
            assert!(read_gerber(t).is_err(), "{t:?}");
        }
    }

    #[test]
    fn unescapes_fields() {
        assert_eq!(unescape_field("a\\u002Cb"), "a,b");
        assert_eq!(unescape_field("\\uD83D\\uDE00"), "\u{1F600}");
        assert_eq!(unescape_field("\\uZZ"), "\\uZZ");
    }
}

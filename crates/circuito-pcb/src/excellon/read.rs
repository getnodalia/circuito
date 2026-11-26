// SPDX-License-Identifier: MIT OR Apache-2.0
//! A small Excellon reader for the files `write_drills` produces and files of the same dialect:
//! metric, coordinates with an explicit decimal point, `T<n>C<d>` tool definitions, round holes and
//! `G85` slots, X2 attributes in `; #@!` comments. Anything else (integer coordinates without a
//! decimal point, inches, routing) is reported as unsupported.

use crate::error::{PcbError, Result};
use crate::geom::point::Point;
use crate::units::{parse_mm, Nm, MAX_COORD};

/// Largest file the reader accepts.
pub const MAX_EXCELLON_BYTES: usize = 128 * 1024 * 1024;
const MAX_HOLES: usize = 5_000_000;
const MAX_TOOLS: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcTool {
    pub number: u32,
    pub diameter: Nm,
    /// The `AperFunction` attribute in effect when the tool was defined.
    pub function: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcHole {
    pub tool: u32,
    pub at: Point,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcSlot {
    pub tool: u32,
    pub from: Point,
    pub to: Point,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ExcellonImage {
    pub file_attrs: Vec<(String, Vec<String>)>,
    pub tools: Vec<ExcTool>,
    pub holes: Vec<ExcHole>,
    pub slots: Vec<ExcSlot>,
    pub finished: bool,
}

impl ExcellonImage {
    pub fn file_attr(&self, name: &str) -> Option<&[String]> {
        self.file_attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
    }

    pub fn tool(&self, number: u32) -> Option<&ExcTool> {
        self.tools.iter().find(|t| t.number == number)
    }
}

fn err(line: usize, detail: impl Into<String>) -> PcbError {
    PcbError::Parse {
        what: "Excellon file",
        detail: format!("line {line}: {}", detail.into()),
    }
}

fn point(x: &str, y: &str, line: usize) -> Result<Point> {
    for v in [x, y] {
        if !v.contains('.') {
            return Err(err(
                line,
                "coordinates without a decimal point are not supported (the zero-suppression format would have to be guessed)",
            ));
        }
    }
    let conv = |s: &str| -> Result<i64> {
        let v = parse_mm(s).map_err(|e| err(line, e.to_string()))?;
        if v.abs() > MAX_COORD {
            return Err(err(
                line,
                format!("coordinate {v} nm is outside the supported range"),
            ));
        }
        Ok(v)
    };
    Ok(Point::new(conv(x)?, conv(y)?))
}

/// Splits `X1.5Y-2.5` into the two numbers; the rest after the Y value is returned too.
fn split_xy(s: &str, line: usize) -> Result<(&str, &str, &str)> {
    let s = s
        .strip_prefix('X')
        .ok_or_else(|| err(line, format!("expected X.. in `{s}`")))?;
    let (x, r) = s
        .split_once('Y')
        .ok_or_else(|| err(line, "expected Y after X"))?;
    let end = r
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(r.len());
    Ok((x, r.get(..end).unwrap_or(""), r.get(end..).unwrap_or("")))
}

pub fn read_excellon(text: &str) -> Result<ExcellonImage> {
    if text.len() > MAX_EXCELLON_BYTES {
        return Err(PcbError::TooLarge {
            what: "Excellon file",
            detail: format!(
                "{} bytes exceed the limit of {MAX_EXCELLON_BYTES}",
                text.len()
            ),
        });
    }
    let mut img = ExcellonImage::default();
    let mut in_header = false;
    let mut metric = false;
    let mut pending_func: Option<String> = None;
    let mut cur: Option<u32> = None;
    let mut seen_header = false;
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let l = raw.trim();
        if l.is_empty() {
            continue;
        }
        if let Some(c) = l.strip_prefix(';') {
            if let Some(a) = c.trim().strip_prefix("#@!") {
                let a = a.trim();
                let (kind, rest) = a.split_once('.').unwrap_or((a, ""));
                let mut it = rest.split(',');
                let name = it.next().unwrap_or("").to_string();
                let vals: Vec<String> = it.map(str::to_string).collect();
                match kind {
                    "TF" => img.file_attrs.push((name, vals)),
                    "TA" if name == "AperFunction" => pending_func = Some(vals.join(",")),
                    "TD" => pending_func = None,
                    _ => {}
                }
            }
            continue;
        }
        if l == "M48" {
            in_header = true;
            seen_header = true;
            continue;
        }
        if !seen_header {
            return Err(err(line, "the file does not start with M48"));
        }
        if in_header {
            if l == "%" || l == "M95" {
                in_header = false;
            } else if l.starts_with("METRIC") {
                metric = true;
            } else if l.starts_with("INCH") {
                return Err(err(line, "inch files are not supported"));
            } else if l.starts_with("FMAT") {
                // format 2 only
                if l != "FMAT,2" {
                    return Err(err(line, "only FMAT,2 is supported"));
                }
            } else if let Some(t) = l.strip_prefix('T') {
                let (n, d) = t
                    .split_once('C')
                    .ok_or_else(|| err(line, "a tool definition needs C<diameter>"))?;
                let number: u32 = n.parse().map_err(|_| err(line, "bad tool number"))?;
                let diameter = parse_mm(d).map_err(|e| err(line, e.to_string()))?;
                if diameter <= 0 {
                    return Err(err(line, "tool diameter must be positive"));
                }
                if img.tools.len() >= MAX_TOOLS {
                    return Err(err(line, "too many tools"));
                }
                if img.tool(number).is_some() {
                    return Err(err(line, format!("tool T{number} is defined twice")));
                }
                img.tools.push(ExcTool {
                    number,
                    diameter,
                    function: pending_func.clone(),
                });
            }
            continue;
        }
        if !metric {
            return Err(err(line, "the file does not declare METRIC units"));
        }
        match l {
            "G90" | "G05" | "G00" => continue,
            "M30" | "M00" => {
                img.finished = true;
                continue;
            }
            _ => {}
        }
        if let Some(t) = l.strip_prefix('T') {
            let n: u32 = t.parse().map_err(|_| err(line, "bad tool select"))?;
            if n == 0 {
                cur = None;
            } else if img.tool(n).is_some() {
                cur = Some(n);
            } else {
                return Err(err(line, format!("tool T{n} is used before it is defined")));
            }
            continue;
        }
        if l.starts_with('X') {
            let tool = cur.ok_or_else(|| err(line, "a hole before any tool is selected"))?;
            if img.holes.len() + img.slots.len() >= MAX_HOLES {
                return Err(err(line, "too many holes"));
            }
            let (x, y, rest) = split_xy(l, line)?;
            let a = point(x, y, line)?;
            if rest.is_empty() {
                img.holes.push(ExcHole { tool, at: a });
            } else if let Some(r2) = rest.strip_prefix("G85") {
                let (x2, y2, tail) = split_xy(r2, line)?;
                if !tail.is_empty() {
                    return Err(err(line, format!("unexpected `{tail}`")));
                }
                img.slots.push(ExcSlot {
                    tool,
                    from: a,
                    to: point(x2, y2, line)?,
                });
            } else {
                return Err(err(line, format!("unexpected `{rest}`")));
            }
            continue;
        }
        return Err(err(line, format!("unsupported command `{l}`")));
    }
    if !seen_header {
        return Err(err(1, "the file does not start with M48"));
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_holes_and_slots() {
        let t = "M48\n; #@! TF.FileFunction,Plated,1,2,PTH\nMETRIC,TZ\n; #@! TA.AperFunction,Plated,PTH,ViaDrill\nT1C0.300000\n%\nG90\nT1\nX1.500000Y-2.000000\nX0.000000Y0.000000G85X1.000000Y0.000000\nT0\nM30\n";
        let g = read_excellon(t).unwrap();
        assert!(g.finished);
        assert_eq!(g.tools[0].diameter, 300_000);
        assert_eq!(g.tools[0].function.as_deref(), Some("Plated,PTH,ViaDrill"));
        assert_eq!(g.holes[0].at, Point::new(1_500_000, -2_000_000));
        assert_eq!(g.slots[0].to, Point::new(1_000_000, 0));
        assert_eq!(
            g.file_attr("FileFunction").unwrap(),
            ["Plated", "1", "2", "PTH"]
        );
    }

    #[test]
    fn hostile_input_is_an_error() {
        for t in [
            "T1\nX1.0Y1.0\n",
            "M48\nMETRIC\n%\nT1\nX1.0Y1.0\n",
            "M48\nMETRIC\nT1C0.3\n%\nT2\n",
            "M48\nMETRIC\nT1C0.3\n%\nT1\nX100Y200\n",
            "M48\nINCH\n%\n",
            "M48\nMETRIC\nT1C0.3\n%\nT1\nX99999999.0Y1.0\n",
            "M48\nMETRIC\nT1C0.3\nT1C0.4\n%\n",
            "M48\nMETRIC\nT1C-1\n%\n",
            "M48\nMETRIC\nT1C0.3\n%\nT1\nX1.0\n",
        ] {
            assert!(read_excellon(t).is_err(), "{t:?}");
        }
    }
}

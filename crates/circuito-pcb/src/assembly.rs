// SPDX-License-Identifier: MIT OR Apache-2.0
//! Assembly outputs: the pick-and-place file and the bill of materials, as CSV.
//!
//! Both need every footprint they list to have a unique reference designator; a board where one
//! is missing or repeated is refused with the offending footprint named. Fields are written as RFC
//! 4180 CSV (quoted when they contain a comma, a quote or a line break, `\n` line ends). Text that
//! a spreadsheet would run as a formula (a leading `=`, `+`, `-`, `@`, tab or carriage return) is
//! written with a leading apostrophe, so opening the file never executes anything from a part
//! name.
//!
//! # Pick and place
//!
//! Columns: `Ref,Value,Package,PosX,PosY,Rot,Side`. `PosX` and `PosY` are the footprint origin in
//! millimetres with six decimals (exact), relative to [`AssemblyOptions::origin`]. `Rot` is the
//! footprint rotation on the board in degrees counter-clockwise, normalised to `0 <= Rot < 360`,
//! with three decimals, for both sides: no mirroring correction is applied for bottom-side parts,
//! so a fabricator whose machine expects another convention must convert. `Side` is `top` or
//! `bottom`. By default only surface-mount footprints (at least one SMD pad and no through-hole
//! pad) are listed, and footprints marked `exclude_from_pos` never are.
//!
//! # Bill of materials
//!
//! Footprints marked `exclude_from_bom` are left out. The rest are grouped by (value, package);
//! columns `Qty,Value,Package,References`, references in natural order separated by spaces, groups
//! ordered by their first reference.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::board::{Board, Footprint, PadKind};
use crate::error::{PcbError, Result};
use crate::geom::point::Point;
use crate::layer::Side;
use crate::units::{format_deg, format_mm};

/// Options of the pick-and-place file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssemblyOptions {
    /// Subtracted from every position (nm), for example the board's lower-left corner.
    pub origin: Point,
    /// List surface-mount footprints only.
    pub smd_only: bool,
}

impl Default for AssemblyOptions {
    fn default() -> Self {
        AssemblyOptions {
            origin: Point::new(0, 0),
            smd_only: true,
        }
    }
}

/// One line of the bill of materials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BomLine {
    pub quantity: usize,
    pub value: String,
    pub package: String,
    /// In natural order (`R2` before `R10`).
    pub references: Vec<String>,
}

/// Compares references the way a person would: `C2 < C10`, `R1 < R1A`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn chunks(s: &str) -> Vec<(bool, &str)> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut kind: Option<bool> = None;
        for (i, c) in s.char_indices() {
            let d = c.is_ascii_digit();
            if kind != Some(d) {
                if let (Some(k), Some(t)) = (kind, s.get(start..i)) {
                    out.push((k, t));
                }
                start = i;
                kind = Some(d);
            }
        }
        if let (Some(k), Some(t)) = (kind, s.get(start..)) {
            out.push((k, t));
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(cb.iter()) {
        let o = if x.0 && y.0 {
            let (tx, ty) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
            tx.len()
                .cmp(&ty.len())
                .then_with(|| tx.cmp(ty))
                .then_with(|| x.1.len().cmp(&y.1.len()))
        } else {
            x.1.cmp(y.1)
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    ca.len().cmp(&cb.len())
}

fn csv_field(s: &str) -> String {
    let mut t = s.to_string();
    if t.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        t.insert(0, '\'');
    }
    if t.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", t.replace('"', "\"\""))
    } else {
        t
    }
}

fn check_refs<'a>(fps: impl Iterator<Item = &'a Footprint>) -> Result<()> {
    let mut seen: BTreeMap<&str, u64> = BTreeMap::new();
    for fp in fps {
        if fp.reference.trim().is_empty() {
            return Err(PcbError::Invalid(format!(
                "footprint {} has no reference designator; assembly files need one on every part",
                fp.id.0
            )));
        }
        if let Some(prev) = seen.insert(fp.reference.as_str(), fp.id.0) {
            return Err(PcbError::Invalid(format!(
                "reference {} is used by footprints {prev} and {}",
                fp.reference, fp.id.0
            )));
        }
    }
    Ok(())
}

fn is_smd(fp: &Footprint) -> bool {
    fp.pads.iter().any(|p| p.kind == PadKind::Smd)
        && !fp.pads.iter().any(|p| p.kind == PadKind::ThroughHole)
}

/// The pick-and-place CSV.
pub fn pick_and_place_csv(board: &Board, opts: &AssemblyOptions) -> Result<String> {
    board.validate()?;
    let mut list: Vec<&Footprint> = board
        .footprints
        .iter()
        .filter(|f| !f.exclude_from_pos && (!opts.smd_only || is_smd(f)))
        .collect();
    check_refs(list.iter().copied())?;
    list.sort_by(|a, b| natural_cmp(&a.reference, &b.reference));
    let mut out = String::from("Ref,Value,Package,PosX,PosY,Rot,Side\n");
    for f in list {
        let x = f
            .position
            .x
            .checked_sub(opts.origin.x)
            .ok_or(PcbError::Overflow("pick and place origin"))?;
        let y = f
            .position
            .y
            .checked_sub(opts.origin.y)
            .ok_or(PcbError::Overflow("pick and place origin"))?;
        let rot = i64::from(f.rotation).rem_euclid(360_000);
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{}",
            csv_field(&f.reference),
            csv_field(&f.value),
            csv_field(&f.package),
            format_mm(x),
            format_mm(y),
            format_deg(rot),
            if f.side == Side::Front {
                "top"
            } else {
                "bottom"
            }
        );
    }
    Ok(out)
}

/// The bill of materials, grouped.
pub fn bom_lines(board: &Board) -> Result<Vec<BomLine>> {
    board.validate()?;
    let fps: Vec<&Footprint> = board
        .footprints
        .iter()
        .filter(|f| !f.exclude_from_bom)
        .collect();
    check_refs(fps.iter().copied())?;
    let mut groups: BTreeMap<(&str, &str), Vec<String>> = BTreeMap::new();
    for f in fps {
        groups
            .entry((f.value.as_str(), f.package.as_str()))
            .or_default()
            .push(f.reference.clone());
    }
    let mut lines: Vec<BomLine> = groups
        .into_iter()
        .map(|((value, package), mut refs)| {
            refs.sort_by(|a, b| natural_cmp(a, b));
            BomLine {
                quantity: refs.len(),
                value: value.to_string(),
                package: package.to_string(),
                references: refs,
            }
        })
        .collect();
    lines.sort_by(|a, b| {
        let fa = a.references.first().map(String::as_str).unwrap_or("");
        let fb = b.references.first().map(String::as_str).unwrap_or("");
        natural_cmp(fa, fb)
    });
    Ok(lines)
}

/// The bill of materials as CSV.
pub fn bom_csv(board: &Board) -> Result<String> {
    let mut out = String::from("Qty,Value,Package,References\n");
    for l in bom_lines(board)? {
        let _ = writeln!(
            out,
            "{},{},{},{}",
            l.quantity,
            csv_field(&l.value),
            csv_field(&l.package),
            csv_field(&l.references.join(" "))
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["R10", "R2", "C1", "R1A", "R1", "U01", "U1", "J2"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["C1", "J2", "R1", "R1A", "R2", "R10", "U1", "U01"]);
        assert_eq!(natural_cmp("R1", "R1"), Ordering::Equal);
        assert_eq!(natural_cmp("", "A"), Ordering::Less);
        // huge numbers do not overflow
        assert_eq!(
            natural_cmp("R99999999999999999999999", "R100000000000000000000000"),
            Ordering::Less
        );
    }

    #[test]
    fn csv_quoting_and_formula_neutralising() {
        assert_eq!(csv_field("10k"), "10k");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_field("=1+1"), "'=1+1");
        assert_eq!(csv_field("-5V"), "'-5V");
        assert_eq!(csv_field("line\nbreak"), "\"line\nbreak\"");
    }
}

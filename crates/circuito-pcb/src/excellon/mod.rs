// SPDX-License-Identifier: MIT OR Apache-2.0
//! Excellon drill files (the common NC drill dialect, with X2 attributes in `; #@!` comments).
//!
//! # What is written
//!
//! * `PTH` (plated through holes: through-hole pad drills and through vias),
//!   `NPTH` (non-plated holes), and one file for every blind, buried or micro via layer span.
//! * Metric. Coordinates and tool diameters are written **with an explicit decimal point and six
//!   decimals** (`X10.500000Y-3.250000`), which is exactly one nanometre per last digit. Because
//!   the decimal point is explicit, leading- and trailing-zero suppression do not apply: no zero is
//!   ever suppressed, so the `TZ` declaration in the header describes nothing that could be
//!   misread, whichever convention a reader assumes.
//! * Round holes as `X..Y..`, slots as the canned slot `X..Y..G85X..Y..` (width is the tool).
//! * A tool table in the header, sorted by diameter, with the X2 aperture function of each tool
//!   (`ViaDrill`, `ComponentDrill`, `MicroViaDrill`, `MechanicalDrill`) in a `; #@! TA.AperFunction`
//!   comment. Holes in a tool are sorted by position and duplicates removed.
//!
//! Not written: drill maps, route-mode outlines, backdrill, castellated holes.

pub mod read;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::board::{Board, PadKind};
use crate::error::Result;
use crate::geom::point::Point;
use crate::geom::shape::Core;
use crate::transform::pad_hole;
use crate::units::{format_mm, Nm};

pub use read::{read_excellon, ExcHole, ExcSlot, ExcTool, ExcellonImage, MAX_EXCELLON_BYTES};

/// Options of the drill writer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DrillOptions {
    /// Written as `TF.CreationDate` when set; otherwise no date is written.
    pub creation_date: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrillFile {
    pub file_name: String,
    /// The `TF.FileFunction` value, for example `Plated,1,4,PTH`.
    pub function: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DrillSet {
    pub files: Vec<DrillFile>,
    pub warnings: Vec<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Component,
    Via,
    Micro,
    Mechanical,
}

impl Class {
    fn name(self) -> &'static str {
        match self {
            Class::Component => "ComponentDrill",
            Class::Via => "ViaDrill",
            Class::Micro => "MicroViaDrill",
            Class::Mechanical => "MechanicalDrill",
        }
    }
}

/// Which file a hole goes to: plating and the copper layer span (indices into the copper stack).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Group {
    plated: bool,
    lo: usize,
    hi: usize,
}

struct Hole {
    group: Group,
    class: Class,
    dia: Nm,
    a: Point,
    /// The other end of a slot.
    b: Option<Point>,
}

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "board".into()
    } else {
        s
    }
}

fn collect(board: &Board, warnings: &mut Vec<String>) -> Result<Vec<Hole>> {
    let n = board.copper_layers as usize;
    let full = Group {
        plated: true,
        lo: 0,
        hi: n.saturating_sub(1),
    };
    let mut out = Vec::new();
    for fp in &board.footprints {
        for pad in &fp.pads {
            let Some(shape) = pad_hole(fp, pad)? else {
                continue;
            };
            let (group, class) = match pad.kind {
                PadKind::Npth => (
                    Group {
                        plated: false,
                        ..full
                    },
                    Class::Mechanical,
                ),
                _ => (full, Class::Component),
            };
            let (a, b) = match shape.core {
                Core::Point(c) => (c, None),
                Core::Segment(a, b) => (a, Some(b)),
                Core::Region(_) => continue,
            };
            out.push(Hole {
                group,
                class,
                dia: shape.diam,
                a,
                b,
            });
        }
    }
    for v in &board.vias {
        let (Some(i), Some(j)) = (
            v.from.copper_index(board.copper_layers),
            v.to.copper_index(board.copper_layers),
        ) else {
            warnings.push(format!(
                "via {} has an end on a layer this board does not have and was not drilled",
                v.id.0
            ));
            continue;
        };
        let (lo, hi) = (i.min(j), i.max(j));
        let class = if v.kind == crate::board::ViaKind::Micro {
            Class::Micro
        } else {
            Class::Via
        };
        out.push(Hole {
            group: Group {
                plated: true,
                lo,
                hi,
            },
            class,
            dia: v.drill,
            a: v.position,
            b: None,
        });
    }
    Ok(out)
}

/// Writes the drill files of a board.
pub fn write_drills(board: &Board, opts: &DrillOptions) -> Result<DrillSet> {
    board.validate()?;
    let mut set = DrillSet::default();
    let holes = collect(board, &mut set.warnings)?;
    let n = board.copper_layers as usize;
    let mut groups: BTreeMap<Group, Vec<&Hole>> = BTreeMap::new();
    for h in &holes {
        groups.entry(h.group).or_default().push(h);
    }
    let base = sanitize(&board.name);
    let ver = env!("CARGO_PKG_VERSION");
    // PTH first, then NPTH, then blind and buried spans in stack order
    let mut order: Vec<Group> = groups.keys().copied().collect();
    order.sort_by_key(|g| {
        let through = g.lo == 0 && g.hi + 1 == n;
        (u8::from(!through), u8::from(!g.plated), g.lo, g.hi)
    });
    for g in order {
        let Some(list) = groups.get(&g) else { continue };
        let through = g.lo == 0 && g.hi + 1 == n;
        let (function, suffix, plating) = if through && g.plated {
            (format!("Plated,1,{n},PTH"), "PTH".to_string(), "Plated,PTH")
        } else if through {
            (
                format!("NonPlated,1,{n},NPTH"),
                "NPTH".to_string(),
                "NonPlated,NPTH",
            )
        } else {
            let kind = if g.lo == 0 || g.hi + 1 == n {
                "Blind"
            } else {
                "Buried"
            };
            (
                format!("Plated,{},{},{kind}", g.lo + 1, g.hi + 1),
                format!("{}-{}", g.lo + 1, g.hi + 1),
                "Plated,PTH",
            )
        };
        // tools: (diameter, class), ascending
        let mut tools: BTreeMap<(Nm, Class), Vec<&Hole>> = BTreeMap::new();
        for h in list {
            tools.entry((h.dia, h.class)).or_default().push(h);
        }
        let mut out = String::new();
        out.push_str("M48\n");
        let _ = writeln!(out, "; #@! TF.GenerationSoftware,Nodalia,Circuito,{ver}");
        if let Some(d) = &opts.creation_date {
            let _ = writeln!(out, "; #@! TF.CreationDate,{}", sanitize_comment(d));
        }
        let _ = writeln!(out, "; #@! TF.SameCoordinates,Original");
        let _ = writeln!(out, "; #@! TF.FileFunction,{function}");
        out.push_str("; #@! TF.FilePolarity,Positive\n");
        out.push_str("FMAT,2\nMETRIC,TZ\n");
        for (i, ((dia, class), _)) in tools.iter().enumerate() {
            let _ = writeln!(out, "; #@! TA.AperFunction,{plating},{}", class.name());
            let _ = writeln!(out, "T{}C{}", i + 1, format_mm(*dia));
        }
        out.push_str("%\nG90\nG05\n");
        for (i, (_, hs)) in tools.iter().enumerate() {
            let _ = writeln!(out, "T{}", i + 1);
            let mut round: Vec<Point> = hs.iter().filter(|h| h.b.is_none()).map(|h| h.a).collect();
            round.sort_by_key(|p| (p.x, p.y));
            round.dedup();
            for p in round {
                let _ = writeln!(out, "X{}Y{}", format_mm(p.x), format_mm(p.y));
            }
            let mut slots: Vec<(Point, Point)> = hs
                .iter()
                .filter_map(|h| {
                    h.b.map(|b| {
                        if (h.a.x, h.a.y) <= (b.x, b.y) {
                            (h.a, b)
                        } else {
                            (b, h.a)
                        }
                    })
                })
                .collect();
            slots.sort_by_key(|(a, b)| (a.x, a.y, b.x, b.y));
            slots.dedup();
            for (a, b) in slots {
                let _ = writeln!(
                    out,
                    "X{}Y{}G85X{}Y{}",
                    format_mm(a.x),
                    format_mm(a.y),
                    format_mm(b.x),
                    format_mm(b.y)
                );
            }
        }
        out.push_str("T0\nM30\n");
        set.files.push(DrillFile {
            file_name: format!("{base}-{suffix}.drl"),
            function,
            content: out,
        });
    }
    if set.files.is_empty() {
        set.warnings.push("the board has no holes".into());
    }
    set.warnings.sort();
    Ok(set)
}

/// Keeps a free-text value on one line and free of control characters.
fn sanitize_comment(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

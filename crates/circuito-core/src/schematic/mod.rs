// SPDX-License-Identifier: MIT OR Apache-2.0
//! The schematic model: parts with typed pins, wires on an integer grid, junctions, net labels,
//! power ports, no-connect flags and sheets, plus the `.circuito` JSON file format.
//!
//! Coordinates are integers in grid units; one unit is 1.27 mm (50 mil) in a printed schematic.
//! Nothing here depends on how the schematic is drawn.
//!
//! # File format
//! A `.circuito` file is pretty-printed JSON with `format_version: 1`. Reading is bounded (see the
//! `MAX_*` constants), refuses any other format version with a message that says which version was
//! found, and never panics on malformed input.

pub mod builder;
mod nets;

pub use nets::{resolve_nets, LabelRef, Net, NetResolution, NetWarning, PinRef, PortRef};

use crate::netlist::{Analysis, Model};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// The only file format version this build reads and writes.
pub const FORMAT_VERSION: u32 = 1;
/// Largest `.circuito` file accepted, in bytes.
pub const MAX_FILE_BYTES: usize = 64 * 1024 * 1024;
/// Most sheets in one schematic.
pub const MAX_SHEETS: usize = 256;
/// Most parts on a sheet.
pub const MAX_PARTS: usize = 200_000;
/// Most wires on a sheet.
pub const MAX_WIRES: usize = 500_000;
/// Most vertices in one wire.
pub const MAX_WIRE_VERTICES: usize = 10_000;
/// Most labels, ports, junctions and no-connect flags (each) on a sheet.
pub const MAX_MARKS: usize = 500_000;
/// Longest text field in characters.
pub const MAX_TEXT: usize = 1024;
/// Largest absolute coordinate.
pub const MAX_COORD: i32 = 10_000_000;

/// Everything that can go wrong reading, checking or resolving a schematic.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SchematicError {
    /// The text is not valid JSON or does not match the file format.
    #[error("not a valid .circuito file: {0}")]
    Json(String),
    /// The file is too large.
    #[error("the file is {0} bytes; the limit is {MAX_FILE_BYTES}")]
    TooLarge(usize),
    /// The file declares a format this build does not read.
    #[error(
        "unsupported format_version {found} (this build reads version {FORMAT_VERSION}); {hint}"
    )]
    Version {
        /// The version in the file.
        found: String,
        /// What to do about it.
        hint: &'static str,
    },
    /// A limit on counts, lengths or coordinates is exceeded.
    #[error("{0}")]
    Limit(String),
    /// A part refers to a library symbol that does not exist.
    #[error("part {refdes} uses unknown library symbol `{lib_id}`")]
    UnknownPart {
        /// Reference designator.
        refdes: String,
        /// Missing symbol id.
        lib_id: String,
    },
}

/// A grid point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal grid coordinate (grows to the right).
    pub x: i32,
    /// Vertical grid coordinate (grows downwards).
    pub y: i32,
}

impl Point {
    /// A point.
    #[must_use]
    pub const fn new(x: i32, y: i32) -> Self {
        Point { x, y }
    }
}

/// Part rotation in degrees, counter-clockwise on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Rotation {
    /// No rotation.
    #[default]
    #[serde(rename = "0")]
    R0,
    /// 90 degrees.
    #[serde(rename = "90")]
    R90,
    /// 180 degrees.
    #[serde(rename = "180")]
    R180,
    /// 270 degrees.
    #[serde(rename = "270")]
    R270,
}

/// The electrical type of a pin, used by the rules check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PinType {
    /// Passive terminal (resistor, capacitor, transistor terminal...).
    Passive,
    /// Input.
    Input,
    /// Output.
    Output,
    /// Bidirectional.
    Bidirectional,
    /// Power input (needs a power source on its net).
    PowerIn,
    /// Power output (drives a supply net).
    PowerOut,
    /// Open-collector or open-drain output.
    OpenCollector,
    /// May be left unconnected without a rules violation.
    Unconnected,
    /// Not connected: must stay unconnected.
    Nc,
}

/// A placed part.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// Reference designator, e.g. `R1`.
    pub refdes: String,
    /// Library symbol id, e.g. `R`.
    pub lib_id: String,
    /// The value text: `4.7k`, `100n`, or a source description such as `SIN(0 1 1k)`.
    #[serde(default)]
    pub value: String,
    /// Position of the symbol origin.
    pub position: Point,
    /// Rotation.
    #[serde(default)]
    pub rotation: Rotation,
    /// Mirrored left-to-right before rotating.
    #[serde(default)]
    pub mirror: bool,
    /// Simulation model name for diodes, transistors, switches and op-amps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Extra named fields (`wiper`, `ratio`, `k`, ...).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

/// A wire: a polyline of at least two grid points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wire {
    /// The vertices, in order.
    pub points: Vec<Point>,
}

/// A junction dot: it connects every wire and pin that passes through or ends at its point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Junction {
    /// Position.
    pub position: Point,
}

/// How far a label's name reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LabelScope {
    /// Connects only to labels of the same name on the same sheet.
    #[default]
    Local,
    /// Connects to labels of the same name on every sheet.
    Global,
}

/// A net label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    /// The net name.
    pub name: String,
    /// Position.
    pub position: Point,
    /// Reach of the name.
    #[serde(default)]
    pub scope: LabelScope,
}

/// The symbol drawn for a power port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PowerStyle {
    /// Ground symbol.
    Ground,
    /// Supply rail bar/arrow.
    #[default]
    Supply,
}

/// A power port: a global net name drawn as a symbol (`GND`, `VCC`, `+5V`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PowerPort {
    /// The net name; `0` and `GND` (any case) are ground.
    pub name: String,
    /// Position.
    pub position: Point,
    /// Symbol style.
    #[serde(default)]
    pub style: PowerStyle,
}

/// A no-connect flag placed on a pin that is deliberately left open.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NoConnect {
    /// Position.
    pub position: Point,
}

/// One sheet of a schematic.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Sheet {
    /// Sheet name.
    pub name: String,
    /// Placed parts.
    #[serde(default)]
    pub parts: Vec<Part>,
    /// Wires.
    #[serde(default)]
    pub wires: Vec<Wire>,
    /// Junctions.
    #[serde(default)]
    pub junctions: Vec<Junction>,
    /// Net labels.
    #[serde(default)]
    pub labels: Vec<Label>,
    /// Power ports.
    #[serde(default)]
    pub power_ports: Vec<PowerPort>,
    /// No-connect flags.
    #[serde(default)]
    pub no_connects: Vec<NoConnect>,
}

/// A schematic: sheets plus the simulation models and analyses that go with it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schematic {
    /// File format version; always [`FORMAT_VERSION`].
    pub format_version: u32,
    /// Title.
    #[serde(default)]
    pub title: String,
    /// Sheets (at least one).
    pub sheets: Vec<Sheet>,
    /// Model cards used by the parts.
    #[serde(default)]
    pub models: Vec<Model>,
    /// Analyses to run on the extracted netlist.
    #[serde(default)]
    pub analyses: Vec<Analysis>,
}

impl Default for Schematic {
    fn default() -> Self {
        Schematic::new("")
    }
}

impl Schematic {
    /// A schematic with one empty sheet.
    #[must_use]
    pub fn new(title: &str) -> Self {
        Schematic {
            format_version: FORMAT_VERSION,
            title: title.to_string(),
            sheets: vec![Sheet {
                name: "Sheet1".into(),
                ..Sheet::default()
            }],
            models: Vec::new(),
            analyses: Vec::new(),
        }
    }

    /// Serialize to the `.circuito` JSON text (pretty-printed, stable field order).
    ///
    /// # Errors
    /// Only if a number cannot be represented, which a valid schematic never has.
    pub fn to_json(&self) -> Result<String, SchematicError> {
        serde_json::to_string_pretty(self).map_err(|e| SchematicError::Json(e.to_string()))
    }

    /// Read a `.circuito` file's text. The size, counts, lengths and coordinates are bounded.
    ///
    /// # Errors
    /// For malformed JSON, a different `format_version`, or a limit that is exceeded.
    pub fn from_json(text: &str) -> Result<Schematic, SchematicError> {
        if text.len() > MAX_FILE_BYTES {
            return Err(SchematicError::TooLarge(text.len()));
        }
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| SchematicError::Json(e.to_string()))?;
        let obj = value
            .as_object()
            .ok_or_else(|| SchematicError::Json("the top level must be an object".into()))?;
        match obj.get("format_version") {
            None => {
                return Err(SchematicError::Json("missing `format_version`".into()));
            }
            Some(v) => match v.as_u64() {
                Some(n) if n == u64::from(FORMAT_VERSION) => {}
                Some(n) if n > u64::from(FORMAT_VERSION) => {
                    return Err(SchematicError::Version {
                        found: n.to_string(),
                        hint: "the file was written by a newer Circuito; update the application to open it",
                    })
                }
                _ => {
                    return Err(SchematicError::Version {
                        found: v.to_string(),
                        hint: "format_version must be the integer 1",
                    })
                }
            },
        }
        let s: Schematic =
            serde_json::from_value(value).map_err(|e| SchematicError::Json(e.to_string()))?;
        s.check_limits()?;
        Ok(s)
    }

    /// Check counts, text lengths and coordinates against the `MAX_*` limits.
    ///
    /// # Errors
    /// [`SchematicError::Limit`] naming what is too big.
    pub fn check_limits(&self) -> Result<(), SchematicError> {
        let lim = |m: String| Err(SchematicError::Limit(m));
        if self.sheets.is_empty() {
            return lim("a schematic needs at least one sheet".into());
        }
        if self.sheets.len() > MAX_SHEETS {
            return lim(format!("more than {MAX_SHEETS} sheets"));
        }
        let text = |what: &str, s: &str| -> Result<(), SchematicError> {
            if s.chars().count() > MAX_TEXT {
                Err(SchematicError::Limit(format!(
                    "{what} is longer than {MAX_TEXT} characters"
                )))
            } else {
                Ok(())
            }
        };
        let pt = |what: &str, p: Point| -> Result<(), SchematicError> {
            if p.x.abs() > MAX_COORD || p.y.abs() > MAX_COORD {
                Err(SchematicError::Limit(format!(
                    "{what} at ({}, {}) is outside the +/-{MAX_COORD} coordinate range",
                    p.x, p.y
                )))
            } else {
                Ok(())
            }
        };
        text("title", &self.title)?;
        for m in &self.models {
            text("model name", &m.name)?;
            if m.params.len() > 1000 {
                return lim(format!("model {} has more than 1000 parameters", m.name));
            }
        }
        if self.models.len() > MAX_MARKS || self.analyses.len() > 1000 {
            return lim("too many models or analyses".into());
        }
        for sh in &self.sheets {
            text("sheet name", &sh.name)?;
            if sh.parts.len() > MAX_PARTS {
                return lim(format!(
                    "sheet `{}` has more than {MAX_PARTS} parts",
                    sh.name
                ));
            }
            if sh.wires.len() > MAX_WIRES {
                return lim(format!(
                    "sheet `{}` has more than {MAX_WIRES} wires",
                    sh.name
                ));
            }
            if sh.labels.len() > MAX_MARKS
                || sh.power_ports.len() > MAX_MARKS
                || sh.junctions.len() > MAX_MARKS
                || sh.no_connects.len() > MAX_MARKS
            {
                return lim(format!(
                    "sheet `{}` has too many labels, ports, junctions or flags",
                    sh.name
                ));
            }
            for p in &sh.parts {
                text("refdes", &p.refdes)?;
                text("lib_id", &p.lib_id)?;
                text("value", &p.value)?;
                pt(&format!("part {}", p.refdes), p.position)?;
                for (k, v) in &p.fields {
                    text("field name", k)?;
                    text("field value", v)?;
                }
            }
            for (i, w) in sh.wires.iter().enumerate() {
                if w.points.len() < 2 {
                    return lim(format!(
                        "wire {} on sheet `{}` has fewer than 2 points",
                        i + 1,
                        sh.name
                    ));
                }
                if w.points.len() > MAX_WIRE_VERTICES {
                    return lim(format!(
                        "wire {} on sheet `{}` has more than {MAX_WIRE_VERTICES} vertices",
                        i + 1,
                        sh.name
                    ));
                }
                for p in &w.points {
                    pt(&format!("wire {}", i + 1), *p)?;
                }
            }
            for j in &sh.junctions {
                pt("junction", j.position)?;
            }
            for l in &sh.labels {
                text("label", &l.name)?;
                pt(&format!("label {}", l.name), l.position)?;
            }
            for p in &sh.power_ports {
                text("power port", &p.name)?;
                pt(&format!("power port {}", p.name), p.position)?;
            }
            for n in &sh.no_connects {
                pt("no-connect flag", n.position)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Schematic {
        let mut s = Schematic::new("sample");
        s.sheets[0].parts.push(Part {
            refdes: "R1".into(),
            lib_id: "R".into(),
            value: "1k".into(),
            position: Point::new(10, 10),
            rotation: Rotation::R90,
            mirror: true,
            model: None,
            fields: BTreeMap::new(),
        });
        s.sheets[0].wires.push(Wire {
            points: vec![Point::new(0, 0), Point::new(5, 0), Point::new(5, 5)],
        });
        s.sheets[0].junctions.push(Junction {
            position: Point::new(5, 0),
        });
        s.sheets[0].labels.push(Label {
            name: "OUT".into(),
            position: Point::new(0, 0),
            scope: LabelScope::Global,
        });
        s.sheets[0].power_ports.push(PowerPort {
            name: "GND".into(),
            position: Point::new(5, 5),
            style: PowerStyle::Ground,
        });
        s.sheets[0].no_connects.push(NoConnect {
            position: Point::new(9, 9),
        });
        s.models
            .push(Model::new("DX", crate::netlist::ModelKind::D).with("IS", 1e-14));
        s.analyses.push(Analysis::Op);
        s
    }

    #[test]
    fn json_round_trip() {
        let s = sample();
        let text = s.to_json().unwrap();
        assert!(text.contains("\"format_version\": 1"));
        let back = Schematic::from_json(&text).unwrap();
        assert_eq!(s, back);
        assert_eq!(text, back.to_json().unwrap());
    }

    #[test]
    fn refuses_other_versions() {
        let mut v: serde_json::Value = serde_json::from_str(&sample().to_json().unwrap()).unwrap();
        v["format_version"] = 2.into();
        let e = Schematic::from_json(&v.to_string()).unwrap_err();
        assert!(e.to_string().contains("newer Circuito"), "{e}");
        v["format_version"] = 0.into();
        assert!(matches!(
            Schematic::from_json(&v.to_string()),
            Err(SchematicError::Version { .. })
        ));
        v["format_version"] = "1".into();
        assert!(matches!(
            Schematic::from_json(&v.to_string()),
            Err(SchematicError::Version { .. })
        ));
        v.as_object_mut().unwrap().remove("format_version");
        assert!(matches!(
            Schematic::from_json(&v.to_string()),
            Err(SchematicError::Json(_))
        ));
    }

    #[test]
    fn malformed_input_is_an_error() {
        for bad in [
            "", "{", "[]", "null", "{\"format_version\":1}", "{\"format_version\":1,\"sheets\":[]}",
            "{\"format_version\":1,\"sheets\":[{\"name\":\"a\",\"wires\":[{\"points\":[{\"x\":0,\"y\":0}]}]}]}",
            "{\"format_version\":1,\"sheets\":[{\"name\":\"a\",\"parts\":[{\"refdes\":1}]}]}",
            "{\"format_version\":1,\"sheets\":[{\"name\":\"a\",\"junctions\":[{\"position\":{\"x\":99999999,\"y\":0}}]}]}",
            "{\"format_version\":1,\"sheets\":[{\"name\":\"a\",\"junctions\":[{\"position\":{\"x\":1e99,\"y\":0}}]}]}",
        ] {
            assert!(Schematic::from_json(bad).is_err(), "{bad}");
        }
        // Deeply nested garbage must not overflow the stack.
        let deep = "[".repeat(100_000);
        assert!(Schematic::from_json(&deep).is_err());
    }

    #[test]
    fn truncations_never_panic() {
        let text = sample().to_json().unwrap();
        for i in (0..text.len()).step_by(7) {
            if text.is_char_boundary(i) {
                let _ = Schematic::from_json(&text[..i]);
            }
        }
    }

    #[test]
    fn size_limit() {
        let big = " ".repeat(MAX_FILE_BYTES + 1);
        assert!(matches!(
            Schematic::from_json(&big),
            Err(SchematicError::TooLarge(_))
        ));
    }
}

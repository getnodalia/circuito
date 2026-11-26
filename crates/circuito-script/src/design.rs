// SPDX-License-Identifier: MIT OR Apache-2.0
//! The design: a circuit as the text form describes it, without any geometry.
//!
//! A [`Design`] lists parts, the net each pin is on, the model cards and the analyses. The text
//! form is parsed into one and printed from one; the schematic is converted to one and built
//! from one.

use circuito_core::netlist::{Analysis, Model};
use circuito_core::schematic::{Point, Rotation};
use std::collections::BTreeMap;

/// Where a pin goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinNet {
    /// Nothing is connected and nothing is said about it.
    Open,
    /// Deliberately unconnected (a no-connect flag).
    NoConnect,
    /// On the net of this name.
    Net(String),
}

/// Where a part sits in the drawing, when the text says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    /// Position of the symbol origin.
    pub position: Point,
    /// Rotation.
    pub rotation: Rotation,
    /// Mirrored left to right.
    pub mirror: bool,
}

/// One part of a design.
#[derive(Debug, Clone, PartialEq)]
pub struct PartSpec {
    /// Reference designator.
    pub refdes: String,
    /// Library symbol id.
    pub lib_id: String,
    /// Value text (empty for parts without one).
    pub value: String,
    /// Model name.
    pub model: Option<String>,
    /// Extra fields.
    pub fields: BTreeMap<String, String>,
    /// One entry per pin of the symbol, in the symbol's pin order.
    pub pins: Vec<PinNet>,
    /// Drawing position, if known.
    pub place: Option<Place>,
    /// Line in the text this came from (1-based; 0 when it did not come from text).
    pub line: usize,
}

/// A circuit without geometry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Design {
    /// Title.
    pub title: String,
    /// Parts, in the order they were written.
    pub parts: Vec<PartSpec>,
    /// Model cards.
    pub models: Vec<Model>,
    /// Analyses.
    pub analyses: Vec<Analysis>,
}

/// What a net is for, which decides how it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetKind {
    /// Ground.
    Ground,
    /// A supply rail such as `vcc` or `+5V`.
    Supply,
    /// Anything else.
    Signal,
}

/// A net of a design.
#[derive(Debug, Clone, PartialEq)]
pub struct NetInfo {
    /// Lower-case key; ground is `0`.
    pub key: String,
    /// The name as first written.
    pub name: String,
    /// What it is for.
    pub kind: NetKind,
    /// The pins on it as (part index, pin index).
    pub pins: Vec<(usize, usize)>,
}

/// The key a net name is compared by: case-insensitive, with every ground spelling as `0`.
#[must_use]
pub fn net_key(name: &str) -> String {
    if is_ground(name) {
        "0".to_string()
    } else {
        name.to_lowercase()
    }
}

/// Whether the name is ground (`0` or `gnd`, any case).
#[must_use]
pub fn is_ground(name: &str) -> bool {
    name == "0" || name.eq_ignore_ascii_case("gnd")
}

/// Whether the name looks like a supply rail: `vcc`, `vdd`, `vee`, `vss`, `v+`, `v-`, `+5v`, `-12V`.
#[must_use]
pub fn is_supply(name: &str) -> bool {
    let l = name.to_lowercase();
    if matches!(l.as_str(), "vcc" | "vdd" | "vee" | "vss" | "v+" | "v-") {
        return true;
    }
    let mut chars = l.chars();
    matches!(chars.next(), Some('+' | '-')) && chars.next().is_some_and(|c| c.is_ascii_digit())
}

/// Whether the name is anonymous: `n1`, `n2`… only join pins in the text and are not drawn.
#[must_use]
pub fn is_anonymous(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('n' | 'N'))
        && chars.as_str().chars().count() > 0
        && chars.all(|c| c.is_ascii_digit())
}

/// The kind of a net from its name.
#[must_use]
pub fn kind_of(name: &str) -> NetKind {
    if is_ground(name) {
        NetKind::Ground
    } else if is_supply(name) {
        NetKind::Supply
    } else {
        NetKind::Signal
    }
}

impl Design {
    /// The nets of the design in order of first appearance.
    #[must_use]
    pub fn nets(&self) -> Vec<NetInfo> {
        let mut out: Vec<NetInfo> = Vec::new();
        let mut index: BTreeMap<String, usize> = BTreeMap::new();
        for (pi, part) in self.parts.iter().enumerate() {
            for (k, pin) in part.pins.iter().enumerate() {
                let PinNet::Net(name) = pin else { continue };
                let key = net_key(name);
                let at = *index.entry(key.clone()).or_insert_with(|| {
                    out.push(NetInfo {
                        key,
                        name: if is_ground(name) {
                            "0".to_string()
                        } else {
                            name.clone()
                        },
                        kind: kind_of(name),
                        pins: Vec::new(),
                    });
                    out.len() - 1
                });
                if let Some(n) = out.get_mut(at) {
                    n.pins.push((pi, k));
                }
            }
        }
        out
    }

    /// The index of the part with this reference designator (ignoring case).
    #[must_use]
    pub fn part_index(&self, refdes: &str) -> Option<usize> {
        self.parts
            .iter()
            .position(|p| p.refdes.eq_ignore_ascii_case(refdes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_classified() {
        assert!(is_ground("GND") && is_ground("0") && !is_ground("00"));
        for s in ["vcc", "VDD", "+5V", "-12v", "+3.3V", "V+"] {
            assert!(is_supply(s), "{s}");
        }
        for s in ["in", "out", "v1", "+", "-x", "vref"] {
            assert!(!is_supply(s), "{s}");
        }
        assert!(
            is_anonymous("n1") && is_anonymous("N42") && !is_anonymous("n") && !is_anonymous("n1a")
        );
        assert_eq!(net_key("GND"), "0");
        assert_eq!(net_key("Out"), "out");
    }

    #[test]
    fn nets_are_collected_in_order() {
        let part = |r: &str, a: &str, b: &str| PartSpec {
            refdes: r.into(),
            lib_id: "R".into(),
            value: "1k".into(),
            model: None,
            fields: BTreeMap::new(),
            pins: vec![PinNet::Net(a.into()), PinNet::Net(b.into())],
            place: None,
            line: 0,
        };
        let d = Design {
            parts: vec![part("R1", "in", "out"), part("R2", "OUT", "gnd")],
            ..Design::default()
        };
        let nets = d.nets();
        assert_eq!(nets.len(), 3);
        assert_eq!(nets[1].pins, vec![(0, 1), (1, 0)]);
        assert_eq!(nets[2].kind, NetKind::Ground);
        assert_eq!(nets[2].name, "0");
    }
}

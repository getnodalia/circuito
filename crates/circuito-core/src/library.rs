// SPDX-License-Identifier: MIT OR Apache-2.0
//! The built-in part library: original symbol geometry defined as data, pin electrical types and
//! default simulation models.
//!
//! Every symbol here is original work drawn for Circuito from textbook schematic conventions (a
//! zig-zag or box for a resistor, two plates for a capacitor, a triangle and bar for a diode). No
//! other tool's symbols were consulted or copied. Coordinates are grid units (1.27 mm); a symbol's
//! origin is its centre and pins sit on integer points so wires meet them exactly.

use crate::netlist::{Model, ModelKind};
use crate::schematic::{PinType, Point, PowerStyle, Rotation};
use std::sync::OnceLock;

/// What the part is, which decides how it becomes a netlist element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    /// Resistor.
    Resistor,
    /// Capacitor (polarised or not).
    Capacitor,
    /// Inductor.
    Inductor,
    /// Independent voltage source.
    VSource,
    /// Independent current source.
    ISource,
    /// Diode, LED or zener.
    Diode,
    /// NPN bipolar transistor.
    Npn,
    /// PNP bipolar transistor.
    Pnp,
    /// N-channel MOSFET.
    Nmos,
    /// P-channel MOSFET.
    Pmos,
    /// Op-amp (3 or 5 pins).
    OpAmp,
    /// Voltage-controlled switch.
    Switch,
    /// Potentiometer: two resistors sharing the wiper.
    Potentiometer,
    /// Two coupled inductors.
    Transformer,
    /// Fuse: a small series resistance.
    Fuse,
}

/// A symbol shape in the part's own coordinates.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A polyline, optionally closed and filled.
    Polyline {
        /// Vertices.
        points: Vec<Point>,
        /// Join the last vertex to the first.
        closed: bool,
        /// Fill the interior.
        filled: bool,
    },
    /// An axis-aligned rectangle.
    Rect {
        /// One corner.
        min: Point,
        /// The opposite corner.
        max: Point,
    },
    /// A circle.
    Circle {
        /// Centre.
        center: Point,
        /// Radius in grid units.
        radius: i32,
    },
    /// A circular arc, angles in degrees counter-clockwise from the +x axis.
    Arc {
        /// Centre.
        center: Point,
        /// Radius in grid units.
        radius: i32,
        /// Start angle.
        start_deg: i32,
        /// End angle.
        end_deg: i32,
    },
}

/// A pin of a symbol.
#[derive(Debug, Clone, PartialEq)]
pub struct PinDef {
    /// Pin number as printed.
    pub number: &'static str,
    /// Pin name.
    pub name: &'static str,
    /// Position relative to the symbol origin, before mirroring and rotation.
    pub position: Point,
    /// Electrical type.
    pub pin_type: PinType,
}

/// A library symbol.
#[derive(Debug, Clone, PartialEq)]
pub struct PartDef {
    /// Library id used in `Part::lib_id`.
    pub id: &'static str,
    /// Human name.
    pub name: &'static str,
    /// Reference designator prefix (`R`, `C`, `Q`...).
    pub prefix: &'static str,
    /// One-line description.
    pub description: &'static str,
    /// What the part is.
    pub device: Device,
    /// Pins.
    pub pins: Vec<PinDef>,
    /// Symbol shapes.
    pub shapes: Vec<Shape>,
    /// Default value text for a newly placed part.
    pub default_value: &'static str,
    /// Default model name, for parts that need one.
    pub default_model: Option<&'static str>,
}

/// A power symbol: a net name drawn in a style.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerSymbol {
    /// Library id.
    pub id: &'static str,
    /// Default net name.
    pub net: &'static str,
    /// Style.
    pub style: PowerStyle,
    /// Shapes; the connection point is the origin.
    pub shapes: Vec<Shape>,
}

/// The set of symbols available to a schematic.
#[derive(Debug, Clone)]
pub struct Library {
    parts: Vec<PartDef>,
    power: Vec<PowerSymbol>,
}

const fn p(x: i32, y: i32) -> Point {
    Point::new(x, y)
}

fn line(pts: &[(i32, i32)]) -> Shape {
    Shape::Polyline {
        points: pts.iter().map(|&(x, y)| p(x, y)).collect(),
        closed: false,
        filled: false,
    }
}

fn poly(pts: &[(i32, i32)], filled: bool) -> Shape {
    Shape::Polyline {
        points: pts.iter().map(|&(x, y)| p(x, y)).collect(),
        closed: true,
        filled,
    }
}

fn pin(number: &'static str, name: &'static str, x: i32, y: i32, pin_type: PinType) -> PinDef {
    PinDef {
        number,
        name,
        position: p(x, y),
        pin_type,
    }
}

#[allow(clippy::too_many_arguments)]
fn def(
    id: &'static str,
    name: &'static str,
    prefix: &'static str,
    description: &'static str,
    device: Device,
    pins: Vec<PinDef>,
    shapes: Vec<Shape>,
    default_value: &'static str,
    default_model: Option<&'static str>,
) -> PartDef {
    PartDef {
        id,
        name,
        prefix,
        description,
        device,
        pins,
        shapes,
        default_value,
        default_model,
    }
}

fn two_terminal_leads() -> Vec<Shape> {
    vec![line(&[(0, -4), (0, -3)]), line(&[(0, 3), (0, 4)])]
}

fn build() -> Library {
    use PinType::{Input, Output, Passive, PowerIn, PowerOut};
    let mut parts = Vec::new();

    let mut shapes = two_terminal_leads();
    shapes.push(Shape::Rect {
        min: p(-1, -3),
        max: p(1, 3),
    });
    parts.push(def(
        "R",
        "Resistor",
        "R",
        "Fixed resistor",
        Device::Resistor,
        vec![pin("1", "~", 0, -4, Passive), pin("2", "~", 0, 4, Passive)],
        shapes,
        "1k",
        None,
    ));

    let mut shapes = vec![line(&[(0, -4), (0, -1)]), line(&[(0, 1), (0, 4)])];
    shapes.push(line(&[(-3, -1), (3, -1)]));
    shapes.push(line(&[(-3, 1), (3, 1)]));
    parts.push(def(
        "C",
        "Capacitor",
        "C",
        "Non-polarised capacitor",
        Device::Capacitor,
        vec![pin("1", "~", 0, -4, Passive), pin("2", "~", 0, 4, Passive)],
        shapes,
        "100n",
        None,
    ));

    let mut shapes = vec![line(&[(0, -4), (0, -1)]), line(&[(0, 1), (0, 4)])];
    shapes.push(line(&[(-3, -1), (3, -1)]));
    shapes.push(Shape::Arc {
        center: p(0, 6),
        radius: 5,
        start_deg: 53,
        end_deg: 127,
    });
    shapes.push(line(&[(-3, -3), (-1, -3)]));
    shapes.push(line(&[(-2, -4), (-2, -2)]));
    parts.push(def(
        "C_POL",
        "Polarised capacitor",
        "C",
        "Electrolytic capacitor; pin 1 is positive",
        Device::Capacitor,
        vec![pin("1", "+", 0, -4, Passive), pin("2", "-", 0, 4, Passive)],
        shapes,
        "10u",
        None,
    ));

    let mut shapes = vec![line(&[(0, -4), (0, -3)]), line(&[(0, 3), (0, 4)])];
    for k in 0..3 {
        shapes.push(Shape::Arc {
            center: p(0, -2 + 2 * k),
            radius: 1,
            start_deg: -90,
            end_deg: 90,
        });
    }
    parts.push(def(
        "L",
        "Inductor",
        "L",
        "Inductor",
        Device::Inductor,
        vec![pin("1", "~", 0, -4, Passive), pin("2", "~", 0, 4, Passive)],
        shapes,
        "10m",
        None,
    ));

    let shapes = vec![
        line(&[(0, -4), (0, -3)]),
        line(&[(0, 3), (0, 4)]),
        Shape::Circle {
            center: p(0, 0),
            radius: 3,
        },
        line(&[(0, -2), (0, -1)]),
        line(&[(-1, -1), (1, -1)]),
        line(&[(-1, 1), (1, 1)]),
    ];
    parts.push(def(
        "V",
        "Voltage source",
        "V",
        "Independent voltage source; value is a source description such as 5, AC 1 or SIN(0 1 1k)",
        Device::VSource,
        vec![pin("1", "+", 0, -4, PowerOut), pin("2", "-", 0, 4, Passive)],
        shapes,
        "5",
        None,
    ));

    let shapes = vec![
        line(&[(0, -4), (0, -3)]),
        line(&[(0, 3), (0, 4)]),
        Shape::Circle {
            center: p(0, 0),
            radius: 3,
        },
        line(&[(0, 2), (0, -2)]),
        poly(&[(0, -2), (-1, 0), (1, 0)], true),
    ];
    parts.push(def(
        "I",
        "Current source",
        "I",
        "Independent current source; current flows from pin 1 through the source to pin 2",
        Device::ISource,
        vec![pin("1", "+", 0, -4, Passive), pin("2", "-", 0, 4, Passive)],
        shapes,
        "1m",
        None,
    ));

    let diode_body = || -> Vec<Shape> {
        vec![
            line(&[(0, -4), (0, -2)]),
            line(&[(0, 2), (0, 4)]),
            poly(&[(-2, -2), (2, -2), (0, 2)], false),
            line(&[(-2, 2), (2, 2)]),
        ]
    };
    parts.push(def(
        "D",
        "Diode",
        "D",
        "Signal or rectifier diode",
        Device::Diode,
        vec![pin("A", "A", 0, -4, Passive), pin("K", "K", 0, 4, Passive)],
        diode_body(),
        "",
        Some("D"),
    ));

    let mut shapes = diode_body();
    shapes.push(line(&[(3, -2), (5, -4)]));
    shapes.push(line(&[(4, 0), (6, -2)]));
    parts.push(def(
        "LED",
        "LED",
        "D",
        "Light-emitting diode",
        Device::Diode,
        vec![pin("A", "A", 0, -4, Passive), pin("K", "K", 0, 4, Passive)],
        shapes,
        "",
        Some("LED"),
    ));

    let mut shapes = diode_body();
    shapes.push(line(&[(-2, 2), (-3, 1)]));
    shapes.push(line(&[(2, 2), (3, 3)]));
    parts.push(def(
        "ZENER",
        "Zener diode",
        "D",
        "Zener (breakdown) diode",
        Device::Diode,
        vec![pin("A", "A", 0, -4, Passive), pin("K", "K", 0, 4, Passive)],
        shapes,
        "5.1V",
        Some("DZ"),
    ));

    let bjt = |emitter_arrow_out: bool| -> Vec<Shape> {
        let mut s = vec![
            Shape::Circle {
                center: p(0, 0),
                radius: 4,
            },
            line(&[(-4, 0), (-1, 0)]),
            line(&[(-1, -2), (-1, 2)]),
            line(&[(-1, -1), (3, -3)]),
            line(&[(3, -3), (3, -4)]),
            line(&[(-1, 1), (3, 3)]),
            line(&[(3, 3), (3, 4)]),
        ];
        if emitter_arrow_out {
            s.push(poly(&[(3, 3), (1, 2), (2, 1)], true));
        } else {
            s.push(poly(&[(-1, 1), (1, 1), (0, 2)], true));
        }
        s
    };
    parts.push(def(
        "NPN",
        "NPN transistor",
        "Q",
        "NPN bipolar transistor",
        Device::Npn,
        vec![
            pin("C", "C", 3, -4, Passive),
            pin("B", "B", -4, 0, Passive),
            pin("E", "E", 3, 4, Passive),
        ],
        bjt(true),
        "",
        Some("NPN"),
    ));
    parts.push(def(
        "PNP",
        "PNP transistor",
        "Q",
        "PNP bipolar transistor",
        Device::Pnp,
        vec![
            pin("C", "C", 3, 4, Passive),
            pin("B", "B", -4, 0, Passive),
            pin("E", "E", 3, -4, Passive),
        ],
        bjt(false),
        "",
        Some("PNP"),
    ));

    let mos = || -> Vec<Shape> {
        vec![
            line(&[(-4, 0), (-2, 0)]),
            line(&[(-2, -2), (-2, 2)]),
            line(&[(-1, -3), (-1, -1)]),
            line(&[(-1, -1), (-1, 1)]),
            line(&[(-1, 1), (-1, 3)]),
            line(&[(-1, -2), (3, -2)]),
            line(&[(3, -2), (3, -4)]),
            line(&[(-1, 2), (3, 2)]),
            line(&[(3, 2), (3, 4)]),
            line(&[(-1, 0), (3, 0)]),
            line(&[(3, 0), (3, 2)]),
        ]
    };
    parts.push(def(
        "NMOS",
        "N-channel MOSFET",
        "M",
        "Enhancement N-channel MOSFET (bulk tied to source)",
        Device::Nmos,
        vec![
            pin("D", "D", 3, -4, Passive),
            pin("G", "G", -4, 0, Passive),
            pin("S", "S", 3, 4, Passive),
        ],
        mos(),
        "",
        Some("NMOS"),
    ));
    parts.push(def(
        "PMOS",
        "P-channel MOSFET",
        "M",
        "Enhancement P-channel MOSFET (bulk tied to source)",
        Device::Pmos,
        vec![
            pin("D", "D", 3, 4, Passive),
            pin("G", "G", -4, 0, Passive),
            pin("S", "S", 3, -4, Passive),
        ],
        mos(),
        "",
        Some("PMOS"),
    ));

    let tri = || -> Vec<Shape> {
        vec![
            poly(&[(-4, -4), (-4, 4), (4, 0)], false),
            line(&[(-3, -2), (-1, -2)]),
            line(&[(-2, -3), (-2, -1)]),
            line(&[(-3, 2), (-1, 2)]),
        ]
    };
    parts.push(def(
        "OPAMP",
        "Op-amp",
        "U",
        "Operational amplifier model with output limits; supply rails are model parameters",
        Device::OpAmp,
        vec![
            pin("3", "+", -4, -2, Input),
            pin("2", "-", -4, 2, Input),
            pin("1", "OUT", 4, 0, Output),
        ],
        tri(),
        "",
        Some("OPAMP"),
    ));
    let mut tri_pwr = tri();
    tri_pwr.push(line(&[(0, -4), (0, -2)]));
    tri_pwr.push(line(&[(0, 4), (0, 2)]));
    parts.push(def("OPAMP_PWR", "Op-amp with supply pins", "U", "Operational amplifier with explicit supply pins (checked by the rules check; the model ignores them)", Device::OpAmp,
        vec![pin("3", "+", -4, -2, Input), pin("2", "-", -4, 2, Input), pin("1", "OUT", 4, 0, Output),
             pin("8", "V+", 0, -4, PowerIn), pin("4", "V-", 0, 4, PowerIn)], tri_pwr, "", Some("OPAMP")));

    let shapes = vec![
        line(&[(-4, -2), (-2, -2)]),
        line(&[(2, -2), (4, -2)]),
        line(&[(-2, -2), (2, -4)]),
        line(&[(-4, 2), (-3, 2)]),
        line(&[(3, 2), (4, 2)]),
        Shape::Rect {
            min: p(-3, 1),
            max: p(3, 3),
        },
    ];
    parts.push(def(
        "SWITCH",
        "Voltage-controlled switch",
        "S",
        "Switch closed by the voltage between C+ and C-, with hysteresis",
        Device::Switch,
        vec![
            pin("1", "1", -4, -2, Passive),
            pin("2", "2", 4, -2, Passive),
            pin("3", "C+", -4, 2, Input),
            pin("4", "C-", 4, 2, Input),
        ],
        shapes,
        "",
        Some("SW"),
    ));

    let mut shapes = two_terminal_leads();
    shapes.push(Shape::Rect {
        min: p(-1, -3),
        max: p(1, 3),
    });
    shapes.push(line(&[(4, 0), (1, 0)]));
    shapes.push(poly(&[(1, 0), (2, -1), (2, 1)], true));
    parts.push(def(
        "POT",
        "Potentiometer",
        "RV",
        "Potentiometer; field `wiper` (0 to 1, default 0.5) sets the wiper position from pin 1",
        Device::Potentiometer,
        vec![
            pin("1", "A", 0, -4, Passive),
            pin("2", "B", 0, 4, Passive),
            pin("3", "W", 4, 0, Passive),
        ],
        shapes,
        "10k",
        None,
    ));

    let mut shapes = vec![
        line(&[(-4, -3), (-2, -3)]),
        line(&[(-4, 3), (-2, 3)]),
        line(&[(2, -3), (4, -3)]),
        line(&[(2, 3), (4, 3)]),
    ];
    for k in 0..3 {
        shapes.push(Shape::Arc {
            center: p(-2, -2 + 2 * k),
            radius: 1,
            start_deg: 90,
            end_deg: 270,
        });
        shapes.push(Shape::Arc {
            center: p(2, -2 + 2 * k),
            radius: 1,
            start_deg: -90,
            end_deg: 90,
        });
    }
    shapes.push(line(&[(0, -3), (0, 3)]));
    parts.push(def("TRANSFORMER", "Transformer", "T", "Two coupled inductors; value is the primary inductance, fields `ratio` (turns N1:N2, default 1) and `k` (coupling, default 0.999)", Device::Transformer,
        vec![pin("P1", "P1", -4, -3, Passive), pin("P2", "P2", -4, 3, Passive), pin("S1", "S1", 4, -3, Passive), pin("S2", "S2", 4, 3, Passive)], shapes, "10m", None));

    let mut shapes = two_terminal_leads();
    shapes.push(Shape::Rect {
        min: p(-1, -3),
        max: p(1, 3),
    });
    shapes.push(line(&[(0, -3), (0, 3)]));
    parts.push(def(
        "FUSE",
        "Fuse",
        "F",
        "Fuse; value is the rating, field `rseries` (default 10m) its resistance",
        Device::Fuse,
        vec![pin("1", "~", 0, -4, Passive), pin("2", "~", 0, 4, Passive)],
        shapes,
        "1A",
        None,
    ));

    let power = vec![
        PowerSymbol {
            id: "GND",
            net: "0",
            style: PowerStyle::Ground,
            shapes: vec![
                line(&[(0, 0), (0, 2)]),
                line(&[(-2, 2), (2, 2)]),
                line(&[(-1, 3), (1, 3)]),
            ],
        },
        PowerSymbol {
            id: "VCC",
            net: "VCC",
            style: PowerStyle::Supply,
            shapes: vec![line(&[(0, 0), (0, -2)]), line(&[(-2, -2), (2, -2)])],
        },
    ];
    Library { parts, power }
}

impl Library {
    /// The built-in library, built once.
    #[must_use]
    pub fn builtin() -> &'static Library {
        static LIB: OnceLock<Library> = OnceLock::new();
        LIB.get_or_init(build)
    }

    /// A symbol by id (exact match).
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&PartDef> {
        self.parts.iter().find(|d| d.id == id)
    }

    /// All symbols.
    #[must_use]
    pub fn parts(&self) -> &[PartDef] {
        &self.parts
    }

    /// The power symbols (`GND`, `VCC`).
    #[must_use]
    pub fn power_symbols(&self) -> &[PowerSymbol] {
        &self.power
    }

    /// A generic default model by name (`D`, `LED`, `DZ`, `NPN`, `PNP`, `NMOS`, `PMOS`, `SW`,
    /// `OPAMP`). These are textbook-typical values, not tied to a real part number.
    #[must_use]
    pub fn default_model(name: &str) -> Option<Model> {
        Some(match name.to_ascii_uppercase().as_str() {
            "D" => Model::new("D", ModelKind::D)
                .with("IS", 1e-14)
                .with("N", 1.0)
                .with("RS", 1.0)
                .with("CJO", 2e-12)
                .with("TT", 5e-9),
            "LED" => Model::new("LED", ModelKind::D)
                .with("IS", 1e-18)
                .with("N", 2.0)
                .with("RS", 2.0)
                .with("CJO", 30e-12),
            "DZ" => Model::new("DZ", ModelKind::D)
                .with("IS", 1e-14)
                .with("N", 1.0)
                .with("RS", 2.0)
                .with("BV", 5.1)
                .with("IBV", 5e-3),
            "NPN" => Model::new("NPN", ModelKind::Npn)
                .with("IS", 1e-14)
                .with("BF", 100.0)
                .with("BR", 1.0)
                .with("VAF", 100.0)
                .with("CJE", 5e-12)
                .with("CJC", 3e-12)
                .with("TF", 0.4e-9),
            "PNP" => Model::new("PNP", ModelKind::Pnp)
                .with("IS", 1e-14)
                .with("BF", 100.0)
                .with("BR", 1.0)
                .with("VAF", 100.0)
                .with("CJE", 5e-12)
                .with("CJC", 3e-12)
                .with("TF", 0.4e-9),
            "NMOS" => Model::new("NMOS", ModelKind::Nmos)
                .with("VTO", 1.0)
                .with("KP", 2e-5)
                .with("LAMBDA", 0.02)
                .with("W", 100e-6)
                .with("L", 10e-6),
            "PMOS" => Model::new("PMOS", ModelKind::Pmos)
                .with("VTO", -1.0)
                .with("KP", 1e-5)
                .with("LAMBDA", 0.02)
                .with("W", 100e-6)
                .with("L", 10e-6),
            "SW" => Model::new("SW", ModelKind::Sw)
                .with("VT", 0.5)
                .with("VH", 0.0)
                .with("RON", 1.0)
                .with("ROFF", 1e9),
            "OPAMP" => Model::new("OPAMP", ModelKind::OpAmp)
                .with("GAIN", 1e5)
                .with("ROUT", 75.0)
                .with("VMAX", 13.0)
                .with("VMIN", -13.0),
            _ => return None,
        })
    }
}

/// Apply mirror (left-right) then rotation to a point relative to the origin.
#[must_use]
pub fn transform(pt: Point, rotation: Rotation, mirror: bool) -> Point {
    let (x, y) = if mirror { (-pt.x, pt.y) } else { (pt.x, pt.y) };
    let (rx, ry) = match rotation {
        Rotation::R0 => (x, y),
        Rotation::R90 => (y, -x),
        Rotation::R180 => (-x, -y),
        Rotation::R270 => (-y, x),
    };
    Point::new(rx, ry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn library_has_about_twenty_parts() {
        let lib = Library::builtin();
        assert!(lib.parts().len() + lib.power_symbols().len() >= 20);
        let ids: HashSet<_> = lib.parts().iter().map(|d| d.id).collect();
        assert_eq!(ids.len(), lib.parts().len(), "ids are unique");
        for need in [
            "R",
            "C",
            "C_POL",
            "L",
            "V",
            "I",
            "D",
            "LED",
            "ZENER",
            "NPN",
            "PNP",
            "NMOS",
            "PMOS",
            "OPAMP",
            "SWITCH",
            "POT",
            "TRANSFORMER",
            "FUSE",
        ] {
            assert!(lib.get(need).is_some(), "{need}");
        }
        assert_eq!(lib.power_symbols().len(), 2);
    }

    #[test]
    fn pins_are_distinct_and_on_the_grid() {
        for d in Library::builtin().parts() {
            let pts: HashSet<_> = d.pins.iter().map(|p| p.position).collect();
            assert_eq!(pts.len(), d.pins.len(), "{}: pins overlap", d.id);
            let nums: HashSet<_> = d.pins.iter().map(|p| p.number).collect();
            assert_eq!(nums.len(), d.pins.len(), "{}: pin numbers repeat", d.id);
            assert!(!d.shapes.is_empty());
            // Every pin's tip touches a shape end or lies on a shape (leads are drawn to the pin).
            for pin in &d.pins {
                let touches = d.shapes.iter().any(|s| match s {
                    Shape::Polyline { points, closed, .. } => {
                        let mut segs: Vec<(Point, Point)> =
                            points.windows(2).map(|w| (w[0], w[1])).collect();
                        if *closed {
                            if let (Some(f), Some(l)) = (points.first(), points.last()) {
                                segs.push((*l, *f));
                            }
                        }
                        points.contains(&pin.position)
                            || segs.iter().any(|(a, b)| {
                                let (px, py) =
                                    (i64::from(pin.position.x), i64::from(pin.position.y));
                                let (ax, ay, bx, by) = (
                                    i64::from(a.x),
                                    i64::from(a.y),
                                    i64::from(b.x),
                                    i64::from(b.y),
                                );
                                (bx - ax) * (py - ay) == (by - ay) * (px - ax)
                                    && px >= ax.min(bx)
                                    && px <= ax.max(bx)
                                    && py >= ay.min(by)
                                    && py <= ay.max(by)
                            })
                    }
                    _ => false,
                });
                assert!(
                    touches,
                    "{}: pin {} has no lead drawn to it",
                    d.id, pin.number
                );
            }
        }
    }

    #[test]
    fn default_models_exist_for_every_model_name() {
        for d in Library::builtin().parts() {
            if let Some(m) = d.default_model {
                assert!(Library::default_model(m).is_some(), "{m}");
            }
        }
        let led = Library::default_model("led").unwrap();
        assert!(led.unknown_params().is_empty());
        for n in [
            "D", "LED", "DZ", "NPN", "PNP", "NMOS", "PMOS", "SW", "OPAMP",
        ] {
            let m = Library::default_model(n).unwrap();
            assert!(
                m.unknown_params().is_empty(),
                "{n}: {:?}",
                m.unknown_params()
            );
        }
    }

    #[test]
    fn transforms_are_rotations() {
        let q = Point::new(2, -3);
        assert_eq!(transform(q, Rotation::R0, false), q);
        assert_eq!(transform(q, Rotation::R180, false), Point::new(-2, 3));
        let r = transform(transform(q, Rotation::R90, false), Rotation::R270, false);
        assert_eq!(r, q);
        assert_eq!(transform(q, Rotation::R0, true), Point::new(-2, -3));
    }
}

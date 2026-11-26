// SPDX-License-Identifier: MIT OR Apache-2.0
//! A small programmatic way to assemble a schematic: place library parts, then attach labels and
//! power ports directly at pin positions or join pins with wires.
//!
//! This is what the tests, the examples and any script use; the interactive editor edits the same
//! [`Schematic`] data.

use super::{
    Junction, Label, LabelScope, NoConnect, Part, Point, PowerPort, PowerStyle, Rotation,
    Schematic, Wire,
};
use crate::library::{transform, Library};
use std::collections::BTreeMap;

/// A handle to a placed part: its index on sheet 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartId(pub usize);

/// Assembles a one-sheet schematic.
#[derive(Debug, Clone)]
pub struct Builder {
    /// The schematic being built; public so tests can adjust it directly.
    pub schematic: Schematic,
}

impl Builder {
    /// Start an empty schematic.
    #[must_use]
    pub fn new(title: &str) -> Self {
        Builder {
            schematic: Schematic::new(title),
        }
    }

    /// Place a library part. The `value` is the value text (empty for parts that need none).
    pub fn place(&mut self, lib_id: &str, refdes: &str, value: &str, x: i32, y: i32) -> PartId {
        let sheet = &mut self.schematic.sheets[0];
        sheet.parts.push(Part {
            refdes: refdes.to_string(),
            lib_id: lib_id.to_string(),
            value: value.to_string(),
            position: Point::new(x, y),
            rotation: Rotation::R0,
            mirror: false,
            model: None,
            fields: BTreeMap::new(),
        });
        PartId(sheet.parts.len() - 1)
    }

    /// Rotate a placed part.
    pub fn rotate(&mut self, part: PartId, rotation: Rotation) {
        if let Some(p) = self.schematic.sheets[0].parts.get_mut(part.0) {
            p.rotation = rotation;
        }
    }

    /// Set a named field on a part.
    pub fn field(&mut self, part: PartId, key: &str, value: &str) {
        if let Some(p) = self.schematic.sheets[0].parts.get_mut(part.0) {
            p.fields.insert(key.to_string(), value.to_string());
        }
    }

    /// Set a part's model name.
    pub fn model(&mut self, part: PartId, name: &str) {
        if let Some(p) = self.schematic.sheets[0].parts.get_mut(part.0) {
            p.model = Some(name.to_string());
        }
    }

    /// Absolute position of a pin, by pin number or name (`"1"`, `"K"`, `"OUT"`). `None` when the
    /// part or pin does not exist.
    #[must_use]
    pub fn pin(&self, part: PartId, pin: &str) -> Option<Point> {
        let p = self.schematic.sheets[0].parts.get(part.0)?;
        let def = Library::builtin().get(&p.lib_id)?;
        let pd = def
            .pins
            .iter()
            .find(|d| d.number == pin)
            .or_else(|| def.pins.iter().find(|d| d.name == pin))?;
        let o = transform(pd.position, p.rotation, p.mirror);
        Some(Point::new(p.position.x + o.x, p.position.y + o.y))
    }

    /// Put a net label exactly on a pin.
    pub fn label_pin(&mut self, part: PartId, pin: &str, name: &str) {
        if let Some(at) = self.pin(part, pin) {
            self.label(at, name, LabelScope::Local);
        }
    }

    /// Put a power port (ground for `0`/`GND`) exactly on a pin.
    pub fn power_pin(&mut self, part: PartId, pin: &str, name: &str) {
        if let Some(at) = self.pin(part, pin) {
            self.power(at, name);
        }
    }

    /// Flag a pin as deliberately unconnected.
    pub fn no_connect_pin(&mut self, part: PartId, pin: &str) {
        if let Some(at) = self.pin(part, pin) {
            self.schematic.sheets[0]
                .no_connects
                .push(NoConnect { position: at });
        }
    }

    /// Add a label at a point.
    pub fn label(&mut self, at: Point, name: &str, scope: LabelScope) {
        self.schematic.sheets[0].labels.push(Label {
            name: name.to_string(),
            position: at,
            scope,
        });
    }

    /// Add a power port at a point.
    pub fn power(&mut self, at: Point, name: &str) {
        let style = if matches!(name.to_lowercase().as_str(), "0" | "gnd") {
            PowerStyle::Ground
        } else {
            PowerStyle::Supply
        };
        self.schematic.sheets[0].power_ports.push(PowerPort {
            name: name.to_string(),
            position: at,
            style,
        });
    }

    /// Add a junction at a point.
    pub fn junction(&mut self, at: Point) {
        self.schematic.sheets[0]
            .junctions
            .push(Junction { position: at });
    }

    /// Add a wire through the given points.
    pub fn wire(&mut self, points: &[Point]) {
        self.schematic.sheets[0].wires.push(Wire {
            points: points.to_vec(),
        });
    }

    /// Join two pins with an L-shaped wire (horizontal first, then vertical).
    pub fn connect(&mut self, a: (PartId, &str), b: (PartId, &str)) {
        if let (Some(pa), Some(pb)) = (self.pin(a.0, a.1), self.pin(b.0, b.1)) {
            if pa.x == pb.x || pa.y == pb.y {
                self.wire(&[pa, pb]);
            } else {
                self.wire(&[pa, Point::new(pb.x, pa.y), pb]);
            }
        }
    }

    /// Finish and return the schematic.
    #[must_use]
    pub fn build(self) -> Schematic {
        self.schematic
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schematic::resolve_nets;

    #[test]
    fn pins_are_found_by_number_or_name() {
        let mut b = Builder::new("t");
        let d = b.place("D", "D1", "", 0, 0);
        assert_eq!(b.pin(d, "A"), Some(Point::new(0, -4)));
        assert_eq!(b.pin(d, "K"), Some(Point::new(0, 4)));
        assert_eq!(b.pin(d, "Z"), None);
        assert_eq!(b.pin(PartId(9), "A"), None);
    }

    #[test]
    fn connect_joins_pins() {
        let mut b = Builder::new("t");
        let r1 = b.place("R", "R1", "1k", 0, 0);
        let r2 = b.place("R", "R2", "1k", 20, 0);
        b.connect((r1, "2"), (r2, "1"));
        let s = b.build();
        let r = resolve_nets(&s, Library::builtin()).unwrap();
        assert_eq!(r.nets.iter().filter(|n| n.pins.len() == 2).count(), 1);
    }
}

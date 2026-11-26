// SPDX-License-Identifier: MIT OR Apache-2.0
//! The parts palette: which library parts are offered, how they are grouped and what they are
//! called in the interface's language. Drawing the palette is the window's job.

use circuito_core::library::Library;

use crate::i18n::tr;

/// One part the palette offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The library id (`R`, `NPN`…).
    pub id: &'static str,
}

/// A titled run of parts.
#[derive(Clone, Copy, Debug)]
pub struct Group {
    pub title: &'static str,
    pub entries: &'static [Entry],
}

/// What is dragged out of the palette onto the drawing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    /// A library part.
    Part(&'static str),
    /// A power port of this name.
    Power(String),
}

const fn e(id: &'static str) -> Entry {
    Entry { id }
}

/// The groups, in the order the palette lists them. The titles are English; see [`Group::label`].
pub const GROUPS: [Group; 4] = [
    Group {
        title: "Passive",
        entries: &[
            e("R"),
            e("C"),
            e("C_POL"),
            e("L"),
            e("POT"),
            e("TRANSFORMER"),
            e("FUSE"),
        ],
    },
    Group {
        title: "Sources",
        entries: &[e("V"), e("I")],
    },
    Group {
        title: "Semiconductors",
        entries: &[
            e("D"),
            e("LED"),
            e("ZENER"),
            e("NPN"),
            e("PNP"),
            e("NMOS"),
            e("PMOS"),
        ],
    },
    Group {
        title: "Analog and switching",
        entries: &[e("OPAMP"), e("OPAMP_PWR"), e("SWITCH")],
    },
];

/// The power ports the palette offers: the net each puts its pin on.
pub const POWER_PORTS: [&str; 5] = ["GND", "VCC", "+5V", "+3.3V", "+12V"];

impl Group {
    /// The title in the interface's language.
    pub fn label(&self) -> &'static str {
        match self.title {
            "Passive" => tr("Passive"),
            "Sources" => tr("Sources"),
            "Semiconductors" => tr("Semiconductors"),
            _ => tr("Analog and switching"),
        }
    }
}

impl Entry {
    /// The part's name in the interface's language.
    pub fn label(&self) -> &'static str {
        match self.id {
            "R" => tr("Resistor"),
            "C" => tr("Capacitor"),
            "C_POL" => tr("Polarised capacitor"),
            "L" => tr("Inductor"),
            "POT" => tr("Potentiometer"),
            "TRANSFORMER" => tr("Transformer"),
            "FUSE" => tr("Fuse"),
            "V" => tr("Voltage source"),
            "I" => tr("Current source"),
            "D" => tr("Diode"),
            "LED" => tr("LED"),
            "ZENER" => tr("Zener diode"),
            "NPN" => tr("NPN transistor"),
            "PNP" => tr("PNP transistor"),
            "NMOS" => tr("N-channel MOSFET"),
            "PMOS" => tr("P-channel MOSFET"),
            "OPAMP" => tr("Op-amp"),
            "OPAMP_PWR" => tr("Op-amp with supply pins"),
            "SWITCH" => tr("Switch"),
            _ => "",
        }
    }

    /// Whether the library has the part (it always does for the entries above).
    pub fn exists(&self) -> bool {
        Library::builtin().get(self.id).is_some()
    }

    /// Whether `needle` (lower case) is in the part's id or either language's name.
    pub fn matches(&self, needle: &str) -> bool {
        needle.is_empty()
            || self.id.to_lowercase().contains(needle)
            || self.label().to_lowercase().contains(needle)
    }
}

/// The name of a power port as the palette shows it.
pub fn power_label(name: &str) -> String {
    if name == "GND" {
        tr("Ground").to_owned()
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_a_library_part_with_a_name() {
        let mut seen = std::collections::HashSet::new();
        for group in GROUPS {
            for entry in group.entries {
                assert!(entry.exists(), "{} is not in the library", entry.id);
                assert!(!entry.label().is_empty(), "{} has no name", entry.id);
                assert!(seen.insert(entry.id), "{} twice", entry.id);
            }
        }
    }

    #[test]
    fn every_library_part_is_offered() {
        let offered: std::collections::HashSet<&str> = GROUPS
            .iter()
            .flat_map(|g| g.entries.iter().map(|e| e.id))
            .collect();
        for def in Library::builtin().parts() {
            assert!(offered.contains(def.id), "{} is not in the palette", def.id);
        }
    }

    #[test]
    fn the_search_finds_by_id_and_by_name() {
        let r = GROUPS[0].entries[0];
        assert!(r.matches("res") && r.matches("r") && r.matches(""));
        assert!(!r.matches("zzz"));
    }
}

// SPDX-License-Identifier: MIT OR Apache-2.0
//! Design rules and manufacturer capability profiles.
//!
//! The two built-in profiles carry Circuito's own conservative generic values; they are not copied
//! from any fabricator's published table. Check a real fabricator's current capabilities before
//! ordering boards.

use serde::{Deserialize, Serialize};

use super::violation::ViolationCode;
use crate::error::{PcbError, Result};
use crate::geom::arc::DEFAULT_ARC_TOLERANCE;
use crate::units::{check_dim, Nm, MIL};

/// Minimum copper clearance between objects on different nets, by pair of kinds. A net class may
/// raise (never lower) the value for pairs involving its nets, and a zone's own clearance applies
/// to pairs involving that zone: the requirement is the largest of the applicable values.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearanceTable {
    pub track_track: Nm,
    pub track_pad: Nm,
    pub track_via: Nm,
    pub pad_pad: Nm,
    pub pad_via: Nm,
    pub via_via: Nm,
    /// Zone copper against any other copper.
    pub zone_copper: Nm,
}

impl ClearanceTable {
    pub fn uniform(v: Nm) -> ClearanceTable {
        ClearanceTable {
            track_track: v,
            track_pad: v,
            track_via: v,
            pad_pad: v,
            pad_via: v,
            via_via: v,
            zone_copper: v,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub min_track_width: Nm,
    pub clearance: ClearanceTable,
    /// Minimum distance from copper to the board edge (and to cut-outs).
    pub copper_to_edge: Nm,
    pub min_annular_ring: Nm,
    /// Smallest drill for pads (round hole diameter or slot width) and plated vias.
    pub min_drill: Nm,
    pub min_via_diameter: Nm,
    pub min_via_drill: Nm,
    pub min_microvia_diameter: Nm,
    pub min_microvia_drill: Nm,
    /// Minimum distance between the edges of two holes.
    pub hole_to_hole: Nm,
    /// Minimum distance between silkscreen and exposed pads.
    pub silk_to_pad: Nm,
    pub min_silk_width: Nm,
    /// Mask opening expansion for pads that do not set their own.
    pub mask_margin: Nm,
    /// Minimum width of solder mask between two openings.
    pub min_mask_web: Nm,
    pub check_courtyards: bool,
    /// Report tracks of one net meeting at less than 90 degrees.
    pub reject_acute_traces: bool,
    pub check_dangling: bool,
    /// Largest deviation (nm) allowed when arcs are approximated for checks.
    pub arc_tolerance: Nm,
    /// Stop after this many violations (the report says so).
    pub max_violations: usize,
    /// Rules to skip.
    pub disabled: Vec<ViolationCode>,
}

impl Default for Rules {
    fn default() -> Self {
        CapabilityProfile::generic_2layer().rules()
    }
}

impl Rules {
    pub fn validate(&self) -> Result<()> {
        let c = &self.clearance;
        for (n, v) in [
            ("minimum track width", self.min_track_width),
            ("track-track clearance", c.track_track),
            ("track-pad clearance", c.track_pad),
            ("track-via clearance", c.track_via),
            ("pad-pad clearance", c.pad_pad),
            ("pad-via clearance", c.pad_via),
            ("via-via clearance", c.via_via),
            ("zone clearance", c.zone_copper),
            ("copper to edge", self.copper_to_edge),
            ("annular ring", self.min_annular_ring),
            ("minimum drill", self.min_drill),
            ("minimum via diameter", self.min_via_diameter),
            ("minimum via drill", self.min_via_drill),
            ("minimum microvia diameter", self.min_microvia_diameter),
            ("minimum microvia drill", self.min_microvia_drill),
            ("hole to hole", self.hole_to_hole),
            ("silk to pad", self.silk_to_pad),
            ("minimum silk width", self.min_silk_width),
            ("mask web", self.min_mask_web),
        ] {
            check_dim(v, &format!("rule {n}"))?;
        }
        if self.mask_margin.abs() > crate::units::MAX_DIM {
            return Err(PcbError::range("rule mask margin", "out of range"));
        }
        if self.arc_tolerance < 1 || self.arc_tolerance > 1_000_000 {
            return Err(PcbError::range(
                "rule arc tolerance",
                format!("{} nm is outside 1..=1000000", self.arc_tolerance),
            ));
        }
        if self.max_violations == 0 {
            return Err(PcbError::range("rule max violations", "must be at least 1"));
        }
        Ok(())
    }

    pub(crate) fn enabled(&self, c: ViolationCode) -> bool {
        !self.disabled.contains(&c)
    }
}

/// A manufacturing capability profile: the limits a board has to respect to be made by a process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityProfile {
    pub name: String,
    pub description: String,
    pub max_copper_layers: u8,
    pub allow_blind_buried_vias: bool,
    pub min_track_width: Nm,
    pub min_clearance: Nm,
    pub copper_to_edge: Nm,
    pub min_annular_ring: Nm,
    pub min_drill: Nm,
    pub min_via_diameter: Nm,
    pub min_via_drill: Nm,
    pub hole_to_hole: Nm,
    pub min_mask_web: Nm,
    pub silk_to_pad: Nm,
    pub min_silk_width: Nm,
}

impl CapabilityProfile {
    /// A generic two-layer process: 6 mil (0.1524 mm) tracks and spaces. Values are Circuito's own
    /// conservative choices.
    pub fn generic_2layer() -> CapabilityProfile {
        CapabilityProfile {
            name: "generic-2layer-6-6".into(),
            description: "Generic 2-layer process, 6 mil track and 6 mil space (Circuito's own conservative values)".into(),
            max_copper_layers: 2,
            allow_blind_buried_vias: false,
            min_track_width: 6 * MIL,
            min_clearance: 6 * MIL,
            copper_to_edge: 300_000,
            min_annular_ring: 150_000,
            min_drill: 300_000,
            min_via_diameter: 600_000,
            min_via_drill: 300_000,
            hole_to_hole: 250_000,
            min_mask_web: 100_000,
            silk_to_pad: 100_000,
            min_silk_width: 150_000,
        }
    }

    /// A generic four-layer process: 5 mil (0.127 mm) tracks and spaces. Values are Circuito's own
    /// conservative choices.
    pub fn generic_4layer() -> CapabilityProfile {
        CapabilityProfile {
            name: "generic-4layer-5-5".into(),
            description: "Generic 4-layer process, 5 mil track and 5 mil space (Circuito's own conservative values)".into(),
            max_copper_layers: 4,
            allow_blind_buried_vias: false,
            min_track_width: 5 * MIL,
            min_clearance: 5 * MIL,
            copper_to_edge: 250_000,
            min_annular_ring: 125_000,
            min_drill: 200_000,
            min_via_diameter: 450_000,
            min_via_drill: 200_000,
            hole_to_hole: 200_000,
            min_mask_web: 100_000,
            silk_to_pad: 100_000,
            min_silk_width: 150_000,
        }
    }

    /// All built-in profiles.
    pub fn builtin() -> Vec<CapabilityProfile> {
        vec![Self::generic_2layer(), Self::generic_4layer()]
    }

    /// Looks a built-in profile up by name.
    pub fn by_name(name: &str) -> Option<CapabilityProfile> {
        Self::builtin().into_iter().find(|p| p.name == name)
    }

    /// The design rules that make a board respect this profile.
    pub fn rules(&self) -> Rules {
        Rules {
            min_track_width: self.min_track_width,
            clearance: ClearanceTable::uniform(self.min_clearance),
            copper_to_edge: self.copper_to_edge,
            min_annular_ring: self.min_annular_ring,
            min_drill: self.min_drill,
            min_via_diameter: self.min_via_diameter,
            min_via_drill: self.min_via_drill,
            min_microvia_diameter: self.min_via_diameter,
            min_microvia_drill: self.min_via_drill,
            hole_to_hole: self.hole_to_hole,
            silk_to_pad: self.silk_to_pad,
            min_silk_width: self.min_silk_width,
            mask_margin: 50_000,
            min_mask_web: self.min_mask_web,
            check_courtyards: true,
            reject_acute_traces: true,
            check_dangling: true,
            arc_tolerance: DEFAULT_ARC_TOLERANCE,
            max_violations: 100_000,
            disabled: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_validate_and_are_ordered() {
        let a = CapabilityProfile::generic_2layer();
        let b = CapabilityProfile::generic_4layer();
        a.rules().validate().unwrap();
        b.rules().validate().unwrap();
        assert!(b.min_clearance < a.min_clearance);
        assert_eq!(a.min_track_width, 152_400);
        assert_eq!(b.min_track_width, 127_000);
        assert_eq!(CapabilityProfile::by_name("generic-4layer-5-5"), Some(b));
        assert!(CapabilityProfile::by_name("nope").is_none());
        let r = Rules {
            arc_tolerance: 0,
            ..Rules::default()
        };
        assert!(r.validate().is_err());
    }

    #[test]
    fn rules_round_trip_json() {
        let r = Rules::default();
        let j = serde_json::to_string(&r).unwrap();
        assert_eq!(serde_json::from_str::<Rules>(&j).unwrap(), r);
    }
}

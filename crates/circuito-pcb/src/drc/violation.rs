// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the design rules check reports: stable codes, severities and located, measured findings.

use serde::{Deserialize, Serialize};

use crate::board::Id;
use crate::geom::Point;
use crate::layer::Layer;
use crate::units::Nm;

/// Stable identifier of a rule. The string form (`as_str`) never changes between releases; the app
/// translates messages from the code and the numbers.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ViolationCode {
    #[serde(rename = "track.width")]
    TrackWidth,
    #[serde(rename = "clearance.track_track")]
    ClearanceTrackTrack,
    #[serde(rename = "clearance.track_pad")]
    ClearanceTrackPad,
    #[serde(rename = "clearance.track_via")]
    ClearanceTrackVia,
    #[serde(rename = "clearance.pad_pad")]
    ClearancePadPad,
    #[serde(rename = "clearance.pad_via")]
    ClearancePadVia,
    #[serde(rename = "clearance.via_via")]
    ClearanceViaVia,
    #[serde(rename = "clearance.zone_copper")]
    ClearanceZoneCopper,
    #[serde(rename = "short.circuit")]
    ShortCircuit,
    #[serde(rename = "connectivity.unconnected")]
    Unconnected,
    #[serde(rename = "track.dangling")]
    TrackDangling,
    #[serde(rename = "track.acute_angle")]
    TrackAcuteAngle,
    #[serde(rename = "via.diameter")]
    ViaDiameter,
    #[serde(rename = "via.annular_ring")]
    ViaAnnularRing,
    #[serde(rename = "pad.annular_ring")]
    PadAnnularRing,
    #[serde(rename = "drill.min")]
    DrillMin,
    #[serde(rename = "drill.hole_to_hole")]
    HoleToHole,
    #[serde(rename = "silk.over_pad")]
    SilkOverPad,
    #[serde(rename = "silk.width")]
    SilkWidth,
    #[serde(rename = "mask.bridge")]
    MaskBridge,
    #[serde(rename = "mask.sliver")]
    MaskSliver,
    #[serde(rename = "courtyard.overlap")]
    CourtyardOverlap,
    #[serde(rename = "edge.clearance")]
    EdgeClearance,
    #[serde(rename = "edge.outside")]
    EdgeOutside,
    #[serde(rename = "outline.missing")]
    OutlineMissing,
    #[serde(rename = "keepout.track")]
    KeepoutTrack,
    #[serde(rename = "keepout.via")]
    KeepoutVia,
    #[serde(rename = "keepout.pad")]
    KeepoutPad,
    #[serde(rename = "keepout.zone")]
    KeepoutZone,
    #[serde(rename = "zone.unfilled")]
    ZoneUnfilled,
    #[serde(rename = "profile.layers")]
    ProfileLayers,
    #[serde(rename = "profile.via_type")]
    ProfileViaType,
}

impl ViolationCode {
    /// The stable dotted code, for example `clearance.track_track`.
    pub fn as_str(self) -> &'static str {
        use ViolationCode::*;
        match self {
            TrackWidth => "track.width",
            ClearanceTrackTrack => "clearance.track_track",
            ClearanceTrackPad => "clearance.track_pad",
            ClearanceTrackVia => "clearance.track_via",
            ClearancePadPad => "clearance.pad_pad",
            ClearancePadVia => "clearance.pad_via",
            ClearanceViaVia => "clearance.via_via",
            ClearanceZoneCopper => "clearance.zone_copper",
            ShortCircuit => "short.circuit",
            Unconnected => "connectivity.unconnected",
            TrackDangling => "track.dangling",
            TrackAcuteAngle => "track.acute_angle",
            ViaDiameter => "via.diameter",
            ViaAnnularRing => "via.annular_ring",
            PadAnnularRing => "pad.annular_ring",
            DrillMin => "drill.min",
            HoleToHole => "drill.hole_to_hole",
            SilkOverPad => "silk.over_pad",
            SilkWidth => "silk.width",
            MaskBridge => "mask.bridge",
            MaskSliver => "mask.sliver",
            CourtyardOverlap => "courtyard.overlap",
            EdgeClearance => "edge.clearance",
            EdgeOutside => "edge.outside",
            OutlineMissing => "outline.missing",
            KeepoutTrack => "keepout.track",
            KeepoutVia => "keepout.via",
            KeepoutPad => "keepout.pad",
            KeepoutZone => "keepout.zone",
            ZoneUnfilled => "zone.unfilled",
            ProfileLayers => "profile.layers",
            ProfileViaType => "profile.via_type",
        }
    }

    /// The severity this rule reports with.
    pub fn severity(self) -> Severity {
        use ViolationCode::*;
        match self {
            TrackDangling | TrackAcuteAngle | SilkOverPad | SilkWidth | MaskSliver
            | ZoneUnfilled => Severity::Warning,
            _ => Severity::Error,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjKind {
    Track,
    Via,
    Pad,
    Zone,
    Footprint,
    Keepout,
    Graphic,
    Text,
    Net,
    Board,
}

/// A reference to a board object by kind and id.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ObjRef {
    pub kind: ObjKind,
    pub id: Id,
}

/// One finding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    pub code: ViolationCode,
    pub severity: Severity,
    /// The copper (or other) layer concerned, when one applies.
    pub layer: Option<Layer>,
    /// Marker positions in nm (an approximate point near the problem; a pad centre for connectivity).
    pub locations: Vec<Point>,
    /// The objects involved, in a stable order.
    pub objects: Vec<ObjRef>,
    /// The net concerned, by name.
    pub net: Option<String>,
    /// What was measured (nm) and what the rule requires (nm), when the rule is a size or distance.
    pub measured: Option<Nm>,
    pub required: Option<Nm>,
    /// English description for logs; the app builds its translated text from the other fields.
    pub message: String,
}

impl Violation {
    pub(crate) fn sort_key(&self) -> (ViolationCode, Option<Layer>, Vec<ObjRef>, Vec<Point>) {
        (
            self.code,
            self.layer,
            self.objects.clone(),
            self.locations.clone(),
        )
    }
}

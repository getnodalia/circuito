// SPDX-License-Identifier: MIT OR Apache-2.0
//! Board layers and the side of the board.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{PcbError, Result};

/// Most copper layers a board may have.
pub const MAX_COPPER_LAYERS: u8 = 32;

/// A board layer. Copper layers are `F.Cu`, `In1.Cu`..`In30.Cu` and `B.Cu`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    FCu,
    /// Inner copper layer `1..=30`, counted from the top.
    InnerCu(u8),
    BCu,
    FMask,
    BMask,
    FPaste,
    BPaste,
    FSilk,
    BSilk,
    FCourtyard,
    BCourtyard,
    FFab,
    BFab,
    EdgeCuts,
    /// Drill drawing layer (annotations only; drill data comes from pads and vias).
    Drill,
}

/// Top or bottom of the board.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Front,
    Back,
}

impl Side {
    pub fn flip(self) -> Side {
        match self {
            Side::Front => Side::Back,
            Side::Back => Side::Front,
        }
    }
    pub fn copper(self) -> Layer {
        match self {
            Side::Front => Layer::FCu,
            Side::Back => Layer::BCu,
        }
    }
    pub fn mask(self) -> Layer {
        match self {
            Side::Front => Layer::FMask,
            Side::Back => Layer::BMask,
        }
    }
    pub fn paste(self) -> Layer {
        match self {
            Side::Front => Layer::FPaste,
            Side::Back => Layer::BPaste,
        }
    }
    pub fn silk(self) -> Layer {
        match self {
            Side::Front => Layer::FSilk,
            Side::Back => Layer::BSilk,
        }
    }
    pub fn courtyard(self) -> Layer {
        match self {
            Side::Front => Layer::FCourtyard,
            Side::Back => Layer::BCourtyard,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Side::Front => "front",
            Side::Back => "back",
        }
    }
}

impl Layer {
    pub fn is_copper(self) -> bool {
        matches!(self, Layer::FCu | Layer::InnerCu(_) | Layer::BCu)
    }

    /// The board side of an outer layer, `None` for inner copper and board-wide layers.
    pub fn side(self) -> Option<Side> {
        match self {
            Layer::FCu
            | Layer::FMask
            | Layer::FPaste
            | Layer::FSilk
            | Layer::FCourtyard
            | Layer::FFab => Some(Side::Front),
            Layer::BCu
            | Layer::BMask
            | Layer::BPaste
            | Layer::BSilk
            | Layer::BCourtyard
            | Layer::BFab => Some(Side::Back),
            _ => None,
        }
    }

    /// The same layer on the other side of the board (`F.SilkS` becomes `B.SilkS`); layers that are
    /// not tied to a side (inner copper, edge cuts, drill) are returned unchanged.
    pub fn flipped(self) -> Layer {
        match self {
            Layer::FCu => Layer::BCu,
            Layer::BCu => Layer::FCu,
            Layer::FMask => Layer::BMask,
            Layer::BMask => Layer::FMask,
            Layer::FPaste => Layer::BPaste,
            Layer::BPaste => Layer::FPaste,
            Layer::FSilk => Layer::BSilk,
            Layer::BSilk => Layer::FSilk,
            Layer::FCourtyard => Layer::BCourtyard,
            Layer::BCourtyard => Layer::FCourtyard,
            Layer::FFab => Layer::BFab,
            Layer::BFab => Layer::FFab,
            other => other,
        }
    }

    /// Position in the copper stack for a board with `copper_layers` copper layers
    /// (`F.Cu` is 0, `B.Cu` is the last). `None` for non-copper layers or inner layers the board lacks.
    pub fn copper_index(self, copper_layers: u8) -> Option<usize> {
        match self {
            Layer::FCu => Some(0),
            Layer::BCu if copper_layers >= 2 => Some(copper_layers as usize - 1),
            Layer::InnerCu(n) if n >= 1 && (n as usize) + 1 < copper_layers as usize => {
                Some(n as usize)
            }
            _ => None,
        }
    }

    /// The copper layers of a board, top to bottom.
    pub fn copper_stack(copper_layers: u8) -> Vec<Layer> {
        let n = copper_layers.clamp(1, MAX_COPPER_LAYERS);
        let mut v = vec![Layer::FCu];
        if n >= 2 {
            v.extend((1..n - 1).map(Layer::InnerCu));
            v.push(Layer::BCu);
        }
        v
    }

    /// The canonical name (`F.Cu`, `In3.Cu`, `Edge.Cuts`...).
    pub fn name(self) -> String {
        match self {
            Layer::FCu => "F.Cu".into(),
            Layer::InnerCu(n) => format!("In{n}.Cu"),
            Layer::BCu => "B.Cu".into(),
            Layer::FMask => "F.Mask".into(),
            Layer::BMask => "B.Mask".into(),
            Layer::FPaste => "F.Paste".into(),
            Layer::BPaste => "B.Paste".into(),
            Layer::FSilk => "F.SilkS".into(),
            Layer::BSilk => "B.SilkS".into(),
            Layer::FCourtyard => "F.CrtYd".into(),
            Layer::BCourtyard => "B.CrtYd".into(),
            Layer::FFab => "F.Fab".into(),
            Layer::BFab => "B.Fab".into(),
            Layer::EdgeCuts => "Edge.Cuts".into(),
            Layer::Drill => "Drill".into(),
        }
    }

    /// Parses a canonical layer name.
    pub fn parse(s: &str) -> Result<Layer> {
        let l = match s {
            "F.Cu" => Layer::FCu,
            "B.Cu" => Layer::BCu,
            "F.Mask" => Layer::FMask,
            "B.Mask" => Layer::BMask,
            "F.Paste" => Layer::FPaste,
            "B.Paste" => Layer::BPaste,
            "F.SilkS" => Layer::FSilk,
            "B.SilkS" => Layer::BSilk,
            "F.CrtYd" => Layer::FCourtyard,
            "B.CrtYd" => Layer::BCourtyard,
            "F.Fab" => Layer::FFab,
            "B.Fab" => Layer::BFab,
            "Edge.Cuts" => Layer::EdgeCuts,
            "Drill" => Layer::Drill,
            other => {
                let inner = other
                    .strip_prefix("In")
                    .and_then(|r| r.strip_suffix(".Cu"))
                    .and_then(|d| {
                        if d.bytes().all(|c| c.is_ascii_digit()) && !d.starts_with('0') {
                            d.parse::<u8>().ok()
                        } else {
                            None
                        }
                    });
                match inner {
                    Some(n) if (1..=MAX_COPPER_LAYERS - 2).contains(&n) => Layer::InnerCu(n),
                    _ => return Err(PcbError::Parse { what: "layer name", detail: format!("`{s}` is not a layer name (expected F.Cu, In1.Cu..In30.Cu, B.Cu, F.Mask, B.Mask, F.Paste, B.Paste, F.SilkS, B.SilkS, F.CrtYd, B.CrtYd, F.Fab, B.Fab, Edge.Cuts or Drill)") }),
                }
            }
        };
        Ok(l)
    }
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

impl Serialize for Layer {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.name())
    }
}

impl<'de> Deserialize<'de> for Layer {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Layer, D::Error> {
        let s = String::deserialize(d)?;
        Layer::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        let mut all = Layer::copper_stack(32);
        all.extend([
            Layer::FMask,
            Layer::BMask,
            Layer::FPaste,
            Layer::BPaste,
            Layer::FSilk,
            Layer::BSilk,
            Layer::FCourtyard,
            Layer::BCourtyard,
            Layer::FFab,
            Layer::BFab,
            Layer::EdgeCuts,
            Layer::Drill,
        ]);
        for l in all {
            assert_eq!(Layer::parse(&l.name()).unwrap(), l);
        }
        for bad in [
            "", "In0.Cu", "In31.Cu", "In01.Cu", "In1", "f.cu", "In1.Cu ", "In-1.Cu",
        ] {
            assert!(Layer::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn copper_indices() {
        assert_eq!(Layer::FCu.copper_index(4), Some(0));
        assert_eq!(Layer::InnerCu(1).copper_index(4), Some(1));
        assert_eq!(Layer::InnerCu(2).copper_index(4), Some(2));
        assert_eq!(Layer::InnerCu(3).copper_index(4), None);
        assert_eq!(Layer::BCu.copper_index(4), Some(3));
        assert_eq!(Layer::BCu.copper_index(1), None);
        assert_eq!(
            Layer::copper_stack(4),
            vec![Layer::FCu, Layer::InnerCu(1), Layer::InnerCu(2), Layer::BCu]
        );
        assert_eq!(Layer::copper_stack(1), vec![Layer::FCu]);
    }
}

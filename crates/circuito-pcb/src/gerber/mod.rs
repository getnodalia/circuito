// SPDX-License-Identifier: MIT OR Apache-2.0
//! Gerber RS-274X / X2 output, written from the public Ucamco specification, and a small reader
//! of our own that proves the files say what the board says.
//!
//! # What is written
//!
//! One file per layer: every copper layer, solder mask and paste on both sides, silkscreen on both
//! sides and the board profile. Format `FSLAX46Y46`, units `MOMM`: a coordinate integer is exactly
//! a count of nanometres, so **no coordinate is ever rounded**. Aperture sizes are written as
//! millimetres with six decimals, which is also exact. File and aperture and object attributes
//! follow the X2 specification: `TF.GenerationSoftware`, `TF.SameCoordinates`, `TF.FileFunction`,
//! `TF.FilePolarity`, optional `TF.CreationDate` (only when the caller supplies one, so output is
//! byte-for-byte reproducible), `TA.AperFunction` and `TO.N`, `TO.C`, `TO.P`.
//!
//! Apertures: `C`, `R`, `O` and two macros of our own, `RECTROT` (a rectangle at any angle) and
//! `RRECT` (a rounded rectangle at any angle). Arcs are written as true `G75` arcs, never as
//! polylines. A zone is written as filled regions (`G36`/`G37`), each hole as a clear-polarity
//! region (`LPC`). Filled zone geometry must already exist in the board: this crate does not fill
//! zones, and an unfilled zone is reported as a warning and left out.

pub mod layers;
pub mod read;
pub(crate) mod writer;

pub use layers::{layer_list, write_all, write_layer};
pub use read::{
    read_gerber, Aperture, AttrList, GKind, GObject, GRegion, GerberImage, MacroDef, Polarity,
    MAX_GERBER_BYTES,
};

use crate::error::{PcbError, Result};
use crate::units::{Nm, MAX_DIM};

/// Options of the Gerber writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GerberOptions {
    /// Written as `TF.CreationDate` when set (ISO 8601 text); otherwise no date is written.
    pub creation_date: Option<String>,
    /// Stroke width of the board profile, nm.
    pub profile_width: Nm,
    /// Solder mask expansion of pads and vias without their own value, nm.
    pub mask_margin: Nm,
}

impl Default for GerberOptions {
    fn default() -> Self {
        GerberOptions {
            creation_date: None,
            profile_width: 100_000,
            mask_margin: 50_000,
        }
    }
}

impl GerberOptions {
    pub(crate) fn validate(&self) -> Result<()> {
        if !(1..=MAX_DIM).contains(&self.profile_width) {
            return Err(PcbError::range(
                "profile width",
                format!("{} nm is outside 1..={MAX_DIM}", self.profile_width),
            ));
        }
        if self.mask_margin.abs() > MAX_DIM {
            return Err(PcbError::range(
                "mask margin",
                format!("{} nm is outside -{MAX_DIM}..={MAX_DIM}", self.mask_margin),
            ));
        }
        Ok(())
    }
}

/// One Gerber file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GerberFile {
    /// A suggested file name, `<board>-<layer>.gbr`.
    pub file_name: String,
    /// The `TF.FileFunction` value, for example `Copper,L1,Top`.
    pub function: String,
    /// `Positive` or `Negative`.
    pub polarity: String,
    pub content: String,
}

/// The files of a board and what could not be drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GerberSet {
    pub files: Vec<GerberFile>,
    pub warnings: Vec<String>,
}

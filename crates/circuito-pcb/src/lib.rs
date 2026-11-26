// SPDX-License-Identifier: MIT OR Apache-2.0
//! Circuito PCB engine: the board model, exact integer geometry, the design rules check, and
//! manufacturing outputs (Gerber RS-274X/X2, Excellon, pick-and-place, BOM).
//!
//! Every coordinate is an integer count of nanometres. See the crate README for what is verified,
//! what is not, and the limits.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod artwork;
pub mod assembly;
pub mod board;
pub mod drc;
pub mod error;
pub mod excellon;
pub mod font;
pub mod geom;
pub mod gerber;
pub mod io;
pub mod layer;
pub mod route;
pub mod stackup;
pub mod transform;
pub mod units;

pub use error::{PcbError, Result};

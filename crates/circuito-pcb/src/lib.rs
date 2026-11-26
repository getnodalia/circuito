// SPDX-License-Identifier: MIT OR Apache-2.0
//! Circuito PCB engine.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod artwork;
pub mod board;
pub mod drc;
pub mod error;
pub mod font;
pub mod geom;
pub mod io;
pub mod layer;
pub mod stackup;
pub mod transform;
pub mod units;

pub use error::{PcbError, Result};

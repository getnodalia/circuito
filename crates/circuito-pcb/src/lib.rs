// SPDX-License-Identifier: MIT OR Apache-2.0
//! Circuito PCB engine.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod error;
pub mod geom;
pub mod units;

pub use error::{PcbError, Result};

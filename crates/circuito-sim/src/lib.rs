// SPDX-License-Identifier: MIT OR Apache-2.0
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! Circuito's circuit simulator: modified nodal analysis with Newton-Raphson, DC operating point
//! and sweep, small-signal AC and transient analyses.
//!
//! See the README for what is modelled, the references each model was checked against and the
//! known limits.

mod analysis;
mod compile;
pub mod devices;
mod engine;
mod error;
mod options;
mod result;

pub use analysis::{ac_frequencies, simulate, simulate_all, MAX_SAMPLES};
pub use compile::MAX_UNKNOWNS;
pub use error::SimError;
pub use options::{Control, Integration, Options};
pub use result::{AnalysisResult, Kind, RunInfo, Signal, Table, Unit, Values};

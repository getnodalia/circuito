// SPDX-License-Identifier: MIT OR Apache-2.0
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
//! Everything Circuito's window needs that is not drawing.
//!
//! The window (`main.rs` and its modules) only draws and collects what the person did. The rest
//! lives here so it can be tested without a display:
//!
//! * [`document`]: a circuit read from a file, with its rules check and netlist;
//! * [`geometry`]: the view, hit testing and where a finding points;
//! * [`sim`]: analyses on a worker thread, with progress and cancellation;
//! * [`plot`]: results turned into the traces a plot draws;
//! * [`workspace`]: the open circuits and what the person selected, with its reducer;
//! * [`findings`], [`format`], [`commands`], [`settings`], [`i18n`], [`icon`].

pub mod commands;
pub mod document;
pub mod findings;
pub mod format;
pub mod geometry;
pub mod i18n;
pub mod icon;
pub mod plot;
pub mod settings;
pub mod sim;
mod spanish;
pub mod workspace;

/// The name in the interface.
pub const NAME: &str = "Circuito";
/// The application id (desktop file, D-Bus names, window class).
pub const APP_ID: &str = "dev.danielmiguelt.Circuito";

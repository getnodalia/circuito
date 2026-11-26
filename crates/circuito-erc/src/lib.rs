// SPDX-License-Identifier: MIT OR Apache-2.0
//! Netlist extraction from a schematic and the electrical rules check (ERC).
//!
//! * [`extract`] turns a [`circuito_core::schematic::Schematic`] into a
//!   [`circuito_core::netlist::Netlist`] with deterministic net names.
//! * [`check`] runs every rule and returns an [`ErcReport`] of violations. Each violation has a
//!   stable code, a severity, a position and a message naming the parts and nets involved. The
//!   engine returns data; translation of the message is the application's job.

mod extract;
mod matrix;
mod rules;

pub use extract::{default_models, extract, ExtractIssue, Extraction, IssueKind};
pub use matrix::{conflict, Verdict};
pub use rules::{check, ErcReport, Rule, Severity, Violation};

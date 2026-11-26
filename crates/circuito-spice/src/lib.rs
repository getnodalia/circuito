// SPDX-License-Identifier: MIT OR Apache-2.0
//! Circuito's SPICE netlist reader and writer.
//!
//! [`parse`] reads text into a [`circuito_core::netlist::Netlist`]; [`write`] prints one back so
//! that reading it again gives an equal netlist. Errors carry line numbers. The reader never opens
//! a file: `.include` and `.lib` are rejected.

pub mod expr;
mod parser;
mod writer;

pub use parser::{parse, ParseError, Parsed, Warning, MAX_INPUT_BYTES, MAX_LINE_BYTES};
pub use writer::write;

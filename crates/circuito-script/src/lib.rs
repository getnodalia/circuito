// SPDX-License-Identifier: MIT OR Apache-2.0
//! Circuito's circuit description language.
//!
//! A short text form of a schematic: one line per part with the net each pin is on, plus model
//! cards and analyses. [`parse`] reads it into a [`Design`], [`print`] writes one back, and
//! [`build`] turns a design into a drawn schematic with an automatic layout and routing.
//! [`design_of`] goes the other way and [`apply`] changes a schematic to match a text while
//! keeping what the person drew.

mod apply;
mod build;
mod design;
mod parse;
mod print;
mod problem;

pub use apply::{apply, design_of};
pub use build::{build, BuildError};
pub use design::{
    is_anonymous, is_ground, is_supply, kind_of, net_key, Design, NetInfo, NetKind, PartSpec,
    PinNet, Place,
};
pub use parse::{infer_lib, parse, takes_value, Parsed, FIELDS};
pub use print::{print, token_name, PrintOptions};
pub use problem::{Diagnostic, Problem};

/// Why a text could not become a schematic.
#[derive(Debug, Clone, PartialEq)]
pub enum CompileError {
    /// Lines that could not be understood.
    Syntax(Vec<Diagnostic>),
    /// The design could not be drawn.
    Build(BuildError),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Syntax(d) => {
                for (i, x) in d.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{x}")?;
                }
                Ok(())
            }
            CompileError::Build(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for CompileError {}

/// Read a text and draw it.
///
/// # Errors
/// With the problems of the text, or why it could not be drawn.
pub fn compile(text: &str) -> Result<circuito_core::schematic::Schematic, CompileError> {
    let parsed = parse(text);
    if !parsed.problems.is_empty() {
        return Err(CompileError::Syntax(parsed.problems));
    }
    build(&parsed.design, None).map_err(CompileError::Build)
}

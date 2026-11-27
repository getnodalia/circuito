// SPDX-License-Identifier: MIT OR Apache-2.0
//! KiCad file formats for Circuito, written from KiCad's public file-format documentation.
//!
//! [`sexpr`] reads and writes the S-expression syntax every KiCad file shares. [`pcb`] imports a
//! `.kicad_pcb` board into the PCB engine's model. Everything read from a file is hostile: sizes,
//! depth and counts are bounded and every failure is an [`Error`].
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod error;
pub mod pcb;
pub mod sexpr;

pub use error::{Error, Result};

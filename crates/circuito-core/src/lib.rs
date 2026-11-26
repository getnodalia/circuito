// SPDX-License-Identifier: MIT OR Apache-2.0
//! The shared model of Circuito: engineering values, the netlist, the schematic with its net
//! resolution, and the built-in part library.
//!
//! Nothing here touches the file system, a window or a network. Everything read from outside is
//! hostile: sizes are bounded and every failure is an error with a message a person can act on.

pub mod edit;
pub mod library;
pub mod netlist;
pub mod schematic;
pub mod source;
pub mod value;

pub use value::{format_value, parse_value, ValueError};

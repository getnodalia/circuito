// SPDX-License-Identifier: MIT OR Apache-2.0
//! The SPICE writer.

use circuito_core::netlist::{Netlist, NetlistError};

/// Write `netlist` as canonical SPICE text. The text reads back to an equal netlist.
///
/// The netlist is validated first, so what is written is always something the reader accepts.
///
/// # Errors
/// The first problem [`Netlist::validate`] finds.
pub fn write(netlist: &Netlist) -> Result<String, NetlistError> {
    netlist.validate()?;
    let mut n = netlist.clone();
    n.title = n.title.trim().to_string();
    Ok(n.to_string())
}

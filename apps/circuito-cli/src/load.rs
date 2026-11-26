// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading input files: SPICE netlists and `.circuito` schematics.

use circuito_core::netlist::Netlist;
use circuito_core::schematic::Schematic;
use std::io::Read;

/// Largest input read, in bytes. The parsers enforce their own, stricter limits too.
pub const MAX_INPUT: u64 = 64 * 1024 * 1024;

/// Read a file (or stdin for `-`) as UTF-8 text, never more than [`MAX_INPUT`] bytes.
///
/// # Errors
/// A message naming the file and the problem.
pub fn read_text(path: &str, stdin: &mut dyn Read) -> Result<String, String> {
    let mut buf = Vec::new();
    if path == "-" {
        stdin
            .take(MAX_INPUT + 1)
            .read_to_end(&mut buf)
            .map_err(|e| format!("cannot read standard input: {e}"))?;
    } else {
        let f = std::fs::File::open(path).map_err(|e| format!("cannot open {path}: {e}"))?;
        f.take(MAX_INPUT + 1)
            .read_to_end(&mut buf)
            .map_err(|e| format!("cannot read {path}: {e}"))?;
    }
    if buf.len() as u64 > MAX_INPUT {
        return Err(format!("{path} is larger than {MAX_INPUT} bytes"));
    }
    String::from_utf8(buf).map_err(|_| format!("{path} is not valid UTF-8 text"))
}

/// Is this text a `.circuito` schematic (JSON) rather than a SPICE deck?
fn is_schematic(path: &str, text: &str) -> bool {
    path.ends_with(".circuito") || text.trim_start().starts_with('{')
}

/// A netlist ready to simulate, with what was said while reading it.
pub struct Loaded {
    pub netlist: Netlist,
    /// Warnings from the reader, already formatted.
    pub warnings: Vec<String>,
}

/// Load `path` as a netlist: a SPICE deck is parsed, a schematic is extracted.
///
/// # Errors
/// Read errors, parse errors with their line, a schematic that cannot be extracted, or parts that
/// were skipped during extraction (simulating the rest would answer a different question).
pub fn load_netlist(path: &str, stdin: &mut dyn Read) -> Result<Loaded, String> {
    let text = read_text(path, stdin)?;
    if is_schematic(path, &text) {
        let (extraction, schematic_warnings) = extract_text(path, &text)?;
        if !extraction.issues.is_empty() {
            return Err(format_issues(path, &extraction.issues));
        }
        Ok(Loaded {
            netlist: extraction.netlist,
            warnings: schematic_warnings,
        })
    } else {
        let parsed = circuito_spice::parse(&text).map_err(|e| format!("{path}: {e}"))?;
        let warnings = parsed
            .warnings
            .iter()
            .map(|w| {
                if w.line == 0 {
                    w.message.clone()
                } else {
                    format!("line {}: {}", w.line, w.message)
                }
            })
            .collect();
        Ok(Loaded {
            netlist: parsed.netlist,
            warnings,
        })
    }
}

/// Parse a schematic and extract its netlist.
///
/// # Errors
/// A message naming the file.
pub fn extract_text(
    path: &str,
    text: &str,
) -> Result<(circuito_erc::Extraction, Vec<String>), String> {
    let schematic = Schematic::from_json(text).map_err(|e| format!("{path}: {e}"))?;
    let extraction = circuito_erc::extract(&schematic).map_err(|e| format!("{path}: {e}"))?;
    Ok((extraction, Vec::new()))
}

/// One line per part that extraction skipped.
#[must_use]
pub fn format_issues(path: &str, issues: &[circuito_erc::ExtractIssue]) -> String {
    let mut s = format!(
        "{path}: {} part(s) could not be turned into circuit elements, so the netlist would be incomplete:",
        issues.len()
    );
    for i in issues {
        s.push_str(&format!("\n  {}: {}", i.refdes, i.message));
    }
    s
}

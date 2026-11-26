// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading and writing `.circuito-pcb` files (JSON, `format_version: 1`).
//!
//! The reader refuses files larger than [`MAX_FILE_BYTES`], files with a different `format` or an
//! unknown `format_version` (a newer version is reported as such, never half-read), unknown fields,
//! and boards that fail [`Board::validate`] (limits, coordinate ranges, unique ids, valid references).

use serde::{Deserialize, Serialize};

use crate::board::Board;
use crate::error::{PcbError, Result};

/// The format identifier stored in every file.
pub const FORMAT_NAME: &str = "circuito-pcb";
/// The only version this engine reads and writes.
pub const FORMAT_VERSION: u32 = 1;
/// Largest accepted file in bytes.
pub const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Serialize)]
struct FileOut<'a> {
    format: &'static str,
    format_version: u32,
    board: &'a Board,
}

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    format_version: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileIn {
    #[allow(dead_code)]
    format: String,
    #[allow(dead_code)]
    format_version: u32,
    board: Board,
}

impl Board {
    /// Serialises the board after validating it. The output is deterministic.
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        serde_json::to_string_pretty(&FileOut {
            format: FORMAT_NAME,
            format_version: FORMAT_VERSION,
            board: self,
        })
        .map_err(|e| PcbError::Parse {
            what: "board",
            detail: e.to_string(),
        })
    }

    /// Parses and validates a `.circuito-pcb` document.
    pub fn from_json(text: &str) -> Result<Board> {
        if text.len() > MAX_FILE_BYTES {
            return Err(PcbError::TooLarge {
                what: "board file",
                detail: format!("{} bytes exceed the limit of {MAX_FILE_BYTES}", text.len()),
            });
        }
        let header: Header = serde_json::from_str(text).map_err(|e| PcbError::Parse {
            what: "board file",
            detail: format!("not a valid JSON document: {e}"),
        })?;
        match header.format.as_deref() {
            Some(FORMAT_NAME) => {}
            Some(other) => {
                return Err(PcbError::Parse {
                    what: "board file",
                    detail: format!("format is `{other}`, expected `{FORMAT_NAME}`"),
                })
            }
            None => {
                return Err(PcbError::Parse {
                    what: "board file",
                    detail: format!("missing `format` (expected `{FORMAT_NAME}`)"),
                })
            }
        }
        match header.format_version {
            Some(serde_json::Value::Number(n)) => match n.as_u64() {
                Some(v) if v == FORMAT_VERSION as u64 => {}
                Some(v) => {
                    return Err(PcbError::UnsupportedVersion {
                        what: "board",
                        found: v,
                        supported: FORMAT_VERSION,
                    })
                }
                None => {
                    return Err(PcbError::Parse {
                        what: "board file",
                        detail: "`format_version` must be a positive integer".into(),
                    })
                }
            },
            _ => {
                return Err(PcbError::Parse {
                    what: "board file",
                    detail: "missing or non-numeric `format_version`".into(),
                })
            }
        }
        let file: FileIn = serde_json::from_str(text).map_err(|e| PcbError::Parse {
            what: "board file",
            detail: e.to_string(),
        })?;
        file.board.validate()?;
        Ok(file.board)
    }
}

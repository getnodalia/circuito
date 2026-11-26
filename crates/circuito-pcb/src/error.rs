// SPDX-License-Identifier: MIT OR Apache-2.0
//! Error type of the PCB engine. Every message says what was wrong and, where it helps, where.

use thiserror::Error;

/// Everything that can go wrong in this crate. Hostile input produces one of these, never a panic.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PcbError {
    /// A coordinate, size or count is outside what the engine accepts.
    #[error("{what}: {detail}")]
    OutOfRange { what: String, detail: String },
    /// Integer arithmetic on coordinates would overflow.
    #[error("coordinate arithmetic overflow in {0}")]
    Overflow(&'static str),
    /// A geometric object is malformed (self-intersecting polygon, degenerate arc...).
    #[error("invalid geometry in {what}: {detail}")]
    Geometry { what: String, detail: String },
    /// The board is internally inconsistent (duplicate ids, unknown net...).
    #[error("invalid board: {0}")]
    Invalid(String),
    /// A file could not be read.
    #[error("cannot read {what}: {detail}")]
    Parse { what: &'static str, detail: String },
    /// The file declares a format version this engine does not know.
    #[error("unsupported {what} format version {found} (this engine reads version {supported})")]
    UnsupportedVersion {
        what: &'static str,
        found: u64,
        supported: u32,
    },
    /// The input is larger than the engine's safety limits.
    #[error("{what} is too large: {detail}")]
    TooLarge { what: &'static str, detail: String },
    /// A feature of the input is valid but not implemented here.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

/// Result alias used across the crate.
pub type Result<T> = std::result::Result<T, PcbError>;

impl PcbError {
    pub(crate) fn range(what: impl Into<String>, detail: impl Into<String>) -> Self {
        PcbError::OutOfRange {
            what: what.into(),
            detail: detail.into(),
        }
    }
    pub(crate) fn geometry(what: impl Into<String>, detail: impl Into<String>) -> Self {
        PcbError::Geometry {
            what: what.into(),
            detail: detail.into(),
        }
    }
    pub(crate) fn invalid(detail: impl Into<String>) -> Self {
        PcbError::Invalid(detail.into())
    }
}

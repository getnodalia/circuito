// SPDX-License-Identifier: MIT OR Apache-2.0
//! Errors of the KiCad readers: each says what was wrong and, where it helps, where.

use thiserror::Error;

/// Everything that can go wrong reading a KiCad file.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    /// The text is not valid S-expression syntax.
    #[error("line {line}: {detail}")]
    Syntax { line: usize, detail: String },
    /// The input is larger than the reader's safety limits.
    #[error("{what} is too large: {detail}")]
    TooLarge { what: &'static str, detail: String },
    /// The file is not the kind of file expected.
    #[error("not a {expected} file: {detail}")]
    WrongKind {
        expected: &'static str,
        detail: String,
    },
    /// A value in the file is missing or malformed.
    #[error("{context}: {detail}")]
    Invalid { context: String, detail: String },
    /// The board the file describes was refused by the PCB engine.
    #[error("the imported board is not valid: {0}")]
    Board(String),
}

pub type Result<T> = std::result::Result<T, Error>;

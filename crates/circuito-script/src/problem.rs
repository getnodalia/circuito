// SPDX-License-Identifier: MIT OR Apache-2.0
//! What can be wrong with a line of the text form. Each problem carries the data a message needs;
//! [`Problem`]'s `Display` is English, and an interface words them in its own language.

use std::fmt;

/// One thing wrong with the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A `(` without its `)`, or a `"` without its closing one.
    Unbalanced,
    /// The first word of a line is not a valid reference designator.
    BadRefdes {
        /// The word.
        text: String,
    },
    /// The part kind cannot be told from the reference designator.
    UnknownKind {
        /// The reference designator.
        refdes: String,
    },
    /// `REF:KIND` names a kind that does not exist.
    UnknownPart {
        /// The name after the colon.
        name: String,
    },
    /// Two parts have the same reference designator.
    DuplicateRefdes {
        /// The designator.
        refdes: String,
    },
    /// A part has the wrong number of nets.
    NetCount {
        /// The part.
        refdes: String,
        /// How many pins it has.
        expected: usize,
        /// How many nets the line gives.
        found: usize,
        /// The pins in order, for the message.
        pins: String,
    },
    /// `name=net` names neither a pin nor a field of the part.
    UnknownPin {
        /// The part.
        refdes: String,
        /// The name before the `=`.
        key: String,
        /// The pins in order, for the message.
        pins: String,
    },
    /// A pin is given twice.
    PinTwice {
        /// The part.
        refdes: String,
        /// The pin.
        key: String,
    },
    /// A value is given to a part that has none.
    NoValue {
        /// The part.
        refdes: String,
    },
    /// A value or field is not a number or source description.
    BadValue {
        /// The part.
        refdes: String,
        /// The text.
        text: String,
        /// Why, in English.
        why: String,
    },
    /// An `@x,y` position that cannot be read.
    BadLayout {
        /// The text.
        text: String,
    },
    /// A `model` line that is incomplete.
    ModelUsage,
    /// A model kind that does not exist.
    UnknownModelKind {
        /// The kind.
        kind: String,
    },
    /// A number that cannot be read.
    BadNumber {
        /// The text.
        text: String,
    },
    /// An analysis line with the wrong shape.
    AnalysisUsage {
        /// The correct shape, in the language's own words.
        usage: &'static str,
    },
    /// An analysis whose numbers do not make sense.
    BadAnalysis {
        /// Why, in English.
        why: String,
    },
    /// The text is too long or has too many parts.
    TooMuch,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::Unbalanced => write!(f, "a parenthesis or quote is not closed"),
            Problem::BadRefdes { text } => write!(
                f,
                "`{text}` is not a part name: start with a letter, then letters, digits or _"
            ),
            Problem::UnknownKind { refdes } => write!(
                f,
                "cannot tell what kind of part `{refdes}` is; write it as `{refdes}:KIND`, for example `{refdes}:R`"
            ),
            Problem::UnknownPart { name } => write!(f, "there is no part kind `{name}`"),
            Problem::DuplicateRefdes { refdes } => {
                write!(f, "there is already a part called `{refdes}`")
            }
            Problem::NetCount {
                refdes,
                expected,
                found,
                pins,
            } => write!(
                f,
                "{refdes} needs {expected} nets (pins {pins}) after an optional value, but the line has {found}"
            ),
            Problem::UnknownPin { refdes, key, pins } => write!(
                f,
                "{refdes} has no pin or field `{key}` (pins: {pins})"
            ),
            Problem::PinTwice { refdes, key } => {
                write!(f, "pin `{key}` of {refdes} is given twice")
            }
            Problem::NoValue { refdes } => write!(f, "{refdes} has no value"),
            Problem::BadValue { refdes, text, why } => {
                write!(f, "{refdes}: `{text}` is not a valid value ({why})")
            }
            Problem::BadLayout { text } => write!(
                f,
                "`{text}` is not a position: write `@x,y` with whole numbers, then optionally r90, r180, r270 or mirror"
            ),
            Problem::ModelUsage => write!(f, "write a model as `model NAME KIND param=value ...`"),
            Problem::UnknownModelKind { kind } => {
                write!(f, "there is no model kind `{kind}` (D, NPN, PNP, NMOS, PMOS, SW, OPAMP)")
            }
            Problem::BadNumber { text } => write!(f, "`{text}` is not a number"),
            Problem::AnalysisUsage { usage } => write!(f, "write it as `{usage}`"),
            Problem::BadAnalysis { why } => write!(f, "{why}"),
            Problem::TooMuch => write!(f, "the text is too long"),
        }
    }
}

/// A problem and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Line, starting at 1.
    pub line: usize,
    /// First column of the offending text, in characters, starting at 0.
    pub start: usize,
    /// One past the last column.
    pub end: usize,
    /// What is wrong.
    pub problem: Problem,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.problem)
    }
}

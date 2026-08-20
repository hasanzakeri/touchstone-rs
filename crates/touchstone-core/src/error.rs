//! Error types.
//!
//! Parse failures carry a 1-based line number so a user can go straight to
//! the offending line; the `kind` says what was wrong there.

use std::fmt;

/// Errors produced while reading or parsing a Touchstone file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("line {line}: {kind}")]
    Parse { line: usize, kind: ParseErrorKind },
    #[error("not implemented yet: {0}")]
    Unimplemented(&'static str),
}

/// What went wrong on a given line. Grows with the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseErrorKind {
    InvalidOptionLine(String),
    InvalidNumber(String),
    UnexpectedData(String),
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseErrorKind::InvalidOptionLine(s) => write!(f, "invalid option line: {s}"),
            ParseErrorKind::InvalidNumber(s) => write!(f, "invalid number: {s}"),
            ParseErrorKind::UnexpectedData(s) => write!(f, "unexpected data: {s}"),
        }
    }
}

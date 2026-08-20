//! Error types.
//!
//! Parse failures carry a 1-based line number so a user can go straight to
//! the offending line; the `kind` says what was wrong there.

use std::fmt;

use crate::model::{Format, Parameter};

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
///
/// `Eq` is deliberately not derived: [`ParseErrorKind::FrequencyNotAscending`]
/// carries `f64` values, which is worth more than an equivalence relation on
/// an error type only ever compared in tests.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ParseErrorKind {
    InvalidOptionLine(String),
    InvalidNumber(String),
    UnexpectedData(String),
    /// The file has no option line at all. Spec v1.1 §3 requires one; the
    /// documented defaults cover *omitted parts*, not an absent line.
    MissingOptionLine,
    /// Network data appeared before the option line, so the unit and format
    /// needed to interpret it were not yet known.
    DataBeforeOptionLine,
    /// A data line did not hold exactly one frequency point's worth of
    /// values. Covers truncated rows, trailing garbage, and line wrapping.
    WrongValueCount {
        expected: usize,
        found: usize,
    },
    /// Spec v1.1 §3 requires data sets in increasing frequency order.
    FrequencyNotAscending {
        previous_hz: f64,
        current_hz: f64,
    },
    /// A value format this version cannot convert yet.
    UnsupportedFormat(Format),
    /// A network parameter type this version cannot handle yet.
    UnsupportedParameter(Parameter),
    /// A port count this version cannot handle yet.
    UnsupportedPortCount(usize),
    /// A 2-port noise section was found. Detected on purpose, and named, so
    /// the failure does not masquerade as a frequency-ordering error.
    NoiseSectionUnsupported,
    /// Carriage-return-only line endings, which would collapse the whole
    /// file into a single line.
    UnsupportedLineEndings,
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseErrorKind::InvalidOptionLine(s) => write!(f, "invalid option line: {s}"),
            ParseErrorKind::InvalidNumber(s) => write!(f, "invalid number: {s}"),
            ParseErrorKind::UnexpectedData(s) => write!(f, "unexpected data: {s}"),
            ParseErrorKind::MissingOptionLine => {
                write!(
                    f,
                    "missing option line (expected a line beginning with '#')"
                )
            }
            ParseErrorKind::DataBeforeOptionLine => write!(f, "data line before the option line"),
            ParseErrorKind::WrongValueCount { expected, found } => {
                write!(
                    f,
                    "expected {expected} values on a data line, found {found}"
                )
            }
            ParseErrorKind::FrequencyNotAscending {
                previous_hz,
                current_hz,
            } => write!(
                f,
                "frequencies must increase: {current_hz} hz follows {previous_hz} hz"
            ),
            ParseErrorKind::UnsupportedFormat(fmt) => write!(
                f,
                "unsupported format {}: only ri is supported in this version",
                fmt.as_str().to_ascii_lowercase()
            ),
            ParseErrorKind::UnsupportedParameter(p) => write!(
                f,
                "unsupported parameter {}: only s-parameters are supported in this version",
                p.as_str().to_ascii_lowercase()
            ),
            ParseErrorKind::UnsupportedPortCount(n) => write!(
                f,
                "unsupported port count {n}: only 2-port files are supported in this version"
            ),
            ParseErrorKind::NoiseSectionUnsupported => {
                write!(
                    f,
                    "noise parameter section is not supported in this version"
                )
            }
            ParseErrorKind::UnsupportedLineEndings => {
                write!(f, "carriage-return-only line endings are not supported")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The out-of-scope messages are a deliverable: they are what a first
    /// user sees when they point the parser at an MA file, and they must not
    /// read like a bug report.
    #[test]
    fn unsupported_messages_name_the_scope_limit() {
        let err = Error::Parse {
            line: 6,
            kind: ParseErrorKind::UnsupportedFormat(Format::Ma),
        };
        assert_eq!(
            err.to_string(),
            "line 6: unsupported format ma: only ri is supported in this version"
        );

        let err = Error::Parse {
            line: 1,
            kind: ParseErrorKind::UnsupportedParameter(Parameter::Y),
        };
        assert_eq!(
            err.to_string(),
            "line 1: unsupported parameter y: only s-parameters are supported in this version"
        );

        let err = Error::Parse {
            line: 42,
            kind: ParseErrorKind::NoiseSectionUnsupported,
        };
        assert_eq!(
            err.to_string(),
            "line 42: noise parameter section is not supported in this version"
        );
    }

    #[test]
    fn value_count_and_ordering_messages_quote_the_numbers() {
        let err = Error::Parse {
            line: 3,
            kind: ParseErrorKind::WrongValueCount {
                expected: 9,
                found: 8,
            },
        };
        assert_eq!(
            err.to_string(),
            "line 3: expected 9 values on a data line, found 8"
        );

        let err = Error::Parse {
            line: 4,
            kind: ParseErrorKind::FrequencyNotAscending {
                previous_hz: 2e9,
                current_hz: 1e9,
            },
        };
        assert_eq!(
            err.to_string(),
            "line 4: frequencies must increase: 1000000000 hz follows 2000000000 hz"
        );
    }
}

//! Error types.
//!
//! Parse failures carry a 1-based line number so a user can go straight to
//! the offending line; the `kind` says what was wrong there.

use std::fmt;

use crate::model::Parameter;

/// Errors produced while reading or parsing a Touchstone file.
///
/// `#[non_exhaustive]`: this will grow (v2 keyword errors, writer errors),
/// and each addition would otherwise be a breaking change for any caller
/// matching on it exhaustively.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("failed to read {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("line {line}: {kind}")]
    Parse { line: usize, kind: ParseErrorKind },
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
    /// A token that is not a usable number, quoted as the file spells it.
    InvalidNumber(String),
    /// The file carried an option line but no network data.
    NoDataLines,
    /// The file has no option line at all. Spec v1.1 §3 requires one; the
    /// documented defaults cover *omitted parts*, not an absent line.
    MissingOptionLine,
    /// Network data appeared before the option line, so the unit and format
    /// needed to interpret it were not yet known.
    DataBeforeOptionLine,
    /// A data set did not hold exactly one frequency point's worth of
    /// values. Covers truncated rows and trailing garbage.
    WrongValueCount {
        expected: usize,
        found: usize,
    },
    /// The first data set's size fits no port count, so the file's shape
    /// could not be deduced. A data set holds `1 + 2n²` values, and `found`
    /// solves that for no whole `n` — the file is truncated or malformed,
    /// unless the caller can supply the port count another way.
    IndeterminatePortCount {
        found: usize,
    },
    /// A port count that cannot describe a data set: zero, or one so large
    /// that `1 + 2n²` overflows. Reachable because the count can come from a
    /// filename or a caller without ever being vetted — `x.s99999999999p`
    /// names 10¹¹ ports. Not a policy ceiling; see ADR 0006.
    UnusablePortCount {
        nports: usize,
    },
    /// A value pair converted to something that is not a finite complex
    /// number. Checked after conversion, not on the raw token: `-inf` in a
    /// `DB` magnitude column is a legitimate way to write a zero-magnitude
    /// entry, and it converts to exactly `0+0i`.
    NonFiniteValue {
        first: f64,
        second: f64,
    },
    /// Spec v1.1 §3 requires data sets in increasing frequency order.
    FrequencyNotAscending {
        previous_hz: f64,
        current_hz: f64,
    },
    /// A network parameter type this version cannot handle yet.
    UnsupportedParameter(Parameter),
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
            ParseErrorKind::NoDataLines => write!(f, "no data lines"),
            ParseErrorKind::MissingOptionLine => {
                write!(
                    f,
                    "missing option line (expected a line beginning with '#')"
                )
            }
            ParseErrorKind::DataBeforeOptionLine => write!(f, "data line before the option line"),
            ParseErrorKind::WrongValueCount { expected, found } => {
                write!(f, "expected {expected} values in a data set, found {found}")
            }
            ParseErrorKind::IndeterminatePortCount { found } => write!(
                f,
                "cannot determine the port count: a data set of {found} values \
                 is not 1 + 2n^2 for any n (name the file '.sNp' or pass an \
                 explicit port count)"
            ),
            ParseErrorKind::UnusablePortCount { nports: 0 } => {
                write!(f, "port count must be at least 1")
            }
            ParseErrorKind::UnusablePortCount { nports } => write!(
                f,
                "port count {nports} is too large: one data set would hold \
                 1 + 2*{nports}^2 values, which does not fit in memory"
            ),
            ParseErrorKind::NonFiniteValue { first, second } => write!(
                f,
                "value pair '{first} {second}' is not a finite complex number"
            ),
            ParseErrorKind::FrequencyNotAscending {
                previous_hz,
                current_hz,
            } => write!(
                f,
                "frequencies must increase: {current_hz} hz follows {previous_hz} hz"
            ),
            ParseErrorKind::UnsupportedParameter(p) => write!(
                f,
                "unsupported parameter {}: only s-parameters are supported in this version",
                p.as_str().to_ascii_lowercase()
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
    /// user sees when they point the parser at a Y-parameter file, and they
    /// must not read like a bug report.
    #[test]
    fn unsupported_messages_name_the_scope_limit() {
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
            "line 3: expected 9 values in a data set, found 8"
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

    /// Both messages have to tell the reader what to *do*, not just that
    /// something is wrong — the port-count one because the fix (name the file
    /// `.sNp`) is not guessable, and the non-finite one because the offending
    /// pair is invisible in a line of forty numbers.
    #[test]
    fn the_new_diagnostics_say_what_to_do_about_them() {
        let err = Error::Parse {
            line: 2,
            kind: ParseErrorKind::IndeterminatePortCount { found: 8 },
        };
        assert_eq!(
            err.to_string(),
            "line 2: cannot determine the port count: a data set of 8 values is not \
             1 + 2n^2 for any n (name the file '.sNp' or pass an explicit port count)"
        );

        let err = Error::Parse {
            line: 7,
            kind: ParseErrorKind::NonFiniteValue {
                first: f64::NAN,
                second: 0.0,
            },
        };
        assert_eq!(
            err.to_string(),
            "line 7: value pair 'NaN 0' is not a finite complex number"
        );
    }

    /// An unusable port count has two quite different causes, and one
    /// message for both would explain neither.
    #[test]
    fn an_unusable_port_count_says_which_way_it_is_unusable() {
        let err = Error::Parse {
            line: 2,
            kind: ParseErrorKind::UnusablePortCount { nports: 0 },
        };
        assert_eq!(err.to_string(), "line 2: port count must be at least 1");

        let err = Error::Parse {
            line: 2,
            kind: ParseErrorKind::UnusablePortCount {
                nports: 99_999_999_999,
            },
        };
        assert_eq!(
            err.to_string(),
            "line 2: port count 99999999999 is too large: one data set would hold \
             1 + 2*99999999999^2 values, which does not fit in memory"
        );
    }

    #[test]
    fn a_file_with_an_option_line_but_no_data_says_so_plainly() {
        let err = Error::Parse {
            line: 3,
            kind: ParseErrorKind::NoDataLines,
        };
        assert_eq!(err.to_string(), "line 3: no data lines");
    }
}

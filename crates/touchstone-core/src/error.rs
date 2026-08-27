//! Error types.
//!
//! Parse failures carry a 1-based line number so a user can go straight to
//! the offending line; the `kind` says what was wrong there.

use std::fmt;

use crate::model::Parameter;
use crate::parser::NOISE_VALUES_PER_SET;

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
    /// The noise section's own frequencies failed to increase. Kept apart
    /// from [`ParseErrorKind::FrequencyNotAscending`] because the generic
    /// wording would read as though a noise frequency had been compared
    /// against the S-parameter sweep — which happens only at the boundary,
    /// and is how the boundary is found at all.
    NoiseFrequencyNotAscending {
        previous_hz: f64,
        current_hz: f64,
    },
    /// A line in the noise section did not hold the five values a noise
    /// point needs. Kept apart from [`ParseErrorKind::WrongValueCount`]
    /// because "expected 5 values" reported against what looks like a
    /// perfectly good 2-port data line sends the reader hunting in the wrong
    /// place: what it means is that the noise section has already begun, so
    /// the message says where that happened.
    MalformedNoiseLine {
        found: usize,
        /// Line the noise section was found to start on.
        noise_starts_at: usize,
    },
    /// A noise-parameter entry that is not a finite number. `column` names
    /// which of the five entries it was — a row of bare numbers gives the
    /// reader nothing else to go on.
    NonFiniteNoiseValue {
        column: &'static str,
        value: f64,
    },
    /// A list of reference resistances that does not have one entry per port.
    ///
    /// Reachable two ways, so the message says which: a "Version 1.1" option
    /// line whose `R` list is the wrong length, or a v2 `[Reference]` keyword
    /// with the wrong number of arguments. A single option-line value is
    /// always legal — it is the reference for every port — so this only ever
    /// fires on a list of two or more.
    WrongResistanceCount {
        /// Where the values came from, named as the file spells it.
        source: &'static str,
        expected: usize,
        found: usize,
    },
    /// A network parameter type this version cannot handle yet.
    UnsupportedParameter(Parameter),
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
            ParseErrorKind::NoiseFrequencyNotAscending {
                previous_hz,
                current_hz,
            } => write!(
                f,
                "noise frequencies must increase: {current_hz} hz follows {previous_hz} hz"
            ),
            ParseErrorKind::MalformedNoiseLine {
                found,
                noise_starts_at,
            } => write!(
                f,
                "expected {NOISE_VALUES_PER_SET} values in a noise parameter \
                 line, found {found} (the noise section begins at line \
                 {noise_starts_at})"
            ),
            ParseErrorKind::NonFiniteNoiseValue { column, value } => {
                write!(f, "noise {column} value '{value}' is not a finite number")
            }
            ParseErrorKind::WrongResistanceCount {
                source,
                expected,
                found,
            } => write!(
                f,
                "{source} gives {found} reference resistances for a \
                 {expected}-port network"
            ),
            ParseErrorKind::UnsupportedParameter(p) => write!(
                f,
                "unsupported parameter {}: only s-parameters are supported in this version",
                p.as_str().to_ascii_lowercase()
            ),
            ParseErrorKind::UnsupportedLineEndings => {
                write!(f, "carriage-return-only line endings are not supported")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The out-of-scope message is a deliverable: it is what a first user
    /// sees when they point the parser at a Y-parameter file, and it must
    /// not read like a bug report.
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
    }

    /// Every noise diagnostic has to say *noise* somewhere. The section is
    /// found by inference rather than announced by a keyword, so a reader
    /// who gets a bare "frequencies must increase" on a line they think is
    /// S-data has no way to tell whether the parser found the boundary in
    /// the wrong place — which is the first thing they will suspect.
    #[test]
    fn noise_diagnostics_say_that_they_are_about_the_noise_section() {
        let err = Error::Parse {
            line: 19,
            kind: ParseErrorKind::NoiseFrequencyNotAscending {
                previous_hz: 4e9,
                current_hz: 4e9,
            },
        };
        assert_eq!(
            err.to_string(),
            "line 19: noise frequencies must increase: 4000000000 hz follows 4000000000 hz"
        );

        // The count case names the boundary line, because the fix is
        // usually somewhere other than the line being complained about.
        let err = Error::Parse {
            line: 20,
            kind: ParseErrorKind::MalformedNoiseLine {
                found: 9,
                noise_starts_at: 16,
            },
        };
        assert_eq!(
            err.to_string(),
            "line 20: expected 5 values in a noise parameter line, found 9 \
             (the noise section begins at line 16)"
        );

        let err = Error::Parse {
            line: 17,
            kind: ParseErrorKind::NonFiniteNoiseValue {
                column: "nfmin",
                value: f64::NAN,
            },
        };
        assert_eq!(
            err.to_string(),
            "line 17: noise nfmin value 'NaN' is not a finite number"
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

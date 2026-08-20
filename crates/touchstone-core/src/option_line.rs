//! The option line: `# <freq unit> <parameter> <format> R <n>`.
//!
//! Spec v1.1 §3. Every part is optional and, apart from the leading `#` and
//! the value that follows `R`, the parts may appear in any order; omitted
//! parts take the documented defaults. Matching is case-insensitive (§2), so
//! `# hZ s Ri r 50` is as valid as `# HZ S RI R 50`.
//!
//! Kept separate from the data parser because it is pure, has by far the
//! densest test matrix in the crate, and is the one piece the v2 parser will
//! reuse unchanged — v2 files still carry an option line.

use crate::error::{Error, ParseErrorKind};
use crate::model::{Format, FreqUnit, Parameter};

/// What an option line says, with defaults filled in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Options {
    pub freq_unit: FreqUnit,
    pub parameter: Parameter,
    pub format: Format,
    pub resistance: f64,
}

impl Default for Options {
    /// Spec v1.1 §3 defaults: GHz, S-parameters, magnitude-angle, 50 Ω.
    fn default() -> Self {
        Options {
            freq_unit: FreqUnit::GHz,
            parameter: Parameter::S,
            format: Format::Ma,
            resistance: 50.0,
        }
    }
}

/// Parse the body of an option line — everything after the `#`.
///
/// `line` is the 1-based source line number, carried into any error.
pub(crate) fn parse_option_line(body: &str, line: usize) -> Result<Options, Error> {
    let mut opts = Options::default();
    // Each category may be set at most once. Two frequency units on one
    // line is not something the spec defines, and picking one silently is a
    // coin flip on how every frequency in the file gets scaled.
    let (mut seen_unit, mut seen_param, mut seen_format, mut seen_resistance) =
        (false, false, false, false);

    let mut tokens = body.split_whitespace();
    while let Some(token) = tokens.next() {
        let lower = token.to_ascii_lowercase();
        match lower.as_str() {
            "hz" | "khz" | "mhz" | "ghz" => {
                claim(&mut seen_unit, "frequency unit", line)?;
                opts.freq_unit = match lower.as_str() {
                    "hz" => FreqUnit::Hz,
                    "khz" => FreqUnit::KHz,
                    "mhz" => FreqUnit::MHz,
                    _ => FreqUnit::GHz,
                };
            }
            "s" | "y" | "z" | "g" | "h" => {
                claim(&mut seen_param, "parameter type", line)?;
                opts.parameter = match lower.as_str() {
                    "s" => Parameter::S,
                    "y" => Parameter::Y,
                    "z" => Parameter::Z,
                    "g" => Parameter::G,
                    _ => Parameter::H,
                };
            }
            "ri" | "ma" | "db" => {
                claim(&mut seen_format, "value format", line)?;
                opts.format = match lower.as_str() {
                    "ri" => Format::Ri,
                    "ma" => Format::Ma,
                    _ => Format::Db,
                };
            }
            "r" => {
                claim(&mut seen_resistance, "reference resistance", line)?;
                // `R` is the one token whose value is positional. Real files
                // separate them generously (`R     50.00`); the glued forms
                // `R50` and `R=50` are not accepted here.
                let value = tokens
                    .next()
                    .ok_or_else(|| invalid(line, "'r' given with no value"))?;
                let ohms: f64 = value.parse().map_err(|_| {
                    parse_err(line, ParseErrorKind::InvalidNumber(value.to_string()))
                })?;
                if !ohms.is_finite() || ohms <= 0.0 {
                    return Err(invalid(
                        line,
                        format!("reference resistance must be a positive number, got '{value}'"),
                    ));
                }
                opts.resistance = ohms;
            }
            other => {
                return Err(invalid(line, format!("unknown token '{other}'")));
            }
        }
    }

    Ok(opts)
}

/// Mark a category as seen, rejecting a second occurrence.
fn claim(seen: &mut bool, what: &str, line: usize) -> Result<(), Error> {
    if *seen {
        return Err(invalid(line, format!("duplicate {what}")));
    }
    *seen = true;
    Ok(())
}

fn invalid(line: usize, detail: impl Into<String>) -> Error {
    parse_err(line, ParseErrorKind::InvalidOptionLine(detail.into()))
}

fn parse_err(line: usize, kind: ParseErrorKind) -> Error {
    Error::Parse { line, kind }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> Options {
        parse_option_line(body, 1).expect("should parse")
    }

    fn kind(body: &str) -> ParseErrorKind {
        match parse_option_line(body, 7) {
            Err(Error::Parse { line, kind }) => {
                assert_eq!(line, 7, "the source line number must be carried through");
                kind
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_hash_means_every_default() {
        // Spec v1.1 §3 lists `#` alone as the minimum legal option line.
        assert_eq!(parse(""), Options::default());
        assert_eq!(parse("   "), Options::default());
    }

    #[test]
    fn parses_the_canonical_form() {
        assert_eq!(
            parse(" GHZ S RI R 50"),
            Options {
                freq_unit: FreqUnit::GHz,
                parameter: Parameter::S,
                format: Format::Ri,
                resistance: 50.0,
            }
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(parse(" hZ s Ri r 50"), parse(" HZ S RI R 50"));
    }

    #[test]
    fn tokens_may_appear_in_any_order() {
        let canonical = parse(" MHZ S RI R 75");
        assert_eq!(parse(" S RI R 75 MHZ"), canonical);
        assert_eq!(parse(" R 75 MHZ RI S"), canonical);
        assert_eq!(parse(" RI MHZ R 75 S"), canonical);
    }

    #[test]
    fn extra_whitespace_and_tabs_are_ignored() {
        // Matches the spacing a real Skyworks export uses.
        assert_eq!(parse("  HZ   S   DB   R     50.00 ").resistance, 50.0);
        assert_eq!(parse("\tGHZ\tS\tRI\tR\t50").format, Format::Ri);
    }

    #[test]
    fn omitted_parts_fall_back_to_defaults() {
        assert_eq!(
            parse(" MHZ"),
            Options {
                freq_unit: FreqUnit::MHz,
                ..Options::default()
            }
        );
        assert_eq!(
            parse(" RI"),
            Options {
                format: Format::Ri,
                ..Options::default()
            }
        );
        assert_eq!(
            parse(" R 100"),
            Options {
                resistance: 100.0,
                ..Options::default()
            }
        );
    }

    #[test]
    fn every_unit_parameter_and_format_keyword_is_recognized() {
        for (kw, unit) in [
            ("HZ", FreqUnit::Hz),
            ("KHZ", FreqUnit::KHz),
            ("MHZ", FreqUnit::MHz),
            ("GHZ", FreqUnit::GHz),
        ] {
            assert_eq!(parse(kw).freq_unit, unit, "unit keyword {kw}");
        }
        for (kw, param) in [
            ("S", Parameter::S),
            ("Y", Parameter::Y),
            ("Z", Parameter::Z),
            ("G", Parameter::G),
            ("H", Parameter::H),
        ] {
            assert_eq!(parse(kw).parameter, param, "parameter keyword {kw}");
        }
        for (kw, format) in [("RI", Format::Ri), ("MA", Format::Ma), ("DB", Format::Db)] {
            assert_eq!(parse(kw).format, format, "format keyword {kw}");
        }
    }

    #[test]
    fn resistance_need_not_be_an_integer() {
        assert_eq!(parse(" R 50.00").resistance, 50.0);
        assert_eq!(parse(" R 1e2").resistance, 100.0);
        assert_eq!(parse(" R .5").resistance, 0.5);
    }

    #[test]
    fn a_repeated_category_is_rejected() {
        assert_eq!(
            kind(" GHZ MHZ S RI"),
            ParseErrorKind::InvalidOptionLine("duplicate frequency unit".into())
        );
        assert_eq!(
            kind(" GHZ S Y RI"),
            ParseErrorKind::InvalidOptionLine("duplicate parameter type".into())
        );
        assert_eq!(
            kind(" GHZ S RI MA"),
            ParseErrorKind::InvalidOptionLine("duplicate value format".into())
        );
        assert_eq!(
            kind(" R 50 R 75"),
            ParseErrorKind::InvalidOptionLine("duplicate reference resistance".into())
        );
    }

    #[test]
    fn r_must_be_followed_by_a_positive_number() {
        assert_eq!(
            kind(" GHZ S RI R"),
            ParseErrorKind::InvalidOptionLine("'r' given with no value".into())
        );
        assert_eq!(kind(" R abc"), ParseErrorKind::InvalidNumber("abc".into()));
        assert!(matches!(
            kind(" R -50"),
            ParseErrorKind::InvalidOptionLine(m) if m.contains("positive")
        ));
        assert!(matches!(
            kind(" R 0"),
            ParseErrorKind::InvalidOptionLine(m) if m.contains("positive")
        ));
        assert!(matches!(
            kind(" R inf"),
            ParseErrorKind::InvalidOptionLine(m) if m.contains("positive")
        ));
    }

    #[test]
    fn unknown_tokens_are_rejected_rather_than_skipped() {
        // Quietly ignoring a token we do not understand risks misreading the
        // unit or format, which silently rescales or transposes every value.
        assert_eq!(
            kind(" GHZ S RI R 50 EXTRA"),
            ParseErrorKind::InvalidOptionLine("unknown token 'extra'".into())
        );
    }

    #[test]
    fn the_glued_r_forms_are_not_accepted_yet() {
        // `R50` / `R=50` are a lenient-mode question; no real file needs them.
        assert!(matches!(
            kind(" GHZ S RI R50"),
            ParseErrorKind::InvalidOptionLine(m) if m.contains("r50")
        ));
    }
}

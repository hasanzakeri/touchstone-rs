//! The option line: `# <freq unit> <parameter> <format> R <n1> [… <np>]`.
//!
//! Spec v1.1 §3. Every part is optional and, apart from the leading `#` and
//! the values that follow `R`, the parts may appear in any order; omitted
//! parts take the documented defaults. Matching is case-insensitive (§2), so
//! `# hZ s Ri r 50` is as valid as `# HZ S RI R 50`.
//!
//! `R` takes **one or more** values. A single value is the reference for every
//! port; one value per port is what the 2.1 document designates a "Version
//! 1.1" file, and is the only substantive difference between 1.0 and 1.1
//! syntax. The list has to be contiguous — the same document makes it the one
//! exception to the free ordering above — and needs no delimiter, because no
//! other option-line token parses as a number, so numeric tokens after `R` can
//! simply be taken until one does not.
//!
//! Kept separate from the data parser because it is pure, has by far the
//! densest test matrix in the crate, and is the one piece the v2 parser reuses
//! unchanged — v2 files still carry an option line.

use crate::error::{Error, ParseErrorKind};
use crate::model::{Format, FreqUnit, Parameter};

/// What an option line says, with defaults filled in.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Options {
    pub freq_unit: FreqUnit,
    pub parameter: Parameter,
    pub format: Format,
    /// Every value given after `R`, in order. Never empty: an option line with
    /// no `R` at all takes the documented 50 Ω default.
    ///
    /// Not checked against the port count here, because the option line is
    /// read before the port count is known — a v1 file may not state one at
    /// all until its first data set closes. The parser checks it once it can.
    pub resistances: Vec<f64>,
}

impl Default for Options {
    /// Spec v1.1 §3 defaults: GHz, S-parameters, magnitude-angle, 50 Ω.
    fn default() -> Self {
        Options {
            freq_unit: FreqUnit::GHz,
            parameter: Parameter::S,
            format: Format::Ma,
            resistances: vec![50.0],
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

    let mut tokens = body.split_whitespace().peekable();
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
                // `R` is the one token whose values are positional. Real files
                // separate them generously (`R     50.00`); the glued forms
                // `R50` and `R=50` are not accepted here.
                //
                // The first value is mandatory and is reported by name if it
                // is not a number — `R abc` is a typo worth pointing at, not a
                // reason to claim `R` had no value. Any further values are
                // taken only while they parse, so the list ends at the next
                // keyword without needing to know what keywords there are.
                let first = tokens
                    .next()
                    .ok_or_else(|| invalid(line, "'r' given with no value"))?;
                let ohms: f64 = first.parse().map_err(|_| {
                    parse_err(line, ParseErrorKind::InvalidNumber(first.to_string()))
                })?;
                let mut values = vec![check_ohms(ohms, first, line)?];
                while let Some(&next) = tokens.peek() {
                    let Ok(ohms) = next.parse::<f64>() else { break };
                    tokens.next();
                    values.push(check_ohms(ohms, next, line)?);
                }
                opts.resistances = values;
            }
            other => {
                return Err(invalid(line, format!("unknown token '{other}'")));
            }
        }
    }

    Ok(opts)
}

/// A reference resistance has to be a positive, finite number of ohms.
///
/// `token` is quoted back rather than the parsed value, so `R 1e400` names
/// what the file actually says instead of the `inf` it became.
fn check_ohms(ohms: f64, token: &str, line: usize) -> Result<f64, Error> {
    if !ohms.is_finite() || ohms <= 0.0 {
        return Err(invalid(
            line,
            format!("reference resistance must be a positive number, got '{token}'"),
        ));
    }
    Ok(ohms)
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
                resistances: vec![50.0],
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
        // The runs of spaces and the trailing one are what real vendor
        // exports write; the tab form is what several instruments emit.
        assert_eq!(parse("  HZ   S   DB   R     50.00 ").resistances, [50.0]);
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
                resistances: vec![100.0],
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
        assert_eq!(parse(" R 50.00").resistances, [50.0]);
        assert_eq!(parse(" R 1e2").resistances, [100.0]);
        assert_eq!(parse(" R .5").resistances, [0.5]);
    }

    /// The 2.1 document's "Version 1.1" option line: one reference resistance
    /// per port instead of one for all of them. Files using it circulate, and
    /// before this they were rejected for an `unknown token '75'`.
    #[test]
    fn r_takes_one_value_per_port() {
        assert_eq!(parse(" GHZ S RI R 50 75").resistances, [50.0, 75.0]);
        assert_eq!(
            parse(" GHZ S RI R 50 75 100 25").resistances,
            [50.0, 75.0, 100.0, 25.0]
        );
        // Nothing here knows the port count, so nothing here objects to a list
        // that will not match it. That check belongs to the parser.
        assert_eq!(parse(" R 1 2 3 4 5 6 7").resistances.len(), 7);
    }

    /// The list is delimited by the next thing that is not a number, which is
    /// exact rather than a guess: no unit, parameter or format keyword parses
    /// as one.
    #[test]
    fn the_resistance_list_ends_at_the_next_keyword() {
        let expected = [50.0, 75.0];
        assert_eq!(parse(" R 50 75 GHZ").resistances, expected);
        assert_eq!(parse(" R 50 75 RI S").resistances, expected);
        assert_eq!(parse(" S R 50 75 MHZ DB").resistances, expected);
        // And the rest of the line is still read, not swallowed by the list.
        assert_eq!(parse(" R 50 75 MHZ DB").freq_unit, FreqUnit::MHz);
        assert_eq!(parse(" R 50 75 MHZ DB").format, Format::Db);
    }

    /// Every value gets the same scrutiny as a lone one — a negative or zero
    /// impedance is meaningless wherever it sits in the list, and the message
    /// names the offending token rather than the position.
    #[test]
    fn every_value_in_the_list_must_be_positive() {
        assert!(matches!(
            kind(" R 50 -75"),
            ParseErrorKind::InvalidOptionLine(m) if m.contains("'-75'")
        ));
        assert!(matches!(
            kind(" R 50 75 0"),
            ParseErrorKind::InvalidOptionLine(m) if m.contains("'0'")
        ));
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

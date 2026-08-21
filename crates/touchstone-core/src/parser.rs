//! The Touchstone v1 data-line parser.
//!
//! One pass over the logical lines: header comments, then the option line,
//! then one frequency block per data line. This version handles 2-port `RI`
//! files; everything outside that is rejected with a message that names the
//! limit rather than looking like a bug (see ADR 0004).

use num_complex::Complex64;

use crate::ParseOptions;
use crate::error::{Error, ParseErrorKind};
use crate::lines::{has_cr_only_line_endings, logical_lines};
use crate::model::{Format, Metadata, Network, Parameter, Version};
use crate::option_line::{Options, parse_option_line};

/// Parse a v1 Touchstone file.
pub(crate) fn parse_v1(input: &str, opts: &ParseOptions) -> Result<Network, Error> {
    if has_cr_only_line_endings(input) {
        return Err(err(1, ParseErrorKind::UnsupportedLineEndings));
    }

    let mut comments: Vec<String> = Vec::new();
    let mut options: Option<Options> = None;
    let mut option_line: Option<String> = None;
    // Where the option line was found, so a header-only file can point at
    // it instead of an arbitrary line 1.
    let mut option_line_number: Option<usize> = None;
    let mut nports = opts.nports;

    let mut freq_hz: Vec<f64> = Vec::new();
    let mut s: Vec<Complex64> = Vec::new();
    // Scratch buffer for the current data line. Reused rather than
    // reallocated per line, and the place where multi-line blocks will
    // accumulate once wrapped layouts are supported.
    let mut values: Vec<f64> = Vec::new();

    for line in logical_lines(input) {
        if line.content.is_empty() {
            // Blank, or nothing but a comment. Only the header block is
            // retained: files in the wild carry a comment on every data row
            // (a Philips transistor export has one per line), and keeping
            // hundreds of those costs allocations no consumer wants.
            if freq_hz.is_empty() {
                if let Some(text) = line.comment {
                    comments.push(text.to_string());
                }
            }
            continue;
        }

        if let Some(body) = line.content.strip_prefix('#') {
            // Spec v1.1 §3: option lines after the first are ignored.
            if options.is_none() {
                options = Some(parse_option_line(body, line.number)?);
                option_line = Some(line.content.to_string());
                option_line_number = Some(line.number);
            }
            continue;
        }

        let opts_ref = options
            .as_ref()
            .ok_or_else(|| err(line.number, ParseErrorKind::DataBeforeOptionLine))?;

        values.clear();
        for token in line.content.split_whitespace() {
            let value: f64 = token.parse().map_err(|_| {
                err(
                    line.number,
                    ParseErrorKind::InvalidNumber(token.to_string()),
                )
            })?;
            values.push(value);
        }

        // The parameter type and value format are properties of the file, so
        // they are checked before anything that depends on the port count —
        // an MA file should say so, not complain about a value count.
        check_option_scope(opts_ref, line.number)?;

        let n = match nports {
            Some(n) => n,
            // A value count matching no single-line layout says the line is
            // malformed, not that the file has that many ports. Report the
            // count mismatch against this version's only supported shape,
            // which is the actionable message.
            None => match infer_nports(values.len()) {
                Some(inferred) => {
                    nports = Some(inferred);
                    inferred
                }
                None => {
                    return Err(err(
                        line.number,
                        ParseErrorKind::WrongValueCount {
                            expected: values_per_point(SUPPORTED_PORTS),
                            found: values.len(),
                        },
                    ));
                }
            },
        };
        if n != SUPPORTED_PORTS {
            return Err(err(line.number, ParseErrorKind::UnsupportedPortCount(n)));
        }

        let scale = opts_ref.freq_unit.to_hz();

        // A noise section is five values per line whose frequency steps back
        // into the already-covered sweep — which is exactly how the spec says
        // a reader locates the boundary. Detected here so the failure names
        // itself instead of surfacing as a value-count or ordering error.
        if n == 2 && values.len() == NOISE_VALUES_PER_LINE && !freq_hz.is_empty() {
            let candidate = values[0] * scale;
            if freq_hz[freq_hz.len() - 1] >= candidate {
                return Err(err(line.number, ParseErrorKind::NoiseSectionUnsupported));
            }
        }

        let expected = values_per_point(n);
        if values.len() != expected {
            return Err(err(
                line.number,
                ParseErrorKind::WrongValueCount {
                    expected,
                    found: values.len(),
                },
            ));
        }

        let frequency = values[0] * scale;
        if !frequency.is_finite() {
            return Err(err(
                line.number,
                ParseErrorKind::InvalidNumber(values[0].to_string()),
            ));
        }
        if let Some(&previous) = freq_hz.last()
            && frequency <= previous
        {
            return Err(err(
                line.number,
                ParseErrorKind::FrequencyNotAscending {
                    previous_hz: previous,
                    current_hz: frequency,
                },
            ));
        }

        freq_hz.push(frequency);
        push_point(&mut s, &values[1..], n, opts_ref.format);
    }

    let Some(opts_ref) = options else {
        return Err(err(1, ParseErrorKind::MissingOptionLine));
    };
    if freq_hz.is_empty() {
        let line = option_line_number.expect("set alongside `options`, checked just above");
        return Err(err(
            line,
            ParseErrorKind::UnexpectedData("no data lines".to_string()),
        ));
    }
    let n = nports.expect("set alongside the first data line");

    Ok(Network {
        freq_hz,
        s,
        nports: n,
        z0: vec![opts_ref.resistance; n],
        noise: None,
        metadata: Metadata {
            version: Version::V1,
            freq_unit: opts_ref.freq_unit,
            parameter: opts_ref.parameter,
            format: opts_ref.format,
            resistance: opts_ref.resistance,
            option_line,
            comments,
        },
    })
}

/// Entries on one line of the noise section: frequency, NFmin, |Γopt|,
/// ∠Γopt, Rn.
const NOISE_VALUES_PER_LINE: usize = 5;

/// Values on one data line for an `n`-port network: a frequency plus one
/// real/imaginary pair per matrix entry.
fn values_per_point(n: usize) -> usize {
    1 + 2 * n * n
}

/// The only port count this version reads.
const SUPPORTED_PORTS: usize = 2;

/// Guess the port count from how many values a data line carried.
///
/// Unambiguous for the layouts that fit on one line: 3 values is a 1-port,
/// 9 a 2-port. `None` for anything else — a wrapped multi-line block cannot
/// be resolved from one line, and neither can a malformed one.
fn infer_nports(value_count: usize) -> Option<usize> {
    match value_count {
        3 => Some(1),
        9 => Some(2),
        _ => None,
    }
}

/// Reject parameter types and value formats outside this version's scope.
fn check_option_scope(opts: &Options, line: usize) -> Result<(), Error> {
    if opts.parameter != Parameter::S {
        return Err(err(
            line,
            ParseErrorKind::UnsupportedParameter(opts.parameter),
        ));
    }
    if opts.format != Format::Ri {
        return Err(err(line, ParseErrorKind::UnsupportedFormat(opts.format)));
    }
    Ok(())
}

/// Append one frequency point, converting to complex and reordering to
/// row-major `(row, column)`.
///
/// **Spec v1.1 §3: a 2-port v1 data line lists S11, S21, S12, S22 — 21
/// before 12**, unlike every other port count, which is plain row-major.
/// Getting this wrong silently transposes the matrix, and no passive
/// device's data can reveal the mistake, because a reciprocal network has
/// S21 == S12. The dedicated asymmetric fixture in the integration tests is
/// what guards this.
fn push_point(s: &mut Vec<Complex64>, pairs: &[f64], nports: usize, format: Format) {
    debug_assert_eq!(pairs.len(), 2 * nports * nports);
    let value = |i: usize| to_complex(pairs[2 * i], pairs[2 * i + 1], format);
    if nports == 2 {
        s.extend_from_slice(&[value(0), value(2), value(1), value(3)]);
    } else {
        s.extend((0..nports * nports).map(value));
    }
}

/// Build a complex value from an on-disk pair.
///
/// Only `RI` is reachable: [`check_supported`] rejects `MA` and `DB` before
/// any data is read. The MA/DB conversions land with the all-formats
/// milestone; the fallthrough is deliberately a value rather than a panic.
fn to_complex(a: f64, b: f64, format: Format) -> Complex64 {
    match format {
        Format::Ri => Complex64::new(a, b),
        Format::Ma | Format::Db => Complex64::new(a, b),
    }
}

fn err(line: usize, kind: ParseErrorKind) -> Error {
    Error::Parse { line, kind }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_per_point_counts_a_frequency_plus_one_pair_per_entry() {
        assert_eq!(values_per_point(1), 3);
        assert_eq!(values_per_point(2), 9);
        assert_eq!(values_per_point(4), 33);
    }

    #[test]
    fn port_count_is_inferred_only_from_single_line_layouts() {
        assert_eq!(infer_nports(3), Some(1));
        assert_eq!(infer_nports(9), Some(2));
        // A malformed or wrapped line is not a 19-port network.
        assert_eq!(infer_nports(19), None);
        assert_eq!(infer_nports(8), None);
        assert_eq!(infer_nports(0), None);
    }

    #[test]
    fn two_port_points_are_stored_transposed_from_file_order() {
        // File order S11 S21 S12 S22 -> row-major S11 S12 S21 S22.
        let mut s = Vec::new();
        push_point(
            &mut s,
            &[1.0, 0.0, 2.0, 0.0, 3.0, 0.0, 4.0, 0.0],
            2,
            Format::Ri,
        );
        assert_eq!(
            s,
            [
                Complex64::new(1.0, 0.0), // S11
                Complex64::new(3.0, 0.0), // S12, third pair in the file
                Complex64::new(2.0, 0.0), // S21, second pair in the file
                Complex64::new(4.0, 0.0), // S22
            ]
        );
    }
}

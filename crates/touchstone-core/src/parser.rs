//! The Touchstone v1 data-line parser.
//!
//! One pass over the logical lines: header comments, then the option line,
//! then the network data. Data for a 3-port or larger network spans several
//! lines, so the data values are accumulated into *data sets* rather than
//! read one point per line — see [`DataSets`] for the boundary rule.
//!
//! This version handles every value format (`RI`/`MA`/`DB`) at every port
//! count. Parameter types other than `S`, and the 2-port noise section, are
//! rejected with a message that names the limit rather than looking like a
//! bug (see ADR 0004).

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

    let mut sets = DataSets::new(opts.nports);
    // Scratch buffer for one line's tokens. Reused rather than reallocated
    // per line; `DataSets` owns the cross-line accumulation.
    let mut values: Vec<f64> = Vec::new();

    for line in logical_lines(input) {
        if line.content.is_empty() {
            // Blank, or nothing but a comment. Only the header block is
            // retained: files in the wild carry a comment on every data row
            // (a Philips transistor export has one per line), and keeping
            // hundreds of those costs allocations no consumer wants. Blank
            // lines between frequency blocks — which Keysight's multi-port
            // examples and the QUCS export both emit — fall through here
            // without disturbing a data set in progress.
            if sets.before_any_data() {
                if let Some(text) = line.comment {
                    comments.push(text.to_string());
                }
            }
            continue;
        }

        if let Some(body) = line.content.strip_prefix('#') {
            // Spec v1.1 §3: option lines after the first are ignored.
            if options.is_none() {
                let parsed = parse_option_line(body, line.number)?;
                // Checked here, once, against the option line itself — not
                // once per data row. This is a property of the file, not of
                // any particular row, so a data line should never be blamed
                // for it, and a parameter-only file (no data at all) should
                // report the parameter problem rather than "no data lines".
                check_option_scope(&parsed, line.number)?;
                options = Some(parsed);
                option_line = Some(line.content.to_string());
                option_line_number = Some(line.number);
            }
            continue;
        }

        let opts_ref = options
            .as_ref()
            .ok_or_else(|| err(line.number, ParseErrorKind::DataBeforeOptionLine))?;

        // `content` is trimmed and non-empty here, so there is at least one
        // token. Kept as a `&str` borrowed from the input: it is only read
        // when a frequency turns out to be unusable.
        let first_token = line.content.split_whitespace().next().unwrap_or_default();

        values.clear();
        for token in line.content.split_whitespace() {
            // Non-finite tokens are *not* rejected here. `-inf` is how a real
            // ADS export writes a zero-magnitude entry in a `DB` file, and it
            // converts to an exact complex zero; the finiteness check belongs
            // after conversion, where it can tell that apart from an `inf`
            // that stays infinite. See `push_point`.
            let value: f64 = token.parse().map_err(|_| {
                err(
                    line.number,
                    ParseErrorKind::InvalidNumber(token.to_string()),
                )
            })?;
            values.push(value);
        }

        sets.push_line(&values, first_token, line.number, opts_ref)?;
    }

    let Some(opts_ref) = options else {
        return Err(err(1, ParseErrorKind::MissingOptionLine));
    };
    sets.finish(&opts_ref)?;

    if sets.freq_hz.is_empty() {
        let line = option_line_number.expect("set alongside `options`, checked just above");
        return Err(err(line, ParseErrorKind::NoDataLines));
    }
    let n = sets.nports.expect("set alongside the first data set");

    Ok(Network {
        freq_hz: sets.freq_hz,
        s: sets.s,
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

/// Accumulates data values into *data sets* — one frequency point each —
/// and turns every completed set into a row of the network matrix.
///
/// A 1- or 2-port data set fits on one line, but a 3-port or larger one is
/// spread over several, so the parser cannot equate a line with a point. The
/// boundary rule is:
///
/// > **A line whose token count is odd starts a new data set; an even count
/// > continues the current one.**
///
/// That is exact, not a heuristic. Spec v1.1 §3 puts the frequency value
/// first in the *first* line of a data set and nowhere else, and every value
/// after it is part of a pair that is never split across lines — so a data
/// set's first line always holds `1 + 2k` tokens and every continuation line
/// holds `2k`. A 6-port file's lines run `9, 4, 8, 4, 8, 4, …`; only the
/// first of each set is odd.
///
/// Counting lines instead would break on the wild files this milestone
/// exists to read, which wrap inconsistently; trusting the blank lines that
/// Keysight's examples put between blocks would break on the many files that
/// omit them.
struct DataSets<'a> {
    freq_hz: Vec<f64>,
    s: Vec<Complex64>,
    /// `None` until the first complete data set fixes it, unless the caller
    /// or the `.sNp` extension supplied it up front.
    nports: Option<usize>,
    /// Values of the data set currently being accumulated.
    block: Vec<f64>,
    /// Source line `block` started on, so an error about a wrapped data set
    /// points at its beginning rather than at whichever line completed it.
    block_line: usize,
    /// The source token that became `block[0]`.
    ///
    /// Borrowed from the input rather than copied, so carrying it costs
    /// nothing per data set. It exists so a rejected frequency is quoted as
    /// the file spells it: `1e400` parses to `inf`, and reporting the parsed
    /// `f64` would tell the reader to search their file for a word that is
    /// not in it.
    frequency_token: &'a str,
}

impl<'a> DataSets<'a> {
    fn new(nports: Option<usize>) -> Self {
        DataSets {
            freq_hz: Vec::new(),
            s: Vec::new(),
            nports,
            block: Vec::new(),
            block_line: 0,
            frequency_token: "",
        }
    }

    /// Whether no data value has been seen yet — including a partially
    /// accumulated set, which still means the header block is over.
    fn before_any_data(&self) -> bool {
        self.freq_hz.is_empty() && self.block.is_empty()
    }

    /// Take one data line's values. `first_token` is that line's first token
    /// as it appears in the source, used only for error messages.
    fn push_line(
        &mut self,
        values: &[f64],
        first_token: &'a str,
        line: usize,
        opts: &Options,
    ) -> Result<(), Error> {
        if values.len() % 2 == 1 && !self.block.is_empty() {
            // An odd count opens a new data set, so whatever is buffered is
            // as complete as it will ever be.
            self.flush(opts)?;
        }
        if self.block.is_empty() {
            self.block_line = line;
            self.frequency_token = first_token;
        }
        self.block.extend_from_slice(values);

        // Once the port count is known, so is a data set's size, and waiting
        // for the next odd line to notice a completed set would only push
        // errors further from their cause — and would let a file whose *last*
        // set is truncated slip through to the end before failing.
        //
        // This is also where a port count that cannot describe a data set is
        // caught, on the first data line rather than after reading the file.
        if let Some(n) = self.nports {
            let expected = values_per_set(n, line)?;
            if self.block.len() >= expected {
                self.flush(opts)?;
            }
        }
        Ok(())
    }

    /// Emit any set still buffered at end of file.
    fn finish(&mut self, opts: &Options) -> Result<(), Error> {
        if self.block.is_empty() {
            return Ok(());
        }
        self.flush(opts)
    }

    /// Turn the buffered data set into one frequency point.
    fn flush(&mut self, opts: &Options) -> Result<(), Error> {
        debug_assert!(!self.block.is_empty(), "callers check before flushing");
        let line = self.block_line;
        let scale = opts.freq_unit.to_hz();

        let n = match self.nports {
            Some(n) => n,
            None => {
                let found = self.block.len();
                let n = nports_from_data_set(found)
                    .ok_or_else(|| err(line, ParseErrorKind::IndeterminatePortCount { found }))?;
                self.nports = Some(n);
                n
            }
        };

        // Spec v1.1 §3 allows a noise section only in 2-port files, and says
        // a reader finds its start by the frequency stepping back into the
        // already-covered sweep. The doc states the bound twice and
        // inconsistently — p10 says the first noise frequency is *less than*
        // the last S-parameter frequency, p11 says the lowest is *less than
        // or equal to* the highest — and `<=` is the only reading that
        // accepts Keysight's own example and the real ADS export, both of
        // which restart the noise sweep at the S-sweep's first frequency.
        //
        // Tested on a *completed* set, not on a 5-value line: a legitimate
        // 2-port set wrapped as 5 + 4 also opens with five values, and
        // misreading that as noise would reject a valid file.
        if n == 2
            && self.block.len() == NOISE_VALUES_PER_LINE
            && let Some(&last_s_freq) = self.freq_hz.last()
            && last_s_freq >= self.block[0] * scale
        {
            return Err(err(line, ParseErrorKind::NoiseSectionUnsupported));
        }

        let expected = values_per_set(n, line)?;
        if self.block.len() != expected {
            return Err(err(
                line,
                ParseErrorKind::WrongValueCount {
                    expected,
                    found: self.block.len(),
                },
            ));
        }

        let frequency = self.block[0] * scale;
        if !frequency.is_finite() {
            return Err(err(
                line,
                ParseErrorKind::InvalidNumber(self.frequency_token.to_string()),
            ));
        }
        if let Some(&previous) = self.freq_hz.last()
            && frequency <= previous
        {
            return Err(err(
                line,
                ParseErrorKind::FrequencyNotAscending {
                    previous_hz: previous,
                    current_hz: frequency,
                },
            ));
        }

        push_point(&mut self.s, &self.block[1..], n, opts.format)
            .map_err(|kind| err(line, kind))?;
        self.freq_hz.push(frequency);
        self.block.clear();
        Ok(())
    }
}

/// Entries on one line of the noise section: frequency, NFmin, |Γopt|,
/// ∠Γopt, Rn.
const NOISE_VALUES_PER_LINE: usize = 5;

/// Values in one data set for an `n`-port network: a frequency plus one
/// pair per matrix entry.
///
/// `None` for a port count that cannot describe a data set at all: zero, or
/// one so large that `1 + 2n²` overflows a `usize`.
///
/// This is not the arbitrary ceiling ADR 0006 declined to impose. Nothing
/// vets the count before it arrives — `parse_file` takes it from the
/// filename, so `x.s99999999999p` hands over 10¹¹ ports — and a count whose
/// data set will not fit in a `usize` cannot describe a file that fits on a
/// disk either. Left unchecked the multiplication wraps, and a wrapped
/// `expected` that happens to match the accumulated length would send
/// [`push_point`] indexing past the end of its slice.
fn values_per_point(n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    n.checked_mul(n)?.checked_mul(2)?.checked_add(1)
}

/// [`values_per_point`], or the error to report for a port count that cannot
/// describe a data set.
fn values_per_set(n: usize, line: usize) -> Result<usize, Error> {
    values_per_point(n).ok_or_else(|| err(line, ParseErrorKind::UnusablePortCount { nports: n }))
}

/// Port count implied by the size of one complete data set.
///
/// The inverse of [`values_per_point`]: `1 + 2n²` is strictly increasing in
/// `n`, so a set's length names its port count exactly. This is what lets a
/// file with no `.sNp` extension — a string handed to `parse_str`, say — be
/// read at all, and it resolves the one shape that looks ambiguous line by
/// line: a 4-port's first line holds nine tokens, exactly like a *complete*
/// 2-port set, but the 4-port's set runs on to 33 before the next odd line
/// closes it.
///
/// `None` for a length that solves for no whole `n`, which means a truncated
/// or malformed set rather than an exotic port count.
fn nports_from_data_set(len: usize) -> Option<usize> {
    // `len - 1` is the value count, which must be an even number of pairs.
    if len < 3 || len % 2 == 0 {
        return None;
    }
    let entries = (len - 1) / 2;
    let n = entries.isqrt();
    (n * n == entries).then_some(n)
}

/// Reject parameter types outside this version's scope.
///
/// Spec v1.1 §3 permits `S`/`Y`/`Z`/`G`/`H` at every port count; we read `S`
/// only. The value format is not restricted: all three convert.
fn check_option_scope(opts: &Options, line: usize) -> Result<(), Error> {
    if opts.parameter != Parameter::S {
        return Err(err(
            line,
            ParseErrorKind::UnsupportedParameter(opts.parameter),
        ));
    }
    Ok(())
}

/// Append one frequency point, converting to complex and reordering to
/// row-major `(row, column)`.
///
/// **Spec v1.1 §3: a 2-port v1 data set lists S11, S21, S12, S22 — 21
/// before 12**, unlike every other port count, which is plain row-major
/// (§3 p7–8 writes a 3-port as `<N11> <N12> <N13>` / `<N21> …`). Getting
/// this wrong silently transposes the matrix, and no passive device's data
/// can reveal the mistake, because a reciprocal network has S21 == S12. The
/// dedicated asymmetric fixtures in the integration tests are what guard it.
fn push_point(
    s: &mut Vec<Complex64>,
    pairs: &[f64],
    nports: usize,
    format: Format,
) -> Result<(), ParseErrorKind> {
    debug_assert_eq!(pairs.len(), 2 * nports * nports);
    let value = |i: usize| -> Result<Complex64, ParseErrorKind> {
        let (first, second) = (pairs[2 * i], pairs[2 * i + 1]);
        let z = to_complex(first, second, format);
        // Checked on the converted value rather than on the tokens, which is
        // what lets `-inf` dB through: it means a magnitude of exactly zero,
        // and `10^(-inf/20)` is `0.0`. An `inf` that stays infinite after
        // conversion, or any `NaN`, is still the plausible-looking wrong data
        // ADR 0004 argues against, and fails here.
        if z.is_finite() {
            Ok(z)
        } else {
            Err(ParseErrorKind::NonFiniteValue { first, second })
        }
    };
    if nports == 2 {
        let (s11, s21, s12, s22) = (value(0)?, value(1)?, value(2)?, value(3)?);
        s.extend_from_slice(&[s11, s12, s21, s22]);
    } else {
        s.reserve(nports * nports);
        for i in 0..nports * nports {
            s.push(value(i)?);
        }
    }
    Ok(())
}

/// Build a complex value from an on-disk pair, per spec v1.1 §3 p5.
fn to_complex(a: f64, b: f64, format: Format) -> Complex64 {
    match format {
        Format::Ri => Complex64::new(a, b),
        // Angles are in degrees "by convention" (§2 rule 5) and explicitly
        // for the data formats (§3 p5).
        Format::Ma => from_polar(a, b),
        // "DB for dB-angle (dB = 20*log10|magnitude|)" — so the magnitude is
        // 10^(dB/20), and the angle is handled exactly as in MA.
        Format::Db => from_polar(10f64.powf(a / 20.0), b),
    }
}

/// A magnitude and an angle *in degrees* as a complex number.
///
/// Written out rather than calling `Complex64::from_polar`, which lives
/// behind num-complex's `std` feature — deliberately off here, so the
/// workspace's `num-complex` unifies with the range rust-numpy accepts. The
/// arithmetic is the same.
fn from_polar(magnitude: f64, angle_deg: f64) -> Complex64 {
    let radians = angle_deg.to_radians();
    Complex64::new(magnitude * radians.cos(), magnitude * radians.sin())
}

fn err(line: usize, kind: ParseErrorKind) -> Error {
    Error::Parse { line, kind }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tolerance for a value that has been through a polar conversion.
    fn close(a: Complex64, b: Complex64) -> bool {
        (a - b).l1_norm() < 1e-12
    }

    #[test]
    fn values_per_point_counts_a_frequency_plus_one_pair_per_entry() {
        assert_eq!(values_per_point(1), Some(3));
        assert_eq!(values_per_point(2), Some(9));
        assert_eq!(values_per_point(4), Some(33));
        assert_eq!(values_per_point(16), Some(513));
    }

    /// The port count is not vetted before it reaches here: `parse_file`
    /// takes it from the filename. Unchecked, `1 + 2n²` wraps, and a wrapped
    /// size that happened to match the accumulated length would send
    /// `push_point` past the end of its slice.
    #[test]
    fn a_port_count_that_cannot_describe_a_data_set_is_rejected_not_wrapped() {
        // Zero ports is not a network, and would yield a `Network` whose
        // `at()` panics on every index.
        assert_eq!(values_per_point(0), None);

        // Both overflow points, written against `usize::BITS` so the test
        // means the same thing on a 32-bit target.
        //
        // `n * n` overflows from 2^(bits/2) up.
        let square_overflows = 1usize << (usize::BITS / 2);
        assert!(square_overflows.checked_mul(square_overflows).is_none());
        assert_eq!(values_per_point(square_overflows), None);
        assert_eq!(values_per_point(usize::MAX), None);

        // One below that, `n * n` fits and the *doubling* is what overflows
        // — the step a `checked_mul` on the square alone would miss.
        let doubling_overflows = square_overflows - 1;
        assert!(
            doubling_overflows
                .checked_mul(doubling_overflows)
                .is_some_and(|sq| sq.checked_mul(2).is_none())
        );
        assert_eq!(values_per_point(doubling_overflows), None);

        // Ordinary counts stay exact rather than clamped.
        assert_eq!(values_per_point(1000), Some(2_000_001));
    }

    #[test]
    fn a_data_set_length_names_its_port_count() {
        for n in 1..=32usize {
            let len = values_per_point(n).expect("small counts are usable");
            assert_eq!(nports_from_data_set(len), Some(n), "{n}-port");
        }
    }

    #[test]
    fn a_length_solving_for_no_whole_port_count_is_rejected() {
        // Even lengths cannot hold a frequency plus whole pairs.
        assert_eq!(nports_from_data_set(8), None);
        assert_eq!(nports_from_data_set(10), None);
        // Odd, but 2 and 3 entries are not square.
        assert_eq!(nports_from_data_set(5), None);
        assert_eq!(nports_from_data_set(7), None);
        // Degenerate: a frequency alone, or nothing.
        assert_eq!(nports_from_data_set(1), None);
        assert_eq!(nports_from_data_set(0), None);
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
        )
        .expect("finite");
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

    /// Spec v1.1 §3 p7–8: 3-port and larger matrices are plain row-major,
    /// with no trace of the 2-port swap above.
    #[test]
    fn three_port_points_keep_file_order() {
        let mut s = Vec::new();
        let pairs: Vec<f64> = (1..=9).flat_map(|i| [f64::from(i), 0.0]).collect();
        push_point(&mut s, &pairs, 3, Format::Ri).expect("finite");
        let reals: Vec<f64> = s.iter().map(|z| z.re).collect();
        assert_eq!(reals, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    }

    #[test]
    fn magnitude_angle_converts_through_the_unit_circle() {
        assert!(close(
            to_complex(2.0, 0.0, Format::Ma),
            Complex64::new(2.0, 0.0)
        ));
        assert!(close(
            to_complex(2.0, 90.0, Format::Ma),
            Complex64::new(0.0, 2.0)
        ));
        assert!(close(
            to_complex(1.0, 180.0, Format::Ma),
            Complex64::new(-1.0, 0.0)
        ));
        assert!(close(
            to_complex(1.0, -90.0, Format::Ma),
            Complex64::new(0.0, -1.0)
        ));
    }

    #[test]
    fn db_is_twenty_log_ten_of_the_magnitude() {
        // 20*log10(10) = 20 dB, and 0 dB is unity.
        assert!(close(
            to_complex(20.0, 0.0, Format::Db),
            Complex64::new(10.0, 0.0)
        ));
        assert!(close(
            to_complex(0.0, 0.0, Format::Db),
            Complex64::new(1.0, 0.0)
        ));
        // -6.020599913 dB is a half.
        assert!(close(
            to_complex(-6.020599913279624, 0.0, Format::Db),
            Complex64::new(0.5, 0.0)
        ));
        // A 10*log10 mix-up would put this at 0.1, not 0.31622...
        assert!(close(
            to_complex(-10.0, 0.0, Format::Db),
            Complex64::new(0.316_227_766_016_837_9, 0.0)
        ));
    }

    /// The real ADS `DB` export writes a zero-magnitude S12 as `-inf`, which
    /// is a legitimate exact zero rather than a bad token — the reason the
    /// finiteness check sits after conversion.
    #[test]
    fn minus_infinity_db_is_an_exact_zero() {
        let z = to_complex(f64::NEG_INFINITY, 0.0, Format::Db);
        assert!(z.is_finite());
        assert_eq!(z.l1_norm(), 0.0);

        let mut s = Vec::new();
        push_point(
            &mut s,
            &[f64::NEG_INFINITY, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            2,
            Format::Db,
        )
        .expect("-inf dB is a finite zero");
        assert_eq!(s[0].l1_norm(), 0.0);
    }

    #[test]
    fn a_value_that_stays_infinite_after_conversion_is_rejected() {
        for (format, pair) in [
            (Format::Ri, [f64::INFINITY, 0.0]),
            (Format::Ri, [0.0, f64::NAN]),
            (Format::Ma, [f64::INFINITY, 0.0]),
            // +inf dB is an infinite magnitude, unlike -inf.
            (Format::Db, [f64::INFINITY, 0.0]),
            (Format::Db, [0.0, f64::NAN]),
        ] {
            let mut s = Vec::new();
            assert!(
                matches!(
                    push_point(&mut s, &pair, 1, format),
                    Err(ParseErrorKind::NonFiniteValue { .. })
                ),
                "{format:?} {pair:?}"
            );
        }
    }
}

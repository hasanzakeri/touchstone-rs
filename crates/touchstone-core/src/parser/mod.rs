//! The data-line parsers, and the arithmetic both spec versions share.
//!
//! Touchstone 1.0 and 2.0 differ in how a file *describes itself*, not in what
//! its numbers mean. A v1 file states almost nothing and the reader infers the
//! rest — the port count from a data set's length, the noise section from a
//! frequency stepping backwards. A v2 file states all of it in keywords, so
//! its parser is a state machine rather than a set of inferences. The two are
//! different enough to belong in separate modules and identical enough that
//! everything below is written once and used by both: a value pair becomes a
//! complex number the same way in either version, and so does a noise point.

use num_complex::Complex64;

use crate::error::{Error, ParseErrorKind};
use crate::model::{Format, Parameter};
use crate::option_line::Options;

pub(crate) mod v1;

pub(crate) use v1::parse_v1;

/// Values in one noise data set: frequency, NFmin, |Γopt|, ∠Γopt, Rn.
///
/// Spec v1.1 §3 p10 and spec 2.0 p23 both put all five on one line. In v1 that
/// is load-bearing — five is odd, so under ADR 0006's accumulation rule every
/// noise line opens a set of its own and closes as soon as it is full. In v2
/// the `[Noise Data]` keyword delimits the section instead, and the count is
/// simply how many values a point holds.
pub(crate) const NOISE_VALUES_PER_SET: usize = 5;

/// Values in one data set for an `n`-port network: a frequency plus one pair
/// per matrix entry.
///
/// `None` for a port count that cannot describe a data set at all: zero, or
/// one so large that `1 + 2n²` overflows a `usize`.
///
/// This is not the arbitrary ceiling ADR 0006 declined to impose. Nothing vets
/// the count before it arrives — `parse_file` takes it from the filename, so
/// `x.s99999999999p` hands over 10¹¹ ports, and a v2 file's
/// `[Number of Ports]` is whatever integer it wrote — and a count whose data
/// set will not fit in a `usize` cannot describe a file that fits on a disk
/// either. Left unchecked the multiplication wraps, and a wrapped `expected`
/// that happens to match the accumulated length would send the placement
/// routines indexing past the end of their slice.
pub(crate) fn values_per_point(n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    n.checked_mul(n)?.checked_mul(2)?.checked_add(1)
}

/// [`values_per_point`], or the error to report for a port count that cannot
/// describe a data set.
pub(crate) fn values_per_set(n: usize, line: usize) -> Result<usize, Error> {
    values_per_point(n).ok_or_else(|| err(line, ParseErrorKind::UnusablePortCount { nports: n }))
}

/// Reject parameter types outside this version's scope.
///
/// Spec v1.1 §3 and spec 2.0 p6 both permit `S`/`Y`/`Z`/`G`/`H`; we read `S`
/// only. The value format is not restricted: all three convert.
pub(crate) fn check_option_scope(opts: &Options, line: usize) -> Result<(), Error> {
    if opts.parameter != Parameter::S {
        return Err(err(
            line,
            ParseErrorKind::UnsupportedParameter(opts.parameter),
        ));
    }
    Ok(())
}

/// Build a complex value from an on-disk pair, per spec v1.1 §3 p5 and spec
/// 2.0 p6 — which describe the three formats in identical terms.
pub(crate) fn to_complex(a: f64, b: f64, format: Format) -> Complex64 {
    match format {
        Format::Ri => Complex64::new(a, b),
        // Angles are in degrees "by convention" (v1.1 §2 rule 5, 2.0 rule 5)
        // and explicitly for the data formats.
        Format::Ma => from_polar(a, b),
        // "DB for dB-angle (dB = 20*log10|magnitude|)" — so the magnitude is
        // 10^(dB/20), and the angle is handled exactly as in MA.
        Format::Db => from_polar(10f64.powf(a / 20.0), b),
    }
}

/// A magnitude and an angle *in degrees* as a complex number.
///
/// Written out rather than calling `Complex64::from_polar`, which lives behind
/// num-complex's `std` feature — deliberately off here, so the workspace's
/// `num-complex` unifies with the range rust-numpy accepts. The arithmetic is
/// the same.
pub(crate) fn from_polar(magnitude: f64, angle_deg: f64) -> Complex64 {
    let radians = angle_deg.to_radians();
    Complex64::new(magnitude * radians.cos(), magnitude * radians.sin())
}

/// One noise point's five values, validated and converted.
///
/// Returns `(frequency_hz, nfmin_db, gamma_opt, rn)`. `rn` comes back exactly
/// as written; what that number *means* differs by version, which is the
/// caller's business — see [`crate::model::NoiseData`] and ADR 0010.
///
/// Spec v1.1 §3 p10 and spec 2.0 p23 give the entries as `<freq> <NFmin dB>
/// <|Γopt|> <∠Γopt> <Rn>` and both mark the third and fourth **"(MA)"** — a
/// linear magnitude and an angle in degrees — *whatever* the option line's
/// value format says. So Γopt is built with [`from_polar`] even in an `RI` or
/// `DB` file, in either spec version. The three ADS exports of one device in
/// `tests/data/` confirm it from the tool's side rather than the page's: their
/// noise sections are byte-for-byte identical while their S-data is written
/// three different ways.
///
/// Nothing here range-checks the values. `|Γopt|` above 1 is unphysical and
/// vendor documentation writes it *negative*; the spec likewise leaves
/// clamping an out-of-range `Rn` to the simulator, as something it *may* do.
/// This is an I/O layer — it reports the file. Non-finite values are the
/// exception and are rejected with the column named, because a row of five
/// bare numbers gives the reader nothing else to go on.
pub(crate) fn noise_point_from_values(
    values: &[f64; NOISE_VALUES_PER_SET],
    scale: f64,
    line: usize,
    frequency_token: &str,
) -> Result<(f64, f64, Complex64, f64), Error> {
    let &[in_units, nfmin_db, gamma_magnitude, gamma_angle_deg, rn] = values;

    // Checked first, because a frequency that is not a usable number makes
    // every later question about this point meaningless.
    let frequency = in_units * scale;
    if !frequency.is_finite() {
        return Err(err(
            line,
            ParseErrorKind::InvalidNumber(frequency_token.to_string()),
        ));
    }

    // Unlike the S-values, these are checked before conversion rather than
    // after. The two are equivalent for Γopt — finite inputs to `from_polar`
    // cannot produce a non-finite output, and a non-finite input always does —
    // and NFmin and Rn are stored as written, so there is no conversion to
    // check downstream of.
    for (column, value) in [
        ("nfmin", nfmin_db),
        ("|gamma_opt|", gamma_magnitude),
        ("angle(gamma_opt)", gamma_angle_deg),
        ("rn", rn),
    ] {
        if !value.is_finite() {
            return Err(err(
                line,
                ParseErrorKind::NonFiniteNoiseValue { column, value },
            ));
        }
    }

    Ok((
        frequency,
        nfmin_db,
        from_polar(gamma_magnitude, gamma_angle_deg),
        rn,
    ))
}

pub(crate) fn err(line: usize, kind: ParseErrorKind) -> Error {
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

    /// The port count is not vetted before it reaches here: `parse_file` takes
    /// it from the filename and a v2 file simply declares it. Unchecked,
    /// `1 + 2n²` wraps, and a wrapped size that happened to match the
    /// accumulated length would send a placement routine past the end of its
    /// slice.
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

    /// Γopt is polar in every format, so the same five tokens must produce the
    /// same point whichever way the S-data around them is written. This is the
    /// rule stated on the page; the byte-identical noise sections of the three
    /// ADS exports are the same rule observed in a real tool's output.
    #[test]
    fn a_noise_point_is_polar_regardless_of_the_files_format() {
        let values = [4.0, 0.7, 0.64, 69.0, 0.38];
        let (f, nfmin, gamma, rn) =
            noise_point_from_values(&values, 1e9, 1, "4").expect("all finite");
        assert_eq!((f, nfmin, rn), (4e9, 0.7, 0.38));
        assert!(close(gamma, from_polar(0.64, 69.0)));
        // Reading columns 3 and 4 as real/imaginary instead would put Γopt two
        // orders of magnitude out, and silently.
        assert!(gamma.l1_norm() < 1.0);
    }

    #[test]
    fn a_non_finite_noise_column_is_rejected_by_name() {
        for (index, column) in [
            (1, "nfmin"),
            (2, "|gamma_opt|"),
            (3, "angle(gamma_opt)"),
            (4, "rn"),
        ] {
            let mut values = [4.0, 0.7, 0.64, 69.0, 0.38];
            values[index] = f64::NAN;
            assert!(
                matches!(
                    noise_point_from_values(&values, 1e9, 1, "4"),
                    Err(Error::Parse {
                        kind: ParseErrorKind::NonFiniteNoiseValue { column: c, .. },
                        ..
                    }) if c == column
                ),
                "column {column}"
            );
        }
    }

    /// A frequency that overflows to infinity is quoted as the file spells it,
    /// not as the `f64` it became — `1e400` is not a word the reader can find
    /// by searching their file.
    #[test]
    fn a_non_finite_noise_frequency_is_quoted_as_written() {
        let values = [f64::INFINITY, 0.7, 0.64, 69.0, 0.38];
        assert!(matches!(
            noise_point_from_values(&values, 1e9, 7, "1e400"),
            Err(Error::Parse {
                line: 7,
                kind: ParseErrorKind::InvalidNumber(token),
                ..
            }) if token == "1e400"
        ));
    }
}

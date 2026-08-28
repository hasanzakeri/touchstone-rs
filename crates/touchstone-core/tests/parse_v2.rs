//! End-to-end tests for Touchstone 2.0 and 2.1 files.
//!
//! Three kinds of fixture, each doing something the others cannot:
//!
//! - **The specification's own worked examples**, transcribed as inline
//!   `&str` consts. These are the only fixtures that pin our reading against
//!   the document rather than against ourselves — most importantly Examples 5
//!   and 6, which give one 4-port device as both a Full and a Lower matrix and
//!   so settle the triangular layout with the document's own numbers.
//! - **Hand-written inline fixtures**, for shapes no exporter produces: a
//!   non-reciprocal 2-port written in both data orders, malformed headers, a
//!   truncated point.
//! - **Real exports in `tests/data/`**, which prove the parser agrees with
//!   what a tool actually writes — multi-line `[Reference]` payloads, blank
//!   lines between records, comments in places the grammar does not require.
//!
//! Where an example in the document uses a parameter type this version does
//! not read (`Z`, `H`), the transcription says `S` instead and says so. No
//! numeric value is ever altered.

use std::path::Path;

use touchstone_core::{
    Complex64, Error, Format, FreqUnit, MatrixFormat, Network, ParseErrorKind, TwoPortOrder,
    Version, parse_file, parse_str,
};

fn ok(input: &str) -> Network {
    parse_str(input).expect("should parse")
}

/// The `(line, kind)` of the parse error `input` produces.
fn fails(input: &str) -> (usize, ParseErrorKind) {
    match parse_str(input) {
        Err(Error::Parse { line, kind }) => (line, kind),
        other => panic!("expected a parse error, got {other:?}"),
    }
}

fn kind(input: &str) -> ParseErrorKind {
    fails(input).1
}

/// Every entry of two networks agrees to within `tolerance`.
fn assert_same_matrix(a: &Network, b: &Network, tolerance: f64, what: &str) {
    assert_eq!(a.nports, b.nports, "{what}: port count");
    assert_eq!(a.freq_hz, b.freq_hz, "{what}: frequencies");
    for fi in 0..a.nfreqs() {
        for row in 0..a.nports {
            for col in 0..a.nports {
                let (left, right) = (a.at(fi, row, col), b.at(fi, row, col));
                let error = (left - right).l1_norm();
                // `<=`, so a tolerance of zero means "bit-identical" rather
                // than "impossible".
                assert!(
                    error <= tolerance,
                    "{what}: S({},{}) at point {fi}: {left} vs {right} (off by {error})",
                    row + 1,
                    col + 1
                );
            }
        }
    }
}

/// A network's reference impedance, asserted flat across the sweep and real —
/// which everything a conforming file can declare is.
fn constant_z0(net: &Network) -> Vec<f64> {
    assert_eq!(net.z0.len(), net.nfreqs() * net.nports, "z0 must be (F, N)");
    let per_port: Vec<f64> = (0..net.nports)
        .map(|port| {
            let z = net.z0_at(0, port);
            assert_eq!(z.im, 0.0, "port {port}: a declared reference is real");
            z.re
        })
        .collect();
    for fi in 1..net.nfreqs() {
        for (port, &expected) in per_port.iter().enumerate() {
            assert_eq!(
                net.z0_at(fi, port),
                Complex64::new(expected, 0.0),
                "z0 changed at frequency {fi}, port {port}"
            );
        }
    }
    per_port
}

/// The smallest complete 2.0 file.
const MINIMAL: &str = "\
[Version] 2.0
# GHZ S RI R 50
[Number of Ports] 1
[Number of Frequencies] 1
[Network Data]
1.0 0.1 0.2
[End]
";

#[test]
fn a_minimal_v2_file_parses() {
    let net = ok(MINIMAL);
    assert_eq!(net.nports, 1);
    assert_eq!(net.freq_hz, [1e9]);
    assert_eq!(net.at(0, 0, 0), Complex64::new(0.1, 0.2));
    assert_eq!(constant_z0(&net), [50.0]);
    assert!(net.noise.is_none());

    assert_eq!(net.metadata.version, Version::V2_0);
    assert_eq!(net.metadata.freq_unit, FreqUnit::GHz);
    assert_eq!(net.metadata.format, Format::Ri);
    // Absent keywords are recorded as absent rather than as their defaults, so
    // a writer can tell "the file said Full" from "the file said nothing".
    assert_eq!(net.metadata.matrix_format, None);
    assert_eq!(net.metadata.two_port_order, None);
    assert_eq!(net.metadata.reference, None);
}

/// The 2.1 document states that apart from this string, 2.1 files are
/// identical to 2.0 files, and that it makes no difference which is written.
/// So the two must parse the same way and differ only in what they record.
#[test]
fn version_2_1_is_the_same_grammar() {
    let net = ok(&MINIMAL.replace("[Version] 2.0", "[Version] 2.1"));
    assert_eq!(net.metadata.version, Version::V2_1);
    assert_same_matrix(&net, &ok(MINIMAL), 0.0, "2.1 against 2.0");
}

#[test]
fn an_unknown_version_is_rejected_by_name() {
    assert_eq!(
        kind(&MINIMAL.replace("[Version] 2.0", "[Version] 3.0")),
        ParseErrorKind::UnsupportedVersion("3.0".into())
    );
}

/// **The ordering guard**, and the largest correctness risk in v2.
///
/// `[Two-Port Data Order]` exists because Touchstone 1.0 writes a 2-port as
/// S11 S21 S12 S22 — 21 before 12 — while enough tools adopted plain row-major
/// that a v2 file has to say which it means. Reading the argument backwards
/// transposes every matrix silently, and no reciprocal device's data can
/// reveal it: the fixtures here are deliberately unilateral, with S21 four
/// orders of magnitude above S12.
mod two_port_data_order {
    use super::*;

    /// `21_12`: the Touchstone 1.0 order, S21 in the second pair.
    const ORDER_21_12: &str = "\
[Version] 2.0
# GHZ S RI R 50
[Number of Ports] 2
[Two-Port Data Order] 21_12
[Number of Frequencies] 2
[Network Data]
1.0  0.1 0.2  9.0 9.1  0.01 0.02  0.3 0.4
2.0  0.5 0.6  8.0 8.1  0.03 0.04  0.7 0.8
[End]
";

    /// `12_21`: row-major, S12 in the second pair. The same device — every
    /// pair is identical, only the two middle ones are swapped.
    const ORDER_12_21: &str = "\
[Version] 2.0
# GHZ S RI R 50
[Number of Ports] 2
[Two-Port Data Order] 12_21
[Number of Frequencies] 2
[Network Data]
1.0  0.1 0.2  0.01 0.02  9.0 9.1  0.3 0.4
2.0  0.5 0.6  0.03 0.04  8.0 8.1  0.7 0.8
[End]
";

    /// The same device again in Touchstone 1.0, whose order is `21_12` by
    /// definition and which says so nowhere.
    const AS_V1: &str = "\
# GHZ S RI R 50
1.0  0.1 0.2  9.0 9.1  0.01 0.02  0.3 0.4
2.0  0.5 0.6  8.0 8.1  0.03 0.04  0.7 0.8
";

    #[test]
    fn twenty_one_twelve_puts_s21_in_the_second_pair() {
        let net = ok(ORDER_21_12);
        assert_eq!(net.at(0, 0, 0), Complex64::new(0.1, 0.2), "S11");
        assert_eq!(net.at(0, 1, 0), Complex64::new(9.0, 9.1), "S21");
        assert_eq!(net.at(0, 0, 1), Complex64::new(0.01, 0.02), "S12");
        assert_eq!(net.at(0, 1, 1), Complex64::new(0.3, 0.4), "S22");
        assert_eq!(net.metadata.two_port_order, Some(TwoPortOrder::S21First));
    }

    #[test]
    fn twelve_twenty_one_puts_s12_in_the_second_pair() {
        let net = ok(ORDER_12_21);
        assert_eq!(net.at(0, 0, 0), Complex64::new(0.1, 0.2), "S11");
        assert_eq!(net.at(0, 0, 1), Complex64::new(0.01, 0.02), "S12");
        assert_eq!(net.at(0, 1, 0), Complex64::new(9.0, 9.1), "S21");
        assert_eq!(net.at(0, 1, 1), Complex64::new(0.3, 0.4), "S22");
        assert_eq!(net.metadata.two_port_order, Some(TwoPortOrder::S12First));
    }

    /// The test the whole keyword exists for: two files that write the same
    /// device in the two permitted orders must read back identically. Swap the
    /// mapping and this fails loudly, where a reciprocal fixture could not
    /// tell the difference at all.
    #[test]
    fn the_two_orders_describe_the_same_device() {
        assert_same_matrix(&ok(ORDER_21_12), &ok(ORDER_12_21), 0.0, "21_12 vs 12_21");
    }

    /// And across versions: Touchstone 1.0's silent convention *is* `21_12`,
    /// so a v1 file and the `21_12` v2 file of one device must agree. This is
    /// what ties the v2 mapping to the v1 one that M1 pinned, rather than
    /// leaving the two free to drift into being wrong together.
    #[test]
    fn the_v1_convention_is_twenty_one_twelve() {
        assert_same_matrix(&ok(AS_V1), &ok(ORDER_21_12), 0.0, "v1 vs 21_12");
    }

    /// Required for a 2-port (spec 2.0 p8), and its absence is the case where
    /// a reader would otherwise have to guess — which is the situation the
    /// keyword was introduced to end.
    #[test]
    fn a_two_port_file_must_say_which_order_it_used() {
        let without = ORDER_21_12.replace("[Two-Port Data Order] 21_12\n", "");
        assert_eq!(
            kind(&without),
            ParseErrorKind::MissingKeyword("[Two-Port Data Order]")
        );
    }

    /// "Otherwise, it is not permitted" — the same page. A 4-port carrying it
    /// is telling the reader something that cannot be true.
    #[test]
    fn any_other_port_count_may_not_carry_it() {
        let input = MINIMAL.replace(
            "[Number of Frequencies] 1",
            "[Two-Port Data Order] 21_12\n[Number of Frequencies] 1",
        );
        assert!(matches!(
            kind(&input),
            ParseErrorKind::KeywordNotPermitted { keyword, detail }
                if keyword == "[Two-Port Data Order]" && detail.contains("1 ports")
        ));
    }

    #[test]
    fn an_unrecognized_order_is_rejected_rather_than_guessed() {
        let input = ORDER_21_12.replace("21_12", "11_22");
        assert!(matches!(
            kind(&input),
            ParseErrorKind::InvalidKeywordArgument { keyword, detail }
                if keyword == "[Two-Port Data Order]" && detail.contains("11_22")
        ));
    }
}

/// `[Matrix Format]` Lower and Upper, the feature the comparable Rust crate
/// does not implement.
///
/// The fixtures are the specification's Examples 5 and 6 (2.0 pp11–12): one
/// 4-port device written first as a Full matrix and then as a Lower one. They
/// are the document's own demonstration that the two describe the same
/// network, which makes them a far better oracle than anything we could
/// generate — a wrong triangular index map cannot agree with them by accident.
mod matrix_format {
    use super::*;

    /// Spec 2.0 Example 5, verbatim but for the parameter type: the document
    /// writes `# GHz S MA R 50`, which this already is.
    const EXAMPLE_5_FULL: &str = "\
! 4-port S-parameter data
! Default impedance is overridden by the [Reference] keyword arguments
[Version] 2.0
# GHz S MA R 50
[Number of Ports] 4
[Number of Frequencies] 1
[Reference] 50 75 0.01 0.01
[Matrix Format] Full
[Network Data]
5.00000 0.60 161.24 0.40 -42.20 0.42 -66.58 0.53 -79.34 !row 1
        0.40 -42.20 0.60 161.20 0.53 -79.34 0.42 -66.58 !row 2
        0.42 -66.58 0.53 -79.34 0.60 161.24 0.40 -42.20 !row 3
        0.53 -79.34 0.42 -66.58 0.40 -42.20 0.60 161.24 !row 4
[End]
";

    /// Spec 2.0 Example 6: the same device as a Lower matrix, and with the
    /// `[Reference]` arguments split across two lines — which the document
    /// calls out in its own comment.
    const EXAMPLE_6_LOWER: &str = "\
! 4-port S-parameter data
! Note that [Reference] arguments are split across two lines
[Version] 2.0
# GHz S MA R 50
[Number of Ports] 4
[Number of Frequencies] 1
[Reference] 50 75
0.01 0.01
[Matrix Format] Lower
[Network Data]
5.00000 0.60 161.24                                     !row 1
        0.40 -42.20 0.60 161.20                         !row 2
        0.42 -66.58 0.53 -79.34 0.60 161.24             !row 3
        0.53 -79.34 0.42 -66.58 0.40 -42.20 0.60 161.24 !row 4
[End]
";

    /// The Upper form of the same device. The document gives no Upper example
    /// with data, so this is transposed from Example 5's own numbers — every
    /// value appears there, only the layout is ours.
    const EXAMPLE_5_AS_UPPER: &str = "\
[Version] 2.0
# GHz S MA R 50
[Number of Ports] 4
[Number of Frequencies] 1
[Reference] 50 75 0.01 0.01
[Matrix Format] Upper
[Network Data]
5.00000 0.60 161.24 0.40 -42.20 0.42 -66.58 0.53 -79.34 !row 1
        0.60 161.20 0.53 -79.34 0.42 -66.58             !row 2
        0.60 161.24 0.40 -42.20                         !row 3
        0.60 161.24                                     !row 4
[End]
";

    /// The specification's own pair. If the triangular index map is wrong in
    /// any way — rows for columns, diagonal misplaced, mirror missing — the
    /// two stop agreeing.
    #[test]
    fn the_specs_full_and_lower_examples_are_the_same_network() {
        assert_same_matrix(
            &ok(EXAMPLE_5_FULL),
            &ok(EXAMPLE_6_LOWER),
            1e-12,
            "spec Example 5 (Full) vs Example 6 (Lower)",
        );
    }

    #[test]
    fn upper_describes_the_same_network_as_lower_and_full() {
        assert_same_matrix(
            &ok(EXAMPLE_5_FULL),
            &ok(EXAMPLE_5_AS_UPPER),
            1e-12,
            "Full vs Upper",
        );
        assert_same_matrix(
            &ok(EXAMPLE_6_LOWER),
            &ok(EXAMPLE_5_AS_UPPER),
            1e-12,
            "Lower vs Upper",
        );
    }

    /// A triangular file describes a *whole* matrix: the half it does not
    /// write is filled from the half it does. Without the mirror those entries
    /// would come back as zero, and a reciprocal device would look unilateral.
    #[test]
    fn the_unwritten_triangle_is_filled_by_symmetry() {
        let net = ok(EXAMPLE_6_LOWER);
        for row in 0..4 {
            for col in 0..4 {
                assert_eq!(
                    net.at(0, row, col),
                    net.at(0, col, row),
                    "S({},{}) against its transpose",
                    row + 1,
                    col + 1
                );
            }
        }
        // And nothing was left at zero.
        for fi in 0..net.nfreqs() {
            for row in 0..4 {
                for col in 0..4 {
                    assert!(net.at(fi, row, col).l1_norm() > 0.1, "S({row},{col}) unset");
                }
            }
        }
    }

    /// A Lower or Upper point holds `n² + n + 1` values against a Full point's
    /// `2n² + 1` — for a 4-port, 21 against 33. Nothing in the file states
    /// either count, so a reader that used the wrong one would reframe every
    /// point and read the file as a different network entirely.
    ///
    /// Counting the fixtures' own data values is what makes this a test of the
    /// framing rather than a restatement of the parser's arithmetic.
    #[test]
    fn a_triangular_point_holds_fewer_values_than_a_full_one() {
        fn data_values(fixture: &str) -> usize {
            fixture
                .lines()
                .skip_while(|line| !line.starts_with("[Network Data]"))
                .skip(1)
                .take_while(|line| !line.starts_with("[End]"))
                .map(|line| {
                    line.split('!')
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .count()
                })
                .sum()
        }
        assert_eq!(data_values(EXAMPLE_5_FULL), 33, "4-port Full: 2n^2 + 1");
        assert_eq!(
            data_values(EXAMPLE_6_LOWER),
            21,
            "4-port Lower: n^2 + n + 1"
        );
        assert_eq!(data_values(EXAMPLE_5_AS_UPPER), 21, "4-port Upper");

        // And both still yield a whole 4x4 matrix at one frequency.
        for fixture in [EXAMPLE_5_FULL, EXAMPLE_6_LOWER, EXAMPLE_5_AS_UPPER] {
            let net = ok(fixture);
            assert_eq!(net.nfreqs(), 1);
            assert_eq!(net.s.len(), 16);
        }
    }

    #[test]
    fn the_matrix_format_is_recorded() {
        assert_eq!(
            ok(EXAMPLE_5_FULL).metadata.matrix_format,
            Some(MatrixFormat::Full)
        );
        assert_eq!(
            ok(EXAMPLE_6_LOWER).metadata.matrix_format,
            Some(MatrixFormat::Lower)
        );
        assert_eq!(
            ok(EXAMPLE_5_AS_UPPER).metadata.matrix_format,
            Some(MatrixFormat::Upper)
        );
    }

    /// Case-insensitivity is a general rule of the format (2.0 rule 1), and it
    /// applies to keyword arguments as much as to keywords.
    #[test]
    fn the_argument_is_case_insensitive() {
        for spelling in ["Lower", "lower", "LOWER"] {
            let input = EXAMPLE_6_LOWER.replace(
                "[Matrix Format] Lower",
                &format!("[Matrix Format] {spelling}"),
            );
            assert_eq!(
                ok(&input).metadata.matrix_format,
                Some(MatrixFormat::Lower),
                "{spelling}"
            );
        }
    }

    #[test]
    fn an_unrecognized_matrix_format_is_rejected() {
        let input = EXAMPLE_6_LOWER.replace("Lower", "Diagonal");
        assert!(matches!(
            kind(&input),
            ParseErrorKind::InvalidKeywordArgument { keyword, detail }
                if keyword == "[Matrix Format]" && detail.contains("Diagonal")
        ));
    }
}

/// `[Reference]`, whose arguments may begin on the line after the keyword and
/// run across several lines (spec 2.0 p10).
mod reference {
    use super::*;

    const FOUR_PORT: &str = "\
[Version] 2.0
# GHz S MA R 50
[Number of Ports] 4
[Number of Frequencies] 1
[Reference] 50 75 100 25
[Network Data]
5.0 0.6 161.24 0.4 -42.2 0.42 -66.58 0.53 -79.34
    0.4 -42.2 0.6 161.2 0.53 -79.34 0.42 -66.58
    0.42 -66.58 0.53 -79.34 0.6 161.24 0.4 -42.2
    0.53 -79.34 0.42 -66.58 0.4 -42.2 0.6 161.24
[End]
";

    #[test]
    fn per_port_references_reach_z0_in_order() {
        let net = ok(FOUR_PORT);
        assert_eq!(constant_z0(&net), [50.0, 75.0, 100.0, 25.0]);
        assert_eq!(net.metadata.reference, Some(vec![50.0, 75.0, 100.0, 25.0]));
    }

    /// The layout real exports use: the keyword alone, then one value per line
    /// with its own trailing comment.
    ///
    /// This is the trap the module was written around. Once comments are
    /// stripped, a `[Reference]` payload line is indistinguishable from a data
    /// line — a reader that noted the keyword and then resumed its normal loop
    /// would take `50 75 100 25` as a frequency point and be four values into
    /// the file before anything looked wrong.
    #[test]
    fn arguments_may_begin_on_the_following_lines() {
        let split = FOUR_PORT.replace(
            "[Reference] 50 75 100 25",
            "[Reference]\n  50  ! Port[1]\n  75  ! Port[2]\n  100 ! Port[3]\n  25  ! Port[4]",
        );
        let net = ok(&split);
        assert_eq!(constant_z0(&net), [50.0, 75.0, 100.0, 25.0]);
        // And the data that followed was still read as data.
        assert_eq!(net.nfreqs(), 1);
        assert_eq!(net.freq_hz, [5e9]);
    }

    #[test]
    fn arguments_may_be_split_across_lines_unevenly() {
        let split = FOUR_PORT.replace("[Reference] 50 75 100 25", "[Reference] 50\n75 100\n25");
        assert_eq!(constant_z0(&ok(&split)), [50.0, 75.0, 100.0, 25.0]);
    }

    /// Spec 2.0 p10 requires an argument for every port. Unlike the option
    /// line's `R`, a lone value is not a shorthand for "all of them".
    #[test]
    fn too_few_values_are_reported_against_the_keyword() {
        let short = FOUR_PORT.replace("[Reference] 50 75 100 25", "[Reference] 50 75");
        assert_eq!(
            kind(&short),
            ParseErrorKind::WrongResistanceCount {
                source: "[Reference]",
                expected: 4,
                found: 2,
            }
        );
    }

    #[test]
    fn a_reference_impedance_must_be_positive() {
        let bad = FOUR_PORT.replace("[Reference] 50 75 100 25", "[Reference] 50 75 -100 25");
        assert!(matches!(
            kind(&bad),
            ParseErrorKind::InvalidKeywordArgument { keyword, detail }
                if keyword == "[Reference]" && detail.contains("-100")
        ));
    }

    /// Absent, the option line's `R` supplies the reference — which is the
    /// only source a v1 file ever has.
    #[test]
    fn the_option_line_supplies_the_reference_when_the_keyword_is_absent() {
        let without = FOUR_PORT.replace("[Reference] 50 75 100 25\n", "");
        let net = ok(&without);
        assert_eq!(constant_z0(&net), [50.0; 4]);
        assert_eq!(net.metadata.reference, None);
    }
}

/// Keyword-block rules: what is required, what may not repeat, what may not
/// appear, and what has to come first.
mod header_rules {
    use super::*;

    #[test]
    fn the_port_count_is_required() {
        let without = MINIMAL.replace("[Number of Ports] 1\n", "");
        assert_eq!(
            kind(&without),
            ParseErrorKind::MissingKeyword("[Number of Ports]")
        );
    }

    #[test]
    fn the_frequency_count_is_required() {
        let without = MINIMAL.replace("[Number of Frequencies] 1\n", "");
        assert_eq!(
            kind(&without),
            ParseErrorKind::MissingKeyword("[Number of Frequencies]")
        );
    }

    #[test]
    fn the_network_data_keyword_is_required() {
        let without = MINIMAL.replace("[Network Data]\n", "");
        assert_eq!(
            kind(&without),
            ParseErrorKind::MissingKeyword("[Network Data]")
        );
    }

    #[test]
    fn a_keyword_may_not_repeat() {
        let twice = MINIMAL.replace(
            "[Number of Frequencies] 1",
            "[Number of Frequencies] 1\n[Number of Frequencies] 2",
        );
        assert_eq!(
            kind(&twice),
            ParseErrorKind::DuplicateKeyword("[Number of Frequencies]")
        );
    }

    /// Spec 2.0 p8: `[Number of Ports]` is the first keyword after the option
    /// line, and the keywords that describe ports lean on it having been read.
    #[test]
    fn the_port_count_must_follow_the_option_line() {
        let input = "\
[Version] 2.0
[Number of Ports] 1
# GHZ S RI R 50
[Number of Frequencies] 1
[Network Data]
1.0 0.1 0.2
[End]
";
        assert!(matches!(
            kind(input),
            ParseErrorKind::KeywordOutOfOrder { keyword, .. } if keyword == "[Number of Ports]"
        ));
    }

    #[test]
    fn reference_must_follow_the_port_count() {
        let input = MINIMAL.replace("[Number of Ports] 1", "[Reference] 50\n[Number of Ports] 1");
        assert!(matches!(
            kind(&input),
            ParseErrorKind::KeywordOutOfOrder { keyword, detail }
                if keyword == "[Reference]" && detail.contains("[Number of Ports]")
        ));
    }

    #[test]
    fn a_count_keyword_needs_a_positive_integer() {
        for bad in ["0", "-1", "two", ""] {
            let input = MINIMAL.replace("[Number of Ports] 1", &format!("[Number of Ports] {bad}"));
            assert!(
                matches!(
                    kind(&input),
                    ParseErrorKind::InvalidKeywordArgument { keyword, .. }
                        if keyword == "[Number of Ports]"
                ),
                "{bad:?}"
            );
        }
    }

    /// The declared count is validated against the data rather than used to
    /// drive the read, so a file that disagrees with itself says so instead of
    /// quietly returning whichever number happened to win.
    #[test]
    fn a_frequency_count_that_disagrees_with_the_data_is_an_error() {
        let input = MINIMAL.replace("[Number of Frequencies] 1", "[Number of Frequencies] 2");
        assert_eq!(
            kind(&input),
            ParseErrorKind::DeclaredCountMismatch {
                keyword: "[Number of Frequencies]",
                declared: 2,
                found: 1,
            }
        );
    }

    /// Mixed-mode data is arranged by mode rather than by port, so a file
    /// carrying `[Mixed-Mode Order]` cannot be read as if it were single-ended
    /// — every value would land in the wrong place. Out of scope, and said so.
    #[test]
    fn mixed_mode_data_is_refused_by_name() {
        let input = MINIMAL.replace("[Network Data]", "[Mixed-Mode Order] S1\n[Network Data]");
        assert_eq!(kind(&input), ParseErrorKind::MixedModeUnsupported);
    }

    /// Spec 2.0 p26 reserves `[Begin Information]` for keywords it does not
    /// define, so there is nothing inside to read — but it still has to be
    /// stepped over rather than fallen into.
    #[test]
    fn an_information_block_is_skipped_whole() {
        let input = MINIMAL.replace(
            "[Network Data]",
            "[Begin Information]\nWhatever Future Keyword 3\nand its arguments\n[End Information]\n[Network Data]",
        );
        let net = ok(&input);
        assert_eq!(net.freq_hz, [1e9]);
    }

    #[test]
    fn an_unclosed_information_block_is_reported() {
        let input = MINIMAL.replace(
            "[Network Data]",
            "[Begin Information]\nsomething\n[Network Data]",
        );
        assert!(matches!(
            kind(&input),
            ParseErrorKind::KeywordOutOfOrder { keyword, .. } if keyword == "[Network Data]"
        ));
    }
}

/// Data-line rules. Spec 2.0 p13 makes line breaks meaningless in v2: a point
/// may be split anywhere or written on one line, and a new point begins every
/// `2n² + 1` values. That is exact here in a way it never was for v1, because
/// the port count and matrix format are stated rather than inferred.
mod data_lines {
    use super::*;

    const FOUR_PORT_ONE_LINE: &str = "\
[Version] 2.0
# GHZ S RI R 50
[Number of Ports] 2
[Two-Port Data Order] 12_21
[Number of Frequencies] 2
[Network Data]
1.0 0.1 0.2 0.3 0.4 0.5 0.6 0.7 0.8 2.0 0.9 1.0 1.1 1.2 1.3 1.4 1.5 1.6
[End]
";

    /// Two whole points on one line, which v1's odd/even rule could not have
    /// framed. Nothing separates them but the count.
    #[test]
    fn several_points_may_share_one_line() {
        let net = ok(FOUR_PORT_ONE_LINE);
        assert_eq!(net.freq_hz, [1e9, 2e9]);
        assert_eq!(net.at(0, 0, 0), Complex64::new(0.1, 0.2));
        assert_eq!(net.at(1, 1, 1), Complex64::new(1.5, 1.6));
    }

    /// And the same values split at a point no rule would predict — mid-pair,
    /// even — must read identically.
    #[test]
    fn a_point_may_be_split_anywhere() {
        let awkward = "\
[Version] 2.0
# GHZ S RI R 50
[Number of Ports] 2
[Two-Port Data Order] 12_21
[Number of Frequencies] 2
[Network Data]
1.0 0.1
0.2 0.3 0.4 0.5
0.6 0.7
0.8 2.0 0.9 1.0 1.1
1.2 1.3 1.4 1.5 1.6
[End]
";
        assert_same_matrix(
            &ok(FOUR_PORT_ONE_LINE),
            &ok(awkward),
            0.0,
            "one line vs split",
        );
    }

    #[test]
    fn blank_lines_and_comments_between_points_are_ignored() {
        let spaced = FOUR_PORT_ONE_LINE.replace(
            "[Network Data]\n",
            "[Network Data]\n\n! a comment between the keyword and the data\n\n",
        );
        assert_same_matrix(&ok(FOUR_PORT_ONE_LINE), &ok(&spaced), 0.0, "spaced");
    }

    #[test]
    fn frequencies_must_increase() {
        let input = MINIMAL.replace(
            "[Number of Frequencies] 1\n[Network Data]\n1.0 0.1 0.2",
            "[Number of Frequencies] 2\n[Network Data]\n2.0 0.1 0.2\n1.0 0.3 0.4",
        );
        assert!(matches!(
            kind(&input),
            ParseErrorKind::FrequencyNotAscending { .. }
        ));
    }

    /// A point cut short at end of file is a truncated file, reported as the
    /// count it is rather than as a frequency count that happens to disagree.
    #[test]
    fn a_truncated_final_point_is_reported_as_a_value_count() {
        let input = MINIMAL.replace("1.0 0.1 0.2", "1.0 0.1");
        assert_eq!(
            kind(&input),
            ParseErrorKind::WrongValueCount {
                expected: 3,
                found: 2
            }
        );
    }

    #[test]
    fn a_token_that_is_not_a_number_is_quoted_back() {
        let input = MINIMAL.replace("1.0 0.1 0.2", "1.0 0.1 oops");
        assert_eq!(kind(&input), ParseErrorKind::InvalidNumber("oops".into()));
    }

    /// Spec 2.0 p25: `[End]` closes the file and non-comment text after it is
    /// an error. Comments are not.
    #[test]
    fn data_after_end_is_an_error_but_comments_are_not() {
        let input = format!("{MINIMAL}2.0 0.3 0.4\n");
        assert_eq!(kind(&input), ParseErrorKind::TrailingDataAfterEnd);

        let commented = format!("{MINIMAL}! trailing remarks are fine\n");
        assert_eq!(ok(&commented).nfreqs(), 1);
    }

    /// `[End]` is not required. Nothing becomes ambiguous without it — the
    /// data simply ends — and rejecting an otherwise complete file over a
    /// missing terminator would discard good data for no diagnostic gain.
    #[test]
    fn a_missing_end_keyword_is_tolerated() {
        let without = MINIMAL.replace("[End]\n", "");
        assert_same_matrix(&ok(MINIMAL), &ok(&without), 0.0, "with vs without [End]");
    }
}

/// How a file that is not quite either version is diagnosed.
mod version_dispatch {
    use super::*;

    /// Keywords are not permitted in 1.x files, so one appearing without a
    /// `[Version]` is worth saying outright. Before this, the reader was told
    /// that some line held an unparseable number.
    #[test]
    fn a_v2_keyword_without_a_version_names_the_problem() {
        let input = "# GHZ S RI R 50\n[Number of Ports] 1\n1.0 0.1 0.2\n";
        assert_eq!(
            kind(input),
            ParseErrorKind::V2KeywordInV1File("[Number of Ports]")
        );
    }

    /// `[Version]` shall precede all other non-comment lines, so a file whose
    /// first keyword is something else is malformed however it is read.
    #[test]
    fn a_leading_keyword_that_is_not_version_is_rejected() {
        let input = "[Number of Ports] 1\n# GHZ S RI R 50\n[Network Data]\n1.0 0.1 0.2\n";
        assert!(matches!(
            kind(input),
            ParseErrorKind::KeywordOutOfOrder { keyword, .. } if keyword == "[Number of Ports]"
        ));
    }

    /// Comments and blank lines come before everything, including `[Version]`.
    #[test]
    fn comments_may_precede_the_version_keyword() {
        let input = format!("! a header comment\n\n{MINIMAL}");
        let net = ok(&input);
        assert_eq!(net.metadata.version, Version::V2_0);
        assert_eq!(net.metadata.comments, ["a header comment"]);
    }

    /// A misspelling is named rather than read as data — reading
    /// `[Netwrok Data]` as a number would turn a typo into a different file.
    #[test]
    fn a_misspelled_keyword_is_quoted_back() {
        let input = MINIMAL.replace("[Network Data]", "[Netwrok Data]");
        assert_eq!(
            kind(&input),
            ParseErrorKind::UnknownKeyword("[Netwrok Data]".into())
        );
    }

    /// The 2.0 errata's own correction: the document's keyword list on p5
    /// prints `[Two-Port Order]`, which is not the keyword. A file written to
    /// that list gets told so rather than being half-read.
    #[test]
    fn the_erratas_wrong_keyword_name_is_not_accepted() {
        let input = MINIMAL.replace("[Network Data]", "[Two-Port Order] 21_12\n[Network Data]");
        assert_eq!(
            kind(&input),
            ParseErrorKind::UnknownKeyword("[Two-Port Order]".into())
        );
    }
}

/// The `[Noise Data]` section.
///
/// The five columns mean the same thing as in v1 — Γopt is a magnitude and an
/// angle whatever the option line says — with one exception that the
/// specification demonstrates itself: the effective noise resistance is
/// normalized in a 1.x file and in ohms in a 2.x one.
mod noise {
    use super::*;

    /// Spec 2.0 Example 18: the v1 form of a 2-port with noise.
    const EXAMPLE_18_V1: &str = "\
!2-port network, S-parameter and noise data
!Default MA format, GHz frequencies, 50 ohm reference, S-parameters
#
! NETWORK PARAMETERS
2   .95 -26   3.57 157 .04 76 .66 -14
22 .60 -144 1.30 40   .14 40 .56 -85
! NOISE PARAMETERS
4    .7 .64  69 .38
18 2.7 .46 -33 .40
";

    /// Spec 2.0 Example 20: the same device in 2.0 syntax.
    ///
    /// Transcribed with **`[Two-Port Data Order] 21_12` added**, which the
    /// document's own example omits. Page 8 makes the keyword required when
    /// the port count is 2, so Examples 19 and 20 as printed contradict it —
    /// Examples 3, 12 and 17 all include it. `21_12` is the order the data is
    /// in, being the same rows as Example 18. No value is altered.
    const EXAMPLE_20_V2: &str = "\
!2-port network, S-parameter and noise data
!Default MA format, GHz frequencies, 50 ohm reference, S-parameters
[Version] 2.0
#
[Number of Ports] 2
[Two-Port Data Order] 21_12
[Number of Frequencies] 2
[Number of Noise Frequencies] 2
[Reference] 50 25.0
[Network Data]
! NETWORK PARAMETERS
2   .95 -26   3.57 157 .04 76 .66 -14
22 .60 -144 1.30 40   .14 40 .56 -85
[Noise Data]
! NOISE PARAMETERS
4    .7 .64  69 19
18 2.7 .46 -33 20
[End]
";

    #[test]
    fn the_section_is_read_whole() {
        let net = ok(EXAMPLE_20_V2);
        let noise = net.noise.as_ref().expect("the file has a noise section");
        assert_eq!(noise.freq_hz, [4e9, 18e9]);
        assert_eq!(noise.nfmin_db, [0.7, 2.7]);
        assert_eq!(noise.rn, [19.0, 20.0]);
        // Γopt is a magnitude and an angle in degrees regardless of format —
        // reading `.64 69` as real/imaginary would give 0.64 + 69i.
        assert!(noise.gamma_opt[0].l1_norm() < 1.0);
        assert!((noise.gamma_opt[0].norm() - 0.64).abs() < 1e-12);
    }

    /// **The version divergence**, in the specification's own paired examples.
    ///
    /// One device, written twice. Four of the five noise columns are identical
    /// between them; the fifth reads `.38` in the 1.0 file and `19` in the 2.0
    /// one, because 1.x normalizes the effective noise resistance to the
    /// option line's reference and 2.x writes ohms. `rn` therefore differs by
    /// construction, and `rn_ohms` must not.
    #[test]
    fn rn_differs_between_versions_and_rn_ohms_does_not() {
        let (v1, v2) = (ok(EXAMPLE_18_V1), ok(EXAMPLE_20_V2));
        let (a, b) = (v1.noise.unwrap(), v2.noise.unwrap());

        assert_eq!(a.rn, [0.38, 0.40], "1.0 normalizes to the option line's R");
        assert_eq!(b.rn, [19.0, 20.0], "2.0 writes ohms");
        assert_ne!(a.rn, b.rn, "the field the document shows differing");

        assert_eq!(a.rn_ohms, [19.0, 20.0]);
        assert_eq!(b.rn_ohms, [19.0, 20.0]);

        // And everything else about the section is untouched by the change.
        assert_eq!(a.freq_hz, b.freq_hz);
        assert_eq!(a.nfmin_db, b.nfmin_db);
        assert_eq!(a.gamma_opt, b.gamma_opt);
    }

    /// `[Reference]` has no effect on noise data (spec 2.0 p24) — Γopt and the
    /// normalization stay against the option line's `R`. Example 20 is the
    /// case that can tell: its ports are referenced to 50 and 25 Ω, so a
    /// reader that used `z0` would produce a different answer for port 2.
    #[test]
    fn the_reference_keyword_does_not_reach_the_noise_section() {
        let net = ok(EXAMPLE_20_V2);
        assert_eq!(constant_z0(&net), [50.0, 25.0]);
        assert_eq!(net.noise.unwrap().rn_ohms, [19.0, 20.0]);
    }

    #[test]
    fn a_file_without_a_noise_section_has_no_noise() {
        assert!(ok(MINIMAL).noise.is_none());
    }

    /// Both directions of spec 2.0 p24's conditional: the count is required
    /// when there is a section, and prohibited when there is not.
    #[test]
    fn the_noise_count_and_the_noise_section_require_each_other() {
        let no_count = EXAMPLE_20_V2.replace("[Number of Noise Frequencies] 2\n", "");
        assert_eq!(
            kind(&no_count),
            ParseErrorKind::MissingKeyword("[Number of Noise Frequencies]")
        );

        let no_section = MINIMAL.replace(
            "[Number of Frequencies] 1",
            "[Number of Frequencies] 1\n[Number of Noise Frequencies] 2",
        );
        assert!(matches!(
            kind(&no_section),
            ParseErrorKind::KeywordNotPermitted { keyword, .. }
                if keyword == "[Number of Noise Frequencies]"
        ));
    }

    #[test]
    fn a_declared_noise_count_is_checked_against_the_rows() {
        let input = EXAMPLE_20_V2.replace(
            "[Number of Noise Frequencies] 2",
            "[Number of Noise Frequencies] 3",
        );
        assert_eq!(
            kind(&input),
            ParseErrorKind::DeclaredCountMismatch {
                keyword: "[Number of Noise Frequencies]",
                declared: 3,
                found: 2,
            }
        );
    }

    /// Noise parameters are defined for 2-port networks only.
    #[test]
    fn a_noise_section_needs_two_ports() {
        let input = MINIMAL
            .replace(
                "[Number of Frequencies] 1",
                "[Number of Frequencies] 1\n[Number of Noise Frequencies] 1",
            )
            .replace("[End]", "[Noise Data]\n4 0.7 0.64 69 19\n[End]");
        assert_eq!(
            kind(&input),
            ParseErrorKind::NoiseRequiresTwoPorts { nports: 1 }
        );
    }

    /// Spec 2.0 p24 groups each noise point onto one line. Requiring that is
    /// what turns a truncated row into a message naming the row, rather than a
    /// count mismatch discovered pages later — and unlike v1, the section is
    /// delimited by a keyword, so there is no wrapping to be tolerant of.
    #[test]
    fn a_noise_row_must_hold_exactly_five_values() {
        let short = EXAMPLE_20_V2.replace("4    .7 .64  69 19", "4 .7 .64 69");
        assert!(matches!(
            kind(&short),
            ParseErrorKind::MalformedNoiseLine { found: 4, .. }
        ));

        let long = EXAMPLE_20_V2.replace("4    .7 .64  69 19", "4 .7 .64 69 19 20");
        assert!(matches!(
            kind(&long),
            ParseErrorKind::MalformedNoiseLine { found: 6, .. }
        ));
    }

    /// The malformed-row message names where the section began, because a
    /// reader's first question is usually whether it began in the right place.
    #[test]
    fn a_malformed_row_names_the_sections_first_line() {
        let input = EXAMPLE_20_V2.replace("18 2.7 .46 -33 20", "18 2.7 .46 -33");
        let (line, kind) = fails(&input);
        let noise_line = EXAMPLE_20_V2
            .lines()
            .position(|l| l.starts_with("[Noise Data]"))
            .expect("the fixture has the keyword")
            + 1;
        assert!(line > noise_line);
        assert!(matches!(
            kind,
            ParseErrorKind::MalformedNoiseLine { noise_starts_at, .. }
                if noise_starts_at == noise_line
        ));
    }

    #[test]
    fn noise_frequencies_must_increase() {
        let input = EXAMPLE_20_V2.replace("18 2.7 .46 -33 20", "4 2.7 .46 -33 20");
        assert!(matches!(
            kind(&input),
            ParseErrorKind::NoiseFrequencyNotAscending { .. }
        ));
    }

    /// The noise grid need not match the S-parameter grid, in either length or
    /// spacing — Example 20's own noise points are 4 and 18 GHz against
    /// S-parameters at 2 and 22.
    #[test]
    fn the_noise_grid_is_independent_of_the_s_parameter_grid() {
        let net = ok(EXAMPLE_20_V2);
        assert_eq!(net.freq_hz, [2e9, 22e9]);
        assert_eq!(net.noise.unwrap().freq_hz, [4e9, 18e9]);
    }

    /// Unlike v1, where the rule is the only way to find the section at all, a
    /// v2 noise sweep that starts above the network sweep is still read
    /// exactly right — the keyword has already said where the section is. See
    /// the module documentation in `parser/v2.rs`.
    #[test]
    fn a_noise_sweep_above_the_network_sweep_is_still_read() {
        let input = EXAMPLE_20_V2
            .replace("4    .7 .64  69 19", "30 .7 .64 69 19")
            .replace("18 2.7 .46 -33 20", "40 2.7 .46 -33 20");
        let net = ok(&input);
        assert_eq!(net.noise.unwrap().freq_hz, [30e9, 40e9]);
    }
}

/// Real exports. These prove the parser agrees with what a tool writes, rather
/// than only with the inline fixtures written alongside it.
mod real_files {
    use super::*;

    const V2_4PORT_RI: &str = include_str!("../../../tests/data/hfss_v2_symmetric_4port_ri.ts");
    const V2_4PORT_MA: &str = include_str!("../../../tests/data/hfss_v2_symmetric_4port_ma.ts");
    const V2_4PORT_DB: &str = include_str!("../../../tests/data/hfss_v2_symmetric_4port_db.ts");
    const V2_WAVEGUIDE: &str = include_str!("../../../tests/data/hfss_v2_waveguide_2port_ri.ts");

    /// The multi-line `[Reference]` payload in its natural habitat: the
    /// keyword alone on its line, then one value per line, indented, each with
    /// a trailing `! Port[n]` comment.
    #[test]
    fn a_real_v2_export_reads_its_reference_block_as_references() {
        let net = ok(V2_4PORT_RI);
        assert_eq!(net.nports, 4);
        assert_eq!(net.nfreqs(), 51);
        assert_eq!(constant_z0(&net), [50.0; 4]);
        // The tell that the reference block was not eaten as data: four stray
        // values would have shifted every point and left the count wrong.
        assert_eq!(net.freq_hz[0], 1e8);
        assert_eq!(net.freq_hz[50], 2e10);
        assert_eq!(net.metadata.reference, Some(vec![50.0; 4]));
        assert_eq!(net.metadata.matrix_format, Some(MatrixFormat::Full));
    }

    /// One device exported three ways. Agreement needs no hand-computed
    /// expectation, and a wrong dB base or a degrees/radians slip fails it at
    /// once.
    #[test]
    fn the_three_formats_of_one_export_agree() {
        let (ri, ma, db) = (ok(V2_4PORT_RI), ok(V2_4PORT_MA), ok(V2_4PORT_DB));
        assert_eq!(ri.metadata.format, Format::Ri);
        assert_eq!(ma.metadata.format, Format::Ma);
        assert_eq!(db.metadata.format, Format::Db);
        // Set by the files' own precision, not by the conversion.
        assert_same_matrix(&ri, &ma, 1e-9, "RI vs MA");
        assert_same_matrix(&ri, &db, 1e-9, "RI vs DB");
    }

    /// A real 2-port v2 file carrying `[Two-Port Data Order] 12_21`. The
    /// device is reciprocal — it is a passive waveguide — so it cannot detect
    /// a transposed read; what it proves is that the keyword and its argument
    /// are accepted as a real tool writes them.
    #[test]
    fn a_real_two_port_export_declares_its_data_order() {
        let net = ok(V2_WAVEGUIDE);
        assert_eq!(net.nports, 2);
        assert_eq!(net.nfreqs(), 121);
        assert_eq!(net.metadata.two_port_order, Some(TwoPortOrder::S12First));
        assert_eq!(net.freq_hz[0], 6e9);
        assert_eq!(net.freq_hz[120], 1.2e10);
    }

    /// The `.ts` extension yields no port count, so `[Number of Ports]` is the
    /// only thing that can supply one — which is the arrangement spec 2.0 p4
    /// describes when it suggests the extension.
    #[test]
    fn a_ts_file_takes_its_port_count_from_the_keyword() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/data/hfss_v2_symmetric_4port_ri.ts");
        let net = parse_file(&path).expect("should parse from disk");
        assert_eq!(net.nports, 4);
    }

    /// The two data orders on **real** data, and the version divergence on it
    /// too.
    ///
    /// No tool available for this milestone writes a non-reciprocal 2-port in
    /// Touchstone 2.0, so these are derived from a committed export: the same
    /// amplifier, rewritten in 2.0 syntax twice, once in each data order, with
    /// the noise section's `Rn` converted to ohms as a 2.0 file must write it.
    /// Every other token is reproduced exactly. That the numbers came from a
    /// real export is what makes the agreement below worth asserting — the
    /// device is unilateral and frequency-dependent, so a transposed read or a
    /// misframed point cannot pass unnoticed.
    mod derived_from_a_real_export {
        use super::*;

        const V2_21_12: &str =
            include_str!("../../../tests/data/ads_v2_varying_noise_2port_ri_21_12.ts");
        const V2_12_21: &str =
            include_str!("../../../tests/data/ads_v2_varying_noise_2port_ri_12_21.ts");
        const AS_V1: &str = include_str!("../../../tests/data/ads_varying_noise_2port_ri.s2p");

        #[test]
        fn the_two_orders_agree_on_real_data() {
            assert_same_matrix(&ok(V2_21_12), &ok(V2_12_21), 0.0, "21_12 vs 12_21");
        }

        /// Touchstone 1.0's silent convention is `21_12`, so the v1 export and
        /// the `21_12` rewrite of it must be the same network — bit for bit,
        /// since the tokens are identical and only their arrangement differs.
        #[test]
        fn the_v1_export_matches_its_twenty_one_twelve_rewrite() {
            assert_same_matrix(&ok(AS_V1), &ok(V2_21_12), 0.0, "v1 vs 21_12");
        }

        /// The `rn` / `rn_ohms` split on real measured values rather than the
        /// specification's two-row illustration. `rn` differs by the reference
        /// resistance and `rn_ohms` does not differ at all.
        #[test]
        fn rn_ohms_survives_the_version_change_exactly() {
            let v1 = ok(AS_V1).noise.expect("the export has a noise section");
            let v2 = ok(V2_21_12).noise.expect("the rewrite keeps it");

            assert_eq!(v1.freq_hz, v2.freq_hz);
            assert_eq!(v1.nfmin_db, v2.nfmin_db);
            assert_eq!(v1.gamma_opt, v2.gamma_opt);
            assert_ne!(v1.rn, v2.rn, "1.x normalizes, 2.x writes ohms");
            assert_eq!(
                v1.rn_ohms, v2.rn_ohms,
                "the derived value is the same quantity either way"
            );
            // Every point varies, so this is not agreement between two
            // constants.
            assert!(v1.rn_ohms.windows(2).any(|w| w[0] != w[1]));
        }

        #[test]
        fn the_noise_section_is_recorded_in_full() {
            let net = ok(V2_21_12);
            let noise = net.noise.as_ref().expect("has noise");
            assert_eq!(noise.freq_hz.len(), 10);
            assert_eq!(net.nfreqs(), 10);
            assert_eq!(net.metadata.two_port_order, Some(TwoPortOrder::S21First));
            // Angles well off every axis, so a real/imaginary swap in Γopt
            // could not survive: the source's run from 77.9° to 151.3°.
            assert!(noise.gamma_opt.iter().all(|g| g.im.abs() > 1e-3));
        }
    }

    /// Real v1 exports whose comments sit *between* frequency records rather
    /// than only in a header block — a layout no other fixture has. These are
    /// v1 files, and they belong to this suite because the same exporter wrote
    /// the v2 ones above.
    #[test]
    fn interleaved_comments_between_records_are_not_data() {
        let net = parse_file(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/data/hfss_microstrip_2port_ri_unnormalized.s2p"),
        )
        .expect("should parse");
        assert_eq!(net.nports, 2);
        assert_eq!(net.nfreqs(), 201);
        assert_eq!(net.metadata.version, Version::V1_0);
        // Its option line is `# GHz S RI` with no `R` at all, so the reference
        // is the documented 50 Ω default.
        assert_eq!(constant_z0(&net), [50.0, 50.0]);
    }
}

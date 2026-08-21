//! End-to-end tests for Touchstone v1, 2-port, `RI` files.
//!
//! Fixtures are inline `&str` consts rather than files on purpose: CRLF and
//! trailing-whitespace cases are invisible in a file, and the repository's
//! `trailing-whitespace` / `end-of-file-fixer` hooks would silently rewrite
//! such a fixture and break the test. `tests/data/` holds full-length files
//! whose provenance is documented; the shape of the grammar is pinned here.

use std::path::Path;

use touchstone_core::{
    Complex64, Error, Format, FreqUnit, Network, Parameter, ParseErrorKind, ParseOptions, Version,
    parse_file, parse_str, parse_str_with,
};

/// The smallest file this version accepts.
const MINIMAL: &str = "# GHZ S RI R 50\n1.0 0.1 0.2 0.3 0.4 0.5 0.6 0.7 0.8\n";

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

// ---------------------------------------------------------------- happy path

#[test]
fn reads_a_minimal_two_port_file() {
    let net = ok(MINIMAL);

    assert_eq!(net.nports, 2);
    assert_eq!(net.nfreqs(), 1);
    assert_eq!(net.freq_hz, [1e9]);
    assert_eq!(net.z0, [50.0, 50.0]);
    assert!(net.noise.is_none());
    assert_eq!(net.s.len(), 4);

    assert_eq!(net.metadata.version, Version::V1);
    assert_eq!(net.metadata.freq_unit, FreqUnit::GHz);
    assert_eq!(net.metadata.parameter, Parameter::S);
    assert_eq!(net.metadata.format, Format::Ri);
    assert_eq!(net.metadata.resistance, 50.0);
}

/// **The ordering guard.** Spec v1.1 §3 writes a 2-port line as
/// S11 S21 S12 S22 — 21 *before* 12. A swap here silently transposes every
/// matrix, and no real passive file can detect it: a reciprocal network has
/// S21 == S12, which is why the full-length Murata-style fixtures cannot
/// stand in for this test. The values below are deliberately far apart, the
/// way a unilateral amplifier's are.
#[test]
fn two_port_data_is_ordered_s11_s21_s12_s22() {
    let net = ok("# GHZ S RI R 50\n1.0  0.1 0.2  9.0 9.1  0.01 0.02  0.3 0.4\n");

    assert_eq!(net.at(0, 0, 0), Complex64::new(0.1, 0.2), "S11");
    assert_eq!(
        net.at(0, 1, 0),
        Complex64::new(9.0, 9.1),
        "S21 is the 2nd pair"
    );
    assert_eq!(
        net.at(0, 0, 1),
        Complex64::new(0.01, 0.02),
        "S12 is the 3rd pair"
    );
    assert_eq!(net.at(0, 1, 1), Complex64::new(0.3, 0.4), "S22");
}

#[test]
fn frequencies_are_scaled_to_hz_from_every_unit() {
    for (unit, expected) in [("HZ", 2.0), ("KHZ", 2e3), ("MHZ", 2e6), ("GHZ", 2e9)] {
        let input = format!("# {unit} S RI R 50\n2.0 0 0 0 0 0 0 0 0\n");
        assert_eq!(ok(&input).freq_hz, [expected], "unit {unit}");
    }
}

#[test]
fn a_bare_hash_defaults_to_ghz_and_fifty_ohms() {
    // The bare-`#` defaults include MA, so RI has to be stated; everything
    // else comes from the defaults in spec v1.1 §3.
    let net = ok("# RI\n2.5 0 0 0 0 0 0 0 0\n");
    assert_eq!(net.freq_hz, [2.5e9]);
    assert_eq!(net.z0, [50.0, 50.0]);
    assert_eq!(net.metadata.freq_unit, FreqUnit::GHz);
    assert_eq!(net.metadata.parameter, Parameter::S);
}

#[test]
fn the_option_line_may_be_lower_case_and_reordered() {
    let net = ok("# ri r 75 mhz s\n1.0 0 0 0 0 0 0 0 0\n");
    assert_eq!(net.freq_hz, [1e6]);
    assert_eq!(net.z0, [75.0, 75.0]);
}

#[test]
fn comments_appear_in_every_position_without_disturbing_the_data() {
    let net = ok(concat!(
        "! header one\n",
        "!\n",
        "\n",
        "!header two\n",
        "# GHZ S RI R 50 ! about the option line\n",
        "! a note between the option line and the data\n",
        "1.0 0.1 0.2 0.3 0.4 0.5 0.6 0.7 0.8 ! row one\n",
        "\n",
        "! interleaved\n",
        "2.0 1.1 1.2 1.3 1.4 1.5 1.6 1.7 1.8\n",
        "! trailing note at end of file\n",
    ));

    assert_eq!(net.freq_hz, [1e9, 2e9]);
    assert_eq!(net.at(1, 0, 0), Complex64::new(1.1, 1.2));

    // Only the header block is retained, and only comment-only lines: a
    // trailing comment on the option line is not a comment line, and
    // comments below the first data row are dropped (see ADR 0004).
    assert_eq!(
        net.metadata.comments,
        [
            "header one",
            "",
            "header two",
            "a note between the option line and the data",
        ]
    );
}

#[test]
fn the_option_line_is_kept_verbatim_minus_any_trailing_comment() {
    let net = ok("#  hZ   S   RI   R     50.00 ! exported by something\n1 0 0 0 0 0 0 0 0\n");
    assert_eq!(
        net.metadata.option_line.as_deref(),
        Some("#  hZ   S   RI   R     50.00")
    );
    assert_eq!(net.z0, [50.0, 50.0]);
}

#[test]
fn crlf_line_endings_parse_identically_to_lf() {
    let crlf = MINIMAL.replace('\n', "\r\n");
    assert_eq!(ok(&crlf).freq_hz, ok(MINIMAL).freq_hz);
    assert_eq!(ok(&crlf).s, ok(MINIMAL).s);
}

#[test]
fn number_spellings_real_files_use_all_parse() {
    // Leading-dot, exponent, explicit sign, and integer forms all appear in
    // manufacturer exports.
    let net = ok("# HZ S RI R 50\n1.0E7 .680 -0.012 +1e-4 0 -3.5 0 2 0\n");
    assert_eq!(net.freq_hz, [1e7]);
    assert_eq!(net.at(0, 0, 0), Complex64::new(0.680, -0.012));
    assert_eq!(net.at(0, 1, 0), Complex64::new(1e-4, 0.0));
}

#[test]
fn leading_whitespace_and_tabs_on_data_lines_are_fine() {
    let net = ok("# GHZ S RI R 50\n\t  1.0\t0.1 0.2\t0.3 0.4 0.5 0.6 0.7 0.8  \n");
    assert_eq!(net.freq_hz, [1e9]);
    assert_eq!(net.at(0, 0, 0), Complex64::new(0.1, 0.2));
}

#[test]
fn a_second_option_line_is_ignored_and_the_first_still_governs() {
    // Spec v1.1 §3 says option lines after the first are ignored. If the
    // second were honored, these frequencies would come out in MHz.
    let net = ok(concat!(
        "# GHZ S RI R 50\n",
        "1.0 0 0 0 0 0 0 0 0\n",
        "# MHZ S RI R 75\n",
        "2.0 0 0 0 0 0 0 0 0\n",
    ));
    assert_eq!(net.freq_hz, [1e9, 2e9]);
    assert_eq!(net.z0, [50.0, 50.0]);
}

#[test]
fn many_points_stay_in_order_and_keep_their_values() {
    let mut input = String::from("# HZ S RI R 50\n");
    for i in 1..=50u32 {
        let f = f64::from(i) * 1e6;
        input.push_str(&format!("{f} {i}.5 0 0 0 0 0 0 0\n"));
    }
    let net = ok(&input);
    assert_eq!(net.nfreqs(), 50);
    assert_eq!(net.s.len(), 50 * 4);
    assert_eq!(net.freq_hz[0], 1e6);
    assert_eq!(net.freq_hz[49], 50e6);
    assert_eq!(net.at(49, 0, 0), Complex64::new(50.5, 0.0));
}

#[test]
fn an_explicit_port_count_agreeing_with_the_data_is_accepted() {
    let opts = ParseOptions::new().nports(2);
    let net = parse_str_with(MINIMAL, &opts).expect("should parse");
    assert_eq!(net.nports, 2);
}

// ---------------------------------------------------------------- error path

#[test]
fn an_empty_or_dataless_file_is_an_error() {
    assert_eq!(kind(""), ParseErrorKind::MissingOptionLine);
    assert_eq!(kind("   \n\n"), ParseErrorKind::MissingOptionLine);
    assert_eq!(
        kind("! just a comment\n"),
        ParseErrorKind::MissingOptionLine
    );
    assert_eq!(
        kind("# GHZ S RI R 50\n"),
        ParseErrorKind::UnexpectedData("no data lines".into())
    );
    assert_eq!(
        kind("# GHZ S RI R 50\n! nothing but talk\n"),
        ParseErrorKind::UnexpectedData("no data lines".into())
    );
}

#[test]
fn data_before_the_option_line_is_an_error_naming_its_line() {
    let (line, kind) = fails("1.0 0 0 0 0 0 0 0 0\n# GHZ S RI R 50\n");
    assert_eq!(line, 1);
    assert_eq!(kind, ParseErrorKind::DataBeforeOptionLine);
}

#[test]
fn frequencies_must_strictly_increase() {
    // Descending.
    let (line, kind) = fails("# GHZ S RI R 50\n2.0 0 0 0 0 0 0 0 0\n1.0 0 0 0 0 0 0 0 0\n");
    assert_eq!(line, 3);
    assert_eq!(
        kind,
        ParseErrorKind::FrequencyNotAscending {
            previous_hz: 2e9,
            current_hz: 1e9,
        }
    );

    // Repeated. Spec v1.1 §3 asks for *increasing* order, so a duplicate
    // point is non-conformant; a lenient mode may downgrade this later.
    let (line, kind) = fails("# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0 0\n1.0 0 0 0 0 0 0 0 0\n");
    assert_eq!(line, 3);
    assert!(matches!(kind, ParseErrorKind::FrequencyNotAscending { .. }));
}

#[test]
fn a_wrong_value_count_reports_what_was_expected() {
    // Truncated.
    assert_eq!(
        kind("# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::WrongValueCount {
            expected: 9,
            found: 8,
        }
    );
    // Trailing extra value.
    assert_eq!(
        kind("# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::WrongValueCount {
            expected: 9,
            found: 10,
        }
    );
    // A wrapped 2-port line: unsupported in this version, and reported as
    // the value-count mismatch it is.
    assert_eq!(
        kind("# GHZ S RI R 50\n1.0 0 0 0 0\n0 0 0 0\n"),
        ParseErrorKind::WrongValueCount {
            expected: 9,
            found: 5,
        }
    );
}

#[test]
fn a_garbage_token_is_quoted_back() {
    let (line, kind) = fails("# GHZ S RI R 50\n1.0 0 0 abc 0 0 0 0 0\n");
    assert_eq!(line, 2);
    assert_eq!(kind, ParseErrorKind::InvalidNumber("abc".into()));
}

#[test]
fn out_of_scope_formats_and_parameters_name_the_limit() {
    assert_eq!(
        kind("# GHZ S MA R 50\n1.0 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::UnsupportedFormat(Format::Ma)
    );
    assert_eq!(
        kind("# GHZ S DB R 50\n1.0 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::UnsupportedFormat(Format::Db)
    );
    assert_eq!(
        kind("# GHZ Y RI R 50\n1.0 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::UnsupportedParameter(Parameter::Y)
    );

    // A bare `#` defaults to MA, so it must not be mistaken for RI.
    assert_eq!(
        kind("#\n1.0 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::UnsupportedFormat(Format::Ma)
    );
}

#[test]
fn a_one_port_file_reports_an_unsupported_port_count() {
    assert_eq!(
        kind("# GHZ S RI R 50\n1.0 0.5 0.5\n"),
        ParseErrorKind::UnsupportedPortCount(1)
    );
}

/// The noise section must announce itself. Every real 2-port amplifier file
/// has one, and a generic ordering error here would send the user hunting
/// for corruption in a perfectly valid file.
#[test]
fn a_noise_section_is_reported_by_name_not_as_an_ordering_error() {
    let (line, kind) = fails(concat!(
        "# GHZ S RI R 50\n",
        "2.0 0 0 0 0 0 0 0 0\n",
        "22.0 0 0 0 0 0 0 0 0\n",
        "! NOISE PARAMETERS\n",
        "4.0 0.7 0.64 69.0 0.38\n",
    ));
    assert_eq!(line, 5);
    assert_eq!(kind, ParseErrorKind::NoiseSectionUnsupported);

    assert_eq!(
        Error::Parse { line, kind }.to_string(),
        "line 5: noise parameter section is not supported in this version"
    );
}

#[test]
fn the_unsupported_format_message_reads_as_a_scope_limit() {
    // This is the first error most new users will see, so its wording is
    // pinned, not just its variant.
    let err = parse_str("# GHZ S MA R 50\n1.0 0 0 0 0 0 0 0 0\n").unwrap_err();
    assert_eq!(
        err.to_string(),
        "line 2: unsupported format ma: only ri is supported in this version"
    );
}

#[test]
fn carriage_return_only_files_are_rejected_clearly() {
    let cr_only = MINIMAL.replace('\n', "\r");
    assert_eq!(
        kind(&cr_only),
        ParseErrorKind::UnsupportedLineEndings,
        "a CR-only file would otherwise arrive as one enormous line"
    );
}

#[test]
fn a_bad_option_line_is_reported_at_its_own_line() {
    let (line, kind) =
        fails("! a header\n! and another\n# GHZ S RI R 50 NONSENSE\n1 0 0 0 0 0 0 0 0\n");
    assert_eq!(line, 3);
    assert_eq!(
        kind,
        ParseErrorKind::InvalidOptionLine("unknown token 'nonsense'".into())
    );
}

// ----------------------------------------------------------------- real file

/// A unilateral 2-port simulated in Keysight ADS: S12 is zero, S21 is large,
/// S11 differs from S22 — the ordering guard's real-world counterpart. See
/// `tests/data/README.md` for provenance. Read via `include_str!` rather
/// than at runtime, so `cargo package`/`cargo test --workspace` never
/// depends on the file surviving a move.
const REAL_FILE: &str = include_str!("../../../tests/data/ads_unilateral_2port_ri.s2p");

#[test]
fn a_real_ads_export_parses_and_matches_its_known_values() {
    let net = ok(REAL_FILE);

    assert_eq!(net.nports, 2);
    assert_eq!(net.nfreqs(), 10);
    assert_eq!(net.freq_hz[0], 1e9);
    assert_eq!(net.freq_hz[9], 10e9);
    assert_eq!(net.z0, [50.0, 50.0]);

    // The network is purely resistive, so every frequency carries the same
    // values; spot-check the first and last points.
    for fi in [0, 9] {
        assert_eq!(net.at(fi, 0, 0), Complex64::new(0.333333333, 0.0), "S11");
        assert_eq!(net.at(fi, 1, 0), Complex64::new(-4.44444444, 0.0), "S21");
        assert_eq!(net.at(fi, 0, 1), Complex64::new(0.0, 0.0), "S12");
        assert_eq!(net.at(fi, 1, 1), Complex64::new(-0.333333333, 0.0), "S22");
    }
}

/// The same file, but through `parse_file`, so extension sniffing and the
/// lossy UTF-8 decode are exercised on real bytes rather than only on the
/// hand-written fixtures above.
#[test]
fn a_real_ads_export_parses_from_disk() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/data/ads_unilateral_2port_ri.s2p");
    let net = parse_file(&path).expect("should parse");
    assert_eq!(net.nports, 2);
    assert_eq!(net.nfreqs(), 10);
}

/// The full ADS export this fixture was derived from still carries its
/// noise section, and this version must reject it by name rather than with
/// a generic ordering error.
#[test]
fn the_full_ads_export_with_its_noise_section_is_rejected_by_name() {
    const FULL: &str = include_str!("../../../tests/data/ads_unilateral_2port_ri_with_noise.s2p");
    assert_eq!(kind(FULL), ParseErrorKind::NoiseSectionUnsupported);
}

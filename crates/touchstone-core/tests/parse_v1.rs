//! End-to-end tests for Touchstone v1 files.
//!
//! Two kinds of fixture, deliberately:
//!
//! - **Inline `&str` consts** pin the shape of the grammar. CRLF and
//!   trailing-whitespace cases are invisible in a file, and generated inputs
//!   reach port counts nobody is going to export by hand.
//! - **Real files from `tests/data/`**, pulled in with `include_str!`, prove
//!   the parser agrees with what a tool actually writes rather than only with
//!   itself. Their provenance is documented in that directory's README.
//!
//! The two are not redundant. A generated 4-port fixture proves the wrapping
//! logic is self-consistent; only the ADS export proves that self-consistent
//! reading is also the *right* one.

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

/// `MA` and `DB` go through `cos`/`sin` and `powf`, so a value that is exact
/// on disk in one format is only near-exact in another. Comparisons that
/// cross a format boundary use this; `RI` comparisons stay exact.
///
/// The bound suits hand-written fixtures, whose values are exact on disk.
/// Real exports need [`assert_agrees`], which allows for their own rounding.
fn assert_close(actual: Complex64, expected: Complex64, what: &str) {
    let error = (actual - expected).l1_norm();
    assert!(
        error < 1e-9,
        "{what}: expected {expected}, got {actual} (off by {error})"
    );
}

/// Two readings of the same real device must agree everywhere.
///
/// The bound is set by the *files*, not by the conversion: the ADS exports
/// carry nine significant figures, so `RI` and the reconstruction from a
/// rounded magnitude and angle cannot agree more closely than about 1e-8 no
/// matter how exact the arithmetic is.
fn assert_agrees(actual: &Network, expected: &Network, what: &str) {
    assert_eq!(actual.nports, expected.nports, "{what}: port count");
    assert_eq!(actual.freq_hz, expected.freq_hz, "{what}: frequencies");
    assert_eq!(actual.z0, expected.z0, "{what}: reference impedance");
    for fi in 0..expected.nfreqs() {
        for row in 0..expected.nports {
            for col in 0..expected.nports {
                let (a, e) = (actual.at(fi, row, col), expected.at(fi, row, col));
                assert!(
                    (a - e).l1_norm() < 1e-7,
                    "{what}: S({},{}) at point {fi}: expected {e}, got {a}",
                    row + 1,
                    col + 1
                );
            }
        }
    }
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

// ------------------------------------------------ port counts and wrapping

/// Index-encoded matrix entries: the real part of S(row, col) is
/// `row*10 + col`, one-based, so a transposed or mis-framed read is visible
/// at a glance instead of hiding behind plausible numbers.
fn entry(row: usize, col: usize) -> f64 {
    ((row + 1) * 10 + (col + 1)) as f64
}

/// Render an `n`-port file the way spec v1.1 §3 p9 prescribes: each matrix
/// row starts a new line, no more than four pairs per line, and only the
/// very first line of a data set carries the frequency.
fn conformant_multiport(nports: usize, freqs: &[f64]) -> String {
    let mut out = String::from("# GHZ S RI R 50\n");
    for (fi, freq) in freqs.iter().enumerate() {
        for row in 0..nports {
            for (chunk, cols) in (0..nports).collect::<Vec<_>>().chunks(4).enumerate() {
                let mut line = String::new();
                if row == 0 && chunk == 0 {
                    line.push_str(&freq.to_string());
                }
                for &col in cols {
                    // Offset each frequency so a data set boundary that slips
                    // by one point cannot go unnoticed.
                    line.push_str(&format!(" {} 0", entry(row, col) + fi as f64 * 1000.0));
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

fn assert_matrix_is_row_major(net: &Network, freqs: usize) {
    for fi in 0..freqs {
        for row in 0..net.nports {
            for col in 0..net.nports {
                assert_eq!(
                    net.at(fi, row, col),
                    Complex64::new(entry(row, col) + fi as f64 * 1000.0, 0.0),
                    "S({},{}) at frequency {fi}",
                    row + 1,
                    col + 1
                );
            }
        }
    }
}

#[test]
fn a_one_port_file_reads_its_single_entry() {
    let net = ok("# GHZ S RI R 50\n1.0 0.5 -0.25\n2.0 0.4 -0.3\n");
    assert_eq!(net.nports, 1);
    assert_eq!(net.nfreqs(), 2);
    assert_eq!(net.z0, [50.0]);
    assert_eq!(net.at(0, 0, 0), Complex64::new(0.5, -0.25));
    assert_eq!(net.at(1, 0, 0), Complex64::new(0.4, -0.3));
}

/// **The N ≥ 3 ordering guard.** Spec v1.1 §3 p7–8 lays a 3-port out as
/// `<freq> <N11> <N12> <N13>` / `<N21> <N22> <N23>` / `<N31> <N32> <N33>` —
/// plain row-major, with none of the 2-port's 21-before-12 swap. Applying
/// the 2-port rule here, or forgetting the 2-port rule there, transposes
/// every matrix silently.
#[test]
fn a_three_port_set_is_row_major_across_its_wrapped_lines() {
    let net = ok(&conformant_multiport(3, &[1.0, 2.0]));
    assert_eq!(net.nports, 3);
    assert_eq!(net.nfreqs(), 2);
    assert_eq!(net.s.len(), 2 * 9);
    assert_matrix_is_row_major(&net, 2);
}

/// A 4-port's first line holds nine tokens — byte-identical in shape to a
/// *complete* 2-port data set. Nothing about that line alone distinguishes
/// them; only running the set on to its full 33 values does.
#[test]
fn a_four_port_first_line_is_not_mistaken_for_a_whole_two_port_set() {
    let net = ok(&conformant_multiport(4, &[1.0, 2.0, 3.0]));
    assert_eq!(net.nports, 4, "nine tokens on line one, but a 4-port");
    assert_eq!(net.nfreqs(), 3);
    assert_matrix_is_row_major(&net, 3);
}

/// Above four ports a single matrix *row* no longer fits on a line either,
/// so a data set contains lines that are neither its first nor a row start.
/// This is the layout 3- and 4-port files never produce.
#[test]
fn an_eight_port_set_wraps_each_matrix_row_across_two_lines() {
    let net = ok(&conformant_multiport(8, &[1.0, 2.0]));
    assert_eq!(net.nports, 8);
    assert_eq!(net.nfreqs(), 2);
    assert_eq!(net.s.len(), 2 * 64);
    assert_matrix_is_row_major(&net, 2);
}

/// Real exports separate frequency blocks with blank lines and hang a
/// comment off the first line of each — Keysight's own 3-port example does
/// both. Neither may disturb a data set in progress.
#[test]
fn blank_lines_and_row_comments_do_not_break_a_wrapped_set() {
    let net = ok(concat!(
        "# GHZ S RI R 50\n",
        "1.0  11 0  12 0  13 0   ! frequency line 1\n",
        "     21 0  22 0  23 0\n",
        "     31 0  32 0  33 0\n",
        "\n",
        "2.0  1011 0  1012 0  1013 0   ! frequency line 2\n",
        "\n",
        "     1021 0  1022 0  1023 0\n",
        "     1031 0  1032 0  1033 0\n",
    ));
    assert_eq!(net.nports, 3);
    assert_matrix_is_row_major(&net, 2);
}

/// The tolerance this milestone buys. Spec v1.1 §3 p8 requires *exactly*
/// three pairs per line for a 3-port, but files in the wild wrap however
/// their generator felt like — so a data set is accumulated by token count
/// and any wrapping whose totals come out right is accepted.
#[test]
fn a_set_wrapped_against_the_spec_still_reads_correctly() {
    // Everything on one line, then the same data broken at arbitrary points.
    let one_line = ok(concat!(
        "# GHZ S RI R 50\n",
        "1.0 11 0 12 0 13 0 21 0 22 0 23 0 31 0 32 0 33 0\n",
    ));
    let ragged = ok(concat!(
        "# GHZ S RI R 50\n",
        "1.0 11 0 12 0\n",
        "13 0 21 0 22 0 23 0 31 0 32 0\n",
        "33 0\n",
    ));
    assert_eq!(one_line.nports, 3);
    assert_eq!(one_line.s, ragged.s);
    assert_matrix_is_row_major(&one_line, 1);
    assert_matrix_is_row_major(&ragged, 1);
}

/// A 2-port set split 5 + 4. M1 could not have read this, and its old
/// value-count test asserted the failure; the same input is now valid, and
/// must not be mistaken for the noise section (which also opens with five
/// values).
#[test]
fn a_two_port_set_split_after_two_pairs_is_not_mistaken_for_noise() {
    let net = ok(concat!(
        "# GHZ S RI R 50\n",
        "1.0  0.1 0.2  9.0 9.1\n",
        "     0.01 0.02  0.3 0.4\n",
    ));
    assert_eq!(net.nports, 2);
    assert_eq!(net.nfreqs(), 1);
    assert_eq!(net.at(0, 1, 0), Complex64::new(9.0, 9.1), "S21");
    assert_eq!(net.at(0, 0, 1), Complex64::new(0.01, 0.02), "S12");
}

/// Nothing in the format caps the port count — spec v1.1 §3 p4 says
/// "the Touchstone format supports matrixes of unlimited size", against
/// Keysight's documented 5–99 and the `touchstone` crate's 32.
#[test]
fn a_port_count_beyond_every_other_readers_ceiling_is_accepted() {
    let net = ok(&conformant_multiport(33, &[1.0]));
    assert_eq!(net.nports, 33);
    assert_eq!(net.s.len(), 33 * 33);
    assert_eq!(net.z0.len(), 33);
    assert_matrix_is_row_major(&net, 1);
}

/// The port count from the filename wins over inference, and is what makes a
/// truncated file report a shortfall rather than an unsolvable shape.
#[test]
fn the_extension_supplies_a_port_count_the_data_alone_could_not_fix() {
    let dir = std::env::temp_dir().join("touchstone_rs_m2_extension");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("truncated.s4p");

    // A 4-port set stopping one pair short: 31 values, not 33.
    let full = conformant_multiport(4, &[1.0]);
    let lines: Vec<&str> = full.trim_end().lines().collect();
    let mut input = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i + 1 == lines.len() {
            let mut tokens: Vec<&str> = line.split_whitespace().collect();
            tokens.truncate(tokens.len() - 2);
            input.push_str(&tokens.join(" "));
        } else {
            input.push_str(line);
        }
        input.push('\n');
    }
    std::fs::write(&path, &input).expect("write");

    assert!(matches!(
        parse_str(&input),
        Err(Error::Parse {
            kind: ParseErrorKind::IndeterminatePortCount { .. },
            ..
        })
    ));
    assert!(matches!(
        parse_file(&path),
        Err(Error::Parse {
            kind: ParseErrorKind::WrongValueCount {
                expected: 33,
                found: 31
            },
            ..
        })
    ));
    std::fs::remove_dir_all(&dir).ok();
}

// --------------------------------------------------------------- formats

#[test]
fn magnitude_angle_files_convert_to_the_same_numbers_as_real_imaginary() {
    // 2 @ 90 deg is 2i; 1 @ 180 deg is -1; 0.5 @ 0 deg is 0.5.
    let net = ok("# GHZ S MA R 50\n1.0  2 90  1 180  0.5 0  1 -90\n");
    assert_eq!(net.metadata.format, Format::Ma);
    assert_close(net.at(0, 0, 0), Complex64::new(0.0, 2.0), "S11");
    assert_close(net.at(0, 1, 0), Complex64::new(-1.0, 0.0), "S21");
    assert_close(net.at(0, 0, 1), Complex64::new(0.5, 0.0), "S12");
    assert_close(net.at(0, 1, 1), Complex64::new(0.0, -1.0), "S22");
}

#[test]
fn db_files_use_twenty_log_ten_not_ten() {
    // 20 dB is a magnitude of 10, 0 dB is 1, -20 dB is 0.1.
    let net = ok("# GHZ S DB R 50\n1.0  20 0  0 0  -20 0  -20 180\n");
    assert_eq!(net.metadata.format, Format::Db);
    assert_close(net.at(0, 0, 0), Complex64::new(10.0, 0.0), "S11");
    assert_close(net.at(0, 1, 0), Complex64::new(1.0, 0.0), "S21");
    assert_close(net.at(0, 0, 1), Complex64::new(0.1, 0.0), "S12");
    assert_close(net.at(0, 1, 1), Complex64::new(-0.1, 0.0), "S22");
}

/// A bare `#` means `GHz S MA R 50` (spec v1.1 §3), and MA is now readable —
/// so the minimum legal option line finally parses a file on its own.
#[test]
fn the_minimum_legal_option_line_parses_a_whole_file() {
    let net = ok("#\n1.0  1 0  1 0  1 0  1 0\n");
    assert_eq!(net.metadata.format, Format::Ma);
    assert_eq!(net.metadata.freq_unit, FreqUnit::GHz);
    assert_eq!(net.freq_hz, [1e9]);
    assert_close(net.at(0, 0, 0), Complex64::new(1.0, 0.0), "S11");
}

/// The 2-port swap is a property of the file layout, not of the value
/// format, so it has to survive the polar conversions too.
#[test]
fn the_two_port_swap_applies_in_every_format() {
    let ri = ok("# GHZ S RI R 50\n1.0  1 0  2 0  3 0  4 0\n");
    let ma = ok("# GHZ S MA R 50\n1.0  1 0  2 0  3 0  4 0\n");
    let db = ok(
        "# GHZ S DB R 50\n1.0  0 0  6.020599913279624 0  9.542425094393249 0  12.041199826559248 0\n",
    );
    for net in [&ri, &ma, &db] {
        assert_close(
            net.at(0, 1, 0),
            Complex64::new(2.0, 0.0),
            "S21 is the 2nd pair",
        );
        assert_close(
            net.at(0, 0, 1),
            Complex64::new(3.0, 0.0),
            "S12 is the 3rd pair",
        );
    }
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

/// A header-only file's error must point at the option line itself, not an
/// arbitrary line 1 — which would be actively wrong once the option line
/// isn't the file's first line.
#[test]
fn a_dataless_file_points_at_the_option_line_not_line_one() {
    let (line, kind) = fails("! preface\n! more preface\n# GHZ S RI R 50\n! trailing talk\n");
    assert_eq!(line, 3, "the option line is on line 3, not line 1");
    assert_eq!(kind, ParseErrorKind::UnexpectedData("no data lines".into()));
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

/// With the port count known, a malformed data set is reported against the
/// size that count implies.
#[test]
fn a_wrong_value_count_reports_what_was_expected() {
    let two_port = ParseOptions::new().nports(2);
    let kind_of = |input: &str| match parse_str_with(input, &two_port) {
        Err(Error::Parse { kind, .. }) => kind,
        other => panic!("expected a parse error, got {other:?}"),
    };

    // Truncated.
    assert_eq!(
        kind_of("# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::WrongValueCount {
            expected: 9,
            found: 8,
        }
    );
    // Trailing extra value.
    assert_eq!(
        kind_of("# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::WrongValueCount {
            expected: 9,
            found: 10,
        }
    );
    // A data set left short at end of file, spread over two lines.
    assert_eq!(
        kind_of("# GHZ S RI R 50\n1.0 0 0 0 0\n0 0\n"),
        ParseErrorKind::WrongValueCount {
            expected: 9,
            found: 7,
        }
    );
}

/// Without a port count from the caller or the filename, a malformed first
/// data set cannot be measured against anything — there is no `n` for which
/// its size is legal. Saying so, and naming the two ways to supply the count,
/// beats inventing an expectation the file never claimed.
#[test]
fn an_unmeasurable_first_data_set_reports_the_shape_it_could_not_solve() {
    assert_eq!(
        kind("# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::IndeterminatePortCount { found: 8 }
    );
    assert_eq!(
        kind("# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::IndeterminatePortCount { found: 10 }
    );
    // Odd, but two entries is not a square matrix.
    assert_eq!(
        kind("# GHZ S RI R 50\n1.0 0 0 0 0\n"),
        ParseErrorKind::IndeterminatePortCount { found: 5 }
    );
}

#[test]
fn a_garbage_token_is_quoted_back() {
    let (line, kind) = fails("# GHZ S RI R 50\n1.0 0 0 abc 0 0 0 0 0\n");
    assert_eq!(line, 2);
    assert_eq!(kind, ParseErrorKind::InvalidNumber("abc".into()));
}

#[test]
fn out_of_scope_parameters_name_the_limit() {
    for (param, expected) in [
        ("Y", Parameter::Y),
        ("Z", Parameter::Z),
        ("G", Parameter::G),
        ("H", Parameter::H),
    ] {
        let input = format!("# GHZ {param} RI R 50\n1.0 0 0 0 0 0 0 0 0\n");
        assert_eq!(
            kind(&input),
            ParseErrorKind::UnsupportedParameter(expected),
            "parameter {param}"
        );
    }
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

/// Noise detection compares against the last S-parameter frequency because
/// spec v1.1 guarantees genuine noise data satisfies that bound (the lowest
/// noise frequency is always at or below the highest network-parameter
/// frequency). A file whose 5-value tail keeps *ascending* past the S-sweep
/// is already non-conformant a second way, and this version does not try to
/// guess that it's noise — it reports the generic, still-accurate
/// value-count mismatch instead. Deliberate, not an oversight: dropping the
/// frequency comparison entirely would misclassify a legitimate M2-style
/// wrapped 2-port continuation (which also has 5 values on its first line)
/// as an unsupported noise section.
#[test]
fn a_five_value_line_that_keeps_ascending_is_not_mistaken_for_noise() {
    assert_eq!(
        kind("# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0 0\n2.0 0 0 0 0\n"),
        ParseErrorKind::WrongValueCount {
            expected: 9,
            found: 5,
        }
    );
}

/// Rust's `f64` parser accepts "nan" and "inf" as tokens. Whether that is a
/// problem depends on what the value *converts to*, not on how it is spelled
/// — which is the whole reason the check moved downstream of the conversion.
#[test]
fn values_that_stay_non_finite_after_conversion_are_rejected() {
    assert!(matches!(
        kind("# GHZ S RI R 50\n1.0 nan 0 0 0 0 0 0 0\n"),
        ParseErrorKind::NonFiniteValue { .. }
    ));
    assert!(matches!(
        kind("# GHZ S RI R 50\n1.0 0 inf 0 0 0 0 0 0\n"),
        ParseErrorKind::NonFiniteValue { .. }
    ));
    // An infinite *magnitude* is still infinite in every format.
    assert!(matches!(
        kind("# GHZ S MA R 50\n1.0 inf 0 0 0 0 0 0 0\n"),
        ParseErrorKind::NonFiniteValue { .. }
    ));
    assert!(matches!(
        kind("# GHZ S DB R 50\n1.0 inf 0 0 0 0 0 0 0\n"),
        ParseErrorKind::NonFiniteValue { .. }
    ));
    // A frequency is not a converted pair, so it is still caught as the bad
    // token it is.
    assert_eq!(
        kind("# GHZ S RI R 50\ninf 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::InvalidNumber("inf".into())
    );
}

/// The counterpart: `-inf` in a `DB` magnitude column is not a bad token at
/// all. It is how a real ADS export writes an entry whose magnitude is
/// exactly zero, and `10^(-inf/20)` is `0`. Rejecting it would make the
/// committed DB fixture unreadable.
#[test]
fn minus_infinity_db_reads_as_an_exact_zero() {
    let net = ok("# GHZ S DB R 50\n1.0 0 0 0 0 -inf 0 0 0\n");
    assert_eq!(net.at(0, 0, 1), Complex64::new(0.0, 0.0), "S12");
    // 0 dB is unity, so the entries around it are unaffected.
    assert_eq!(net.at(0, 0, 0), Complex64::new(1.0, 0.0), "S11");
}

#[test]
fn the_unsupported_parameter_message_reads_as_a_scope_limit() {
    // The wording is pinned, not just the variant: this is the error a user
    // pointing us at a Y-parameter file sees. Reported at the option line
    // (line 1), since the parameter is a property of the option line, not of
    // any data row.
    let err = parse_str("# GHZ Y RI R 50\n1.0 0 0 0 0 0 0 0 0\n").unwrap_err();
    assert_eq!(
        err.to_string(),
        "line 1: unsupported parameter y: only s-parameters are supported in this version"
    );
}

/// An out-of-scope option line is reported immediately, even with no data
/// lines at all — a better diagnosis than "no data lines", which would be
/// true but useless: the file wouldn't parse even if it had data.
#[test]
fn an_out_of_scope_parameter_is_reported_even_without_any_data() {
    assert_eq!(
        kind("# GHZ Z RI R 50\n"),
        ParseErrorKind::UnsupportedParameter(Parameter::Z)
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

/// The same device, exported by ADS in all three formats. **This is the
/// strongest correctness check in the suite**: it needs no hand-computed
/// expectations, and a wrong dB base, a degrees/radians slip, or a sign
/// error in the angle all break it immediately. The `DB` file's S12 column
/// is literally `-inf`, so it also proves the zero-magnitude case survives a
/// round trip through the conversion.
#[test]
fn the_three_ads_exports_of_one_device_agree() {
    const MA: &str = include_str!("../../../tests/data/ads_unilateral_2port_ma.s2p");
    const DB: &str = include_str!("../../../tests/data/ads_unilateral_2port_db.s2p");

    let ri = ok(REAL_FILE);
    let ma = ok(MA);
    let db = ok(DB);

    assert_eq!(ma.metadata.format, Format::Ma);
    assert_eq!(db.metadata.format, Format::Db);

    for other in [&ma, &db] {
        assert_eq!(other.nports, ri.nports);
        assert_eq!(other.freq_hz, ri.freq_hz);
        assert_eq!(other.z0, ri.z0);
    }

    for fi in 0..ri.nfreqs() {
        for (row, col) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let expected = ri.at(fi, row, col);
            // The exports carry nine significant figures, so agreement is
            // limited by the file's own precision, not by the conversion.
            for (name, other) in [("ma", &ma), ("db", &db)] {
                let actual = other.at(fi, row, col);
                let error = (actual - expected).l1_norm();
                assert!(
                    error < 1e-7,
                    "{name} S({},{}) at point {fi}: expected {expected}, got {actual}",
                    row + 1,
                    col + 1
                );
            }
        }
    }

    // The device is unilateral: S12 is a true zero, written `-inf` dB.
    assert_eq!(db.at(0, 0, 1).l1_norm(), 0.0, "S12 from -inf dB");
}

// -------------------------------------- real 1-port ADS exports, varied

/// One 1-port device, exported repeatedly with a single thing changed each
/// time: the frequency unit, the number spelling, or the precision. Each
/// pair below isolates exactly one axis, so a failure names its own cause.
mod one_port {
    use super::*;

    const RI_GHZ: &str = include_str!("../../../tests/data/ads_1port_ri_ghz.s1p");
    const RI_GHZ_SCI: &str = include_str!("../../../tests/data/ads_1port_ri_ghz_scientific.s1p");
    const MA_GHZ: &str = include_str!("../../../tests/data/ads_1port_ma_ghz.s1p");
    const MA_MHZ: &str = include_str!("../../../tests/data/ads_1port_ma_mhz.s1p");
    const MA_HZ: &str = include_str!("../../../tests/data/ads_1port_ma_hz.s1p");
    const DB_GHZ: &str = include_str!("../../../tests/data/ads_1port_db_ghz.s1p");
    const DB_LOW_PRECISION: &str =
        include_str!("../../../tests/data/ads_1port_db_ghz_low_precision.s1p");

    #[test]
    fn a_real_one_port_export_reads_its_single_entry() {
        let net = ok(RI_GHZ);
        assert_eq!(net.nports, 1);
        assert_eq!(net.nfreqs(), 30);
        assert_eq!(net.s.len(), 30);
        assert_eq!(net.z0, [50.0]);
        assert_eq!(net.freq_hz[0], 50e6, "0.05 GHz");
        assert_eq!(net.freq_hz[29], 1.5e9);
        assert_eq!(net.at(0, 0, 0), Complex64::new(0.973077725, -0.144262395));
        assert_eq!(net.at(29, 0, 0), Complex64::new(0.792504104, 0.367509596));
    }

    /// The same sweep written in Hz, MHz and GHz. Normalization happens on
    /// read, so all three must land on **bit-identical** arrays — not merely
    /// close ones, since 0.05 GHz, 50 MHz and 50000000 Hz name one number.
    #[test]
    fn the_frequency_unit_changes_the_file_but_not_the_result() {
        let ghz = ok(MA_GHZ);
        for (unit, other) in [("mhz", ok(MA_MHZ)), ("hz", ok(MA_HZ))] {
            assert_eq!(other.freq_hz, ghz.freq_hz, "{unit}: frequencies");
            assert_eq!(other.s, ghz.s, "{unit}: values");
        }
        assert_eq!(ghz.metadata.freq_unit, FreqUnit::GHz);
        assert_eq!(ok(MA_MHZ).metadata.freq_unit, FreqUnit::MHz);
        assert_eq!(ok(MA_HZ).metadata.freq_unit, FreqUnit::Hz);
    }

    /// `5.000000000e-02` and `0.05` are the same number. The exponent form
    /// is what QUCS and several instruments emit, and it applies to the
    /// frequency column as much as to the values — the frequencies here must
    /// come out exactly equal, not merely close.
    #[test]
    fn scientific_notation_reads_as_the_same_number_as_decimal() {
        let plain = ok(RI_GHZ);
        let scientific = ok(RI_GHZ_SCI);
        assert_eq!(scientific.freq_hz, plain.freq_hz);
        assert_agrees(&scientific, &plain, "scientific notation");
    }

    /// An export rounded to four significant figures rather than nine. It
    /// must parse identically in structure and land near the full-precision
    /// reading — the tolerance is the file's, not the parser's, which is the
    /// point: rounding in the source is not an error to be rejected.
    #[test]
    fn a_low_precision_export_parses_and_stays_within_its_own_rounding() {
        let full = ok(DB_GHZ);
        let coarse = ok(DB_LOW_PRECISION);
        assert_eq!(coarse.nports, 1);
        assert_eq!(coarse.freq_hz, full.freq_hz);
        for fi in 0..full.nfreqs() {
            let error = (coarse.at(fi, 0, 0) - full.at(fi, 0, 0)).l1_norm();
            assert!(error < 1e-3, "point {fi} drifted by {error}");
        }
    }

    /// The 1-port device in all three formats, same check as the multi-port
    /// families get.
    #[test]
    fn the_one_port_exports_agree_across_all_three_formats() {
        let ri = ok(RI_GHZ);
        assert_agrees(&ok(MA_GHZ), &ri, "1-port ma");
        assert_agrees(&ok(DB_GHZ), &ri, "1-port db");
    }
}

// ------------------------------------------ real multi-port ADS exports

/// The multi-port exports, generated in Keysight ADS for this milestone.
///
/// All are **non-reciprocal** and **frequency-dependent** by construction,
/// and neither property is decoration. A reciprocal device cannot catch a
/// transposed read, because `S(i,j) == S(j,i)` makes the bug invisible — the
/// reason the spec's own 3-port example (a power divider) would be useless
/// here. A frequency-flat device cannot catch a data-set boundary that slips
/// by a whole point, because every point would then be wrong identically and
/// still self-consistent. See `tests/data/README.md`.
mod multiport {
    use super::*;

    const RI_3: &str = include_str!("../../../tests/data/ads_asymmetric_3port_ri.s3p");
    const MA_3: &str = include_str!("../../../tests/data/ads_asymmetric_3port_ma.s3p");
    const DB_3: &str = include_str!("../../../tests/data/ads_asymmetric_3port_db.s3p");
    const RI_4: &str = include_str!("../../../tests/data/ads_asymmetric_4port_ri.s4p");
    const MA_4: &str = include_str!("../../../tests/data/ads_asymmetric_4port_ma.s4p");
    const DB_4: &str = include_str!("../../../tests/data/ads_asymmetric_4port_db.s4p");
    const RI_4_SCI: &str =
        include_str!("../../../tests/data/ads_asymmetric_4port_ri_scientific.s4p");
    const RI_16: &str = include_str!("../../../tests/data/ads_asymmetric_16port_ri.s16p");
    const MA_16: &str = include_str!("../../../tests/data/ads_asymmetric_16port_ma.s16p");
    const DB_16: &str = include_str!("../../../tests/data/ads_asymmetric_16port_db.s16p");

    /// Every entry of `net` differs from its transpose, so any test built on
    /// this file is capable of failing when the matrix is transposed.
    fn assert_not_reciprocal(net: &Network) {
        let mut off_diagonal = 0;
        for fi in 0..net.nfreqs() {
            for row in 0..net.nports {
                for col in 0..row {
                    assert_ne!(
                        net.at(fi, row, col),
                        net.at(fi, col, row),
                        "S({},{}) equals its transpose at point {fi}; this fixture \
                         cannot detect a transposed read",
                        row + 1,
                        col + 1
                    );
                    off_diagonal += 1;
                }
            }
        }
        assert!(off_diagonal > 0);
    }

    /// A 3-port data set is three lines of 7, 6, 6 tokens. The two values
    /// below are the second pair of the set's first line and the first pair
    /// of its second line — S(1,2) and S(2,1). Reading the matrix
    /// column-major, or applying the 2-port's 21-before-12 rule here, swaps
    /// exactly these two.
    #[test]
    fn a_real_three_port_export_is_row_major() {
        let net = ok(RI_3);
        assert_eq!(net.nports, 3);
        assert_eq!(net.nfreqs(), 10);
        assert_eq!(net.freq_hz[0], 1e9);
        assert_eq!(net.freq_hz[9], 10e9);
        assert_not_reciprocal(&net);

        assert_eq!(
            net.at(0, 0, 1),
            Complex64::new(-0.170972558, 0.0284282697),
            "S(1,2), the second pair of the data set's first line"
        );
        assert_eq!(
            net.at(0, 1, 0),
            Complex64::new(-0.0999570284, -0.0120456901),
            "S(2,1), the first pair of the data set's second line"
        );
        assert_eq!(
            net.at(9, 2, 2),
            Complex64::new(-0.552727569, -0.480091487),
            "S(3,3) at the last point, the final pair of the final data set"
        );
    }

    /// A 4-port data set opens with nine tokens — the same shape a
    /// *complete* 2-port set has. Only running on to 33 values distinguishes
    /// them, so the port count here is the assertion.
    #[test]
    fn a_real_four_port_export_is_not_read_as_a_two_port() {
        let net = ok(RI_4);
        assert_eq!(net.nports, 4, "nine tokens on the set's first line");
        assert_eq!(net.nfreqs(), 10);
        assert_not_reciprocal(&net);

        assert_eq!(
            net.at(0, 0, 0),
            Complex64::new(0.48734362, 0.15525673),
            "S(1,1)"
        );
        assert_eq!(
            net.at(0, 0, 3),
            Complex64::new(0.0073463711, 0.0483004276),
            "S(1,4), the last pair of the set's first line"
        );
        assert_eq!(
            net.at(0, 3, 0),
            Complex64::new(0.00380342348, -0.0139156333),
            "S(4,1), which a transposed read would swap with S(1,4)"
        );
    }

    /// Sixteen ports is the layout 3- and 4-port files never produce: a
    /// single matrix *row* is sixteen pairs, so it spans four lines and a
    /// data set contains lines that are neither its first nor a row start.
    /// The file's lines run `9, 8, 8, 8, …` — one odd line per data set, 64
    /// lines apiece.
    #[test]
    fn a_real_sixteen_port_export_wraps_each_matrix_row_over_four_lines() {
        let net = ok(RI_16);
        assert_eq!(net.nports, 16);
        assert_eq!(net.nfreqs(), 10);
        assert_eq!(net.s.len(), 10 * 256);
        assert_eq!(net.z0.len(), 16);
        assert_not_reciprocal(&net);

        // S(1,16) closes row 1, on the *fourth* line of the data set; a
        // reader that stopped wrapping after one continuation line would
        // never reach it.
        assert_eq!(
            net.at(0, 0, 15),
            Complex64::new(0.0581396322, -0.0160554818),
            "S(1,16)"
        );
        // S(16,1) opens row 16, on the set's 61st line.
        assert_eq!(
            net.at(0, 15, 0),
            Complex64::new(0.24924568, -0.0646806443),
            "S(16,1)"
        );
        assert_eq!(
            net.at(9, 15, 15),
            Complex64::new(-0.370522595, -0.0637966795),
            "S(16,16) at the last point"
        );
    }

    /// Exponent-form numbers inside a *wrapped* data set. The 1-port
    /// scientific fixture covers the spelling on its own; what is new here
    /// is that a token like `4.870256e-01` is still one token to
    /// `split_whitespace`, so the token counts the data-set boundary rule
    /// depends on are unchanged by the notation.
    #[test]
    fn scientific_notation_survives_a_wrapped_multiport_data_set() {
        let plain = ok(RI_4);
        let scientific = ok(RI_4_SCI);
        assert_eq!(scientific.nports, 4);
        assert_eq!(scientific.nfreqs(), 10);
        assert_agrees(&scientific, &plain, "4-port scientific notation");
    }

    /// Each device exported three ways. This is the check that needs no
    /// hand-computed expectations at all: a wrong dB base, a degrees/radians
    /// slip, or a sign error in the angle fails it immediately, at every
    /// port count and across 2,560 values for the 16-port pair alone.
    #[test]
    fn the_multiport_exports_agree_across_all_three_formats() {
        for (n, ri, ma, db) in [
            (3, RI_3, MA_3, DB_3),
            (4, RI_4, MA_4, DB_4),
            (16, RI_16, MA_16, DB_16),
        ] {
            let reference = ok(ri);
            assert_eq!(reference.nports, n);
            let ma = ok(ma);
            let db = ok(db);
            assert_eq!(ma.metadata.format, Format::Ma);
            assert_eq!(db.metadata.format, Format::Db);
            assert_agrees(&ma, &reference, &format!("{n}-port ma"));
            assert_agrees(&db, &reference, &format!("{n}-port db"));
        }
    }
}

/// The QUCS export of the same device: no `!` header at all, verbose
/// `e+009` exponents, and a blank line before a noise-shaped tail with no
/// `! Noise params` label. Its S-data half is M2's business — reaching the
/// noise boundary and naming it proves every layout quirk before that point
/// was handled, since anything else would have failed earlier and
/// differently. Parsing the tail is M3's job.
#[test]
fn the_qucs_export_parses_its_layout_quirks_and_stops_at_the_noise_tail() {
    const QUCS: &str = include_str!("../../../tests/data/qucs_unilateral_2port_ri_with_noise.s2p");
    let (line, kind) = fails(QUCS);
    assert_eq!(kind, ParseErrorKind::NoiseSectionUnsupported);
    assert_eq!(line, 13, "the first line of the unlabelled noise tail");
}

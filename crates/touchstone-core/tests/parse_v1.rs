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
    Complex64, Error, Format, FreqUnit, Network, NoiseData, Parameter, ParseErrorKind,
    ParseOptions, Version, parse_file, parse_str, parse_str_with,
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

    assert_eq!(net.metadata.version, Version::V1_0);
    assert_eq!(net.metadata.freq_unit, FreqUnit::GHz);
    assert_eq!(net.metadata.parameter, Parameter::S);
    assert_eq!(net.metadata.format, Format::Ri);
    assert_eq!(net.metadata.resistances, [50.0]);
}

/// The 2.1 document's "Version 1.1" file: a v1 option line carrying one
/// reference resistance per port rather than one for all of them.
///
/// Nothing in such a file announces itself — 1.x files have no `[Version]`
/// keyword — so the option line's shape is the only evidence, and `z0` is the
/// only place the difference shows up in the parsed network.
mod version_1_1 {
    use super::*;

    /// A 2-port whose ports are referenced to different impedances. Before
    /// this was supported the file failed with `unknown token '75'`.
    const PER_PORT_R: &str = "# GHZ S RI R 50 75\n1.0 0.1 0.2 0.3 0.4 0.5 0.6 0.7 0.8\n";

    #[test]
    fn per_port_resistances_reach_z0_in_order() {
        let net = ok(PER_PORT_R);
        assert_eq!(net.z0, [50.0, 75.0]);
        assert_eq!(net.metadata.version, Version::V1_1);
        assert_eq!(net.metadata.resistances, [50.0, 75.0]);
    }

    /// A single value still means "this port and every other", which is what
    /// keeps every 1.0 file in the suite reading exactly as before.
    #[test]
    fn one_value_is_still_broadcast_to_every_port() {
        let net = ok("# GHZ S RI R 75\n1.0 0.1 0.2 0.3 0.4 0.5 0.6 0.7 0.8\n");
        assert_eq!(net.z0, [75.0, 75.0]);
        assert_eq!(net.metadata.version, Version::V1_0);
    }

    /// The count cannot be checked when the option line is read — a v1 file
    /// does not state its port count, and here it is not known until the first
    /// data set closes. The error still points at the option line, because
    /// that is where the mistake is.
    #[test]
    fn a_list_that_does_not_match_the_port_count_names_the_option_line() {
        let (line, kind) = fails("# GHZ S RI R 50 75 100\n1.0 0.1 0.2 0.3 0.4 0.5 0.6 0.7 0.8\n");
        assert_eq!(line, 1, "the option line, not the data line");
        assert_eq!(
            kind,
            ParseErrorKind::WrongResistanceCount {
                source: "the option line",
                expected: 2,
                found: 3,
            }
        );
    }
}

/// **The ordering guard.** Spec v1.1 §3 writes a 2-port line as
/// S11 S21 S12 S22 — 21 *before* 12. A swap here silently transposes every
/// matrix, and no passive device's data can detect it: a reciprocal network
/// has S21 == S12, so a transposed read of a filter or a capacitor is
/// indistinguishable from a correct one however many points it has. The
/// values below are deliberately far apart, the way a unilateral amplifier's
/// are.
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

/// Index-encoded matrix entries, so a transposed or mis-framed read is
/// visible at a glance instead of hiding behind plausible numbers. The real
/// part of S(row, col) at frequency `fi` reads back as `fi`, `row` and `col`
/// spelled out: point 2's S(3,4) is `200304`.
///
/// **Every position must encode to a distinct value**, or the helper stops
/// being able to detect the thing it exists to detect. Two digits per index
/// is not enough — at eleven ports and up, `row*10 + col` collides (S(1,11)
/// and S(2,1) both give 21), and the 33-port test below would then accept a
/// swap between exactly those positions. Three digits per index, with the
/// frequency above both, is unique for every port count this suite reaches.
fn entry(fi: usize, row: usize, col: usize) -> f64 {
    assert!(row < 999 && col < 999, "index encoding needs a wider field");
    (fi * 1_000_000 + (row + 1) * 1_000 + (col + 1)) as f64
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
                    line.push_str(&format!(" {} 0", entry(fi, row, col)));
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
                    Complex64::new(entry(fi, row, col), 0.0),
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
    // Values follow `entry`: 1002 is S(1,2) at point 0, 1001002 at point 1.
    let net = ok(concat!(
        "# GHZ S RI R 50\n",
        "1.0  1001 0  1002 0  1003 0   ! frequency line 1\n",
        "     2001 0  2002 0  2003 0\n",
        "     3001 0  3002 0  3003 0\n",
        "\n",
        "2.0  1001001 0  1001002 0  1001003 0   ! frequency line 2\n",
        "\n",
        "     1002001 0  1002002 0  1002003 0\n",
        "     1003001 0  1003002 0  1003003 0\n",
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
    // Values follow `entry`: 2003 is S(2,3) at point 0.
    let one_line = ok(concat!(
        "# GHZ S RI R 50\n",
        "1.0 1001 0 1002 0 1003 0 2001 0 2002 0 2003 0 3001 0 3002 0 3003 0\n",
    ));
    let ragged = ok(concat!(
        "# GHZ S RI R 50\n",
        "1.0 1001 0 1002 0\n",
        "1003 0 2001 0 2002 0 2003 0 3001 0 3002 0\n",
        "3003 0\n",
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

/// A port count arrives unvetted from the filename, so `x.s99999999999p`
/// hands the parser 10¹¹ ports. `1 + 2n²` overflows there, and left
/// unchecked it wraps: the reported size becomes nonsense, and a wrapped
/// size that happened to match the accumulated length would index past the
/// end of the value slice.
///
/// This is arithmetic, not the policy ceiling ADR 0006 declined to impose —
/// a data set that does not fit in a `usize` cannot describe a file that
/// fits on a disk.
#[test]
fn a_port_count_too_large_to_describe_a_data_set_is_rejected() {
    let dir = std::env::temp_dir().join(format!(
        "touchstone_rs_m2_huge_ports_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("wild.s99999999999p");
    std::fs::write(&path, MINIMAL).expect("write");

    let err = parse_file(&path).expect_err("10^11 ports cannot describe a data set");
    assert!(
        matches!(
            err,
            Error::Parse {
                kind: ParseErrorKind::UnusablePortCount {
                    nports: 99_999_999_999
                },
                ..
            }
        ),
        "got {err:?}"
    );
    // Reported at the first data line, not after reading the whole file.
    assert!(matches!(err, Error::Parse { line: 2, .. }), "got {err:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// Zero ports is not a network. Left through, it yields a `Network` with an
/// empty `s` whose `at()` panics on every index — a broken value handed back
/// as a success.
#[test]
fn a_zero_port_count_is_rejected_rather_than_producing_an_empty_network() {
    let opts = ParseOptions::new().nports(0);
    let err =
        parse_str_with("# GHZ S RI R 50\n1.0\n", &opts).expect_err("0 ports is not a network");
    assert!(
        matches!(
            err,
            Error::Parse {
                kind: ParseErrorKind::UnusablePortCount { nports: 0 },
                ..
            }
        ),
        "got {err:?}"
    );
    assert_eq!(err.to_string(), "line 2: port count must be at least 1");
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
    // The process id keeps two concurrent `cargo test` runs on one machine
    // from sharing this directory — and, worse, from `remove_dir_all`-ing it
    // out from under each other mid-test.
    let dir =
        std::env::temp_dir().join(format!("touchstone_rs_m2_extension_{}", std::process::id()));
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

// ----------------------------------------------------------- noise parameters

/// Built to the shape of spec v1.1 §3 p11's own worked example (Example 8),
/// with values of our own — the ADR conventions keep spec text out of this
/// repository, and it is the shape that carries the meaning anyway.
///
/// The whole feature in six lines: two S-parameter points spanning 2–22 GHz,
/// then two noise points at 4 and 18 GHz — *inside* the sweep that was just
/// covered. That step back is the only thing marking the boundary. The
/// `! NOISE PARAMETERS` line is a comment like any other and the parser never
/// reads it, which is what makes the unlabelled QUCS export below work.
const NOISY_TWO_PORT: &str = concat!(
    "!2-port network, S-parameter and noise data\n",
    "# GHZ S MA R 50\n",
    "2 .9 -25 3.5 155 .05 75 .65 -15\n",
    "22 .6 -145 1.3 40 .15 40 .55 -85\n",
    "! NOISE PARAMETERS\n",
    "4 .75 .62 61 .35\n",
    "18 2.55 .44 -37 .42\n",
);

/// The noise rows of [`NOISY_TWO_PORT`], reused under other option lines.
const NOISE_ROWS: &str = concat!("4 .75 .62 61 .35\n", "18 2.55 .44 -37 .42\n");

fn noise_of(input: &str) -> NoiseData {
    ok(input).noise.expect("should have a noise section")
}

#[test]
fn a_file_with_both_sections_reads_both_of_them() {
    let net = ok(NOISY_TWO_PORT);

    // The S-data is unaffected by what follows it.
    assert_eq!(net.nports, 2);
    assert_eq!(net.freq_hz, [2e9, 22e9]);
    assert_eq!(net.nfreqs(), 2, "the noise rows are not extra S points");

    let noise = net.noise.expect("the file has a noise section");
    assert_eq!(
        noise.freq_hz,
        [4e9, 18e9],
        "noise frequencies normalize to Hz like any other"
    );
    assert_eq!(noise.nfmin_db, [0.75, 2.55]);
    // `.62 <61` and `.44 <-37`, per p10's "(MA)" — see the test below.
    // Written to ten figures, which is far inside `assert_close`'s 1e-9:
    // these are expectations, not a claim about the last bit.
    assert_close(
        noise.gamma_opt[0],
        Complex64::new(0.300_581_964_6, 0.542_264_218_4),
        "gamma_opt at 4 GHz",
    );
    assert_close(
        noise.gamma_opt[1],
        Complex64::new(0.351_399_624_4, -0.264_798_610_2),
        "gamma_opt at 18 GHz",
    );
}

/// **Γopt is magnitude-and-angle in every file.** Spec v1.1 §3 p10 marks the
/// third and fourth noise entries "(MA)" flatly, so — unlike the network data
/// — they do not follow the option line's format. Reading `.62 61` as
/// real/imaginary in an `RI` file would give `0.62 + 61i`: two orders of
/// magnitude out, and silent.
///
/// The three real ADS exports prove the same thing from the other direction
/// (see `the_three_ads_exports_agree_on_their_noise_section`); this pins it
/// on values whose polar reading is unmistakable.
#[test]
fn gamma_opt_is_magnitude_and_angle_whatever_the_option_line_says() {
    let with_format = |format: &str| {
        // The S-data means something different in each format, which is fine
        // — it is the noise rows, byte-identical across the three, that are
        // under test.
        noise_of(&format!(
            "# GHZ S {format} R 50\n\
             2 .5 10 .5 10 .5 10 .5 10\n\
             22 .5 10 .5 10 .5 10 .5 10\n\
             {NOISE_ROWS}"
        ))
    };
    let (ri, ma, db) = (with_format("RI"), with_format("MA"), with_format("DB"));

    // Exactly equal, not merely close: the noise path never consults the
    // format, so the three runs are the same arithmetic on the same tokens.
    assert_eq!(ri, ma, "RI and MA must read the noise section identically");
    assert_eq!(ri, db, "RI and DB must read the noise section identically");

    // And it really is polar, rather than the RI file's numbers taken as-is.
    assert_close(
        ri.gamma_opt[0],
        Complex64::new(0.300_581_964_6, 0.542_264_218_4),
        "gamma_opt from an RI file",
    );
}

/// `Rn` is stored as the file writes it — normalized to the option line's
/// `R`, which is what spec v1.1 §3 p11 says both it and Γopt are given
/// against — and not converted to ohms. With `R 50` on the option line a
/// denormalizing reader would report 17.5 and 21 here.
///
/// This is an API promise, not an oversight: `z0` is on the same `Network`
/// for anyone who wants ohms, and the writer needs the on-disk value back.
#[test]
fn rn_is_kept_normalized_the_way_the_file_writes_it() {
    let noise = noise_of(NOISY_TWO_PORT);
    assert_eq!(noise.rn, [0.35, 0.42]);
}

/// **The `<=` boundary.** Spec p10 says the first noise frequency is *less
/// than* the last S-parameter frequency; p11 says the lowest is *less than or
/// equal to* the highest. A noise sweep starting exactly at the S-sweep's
/// last frequency is legal under p11 and invisible under p10 — and the ADS
/// exports restart theirs inside the S span, so `<=` is the reading that
/// reads real files. See ADR 0007.
///
/// Also pins the other half of the rule: only the *first* noise frequency is
/// bounded by the S-sweep. The rest are free to run past its top, as the
/// second row here does.
#[test]
fn a_noise_sweep_may_begin_at_the_last_s_parameter_frequency() {
    let noise = noise_of(concat!(
        "# GHZ S RI R 50\n",
        "1.0 0 0 0 0 0 0 0 0\n",
        "2.0 0 0 0 0 0 0 0 0\n",
        "2.0 0.7 0.5 45 0.1\n",
        "3.0 0.8 0.4 30 0.2\n",
    ));
    assert_eq!(noise.freq_hz, [2e9, 3e9]);
}

/// A section of one row has no following line to close it, so only the
/// end-of-file flush can emit it. Real files do this: a vendor transistor
/// file may carry two noise rows against twenty-odd S-parameter points.
#[test]
fn a_single_noise_row_is_read_at_end_of_file() {
    let noise = noise_of(concat!(
        "# GHZ S RI R 50\n",
        "1.0 0 0 0 0 0 0 0 0\n",
        "2.0 0 0 0 0 0 0 0 0\n",
        "1.5 0.7 0.5 45 0.1\n",
    ));
    assert_eq!(noise.freq_hz, [1.5e9]);
    assert_eq!(noise.nfmin_db, [0.7]);
    assert_eq!(noise.rn, [0.1]);
}

/// Spec v1.1 §3 p11 states outright that the two sets of frequencies need not
/// match, and vendor files routinely measure noise at a handful of points
/// against hundreds of S-parameter ones — two against twenty-odd is a
/// perfectly ordinary shape. The arrays differ in length and in grid, and
/// nothing may assume otherwise.
#[test]
fn the_noise_grid_need_not_match_the_s_grid() {
    let net = ok(concat!(
        "# GHZ S RI R 50\n",
        "1.0 0 0 0 0 0 0 0 0\n",
        "2.0 0 0 0 0 0 0 0 0\n",
        "3.0 0 0 0 0 0 0 0 0\n",
        "4.0 0 0 0 0 0 0 0 0\n",
        "5.0 0 0 0 0 0 0 0 0\n",
        "1.5 0.7 0.5 45 0.1\n",
        "3.5 0.9 0.4 -60 0.2\n",
    ));
    assert_eq!(net.nfreqs(), 5);
    let noise = net.noise.expect("a noise section");
    assert_eq!(noise.freq_hz, [1.5e9, 3.5e9]);
    assert_eq!(noise.freq_hz.len(), 2);
}

/// Nothing in the noise section is range-checked. `|Γopt|` above 1 is
/// unphysical, and Keysight's own documented example writes it *negative*
/// (a magnitude of `-0.1211`, which is simply the phase turned around);
/// the spec describes clamping `Rn` as something "a simulator may" do, not
/// something a reader does. This is an I/O layer: it reports the file.
#[test]
fn unphysical_noise_values_are_reported_not_rejected() {
    let noise = noise_of(concat!(
        "# GHZ S RI R 50\n",
        "1.0 0 0 0 0 0 0 0 0\n",
        "2.0 0 0 0 0 0 0 0 0\n",
        "1.0 2.0 -0.1211 -0.0003 .4\n",
        "2.0 2.5 1.9 0 -0.5\n",
    ));
    assert_close(
        noise.gamma_opt[0],
        Complex64::new(-0.121_099_999_999_999_99, 6.339_986_264_000_35e-7),
        "a negative magnitude is a phase flip, not an error",
    );
    assert_close(
        noise.gamma_opt[1],
        Complex64::new(1.9, 0.0),
        "|gamma| > 1 is passed through",
    );
    assert_eq!(noise.rn, [0.4, -0.5], "a negative rn is passed through");
}

// The negative paths. A noise section is found by inference, so every one of
// these has to say *noise* — a reader who is told "frequencies must increase"
// on a line they believe is S-data cannot tell whether the parser simply
// found the boundary in the wrong place.

/// A 5-value tail whose frequency keeps *ascending* is not a noise section:
/// spec p11 bounds the lowest noise frequency by the highest S one, and that
/// bound is the entire boundary rule. Such a file is malformed some other
/// way, and gets the accurate value-count message rather than a guess.
///
/// Dropping the frequency comparison would misclassify a legitimate 2-port
/// set wrapped as 5 + 4 (ADR 0006) as a noise section, and silently discard
/// half of it.
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

/// Spec v1.1 §3: noise parameters "can only be included in 2-port network
/// descriptions". A 5-value tail anywhere else is malformed data, and gets
/// the ordinary value-count message.
#[test]
fn noise_is_looked_for_only_in_two_port_files() {
    // 1-port: a set is 3 values, so 5 is simply the wrong count.
    assert_eq!(
        kind("# GHZ S RI R 50\n1.0 0.1 0.2\n2.0 0.1 0.2\n1.0 0.7 0.5 45 0.1\n"),
        ParseErrorKind::WrongValueCount {
            expected: 3,
            found: 5,
        }
    );
    // 3-port: the tail is swallowed as the start of a 19-value set.
    let three_port = concat!(
        "# GHZ S RI R 50\n",
        "1.0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        "2.0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        "1.0 0.7 0.5 45 0.1\n",
    );
    assert_eq!(
        kind(three_port),
        ParseErrorKind::WrongValueCount {
            expected: 19,
            found: 5,
        }
    );
}

/// The file has no S-data for a noise row to step back from, so there is no
/// boundary to find and the row is what it looks like: a bad data set.
#[test]
fn a_file_that_opens_with_a_noise_row_has_no_boundary_to_find() {
    let opts = ParseOptions::new().nports(2);
    let input = "# GHZ S RI R 50\n1.0 0.7 0.5 45 0.1\n";
    assert!(matches!(
        parse_str_with(input, &opts),
        Err(Error::Parse {
            kind: ParseErrorKind::WrongValueCount {
                expected: 9,
                found: 5
            },
            ..
        })
    ));
}

/// **Why the boundary is tested as soon as five values are buffered.** A
/// truncated noise row holds four values — an even count, which the
/// accumulation rule treats as a *continuation*. Left to close on its own,
/// the pair would total nine, exactly a 2-port data set, and the file would
/// be blamed for the frequency-ordering violation that reading is invented
/// from. The boundary is therefore recognized at the fifth value, before the
/// next line can be absorbed.
#[test]
fn a_truncated_noise_row_is_reported_as_one_not_as_a_wrapped_data_set() {
    let (line, kind) = fails(concat!(
        "# GHZ S RI R 50\n",
        "1.0 0 0 0 0 0 0 0 0\n",
        "2.0 0 0 0 0 0 0 0 0\n",
        "1.0 0.7 0.5 45 0.1\n",
        "2.0 0.7 0.5 45\n",
    ));
    assert_eq!(line, 5, "the truncated row, not the row that closed it");
    assert_eq!(
        kind,
        ParseErrorKind::MalformedNoiseLine {
            found: 4,
            noise_starts_at: 4,
        }
    );
    assert_eq!(
        Error::Parse { line, kind }.to_string(),
        "line 5: expected 5 values in a noise parameter line, found 4 \
         (the noise section begins at line 4)"
    );
}

/// **The known cost of testing the boundary early**, pinned so it is a
/// decision rather than a surprise.
///
/// A 2-port file that *both* wraps its data sets 5 + 4 and breaks its own
/// frequency order is invalid under either reading of its fourth line: as the
/// start of a noise section, or as a wrapped data set that steps backwards.
/// The parser draws the boundary, because that is the only reading under
/// which such a file could have been well-formed — and then rejects the 4
/// values that follow. Written nine tokens to a line, the same defect reports
/// itself directly as an ordering fault.
///
/// The file is rejected either way, so nothing is read wrongly; only the
/// diagnosis differs, and it names the line the section was judged to start
/// on so a reader who meant a wrapped set can see the inference. Trading that
/// away would mean making the boundary rule depend on how earlier sets
/// happened to be wrapped — a worse rule, for a message on doubly-malformed
/// input in a layout no generator emits.
#[test]
fn a_wrapped_two_port_set_that_steps_backwards_is_read_as_a_noise_section() {
    let wrapped = concat!(
        "# GHZ S RI R 50\n",
        "10.0 0 0 0 0\n",
        "     0 0 0 0\n",
        "9.0 0 0 0 0\n",
        "     0 0 0 0\n",
    );
    assert_eq!(
        kind(wrapped),
        ParseErrorKind::MalformedNoiseLine {
            found: 4,
            noise_starts_at: 4,
        }
    );

    // The same defect, unwrapped: diagnosed directly.
    assert_eq!(
        kind("# GHZ S RI R 50\n10.0 0 0 0 0 0 0 0 0\n9.0 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::FrequencyNotAscending {
            previous_hz: 10e9,
            current_hz: 9e9,
        }
    );
}

/// Spec v1.1 §3 puts the noise data after *all* the network data. A 9-value
/// line inside the section looks perfectly well-formed on its own, so the
/// message has to name the section and where it began — otherwise the reader
/// is told five values were expected on a line that plainly holds a fine
/// 2-port point, with nothing to say why.
#[test]
fn s_data_after_the_noise_section_is_blamed_on_the_section_it_is_in() {
    let (line, kind) = fails(concat!(
        "# GHZ S RI R 50\n",
        "1.0 0 0 0 0 0 0 0 0\n",
        "2.0 0 0 0 0 0 0 0 0\n",
        "1.0 0.7 0.5 45 0.1\n",
        "3.0 0 0 0 0 0 0 0 0\n",
    ));
    assert_eq!(line, 5);
    assert_eq!(
        kind,
        ParseErrorKind::MalformedNoiseLine {
            found: 9,
            noise_starts_at: 4,
        }
    );
}

/// The noise sweep is strict in the same way the S sweep is (ADR 0004): a
/// repeated or backwards frequency is non-conformant, and silently keeping
/// both points would leave a `Network` whose noise arrays cannot be
/// interpolated. Reported apart from the S-data ordering error so it cannot
/// be read as the boundary having been found in the wrong place.
#[test]
fn noise_frequencies_must_increase() {
    let prefix = "# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0 0\n2.0 0 0 0 0 0 0 0 0\n1.0 0.7 0.5 45 0.1\n";

    let (line, backwards) = fails(&format!("{prefix}0.5 0.8 0.4 30 0.2\n"));
    assert_eq!(line, 5);
    assert_eq!(
        backwards,
        ParseErrorKind::NoiseFrequencyNotAscending {
            previous_hz: 1e9,
            current_hz: 0.5e9,
        }
    );

    // Equal counts as not increasing, exactly as it does for S-data.
    assert_eq!(
        kind(&format!("{prefix}1.0 0.8 0.4 30 0.2\n")),
        ParseErrorKind::NoiseFrequencyNotAscending {
            previous_hz: 1e9,
            current_hz: 1e9,
        }
    );
}

/// Five bare numbers give the reader nothing to go on, so the message names
/// the column. Unlike an S-value, these are checked before conversion: NFmin
/// and Rn are stored as written, so there is no converted value to check.
#[test]
fn a_non_finite_noise_value_names_its_column() {
    let with_row =
        |row: &str| format!("# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0 0\n2.0 0 0 0 0 0 0 0 0\n{row}\n");
    for (row, column) in [
        ("1.0 nan 0.5 45 0.1", "nfmin"),
        ("1.0 0.7 inf 45 0.1", "|gamma_opt|"),
        ("1.0 0.7 0.5 nan 0.1", "angle(gamma_opt)"),
        ("1.0 0.7 0.5 45 inf", "rn"),
    ] {
        let found = kind(&with_row(row));
        assert!(
            matches!(found, ParseErrorKind::NonFiniteNoiseValue { column: c, .. } if c == column),
            "row {row:?} should name {column}, got {found:?}"
        );
    }

    // An unusable *frequency* is still quoted as the file spells it, the way
    // it is anywhere else — `1e400` is not a word in the reader's file.
    assert_eq!(
        kind(&with_row("1e400 0.7 0.5 45 0.1")),
        ParseErrorKind::InvalidNumber("1e400".to_string())
    );
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
    assert_eq!(kind("# GHZ S RI R 50\n"), ParseErrorKind::NoDataLines);
    assert_eq!(
        kind("# GHZ S RI R 50\n! nothing but talk\n"),
        ParseErrorKind::NoDataLines
    );
}

/// A header-only file's error must point at the option line itself, not an
/// arbitrary line 1 — which would be actively wrong once the option line
/// isn't the file's first line.
#[test]
fn a_dataless_file_points_at_the_option_line_not_line_one() {
    let (line, kind) = fails("! preface\n! more preface\n# GHZ S RI R 50\n! trailing talk\n");
    assert_eq!(line, 3, "the option line is on line 3, not line 1");
    assert_eq!(kind, ParseErrorKind::NoDataLines);
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

/// A rejected frequency is quoted as the file spells it, not as the `f64` it
/// parsed to. `1e400` overflows to infinity, and reporting "inf" would send
/// the reader searching their file for a word that is not in it — the whole
/// value of quoting the token is that it can be found.
#[test]
fn a_rejected_frequency_is_quoted_as_the_file_spells_it() {
    assert_eq!(
        kind("# GHZ S RI R 50\n1e400 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::InvalidNumber("1e400".into())
    );
    // Also when the token is finite but the unit scaling takes it past the
    // end of the range.
    assert_eq!(
        kind("# GHZ S RI R 50\n1e308 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::InvalidNumber("1e308".into())
    );
    // And on a wrapped data set, where the frequency is on an earlier line
    // than the one that completes the set.
    assert_eq!(
        kind("# GHZ S RI R 50\n1e400 1 0 2 0 3 0\n4 0 5 0 6 0\n7 0 8 0 9 0\n"),
        ParseErrorKind::InvalidNumber("1e400".into())
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

/// A DOS-era exporter signs off with `0x1A`, the CP/M end-of-file marker.
/// It is not whitespace, so `split_whitespace` hands it over as a token that
/// cannot be a number — and one that prints as nothing at all, leaving the
/// reader an "invalid number: " with an empty quote. Vendor transistor files
/// from the early 1990s end exactly this way, right after their noise
/// section — which is where this turned up.
#[test]
fn a_trailing_dos_eof_marker_does_not_stop_a_file_reading() {
    let net = ok(&format!("{MINIMAL}\u{1a}\r\n"));
    assert_eq!(net.nfreqs(), 1);

    // With a noise section, since that is where a real file puts it.
    let with_noise = ok(&format!("{NOISY_TWO_PORT}\u{1a}\n"));
    assert_eq!(
        with_noise.noise.expect("a noise section").freq_hz,
        [4e9, 18e9]
    );

    // Not a licence to ignore the byte anywhere else: mid-file it would be
    // hiding data behind it, and it is still an error.
    assert!(matches!(
        kind("# GHZ S RI R 50\n\u{1a}\n1.0 0 0 0 0 0 0 0 0\n"),
        ParseErrorKind::InvalidNumber(_)
    ));
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
/// `tests/data/README.md` for provenance.
///
/// `include_str!` resolves relative to *this file* at compile time, so the
/// test does not care what directory it is run from — which `fs::read` would.
/// The path leaves the crate, though, so these fixtures would not travel in
/// a packaged `touchstone-core`; see the release-engineering note in
/// BLUEPRINT.md before the first crates.io publish.
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

/// The full ADS export the fixture above was truncated from, noise section
/// intact. Its `! Noise params` line is a comment the parser never reads —
/// what marks the boundary is the frequency dropping from 10 GHz back to 1.
#[test]
fn the_full_ads_export_reads_both_of_its_sections() {
    const FULL: &str = include_str!("../../../tests/data/ads_unilateral_2port_ri_with_noise.s2p");
    let net = ok(FULL);

    // The S-data must survive unchanged: this is the same ten points as the
    // truncated sibling, which the tests above pin value by value.
    let truncated = ok(include_str!(
        "../../../tests/data/ads_unilateral_2port_ri.s2p"
    ));
    assert_agrees(&net, &truncated, "with and without the noise section");

    let noise = net.noise.expect("the export carries a noise section");
    assert_eq!(noise.freq_hz.len(), 10);
    assert_eq!(noise.freq_hz[0], 1e9);
    assert_eq!(noise.freq_hz[9], 10e9);
    assert_eq!(noise.nfmin_db[0], 0.867_147_791);
    assert_eq!(noise.rn[0], 0.02, "normalized, exactly as written");
    // `0.668046366 <180` — the angle is exactly half a turn, so the real
    // part carries the whole value and the imaginary part is what `sin`
    // makes of π: not quite zero, and not worth pretending otherwise.
    assert_close(
        noise.gamma_opt[0],
        Complex64::new(-0.668_046_366, 0.0),
        "gamma_opt at 1 GHz",
    );

    // The truncated fixture is the same file without the section, so it must
    // report no noise at all rather than an empty one.
    assert!(truncated.noise.is_none());
}

/// The three ADS exports of one device carry **byte-identical noise
/// sections** while writing their S-data three different ways — which is the
/// spec's "(MA)" rule for Γopt observed in a real tool's output rather than
/// read off a page. Equality here is exact, not approximate: the noise path
/// never consults the format, so all three do the same arithmetic on the
/// same tokens.
#[test]
fn the_three_ads_exports_agree_on_their_noise_section() {
    let ri = noise_of(include_str!(
        "../../../tests/data/ads_unilateral_2port_ri_with_noise.s2p"
    ));
    let ma = noise_of(include_str!(
        "../../../tests/data/ads_unilateral_2port_ma_with_noise.s2p"
    ));
    let db = noise_of(include_str!(
        "../../../tests/data/ads_unilateral_2port_db_with_noise.s2p"
    ));
    assert_eq!(ri, ma, "RI and MA");
    assert_eq!(ri, db, "RI and DB");
}

// ------------------------------- the varying-noise 2-port ADS family (M3)

const VARYING_NOISE_RI: &str = include_str!("../../../tests/data/ads_varying_noise_2port_ri.s2p");
const VARYING_NOISE_MA: &str = include_str!("../../../tests/data/ads_varying_noise_2port_ma.s2p");
const VARYING_NOISE_DB: &str = include_str!("../../../tests/data/ads_varying_noise_2port_db.s2p");

/// The fixture family M3 was generated for. The unilateral device's noise
/// section repeats one row ten times — every angle exactly 180°, so `sin` is
/// zero and a real/imaginary swap in Γopt is invisible, and every row equal,
/// so a section read one row out of step would look perfectly self-consistent.
/// Here nothing repeats and no angle sits on an axis.
///
/// The `assert!`s on variation are deliberate: they make the fixture guard
/// its own usefulness, the way `multiport::assert_not_reciprocal` does for
/// the multi-port files. A re-export that flattened this data would fail
/// here rather than quietly weakening the suite.
#[test]
fn the_varying_noise_export_reads_a_section_where_every_value_moves() {
    let net = ok(VARYING_NOISE_RI);
    assert_eq!(net.nfreqs(), 10);

    let noise = net.noise.expect("the export carries a noise section");
    assert_eq!(noise.freq_hz.len(), 10);
    assert_eq!(noise.freq_hz[0], 1e9);
    assert_eq!(noise.freq_hz[9], 10e9);

    // Every column genuinely varies point to point.
    for (what, values) in [("nfmin_db", &noise.nfmin_db), ("rn", &noise.rn)] {
        assert!(
            values.windows(2).all(|w| w[0] != w[1]),
            "{what} must change at every point for this fixture to be worth having"
        );
    }
    assert!(
        noise.gamma_opt.windows(2).all(|w| w[0] != w[1]),
        "gamma_opt must change at every point"
    );
    assert!(
        noise.gamma_opt.iter().all(|g| g.im.abs() > 1e-3),
        "no angle may sit on the real axis, or a re/im swap would go unseen"
    );

    // First point, spelled out: `6.74671044  0.829397701  77.8913543  6.75900622`.
    assert_eq!(noise.nfmin_db[0], 6.746_710_44);
    assert_eq!(noise.rn[0], 6.759_006_22, "normalized, exactly as written");
    assert_close(
        noise.gamma_opt[0],
        Complex64::new(0.173_979_524_4, 0.810_944_925_1),
        "gamma_opt at 1 GHz",
    );
}

/// The cross-format check extended to the noise section. The S-data is
/// written three ways and must agree within the exports' own rounding; the
/// noise rows are byte-identical in all three files, so those must agree
/// *exactly*. That difference is the point: it is the spec's "(MA)" rule for
/// Γopt showing up in a real tool's output rather than on a page.
#[test]
fn the_three_varying_noise_exports_agree_on_both_sections() {
    let (ri, ma, db) = (
        ok(VARYING_NOISE_RI),
        ok(VARYING_NOISE_MA),
        ok(VARYING_NOISE_DB),
    );
    assert_eq!(ri.metadata.format, Format::Ri);
    assert_eq!(ma.metadata.format, Format::Ma);
    assert_eq!(db.metadata.format, Format::Db);

    assert_agrees(&ma, &ri, "varying-noise 2-port ma");
    assert_agrees(&db, &ri, "varying-noise 2-port db");

    let noise = |net: &Network| net.noise.clone().expect("all three carry noise");
    assert_eq!(noise(&ma), noise(&ri), "MA and RI noise sections");
    assert_eq!(noise(&db), noise(&ri), "DB and RI noise sections");
}

/// **Noise on a coarser grid than the S-data.** ADS computes noise at the
/// S-parameter sweep's frequencies, so this fixture is the RI export with
/// every second noise row deleted — each surviving row byte-identical to the
/// source. Vendor files are routinely shaped this way — two noise points
/// against twenty-odd S-parameter ones is ordinary — and nothing in the
/// reader may assume the two arrays share a length or a grid.
#[test]
fn noise_may_sit_on_a_coarser_grid_than_the_s_data() {
    let net = ok(include_str!(
        "../../../tests/data/ads_varying_noise_2port_ri_coarse_grid.s2p"
    ));
    assert_eq!(net.nfreqs(), 10, "the S sweep is untouched");

    let noise = net.noise.expect("a noise section");
    assert_eq!(noise.freq_hz, [1e9, 3e9, 5e9, 7e9, 9e9]);

    // Every surviving row must read exactly as it does in the full export:
    // dropping rows may not shift what lands in the ones that remain, which
    // is the failure a repeating noise section could never expose.
    let full = noise_of(VARYING_NOISE_RI);
    for (sparse, complete) in (0..5).map(|i| (i, i * 2)) {
        assert_eq!(
            noise.freq_hz[sparse], full.freq_hz[complete],
            "freq {sparse}"
        );
        assert_eq!(
            noise.nfmin_db[sparse], full.nfmin_db[complete],
            "nfmin {sparse}"
        );
        assert_eq!(
            noise.gamma_opt[sparse], full.gamma_opt[complete],
            "gamma_opt {sparse}"
        );
        assert_eq!(noise.rn[sparse], full.rn[complete], "rn {sparse}");
    }
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
        assert_eq!(ghz.metadata.freq_unit, FreqUnit::GHz);

        for (unit, expected_unit, other) in [
            ("mhz", FreqUnit::MHz, ok(MA_MHZ)),
            ("hz", FreqUnit::Hz, ok(MA_HZ)),
        ] {
            // The metadata still records what the file said, even though the
            // arrays no longer differ.
            assert_eq!(other.metadata.freq_unit, expected_unit, "{unit}: metadata");
            assert_eq!(other.freq_hz, ghz.freq_hz, "{unit}: frequencies");
            assert_eq!(other.s, ghz.s, "{unit}: values");
        }
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
/// `e+009` exponents, a blank line between the two sections, and — the part
/// that matters here — **no `! Noise params` label anywhere**. Its noise
/// section is found by the frequency heuristic alone, which is the harder
/// and more representative case: a comment cue is a convention, not a rule,
/// and a parser that needed one would fail on this real file.
#[test]
fn the_qucs_export_finds_its_noise_section_without_a_comment_to_mark_it() {
    const QUCS: &str = include_str!("../../../tests/data/qucs_unilateral_2port_ri_with_noise.s2p");
    let net = ok(QUCS);

    assert_eq!(net.nports, 2);
    assert_eq!(net.nfreqs(), 10, "ten S points, not twenty");
    assert_eq!(net.freq_hz[9], 1e10);

    let noise = net.noise.expect("the unlabelled tail is a noise section");
    assert_eq!(noise.freq_hz.len(), 10);
    assert_eq!(
        noise.freq_hz[0], 1e9,
        "the tail restarts the sweep at 1 GHz — the drop from 10 GHz is the \
         only thing marking the boundary"
    );
    assert_eq!(noise.nfmin_db[0], 9.486_103_827_042_164e-1);
    assert_eq!(noise.rn[0], 1.826_178_747_361_001e-2);
    assert_close(
        noise.gamma_opt[0],
        Complex64::new(-0.684_967_198_379_499_8, 0.0),
        "gamma_opt at 1 GHz",
    );
}

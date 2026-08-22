//! Splitting a file into logical lines: content, trailing comment, and a
//! 1-based line number.
//!
//! This is its own module because more than the v1 data parser needs it.
//! The noise-section boundary search and the v2 `[Keyword]` scanner consume
//! exactly the same stream of numbered, comment-stripped lines, and having
//! them share one implementation is the difference between adding a feature
//! and re-deriving comment handling three times.

/// One source line, split at the first `!`.
///
/// Spec v1.1 §2: a comment runs to the end of the line and comments do not
/// nest, so the first `!` always wins and there is nothing to escape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogicalLine<'a> {
    /// 1-based line number, for `Error::Parse { line }`.
    pub number: usize,
    /// Everything before the first `!`, trimmed. Empty when the line held
    /// nothing but whitespace and/or a comment.
    pub content: &'a str,
    /// Comment text after the first `!`, trimmed, with the `!` removed.
    pub comment: Option<&'a str>,
}

/// Split `input` into numbered logical lines.
///
/// Blank and comment-only lines are **yielded**, not skipped, with an empty
/// `content`: the caller distinguishes them by `content.is_empty()`. That is
/// deliberate — header comments feed `Metadata.comments`, so a filter here
/// would only force the caller to reconstruct what was dropped.
///
/// `str::lines` splits on `\n` and strips a trailing `\r`, which covers both
/// LF and CRLF. CR-only input is rejected upstream by
/// [`has_cr_only_line_endings`] rather than silently collapsing to one line.
pub(crate) fn logical_lines(input: &str) -> impl Iterator<Item = LogicalLine<'_>> {
    input.lines().enumerate().map(|(i, raw)| {
        let (content, comment) = match raw.split_once('!') {
            Some((before, after)) => (before, Some(after.trim())),
            None => (raw, None),
        };
        LogicalLine {
            number: i + 1,
            content: content.trim(),
            comment,
        }
    })
}

/// Whether `input` uses carriage returns as its only line terminator.
///
/// Such a file would arrive at the parser as one enormous line and fail with
/// a baffling value-count error on line 1, so it is worth naming explicitly.
pub(crate) fn has_cr_only_line_endings(input: &str) -> bool {
    input.contains('\r') && !input.contains('\n')
}

/// `input` with a trailing DOS end-of-file marker removed.
///
/// A lone `0x1A` (Ctrl-Z, ASCII SUB) at the end of a text file is the CP/M
/// and MS-DOS end-of-file convention, and tools of that era emit one —
/// vendor transistor files from the early 1990s end with exactly this, right
/// after their noise section. The byte carries no data — it *is* the end of
/// the data — but `split_whitespace` does not treat it as whitespace, so left
/// in place it arrives as a token that cannot be a number and that is
/// invisible when quoted back at the reader.
///
/// Only a *trailing* marker is stripped, along with the whitespace around it.
/// A `0x1A` anywhere else stays an error: truncating a file at the first one,
/// as DOS itself did, could silently discard real data — the outcome ADR 0004
/// weighs everything against.
pub(crate) fn without_trailing_eof_marker(input: &str) -> &str {
    let trimmed = input.trim_end();
    trimmed.strip_suffix('\u{1a}').unwrap_or(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(input: &str) -> Vec<(usize, &str, Option<&str>)> {
        logical_lines(input)
            .map(|l| (l.number, l.content, l.comment))
            .collect()
    }

    #[test]
    fn splits_content_from_trailing_comment() {
        assert_eq!(
            parts("1.0 0.5 0.5 ! row 1\n"),
            [(1, "1.0 0.5 0.5", Some("row 1"))]
        );
    }

    #[test]
    fn comment_only_and_blank_lines_yield_empty_content() {
        assert_eq!(
            parts("! header\n\n   \n! another"),
            [
                (1, "", Some("header")),
                (2, "", None),
                (3, "", None),
                (4, "", Some("another")),
            ]
        );
    }

    #[test]
    fn an_empty_comment_is_some_not_none() {
        // `!` alone carries no text but is still a comment, not data.
        assert_eq!(parts("!"), [(1, "", Some(""))]);
    }

    #[test]
    fn only_the_first_bang_splits() {
        assert_eq!(
            parts("1.0 ! see ! the second bang stays"),
            [(1, "1.0", Some("see ! the second bang stays"))]
        );
    }

    #[test]
    fn crlf_endings_leave_no_carriage_return_behind() {
        assert_eq!(
            parts("# GHZ S RI R 50\r\n1.0 0 0 0 0 0 0 0 0\r\n"),
            [
                (1, "# GHZ S RI R 50", None),
                (2, "1.0 0 0 0 0 0 0 0 0", None),
            ]
        );
    }

    #[test]
    fn tabs_and_surrounding_whitespace_are_trimmed() {
        assert_eq!(parts("\t 1.0 0 0 \t\n"), [(1, "1.0 0 0", None)]);
    }

    #[test]
    fn line_numbers_are_one_based_and_count_blanks() {
        let lines: Vec<usize> = logical_lines("a\n\nb\n").map(|l| l.number).collect();
        assert_eq!(lines, [1, 2, 3]);
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert_eq!(parts(""), []);
    }

    #[test]
    fn a_trailing_dos_eof_marker_is_not_part_of_the_file() {
        // How a DOS-era export ends: CRLF, the marker, CRLF.
        assert_eq!(
            without_trailing_eof_marker("1 0 0\r\n\u{1a}\r\n"),
            "1 0 0\r\n"
        );
        assert_eq!(without_trailing_eof_marker("1 0 0\n\u{1a}"), "1 0 0\n");
        // The marker on the same line as the last value, and with no
        // whitespace at all after it.
        assert_eq!(without_trailing_eof_marker("1 0 0\u{1a}"), "1 0 0");
    }

    /// Only the trailing marker goes. Truncating at the first one — what DOS
    /// itself did — would silently drop whatever followed, and data quietly
    /// lost is the outcome this parser is built to avoid.
    #[test]
    fn a_marker_in_the_middle_is_left_where_it_is() {
        let input = "1 0 0\n\u{1a}\n2 0 0\n";
        assert_eq!(without_trailing_eof_marker(input), input);
    }

    #[test]
    fn input_without_a_marker_is_returned_untouched() {
        // Not even the trailing newline is trimmed, so line numbering and
        // every other behaviour are exactly as they were.
        for input in ["1 0 0\n", "1 0 0", "", "   \n"] {
            assert_eq!(without_trailing_eof_marker(input), input, "{input:?}");
        }
    }

    #[test]
    fn detects_cr_only_endings() {
        assert!(has_cr_only_line_endings("a\rb\rc"));
        assert!(!has_cr_only_line_endings("a\r\nb"));
        assert!(!has_cr_only_line_endings("a\nb"));
        assert!(!has_cr_only_line_endings("a"));
    }
}

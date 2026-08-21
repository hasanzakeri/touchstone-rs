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
    fn detects_cr_only_endings() {
        assert!(has_cr_only_line_endings("a\rb\rc"));
        assert!(!has_cr_only_line_endings("a\r\nb"));
        assert!(!has_cr_only_line_endings("a\nb"));
        assert!(!has_cr_only_line_endings("a"));
    }
}

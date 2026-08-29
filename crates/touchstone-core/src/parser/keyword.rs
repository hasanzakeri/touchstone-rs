//! Recognizing a `[Keyword]` line and splitting it from its argument.
//!
//! Spec 2.0's general rule 7 is short and unusually precise, and every clause
//! of it is a decision here: keywords are enclosed in square brackets and
//! start in column 1; no space or tab may follow `[` or precede `]`; the
//! spelling and the placement of non-alphabetic characters must match the
//! document exactly; and where a keyword is several words, **one space or one
//! dash** separates them. Rule 8 adds that an argument is separated from the
//! closing bracket by at least one whitespace character.
//!
//! Two of those are relaxed deliberately, and the rest are enforced.
//!
//! **Column 1 is not enforced.** [`crate::lines`] trims each line before
//! anything sees it, so the information is already gone by here — but it would
//! not be worth spending even if it were present. Real files indent, no data
//! line can begin with `[`, and nothing becomes ambiguous. This is the kind of
//! narrow, named tolerance ADR 0004 admits.
//!
//! **Space and dash are interchangeable** rather than each keyword having one
//! fixed spelling, because rule 7 offers both without saying which belongs
//! where — the document itself writes `[Two-Port Data Order]` with one of each.
//! Repeated separators are *not* collapsed: the rule says one.
//!
//! Everything else is an error rather than a guess. A line that opens with `[`
//! was meant to be a keyword, and reading `[Netwrok Data]` as data would turn
//! a typo into a file that parses to something wrong.

use super::err;
use crate::error::{Error, ParseErrorKind};

/// A keyword defined by spec 2.0 and carried unchanged into 2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keyword {
    Version,
    NumberOfPorts,
    TwoPortDataOrder,
    NumberOfFrequencies,
    NumberOfNoiseFrequencies,
    Reference,
    MatrixFormat,
    MixedModeOrder,
    NetworkData,
    NoiseData,
    BeginInformation,
    EndInformation,
    End,
}

impl Keyword {
    /// The keyword as the specification spells it, brackets included.
    ///
    /// One source of truth shared by error messages and, later, the writer, so
    /// the two cannot drift apart — the same reason `Format::as_str` exists.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Keyword::Version => "[Version]",
            Keyword::NumberOfPorts => "[Number of Ports]",
            Keyword::TwoPortDataOrder => "[Two-Port Data Order]",
            Keyword::NumberOfFrequencies => "[Number of Frequencies]",
            Keyword::NumberOfNoiseFrequencies => "[Number of Noise Frequencies]",
            Keyword::Reference => "[Reference]",
            Keyword::MatrixFormat => "[Matrix Format]",
            Keyword::MixedModeOrder => "[Mixed-Mode Order]",
            Keyword::NetworkData => "[Network Data]",
            Keyword::NoiseData => "[Noise Data]",
            Keyword::BeginInformation => "[Begin Information]",
            Keyword::EndInformation => "[End Information]",
            Keyword::End => "[End]",
        }
    }

    /// The keyword whose name normalizes to `normalized`.
    fn from_normalized(normalized: &str) -> Option<Self> {
        Some(match normalized {
            "version" => Keyword::Version,
            "number of ports" => Keyword::NumberOfPorts,
            "two port data order" => Keyword::TwoPortDataOrder,
            "number of frequencies" => Keyword::NumberOfFrequencies,
            "number of noise frequencies" => Keyword::NumberOfNoiseFrequencies,
            "reference" => Keyword::Reference,
            "matrix format" => Keyword::MatrixFormat,
            "mixed mode order" => Keyword::MixedModeOrder,
            "network data" => Keyword::NetworkData,
            "noise data" => Keyword::NoiseData,
            "begin information" => Keyword::BeginInformation,
            "end information" => Keyword::EndInformation,
            "end" => Keyword::End,
            _ => return None,
        })
    }
}

/// A keyword and whatever followed it on the same line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeywordLine<'a> {
    pub keyword: Keyword,
    /// The text after the closing bracket, trimmed. Empty when the keyword
    /// stood alone — which is not the same as having no argument, since
    /// `[Reference]` and `[Mixed-Mode Order]` may both put theirs on the lines
    /// that follow.
    pub argument: &'a str,
}

/// Whether `content` is a line the keyword reader should be given.
///
/// Only the opening bracket is examined. Everything past that is
/// [`parse_keyword_line`]'s business, so that a line meant as a keyword is
/// diagnosed as a broken keyword rather than silently falling through to be
/// read as data.
pub(crate) fn looks_like_keyword(content: &str) -> bool {
    content.starts_with('[')
}

/// Split a `[Keyword] argument` line.
///
/// `content` must be a trimmed, comment-stripped line for which
/// [`looks_like_keyword`] holds.
pub(crate) fn parse_keyword_line(content: &str, line: usize) -> Result<KeywordLine<'_>, Error> {
    debug_assert!(looks_like_keyword(content));
    let malformed = |detail: &'static str| {
        err(
            line,
            ParseErrorKind::MalformedKeyword {
                text: content.to_string(),
                detail,
            },
        )
    };

    let close = content
        .find(']')
        .ok_or_else(|| malformed("no closing ']'"))?;
    let name = &content[1..close];
    if name.is_empty() {
        return Err(malformed("empty keyword"));
    }
    if name.starts_with([' ', '\t']) || name.ends_with([' ', '\t']) {
        return Err(malformed("no space is allowed inside the brackets"));
    }

    let rest = &content[close + 1..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return Err(malformed(
            "an argument must be separated from ']' by a space",
        ));
    }

    // Case-insensitive, and one dash reads as one space: rule 7 permits either
    // between the words of a multi-word keyword without assigning them.
    let normalized = name.to_ascii_lowercase().replace('-', " ");
    let keyword = Keyword::from_normalized(&normalized)
        .ok_or_else(|| err(line, ParseErrorKind::UnknownKeyword(format!("[{name}]"))))?;

    Ok(KeywordLine {
        keyword,
        argument: rest.trim(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(content: &str) -> KeywordLine<'_> {
        parse_keyword_line(content, 1).expect("should parse")
    }

    fn kind(content: &str) -> ParseErrorKind {
        match parse_keyword_line(content, 4) {
            Err(Error::Parse { line, kind }) => {
                assert_eq!(line, 4, "the source line number must be carried through");
                kind
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn splits_a_keyword_from_its_argument() {
        assert_eq!(
            parse("[Version] 2.0"),
            KeywordLine {
                keyword: Keyword::Version,
                argument: "2.0",
            }
        );
        assert_eq!(
            parse("[Number of Ports] 4"),
            KeywordLine {
                keyword: Keyword::NumberOfPorts,
                argument: "4",
            }
        );
    }

    /// A keyword may stand alone. That is not the same as having no argument:
    /// `[Reference]`'s values are allowed to begin on the next line, which is
    /// exactly what real exports do.
    #[test]
    fn a_keyword_may_stand_alone() {
        assert_eq!(
            parse("[Reference]"),
            KeywordLine {
                keyword: Keyword::Reference,
                argument: "",
            }
        );
        assert_eq!(parse("[Network Data]").argument, "");
        assert_eq!(parse("[End]").keyword, Keyword::End);
    }

    #[test]
    fn matching_is_case_insensitive() {
        for spelling in [
            "[Number of Ports]",
            "[NUMBER OF PORTS]",
            "[number of ports]",
        ] {
            assert_eq!(
                parse(spelling).keyword,
                Keyword::NumberOfPorts,
                "{spelling}"
            );
        }
    }

    /// Rule 7 allows one space or one dash between the words of a multi-word
    /// keyword and does not say which goes where — the document's own
    /// `[Two-Port Data Order]` uses both. So the two are interchangeable.
    #[test]
    fn a_dash_and_a_space_are_the_same_separator() {
        for spelling in [
            "[Two-Port Data Order]",
            "[Two Port Data Order]",
            "[Two-Port-Data-Order]",
            "[TWO-PORT DATA ORDER]",
        ] {
            assert_eq!(
                parse(spelling).keyword,
                Keyword::TwoPortDataOrder,
                "{spelling}"
            );
        }
        assert_eq!(parse("[Number-of-Ports] 2").keyword, Keyword::NumberOfPorts);
        assert_eq!(parse("[Mixed Mode Order]").keyword, Keyword::MixedModeOrder);
    }

    /// "Only *one*" space or dash, so a doubled separator is not the keyword.
    /// Collapsing runs would be inventing a rule the document declines to give.
    #[test]
    fn repeated_separators_are_not_collapsed() {
        assert!(matches!(
            kind("[Number  of Ports] 2"),
            ParseErrorKind::UnknownKeyword(_)
        ));
    }

    /// `[End]` and `[End Information]` differ by one word and mean entirely
    /// different things — one closes the file, the other closes a metadata
    /// block. Worth pinning that they do not collide.
    #[test]
    fn end_and_end_information_are_distinct() {
        assert_eq!(parse("[End]").keyword, Keyword::End);
        assert_eq!(parse("[End Information]").keyword, Keyword::EndInformation);
        assert_eq!(
            parse("[Begin Information]").keyword,
            Keyword::BeginInformation
        );
    }

    #[test]
    fn whitespace_inside_the_brackets_is_rejected() {
        for spelling in ["[ Version] 2.0", "[Version ] 2.0", "[ Version ] 2.0"] {
            assert!(
                matches!(
                    kind(spelling),
                    ParseErrorKind::MalformedKeyword { detail, .. }
                        if detail.contains("inside the brackets")
                ),
                "{spelling}"
            );
        }
    }

    #[test]
    fn an_unclosed_bracket_is_reported_as_such() {
        assert!(matches!(
            kind("[Version 2.0"),
            ParseErrorKind::MalformedKeyword { detail, .. } if detail.contains("closing")
        ));
        assert!(matches!(
            kind("["),
            ParseErrorKind::MalformedKeyword { detail, .. } if detail.contains("closing")
        ));
    }

    #[test]
    fn an_empty_keyword_is_reported_as_such() {
        assert!(matches!(
            kind("[] 2.0"),
            ParseErrorKind::MalformedKeyword { detail, .. } if detail.contains("empty")
        ));
    }

    /// Rule 8: an argument is separated from the closing bracket by whitespace.
    /// Without this, `[Version]2.0` would parse as a bare `[Version]` and the
    /// argument would vanish.
    #[test]
    fn an_argument_glued_to_the_bracket_is_rejected() {
        assert!(matches!(
            kind("[Version]2.0"),
            ParseErrorKind::MalformedKeyword { detail, .. } if detail.contains("separated")
        ));
    }

    /// A misspelling is named, not read as data. The quoted text is what the
    /// file says, so the reader can see the typo.
    #[test]
    fn an_unknown_keyword_is_quoted_back() {
        assert_eq!(
            kind("[Netwrok Data]"),
            ParseErrorKind::UnknownKeyword("[Netwrok Data]".into())
        );
        // A v1 keyword that never existed, and a plausible near-miss.
        assert_eq!(
            kind("[Two-Port Order] 21_12"),
            ParseErrorKind::UnknownKeyword("[Two-Port Order]".into())
        );
    }

    /// Tabs separate an argument as well as spaces, and the argument comes
    /// back trimmed either way.
    #[test]
    fn arguments_are_trimmed_of_any_whitespace() {
        assert_eq!(parse("[Version]\t2.0").argument, "2.0");
        assert_eq!(parse("[Version]    2.0   ").argument, "2.0");
        assert_eq!(parse("[Reference] 50 75 100 25").argument, "50 75 100 25");
    }

    #[test]
    fn only_a_leading_bracket_offers_a_line_to_the_keyword_reader() {
        assert!(looks_like_keyword("[Version] 2.0"));
        assert!(looks_like_keyword("[oops"));
        assert!(!looks_like_keyword("1.0 0.5 0.5"));
        assert!(!looks_like_keyword("# GHZ S RI R 50"));
        assert!(!looks_like_keyword(""));
    }

    /// The canonical spellings are what error messages quote and what a writer
    /// will emit, so they have to match the document exactly.
    #[test]
    fn canonical_spellings_round_trip_through_matching() {
        for keyword in [
            Keyword::Version,
            Keyword::NumberOfPorts,
            Keyword::TwoPortDataOrder,
            Keyword::NumberOfFrequencies,
            Keyword::NumberOfNoiseFrequencies,
            Keyword::Reference,
            Keyword::MatrixFormat,
            Keyword::MixedModeOrder,
            Keyword::NetworkData,
            Keyword::NoiseData,
            Keyword::BeginInformation,
            Keyword::EndInformation,
            Keyword::End,
        ] {
            let text = keyword.as_str();
            assert_eq!(parse(text).keyword, keyword, "{text}");
        }
    }
}

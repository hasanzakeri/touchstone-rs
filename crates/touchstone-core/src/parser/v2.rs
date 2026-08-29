//! The Touchstone 2.0 parser (and 2.1, which is the same grammar).
//!
//! Where [`super::v1`] infers, this reads. A v2 file states its port count,
//! its frequency count, which entries of the matrix it wrote and — for a
//! 2-port — which order it wrote them in, so nothing here has to be deduced
//! from the shape of the data. The parser is a state machine over the keyword
//! sequence rather than a set of heuristics, and the one thing it borrows from
//! v1 is the arithmetic in [`super`].
//!
//! The consequence for data lines is spec 2.0 p13: a frequency point may be
//! split across any number of lines or written on one line of any length, and
//! a new point begins every `2n² + 1` values for a Full matrix, `n² + n + 1`
//! for a triangular one. Counting values is exact here in a way it never was
//! for v1, because the port count and the matrix format are given rather than
//! guessed — so ADR 0006's odd/even rule has no job in this module.

use num_complex::Complex64;

use super::keyword::{Keyword, KeywordLine, looks_like_keyword, parse_keyword_line};
use super::{
    NOISE_VALUES_PER_SET, broadcast_reference, check_option_scope, err, noise_point_from_values,
    reference_per_port, to_complex, values_per_point, values_per_set,
};
use crate::ParseOptions;
use crate::error::{Error, ParseErrorKind};
use crate::lines::{LogicalLine, logical_lines};
use crate::model::{Format, MatrixFormat, Metadata, Network, TwoPortOrder, Version};
use crate::option_line::{Options, parse_option_line};

/// Parse a Touchstone 2.0 or 2.1 file.
///
/// `version` has already been established by the caller's sniff, which is what
/// routed the file here at all.
pub(crate) fn parse_v2(
    input: &str,
    version: Version,
    opts: &ParseOptions,
) -> Result<Network, Error> {
    // The most values this file could possibly hold. Every token needs at
    // least one byte and one separator, so a file of `len` bytes cannot carry
    // more than `len / 2 + 1` of them.
    //
    // This exists because the header's counts are *declarations*, not
    // measurements, and everything the reader sizes from them scales as `n²`
    // or `F·n²`. A hundred-byte file naming forty thousand ports is asking for
    // a matrix of two and a half petabytes; without a bound taken from the
    // file itself, believing it aborts the process before a single data value
    // has been read. The bound is loose — it does not have to be tight, only
    // finite and derived from something the file cannot lie about.
    let file_values = input.len() / 2 + 1;

    let mut header = Header::new(version);
    let mut lines = logical_lines(input);

    // The header runs until `[Network Data]`. Everything the data reader needs
    // is settled by then, which is the whole point of the keyword block.
    let data_starts_at = header.read(&mut lines)?;
    let (declared_noise, noise_count_at) = (header.nnoise, header.nnoise_at);
    let plan = header.finish(data_starts_at, file_values, opts)?;

    let mut data = NetworkData::new(&plan, file_values);
    let mut noise = NoisePoints::default();
    let mut section = Section::Network;

    for line in lines {
        if line.content.is_empty() {
            continue;
        }
        if looks_like_keyword(line.content) {
            let keyword = parse_keyword_line(line.content, line.number)?;
            match (keyword.keyword, section) {
                // Anything at all past `[End]` is content past `[End]`, which
                // is the accurate complaint whatever the keyword happens to be.
                (_, Section::Ended) => {
                    return Err(err(line.number, ParseErrorKind::TrailingDataAfterEnd));
                }
                // A second one of either, which is a repeat rather than a
                // misplacement — the generic ordering message would be true
                // and would send the reader looking in the wrong place.
                (Keyword::NoiseData, Section::Noise) => {
                    return Err(err(
                        line.number,
                        ParseErrorKind::DuplicateKeyword(Keyword::NoiseData.as_str()),
                    ));
                }
                (Keyword::NoiseData, Section::Network) => {
                    data.check_complete(&plan)?;
                    if plan.nports != 2 {
                        return Err(err(
                            line.number,
                            ParseErrorKind::NoiseRequiresTwoPorts {
                                nports: plan.nports,
                            },
                        ));
                    }
                    if declared_noise.is_none() {
                        return Err(err(
                            line.number,
                            ParseErrorKind::MissingKeyword(
                                Keyword::NumberOfNoiseFrequencies.as_str(),
                            ),
                        ));
                    }
                    noise.starts_at = line.number;
                    section = Section::Noise;
                }
                (Keyword::End, Section::Network | Section::Noise) => section = Section::Ended,
                (other, _) => {
                    return Err(err(
                        line.number,
                        ParseErrorKind::KeywordOutOfOrder {
                            keyword: other.as_str(),
                            detail: "the network data has already begun",
                        },
                    ));
                }
            }
            continue;
        }
        match section {
            Section::Network => data.push_line(line.content, line.number, &plan)?,
            Section::Noise => noise.push_line(line.content, line.number, plan.freq_scale)?,
            // Spec 2.0 p25: `[End]` defines the end of the file, and
            // non-comment text after it is an error. Comments are not, and
            // never reach here — they have no content.
            Section::Ended => return Err(err(line.number, ParseErrorKind::TrailingDataAfterEnd)),
        }
    }

    data.check_complete(&plan)?;
    if data.freq_hz.len() != plan.nfreqs {
        return Err(err(
            plan.data_starts_at,
            ParseErrorKind::DeclaredCountMismatch {
                keyword: Keyword::NumberOfFrequencies.as_str(),
                declared: plan.nfreqs,
                found: data.freq_hz.len(),
            },
        ));
    }

    // Spec 2.0 p24 makes the two conditional on each other in both
    // directions: the count is required if noise data is present and
    // prohibited if it is not. A file that declares a count and then has no
    // section is as wrong as one that has a section and declares nothing.
    //
    // Whether the section was *entered*, not which state the reader ended in —
    // `[End]` moves past `Noise` and would otherwise answer yes for a file
    // that never had one.
    let had_noise_section = noise.starts_at != 0;
    let noise = match (declared_noise, had_noise_section) {
        (Some(declared), true) => {
            let found = noise.data.freq_hz.len();
            if found != declared {
                return Err(err(
                    noise.starts_at,
                    ParseErrorKind::DeclaredCountMismatch {
                        keyword: Keyword::NumberOfNoiseFrequencies.as_str(),
                        declared,
                        found,
                    },
                ));
            }
            Some(noise.data)
        }
        (Some(_), false) => {
            return Err(err(
                noise_count_at,
                ParseErrorKind::KeywordNotPermitted {
                    keyword: Keyword::NumberOfNoiseFrequencies.as_str(),
                    detail: "the file has no [Noise Data] section".to_string(),
                },
            ));
        }
        (None, _) => None,
    };

    let z0 = broadcast_reference(&plan.reference, data.freq_hz.len());
    Ok(Network {
        freq_hz: data.freq_hz,
        s: data.s,
        nports: plan.nports,
        z0,
        noise,
        metadata: plan.metadata,
    })
}

/// Which block of the file the reader is in. `[End]` may close either of the
/// first two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Network,
    Noise,
    Ended,
}

/// Everything the data reader needs, fixed before the first value is read.
struct Plan {
    nports: usize,
    nfreqs: usize,
    /// Reference impedance per port, from `[Reference]` or the option line.
    reference: Vec<f64>,
    /// Where each pair of a data set belongs in the `(row, column)` matrix.
    /// Its length is the number of pairs a point holds.
    entry_order: Vec<(usize, usize)>,
    /// Whether an entry off the diagonal also fills its transpose, which is
    /// what makes a triangular file describe a whole matrix.
    mirror: bool,
    values_per_point: usize,
    format: Format,
    freq_scale: f64,
    data_starts_at: usize,
    metadata: Metadata,
}

/// The keyword block: read once, in any order the spec permits, then frozen
/// into a [`Plan`].
struct Header {
    version: Version,
    options: Option<Options>,
    option_line: Option<String>,
    option_line_at: usize,
    nports: Option<usize>,
    nfreqs: Option<usize>,
    nnoise: Option<usize>,
    /// Line `[Number of Noise Frequencies]` was seen on, so a count declared
    /// for a section the file never has is reported where it was declared.
    nnoise_at: usize,
    reference: Option<Vec<f64>>,
    matrix_format: Option<MatrixFormat>,
    two_port_order: Option<TwoPortOrder>,
    comments: Vec<String>,
    /// Line each keyword was seen on, for the duplicate check's message.
    seen: Vec<Keyword>,
}

impl Header {
    fn new(version: Version) -> Self {
        Header {
            version,
            options: None,
            option_line: None,
            option_line_at: 0,
            nports: None,
            nfreqs: None,
            nnoise: None,
            nnoise_at: 0,
            reference: None,
            matrix_format: None,
            two_port_order: None,
            comments: Vec::new(),
            seen: Vec::new(),
        }
    }

    /// Consume lines up to and including `[Network Data]`, returning the line
    /// it was found on.
    fn read<'a>(
        &mut self,
        lines: &mut impl Iterator<Item = LogicalLine<'a>>,
    ) -> Result<usize, Error> {
        while let Some(line) = lines.next() {
            if line.content.is_empty() {
                // Only header comments are kept, exactly as in v1: files carry
                // a comment on every data row and retaining hundreds of them
                // costs allocations no consumer wants.
                if let Some(text) = line.comment {
                    self.comments.push(text.to_string());
                }
                continue;
            }

            if let Some(body) = line.content.strip_prefix('#') {
                if self.options.is_some() {
                    // Spec 2.0 p6: option lines after the first are ignored.
                    continue;
                }
                let parsed = parse_option_line(body, line.number)?;
                check_option_scope(&parsed, line.number)?;
                self.options = Some(parsed);
                self.option_line = Some(line.content.to_string());
                self.option_line_at = line.number;
                continue;
            }

            if !looks_like_keyword(line.content) {
                // Data has arrived in the header. Which of the two things is
                // missing depends on how far the header got: with no option
                // line the file cannot be interpreted at all, and with one it
                // is `[Network Data]` that should have come first.
                return Err(err(
                    line.number,
                    if self.options.is_none() {
                        ParseErrorKind::DataBeforeOptionLine
                    } else {
                        ParseErrorKind::MissingKeyword(Keyword::NetworkData.as_str())
                    },
                ));
            }

            let keyword = parse_keyword_line(line.content, line.number)?;
            if keyword.keyword == Keyword::Version {
                // The dispatcher has already read this line — it is what
                // routed the file here — so it is expected rather than
                // rejected. It must still be the first thing in the file,
                // which is what makes the dispatcher's single-line sniff a
                // complete reading of the rule rather than a shortcut.
                if self.options.is_some() || !self.seen.is_empty() {
                    return Err(err(
                        line.number,
                        ParseErrorKind::KeywordOutOfOrder {
                            keyword: Keyword::Version.as_str(),
                            detail: "it must precede every other non-comment line",
                        },
                    ));
                }
                self.claim(Keyword::Version, line.number)?;
                continue;
            }
            if keyword.keyword == Keyword::NetworkData {
                self.claim(Keyword::NetworkData, line.number)?;
                return Ok(line.number);
            }
            self.apply(keyword, line.number, lines)?;
        }
        Err(err(
            1,
            ParseErrorKind::MissingKeyword(Keyword::NetworkData.as_str()),
        ))
    }

    /// Record that `keyword` has been seen, rejecting a second occurrence.
    fn claim(&mut self, keyword: Keyword, line: usize) -> Result<(), Error> {
        if self.seen.contains(&keyword) {
            return Err(err(
                line,
                ParseErrorKind::DuplicateKeyword(keyword.as_str()),
            ));
        }
        self.seen.push(keyword);
        Ok(())
    }

    /// Apply one header keyword.
    fn apply<'a>(
        &mut self,
        line: KeywordLine<'_>,
        at: usize,
        rest: &mut impl Iterator<Item = LogicalLine<'a>>,
    ) -> Result<(), Error> {
        let name = line.keyword.as_str();
        self.claim(line.keyword, at)?;

        // Every keyword below `[Number of Ports]` needs the port count, and
        // the spec puts it first for exactly that reason.
        let needs_ports = matches!(
            line.keyword,
            Keyword::Reference | Keyword::TwoPortDataOrder | Keyword::MixedModeOrder
        );
        if needs_ports && self.nports.is_none() {
            return Err(err(
                at,
                ParseErrorKind::KeywordOutOfOrder {
                    keyword: name,
                    detail: "it must follow [Number of Ports]",
                },
            ));
        }

        match line.keyword {
            Keyword::Version => unreachable!("handled by the caller"),
            Keyword::NumberOfPorts => {
                if self.options.is_none() {
                    return Err(err(
                        at,
                        ParseErrorKind::KeywordOutOfOrder {
                            keyword: name,
                            detail: "it must follow the option line",
                        },
                    ));
                }
                let nports = positive_integer(line.argument, name, at)?;
                // Rejected here rather than where the multiplication would
                // wrap: a count this large cannot describe a file that fits on
                // a disk, so there is nothing to be gained by reading on.
                values_per_point(nports)
                    .ok_or_else(|| err(at, ParseErrorKind::UnusablePortCount { nports }))?;
                self.nports = Some(nports);
            }
            Keyword::NumberOfFrequencies => {
                self.nfreqs = Some(positive_integer(line.argument, name, at)?);
            }
            Keyword::NumberOfNoiseFrequencies => {
                self.nnoise = Some(positive_integer(line.argument, name, at)?);
                self.nnoise_at = at;
            }
            Keyword::TwoPortDataOrder => {
                let nports = self.nports.expect("checked above");
                if nports != 2 {
                    return Err(err(
                        at,
                        ParseErrorKind::KeywordNotPermitted {
                            keyword: name,
                            detail: format!("this file has {nports} ports, not 2"),
                        },
                    ));
                }
                self.two_port_order = Some(match line.argument {
                    "21_12" => TwoPortOrder::S21First,
                    "12_21" => TwoPortOrder::S12First,
                    other => {
                        return Err(err(
                            at,
                            ParseErrorKind::InvalidKeywordArgument {
                                keyword: name,
                                detail: format!("expected 21_12 or 12_21, found '{other}'"),
                            },
                        ));
                    }
                });
            }
            Keyword::MatrixFormat => {
                self.matrix_format = Some(match line.argument.to_ascii_lowercase().as_str() {
                    "full" => MatrixFormat::Full,
                    "lower" => MatrixFormat::Lower,
                    "upper" => MatrixFormat::Upper,
                    // Quoting `line.argument`, not the lowercased copy the
                    // match ran on, so the reader sees their own spelling.
                    _ => {
                        return Err(err(
                            at,
                            ParseErrorKind::InvalidKeywordArgument {
                                keyword: name,
                                detail: format!(
                                    "expected Full, Lower or Upper, found '{}'",
                                    line.argument
                                ),
                            },
                        ));
                    }
                });
            }
            Keyword::Reference => {
                let nports = self.nports.expect("checked above");
                self.reference = Some(read_reference(line.argument, nports, at, rest)?);
            }
            Keyword::MixedModeOrder => return Err(err(at, ParseErrorKind::MixedModeUnsupported)),
            Keyword::BeginInformation => skip_information_block(at, rest)?,
            Keyword::EndInformation => {
                return Err(err(
                    at,
                    ParseErrorKind::KeywordOutOfOrder {
                        keyword: name,
                        detail: "there is no [Begin Information] before it",
                    },
                ));
            }
            Keyword::NoiseData | Keyword::End => {
                return Err(err(
                    at,
                    ParseErrorKind::KeywordOutOfOrder {
                        keyword: name,
                        detail: "it must follow [Network Data]",
                    },
                ));
            }
            Keyword::NetworkData => unreachable!("handled by the caller"),
        }
        Ok(())
    }

    /// Freeze the header into the plan the data reader runs on.
    ///
    /// `file_values` is the most values the source could hold; it bounds the
    /// declared port count before anything is sized from it. `opts` is checked
    /// here too, since this is where the file's own count becomes known.
    fn finish(
        self,
        data_starts_at: usize,
        file_values: usize,
        opts: &ParseOptions,
    ) -> Result<Plan, Error> {
        let options = self
            .options
            .ok_or_else(|| err(data_starts_at, ParseErrorKind::MissingOptionLine))?;
        let nports = self.nports.ok_or_else(|| {
            err(
                data_starts_at,
                ParseErrorKind::MissingKeyword(Keyword::NumberOfPorts.as_str()),
            )
        })?;
        let nfreqs = self.nfreqs.ok_or_else(|| {
            err(
                data_starts_at,
                ParseErrorKind::MissingKeyword(Keyword::NumberOfFrequencies.as_str()),
            )
        })?;

        // Required for a 2-port and prohibited otherwise (spec 2.0 p8). The
        // prohibition is enforced where the keyword is read; this is the other
        // half.
        if nports == 2 && self.two_port_order.is_none() {
            return Err(err(
                data_starts_at,
                ParseErrorKind::MissingKeyword(Keyword::TwoPortDataOrder.as_str()),
            ));
        }

        // A caller who asserts a port count and a file that declares a
        // different one disagree about what the data means, and one of them is
        // wrong. Saying so beats honouring either silently — the option is
        // documented as asserting the count, not suggesting it.
        if let Some(requested) = opts.nports
            && requested != nports
        {
            return Err(err(
                data_starts_at,
                ParseErrorKind::PortCountMismatch {
                    requested,
                    declared: nports,
                },
            ));
        }

        let matrix_format = self.matrix_format.unwrap_or(MatrixFormat::Full);

        // Checked before `entry_order` is built, because that table holds one
        // entry per matrix element and is the first thing the declared port
        // count gets to size. A file too small to contain one data point of
        // the shape it claims is malformed however the rest of it reads.
        let values_per_point = match matrix_format {
            MatrixFormat::Full => values_per_set(nports, data_starts_at)?,
            // `n² + n + 1`, which is smaller than the Full count checked above
            // and so cannot overflow where that one did not.
            _ => nports * nports + nports + 1,
        };
        if values_per_point > file_values {
            return Err(err(
                data_starts_at,
                ParseErrorKind::DeclaredShapeExceedsFile {
                    keyword: Keyword::NumberOfPorts.as_str(),
                    values_per_point,
                    file_values,
                },
            ));
        }

        let reference = match &self.reference {
            // Spec 2.0 p10: `[Reference]` supersedes the option line's `R`.
            Some(values) => values.clone(),
            None => reference_per_port(
                &options.resistances,
                nports,
                "the option line",
                self.option_line_at,
            )?,
        };

        let entry_order = entry_order(nports, matrix_format, self.two_port_order);
        debug_assert_eq!(values_per_point, 1 + 2 * entry_order.len());
        Ok(Plan {
            nports,
            nfreqs,
            reference,
            values_per_point,
            mirror: matrix_format != MatrixFormat::Full,
            entry_order,
            format: options.format,
            freq_scale: options.freq_unit.to_hz(),
            data_starts_at,
            metadata: Metadata {
                version: self.version,
                freq_unit: options.freq_unit,
                parameter: options.parameter,
                format: options.format,
                resistances: options.resistances,
                reference: self.reference,
                matrix_format: self.matrix_format,
                two_port_order: self.two_port_order,
                option_line: self.option_line,
                comments: self.comments,
            },
        })
    }
}

/// Where each pair of a data set belongs, in the order the file writes them.
///
/// Spec 2.0 pp16–17 for the triangles: data stays row-wise, "row" meaning the
/// matrix's rows rather than the file's lines. Lower runs `11`, `21 22`,
/// `31 32 33`; Upper runs `11 12 13`, `22 23`, `33`.
///
/// The 2-port Full case is the one that carries `[Two-Port Data Order]`, and
/// it is the only place in the format where the file order is not row-major.
/// The triangular 2-port cases need no such distinction: they hold three pairs
/// whose off-diagonal entry fills both halves, so the two orders describe the
/// same matrix — which is why spec 2.0 p14 can say Lower and Upper are
/// identical at two ports.
fn entry_order(
    nports: usize,
    matrix_format: MatrixFormat,
    two_port_order: Option<TwoPortOrder>,
) -> Vec<(usize, usize)> {
    if nports == 2
        && matrix_format == MatrixFormat::Full
        && two_port_order == Some(TwoPortOrder::S21First)
    {
        // S11 S21 S12 S22 — 21 before 12, as Touchstone 1.0 writes it.
        return vec![(0, 0), (1, 0), (0, 1), (1, 1)];
    }
    match matrix_format {
        MatrixFormat::Full => (0..nports)
            .flat_map(|row| (0..nports).map(move |col| (row, col)))
            .collect(),
        MatrixFormat::Lower => (0..nports)
            .flat_map(|row| (0..=row).map(move |col| (row, col)))
            .collect(),
        MatrixFormat::Upper => (0..nports)
            .flat_map(|row| (row..nports).map(move |col| (row, col)))
            .collect(),
    }
}

/// Accumulates values into frequency points.
struct NetworkData {
    freq_hz: Vec<f64>,
    s: Vec<Complex64>,
    block: Vec<f64>,
    /// Source line the current point started on, so a wrapped point's error
    /// points at its beginning.
    block_line: usize,
    /// The token that became the current point's frequency, quoted back when
    /// it turns out to be unusable.
    frequency_token: String,
}

impl NetworkData {
    /// `file_values` bounds every reservation below.
    ///
    /// `[Number of Frequencies]` is a declaration, and reserving on its word
    /// alone lets a file of a few dozen bytes ask for an array of any size it
    /// likes — which is not a rejected file but a dead process. Capping at what
    /// the source could contain keeps the reservation useful for real files
    /// and makes a false one unable to reserve anything the file could not
    /// have held. A count that turns out to be a lie is still reported, by the
    /// mismatch check at the end; this is only about not believing it early.
    fn new(plan: &Plan, file_values: usize) -> Self {
        let points = plan.nfreqs.min(file_values);
        NetworkData {
            freq_hz: Vec::with_capacity(points),
            // A point needs more values than it yields matrix entries, in
            // every matrix format, so the same bound covers this too.
            s: Vec::with_capacity(
                points
                    .saturating_mul(plan.nports)
                    .saturating_mul(plan.nports)
                    .min(file_values),
            ),
            block: Vec::with_capacity(plan.values_per_point),
            block_line: 0,
            frequency_token: String::new(),
        }
    }

    fn push_line(&mut self, content: &str, line: usize, plan: &Plan) -> Result<(), Error> {
        for token in content.split_whitespace() {
            if self.block.is_empty() {
                self.block_line = line;
                self.frequency_token.clear();
                self.frequency_token.push_str(token);
            }
            let value: f64 = token
                .parse()
                .map_err(|_| err(line, ParseErrorKind::InvalidNumber(token.to_string())))?;
            self.block.push(value);

            // A point is complete the moment it is full. Line breaks carry no
            // meaning here (spec 2.0 p13), so there is nothing else to wait
            // for — and closing eagerly keeps an error next to its cause.
            if self.block.len() == plan.values_per_point {
                self.flush(plan)?;
            }
        }
        Ok(())
    }

    /// Fail if a point is half-read — a truncated file, or one that ran into
    /// `[Noise Data]` mid-point. Reported as the value count it is, against
    /// the line the point began on.
    fn check_complete(&self, plan: &Plan) -> Result<(), Error> {
        if self.block.is_empty() {
            return Ok(());
        }
        Err(err(
            self.block_line,
            ParseErrorKind::WrongValueCount {
                expected: plan.values_per_point,
                found: self.block.len(),
            },
        ))
    }

    fn flush(&mut self, plan: &Plan) -> Result<(), Error> {
        let line = self.block_line;
        let frequency = self.block[0] * plan.freq_scale;
        if !frequency.is_finite() {
            return Err(err(
                line,
                ParseErrorKind::InvalidNumber(self.frequency_token.clone()),
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

        let n = plan.nports;
        let base = self.s.len();
        self.s.resize(base + n * n, Complex64::new(0.0, 0.0));
        for (index, &(row, col)) in plan.entry_order.iter().enumerate() {
            let (first, second) = (self.block[1 + 2 * index], self.block[2 + 2 * index]);
            let value = to_complex(first, second, plan.format);
            // Checked after conversion, not on the token: `-inf` in a `DB`
            // magnitude is a zero-magnitude entry and converts to exactly
            // zero. See ADR 0006.
            if !value.is_finite() {
                return Err(err(line, ParseErrorKind::NonFiniteValue { first, second }));
            }
            self.s[base + row * n + col] = value;
            if plan.mirror && row != col {
                self.s[base + col * n + row] = value;
            }
        }

        self.freq_hz.push(frequency);
        self.block.clear();
        Ok(())
    }
}

/// The `[Noise Data]` section.
///
/// Everything about a noise point's five columns is shared with v1 — the
/// section is where the two versions differ, not its contents. What v1 has to
/// *infer* from a frequency stepping backwards, v2 states with a keyword, so
/// none of ADR 0007's boundary machinery applies here: there is no eager
/// five-value test, no terminal-section rule, and no ambiguity to resolve.
///
/// Two consequences of that are worth stating, because they are departures
/// from v1 rather than oversights:
///
/// - **A noise point must be one line.** Spec 2.0 p24 says each noise
///   frequency and its data shall be grouped into a single line, and since the
///   section is delimited there is no reason to be more permissive than the
///   document: requiring it turns a truncated row into a message naming that
///   row, where accumulating by count would silently merge it with the next
///   and surface as a count mismatch pages later.
/// - **The bound on the first noise frequency is not enforced.** The
///   specification requires it to be no greater than the highest network
///   frequency, and in v1 that sentence is load-bearing — it is the only way
///   to find the section at all. Here the keyword has already found it, so a
///   file that breaks the rule is still read exactly right, and rejecting it
///   would discard good data for no diagnostic gain. ADR 0004's test does not
///   bite, because there is no silently-wrong reading available.
#[derive(Default)]
struct NoisePoints {
    data: crate::model::NoiseData,
    /// Line `[Noise Data]` was found on, named in a malformed row's error
    /// because that is usually where a reader's real question points.
    starts_at: usize,
}

impl NoisePoints {
    fn push_line(&mut self, content: &str, line: usize, scale: f64) -> Result<(), Error> {
        let tokens: Vec<&str> = content.split_whitespace().collect();
        let Ok(tokens) = <&[&str; NOISE_VALUES_PER_SET]>::try_from(&tokens[..]) else {
            return Err(err(
                line,
                ParseErrorKind::MalformedNoiseLine {
                    found: tokens.len(),
                    noise_starts_at: self.starts_at,
                },
            ));
        };

        let mut values = [0.0f64; NOISE_VALUES_PER_SET];
        for (value, token) in values.iter_mut().zip(tokens) {
            *value = token
                .parse()
                .map_err(|_| err(line, ParseErrorKind::InvalidNumber(token.to_string())))?;
        }
        let (frequency, nfmin_db, gamma_opt, rn) =
            noise_point_from_values(&values, scale, line, tokens[0])?;

        if let Some(&previous) = self.data.freq_hz.last()
            && frequency <= previous
        {
            return Err(err(
                line,
                ParseErrorKind::NoiseFrequencyNotAscending {
                    previous_hz: previous,
                    current_hz: frequency,
                },
            ));
        }

        self.data.freq_hz.push(frequency);
        self.data.nfmin_db.push(nfmin_db);
        self.data.gamma_opt.push(gamma_opt);
        self.data.rn.push(rn);
        // A 2.x file writes the effective noise resistance in ohms already —
        // the one place the noise columns differ between versions, and the
        // reason `rn_ohms` exists. See ADR 0010.
        self.data.rn_ohms.push(rn);
        Ok(())
    }
}

/// Read `[Reference]`'s arguments, which may continue onto following lines.
///
/// Spec 2.0 p10 allows the values to begin on the line after the keyword and
/// to span several lines, and real exports use that freedom fully — one value
/// per line, indented, each with its own trailing comment. Since the port
/// count is known, the list is complete when it has that many values, and no
/// terminator is needed.
///
/// This is a trap worth naming. Once comments are stripped, a `[Reference]`
/// payload line is indistinguishable from a data line, so a reader that only
/// looked for the keyword and then resumed its normal loop would silently take
/// the reference values as a frequency point.
fn read_reference<'a>(
    argument: &str,
    nports: usize,
    at: usize,
    rest: &mut impl Iterator<Item = LogicalLine<'a>>,
) -> Result<Vec<f64>, Error> {
    let keyword = Keyword::Reference.as_str();
    let mut values: Vec<f64> = Vec::with_capacity(nports);
    let push_all = |text: &str, line: usize, values: &mut Vec<f64>| -> Result<(), Error> {
        for token in text.split_whitespace() {
            let ohms: f64 = token
                .parse()
                .map_err(|_| err(line, ParseErrorKind::InvalidNumber(token.to_string())))?;
            if !ohms.is_finite() || ohms <= 0.0 {
                return Err(err(
                    line,
                    ParseErrorKind::InvalidKeywordArgument {
                        keyword,
                        detail: format!("reference impedances must be positive, found '{token}'"),
                    },
                ));
            }
            values.push(ohms);
        }
        Ok(())
    };

    push_all(argument, at, &mut values)?;
    while values.len() < nports {
        let Some(line) = rest.next() else {
            break;
        };
        if line.content.is_empty() {
            continue;
        }
        if looks_like_keyword(line.content) {
            // The next keyword has arrived with the list still short. Reported
            // against the list, not the keyword, because the list is what is
            // wrong.
            break;
        }
        push_all(line.content, line.number, &mut values)?;
    }

    if values.len() != nports {
        return Err(err(
            at,
            ParseErrorKind::WrongResistanceCount {
                source: keyword,
                expected: nports,
                found: values.len(),
            },
        ));
    }
    Ok(values)
}

/// Skip everything between `[Begin Information]` and `[End Information]`.
///
/// Spec 2.0 p26 reserves the block for future informational keywords and
/// defines none, so there is nothing in it to read — but it still has to be
/// stepped over rather than fallen into, since its contents are neither
/// keywords we know nor data.
fn skip_information_block<'a>(
    at: usize,
    rest: &mut impl Iterator<Item = LogicalLine<'a>>,
) -> Result<(), Error> {
    for line in rest.by_ref() {
        if line.content.is_empty() || !looks_like_keyword(line.content) {
            continue;
        }
        let keyword = parse_keyword_line(line.content, line.number)?;
        if keyword.keyword == Keyword::EndInformation {
            return Ok(());
        }
        return Err(err(
            line.number,
            ParseErrorKind::KeywordOutOfOrder {
                keyword: keyword.keyword.as_str(),
                detail: "[Begin Information] is still open",
            },
        ));
    }
    Err(err(
        at,
        ParseErrorKind::MissingKeyword(Keyword::EndInformation.as_str()),
    ))
}

/// A keyword argument that has to be an integer greater than zero.
fn positive_integer(argument: &str, keyword: &'static str, line: usize) -> Result<usize, Error> {
    if argument.is_empty() {
        return Err(err(
            line,
            ParseErrorKind::InvalidKeywordArgument {
                keyword,
                detail: "no argument given".to_string(),
            },
        ));
    }
    match argument.parse::<usize>() {
        Ok(0) | Err(_) => Err(err(
            line,
            ParseErrorKind::InvalidKeywordArgument {
                keyword,
                detail: format!("expected an integer greater than 0, found '{argument}'"),
            },
        )),
        Ok(value) => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The order the file writes a 2-port's four pairs in, which is the whole
    /// reason `[Two-Port Data Order]` exists.
    #[test]
    fn the_two_port_orders_are_transposes_of_each_other() {
        assert_eq!(
            entry_order(2, MatrixFormat::Full, Some(TwoPortOrder::S21First)),
            [(0, 0), (1, 0), (0, 1), (1, 1)]
        );
        assert_eq!(
            entry_order(2, MatrixFormat::Full, Some(TwoPortOrder::S12First)),
            [(0, 0), (0, 1), (1, 0), (1, 1)]
        );
    }

    /// Spec 2.0 p17's own worked 3-port layouts.
    #[test]
    fn triangular_orders_follow_the_specs_worked_example() {
        assert_eq!(
            entry_order(3, MatrixFormat::Lower, None),
            [(0, 0), (1, 0), (1, 1), (2, 0), (2, 1), (2, 2)]
        );
        assert_eq!(
            entry_order(3, MatrixFormat::Upper, None),
            [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)]
        );
    }

    /// A triangle holds `n(n+1)/2` pairs, so a point is `n² + n + 1` values
    /// against a Full matrix's `2n² + 1`.
    #[test]
    fn a_triangle_holds_half_the_matrix_plus_its_diagonal() {
        for n in 1..=16usize {
            let full = entry_order(n, MatrixFormat::Full, None).len();
            let lower = entry_order(n, MatrixFormat::Lower, None).len();
            let upper = entry_order(n, MatrixFormat::Upper, None).len();
            assert_eq!(full, n * n, "{n}-port full");
            assert_eq!(lower, n * (n + 1) / 2, "{n}-port lower");
            assert_eq!(upper, lower, "{n}-port upper");
            assert_eq!(1 + 2 * full, values_per_point(n).unwrap());
            assert_eq!(1 + 2 * lower, n * n + n + 1);
        }
    }

    /// Every entry of the matrix is covered exactly once by a triangle plus
    /// its mirror — the property that makes a triangular file describe a whole
    /// network rather than half of one.
    #[test]
    fn a_triangle_and_its_mirror_cover_every_entry() {
        for format in [MatrixFormat::Lower, MatrixFormat::Upper] {
            for n in 1..=6usize {
                let mut covered = vec![0u8; n * n];
                for (row, col) in entry_order(n, format, None) {
                    covered[row * n + col] += 1;
                    if row != col {
                        covered[col * n + row] += 1;
                    }
                }
                assert!(
                    covered.iter().all(|&c| c == 1),
                    "{format:?} {n}-port: {covered:?}"
                );
            }
        }
    }

    /// At one port there is nothing to order or mirror, and all three formats
    /// carry the single `11` entry — spec 2.0 p14 says so outright.
    #[test]
    fn one_port_is_the_same_in_every_matrix_format() {
        for format in [MatrixFormat::Full, MatrixFormat::Lower, MatrixFormat::Upper] {
            assert_eq!(entry_order(1, format, None), [(0, 0)], "{format:?}");
        }
    }

    #[test]
    fn a_count_argument_must_be_a_positive_integer() {
        assert_eq!(positive_integer("4", "[k]", 1).unwrap(), 4);
        for bad in ["0", "-1", "2.5", "four", ""] {
            assert!(
                matches!(
                    positive_integer(bad, "[k]", 1),
                    Err(Error::Parse {
                        kind: ParseErrorKind::InvalidKeywordArgument { .. },
                        ..
                    })
                ),
                "{bad:?}"
            );
        }
    }
}

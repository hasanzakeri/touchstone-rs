# 0006 — Data-set accumulation and line-wrapping tolerance

Status: Accepted (2026-08-21)

## Context

Through M1 the parser equated one line with one frequency point, which is
true for 1- and 2-port files and false for everything else. Spec v1.1 §3
allows at most four value pairs per data line, requires each row of the
network matrix to begin on a new line, and puts the frequency value only on
the first line of a data set. A 3-port point therefore occupies three lines,
a 6-port point twelve, and a 16-port point sixty-four.

Two problems follow. First, the parser needs a rule for where one frequency
point ends and the next begins. Second, it needs the port count in order to
apply that rule — but for a string handed to `parse_str` there is no `.sNp`
extension to read it from, and a 4-port's first line holds nine tokens,
exactly as many as a *complete* 2-port data set.

Files in the wild make this worse. The blueprint's survey found generators
that wrap inconsistently, that separate frequency blocks with blank lines,
and that hang a comment off the first line of each block. A rule that
depends on counting lines, or on those blank lines being present, breaks on
real data.

## Decision

### The boundary rule: odd token counts open a data set

**A data line whose token count is odd starts a new data set; an even count
continues the one in progress.**

This is an exact consequence of the format rather than a heuristic. Spec
v1.1 §3 puts the frequency first in a data set's first line and nowhere
else, and every remaining value belongs to a pair that is never split across
a line boundary. A data set's first line therefore always holds `1 + 2k`
tokens and every continuation line holds `2k`. A 6-port file's lines run
`9, 4, 8, 4, 8, 4, …`, and only the first line of each set is odd.

Values accumulate into a buffer; a completed set becomes one frequency
point. Blank and comment-only lines pass through without disturbing a set in
progress, which is what makes the block separators and per-row comments in
Keysight's and QUCS's output harmless.

### Port count: from the caller, the filename, or the data set's size

Precedence is `ParseOptions::nports` > the `.sNp` extension > inference.

Inference reads the first completed data set's length: it holds `1 + 2n²`
values, which is strictly increasing in `n` and so names the port count
exactly. This resolves the 4-port/2-port ambiguity above — the 4-port's set
runs on past its nine-token first line to a full 33 values before the next
odd line closes it. A length that solves for no whole `n` means a truncated
or malformed set, not an exotic port count, and is reported as
`IndeterminatePortCount` naming both ways to supply the count. Once known,
the count is fixed for the file and later sets are measured against it.

### Accepted tolerance: any wrapping whose totals are right

Spec v1.1 §3 requires *exactly* three pairs per line for a 3-port and four
for a 4-port. We do not enforce that. Because sets are delimited by the
odd/even rule and validated on their total size, a file that wraps at other
points parses identically to one that does not.

This is a deliberate leniency in the sense of ADR 0004, not an oversight:
it admits no ambiguity, since the accepted files carry exactly the same
values in exactly the same order, and it costs nothing to allow. Rejecting
these files would buy strictness with no corresponding protection against
misreading data.

### Finiteness is checked after conversion, not on the token

M1 rejected any token that parsed to a non-finite `f64`. That is wrong for
`DB` files: a magnitude of exactly zero has no finite dB value, and the real
Keysight ADS export in `tests/data/` writes S12 as literally `-inf`.
`10^(-inf/20)` is `0`, so the value is perfectly usable.

The check therefore moved to the converted complex value. `-inf` dB yields
an exact zero and is accepted; an `inf` that is still infinite after
conversion, and any `NaN`, fail with `NonFiniteValue`. The frequency is
checked separately, since it is not a converted pair.

### No ceiling on the port count

Spec v1.1 §3 says the format "supports matrixes of unlimited size", and we
enforce no limit. Keysight's documentation describes 5–99 ports and the
crates.io `touchstone` crate stops at 32; neither figure comes from the
specification, and a bogus count is already caught by the value-count check
on the first data set.

## Consequences

- Every port count reads, wrapped or not, and the M1-era `UnsupportedPortCount`
  and `UnsupportedFormat` error kinds no longer have a way to occur. Both
  were removed rather than left permanently unconstructible; `ParseErrorKind`
  is `#[non_exhaustive]`, so restoring a variant later is not a breaking
  change.
- `parse_str` on a malformed 2-port file now reports `IndeterminatePortCount`
  where M1 reported `WrongValueCount { expected: 9, .. }`. The old message
  was only possible because M1 assumed every file was a 2-port; the new one
  is honest about not knowing, and names the fix. `parse_file` on a `.s2p`
  still gets the specific message, which is the common case.
- The noise-section check moved from a 5-value *line* to a completed 5-value
  *set*. A legitimate 2-port set wrapped as 5 + 4 also opens with five
  values, and under the old check would have been misreported as an
  unsupported noise section.
- A file whose last data set is truncated fails at that set rather than at
  end of file, because a known port count lets a completed set be emitted as
  soon as it is full.

## Spec wrinkles recorded

Two places where the specification does not say one thing clearly, noted so
a future reader does not mistake our reading for carelessness.

1. **The noise boundary is stated twice, inconsistently.** Spec v1.1 §3 p10
   says the first noise point's frequency is *less than* the last
   S-parameter frequency; p11 says the lowest noise frequency is *less than
   or equal to* the highest network-parameter frequency. We use `<=`. It is
   the only reading that accepts Keysight's own documented example and the
   real ADS export in `tests/data/`, both of which restart the noise sweep at
   the S-sweep's first frequency rather than below it.

2. **The spec appears to permit carriage-return-only line endings** — §3 p6
   and p8 both describe a data line as "terminated by a newline character
   (CR or CR/LF)". We reject CR-only files (`UnsupportedLineEndings`),
   because such a file would otherwise arrive as one enormous line and fail
   with a baffling error. This is a genuine deviation from the letter of the
   spec, taken knowingly; it is on the lenient-mode backlog.

## Alternatives considered

- **Delimit data sets by blank lines.** Keysight's multi-port examples put
  one between frequency blocks, but the format does not require it and many
  generators omit it entirely. It would work on the examples and fail on the
  corpus.
- **Count lines per data set** (`nports` rows, or `nports * ceil(nports/4)`
  lines). Correct for conformant files, and broken by exactly the
  inconsistent wrapping this milestone set out to tolerate.
- **Require an explicit port count for anything above 2 ports.** Safe, and a
  worse API: `parse_str` would fail on a valid 4-port string that
  `parse_file` reads without complaint, for no reason the caller could see.
- **Keep rejecting non-finite tokens and drop the DB fixture.** Would have
  left a real, correct, vendor-produced file unreadable in order to preserve
  a check that was only ever a proxy for the property actually wanted.

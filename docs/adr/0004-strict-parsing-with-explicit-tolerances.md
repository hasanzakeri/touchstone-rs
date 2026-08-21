# 0004 — Strict parsing, with explicit and named tolerances

Status: Accepted (2026-08-20)

## Context

Real Touchstone files diverge from spec v1.1 in nameable, recurring ways:
non-ASCII bytes inside comments (a degree sign in a temperature note), a
comment trailing every single data row rather than only the header, a
noise-parameter section appended after the S-data with no dedicated marker,
numbers written without a leading zero (`.680`), and vendor documentation
(Keysight's own Touchstone wiki page) that itself lists a file's frequencies
out of order. None of this is exotic; it is what manufacturer and EDA-tool
exports actually look like.

At the same time, the crate is pre-1.0 and has no warnings channel yet.
Given a file that violates the spec, the only two behaviors available today
are "reject it" and "silently produce a possibly-wrong `Network`". For
S-parameter data, a silently wrong value is far more expensive than a
rejected file — a mis-scaled frequency or a transposed matrix entry can
propagate into a design decision unnoticed. Every case below is therefore a
deliberate choice between those two outcomes, not a placeholder.

## Decision

**Strict by default**, with a short, explicit list of tolerances — most of
them narrower than "be lenient," specific to a documented reason:

**Errors** (the general rule): missing option line; a data line preceding
the option line; an unrecognized option-line token; a duplicate option-line
category (two frequency units, two formats, ...); a missing, non-numeric, or
non-positive `R` value; a data line whose value count doesn't match the
inferred or stated port count; an unparseable numeric token; a frequency
that fails to strictly increase; carriage-return-only line endings (which
would otherwise collapse a whole file into a single, baffling line-1 error);
and, for this version specifically, any parameter type other than `S`, any
format other than `RI`, and any port count other than 2 — each reported by
name (`"only ri is supported in this version"`), not as a generic failure,
because this is the first error most new users will hit.

**One spec-mandated silent tolerance**: a second or later option line is
ignored, exactly as §3 requires.

**Deliberate tolerances, each narrower than blanket leniency**:
- Non-ASCII bytes are accepted via a lossy UTF-8 decode. §2 forbids them,
  but every real occurrence observed is a stray byte inside a comment (a
  degree sign), and rejecting an entire measurement file over one character
  in a comment is indefensible.
- Tab characters are accepted; §2 only discourages them.
- A noise-parameter section is *detected*, not merely rejected as malformed
  data: five values on a line whose frequency steps back into the
  already-covered sweep is exactly how the spec says a reader locates the
  boundary. The failure names itself (`NoiseSectionUnsupported`) instead of
  surfacing as a misleading "frequencies must increase" — otherwise this
  version would reject every real amplifier file with the wrong diagnosis.

**Duplicate frequencies are an error**, not a tolerance. Spec v1.1 §3 (and
Keysight's Genesys documentation, independently) says data sets must be in
*increasing* order; a repeated value is non-conformant on the letter of both
documents. Neither document says what a reader should do about a violation,
and the only place either permits equality is *across* sections — the
lowest noise-parameter frequency may equal the highest network-parameter
frequency. `scikit-rf` tolerates duplicate points in practice, which is
worth recording as a real trade-off: a file that reads there may not read
here. Erroring is our call, made deliberately rather than left as an
oversight.

**Errors carry a 1-based line number.** Columns are out of scope for this
version (planned alongside fuzzing).

**Leniency has a home already, so it doesn't need one invented later**: a
future lenient mode adds fields to the existing `#[non_exhaustive]`
`ParseOptions`, and downgrades some entries in the error list above to
warnings returned alongside a successfully parsed `Network` — through new
`_with`-style entry points, never by changing `Network`'s own fields, which
are public and constructed directly in tests today.

## Consequences

- Some real files — anything in MA or DB, anything with more or fewer than
  two ports, anything carrying a noise section — are rejected until M2 or
  M3, and their error messages say so explicitly rather than reading like a
  parser bug.
- The strict list above is, in effect, the backlog for the eventual lenient
  mode: duplicate frequencies, comma-separated values (Keysight's wiki
  claims commas are valid; the spec says whitespace), the glued `R50`/`R=50`
  option-line forms, non-monotonic frequencies, `NaN`/`inf` S-values, and
  carriage-return-only endings are all candidates, each already justified
  above rather than needing new research when M6 arrives.
- The lossy UTF-8 decode means a comment can end up containing U+FFFD in
  place of an unrecognized byte; this is visible only in `Metadata.comments`
  and never affects numeric data.
- `Metadata.comments` retains only comments seen before the first data
  line — a real file with a comment on every data row (observed in a
  Philips transistor export) would otherwise force hundreds of `String`
  allocations no consumer wants. This means `Metadata.comments` is not a
  complete record of every comment in the source file, which matters for
  M5's round-trip fidelity claim and should be stated there, not assumed.

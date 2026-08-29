# 0010 — Effective noise resistance across spec versions

Status: Accepted (2026-08-28)

Extends [ADR 0007](0007-noise-parameter-section.md), which remains in force.
Nothing it decided is reversed here.

## Context

ADR 0007 settled what the five noise columns mean in a Touchstone 1.0 file,
and decided that the fifth — the effective noise resistance — is **stored
exactly as written**, still normalized, rather than converted to ohms on read.
The reasoning was that denormalizing would invent a quantity the file does not
contain, contradict the field's documented meaning, and cost the writer its
round trip.

That reasoning holds. What has changed is that "as written" and "normalized"
were the same thing when it was written, and are not any more.

Spec 2.0 p24 defines the column, and its compatibility notes on p27 repeat the
point: the effective noise resistance is normalized in a 1.0 or 1.1 file and
**not normalized — plain ohms — in a 2.0 or 2.1 file**. The 2.1 document adds
that a 1.1 file normalizes against *port 1's* resistance specifically, which
is the only port whose value is distinguished for this purpose.

The specification demonstrates it in its own worked examples. Spec 2.0 pp24–25
give one device twice, as Example 18 in 1.0 syntax and Example 20 in 2.0
syntax. Four of the five noise columns are byte-identical between them. The
fifth reads `.38` in the first and `19` in the second — a factor of the
option line's default 50 Ω reference.

So `NoiseData::rn` had come to mean two different physical quantities
depending on what wrote the file, while its documentation claimed one. A
caller who read it without knowing the version could be wrong by a factor of
fifty, and nothing in the returned data would look wrong. `Metadata` is not
exposed to Python until the writer milestone, so from Python the version was
not even discoverable.

The other columns do **not** diverge. Spec 2.0 p24 repeats v1's rule that the
source reflection coefficient is a magnitude regardless of the option line's
format, so ADR 0007's Γopt decision carries into 2.0 and 2.1 unchanged — which
is worth stating, because it was the thing most likely to have changed and did
not.

## Decision

**`rn` keeps exactly what the file wrote. A derived `rn_ohms` is added
alongside it.**

- 1.0 and 1.1: `rn_ohms = rn × resistances[0]`, the option line's reference,
  port 1's where they differ.
- 2.0 and 2.1: `rn_ohms = rn`, which is already in ohms.

Both fields are documented in terms of what they hold rather than in terms of
a version, so neither docstring can become false the way the old one did.

This preserves ADR 0007 entirely: nothing is overwritten, nothing is invented
in place, and the writer still has the on-disk value it needs to reproduce its
input. It costs one derived array — a scalar multiply over a vector that is
typically a few dozen points long — and one name.

The alternative framings all move the problem rather than solving it. Storing
one field always in ohms means a 1.x file's numbers silently change and the
writer must re-normalize to round-trip. Storing one field always normalized
means inventing a value a 2.x file does not contain, which is the move ADR
0007 explicitly rejected. Storing only what the file wrote and telling callers
to consult the version leaves the factor-of-fifty trap in place and merely
documents it.

### `[Reference]` does not reach the noise section

Spec 2.0 p24 says the keyword has no effect on noise parameter data, so both
Γopt's reference and the normalization stay against the **option line's** `R`
even in a file whose ports are referenced to different impedances by
`[Reference]`. That distinction is testable: the specification's Example 20
references its two ports to 50 and 25 Ω, so a reader that reached for `z0`
would produce a different answer.

## Consequences

- `NoiseData` gains `rn_ohms`, in the core and in the Python binding.
- The cross-version assertion is exact rather than approximate. One committed
  export and its rewrite in 2.0 syntax produce `rn` values that differ by
  construction and `rn_ohms` values that agree **to the last bit** — the
  derived fixtures carry the converted numbers at full round-trip precision
  precisely so that this can be asserted as equality rather than closeness.
- The README's description of `rn` is corrected. It said "normalized to z0, as
  written", which was true of every file the library could then read and is
  not true now.
- A v1 file's `rn` is unchanged, so nothing that read it before reads
  differently.

## Consequences for the v2 section itself

Recorded here because they are noise decisions, though they follow from the
keyword rather than from the columns:

- **The section is announced, so none of ADR 0007's boundary machinery
  applies.** No eager five-value test, no terminal-section rule, no `<=`
  reading to resolve. That whole apparatus exists because a v1 file gives a
  reader nothing else to go on.
- **A v2 noise point must be one line.** Spec 2.0 p24 groups each noise
  frequency and its data onto a single line. ADR 0006's wrapping tolerance is
  not extended here: with the section delimited, requiring one line turns a
  truncated row into a message naming that row, where accumulating by count
  would merge it with the next and surface as a count mismatch pages later.
- **The bound on the first noise frequency is not enforced.** The
  specification requires it to be no greater than the highest network
  frequency. In v1 that sentence is load-bearing — it is the only way to find
  the section, and ADR 0007 rests on it. In v2 the keyword has already found
  the section, so a file that breaks the rule is still read exactly right, and
  rejecting it would discard good data for no diagnostic gain. ADR 0004's test
  does not bite here: there is no silently-wrong reading to prefer against.
- **`[Number of Noise Frequencies]` is required with a section and prohibited
  without one**, in both directions, and is validated against the rows read
  rather than used to drive the read.

## Alternatives considered

- **Store one `rn`, always in ohms.** Uniform, and arguably the more useful
  unit — it is what the newer document chose. Reverses ADR 0007, changes every
  existing v1 result by a factor of the reference, and costs the writer its
  round trip.
- **Store one `rn`, always normalized.** Keeps the old docstring true with no
  new API, and invents a value a 2.x file does not contain.
- **Store what the file wrote and expose the version.** Faithful and minimal,
  and it puts the factor-of-fifty trap on the caller: someone who does not know
  to branch is still silently wrong. Exposing `Metadata` early would also
  pre-empt a design the writer milestone should make.

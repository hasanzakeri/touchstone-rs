# 0007 — The noise-parameter section: finding it, and what it means

Status: Accepted (2026-08-22)

## Context

A Touchstone v1 2-port file may append a noise-parameter section after its
network data. Spec v1.1 §3 p10–11 describes it in about a page, and almost
every sentence of that page turns into a decision here, because the section
is the least self-describing thing in the format:

- **Nothing marks where it starts.** There is no keyword, no separator, and
  no required comment. The `! Noise params` line that Keysight ADS writes is
  a comment like any other; the QUCS export of the same device writes no such
  line at all. A reader has to *infer* the boundary from the data.
- **The rule for inferring it is stated twice, and the two disagree.** p10
  says the first noise point's frequency is less than the last S-parameter
  frequency. p11 says the lowest noise frequency is less than or equal to the
  highest network-parameter frequency, and adds that this is what lets a
  reader find the boundary at all.
- **The entries do not follow the option line.** p10 gives five entries per
  line and marks the third and fourth `(MA)` — a magnitude and an angle in
  degrees — with no qualification, so they mean the same thing in an `RI`
  file as in an `MA` one.
- **Two of the five are normalized** to the option line's `R`, per p11.
- The spec says nothing at all about the order of noise frequencies, about
  what a reader should do with an unphysical value, or about what happens
  after the section ends.

M1 detected the boundary and rejected the file by name
(`NoiseSectionUnsupported`); ADR 0006 moved that check from a five-value
*line* to a completed five-value *set*. This ADR is the milestone that builds
`NoiseData` there instead, and settles the rest of the page.

## Decision

### The boundary: `<=`, and tested at the fifth value

**A data set of exactly five values, in a 2-port file, whose frequency is
less than or equal to the last S-parameter frequency seen, begins the noise
section.**

`<=`, not `<`, resolves the p10/p11 contradiction in p11's favour. It is the
only reading that accepts the files we have: Keysight's own documented
example and all four committed exports with noise sections restart the noise
sweep at the S-sweep's *first* frequency rather than descending below its
last. Under p10's wording those files have no findable boundary.

The condition is tested **as soon as five values are buffered**, not only
when the set closes. This is not an optimization; it is what makes a
truncated noise row diagnosable. A row of four values is an *even* count,
which ADR 0006's accumulation rule treats as a continuation — so a five-value
noise row followed by a four-value one totals nine, exactly the shape of a
2-port data set, and the file would be reported for a frequency-ordering
violation invented by that reading.

Testing at the fifth value cannot fire on a **well-formed** set: a legitimate
2-port set wrapped as 5 + 4, which ADR 0006 accepts, opens with an
*ascending* frequency, and the boundary condition is precisely that the
frequency does not ascend.

It can fire early on a **malformed** one, and that is the accepted cost. A
2-port file that both wraps its sets 5 + 4 and breaks its own frequency order
has the boundary drawn at the offending line and is then rejected for the
malformed noise row that follows, where the same file written nine tokens to
a line reports the ordering fault directly. Both readings describe an invalid
file and neither reads any data wrongly — only the diagnosis differs. The
noise reading is preferred because it is the only one under which the file
could have been valid, and the error names the line the section was judged to
begin on so that a reader who meant a wrapped data set can see the inference
that was made. The alternative — making the boundary rule conditional on how
earlier sets happened to be wrapped — buys a better message on doubly
malformed input, in a layout no generator emits, at the price of a rule that
can no longer be stated in one sentence.

Everything after the boundary is noise. The section is terminal: spec §3 puts
it after all the network data, so S-parameter data appearing later is
malformed rather than a return to the first section, and is reported as a
malformed noise line naming where the section began.

### Γopt is magnitude-and-angle in every file

p10's `(MA)` on the third and fourth entries is unconditional, so Γopt is
built from a linear magnitude and an angle in degrees even in an `RI` or `DB`
file. Reading `.62 61` as real/imaginary would give `0.62 + 61i` — two orders
of magnitude out, and silent.

The three ADS exports of one device in `tests/data/` demonstrate this from
the tool's side rather than the page's: their S-data is written three
different ways and their noise sections are **byte-identical**. Both 2-port
families in `tests/data/` are committed in all three formats for exactly this
reason, and the tests assert the parsed noise sections are *equal*, not
merely close — the noise path never consults the format, so the three runs
are the same arithmetic on the same tokens.

### Rn is stored as written, still normalized

p11 says Γopt and Rn are given against the option line's `R`, so the number
in the file is already normalized. We store it unchanged rather than
multiplying by `R`.

Denormalizing would invent a quantity the file does not contain, contradict
the field's documented meaning in `NoiseData` and in the Python binding, and
cost M5's writer the value it needs to round-trip. `z0` is on the same
`Network` for any caller who wants ohms. This is worth stating because the
opposite convention is defensible and is what some consumers expect — the
choice is deliberate, not an omission.

### Noise frequencies must strictly increase

The spec is silent here, so this follows ADR 0004's stance for S-data:
strictly increasing, duplicates included, because a repeated or backwards
point leaves noise arrays that cannot be interpolated and there is no
warnings channel to report a tolerated one through. It joins the S-data
equivalent on M6's lenient-mode backlog.

It gets its own error kind rather than reusing `FrequencyNotAscending`. The
generic wording, printed against a line the reader believes is S-data, reads
as though the parser had compared a noise frequency with the S-sweep — which
is a thing it does, at the boundary, and the reader has no way to tell that
apart from a boundary found in the wrong place. Every noise diagnostic
therefore says *noise*, and the malformed-line one also names the line the
section was judged to start on, because that is usually where the real
mistake is.

### Values are reported, not judged

Nothing in the section is range-checked. `|Γopt|` above 1 is unphysical, and
Keysight's own documented example writes it *negative* — which is simply a
phase turned around, and converts perfectly well. p10 leaves clamping an
out-of-range `Rn` to the simulator, as something it *may* do; a reader that
did it would be silently altering measured data. This is an I/O layer.

Non-finite values are the exception, and are rejected: an `inf` or `NaN` in
any of the four columns fails with the column named. Unlike an S-value, these
are checked *before* conversion rather than after — the `-inf` case that put
the S-value check downstream (ADR 0006) has no analogue here, since NFmin and
Rn are stored as written and a non-finite input to the polar conversion
always produces a non-finite output.

### Found on the way: a trailing DOS end-of-file marker is ignored

Not a noise decision, recorded here because this milestone is what exposed
it — the same way ADR 0006 recorded the carriage-return-only line endings it
ran into. A vendor transistor export from 1992 had never been read past its
noise section; once it was, it failed on its last line instead, a lone
`0x1A` (Ctrl-Z, ASCII SUB). That is the CP/M and MS-DOS end-of-file
convention, and files of that vintage carry it.

The byte carries no data, and it is not whitespace either, so it arrived as a
token that could not be a number *and* printed as nothing — `invalid number:`
followed by an empty quote, on a line the reader would see as blank. A
trailing marker, with any whitespace around it, is now stripped before
tokenizing.

This is a tolerance of exactly the kind ADR 0004 admits: unambiguous, named,
and far narrower than leniency. A `0x1A` anywhere *other* than the end is
still an error, because truncating a file at the first one — what DOS itself
did — could silently discard real data, and silently wrong beats loudly
rejected in no case this library cares about.

## Consequences

- `Network.noise` is populated for v1 2-port files with a noise section, and
  the Python binding's `NoiseData` — built at the scaffold session and unused
  since — is now reachable. No API shape changed to get here.
- `NoiseSectionUnsupported` is removed rather than left unconstructible,
  following the precedent ADR 0006 set for `UnsupportedFormat` and
  `UnsupportedPortCount`. `ParseErrorKind` is `#[non_exhaustive]`.
- Three error kinds are added: `NoiseFrequencyNotAscending`,
  `MalformedNoiseLine`, and `NonFiniteNoiseValue`.
- A frequency that is not a usable number is now rejected *before* the
  section and ordering questions are asked of it, since neither means
  anything without it. In an S-data set this moves the check ahead of the
  value-count check: a truncated set whose frequency also overflows now
  reports the frequency rather than the count.
- The noise arrays are independent of the S arrays in both length and grid,
  as p11 permits. Callers must not assume otherwise, and
  `ads_varying_noise_2port_ri_coarse_grid.s2p` exists to keep us honest about it.
- Noise sections in files that are not 2-port are not looked for at all, per
  §3 p10. A five-value tail elsewhere is reported as the wrong value count it
  is.
- Vendor 2-port transistor files — the ones that carry noise sections at all,
  and the reason this milestone exists — read whole for the first time,
  including one whose noise sits on a grid of two points against twenty-one
  S-parameter ones.

## Spec wrinkles recorded

- **§3 p10 permits noise data after G-, H-, S-, Y- or Z-parameters**, not
  only S. Only `S` parses today (ADR 0004), so this is invisible now, but the
  detection deliberately keys on the port count and the frequency step, never
  on the parameter type — M9 can add the other types without touching it.
- The `<=` reading is carried forward from ADR 0006, which recorded the p10 /
  p11 contradiction when the boundary was still being rejected.

## Alternatives considered

- **Trust the `! Noise params` comment.** ADS writes one; QUCS writes
  nothing, and the spec requires nothing. A parser that needed the cue would
  fail on a real file we already have, and one that merely preferred it would
  have two boundary rules that can disagree.
- **Test the boundary only on a closed data set.** Simpler, and what M1 and
  M2 did while the section was being rejected. It cannot distinguish a
  truncated noise row from a wrapped 2-port set, and misreports the file.
- **Denormalize `Rn` to ohms on read.** Friendlier for a caller who wants
  ohms, at the cost of a value the file does not contain, a contradicted
  docstring, and a writer that can no longer reproduce its input.
- **Validate `0 <= |Γopt| <= 1`.** Would reject vendor data that is
  demonstrably written outside that range, in a library whose job is to
  report what the file says.
- **Reuse `WrongValueCount` and `FrequencyNotAscending` inside the section.**
  Fewer error kinds, and both messages are literally accurate — but a reader
  who is told "expected 5 values" against a well-formed 2-port data line, or
  "frequencies must increase" against a noise row, has no way to see that the
  parser is in the noise section at all.

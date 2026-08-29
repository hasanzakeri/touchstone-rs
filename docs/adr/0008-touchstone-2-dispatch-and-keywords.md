# 0008 — Touchstone 2.0: version dispatch and the keyword model

Status: Accepted (2026-08-28)

## Context

Until this milestone the library read one grammar. `Version::V2` existed in
the data model and was never constructed, and the entry point called the v1
parser unconditionally. Adding 2.0 meant answering three questions at once:
how a reader decides which version it is holding, how the keyword header is
recognized, and how much of the v1 parser's hard-won inference survives.

The third turns out to be the interesting one. Touchstone 1.0 states almost
nothing about itself — a frequency unit, a value format, a reference
resistance — so everything else has to be deduced: the port count from the
length of a data set, the end of the network data from a frequency stepping
backwards. ADR 0006 and ADR 0007 are both about making those deductions exact.

A 2.0 file states all of it. That is not a small convenience; it changes what
kind of program the parser is.

## Decision

### Dispatch is one line of lookahead

**The first line with content decides.** If it opens a keyword, that keyword
must be `[Version]`; anything else is a 1.x file.

Spec 2.0 p6 requires `[Version]` in every 2.0 file and requires it to precede
all other non-comment, non-blank lines, so this is a complete reading of the
rule rather than a shortcut — there is nowhere else it can legally be. Real
exports put it on line 1, ahead even of their comment header.

A file whose first keyword is something else is rejected by name rather than
searched further. It is malformed under the rule either way, and the only
thing a search could change is which error the reader gets.

The v2 parser re-reads that same `[Version]` line as it walks the header, and
requires it still to be first. The two readings agree by construction, and the
second is what makes the first safe.

### A `[` line is always a keyword, never data

No data line can begin with `[`. So a v1 parse that meets one reports it as a
2.0 keyword in a file with no `[Version]`, and a malformed or misspelled
keyword is reported as that, rather than either falling through to be read as
a number. Before this, pointing the reader at a 2.0 file produced a complaint
about an unparseable number on some line, which is true and useless.

### Keyword matching: two relaxations, everything else enforced

Spec 2.0's general rule 7 is short and unusually precise. Enforced: brackets
required, no whitespace immediately inside them, and — from rule 8 — an
argument separated from the closing bracket by whitespace. Also enforced:
matching is case-insensitive, which is a general property of the format.

**A space and a dash are the same separator.** Rule 7 permits either between
the words of a multi-word keyword without saying which belongs where, and the
document's own `[Two-Port Data Order]` uses one of each. Repeated separators
are *not* collapsed: the rule says one.

**Column 1 is not enforced.** Rule 7 requires a keyword to start there, but
lines are trimmed before anything sees them, real files indent, and no data
line can begin with `[` — so nothing becomes ambiguous. This is a narrow,
named tolerance of exactly the kind ADR 0004 admits.

### v2 data is counted, not inferred

Spec 2.0 p13: a frequency point may be split across any number of lines or
written on one line of any length, and a new point begins every `2n² + 1`
values for a Full matrix, `n² + n + 1` for a triangular one.

So **ADR 0006's odd/even accumulation rule has no job in the v2 parser.** That
rule is exact for v1 because a v1 data set's first line always holds an odd
count and its continuations an even one; it exists because nothing else could
frame the points. Here the port count and the matrix format are stated, the
count is arithmetic, and line breaks carry no meaning at all. Several points
may share one line, and a point may be split mid-pair.

`[Number of Frequencies]` is **validated against** the points read rather than
used to drive the read. A file that disagrees with itself is reported, instead
of quietly returning whichever of the two numbers happened to win.

### A declared count may not size anything the file could not contain

The counts in a v2 header are claims, and everything sized from them grows as
`n²` or `F·n²`: the index table mapping each pair to its matrix entry, the
matrix itself, the frequency vector. A hundred-byte file may legally *say*
`[Number of Ports] 40000`, and believing it asks for an index table of 1.6
billion entries and a matrix of two and a half petabytes.

That is not a rejected file. It is an allocation abort, which in a Rust library
is a dead process rather than an `Err` — and through the Python binding it
takes the interpreter with it, so a caller cannot catch it at all. A corrupt
count is not an exotic input, and "we validate it afterwards" is no defence
when the process does not survive to get there.

**So every declared count is bounded by the file itself before anything is
sized from it.** A file of `len` bytes cannot hold more than `len / 2 + 1`
whitespace-separated values, since each needs a byte and a separator. That
ceiling is loose by design — it does not need to be tight, only finite and
derived from something the file cannot lie about — and it is used two ways:

- **The port count is checked against it and rejected outright** when a single
  data point of the declared shape would need more values than the whole file
  holds. Such a file is malformed however the rest of it reads, and this is
  the check that has to come first, because the index table is the first thing
  the port count gets to size.
- **The frequency count only caps a reservation**, never a rejection. A file
  may legitimately declare more points than it turns out to contain — that is
  precisely what the mismatch check exists to report — so the count is still
  believed as a *hint* for reserving capacity, just never beyond what the file
  could hold. The arithmetic saturates rather than wrapping.

The ordinary case is unaffected: a real file's declared counts are far below
the ceiling, and the reservation is as useful as it ever was.

### A caller's asserted port count is checked, not ignored

`ParseOptions::nports` exists because a v1 file does not state its port count.
A v2 file does. When a caller asserts one and the file declares another, the
two disagree about what the data means and one of them is wrong, so the
disagreement is reported.

Honouring the file silently would make a documented assertion a no-op — the
option's own documentation says it asserts the count rather than suggesting
it. Honouring the caller would misread data that describes itself perfectly
well. Neither is worth the quiet.

### Two checks belong before the version is known

Carriage-return-only line endings and the trailing DOS end-of-file marker are
properties of the bytes, not of either grammar, so both are handled ahead of
the version sniff rather than inside each parser.

For the line endings this is not tidiness. Such a file has no line breaks the
reader recognizes, so it arrives as one enormous line — and the sniff would
take the entire file as the `[Version]` argument and quote it back in the
error. The message would grow with the input, which for a large file is a
second problem on top of the first.

### Two tolerances, stated

- **A missing `[End]` is accepted.** The keyword marks the end of the file,
  but nothing becomes ambiguous without it — the data simply stops — and
  rejecting an otherwise complete file over a missing terminator would discard
  good data for no diagnostic gain. Non-comment text *after* `[End]` is an
  error, which is what p25 actually calls out.
- **The bound on the first noise frequency is not enforced in v2.** See ADR
  0010.

Both are cases where ADR 0004's test — silently wrong beats loudly rejected in
no case here — simply does not bite, because no silently-wrong reading is
available.

### Mixed-mode is refused by name

Mixed-mode data (2.0 pp20–22) arranges the matrix by mode rather than by port,
so a file carrying `[Mixed-Mode Order]` cannot be read as if it were
single-ended: every value would land somewhere wrong. It is refused with a
message that says so, rather than half-read.

## Consequences

- `parser.rs` becomes `parser/{mod,v1,v2,keyword}.rs`. The arithmetic that does
  not depend on the version — the complex conversion, the polar reading of
  Γopt, the value-count formula — lives in the parent and is written once.
- `Version` becomes four values: `V1_0`, `V1_1`, `V2_0`, `V2_1`, and gains
  `#[non_exhaustive]`. Two grammars, four values, so a writer can reproduce the
  argument its source wrote.
- `Metadata` records `matrix_format`, `two_port_order` and `reference` as
  `Option`s. A v1 2-port is always `S21First` and always Full but never *says*
  so, and `None` is what preserves the difference between a file that stated
  something and one that did not — which M5's writer needs.
- Fourteen error kinds are added. Every one names the rule it enforces rather
  than reading like an internal failure, which is the standard ADR 0004 set.
- The v1 parser is unchanged in behaviour. Its whole test suite passes
  untouched, which is what makes that claim checkable rather than asserted.

## Spec wrinkles recorded

- **The keyword list on p5 prints a keyword that does not exist.** It gives
  `[Two-Port Order]`; the body on p8 and p13 uses `[Two-Port Data Order]`, and
  the published errata corrects p5. A file written to the list is rejected as
  an unknown keyword, which is the right outcome, and there is a test for it.
- **Examples 19 and 20 omit `[Two-Port Data Order]`** although p8 makes it
  required for a 2-port file — Examples 3, 12 and 17 include it. The
  requirement is kept: defaulting the order would reintroduce exactly the
  silent transpose the keyword was introduced to end. Where those examples are
  transcribed as fixtures, the keyword is added and the addition is noted.
- **`[Matrix Format] Full` is emitted even for 2-port files** by at least one
  real exporter, where the distinction is meaningless. Harmless, and accepted.

## Alternatives considered

- **Scan the file for `[Version]` anywhere.** Would accept files the rule
  forbids, and buys nothing: such a file is malformed under p6 whether or not
  we find the keyword, so the only difference is the wording of the error.
- **Infer the version from the presence of any keyword.** Cheaper to describe,
  but it silently accepts a 2.0 file with no `[Version]` at all — and the
  version is what decides whether `Rn` is normalized (ADR 0010). Guessing it
  would make a noise section mean whatever we assumed.
- **Reuse ADR 0006's odd/even accumulation for v2 as well.** One code path
  instead of two, but it would reject files the format explicitly permits:
  several points on one line, or a point split mid-pair. The rule is a
  consequence of v1's silence, not a property of Touchstone.
- **Default `[Two-Port Data Order]` to the v1 convention when absent.** Would
  accept the specification's own Examples 19 and 20 as printed. Rejected: a
  guessed order transposes the matrix silently, which is the one failure this
  format makes easiest and most expensive.
- **Enforce column 1 for keywords** by carrying the untrimmed line through.
  Costs a field on every logical line to reject files nothing is confused by.
- **Impose a fixed ceiling on the port count** — the comparable Rust crate
  stops at 32 — instead of bounding it by the file. Simpler, and it reintroduces
  the arbitrary limit ADR 0006 declined to impose: the format says matrices are
  of unlimited size, and a file large enough to hold a 200-port matrix should
  read. Bounding by the file's own length rejects exactly the impossible cases
  and nothing else.
- **Reserve nothing and let the vectors grow.** Removes the problem by removing
  the optimization, and gives up a real one — the declared counts are accurate
  in every honest file, which is all of them.

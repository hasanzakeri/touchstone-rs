# 0009 — Reference impedance: a per-frequency complex `z0`, and the Version 1.1 option line

Status: Accepted (2026-08-28)

## Context

ADR 0003 recorded, at the scaffold session, that reference impedance was
per-port and real, and that **complex reference impedance was "a known v2
concern"** to be addressed when the v2 milestone arrived. That assumption was
carried for three milestones without being checked.

It is wrong. Spec 2.0 p10 states under `[Reference]` that complex and
imaginary impedance values are not supported, p19 repeats it for the
mixed-mode reference impedances, and the 2.1 document carries the same
sentence unchanged. The published issue-resolution index lists nothing
pending that would add it. **Complex reference impedance is not a feature of
the format in any published version.**

What `[Reference]` does introduce is a reference that is per-port and
*non-uniform* — a 4-port file may declare four different values — where a v1
option line has one `R` for every port. That is a genuine change, and one the
existing `Vec<f64>` already modelled.

Two other things arrived with it:

- **Complex port impedance is real, just not in the format.** Field solvers
  compute a modal impedance per port *per frequency*, complex, and write it
  beside the S-parameters when asked not to renormalize — as a comment
  convention, outside the specification entirely. Every such file gathered for
  this milestone carries it in Touchstone 1.0 form; every 2.0 file gathered
  carries a uniform real `[Reference]` and no impedance data at all. (That is a
  statement about these files and the tool versions that produced them, not
  about what any vendor's software can do.)
- **The 2.1 document defines a "Version 1.1" file**: a v1 option line carrying
  one `R` value per port rather than one for all of them. Its introduction is
  the substantive difference between the 2.0 and 2.1 documents.

## Decision

### `z0` is one `(F, N)` complex array

`Network::z0` holds `F·N` complex values, row-major `(frequency, port)` — the
same layout `s` already uses, reshaped once at the NumPy boundary.

Nothing a conforming file can say populates either the frequency axis or the
imaginary part. Parsing one broadcasts the declared per-port values across the
sweep and leaves every imaginary part zero.

The shape is chosen for what a solver's own port impedance needs. Reading that
convention is a later milestone; committing to the array's shape now means it
will not have to change then, and a public attribute is not reshaped twice.
Nothing is published, so the cost of choosing now is zero and the cost of
choosing later is a second breaking change.

It is also exactly scikit-rf's shape and dtype, which makes the interop
milestone a handoff rather than a conversion.

The declared values stay **real** in `Metadata` — `resistances` from the
option line, `reference` from the keyword — because the specification makes
them real. `z0` is the reference environment that resulted; `Metadata` is what
the file said. A writer needs the latter to reproduce its input, including
which of the two sources supplied it.

### The cost, stated

Every conforming file now stores `F·N` values to carry `N` distinct ones. For a
401-point 16-port sweep that is about 100 KB of duplication. This is accepted:
it is what scikit-rf already does, it is small beside the `F·N²` matrix beside
it, and the alternative is a public attribute that changes shape after release.

### Version 1.1 is supported

The option line's `R` takes one or more values. One is the reference for every
port; more than one must equal the port count.

The list needs no delimiter and no lookahead table: no other option-line token
parses as a number, so values are taken until one does not. That is exact
rather than heuristic. The first value stays mandatory and is still reported
by name when it is not a number, so a typo points at itself instead of
claiming `R` had no value.

**The count cannot be checked where it is read.** A v1 file does not state its
port count — it is unknown until the first data set closes — so the check
happens there, and the error still points back at the option line, which is
where the mistake is.

A file whose option line carries more than one value is recorded as
`Version::V1_1`. Nothing in such a file announces itself; 1.x files carry no
`[Version]` keyword, so the option line's shape is the only evidence there is.

### `[Reference]` does not broadcast

Spec 2.0 p10 requires an argument for every port represented in the data, so a
single value there is an error rather than a shorthand — unlike the option
line's `R`, where one value covering every port is the defined behaviour. The
two look similar and are not, and the reader treats them differently on
purpose.

Its arguments may begin on the line following the keyword and span several
lines, which is what real exports do: the keyword alone, then one value per
line, indented, each with a trailing comment. **This is a trap worth naming.**
Once comments are stripped, a `[Reference]` payload line is indistinguishable
from a data line — a reader that noted the keyword and resumed its normal loop
would take the reference values as a frequency point and be several values
into the file before anything looked wrong. It is easy to fall into: the first
attempt at an independent check of these files during this milestone did
exactly that.

## Consequences

- `Network::z0` is `Vec<Complex64>` of length `F·N`; `net.z0` in Python is
  `complex128` of shape `(F, N)`. `Network::z0_at(fi, port)` joins `at()`.
- The Python constructor accepts `(N,)` and tiles it, accepts `(F, N)` as
  given, and takes real arrays and plain sequences as well as complex ones. A
  caller who has only seen conforming files has no reason to hold complex
  values, and making them build some would tax the ordinary case.
- `Metadata.resistance: f64` becomes `resistances: Vec<f64>`, and `reference:
  Option<Vec<f64>>` is added. This reaches into the v1 option-line parser that
  M1–M3 depend on; every v1 test passing unchanged is the guard.
- ADR 0003's line about complex reference impedance being a v2 concern is
  superseded by this one. The type did change — but for interoperability, not
  because the format required it.
- The integration tests assert, through a shared helper, that a parsed `z0` is
  flat across the sweep and real. A broadcast that filled only the first row
  would fail it.

## Alternatives considered

- **Keep `z0` real and `(N,)`.** The most faithful reading of the format, and
  the smallest change. Rejected because it makes the solver-written impedance
  unrepresentable, guaranteeing a second breaking change to a public attribute
  later.
- **Keep `z0` as the declared reference and add a separate per-frequency
  attribute for the measured one.** Conceptually the tidiest: a declared
  reference and a measured modal impedance are different quantities. Rejected
  on use: for an un-renormalized export the two coincide — the measured
  impedance *is* what the S-data is referenced to — and such a file declares
  nothing, so `z0` would report the 50 Ω default while the truth sat in the
  other attribute. The taxonomy is cleaner and the result is worse.
- **Defer the whole question until the impedance reader is written.** Would
  leave `z0` real now and reshape it twice.
- **Defer Version 1.1 to a milestone of its own.** It is a v1 option-line
  feature and sits oddly in a v2 milestone. Rejected: it is the substantive
  content of the 2.1 document, the change is small and self-contained, and
  files using the syntax circulate — until now they failed with an unknown
  token.

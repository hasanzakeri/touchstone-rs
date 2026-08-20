# 0003 — Normalized in-memory data model

Status: Accepted (2026-08-20)

## Context

Touchstone files vary along several axes: frequency unit (Hz–GHz), value
format (`RI`/`MA`/`DB`), parameter type, spec version (1.0 vs 2.0/2.1),
and layout quirks (the transposed 2-port ordering, line wrapping).
Consumers should not have to care about any of that after parsing.

## Decision

Normalize on read, preserve enough metadata to round-trip on write:

- Frequencies are always Hz (`f64`), ascending.
- Values are always complex (`Complex64` / NumPy `complex128`),
  converted from `MA`/`DB` at parse time.
- Network data is one flat `Vec<Complex64>` of length F·N·N, row-major
  `(frequency, row, column)` — the natural NumPy `(F, N, N)` shape, handed
  over without copying.
- The original option line is kept verbatim in `Metadata`, alongside the
  parsed unit/format/parameter/resistance, so a write can reproduce the
  source style.
- Reference impedance is per-port `Vec<f64>` (real). Complex reference
  impedance is a known v2 concern and will be addressed with the v2
  milestone; changing `z0`'s type before 1.0 is acceptable.
- Noise parameters live in an `Option<NoiseData>` side structure
  (frequencies in Hz, NFmin in dB, Γopt complex, Rn normalized).

Target spec versions: Touchstone 1.0 and 2.0. The 2.1 deltas will be
scoped in a separate ADR when the v2 milestone starts.

## Consequences

- Python users get `net.f`, `net.s`, `net.z0` in one predictable shape and
  dtype regardless of source file style.
- Writing in a *different* format than the source is a deliberate feature
  (`format=` parameter), not an accident of normalization.
- `DB`/`MA`→complex conversion is lossy at the last ulp; round-trip tests
  must compare within a float tolerance, not bit-exactly.

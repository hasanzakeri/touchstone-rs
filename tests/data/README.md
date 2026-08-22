# Test data

Real-world and simulator-generated Touchstone files, with provenance noted
per file. See `docs/adr/0005-test-data-provenance-and-licensing.md` for the
policy: manufacturer-published files (Mini-Circuits, Murata, Skyworks,
Infineon, NXP, ...) are never committed here — their terms uniformly
prohibit redistribution. Files come from our own EDA-tool output,
hand-written fixtures, or permissively-licensed public data with
attribution instead.

Every file here is authored by the project's own user, so redistribution is
unrestricted. Files are committed **verbatim**: the `trailing-whitespace` and
`end-of-file-fixer` pre-commit hooks are configured to skip this directory,
because a fixture that has been tidied is no longer evidence of what a real
exporter emits.

## What a fixture has to be able to catch

Two properties decide whether a file can fail when the parser is wrong, and a
fixture lacking either is close to worthless here:

- **Non-reciprocal** — `S(i,j) != S(j,i)` throughout. A reciprocal device
  cannot detect a transposed read, because the bug is invisible in the data.
  This is why the multi-port files below were built asymmetric on purpose,
  and why the Touchstone spec's own 3-port example (a power divider) would be
  useless as a fixture.
- **Frequency-dependent** — values change from point to point. A
  frequency-flat device cannot detect a data-set boundary that slips by a
  whole point, since every point would then be wrong in the same way and
  still look self-consistent.

The 2-port `unilateral` family below is frequency-flat (it is purely
resistive) and so satisfies only the first. That was adequate for M1, where
one line was one point; the multi-port files added at M2 satisfy both.
`multiport::assert_not_reciprocal` in the integration tests asserts the first
property on the fixture itself, so a fixture that quietly loses it fails
rather than silently weakening the suite.

## The unilateral 2-port family

One simulated device: a unilateral 2-port (VCVS gain block, output at port
2), with a 100 Ω shunt resistor at port 1 and a 25 Ω shunt resistor at port
2. Frequency-independent, since the network is purely resistive.

| File | Source | Notes |
|---|---|---|
| `ads_unilateral_2port_ri.s2p` | **Derived** from `ads_unilateral_2port_ri_with_noise.s2p`: a byte-identical prefix, mechanically truncated before the `! Noise params` line (verified with `cmp`) | `# ghz S ri R 50`. The M1 fixture — real S-parameter data, without the section this version can't read yet. |
| `ads_unilateral_2port_ri_with_noise.s2p` | Keysight ADS export, 2026-08-20, by the project author | `# ghz S ri R 50`, 10 points. Carries a trailing noise-parameter section — **not parseable by M1**; banked for M3. |
| `ads_unilateral_2port_ma_with_noise.s2p` | Keysight ADS export, 2026-08-20 | `# ghz S ma R 50`. Same device in MA format. Carries a noise section, so **not parseable until M3**. |
| `ads_unilateral_2port_ma.s2p` | **Derived** from the file above, truncated before the `! Noise params` line and verified byte-identical as a prefix with `cmp` | `# ghz S ma R 50`. The MA half of the cross-format agreement test. |
| `ads_unilateral_2port_db_with_noise.s2p` | Keysight ADS export, 2026-08-20 | `# ghz S db R 50`. S12 is exactly zero, so its dB column is literally `-inf`. Carries a noise section, so **not parseable until M3**. |
| `ads_unilateral_2port_db.s2p` | **Derived** from the file above, same truncation and `cmp` check | `# ghz S db R 50`. The DB half of the cross-format agreement test, and the parser's only real `-inf` magnitude — the case that put the finiteness check *after* the conversion rather than on the token (see ADR 0006). |
| `qucs_unilateral_2port_ri_with_noise.s2p` | QUCS export, 2026-08-20, by the project author | `# HZ S RI R 50`. No `!` comment header at all; numbers in verbose `e+009`-style exponent form; a blank line separates the S-data block from a second, noise-shaped block with no `! Noise params` label — exercises the noise detector's frequency-only heuristic rather than a comment cue. Banked for M2 (layout quirks) and M3 (noise). |

## The asymmetric multi-port family (added at M2)

Keysight ADS exports, 2026-08-21, by the project author. Non-reciprocal and
frequency-dependent by construction — see the section above for why both
matter. All are `# ghz S <format> R 50`, 10 points from 1 to 10 GHz, with no
noise section (spec v1.1 §3 p10 permits noise only in 2-port files).

Each port count exists in RI, MA and DB, which is what makes cross-format
agreement assertable without a single hand-computed expectation: a wrong dB
base, a degrees/radians slip, or a sign error in the angle fails it at once.

| File | Notes |
|---|---|
| `ads_asymmetric_3port_ri.s3p`, `_ma.s3p`, `_db.s3p` | Data sets of 7, 6, 6 tokens. The gentlest wrapped layout, and the file that proves N ≥ 3 is plain **row-major** rather than carrying the 2-port's 21-before-12 swap. ADS's `!` header comment wraps across five lines and does not mirror the data's row structure — a good check that comment handling is independent of layout. |
| `ads_asymmetric_4port_ri.s4p`, `_ma.s4p`, `_db.s4p` | Data sets of 9, 8, 8, 8 tokens. The first line holds **nine** tokens, byte-identical in shape to a *complete* 2-port data set — the one genuinely ambiguous layout, resolved only by the set running on to 33 values. |
| `ads_asymmetric_4port_ri_scientific.s4p` | The same 4-port in exponent form (`4.87e-01`), so the notation is exercised inside a wrapped set rather than only on a single line. |
| `ads_asymmetric_16port_ri.s16p`, `_ma.s16p`, `_db.s16p` | 64 lines per data set, `9, 8, 8, 8, …`. The only files here where a single matrix *row* wraps: a 16-port row is 16 pairs, so it spans four lines and a data set contains lines that are neither its first nor a row start. 3- and 4-port layouts never produce that. ~110 KB each. |

## The 1-port family (added at M2)

Keysight ADS exports, 2026-08-21, by the project author. One device, 30
points from 0.05 to 1.5 GHz, re-exported with exactly one thing changed each
time so a failing test names its own cause.

| File | Notes |
|---|---|
| `ads_1port_ri_ghz.s1p` | The reference: `# ghz S ri R 50`, three values per line. |
| `ads_1port_ri_ghz_scientific.s1p` | Same data in exponent form (`5.000000000e-02`), which is what QUCS and several instruments emit. Slightly *more* precise than the decimal export, so the two agree to ~6e-10 rather than exactly. |
| `ads_1port_ma_ghz.s1p`, `ads_1port_ma_mhz.s1p`, `ads_1port_ma_hz.s1p` | One sweep written in three frequency units. Since normalization happens on read, all three must yield **bit-identical** arrays — `0.05 GHz`, `50 MHz` and `50000000 Hz` name one number. |
| `ads_1port_db_ghz.s1p` | The DB member of the cross-format check. |
| `ads_1port_db_ghz_low_precision.s1p` | The same export rounded to four significant figures instead of nine. Rounding in the source is not an error to reject; the test asserts it parses and lands within its own rounding of the full-precision file. |

## Local-only harness

Manufacturer files (Murata, Skyworks, Infineon, Philips/NXP, ...) are
useful for real-world coverage but cannot be committed. They're exercised
through a gitignored local harness instead — see `local/README.md` (not
tracked; ask a prior session's BLUEPRINT.md notes if it's missing).

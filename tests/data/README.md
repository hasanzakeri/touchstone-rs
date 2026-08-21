# Test data

Real-world and simulator-generated Touchstone files, with provenance noted
per file. See `docs/adr/0005-test-data-provenance-and-licensing.md` for the
policy: manufacturer-published files (Mini-Circuits, Murata, Skyworks,
Infineon, NXP, ...) are never committed here — their terms uniformly
prohibit redistribution. Files come from our own EDA-tool output,
hand-written fixtures, or permissively-licensed public data with
attribution instead.

## Provenance

All files below describe the same simulated device: a unilateral 2-port
(VCVS gain block, output at port 2), with a 100 Ω shunt resistor at port 1
and a 25 Ω shunt resistor at port 2. Frequency-independent, since the
network is purely resistive.

| File | Source | Notes |
|---|---|---|
| `ads_unilateral_2port_ri.s2p` | **Derived** from `ads_unilateral_2port_ri_with_noise.s2p`: a byte-identical prefix, mechanically truncated before the `! Noise params` line (verified with `cmp`) | `# ghz S ri R 50`. The M1 fixture — real S-parameter data, without the section this version can't read yet. |
| `ads_unilateral_2port_ri_with_noise.s2p` | Keysight ADS export, 2026-08-20, by the project author | `# ghz S ri R 50`, 10 points. Carries a trailing noise-parameter section — **not parseable by M1**; banked for M3. |
| `ads_unilateral_2port_ma_with_noise.s2p` | Keysight ADS export, 2026-08-20 | `# ghz S ma R 50`. Same device in MA format; banked for M2 (and M3, for its noise section). |
| `ads_unilateral_2port_db_with_noise.s2p` | Keysight ADS export, 2026-08-20 | `# ghz S db R 50`. S12 is exactly zero, so its dB column is literally `-inf` — a real edge case for M2's DB parsing (`f64::parse` accepts `"-inf"` natively). Banked for M2/M3. |
| `qucs_unilateral_2port_ri_with_noise.s2p` | QUCS export, 2026-08-20, by the project author | `# HZ S RI R 50`. No `!` comment header at all; numbers in verbose `e+009`-style exponent form; a blank line separates the S-data block from a second, noise-shaped block with no `! Noise params` label — exercises the noise detector's frequency-only heuristic rather than a comment cue. Banked for M2 (layout quirks) and M3 (noise). |

All five files are simulator output authored by the project's own user —
not third-party data — so redistribution is unrestricted.

## Local-only harness

Manufacturer files (Murata, Skyworks, Infineon, Philips/NXP, ...) are
useful for real-world coverage but cannot be committed. They're exercised
through a gitignored local harness instead — see `local/README.md` (not
tracked; ask a prior session's BLUEPRINT.md notes if it's missing).

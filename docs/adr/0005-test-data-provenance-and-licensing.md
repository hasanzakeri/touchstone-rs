# 0005 — Test data provenance and licensing

Status: Accepted (2026-08-20)

## Context

The working assumption going into M1 (recorded in the untracked
`BLUEPRINT.md`) was that "manufacturer sites usually allow use but verify."
Verifying it turned the assumption around: every vendor whose terms could be
retrieved explicitly prohibits redistributing S-parameter data files, not
merely commercial use of them.

| Vendor | Verdict | Clause |
|---|---|---|
| Murata | No | S-parameter download terms, clause 5: *"You shall not redistribute or reproduce the DATA without prior consent of Murata."* Clause 1 independently limits use to characteristic confirmation and circuit simulation. |
| Skyworks | No | Site terms: informational/personal/non-commercial only, *"you will not copy, transfer or transmit such information to another person or entity."* |
| Infineon | No | Usage terms grant distribution *"solely for informational and non-commercial or personal use"* — narrower than a permissive open-source sublicense. |
| Mini-Circuits | No | Terms forbid distributing "Information" (defined to include specs and characterizations) and forbid removing copyright notices, so a vendor header cannot even be stripped to launder it. |
| NXP / Philips | No (unverified) | nxp.com blocks automated retrieval; search-surfaced terms match the pattern above. The provenance of legacy 1992-era `.S2P` transistor files is genuinely murky and was not resolved. |
| Coilcraft, Analog Devices | Unverified | Both sites return 403 to automated requests. Their file-level readmes carry only warranty disclaimers — silence is not permission. |

Peer projects ship such files anyway, but that is not precedent: `scikit-rf`
(BSD-3) ships Mini-Circuits `LFCN-2352+` `.s2p` data with no NOTICE file, no
data-specific README, and no issue ever raised about it; the `touchstone`
Rust crate copied the same files; a Perl Touchstone module ships nine Murata
files *and bundles Murata's own terms as a license file* — a document whose
own clause 5 forbids exactly what shipping it does. None of these represent
a considered legal position; they represent nobody having checked.

The legitimate counter-example is what LibreVNA and nanovna-saver do: ship
files *named* after commercial parts (`Mini-circuits_VAT-10+.s2p`,
`Murata_RF1419D.s2p`) that are, on inspection, the project's own VNA
measurements — bare data, no vendor header. Measuring or simulating a device
yourself produces your own data, restricted only by whatever you choose.

## Decision

Committed test fixtures come from, in order of preference:

1. **Our own EDA-tool output.** Simulation or measurement output is the
   author's own work; a tool's EULA restricts the software, not what it
   produces (the FSF states this position directly for GPL-licensed
   programs, and it is the cleanest ground available here since both ADI's
   and Keysight's terms pages return 403 to automated retrieval). This is
   the primary source going forward — the project author has access to
   LTspice, Keysight ADS, and HFSS, and can produce a fixture for a specific
   milestone need on request.
2. **Hand-written synthetic fixtures**, inline in test source, for pinning
   exact grammar behavior — most of the M1 test matrix.
3. **Permissively-licensed public data with attribution** — e.g. a
   US-federal-work dataset (17 U.S.C. §105) or a CC-BY-licensed release —
   segregated and credited in this directory's provenance table.

**Never**: a manufacturer-published file, and never by stripping a vendor
header to obscure its origin. A provenance row (source, date, license basis)
is required for every file in this directory. Vendor files remain useful for
real-world coverage, but only through the local, gitignored harness
described in the untracked `BLUEPRINT.md` — never committed.

M1's fixture (`tests/data/ads_unilateral_2port_ri.s2p`) is the first
instance of source (1): a unilateral 2-port simulated in Keysight ADS,
exported in RI, MA, and DB, plus a QUCS export of the same device — all
committed, since nothing restricts them, and several banked ahead of the
milestone that consumes them (see `tests/data/README.md`).

## Consequences

- Fixtures are reproducible and can be built to exercise a specific spec
  corner on demand, rather than hoping a found file happens to.
- This project does not inherit `scikit-rf`'s BSD license by implication
  for any vendor-originated file, because no vendor-originated file is
  committed here.
- The claim "this parser reads what a real instrument or real EDA tool
  emits" rests on the local harness (manufacturer files, present but
  uncommitted) and on the committed ADS/HFSS/LTspice exports — not on
  redistributed vendor data. That is a real trade-off worth naming: a
  cleanly-licensed real-world dataset, if one turns up, would still add
  value the local harness alone does not (a permanent, CI-visible fixture).

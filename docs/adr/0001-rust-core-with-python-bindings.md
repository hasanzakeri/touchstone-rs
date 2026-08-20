# 0001 — Rust core with Python bindings, positioned Python-first

Status: Accepted (2026-08-20)

## Context

Touchstone `.sNp` files are the de-facto interchange format for RF network
parameter data. In Python, scikit-rf is the standard tool and its reader is
pure Python — noticeably slow on large multiport sweeps (many ports × many
frequency points) and on directories of files.

On the Rust side, a `touchstone` crate already exists on crates.io
(actively maintained, v1.0 + v2.x parsing, RI/MA/DB, N-port, writer,
line-numbered errors). It has no Python bindings and does not parse noise
parameters. Competing with it head-on as "the Rust Touchstone crate" would
duplicate most of its ground.

## Decision

Build a parser/writer with a pure-Rust core (`touchstone-core`) and PyO3
bindings, and position the project **Python-first**: the product is the
PyPI package (`touchstone-rs`), pitched as a fast I/O layer that
*complements* scikit-rf (interop via `to_skrf()`) rather than replacing it.
The Rust crate is published to crates.io as a secondary audience.

Scope is deliberately I/O-only: parse and write Touchstone 1.0/2.0, expose
NumPy arrays. No network math, plotting, or cascading — scikit-rf and
others already do that well.

Bindings use PyO3 with abi3 (`abi3-py310`) wheels: one build per platform
covers CPython ≥ 3.10. Parsed vectors are moved into NumPy arrays without
copying (`Vec` ownership handoff via rust-numpy).

## Consequences

- Functional differentiators over existing tools: speed (to be proven by
  benchmarks, never claimed without numbers), noise-parameter support,
  and later parallel batch reading.
- abi3 excludes free-threaded CPython builds (3.13t/3.14t); those would
  need separate wheels if ever demanded.
- The performance claim must be validated against scikit-rf with published
  benchmark numbers before it appears in any README pitch.

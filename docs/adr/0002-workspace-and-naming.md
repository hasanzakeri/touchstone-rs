# 0002 — Workspace layout, naming, and toolchain

Status: Accepted (2026-08-20)

## Context

The project ships both a Rust library and a Python package from one
repository. The crate name `touchstone` is taken on crates.io by an
unrelated, actively maintained project; PyPI has no `touchstone-rs`.

## Decision

- Cargo workspace with two crates:
  - `crates/touchstone-core` — pure Rust parser/writer, no Python
    dependencies, publishable to crates.io.
  - `crates/touchstone-py` — thin PyO3 layer (`publish = false`),
    compiled as the extension module `touchstone_rs._core` and shipped
    only inside PyPI wheels.
- Python package name: `touchstone-rs` on PyPI, importable as
  `touchstone_rs`, with a pure-Python facade in `python/touchstone_rs/`
  (re-exports, type stubs, later the scikit-rf interop helpers).
- Build tooling: maturin (≥ 1.14) as the build backend; **uv manages
  everything Python-side** (venv, dev dependencies, running tests). No
  bare `python`/`pip` invocations — this machine also has conda installed
  and PATH-based resolution is not trusted. The interpreter is pinned via
  `.python-version`.
- Edition 2024, `rust-version = 1.85` (the maximum MSRV across the
  dependency set; edition 2024 itself requires 1.85).
- Dual MIT/Apache-2.0 license, the Rust ecosystem convention.

## Consequences

- Rust users can depend on `touchstone-core` without any Python machinery;
  Python packaging never leaks into the core crate.
- Two crates mean slightly more manifest boilerplate, shared via
  `[workspace.package]` / `[workspace.dependencies]`.
- The core crate versions in lockstep with the Python package for now
  (single `0.x` line); revisit if their release cadences diverge.

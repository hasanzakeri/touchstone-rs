# Architecture Decision Records

Decisions that shape this project, recorded at the point they were made.

## Conventions

- Format: lightweight [MADR](https://adr.github.io/madr/)-style — Context / Decision / Consequences.
- ADRs are point-in-time records. They are never edited to change a decision;
  a reversal is a new ADR that marks the old one `Superseded by NNNN`.
  Status is one of: `Proposed`, `Accepted`, `Superseded by NNNN`.
- **Copyright**: the Touchstone specifications are published by the IBIS Open
  Forum. ADRs cite the spec by version and section number and paraphrase —
  spec text is never reproduced verbatim in this repository.

## Index

| ADR | Title | Status |
|---|---|---|
| [0001](0001-rust-core-with-python-bindings.md) | Rust core with Python bindings, positioned Python-first | Accepted |
| [0002](0002-workspace-and-naming.md) | Workspace layout, naming, and toolchain | Accepted |
| [0003](0003-normalized-data-model.md) | Normalized in-memory data model | Accepted |
| [0004](0004-strict-parsing-with-explicit-tolerances.md) | Strict parsing, with explicit and named tolerances | Accepted |
| [0005](0005-test-data-provenance-and-licensing.md) | Test data provenance and licensing | Accepted |
| [0006](0006-data-set-accumulation-and-line-wrapping.md) | Data-set accumulation and line-wrapping tolerance | Accepted |

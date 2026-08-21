# touchstone-rs

[![CI](https://github.com/hasanzakeri/touchstone-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/hasanzakeri/touchstone-rs/actions/workflows/ci.yml)

Fast Touchstone (`.sNp`) file I/O for Python, backed by a Rust parser.

Touchstone files are the standard interchange format for RF network
parameter measurements (S-parameters and friends). This library reads and
writes them — versions 1.0 and 2.0, all value formats, any port count,
including the noise-parameter sections most tools skip — and hands the data
to Python as NumPy arrays without copying.

It is an I/O layer, not an analysis tool: read fast here, analyze in
[scikit-rf](https://scikit-rf.org/). No network math, no plotting.

> **Status: early development.** `ts.read()` works today for Touchstone 1.0,
> 2-port files in `RI` format. Other formats, port counts, and noise
> sections are next. Nothing is published to PyPI yet.

## API

```python
import touchstone_rs as ts

net = ts.read("filter.s2p")   # v1.0, 2-port, RI, today
net.f        # np.float64, shape (F,)      — frequencies, always Hz
net.s        # np.complex128, shape (F, N, N)
net.z0       # np.float64, shape (N,)      — per-port reference impedance
net.noise    # NoiseData | None            — noise parameters, if present
```

Planned:

```python
ts.write("out.s2p", net, format="MA")      # round-trip, any format
```

## Roadmap

| Milestone | Status |
|---|---|
| Project scaffold: workspace, bindings, CI | done |
| Touchstone 1.0, 2-port, RI format | done |
| All formats (RI/MA/DB), all port counts | — |
| Noise parameters | — |
| Touchstone 2.0 | — |
| Writer + round-trip property tests | — |
| Fuzzing, strict/lenient modes | — |
| Benchmarks vs scikit-rf (published in this README) | — |
| scikit-rf interop (`to_skrf()`) | — |
| Parallel batch reading (`read_dir`) | — |

Performance claims will appear here only as measured benchmark numbers.

## Design

- **Zero-copy into NumPy.** The Rust parser builds each array once and
  hands ownership to NumPy — no serialization layer, no per-element
  conversion, no copy.
- **Normalize on read, round-trip on write.** Frequencies are always Hz,
  values always `complex128`, regardless of the file's unit and format;
  the original option line is preserved so writes can reproduce the
  source style.
- **One wheel per platform.** abi3 (`cp310-abi3`) wheels cover CPython
  3.10 through 3.14+ from a single build.
- **Errors with line numbers.** Touchstone files in the wild are messy, and
  the parser is designed around that fact — strict by default today, with a
  lenient mode planned for the files that need it.

## Development

Rust ≥ 1.85 and [uv](https://docs.astral.sh/uv/) are required.

```sh
uv sync                    # build the extension into .venv
uv run pytest              # python tests
cargo test --workspace     # rust tests
uv run pre-commit install  # commit/push hooks (fmt, lint, tests)
```

Design decisions are recorded in [docs/adr/](docs/adr/).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
at your option.

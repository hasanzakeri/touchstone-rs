# touchstone-rs

[![CI](https://github.com/hasanzakeri/touchstone-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/hasanzakeri/touchstone-rs/actions/workflows/ci.yml)

Fast Touchstone (`.sNp`) file I/O for Python, backed by a Rust parser.

Touchstone files are the standard interchange format for RF network
parameter measurements (S-parameters and friends). This library reads and
writes them — versions 1.0 through 2.1, all value formats, any port count,
including the noise-parameter sections most tools skip and the triangular
`[Matrix Format]` layouts most readers do not implement — and hands the data
to Python as NumPy arrays without copying.

It is an I/O layer, not an analysis tool: read fast here, analyze in
[scikit-rf](https://scikit-rf.org/). No network math, no plotting.

> **Status: early development.** `ts.read()` works today for Touchstone 1.0,
> 1.1, 2.0 and 2.1 S-parameter files in every value format (`RI`/`MA`/`DB`),
> at any port count, including noise parameters and `[Matrix Format]` Lower
> and Upper. The other parameter types (`Y`/`Z`/`G`/`H`) and mixed-mode data
> are not read. Nothing is published to PyPI yet.

## API

```python
import touchstone_rs as ts

net = ts.read("coupler.s4p")  # or "device.ts" — the version is read from the file
net.f        # np.float64, shape (F,)      — frequencies, always Hz
net.s        # np.complex128, shape (F, N, N)
net.z0       # np.complex128, shape (F, N) — per-port reference impedance
net.noise    # NoiseData | None            — noise parameters, if present

amp = ts.read("lna.s2p")
amp.noise.f          # np.float64, shape (M,)   — Hz; M need not equal F
amp.noise.nfmin_db   # np.float64, shape (M,)   — minimum noise figure, dB
amp.noise.gamma_opt  # np.complex128, shape (M,)
amp.noise.rn         # np.float64, shape (M,)   — exactly as the file wrote it
amp.noise.rn_ohms    # np.float64, shape (M,)   — in ohms, whatever wrote it
```

Two of those need a word.

**`rn` is not the same quantity in every version.** A Touchstone 1.0 or 1.1
file normalizes the effective noise resistance to the option line's reference
resistance; a 2.0 or 2.1 file writes ohms. `rn` is what the file says and
`rn_ohms` is what it means — reach for the second unless you are writing the
file back out. See [ADR 0010](docs/adr/0010-effective-noise-resistance-across-versions.md).

**`z0` is per-frequency and complex** although nothing a conforming file can
say makes it either: the format's reference impedance is one real value per
port for the whole sweep, so every row comes back identical with a zero
imaginary part. The shape is what a field solver's own port impedance needs,
fixed now so it will not change later. See
[ADR 0009](docs/adr/0009-reference-impedance-and-the-1-1-option-line.md).

Version 1.0 noise sections are found the way the format demands — by the
frequency stepping back into the sweep already covered — so files that carry
no comment to announce one still read
([ADR 0007](docs/adr/0007-noise-parameter-section.md)). A 2.0 file announces
its own with `[Noise Data]` and needs no inference at all.

Planned:

```python
ts.write("out.s2p", net, format="MA")      # round-trip, any format
```

## Roadmap

| Milestone | Status |
|---|---|
| Project scaffold: workspace, bindings, CI | done |
| Touchstone 1.0, 2-port, RI format | done |
| All formats (RI/MA/DB), all port counts | done |
| Noise parameters | done |
| Touchstone 2.0 / 2.1 | done |
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

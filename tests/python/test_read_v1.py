"""Reading Touchstone v1 files through the Python bindings.

These tests exercise the *binding*, not the grammar: the Rust integration
suite in crates/touchstone-core/tests/parse_v1.rs owns the spec matrix.
What matters here is that values survive the crossing into NumPy with the
right dtype, shape, and orientation, and that errors arrive as
TouchstoneError with their line numbers intact.
"""

from pathlib import Path

import numpy as np
import pytest
import touchstone_rs as ts

# A unilateral 2-port: S21 is large, S12 nearly zero, and S11 != S22. Written
# in file order, which spec v1.1 §3 defines as S11 S21 S12 S22.
UNILATERAL = "# MHZ S RI R 50\n1.0  0.1 0.2  9.0 9.1  0.01 0.02  0.3 0.4\n"

# Committed fixtures, for the tests that need a real tool's output rather
# than a hand-written string. See tests/data/README.md for provenance.
DATA = Path(__file__).resolve().parents[1] / "data"


def write(tmp_path: Path, text: str, name: str = "device.s2p") -> Path:
    path = tmp_path / name
    path.write_text(text)
    return path


def test_read_returns_arrays_in_the_documented_shapes(tmp_path: Path) -> None:
    net = ts.read(write(tmp_path, UNILATERAL))

    assert net.nports == 2
    assert net.noise is None
    assert net.f.dtype == np.float64
    assert net.f.shape == (1,)
    assert net.s.dtype == np.complex128
    assert net.s.shape == (1, 2, 2)
    assert net.z0.dtype == np.float64
    np.testing.assert_array_equal(net.z0, [50.0, 50.0])
    assert repr(net) == "<Network 2-port, 1 frequency points>"


def test_frequencies_are_normalized_to_hz(tmp_path: Path) -> None:
    # The file says MHz; the array must say Hz. This is the visible proof
    # that normalization happens on read.
    net = ts.read(write(tmp_path, UNILATERAL))
    np.testing.assert_array_equal(net.f, [1e6])


def test_s_matrix_is_not_transposed(tmp_path: Path) -> None:
    # The ordering guard at the binding boundary: a 2-port file lists S21
    # before S12, so a swap anywhere in the stack shows up here. Only an
    # asymmetric device can reveal it -- a reciprocal one has S21 == S12.
    net = ts.read(write(tmp_path, UNILATERAL))

    assert net.s[0, 0, 0] == 0.1 + 0.2j, "S11"
    assert net.s[0, 1, 0] == 9.0 + 9.1j, "S21, the second pair in the file"
    assert net.s[0, 0, 1] == 0.01 + 0.02j, "S12, the third pair in the file"
    assert net.s[0, 1, 1] == 0.3 + 0.4j, "S22"


def test_multiple_points_keep_their_order(tmp_path: Path) -> None:
    text = "# GHZ S RI R 75\n" + "".join(
        f"{i}.0 {i}.1 0 0 0 0 0 0 0\n" for i in range(1, 6)
    )
    net = ts.read(write(tmp_path, text))

    assert net.f.shape == (5,)
    assert net.s.shape == (5, 2, 2)
    np.testing.assert_array_equal(net.f, [1e9, 2e9, 3e9, 4e9, 5e9])
    assert net.s[4, 0, 0] == 5.1 + 0j
    np.testing.assert_array_equal(net.z0, [75.0, 75.0])


def test_uppercase_extension_is_recognized(tmp_path: Path) -> None:
    # Real manufacturer files ship as both .s2p and .S2P.
    net = ts.read(write(tmp_path, UNILATERAL, name="DEVICE.S2P"))
    assert net.nports == 2


def test_crlf_file_reads_the_same_as_lf(tmp_path: Path) -> None:
    path = tmp_path / "crlf.s2p"
    path.write_bytes(UNILATERAL.replace("\n", "\r\n").encode())
    net = ts.read(path)
    assert net.s[0, 1, 0] == 9.0 + 9.1j


def test_non_ascii_in_a_comment_does_not_prevent_reading(tmp_path: Path) -> None:
    # Spec v1.1 §2 allows only ASCII, but real exports carry the odd high
    # byte in a comment. Latin-1 bytes are not valid UTF-8, so this also
    # covers the lossy decode.
    path = tmp_path / "degrees.s2p"
    path.write_bytes(b"! Temperature = +25 \xb0C\n" + UNILATERAL.encode())
    net = ts.read(path)
    assert net.s[0, 0, 0] == 0.1 + 0.2j


def test_parse_errors_carry_their_line_number(tmp_path: Path) -> None:
    text = "# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0 0\n2.0 0 0 nonsense 0 0 0 0 0\n"
    with pytest.raises(ts.TouchstoneError, match=r"^line 3: invalid number: nonsense$"):
        ts.read(write(tmp_path, text))


def test_an_unsupported_parameter_says_what_is_supported(tmp_path: Path) -> None:
    with pytest.raises(ts.TouchstoneError, match="only s-parameters are supported"):
        ts.read(write(tmp_path, "# GHZ Y RI R 50\n1.0 0 0 0 0 0 0 0 0\n"))


def test_a_real_multiport_export_crosses_into_numpy_with_the_right_shape() -> None:
    # A real 16-port ADS export: 64 lines per data set, and a single matrix
    # row spans four of them. The array shape is the proof that the port
    # count survived the crossing into NumPy, not just the parse.
    net = ts.read(DATA / "ads_asymmetric_16port_ri.s16p")

    assert net.nports == 16
    assert net.s.shape == (10, 16, 16)
    assert net.z0.shape == (16,)
    assert net.s.dtype == np.complex128
    assert repr(net) == "<Network 16-port, 10 frequency points>"

    # Non-reciprocal by construction, so a transpose anywhere in the stack is
    # visible here. S(1,16) closes row 1 on the data set's fourth line;
    # S(16,1) opens row 16 on its 61st.
    assert net.s[0, 0, 15] == 0.0581396322 - 0.0160554818j
    assert net.s[0, 15, 0] == 0.24924568 - 0.0646806443j
    assert net.s[0, 0, 15] != net.s[0, 15, 0], "the fixture must not be reciprocal"


def test_a_real_multiport_export_agrees_across_formats() -> None:
    # The same device written three ways. Nothing here is hand-computed, so a
    # conversion bug cannot be hidden by a matching expectation.
    ri = ts.read(DATA / "ads_asymmetric_4port_ri.s4p")
    for name in ("ma", "db"):
        other = ts.read(DATA / f"ads_asymmetric_4port_{name}.s4p")
        np.testing.assert_array_equal(other.f, ri.f, err_msg=name)
        # 1e-7 is the files' nine significant figures talking, not the
        # conversion arithmetic.
        np.testing.assert_allclose(other.s, ri.s, atol=1e-7, err_msg=name)


def test_a_one_port_file_reads_as_a_one_by_one_matrix(tmp_path: Path) -> None:
    net = ts.read(write(tmp_path, "# GHZ S MA R 50\n1.0 0.5 90\n", name="load.s1p"))
    assert net.nports == 1
    assert net.s.shape == (1, 1, 1)
    assert net.s[0, 0, 0] == pytest.approx(0.5j)


def test_db_and_ma_files_reach_numpy_as_complex_values(tmp_path: Path) -> None:
    # The formats differ only on disk; what arrives is always complex128.
    ma = ts.read(write(tmp_path, "# GHZ S MA R 50\n1.0  2 90  1 180  0.5 0  1 -90\n"))
    db = ts.read(write(tmp_path, "# GHZ S DB R 50\n1.0  20 0  0 0  -20 0  -20 180\n"))

    assert ma.s.dtype == np.complex128
    assert db.s.dtype == np.complex128
    assert ma.s[0, 0, 0] == pytest.approx(2j)
    assert db.s[0, 0, 0] == pytest.approx(10 + 0j)
    assert db.s[0, 0, 1] == pytest.approx(0.1 + 0j), "S12, the third pair"


def test_a_noise_section_arrives_as_four_parallel_arrays(tmp_path: Path) -> None:
    text = (
        "# GHZ S RI R 50\n"
        "2.0 0 0 0 0 0 0 0 0\n"
        "22.0 0 0 0 0 0 0 0 0\n"
        "! NOISE PARAMETERS\n"
        "4.0 0.75 0.62 61.0 0.35\n"
        "18.0 2.55 0.44 -37.0 0.42\n"
    )
    noise = ts.read(write(tmp_path, text)).noise

    assert noise is not None
    assert noise.f.dtype == np.float64
    assert noise.nfmin_db.dtype == np.float64
    assert noise.gamma_opt.dtype == np.complex128
    assert noise.rn.dtype == np.float64
    for array in (noise.f, noise.nfmin_db, noise.gamma_opt, noise.rn):
        assert array.shape == (2,)

    # Hz here as everywhere, and on a grid of its own: these two points are
    # not the frequencies the S-data was sampled at.
    np.testing.assert_array_equal(noise.f, [4e9, 18e9])
    np.testing.assert_array_equal(noise.nfmin_db, [0.75, 2.55])
    # Normalized to the option line's R, exactly as written -- not ohms.
    np.testing.assert_array_equal(noise.rn, [0.35, 0.42])
    # 0.62 <61 deg: magnitude-and-angle even in an RI file (spec v1.1 3 p10).
    assert noise.gamma_opt[0] == pytest.approx(0.3005819646 + 0.5422642184j)


def test_a_real_export_with_noise_reads_both_of_its_sections() -> None:
    net = ts.read(DATA / "ads_varying_noise_2port_ri.s2p")

    assert net.f.shape == (10,)
    assert net.s.shape == (10, 2, 2)
    noise = net.noise
    assert noise is not None
    assert noise.f.shape == (10,)
    assert noise.gamma_opt[0] == pytest.approx(0.1739795244 + 0.8109449251j)


def test_a_file_without_noise_reports_none() -> None:
    # The truncated sibling of the file above: same device, section removed.
    assert ts.read(DATA / "ads_unilateral_2port_ri.s2p").noise is None


def test_a_malformed_noise_line_names_the_section_and_its_line(tmp_path: Path) -> None:
    text = (
        "# GHZ S RI R 50\n"
        "2.0 0 0 0 0 0 0 0 0\n"
        "22.0 0 0 0 0 0 0 0 0\n"
        "4.0 0.75 0.62 61.0 0.35\n"
        "18.0 2.55 0.44 -37.0\n"
    )
    with pytest.raises(ts.TouchstoneError, match="line 5: .*noise parameter line"):
        ts.read(write(tmp_path, text))

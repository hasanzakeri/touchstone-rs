"""Reading Touchstone v1, 2-port, RI files through the Python bindings.

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


def test_an_unsupported_format_says_what_is_supported(tmp_path: Path) -> None:
    with pytest.raises(ts.TouchstoneError, match="only ri is supported"):
        ts.read(write(tmp_path, "# GHZ S MA R 50\n1.0 0 0 0 0 0 0 0 0\n"))


def test_a_noise_section_is_reported_as_such(tmp_path: Path) -> None:
    text = (
        "# GHZ S RI R 50\n"
        "2.0 0 0 0 0 0 0 0 0\n"
        "22.0 0 0 0 0 0 0 0 0\n"
        "! NOISE PARAMETERS\n"
        "4.0 0.7 0.64 69.0 0.38\n"
    )
    with pytest.raises(ts.TouchstoneError, match="noise parameter section"):
        ts.read(write(tmp_path, text))

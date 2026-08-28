"""Reading Touchstone 2.0 files through the Python bindings.

These test the *binding*, not the grammar: the Rust integration suite in
crates/touchstone-core/tests/parse_v2.rs owns the spec matrix. What matters
here is that v2 files reach NumPy with the right dtype, shape and orientation
through the same ts.read() as v1, and that the two things a v2 file can say
which a v1 file cannot -- a per-port reference, and an effective noise
resistance in ohms -- arrive intact.
"""

from collections.abc import Callable
from pathlib import Path

import numpy as np
import pytest
import touchstone_rs as ts

DATA = Path(__file__).resolve().parents[1] / "data"

# A unilateral 2-port: S21 is large, S12 nearly zero, S11 != S22. Written in
# 21_12 order, which is what Touchstone 1.0 uses and says nowhere.
UNILATERAL_21_12 = """\
[Version] 2.0
# MHZ S RI R 50
[Number of Ports] 2
[Two-Port Data Order] 21_12
[Number of Frequencies] 1
[Network Data]
1.0  0.1 0.2  9.0 9.1  0.01 0.02  0.3 0.4
[End]
"""

# The same device in the other order: only the two middle pairs move.
UNILATERAL_12_21 = UNILATERAL_21_12.replace(
    "[Two-Port Data Order] 21_12", "[Two-Port Data Order] 12_21"
).replace("9.0 9.1  0.01 0.02", "0.01 0.02  9.0 9.1")


def write(tmp_path: Path, text: str, name: str = "device.ts") -> Path:
    path = tmp_path / name
    path.write_text(text)
    return path


def test_read_returns_the_documented_shapes(tmp_path: Path) -> None:
    net = ts.read(write(tmp_path, UNILATERAL_21_12))

    assert net.nports == 2
    assert net.noise is None
    assert net.f.dtype == np.float64
    assert net.f.shape == (1,)
    assert net.s.dtype == np.complex128
    assert net.s.shape == (1, 2, 2)
    assert net.z0.dtype == np.complex128
    assert net.z0.shape == (1, 2)
    assert repr(net) == "<Network 2-port, 1 frequency points>"


def test_the_two_data_orders_cross_into_numpy_identically(tmp_path: Path) -> None:
    """The transpose guard, on the Python side of the boundary.

    A mirrored read would put 9+9.1j at [0, 0, 1] instead of [0, 1, 0]. The
    device is unilateral so the two are three orders of magnitude apart.
    """
    a = ts.read(write(tmp_path, UNILATERAL_21_12, "a.ts"))
    b = ts.read(write(tmp_path, UNILATERAL_12_21, "b.ts"))

    np.testing.assert_array_equal(a.s, b.s)
    assert a.s[0, 1, 0] == 9.0 + 9.1j, "S21"
    assert a.s[0, 0, 1] == 0.01 + 0.02j, "S12"


def test_a_ts_extension_takes_its_port_count_from_the_keyword(tmp_path: Path) -> None:
    """`.ts` encodes no port count, unlike `.sNp`, so the keyword is the only
    source there is."""
    net = ts.read(write(tmp_path, UNILATERAL_21_12, "no_port_count_here.ts"))
    assert net.nports == 2


def test_per_port_reference_reaches_z0(tmp_path: Path) -> None:
    text = UNILATERAL_21_12.replace(
        "[Number of Frequencies] 1", "[Number of Frequencies] 1\n[Reference] 50 75"
    )
    net = ts.read(write(tmp_path, text))

    assert net.z0.shape == (1, 2)
    np.testing.assert_array_equal(net.z0, [[50 + 0j, 75 + 0j]])


def test_a_real_v2_export_crosses_into_numpy(tmp_path: Path) -> None:
    net = ts.read(DATA / "hfss_v2_symmetric_4port_ri.ts")

    assert net.nports == 4
    assert net.s.shape == (51, 4, 4)
    assert net.z0.shape == (51, 4)
    # A reciprocal structure, so the matrix must come back symmetric across
    # the whole sweep. The tolerance is the file's, not the parser's.
    for i in range(4):
        for j in range(4):
            np.testing.assert_allclose(net.s[:, i, j], net.s[:, j, i], atol=1e-12)


def test_the_three_formats_of_one_export_agree() -> None:
    ri = ts.read(DATA / "hfss_v2_symmetric_4port_ri.ts")
    ma = ts.read(DATA / "hfss_v2_symmetric_4port_ma.ts")
    db = ts.read(DATA / "hfss_v2_symmetric_4port_db.ts")

    np.testing.assert_allclose(ri.s, ma.s, atol=1e-9)
    np.testing.assert_allclose(ri.s, db.s, atol=1e-9)


def test_noise_arrives_with_both_readings_of_rn() -> None:
    """rn is what the file wrote; rn_ohms is that in ohms whatever wrote it.

    A 1.x file normalizes this column to the option line's reference and a 2.x
    file writes ohms, so the same device read from the two versions gives
    different `rn` and identical `rn_ohms`. Missing that is a factor-of-fifty
    error with nothing to make it visible.
    """
    v2 = ts.read(DATA / "ads_v2_varying_noise_2port_ri_21_12.ts")
    v1 = ts.read(DATA / "ads_varying_noise_2port_ri.s2p")

    assert v2.noise is not None and v1.noise is not None
    assert v2.noise.rn.dtype == np.float64
    assert v2.noise.gamma_opt.dtype == np.complex128
    assert v2.noise.f.shape == (10,)

    assert not bool((v1.noise.rn == v2.noise.rn).all()), (
        "1.x normalizes, 2.x writes ohms"
    )
    np.testing.assert_array_equal(v1.noise.rn_ohms, v2.noise.rn_ohms)
    np.testing.assert_array_equal(v1.noise.gamma_opt, v2.noise.gamma_opt)


def test_the_noise_grid_need_not_match_the_frequency_grid() -> None:
    """Nothing may assume the two arrays share a length."""
    net = ts.read(DATA / "ads_varying_noise_2port_ri_coarse_grid.s2p")
    assert net.noise is not None
    assert net.noise.f.size != net.f.size


# Each entry removes or corrupts one thing a v2 file cannot do without, and
# names the part of the message that must mention it.
MANGLED: list[tuple[Callable[[str], str], str]] = [
    (lambda t: t.replace("[Version] 2.0", "[Version] 3.0"), "unsupported version"),
    (lambda t: t.replace("[Number of Ports] 2\n", ""), "Number of Ports"),
    (lambda t: t.replace("[Two-Port Data Order] 21_12\n", ""), "Two-Port Data Order"),
    (lambda t: t.replace("[Network Data]", "[Netwrok Data]"), "unknown keyword"),
]


@pytest.mark.parametrize(("mangle", "match"), MANGLED)
def test_malformed_v2_files_raise_touchstone_error(
    tmp_path: Path, mangle: Callable[[str], str], match: str
) -> None:
    text = mangle(UNILATERAL_21_12)
    with pytest.raises(ts.TouchstoneError, match=match):
        ts.read(write(tmp_path, text))


def test_errors_keep_their_line_numbers(tmp_path: Path) -> None:
    text = UNILATERAL_21_12.replace("1.0  0.1 0.2", "1.0  0.1 oops")
    with pytest.raises(ts.TouchstoneError, match=r"^line 7:"):
        ts.read(write(tmp_path, text))

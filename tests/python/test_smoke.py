"""Smoke tests: module import, error type, array construction and validation.

Reading files is covered in test_read_v1.py.
"""

import numpy as np
import pytest
import touchstone_rs as ts


def test_version() -> None:
    assert ts.__version__ == "0.1.0"


def test_error_is_valueerror_subclass() -> None:
    assert issubclass(ts.TouchstoneError, ValueError)


def test_read_missing_file_raises() -> None:
    with pytest.raises(ts.TouchstoneError, match="failed to read"):
        ts.read("does-not-exist.s2p")


def test_network_from_arrays() -> None:
    f = np.array([1e9, 2e9, 3e9])
    s = np.zeros((3, 2, 2), dtype=np.complex128)
    s[1, 1, 0] = 0.5 - 0.25j
    net = ts.Network(f, s)

    assert net.nports == 2
    assert net.noise is None
    assert net.f.dtype == np.float64
    assert net.f.shape == (3,)
    assert net.s.dtype == np.complex128
    assert net.s.shape == (3, 2, 2)
    assert net.s[1, 1, 0] == 0.5 - 0.25j
    assert net.z0.dtype == np.complex128
    assert net.z0.shape == (3, 2)
    np.testing.assert_array_equal(net.z0, np.full((3, 2), 50.0 + 0j))
    assert repr(net) == "<Network 2-port, 3 frequency points>"


def test_network_custom_z0_is_broadcast_over_the_sweep() -> None:
    """One value per port is tiled across the frequencies.

    That is the shape every Touchstone file declares, so it is the shape a
    caller is likeliest to have; z0 itself is (F, N) so it can also hold a
    reference that genuinely varies with frequency.
    """
    f = np.array([1e9, 2e9])
    s = np.zeros((2, 3, 3), dtype=np.complex128)
    net = ts.Network(f, s, z0=np.array([50.0, 75.0, 50.0]))

    assert net.z0.shape == (2, 3)
    np.testing.assert_array_equal(net.z0, [[50, 75, 50], [50, 75, 50]])


def test_network_accepts_a_per_frequency_z0() -> None:
    f = np.array([1e9, 2e9])
    s = np.zeros((2, 2, 2), dtype=np.complex128)
    z0 = np.array([[50.0, 75.0], [50.0 + 1j, 75.0 - 2j]])
    net = ts.Network(f, s, z0=z0)

    np.testing.assert_array_equal(net.z0, z0)


def test_network_z0_accepts_real_input_and_plain_sequences() -> None:
    """Reference impedances are real in every published version of the format,
    so a caller holding real values is the ordinary case, not an error."""
    f = np.array([1e9])
    s = np.zeros((1, 2, 2), dtype=np.complex128)

    from_list = ts.Network(f, s, z0=[50.0, 75.0])
    from_int_array = ts.Network(f, s, z0=np.array([50, 75]))

    for net in (from_list, from_int_array):
        assert net.z0.dtype == np.complex128
        np.testing.assert_array_equal(net.z0, [[50 + 0j, 75 + 0j]])


def test_network_rejects_a_z0_that_fits_neither_shape() -> None:
    f = np.array([1e9])
    s = np.zeros((1, 3, 3), dtype=np.complex128)
    with pytest.raises(ValueError, match=r"z0 must have shape \(3,\) or \(1, 3\)"):
        ts.Network(f, s, z0=np.array([50.0, 75.0]))


@pytest.mark.parametrize(
    ("f_len", "s_shape", "match"),
    [
        (2, (3, 2, 2), "frequency entries"),
        (3, (3, 2, 3), "shape"),
    ],
)
def test_network_shape_validation(
    f_len: int, s_shape: tuple[int, int, int], match: str
) -> None:
    f = np.linspace(1e9, 2e9, f_len)
    s = np.zeros(s_shape, dtype=np.complex128)
    with pytest.raises(ValueError, match=match):
        ts.Network(f, s)

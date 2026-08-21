"""Smoke tests: module import, error type, array construction and validation.

Reading files is covered in test_read_v1.py.
"""

import numpy as np
import pytest
import touchstone_rs as ts


def test_version():
    assert ts.__version__ == "0.1.0"


def test_error_is_valueerror_subclass():
    assert issubclass(ts.TouchstoneError, ValueError)


def test_read_missing_file_raises():
    with pytest.raises(ts.TouchstoneError, match="failed to read"):
        ts.read("does-not-exist.s2p")


def test_network_from_arrays():
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
    assert net.z0.dtype == np.float64
    np.testing.assert_array_equal(net.z0, [50.0, 50.0])
    assert repr(net) == "<Network 2-port, 3 frequency points>"


def test_network_custom_z0():
    f = np.array([1e9])
    s = np.zeros((1, 3, 3), dtype=np.complex128)
    net = ts.Network(f, s, z0=np.array([50.0, 75.0, 50.0]))
    np.testing.assert_array_equal(net.z0, [50.0, 75.0, 50.0])


@pytest.mark.parametrize(
    ("f_len", "s_shape", "match"),
    [
        (2, (3, 2, 2), "frequency entries"),
        (3, (3, 2, 3), "shape"),
    ],
)
def test_network_shape_validation(f_len, s_shape, match):
    f = np.linspace(1e9, 2e9, f_len)
    s = np.zeros(s_shape, dtype=np.complex128)
    with pytest.raises(ValueError, match=match):
        ts.Network(f, s)

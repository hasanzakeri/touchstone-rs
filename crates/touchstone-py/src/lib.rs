//! Python bindings for `touchstone-core`, exposed as `touchstone_rs._core`.
//!
//! Parsed data is moved into NumPy arrays once, at construction, without
//! copying (`Vec` ownership is handed to NumPy); attribute access then only
//! clones reference-counted handles.

use std::path::PathBuf;

use numpy::{Complex64, IntoPyArray, PyArray1, PyArray3, PyArrayMethods};
use pyo3::create_exception;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

create_exception!(
    _core,
    TouchstoneError,
    PyValueError,
    "Raised when a Touchstone file cannot be read or parsed."
);

fn to_py_err(err: touchstone_core::Error) -> PyErr {
    TouchstoneError::new_err(err.to_string())
}

/// Noise parameters attached to a network (2-port files only).
#[pyclass(module = "touchstone_rs", frozen)]
pub struct NoiseData {
    f: Py<PyArray1<f64>>,
    nfmin_db: Py<PyArray1<f64>>,
    gamma_opt: Py<PyArray1<Complex64>>,
    rn: Py<PyArray1<f64>>,
}

#[pymethods]
impl NoiseData {
    /// Noise frequencies in Hz.
    #[getter]
    fn f(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.f.clone_ref(py)
    }

    /// Minimum noise figure in dB.
    #[getter]
    fn nfmin_db(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.nfmin_db.clone_ref(py)
    }

    /// Optimal source reflection coefficient.
    #[getter]
    fn gamma_opt(&self, py: Python<'_>) -> Py<PyArray1<Complex64>> {
        self.gamma_opt.clone_ref(py)
    }

    /// Effective noise resistance, normalized to the reference impedance.
    #[getter]
    fn rn(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.rn.clone_ref(py)
    }
}

impl NoiseData {
    fn from_core(py: Python<'_>, noise: touchstone_core::NoiseData) -> Self {
        NoiseData {
            f: noise.freq_hz.into_pyarray(py).unbind(),
            nfmin_db: noise.nfmin_db.into_pyarray(py).unbind(),
            gamma_opt: noise.gamma_opt.into_pyarray(py).unbind(),
            rn: noise.rn.into_pyarray(py).unbind(),
        }
    }
}

/// An N-port network sampled at F frequencies.
#[pyclass(module = "touchstone_rs", frozen)]
pub struct Network {
    f: Py<PyArray1<f64>>,
    s: Py<PyArray3<Complex64>>,
    z0: Py<PyArray1<f64>>,
    #[pyo3(get)]
    nports: usize,
    noise: Option<Py<NoiseData>>,
}

#[pymethods]
impl Network {
    /// Build a network from arrays: `f` (F, Hz), `s` (F, N, N), and an
    /// optional per-port `z0` (N, defaults to 50 Ω).
    #[new]
    #[pyo3(signature = (f, s, z0=None))]
    fn new(
        py: Python<'_>,
        f: numpy::PyReadonlyArray1<'_, f64>,
        s: numpy::PyReadonlyArray3<'_, Complex64>,
        z0: Option<numpy::PyReadonlyArray1<'_, f64>>,
    ) -> PyResult<Self> {
        let s_shape = s.as_array().shape().to_vec();
        let (nf, nports) = (s_shape[0], s_shape[1]);
        if s_shape[1] != s_shape[2] {
            return Err(PyValueError::new_err(format!(
                "s must have shape (F, N, N), got {s_shape:?}"
            )));
        }
        let f_len = f.as_array().len();
        if f_len != nf {
            return Err(PyValueError::new_err(format!(
                "f has {f_len} points but s has {nf} frequency entries"
            )));
        }
        let z0_vec = match &z0 {
            Some(z0) => {
                let z0_len = z0.as_array().len();
                if z0_len != nports {
                    return Err(PyValueError::new_err(format!(
                        "z0 has {z0_len} entries for a {nports}-port network"
                    )));
                }
                z0.as_array().to_vec()
            }
            None => vec![50.0; nports],
        };
        let s_flat: Vec<Complex64> = s.as_array().iter().copied().collect();
        Ok(Network {
            f: f.as_array().to_vec().into_pyarray(py).unbind(),
            s: s_flat
                .into_pyarray(py)
                .reshape([nf, nports, nports])?
                .unbind(),
            z0: z0_vec.into_pyarray(py).unbind(),
            nports,
            noise: None,
        })
    }

    /// Frequencies in Hz, shape (F,), float64.
    #[getter]
    fn f(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.f.clone_ref(py)
    }

    /// Network parameters, shape (F, N, N), complex128.
    #[getter]
    fn s(&self, py: Python<'_>) -> Py<PyArray3<Complex64>> {
        self.s.clone_ref(py)
    }

    /// Per-port reference impedance, shape (N,), float64.
    #[getter]
    fn z0(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.z0.clone_ref(py)
    }

    /// Noise parameters, if the file carried a noise section.
    #[getter]
    fn noise(&self, py: Python<'_>) -> Option<Py<NoiseData>> {
        self.noise.as_ref().map(|n| n.clone_ref(py))
    }

    fn __repr__(&self, py: Python<'_>) -> PyResult<String> {
        let nf = self.f.bind(py).len()?;
        Ok(format!(
            "<Network {}-port, {} frequency points>",
            self.nports, nf
        ))
    }
}

impl Network {
    fn from_core(py: Python<'_>, net: touchstone_core::Network) -> PyResult<Self> {
        let (nf, n) = (net.freq_hz.len(), net.nports);
        let noise = match net.noise {
            Some(noise) => Some(Py::new(py, NoiseData::from_core(py, noise))?),
            None => None,
        };
        Ok(Network {
            f: net.freq_hz.into_pyarray(py).unbind(),
            s: net.s.into_pyarray(py).reshape([nf, n, n])?.unbind(),
            z0: net.z0.into_pyarray(py).unbind(),
            nports: n,
            noise,
        })
    }
}

/// Read and parse a Touchstone `.sNp` file.
#[pyfunction]
fn read(py: Python<'_>, path: PathBuf) -> PyResult<Network> {
    let net = touchstone_core::parse_file(&path).map_err(to_py_err)?;
    Network::from_core(py, net)
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("TouchstoneError", m.py().get_type::<TouchstoneError>())?;
    m.add_class::<Network>()?;
    m.add_class::<NoiseData>()?;
    m.add_function(wrap_pyfunction!(read, m)?)?;
    Ok(())
}

//! Python bindings for `touchstone-core`, exposed as `touchstone_rs._core`.
//!
//! Parsed data is moved into NumPy arrays once, at construction, without
//! copying (`Vec` ownership is handed to NumPy); attribute access then only
//! clones reference-counted handles.

use std::path::PathBuf;

use numpy::{Complex64, IntoPyArray, PyArray1, PyArray2, PyArray3, PyArrayMethods};
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
    rn_ohms: Py<PyArray1<f64>>,
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

    /// Effective noise resistance exactly as the file writes it.
    ///
    /// Not the same quantity in every version: a Touchstone 1.0 or 1.1 file
    /// normalizes it to the option line's reference resistance, while a 2.0 or
    /// 2.1 file writes ohms. Use `rn_ohms` for a number whose meaning does not
    /// depend on what wrote the file.
    #[getter]
    fn rn(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.rn.clone_ref(py)
    }

    /// Effective noise resistance in ohms, whatever the source version.
    #[getter]
    fn rn_ohms(&self, py: Python<'_>) -> Py<PyArray1<f64>> {
        self.rn_ohms.clone_ref(py)
    }
}

impl NoiseData {
    fn from_core(py: Python<'_>, noise: touchstone_core::NoiseData) -> Self {
        NoiseData {
            f: noise.freq_hz.into_pyarray(py).unbind(),
            nfmin_db: noise.nfmin_db.into_pyarray(py).unbind(),
            gamma_opt: noise.gamma_opt.into_pyarray(py).unbind(),
            rn: noise.rn.into_pyarray(py).unbind(),
            rn_ohms: noise.rn_ohms.into_pyarray(py).unbind(),
        }
    }
}

/// A caller-supplied `z0`, as its shape and a flat complex vector.
///
/// Three forms are accepted, and real ones are widened rather than refused:
/// the specification's reference impedances are real, so a caller who has only
/// ever seen a conforming file has no reason to hold complex values, and
/// making them build some would be a tax on the ordinary case. The complex
/// form exists for a solver's own per-frequency port impedance.
fn z0_values(obj: &Bound<'_, PyAny>) -> PyResult<(Vec<usize>, Vec<Complex64>)> {
    if let Ok(array) = obj.extract::<numpy::PyReadonlyArrayDyn<'_, Complex64>>() {
        let array = array.as_array();
        return Ok((array.shape().to_vec(), array.iter().copied().collect()));
    }
    if let Ok(array) = obj.extract::<numpy::PyReadonlyArrayDyn<'_, f64>>() {
        let array = array.as_array();
        return Ok((
            array.shape().to_vec(),
            array.iter().map(|&r| Complex64::new(r, 0.0)).collect(),
        ));
    }
    // Anything else numeric: an integer array, a float32 one, a list, a tuple,
    // a nested list. Converting through NumPy handles every dtype and every
    // shape at once, where matching on dtypes one at a time would accept a
    // 1-D integer array and reject a 2-D one for no reason a caller could see.
    let converted = numpy::get_array_module(obj.py())?
        .call_method1("asarray", (obj, "complex128"))
        .map_err(|_| {
            PyValueError::new_err("z0 must be an array or sequence of real or complex numbers")
        })?;
    let array = converted.extract::<numpy::PyReadonlyArrayDyn<'_, Complex64>>()?;
    let array = array.as_array();
    Ok((array.shape().to_vec(), array.iter().copied().collect()))
}

/// An N-port network sampled at F frequencies.
#[pyclass(module = "touchstone_rs", frozen)]
pub struct Network {
    f: Py<PyArray1<f64>>,
    s: Py<PyArray3<Complex64>>,
    z0: Py<PyArray2<Complex64>>,
    #[pyo3(get)]
    nports: usize,
    noise: Option<Py<NoiseData>>,
}

#[pymethods]
impl Network {
    /// Build a network from arrays: `f` (F, Hz), `s` (F, N, N), and an
    /// optional `z0`, either (N,) or (F, N), defaulting to 50 Ω.
    ///
    /// A one-dimensional `z0` is one value per port for the whole sweep — what
    /// every Touchstone file declares — and is tiled across the frequencies
    /// here so callers need not do it themselves. Real input is accepted and
    /// widened, since the specification's own reference impedances are real.
    #[new]
    #[pyo3(signature = (f, s, z0=None))]
    fn new(
        py: Python<'_>,
        f: numpy::PyReadonlyArray1<'_, f64>,
        s: numpy::PyReadonlyArray3<'_, Complex64>,
        z0: Option<Bound<'_, PyAny>>,
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
        let z0_flat: Vec<Complex64> = match z0 {
            None => vec![Complex64::new(50.0, 0.0); nf * nports],
            Some(z0) => {
                let (shape, values) = z0_values(&z0)?;
                match shape.as_slice() {
                    [n] if *n == nports => (0..nf).flat_map(|_| values.iter().copied()).collect(),
                    [rows, cols] if *rows == nf && *cols == nports => values,
                    other => {
                        return Err(PyValueError::new_err(format!(
                            "z0 must have shape ({nports},) or ({nf}, {nports}) for a \
                             {nports}-port network at {nf} frequencies, got {other:?}"
                        )));
                    }
                }
            }
        };
        let s_flat: Vec<Complex64> = s.as_array().iter().copied().collect();
        Ok(Network {
            f: f.as_array().to_vec().into_pyarray(py).unbind(),
            s: s_flat
                .into_pyarray(py)
                .reshape([nf, nports, nports])?
                .unbind(),
            z0: z0_flat.into_pyarray(py).reshape([nf, nports])?.unbind(),
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

    /// Reference impedance, shape (F, N), complex128.
    ///
    /// Per frequency and complex, though a conforming Touchstone file states
    /// neither: its reference impedance is one real number per port for the
    /// whole sweep, so every row comes back identical with a zero imaginary
    /// part. The shape is what a field solver's own per-frequency port
    /// impedance needs, and is fixed now so it will not have to change.
    #[getter]
    fn z0(&self, py: Python<'_>) -> Py<PyArray2<Complex64>> {
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
        // `net.metadata` (unit, format, resistance, option line, comments)
        // is intentionally not surfaced here yet. Its only real consumer is
        // the writer, which should design the Python shape (frozen pyclass
        // vs. dict, tuple vs. list for comments) — that lands with the
        // writer milestone rather than being guessed at now.
        Ok(Network {
            f: net.freq_hz.into_pyarray(py).unbind(),
            s: net.s.into_pyarray(py).reshape([nf, n, n])?.unbind(),
            z0: net.z0.into_pyarray(py).reshape([nf, n])?.unbind(),
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

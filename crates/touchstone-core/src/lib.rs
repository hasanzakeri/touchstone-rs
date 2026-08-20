//! Parser and writer for Touchstone `.sNp` files (versions 1.0 and 2.0).
//!
//! Data is normalized on read: frequencies to Hz, network parameters to
//! complex values regardless of the on-disk format (`RI`/`MA`/`DB`). The
//! original option line is preserved in [`Metadata`] so writes can
//! round-trip faithfully.
//!
//! Parsing is not implemented yet; the public types below define the stable
//! API surface.

use std::fmt;
use std::path::Path;

pub use num_complex::Complex64;

/// On-disk representation of complex values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Real / imaginary pairs.
    Ri,
    /// Linear magnitude / angle in degrees.
    Ma,
    /// dB magnitude (20·log10) / angle in degrees.
    Db,
}

/// Frequency unit given in the option line. Frequencies are always
/// normalized to Hz in [`Network::freq_hz`]; this only records the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreqUnit {
    Hz,
    KHz,
    MHz,
    GHz,
}

impl FreqUnit {
    /// Factor that converts a value in this unit to Hz.
    pub fn to_hz(self) -> f64 {
        match self {
            FreqUnit::Hz => 1.0,
            FreqUnit::KHz => 1e3,
            FreqUnit::MHz => 1e6,
            FreqUnit::GHz => 1e9,
        }
    }
}

/// Network parameter type given in the option line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parameter {
    S,
    Y,
    Z,
    /// Hybrid-g parameters.
    G,
    /// Hybrid-h parameters.
    H,
}

/// Touchstone specification version the file was parsed as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    V1,
    V2,
}

/// Source-file details preserved for faithful round-tripping.
#[derive(Debug, Clone)]
pub struct Metadata {
    pub version: Version,
    pub freq_unit: FreqUnit,
    pub parameter: Parameter,
    pub format: Format,
    /// The `R` value from the option line.
    pub resistance: f64,
    /// The option line exactly as it appeared in the file, if any.
    pub option_line: Option<String>,
    /// Comment lines (`!`), in order of appearance.
    pub comments: Vec<String>,
}

impl Default for Metadata {
    /// Option-line defaults per the v1 specification: `# GHZ S MA R 50`.
    fn default() -> Self {
        Metadata {
            version: Version::V1,
            freq_unit: FreqUnit::GHz,
            parameter: Parameter::S,
            format: Format::Ma,
            resistance: 50.0,
            option_line: None,
            comments: Vec::new(),
        }
    }
}

/// Noise parameters from the optional noise section of 2-port v1 files
/// (and the `[Noise Data]` section of v2 files). All vectors share one
/// length; `freq_hz` is normalized to Hz.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NoiseData {
    pub freq_hz: Vec<f64>,
    /// Minimum noise figure in dB.
    pub nfmin_db: Vec<f64>,
    /// Optimal source reflection coefficient.
    pub gamma_opt: Vec<Complex64>,
    /// Effective noise resistance, normalized to the reference impedance.
    pub rn: Vec<f64>,
}

/// A parsed Touchstone file: an N-port network sampled at F frequencies.
#[derive(Debug, Clone)]
pub struct Network {
    /// Frequencies in Hz, length F, ascending.
    pub freq_hz: Vec<f64>,
    /// Network parameters, length F·N·N, laid out row-major as
    /// `(frequency, row, column)`.
    pub s: Vec<Complex64>,
    pub nports: usize,
    /// Per-port reference impedance, length N.
    pub z0: Vec<f64>,
    pub noise: Option<NoiseData>,
    pub metadata: Metadata,
}

impl Network {
    /// Number of frequency points.
    pub fn nfreqs(&self) -> usize {
        self.freq_hz.len()
    }

    /// Parameter at frequency index `fi`, ports `(row, col)`, zero-based.
    pub fn at(&self, fi: usize, row: usize, col: usize) -> Complex64 {
        let n = self.nports;
        self.s[fi * n * n + row * n + col]
    }
}

/// Errors produced while reading or parsing a Touchstone file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {path}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("line {line}: {kind}")]
    Parse { line: usize, kind: ParseErrorKind },
    #[error("not implemented yet: {0}")]
    Unimplemented(&'static str),
}

/// What went wrong on a given line. Grows with the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseErrorKind {
    InvalidOptionLine(String),
    InvalidNumber(String),
    UnexpectedData(String),
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseErrorKind::InvalidOptionLine(s) => write!(f, "invalid option line: {s}"),
            ParseErrorKind::InvalidNumber(s) => write!(f, "invalid number: {s}"),
            ParseErrorKind::UnexpectedData(s) => write!(f, "unexpected data: {s}"),
        }
    }
}

/// Parse a Touchstone file from a string. The port count is inferred from
/// the file contents (and, for v1 files parsed via [`parse_file`], the
/// `.sNp` extension).
pub fn parse_str(_input: &str) -> Result<Network, Error> {
    Err(Error::Unimplemented("parse_str"))
}

/// Read and parse a Touchstone file from disk.
pub fn parse_file(path: &Path) -> Result<Network, Error> {
    let contents = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse_str(&contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_indexing_is_row_major() {
        let net = Network {
            freq_hz: vec![1e9, 2e9],
            s: (0..8).map(|i| Complex64::new(i as f64, 0.0)).collect(),
            nports: 2,
            z0: vec![50.0, 50.0],
            noise: None,
            metadata: Metadata::default(),
        };
        assert_eq!(net.nfreqs(), 2);
        // Second frequency, S21 (row 1, col 0) -> flat index 4 + 2.
        assert_eq!(net.at(1, 1, 0), Complex64::new(6.0, 0.0));
    }

    #[test]
    fn option_line_defaults_match_v1_spec() {
        let m = Metadata::default();
        assert_eq!(m.freq_unit, FreqUnit::GHz);
        assert_eq!(m.parameter, Parameter::S);
        assert_eq!(m.format, Format::Ma);
        assert_eq!(m.resistance, 50.0);
    }

    #[test]
    fn parse_is_unimplemented_stub() {
        assert!(matches!(
            parse_str("# GHZ S RI R 50\n1.0 0.0 0.0 0.0 0.0 0.0 0.0 0.0 0.0"),
            Err(Error::Unimplemented("parse_str"))
        ));
    }

    #[test]
    fn freq_unit_conversion() {
        assert_eq!(FreqUnit::GHz.to_hz(), 1e9);
        assert_eq!(FreqUnit::Hz.to_hz(), 1.0);
    }
}

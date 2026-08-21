//! The normalized in-memory data model.
//!
//! Everything here is deliberately free of parsing concerns: these are the
//! types a consumer sees, with the on-disk variation (frequency unit, value
//! format, parameter type) reduced to metadata. See ADR 0003.

use num_complex::Complex64;

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

impl Format {
    /// The option-line keyword for this format, in the spec's uppercase.
    ///
    /// One source of truth shared by error messages and, later, the writer,
    /// so the two cannot drift apart.
    pub fn as_str(self) -> &'static str {
        match self {
            Format::Ri => "RI",
            Format::Ma => "MA",
            Format::Db => "DB",
        }
    }
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

    /// The option-line keyword for this unit, in conventional casing.
    pub fn as_str(self) -> &'static str {
        match self {
            FreqUnit::Hz => "Hz",
            FreqUnit::KHz => "kHz",
            FreqUnit::MHz => "MHz",
            FreqUnit::GHz => "GHz",
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

impl Parameter {
    /// The option-line keyword for this parameter type.
    pub fn as_str(self) -> &'static str {
        match self {
            Parameter::S => "S",
            Parameter::Y => "Y",
            Parameter::Z => "Z",
            Parameter::G => "G",
            Parameter::H => "H",
        }
    }
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
    /// The option line as it appeared in the file, if any — trimmed, with
    /// any trailing `!` comment stripped, but otherwise verbatim (original
    /// spacing and case intact) so a write can reproduce the source style.
    pub option_line: Option<String>,
    /// Comment lines (`!`) seen before the first data line, in order, with
    /// the leading `!` removed. Comments trailing a data line are not
    /// retained, so this is not a complete record of every comment in the
    /// source file — see ADR 0004.
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
    fn freq_unit_conversion() {
        assert_eq!(FreqUnit::GHz.to_hz(), 1e9);
        assert_eq!(FreqUnit::Hz.to_hz(), 1.0);
    }

    #[test]
    fn keywords_match_the_option_line_vocabulary() {
        assert_eq!(Format::Ri.as_str(), "RI");
        assert_eq!(Format::Db.as_str(), "DB");
        assert_eq!(FreqUnit::KHz.as_str(), "kHz");
        assert_eq!(FreqUnit::GHz.as_str(), "GHz");
        assert_eq!(Parameter::S.as_str(), "S");
        assert_eq!(Parameter::H.as_str(), "H");
    }
}

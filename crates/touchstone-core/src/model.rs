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
///
/// Four values for two grammars. The 1.x pair share one syntax and differ only
/// in the option line: a 1.0 file gives at most one reference resistance for
/// every port, a 1.1 file gives one per port. The 2.x pair share one syntax
/// too — the 2.1 document states that apart from the `[Version]` argument
/// string, 2.1 files are identical to 2.0 files — and are kept apart so a
/// writer can reproduce the argument the source wrote.
///
/// "Version 1.1" is the specification's own designation for the per-port
/// option line; no file announces it, since 1.x files carry no `[Version]`
/// keyword at all. It is inferred from the option line having more than one
/// `R` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Version {
    /// One reference resistance for all ports.
    V1_0,
    /// Per-port reference resistances on the option line.
    V1_1,
    V2_0,
    V2_1,
}

/// Which entries of the matrix a v2 file writes, from `[Matrix Format]`.
///
/// `Lower` and `Upper` carry one triangle including the diagonal and leave the
/// other half to symmetry, which spec 2.0 p11 notes suits interconnects — all
/// ports are still represented, the file is simply smaller. A v1 file has no
/// equivalent and is always `Full`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixFormat {
    Full,
    /// Lower triangle including the diagonal: `11`, `21 22`, `31 32 33`, …
    Lower,
    /// Upper triangle including the diagonal: `11 12 13`, `22 23`, `33`, …
    Upper,
}

impl MatrixFormat {
    /// The keyword argument for this format, in the spec's capitalization.
    pub fn as_str(self) -> &'static str {
        match self {
            MatrixFormat::Full => "Full",
            MatrixFormat::Lower => "Lower",
            MatrixFormat::Upper => "Upper",
        }
    }
}

/// The order a 2-port file writes its off-diagonal entries in, from
/// `[Two-Port Data Order]`.
///
/// The whole reason the keyword exists. Touchstone 1.0 writes S11 S21 S12 S22
/// — 21 before 12, unlike every other port count — and enough tools adopted
/// the natural row-major order instead that a 2-port v2 file has to say which
/// it means. Guessing wrong transposes the matrix in silence, and no
/// reciprocal device's data can reveal it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwoPortOrder {
    /// `21_12`: S11 S21 S12 S22, the Touchstone 1.0 convention.
    S21First,
    /// `12_21`: S11 S12 S21 S22, plain row-major.
    S12First,
}

impl TwoPortOrder {
    /// The keyword argument for this order, as the spec spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            TwoPortOrder::S21First => "21_12",
            TwoPortOrder::S12First => "12_21",
        }
    }
}

/// Source-file details preserved for faithful round-tripping.
#[derive(Debug, Clone)]
pub struct Metadata {
    pub version: Version,
    pub freq_unit: FreqUnit,
    pub parameter: Parameter,
    pub format: Format,
    /// The `R` values from the option line, in the order written.
    ///
    /// One value in a 1.0 or 2.x file, one *per port* in a 1.1 file. This is
    /// what the option line said, not the reference environment that resulted:
    /// [`Network::z0`] is where the latter lives, already expanded to every
    /// port and every frequency, and in a v2 file `[Reference]` may have
    /// overridden this entirely.
    pub resistances: Vec<f64>,
    /// The `[Reference]` values, when a v2 file gave them. `None` means the
    /// option line's `R` supplied the reference instead, which a writer needs
    /// to know in order to reproduce the file rather than merely its numbers.
    pub reference: Option<Vec<f64>>,
    /// Which entries the file wrote, from `[Matrix Format]`. `None` when the
    /// keyword was absent — v2 defaults it to `Full`, and v1 has no such
    /// keyword at all, so this records that nothing said so.
    pub matrix_format: Option<MatrixFormat>,
    /// The order a 2-port file wrote its off-diagonal entries in. `Some` only
    /// for a v2 2-port file, where `[Two-Port Data Order]` is mandatory; a v1
    /// file always uses [`TwoPortOrder::S21First`] without saying so.
    pub two_port_order: Option<TwoPortOrder>,
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
            version: Version::V1_0,
            freq_unit: FreqUnit::GHz,
            parameter: Parameter::S,
            format: Format::Ma,
            resistances: vec![50.0],
            reference: None,
            matrix_format: None,
            two_port_order: None,
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
    /// Effective noise resistance **exactly as the file writes it**, which is
    /// not the same quantity in every version.
    ///
    /// A 1.0 or 1.1 file writes it normalized to the option line's reference
    /// resistance; a 2.0 or 2.1 file writes it in ohms. The specification
    /// demonstrates the difference with its own paired examples, where one
    /// device's noise row changes in this column alone, from `0.38` to `19`
    /// against a 50 Ω reference.
    ///
    /// Use [`NoiseData::rn_ohms`] for a number that means the same thing
    /// whatever wrote the file. This field exists because it is what the file
    /// says, and a writer needs it to reproduce its input. See ADR 0010.
    pub rn: Vec<f64>,
    /// Effective noise resistance in ohms, whatever the source version.
    ///
    /// Derived: `rn` multiplied by the option line's reference resistance for
    /// a 1.x file — port 1's, where they differ — and `rn` unchanged for a 2.x
    /// one, which already writes ohms.
    pub rn_ohms: Vec<f64>,
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
    /// Reference impedance, length F·N, laid out row-major as
    /// `(frequency, port)` — the same convention as [`Network::s`].
    ///
    /// Per frequency *and* complex, though no Touchstone file states either:
    /// the specification's reference impedance is one real number per port,
    /// constant across the sweep, and spec 2.0 and 2.1 both say outright that
    /// complex and imaginary values are not supported. Parsing a conforming
    /// file therefore fills every row identically and every imaginary part
    /// with zero.
    ///
    /// The shape exists because a reference impedance that is neither of those
    /// things is nonetheless what field solvers compute and write beside their
    /// S-parameters, per frequency and complex, when they are asked not to
    /// renormalize. Reading that is a later milestone; the array it lands in
    /// is this one, chosen now so it need not change shape then. See ADR 0009.
    pub z0: Vec<Complex64>,
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

    /// Reference impedance at frequency index `fi` and `port`, zero-based.
    pub fn z0_at(&self, fi: usize, port: usize) -> Complex64 {
        self.z0[fi * self.nports + port]
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
            // Distinct per port and per frequency, so an index that collapsed
            // either axis would land on the wrong number rather than the right
            // one by luck.
            z0: (0..4).map(|i| Complex64::new(f64::from(i), 0.0)).collect(),
            noise: None,
            metadata: Metadata::default(),
        };
        assert_eq!(net.nfreqs(), 2);
        // Second frequency, S21 (row 1, col 0) -> flat index 4 + 2.
        assert_eq!(net.at(1, 1, 0), Complex64::new(6.0, 0.0));
        // Second frequency, port 1 -> flat index 2 + 1.
        assert_eq!(net.z0_at(1, 1), Complex64::new(3.0, 0.0));
    }

    #[test]
    fn option_line_defaults_match_v1_spec() {
        let m = Metadata::default();
        assert_eq!(m.freq_unit, FreqUnit::GHz);
        assert_eq!(m.parameter, Parameter::S);
        assert_eq!(m.format, Format::Ma);
        assert_eq!(m.resistances, [50.0]);
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

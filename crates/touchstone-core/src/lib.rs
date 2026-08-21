//! Parser and writer for Touchstone `.sNp` files (versions 1.0 and 2.0).
//!
//! Data is normalized on read: frequencies to Hz, network parameters to
//! complex values regardless of the on-disk format (`RI`/`MA`/`DB`). The
//! original option line is preserved in [`Metadata`] so writes can
//! round-trip faithfully.
//!
//! This version reads Touchstone 1.0 files holding 2-port S-parameters in
//! `RI` format. Other formats, port counts, and noise sections are rejected
//! with an error that names the limit. Parsing is strict by default: see
//! `docs/adr/0004-strict-parsing-with-explicit-tolerances.md`.
//!
//! ```
//! let net = touchstone_core::parse_str("# GHZ S RI R 50\n1.0 0.1 0.2 0.9 0.0 0.0 0.0 0.3 0.4\n")?;
//! assert_eq!(net.freq_hz, [1e9]);
//! assert_eq!(net.at(0, 0, 0), touchstone_core::Complex64::new(0.1, 0.2));
//! # Ok::<(), touchstone_core::Error>(())
//! ```

use std::path::Path;

mod error;
mod lines;
mod model;
mod option_line;
mod parser;

pub use error::{Error, ParseErrorKind};
pub use model::{Format, FreqUnit, Metadata, Network, NoiseData, Parameter, Version};
pub use num_complex::Complex64;

/// Knobs that change how a file is read.
///
/// Marked `#[non_exhaustive]` on purpose: later milestones add fields (a
/// lenient mode, line-wrap tolerance) and construction goes through
/// [`ParseOptions::new`] plus the builder methods, so growing this struct is
/// not a breaking change.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct ParseOptions {
    /// Expected port count. `None` infers it from the first data line.
    pub nports: Option<usize>,
}

impl ParseOptions {
    /// Default options: infer everything from the file.
    pub fn new() -> Self {
        Self::default()
    }

    /// Assert the port count instead of inferring it.
    pub fn nports(mut self, nports: usize) -> Self {
        self.nports = Some(nports);
        self
    }
}

/// Parse a Touchstone file from a string, inferring the port count from the
/// first data line.
///
/// Use [`parse_str_with`] to state the port count explicitly. Reading from
/// disk instead? [`parse_file`] additionally takes the `.sNp` extension into
/// account.
pub fn parse_str(input: &str) -> Result<Network, Error> {
    parse_str_with(input, &ParseOptions::new())
}

/// Parse a Touchstone file from a string with explicit options.
pub fn parse_str_with(input: &str, options: &ParseOptions) -> Result<Network, Error> {
    parser::parse_v1(input, options)
}

/// Read and parse a Touchstone file from disk.
///
/// The port count comes from the `.sNp` filename extension when there is
/// one; otherwise it is inferred from the data.
pub fn parse_file(path: &Path) -> Result<Network, Error> {
    parse_file_with(path, &ParseOptions::new())
}

/// Read and parse a Touchstone file from disk with explicit options.
///
/// An `nports` given in `options` wins over the filename extension — the
/// caller knows more about the file than its name does.
///
/// The bytes are decoded as UTF-8 lossily. Spec v1.1 §2 permits only ASCII,
/// but real exports do carry stray high bytes (a degree sign in a comment,
/// for instance), and rejecting a whole measurement file over one character
/// in a comment would be indefensible.
pub fn parse_file_with(path: &Path, options: &ParseOptions) -> Result<Network, Error> {
    let bytes = std::fs::read(path).map_err(|source| Error::Io {
        path: path.display().to_string(),
        source,
    })?;
    let contents = String::from_utf8_lossy(&bytes);

    let mut options = options.clone();
    if options.nports.is_none() {
        options.nports = nports_from_extension(path);
    }
    parse_str_with(&contents, &options)
}

/// Port count implied by a `.sNp` extension, per the v1.1 §2 filename
/// convention. Case-insensitive, so `.s2p` and `.S2P` both work — real
/// files use each. Anything else (including the v2 `.ts`) yields `None`.
fn nports_from_extension(path: &Path) -> Option<usize> {
    let ext = path.extension()?.to_str()?;
    let bytes = ext.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let starts_with_s = bytes[0].eq_ignore_ascii_case(&b's');
    let ends_with_p = bytes[bytes.len() - 1].eq_ignore_ascii_case(&b'p');
    if !starts_with_s || !ends_with_p {
        return None;
    }
    ext[1..ext.len() - 1]
        .parse::<usize>()
        .ok()
        .filter(|&n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn extension_gives_the_port_count_in_either_case() {
        for (name, expected) in [
            ("filter.s2p", Some(2)),
            ("FILTER.S2P", Some(2)),
            ("thing.s1p", Some(1)),
            ("array.s16p", Some(16)),
            ("array.s16P", Some(16)),
        ] {
            assert_eq!(
                nports_from_extension(&PathBuf::from(name)),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn non_snp_extensions_yield_no_port_count() {
        for name in [
            "net.ts", // Touchstone 2
            "notes.txt",
            "data.s0p", // zero ports is not a network
            "weird.sp",
            "weird.sxp",
            "noextension",
        ] {
            assert_eq!(nports_from_extension(&PathBuf::from(name)), None, "{name}");
        }
    }

    #[test]
    fn explicit_options_override_inference() {
        // A 2-port line read as anything else fails the value-count check,
        // which proves the stated count was the one used.
        let input = "# GHZ S RI R 50\n1.0 0 0 0 0 0 0 0 0\n";
        let opts = ParseOptions::new().nports(3);
        assert!(matches!(
            parse_str_with(input, &opts),
            Err(Error::Parse {
                kind: ParseErrorKind::UnsupportedPortCount(3),
                ..
            })
        ));
    }
}

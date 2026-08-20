//! Parser and writer for Touchstone `.sNp` files (versions 1.0 and 2.0).
//!
//! Data is normalized on read: frequencies to Hz, network parameters to
//! complex values regardless of the on-disk format (`RI`/`MA`/`DB`). The
//! original option line is preserved in [`Metadata`] so writes can
//! round-trip faithfully.
//!
//! Parsing is not implemented yet; the public types below define the stable
//! API surface.

use std::path::Path;

mod error;
mod model;

pub use error::{Error, ParseErrorKind};
pub use model::{Format, FreqUnit, Metadata, Network, NoiseData, Parameter, Version};
pub use num_complex::Complex64;

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
    fn parse_is_unimplemented_stub() {
        assert!(matches!(
            parse_str("# GHZ S RI R 50\n1.0 0.0 0.0 0.0 0.0 0.0 0.0 0.0 0.0"),
            Err(Error::Unimplemented("parse_str"))
        ));
    }
}

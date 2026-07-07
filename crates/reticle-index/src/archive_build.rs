//! The two-pass external `.rtla` archive builder (Wave 2 lane 2A).
//!
//! **Contract stub.** This module's public surface is frozen by ADR 0062; its body
//! is implemented by lane 2A. The builder must be *external* (bounded memory): the
//! ADR 0016 in-RAM builder holds the whole archive at once, which does not scale to
//! the multi-gigabyte dies this format exists to stream. Two passes:
//!
//! 1. **Count** each tile's records per level in a streaming scan, writing per-tile
//!    spill files, so peak memory is one tile's worth, not the whole archive.
//! 2. **Concatenate** the spill files into the final tile-contiguous layout behind
//!    the header and directory.
//!
//! Every count read from an input stream or header is untrusted: never reserve
//! capacity from a length field beyond what the remaining input can hold (the OASIS
//! OOM lesson, commit 1b1b56b).

use crate::archive::RtlaHeader;

/// Why building a `.rtla` archive failed.
#[derive(Debug)]
pub enum BuildError {
    /// The builder is not yet implemented (contract stub; lane 2A).
    NotImplemented,
    /// An I/O error while reading input or writing spill/output files.
    Io(std::io::Error),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotImplemented => {
                write!(f, "rtla builder: not yet implemented (Wave 2 lane 2A)")
            }
            Self::Io(e) => write!(f, "rtla builder I/O error: {e}"),
        }
    }
}

impl std::error::Error for BuildError {}

/// Builds a `.rtla` archive at `_out_path` from renderable records supplied by
/// `_header` and a record source, using bounded memory.
///
/// **Contract stub** (ADR 0062): the signature is frozen; lane 2A fills the body and
/// will refine the record-source parameter into the streaming form it needs. Returns
/// [`BuildError::NotImplemented`] until then, so callers compile and fail honestly
/// rather than silently producing an empty archive.
///
/// # Errors
///
/// Returns [`BuildError`]; currently always [`BuildError::NotImplemented`].
pub fn build_rtla(_header: &RtlaHeader, _out_path: &std::path::Path) -> Result<(), BuildError> {
    Err(BuildError::NotImplemented)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{RTLA_MAGIC, RTLA_VERSION, RtlaHeader};
    use crate::streaming::ArchivableRect;
    use reticle_geometry::{Point, Rect};

    #[test]
    fn builder_stub_fails_honestly() {
        // The contract stub must fail with a clear error, never pretend success.
        // Lane 2A replaces this test with a real build-and-read round-trip.
        let header = RtlaHeader {
            magic: RTLA_MAGIC,
            version: RTLA_VERSION,
            world: ArchivableRect::from_rect(Rect::new(Point::new(0, 0), Point::new(10, 10))),
            dbu_per_micron: 1000,
            levels: vec![],
        };
        let err = build_rtla(&header, std::path::Path::new("unused.rtla")).unwrap_err();
        assert!(matches!(err, BuildError::NotImplemented));
    }
}

//! The F1 gallery-manifest contract: the per-die metadata the start-screen gallery renders
//! and the content pipeline produces.
//!
//! This is a JSON, git-committed metadata file (distinct from the rkyv `.rtla` tile
//! archive it points at). The gallery UI deserializes it to draw cards (name, technology,
//! size, source, license badge, streaming badge, landmark deep links) generically over
//! whatever dies the pipeline verified. Every die's license is a CHECKED step:
//! [`License::Verified`] (an SPDX id plus the SHA-256 of the license text) or
//! [`License::Excluded`] with a reason. No die is ever listed without one, mirroring the
//! `xtask verify-licenses` redistribution gate (ADR 0070).
//!
//! Coordinates are integer DBU and zoom is milli-scaled, so the manifest is byte-stable and
//! carries no float (consistent with the rest of the project).

use serde::{Deserialize, Serialize};

/// The gallery manifest: every verified die the library ships.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct GalleryManifest {
    /// Manifest schema version, so a consumer can refuse an incompatible file.
    pub version: u32,
    /// The dies, in a stable order (the pipeline sorts by id).
    pub dies: Vec<DieEntry>,
}

/// One die in the gallery.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DieEntry {
    /// Stable id (unique within the manifest).
    pub id: String,
    /// Human-readable name for the card.
    pub name: String,
    /// The PDK/technology id (for example `sky130`, `ihp-sg13g2`, `gf180`).
    pub technology: String,
    /// Bounding-box width in DBU.
    pub width_dbu: i64,
    /// Bounding-box height in DBU.
    pub height_dbu: i64,
    /// Where the layout came from.
    pub source: Source,
    /// The redistribution license verdict (always present; see [`License`]).
    pub license: License,
    /// Streaming badge fields for the `.rtla` archive this die streams from. `Some` for a
    /// verified, streamable die; `None` for an excluded die (no archive is ever uploaded
    /// for an unverified license). The gallery shows verified dies; excluded entries are
    /// a ledger of what was skipped and why.
    pub streaming: Option<Streaming>,
    /// Curated landmark deep links into the die (editorial: a few well-chosen views).
    pub landmarks: Vec<Landmark>,
    /// How and when the die was fetched and converted.
    pub provenance: Provenance,
}

/// Where a die's layout originated.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Source {
    /// The source repository (owner/name or a URL).
    pub repo: String,
    /// The exact commit the layout was taken from.
    pub commit: String,
    /// A browsable URL for the source.
    pub url: String,
}

/// The redistribution license verdict for a die. A die is only ever listed with a
/// [`Verified`](License::Verified) license (an identified SPDX id plus the SHA-256 of the
/// license text) or [`Excluded`](License::Excluded) with a reason; there is no third state.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "verdict")]
pub enum License {
    /// The license was identified and its text hashed.
    Verified {
        /// The SPDX identifier (for example `Apache-2.0`, `CERN-OHL-S-2.0`).
        spdx: String,
        /// Lowercase-hex SHA-256 of the license text (64 chars).
        text_sha256: String,
    },
    /// The die is listed as excluded from redistribution, with a reason.
    Excluded {
        /// Why the die is excluded (unidentified or disallowed license).
        reason: String,
    },
}

/// Streaming badge fields: what the card shows about the die's `.rtla` archive.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Streaming {
    /// The content-hash R2 object key of the `.rtla` archive.
    pub archive_key: String,
    /// The number of tiles in the archive.
    pub tile_count: u32,
    /// The archive size in bytes.
    pub total_bytes: u64,
}

/// A curated deep link into a die: a labelled camera view over a set of layers.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Landmark {
    /// The label shown in the landmarks dropdown.
    pub label: String,
    /// The cell the landmark frames.
    pub cell: String,
    /// The camera view for the landmark.
    pub view: View,
    /// The GDS layer numbers to show (empty means all visible).
    pub layers: Vec<u32>,
}

/// A camera view in integer DBU with milli-scaled zoom (pixels-per-DBU times 1000), so a
/// landmark round-trips exactly through a permalink.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct View {
    /// The view centre x in DBU.
    pub x_dbu: i64,
    /// The view centre y in DBU.
    pub y_dbu: i64,
    /// Zoom as pixels-per-DBU times 1000 (milli-scaled to avoid a float).
    pub zoom_milli: i64,
}

/// How and when a die was fetched and converted, for provenance.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Provenance {
    /// The UTC timestamp the die was fetched (RFC 3339).
    pub fetched_utc: String,
    /// The converter and version that produced the archive.
    pub converter: String,
    /// Repo path to the `.rtla.NOTICE` file backing the license verdict.
    pub notice_path: String,
}

impl GalleryManifest {
    /// Validates the manifest: ids are unique and sorted, every verified license carries a
    /// 64-char lowercase-hex text hash, and every archive key is non-empty. Returns the
    /// first offending die id, or `Ok(())`.
    ///
    /// # Errors
    ///
    /// Returns a message naming the first die that violates the contract.
    pub fn validate(&self) -> Result<(), String> {
        for (i, die) in self.dies.iter().enumerate() {
            if i > 0 {
                let prev = &self.dies[i - 1].id;
                if &die.id == prev {
                    return Err(format!("duplicate die id `{}`", die.id));
                }
                if &die.id < prev {
                    return Err(format!("dies not sorted by id at `{}`", die.id));
                }
            }
            match &die.license {
                License::Verified { text_sha256, .. } => {
                    if !is_sha256_hex(text_sha256) {
                        return Err(format!("die `{}` has a bad license text hash", die.id));
                    }
                    // A verified die is streamable and must carry a non-empty archive key.
                    match &die.streaming {
                        Some(s) if !s.archive_key.is_empty() => {}
                        _ => {
                            return Err(format!(
                                "verified die `{}` must have a streaming archive key",
                                die.id
                            ));
                        }
                    }
                }
                License::Excluded { .. } => {
                    // An unverified die is never uploaded, so it carries no archive.
                    if die.streaming.is_some() {
                        return Err(format!(
                            "excluded die `{}` must not carry a streaming archive",
                            die.id
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Whether `s` is a 64-character lowercase hex string.
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

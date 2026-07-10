//! The F5 contract: the plugin manifest, the v0 host-function table, and the deterministic
//! repo-committed index.
//!
//! A manifest is parsed from plugin-provided bytes, so it is untrusted input: every
//! count- and length-bearing field is capped ([`MAX_ID_LEN`] and friends) and
//! [`Manifest::validate`] / [`Index::validate`] reject anything past the cap with a
//! structured [`ManifestError`], never a panic. The index is committed to the repo and is
//! deterministically ordered by plugin id, so it hashes and diffs stably (the leaderboard
//! pattern: the record format is the API, no server and no accounts).
//!
//! # ABI stability
//!
//! [`ABI_VERSION`] is `0` and explicitly UNSTABLE until the v8.2.0 tag. A manifest's
//! `api_version` must equal it ([`Manifest::abi_compatible`]); the field exists so a
//! post-campaign ABI break is an honest version bump, not a silent incompatibility.

use serde::{Deserialize, Serialize};

/// The ABI version this host implements. `0` and explicitly unstable until v8.2.0.
pub const ABI_VERSION: u32 = 0;

/// Maximum bytes in a plugin id (untrusted-input cap).
pub const MAX_ID_LEN: usize = 128;
/// Maximum bytes in a display name or version string (untrusted-input cap).
pub const MAX_NAME_LEN: usize = 256;
/// Maximum bytes in the entry export name (untrusted-input cap).
pub const MAX_ENTRY_LEN: usize = 128;
/// Maximum permissions a single manifest may request (untrusted-input cap).
pub const MAX_PERMISSIONS: usize = 16;
/// Maximum entries a committed index may carry (untrusted-input cap).
pub const MAX_INDEX_ENTRIES: usize = 4096;

/// A capability a plugin requests, gated at instantiation from the manifest. A host
/// function is only callable when its required permission was granted (see
/// [`HostFn::required_permission`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    /// Read the current document's cells and shapes.
    ReadDocument,
    /// Read the current selection.
    ReadSelection,
    /// Read the active technology (layers, rules).
    ReadTechnology,
    /// Stage an edit through the command and undo machinery (never a direct mutation).
    StageEdit,
}

/// A v0 host function a plugin may call. Read-only queries plus a single staged-edit
/// funnel: every edit goes through the command and undo machinery, so a plugin's effect
/// is replayable and undoable by construction. Each requires a granted [`Permission`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostFn {
    /// Query shapes in a cell (read-only).
    QueryShapes,
    /// Query the current selection (read-only).
    QuerySelection,
    /// Query the active technology (read-only).
    QueryTechnology,
    /// Submit a staged edit to the command/undo funnel.
    StageEdit,
}

impl HostFn {
    /// The permission a caller must have been granted to invoke this host function.
    #[must_use]
    pub fn required_permission(self) -> Permission {
        match self {
            Self::QueryShapes => Permission::ReadDocument,
            Self::QuerySelection => Permission::ReadSelection,
            Self::QueryTechnology => Permission::ReadTechnology,
            Self::StageEdit => Permission::StageEdit,
        }
    }
}

/// A plugin manifest, parsed from untrusted plugin-provided bytes.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Manifest {
    /// Stable plugin id (reverse-dns style), unique within an index.
    pub id: String,
    /// The plugin's own version string.
    pub version: String,
    /// The ABI version the plugin targets; must equal [`ABI_VERSION`].
    pub api_version: u32,
    /// Human-readable display name.
    pub name: String,
    /// The exported wasm function name the host calls to run the plugin.
    pub entry: String,
    /// The capabilities the plugin requests; granted or denied at instantiation.
    pub permissions: Vec<Permission>,
}

impl Manifest {
    /// Whether the manifest targets the ABI this host implements.
    #[must_use]
    pub fn abi_compatible(&self) -> bool {
        self.api_version == ABI_VERSION
    }

    /// Validates the manifest against the untrusted-input caps and the ABI version.
    ///
    /// # Errors
    ///
    /// Returns a [`ManifestError`] when a field is empty where it must not be, exceeds its
    /// cap, requests a duplicate permission, or targets a different ABI version.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.id.is_empty() {
            return Err(ManifestError::EmptyField("id"));
        }
        if self.entry.is_empty() {
            return Err(ManifestError::EmptyField("entry"));
        }
        cap("id", self.id.len(), MAX_ID_LEN)?;
        cap("version", self.version.len(), MAX_NAME_LEN)?;
        cap("name", self.name.len(), MAX_NAME_LEN)?;
        cap("entry", self.entry.len(), MAX_ENTRY_LEN)?;
        cap("permissions", self.permissions.len(), MAX_PERMISSIONS)?;
        if !self.abi_compatible() {
            return Err(ManifestError::AbiMismatch {
                wanted: self.api_version,
                host: ABI_VERSION,
            });
        }
        // Duplicate permissions are a manifest error: capability gating must be a set.
        for (i, p) in self.permissions.iter().enumerate() {
            if self.permissions[..i].contains(p) {
                return Err(ManifestError::DuplicatePermission);
            }
        }
        Ok(())
    }
}

/// One entry in the deterministic, repo-committed plugin index: a validated manifest, the
/// content hash of the plugin's wasm bytes, and where its source lives.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct IndexEntry {
    /// The plugin's manifest.
    pub manifest: Manifest,
    /// Lowercase hex SHA-256 of the plugin's wasm bytes (64 hex chars).
    pub wasm_sha256: String,
    /// Where the plugin's source lives (a repo path or a URL), for provenance.
    pub source: String,
}

/// The static plugin index: entries ordered deterministically by manifest id.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Index {
    /// The committed entries, sorted ascending by `manifest.id`.
    pub entries: Vec<IndexEntry>,
}

impl Index {
    /// Validates the index: the entry count is capped, every manifest validates, every
    /// content hash is a 64-char lowercase hex string, ids are unique, and the entries are
    /// sorted ascending by id (so the committed file is deterministic).
    ///
    /// # Errors
    ///
    /// Returns the first [`ManifestError`] encountered.
    pub fn validate(&self) -> Result<(), ManifestError> {
        cap("index", self.entries.len(), MAX_INDEX_ENTRIES)?;
        for (i, entry) in self.entries.iter().enumerate() {
            entry.manifest.validate()?;
            if !is_sha256_hex(&entry.wasm_sha256) {
                return Err(ManifestError::BadHash);
            }
            if i > 0 {
                let prev = &self.entries[i - 1].manifest.id;
                let cur = &entry.manifest.id;
                if cur == prev {
                    return Err(ManifestError::DuplicateId);
                }
                if cur < prev {
                    return Err(ManifestError::Unsorted);
                }
            }
        }
        Ok(())
    }
}

/// A structured manifest or index error. Never a panic: untrusted input yields one of
/// these, which a caller reports.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ManifestError {
    /// A required field was empty; the argument names it.
    EmptyField(&'static str),
    /// A field exceeded its cap; carries the field name, its length, and the cap.
    TooLong {
        /// The offending field name.
        field: &'static str,
        /// The observed length.
        len: usize,
        /// The maximum allowed length.
        cap: usize,
    },
    /// The manifest targets a different ABI version than the host implements.
    AbiMismatch {
        /// The `api_version` the manifest requested.
        wanted: u32,
        /// The [`ABI_VERSION`] the host implements.
        host: u32,
    },
    /// The permission list contained a duplicate.
    DuplicatePermission,
    /// Two index entries share a plugin id.
    DuplicateId,
    /// The index entries were not sorted ascending by id.
    Unsorted,
    /// A content hash was not a 64-char lowercase hex string.
    BadHash,
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "manifest field `{field}` is empty"),
            Self::TooLong { field, len, cap } => {
                write!(
                    f,
                    "manifest field `{field}` is {len} bytes, over the {cap} cap"
                )
            }
            Self::AbiMismatch { wanted, host } => {
                write!(f, "manifest targets ABI v{wanted}, host implements v{host}")
            }
            Self::DuplicatePermission => write!(f, "manifest requests a duplicate permission"),
            Self::DuplicateId => write!(f, "two index entries share a plugin id"),
            Self::Unsorted => write!(f, "index entries are not sorted by id"),
            Self::BadHash => write!(f, "content hash is not 64-char lowercase hex"),
        }
    }
}

impl std::error::Error for ManifestError {}

/// Returns `Err(TooLong)` when `len` exceeds `cap`.
fn cap(field: &'static str, len: usize, cap: usize) -> Result<(), ManifestError> {
    if len > cap {
        Err(ManifestError::TooLong { field, len, cap })
    } else {
        Ok(())
    }
}

/// Whether `s` is a 64-character lowercase hex string (a SHA-256 digest).
fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

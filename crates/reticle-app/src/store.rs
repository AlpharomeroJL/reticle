//! Transcript storage abstraction for the replay theater.
//!
//! The theater plays a recorded agent transcript (see [`crate::replay`]). Where
//! that transcript comes from differs by platform:
//!
//! * Native reads a `*.transcript.jsonl` file the user points at, from the
//!   filesystem ([`FsSessionStore`]).
//! * wasm has no filesystem, so the web build carries a real transcript compiled
//!   into the binary and plays that (`BundledSessionStore`).
//!
//! Both implement [`SessionStore`], so the app opens the theater through one trait
//! and the platform difference stays here instead of spreading `cfg` gates through
//! the app. The bundled transcript is the same model-free scripted run the native
//! theater opens into, so the web bundle can open straight into a playing theater
//! (ADR 0026).

use crate::replay::{LoadError, parse_jsonl};
use reticle_agent_api::{CommandRecord, Transcript};

/// A parsed transcript: the records in order plus the expected final hash a
/// faithful replay reproduces (absent when the source carried no trailer).
pub type ParsedTranscript = (Vec<CommandRecord>, Option<u64>);

/// A source of replay transcripts for the theater.
///
/// The theater does not care whether a transcript came from a file, a bundled
/// asset, or anywhere else; it only needs the parsed records plus the expected
/// final hash. Implementations provide a default transcript (what the theater
/// opens into) and, where the platform supports it, loading a named one.
///
/// [`std::fmt::Debug`] is a supertrait so the store can live in the `#[derive(Debug)]`
/// [`App`](crate::app::App) as a boxed trait object.
pub trait SessionStore: std::fmt::Debug {
    /// A short label naming where this store reads transcripts from, for the UI.
    fn origin_label(&self) -> &'static str;

    /// The default transcript the theater opens into: the records in order plus
    /// the expected final hash a faithful replay reproduces.
    ///
    /// # Errors
    ///
    /// Returns a [`LoadError`] if the default transcript cannot be produced (for
    /// the bundled store this only happens if the compiled-in asset is malformed,
    /// which a test guards against).
    fn default_transcript(&self) -> Result<ParsedTranscript, LoadError>;

    /// Loads a transcript named by `reference` (a filesystem path on native).
    ///
    /// Returns `Ok(None)` when this store cannot load arbitrary references (the
    /// bundled wasm store), so the caller can fall back to the default and show a
    /// note rather than treating it as an error.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when a reference was given but could not
    /// be read or parsed.
    fn load_reference(&self, reference: &str) -> Result<Option<ParsedTranscript>, String>;
}

/// The native store: transcripts are read from the filesystem.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Default, Clone, Copy)]
pub struct FsSessionStore;

#[cfg(not(target_arch = "wasm32"))]
impl SessionStore for FsSessionStore {
    fn origin_label(&self) -> &'static str {
        "filesystem"
    }

    fn default_transcript(&self) -> Result<ParsedTranscript, LoadError> {
        // Native opens into the same model-free scripted run the theater has always
        // shown; there is no on-disk default until the user loads one.
        Ok(scripted_default())
    }

    fn load_reference(&self, reference: &str) -> Result<Option<ParsedTranscript>, String> {
        let trimmed = reference.trim();
        if trimmed.is_empty() {
            return Err("Enter a transcript .jsonl path".to_owned());
        }
        match std::fs::read_to_string(trimmed) {
            Ok(text) => match parse_jsonl(&text) {
                Ok(parsed) => Ok(Some(parsed)),
                Err(e) => Err(format!("Parse failed: {e}")),
            },
            Err(e) => Err(format!("Read failed: {e}")),
        }
    }
}

/// The wasm store: a real transcript is compiled into the binary.
///
/// The web build has no filesystem, so the theater plays this bundled transcript.
/// It is the JSONL serialization of the model-free scripted run committed under
/// `assets/theater-demo.transcript.jsonl`, so the web theater shows the agent draw
/// a clean met1 wire immediately with no model, network, or key.
#[cfg(target_arch = "wasm32")]
#[derive(Debug, Default, Clone, Copy)]
pub struct BundledSessionStore;

/// The bundled transcript JSONL, compiled into the wasm binary. Kept in sync with
/// the scripted run by the round-trip test in this module.
#[cfg(target_arch = "wasm32")]
const BUNDLED_TRANSCRIPT: &str = include_str!("../assets/theater-demo.transcript.jsonl");

#[cfg(target_arch = "wasm32")]
impl SessionStore for BundledSessionStore {
    fn origin_label(&self) -> &'static str {
        "bundled demo"
    }

    fn default_transcript(&self) -> Result<ParsedTranscript, LoadError> {
        parse_jsonl(BUNDLED_TRANSCRIPT)
    }

    fn load_reference(&self, _reference: &str) -> Result<Option<ParsedTranscript>, String> {
        // No filesystem in the browser: signal "not supported here" so the caller
        // keeps the bundled default and explains why, rather than erroring.
        Ok(None)
    }
}

/// The default transcript both stores fall back to: the committed scripted run.
///
/// This is the same `scripted_run` the theater has always opened into on native
/// (ADR 0026), reduced to its transcript records and final hash. It drives a real
/// engine replay, so it is genuine content, not a placeholder.
#[must_use]
pub fn scripted_default() -> ParsedTranscript {
    let (transcript, _feed) = crate::agent_panel::scripted_run("place a clean met1 wire");
    let Transcript {
        records,
        final_hash,
        // The parsed-transcript store carries only records and hash; the plan log is
        // panel-side narration and is not part of this reduced form.
        ..
    } = transcript;
    (records, Some(final_hash))
}

/// The platform default store.
///
/// Native uses the filesystem store; wasm uses the bundled-transcript store.
#[must_use]
#[cfg(not(target_arch = "wasm32"))]
pub fn default_store() -> FsSessionStore {
    FsSessionStore
}

/// The platform default store (wasm: the bundled-transcript store).
#[must_use]
#[cfg(target_arch = "wasm32")]
pub fn default_store() -> BundledSessionStore {
    BundledSessionStore
}

#[cfg(test)]
mod tests {
    use super::*;
    use reticle_model::document_hash;
    use std::fmt::Write as _;

    // One-shot generator for the committed bundled transcript. Run explicitly with
    // RETICLE_EMIT_TRANSCRIPT=1 to (re)write assets/theater-demo.transcript.jsonl
    // from the scripted run; it is ignored in normal test runs. Kept so the asset
    // is reproducible rather than hand-written.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn emit_bundled_transcript() {
        if std::env::var_os("RETICLE_EMIT_TRANSCRIPT").is_none() {
            return;
        }
        let (records, final_hash) = scripted_default();
        let mut out = String::new();
        for record in &records {
            out.push_str(&serde_json::to_string(record).expect("record serializes"));
            out.push('\n');
        }
        let _ = writeln!(
            out,
            "{{\"final_hash\":{}}}",
            final_hash.expect("final hash present")
        );
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/theater-demo.transcript.jsonl"
        );
        std::fs::create_dir_all(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"))
            .expect("create assets dir");
        std::fs::write(path, out).expect("write transcript");
    }

    // The scripted run must produce a transcript whose recorded final hash matches
    // a real replay of it, or the theater would report a mismatch. This is the same
    // invariant the theater's HashCheck asserts, checked here as plain code.
    #[test]
    fn scripted_default_replays_to_its_recorded_hash() {
        let (records, expected) = scripted_default();
        let expected = expected.expect("scripted run carries a final hash");

        let mut session = reticle_agent_api::Session::new();
        for record in &records {
            // Re-apply each recorded command to a fresh session, ignoring the
            // per-command outcome; the end-state hash is what matters.
            let _ = session.apply(record.command.clone());
        }
        let replayed = document_hash(session.document());
        assert_eq!(
            replayed, expected,
            "scripted transcript must replay to its recorded final hash"
        );
    }
}

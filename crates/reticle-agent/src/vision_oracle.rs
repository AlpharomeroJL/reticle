//! A second, multimodal oracle: render a layout region and ask a local vision model
//! whether it matches a task's intent.
//!
//! The agent benchmark already grades a task with authoritative oracles: DRC violation
//! counts and the task's [`Checker`](reticle_bench::Checker) (rect-present, intent,
//! extraction, ...). This module adds a *second, independent* oracle of a different
//! modality: it renders the task's layout to a PNG through the existing
//! [`RenderPng`](reticle_agent_api::AgentCommand::RenderPng) path and asks a local,
//! vision-capable model (over Ollama) a yes/no question about what the render shows.
//! Two oracles that reach a verdict by unrelated means (geometry vs pixels) catching the
//! same faithful/corrupt distinction is stronger evidence than either alone; where they
//! disagree is a signal worth surfacing, not a failure.
//!
//! # Honest not-run (never an error)
//!
//! A vision model is heavy and may not be installed, may not fit VRAM, or may be off on a
//! headless host. This oracle mirrors the external-oracle pattern used by the LEF/DEF and
//! Tiny Tapeout cross-checks: it *probes* availability and, when the model (or the GPU it
//! needs to render) is absent, returns a [`VisionOutcome::Skipped`] carrying a printable
//! reason. A skip is never an error and never fails a gate; a caller logs the reason and
//! continues. Only a model that is actually present and answers produces a
//! [`VisionOutcome::Ran`] verdict. Numbers are only ever reported when the oracle really
//! ran; nothing here fabricates a verdict.
//!
//! # Wire protocol
//!
//! The request goes to Ollama's native generate endpoint (`{base}/api/generate`) with a
//! non-streaming body `{ "model", "prompt", "images": [<base64 PNG>], "stream": false }`.
//! The base64 image is the rendered PNG; the prompt states the intent and asks for a
//! `YES`/`NO` first word. The response is a single JSON object whose `response` field
//! holds the model's text, which `parse_verdict` reduces to a boolean plus the model's
//! own rationale. An error body (`{"error": ...}`), a transport failure, or an unparsable
//! reply all become an honest [`VisionOutcome::Skipped`], not a panic and not an `Err`.
//!
//! # Determinism
//!
//! A vision model's verdict is not deterministic and nothing here pins a seed, exactly as
//! for the [`ollama`](crate::ollama) proposal backend. This oracle is a *second opinion*
//! reported alongside the authoritative checker, never the authority itself: the graded
//! pass/fail of a task remains the deterministic checker's. The vision verdict and the
//! measured agreement rate are provenance, not the verdict of record.

use serde::Deserialize;

use reticle_agent_api::Session;

use crate::model::HttpTransport;
use crate::redact::ApiKey;

/// The default base URL of the Ollama server for the vision oracle. Note this is the
/// *native* API root (`/api/generate` is appended), not the OpenAI-compatible `/v1` root
/// the proposal backend uses, because image input is cleanest over the native endpoint.
pub const DEFAULT_VISION_BASE_URL: &str = "http://localhost:11434";

/// The default vision model id. A 7B-parameter `llava` fits comfortably in a 16 GB card
/// (well under the ~8 GB resident budget) and speaks Ollama's image input. Override with
/// [`ENV_VISION_MODEL`].
pub const DEFAULT_VISION_MODEL: &str = "llava:7b";

/// Environment variable naming the Ollama base URL for the vision oracle.
pub const ENV_VISION_BASE_URL: &str = "RETICLE_VISION_BASE_URL";

/// Environment variable naming the vision model id to request.
pub const ENV_VISION_MODEL: &str = "RETICLE_VISION_MODEL";

/// Environment variable holding an optional API key (Ollama needs none; present for a
/// keyed OpenAI-compatible host).
pub const ENV_VISION_API_KEY: &str = "RETICLE_VISION_API_KEY";

/// A vision model's verdict on one rendered layout: whether it matches the asked intent,
/// plus the model's own short rationale (kept verbatim for logs and the RESULT record).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VisionVerdict {
    /// Whether the model judged the render to match the intent (its `YES`/`NO`).
    pub matches: bool,
    /// The model's reasoning text, trimmed, exactly as returned.
    pub rationale: String,
}

/// The outcome of asking the vision oracle about one render.
///
/// Either the model ran and produced a [`VisionVerdict`], or the oracle did not run and
/// carries a printable reason (no model, no GPU to render, a transport or parse failure).
/// A [`Skipped`](VisionOutcome::Skipped) is the honest not-run: never an error, never a
/// fabricated verdict.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum VisionOutcome {
    /// The model ran and returned this verdict.
    Ran(VisionVerdict),
    /// The oracle did not run; the string says why.
    Skipped(String),
}

impl VisionOutcome {
    /// The verdict if the oracle ran, or `None` if it was skipped.
    #[must_use]
    pub fn ran(&self) -> Option<&VisionVerdict> {
        match self {
            Self::Ran(v) => Some(v),
            Self::Skipped(_) => None,
        }
    }

    /// True when the oracle did not run.
    #[must_use]
    pub fn is_skipped(&self) -> bool {
        matches!(self, Self::Skipped(_))
    }
}

/// The second, multimodal oracle: renders a layout and asks a local vision model whether
/// it matches an intent.
///
/// Built with [`VisionOracle::from_env`] (base URL, model id, and optional key from the
/// environment). The optional key follows the same [`redact`](crate::redact) discipline
/// as the proposal backends: never printed, never serialized, scrubbed from every error
/// string this type surfaces.
pub struct VisionOracle {
    /// Base URL of the Ollama server (`/api/generate` is appended).
    base_url: String,
    /// The vision model id to request.
    model: String,
    /// An optional API key. `None` for a keyless Ollama server. When present it reaches
    /// the wire only as `Authorization: Bearer` and is never printed or serialized.
    key: Option<ApiKey>,
    /// The HTTP transport, boxed behind the shared trait so tests inject a scripted
    /// transport with no network.
    transport: Box<dyn HttpTransport>,
}

impl std::fmt::Debug for VisionOracle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Omit `key` (a secret; even presence is a bool) and `transport` (not printable).
        f.debug_struct("VisionOracle")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("has_key", &self.key.is_some())
            .finish_non_exhaustive()
    }
}

impl VisionOracle {
    /// Builds a vision oracle from the environment.
    ///
    /// Reads [`ENV_VISION_BASE_URL`] (default [`DEFAULT_VISION_BASE_URL`]),
    /// [`ENV_VISION_MODEL`] (default [`DEFAULT_VISION_MODEL`]), and the optional
    /// [`ENV_VISION_API_KEY`]. Never fails: availability is checked separately by
    /// [`availability`](Self::availability), so a caller can build the oracle and inspect
    /// it before deciding to run.
    #[must_use]
    pub fn from_env() -> Self {
        let base_url = std::env::var(ENV_VISION_BASE_URL)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_VISION_BASE_URL.to_owned());
        let model = std::env::var(ENV_VISION_MODEL)
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_VISION_MODEL.to_owned());
        let key = ApiKey::from_env_named(ENV_VISION_API_KEY);
        Self::build(base_url, model, key)
    }

    /// Builds an oracle from explicit parts and the real HTTP transport.
    fn build(base_url: String, model: String, key: Option<ApiKey>) -> Self {
        Self {
            base_url,
            model,
            key,
            transport: Box::new(super::model::openai_ureq_transport()),
        }
    }

    /// Builds an oracle around explicit parts, for tests. Test-only.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn for_test(base_url: &str, model: &str, key: Option<ApiKey>) -> Self {
        Self::build(base_url.to_owned(), model.to_owned(), key)
    }

    /// Overrides the model id.
    #[must_use]
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Overrides the base URL (for a non-default Ollama or OpenAI-compatible host).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Swaps in a scripted transport. Test-only.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_transport(mut self, transport: Box<dyn HttpTransport>) -> Self {
        self.transport = transport;
        self
    }

    /// The configured vision model id.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Whether this oracle can run on this host, or a printable reason it cannot.
    ///
    /// Probes the same way the container oracles do (a cheap CLI check, no network): the
    /// `ollama` binary must be on the path and the configured model must already be pulled
    /// (`ollama list` lists it). Returns `Ok(())` when both hold, otherwise `Err(reason)`
    /// so a caller can print an honest skip and continue. This does not attempt a pull; a
    /// lane pulls the model up front, and a missing model here is a not-run, not an error.
    ///
    /// # Errors
    ///
    /// Returns the reason string when `ollama` is not on the path, or when the configured
    /// model is not present locally.
    pub fn availability(&self) -> Result<(), String> {
        if !ollama_available() {
            return Err("`ollama` CLI not found on PATH".to_owned());
        }
        if !vision_model_present(&self.model) {
            return Err(format!(
                "vision model `{}` not present locally (run `ollama pull {}`)",
                self.model, self.model
            ));
        }
        Ok(())
    }

    /// Renders `session`'s layout to a PNG and asks the model whether it matches `intent`.
    ///
    /// The render goes through the crate's shared `RenderPng` path (the same one the run
    /// writer uses), framing the document's bounding box. A host with no GPU adapter (or a
    /// document with no cell) yields an honest [`VisionOutcome::Skipped`] with the render
    /// note, exactly like a missing model does, so the whole oracle degrades to a not-run
    /// rather than an error.
    #[must_use]
    pub fn verify_session(&self, session: &Session, intent: &str) -> VisionOutcome {
        match crate::run::render_png_bytes(session) {
            Ok(png) => self.verify(&png, intent),
            Err(note) => {
                VisionOutcome::Skipped(format!("render unavailable (honest not-run): {note}"))
            }
        }
    }

    /// Asks the model whether the PNG `png` matches `intent`, returning a verdict or an
    /// honest skip.
    ///
    /// Posts the base64 image and the intent prompt to `{base}/api/generate`. A transport
    /// failure, an error body, or an unparsable reply all become
    /// [`VisionOutcome::Skipped`] (with a key-scrubbed reason), never an error and never a
    /// panic.
    #[must_use]
    pub fn verify(&self, png: &[u8], intent: &str) -> VisionOutcome {
        let url = format!("{}/api/generate", self.base_url.trim_end_matches('/'));
        let body = self.build_request(png, intent);
        // An empty bearer means "no Authorization header", which the transport honors.
        let bearer = self.key.as_ref().map_or("", ApiKey::expose);

        let raw = match self.transport.post_json(&url, bearer, &body) {
            Ok(raw) => raw,
            Err(e) => {
                return VisionOutcome::Skipped(format!(
                    "vision model call failed (honest not-run): {}",
                    self.scrub(&e)
                ));
            }
        };

        match parse_generate_response(&raw) {
            Ok(text) => VisionOutcome::Ran(parse_verdict(&text)),
            Err(e) => VisionOutcome::Skipped(format!(
                "vision model produced no usable verdict (honest not-run): {}",
                self.scrub(&e)
            )),
        }
    }

    /// Builds the non-streaming `/api/generate` request body: the model, the intent
    /// prompt, and the render as a single base64 image.
    fn build_request(&self, png: &[u8], intent: &str) -> serde_json::Value {
        serde_json::json!({
            "model": self.model,
            "prompt": build_prompt(intent),
            "images": [base64_encode(png)],
            "stream": false,
            // Greedy decode (temperature 0) so a given render yields a stable verdict
            // rather than drifting across runs at Ollama's default sampling temperature.
            // The oracle is still non-authoritative, but a pinned temperature makes the
            // measured agreement rate reproducible on a fixed render.
            "options": { "temperature": 0 },
        })
    }

    /// Scrubs `text` of the key when one is set, or returns it unchanged.
    fn scrub(&self, text: &str) -> String {
        match &self.key {
            Some(k) => k.scrub(text),
            None => text.to_owned(),
        }
    }
}

/// Builds the structured prompt for one verification: a clean binary geometry-vs-blank
/// question, with the intent appended as a trailing hint.
///
/// The exact phrasing matters for a small local vision model: a 7B `llava` reliably answers
/// "does this render show drawn geometry or is it blank" (YES anchored to *contains shapes*),
/// but flips to a spurious NO when the same question is framed as "does it *match* the
/// intent" or when it is asked to justify its answer (which also makes it hallucinate
/// geometry into a blank image). So the question stays binary and geometry-anchored and the
/// intent rides as a suffix, which is what discriminates a faithful layout from an empty one
/// on this host. This is a deliberately coarse second opinion (present-vs-blank consistent
/// with the intent), not a fine intent-conformance judge; see the ADR for that scoping.
fn build_prompt(intent: &str) -> String {
    format!(
        "This is a rendered integrated-circuit layout region. Does it contain drawn \
         geometry (filled colored shapes), or is it an empty or blank layout? Answer YES \
         if it contains shapes, NO if it is blank. Intent: {intent}."
    )
}

/// Whether the `ollama` CLI is available on the path (a `--version` that exits cleanly).
#[must_use]
pub fn ollama_available() -> bool {
    std::process::Command::new("ollama")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// Whether `model` is present in the local Ollama model store (via `ollama list`; no pull
/// is attempted).
///
/// Matches either the exact tag (`llava:7b`) or the model family (the part before `:`), so
/// a caller that names `llava` finds a pulled `llava:7b` and vice versa.
#[must_use]
pub fn vision_model_present(model: &str) -> bool {
    let Ok(out) = std::process::Command::new("ollama").arg("list").output() else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let listing = String::from_utf8_lossy(&out.stdout);
    let family = model.split(':').next().unwrap_or(model);
    listing.lines().any(|line| {
        let Some(name) = line.split_whitespace().next() else {
            return false;
        };
        name == model || name.split(':').next() == Some(family)
    })
}

/// A minimal view of Ollama's `/api/generate` response: the text, or an error string.
#[derive(Deserialize)]
struct GenerateResponse {
    /// The model's generated text (present on success, non-streaming).
    #[serde(default)]
    response: Option<String>,
    /// An error message (present on a failure body).
    #[serde(default)]
    error: Option<String>,
}

/// Parses an `/api/generate` response body into the model's text.
///
/// # Errors
///
/// Returns an error string when the body is unparsable, is an `{"error": ...}` body, or
/// carries no non-empty text, so the caller can record an honest skip.
fn parse_generate_response(raw: &str) -> Result<String, String> {
    let parsed: GenerateResponse =
        serde_json::from_str(raw).map_err(|e| format!("parsing vision response: {e}"))?;
    if let Some(err) = parsed.error {
        return Err(format!("vision model error: {err}"));
    }
    parsed
        .response
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "vision response contained no text".to_owned())
        .map(|s| s.trim().to_owned())
}

/// Reduces a model's free-text answer to a boolean verdict plus its rationale.
///
/// Scans the words of the answer for the first standalone `yes` or `no` (case-insensitive)
/// and takes that as the verdict; word splitting means `not`/`notable` never count as a
/// `no`. When neither word appears the verdict is conservatively `false` ("does not
/// clearly match"). The full trimmed text is kept as the rationale.
#[must_use]
fn parse_verdict(text: &str) -> VisionVerdict {
    let matches = first_yes_no(text).unwrap_or(false);
    VisionVerdict {
        matches,
        rationale: text.trim().to_owned(),
    }
}

/// The first standalone `yes` (→ `true`) or `no` (→ `false`) word in `text`, or `None` if
/// neither appears. Case-insensitive; splits on any non-alphanumeric so punctuation does
/// not hide a leading token (`YES,` still counts).
fn first_yes_no(text: &str) -> Option<bool> {
    for word in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if word.eq_ignore_ascii_case("yes") {
            return Some(true);
        }
        if word.eq_ignore_ascii_case("no") {
            return Some(false);
        }
    }
    None
}

/// Standard-alphabet base64 encoding with padding, for the `images` field.
///
/// A tiny hand-rolled encoder rather than a new dependency: the payload is a PNG the wire
/// carries as a base64 string, and the mapping is the fixed RFC 4648 alphabet. Deliberately
/// dependency-free so no `cargo update` can drift the encoding of a committed request.
fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(char::from(ALPHABET[((n >> 18) & 63) as usize]));
        out.push(char::from(ALPHABET[((n >> 12) & 63) as usize]));
        out.push(if chunk.len() > 1 {
            char::from(ALPHABET[((n >> 6) & 63) as usize])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(ALPHABET[(n & 63) as usize])
        } else {
            '='
        });
    }
    out
}

// ----- second-oracle agreement ----------------------------------------------

/// Whether the vision oracle and the authoritative checker agree on one task.
///
/// The authoritative pass/fail is `authoritative_pass` (the deterministic checker's
/// verdict); the vision oracle's is `vision.matches`. They agree when both say the layout
/// is faithful (matches / passes) or both say it is not. Agreement across two unrelated
/// modalities is the signal the second oracle exists to measure.
#[must_use]
pub fn oracles_agree(vision: &VisionVerdict, authoritative_pass: bool) -> bool {
    vision.matches == authoritative_pass
}

/// Whether a deliberately-corrupt layout is caught by *at least one* oracle.
///
/// A corrupt layout is "caught" when some oracle flags it as not-faithful: either the
/// authoritative checker failed (`!authoritative_pass`) or the vision model said it does
/// not match (`!vision_matches`). This is the property a faithful/corrupt fixture pair
/// must satisfy: even if one modality misses, the other should catch it.
#[must_use]
pub fn caught_by_any_oracle(vision_matches: bool, authoritative_pass: bool) -> bool {
    !vision_matches || !authoritative_pass
}

/// A running tally of vision/authoritative agreement over a set of graded tasks.
///
/// [`record`](Self::record) folds one comparison in; [`rate`](Self::rate) is the fraction
/// on which the two oracles agreed. Used to report the measured agreement rate over the
/// committed fixtures, and only ever over tasks the vision oracle actually ran.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct AgreementTally {
    /// How many comparisons were folded in.
    pub total: u32,
    /// How many of them the two oracles agreed on.
    pub agreements: u32,
}

impl AgreementTally {
    /// An empty tally.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds one comparison in: `agreed` is [`oracles_agree`] for that task.
    pub fn record(&mut self, agreed: bool) {
        self.total += 1;
        if agreed {
            self.agreements += 1;
        }
    }

    /// The agreement fraction in `[0, 1]`, or `0.0` when nothing was recorded.
    #[must_use]
    pub fn rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            f64::from(self.agreements) / f64::from(self.total)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgreementTally, VisionOracle, VisionOutcome, base64_encode, caught_by_any_oracle,
        first_yes_no, oracles_agree, parse_generate_response, parse_verdict,
    };
    use crate::model::HttpTransport;
    use crate::redact::ApiKey;
    use std::sync::{Arc, Mutex};

    /// A scripted transport that returns a canned body and ignores the request.
    struct FakeTransport {
        body: String,
    }

    impl HttpTransport for FakeTransport {
        fn post_json(
            &self,
            _url: &str,
            _api_key: &str,
            _body: &serde_json::Value,
        ) -> Result<String, String> {
            Ok(self.body.clone())
        }
    }

    /// A transport that captures the request body and bearer, returning a canned body.
    struct Recording {
        seen_body: Arc<Mutex<Option<serde_json::Value>>>,
        seen_bearer: Arc<Mutex<Option<String>>>,
        body: String,
    }

    impl HttpTransport for Recording {
        fn post_json(
            &self,
            _url: &str,
            api_key: &str,
            body: &serde_json::Value,
        ) -> Result<String, String> {
            *self.seen_body.lock().unwrap() = Some(body.clone());
            *self.seen_bearer.lock().unwrap() = Some(api_key.to_owned());
            Ok(self.body.clone())
        }
    }

    /// A transport that always fails, echoing the key into its error to prove scrubbing.
    struct LeakyErrorTransport;

    impl HttpTransport for LeakyErrorTransport {
        fn post_json(
            &self,
            _url: &str,
            api_key: &str,
            _body: &serde_json::Value,
        ) -> Result<String, String> {
            Err(format!("connection refused with Bearer {api_key}"))
        }
    }

    /// A generate-response body carrying `text`.
    fn generate_body(text: &str) -> String {
        serde_json::json!({ "response": text, "done": true }).to_string()
    }

    #[test]
    fn base64_matches_known_vectors() {
        // The canonical RFC 4648 examples, including the two padding cases.
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn verdict_parses_yes_and_no_first_word() {
        assert!(parse_verdict("YES, the render shows a filled rectangle.").matches);
        assert!(!parse_verdict("No. The image is entirely dark.").matches);
        // The rationale keeps the trimmed original text.
        assert_eq!(
            parse_verdict("  yes it is there  ").rationale,
            "yes it is there"
        );
    }

    #[test]
    fn verdict_word_split_ignores_substrings() {
        // `notable` / `not` must not be read as a `no`; the real `yes` wins.
        assert!(parse_verdict("It is notable that yes, geometry is present").matches);
        // No standalone yes/no at all is a conservative false.
        assert!(!parse_verdict("The layout appears to contain shapes.").matches);
        assert_eq!(first_yes_no("nothing here"), None);
    }

    #[test]
    fn generate_response_parses_text_and_surfaces_errors() {
        assert_eq!(
            parse_generate_response(&generate_body("YES here")).unwrap(),
            "YES here"
        );
        let err =
            parse_generate_response(&serde_json::json!({ "error": "model not found" }).to_string())
                .expect_err("error body");
        assert!(err.contains("model not found"));
        // Empty text is treated as no usable verdict.
        assert!(parse_generate_response(&generate_body("   ")).is_err());
        assert!(parse_generate_response("{not json").is_err());
    }

    #[test]
    fn verify_returns_ran_verdict_on_success() {
        let oracle = VisionOracle::for_test("http://localhost:11434", "llava:7b", None)
            .with_transport(Box::new(FakeTransport {
                body: generate_body("YES, a colored rectangle is visible."),
            }));
        let outcome = oracle.verify(b"\x89PNG-fake-bytes", "a metal rectangle");
        match outcome {
            VisionOutcome::Ran(v) => assert!(v.matches, "the model said yes"),
            VisionOutcome::Skipped(r) => panic!("should have run: {r}"),
        }
    }

    #[test]
    fn verify_error_body_is_honest_skip_not_error() {
        let oracle = VisionOracle::for_test("http://localhost:11434", "missing:7b", None)
            .with_transport(Box::new(FakeTransport {
                body: serde_json::json!({ "error": "model not found" }).to_string(),
            }));
        let outcome = oracle.verify(b"png", "anything");
        assert!(outcome.is_skipped(), "an error body must skip, not verdict");
        assert!(outcome.ran().is_none());
    }

    #[test]
    fn verify_transport_failure_is_skip_and_scrubs_key() {
        let oracle = VisionOracle::for_test(
            "http://localhost:11434",
            "llava:7b",
            Some(ApiKey::from_raw("sk-vision-LEAK-me")),
        )
        .with_transport(Box::new(LeakyErrorTransport));
        let outcome = oracle.verify(b"png", "intent");
        let VisionOutcome::Skipped(reason) = outcome else {
            panic!("a transport failure must be a skip");
        };
        assert!(
            !reason.contains("sk-vision-LEAK-me"),
            "the key must be scrubbed from the skip reason: {reason}"
        );
        assert!(reason.contains("[REDACTED]"));
    }

    #[test]
    fn request_body_carries_model_prompt_and_base64_image() {
        let seen_body = Arc::new(Mutex::new(None::<serde_json::Value>));
        let seen_bearer = Arc::new(Mutex::new(None::<String>));
        let oracle = VisionOracle::for_test("http://localhost:11434", "llava:7b", None)
            .with_transport(Box::new(Recording {
                seen_body: seen_body.clone(),
                seen_bearer: seen_bearer.clone(),
                body: generate_body("YES"),
            }));
        let _ = oracle.verify(b"Man", "a met1 rectangle");

        let req = seen_body.lock().unwrap().clone().expect("request captured");
        assert_eq!(req["model"], "llava:7b");
        assert_eq!(req["stream"], false);
        // The image is the base64 of the PNG bytes, in a one-element array.
        assert_eq!(req["images"][0], "TWFu");
        // The intent made it into the prompt.
        assert!(req["prompt"].as_str().unwrap().contains("a met1 rectangle"));
        // A keyless oracle sends an empty bearer (no Authorization header).
        assert_eq!(seen_bearer.lock().unwrap().clone().unwrap(), "");
    }

    #[test]
    fn agreement_and_catch_logic() {
        // Vision says matches, checker passed: agree; nothing caught (both faithful).
        let faithful = parse_verdict("YES");
        assert!(oracles_agree(&faithful, true));
        assert!(!caught_by_any_oracle(true, true));
        // Vision says no, checker failed: agree; caught by both.
        let corrupt = parse_verdict("NO");
        assert!(oracles_agree(&corrupt, false));
        assert!(caught_by_any_oracle(false, false));
        // Disagreement: vision missed but the authoritative checker caught it.
        assert!(!oracles_agree(&faithful, false));
        assert!(caught_by_any_oracle(true, false));
    }

    #[test]
    fn agreement_tally_rate() {
        let mut tally = AgreementTally::new();
        assert!(
            (tally.rate() - 0.0).abs() < 1e-9,
            "empty tally is 0, not NaN"
        );
        tally.record(true);
        tally.record(true);
        tally.record(false);
        assert_eq!(tally.total, 3);
        assert_eq!(tally.agreements, 2);
        assert!((tally.rate() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn debug_does_not_leak_key() {
        let oracle = VisionOracle::for_test(
            "http://localhost:11434",
            "llava:7b",
            Some(ApiKey::from_raw("sk-vision-hidden")),
        );
        let dbg = format!("{oracle:?}");
        assert!(!dbg.contains("sk-vision-hidden"));
        assert!(dbg.contains("has_key: true"));
    }
}

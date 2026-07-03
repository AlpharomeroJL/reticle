//! Redacting the API key so it never reaches a transcript, log, or artifact.
//!
//! The one secret this crate handles is the Anthropic API key, read from the
//! environment (never from a file or a flag). It is held in an [`ApiKey`] whose
//! [`Debug`](std::fmt::Debug) and [`Display`](std::fmt::Display) impls print a fixed
//! placeholder, so a stray `println!("{key:?}")`, a `serde_json` of a struct that
//! embeds it, or a panic message can never leak it. [`redact`] additionally scrubs the
//! raw key value out of any free text (an error string from the HTTP client, a response
//! body) before it is written anywhere durable.

use std::fmt;

/// The placeholder substituted for the secret everywhere it would otherwise
/// appear.
pub const REDACTED: &str = "[REDACTED]";

/// An API key that cannot be printed, formatted, or serialized in the clear.
///
/// Construct it from the environment with [`ApiKey::from_env`]. The inner secret
/// is reachable only through [`ApiKey::expose`], which the HTTP client calls to
/// set the `x-api-key` header; every other path ([`Debug`](std::fmt::Debug),
/// [`Display`](std::fmt::Display)) yields [`REDACTED`]. The type is deliberately
/// **not** `Serialize`, so it cannot be embedded in a transcript or result record even
/// by accident.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Reads the key from `ANTHROPIC_API_KEY`, returning `None` when the variable
    /// is unset or empty.
    ///
    /// The key is read from the environment only: there is no constructor that
    /// takes it from a file or a command-line flag, so it can never be persisted
    /// through this crate.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        Self::from_env_value(std::env::var("ANTHROPIC_API_KEY").ok())
    }

    /// Builds a key from an already-read environment value: `Some` non-empty string
    /// becomes a key, everything else is `None`.
    ///
    /// Factored out of [`from_env`](Self::from_env) so the accept/reject logic is
    /// unit-testable without mutating the process environment (which is `unsafe` on
    /// edition 2024).
    #[must_use]
    fn from_env_value(value: Option<String>) -> Option<Self> {
        match value {
            Some(v) if !v.is_empty() => Some(Self(v)),
            _ => None,
        }
    }

    /// Wraps an explicit key string. Test-only: production keys come from
    /// [`from_env`](Self::from_env).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The secret value, for setting the request auth header only.
    ///
    /// This is the single place the clear key is reachable; callers must pass the
    /// result straight to the HTTP layer and never log or store it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Replaces every occurrence of the raw key in `text` with [`REDACTED`].
    ///
    /// Used to scrub free text (HTTP error messages, response snippets) that might
    /// echo the key before it is written to a log or transcript.
    #[must_use]
    pub fn scrub(&self, text: &str) -> String {
        redact(text, &self.0)
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ApiKey").field(&REDACTED).finish()
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

/// Replaces every occurrence of `secret` in `text` with [`REDACTED`].
///
/// A no-op when `secret` is empty (there is nothing to hide, and replacing the
/// empty string would corrupt the text).
#[must_use]
pub fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_owned();
    }
    text.replace(secret, REDACTED)
}

#[cfg(test)]
mod tests {
    use super::{ApiKey, REDACTED, redact};

    #[test]
    fn debug_and_display_hide_the_secret() {
        let key = ApiKey::from_raw("sk-ant-supersecret-value");
        assert!(!format!("{key:?}").contains("supersecret"));
        assert!(!format!("{key}").contains("supersecret"));
        assert!(format!("{key}").contains(REDACTED));
    }

    #[test]
    fn expose_returns_the_clear_value() {
        let key = ApiKey::from_raw("sk-ant-clear");
        assert_eq!(key.expose(), "sk-ant-clear");
    }

    #[test]
    fn scrub_removes_the_key_from_text() {
        let key = ApiKey::from_raw("sk-ant-leak-me");
        let text = "request failed with header x-api-key: sk-ant-leak-me (401)";
        let clean = key.scrub(text);
        assert!(!clean.contains("sk-ant-leak-me"));
        assert!(clean.contains(REDACTED));
    }

    #[test]
    fn redact_is_noop_for_empty_secret() {
        assert_eq!(redact("nothing to hide", ""), "nothing to hide");
    }

    #[test]
    fn from_env_value_accepts_nonempty_and_rejects_empty_or_absent() {
        // Exercise the env-parsing logic without mutating the process environment
        // (set_var is unsafe on edition 2024): a present, non-empty value yields a key.
        let key = ApiKey::from_env_value(Some("sk-ant-from-env-test".into())).expect("nonempty");
        assert_eq!(key.expose(), "sk-ant-from-env-test");
        // An empty string or an unset variable yields no key.
        assert!(ApiKey::from_env_value(Some(String::new())).is_none());
        assert!(ApiKey::from_env_value(None).is_none());
    }
}

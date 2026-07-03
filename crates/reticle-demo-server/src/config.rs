//! Runtime configuration read from the environment.
//!
//! The demo binary is configured entirely through environment variables so it can
//! be run identically from `just demo-up`, a Dockerfile `CMD`, or a systemd unit,
//! with no config file to mount. Every value has a safe, non-permissive default so
//! the server never comes up unbounded.

use reticle_demo::LimitConfig;

/// The demo service's default bind host.
pub const DEFAULT_HOST: &str = "127.0.0.1";
/// The demo service's default bind port.
pub const DEFAULT_PORT: u16 = 3040;
/// The in-process relay's default bind address.
pub const DEFAULT_RELAY_ADDR: &str = "127.0.0.1:3041";

/// Environment variable naming the demo service bind host.
pub const HOST_ENV: &str = "HOST";
/// Environment variable naming the demo service bind port.
pub const PORT_ENV: &str = "PORT";
/// Environment variable naming the relay bind address (`host:port`). When set to
/// an external relay this binary still binds its own unless
/// [`RELAY_DISABLE_ENV`] is set; see the deployment doc.
pub const RELAY_ADDR_ENV: &str = "RETICLE_RELAY_ADDR";
/// Environment variable that, when set to a truthy value, disables the in-process
/// relay so the binary composes with an external `reticle-server`.
pub const RELAY_DISABLE_ENV: &str = "RETICLE_RELAY_DISABLE";
/// The Anthropic API key variable. Presence selects the real agent harness; the
/// value is never read here, only whether it is set and non-empty.
pub const API_KEY_ENV: &str = "ANTHROPIC_API_KEY";

/// The resolved runtime configuration.
#[derive(Clone, Debug)]
pub struct DemoConfig {
    /// The demo service bind host.
    pub host: String,
    /// The demo service bind port.
    pub port: u16,
    /// The address the spectator relay binds, or `None` when the in-process relay
    /// is disabled (composing with an external relay).
    pub relay_addr: Option<String>,
    /// The mandatory limit configuration the service enforces.
    pub limits: LimitConfig,
    /// Whether an Anthropic API key is present in the environment (selects the real
    /// harness). The key itself is never stored here.
    pub have_api_key: bool,
}

impl DemoConfig {
    /// Resolves the configuration from the process environment.
    #[must_use]
    pub fn from_env() -> Self {
        let host = std::env::var(HOST_ENV).unwrap_or_else(|_| DEFAULT_HOST.to_owned());
        let port = std::env::var(PORT_ENV)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(DEFAULT_PORT);

        let relay_addr = if env_flag(RELAY_DISABLE_ENV) {
            None
        } else {
            Some(std::env::var(RELAY_ADDR_ENV).unwrap_or_else(|_| DEFAULT_RELAY_ADDR.to_owned()))
        };

        let have_api_key = std::env::var(API_KEY_ENV)
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false);

        Self {
            host,
            port,
            relay_addr,
            limits: demo_limits(),
            have_api_key,
        }
    }

    /// The demo service bind address as `host:port`.
    #[must_use]
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Reads a boolean-ish environment flag: set and one of `1`, `true`, `yes`, `on`
/// (case-insensitive) is truthy; anything else (including unset) is false.
fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// The mandatory, non-permissive limit configuration for a public demo.
///
/// These are the values the deployment doc documents. Each field is tighter than
/// wide open on purpose:
///
/// * `per_ip_rate_per_min = 6`, `per_ip_concurrency = 1`: one live session per
///   client and a slow request cadence, so one visitor cannot monopolize the host.
/// * `global_concurrency = 4`: a hard ceiling on how many agent loops run at once,
///   bounding CPU, memory, and (with a real key) model spend.
/// * `token_budget = 100_000`, `command_budget = 200`: a runaway agent is cancelled
///   before it can burn tokens or issue an unbounded number of edits.
/// * `max_prompt_len = 400`: bounds the input a visitor can submit.
/// * `allowed_vocabulary`: a task-only word set, so the demo cannot be used as a
///   general-purpose model proxy. A prompt straying off these words is rejected with
///   `400` before it reaches a model.
#[must_use]
pub fn demo_limits() -> LimitConfig {
    LimitConfig {
        per_ip_rate_per_min: 6,
        per_ip_concurrency: 1,
        global_concurrency: 4,
        token_budget: 100_000,
        command_budget: 200,
        max_prompt_len: 400,
        allowed_vocabulary: demo_vocabulary(),
    }
}

/// The task vocabulary a demo prompt may draw from (plus the built-in stoplist and
/// bare numbers, handled by `reticle-demo`). Domain words only: the demo is a
/// layout-editing agent, not a chat proxy.
#[must_use]
pub fn demo_vocabulary() -> Vec<String> {
    [
        // verbs
        "place",
        "draw",
        "add",
        "create",
        "make",
        "route",
        "connect",
        "delete",
        "remove",
        "move",
        "fill",
        "check",
        "run",
        "render",
        "export",
        // nouns / layers / objects
        "cell",
        "rectangle",
        "rect",
        "polygon",
        "path",
        "wire",
        "shape",
        "layer",
        "metal",
        "metal1",
        "metal2",
        "met1",
        "met2",
        "poly",
        "li",
        "via",
        "contact",
        "pin",
        "label",
        "guard",
        "ring",
        "array",
        "instance",
        "net",
        "terminal",
        "region",
        "layout",
        "grid",
        // adjectives / qualifiers
        "clean",
        "wide",
        "narrow",
        "small",
        "large",
        "square",
        "round",
        "horizontal",
        "vertical",
        "top",
        "bottom",
        "inner",
        "outer",
        "spacing",
        "width",
        "clearance",
        // units / determiners the stoplist does not cover
        "nm",
        "um",
        "micron",
        "dbu",
        "one",
        "two",
        "three",
        "four",
        "five",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{DemoConfig, demo_limits, demo_vocabulary};

    #[test]
    fn demo_limits_are_non_permissive() {
        let l = demo_limits();
        // Bounded concurrency and budgets, non-empty vocabulary (the filter is on).
        assert_eq!(l.per_ip_concurrency, 1);
        assert!(l.global_concurrency >= l.per_ip_concurrency);
        assert!(l.global_concurrency <= 16, "demo global cap stays modest");
        assert!(l.token_budget > 0 && l.token_budget <= 200_000);
        assert!(l.command_budget > 0 && l.command_budget <= 1_000);
        assert!(l.max_prompt_len <= 400);
        assert!(
            !l.allowed_vocabulary.is_empty(),
            "the vocabulary filter must be configured for a public demo"
        );
    }

    #[test]
    fn vocabulary_covers_the_sample_prompt() {
        // The words the built-in demo prompt uses must all be in-vocabulary, or the
        // service would reject its own example.
        let vocab = demo_vocabulary();
        for word in ["place", "clean", "met1", "rectangle", "cell"] {
            assert!(
                vocab.iter().any(|w| w == word),
                "sample word `{word}` missing from the demo vocabulary"
            );
        }
    }

    #[test]
    fn bind_addr_composes_host_and_port() {
        let cfg = DemoConfig {
            host: "0.0.0.0".into(),
            port: 8080,
            relay_addr: None,
            limits: demo_limits(),
            have_api_key: false,
        };
        assert_eq!(cfg.bind_addr(), "0.0.0.0:8080");
    }
}

//! The demo server's mandatory limit configuration.

use serde::{Deserialize, Serialize};

/// The limits every public demo deployment must enforce. They are mandatory: the
/// server refuses to start without a configuration, and enforces every field so
/// the demo is safe to expose to the open internet.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct LimitConfig {
    /// Maximum submissions per source IP per minute.
    pub per_ip_rate_per_min: u32,
    /// Maximum concurrent sessions per source IP.
    pub per_ip_concurrency: u32,
    /// Maximum concurrent sessions across the whole server.
    pub global_concurrency: u32,
    /// Token budget per session; exceeding it cancels the session.
    pub token_budget: u64,
    /// Command budget per session; exceeding it cancels the session.
    pub command_budget: u32,
    /// Maximum prompt length in characters.
    pub max_prompt_len: usize,
    /// Allowed task-vocabulary words. A prompt containing words outside this set
    /// (beyond a small common stoplist) is rejected before it reaches a model, so
    /// the demo cannot be used as a general-purpose model proxy.
    pub allowed_vocabulary: Vec<String>,
}

impl Default for LimitConfig {
    fn default() -> Self {
        Self {
            per_ip_rate_per_min: 6,
            per_ip_concurrency: 1,
            global_concurrency: 4,
            token_budget: 100_000,
            command_budget: 200,
            max_prompt_len: 400,
            allowed_vocabulary: Vec::new(),
        }
    }
}

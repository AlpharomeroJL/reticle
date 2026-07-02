//! Benchmark task, suite manifest, and results schemas.
//!
//! A task is a TOML file deserialized into [`BenchTask`]; a suite is a versioned
//! [`SuiteManifest`] listing task ids; each run produces [`ResultRecord`]s. These
//! are the frozen data shapes the loader, runner, and results writer share.

use serde::{Deserialize, Serialize};

/// The difficulty tier of a benchmark task: 1 (primitive placement) through 5
/// (real SKY130 layers).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
pub struct Tier(pub u8);

/// A single benchmark task, loaded from a TOML file.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct BenchTask {
    /// Stable task identifier (also the file stem).
    pub id: String,
    /// Difficulty tier.
    pub tier: Tier,
    /// The natural-language prompt given to the model.
    pub prompt: String,
    /// Path, relative to the suite root, of the technology file the task uses.
    pub technology: String,
    /// Name of the checker that decides pass or fail, dispatched by the runner.
    pub checker: String,
    /// Serialized connectivity intent spec, for intent-verified tasks.
    #[serde(default)]
    pub intent: Option<String>,
}

/// A versioned manifest of the tasks in a suite.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct SuiteManifest {
    /// Semantic version of the suite; bumped when tasks are added or promoted.
    pub version: String,
    /// The task ids in the suite, in a stable order.
    pub tasks: Vec<String>,
}

/// One result row from running a task against a model.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ResultRecord {
    /// The task that was run.
    pub task_id: String,
    /// The model identifier (or `mock`).
    pub model: String,
    /// The suite version the task came from.
    pub suite_version: String,
    /// Whether the checker passed.
    pub success: bool,
    /// How many propose-verify-correct iterations were used.
    pub iterations: u32,
    /// DRC violations in the model's first proposal.
    pub first_proposal_violations: u32,
    /// DRC violations in the final document.
    pub final_violations: u32,
    /// Wall-clock time for the whole task, in milliseconds.
    pub wall_ms: u64,
}

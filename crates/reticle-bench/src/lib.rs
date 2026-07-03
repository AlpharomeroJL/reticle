//! Benchmark infrastructure for the Reticle agent suite.
//!
//! This crate freezes the per-task schema ([`BenchTask`]), the checker trait
//! ([`Checker`] with [`CheckResult`]), the versioned [`SuiteManifest`], and the
//! [`ResultRecord`], then builds the machinery around them:
//!
//! - [`loader`]: read a [`BenchTask`] from TOML and a whole suite from a directory.
//! - [`model`]: the [`ModelClient`] seam and a deterministic [`MockModel`] that
//!   scripts a propose-verify-correct sequence with no live model.
//! - [`checkers`]: the built-in [`RectPresent`], [`DrcClean`], and [`IntentCheck`]
//!   checkers and the [`CheckerRegistry`] that dispatches a task's checker name.
//! - [`geom_checkers`]: parameterized geometric checkers ([`ShapeCount`],
//!   [`LayerArea`], [`ContactStack`], [`ViaChain`], [`Comb`], [`GuardRing`],
//!   [`CompoundCell`]) whose parameters are carried in the task's `checker` string
//!   and parsed by [`params`].
//! - [`runner`]: [`run_task`], which drives a session through the loop and records a
//!   [`ResultRecord`] with a deterministic (step-counted) wall time.
//! - [`results`]: write records as JSON and render a Markdown [`Summary`].
//!
//! The end-to-end flow (load a suite, run each task against the mock, summarize) is
//! exercised by the sample suite under `benchmarks/layout-tasks/` and the crate's
//! own tests.

mod checker;
mod schema;

pub mod checkers;
pub mod geom_checkers;
pub mod loader;
pub mod mining;
pub mod model;
pub mod params;
pub mod results;
pub mod runner;

pub use checker::{CheckFailure, CheckResult, Checker};
pub use checkers::{CheckerRegistry, DrcClean, IntentCheck, RectPresent};
pub use geom_checkers::{
    Comb, CompoundCell, ContactStack, GuardRing, LayerArea, ShapeCount, ViaChain,
};
pub use loader::{LoadError, load_manifest, load_suite, load_task};
pub use model::{Context, MockModel, ModelClient};
pub use params::{ParamError, ParsedChecker};
pub use results::{Summary, TierStats, WriteError, summarize, write_records};
pub use runner::{RunError, RunOptions, run_task};
pub use schema::{BenchTask, ResultRecord, SuiteManifest, Tier};

#[cfg(test)]
mod tests {
    use super::{BenchTask, ResultRecord, SuiteManifest, Tier};

    #[test]
    fn task_and_manifest_round_trip_json() {
        let task = BenchTask {
            id: "t1_place_rect".into(),
            tier: Tier(1),
            prompt: "Place a 1um metal1 rectangle.".into(),
            technology: "tech/sky130.tech".into(),
            checker: "rect_present".into(),
            intent: None,
        };
        let back: BenchTask = serde_json::from_str(&serde_json::to_string(&task).unwrap()).unwrap();
        assert_eq!(task, back);

        let manifest = SuiteManifest {
            version: "0.1.0".into(),
            tasks: vec![task.id.clone()],
        };
        let back: SuiteManifest =
            serde_json::from_str(&serde_json::to_string(&manifest).unwrap()).unwrap();
        assert_eq!(manifest, back);

        let record = ResultRecord {
            task_id: task.id,
            model: "mock".into(),
            suite_version: manifest.version,
            success: true,
            iterations: 2,
            first_proposal_violations: 3,
            final_violations: 0,
            wall_ms: 1200,
        };
        let back: ResultRecord =
            serde_json::from_str(&serde_json::to_string(&record).unwrap()).unwrap();
        assert_eq!(record, back);
    }
}

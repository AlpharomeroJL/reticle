//! Benchmark infrastructure for the Reticle agent suite.
//!
//! This crate freezes the per-task schema ([`BenchTask`]), the checker trait
//! ([`Checker`] with [`CheckResult`]), the versioned [`SuiteManifest`], and the
//! [`ResultRecord`]. The loader, the checker-trait runner, the results writer, and
//! the deterministic mock model that exercises the propose-verify-correct loop
//! land in a later wave; these are the shapes they share.

mod checker;
mod schema;

pub use checker::{CheckFailure, CheckResult, Checker};
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

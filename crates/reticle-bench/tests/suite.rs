//! Integration test over the committed `benchmarks/layout-tasks` suite.
//!
//! The unit tests prove each checker in isolation; this proves the *authored suite*
//! is well formed end to end: every task the manifest lists loads, and every task's
//! checker is dispatchable (its `checker` string, plus any `intent`, compiles into a
//! registered [`Checker`] the runner can look up by name). A task whose checker
//! string is malformed or whose intent JSON does not parse fails here, before any
//! model runs it.
//!
//! It deliberately does *not* run the model: solving the 50 tasks against a live or
//! mock model is the runner/model lane's concern. Loading plus checker dispatch is
//! what this lane guarantees.

use std::path::PathBuf;

use reticle_bench::{BenchTask, CheckerRegistry, Tier, load_suite};

/// The committed suite directory, relative to this crate's manifest.
fn suite_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("benchmarks")
        .join("layout-tasks")
}

/// Loads the committed suite (panicking with the structured load error on failure).
fn load() -> (reticle_bench::SuiteManifest, Vec<BenchTask>) {
    load_suite(&suite_dir()).unwrap_or_else(|e| panic!("suite must load: {e}"))
}

#[test]
fn suite_loads_and_is_versioned() {
    let (manifest, tasks) = load();
    assert_eq!(manifest.version, "0.2.0", "suite version bumped to 0.2.0");
    // 3 sample tasks + 50 authored tasks.
    assert_eq!(manifest.tasks.len(), 53, "manifest lists every task");
    assert_eq!(tasks.len(), 53, "every listed task loaded");
}

#[test]
fn every_task_checker_is_dispatchable() {
    let (_manifest, tasks) = load();
    for task in &tasks {
        // Building the per-task registry compiles the task's checker (and intent, if
        // any). A malformed checker string or intent spec errors here.
        let registry = CheckerRegistry::for_task(task)
            .unwrap_or_else(|e| panic!("task `{}` checker must compile: {e}", task.id));
        // The runner resolves the checker by the exact `checker` string; it must be
        // present in the registry for the task to run at all.
        assert!(
            registry.get(&task.checker).is_some(),
            "task `{}` names checker `{}`, which the registry does not resolve",
            task.id,
            task.checker
        );
    }
}

#[test]
fn every_task_has_a_prompt_and_known_technology() {
    let (_manifest, tasks) = load();
    for task in &tasks {
        assert!(
            !task.prompt.trim().is_empty(),
            "task `{}` has an empty prompt",
            task.id
        );
        assert_eq!(
            task.technology, "sky130.tech",
            "task `{}` uses the suite technology file",
            task.id
        );
    }
}

#[test]
fn tier_coverage_spans_one_through_four() {
    let (_manifest, tasks) = load();
    let count = |tier: u8| tasks.iter().filter(|t| t.tier == Tier(tier)).count();
    // Every tier 1..=4 is represented; the authored set is weighted toward the
    // structured and connectivity tiers.
    assert!(count(1) >= 6, "tier 1 present (got {})", count(1));
    assert!(count(2) >= 10, "tier 2 present (got {})", count(2));
    assert!(count(3) >= 20, "tier 3 present (got {})", count(3));
    assert!(count(4) >= 8, "tier 4 present (got {})", count(4));
    // No task lands outside 1..=4.
    assert!(
        tasks.iter().all(|t| (1..=4).contains(&t.tier.0)),
        "all tasks are in tiers 1 through 4"
    );
}

#[test]
fn intent_tasks_carry_a_spec_and_geometric_tasks_do_not_need_one() {
    let (_manifest, tasks) = load();
    for task in &tasks {
        if task.checker == "intent" {
            assert!(
                task.intent.is_some(),
                "intent task `{}` must carry an intent spec",
                task.id
            );
        }
    }
    // At least the authored connectivity-intent group plus the sample is present.
    let intent_tasks = tasks.iter().filter(|t| t.checker == "intent").count();
    assert!(
        intent_tasks >= 12,
        "expected the tier-3 intent group (got {intent_tasks})"
    );
}

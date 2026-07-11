//! Integration test over the committed `benchmarks/layout-tasks` suite.
//!
//! The unit tests prove each checker in isolation; this proves the *authored suite*
//! is well formed end to end: every task the manifest lists loads, and every task's
//! checker is dispatchable (its `checker` string, plus any `intent`, compiles into a
//! registered [`Checker`] the runner can look up by name). A task whose checker
//! string is malformed or whose intent JSON does not parse fails here, before any
//! model runs it.
//!
//! It deliberately does *not* run the model: solving the 60 authored tasks against a
//! live or mock model is the runner/model lane's concern. Loading plus checker
//! dispatch is what this suite test guarantees; `tier5_solvability.rs` additionally
//! proves each tier-5 task is satisfiable by correct SKY130 geometry.

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
    assert_eq!(manifest.version, "0.6.0", "suite version bumped to 0.6.0");
    // 83 (v0.5.0) + 5 v0.6.0 coverage tasks (shape-count range, layer-area window,
    // via2 contact stack, XOR boolean, six-via chain).
    assert_eq!(manifest.tasks.len(), 88, "manifest lists every task");
    assert_eq!(tasks.len(), 88, "every listed task loaded");
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
fn tier_coverage_spans_one_through_five() {
    let (_manifest, tasks) = load();
    let count = |tier: u8| tasks.iter().filter(|t| t.tier == Tier(tier)).count();
    // Every tier 1..=5 is represented; the authored set is weighted toward the
    // structured and connectivity tiers, with a real-SKY130 tier-5 group on top.
    assert!(count(1) >= 6, "tier 1 present (got {})", count(1));
    assert!(count(2) >= 10, "tier 2 present (got {})", count(2));
    assert!(count(3) >= 20, "tier 3 present (got {})", count(3));
    assert!(count(4) >= 8, "tier 4 present (got {})", count(4));
    assert!(count(5) >= 10, "tier 5 present (got {})", count(5));
    // No task lands outside 1..=5.
    assert!(
        tasks.iter().all(|t| (1..=5).contains(&t.tier.0)),
        "all tasks are in tiers 1 through 5"
    );
}

#[test]
fn tier5_tasks_are_the_real_sky130_group() {
    let (_manifest, tasks) = load();
    let tier5: Vec<&BenchTask> = tasks.iter().filter(|t| t.tier == Tier(5)).collect();
    assert_eq!(tier5.len(), 10, "the tier-5 group has exactly 10 tasks");
    for task in &tier5 {
        // Naming convention ties the id to the tier, like every other tier group.
        assert!(
            task.id.starts_with("t5_"),
            "tier-5 task `{}` must carry the t5_ prefix",
            task.id
        );
        // Tier 5 is the real-SKY130 tier: every prompt cites at least one real
        // periphery rule id or a real sky130_fd_sc_hd cell, so the task is
        // grounded in the PDK rather than in made-up numbers.
        let real_references = [
            "m1.1",
            "m1.2",
            "m1.4",
            "m1.6",
            "m2.1",
            "m2.4",
            "li.5",
            "ct.1",
            "licon.1",
            "via.1a",
            "sky130_fd_sc_hd",
        ];
        assert!(
            real_references.iter().any(|r| task.prompt.contains(r)),
            "tier-5 task `{}` must cite a real SKY130 rule or cell",
            task.id
        );
    }
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

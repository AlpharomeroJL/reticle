//! Two-way solvability proof for the v0.5.0 generator task group.
//!
//! `suite.rs` proves the committed suite loads and every checker dispatches; this
//! test goes one step further for the 8 generator tasks the same way
//! `wave3_tasks.rs` and `tier5_solvability.rs` do for their groups: each task, as
//! loaded from `benchmarks/layout-tasks` (checker compiled exactly as the runner
//! compiles it), is run against a reference document that satisfies the prompt and
//! must PASS, then against a deliberately spoiled variant and must FAIL. That keeps
//! every new task honest in both directions: it is satisfiable, and it is not
//! vacuous.
//!
//! The reference document for a generator task is the geometry the named generator
//! itself emits for the prompt's parameters: because generators are DRC-clean by
//! construction and the `generator` checker fingerprints the generator's own output,
//! a correct model answer (which reproduces that structure) scores exactly as this
//! reference does. The spoiled variant is an empty document (no structure) or, for
//! one task, a DRC-dirty variant that keeps the structure but adds an illegal shape,
//! exercising the checker's DRC-cleanliness half.

use std::path::PathBuf;

use reticle_agent_api::Transcript;
use reticle_bench::{BenchTask, CheckResult, CheckerRegistry, load_suite};
use reticle_gen::Registry;
use reticle_geometry::{LayerId, Point, Rect};
use reticle_model::{Cell, Document, DrawShape, ShapeKind, Technology};
use serde_json::{Value, json};

/// li1 (67/20), the layer the sub-min-width dirty shape is drawn on.
const LI1: LayerId = LayerId::new(67, 20);

/// The committed suite directory, relative to this crate's manifest.
fn suite_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("benchmarks")
        .join("layout-tasks")
}

/// Loads the named task from the committed suite.
fn task(id: &str) -> BenchTask {
    let (_manifest, tasks) =
        load_suite(&suite_dir()).unwrap_or_else(|e| panic!("suite loads: {e}"));
    tasks
        .into_iter()
        .find(|t| t.id == id)
        .unwrap_or_else(|| panic!("task `{id}` is in the suite"))
}

/// Runs `task`'s own checker (compiled exactly as the runner compiles it) over `doc`.
fn check(task: &BenchTask, doc: &Document) -> CheckResult {
    let registry = CheckerRegistry::for_task(task)
        .unwrap_or_else(|e| panic!("task `{}` checker compiles: {e}", task.id));
    let checker = registry
        .get(&task.checker)
        .unwrap_or_else(|| panic!("task `{}` checker resolves", task.id));
    checker.check(doc, &Transcript::default())
}

/// A one-cell document named `top` holding exactly what generator `id` emits for
/// `params`: the canonical correct answer to the corresponding task.
fn doc_from_generator(id: &str, params: &Value) -> Document {
    let registry = Registry::with_builtins();
    let tech = Technology::default();
    let mut cell = Cell::new("top");
    registry
        .generate(id, params, &tech, &mut cell)
        .unwrap_or_else(|e| panic!("generator `{id}` produces reference geometry: {e}"));
    let mut doc = Document::new();
    doc.insert_cell(cell);
    doc.set_top_cells(vec!["top".into()]);
    doc
}

/// An empty one-cell document named `top`: the vacuous spoiled answer that has none
/// of the generator's structure.
fn empty_doc() -> Document {
    let mut doc = Document::new();
    doc.insert_cell(Cell::new("top"));
    doc.set_top_cells(vec!["top".into()]);
    doc
}

/// Asserts a task passes its reference (the generator's own output for `params`) and
/// fails on an empty document. Takes `params` by value so call sites pass a
/// `json!(...)` literal directly.
#[allow(clippy::needless_pass_by_value)]
fn assert_generator_task(id: &str, generator: &str, params: Value) {
    let task = task(id);
    let good = doc_from_generator(generator, &params);
    let pass = check(&task, &good);
    assert!(
        pass.is_pass(),
        "`{id}` must pass its generator reference, got {pass:?}"
    );
    let fail = check(&task, &empty_doc());
    assert!(
        matches!(fail, CheckResult::Fail(ref f) if !f.is_empty()),
        "`{id}` must reject an empty document"
    );
}

#[test]
fn guard_ring_task_two_way() {
    assert_generator_task(
        "t3_gen_guard_ring",
        "guard_ring",
        json!({
            "layer": "li1", "region_width": 2000, "region_height": 2000,
            "ring_width": 400, "taps": true,
        }),
    );
}

#[test]
fn via_farm_4x4_task_two_way() {
    assert_generator_task(
        "t3_gen_via_farm_4x4",
        "via_farm",
        json!({ "cut": "mcon", "rows": 4, "cols": 4 }),
    );
}

#[test]
fn via_farm_via_power_task_two_way() {
    assert_generator_task(
        "t3_gen_via_farm_via_power",
        "via_farm",
        json!({ "cut": "via", "rows": 6, "cols": 6 }),
    );
}

#[test]
fn fill_decap_task_two_way() {
    assert_generator_task(
        "t3_gen_fill_decap",
        "fill",
        json!({
            "layer": "li1", "region_width": 10000, "region_height": 10000,
            "tile": 400, "target_density_permille": 600,
        }),
    );
}

#[test]
fn test_vdp_task_two_way() {
    assert_generator_task(
        "t3_gen_test_vdp",
        "test_structure",
        json!({
            "kind": "van_der_pauw", "layer": "li1",
            "feature_width": 400, "feature_length": 2000, "count": 8,
        }),
    );
}

#[test]
fn test_serpentine_task_two_way() {
    assert_generator_task(
        "t3_gen_test_serpentine",
        "test_structure",
        json!({
            "kind": "serpentine", "layer": "met1",
            "feature_width": 400, "feature_length": 3000, "count": 8,
        }),
    );
}

#[test]
fn pad_ring_task_two_way() {
    assert_generator_task(
        "t3_gen_pad_ring",
        "pad_ring",
        json!({
            "die_width": 200_000, "die_height": 200_000, "pad_pitch": 100_000,
            "pad_size": 60_000, "power_pads": 4,
        }),
    );
}

#[test]
fn seal_ring_task_two_way() {
    assert_generator_task(
        "t3_gen_seal_ring",
        "seal_ring",
        json!({
            "stack": "up_to_met3", "die_width": 100_000, "die_height": 100_000,
            "ring_width": 900,
        }),
    );
}

/// The DRC-cleanliness half of the `generator` checker bites: a document that carries
/// the full guard-ring structure but also an illegal (sub-min-width) li1 rectangle
/// fails, even though its per-layer fingerprint is satisfied. This proves the checker
/// does not pass a structurally-complete-but-dirty answer.
#[test]
fn guard_ring_task_rejects_a_drc_dirty_answer() {
    let params = json!({
        "layer": "li1", "region_width": 2000, "region_height": 2000,
        "ring_width": 400, "taps": true,
    });
    let mut dirty = doc_from_generator("guard_ring", &params);
    // A 100-wide li1 rect is below the li.1 minimum width (170) and minimum area, so
    // the SKY130 subset flags it.
    dirty.cell_mut("top").unwrap().shapes.push(DrawShape::new(
        LI1,
        ShapeKind::Rect(Rect::new(Point::new(6000, 6000), Point::new(6100, 6010))),
    ));
    let task = task("t3_gen_guard_ring");
    let result = check(&task, &dirty);
    assert!(
        matches!(result, CheckResult::Fail(ref f) if f.iter().any(|x| x.reason.contains("DRC-clean"))),
        "a DRC-dirty answer must fail on the cleanliness half, got {result:?}"
    );
}

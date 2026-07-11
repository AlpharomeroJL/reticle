//! Adversarial harness for the sandboxed PCell producer (`reticle_script::pcell::produce`).
//!
//! Written by the `pcell-harness` lane as a fresh pair of eyes: this file does not reuse the
//! producer's own fixtures, scripts, or hostile-input corpus (`src/pcell/tests.rs`), and does
//! not trust its assertions. Every invariant below is independently re-derived and re-checked
//! against the real, merged `produce`. Where an assertion below encodes an invariant the
//! producer *should* hold but a real gap was found, the test is left failing (red) on purpose
//! and reported in `scratch/lanes/pcell-harness/RESULT.md` rather than silently patched here
//! (this lane owns tests only, not the producer, cache, or sandbox source).
//!
//! Sections: determinism, sandbox-limit enforcement, host isolation, cache integration
//! (`reticle_gen::PCellCache`), and zero-panic behavior on malformed input.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use reticle_gen::{FieldSchema, PCellCache, PCellDef, ParamSchema};
use reticle_model::{Cell, ShapeKind, Technology};
use reticle_script::{ProduceError, SandboxLimits, produce};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------
// Fixtures: original scripts and defs, deliberately not shared with the producer's own
// `src/pcell/tests.rs` fixtures.
// ---------------------------------------------------------------------------------------

/// A param-driven tiled-grid generator: one `UNIT` cell stamped into a `steps` x `steps`
/// array on `pitch` centers. Three integer parameters (`unit`, `steps`, `pitch`) give a
/// three-dimensional sweep for the determinism and cache tests.
///
/// `GRID` (the declared top cell) also carries its own marker rect sized by `unit`, not just
/// the array of `UNIT` (a separate, non-top cell): `produce` returns only the extracted top
/// cell, not the whole document, so a parameter that affects nothing but a *descendant*
/// cell's own fields would be invisible to a caller inspecting the returned `Cell` alone.
const GRID_SCRIPT: &str = r#"
create_cell("UNIT");
add_rect("UNIT", 11, 0, 0, 0, unit, unit);
create_cell("GRID");
add_rect("GRID", 12, 0, 0, 0, unit, 1);
add_array("GRID", "UNIT", 0, 0, steps, steps, pitch, pitch);
set_top_cells(["GRID"]);
"#;

/// A param-driven rectangular frame (four bars), built from `w` (width), `h` (height), and
/// `t` (bar thickness).
const FRAME_SCRIPT: &str = r#"
create_cell("FRAME");
add_rect("FRAME", 3, 0, 0, 0, w, t);
add_rect("FRAME", 3, 0, 0, h - t, w, h);
add_rect("FRAME", 3, 0, 0, 0, t, h);
add_rect("FRAME", 3, 0, w - t, 0, w, h);
set_top_cells(["FRAME"]);
"#;

/// A single bounded-integer field, mirroring how a real PCell schema declares a dimension.
fn int_field(name: &str, default: i64, max: i64) -> FieldSchema {
    FieldSchema::int(name, name, default, 0, max, "dbu")
}

/// A minimal [`ParamSchema`] for `fields`.
fn schema_of(id: &str, fields: Vec<FieldSchema>) -> ParamSchema {
    ParamSchema {
        generator_id: id.to_owned(),
        title: id.to_owned(),
        description: "pcell-harness fixture".to_owned(),
        fields,
    }
}

/// A [`PCellDef`] with the given id, schema fields, and script.
fn def_of(id: &str, fields: Vec<FieldSchema>, script: &str) -> PCellDef {
    PCellDef {
        id: id.to_owned(),
        title: id.to_owned(),
        description: "pcell-harness fixture".to_owned(),
        schema: schema_of(id, fields),
        script: script.to_owned(),
        engine_version: "harness-1.0.0".to_owned(),
    }
}

/// The tiled-grid fixture.
fn grid_def() -> PCellDef {
    def_of(
        "harness.grid",
        vec![
            int_field("unit", 50, 100_000),
            int_field("steps", 4, 64),
            int_field("pitch", 100, 1_000_000),
        ],
        GRID_SCRIPT,
    )
}

/// The rectangular-frame fixture.
fn frame_def() -> PCellDef {
    def_of(
        "harness.frame",
        vec![
            int_field("w", 1000, 100_000),
            int_field("h", 1000, 100_000),
            int_field("t", 50, 1000),
        ],
        FRAME_SCRIPT,
    )
}

/// A no-parameter def wrapping an arbitrary script, for hostile-*script* tests where the
/// params object itself is not under test (an empty schema means `produce` never looks up
/// any field in `params`, so its shape genuinely does not matter here).
fn hostile_def(script: &str) -> PCellDef {
    def_of("harness.hostile", vec![], script)
}

// ---------------------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------------------

/// For two independent fixtures, the same `(def, params)` produced four times over must
/// yield byte-identical geometry and an identical `param_hash` every time (not merely
/// "twice," which could hide a subtle iteration-order or timing-dependent flake).
#[test]
fn determinism_holds_across_repeated_runs_and_multiple_defs() {
    let tech = Technology::default();
    let fixtures = [
        (grid_def(), json!({ "unit": 40, "steps": 5, "pitch": 90 })),
        (frame_def(), json!({ "w": 800, "h": 600, "t": 20 })),
    ];

    for (def, params) in fixtures {
        let mut prior: Option<(Cell, String)> = None;
        for run in 0..4 {
            let (cell, meta) = produce(&def, &params, &tech, SandboxLimits::default())
                .unwrap_or_else(|e| panic!("{}: run {run} failed: {e}", def.id));
            assert!(
                meta.has_valid_hash(),
                "{}: param_hash must be well-formed",
                def.id
            );
            if let Some((prev_cell, prev_hash)) = &prior {
                assert_eq!(
                    *prev_cell, cell,
                    "{}: geometry must be byte-identical run over run",
                    def.id
                );
                assert_eq!(
                    *prev_hash, meta.param_hash,
                    "{}: param_hash must be stable run over run",
                    def.id
                );
            }
            prior = Some((cell, meta.param_hash));
        }
    }
}

/// A 3x3x3 parameter sweep over the grid fixture: every one of the 27 combinations must
/// produce geometry AND a `param_hash` distinct from every other combination. This is a much
/// stronger check than "params A differ from params B": it proves there is no accidental
/// collision anywhere across a real grid of inputs, not just at one pair of sample points.
#[test]
fn parameter_sweep_yields_pairwise_distinct_geometry_and_hash() {
    let def = grid_def();
    let tech = Technology::default();
    let mut geometries: HashSet<String> = HashSet::new();
    let mut hashes: HashSet<String> = HashSet::new();
    let mut combos = 0usize;

    for unit in [10, 20, 30] {
        for steps in [2, 3, 4] {
            for pitch in [50, 100, 150] {
                let params = json!({ "unit": unit, "steps": steps, "pitch": pitch });
                let (cell, meta) = produce(&def, &params, &tech, SandboxLimits::default())
                    .unwrap_or_else(|e| panic!("unit={unit} steps={steps} pitch={pitch}: {e}"));
                geometries.insert(format!("{cell:?}"));
                hashes.insert(meta.param_hash);
                combos += 1;
            }
        }
    }

    assert_eq!(
        geometries.len(),
        combos,
        "every parameter combination in the sweep must produce distinct geometry"
    );
    assert_eq!(
        hashes.len(),
        combos,
        "every parameter combination in the sweep must produce a distinct param_hash"
    );
}

/// The stamped `param_hash` must be computed over the EFFECTIVE parameters (each schema
/// field's provided value, or its schema default when the caller omitted it) -- not the raw,
/// possibly-partial `params` object passed to `produce`. This test reconstructs the expected
/// effective object by hand from the schema's own declared defaults (the producer's internal
/// `sandbox::effective_params` is a private function this integration test cannot call
/// either way), then cross-checks the hash the public `reticle_gen::param_hash` primitive
/// gives that hand-built object, independent of anything the producer computed internally.
#[test]
fn stamped_hash_matches_hand_reconstructed_effective_params_not_raw_input() {
    let def = frame_def();
    let tech = Technology::default();

    // Omit `t`; the schema default (declared in `frame_def`) is 50.
    let partial = json!({ "w": 500, "h": 500 });
    let (_cell, meta) = produce(&def, &partial, &tech, SandboxLimits::default())
        .expect("defaults must fill in for the omitted field");

    let expected_effective = json!({ "w": 500, "h": 500, "t": 50 });
    let expected_hash = reticle_gen::param_hash(&def.id, &def.engine_version, &expected_effective);
    assert_eq!(
        meta.param_hash, expected_hash,
        "stamped hash must equal the hash of the hand-reconstructed, schema-defaulted params"
    );

    // And it must differ from hashing the raw, incomplete params directly: the identity is
    // over the defaulted object, not the caller's literal input.
    let raw_hash = reticle_gen::param_hash(&def.id, &def.engine_version, &partial);
    assert_ne!(
        meta.param_hash, raw_hash,
        "stamped hash must NOT equal the hash of the raw, incomplete params"
    );
}

/// Omitting every parameter and explicitly spelling out every schema default must be the
/// same produce: identical geometry, identical `param_hash`.
#[test]
fn omitting_all_params_matches_explicitly_spelling_out_the_defaults() {
    let def = frame_def();
    let tech = Technology::default();
    let (cell_omitted, meta_omitted) = produce(&def, &json!({}), &tech, SandboxLimits::default())
        .expect("an all-default produce must succeed");
    let (cell_spelled, meta_spelled) = produce(
        &def,
        &json!({ "w": 1000, "h": 1000, "t": 50 }),
        &tech,
        SandboxLimits::default(),
    )
    .expect("spelling out the defaults must succeed identically");

    assert_eq!(
        cell_omitted, cell_spelled,
        "geometry must match regardless of which defaults were omitted"
    );
    assert_eq!(
        meta_omitted.param_hash, meta_spelled.param_hash,
        "param_hash must match regardless of which defaults were omitted"
    );
}

// ---------------------------------------------------------------------------------------
// Sandbox limit enforcement
// ---------------------------------------------------------------------------------------

/// A large-but-technically-finite busy loop, deliberately a different shape from the
/// producer's own `loop { i += 1; }` fixture: a bounded `for` range with arithmetic in the
/// body. Run unbounded this would take a very long time (which is exactly why the operation
/// cap must fire long before it gets anywhere close to finishing).
const RUNAWAY_FOR_LOOP: &str = r"
let acc = 0;
for i in 0..999999999 {
    acc += i % 7;
}
";

/// A hostile op-count script, differing in shape from the producer's own fixture, must still
/// be rejected by the operation cap in bounded wall time, not hang.
#[test]
fn runaway_for_loop_is_rejected_by_the_operation_cap_in_bounded_time() {
    let d = hostile_def(RUNAWAY_FOR_LOOP);
    let limits = SandboxLimits {
        max_operations: 200_000,
        ..SandboxLimits::default()
    };

    let start = Instant::now();
    let err = produce(&d, &json!({}), &Technology::default(), limits)
        .expect_err("a huge-range busy loop must be rejected before completion");
    let elapsed = start.elapsed();

    match &err {
        ProduceError::LimitExceeded(msg) => {
            assert!(msg.contains("operation"), "message names the bound: {msg}");
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(5),
        "must be bounded, took {elapsed:?}"
    );
}

/// A fixed-cost script (a bounded 1000-iteration loop, no infinite anything): a starved
/// operation cap must reject it, and a generous one must let the very same script complete.
/// This proves the cap is a genuine two-sided threshold (it does not just always reject, or
/// always pass regardless of the configured limit).
#[test]
fn operation_cap_is_a_genuine_threshold_not_a_blanket_reject() {
    let script = r#"
    let acc = 0;
    for i in 0..1000 {
        acc += i;
    }
    create_cell("DONE");
    add_rect("DONE", 1, 0, 0, 0, acc, 1);
    set_top_cells(["DONE"]);
    "#;
    let d = hostile_def(script);
    let tech = Technology::default();

    let starved = SandboxLimits {
        max_operations: 5,
        ..SandboxLimits::default()
    };
    let err = produce(&d, &json!({}), &tech, starved)
        .expect_err("5 operations cannot possibly run a 1000-iteration loop");
    assert!(matches!(err, ProduceError::LimitExceeded(_)));

    let generous = SandboxLimits {
        max_operations: 10_000_000,
        ..SandboxLimits::default()
    };
    produce(&d, &json!({}), &tech, generous)
        .expect("the same, finite, bounded script must complete under a generous cap");
}

/// A script that creates exactly 20 cells: `max_cells = 20` must accept it (the cap is
/// inclusive), and `max_cells = 19` must reject it. Neither the producer's own tests nor an
/// "eventually rejects" check proves the boundary is exact; this does.
#[test]
fn cell_output_cap_boundary_is_exact() {
    let script = r#"
    let i = 0;
    while i < 20 {
        create_cell("cell_" + i);
        i += 1;
    }
    set_top_cells(["cell_0"]);
    "#;
    let d = hostile_def(script);
    let tech = Technology::default();
    let ample_ops = 10_000_000;

    let at_cap = SandboxLimits {
        max_operations: ample_ops,
        max_shapes: 1_000_000,
        max_cells: 20,
    };
    produce(&d, &json!({}), &tech, at_cap)
        .expect("exactly 20 cells against max_cells=20 must be accepted");

    let over_cap = SandboxLimits {
        max_operations: ample_ops,
        max_shapes: 1_000_000,
        max_cells: 19,
    };
    let err = produce(&d, &json!({}), &tech, over_cap)
        .expect_err("20 cells against max_cells=19 must reject");
    match &err {
        ProduceError::LimitExceeded(msg) => assert!(msg.contains("cell"), "names the bound: {msg}"),
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

/// The same exact-boundary proof as above, for `max_shapes`.
#[test]
fn shape_output_cap_boundary_is_exact() {
    let script = r#"
    create_cell("S");
    let i = 0;
    while i < 20 {
        add_rect("S", 1, 0, 0, 0, 10, 10);
        i += 1;
    }
    set_top_cells(["S"]);
    "#;
    let d = hostile_def(script);
    let tech = Technology::default();
    let ample_ops = 10_000_000;

    let at_cap = SandboxLimits {
        max_operations: ample_ops,
        max_shapes: 20,
        max_cells: 10,
    };
    produce(&d, &json!({}), &tech, at_cap)
        .expect("exactly 20 shapes against max_shapes=20 must be accepted");

    let over_cap = SandboxLimits {
        max_operations: ample_ops,
        max_shapes: 19,
        max_cells: 10,
    };
    let err = produce(&d, &json!({}), &tech, over_cap)
        .expect_err("20 shapes against max_shapes=19 must reject");
    match &err {
        ProduceError::LimitExceeded(msg) => {
            assert!(msg.contains("shape"), "names the bound: {msg}");
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

/// FINDING (pcell-harness, see `RESULT.md`): `over_output_caps` sums stored shape records
/// (`Cell::shapes`) and counts cells, but never sums a cell's `instances` or `arrays` Vecs.
/// A script that loops calling `add_instance` (or `add_array`) therefore stores an unbounded
/// number of `Instance` / `ArrayInstance` records -- each a real heap allocation, and each
/// also duplicated into `EditableDocument`'s permanent, unbounded undo log -- while staying
/// completely within `max_shapes` and `max_cells`. The only thing standing between this and
/// unbounded memory growth is the operation cap, which a caller has no particular reason to
/// tighten: the documented purpose of `max_shapes`/`max_cells` is exactly "how much output"
/// (`SandboxLimits`'s own doc comment), and this script's *stored* output (one shape, two
/// cells) is trivially within any reasonable cap.
///
/// This test encodes the invariant the sandbox *should* hold (over-cap instance/array output
/// is rejected, matching how over-cap shape output already is, proven above) and currently
/// fails against the merged producer: `produce` returns `Ok` with 20,000 stored instances and
/// 20,000 stored arrays despite `max_shapes = 10` / `max_cells = 10`. Do not delete or weaken
/// this test if it starts passing -- that means the gap was closed.
#[test]
fn instance_and_array_output_is_not_bounded_by_max_shapes_or_max_cells() {
    let script = r#"
    create_cell("LEAF");
    add_rect("LEAF", 1, 0, 0, 0, 1, 1);
    create_cell("TOP");
    let i = 0;
    while i < 10000 {
        add_instance("TOP", "LEAF", i, 0);
        add_array("TOP", "LEAF", i, 100, 2, 2, 5, 5);
        i += 1;
    }
    set_top_cells(["TOP"]);
    "#;
    let d = hostile_def(script);
    // A caller who wants a small produced document sets max_shapes/max_cells tight; the
    // operation cap is left generous on purpose, so the operation cap is definitely not
    // what is (or should be) doing the rejecting in this test.
    let limits = SandboxLimits {
        max_operations: 10_000_000,
        max_shapes: 10,
        max_cells: 10,
    };
    let err = produce(&d, &json!({}), &Technology::default(), limits).expect_err(
        "PRODUCER DEFECT (pcell-harness finding, see RESULT.md): add_instance/add_array \
         output is not counted by max_shapes/max_cells, so a script that stores 10,000 \
         Instance records and 10,000 ArrayInstance records on one cell succeeds when the \
         output cap should reject it",
    );
    assert!(matches!(err, ProduceError::LimitExceeded(_)));
}

// ---------------------------------------------------------------------------------------
// Host isolation
// ---------------------------------------------------------------------------------------

/// A battery of filesystem/import escape attempts, each of which must fail as a clean
/// `ProduceError::Script`, never a panic and never actual disk I/O. A real marker file is
/// seeded beforehand and re-read afterward (byte-for-byte) so this proves no host escape
/// occurred, not merely that the call returned an error.
#[test]
fn filesystem_and_import_access_attempts_touch_nothing_on_disk() {
    let marker_dir = std::env::temp_dir().join("reticle-pcell-harness-marker");
    std::fs::create_dir_all(&marker_dir).expect("create scratch marker dir");
    let marker_path = marker_dir.join("untouched.txt");
    std::fs::write(&marker_path, b"before").expect("seed marker file");
    // Forward-slash form for embedding in a rhai string literal: sidesteps Windows
    // backslashes being read as rhai escape sequences inside a double-quoted literal.
    let marker_str = marker_path.to_string_lossy().replace('\\', "/");

    let attempts: Vec<String> = vec![
        r#"import "../../../../etc/passwd" as e; e::run();"#.to_owned(),
        r#"import "evil_plugin" as e; e::run();"#.to_owned(),
        format!(r#"open("{marker_str}");"#),
        format!(r#"read_file("{marker_str}");"#),
        format!(r#"write_file("{marker_str}", "pwned");"#),
        format!(r#"std::fs::write("{marker_str}", "pwned");"#),
        format!(r#"read_to_string("{marker_str}");"#),
    ];

    let tech = Technology::default();
    for script in &attempts {
        let d = hostile_def(script);
        let err = produce(&d, &json!({}), &tech, SandboxLimits::default()).expect_err(&format!(
            "no host function reaches the filesystem: {script:?}"
        ));
        assert!(
            matches!(err, ProduceError::Script(_)),
            "expected a clean Script error for {script:?}, got {err:?}"
        );
    }

    let after = std::fs::read(&marker_path).expect("marker file must still be readable");
    assert_eq!(
        after, b"before",
        "marker file must be byte-for-byte untouched by every attempt"
    );
    let _ = std::fs::remove_file(&marker_path);
}

/// `rhai`'s own built-in `eval` (if reachable at all) shares the same engine and call stack
/// as the outer script, so an infinite loop hidden behind it must still be caught by the same
/// operation-cap `on_progress` callback rather than escaping the bound. Whichever of the two
/// clean outcomes holds (blocked outright as a `Script` error, or reached and then capped as
/// `LimitExceeded`), the one thing that must never happen is an uncapped hang.
#[test]
fn nested_eval_is_still_bound_by_the_operation_cap() {
    let d = hostile_def(r#"eval("let i = 0; loop { i += 1; }");"#);
    let limits = SandboxLimits {
        max_operations: 100_000,
        ..SandboxLimits::default()
    };

    let start = Instant::now();
    let result = produce(&d, &json!({}), &Technology::default(), limits);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_secs(5),
        "an eval'd loop must still be bounded, took {elapsed:?}"
    );
    match result {
        Err(ProduceError::LimitExceeded(_) | ProduceError::Script(_)) => {}
        other => panic!("expected a clean, bounded rejection, got {other:?}"),
    }
}

/// Each `produce` call must build a fresh, isolated host and engine (per the module docs):
/// a cell created by one run must not be visible to a later, unrelated run. This drives the
/// query from *within* the second script (produce returns only the extracted top cell, not
/// the whole document, so the host cannot observe this any other way) and encodes the answer
/// into that cell's own geometry.
#[test]
fn produce_runs_never_leak_state_between_each_other() {
    let tech = Technology::default();
    let leaker = hostile_def(r#"create_cell("LEAK"); set_top_cells(["LEAK"]);"#);
    produce(&leaker, &json!({}), &tech, SandboxLimits::default()).expect("first run produces LEAK");

    let prober = hostile_def(
        r#"
        create_cell("OTHER");
        let leaked = has_cell("LEAK");
        add_rect("OTHER", 9, 0, 0, 0, if leaked { 1 } else { 0 }, 1);
        set_top_cells(["OTHER"]);
        "#,
    );
    let (cell, _meta) = produce(&prober, &json!({}), &tech, SandboxLimits::default())
        .expect("second run produces OTHER");

    assert_eq!(cell.name, "OTHER");
    match &cell.shapes[0].kind {
        ShapeKind::Rect(r) => assert_eq!(
            r.max.x, 0,
            "LEAK from an earlier, unrelated produce() call must not be visible to this one"
        ),
        other => panic!("expected a rect, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------
// Cache integration (reticle_gen::PCellCache)
// ---------------------------------------------------------------------------------------

/// Produces `(def, params)` through `cache`: a hit returns the cached geometry without
/// invoking `produce` again; a miss produces, inserts under the stamped `param_hash`, and
/// returns the fresh geometry. Mirrors the integration the `pcell-cache` and `pcell-produce`
/// lanes were built to support (ADR 0107): before this harness, nothing in the workspace
/// actually wired `produce` and `PCellCache` together end to end.
///
/// The lookup key is the hash of the RAW `params` this function was handed (there is no
/// public way to compute `produce`'s EFFECTIVE, schema-defaulted hash without calling
/// `produce` itself); the insert key is always `meta.param_hash`, the authoritative hash
/// `produce` actually stamped. The two are equal whenever `params` already spells out every
/// schema field explicitly, but need not be otherwise -- see
/// `raw_param_keyed_caching_never_hits_when_a_defaulted_field_is_omitted` below, which
/// exercises exactly that gap rather than assuming it away.
fn produce_cached(
    cache: &mut PCellCache,
    def: &PCellDef,
    params: &Value,
    tech: &Technology,
    limits: SandboxLimits,
) -> Result<Cell, ProduceError> {
    let raw_hash = def.param_hash(params);
    if let Some(cell) = cache.get(&raw_hash) {
        return Ok(cell);
    }
    let (cell, meta) = produce(def, params, tech, limits)?;
    cache.insert(meta.param_hash, cell.clone());
    Ok(cell)
}

/// Producing the same `(def, params)` a second time through `PCellCache` must hit, and the
/// cached geometry must be byte-identical to a fresh, uncached `produce` call.
#[test]
fn cache_hit_on_repeated_produce_of_the_same_def_and_params_matches_fresh_geometry() {
    let def = grid_def();
    let params = json!({ "unit": 25, "steps": 3, "pitch": 60 });
    let tech = Technology::default();
    let mut cache = PCellCache::new();

    let first = produce_cached(&mut cache, &def, &params, &tech, SandboxLimits::default())
        .expect("first produce must miss and succeed");
    assert_eq!(cache.stats().misses, 1);
    assert_eq!(cache.stats().hits, 0);

    let (fresh, _meta) = produce(&def, &params, &tech, SandboxLimits::default())
        .expect("an independent, uncached produce for comparison");

    let second = produce_cached(&mut cache, &def, &params, &tech, SandboxLimits::default())
        .expect("second produce must hit");
    assert_eq!(
        cache.stats().hits,
        1,
        "second identical call must be a cache hit"
    );
    assert_eq!(cache.stats().misses, 1, "miss count must not grow on a hit");

    assert_eq!(
        first, fresh,
        "first (miss-path) geometry must match a fresh, uncached produce"
    );
    assert_eq!(
        second, fresh,
        "cache-hit geometry must be byte-identical to a fresh, uncached produce"
    );
}

/// Two different `PCellDef`s with numerically similar params must occupy two distinct cache
/// entries: the hash (and therefore the cache key) is specific to the def's `id`, not just
/// the shape of the params value.
#[test]
fn cache_keys_are_specific_to_the_def_not_just_the_params_value() {
    let grid = grid_def();
    let frame = frame_def();
    let tech = Technology::default();
    let mut cache = PCellCache::new();

    let grid_params = json!({ "unit": 10, "steps": 2, "pitch": 30 });
    let frame_params = json!({ "w": 10, "h": 2, "t": 30 }); // deliberately similar numbers

    let grid_hash = grid.param_hash(&grid_params);
    let frame_hash = frame.param_hash(&frame_params);
    assert_ne!(
        grid_hash, frame_hash,
        "different generator_id must change the hash even with similar-shaped params"
    );

    produce_cached(
        &mut cache,
        &grid,
        &grid_params,
        &tech,
        SandboxLimits::default(),
    )
    .expect("grid produce");
    produce_cached(
        &mut cache,
        &frame,
        &frame_params,
        &tech,
        SandboxLimits::default(),
    )
    .expect("frame produce");
    assert_eq!(
        cache.len(),
        2,
        "two distinct defs must occupy two distinct cache entries"
    );
}

/// Forcing eviction under capacity pressure (real produced geometry flowing through the
/// cache, not the synthetic placeholder `Cell`s `cache.rs`'s own unit tests use) must never
/// corrupt content: re-requesting an evicted entry re-produces it, and the result must still
/// match the original geometry exactly.
#[test]
fn eviction_under_capacity_pressure_never_corrupts_recomputed_geometry() {
    let def = grid_def();
    let tech = Technology::default();
    let mut cache = PCellCache::with_capacity(2);

    let params: Vec<Value> = (1..=5)
        .map(|n| json!({ "unit": 10 * n, "steps": 2, "pitch": 40 }))
        .collect();

    let mut first_run = Vec::new();
    for p in &params {
        first_run.push(
            produce_cached(&mut cache, &def, p, &tech, SandboxLimits::default())
                .unwrap_or_else(|e| panic!("first pass for {p}: {e}")),
        );
    }
    assert!(
        cache.stats().evictions > 0,
        "5 entries through a capacity-2 cache must evict"
    );
    assert_eq!(cache.len(), 2, "cache must never exceed its capacity");

    for (p, expected) in params.iter().zip(&first_run) {
        let again = produce_cached(&mut cache, &def, p, &tech, SandboxLimits::default())
            .unwrap_or_else(|e| panic!("second pass for {p}: {e}"));
        assert_eq!(
            &again, expected,
            "re-produced-after-eviction geometry must match the original for {p}"
        );
    }
}

/// FINDING (integration seam, not a `pcell-produce` sandbox defect; see RESULT.md):
/// `produce`'s stamped `param_hash` (the cache-insert key) is over the EFFECTIVE,
/// schema-defaulted params, but there is no public API anywhere in `reticle_gen` to compute
/// that same effective hash BEFORE calling `produce` -- `PCellDef::param_hash` only hashes
/// whatever raw value it is given, and the defaulting logic (`sandbox::effective_params`) is
/// private to `reticle_script`. So the only hash a cache-aware caller can pre-compute for a
/// lookup is over the RAW params they were handed.
///
/// This test calls `produce_cached` with the exact same (def, params) TWICE -- literally the
/// scenario the brief requires a hit for -- but the params happen to omit a defaulted field.
/// The first call misses (empty cache) as expected; the SECOND, IDENTICAL call also misses,
/// because the first call's insert key (the effective hash) differs from the raw hash both
/// calls look up by. A caller who consistently omits a defaulted field therefore never
/// benefits from the cache at all, even calling with byte-identical input twice. Nothing is
/// corrupted (both calls still produce byte-identical, correct geometry, checked below) --
/// this is a missed-hit / cache-defeat gap, not a wrong-answer bug. Reported for whichever
/// lane owns `PCellDef` (a public effective-params accessor would let a caller key the cache
/// correctly without calling `produce` first), not `pcell-produce`.
#[test]
fn raw_param_keyed_caching_never_hits_when_a_defaulted_field_is_omitted() {
    let def = frame_def();
    let tech = Technology::default();
    let mut cache = PCellCache::new();

    // `t` is omitted every time (schema default 50 fills it in); this is the SAME params
    // value on both calls, which is exactly "the same (def, params) twice."
    let omitted = json!({ "w": 1000, "h": 1000 });

    let cell_a = produce_cached(&mut cache, &def, &omitted, &tech, SandboxLimits::default())
        .expect("first produce");
    let cell_b = produce_cached(&mut cache, &def, &omitted, &tech, SandboxLimits::default())
        .expect("second produce, with the identical params as the first call");

    // Both calls really are the same produced instance: nothing is corrupted or stale.
    assert_eq!(
        cell_a, cell_b,
        "identical (def, params) must still be the same instance"
    );

    assert_eq!(
        cache.stats().hits,
        1,
        "INTEGRATION GAP (pcell-harness finding, see RESULT.md): calling produce_cached \
         with the exact same (def, params) twice should hit the second time, but a \
         raw-params-keyed lookup misses both times because produce's insert key is the \
         EFFECTIVE (schema-defaulted) hash, which nothing outside reticle_script can \
         pre-compute -- there is no public effective-params accessor to key a pre-produce \
         cache lookup with"
    );
}

// ---------------------------------------------------------------------------------------
// Zero panic on malformed input
// ---------------------------------------------------------------------------------------

/// An independent hostile-SCRIPT battery: deliberately NOT the same cases as the producer's
/// own `adversarial_inputs_never_panic` suite (this harness does not trust it and re-derives
/// its own). Reaching the end of the loop without a panic is the entire assertion; the
/// `Result` is otherwise discarded.
#[test]
fn independent_hostile_script_corpus_never_panics() {
    let tight = SandboxLimits {
        max_operations: 200_000,
        max_shapes: 5_000,
        max_cells: 200,
    };
    let tech = Technology::default();

    let scripts: &[&str] = &[
        // Unicode identifiers, escapes, and cell names.
        "create_cell(\"\u{1F600}\"); set_top_cells([\"\u{1F600}\"]);",
        "create_cell(\"a\\u{0}b\");",
        // Deep MUTUAL function-call nesting (distinct from straight self-recursion): tests
        // the call-depth cap via a cycle of three functions rather than one.
        "fn a(x){b(x)} fn b(x){c(x)} fn c(x){a(x+1)} a(0);",
        // Arity / type confusion against the registered API.
        "add_rect();",
        "add_rect(1, 2, 3, 4, 5, 6, 7);",
        "create_cell(123);",
        "add_array(\"A\", \"B\", 0, 0, -1, -1, 1, 1);",
        "add_polygon(\"A\", 1, 0, [1, 2, 3]);", // odd-length point array
        "add_path(\"A\", 1, 0, 10, [1]);",      // too few points
        "run_drc(12345);",
        "load_technology(12345);",
        "export_gds(1, 2, 3);",
        // Arithmetic edge cases at the Dbu/i64 boundary.
        "create_cell(\"C\"); add_rect(\"C\", 1, 0, 9223372036854775807, 0, 1, 1); set_top_cells([\"C\"]);",
        "create_cell(\"C\"); add_rect(\"C\", 1, 0, -9223372036854775808, 0, 1, 1); set_top_cells([\"C\"]);",
        // Empty / whitespace / comment-only sources.
        "",
        "   \t\n  ",
        "// just a comment\n/* block */",
        // A cell instancing itself (a cyclic hierarchy produce never flattens, but the
        // output-cap and extraction paths must still not choke on it).
        "create_cell(\"A\"); add_instance(\"A\", \"A\", 0, 0); set_top_cells([\"A\"]);",
    ];

    for script in scripts {
        let d = hostile_def(script);
        let _ = produce(&d, &json!({}), &tech, tight);
    }
}

/// An independent hostile-PARAMS battery, paired with a def that actually declares a field
/// (unlike `hostile_def`'s empty schema, which never looks anything up in `params`, making
/// its shape irrelevant): every weird top-level params shape must still error or succeed
/// cleanly, never panic.
#[test]
fn independent_hostile_params_corpus_never_panics() {
    let d = def_of(
        "harness.one_field",
        vec![int_field("w", 10, 100_000)],
        r#"create_cell("C"); add_rect("C", 1, 0, 0, 0, w, w); set_top_cells(["C"]);"#,
    );
    let tech = Technology::default();

    let cases: &[Value] = &[
        Value::Null,
        json!([1, 2, 3]),
        json!("not an object"),
        json!({ "w": "not a number" }),
        json!({ "w": [1, 2, 3] }),
        json!({ "w": { "nested": { "deep": 1 } } }),
        json!({ "w": 9_999_999_999_i64 }),
        json!({ "w": -1 }),
        json!({ "unrelated": { "a": [1, 2, { "b": true }] } }),
        json!({}),
    ];

    for params in cases {
        let _ = produce(&d, params, &tech, SandboxLimits::default());
    }
}

/// Rust panics on integer overflow in a debug build; `rhai`'s own `INT` arithmetic must be
/// checked (raising a script error) rather than relying on the native operator, or this would
/// panic the whole process -- fatal in wasm, which kills the tab (exactly the failure mode
/// the sandbox exists to prevent). This drives that boundary directly rather than assuming
/// `rhai` is safe. A panic here surfaces as this test failing (Rust's default `panic=unwind`
/// test harness catches it per-test), not a crashed process, so it is safe to check for real.
#[test]
fn integer_overflow_arithmetic_in_script_is_a_clean_error_not_a_panic() {
    let d = hostile_def("let x = 9223372036854775807; let y = x + 1; create_cell(\"C\" + y);");
    let _ = produce(
        &d,
        &json!({}),
        &Technology::default(),
        SandboxLimits::default(),
    );
}

/// A script of exactly the pre-parse size cap must not be rejected for its size; one byte
/// over must be rejected specifically for it. Mirrors the private `sandbox::MAX_SCRIPT_BYTES`
/// (inaccessible to this integration test) as a literal, so this test also acts as a canary:
/// if that constant is ever silently changed, this boundary check will change behavior too.
#[test]
fn script_source_size_boundary_is_exact() {
    const MAX_SCRIPT_BYTES: usize = 1_000_000;
    let tail = r#"create_cell("C"); set_top_cells(["C"]);"#;
    let pad_len = MAX_SCRIPT_BYTES - tail.len() - 4; // 4 bytes of comment fence: "/*" + "*/"
    let script_at_cap = format!("/*{}*/{tail}", "x".repeat(pad_len));
    assert_eq!(
        script_at_cap.len(),
        MAX_SCRIPT_BYTES,
        "test construction sanity check"
    );

    let d = hostile_def(&script_at_cap);
    let result = produce(
        &d,
        &json!({}),
        &Technology::default(),
        SandboxLimits::default(),
    );
    assert!(
        !matches!(&result, Err(ProduceError::LimitExceeded(m)) if m.contains("byte")),
        "a script at exactly the byte cap must not be rejected for its size, got {result:?}"
    );

    let script_over_cap = format!("{script_at_cap} ");
    let d_over = hostile_def(&script_over_cap);
    let err = produce(
        &d_over,
        &json!({}),
        &Technology::default(),
        SandboxLimits::default(),
    )
    .expect_err("one byte over the script-size cap must be rejected");
    match &err {
        ProduceError::LimitExceeded(msg) => assert!(msg.contains("byte"), "names the bound: {msg}"),
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

/// Builds a value nested `depth` levels deep in single-element arrays around a scalar `0`
/// (`depth` 0 is the bare scalar `0` itself).
fn nested_array(depth: u32) -> Value {
    let mut v = json!(0);
    for _ in 0..depth {
        v = json!([v]);
    }
    v
}

/// A parameter value nested exactly at the depth cap must not be rejected for nesting depth;
/// one level deeper must be. Mirrors the private `sandbox::MAX_PARAM_DEPTH` as a literal (see
/// `script_source_size_boundary_is_exact` for why that is an acceptable black-box mirror).
/// The params object itself (`{ "w": ... }`) is one nesting level, so the array chain under
/// `w` may only go one level less deep than the raw cap before the combined nesting trips it.
#[test]
fn param_nesting_depth_boundary_is_exact() {
    const MAX_PARAM_DEPTH: u32 = 32;
    let d = def_of(
        "harness.deep",
        vec![int_field("w", 1, 10)],
        "create_cell(\"C\"); set_top_cells([\"C\"]);",
    );
    let tech = Technology::default();

    let at_cap = json!({ "w": nested_array(MAX_PARAM_DEPTH - 1) });
    let result = produce(&d, &at_cap, &tech, SandboxLimits::default());
    assert!(
        !matches!(&result, Err(ProduceError::InvalidParams(m)) if m.contains("depth")),
        "exactly at the depth cap must not be rejected for nesting depth, got {result:?}"
    );

    let over_cap = json!({ "w": nested_array(MAX_PARAM_DEPTH) });
    let err = produce(&d, &over_cap, &tech, SandboxLimits::default())
        .expect_err("one level past the depth cap must be rejected");
    match &err {
        ProduceError::InvalidParams(msg) => {
            assert!(msg.contains("depth"), "names the bound: {msg}");
        }
        other => panic!("expected InvalidParams, got {other:?}"),
    }
}

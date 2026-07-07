//! Regression pins for inputs the v8 fuzz campaign crashed the GDSII importer
//! with. Each fixture under `tests/fuzz-regressions/gds/` is a minimized
//! libFuzzer artifact (all were the gds21/chrono out-of-range-date panic).
//!
//! The assertion here is deliberately stronger than "returns `Err`": import
//! must complete with NO panic AT ALL, not even one contained by
//! `catch_unwind`, because on `wasm32-unknown-unknown` a panic aborts the
//! instance and cannot be caught. The panic hook fires for contained panics
//! too, so counting hook invocations distinguishes "truly panic-free" from
//! "panicked but caught". Before the `sanitize_date_records` fix this test
//! fails with a nonzero hook count; that is the seeded-bad half of the
//! two-way contract.

use std::sync::atomic::{AtomicUsize, Ordering};

use reticle_io::Gds;
use reticle_model::Importer;

static PANICS: AtomicUsize = AtomicUsize::new(0);

#[test]
fn fuzz_crash_fixtures_import_without_any_panic() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fuzz-regressions/gds");
    let mut fixtures: Vec<_> = std::fs::read_dir(dir)
        .expect("fixture dir exists")
        .map(|e| e.expect("readable entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "gds"))
        .collect();
    fixtures.sort();
    assert!(
        !fixtures.is_empty(),
        "no committed fixtures found under {dir}"
    );

    // Count every panic raised anywhere in-process, including ones later
    // caught. The previous hook is kept silent for the duration so contained
    // panics do not spray backtraces into test output.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {
        PANICS.fetch_add(1, Ordering::SeqCst);
    }));

    let mut outcomes = Vec::new();
    for path in &fixtures {
        let bytes = std::fs::read(path).expect("fixture readable");
        let outcome = Gds.import(&bytes);
        outcomes.push((path.clone(), outcome.map(|_| ()).map_err(|e| e.to_string())));
    }

    std::panic::set_hook(previous);
    let panics = PANICS.swap(0, Ordering::SeqCst);

    assert_eq!(
        panics, 0,
        "import panicked (even if contained) on at least one fixture; on wasm \
         that panic would abort the instance. Outcomes: {outcomes:?}"
    );
}

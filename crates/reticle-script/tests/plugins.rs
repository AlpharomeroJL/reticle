//! Tests for the plugin-directory convention ([`ScriptEngine::run_plugin_dir`]).
//!
//! One test runs the crate's own worked example scripts under `examples/`; another
//! writes numbered scripts into a temporary directory and asserts they run in
//! sorted filename order against one shared document.

use std::path::PathBuf;

use reticle_script::ScriptEngine;

/// The worked example scripts under `examples/` load and run, producing the cells
/// they build.
#[test]
fn runs_worked_examples() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples");
    let mut engine = ScriptEngine::new();
    let ran = engine
        .run_plugin_dir(&dir)
        .expect("examples should run cleanly");
    assert_eq!(ran, 2, "two example scripts present");

    let doc = engine.document();
    // param_cell.rhai builds these.
    assert!(doc.cell("PIXEL").is_some(), "PIXEL cell from param_cell");
    assert!(doc.cell("SENSOR").is_some(), "SENSOR cell from param_cell");
    // The 8x6 array of a 2-shape pixel flattens to 96 shapes.
    assert_eq!(doc.flatten("SENSOR").len(), 96);
    // drc_sweep.rhai builds this.
    assert!(
        doc.cell("DRC_TEST").is_some(),
        "DRC_TEST cell from drc_sweep"
    );
}

/// Scripts in a plugin directory run in sorted (lexicographic) filename order.
#[test]
fn plugin_dir_runs_in_sorted_order() {
    let tmp = std::env::temp_dir().join(format!("reticle-script-plugins-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create temp plugin dir");

    // Written out of order; must execute 10, then 20, then 30.
    std::fs::write(
        tmp.join("20_shape.rhai"),
        r#"add_rect("TOP", 1, 0, 0, 0, 10, 10);"#,
    )
    .unwrap();
    std::fs::write(tmp.join("10_cell.rhai"), r#"create_cell("TOP");"#).unwrap();
    std::fs::write(
        tmp.join("30_more.rhai"),
        r#"add_rect("TOP", 1, 0, 10, 10, 20, 20);"#,
    )
    .unwrap();
    // A non-.rhai file must be ignored.
    std::fs::write(tmp.join("notes.txt"), "ignore me").unwrap();

    let mut engine = ScriptEngine::new();
    let ran = engine.run_plugin_dir(&tmp).expect("plugin dir runs");
    assert_eq!(ran, 3, "three .rhai files, the .txt ignored");

    let doc = engine.document();
    let top = doc.cell("TOP").expect("TOP created by 10_cell.rhai");
    assert_eq!(top.shapes.len(), 2, "both shape scripts ran after the cell");

    std::fs::remove_dir_all(&tmp).ok();
}

/// A missing plugin directory returns an I/O error rather than panicking.
#[test]
fn missing_plugin_dir_errors() {
    let mut engine = ScriptEngine::new();
    let result = engine.run_plugin_dir("this/directory/does/not/exist");
    assert!(result.is_err(), "missing dir should error");
}

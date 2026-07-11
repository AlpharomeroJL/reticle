// SPDX-License-Identifier: MIT OR Apache-2.0
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

//! Reticle desktop shell (ADR 0119): a Tauri window on the system webview
//! (WebView2 on Windows) that loads the offline Trunk-built web bundle, and the
//! native-only home for features the browser build honestly defers (ADR 0115):
//! the rhai PCell producer (`reticle_script::produce`) today, and, on the
//! roadmap, the real agent (`reticle_agent`).
//!
//! # What this binary does
//!
//! - Opens one native window whose webview loads the assets embedded at
//!   *compile time* from `../crates/web/dist` (`tauri.conf.json`'s
//!   `build.frontendDist`; run `just web-build` first to produce that
//!   directory). Tauri's asset protocol serves those embedded bytes from the
//!   compiled binary at run time; nothing is fetched over the network and no
//!   local server is started, so the window opens and the bundled app boots
//!   identically with the network adapter off. See `docs/src/desktop.md`.
//! - Adds a native "Reticle" menu with one action, "Regenerate demo PCell
//!   (native, offline)". Clicking it runs the REAL sandboxed rhai producer
//!   (`reticle_script::produce`, ADR 0102/0107) against a small built-in
//!   fixture and shows the result in an alert inside the webview. This is the
//!   concrete, orchestrator-drivable proof that live, native-only produce runs
//!   here: the browser build cannot run it at all (ADR 0115) and instead shows
//!   only the predicted provenance.
//!
//! `reticle-agent` (the real propose-verify-correct agent) is a declared
//! dependency of this crate for the same native-only-home reason (see
//! Cargo.toml), but is not yet wired to a menu action: its live run mode needs
//! a reachable model backend (network, or a local server), which is out of
//! scope for the network-disabled offline proof this lane targets. Wiring an
//! agent action is follow-on work (see `scratch/lanes/tauri/RESULT.md`).

use reticle_gen::{FieldSchema, PCellDef, ParamSchema};
use reticle_model::Technology;
use reticle_script::{SandboxLimits, produce};
use tauri::Manager;
use tauri::menu::{MenuBuilder, SubmenuBuilder};

/// The bundled PCell fixture for the native-produce proof: a small parametric
/// pixel array. This is the `reticle-script/examples/param_cell.rhai` body
/// (that crate's own committed worked example) with its leading `let`
/// parameter block replaced by bare identifiers, exactly as
/// `reticle_script::produce` expects: each schema field name is injected into
/// the script's scope from the effective parameters before it runs.
///
/// This mirrors the equivalent fixture already proven in-tree
/// (`reticle_app::pcell_panel`'s browser demo, and
/// `reticle_script::pcell::tests::sensor_def`), duplicated here as a small
/// literal rather than imported: neither is a public API of its crate (the
/// former is a private fn of a GUI crate this crate deliberately does not
/// depend on; the latter is test-only code).
const SENSOR_SCRIPT: &str = r#"
create_cell("PIXEL");
add_rect("PIXEL", 1, 0, 0, 0, pixel_w, pixel_h);
let via_lo_x = (pixel_w - via) / 2;
let via_lo_y = (pixel_h - via) / 2;
add_rect("PIXEL", 2, 0, via_lo_x, via_lo_y, via_lo_x + via, via_lo_y + via);
create_cell("SENSOR");
add_array("SENSOR", "PIXEL", 0, 0, columns, rows, pitch_x, pitch_y);
set_top_cells(["SENSOR"]);
"#;

/// A DBU-valued integer field, named the same as its own one-line doc (a demo
/// fixture has no need for richer prose per field).
fn int_field(name: &str, default: i64) -> FieldSchema {
    FieldSchema::int(name, name, default, 0, 1_000_000, "dbu")
}

/// Builds the fixture definition. `engine_version` is stamped from this
/// crate's own `CARGO_PKG_VERSION` so the produced provenance never drifts
/// from what actually shipped in this binary.
fn sensor_def() -> PCellDef {
    PCellDef {
        id: "desktop.demo_sensor".to_owned(),
        title: "Demo sensor (desktop fixture)".to_owned(),
        description: "Bundled offline fixture for the native produce proof (ADR 0119).".to_owned(),
        schema: ParamSchema {
            generator_id: "desktop.demo_sensor".to_owned(),
            title: "Demo sensor".to_owned(),
            description: "A parametric pixel array.".to_owned(),
            fields: vec![
                int_field("pixel_w", 800),
                int_field("pixel_h", 800),
                int_field("via", 200),
                int_field("columns", 8),
                int_field("rows", 6),
                int_field("pitch_x", 1000),
                int_field("pitch_y", 1000),
            ],
        },
        script: SENSOR_SCRIPT.to_owned(),
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// Runs the real sandboxed produce against the bundled fixture's default
/// parameters (an empty params object; every field falls back to its schema
/// default) and formats a human-readable report of the result: success, or a
/// clean rejection. Never panics: `reticle_script::produce` is itself
/// panic-free on any script or parameter input (see its docs), and this
/// function has no unwrap/expect on that result.
fn run_native_produce() -> String {
    let def = sensor_def();
    let params = serde_json::json!({});
    match produce(
        &def,
        &params,
        &Technology::default(),
        SandboxLimits::default(),
    ) {
        Ok((cell, meta)) => format!(
            "Live PCell produce ran NATIVELY (no network, no browser wasm).\n\n\
             Top cell: {}\n\
             Shapes: {}\n\
             Instances: {}\n\
             Arrays: {}\n\
             \n\
             Provenance: generator_id={}, engine_version={}, param_hash={}",
            cell.name,
            cell.shapes.len(),
            cell.instances.len(),
            cell.arrays.len(),
            meta.generator_id,
            meta.engine_version,
            meta.param_hash,
        ),
        Err(error) => format!("Produce was cleanly rejected (no crash): {error}"),
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle();
            let submenu = SubmenuBuilder::new(handle, "Reticle")
                .text(
                    "produce_demo_pcell",
                    "Regenerate demo PCell (native, offline)",
                )
                .separator()
                .quit()
                .build()?;
            let menu = MenuBuilder::new(handle).item(&submenu).build()?;
            app.set_menu(menu)?;
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == "produce_demo_pcell" {
                let report = run_native_produce();
                println!("{report}");
                if let Some(window) = app.get_webview_window("main") {
                    let js_message = serde_json::to_string(&report).unwrap_or_default();
                    let _ = window.eval(format!("alert({js_message});"));
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running the Reticle desktop shell");
}

#[cfg(test)]
mod tests {
    use super::{run_native_produce, sensor_def};

    /// Proves the bundled fixture is well-formed and `reticle_script::produce`
    /// actually runs against it (this is the same call the menu action makes,
    /// exercised here with no GUI/webview needed): a headless check that the
    /// Gate-4 GUI proof's alert text is not a guess.
    #[test]
    fn native_produce_runs_against_the_bundled_fixture() {
        let report = run_native_produce();
        assert!(
            report.starts_with("Live PCell produce ran NATIVELY"),
            "unexpected report: {report}"
        );
        assert!(report.contains("Top cell: SENSOR"));
        assert!(report.contains("Shapes: 0"));
        assert!(report.contains("Instances: 0"));
        assert!(report.contains("Arrays: 1"));
        assert!(report.contains("generator_id=desktop.demo_sensor"));
        assert!(report.contains(&format!("engine_version={}", env!("CARGO_PKG_VERSION"))));
    }

    /// The fixture's default parameters produce the same array shape the
    /// `reticle-script` test suite's equivalent fixture proves
    /// (`pcell::tests::absent_params_fall_back_to_schema_defaults`):
    /// columns=8, rows=6.
    #[test]
    fn fixture_defaults_match_the_documented_shape() {
        let def = sensor_def();
        let (cell, _meta) = reticle_script::produce(
            &def,
            &serde_json::json!({}),
            &reticle_model::Technology::default(),
            reticle_script::SandboxLimits::default(),
        )
        .expect("the bundled fixture is a valid, deterministic script");
        assert_eq!(cell.arrays[0].columns, 8);
        assert_eq!(cell.arrays[0].rows, 6);
    }
}

//! Reticle build automation (`xtask`).
//!
//! Subcommands:
//! - `gen-layout` — write a deterministic chip-like layout as GDSII (by shape
//!   count, layer count, and hierarchy depth).
//! - `capture-media` — regenerate the hero image and demo GIFs (Wave 5).
//! - `perf-check` — compare benchmarks against the committed history (Wave 5).

mod generator;
mod media;
mod perf;

use reticle_io::Gds;
use reticle_model::Exporter;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map_or("", String::as_str) {
        "gen-layout" => gen_layout(&args[1..]),
        "capture-media" => cmd_capture_media(),
        "perf-check" => perf::perf_check(),
        "" => {
            eprintln!("usage: xtask <gen-layout|capture-media|perf-check> [options]");
            ExitCode::FAILURE
        }
        other => {
            eprintln!("unknown xtask subcommand: {other}");
            ExitCode::FAILURE
        }
    }
}

/// Generates a deterministic hierarchical layout and writes it as GDSII (which,
/// unlike the OASIS subset, preserves the array hierarchy compactly).
fn gen_layout(args: &[String]) -> ExitCode {
    let shapes = flag(args, "--shapes")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1_000_000);
    let layers = flag(args, "--layers")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8);
    let depth = flag(args, "--depth")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(3);
    let out = flag(args, "--out").unwrap_or_else(|| "scratch/gen.gds".to_owned());

    let doc = generator::generate_layout(shapes, layers, depth);
    let bytes = match Gds.export(&doc) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("GDSII export failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(parent) = Path::new(&out).parent()
        && !parent.as_os_str().is_empty()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        eprintln!("could not create {}: {err}", parent.display());
        return ExitCode::FAILURE;
    }
    if let Err(err) = std::fs::write(&out, &bytes) {
        eprintln!("write failed: {err}");
        return ExitCode::FAILURE;
    }

    println!(
        "wrote {out}: {} cells, {} bytes on disk, ~{} flattened leaf shapes, top {:?}",
        doc.cell_count(),
        bytes.len(),
        generator::approximate_shape_count(shapes, depth),
        doc.top_cells(),
    );
    ExitCode::SUCCESS
}

/// Handles `capture-media`: render the hero image and browse GIF into `assets/`.
fn cmd_capture_media() -> ExitCode {
    match media::capture(Path::new("assets")) {
        Ok(true) => {
            println!("media capture complete");
            ExitCode::SUCCESS
        }
        Ok(false) => {
            println!("media capture skipped (no GPU adapter)");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("media capture failed: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Returns the value following `name` in `args`, if present.
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg.as_str() == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

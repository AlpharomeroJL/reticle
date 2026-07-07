//! The Reticle headless command-line pipeline.
//!
//! Wave 3 implements a batch pipeline, import, DRC, route, extract, export, and
//! render-to-image, for validation without any CI service. The binary is named
//! `reticle`.
//!
//! `main` is a thin [`clap`] dispatcher: it parses subcommands and arguments and
//! delegates every stage to the testable functions in [`reticle_cli`].

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use reticle_cli::{
    CliError, Format, RenderOutcome, flatten_top_cell, load_document, pick_top_cell, resolve_rules,
    run_convert, run_drc, run_export, run_extract, run_render, run_route, summarize,
    synth_route_request,
};

/// The Reticle headless layout pipeline.
#[derive(Parser, Debug)]
#[command(name = "reticle", version, about, long_about = None)]
struct Cli {
    /// The pipeline stage to run.
    #[command(subcommand)]
    command: Command,
}

/// The pipeline subcommands.
#[derive(Subcommand, Debug)]
enum Command {
    /// Parse a layout file and print a summary (cells, tops, shapes, layers).
    Import {
        /// The layout file to read (`.gds` is GDSII, else OASIS).
        file: PathBuf,
    },
    /// Run design-rule checking on the top cell and print each violation.
    Drc {
        /// The layout file to read.
        file: PathBuf,
        /// Optional technology file supplying the DRC rules.
        #[arg(long)]
        tech: Option<PathBuf>,
    },
    /// Route a couple of synthesized nets into the top cell and print the report.
    Route {
        /// The layout file to read.
        file: PathBuf,
    },
    /// Extract connectivity on the top cell and print the net count and sizes.
    Extract {
        /// The layout file to read.
        file: PathBuf,
    },
    /// Convert a layout file between GDSII and OASIS.
    Export {
        /// The layout file to read.
        file: PathBuf,
        /// The output file to write.
        #[arg(long)]
        out: PathBuf,
        /// The output format; inferred from `--out` when omitted.
        #[arg(long)]
        format: Option<String>,
    },
    /// Convert a GDSII file into a streamable `.rtla` archive.
    Convert {
        /// The GDSII file to read.
        file: PathBuf,
        /// The `.rtla` archive to write.
        out: PathBuf,
    },
    /// Render the top cell offscreen and save it as a PNG.
    Render {
        /// The layout file to read.
        file: PathBuf,
        /// The PNG file to write.
        #[arg(long)]
        out: PathBuf,
        /// Output image width in pixels.
        #[arg(long, default_value_t = 1024)]
        width: u32,
        /// Output image height in pixels.
        #[arg(long, default_value_t = 1024)]
        height: u32,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Dispatches one parsed subcommand, returning the process exit code.
///
/// Most stages exit `0` on success; `drc` exits non-zero when it finds violations.
fn run(command: Command) -> Result<ExitCode, CliError> {
    match command {
        Command::Import { file } => cmd_import(&file),
        Command::Drc { file, tech } => cmd_drc(&file, tech.as_deref()),
        Command::Route { file } => cmd_route(&file),
        Command::Extract { file } => cmd_extract(&file),
        Command::Export { file, out, format } => cmd_export(&file, &out, format.as_deref()),
        Command::Convert { file, out } => cmd_convert(&file, &out),
        Command::Render {
            file,
            out,
            width,
            height,
        } => cmd_render(&file, &out, width, height),
    }
}

/// Handles `reticle import`: print a summary of the file's contents.
fn cmd_import(file: &Path) -> Result<ExitCode, CliError> {
    let doc = load_document(file)?;
    let summary = summarize(&doc);
    println!("file:   {}", file.display());
    println!("format: {}", Format::from_path(file).label());
    println!("cells:  {}", summary.cell_count);
    if summary.top_cells.is_empty() {
        println!("tops:   (none declared)");
    } else {
        println!("tops:   {}", summary.top_cells.join(", "));
    }
    println!("shapes: {}", summary.shape_count);
    println!("instances: {}", summary.instance_count);
    println!("arrays: {}", summary.array_count);
    if summary.layers.is_empty() {
        println!("layers: 0");
    } else {
        let list: Vec<String> = summary
            .layers
            .iter()
            .map(|l| format!("{}/{}", l.layer, l.datatype))
            .collect();
        println!("layers: {} [{}]", summary.layers.len(), list.join(", "));
    }
    Ok(ExitCode::SUCCESS)
}

/// Handles `reticle drc`: run DRC on the top cell and print each violation. Exits
/// non-zero when any violation is found.
fn cmd_drc(file: &Path, tech: Option<&Path>) -> Result<ExitCode, CliError> {
    let doc = load_document(file)?;
    let top = pick_top_cell(&doc)?;
    // Check the whole design: flatten the hierarchy so an arrayed top cell is checked
    // as its real geometry, not just its (often empty) own shapes.
    let doc = flatten_top_cell(&doc, &top);
    let rules = resolve_rules(&doc, tech)?;
    let rule_count = rules.len();
    let violations = run_drc(&doc, &top, rules)?;
    println!("cell:  {top}");
    println!("rules: {rule_count}");
    for v in &violations {
        println!(
            "  [{}] ({},{})-({},{}): {}",
            v.rule,
            v.location.min.x,
            v.location.min.y,
            v.location.max.x,
            v.location.max.y,
            v.message
        );
    }
    println!("violations: {}", violations.len());
    if violations.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

/// Handles `reticle route`: route synthesized nets into the top cell and report.
fn cmd_route(file: &Path) -> Result<ExitCode, CliError> {
    let doc = load_document(file)?;
    let top = pick_top_cell(&doc)?;
    // Route against the flattened design so synthesized nets have real geometry to
    // navigate around, even when the top cell is a pure array of sub-cells.
    let mut doc = flatten_top_cell(&doc, &top);
    let request = synth_route_request(&doc, &top);
    let net_count = request.nets.len();
    let report = run_route(&mut doc, &request);
    println!("cell:   {top}");
    println!("nets:   {net_count}");
    println!("routed: {}", report.routed);
    println!("failed: {}", report.failed);
    println!("length: {} DBU", report.total_length_dbu);
    Ok(ExitCode::SUCCESS)
}

/// Handles `reticle extract`: extract connectivity and print net count and sizes.
fn cmd_extract(file: &Path) -> Result<ExitCode, CliError> {
    let doc = load_document(file)?;
    let top = pick_top_cell(&doc)?;
    // Extract connectivity over the whole design, not just the top cell's own shapes.
    let doc = flatten_top_cell(&doc, &top);
    let (net_count, sizes) = run_extract(&doc, &top)?;
    println!("cell: {top}");
    println!("nets: {net_count}");
    for (i, size) in sizes.iter().enumerate() {
        println!("  net {i}: {size} shapes");
    }
    Ok(ExitCode::SUCCESS)
}

/// Handles `reticle export`: convert the file to another format.
fn cmd_export(file: &Path, out: &Path, format: Option<&str>) -> Result<ExitCode, CliError> {
    let doc = load_document(file)?;
    let format = match format {
        Some(name) => Format::parse(name)?,
        None => Format::from_path(out),
    };
    run_export(&doc, out, format)?;
    println!(
        "wrote {} ({}) from {}",
        out.display(),
        format.label(),
        file.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// Handles `reticle convert`: stream a GDSII file into a streamable `.rtla` archive.
fn cmd_convert(file: &Path, out: &Path) -> Result<ExitCode, CliError> {
    let summary = run_convert(file, out)?;
    let world = summary.world;
    println!("wrote {} from {}", out.display(), file.display());
    println!("records: {}", summary.record_count);
    println!(
        "world:   ({},{})-({},{}) DBU",
        world.min.x, world.min.y, world.max.x, world.max.y
    );
    println!("dbu/µm:  {}", summary.dbu_per_micron);
    println!("levels:  {}", summary.level_count);
    Ok(ExitCode::SUCCESS)
}

/// Handles `reticle render`: render the top cell offscreen to a PNG.
fn cmd_render(file: &Path, out: &Path, width: u32, height: u32) -> Result<ExitCode, CliError> {
    let doc = load_document(file)?;
    let top = pick_top_cell(&doc)?;
    match run_render(&doc, &top, out, width, height)? {
        RenderOutcome::Rendered {
            path,
            width,
            height,
        } => {
            println!("rendered {top} to {} ({width}x{height})", path.display());
        }
        RenderOutcome::NoGpu => {
            println!("no compatible GPU adapter available; skipping render");
        }
    }
    Ok(ExitCode::SUCCESS)
}

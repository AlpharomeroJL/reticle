//! The demo-script format: a committed, human-readable timed step list.
//!
//! One directive per line; blank lines and `#` comments are ignored. A `viewport`
//! header sizes the window; every other directive becomes a [`Step`] the
//! [`DemoRun`](super::run::DemoRun) executes in order, driving the real editor and
//! capturing full-window frames. The format is deliberately small and declarative so
//! each committed script reads as a storyboard and `just capture-ui` reproduces the
//! media byte-for-byte from it.

use crate::usecases::UseCase;

/// A parsed demo script: the window size plus the ordered steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    /// The window inner size in logical pixels (default 1600x1000).
    pub viewport: (u32, u32),
    /// The steps to run in order.
    pub steps: Vec<Step>,
}

/// One demo directive.
///
/// Instantaneous actions (`RunDrc`, `Select`, ...) apply on the frame they are
/// reached; `Capture`/`Snap` record frames; `Wait` idles so an animation or a just
/// applied edit settles before the next capture.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Enter a bundled worked scenario (loads its document or opens the theater).
    UseCase(UseCase),
    /// Idle for this many frames (let an edit or animation settle).
    Wait(u32),
    /// Idle until the rendered frame shows filled colored geometry (a non-blank,
    /// non-starry render), probing up to this many frames before giving up. An explicit
    /// render-settle wait so a capture never starts on an unrendered frame.
    Settle(u32),
    /// Record `frames` frames at `fps` into the current segment (a GIF clip).
    Capture {
        /// Number of frames to record.
        frames: u32,
        /// Playback rate of the assembled clip.
        fps: u32,
    },
    /// Record a single still named `name` (for example the hero image).
    Snap(String),
    /// Run the DRC engine and populate the DRC panel.
    RunDrc,
    /// Select DRC violation `index` in the panel.
    SelectViolation(usize),
    /// Zoom the canvas to the selected violation's marker.
    ZoomViolation,
    /// Select these shape indices (the first replaces the selection, the rest add).
    Select(Vec<usize>),
    /// Highlight the net connected to shape `index`.
    HighlightNet(usize),
    /// Apply a filter-query string (for example `layer:met1 width<400`).
    Filter(String),
    /// Locate the outline node at `path` (a slash-separated cell/instance path).
    OutlineLocate(String),
    /// Open (`true`) or close (`false`) the 3D layer-stack view.
    View3d(bool),
    /// Orbit the 3D camera by `(dx, dy)` radians-ish (passed to `View3d::drag`).
    Orbit(f32, f32),
    /// Zoom the canvas about its center by `factor` (>1 zooms in).
    Zoom(f32),
    /// Pan the canvas by `(dx, dy)` screen pixels.
    Pan(f32, f32),
    /// Draw a polygon through these DBU vertices (the real add-shape edit).
    AddPoly(Vec<(i64, i64)>),
    /// Move vertex `vertex` of shape `shape` by `delta` DBU (the real edit).
    VertexMove {
        /// Index of the shape to edit.
        shape: usize,
        /// Index of the vertex within the shape.
        vertex: usize,
        /// The move delta in DBU.
        delta: (i64, i64),
    },
    /// Boolean-union the current selection into one shape.
    Union,
    /// Array-duplicate the current selection into a `cols` x `rows` grid at `pitch` DBU.
    Array {
        /// Columns in the array.
        cols: u32,
        /// Rows in the array.
        rows: u32,
        /// Pitch between copies in DBU.
        pitch: i64,
    },
    /// Select the Generate-panel generator with this id (for example `guard_ring`), so
    /// following steps drive its form. Scrolls the Generate panel into view.
    Generator(String),
    /// Set integer parameter `name` of the selected generator to `value`.
    GenParam {
        /// The schema field name.
        name: String,
        /// The integer value to set.
        value: i64,
    },
    /// Place the selected generator's geometry into the top cell as one undo step (the
    /// real `RunGenerator` path the Generate button uses).
    GenPlace,
}

impl Script {
    /// The default window size when a script omits a `viewport` header.
    pub const DEFAULT_VIEWPORT: (u32, u32) = (1600, 1000);

    /// Parses a demo script.
    ///
    /// # Errors
    ///
    /// Returns a message naming the offending line if a directive is unknown or its
    /// arguments do not parse.
    pub fn parse(src: &str) -> Result<Self, String> {
        let mut viewport = Self::DEFAULT_VIEWPORT;
        let mut steps = Vec::new();

        for (lineno, raw) in src.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (verb, rest) = split_verb(line);
            let at = || format!("line {}: `{}`", lineno + 1, raw.trim());

            match verb {
                "viewport" => {
                    viewport = parse_viewport(rest).map_err(|e| format!("{}: {e}", at()))?;
                }
                "use-case" => steps.push(Step::UseCase(
                    parse_use_case(rest).map_err(|e| format!("{}: {e}", at()))?,
                )),
                "wait" => steps.push(Step::Wait(
                    parse_u32(rest).map_err(|e| format!("{}: {e}", at()))?,
                )),
                "settle" => steps.push(Step::Settle(
                    parse_settle(rest).map_err(|e| format!("{}: {e}", at()))?,
                )),
                "capture" => steps.push(parse_capture(rest).map_err(|e| format!("{}: {e}", at()))?),
                "snap" => steps.push(Step::Snap(
                    non_empty(rest, "snap needs a name").map_err(|e| format!("{}: {e}", at()))?,
                )),
                "run-drc" => steps.push(Step::RunDrc),
                "select-violation" => steps.push(Step::SelectViolation(
                    parse_usize(rest).map_err(|e| format!("{}: {e}", at()))?,
                )),
                "zoom-violation" => steps.push(Step::ZoomViolation),
                "select" => steps.push(Step::Select(
                    parse_indices(rest).map_err(|e| format!("{}: {e}", at()))?,
                )),
                "highlight-net" => steps.push(Step::HighlightNet(
                    parse_usize(rest).map_err(|e| format!("{}: {e}", at()))?,
                )),
                "filter" => steps.push(Step::Filter(
                    non_empty(rest, "filter needs a query")
                        .map_err(|e| format!("{}: {e}", at()))?,
                )),
                "outline-locate" => steps.push(Step::OutlineLocate(
                    non_empty(rest, "outline-locate needs a path")
                        .map_err(|e| format!("{}: {e}", at()))?,
                )),
                "view3d" => steps.push(Step::View3d(
                    parse_on_off(rest).map_err(|e| format!("{}: {e}", at()))?,
                )),
                "orbit" => {
                    let (dx, dy) = parse_two_f32(rest).map_err(|e| format!("{}: {e}", at()))?;
                    steps.push(Step::Orbit(dx, dy));
                }
                "zoom" => steps.push(Step::Zoom(
                    parse_f32(rest).map_err(|e| format!("{}: {e}", at()))?,
                )),
                "pan" => {
                    let (dx, dy) = parse_two_f32(rest).map_err(|e| format!("{}: {e}", at()))?;
                    steps.push(Step::Pan(dx, dy));
                }
                "add-poly" => steps.push(Step::AddPoly(
                    parse_points(rest).map_err(|e| format!("{}: {e}", at()))?,
                )),
                "vertex-move" => {
                    steps.push(parse_vertex_move(rest).map_err(|e| format!("{}: {e}", at()))?);
                }
                "union" => steps.push(Step::Union),
                "array" => steps.push(parse_array(rest).map_err(|e| format!("{}: {e}", at()))?),
                "generator" => steps.push(Step::Generator(
                    non_empty(rest, "generator needs an id")
                        .map_err(|e| format!("{}: {e}", at()))?,
                )),
                "gen-param" => {
                    steps.push(parse_gen_param(rest).map_err(|e| format!("{}: {e}", at()))?);
                }
                "gen-place" => steps.push(Step::GenPlace),
                other => return Err(format!("{}: unknown directive `{other}`", at())),
            }
        }
        Ok(Script { viewport, steps })
    }
}

/// Splits a directive line into its verb and the (trimmed) remainder.
fn split_verb(line: &str) -> (&str, &str) {
    match line.split_once(char::is_whitespace) {
        Some((verb, rest)) => (verb, rest.trim()),
        None => (line, ""),
    }
}

fn non_empty(s: &str, msg: &str) -> Result<String, String> {
    if s.is_empty() {
        Err(msg.to_owned())
    } else {
        Ok(s.to_owned())
    }
}

fn parse_viewport(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| "viewport needs WxH, e.g. 1600x1000".to_owned())?;
    Ok((parse_u32(w.trim())?, parse_u32(h.trim())?))
}

fn parse_use_case(s: &str) -> Result<UseCase, String> {
    match s {
        "inspect-cell" => Ok(UseCase::InspectCell),
        "find-violation" => Ok(UseCase::FindAndFixViolation),
        "watch-agent" => Ok(UseCase::WatchTheAgent),
        "build" => Ok(UseCase::BuildWithTools),
        other => Err(format!(
            "unknown use-case `{other}` (inspect-cell|find-violation|watch-agent|build)"
        )),
    }
}

fn parse_capture(s: &str) -> Result<Step, String> {
    let mut it = s.split_whitespace();
    let frames = it
        .next()
        .ok_or_else(|| "capture needs a frame count".to_owned())?;
    let frames = parse_u32(frames)?;
    let fps = match it.next() {
        Some(f) => parse_u32(f)?,
        None => 20,
    };
    Ok(Step::Capture { frames, fps })
}

/// The default `settle` probe budget when the directive omits a count (~3 s at 60 fps).
const DEFAULT_SETTLE_FRAMES: u32 = 180;

/// Parses a `settle` budget: the maximum frames to probe for a colored render before
/// proceeding. An empty argument means [`DEFAULT_SETTLE_FRAMES`]; an explicit count is
/// clamped to at least one probe.
fn parse_settle(s: &str) -> Result<u32, String> {
    if s.is_empty() {
        Ok(DEFAULT_SETTLE_FRAMES)
    } else {
        parse_u32(s).map(|n| n.max(1))
    }
}

fn parse_on_off(s: &str) -> Result<bool, String> {
    match s {
        "on" | "true" => Ok(true),
        "off" | "false" => Ok(false),
        other => Err(format!("expected on|off, got `{other}`")),
    }
}

fn parse_indices(s: &str) -> Result<Vec<usize>, String> {
    let v: Result<Vec<usize>, String> = s.split_whitespace().map(parse_usize).collect();
    let v = v?;
    if v.is_empty() {
        return Err("select needs at least one index".to_owned());
    }
    Ok(v)
}

fn parse_points(s: &str) -> Result<Vec<(i64, i64)>, String> {
    let pts: Result<Vec<(i64, i64)>, String> = s
        .split_whitespace()
        .map(|tok| {
            let (x, y) = tok
                .split_once(',')
                .ok_or_else(|| format!("point `{tok}` needs x,y"))?;
            Ok((parse_i64(x)?, parse_i64(y)?))
        })
        .collect();
    let pts = pts?;
    if pts.len() < 3 {
        return Err("add-poly needs at least 3 points".to_owned());
    }
    Ok(pts)
}

fn parse_vertex_move(s: &str) -> Result<Step, String> {
    let mut it = s.split_whitespace();
    let shape = parse_usize(it.next().unwrap_or(""))?;
    let vertex = parse_usize(it.next().unwrap_or(""))?;
    let delta_tok = it.next().unwrap_or("");
    let (dx, dy) = delta_tok
        .split_once(',')
        .ok_or_else(|| format!("vertex-move delta `{delta_tok}` needs dx,dy"))?;
    Ok(Step::VertexMove {
        shape,
        vertex,
        delta: (parse_i64(dx)?, parse_i64(dy)?),
    })
}

fn parse_array(s: &str) -> Result<Step, String> {
    let mut it = s.split_whitespace();
    let cols = parse_u32(it.next().unwrap_or(""))?;
    let rows = parse_u32(it.next().unwrap_or(""))?;
    let pitch = parse_i64(it.next().unwrap_or(""))?;
    Ok(Step::Array { cols, rows, pitch })
}

fn parse_gen_param(s: &str) -> Result<Step, String> {
    let (name, value) = s
        .split_once(char::is_whitespace)
        .ok_or_else(|| "gen-param needs a name and a value".to_owned())?;
    Ok(Step::GenParam {
        name: name.trim().to_owned(),
        value: parse_i64(value.trim())?,
    })
}

fn parse_two_f32(s: &str) -> Result<(f32, f32), String> {
    let mut it = s.split_whitespace();
    let a = parse_f32(it.next().unwrap_or(""))?;
    let b = parse_f32(it.next().unwrap_or(""))?;
    Ok((a, b))
}

fn parse_u32(s: &str) -> Result<u32, String> {
    s.parse::<u32>()
        .map_err(|_| format!("expected an integer, got `{s}`"))
}

fn parse_usize(s: &str) -> Result<usize, String> {
    s.parse::<usize>()
        .map_err(|_| format!("expected an index, got `{s}`"))
}

fn parse_i64(s: &str) -> Result<i64, String> {
    s.parse::<i64>()
        .map_err(|_| format!("expected a whole number, got `{s}`"))
}

fn parse_f32(s: &str) -> Result<f32, String> {
    s.parse::<f32>()
        .map_err(|_| format!("expected a number, got `{s}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_small_script() {
        let src = "\
# a tour of DRC
viewport 1600x1000
use-case find-violation
run-drc
capture 40 20
select-violation 0
zoom-violation
capture 50
";
        let script = Script::parse(src).expect("parse");
        assert_eq!(script.viewport, (1600, 1000));
        assert_eq!(
            script.steps,
            vec![
                Step::UseCase(UseCase::FindAndFixViolation),
                Step::RunDrc,
                Step::Capture {
                    frames: 40,
                    fps: 20
                },
                Step::SelectViolation(0),
                Step::ZoomViolation,
                Step::Capture {
                    frames: 50,
                    fps: 20
                },
            ]
        );
    }

    #[test]
    fn defaults_viewport_and_capture_fps() {
        let script = Script::parse("snap hero").expect("parse");
        assert_eq!(script.viewport, Script::DEFAULT_VIEWPORT);
        let script = Script::parse("capture 30").expect("parse");
        assert_eq!(
            script.steps,
            vec![Step::Capture {
                frames: 30,
                fps: 20
            }]
        );
    }

    #[test]
    fn parses_settle_with_and_without_a_budget() {
        assert_eq!(
            Script::parse("settle").expect("parse").steps,
            vec![Step::Settle(super::DEFAULT_SETTLE_FRAMES)],
        );
        assert_eq!(
            Script::parse("settle 60").expect("parse").steps,
            vec![Step::Settle(60)],
        );
        // A zero budget is clamped to at least one probe.
        assert_eq!(
            Script::parse("settle 0").expect("parse").steps,
            vec![Step::Settle(1)],
        );
    }

    #[test]
    fn filter_keeps_the_whole_query_including_spaces() {
        let script = Script::parse("filter layer:met1 width<400").expect("parse");
        assert_eq!(
            script.steps,
            vec![Step::Filter("layer:met1 width<400".to_owned())]
        );
    }

    #[test]
    fn parses_editing_directives() {
        let src = "\
use-case build
add-poly 0,0 200,0 200,200 0,200
vertex-move 3 2 50,-50
select 0 1
union
array 2 1 400
view3d on
orbit 0.3 0.1
";
        let script = Script::parse(src).expect("parse");
        assert_eq!(
            script.steps,
            vec![
                Step::UseCase(UseCase::BuildWithTools),
                Step::AddPoly(vec![(0, 0), (200, 0), (200, 200), (0, 200)]),
                Step::VertexMove {
                    shape: 3,
                    vertex: 2,
                    delta: (50, -50)
                },
                Step::Select(vec![0, 1]),
                Step::Union,
                Step::Array {
                    cols: 2,
                    rows: 1,
                    pitch: 400
                },
                Step::View3d(true),
                Step::Orbit(0.3, 0.1),
            ]
        );
    }

    #[test]
    fn parses_generator_directives() {
        let src = "\
use-case build
generator via_farm
gen-param rows 4
gen-param cols 4
gen-place
";
        let script = Script::parse(src).expect("parse");
        assert_eq!(
            script.steps,
            vec![
                Step::UseCase(UseCase::BuildWithTools),
                Step::Generator("via_farm".to_owned()),
                Step::GenParam {
                    name: "rows".to_owned(),
                    value: 4
                },
                Step::GenParam {
                    name: "cols".to_owned(),
                    value: 4
                },
                Step::GenPlace,
            ]
        );
    }

    #[test]
    fn rejects_incomplete_gen_param() {
        assert!(Script::parse("gen-param rows").is_err());
        assert!(Script::parse("generator").is_err());
    }

    #[test]
    fn rejects_unknown_directive_with_line_number() {
        let err = Script::parse("run-drc\nfrobnicate 3").unwrap_err();
        assert!(err.contains("line 2"), "{err}");
        assert!(err.contains("frobnicate"), "{err}");
    }

    #[test]
    fn rejects_bad_viewport() {
        assert!(Script::parse("viewport 1600").is_err());
        assert!(Script::parse("add-poly 0,0 1,1").is_err());
    }
}

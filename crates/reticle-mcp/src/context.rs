//! The read-only context tools.
//!
//! These three tools report on the session without a matching
//! [`AgentCommand`]:
//!
//! * [`get_technology_rules`] returns the active technology's DRC rule table as
//!   structured data.
//! * [`get_document_summary`] returns cell counts, per-cell element counts, and
//!   the document bounding box.
//! * [`get_render_region`] renders a region to a PNG and returns it as an inline
//!   base64 data URI, degrading gracefully when no GPU is available.
//!
//! Each returns a `serde_json::Value` payload that the server wraps in an MCP
//! tool result; they never mutate the session.

use serde_json::{Value, json};

use reticle_agent_api::{AgentCommand, AgentResponse, ErrorCode, Session};
use reticle_geometry::Rect;
use reticle_model::RuleKind;

/// The `get_technology_rules` payload: the active technology's rule table.
///
/// Reports the technology name, database resolution, and one entry per DRC rule
/// with its kind, layer(s), and threshold, so a model can reason about the rules
/// before it draws (for example, to space met1 wires at the required pitch).
pub fn get_technology_rules(session: &Session) -> Value {
    let tech = session.document().technology();
    let rules: Vec<Value> = tech
        .rules
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "kind": rule_kind_str(r.kind),
                "layer": layer_json(r.layer),
                "other_layer": r.other_layer.map(layer_json),
                "value": r.value,
                "value_units": value_units(r.kind),
            })
        })
        .collect();
    json!({
        "technology": tech.name,
        "dbu_per_micron": tech.dbu_per_micron,
        "rule_count": rules.len(),
        "rules": rules,
    })
}

/// The `get_document_summary` payload: cells, per-cell counts, and bounding box.
///
/// Iterates the document's cells (sorted by name for a stable, deterministic
/// order), reporting each cell's direct element counts and its hierarchical
/// bounding box, plus the union bounding box across all cells and the current
/// revision.
pub fn get_document_summary(session: &Session) -> Value {
    let doc = session.document();
    let mut cells: Vec<_> = doc.cells().collect();
    cells.sort_by(|a, b| a.name.cmp(&b.name));

    let mut overall: Option<Rect> = None;
    let cell_summaries: Vec<Value> = cells
        .iter()
        .map(|c| {
            let bbox = doc.cell_bbox(&c.name);
            if let Some(b) = bbox {
                overall = Some(overall.map_or(b, |o| o.union(&b)));
            }
            json!({
                "name": c.name,
                "shapes": c.shapes.len(),
                "instances": c.instances.len(),
                "arrays": c.arrays.len(),
                "labels": c.labels.len(),
                "pins": c.pins.len(),
                "bbox": bbox.map(rect_json),
            })
        })
        .collect();

    json!({
        "cell_count": doc.cell_count(),
        "top_cells": doc.top_cells(),
        "revision": session.revision(),
        "bbox": overall.map(rect_json),
        "cells": cell_summaries,
    })
}

/// The `get_render_region` payload: an inline PNG, or a graceful unavailability
/// note.
///
/// Parses the same `{region, width, height}` arguments as the `render_png`
/// command, applies that command, and on success returns the PNG as a base64
/// `data:` URI plus its dimensions. When rendering is unavailable (no GPU
/// adapter, reported by the engine as an [`ErrorCode::EngineError`]) it returns
/// `available: false` with the reason rather than surfacing a hard tool error, so
/// a headless caller still gets a well-formed answer. Argument errors are
/// returned to the caller as an `Err`.
pub fn get_render_region(session: &mut Session, arguments: &Value) -> Result<Value, String> {
    let cmd = parse_render_args(arguments)?;
    // Pull the requested pixel size back out for the response metadata.
    let AgentCommand::RenderPng { width, height, .. } = &cmd else {
        unreachable!("parse_render_args always builds a RenderPng command");
    };
    let (width, height) = (*width, *height);

    match session.apply(cmd) {
        Ok(AgentResponse::Blob { revision, bytes }) => Ok(json!({
            "available": true,
            "revision": revision,
            "width": width,
            "height": height,
            "mime_type": "image/png",
            "image_data_uri": format!("data:image/png;base64,{}", crate::base64::encode(&bytes)),
        })),
        Ok(other) => Err(format!("unexpected render response: {other:?}")),
        // A missing GPU (or any engine-side render failure) is reported as an
        // unavailability rather than a session-breaking error.
        Err(e) if e.code == ErrorCode::EngineError => Ok(json!({
            "available": false,
            "reason": e.message,
        })),
        Err(e) => Err(e.to_string()),
    }
}

/// Parses `{region, width, height}` into a [`AgentCommand::RenderPng`].
fn parse_render_args(arguments: &Value) -> Result<AgentCommand, String> {
    // The render-region arguments are exactly the `render_png` command fields, so
    // reuse the command's own deserialization by retagging.
    match crate::tools::to_command("render_png", arguments) {
        Some(Ok(cmd)) => Ok(cmd),
        Some(Err(e)) => Err(e),
        None => unreachable!("render_png is a command tool"),
    }
}

// ----- JSON helpers ---------------------------------------------------------

/// A `LayerId` as `{layer, datatype}`.
fn layer_json(layer: reticle_geometry::LayerId) -> Value {
    json!({ "layer": layer.layer, "datatype": layer.datatype })
}

/// A `Rect` as `{min:{x,y}, max:{x,y}}`.
fn rect_json(r: Rect) -> Value {
    json!({
        "min": { "x": r.min.x, "y": r.min.y },
        "max": { "x": r.max.x, "y": r.max.y },
    })
}

/// A `RuleKind` as a stable snake-case string.
fn rule_kind_str(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Width => "width",
        RuleKind::Spacing => "spacing",
        RuleKind::Enclosure => "enclosure",
        RuleKind::Extension => "extension",
        RuleKind::Notch => "notch",
        RuleKind::Area => "area",
        RuleKind::Density => "density",
        RuleKind::Angle => "angle",
        // `RuleKind` is `#[non_exhaustive]`; an unknown future kind is labeled
        // rather than failing to compile.
        _ => "unknown",
    }
}

/// The units of a rule's threshold value, so a model reads `value` correctly.
fn value_units(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Area => "dbu_squared",
        RuleKind::Density => "permille",
        RuleKind::Angle => "millidegrees",
        // Width, spacing, enclosure, extension, notch, and any future length-like
        // kind are all measured in DBU.
        _ => "dbu",
    }
}

#[cfg(test)]
mod tests {
    use super::{get_document_summary, get_render_region, get_technology_rules};
    use reticle_agent_api::{AgentCommand, Session};
    use serde_json::json;

    /// A tiny technology with one width rule, applied through the command surface.
    /// Rule syntax omits a name (`rule <kind> <layer> <datatype> <value>`); the
    /// parser derives the name `<kind>_<layer>_<datatype>`.
    fn session_with_tech() -> Session {
        let mut s = Session::new();
        let tech = "technology demo\n\
                    dbu_per_micron 1000\n\
                    layer 68 20 met1 3A6FD490\n\
                    rule width 68 20 140\n";
        s.apply(AgentCommand::SetTechnology {
            source: tech.into(),
        })
        .expect("set technology");
        s
    }

    /// `get_technology_rules` reports the rule with its kind, layer, and DBU units.
    #[test]
    fn technology_rules_reports_rule_table() {
        let s = session_with_tech();
        let v = get_technology_rules(&s);
        assert_eq!(v["technology"], "demo");
        assert_eq!(v["dbu_per_micron"], 1000);
        assert_eq!(v["rule_count"], 1);
        let rule = &v["rules"][0];
        assert_eq!(rule["name"], "width_68_20");
        assert_eq!(rule["kind"], "width");
        assert_eq!(rule["value"], 140);
        assert_eq!(rule["value_units"], "dbu");
        assert_eq!(rule["layer"], json!({ "layer": 68, "datatype": 20 }));
    }

    /// `get_document_summary` counts cells and shapes and unions the bounding box.
    #[test]
    fn document_summary_counts_and_bbox() {
        let mut s = Session::new();
        s.apply(AgentCommand::CreateCell { name: "top".into() })
            .unwrap();
        s.apply(AgentCommand::AddRect {
            cell: "top".into(),
            layer: reticle_agent_api::args::LayerArg {
                layer: 68,
                datatype: 20,
            },
            rect: reticle_agent_api::args::RectArg {
                min: reticle_agent_api::args::PointArg { x: 0, y: 0 },
                max: reticle_agent_api::args::PointArg { x: 100, y: 200 },
            },
        })
        .unwrap();

        let v = get_document_summary(&s);
        assert_eq!(v["cell_count"], 1);
        assert_eq!(v["cells"][0]["name"], "top");
        assert_eq!(v["cells"][0]["shapes"], 1);
        assert_eq!(
            v["bbox"],
            json!({ "min": { "x": 0, "y": 0 }, "max": { "x": 100, "y": 200 } })
        );
        // Revision advanced twice (create + add).
        assert_eq!(v["revision"], 2);
    }

    /// `get_render_region` returns a well-formed payload whether or not a GPU is
    /// present: either an inline PNG data URI or an `available: false` note. It
    /// must not error on a headless host.
    #[test]
    fn render_region_degrades_without_gpu() {
        let mut s = Session::new();
        s.apply(AgentCommand::CreateCell { name: "top".into() })
            .unwrap();
        s.apply(AgentCommand::AddRect {
            cell: "top".into(),
            layer: reticle_agent_api::args::LayerArg {
                layer: 68,
                datatype: 20,
            },
            rect: reticle_agent_api::args::RectArg {
                min: reticle_agent_api::args::PointArg { x: 0, y: 0 },
                max: reticle_agent_api::args::PointArg { x: 100, y: 100 },
            },
        })
        .unwrap();

        let args = json!({
            "region": { "min": { "x": 0, "y": 0 }, "max": { "x": 100, "y": 100 } },
            "width": 32, "height": 32
        });
        let v = get_render_region(&mut s, &args).expect("render region is graceful");
        assert!(v["available"].is_boolean());
        if v["available"] == json!(true) {
            let uri = v["image_data_uri"].as_str().unwrap();
            assert!(uri.starts_with("data:image/png;base64,"));
            assert_eq!(v["width"], 32);
        } else {
            assert!(v["reason"].is_string());
        }
    }

    /// A malformed region argument is a caller error, not a panic.
    #[test]
    fn render_region_rejects_bad_args() {
        let mut s = Session::new();
        let bad = json!({ "region": "not a rect", "width": 10, "height": 10 });
        assert!(get_render_region(&mut s, &bad).is_err());
    }
}

//! Registration of the Reticle scripting API onto a `rhai` [`Engine`].
//!
//! [`register_api`] installs every host function a script can call, each closing
//! over a clone of the [`SharedHost`]. The functions fall into five groups that
//! mirror the modelling workflow:
//!
//! * **Create / edit**, `create_cell`, `add_rect`, `add_polygon`, `add_path`,
//!   `add_instance`, `add_array`, `set_top_cells`.
//! * **Query**, `cell_count`, `has_cell`, `shape_count`, `instance_count`,
//!   `array_count`, `cell_bbox`, `shapes_bbox`, `flatten_count`.
//! * **Transform**, `flatten_into` (materialize a hierarchy into a flat cell).
//! * **DRC**, `load_technology`, `add_width_rule`, `add_spacing_rule`,
//!   `add_area_rule`, `add_enclosure_rule`, `rule_count`, `run_drc`,
//!   `drc_messages`.
//! * **Export**, `export_gds`, `export_oasis` (both return a `rhai` blob).
//!
//! # Value conventions
//!
//! Coordinates and counts are `rhai` integers ([`INT`], i.e. `i64`); they are
//! narrowed to the model's [`Dbu`] (`i32`) or `u16`
//! layer/datatype with checked conversions that raise a script error on overflow.
//! A bounding box is returned as a four-element integer array
//! `[min_x, min_y, max_x, max_y]`, or an empty array when the cell is missing or
//! empty. Polygon and path vertices are passed as a flat integer array
//! `[x0, y0, x1, y1, ...]`.

use rhai::{Array, Blob, Engine, EvalAltResult, INT};

use reticle_drc::DrcEngine;
use reticle_geometry::{Dbu, Endcap, LayerId, Path, Point, Polygon, Rect, Transform};
use reticle_io::{Gds, Oasis, parse_technology};
use reticle_model::{
    ArrayInstance, Cell, DrawShape, Edit, Exporter, Instance, Rule, RuleKind, RuleSet, ShapeKind,
};

use crate::host::SharedHost;

/// Shorthand for a fallible `rhai` host-function result.
type FnResult<T> = core::result::Result<T, Box<EvalAltResult>>;

/// Builds a `rhai` runtime error carrying `msg`.
///
/// The boxed return is intentional: `rhai`'s error type ([`RhaiError`](rhai) =
/// `Box<EvalAltResult>`) is what every host function must yield, so this helper's
/// output is used directly as the `Err` payload.
#[allow(clippy::unnecessary_box_returns)]
fn err(msg: impl Into<String>) -> Box<EvalAltResult> {
    // `Box<EvalAltResult>` has a blanket `From<impl AsRef<str>>` that yields an
    // `ErrorRuntime`, which is catchable and prints the message.
    Box::<EvalAltResult>::from(msg.into())
}

/// Narrows a `rhai` integer to a database unit, saturating at the [`Dbu`] range.
///
/// Coordinates beyond `i32` cannot exist on the layout grid; clamping keeps a
/// runaway script from overflowing rather than trapping, matching the saturating
/// arithmetic the geometry layer already uses.
fn to_dbu(v: INT) -> Dbu {
    v.clamp(INT::from(Dbu::MIN), INT::from(Dbu::MAX)) as Dbu
}

/// Narrows a `rhai` integer to a GDSII layer/datatype number, erroring if out of
/// the `u16` range.
fn to_u16(v: INT, what: &str) -> FnResult<u16> {
    u16::try_from(v).map_err(|_| err(format!("{what} {v} is out of the valid range 0..=65535")))
}

/// Narrows a `rhai` integer to a non-negative `u32` count, erroring if out of range.
fn to_u32(v: INT, what: &str) -> FnResult<u32> {
    u32::try_from(v).map_err(|_| err(format!("{what} {v} must be in the range 0..=4294967295")))
}

/// Builds a [`LayerId`] from script-supplied layer and datatype integers.
fn layer_id(layer: INT, datatype: INT) -> FnResult<LayerId> {
    Ok(LayerId::new(
        to_u16(layer, "layer")?,
        to_u16(datatype, "datatype")?,
    ))
}

/// Reads a flat `[x0, y0, x1, y1, ...]` array into a vector of points.
///
/// Errors if the array has an odd length or contains a non-integer element.
fn points_from_array(arr: &Array) -> FnResult<Vec<Point>> {
    if !arr.len().is_multiple_of(2) {
        return Err(err(format!(
            "point array must have an even length (got {})",
            arr.len()
        )));
    }
    let mut pts = Vec::with_capacity(arr.len() / 2);
    let mut it = arr.iter();
    while let (Some(x), Some(y)) = (it.next(), it.next()) {
        let x = x
            .as_int()
            .map_err(|t| err(format!("point x coordinate must be an integer, got {t}")))?;
        let y = y
            .as_int()
            .map_err(|t| err(format!("point y coordinate must be an integer, got {t}")))?;
        pts.push(Point::new(to_dbu(x), to_dbu(y)));
    }
    Ok(pts)
}

/// Encodes a [`Rect`] as the four-element array scripts expect.
fn bbox_array(r: Rect) -> Array {
    vec![
        INT::from(r.min.x).into(),
        INT::from(r.min.y).into(),
        INT::from(r.max.x).into(),
        INT::from(r.max.y).into(),
    ]
}

/// Maps an end-cap name to an [`Endcap`], erroring on an unknown name.
fn endcap_from_name(name: &str) -> FnResult<Endcap> {
    match name.to_ascii_lowercase().as_str() {
        "flat" => Ok(Endcap::Flat),
        "square" => Ok(Endcap::Square),
        "round" => Ok(Endcap::Round),
        other => Err(err(format!(
            "unknown endcap '{other}' (expected flat, square, or round)"
        ))),
    }
}

/// Registers every Reticle scripting function onto `engine`, each sharing `host`.
///
/// After this returns, `engine.run(source)` (or [`Engine::eval`]) can execute
/// scripts that build, query, check, and export the document held by `host`.
#[allow(clippy::too_many_lines)]
pub fn register_api(engine: &mut Engine, host: &SharedHost) {
    register_create(engine, host);
    register_query(engine, host);
    register_transform(engine, host);
    register_drc(engine, host);
    register_export(engine, host);
}

/// Registers the create/edit functions.
#[allow(clippy::too_many_lines)]
fn register_create(engine: &mut Engine, host: &SharedHost) {
    // create_cell(name): add a new, empty cell. Errors on a duplicate name.
    {
        let host = host.clone();
        engine.register_fn("create_cell", move |name: &str| -> FnResult<()> {
            host.borrow_mut()
                .apply(Edit::AddCell {
                    cell: Cell::new(name),
                })
                .map_err(err)
        });
    }

    // add_rect(cell, layer, datatype, x0, y0, x1, y1)
    {
        let host = host.clone();
        engine.register_fn(
            "add_rect",
            move |cell: &str,
                  layer: INT,
                  datatype: INT,
                  x0: INT,
                  y0: INT,
                  x1: INT,
                  y1: INT|
                  -> FnResult<()> {
                let id = layer_id(layer, datatype)?;
                let rect = Rect::new(
                    Point::new(to_dbu(x0), to_dbu(y0)),
                    Point::new(to_dbu(x1), to_dbu(y1)),
                );
                host.borrow_mut()
                    .apply(Edit::AddShape {
                        cell: cell.to_string(),
                        shape: DrawShape::new(id, ShapeKind::Rect(rect)),
                    })
                    .map_err(err)
            },
        );
    }

    // add_polygon(cell, layer, datatype, points): points is [x0,y0,x1,y1,...]
    {
        let host = host.clone();
        engine.register_fn(
            "add_polygon",
            move |cell: &str, layer: INT, datatype: INT, points: Array| -> FnResult<()> {
                let id = layer_id(layer, datatype)?;
                let pts = points_from_array(&points)?;
                if pts.len() < 3 {
                    return Err(err(format!(
                        "polygon needs at least 3 vertices (got {})",
                        pts.len()
                    )));
                }
                host.borrow_mut()
                    .apply(Edit::AddShape {
                        cell: cell.to_string(),
                        shape: DrawShape::new(id, ShapeKind::Polygon(Polygon::new(pts))),
                    })
                    .map_err(err)
            },
        );
    }

    // add_path(cell, layer, datatype, width, points): flat, square-capped default
    {
        let host = host.clone();
        engine.register_fn(
            "add_path",
            move |cell: &str,
                  layer: INT,
                  datatype: INT,
                  width: INT,
                  points: Array|
                  -> FnResult<()> {
                add_path_impl(&host, cell, layer, datatype, width, &points, Endcap::Flat)
            },
        );
    }

    // add_path_capped(cell, layer, datatype, width, points, endcap)
    {
        let host = host.clone();
        engine.register_fn(
            "add_path_capped",
            move |cell: &str,
                  layer: INT,
                  datatype: INT,
                  width: INT,
                  points: Array,
                  endcap: &str|
                  -> FnResult<()> {
                let cap = endcap_from_name(endcap)?;
                add_path_impl(&host, cell, layer, datatype, width, &points, cap)
            },
        );
    }

    // add_instance(cell, child, dx, dy): unit-magnification, unrotated placement
    {
        let host = host.clone();
        engine.register_fn(
            "add_instance",
            move |cell: &str, child: &str, dx: INT, dy: INT| -> FnResult<()> {
                host.borrow_mut()
                    .apply(Edit::AddInstance {
                        cell: cell.to_string(),
                        instance: Instance {
                            cell: child.to_string(),
                            transform: Transform::translate(to_dbu(dx), to_dbu(dy)),
                        },
                    })
                    .map_err(err)
            },
        );
    }

    // add_array(cell, child, dx, dy, columns, rows, column_pitch, row_pitch)
    {
        let host = host.clone();
        engine.register_fn(
            "add_array",
            move |cell: &str,
                  child: &str,
                  dx: INT,
                  dy: INT,
                  columns: INT,
                  rows: INT,
                  column_pitch: INT,
                  row_pitch: INT|
                  -> FnResult<()> {
                let array = ArrayInstance {
                    cell: child.to_string(),
                    transform: Transform::translate(to_dbu(dx), to_dbu(dy)),
                    columns: to_u32(columns, "columns")?,
                    rows: to_u32(rows, "rows")?,
                    column_pitch: to_dbu(column_pitch),
                    row_pitch: to_dbu(row_pitch),
                };
                host.borrow_mut()
                    .apply(Edit::AddArray {
                        cell: cell.to_string(),
                        array,
                    })
                    .map_err(err)
            },
        );
    }

    // set_top_cells(names): replace the document's top-cell list
    {
        let host = host.clone();
        engine.register_fn("set_top_cells", move |names: Array| -> FnResult<()> {
            let mut tops = Vec::with_capacity(names.len());
            for n in &names {
                let s = n
                    .clone()
                    .into_string()
                    .map_err(|t| err(format!("top cell name must be a string, got {t}")))?;
                tops.push(s);
            }
            // Setting top cells has no `Edit` variant, so it is stored on the host
            // and folded into the document snapshot the engine hands out (see
            // `ScriptHost::snapshot`).
            host.borrow_mut().set_top_cells(tops);
            Ok(())
        });
    }
}

/// Shared body of `add_path` / `add_path_capped`.
fn add_path_impl(
    host: &SharedHost,
    cell: &str,
    layer: INT,
    datatype: INT,
    width: INT,
    points: &Array,
    endcap: Endcap,
) -> FnResult<()> {
    let id = layer_id(layer, datatype)?;
    let pts = points_from_array(points)?;
    if pts.len() < 2 {
        return Err(err(format!(
            "path needs at least 2 points (got {})",
            pts.len()
        )));
    }
    let path = Path::new(pts, to_dbu(width), endcap);
    host.borrow_mut()
        .apply(Edit::AddShape {
            cell: cell.to_string(),
            shape: DrawShape::new(id, ShapeKind::Path(path)),
        })
        .map_err(err)
}

/// Registers the query functions.
fn register_query(engine: &mut Engine, host: &SharedHost) {
    {
        let host = host.clone();
        engine.register_fn("cell_count", move || -> INT {
            host.borrow().document().cell_count() as INT
        });
    }
    {
        let host = host.clone();
        engine.register_fn("has_cell", move |name: &str| -> bool {
            host.borrow().document().cell(name).is_some()
        });
    }
    {
        let host = host.clone();
        engine.register_fn("shape_count", move |cell: &str| -> INT {
            host.borrow()
                .document()
                .cell(cell)
                .map_or(0, |c| c.shapes.len() as INT)
        });
    }
    {
        let host = host.clone();
        engine.register_fn("instance_count", move |cell: &str| -> INT {
            host.borrow()
                .document()
                .cell(cell)
                .map_or(0, |c| c.instances.len() as INT)
        });
    }
    {
        let host = host.clone();
        engine.register_fn("array_count", move |cell: &str| -> INT {
            host.borrow()
                .document()
                .cell(cell)
                .map_or(0, |c| c.arrays.len() as INT)
        });
    }
    {
        let host = host.clone();
        engine.register_fn("cell_bbox", move |cell: &str| -> Array {
            host.borrow()
                .document()
                .cell_bbox(cell)
                .map_or_else(Array::new, bbox_array)
        });
    }
    {
        let host = host.clone();
        engine.register_fn("shapes_bbox", move |cell: &str| -> Array {
            host.borrow()
                .document()
                .cell(cell)
                .and_then(Cell::shapes_bbox)
                .map_or_else(Array::new, bbox_array)
        });
    }
    {
        let host = host.clone();
        engine.register_fn("flatten_count", move |top: &str| -> INT {
            host.borrow().document().flatten(top).len() as INT
        });
    }
}

/// Registers the transform functions.
fn register_transform(engine: &mut Engine, host: &SharedHost) {
    // flatten_into(top, dest): create `dest` and fill it with `top`'s flattened
    // geometry. Returns the number of shapes written.
    let host = host.clone();
    engine.register_fn(
        "flatten_into",
        move |top: &str, dest: &str| -> FnResult<INT> {
            let shapes = host.borrow().document().flatten(top);
            let count = shapes.len();
            {
                let mut h = host.borrow_mut();
                h.apply(Edit::AddCell {
                    cell: Cell::new(dest),
                })
                .map_err(err)?;
                for shape in shapes {
                    h.apply(Edit::AddShape {
                        cell: dest.to_string(),
                        shape,
                    })
                    .map_err(err)?;
                }
            }
            Ok(count as INT)
        },
    );
}

/// Registers the DRC functions.
fn register_drc(engine: &mut Engine, host: &SharedHost) {
    // load_technology(text): parse a technology file and adopt its rules (and set
    // the document technology). Returns the number of rules loaded.
    {
        let host = host.clone();
        engine.register_fn("load_technology", move |text: &str| -> FnResult<INT> {
            let tech = parse_technology(text).map_err(|e| err(e.to_string()))?;
            let rules = tech.rules.clone();
            let n = rules.len();
            let mut h = host.borrow_mut();
            h.set_technology(tech);
            h.set_rules(rules);
            Ok(n as INT)
        });
    }

    // Single-layer rule helpers.
    register_single_rule(engine, host, "add_width_rule", RuleKind::Width);
    register_single_rule(engine, host, "add_area_rule", RuleKind::Area);
    register_single_rule(engine, host, "add_notch_rule", RuleKind::Notch);

    // add_spacing_rule(name, layer, datatype, value): same-layer spacing.
    register_single_rule(engine, host, "add_spacing_rule", RuleKind::Spacing);

    // add_enclosure_rule(name, layer, datatype, other_layer, other_datatype, value)
    {
        let host = host.clone();
        engine.register_fn(
            "add_enclosure_rule",
            move |name: &str,
                  layer: INT,
                  datatype: INT,
                  other_layer: INT,
                  other_datatype: INT,
                  value: INT|
                  -> FnResult<()> {
                let rule = Rule {
                    name: name.to_string(),
                    kind: RuleKind::Enclosure,
                    layer: layer_id(layer, datatype)?,
                    other_layer: Some(layer_id(other_layer, other_datatype)?),
                    value,
                };
                host.borrow_mut().push_rule(rule);
                Ok(())
            },
        );
    }

    {
        let host = host.clone();
        engine.register_fn("rule_count", move || -> INT {
            host.borrow().rules().len() as INT
        });
    }

    // run_drc(cell): check `cell` against the loaded rules; return violation count.
    {
        let host = host.clone();
        engine.register_fn("run_drc", move |cell: &str| -> INT {
            let h = host.borrow();
            let drc = DrcEngine::new(h.rules().to_vec());
            drc.check_cell(h.document(), cell).len() as INT
        });
    }

    // drc_messages(cell): the human-readable violation messages as a string array.
    {
        let host = host.clone();
        engine.register_fn("drc_messages", move |cell: &str| -> Array {
            let h = host.borrow();
            let drc = DrcEngine::new(h.rules().to_vec());
            drc.check_cell(h.document(), cell)
                .into_iter()
                .map(|v| v.message.into())
                .collect()
        });
    }
}

/// Registers a single-layer rule constructor named `fn_name` for `kind`.
///
/// The script signature is `fn_name(name, layer, datatype, value)`.
fn register_single_rule(engine: &mut Engine, host: &SharedHost, fn_name: &str, kind: RuleKind) {
    let host = host.clone();
    engine.register_fn(
        fn_name,
        move |name: &str, layer: INT, datatype: INT, value: INT| -> FnResult<()> {
            let rule = Rule {
                name: name.to_string(),
                kind,
                layer: layer_id(layer, datatype)?,
                other_layer: None,
                value,
            };
            host.borrow_mut().push_rule(rule);
            Ok(())
        },
    );
}

/// Registers the export functions.
fn register_export(engine: &mut Engine, host: &SharedHost) {
    {
        let host = host.clone();
        engine.register_fn("export_gds", move || -> FnResult<Blob> {
            Gds.export(host.borrow().document())
                .map_err(|e| err(e.to_string()))
        });
    }
    {
        let host = host.clone();
        engine.register_fn("export_oasis", move || -> FnResult<Blob> {
            Oasis
                .export(host.borrow().document())
                .map_err(|e| err(e.to_string()))
        });
    }
}

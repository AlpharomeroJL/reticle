//! The natural-language edit command bar's deterministic grammar parser.
//!
//! This is NOT the LLM agent (see [`crate::agent_panel`]): there is no model call
//! and no network anywhere in this module. It is a small, fixed grammar over the
//! editor's own edit vocabulary, so a user can type a short instruction and get an
//! exact, predictable [`reticle_model::Edit`] batch back, or a clear error. Same
//! input always yields the same output, on native and on wasm.
//!
//! # Grammar
//!
//! Four forms, matched case-insensitively on whitespace-separated tokens:
//!
//! * `add rect <layer> <x0> <y0> <x1> <y1>`: append a rectangle to the top cell.
//!   `<layer>` is a GDSII layer number, optionally `layer/datatype` (`4` or
//!   `4/2`); `<x0> <y0> <x1> <y1>` are DBU integers for two opposite corners (in
//!   either order; the rectangle is normalized). Example: `add rect 4/0 0 0 1000 500`.
//! * `delete selected`: remove every directly-owned selected shape.
//! * `array <cols>x<rows> pitch <px> <py>`: stamp the current selection into a
//!   `<cols>` by `<rows>` grid at column pitch `<px>` and row pitch `<py>` DBU.
//!   Example: `array 3x4 pitch 1000 1000`.
//! * `move <dx> <dy>`: translate every directly-owned selected shape by `<dx>
//!   <dy>` DBU. Example: `move 100 -50`.
//!
//! Anything else, a wrong token count, a bad keyword, a non-numeric token, or a
//! value that violates a hard constraint (a degenerate rectangle, a non-positive
//! array dimension, an array past the element cap) is rejected with a
//! [`NlEditError`] naming the problem. [`parse`] never panics on any input; it
//! only ever returns `Ok` or `Err`.
//!
//! # Architecture: parse, then build, then apply
//!
//! [`parse`] is pure `&str -> Result<NlInstruction, NlEditError>` with no
//! dependency on the app or the document: every grammar and range check happens
//! here, so a valid [`NlInstruction`] is already fully validated. [`build_edits`]
//! is pure too; given a validated instruction plus the bits of editing context it
//! cannot see on its own (the top cell's name and the current selection), it
//! returns the concrete [`reticle_model::Edit`] batch that realizes it, in
//! application order. Neither function touches `egui`, a GPU, or the network, so
//! both are unit-tested here directly, and the round-trip through a real
//! [`crate::history::History`] is tested here too (see the tests module).
//!
//! The app glue ([`crate::app::App`]'s `nl_edit_submit`) supplies that context and
//! applies the returned batch through [`crate::history::History::apply_group`] in
//! one call, so one instruction is always exactly one undo step, however many
//! shapes it touches, and a failing apply cannot leave a half-applied edit behind
//! silently (the caller reports the error).
//!
//! # Honest limits
//!
//! The grammar is intentionally small and fixed: no synonyms, no fuzzy matching,
//! no layer-name lookup (layers are addressed by GDSII number, not by the
//! technology's layer name), and no compound instructions (one instruction per
//! submit). Widening the grammar is a matter of adding another `parse_*` helper
//! and `NlInstruction` variant; nothing about the architecture above changes.

use reticle_geometry::{Dbu, LayerId, Point, Rect};
use reticle_model::{DrawShape, Edit, ShapeKind};

use crate::productivity;

/// A short, human-readable summary of the supported grammar, for a help label or
/// tooltip next to the input bar.
pub const GRAMMAR_HELP: &str = "add rect <layer> <x0> <y0> <x1> <y1> | delete selected | array <cols>x<rows> pitch <px> <py> | move <dx> <dy>";

/// A parsed, fully validated natural-language edit instruction.
///
/// Every field is already range-checked by [`parse`]; turning one of these into
/// edits (see [`build_edits`]) cannot fail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NlInstruction {
    /// `add rect <layer> <x0> <y0> <x1> <y1>`: append a rectangle to the top cell.
    AddRect {
        /// The layer (and datatype) the rectangle is drawn on.
        layer: LayerId,
        /// The rectangle's corners, in DBU (normalized so `min <= max`, and
        /// guaranteed non-degenerate: positive width and height).
        rect: Rect,
    },
    /// `delete selected`: remove every directly-owned selected shape.
    DeleteSelected,
    /// `array <cols>x<rows> pitch <px> <py>`: stamp the selection into a grid.
    Array {
        /// Column count (x repetitions), at least 1.
        cols: u32,
        /// Row count (y repetitions), at least 1.
        rows: u32,
        /// Column pitch in DBU (x spacing between elements).
        pitch_x: Dbu,
        /// Row pitch in DBU (y spacing between elements).
        pitch_y: Dbu,
    },
    /// `move <dx> <dy>`: translate every directly-owned selected shape.
    Move {
        /// X offset in DBU.
        dx: Dbu,
        /// Y offset in DBU.
        dy: Dbu,
    },
}

/// Why a natural-language instruction was rejected.
///
/// Every variant carries enough text to show the user a clear, specific reason
/// (see the [`core::fmt::Display`] impl); none of them is ever produced by a
/// panic; malformed or out-of-range input always returns one of these instead.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NlEditError {
    /// The input was empty or all whitespace.
    Empty,
    /// The first word did not name any known instruction.
    UnknownVerb {
        /// The unrecognized leading word, as typed.
        verb: String,
    },
    /// The instruction's leading keyword was recognized but the rest did not
    /// match its grammar: wrong token count, a missing fixed keyword (`rect`,
    /// `selected`, `pitch`), or a token that is not a whole number.
    Malformed {
        /// Which grammar form was being parsed (`"add rect"`, `"delete
        /// selected"`, `"array"`, or `"move"`).
        form: &'static str,
        /// A human-readable reason, naming the offending token where possible.
        reason: String,
    },
    /// The instruction matched its grammar but a value violates a hard
    /// constraint: a degenerate rectangle, a non-positive array dimension, or an
    /// array past the element cap.
    OutOfRange {
        /// Which grammar form was being validated.
        form: &'static str,
        /// A human-readable reason.
        reason: String,
    },
}

impl core::fmt::Display for NlEditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            NlEditError::Empty => write!(
                f,
                "type an instruction, e.g. `add rect 4 0 0 1000 500`, `delete selected`, \
                 `array 3x4 pitch 1000 1000`, or `move 100 -50`"
            ),
            NlEditError::UnknownVerb { verb } => write!(
                f,
                "unknown instruction `{verb}`; try `add rect`, `delete selected`, `array`, or `move`"
            ),
            NlEditError::Malformed { form, reason } | NlEditError::OutOfRange { form, reason } => {
                write!(f, "{form}: {reason}")
            }
        }
    }
}

impl core::error::Error for NlEditError {}

/// Parses `input` as a natural-language edit instruction.
///
/// Pure and deterministic: the same `input` always yields the same result, with
/// no dependency on the document, the selection, or any other app state (that
/// context is supplied later, to [`build_edits`]). Tokenizes on whitespace and
/// matches keywords case-insensitively; every numeric token is validated as a
/// whole number in range, and every semantic constraint (a non-degenerate
/// rectangle, positive array dimensions, the array element cap) is checked here,
/// so an `Ok` result is already fully validated. Never panics: garbage of any
/// shape returns an [`NlEditError`], never an abort.
///
/// # Errors
///
/// Returns [`NlEditError::Empty`] for blank input, [`NlEditError::UnknownVerb`]
/// when the leading word names no known instruction, [`NlEditError::Malformed`]
/// when the rest does not match that instruction's grammar, and
/// [`NlEditError::OutOfRange`] when the numbers are well-formed but violate a
/// hard constraint.
pub fn parse(input: &str) -> Result<NlInstruction, NlEditError> {
    let tokens: Vec<&str> = input.split_whitespace().collect();
    let Some(verb) = tokens.first() else {
        return Err(NlEditError::Empty);
    };
    let verb_lower = verb.to_ascii_lowercase();
    match verb_lower.as_str() {
        "add" => parse_add_rect(&tokens),
        "delete" => parse_delete_selected(&tokens),
        "array" => parse_array(&tokens),
        "move" => parse_move(&tokens),
        _ => Err(NlEditError::UnknownVerb {
            verb: (*verb).to_owned(),
        }),
    }
}

const ADD_RECT_GRAMMAR: &str = "add rect <layer> <x0> <y0> <x1> <y1>";

/// Parses `add rect <layer> <x0> <y0> <x1> <y1>`. `tokens` includes the leading
/// `add`.
fn parse_add_rect(tokens: &[&str]) -> Result<NlInstruction, NlEditError> {
    if tokens.len() != 7 || !tokens[1].eq_ignore_ascii_case("rect") {
        return Err(NlEditError::Malformed {
            form: "add rect",
            reason: format!("expected `{ADD_RECT_GRAMMAR}`"),
        });
    }
    let layer = parse_layer(tokens[2])?;
    let x0 = parse_dbu(tokens[3], "add rect")?;
    let y0 = parse_dbu(tokens[4], "add rect")?;
    let x1 = parse_dbu(tokens[5], "add rect")?;
    let y1 = parse_dbu(tokens[6], "add rect")?;
    if x0 == x1 || y0 == y1 {
        return Err(NlEditError::OutOfRange {
            form: "add rect",
            reason: "the rectangle must have positive width and height (x0 must not equal x1, \
                     y0 must not equal y1)"
                .to_owned(),
        });
    }
    Ok(NlInstruction::AddRect {
        layer,
        rect: Rect::new(Point::new(x0, y0), Point::new(x1, y1)),
    })
}

const DELETE_SELECTED_GRAMMAR: &str = "delete selected";

/// Parses `delete selected`. `tokens` includes the leading `delete`.
fn parse_delete_selected(tokens: &[&str]) -> Result<NlInstruction, NlEditError> {
    if tokens.len() != 2 || !tokens[1].eq_ignore_ascii_case("selected") {
        return Err(NlEditError::Malformed {
            form: "delete selected",
            reason: format!("expected `{DELETE_SELECTED_GRAMMAR}`"),
        });
    }
    Ok(NlInstruction::DeleteSelected)
}

const ARRAY_GRAMMAR: &str = "array <cols>x<rows> pitch <px> <py>";

/// Parses `array <cols>x<rows> pitch <px> <py>`. `tokens` includes the leading
/// `array`.
fn parse_array(tokens: &[&str]) -> Result<NlInstruction, NlEditError> {
    if tokens.len() != 5 || !tokens[2].eq_ignore_ascii_case("pitch") {
        return Err(NlEditError::Malformed {
            form: "array",
            reason: format!("expected `{ARRAY_GRAMMAR}`"),
        });
    }
    let (cols, rows) = parse_cols_by_rows(tokens[1])?;
    let pitch_x = parse_dbu(tokens[3], "array")?;
    let pitch_y = parse_dbu(tokens[4], "array")?;
    if cols == 0 || rows == 0 {
        return Err(NlEditError::OutOfRange {
            form: "array",
            reason: "columns and rows must each be at least 1".to_owned(),
        });
    }
    let count = productivity::array_element_count(rows, cols);
    if count > productivity::MAX_ARRAY_ELEMENTS {
        return Err(NlEditError::OutOfRange {
            form: "array",
            reason: format!(
                "{count} elements exceeds the {} cap",
                productivity::MAX_ARRAY_ELEMENTS
            ),
        });
    }
    Ok(NlInstruction::Array {
        cols,
        rows,
        pitch_x,
        pitch_y,
    })
}

const MOVE_GRAMMAR: &str = "move <dx> <dy>";

/// Parses `move <dx> <dy>`. `tokens` includes the leading `move`.
fn parse_move(tokens: &[&str]) -> Result<NlInstruction, NlEditError> {
    if tokens.len() != 3 {
        return Err(NlEditError::Malformed {
            form: "move",
            reason: format!("expected `{MOVE_GRAMMAR}`"),
        });
    }
    let dx = parse_dbu(tokens[1], "move")?;
    let dy = parse_dbu(tokens[2], "move")?;
    Ok(NlInstruction::Move { dx, dy })
}

/// Parses a `<cols>x<rows>` token (for example `3x4`), case-insensitive on the
/// separator.
fn parse_cols_by_rows(token: &str) -> Result<(u32, u32), NlEditError> {
    let malformed = || NlEditError::Malformed {
        form: "array",
        reason: format!("`{token}` is not `<cols>x<rows>` (e.g. `3x4`)"),
    };
    let lower = token.to_ascii_lowercase();
    let (cols_str, rows_str) = lower.split_once('x').ok_or_else(malformed)?;
    let cols = cols_str.parse::<u32>().map_err(|_| malformed())?;
    let rows = rows_str.parse::<u32>().map_err(|_| malformed())?;
    Ok((cols, rows))
}

/// Parses a layer token: a bare GDSII layer number (`4`, datatype defaults to
/// `0`) or an explicit `layer/datatype` pair (`4/2`).
fn parse_layer(token: &str) -> Result<LayerId, NlEditError> {
    let malformed = || NlEditError::Malformed {
        form: "add rect",
        reason: format!(
            "`{token}` is not a valid layer (expected a layer number or layer/datatype, e.g. `4` \
             or `4/2`)"
        ),
    };
    let (layer_str, datatype_str) = token.split_once('/').unwrap_or((token, "0"));
    let layer = layer_str.parse::<u16>().map_err(|_| malformed())?;
    let datatype = datatype_str.parse::<u16>().map_err(|_| malformed())?;
    Ok(LayerId::new(layer, datatype))
}

/// Parses a single DBU integer token, tagging a parse failure with which
/// grammar `form` was being read.
fn parse_dbu(token: &str, form: &'static str) -> Result<Dbu, NlEditError> {
    token.parse::<Dbu>().map_err(|_| NlEditError::Malformed {
        form,
        reason: format!("`{token}` is not a whole number"),
    })
}

/// Turns a validated instruction into the concrete document edits that realize
/// it, given the editing context [`parse`] cannot see: which cell to edit and
/// what is currently selected.
///
/// `top_cell` names the cell every edit targets. `selected_direct` is the
/// current selection's directly-owned shapes as `(index, shape)` pairs, index
/// ascending: the only shapes [`NlInstruction::DeleteSelected`] and
/// [`NlInstruction::Move`] can act on (mirroring the productivity panel's cut
/// and move; instanced or arrayed geometry has no cell-local index to remove or
/// move in place). `selected_resolved` is the full resolved selection (including
/// instanced geometry), the source [`NlInstruction::Array`] stamps copies of.
///
/// Returns the edits in application order, ready for a single
/// [`crate::history::History::apply_group`] call so one instruction is always
/// exactly one undo step, however many shapes it touches. An empty result means
/// the instruction was valid but there was nothing to act on (an empty
/// selection for `DeleteSelected`, `Move`, or `Array`); that is not an error, and
/// is never partially applied since nothing is applied at all.
#[must_use]
pub fn build_edits(
    instruction: &NlInstruction,
    top_cell: &str,
    selected_direct: &[(usize, DrawShape)],
    selected_resolved: &[DrawShape],
) -> Vec<Edit> {
    match instruction {
        NlInstruction::AddRect { layer, rect } => vec![Edit::AddShape {
            cell: top_cell.to_owned(),
            shape: DrawShape::new(*layer, ShapeKind::Rect(*rect)),
        }],
        NlInstruction::DeleteSelected => selected_direct
            .iter()
            .rev()
            .map(|(index, _)| Edit::RemoveShape {
                cell: top_cell.to_owned(),
                index: *index,
            })
            .collect(),
        NlInstruction::Move { dx, dy } => {
            // Remove originals in descending index order (so each removal leaves
            // the lower indices valid), then re-add the translated copies.
            let mut edits = Vec::with_capacity(selected_direct.len() * 2);
            for (index, _) in selected_direct.iter().rev() {
                edits.push(Edit::RemoveShape {
                    cell: top_cell.to_owned(),
                    index: *index,
                });
            }
            for (_, shape) in selected_direct {
                edits.push(Edit::AddShape {
                    cell: top_cell.to_owned(),
                    shape: productivity::translate_shape(shape, *dx, *dy),
                });
            }
            edits
        }
        NlInstruction::Array {
            cols,
            rows,
            pitch_x,
            pitch_y,
        } => productivity::array_shapes(selected_resolved, *rows, *cols, *pitch_y, *pitch_x)
            .into_iter()
            .map(|shape| Edit::AddShape {
                cell: top_cell.to_owned(),
                shape,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use reticle_model::{Cell, Document};

    const TOP: &str = "TOP";

    fn rect_shape(layer: LayerId, x0: i32, y0: i32, x1: i32, y1: i32) -> DrawShape {
        DrawShape::new(
            layer,
            ShapeKind::Rect(Rect::new(Point::new(x0, y0), Point::new(x1, y1))),
        )
    }

    fn rect_of(shape: &DrawShape) -> Rect {
        match &shape.kind {
            ShapeKind::Rect(r) => *r,
            other => panic!("expected a rect, got {other:?}"),
        }
    }

    /// A `History` whose top cell already owns `shapes`, for round-trip tests.
    fn seeded_history(shapes: Vec<DrawShape>) -> History {
        let mut history = History::new(Document::new());
        let mut cell = Cell::new(TOP);
        cell.shapes = shapes;
        history.apply(Edit::AddCell { cell }).expect("add cell");
        history
    }

    // ---- parse: add rect ----------------------------------------------------

    #[test]
    fn add_rect_parses() {
        let got = parse("add rect 4 0 0 1000 500").expect("parses");
        assert_eq!(
            got,
            NlInstruction::AddRect {
                layer: LayerId::new(4, 0),
                rect: Rect::new(Point::new(0, 0), Point::new(1000, 500)),
            }
        );
    }

    #[test]
    fn add_rect_with_datatype_parses() {
        let got = parse("add rect 4/2 0 0 100 100").expect("parses");
        assert_eq!(
            got,
            NlInstruction::AddRect {
                layer: LayerId::new(4, 2),
                rect: Rect::new(Point::new(0, 0), Point::new(100, 100)),
            }
        );
    }

    #[test]
    fn add_rect_is_case_and_whitespace_insensitive() {
        let a = parse("ADD RECT 4 0 0 100 100").expect("parses");
        let b = parse("  add   rect  4  0 0 100 100  ").expect("parses");
        assert_eq!(a, b);
    }

    #[test]
    fn add_rect_normalizes_corner_order() {
        // Corners given "backwards" still produce the same normalized rectangle;
        // the user is not required to give min-then-max.
        let forward = parse("add rect 4 0 0 1000 500").expect("parses");
        let backward = parse("add rect 4 1000 500 0 0").expect("parses");
        assert_eq!(forward, backward);
    }

    #[test]
    fn add_rect_rejects_degenerate_rect() {
        let err = parse("add rect 4 0 0 0 500").unwrap_err();
        assert!(matches!(
            err,
            NlEditError::OutOfRange {
                form: "add rect",
                ..
            }
        ));
        let err = parse("add rect 4 0 0 1000 0").unwrap_err();
        assert!(matches!(
            err,
            NlEditError::OutOfRange {
                form: "add rect",
                ..
            }
        ));
    }

    #[test]
    fn add_rect_rejects_wrong_token_count() {
        let err = parse("add rect 4 0 0 1000").unwrap_err();
        assert!(matches!(
            err,
            NlEditError::Malformed {
                form: "add rect",
                ..
            }
        ));
    }

    #[test]
    fn add_rect_rejects_garbage_number() {
        let err = parse("add rect 4 zero 0 100 100").unwrap_err();
        assert!(matches!(
            err,
            NlEditError::Malformed {
                form: "add rect",
                ..
            }
        ));
    }

    #[test]
    fn add_rect_rejects_layer_out_of_range() {
        // u16::MAX is 65535; 99999 overflows it.
        let err = parse("add rect 99999 0 0 100 100").unwrap_err();
        assert!(matches!(
            err,
            NlEditError::Malformed {
                form: "add rect",
                ..
            }
        ));
    }

    #[test]
    fn add_rect_rejects_wrong_keyword() {
        let err = parse("add circle 4 0 0 100 100").unwrap_err();
        assert!(matches!(
            err,
            NlEditError::Malformed {
                form: "add rect",
                ..
            }
        ));
    }

    // ---- parse: delete selected ---------------------------------------------

    #[test]
    fn delete_selected_parses() {
        assert_eq!(
            parse("delete selected").expect("parses"),
            NlInstruction::DeleteSelected
        );
        assert_eq!(
            parse("DELETE SELECTED").expect("parses"),
            NlInstruction::DeleteSelected
        );
    }

    #[test]
    fn delete_selected_rejects_garbage_suffix() {
        let err = parse("delete selected now").unwrap_err();
        assert!(matches!(
            err,
            NlEditError::Malformed {
                form: "delete selected",
                ..
            }
        ));
    }

    #[test]
    fn delete_selected_rejects_wrong_keyword() {
        let err = parse("delete all").unwrap_err();
        assert!(matches!(
            err,
            NlEditError::Malformed {
                form: "delete selected",
                ..
            }
        ));
    }

    // ---- parse: array ---------------------------------------------------------

    #[test]
    fn array_parses() {
        assert_eq!(
            parse("array 3x4 pitch 1000 2000").expect("parses"),
            NlInstruction::Array {
                cols: 3,
                rows: 4,
                pitch_x: 1000,
                pitch_y: 2000,
            }
        );
    }

    #[test]
    fn array_dimension_separator_is_case_insensitive() {
        let a = parse("array 3x4 pitch 100 100").expect("parses");
        let b = parse("array 3X4 pitch 100 100").expect("parses");
        assert_eq!(a, b);
    }

    #[test]
    fn array_rejects_zero_dimension() {
        let err = parse("array 0x4 pitch 100 100").unwrap_err();
        assert!(matches!(err, NlEditError::OutOfRange { form: "array", .. }));
        let err = parse("array 4x0 pitch 100 100").unwrap_err();
        assert!(matches!(err, NlEditError::OutOfRange { form: "array", .. }));
    }

    #[test]
    fn array_rejects_over_the_element_cap() {
        // 1000 * 1000 = 1,000,000 > MAX_ARRAY_ELEMENTS (100_000).
        let err = parse("array 1000x1000 pitch 10 10").unwrap_err();
        assert!(matches!(err, NlEditError::OutOfRange { form: "array", .. }));
    }

    #[test]
    fn array_rejects_missing_pitch_keyword() {
        let err = parse("array 3x4 100 100").unwrap_err();
        assert!(matches!(err, NlEditError::Malformed { form: "array", .. }));
    }

    #[test]
    fn array_rejects_malformed_dims() {
        let err = parse("array 3-4 pitch 100 100").unwrap_err();
        assert!(matches!(err, NlEditError::Malformed { form: "array", .. }));
        let err = parse("array threexfour pitch 100 100").unwrap_err();
        assert!(matches!(err, NlEditError::Malformed { form: "array", .. }));
    }

    // ---- parse: move ------------------------------------------------------

    #[test]
    fn move_parses() {
        assert_eq!(
            parse("move 100 -50").expect("parses"),
            NlInstruction::Move { dx: 100, dy: -50 }
        );
    }

    #[test]
    fn move_rejects_wrong_token_count() {
        let err = parse("move 100").unwrap_err();
        assert!(matches!(err, NlEditError::Malformed { form: "move", .. }));
        let err = parse("move 100 50 25").unwrap_err();
        assert!(matches!(err, NlEditError::Malformed { form: "move", .. }));
    }

    #[test]
    fn move_rejects_garbage_number() {
        let err = parse("move ten twenty").unwrap_err();
        assert!(matches!(err, NlEditError::Malformed { form: "move", .. }));
    }

    // ---- parse: general ----------------------------------------------------

    #[test]
    fn unknown_verb_errors() {
        let err = parse("frobnicate everything").unwrap_err();
        assert_eq!(
            err,
            NlEditError::UnknownVerb {
                verb: "frobnicate".to_owned()
            }
        );
    }

    #[test]
    fn empty_input_errors() {
        assert_eq!(parse(""), Err(NlEditError::Empty));
        assert_eq!(parse("   "), Err(NlEditError::Empty));
        assert_eq!(parse("\t\n"), Err(NlEditError::Empty));
    }

    #[test]
    fn parse_is_deterministic() {
        let inputs = [
            "add rect 4 0 0 1000 500",
            "delete selected",
            "array 3x4 pitch 1000 1000",
            "move 100 -50",
            "",
            "bogus",
            "add rect 4 0 0 0 0",
            "array 0x0 pitch 1 1",
        ];
        for input in inputs {
            assert_eq!(
                parse(input),
                parse(input),
                "parse must be deterministic for {input:?}"
            );
        }
    }

    #[test]
    fn parse_never_panics_on_adversarial_input() {
        // A battery of hostile strings: huge numbers, unicode, only separators,
        // deeply repeated tokens, control characters. None of these are valid
        // grammar; the point is that `parse` returns `Err` for all of them
        // instead of panicking (a wasm panic kills the tab).
        // Bound to locals (rather than borrowed inline) so the `&str`s in the
        // list below borrow from a value that outlives the loop, not a
        // temporary that would be dropped at the end of this statement.
        let long_run = "x".repeat(10_000);
        let repeated_valid = "add rect 4 0 0 100 100 ".repeat(1_000);
        let adversarial: Vec<&str> = vec![
            "",
            " ",
            "\t\t\t",
            "add",
            "add rect",
            "add rect rect rect rect rect rect",
            "add rect 99999999999999999999 0 0 100 100",
            "add rect 4 99999999999999999999 0 100 100",
            "array xxxxx pitch 1 1",
            "array 3x4x5 pitch 1 1",
            "array 3x pitch 1 1",
            "array x4 pitch 1 1",
            "move 99999999999999999999 0",
            "move - -",
            "delete",
            "delete selected selected selected",
            "\u{0}\u{0}\u{0}",
            "\u{1F600}\u{1F600} rect 4 0 0 100 100",
            "add rect \u{1F600} 0 0 100 100",
            "-- -- --",
            "add rect 4 0 0 100 100 extra garbage tokens here to overflow",
            long_run.as_str(),
            repeated_valid.as_str(),
        ];
        for input in adversarial {
            // The only property under test is "does not panic"; every one of
            // these is invalid grammar, so `Err` is also asserted as a sanity
            // check that the battery is actually adversarial.
            assert!(parse(input).is_err(), "expected an error for {input:?}");
        }
    }

    // ---- build_edits --------------------------------------------------------

    #[test]
    fn build_edits_add_rect_targets_top_cell() {
        let instr = NlInstruction::AddRect {
            layer: LayerId::new(4, 0),
            rect: Rect::new(Point::new(0, 0), Point::new(100, 100)),
        };
        let edits = build_edits(&instr, TOP, &[], &[]);
        assert_eq!(edits.len(), 1);
        match &edits[0] {
            Edit::AddShape { cell, shape } => {
                assert_eq!(cell, TOP);
                assert_eq!(shape.layer, LayerId::new(4, 0));
                assert_eq!(
                    rect_of(shape),
                    Rect::new(Point::new(0, 0), Point::new(100, 100))
                );
            }
            other => panic!("expected AddShape, got {other:?}"),
        }
    }

    #[test]
    fn build_edits_delete_selected_removes_in_descending_index_order() {
        let a = rect_shape(LayerId::new(4, 0), 0, 0, 10, 10);
        let b = rect_shape(LayerId::new(4, 0), 20, 20, 30, 30);
        let selected = [(2usize, a), (5usize, b)];
        let edits = build_edits(&NlInstruction::DeleteSelected, TOP, &selected, &[]);
        assert_eq!(edits.len(), 2);
        match (&edits[0], &edits[1]) {
            (Edit::RemoveShape { index: i0, .. }, Edit::RemoveShape { index: i1, .. }) => {
                assert_eq!(*i0, 5, "higher index removed first");
                assert_eq!(*i1, 2);
            }
            other => panic!("expected two RemoveShape edits, got {other:?}"),
        }
    }

    #[test]
    fn build_edits_delete_selected_empty_selection_is_empty() {
        let edits = build_edits(&NlInstruction::DeleteSelected, TOP, &[], &[]);
        assert!(edits.is_empty());
    }

    #[test]
    fn build_edits_move_removes_then_re_adds_translated() {
        let shape = rect_shape(LayerId::new(4, 0), 0, 0, 100, 100);
        let selected = [(0usize, shape)];
        let instr = NlInstruction::Move { dx: 10, dy: 20 };
        let edits = build_edits(&instr, TOP, &selected, &[]);
        assert_eq!(edits.len(), 2);
        match &edits[0] {
            Edit::RemoveShape { index, .. } => assert_eq!(*index, 0),
            other => panic!("expected RemoveShape first, got {other:?}"),
        }
        match &edits[1] {
            Edit::AddShape { shape, .. } => {
                assert_eq!(
                    rect_of(shape),
                    Rect::new(Point::new(10, 20), Point::new(110, 120))
                );
            }
            other => panic!("expected AddShape second, got {other:?}"),
        }
    }

    #[test]
    fn build_edits_move_empty_selection_is_empty() {
        let edits = build_edits(&NlInstruction::Move { dx: 5, dy: 5 }, TOP, &[], &[]);
        assert!(edits.is_empty());
    }

    #[test]
    fn build_edits_array_produces_one_add_per_element() {
        let shape = rect_shape(LayerId::new(4, 0), 0, 0, 100, 100);
        let instr = NlInstruction::Array {
            cols: 2,
            rows: 3,
            pitch_x: 1000,
            pitch_y: 500,
        };
        let edits = build_edits(&instr, TOP, &[], std::slice::from_ref(&shape));
        assert_eq!(edits.len(), 6);
        assert!(edits.iter().all(|e| matches!(e, Edit::AddShape { .. })));
    }

    #[test]
    fn build_edits_array_empty_selection_is_empty() {
        let instr = NlInstruction::Array {
            cols: 2,
            rows: 2,
            pitch_x: 100,
            pitch_y: 100,
        };
        let edits = build_edits(&instr, TOP, &[], &[]);
        assert!(edits.is_empty());
    }

    // ---- one undo group per instruction, through a real History -------------

    #[test]
    fn add_rect_applies_as_one_undo_step() {
        let mut history = seeded_history(vec![]);
        let before_depth = history.undo_depth();
        let before_len = history.document().cell(TOP).unwrap().shapes.len();

        let instr = parse("add rect 4 0 0 1000 500").expect("parses");
        let edits = build_edits(&instr, TOP, &[], &[]);
        history.apply_group(edits).expect("applies");

        assert_eq!(history.undo_depth(), before_depth + 1);
        assert_eq!(
            history.document().cell(TOP).unwrap().shapes.len(),
            before_len + 1
        );
        assert!(history.undo());
        assert_eq!(
            history.document().cell(TOP).unwrap().shapes.len(),
            before_len
        );
    }

    #[test]
    fn delete_selected_applies_as_one_undo_step_for_multiple_shapes() {
        let shapes = vec![
            rect_shape(LayerId::new(4, 0), 0, 0, 10, 10),
            rect_shape(LayerId::new(4, 0), 20, 20, 30, 30),
            rect_shape(LayerId::new(4, 0), 40, 40, 50, 50),
        ];
        let mut history = seeded_history(shapes.clone());
        let before_depth = history.undo_depth();

        let selected_direct: Vec<(usize, DrawShape)> = shapes.into_iter().enumerate().collect();
        let instr = NlInstruction::DeleteSelected;
        let edits = build_edits(&instr, TOP, &selected_direct, &[]);
        assert_eq!(edits.len(), 3);
        history.apply_group(edits).expect("applies");

        // One logical step regardless of how many shapes were removed.
        assert_eq!(history.undo_depth(), before_depth + 1);
        assert!(history.document().cell(TOP).unwrap().shapes.is_empty());

        assert!(history.undo());
        assert_eq!(history.document().cell(TOP).unwrap().shapes.len(), 3);
    }

    #[test]
    fn array_applies_as_one_undo_step_regardless_of_element_count() {
        let shape = rect_shape(LayerId::new(4, 0), 0, 0, 100, 100);
        let mut history = seeded_history(vec![shape.clone()]);
        let before_depth = history.undo_depth();
        let before_len = history.document().cell(TOP).unwrap().shapes.len();

        let instr = parse("array 3x4 pitch 1000 1000").expect("parses");
        let edits = build_edits(&instr, TOP, &[], std::slice::from_ref(&shape));
        assert_eq!(edits.len(), 12);
        history.apply_group(edits).expect("applies");

        // Twelve shapes added, but exactly one undo step.
        assert_eq!(history.undo_depth(), before_depth + 1);
        assert_eq!(
            history.document().cell(TOP).unwrap().shapes.len(),
            before_len + 12
        );

        assert!(history.undo());
        assert_eq!(
            history.document().cell(TOP).unwrap().shapes.len(),
            before_len
        );
    }

    #[test]
    fn move_applies_as_one_undo_step() {
        let shape = rect_shape(LayerId::new(4, 0), 0, 0, 100, 100);
        let mut history = seeded_history(vec![shape.clone()]);
        let before_depth = history.undo_depth();

        let instr = parse("move 250 -125").expect("parses");
        let selected_direct = vec![(0usize, shape)];
        let edits = build_edits(&instr, TOP, &selected_direct, &[]);
        history.apply_group(edits).expect("applies");

        // One remove plus one add is still exactly one undo step.
        assert_eq!(history.undo_depth(), before_depth + 1);
        let cell = history.document().cell(TOP).unwrap();
        assert_eq!(cell.shapes.len(), 1);
        assert_eq!(
            rect_of(&cell.shapes[0]),
            Rect::new(Point::new(250, -125), Point::new(350, -25))
        );

        assert!(history.undo());
        let cell = history.document().cell(TOP).unwrap();
        assert_eq!(
            rect_of(&cell.shapes[0]),
            Rect::new(Point::new(0, 0), Point::new(100, 100))
        );
    }

    #[test]
    fn grammar_help_mentions_every_verb() {
        for verb in ["add rect", "delete selected", "array", "move"] {
            assert!(
                GRAMMAR_HELP.contains(verb),
                "grammar help should mention {verb:?}"
            );
        }
    }
}

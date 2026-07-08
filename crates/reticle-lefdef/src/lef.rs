//! The LEF (Library Exchange Format) parser: technology layers, sites, and macro
//! cell abstracts.
//!
//! LEF describes a process technology and the abstract views of the cells built on
//! it. This parser reads the subset Reticle lowers into a [`Document`]: the routing
//! and cut [`LAYER`](LefLayer) table, the placement [`SITE`](LefSite)s, and the
//! [`MACRO`](LefMacro) cell abstracts (size, pins, and obstructions). Everything
//! else (detailed via geometry, spacing tables, antenna rules, property definitions)
//! is skipped with an [`UnsupportedFeature`](crate::WarningKind::UnsupportedFeature)
//! warning rather than being an error, so a real foundry LEF opens.
//!
//! # Coordinates
//!
//! LEF dimensions are decimal microns. This parser keeps them as `f64` microns in
//! [`LefData`]; conversion to integer DBU happens in [`crate::lower`], which knows
//! the resolution shared with the DEF.
//!
//! [`Document`]: reticle_model::Document

use crate::error::{LefDefError, LefDefWarning, WarningKind};
use crate::lex::{Lexer, Token, parse_number};

/// The parsed content of a LEF file, in microns.
#[derive(Debug, Default)]
pub(crate) struct LefData {
    /// The database resolution in DBU per micron from `UNITS DATABASE MICRONS`,
    /// if the LEF declared one.
    pub dbu_per_micron: Option<i64>,
    /// The routing/cut layer table in declaration order.
    pub layers: Vec<LefLayer>,
    /// Placement sites in declaration order.
    pub sites: Vec<LefSite>,
    /// Macro cell abstracts in declaration order.
    pub macros: Vec<LefMacro>,
    /// Non-fatal problems found while parsing.
    pub warnings: Vec<LefDefWarning>,
}

/// The functional class of a LEF layer, coarsened to what lowering needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LefLayerKind {
    /// A routing (metal) layer.
    Routing,
    /// A cut (via) layer.
    Cut,
    /// Any other layer type (masterslice, overlap, implant, ...).
    Other,
}

/// One LEF `LAYER` block, reduced to what the layer table needs.
#[derive(Debug, Clone)]
pub(crate) struct LefLayer {
    /// The layer name (referenced by macro/pin geometry and DEF routing).
    pub name: String,
    /// The functional class.
    pub kind: LefLayerKind,
    /// Default routing width in microns, if the layer declared `WIDTH`.
    pub width_um: Option<f64>,
}

/// One LEF `SITE` block.
#[derive(Debug, Clone)]
pub(crate) struct LefSite {
    /// The site name.
    pub name: String,
    /// The `CLASS` (CORE, PAD, ...), verbatim, or empty.
    pub class: String,
    /// Site width in microns.
    pub width_um: f64,
    /// Site height in microns.
    pub height_um: f64,
}

/// An axis-aligned rectangle on a named layer, in microns.
#[derive(Debug, Clone)]
pub(crate) struct LefRect {
    /// The layer name this rectangle is drawn on.
    pub layer: String,
    /// Lower-left and upper-right corners in microns: `(x1, y1, x2, y2)`.
    pub coords: (f64, f64, f64, f64),
}

/// One LEF `PIN` inside a macro.
#[derive(Debug, Clone, Default)]
pub(crate) struct LefPin {
    /// The pin name.
    pub name: String,
    /// The `DIRECTION` (INPUT, OUTPUT, INOUT), verbatim, or empty.
    pub direction: String,
    /// The port rectangles across all `PORT` blocks of this pin.
    pub rects: Vec<LefRect>,
}

/// One LEF `MACRO` block.
#[derive(Debug, Clone, Default)]
pub(crate) struct LefMacro {
    /// The macro (cell) name.
    pub name: String,
    /// The `CLASS` (CORE, BLOCK, PAD, ...), verbatim, or empty.
    pub class: String,
    /// The cell size in microns from `SIZE w BY h`, if declared.
    pub size_um: Option<(f64, f64)>,
    /// The pins in declaration order.
    pub pins: Vec<LefPin>,
    /// Obstruction rectangles from the `OBS` block.
    pub obs: Vec<LefRect>,
}

/// Parses LEF source into a [`LefData`].
///
/// # Errors
///
/// Returns [`LefDefError::Lef`] on a structural failure (a number that does not
/// parse where one is required, a statement that ends before its mandatory tokens).
/// Unknown keywords and unmodeled blocks are skipped with a warning, not an error.
pub(crate) fn parse(source: &str) -> Result<LefData, LefDefError> {
    let mut lex = Lexer::new(source);
    let mut data = LefData::default();

    while let Some(tok) = lex.peek() {
        let kw = tok.text;
        match kw {
            "END" => {
                // `END LIBRARY` (or a stray END): consume it and its trailing name.
                lex.bump();
                lex.bump();
            }
            "LAYER" => parse_layer(&mut lex, &mut data)?,
            "SITE" => parse_site(&mut lex, &mut data)?,
            "MACRO" => parse_macro(&mut lex, &mut data)?,
            "UNITS" => parse_units(&mut lex, &mut data),
            // Whole blocks we do not model: skip to their matching `END <name>`.
            "VIA"
            | "VIARULE"
            | "NONDEFAULTRULE"
            | "PROPERTYDEFINITIONS"
            | "SPACING"
            | "BEGINEXT"
            | "ARRAY" => {
                lex.bump(); // keyword
                let name = lex.bump().map_or(kw, |t| t.text);
                skip_block(&mut lex, name);
                data.warnings.push(LefDefWarning::new(
                    WarningKind::UnsupportedFeature,
                    format!("skipped LEF {kw} block"),
                    format!("the `{kw} {name}` block is outside the imported subset"),
                ));
            }
            // Simple single-line directives we accept but do not use.
            _ => {
                skip_statement(&mut lex);
            }
        }
    }

    Ok(data)
}

/// Parses `UNITS ... DATABASE MICRONS <n> ; ... END UNITS`.
fn parse_units(lex: &mut Lexer, data: &mut LefData) {
    lex.bump(); // UNITS
    while let Some(tok) = lex.peek() {
        match tok.text {
            "END" => {
                lex.bump(); // END
                lex.bump(); // UNITS
                return;
            }
            "DATABASE" => {
                lex.bump(); // DATABASE
                // Expect `MICRONS <n>`.
                if lex.peek().map(|t| t.text) == Some("MICRONS") {
                    lex.bump();
                    if let Some(n) = lex.peek().and_then(|t| parse_number(t.text))
                        && n > 0.0
                    {
                        data.dbu_per_micron = Some(n as i64);
                    }
                }
                skip_statement(lex);
            }
            _ => skip_statement(lex),
        }
    }
}

/// Parses a `LAYER <name> ... END <name>` block.
fn parse_layer(lex: &mut Lexer, data: &mut LefData) -> Result<(), LefDefError> {
    lex.bump(); // LAYER
    let name = expect_name(lex, "LAYER")?;
    let mut kind = LefLayerKind::Other;
    let mut width_um = None;

    while let Some(tok) = lex.peek() {
        match tok.text {
            "END" => {
                lex.bump(); // END
                lex.bump(); // name
                break;
            }
            "TYPE" => {
                lex.bump();
                kind = match lex.peek().map(|t| t.text) {
                    Some("ROUTING") => LefLayerKind::Routing,
                    Some("CUT") => LefLayerKind::Cut,
                    _ => LefLayerKind::Other,
                };
                skip_statement(lex);
            }
            "WIDTH" => {
                lex.bump();
                width_um = lex.peek().and_then(|t| parse_number(t.text));
                skip_statement(lex);
            }
            _ => skip_statement(lex),
        }
    }

    data.layers.push(LefLayer {
        name,
        kind,
        width_um,
    });
    Ok(())
}

/// Parses a `SITE <name> ... END <name>` block.
fn parse_site(lex: &mut Lexer, data: &mut LefData) -> Result<(), LefDefError> {
    lex.bump(); // SITE
    let name = expect_name(lex, "SITE")?;
    let mut class = String::new();
    let mut size = (0.0, 0.0);

    while let Some(tok) = lex.peek() {
        match tok.text {
            "END" => {
                lex.bump();
                lex.bump();
                break;
            }
            "CLASS" => {
                lex.bump();
                class = lex.peek().map(|t| t.text.to_string()).unwrap_or_default();
                skip_statement(lex);
            }
            "SIZE" => {
                size = parse_size(lex)?;
            }
            _ => skip_statement(lex),
        }
    }

    data.sites.push(LefSite {
        name,
        class,
        width_um: size.0,
        height_um: size.1,
    });
    Ok(())
}

/// Parses a `MACRO <name> ... END <name>` block.
fn parse_macro(lex: &mut Lexer, data: &mut LefData) -> Result<(), LefDefError> {
    lex.bump(); // MACRO
    let name = expect_name(lex, "MACRO")?;
    let mut m = LefMacro {
        name,
        ..LefMacro::default()
    };

    while let Some(tok) = lex.peek() {
        match tok.text {
            "END" => {
                lex.bump(); // END
                lex.bump(); // name
                break;
            }
            "CLASS" => {
                lex.bump();
                m.class = lex.peek().map(|t| t.text.to_string()).unwrap_or_default();
                skip_statement(lex);
            }
            "SIZE" => {
                m.size_um = Some(parse_size(lex)?);
            }
            "PIN" => {
                let pin = parse_pin(lex)?;
                m.pins.push(pin);
            }
            "OBS" => {
                parse_obs(lex, &mut m.obs)?;
            }
            _ => skip_statement(lex),
        }
    }

    data.macros.push(m);
    Ok(())
}

/// Parses a `PIN <name> ... END <name>` block, collecting all its port rectangles.
fn parse_pin(lex: &mut Lexer) -> Result<LefPin, LefDefError> {
    lex.bump(); // PIN
    let name = expect_name(lex, "PIN")?;
    let mut pin = LefPin {
        name,
        ..LefPin::default()
    };

    while let Some(tok) = lex.peek() {
        match tok.text {
            "END" => {
                lex.bump(); // END
                lex.bump(); // name
                break;
            }
            "DIRECTION" => {
                lex.bump();
                pin.direction = lex.peek().map(|t| t.text.to_string()).unwrap_or_default();
                skip_statement(lex);
            }
            "PORT" => {
                parse_port(lex, &mut pin.rects)?;
            }
            _ => skip_statement(lex),
        }
    }

    Ok(pin)
}

/// Parses a `PORT ... END` block (an unnamed block: `END` has no trailing name),
/// collecting rectangles across `LAYER`/`RECT` runs.
fn parse_port(lex: &mut Lexer, out: &mut Vec<LefRect>) -> Result<(), LefDefError> {
    lex.bump(); // PORT
    let mut current_layer = String::new();
    while let Some(tok) = lex.peek() {
        match tok.text {
            "END" => {
                lex.bump(); // END (unnamed; no trailing name for PORT)
                break;
            }
            "LAYER" => {
                lex.bump();
                current_layer = lex.peek().map(|t| t.text.to_string()).unwrap_or_default();
                skip_statement(lex);
            }
            "RECT" => {
                let coords = parse_rect(lex)?;
                if !current_layer.is_empty() {
                    out.push(LefRect {
                        layer: current_layer.clone(),
                        coords,
                    });
                }
            }
            _ => skip_statement(lex),
        }
    }
    Ok(())
}

/// Parses an `OBS ... END` block (unnamed), collecting rectangles.
fn parse_obs(lex: &mut Lexer, out: &mut Vec<LefRect>) -> Result<(), LefDefError> {
    lex.bump(); // OBS
    let mut current_layer = String::new();
    while let Some(tok) = lex.peek() {
        match tok.text {
            "END" => {
                lex.bump(); // END (unnamed)
                break;
            }
            "LAYER" => {
                lex.bump();
                current_layer = lex.peek().map(|t| t.text.to_string()).unwrap_or_default();
                skip_statement(lex);
            }
            "RECT" => {
                let coords = parse_rect(lex)?;
                if !current_layer.is_empty() {
                    out.push(LefRect {
                        layer: current_layer.clone(),
                        coords,
                    });
                }
            }
            _ => skip_statement(lex),
        }
    }
    Ok(())
}

/// Parses `SIZE <w> BY <h> ;` starting at the `SIZE` keyword.
fn parse_size(lex: &mut Lexer) -> Result<(f64, f64), LefDefError> {
    let line = lex.line();
    lex.bump(); // SIZE
    let w = next_number(lex, "SIZE width")?;
    // Optional `BY` keyword.
    if lex.peek().map(|t| t.text) == Some("BY") {
        lex.bump();
    }
    let h = next_number(lex, "SIZE height")?;
    skip_statement(lex);
    if w < 0.0 || h < 0.0 {
        return Err(LefDefError::lef(line, "SIZE must be non-negative"));
    }
    Ok((w, h))
}

/// Parses `RECT <x1> <y1> <x2> <y2> ;` starting at the `RECT` keyword.
fn parse_rect(lex: &mut Lexer) -> Result<(f64, f64, f64, f64), LefDefError> {
    lex.bump(); // RECT
    let x1 = next_number(lex, "RECT x1")?;
    let y1 = next_number(lex, "RECT y1")?;
    let x2 = next_number(lex, "RECT x2")?;
    let y2 = next_number(lex, "RECT y2")?;
    skip_statement(lex);
    Ok((x1, y1, x2, y2))
}

/// Reads the next token as a required name, erroring if the block ended first.
fn expect_name(lex: &mut Lexer, block: &str) -> Result<String, LefDefError> {
    let line = lex.line();
    match lex.bump() {
        Some(Token { text, .. }) => Ok(text.to_string()),
        None => Err(LefDefError::lef(
            line,
            format!("{block} block ended before its name"),
        )),
    }
}

/// Reads the next token as a required number.
fn next_number(lex: &mut Lexer, what: &str) -> Result<f64, LefDefError> {
    let line = lex.line();
    match lex.bump().and_then(|t| parse_number(t.text)) {
        Some(v) => Ok(v),
        None => Err(LefDefError::lef(
            line,
            format!("expected a number for {what}"),
        )),
    }
}

/// Consumes tokens through the next `;` (or to end of input). Always advances at
/// least one token so a caller loop cannot spin.
fn skip_statement(lex: &mut Lexer) {
    let mut advanced = false;
    while let Some(tok) = lex.bump() {
        advanced = true;
        if tok.text == ";" {
            return;
        }
    }
    // If we consumed nothing (already at end), still record progress via the loop's
    // bump; `advanced` documents intent for readers.
    let _ = advanced;
}

/// Skips tokens until an `END <name>` pair (or end of input) is consumed. Used for
/// whole LEF blocks outside the imported subset. Bounded: a finite token stream is
/// consumed at most once.
fn skip_block(lex: &mut Lexer, name: &str) {
    while let Some(tok) = lex.bump() {
        if tok.text == "END" && lex.peek().is_some_and(|t| t.text == name) {
            lex.bump();
            return;
        }
    }
}

//! The DEF (Design Exchange Format) parser: die area, rows, placement, routed
//! nets, and pins.
//!
//! DEF describes a concrete design built from LEF macros: where the die is, where
//! the rows are, which macro sits at which location and orientation, and how the
//! nets are routed. This parser reads the subset Reticle lowers and overlays:
//! `DESIGN`, `UNITS`, `DIEAREA`, `ROW`, `COMPONENTS`, `PINS`, and `NETS`. Sections
//! outside the subset (`SPECIALNETS`, `GROUPS`, `REGIONS`, `BLOCKAGES`) are skipped
//! with a warning.
//!
//! # Coordinates
//!
//! DEF coordinates are already integer database units (DBU) on the grid set by
//! `UNITS DISTANCE MICRONS`, so this parser keeps them as `i64` and does not scale
//! them. A `*` in a routing coordinate repeats the corresponding coordinate of the
//! previous point, per the DEF routing grammar.

use crate::error::{LefDefError, LefDefWarning, WarningKind};
use crate::lex::{Lexer, parse_number};

/// The parsed content of a DEF file, in DBU.
#[derive(Debug, Default)]
pub(crate) struct DefData {
    /// The `DESIGN` name.
    pub design_name: String,
    /// The database resolution from `UNITS DISTANCE MICRONS`, if declared.
    pub dbu_per_micron: Option<i64>,
    /// The `DIEAREA` corners `(x1, y1, x2, y2)` in DBU, if declared.
    pub die_area: Option<(i64, i64, i64, i64)>,
    /// Placement rows in declaration order.
    pub rows: Vec<DefRow>,
    /// Placed components in declaration order.
    pub components: Vec<DefComponent>,
    /// External pins in declaration order.
    pub pins: Vec<DefPin>,
    /// Routed nets in declaration order.
    pub nets: Vec<DefNet>,
    /// Non-fatal problems found while parsing.
    pub warnings: Vec<LefDefWarning>,
}

/// One `ROW` statement.
#[derive(Debug, Clone)]
pub(crate) struct DefRow {
    /// The row name.
    pub name: String,
    /// The site name tiled along the row.
    pub site: String,
    /// The row origin x in DBU.
    pub orig_x: i64,
    /// The row origin y in DBU.
    pub orig_y: i64,
    /// The site orientation string (`N`, `S`, `FN`, ...).
    pub orient: String,
    /// Number of sites along x.
    pub num_x: u32,
    /// Number of sites along y.
    pub num_y: u32,
    /// Step between sites along x, in DBU.
    pub step_x: i64,
    /// Step between sites along y, in DBU.
    pub step_y: i64,
}

/// One `COMPONENTS` entry.
#[derive(Debug, Clone)]
pub(crate) struct DefComponent {
    /// The instance name.
    pub inst: String,
    /// The referenced macro (cell) name.
    pub macro_name: String,
    /// The placement `(x, y, orient)` in DBU, or `None` when unplaced.
    pub placed: Option<(i64, i64, String)>,
}

/// One `PINS` entry.
#[derive(Debug, Clone, Default)]
pub(crate) struct DefPin {
    /// The pin name.
    pub name: String,
    /// The net the pin connects to (`+ NET`), or empty.
    pub net: String,
    /// The `+ DIRECTION` value, verbatim, or empty.
    pub direction: String,
    /// The `+ LAYER` name of the pin shape, if any.
    pub layer: Option<String>,
    /// The `+ LAYER` rectangle `(x1, y1, x2, y2)` relative to the pin origin, if any.
    pub rect: Option<(i64, i64, i64, i64)>,
    /// The `+ PLACED`/`+ FIXED` location `(x, y, orient)`, if placed.
    pub placed: Option<(i64, i64, String)>,
}

/// One routed net.
#[derive(Debug, Clone, Default)]
pub(crate) struct DefNet {
    /// The net name.
    pub name: String,
    /// The `+ USE` classification, verbatim, or `None`.
    pub use_kind: Option<String>,
    /// The routed segments in declaration order.
    pub segments: Vec<DefSeg>,
}

/// One routed segment: a wire run or a via drop.
#[derive(Debug, Clone)]
pub(crate) enum DefSeg {
    /// A wire on a layer: a polyline in DBU.
    Wire {
        /// The routing-layer name.
        layer: String,
        /// The center-line points in DBU.
        points: Vec<(i64, i64)>,
    },
    /// A via master placed at a point.
    Via {
        /// The via location in DBU.
        at: (i64, i64),
        /// The via master name.
        via: String,
    },
}

/// Parses DEF source into a [`DefData`].
///
/// # Errors
///
/// Returns [`LefDefError::Def`] on a structural failure (a coordinate that does not
/// parse, a statement that ends before its mandatory tokens). Unknown sections are
/// skipped with a warning, not an error.
pub(crate) fn parse(source: &str) -> Result<DefData, LefDefError> {
    let mut lex = Lexer::new(source);
    let mut data = DefData::default();

    while let Some(tok) = lex.peek() {
        match tok.text {
            "DESIGN" => {
                lex.bump();
                data.design_name = lex.bump().map(|t| t.text.to_string()).unwrap_or_default();
                skip_statement(&mut lex);
            }
            "UNITS" => parse_units(&mut lex, &mut data),
            "DIEAREA" => parse_diearea(&mut lex, &mut data)?,
            "ROW" => parse_row(&mut lex, &mut data)?,
            "COMPONENTS" => parse_components(&mut lex, &mut data)?,
            "PINS" => parse_pins(&mut lex, &mut data)?,
            "NETS" => parse_nets(&mut lex, &mut data)?,
            "SPECIALNETS"
            | "GROUPS"
            | "REGIONS"
            | "BLOCKAGES"
            | "SCANCHAINS"
            | "NONDEFAULTRULES"
            | "STYLES"
            | "PROPERTYDEFINITIONS"
            | "VIAS" => {
                let kw = tok.text;
                lex.bump();
                skip_named_section(&mut lex, kw);
                data.warnings.push(LefDefWarning::new(
                    WarningKind::UnsupportedFeature,
                    format!("skipped DEF {kw} section"),
                    format!("the `{kw}` section is outside the imported subset"),
                ));
            }
            "END" => {
                lex.bump(); // END
                lex.bump(); // DESIGN
            }
            _ => skip_statement(&mut lex),
        }
    }

    Ok(data)
}

/// Parses `UNITS DISTANCE MICRONS <n> ;`.
fn parse_units(lex: &mut Lexer, data: &mut DefData) {
    lex.bump(); // UNITS
    if lex.peek().map(|t| t.text) == Some("DISTANCE") {
        lex.bump();
    }
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

/// Parses `DIEAREA ( x1 y1 ) ( x2 y2 ) ;`. A polygonal die area (more than two
/// points) is reduced to its bounding box.
fn parse_diearea(lex: &mut Lexer, data: &mut DefData) -> Result<(), LefDefError> {
    lex.bump(); // DIEAREA
    let mut pts: Vec<(i64, i64)> = Vec::new();
    let mut last = (0, 0);
    while let Some(tok) = lex.peek() {
        match tok.text {
            ";" => {
                lex.bump();
                break;
            }
            "(" => {
                let p = parse_coord_group(lex, last)?;
                last = p;
                pts.push(p);
            }
            _ => {
                lex.bump();
            }
        }
    }
    if let (Some(min), Some(max)) = (
        pts.iter()
            .copied()
            .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1))),
        pts.iter()
            .copied()
            .reduce(|a, b| (a.0.max(b.0), a.1.max(b.1))),
    ) {
        data.die_area = Some((min.0, min.1, max.0, max.1));
    }
    Ok(())
}

/// Parses one `ROW ... ;` statement.
fn parse_row(lex: &mut Lexer, data: &mut DefData) -> Result<(), LefDefError> {
    let line = lex.line();
    lex.bump(); // ROW
    let name = next_word(lex, line, "ROW name")?;
    let site = next_word(lex, line, "ROW site")?;
    let orig_x = next_int(lex, "ROW origX")?;
    let orig_y = next_int(lex, "ROW origY")?;
    let orient = next_word(lex, line, "ROW orient")?;
    let mut row = DefRow {
        name,
        site,
        orig_x,
        orig_y,
        orient,
        num_x: 1,
        num_y: 1,
        step_x: 0,
        step_y: 0,
    };
    while let Some(tok) = lex.peek() {
        match tok.text {
            ";" => {
                lex.bump();
                break;
            }
            "DO" => {
                lex.bump();
                row.num_x = next_int(lex, "ROW DO")?.max(0) as u32;
            }
            "BY" => {
                lex.bump();
                row.num_y = next_int(lex, "ROW BY")?.max(0) as u32;
            }
            "STEP" => {
                lex.bump();
                row.step_x = next_int(lex, "ROW STEP x")?;
                row.step_y = next_int(lex, "ROW STEP y")?;
            }
            _ => {
                lex.bump();
            }
        }
    }
    data.rows.push(row);
    Ok(())
}

/// Parses the `COMPONENTS <n> ; ... END COMPONENTS` section.
fn parse_components(lex: &mut Lexer, data: &mut DefData) -> Result<(), LefDefError> {
    lex.bump(); // COMPONENTS
    skip_statement(lex); // count `;`
    loop {
        match lex.peek().map(|t| t.text) {
            None => break,
            Some("END") => {
                lex.bump(); // END
                lex.bump(); // COMPONENTS
                break;
            }
            Some("-") => {
                let c = parse_component(lex)?;
                data.components.push(c);
            }
            Some(_) => {
                lex.bump();
            }
        }
    }
    Ok(())
}

/// Parses one `- inst macro <clauses> ;` component entry.
fn parse_component(lex: &mut Lexer) -> Result<DefComponent, LefDefError> {
    let line = lex.line();
    lex.bump(); // '-'
    let inst = next_word(lex, line, "component instance")?;
    let macro_name = next_word(lex, line, "component macro")?;
    let mut placed = None;
    while let Some(tok) = lex.peek() {
        match tok.text {
            ";" => {
                lex.bump();
                break;
            }
            "+" => {
                lex.bump(); // '+'
                match lex.bump().map(|t| t.text) {
                    Some("PLACED" | "FIXED" | "COVER") => {
                        placed = Some(parse_placement(lex)?);
                    }
                    _ => skip_to_clause_or_end(lex),
                }
            }
            _ => {
                lex.bump();
            }
        }
    }
    Ok(DefComponent {
        inst,
        macro_name,
        placed,
    })
}

/// Parses the `PINS <n> ; ... END PINS` section.
fn parse_pins(lex: &mut Lexer, data: &mut DefData) -> Result<(), LefDefError> {
    lex.bump(); // PINS
    skip_statement(lex); // count `;`
    loop {
        match lex.peek().map(|t| t.text) {
            None => break,
            Some("END") => {
                lex.bump();
                lex.bump();
                break;
            }
            Some("-") => {
                let p = parse_pin(lex)?;
                data.pins.push(p);
            }
            Some(_) => {
                lex.bump();
            }
        }
    }
    Ok(())
}

/// Parses one `- name + NET ... ;` pin entry.
fn parse_pin(lex: &mut Lexer) -> Result<DefPin, LefDefError> {
    let line = lex.line();
    lex.bump(); // '-'
    let name = next_word(lex, line, "pin name")?;
    let mut pin = DefPin {
        name,
        ..DefPin::default()
    };
    while let Some(tok) = lex.peek() {
        match tok.text {
            ";" => {
                lex.bump();
                break;
            }
            "+" => {
                lex.bump(); // '+'
                match lex.bump().map(|t| t.text) {
                    Some("NET") => {
                        pin.net = lex.bump().map(|t| t.text.to_string()).unwrap_or_default();
                    }
                    Some("DIRECTION") => {
                        pin.direction = lex.bump().map(|t| t.text.to_string()).unwrap_or_default();
                    }
                    Some("LAYER") => {
                        pin.layer = lex.bump().map(|t| t.text.to_string());
                        // Optional two coordinate groups defining the pin rectangle.
                        if lex.peek().map(|t| t.text) == Some("(") {
                            let p1 = parse_coord_group(lex, (0, 0))?;
                            let p2 = parse_coord_group(lex, p1)?;
                            pin.rect = Some((p1.0, p1.1, p2.0, p2.1));
                        }
                    }
                    Some("PLACED" | "FIXED" | "COVER") => {
                        pin.placed = Some(parse_placement(lex)?);
                    }
                    _ => skip_to_clause_or_end(lex),
                }
            }
            _ => {
                lex.bump();
            }
        }
    }
    Ok(pin)
}

/// Parses the `NETS <n> ; ... END NETS` section.
fn parse_nets(lex: &mut Lexer, data: &mut DefData) -> Result<(), LefDefError> {
    lex.bump(); // NETS
    skip_statement(lex); // count `;`
    loop {
        match lex.peek().map(|t| t.text) {
            None => break,
            Some("END") => {
                lex.bump();
                lex.bump();
                break;
            }
            Some("-") => {
                let n = parse_net(lex)?;
                data.nets.push(n);
            }
            Some(_) => {
                lex.bump();
            }
        }
    }
    Ok(())
}

/// Parses one `- name (conn)... + ROUTED ... ;` net entry.
fn parse_net(lex: &mut Lexer) -> Result<DefNet, LefDefError> {
    let line = lex.line();
    lex.bump(); // '-'
    let name = next_word(lex, line, "net name")?;
    let mut net = DefNet {
        name,
        ..DefNet::default()
    };
    while let Some(tok) = lex.peek() {
        match tok.text {
            ";" => {
                lex.bump();
                break;
            }
            "(" => {
                // A `( instance pin )` connection: skip the group.
                skip_paren_group(lex);
            }
            "+" => {
                lex.bump(); // '+'
                match lex.bump().map(|t| t.text) {
                    Some("USE") => {
                        net.use_kind = lex.bump().map(|t| t.text.to_string());
                    }
                    Some("ROUTED" | "FIXED" | "COVER") => {
                        parse_routing(lex, &mut net.segments)?;
                    }
                    _ => skip_to_clause_or_end(lex),
                }
            }
            _ => {
                lex.bump();
            }
        }
    }
    Ok(net)
}

/// Parses a routing body after `ROUTED`/`FIXED`/`COVER`: a first layer name then a
/// run of coordinate groups, `NEW <layer>` breaks, and via masters, stopping at the
/// next clause (`+`) or the statement end (`;`).
fn parse_routing(lex: &mut Lexer, segments: &mut Vec<DefSeg>) -> Result<(), LefDefError> {
    let mut layer = match lex.bump() {
        Some(t) => t.text.to_string(),
        None => return Ok(()),
    };
    let mut points: Vec<(i64, i64)> = Vec::new();
    let mut last = (0, 0);

    let flush = |layer: &str, points: &mut Vec<(i64, i64)>, segments: &mut Vec<DefSeg>| {
        if !points.is_empty() {
            segments.push(DefSeg::Wire {
                layer: layer.to_string(),
                points: std::mem::take(points),
            });
        }
    };

    while let Some(tok) = lex.peek() {
        match tok.text {
            "+" | ";" => break,
            "NEW" => {
                lex.bump();
                flush(&layer, &mut points, segments);
                layer = lex.bump().map(|t| t.text.to_string()).unwrap_or_default();
                last = (0, 0);
            }
            "(" => {
                let p = parse_coord_group(lex, last)?;
                last = p;
                points.push(p);
                // A bare word directly after a coordinate group is a via master.
                if let Some(next) = lex.peek()
                    && is_via_name(next.text)
                {
                    let via = lex.bump().map(|t| t.text.to_string()).unwrap_or_default();
                    segments.push(DefSeg::Via { at: p, via });
                }
            }
            "RECT" => {
                // A `RECT ( dx1 dy1 dx2 dy2 )` patch inside routing: skip it.
                lex.bump();
                skip_paren_group(lex);
            }
            _ => {
                lex.bump();
            }
        }
    }
    flush(&layer, &mut points, segments);
    Ok(())
}

/// Parses `( x y ) <orient>` starting at the `(`.
fn parse_placement(lex: &mut Lexer) -> Result<(i64, i64, String), LefDefError> {
    let (x, y) = parse_coord_group(lex, (0, 0))?;
    let orient = lex.bump().map(|t| t.text.to_string()).unwrap_or_default();
    Ok((x, y, orient))
}

/// Parses a `( x y [ext] )` coordinate group starting at the `(`. A `*` in either
/// slot repeats the corresponding coordinate of `prev`. Extra tokens before the
/// closing `)` (an extension value) are consumed.
fn parse_coord_group(lex: &mut Lexer, prev: (i64, i64)) -> Result<(i64, i64), LefDefError> {
    let line = lex.line();
    if lex.bump().map(|t| t.text) != Some("(") {
        return Err(LefDefError::def(line, "expected `(` starting a coordinate"));
    }
    let x = read_coord(lex, prev.0)?;
    let y = read_coord(lex, prev.1)?;
    // Consume any trailing tokens (an extension value) up to the closing `)`.
    let mut guard = 0u32;
    while let Some(tok) = lex.peek() {
        if tok.text == ")" {
            lex.bump();
            break;
        }
        if matches!(tok.text, "(" | "+" | ";") || guard > 64 {
            break;
        }
        lex.bump();
        guard += 1;
    }
    Ok((x, y))
}

/// Reads one coordinate token: a number rounded to DBU, or `*` repeating `prev`.
fn read_coord(lex: &mut Lexer, prev: i64) -> Result<i64, LefDefError> {
    let line = lex.line();
    match lex.bump() {
        Some(t) if t.text == "*" => Ok(prev),
        Some(t) => match parse_number(t.text) {
            Some(v) => Ok(v.round() as i64),
            None => Err(LefDefError::def(
                line,
                format!("expected a coordinate, found `{}`", t.text),
            )),
        },
        None => Err(LefDefError::def(line, "coordinate ended early")),
    }
}

/// A token that follows a coordinate group and is a via master, not the start of
/// the next point or clause.
fn is_via_name(text: &str) -> bool {
    !matches!(
        text,
        "(" | ")" | "+" | ";" | "NEW" | "RECT" | "TAPER" | "MASK" | "STYLE"
    )
}

/// Reads the next token as a required word.
fn next_word(lex: &mut Lexer, line: usize, what: &str) -> Result<String, LefDefError> {
    match lex.bump() {
        Some(t) => Ok(t.text.to_string()),
        None => Err(LefDefError::def(line, format!("expected {what}"))),
    }
}

/// Reads the next token as a required integer (DBU), rounding a decimal.
fn next_int(lex: &mut Lexer, what: &str) -> Result<i64, LefDefError> {
    let line = lex.line();
    match lex.bump().and_then(|t| parse_number(t.text)) {
        Some(v) => Ok(v.round() as i64),
        None => Err(LefDefError::def(
            line,
            format!("expected an integer for {what}"),
        )),
    }
}

/// Consumes tokens through the next `;` (or to end of input).
fn skip_statement(lex: &mut Lexer) {
    while let Some(tok) = lex.bump() {
        if tok.text == ";" {
            return;
        }
    }
}

/// Peeks forward to the next clause (`+`) or statement end (`;`) without consuming
/// it, discarding the tokens in between.
fn skip_to_clause_or_end(lex: &mut Lexer) {
    while let Some(tok) = lex.peek() {
        if matches!(tok.text, "+" | ";") {
            return;
        }
        lex.bump();
    }
}

/// Consumes a balanced `( ... )` group starting at the `(` (or a stray token if the
/// next token is not `(`). Bounded and non-nesting: DEF connection and rect groups
/// do not nest parentheses.
fn skip_paren_group(lex: &mut Lexer) {
    if lex.peek().map(|t| t.text) != Some("(") {
        lex.bump();
        return;
    }
    while let Some(tok) = lex.bump() {
        if tok.text == ")" {
            return;
        }
    }
}

/// Skips a named DEF section (`<KW> ... END <KW>`) after its keyword was consumed.
fn skip_named_section(lex: &mut Lexer, kw: &str) {
    while let Some(tok) = lex.bump() {
        if tok.text == "END" && lex.peek().is_some_and(|t| t.text == kw) {
            lex.bump();
            return;
        }
    }
}

//! The Reticle technology-file text format and its parser.
//!
//! A technology file describes the database resolution, the layer table, and the
//! declarative DRC rules for a process. The format is a simple, line-oriented
//! text format designed to be hand-writable and diff-friendly.
//!
//! # Grammar
//!
//! One directive per line. Tokens are whitespace-separated. Blank lines and lines
//! whose first non-whitespace character is `#` are ignored. Directive keywords
//! are case-insensitive; layer names are kept verbatim.
//!
//! ```text
//! # A comment.
//! technology <name>                              # optional; sets Technology.name
//! dbu_per_micron <integer>                       # database units per micron (> 0)
//! layer <layer> <datatype> <name> <rgba_hex>     # one layer-table entry
//! rule <kind> <layer> <datatype> <value>         # single-layer rule
//! rule <kind> <layer> <datatype> <olayer> <odatatype> <value>  # two-layer rule
//! ```
//!
//! * `<rgba_hex>` is 8 hex digits `RRGGBBAA` (an optional leading `#` or `0x` is
//!   accepted), e.g. `FF0000FF` for opaque red.
//! * `<kind>` is one of `width`, `spacing`, `enclosure`, `extension`, `notch`,
//!   `area`, `density`, `angle` (see [`RuleKind`]). The two-layer form is used for
//!   `spacing`, `enclosure`, and `extension`; the single-layer form for the rest.
//! * `<value>` is the rule threshold: DBU for length rules, DBU² for `area`,
//!   milli-degrees for `angle`.
//!
//! # Example
//!
//! ```text
//! technology demo_process
//! dbu_per_micron 1000
//! layer 1 0 metal1 4488FFFF
//! layer 2 0 via1   888888FF
//! rule width   1 0 100
//! rule spacing 1 0 140
//! rule enclosure 2 0 1 0 20
//! ```

use crate::IoError;
use reticle_geometry::LayerId;
use reticle_model::{LayerInfo, Result, Rule, RuleKind, Technology};

/// Parses a Reticle technology file into a [`Technology`].
///
/// See the [module documentation](self) for the full grammar. Layers and rules
/// appear in the order they are declared. A missing `dbu_per_micron` directive
/// leaves the resolution at its default of `0`; callers that require a resolution
/// should validate it.
///
/// # Errors
///
/// Returns a [`reticle_model::ModelError`] if any line is malformed: an unknown
/// directive, the wrong number of tokens, a non-numeric where a number is
/// expected, a bad color, or an unknown rule kind. The underlying [`IoError`]
/// (available via the crate's own paths) names the offending line number.
pub fn parse_technology(source: &str) -> Result<Technology> {
    let mut tech = Technology::default();

    for (idx, raw_line) in source.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let mut tokens = line.split_whitespace();
        // Safe: `line` is non-empty after trimming, so at least one token exists.
        let directive = tokens.next().unwrap_or_default().to_ascii_lowercase();
        let rest: Vec<&str> = tokens.collect();

        match directive.as_str() {
            "technology" => parse_technology_name(&rest, line_no, &mut tech)?,
            "dbu_per_micron" => tech.dbu_per_micron = parse_dbu(&rest, line_no)?,
            "layer" => tech.layers.push(parse_layer(&rest, line_no)?),
            "rule" => tech.rules.push(parse_rule(&rest, line_no)?),
            other => {
                return Err(IoError::tech(line_no, format!("unknown directive `{other}`")).into());
            }
        }
    }

    Ok(tech)
}

/// Removes a `#` comment from a line.
///
/// A `#` starts a comment when it begins the (trimmed) line, or when it is
/// preceded by whitespace **and** followed by whitespace or the line end. This
/// lets `#` double as the optional prefix of an `#RRGGBBAA` color token — there a
/// `#` is glued to hex digits, so it is not treated as a comment — while a
/// free-standing `# ...` still comments the rest of the line.
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut prev_ws = true; // treat line start as if preceded by whitespace
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && prev_ws {
            let next_ws = bytes.get(i + 1).is_none_or(u8::is_ascii_whitespace);
            if next_ws {
                return &line[..i];
            }
        }
        prev_ws = b.is_ascii_whitespace();
    }
    line
}

/// Parses the `technology <name>` directive.
fn parse_technology_name(rest: &[&str], line_no: usize, tech: &mut Technology) -> Result<()> {
    if rest.len() != 1 {
        return Err(IoError::tech(line_no, "`technology` expects exactly one name").into());
    }
    tech.name = rest[0].to_string();
    Ok(())
}

/// Parses the `dbu_per_micron <integer>` directive.
fn parse_dbu(rest: &[&str], line_no: usize) -> Result<i64> {
    let [value] = rest else {
        return Err(IoError::tech(line_no, "`dbu_per_micron` expects one integer").into());
    };
    let dbu: i64 = value
        .parse()
        .map_err(|_| IoError::tech(line_no, format!("invalid integer `{value}`")))?;
    if dbu <= 0 {
        return Err(IoError::tech(line_no, "`dbu_per_micron` must be positive").into());
    }
    Ok(dbu)
}

/// Parses a `layer <layer> <datatype> <name> <rgba_hex>` directive.
fn parse_layer(rest: &[&str], line_no: usize) -> Result<LayerInfo> {
    let [layer, datatype, name, rgba] = rest else {
        return Err(IoError::tech(
            line_no,
            "`layer` expects: <layer> <datatype> <name> <rgba_hex>",
        )
        .into());
    };
    let layer = parse_u16(layer, line_no, "layer")?;
    let datatype = parse_u16(datatype, line_no, "datatype")?;
    let color_rgba = parse_rgba(rgba, line_no)?;
    Ok(LayerInfo {
        id: LayerId::new(layer, datatype),
        name: (*name).to_string(),
        color_rgba,
        visible: true,
    })
}

/// Parses a `rule <kind> ...` directive in either the single- or two-layer form.
fn parse_rule(rest: &[&str], line_no: usize) -> Result<Rule> {
    if rest.len() < 4 {
        return Err(IoError::tech(
            line_no,
            "`rule` expects at least a kind, layer, datatype, and value",
        )
        .into());
    }
    let kind = parse_rule_kind(rest[0], line_no)?;
    let layer = parse_u16(rest[1], line_no, "layer")?;
    let datatype = parse_u16(rest[2], line_no, "datatype")?;

    // Four data tokens => single-layer rule; six => two-layer rule.
    let (other_layer, value_token) = match rest.len() {
        4 => (None, rest[3]),
        6 => {
            let ol = parse_u16(rest[3], line_no, "other layer")?;
            let od = parse_u16(rest[4], line_no, "other datatype")?;
            (Some(LayerId::new(ol, od)), rest[5])
        }
        _ => {
            return Err(IoError::tech(
                line_no,
                "`rule` takes 4 tokens (single-layer) or 6 tokens (two-layer)",
            )
            .into());
        }
    };
    let value: i64 = value_token
        .parse()
        .map_err(|_| IoError::tech(line_no, format!("invalid rule value `{value_token}`")))?;

    Ok(Rule {
        name: format!("{}_{}_{}", rest[0].to_ascii_lowercase(), layer, datatype),
        kind,
        layer: LayerId::new(layer, datatype),
        other_layer,
        value,
    })
}

/// Maps a rule-kind keyword to a [`RuleKind`].
fn parse_rule_kind(token: &str, line_no: usize) -> Result<RuleKind> {
    let kind = match token.to_ascii_lowercase().as_str() {
        "width" => RuleKind::Width,
        "spacing" => RuleKind::Spacing,
        "enclosure" => RuleKind::Enclosure,
        "extension" => RuleKind::Extension,
        "notch" => RuleKind::Notch,
        "area" => RuleKind::Area,
        "density" => RuleKind::Density,
        "angle" => RuleKind::Angle,
        other => {
            return Err(IoError::tech(line_no, format!("unknown rule kind `{other}`")).into());
        }
    };
    Ok(kind)
}

/// Parses a `u16` field, naming the field in any error.
fn parse_u16(token: &str, line_no: usize, field: &str) -> Result<u16> {
    token
        .parse()
        .map_err(|_| IoError::tech(line_no, format!("invalid {field} number `{token}`")).into())
}

/// Parses an `RRGGBBAA` hex color, accepting an optional `#` or `0x` prefix.
fn parse_rgba(token: &str, line_no: usize) -> Result<u32> {
    let hex = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
        .or_else(|| token.strip_prefix('#'))
        .unwrap_or(token);
    if hex.len() != 8 {
        return Err(IoError::tech(
            line_no,
            format!("color `{token}` must be 8 hex digits (RRGGBBAA)"),
        )
        .into());
    }
    u32::from_str_radix(hex, 16)
        .map_err(|_| IoError::tech(line_no, format!("invalid hex color `{token}`")).into())
}

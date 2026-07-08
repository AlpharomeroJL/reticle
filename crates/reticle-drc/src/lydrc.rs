//! Compatibility parser for a documented subset of `KLayout` `.lydrc` DRC decks.
//!
//! A `.lydrc` file is `KLayout`'s *DRC macro* format: an XML wrapper
//! (`<klayout-macro>` with `<category>drc</category>`,
//! `<interpreter>dsl</interpreter>`, `<dsl-interpreter-name>drc-dsl-xml</dsl-interpreter-name>`)
//! whose `<text>` element holds a Ruby-based DRC DSL script. This module extracts
//! that script and compiles the **supported subset** of it down to the engine's
//! [`Rule`] vocabulary, so a real `KLayout` rule deck can be run by [`DrcEngine`].
//! Anything outside the subset fails with a clear [`LydrcError`] naming the
//! construct and the line, never a panic and never a silently dropped rule.
//!
//! [`DrcEngine`]: crate::DrcEngine
//!
//! # Supported subset
//!
//! The syntax is pinned to the current `KLayout` DRC reference (fetched at build
//! time, not memory), cited in ADR 0063 and the book chapter *`KLayout` `.lydrc`
//! compatibility*:
//! <https://www.klayout.de/doc/about/drc_ref_layer.html> and
//! <https://www.klayout.de/doc/about/drc_ref_global.html>.
//!
//! * **Layer inputs** -`name = input(layer)` / `name = input(layer, datatype)`.
//! * **Header directives** -`source(...)` and `report(...)` are recognized and
//!   ignored (they configure `KLayout` I/O, not rules).
//! * **Single-layer checks** -`layer.width(v)`, `layer.space(v)`,
//!   `layer.notch(v)`.
//! * **Two-layer checks** -`layer.separation(other, v)` (alias `sep`) and
//!   `outer.enclosing(inner, v)`.
//! * **Minimum area** -`layer.with_area(0, v)` / `with_area(0.0, v)` /
//!   `with_area(nil, v)` (a pure below-threshold selection).
//! * **Reporting** -an optional trailing `.output("name"[, "description"])`
//!   names the rule; the name is the rule id carried on each [`Violation`].
//!
//! [`Violation`]: reticle_model::Violation
//!
//! ## Units
//!
//! Following the `KLayout` DSL, a **floating-point** dimension is micrometres and an
//! **integer** dimension is database units; an explicit `.um` or `.dbu` suffix
//! overrides. Reticle's DBU are nanometres (1 dbu = 1 nm, as in `sky130.rs`), so a
//! µm value is scaled by 1000 (areas by `1_000_000`) into dbu.
//!
//! ## The enclosing swap
//!
//! `KLayout` writes `outer.enclosing(inner, v)` -the *receiver* is the enclosing
//! (outer) layer. The engine's [`Rule`] models enclosure the other way round:
//! `layer` is the enclosed (inner) shape and `other_layer` is the enclosing
//! (outer) shape. The parser swaps the two so the verdicts agree.

use reticle_geometry::LayerId;
use reticle_model::{Rule, RuleKind};
use std::collections::HashMap;
use std::fmt;

/// Why a `.lydrc` deck could not be compiled to engine rules.
///
/// Both variants carry the 1-based line number within the extracted DRC script so
/// the message can point straight at the offending line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LydrcError {
    /// A construct that is valid `KLayout` DRC but outside the supported subset.
    Unsupported {
        /// The offending construct, e.g. `"met1.sized"` or `"connect"`.
        construct: String,
        /// 1-based line within the extracted DRC script.
        line: usize,
    },
    /// A malformed or unresolvable statement (bad number, unknown layer, junk).
    Syntax {
        /// What is wrong, in human terms.
        message: String,
        /// 1-based line within the extracted DRC script.
        line: usize,
    },
}

impl fmt::Display for LydrcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LydrcError::Unsupported { construct, line } => write!(
                f,
                "line {line}: unsupported .lydrc construct `{construct}` (outside the supported subset)"
            ),
            LydrcError::Syntax { message, line } => write!(f, "line {line}: {message}"),
        }
    }
}

impl std::error::Error for LydrcError {}

/// Parses a `.lydrc` deck (or a bare `.drc` script) into engine [`Rule`]s.
///
/// Extracts the DRC script from the XML macro wrapper if present, then compiles
/// each supported statement. Returns the first [`LydrcError`] encountered; the
/// input is arbitrary untrusted text and is never allowed to panic or hang.
pub fn parse_lydrc(text: &str) -> Result<Vec<Rule>, LydrcError> {
    let script = extract_script(text);
    let mut layers: HashMap<String, LayerId> = HashMap::new();
    let mut rules: Vec<Rule> = Vec::new();

    for (idx, raw_line) in script.lines().enumerate() {
        let line = idx + 1;
        let stmt = strip_comment(raw_line).trim();
        if stmt.is_empty() {
            continue;
        }
        parse_statement(stmt, line, &mut layers, &mut rules)?;
    }
    Ok(rules)
}

/// Pulls the DRC DSL script out of a `.lydrc` XML macro wrapper.
///
/// If the text contains a `<text>...</text>` element (the `KLayout` macro body), its
/// XML-unescaped contents are returned; otherwise the input is treated as a bare
/// `.drc` script and returned unchanged. Only the first `<text>` open tag and the
/// last `</text>` close tag are considered, so the scan is linear and cannot blow
/// up on adversarial nesting.
fn extract_script(text: &str) -> String {
    const OPEN: &str = "<text>";
    const CLOSE: &str = "</text>";
    if let Some(start) = text.find(OPEN)
        && let Some(end) = text.rfind(CLOSE)
        && start + OPEN.len() <= end
    {
        let inner = &text[start + OPEN.len()..end];
        return xml_unescape(inner);
    }
    text.to_owned()
}

/// Unescapes the five predefined XML entities. No numeric character references are
/// interpreted (the macro body only needs the named five), keeping this total and
/// allocation-bounded on untrusted input.
fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Removes a trailing Ruby `#` line comment, respecting double-quoted strings so a
/// `#` inside an `.output("...")` label is not mistaken for a comment.
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Compiles one non-empty statement into the layer table or the rule list.
fn parse_statement(
    stmt: &str,
    line: usize,
    layers: &mut HashMap<String, LayerId>,
    rules: &mut Vec<Rule>,
) -> Result<(), LydrcError> {
    // Layer declaration: `name = input(...)`.
    if let Some((lhs, rhs)) = split_assignment(stmt) {
        let name = lhs.trim();
        if !is_ident(name) {
            return Err(LydrcError::Syntax {
                message: format!("`{name}` is not a valid layer name"),
                line,
            });
        }
        let layer = parse_input(rhs.trim(), line)?;
        layers.insert(name.to_owned(), layer);
        return Ok(());
    }

    // Header directives that configure `KLayout` I/O, not rules: recognized, ignored.
    if let Some((head, _)) = split_call(stmt)
        && matches!(head, "source" | "report")
    {
        return Ok(());
    }

    // Otherwise it must be a check chain: `receiver.method(args)[.output(...)]`.
    let rule = parse_check_chain(stmt, line, layers)?;
    rules.push(rule);
    Ok(())
}

/// Splits `lhs = rhs` at the first top-level `=` that is not part of `==`, `<=`,
/// `>=`, or `!=`. Returns `None` when there is no assignment.
fn split_assignment(stmt: &str) -> Option<(&str, &str)> {
    let bytes = stmt.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'=' {
            let prev = if i > 0 { bytes[i - 1] } else { b' ' };
            let next = if i + 1 < bytes.len() {
                bytes[i + 1]
            } else {
                b' '
            };
            if next == b'=' || matches!(prev, b'=' | b'<' | b'>' | b'!') {
                continue;
            }
            return Some((&stmt[..i], &stmt[i + 1..]));
        }
    }
    None
}

/// Splits `name(args)` into the callee name and the raw argument string (without
/// the outer parentheses). Returns `None` if the statement is not a single call.
fn split_call(stmt: &str) -> Option<(&str, &str)> {
    let open = stmt.find('(')?;
    if !stmt.ends_with(')') {
        return None;
    }
    let name = stmt[..open].trim();
    if !is_ident(name) {
        return None;
    }
    let args = &stmt[open + 1..stmt.len() - 1];
    Some((name, args))
}

/// Parses `input(layer)` or `input(layer, datatype)` into a [`LayerId`].
fn parse_input(rhs: &str, line: usize) -> Result<LayerId, LydrcError> {
    let (name, args) = split_call(rhs).ok_or_else(|| LydrcError::Syntax {
        message: format!("expected `input(...)` on the right-hand side, found `{rhs}`"),
        line,
    })?;
    if name != "input" {
        return Err(LydrcError::Unsupported {
            construct: name.to_owned(),
            line,
        });
    }
    let parts = split_args(args);
    let layer = parse_u16(parts.first().copied().unwrap_or(""), line, "layer")?;
    let datatype = match parts.get(1) {
        Some(dt) => parse_u16(dt, line, "datatype")?,
        None => 0,
    };
    if parts.len() > 2 {
        return Err(LydrcError::Syntax {
            message: "input(...) takes a layer and an optional datatype".to_owned(),
            line,
        });
    }
    Ok(LayerId::new(layer, datatype))
}

/// Parses a `receiver.method(args)[.output(...)]` check chain into a [`Rule`].
fn parse_check_chain(
    stmt: &str,
    line: usize,
    layers: &HashMap<String, LayerId>,
) -> Result<Rule, LydrcError> {
    let (receiver, calls) = parse_calls(stmt, line)?;
    let receiver_layer = lookup_layer(receiver, line, layers)?;

    // The last `.output(...)` names the rule; the first non-output call is the check.
    let mut name: Option<String> = None;
    let mut check: Option<(&str, &str)> = None;
    for (method, args) in &calls {
        if *method == "output" {
            name = Some(parse_output_name(args, line)?);
        } else if check.is_none() {
            check = Some((method, args));
        } else {
            return Err(LydrcError::Unsupported {
                construct: format!("{receiver}.{method} (chained check)"),
                line,
            });
        }
    }
    let (method, args) = check.ok_or_else(|| LydrcError::Syntax {
        message: format!("`{stmt}` has no DRC check method"),
        line,
    })?;

    build_rule(receiver, receiver_layer, method, args, name, line, layers)
}

/// Builds the [`Rule`] for a recognized check `method`, or errors if the method is
/// outside the supported subset.
fn build_rule(
    receiver: &str,
    receiver_layer: LayerId,
    method: &str,
    args: &str,
    name: Option<String>,
    line: usize,
    layers: &HashMap<String, LayerId>,
) -> Result<Rule, LydrcError> {
    let rule_name = move |default: &str| name.clone().unwrap_or_else(|| default.to_owned());
    match method {
        // --- Single-layer checks ---------------------------------------------
        "width" => single_layer(
            RuleKind::Width,
            receiver,
            receiver_layer,
            "width",
            args,
            &rule_name,
            line,
        ),
        "space" => single_layer(
            RuleKind::Spacing,
            receiver,
            receiver_layer,
            "space",
            args,
            &rule_name,
            line,
        ),
        "notch" => single_layer(
            RuleKind::Notch,
            receiver,
            receiver_layer,
            "notch",
            args,
            &rule_name,
            line,
        ),
        // --- Two-layer checks ------------------------------------------------
        // `a.separation(b, v)` -> spacing between layer a and layer b.
        "separation" | "sep" => {
            let (other, value) = two_layer_args(args, line, method, layers)?;
            Ok(Rule {
                name: rule_name(&format!("{receiver}.{method}")),
                kind: RuleKind::Spacing,
                layer: receiver_layer,
                other_layer: Some(other),
                value,
            })
        }
        // `outer.enclosing(inner, v)`: the receiver is the *enclosing* (outer)
        // layer, so it maps to the engine's `other_layer`, and the argument (the
        // enclosed inner layer) maps to `layer`. See the module docs.
        "enclosing" | "enclosure" | "enc" => {
            let (inner, value) = two_layer_args(args, line, method, layers)?;
            Ok(Rule {
                name: rule_name(&format!("{receiver}.{method}")),
                kind: RuleKind::Enclosure,
                layer: inner,
                other_layer: Some(receiver_layer),
                value,
            })
        }
        // --- Minimum area ----------------------------------------------------
        // `layer.with_area(lower, upper)` with a zero/nil lower bound selects
        // polygons below `upper`, i.e. a minimum-area rule of threshold `upper`.
        "with_area" => {
            let value = parse_min_area(args, line)?;
            Ok(Rule {
                name: rule_name(&format!("{receiver}.with_area")),
                kind: RuleKind::Area,
                layer: receiver_layer,
                other_layer: None,
                value,
            })
        }
        _ => Err(LydrcError::Unsupported {
            construct: format!("{receiver}.{method}"),
            line,
        }),
    }
}

/// Builds a single-value, single-layer rule (`width`/`space`/`notch`).
fn single_layer(
    kind: RuleKind,
    receiver: &str,
    layer: LayerId,
    method: &str,
    args: &str,
    rule_name: &dyn Fn(&str) -> String,
    line: usize,
) -> Result<Rule, LydrcError> {
    let value = parse_dimension(single_arg(args, line, method)?, line)?;
    Ok(Rule {
        name: rule_name(&format!("{receiver}.{method}")),
        kind,
        layer,
        other_layer: None,
        value,
    })
}

/// Parses `(other_layer, value)` for a two-layer check, resolving the layer name.
fn two_layer_args(
    args: &str,
    line: usize,
    method: &str,
    layers: &HashMap<String, LayerId>,
) -> Result<(LayerId, i64), LydrcError> {
    let parts = split_args(args);
    let [layer_arg, value_arg] = parts.as_slice() else {
        return Err(LydrcError::Syntax {
            message: format!("{method}(...) takes a layer and a value"),
            line,
        });
    };
    let other = lookup_layer(layer_arg.trim(), line, layers)?;
    let value = parse_dimension(value_arg, line)?;
    Ok((other, value))
}

/// Parses the minimum-area threshold from a `with_area(lower, upper)` call.
///
/// Only the below-threshold form is in the supported subset: the lower bound must
/// be `0`, `0.0`, or `nil`, and the upper bound is the minimum area. A two-sided
/// band (both bounds positive) has no single-threshold engine equivalent and is
/// reported as an unsupported construct.
fn parse_min_area(args: &str, line: usize) -> Result<i64, LydrcError> {
    let parts = split_args(args);
    let [lower, upper] = parts.as_slice() else {
        return Err(LydrcError::Unsupported {
            construct: "with_area (only the `with_area(0, min)` below-threshold form is supported)"
                .to_owned(),
            line,
        });
    };
    let lower = lower.trim();
    if !matches!(lower, "0" | "0.0" | "nil") {
        return Err(LydrcError::Unsupported {
            construct: "with_area (only a zero/nil lower bound is supported)".to_owned(),
            line,
        });
    }
    parse_area(upper, line)
}

/// Parses a `KLayout` area literal into dbu². A floating-point literal is µm²
/// (scaled by `1_000_000`), an integer literal is already dbu².
fn parse_area(s: &str, line: usize) -> Result<i64, LydrcError> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix(".dbu") {
        return parse_i64(num.trim(), line);
    }
    if s.contains('.') {
        let um2: f64 = s.parse().map_err(|_| LydrcError::Syntax {
            message: format!("`{s}` is not a number"),
            line,
        })?;
        if !um2.is_finite() {
            return Err(LydrcError::Syntax {
                message: format!("`{s}` is not a finite area"),
                line,
            });
        }
        Ok((um2 * 1_000_000.0).round() as i64)
    } else {
        parse_i64(s, line)
    }
}

/// Resolves a declared layer name, erroring if it was never `input`-declared.
fn lookup_layer(
    name: &str,
    line: usize,
    layers: &HashMap<String, LayerId>,
) -> Result<LayerId, LydrcError> {
    layers.get(name).copied().ok_or_else(|| LydrcError::Syntax {
        message: format!("layer `{name}` used before it was declared with `input(...)`"),
        line,
    })
}

/// A parsed check chain: the receiver identifier and each `(method, raw_args)`
/// call in order, all borrowed from the source statement.
type CheckChain<'a> = (&'a str, Vec<(&'a str, &'a str)>);

/// Splits a `receiver.a(...).b(...)` chain into the receiver identifier and each
/// `(method, raw_args)` call, respecting nested parentheses and quoted strings.
fn parse_calls(stmt: &str, line: usize) -> Result<CheckChain<'_>, LydrcError> {
    let dot = stmt.find('.').ok_or_else(|| LydrcError::Syntax {
        message: format!("`{stmt}` is not a recognized statement"),
        line,
    })?;
    let receiver = stmt[..dot].trim();
    if !is_ident(receiver) {
        return Err(LydrcError::Syntax {
            message: format!("`{receiver}` is not a valid layer name"),
            line,
        });
    }
    let mut calls = Vec::new();
    let mut rest = &stmt[dot + 1..];
    while !rest.is_empty() {
        let open = rest.find('(').ok_or_else(|| LydrcError::Syntax {
            message: format!("expected `(` after `.{rest}`"),
            line,
        })?;
        let method = rest[..open].trim();
        if !is_ident(method) {
            return Err(LydrcError::Syntax {
                message: format!("`{method}` is not a valid method name"),
                line,
            });
        }
        let close = matching_paren(rest, open).ok_or_else(|| LydrcError::Syntax {
            message: "unbalanced parentheses".to_owned(),
            line,
        })?;
        calls.push((method, &rest[open + 1..close]));
        rest = rest[close + 1..].trim_start();
        // Between calls only a `.` may appear (method chaining).
        if let Some(stripped) = rest.strip_prefix('.') {
            rest = stripped;
        } else if !rest.is_empty() {
            return Err(LydrcError::Syntax {
                message: format!("unexpected `{rest}` after a method call"),
                line,
            });
        }
    }
    Ok((receiver, calls))
}

/// Finds the index of the `)` matching the `(` at `open`, honoring nested parens
/// and double-quoted strings. Returns `None` on imbalance.
fn matching_paren(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    for (i, b) in s.bytes().enumerate().skip(open) {
        match b {
            b'"' => in_str = !in_str,
            b'(' if !in_str => depth += 1,
            b')' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Extracts the first string-literal argument of `output("name"[, "desc"])`.
fn parse_output_name(args: &str, line: usize) -> Result<String, LydrcError> {
    let first = split_args(args).into_iter().next().unwrap_or("").trim();
    parse_string_literal(first).ok_or_else(|| LydrcError::Syntax {
        message: "output(...) needs a quoted rule name".to_owned(),
        line,
    })
}

/// Parses a double-quoted string literal, returning its contents.
fn parse_string_literal(s: &str) -> Option<String> {
    let s = s.trim();
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.to_owned())
}

/// Splits a raw argument string on top-level commas, respecting nested parens and
/// quoted strings. Empty input yields an empty vector.
fn split_args(args: &str) -> Vec<&str> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut start = 0usize;
    for (i, b) in args.bytes().enumerate() {
        match b {
            b'"' => in_str = !in_str,
            b'(' if !in_str => depth += 1,
            b')' if !in_str => depth -= 1,
            b',' if !in_str && depth == 0 => {
                parts.push(args[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(args[start..].trim());
    parts
}

/// Requires exactly one argument for a single-value check like `width`/`space`.
fn single_arg<'a>(args: &'a str, line: usize, method: &str) -> Result<&'a str, LydrcError> {
    let parts = split_args(args);
    match parts.as_slice() {
        [one] => Ok(*one),
        _ => Err(LydrcError::Syntax {
            message: format!("{method}(...) takes exactly one value"),
            line,
        }),
    }
}

/// Parses a `KLayout` dimension into database units.
///
/// A floating-point literal is micrometres, an integer literal is database units,
/// and an explicit `.um`/`.dbu` suffix overrides. Micrometres are scaled by 1000
/// (1 dbu = 1 nm).
fn parse_dimension(s: &str, line: usize) -> Result<i64, LydrcError> {
    let s = s.trim();
    if let Some(num) = s.strip_suffix(".um") {
        return um_to_dbu(num.trim(), line);
    }
    if let Some(num) = s.strip_suffix(".dbu") {
        return parse_i64(num.trim(), line);
    }
    // No unit suffix: a decimal point means micrometres, otherwise database units.
    if s.contains('.') {
        um_to_dbu(s, line)
    } else {
        parse_i64(s, line)
    }
}

/// Converts a micrometre literal to database units (nanometres), rounding to the
/// nearest whole dbu.
fn um_to_dbu(s: &str, line: usize) -> Result<i64, LydrcError> {
    let um: f64 = s.parse().map_err(|_| LydrcError::Syntax {
        message: format!("`{s}` is not a number"),
        line,
    })?;
    if !um.is_finite() {
        return Err(LydrcError::Syntax {
            message: format!("`{s}` is not a finite dimension"),
            line,
        });
    }
    Ok((um * 1000.0).round() as i64)
}

/// Parses a plain integer database-unit value.
fn parse_i64(s: &str, line: usize) -> Result<i64, LydrcError> {
    s.trim().parse().map_err(|_| LydrcError::Syntax {
        message: format!("`{s}` is not an integer"),
        line,
    })
}

/// Parses a GDS layer or datatype number (0..=65535).
fn parse_u16(s: &str, line: usize, what: &str) -> Result<u16, LydrcError> {
    s.trim().parse().map_err(|_| LydrcError::Syntax {
        message: format!("`{}` is not a valid {what} number", s.trim()),
        line,
    })
}

/// Whether `s` is a non-empty Ruby-style identifier (letter/underscore start).
fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses `deck` and returns the single rule it must contain.
    fn one_rule(deck: &str) -> Rule {
        let mut rules = parse_lydrc(deck).expect("valid deck");
        assert_eq!(rules.len(), 1, "expected exactly one rule");
        rules.pop().unwrap()
    }

    #[test]
    fn parses_a_single_width_rule() {
        let deck = "met1 = input(68, 20)\nmet1.width(0.14).output(\"m1.1\", \"min width\")\n";
        let r = one_rule(deck);
        assert_eq!(r.name, "m1.1");
        assert_eq!(r.kind, RuleKind::Width);
        assert_eq!(r.layer, LayerId::new(68, 20));
        assert_eq!(r.other_layer, None);
        assert_eq!(r.value, 140, "0.14 um -> 140 dbu");
    }

    #[test]
    fn parses_single_layer_space() {
        let deck = "met1 = input(68, 20)\nmet1.space(0.14).output(\"m1.2\")\n";
        let r = one_rule(deck);
        assert_eq!(r.kind, RuleKind::Spacing);
        assert_eq!(r.layer, LayerId::new(68, 20));
        assert_eq!(r.other_layer, None);
        assert_eq!(r.value, 140);
    }

    #[test]
    fn parses_notch() {
        let deck = "li = input(67, 20)\nli.notch(0.17).output(\"li.notch\")\n";
        let r = one_rule(deck);
        assert_eq!(r.kind, RuleKind::Notch);
        assert_eq!(r.layer, LayerId::new(67, 20));
        assert_eq!(r.value, 170);
    }

    #[test]
    fn parses_two_layer_separation_and_sep_alias() {
        let deck = "\
            a = input(65, 20)\n\
            b = input(66, 20)\n\
            a.separation(b, 0.2).output(\"sep1\")\n\
            a.sep(b, 0.3).output(\"sep2\")\n";
        let rules = parse_lydrc(deck).expect("valid deck");
        assert_eq!(rules.len(), 2);
        for r in &rules {
            assert_eq!(r.kind, RuleKind::Spacing);
            assert_eq!(r.layer, LayerId::new(65, 20));
            assert_eq!(r.other_layer, Some(LayerId::new(66, 20)));
        }
        assert_eq!(rules[0].value, 200);
        assert_eq!(rules[1].value, 300);
    }

    #[test]
    fn enclosing_swaps_receiver_and_argument() {
        // `KLayout`: `outer.enclosing(inner, v)` -receiver is the enclosing layer.
        // Engine: `layer` is the enclosed (inner) shape, `other_layer` the outer.
        let deck = "\
            met1 = input(68, 20)\n\
            mcon = input(67, 44)\n\
            met1.enclosing(mcon, 0.03).output(\"m1.4\")\n";
        let r = one_rule(deck);
        assert_eq!(r.kind, RuleKind::Enclosure);
        assert_eq!(r.layer, LayerId::new(67, 44), "inner = the argument layer");
        assert_eq!(
            r.other_layer,
            Some(LayerId::new(68, 20)),
            "outer = the receiver layer"
        );
        assert_eq!(r.value, 30);
    }

    #[test]
    fn parses_min_area_from_with_area_below_threshold() {
        // `with_area(0.0, T)` selects polygons below T -> an Area rule of value T.
        let deck = "li = input(67, 20)\nli.with_area(0.0, 0.0561).output(\"li.6\")\n";
        let r = one_rule(deck);
        assert_eq!(r.kind, RuleKind::Area);
        assert_eq!(r.layer, LayerId::new(67, 20));
        assert_eq!(r.value, 56_100, "0.0561 um^2 -> 56100 dbu^2");
    }

    #[test]
    fn integer_argument_is_database_units() {
        let deck = "met1 = input(68, 20)\nmet1.width(140).output(\"m1.1\")\n";
        assert_eq!(one_rule(deck).value, 140);
    }

    #[test]
    fn explicit_unit_suffixes() {
        let um = "m = input(68, 20)\nm.width(0.14.um).output(\"w\")\n";
        assert_eq!(one_rule(um).value, 140);
        let dbu = "m = input(68, 20)\nm.width(140.dbu).output(\"w\")\n";
        assert_eq!(one_rule(dbu).value, 140);
    }

    #[test]
    fn extracts_the_script_from_a_lydrc_xml_wrapper() {
        // A real .lydrc macro: the script lives in <text>, XML-escaped.
        let deck = "\
<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<klayout-macro>\n\
 <category>drc</category>\n\
 <interpreter>dsl</interpreter>\n\
 <dsl-interpreter-name>drc-dsl-xml</dsl-interpreter-name>\n\
 <text>\n\
met1 = input(68, 20)\n\
met1.width(0.14).output(\"m1.1\", \"met1 &lt; min width\")\n\
 </text>\n\
</klayout-macro>\n";
        let r = one_rule(deck);
        assert_eq!(r.name, "m1.1");
        assert_eq!(r.kind, RuleKind::Width);
        assert_eq!(r.value, 140);
    }

    #[test]
    fn source_and_report_headers_are_ignored() {
        let deck = "\
            source($input)\n\
            report(\"my deck\", $report)\n\
            met1 = input(68, 20)\n\
            met1.width(0.14).output(\"m1.1\")\n";
        assert_eq!(parse_lydrc(deck).expect("valid").len(), 1);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let deck = "\
            # this is the met1 deck\n\
            \n\
            met1 = input(68, 20)   # met1 layer\n\
            met1.width(0.14).output(\"m1.1\")  # min width\n";
        assert_eq!(parse_lydrc(deck).expect("valid").len(), 1);
    }

    #[test]
    fn hash_inside_a_string_is_not_a_comment() {
        let deck = "m = input(68, 20)\nm.width(0.14).output(\"rule#1\")\n";
        assert_eq!(one_rule(deck).name, "rule#1");
    }

    #[test]
    fn undeclared_layer_is_a_clear_error() {
        let deck = "met1.width(0.14).output(\"m1.1\")\n";
        let err = parse_lydrc(deck).expect_err("met1 was never declared");
        match err {
            LydrcError::Syntax { line, message } => {
                assert_eq!(line, 1);
                assert!(message.contains("met1"), "names the layer: {message}");
            }
            LydrcError::Unsupported { construct, .. } => {
                panic!("expected a syntax error, got unsupported `{construct}`")
            }
        }
    }

    #[test]
    fn unsupported_construct_names_construct_and_line() {
        // Boolean layer algebra is valid `KLayout` but outside the subset.
        let deck = "\
            a = input(1, 0)\n\
            b = input(2, 0)\n\
            a.sized(0.1).output(\"grow\")\n";
        let err = parse_lydrc(deck).expect_err("sized is unsupported");
        match err {
            LydrcError::Unsupported { construct, line } => {
                assert_eq!(line, 3);
                assert!(construct.contains("sized"), "names it: {construct}");
            }
            LydrcError::Syntax { message, .. } => {
                panic!("expected unsupported, got syntax error `{message}`")
            }
        }
    }

    #[test]
    fn with_area_band_is_unsupported() {
        let deck = "l = input(67, 20)\nl.with_area(0.1, 0.2).output(\"band\")\n";
        let err = parse_lydrc(deck).expect_err("two-sided band is unsupported");
        assert!(matches!(err, LydrcError::Unsupported { line: 2, .. }));
    }

    #[test]
    fn malformed_number_is_a_syntax_error() {
        let deck = "m = input(68, 20)\nm.width(wide).output(\"w\")\n";
        let err = parse_lydrc(deck).expect_err("`wide` is not a number");
        assert!(matches!(err, LydrcError::Syntax { line: 2, .. }));
    }

    #[test]
    fn bad_input_layer_number_is_rejected() {
        let deck = "m = input(999999, 0)\n";
        assert!(matches!(
            parse_lydrc(deck),
            Err(LydrcError::Syntax { line: 1, .. })
        ));
    }

    #[test]
    fn arbitrary_untrusted_text_never_panics() {
        // A deterministic sweep of adversarial fragments: unbalanced parens, lone
        // punctuation, huge numbers, nested wrappers, control bytes. The contract
        // is "clear error or rules", never a panic or hang.
        let long_ident = "a".repeat(10_000);
        let fragments = [
            "",
            "(((((",
            ")))))",
            "=====",
            "a.b.c.d.e(",
            "input(",
            "met1.width(",
            ".....",
            "\"unterminated",
            "a = input(1,2,3,4,5)",
            "x.with_area()",
            "x.enclosing()",
            "<text><text><text>",
            "</text></text>",
            long_ident.as_str(),
            "999999999999999999999999.width(1)",
            "m = input(1,0)\nm.width(1.0e400).output(\"x\")",
            "\0\0\0\0",
            "m=input(1,0)\nm.space(-0.5).output(\"neg\")",
        ];
        for frag in fragments {
            // Must return without panicking; either outcome is acceptable.
            let _ = parse_lydrc(frag);
        }
    }
}

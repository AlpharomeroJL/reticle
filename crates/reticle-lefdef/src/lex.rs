//! A shared whitespace tokenizer for the LEF and DEF text grammars.
//!
//! LEF and DEF are both line-oriented, whitespace-separated token streams whose
//! statements are terminated by `;`. This lexer turns a source string into a flat
//! list of [`Token`]s, each tagged with its 1-based line, and hands the parsers a
//! forward cursor over them. `#` begins a comment that runs to the end of the line;
//! `(`, `)`, and `;` are always their own tokens even when written without
//! surrounding whitespace; a `"`-quoted run is one token with the quotes stripped.
//!
//! # Bounded work
//!
//! The token list is built in a single pass and never holds more than one entry per
//! input byte (every token is at least one byte and the specials are one byte
//! each), so tokenizing a slice capped at [`MAX_INPUT_BYTES`](crate::MAX_INPUT_BYTES)
//! allocates `O(input)` and terminates. The parsers advance the cursor by at least
//! one token per loop iteration, so no parse can hang on a finite stream. Neither
//! the lexer nor the parsers ever pre-allocate a collection from a count read out of
//! the input, so a hostile `COMPONENTS 999999999` line cannot force a large
//! allocation (the OASIS out-of-memory lesson, commit 1b1b56b).

/// One lexical token: a slice of the source plus the line it started on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Token<'a> {
    /// The token text, with any surrounding quotes already stripped.
    pub text: &'a str,
    /// The 1-based line the token started on.
    pub line: usize,
}

/// A forward cursor over the tokens of a LEF or DEF source.
#[derive(Debug)]
pub(crate) struct Lexer<'a> {
    tokens: Vec<Token<'a>>,
    pos: usize,
    /// The last line seen, so an error at end-of-input can still name a line.
    last_line: usize,
}

impl<'a> Lexer<'a> {
    /// Tokenizes `source`. Comments (`#` to end of line) are dropped; `(`, `)`, and
    /// `;` become standalone tokens; `"..."` becomes one token without the quotes.
    pub(crate) fn new(source: &'a str) -> Self {
        let mut tokens = Vec::new();
        let bytes = source.as_bytes();
        let mut i = 0;
        let mut line = 1;
        let mut last_line = 1;
        while i < bytes.len() {
            let b = bytes[i];
            match b {
                b'\n' => {
                    line += 1;
                    i += 1;
                }
                b if b.is_ascii_whitespace() => {
                    i += 1;
                }
                b'#' => {
                    // Comment to end of line.
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
                b'(' | b')' | b';' => {
                    // A single-character structural token. Slicing on an ASCII byte
                    // boundary is always valid UTF-8.
                    tokens.push(Token {
                        text: &source[i..=i],
                        line,
                    });
                    last_line = line;
                    i += 1;
                }
                b'"' => {
                    // A quoted run: consume to the closing quote (or end of input),
                    // emitting the interior without the quotes.
                    let start = i + 1;
                    let mut j = start;
                    while j < bytes.len() && bytes[j] != b'"' {
                        if bytes[j] == b'\n' {
                            line += 1;
                        }
                        j += 1;
                    }
                    tokens.push(Token {
                        text: &source[start..j.min(bytes.len())],
                        line,
                    });
                    last_line = line;
                    // Skip the closing quote if present.
                    i = if j < bytes.len() { j + 1 } else { j };
                }
                _ => {
                    // A bare word: run of characters up to whitespace or a special.
                    let start = i;
                    while i < bytes.len() {
                        let c = bytes[i];
                        if c.is_ascii_whitespace() || matches!(c, b'(' | b')' | b';' | b'#' | b'"')
                        {
                            break;
                        }
                        i += 1;
                    }
                    tokens.push(Token {
                        text: &source[start..i],
                        line,
                    });
                    last_line = line;
                }
            }
        }
        Self {
            tokens,
            pos: 0,
            last_line,
        }
    }

    /// The next token without advancing.
    pub(crate) fn peek(&self) -> Option<Token<'a>> {
        self.tokens.get(self.pos).copied()
    }

    /// Advances past and returns the next token.
    pub(crate) fn bump(&mut self) -> Option<Token<'a>> {
        let t = self.tokens.get(self.pos).copied();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    /// The line to blame for an error at the current position: the next token's line,
    /// or the last line seen at end-of-input.
    pub(crate) fn line(&self) -> usize {
        self.peek().map_or(self.last_line, |t| t.line)
    }
}

/// Parses a LEF/DEF numeric literal as `f64`. Both formats write dimensions as
/// decimal microns (LEF) or integer DBU (DEF); one `f64` parse covers both. The
/// caller decides how to round to DBU.
pub(crate) fn parse_number(text: &str) -> Option<f64> {
    let v: f64 = text.parse().ok()?;
    if v.is_finite() { Some(v) } else { None }
}

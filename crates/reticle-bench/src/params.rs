//! Parsing per-task checker parameters out of the frozen [`BenchTask::checker`]
//! string.
//!
//! The task schema is frozen: a task names its checker with a single `checker`
//! string and carries no free-form parameter map. To keep geometric checkers
//! parameterized (which layer, how many shapes, how much area) without changing the
//! schema, parameters are encoded *in* that string as
//! `name:key=value,key2=value2`. [`ParsedChecker`] splits a checker string into its
//! bare `name` and a small key/value map, and offers typed accessors that report a
//! precise error when a value is missing or malformed.
//!
//! The grammar is deliberately tiny and total:
//!
//! - Everything before the first `:` is the checker name (`shape_count`).
//! - The remainder is a comma-separated list of `key=value` pairs.
//! - Whitespace around names, keys, and values is trimmed.
//! - A layer value is written `layer/datatype` (for example `68/20`).
//!
//! [`CheckerRegistry::for_task`](crate::CheckerRegistry::for_task) binds the compiled
//! checker under the *whole* original string, so the runner's
//! `registry.get(&task.checker)` still resolves it verbatim.

use std::collections::BTreeMap;

use reticle_geometry::LayerId;

/// A checker string split into its base name and parameter map.
///
/// Built with [`ParsedChecker::parse`]. The typed accessors
/// ([`layer`](Self::layer), [`u32`](Self::u32), [`i64`](Self::i64)) return a
/// [`ParamError`] naming the offending key so a malformed task fails to build with a
/// clear message rather than mis-checking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedChecker {
    /// The bare checker name (the text before the first `:`).
    name: String,
    /// The parsed `key=value` parameters, in sorted order for deterministic errors.
    params: BTreeMap<String, String>,
}

/// A failure reading a typed parameter from a checker string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamError {
    /// A required key was absent.
    Missing {
        /// The key that was expected.
        key: String,
    },
    /// A key was present but its value did not parse as the expected type.
    Invalid {
        /// The offending key.
        key: String,
        /// The value that failed to parse.
        value: String,
        /// What was expected (for example `"u32"` or `"layer/datatype"`).
        expected: &'static str,
    },
}

impl std::fmt::Display for ParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParamError::Missing { key } => write!(f, "missing checker parameter `{key}`"),
            ParamError::Invalid {
                key,
                value,
                expected,
            } => write!(
                f,
                "checker parameter `{key}` value `{value}` is not a valid {expected}"
            ),
        }
    }
}

impl std::error::Error for ParamError {}

impl ParsedChecker {
    /// Splits `spec` into a checker name and its parameters.
    ///
    /// `spec` is a [`BenchTask::checker`](crate::BenchTask::checker) string such as
    /// `shape_count:layer=68/20,min=3`. A string with no `:` parses to that bare name
    /// and an empty parameter map, so plain names like `drc_clean` still parse.
    ///
    /// Malformed pairs (an empty key, or a fragment with no `=`) are skipped rather
    /// than erroring here; a required-but-absent key surfaces later through the typed
    /// accessors, which is where the checker knows what it needs.
    #[must_use]
    pub fn parse(spec: &str) -> Self {
        let (name, rest) = match spec.split_once(':') {
            Some((name, rest)) => (name.trim(), rest),
            None => (spec.trim(), ""),
        };
        let mut params = BTreeMap::new();
        for pair in rest.split(',') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            if let Some((key, value)) = pair.split_once('=') {
                let key = key.trim();
                if !key.is_empty() {
                    params.insert(key.to_owned(), value.trim().to_owned());
                }
            }
        }
        Self {
            name: name.to_owned(),
            params,
        }
    }

    /// The bare checker name (the text before the first `:`).
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns `true` if `key` was supplied.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.params.contains_key(key)
    }

    /// The raw string value of `key`, if present.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(String::as_str)
    }

    /// The value of `key` parsed as a [`LayerId`] written `layer/datatype`.
    ///
    /// # Errors
    ///
    /// [`ParamError::Missing`] if `key` is absent, or [`ParamError::Invalid`] if the
    /// value is not two `/`-separated `u16`s.
    pub fn layer(&self, key: &str) -> Result<LayerId, ParamError> {
        let raw = self.require(key)?;
        parse_layer(raw).ok_or_else(|| ParamError::Invalid {
            key: key.to_owned(),
            value: raw.to_owned(),
            expected: "layer/datatype",
        })
    }

    /// The value of `key` parsed as a [`LayerId`], or `default` if the key is absent.
    ///
    /// # Errors
    ///
    /// [`ParamError::Invalid`] if the key is present but malformed.
    pub fn layer_or(&self, key: &str, default: LayerId) -> Result<LayerId, ParamError> {
        match self.get(key) {
            None => Ok(default),
            Some(raw) => parse_layer(raw).ok_or_else(|| ParamError::Invalid {
                key: key.to_owned(),
                value: raw.to_owned(),
                expected: "layer/datatype",
            }),
        }
    }

    /// The value of `key` parsed as a [`u32`].
    ///
    /// # Errors
    ///
    /// [`ParamError::Missing`] if absent, [`ParamError::Invalid`] if not a `u32`.
    pub fn u32(&self, key: &str) -> Result<u32, ParamError> {
        let raw = self.require(key)?;
        raw.parse::<u32>().map_err(|_| ParamError::Invalid {
            key: key.to_owned(),
            value: raw.to_owned(),
            expected: "u32",
        })
    }

    /// The value of `key` parsed as a [`u32`], or `default` if the key is absent.
    ///
    /// # Errors
    ///
    /// [`ParamError::Invalid`] if the key is present but not a `u32`.
    pub fn u32_or(&self, key: &str, default: u32) -> Result<u32, ParamError> {
        match self.get(key) {
            None => Ok(default),
            Some(raw) => raw.parse::<u32>().map_err(|_| ParamError::Invalid {
                key: key.to_owned(),
                value: raw.to_owned(),
                expected: "u32",
            }),
        }
    }

    /// The value of `key` parsed as an [`i64`] (used for areas and coordinates).
    ///
    /// # Errors
    ///
    /// [`ParamError::Missing`] if absent, [`ParamError::Invalid`] if not an `i64`.
    pub fn i64(&self, key: &str) -> Result<i64, ParamError> {
        let raw = self.require(key)?;
        raw.parse::<i64>().map_err(|_| ParamError::Invalid {
            key: key.to_owned(),
            value: raw.to_owned(),
            expected: "i64",
        })
    }

    /// The value of `key` parsed as an [`i64`], or `default` if the key is absent.
    ///
    /// # Errors
    ///
    /// [`ParamError::Invalid`] if the key is present but not an `i64`.
    pub fn i64_or(&self, key: &str, default: i64) -> Result<i64, ParamError> {
        match self.get(key) {
            None => Ok(default),
            Some(raw) => raw.parse::<i64>().map_err(|_| ParamError::Invalid {
                key: key.to_owned(),
                value: raw.to_owned(),
                expected: "i64",
            }),
        }
    }

    /// Looks up `key`, mapping absence to [`ParamError::Missing`].
    fn require(&self, key: &str) -> Result<&str, ParamError> {
        self.get(key).ok_or_else(|| ParamError::Missing {
            key: key.to_owned(),
        })
    }
}

/// Parses `layer/datatype` (for example `68/20`) into a [`LayerId`].
fn parse_layer(raw: &str) -> Option<LayerId> {
    let (layer, datatype) = raw.split_once('/')?;
    let layer = layer.trim().parse::<u16>().ok()?;
    let datatype = datatype.trim().parse::<u16>().ok()?;
    Some(LayerId::new(layer, datatype))
}

#[cfg(test)]
mod tests {
    use super::{ParamError, ParsedChecker};
    use reticle_geometry::LayerId;

    #[test]
    fn parses_name_and_params() {
        let p = ParsedChecker::parse("shape_count:layer=68/20,min=3,max=5");
        assert_eq!(p.name(), "shape_count");
        assert_eq!(p.layer("layer").unwrap(), LayerId::new(68, 20));
        assert_eq!(p.u32("min").unwrap(), 3);
        assert_eq!(p.u32("max").unwrap(), 5);
    }

    #[test]
    fn bare_name_has_no_params() {
        let p = ParsedChecker::parse("drc_clean");
        assert_eq!(p.name(), "drc_clean");
        assert!(!p.has("layer"));
        assert!(matches!(p.u32("min"), Err(ParamError::Missing { .. })));
    }

    #[test]
    fn trims_whitespace_around_pairs() {
        let p = ParsedChecker::parse("layer_area : layer = 67/20 , min_area = 56100 ");
        assert_eq!(p.name(), "layer_area");
        assert_eq!(p.layer("layer").unwrap(), LayerId::new(67, 20));
        assert_eq!(p.i64("min_area").unwrap(), 56100);
    }

    #[test]
    fn defaults_apply_when_absent() {
        let p = ParsedChecker::parse("via_chain:vias=4");
        assert_eq!(p.u32("vias").unwrap(), 4);
        assert_eq!(p.u32_or("missing", 7).unwrap(), 7);
        assert_eq!(
            p.layer_or("via", LayerId::new(68, 44)).unwrap(),
            LayerId::new(68, 44)
        );
    }

    #[test]
    fn malformed_layer_is_invalid() {
        let p = ParsedChecker::parse("shape_count:layer=68");
        assert!(matches!(p.layer("layer"), Err(ParamError::Invalid { .. })));
        let p = ParsedChecker::parse("shape_count:layer=x/y");
        assert!(matches!(p.layer("layer"), Err(ParamError::Invalid { .. })));
    }

    #[test]
    fn malformed_number_is_invalid() {
        let p = ParsedChecker::parse("shape_count:min=lots");
        assert!(matches!(p.u32("min"), Err(ParamError::Invalid { .. })));
    }

    #[test]
    fn skips_fragments_without_equals() {
        // A stray fragment with no `=` is ignored, not an error.
        let p = ParsedChecker::parse("shape_count:layer=68/20,,garbage,min=2");
        assert_eq!(p.layer("layer").unwrap(), LayerId::new(68, 20));
        assert_eq!(p.u32("min").unwrap(), 2);
        assert!(!p.has("garbage"));
    }
}

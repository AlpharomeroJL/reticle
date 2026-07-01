//! Import and export for Reticle.
//!
//! Supports GDSII (via `gds21`, Wave 1), an in-house OASIS subset (Wave 1,
//! ADR 0004), a technology-file parser, and VLSIR interop. Each format implements
//! the `reticle-model` [`Importer`]/[`Exporter`] traits so the CLI and app treat
//! them uniformly. Parsers are fuzzed (see `fuzz/`).

use reticle_model::{Document, Exporter, Importer, Result, Technology};

/// GDSII import/export (Wave 1: `gds21`).
#[derive(Debug, Default)]
pub struct Gds;

impl Importer for Gds {
    fn import(&self, bytes: &[u8]) -> Result<Document> {
        let _ = bytes;
        todo!("Wave 1: GDSII import via gds21 (ADR 0004)")
    }
}

impl Exporter for Gds {
    fn export(&self, doc: &Document) -> Result<Vec<u8>> {
        let _ = doc;
        todo!("Wave 1: GDSII export via gds21 (ADR 0004)")
    }
}

/// OASIS import/export (Wave 1: in-house subset, ADR 0004).
#[derive(Debug, Default)]
pub struct Oasis;

impl Importer for Oasis {
    fn import(&self, bytes: &[u8]) -> Result<Document> {
        let _ = bytes;
        todo!("Wave 1: OASIS import (in-house subset, ADR 0004)")
    }
}

impl Exporter for Oasis {
    fn export(&self, doc: &Document) -> Result<Vec<u8>> {
        let _ = doc;
        todo!("Wave 1: OASIS export (in-house subset, ADR 0004)")
    }
}

/// Parses a Reticle technology file (layers, colors, database unit, rules).
///
/// # Errors
///
/// Returns a [`reticle_model::ModelError`] on malformed input. Wave 1 implements
/// the parser; the signature is the frozen contract.
pub fn parse_technology(source: &str) -> Result<Technology> {
    let _ = source;
    todo!("Wave 1: technology-file parser")
}

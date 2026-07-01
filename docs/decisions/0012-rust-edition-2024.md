# 0012, Rust edition 2024

## Context

The toolchain is Rust 1.94 (2026). The workspace can target edition 2021 or 2024.
Edition 2024 is stable since Rust 1.85 and brings tightened defaults (e.g. `unsafe`
attribute rules, RPIT lifetime capture, `gen` reservations) that suit a codebase
meant to read as current and disciplined.

## Decision

Use `edition = "2024"` workspace-wide, with `rust-version = "1.94"`. Editions are
per-crate at the language level, so dependencies on older editions are unaffected.

## Consequences

The code uses modern idioms and stricter safety defaults, reinforcing the
"prefer safe Rust, isolate unsafe" rule. Contributors need Rust ≥ 1.85 to build;
the pinned `rust-toolchain.toml` guarantees this locally. Any dependency that has
not migrated to edition 2024 still compiles under its own edition, so there is no
ecosystem cost.

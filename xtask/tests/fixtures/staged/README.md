# verify-licenses fixtures

Synthetic staged-content directory for the `xtask verify-licenses` gate (the tests
in `xtask/src/verify_licenses.rs`). Every `*.rtla` here is an opaque placeholder: the
gate never parses an archive, it only reads the sibling `<archive>.rtla.NOTICE`
manifest. None of these are real archives or real third-party content.

Each fixture pins one verdict, so a single run over this directory is the two-way
gate (some pass, some are excluded, each for a distinct reason):

| archive | manifest | expected verdict |
| --- | --- | --- |
| `good.rtla` | `Apache-2.0` | VERIFIED |
| `mit.rtla` | `MIT` | VERIFIED |
| `cern.rtla` | `CERN-OHL-S-2.0` | VERIFIED |
| `cc0.rtla` | `CC0-1.0` | VERIFIED |
| `ccby.rtla` | `CC-BY-4.0` | VERIFIED |
| `unknown.rtla` | `NoSuchLicense-9.9` | EXCLUDED (not on allowlist) |
| `forbidden.rtla` | `LicenseRef-Proprietary` | EXCLUDED (not on allowlist) |
| `nospdx.rtla` | source only, no SPDX line | EXCLUDED (no SPDX identifier) |
| `nomanifest.rtla` | (no sibling manifest) | EXCLUDED (no manifest) |

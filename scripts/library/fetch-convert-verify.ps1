# The rerunnable open-silicon library pipeline: fetch -> convert (GDS -> .rtla) ->
# license-verify -> F1 gallery manifest.
#
# Reads the die table (default `scripts/library/dies.json`): one entry per die, each
# naming a source GDS (a path already committed in this repo, or `null` for a die this
# pipeline synthesizes itself with `xtask gen-layout`), its provenance (repo, commit,
# url), and a curated landmark. For each die it:
#
#   1. Resolves the source GDS (or generates a tiny synthetic one).
#   2. Converts it to a `.rtla` archive via the existing `reticle-cli convert` command
#      (this script never touches archive internals; it only calls the CLI arm).
#   3. Writes the sibling `<id>.rtla.NOTICE` (`Source:` + `SPDX-License-Identifier:`,
#      the provenance style of `corpus/tinytapeout/NOTICE.md`) that the license gate
#      reads. A die with no `spdx` in the table gets a NOTICE with no SPDX line at
#      all, which the gate excludes -- this is how the deliberately-unverified demo
#      entry proves the fail-closed path, not a claim about a real license.
#
# Then it runs `xtask verify-licenses` over the output directory (informational: this
# sample intentionally includes one excluded entry, so a non-zero exit here is
# expected and reported, not treated as a script failure) and finally
# `xtask library-manifest`, which re-derives the same verdicts, reads each die's real
# geometry back from its own archive, and writes the F1 `GalleryManifest` JSON. That
# step's exit code DOES gate the script: an invalid or unwritable manifest is fatal.
#
# Rerunnable: every step is a pure function of the die table and the source GDS files,
# so running this again reproduces the same archives, NOTICEs, and manifest (modulo
# the manifest's `fetched_utc`, which reflects the run time by design).
#
# This proves the pipeline machinery on a tiny, already-committed sample. The
# multi-gigabyte bulk shuttle download that populates a real gallery is a separate,
# later step (the orchestrator's valley queue), not this script; see
# `scripts/library/README.md`.
#
# Usage (run from the repo root; set CARGO_TARGET_DIR first as your environment
# requires -- this script does not set it):
#   powershell -File scripts/library/fetch-convert-verify.ps1 `
#       [-DiesMeta <path>] [-LibraryDir <dir>] [-ScratchDir <dir>]

param(
    [string]$DiesMeta = 'scripts/library/dies.json',
    [string]$LibraryDir = 'library',
    [string]$ScratchDir = 'scratch/library-pipeline'
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $DiesMeta)) {
    Write-Output "FAIL  die table not found: $DiesMeta"
    exit 1
}
$dies = Get-Content $DiesMeta -Raw | ConvertFrom-Json
New-Item -ItemType Directory -Force $LibraryDir | Out-Null
New-Item -ItemType Directory -Force $ScratchDir | Out-Null

$failed = $false

foreach ($die in $dies) {
    $id = $die.id
    Write-Output "--- $id ---"

    # Step 1: resolve the source GDS (a committed sample, or a tiny synthetic one).
    if ($die.source_gds) {
        $gds = $die.source_gds
        if (-not (Test-Path $gds)) {
            Write-Output "FAIL  $id (source_gds not found: $gds)"
            $failed = $true
            continue
        }
        Write-Output "OK    $id source: $gds"
    } else {
        $gds = Join-Path $ScratchDir "$id.gds"
        cargo run -p xtask --release -- gen-layout --shapes 16 --layers 1 --depth 1 --out $gds
        if ($LASTEXITCODE -ne 0) {
            Write-Output "FAIL  $id (gen-layout exit $LASTEXITCODE)"
            $failed = $true
            continue
        }
        Write-Output "OK    $id source: synthesized $gds (xtask gen-layout)"
    }

    # Step 2: convert GDS -> .rtla via the existing `reticle-cli convert` arm.
    $rtla = Join-Path $LibraryDir "$id.rtla"
    cargo run -p reticle-cli --release -- convert $gds $rtla
    if ($LASTEXITCODE -ne 0) {
        Write-Output "FAIL  $id (convert exit $LASTEXITCODE)"
        $failed = $true
        continue
    }
    Write-Output "OK    $id converted: $rtla"

    # Step 3: write the sibling NOTICE the license gate reads. No `spdx` in the table
    # means no SPDX line at all -- an intentionally incomplete NOTICE, not a lie about
    # a real license.
    $notice = New-Object System.Collections.Generic.List[string]
    $notice.Add("Source: $($die.url)")
    if ($die.spdx) {
        $notice.Add("SPDX-License-Identifier: $($die.spdx)")
    }
    [System.IO.File]::WriteAllLines("$rtla.NOTICE", $notice)
    Write-Output "OK    $id NOTICE written: $rtla.NOTICE"
}

if ($failed) {
    Write-Output ''
    Write-Output 'fetch-convert-verify: one or more dies failed to fetch/convert; stopping'
    exit 1
}

# Step 4: the license gate, informational. This sample deliberately includes one
# excluded entry (to prove the fail-closed path end to end), so a non-zero exit here
# is expected, not a script failure; a real staging run would act on the printed
# STATUS lines before shipping anything.
Write-Output ''
Write-Output '--- verify-licenses (informational; an excluded entry is expected here) ---'
cargo run -p xtask --release -- verify-licenses $LibraryDir
Write-Output "verify-licenses exit code: $LASTEXITCODE"

# Step 5: the F1 manifest generator. This DOES gate the script: an invalid or
# unwritable manifest is fatal.
Write-Output ''
Write-Output '--- library-manifest ---'
$manifestOut = Join-Path $LibraryDir 'gallery-manifest.json'
cargo run -p xtask --release -- library-manifest $LibraryDir $DiesMeta $manifestOut
if ($LASTEXITCODE -ne 0) {
    Write-Output "FAIL  library-manifest (exit $LASTEXITCODE)"
    exit 1
}

Write-Output ''
Write-Output "fetch-convert-verify: wrote $manifestOut"

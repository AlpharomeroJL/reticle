# Verdict-comparison harness for the .lydrc compatibility subset.
#
# Runs the committed subset.lydrc deck over the committed subset.gds layout with
# KLayout headless inside the PINNED hpretl/iic-osic-tools container (the same
# image and `--skip bash -lc` entrypoint pattern as scripts/tt-precheck.ps1), then
# compares KLayout's per-rule verdicts against the expected verdicts that
# tests/lydrc_engine.rs pins for reticle-drc.
#
# The comparison is at the LAYOUT-LEVEL verdict granularity: for each rule, did the
# tool report at least one violation (`fired`) or none. Raw marker counts are
# printed for the record but are NOT required to match, because KLayout emits one
# edge-pair marker per offending edge while DrcEngine emits one violation per
# offending shape or pair (see docs/src/lydrc-compat.md, "Divergence note"). A
# supported-subset rule whose `fired` verdict disagrees is a real divergence and
# fails this harness.
#
# Usage:
#   powershell -File scripts/lydrc-compare.ps1
#   powershell -File scripts/lydrc-compare.ps1 -ImageTag 2025.01 -OutDir scratch/lydrc
#
# Exit codes: 0 = all supported-subset verdicts agree; 1 = a verdict diverged;
# 2 = setup failure (docker missing, fixtures absent, KLayout error).

param(
    # The fixture layout KLayout reads.
    [string]$Gds = 'crates/reticle-drc/tests/fixtures/subset.gds',
    # The .lydrc deck (supported subset) both tools run.
    [string]$Deck = 'crates/reticle-drc/tests/fixtures/subset.lydrc',
    # The expected per-rule verdicts (reticle-drc side, pinned by lydrc_engine.rs).
    [string]$Expected = 'crates/reticle-drc/tests/fixtures/expected-verdicts.json',
    # Where the KLayout report database and this run's summary are written.
    [string]$OutDir = 'scratch/lydrc',
    # The PINNED image tag (dated, never `latest`), matching scripts/tt-precheck.ps1.
    [string]$ImageTag = '2025.01'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$image = "hpretl/iic-osic-tools:$ImageTag"

function Fail($message) {
    Write-Output "lydrc-compare: $message"
    exit 2
}

# --- Validate inputs -------------------------------------------------------------------
foreach ($f in @($Gds, $Deck, $Expected)) {
    if (-not (Test-Path -LiteralPath $f)) { Fail "missing fixture: $f" }
}
$dockerVersion = (& docker --version) 2>$null
if ($LASTEXITCODE -ne 0) {
    Fail 'docker not found on PATH. Install Docker, or run the exact command in the divergence note under WSL.'
}
Write-Output "lydrc-compare: using $dockerVersion"
Write-Output "lydrc-compare: pinned image $image"

New-Item -ItemType Directory -Force $OutDir | Out-Null
$root = (Get-Location).Path
$reportRel = (Join-Path $OutDir 'out.lyrdb') -replace '\\', '/'
$gdsRel = $Gds -replace '\\', '/'
$deckRel = $Deck -replace '\\', '/'

# --- Run KLayout DRC in the container --------------------------------------------------
# Mirrors the tt-precheck invocation: mount the repo at /work, `--skip` bypasses the
# image UI launcher and execs the assigned command. $input/$report reach the deck via
# klayout -rd (the deck's source()/report() header reads them).
$containerCmd = "klayout -b -r '/work/$deckRel' -rd input='/work/$gdsRel' -rd report='/work/$reportRel'"
$dockerArgs = @(
    'run', '--rm',
    '-v', "${root}:/work",
    $image,
    '--skip', 'bash', '-lc', $containerCmd
)
Write-Output "lydrc-compare: docker run --rm -v ${root}:/work $image --skip bash -lc `"$containerCmd`""

$sw = [System.Diagnostics.Stopwatch]::StartNew()
& docker @dockerArgs | Out-Null
$klExit = $LASTEXITCODE
$sw.Stop()
$wallS = [math]::Round($sw.Elapsed.TotalSeconds, 1)
if ($klExit -ne 0) { Fail "KLayout DRC exited $klExit" }

$reportPath = Join-Path $OutDir 'out.lyrdb'
if (-not (Test-Path -LiteralPath $reportPath)) { Fail "KLayout wrote no report at $reportPath" }

# --- Parse the KLayout report database (per-category item counts) -----------------------
[xml]$db = Get-Content -Raw -LiteralPath $reportPath
$klCounts = @{}
# Declared categories start at zero so a rule that fires nothing is a real 0, not absent.
foreach ($c in $db.'report-database'.categories.category) {
    if ($c.name) { $klCounts[[string]$c.name] = 0 }
}
foreach ($item in $db.'report-database'.items.item) {
    if ($null -eq $item) { continue }
    $cat = ([string]$item.category).Trim().Trim("'")
    if ($klCounts.ContainsKey($cat)) { $klCounts[$cat]++ } else { $klCounts[$cat] = 1 }
}

# --- Compare against the expected (reticle-drc) verdicts --------------------------------
# NB: use a distinctly named variable, not $expected: PowerShell variable names are
# case-insensitive, so $expected would alias the $Expected path parameter.
$verdicts = Get-Content -Raw -LiteralPath $Expected | ConvertFrom-Json
$expectedRules = @($verdicts.rules)
if ($expectedRules.Count -eq 0) { Fail "no expected rules parsed from $Expected" }
$diverged = 0
$rows = @()
foreach ($rule in $expectedRules) {
    $name = [string]$rule.name
    $klCount = if ($klCounts.ContainsKey($name)) { $klCounts[$name] } else { 0 }
    $klFired = $klCount -gt 0
    $expFired = [bool]$rule.fired
    $agree = ($klFired -eq $expFired)
    if (-not $agree) { $diverged++ }
    $rows += [pscustomobject]@{
        Rule          = $name
        Kind          = [string]$rule.kind
        ReticleCount  = [int]$rule.reticle_count
        KLayoutCount  = $klCount
        ReticleFired  = $expFired
        KLayoutFired  = $klFired
        VerdictAgrees = $agree
    }
}

Write-Output ''
Write-Output 'lydrc-compare: per-rule verdicts (fired = at least one violation reported)'
$rows | Format-Table -AutoSize | Out-String | ForEach-Object { Write-Output $_ }

# Persist a machine-readable summary next to the report.
$summary = [pscustomobject]@{
    image        = $image
    deck         = $deckRel
    gds          = $gdsRel
    wall_s       = $wallS
    klayout_exit = $klExit
    diverged     = $diverged
    rules        = $rows
}
$summaryPath = Join-Path $OutDir 'compare-summary.json'
$summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $summaryPath -Encoding utf8

Write-Output "MEASURE|lydrc-compare|image=$image|wall_s=$wallS|klayout_exit=$klExit|diverged=$diverged|summary=$summaryPath"
if ($diverged -gt 0) {
    Write-Output "lydrc-compare: FAILED, $diverged supported-subset rule verdict(s) diverged"
    exit 1
}
Write-Output 'lydrc-compare: OK, all supported-subset rule verdicts agree between reticle-drc and KLayout'
exit 0

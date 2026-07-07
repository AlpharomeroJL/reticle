# Run TinyTapeout's own precheck over a GDS as an external oracle.
#
# TinyTapeout's precheck (the `precheck` module of `TinyTapeout/tt-support-tools`) is
# the authoritative gate a GDS-mode submission must clear: Magic DRC, a set of KLayout
# checks, and structural checks (pins against the analog template, the tile boundary, the
# layer whitelist, forbidden layers, and the top-cell name). It is Linux-native and needs
# Magic, KLayout, gdstk, and the SKY130 PDK, so this script runs it inside a PINNED Docker
# container. The image `hpretl/iic-osic-tools` bundles all four (its PDK lives at
# /foss/pdks, so PDK_ROOT is set for us); a dated tag is pinned below, never `latest`.
#
# What it does, end to end:
#   1. Verifies the GDS exists and begins with a GDSII HEADER record.
#   2. Stages a minimal TinyTapeout project directory (the GDS under gds/<stem>.gds plus a
#      minimal info.yaml whose top_module equals the GDS stem, which the precheck requires
#      and asserts). A -ProjectDir can supply a real project instead.
#   3. Checks out (or reuses) TinyTapeout/tt-support-tools at a pinned ref.
#   4. Runs, inside the container: `python precheck/precheck.py --gds <gds> --tech sky130A`
#      from the tt-support-tools checkout, with the staged project mounted so info.yaml is
#      found by the upward search.
#   5. Copies the precheck's reports directory (results.md, results.xml, magic_drc.txt,
#      drc_*.xml) back to -OutDir and prints a MEASURE line with the wall time and the
#      container exit code (0 = precheck passed, non-zero = failed).
#
# The Rust parser `reticle_cli::tt_precheck::parse_reports_dir` then turns -OutDir into a
# structured PrecheckReport the agent loop consumes like DRC violations. See ADR 0054 and
# docs/src/tapeout.md.
#
# This is additive tooling (the `just tt-precheck` recipe), NOT part of `just ci`: it needs
# Docker and a multi-GB image, exactly like the nightly-only fuzz/miri recipes.
#
# Usage:
#   powershell -File scripts/tt-precheck.ps1 -Gds scratch/tile.gds
#   powershell -File scripts/tt-precheck.ps1 -Gds scratch/tile.gds -OutDir scratch/precheck `
#       -ImageTag 2025.01 -SupportRef main
#   powershell -File scripts/tt-precheck.ps1 -Gds project/gds/tt_um_x.gds -ProjectDir project
#
# WSL fallback (documented, not the default): with tt-support-tools, magic, klayout, gdstk,
# and the SKY130 PDK installed inside a WSL distro, the same precheck command runs there:
#   wsl -d Ubuntu -- bash -lc 'cd tt-support-tools && \
#       PDK_ROOT=$PDK_ROOT python precheck/precheck.py --gds <gds> --tech sky130A'
# See ADR 0054 for the exact fallback steps.

param(
    # The GDS (or OAS) layout file to precheck.
    [Parameter(Mandatory = $true)][string]$Gds,
    # Where to copy the precheck reports directory (results.md, magic_drc.txt, ...).
    [string]$OutDir = 'scratch/precheck-reports',
    # The PINNED iic-osic-tools image tag. A dated tag (YYYY.MM), never `latest`.
    [string]$ImageTag = '2025.01',
    # The tt-support-tools git ref to check out (a tag or commit pins the precheck itself).
    [string]$SupportRef = 'main',
    # An existing checkout of TinyTapeout/tt-support-tools to reuse instead of cloning.
    [string]$SupportDir = '',
    # A real TinyTapeout project directory (with info.yaml + gds/) to mount instead of the
    # synthesized minimal project. When set, -Gds must live under it.
    [string]$ProjectDir = '',
    # The tile footprint the precheck picks its DEF template from (1x2, 2x2, ...). The staged
    # minimal info.yaml records it as `project.tiles`, which drives `tt_analog_<Tiles>.def`.
    # A real -ProjectDir supplies its own info.yaml and ignores this.
    [string]$Tiles = '1x2',
    # The container technology name passed to the precheck.
    [string]$Tech = 'sky130A'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# The pinned image reference. Docker Hub mirror; ghcr.io/iic-jku publishes the same tags.
$image = "hpretl/iic-osic-tools:$ImageTag"

function Fail($message) {
    Write-Output "tt-precheck: $message"
    exit 2
}

# --- 1. Validate the GDS ---------------------------------------------------------------
if (-not (Test-Path -LiteralPath $Gds)) { Fail "GDS not found: $Gds" }
$gdsItem = Get-Item -LiteralPath $Gds
$gdsStem = [System.IO.Path]::GetFileNameWithoutExtension($gdsItem.Name)
$bytes = [System.IO.File]::ReadAllBytes($gdsItem.FullName)
$isGds = ($bytes.Length -ge 6 -and $bytes[0] -eq 0 -and $bytes[1] -eq 6 -and
    $bytes[2] -eq 0 -and $bytes[3] -eq 2)
if (-not $isGds) {
    # OAS is also accepted by the precheck; only warn, do not block.
    Write-Output "tt-precheck: note: $Gds does not begin with a GDSII HEADER record (OK for OAS)."
}

# --- 2. Ensure Docker is present -------------------------------------------------------
$dockerVersion = (& docker --version) 2>$null
if ($LASTEXITCODE -ne 0) {
    Fail "docker not found on PATH. Install Docker, or use the WSL fallback in the header."
}
Write-Output "tt-precheck: using $dockerVersion"
Write-Output "tt-precheck: pinned image $image"

# --- 3. Check out tt-support-tools -----------------------------------------------------
if ($SupportDir -and (Test-Path -LiteralPath $SupportDir)) {
    $supportPath = (Get-Item -LiteralPath $SupportDir).FullName
    Write-Output "tt-precheck: reusing tt-support-tools at $supportPath"
} else {
    $supportPath = Join-Path (Get-Location) 'scratch/tt-support-tools'
    if (-not (Test-Path -LiteralPath $supportPath)) {
        New-Item -ItemType Directory -Force (Split-Path $supportPath) | Out-Null
        Write-Output "tt-precheck: cloning TinyTapeout/tt-support-tools@$SupportRef"
        & git clone --depth 1 --branch $SupportRef `
            https://github.com/TinyTapeout/tt-support-tools $supportPath
        if ($LASTEXITCODE -ne 0) {
            # --branch fails on a commit SHA; fall back to a full clone + checkout.
            & git clone https://github.com/TinyTapeout/tt-support-tools $supportPath
            if ($LASTEXITCODE -ne 0) { Fail 'could not clone tt-support-tools' }
            & git -C $supportPath checkout $SupportRef
        }
    } else {
        Write-Output "tt-precheck: reusing existing checkout at $supportPath"
    }
}

# --- 4. Stage the project (info.yaml + gds/<stem>.gds) ---------------------------------
# The precheck reads top_module from info.yaml (searched upward from the GDS directory)
# and asserts it equals the GDS filename stem. Stage a minimal project unless a real one
# was supplied.
if ($ProjectDir -and (Test-Path -LiteralPath $ProjectDir)) {
    $projectPath = (Get-Item -LiteralPath $ProjectDir).FullName
    # The GDS must already sit under the project (its gds/ directory).
    $gdsInContainer = $gdsItem.FullName.Substring($projectPath.Length).TrimStart('\', '/') -replace '\\', '/'
    $gdsMountArg = "/work/$gdsInContainer"
    Write-Output "tt-precheck: mounting real project $projectPath"
} else {
    $projectPath = Join-Path (Get-Location) "scratch/tt-precheck-project/$gdsStem"
    if (Test-Path -LiteralPath $projectPath) { Remove-Item -Recurse -Force $projectPath }
    New-Item -ItemType Directory -Force (Join-Path $projectPath 'gds') | Out-Null
    Copy-Item -LiteralPath $gdsItem.FullName -Destination (Join-Path $projectPath "gds/$gdsStem.gds")

    # A minimal info.yaml the precheck accepts: the top_module equals the GDS stem, an
    # analog GDS-mode tile with the six analog pins the Reticle template exposes.
    $infoYaml = @"
# Minimal info.yaml staged by scripts/tt-precheck.ps1 so the precheck can find a
# top_module. A real submission supplies its own info.yaml; pass -ProjectDir to use it.
project:
  title: "$gdsStem"
  author: "reticle"
  description: "GDS-mode tile prechecked by reticle just tt-precheck"
  language: "GDS"
  top_module: "$gdsStem"
  tiles: "$Tiles"
  analog_pins: 6
  uses_vapwr: true
pinout:
  ua:
    - "analog 0"
    - "analog 1"
    - "analog 2"
    - "analog 3"
    - "analog 4"
    - "analog 5"
"@
    Set-Content -LiteralPath (Join-Path $projectPath 'info.yaml') -Value $infoYaml -Encoding utf8
    $gdsMountArg = "/work/gds/$gdsStem.gds"
    Write-Output "tt-precheck: staged minimal project at $projectPath (top_module=$gdsStem)"
}

New-Item -ItemType Directory -Force $OutDir | Out-Null
$outPath = (Get-Item -LiteralPath $OutDir).FullName

# --- 5. Run the precheck in the container ----------------------------------------------
# Mounts:
#   $projectPath -> /work        (info.yaml + gds/, so the upward info.yaml search finds it)
#   $supportPath -> /support     (the pinned tt-support-tools checkout)
# The precheck writes reports to /support/precheck/reports; we copy them out afterward.
# PDK_ROOT and the tools are baked into the image.
# The image bundles Magic, KLayout (klayout.db) and the SKY130 PDK, but not `gdstk`,
# which precheck.py imports directly. Install the exact versions tt-support-tools pins in
# precheck/requirements.txt: gdstk 0.9.52 is built against NumPy 1.x, and the image ships
# NumPy 2.2.1, so the pinned numpy 1.26.4 must come too. This is how TinyTapeout intends
# the precheck to run: the heavy EDA tools come from the image, the precheck's own Python
# deps are pip-installed. The image entrypoint sets PYTHONPATH with system dist-packages
# (NumPy 2) ahead of the user site, so prepend the user site to PYTHONPATH so the pinned
# numpy 1.26.4 and gdstk resolve first while KLayout stays available from the image.
$containerCmd = @"
set -e
cd /support/precheck
pip install --quiet --no-input --break-system-packages gdstk==0.9.52 numpy==1.26.4
export PYTHONPATH=/headless/.local/lib/python3.12/site-packages:`$PYTHONPATH
python3 precheck.py --gds '$gdsMountArg' --tech '$Tech'
"@

# The iic-osic-tools image ships an entrypoint launcher (X11/VNC UI bootstrap) that
# treats the container command as its own options unless `--skip` is the FIRST argument
# after the image: `--skip` bypasses the UI startup and execs the assigned command. Without
# it the launcher rejects `bash` ("Unexpected option") and never runs the precheck.
$dockerArgs = @(
    'run', '--rm',
    '-v', "${projectPath}:/work",
    '-v', "${supportPath}:/support",
    '-e', 'PDK=sky130A',
    $image,
    '--skip', 'bash', '-lc', $containerCmd
)

Write-Output "tt-precheck: docker $($dockerArgs -join ' ')"
Write-Output 'tt-precheck: running precheck (this pulls the image on first run; multi-GB)...'

$sw = [System.Diagnostics.Stopwatch]::StartNew()
& docker @dockerArgs
$exit = $LASTEXITCODE
$sw.Stop()
$wallS = [math]::Round($sw.Elapsed.TotalSeconds, 1)

# --- 6. Copy the reports back ----------------------------------------------------------
$reportsSrc = Join-Path $supportPath 'precheck/reports'
if (Test-Path -LiteralPath $reportsSrc) {
    Copy-Item -Path (Join-Path $reportsSrc '*') -Destination $outPath -Recurse -Force -ErrorAction SilentlyContinue
    Write-Output "tt-precheck: reports copied to $outPath"
} else {
    Write-Output "tt-precheck: WARNING no reports directory at $reportsSrc (did the precheck start?)"
}

Write-Output "MEASURE|tt-precheck|image=$image|wall_s=$wallS|exit=$exit|reports=$outPath"
if ($exit -eq 0) {
    Write-Output "tt-precheck: PASSED for $gdsStem (precheck exit 0)"
} else {
    Write-Output "tt-precheck: FAILED for $gdsStem (precheck exit $exit); see $outPath/results.md"
}
exit $exit

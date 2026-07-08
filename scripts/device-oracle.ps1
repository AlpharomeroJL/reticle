# Extract a GDS with Magic inside the pinned container and count its MOSFETs, an
# external oracle for reticle-extract's device recognition.
#
# reticle-extract recognizes SKY130 MOSFETs from poly-over-diffusion geometry. To
# check that recognition against an independent tool, this script runs Magic's own
# device extractor (`extract all` + `ext2spice`) over a committed GDS inside the
# PINNED `hpretl/iic-osic-tools` image (the same image the tt-precheck recipe uses;
# it bundles Magic, Netgen, and the sky130A PDK at /foss/pdks). It then counts the
# nfet/pfet devices Magic emitted. The Rust tests assert the same cell extracts to
# the same device count and types; RESULT.md records the two side by side.
#
# What it does, end to end:
#   1. Verifies the GDS exists and begins with a GDSII HEADER record.
#   2. Stages the GDS into a work directory mounted at /work.
#   3. Runs, inside the container: magic in batch mode over a small Tcl script that
#      reads the GDS, loads the top cell, extracts devices, and writes out.spice;
#      then greps the SPICE for nfet/pfet instance counts.
#   4. Copies out.spice back to -OutDir and prints a MEASURE line with the wall time,
#      the container exit code, and the NMOS/PMOS counts Magic found.
#
# This is additive tooling (a plain script, not part of `just ci`): it needs Docker
# and the multi-GB image, exactly like `just tt-precheck`. If the image or Docker is
# unavailable the script exits non-zero and the tests fall back to the committed
# golden fixture (see crates/reticle-extract/tests/fixtures/inverter.md).
#
# Usage:
#   powershell -File scripts/device-oracle.ps1
#   powershell -File scripts/device-oracle.ps1 -Gds crates/reticle-app/assets/sky130_fd_sc_hd__inv_1.gds
#   powershell -File scripts/device-oracle.ps1 -Gds path/to/cell.gds -TopCell my_cell -OutDir scratch/device-oracle

param(
    # The GDS layout to extract. Defaults to the bundled inverter standard cell.
    [string]$Gds = 'crates/reticle-app/assets/sky130_fd_sc_hd__inv_1.gds',
    # The top cell to load and extract. Defaults to the GDS filename stem.
    [string]$TopCell = '',
    # Where to copy the extracted SPICE netlist.
    [string]$OutDir = 'scratch/device-oracle',
    # The PINNED iic-osic-tools image tag. A dated tag (YYYY.MM), never `latest`.
    [string]$ImageTag = '2025.01'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$image = "hpretl/iic-osic-tools:$ImageTag"

function Fail($message) {
    Write-Output "device-oracle: $message"
    exit 2
}

# --- 1. Validate the GDS ---------------------------------------------------------------
if (-not (Test-Path -LiteralPath $Gds)) { Fail "GDS not found: $Gds" }
$gdsItem = Get-Item -LiteralPath $Gds
$gdsStem = [System.IO.Path]::GetFileNameWithoutExtension($gdsItem.Name)
if (-not $TopCell) { $TopCell = $gdsStem }
$bytes = [System.IO.File]::ReadAllBytes($gdsItem.FullName)
$isGds = ($bytes.Length -ge 6 -and $bytes[0] -eq 0 -and $bytes[1] -eq 6 -and
    $bytes[2] -eq 0 -and $bytes[3] -eq 2)
if (-not $isGds) { Fail "$Gds does not begin with a GDSII HEADER record" }

# --- 2. Ensure Docker is present -------------------------------------------------------
$dockerVersion = (& docker --version) 2>$null
if ($LASTEXITCODE -ne 0) {
    Fail 'docker not found on PATH. Install Docker to run the oracle; else use the golden fixture.'
}
Write-Output "device-oracle: using $dockerVersion"
Write-Output "device-oracle: pinned image $image"
Write-Output "device-oracle: extracting $Gds (top cell $TopCell)"

# --- 3. Stage the work directory (gds + tcl) -------------------------------------------
$workPath = Join-Path (Get-Location) "scratch/device-oracle-work/$gdsStem"
if (Test-Path -LiteralPath $workPath) { Remove-Item -Recurse -Force $workPath }
New-Item -ItemType Directory -Force $workPath | Out-Null
Copy-Item -LiteralPath $gdsItem.FullName -Destination (Join-Path $workPath 'in.gds')

# The Magic batch script: read the GDS, load the top cell, extract devices to .ext,
# then turn the .ext into a SPICE netlist. `ext2spice lvs` selects the LVS-oriented
# format whose device lines name the sky130 fet models (nfet/pfet).
$tcl = @'
crashbackups stop
drc off
gds read /work/in.gds
load TOPCELL_PLACEHOLDER
select top cell
extract all
ext2spice lvs
ext2spice -o /work/out.spice
quit -noprompt
'@
$tcl = $tcl.Replace('TOPCELL_PLACEHOLDER', $TopCell)
Set-Content -LiteralPath (Join-Path $workPath 'oracle.tcl') -Value $tcl -Encoding utf8

New-Item -ItemType Directory -Force $OutDir | Out-Null
$outPath = (Get-Item -LiteralPath $OutDir).FullName

# --- 4. Run Magic in the container -----------------------------------------------------
# The image entrypoint needs `--skip` as its first argument to bypass the VNC/X11 UI
# and exec the assigned command (see scripts/tt-precheck.ps1 for the same trick). The
# sky130A magicrc under $PDK_ROOT supplies the tech so Magic knows the device layers.
$containerCmd = @'
set -e
cd /work
export PDK=sky130A
magic -dnull -noconsole -rcfile "$PDK_ROOT/sky130A/libs.tech/magic/sky130A.magicrc" oracle.tcl < /dev/null
'@

$dockerArgs = @(
    'run', '--rm',
    '-v', "${workPath}:/work",
    '-e', 'PDK=sky130A',
    $image,
    '--skip', 'bash', '-lc', $containerCmd
)

Write-Output 'device-oracle: running magic extraction in the container...'
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$log = (& docker @dockerArgs 2>&1 | Out-String)
$exit = $LASTEXITCODE
$sw.Stop()
$wallS = [math]::Round($sw.Elapsed.TotalSeconds, 1)
Write-Output $log

# --- 5. Copy the SPICE back and count the devices --------------------------------------
# Count device instances host-side from the extracted SPICE: Magic writes one line
# per fet naming the sky130 model (`..._nfet_...` / `..._pfet_...`), so the model
# name is an unambiguous NMOS/PMOS tag.
$nmos = 'NA'; $pmos = 'NA'
$spiceSrc = Join-Path $workPath 'out.spice'
if (Test-Path -LiteralPath $spiceSrc) {
    Copy-Item -LiteralPath $spiceSrc -Destination (Join-Path $outPath 'out.spice') -Force
    Write-Output "device-oracle: SPICE copied to $outPath/out.spice"
    $lines = [System.IO.File]::ReadAllLines((Join-Path $outPath 'out.spice'))
    $nmos = ($lines | Where-Object { $_ -match '_nfet_' }).Count
    $pmos = ($lines | Where-Object { $_ -match '_pfet_' }).Count
} else {
    Write-Output "device-oracle: WARNING no out.spice produced (did magic extract run?)"
}

Write-Output "MEASURE|device-oracle|image=$image|cell=$TopCell|wall_s=$wallS|exit=$exit|nmos=$nmos|pmos=$pmos"
if ($exit -eq 0) {
    Write-Output "device-oracle: magic extracted $TopCell -> nmos=$nmos pmos=$pmos"
} else {
    Write-Output "device-oracle: FAILED (magic exit $exit); falling back to the golden fixture is appropriate"
}
exit $exit

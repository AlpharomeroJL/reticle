<#
.SYNOPSIS
  GDS round-trip interop harness: compares Reticle's GDS round-trip against KLayout and
  gdspy in the pinned hpretl/iic-osic-tools container, and writes a divergence report.

.DESCRIPTION
  Steps:
    1. (container, gdspy)  generate clean.gds and odd.gds fixtures
    2. (host, Rust)        Reticle reads each fixture and re-exports GDS
    3. (container)         KLayout and gdspy each read+re-export each fixture
    4. (container, KLayout normalizer) diff every tool's output, write the report
    5. (host)              copy the report to docs/interop/

  Also validates the conformant-OASIS writer: Reticle exports each fixture as OASIS
  (oasis_std) on the host, and KLayout attempts to read it in the container.

  If Docker is unavailable, prints the exact command that was skipped and exits 3 so the
  caller can record the container comparison as not-run (the writer and PDK still ship).

.PARAMETER Image
  The pinned container image (default hpretl/iic-osic-tools:2025.01).
#>
param(
  [string]$Image = "hpretl/iic-osic-tools:2025.01",
  [string]$Work = "$PSScriptRoot/work",
  [string]$Report = "$PSScriptRoot/../../docs/interop/gds-roundtrip.generated.md"
)

$ErrorActionPreference = "Stop"
$harness = ($PSScriptRoot -replace '\\', '/')

function Invoke-Docker([string[]]$DockerArgs) {
  Write-Host "docker $($DockerArgs -join ' ')" -ForegroundColor DarkGray
  & docker @DockerArgs
  if ($LASTEXITCODE -ne 0) { throw "docker exited $LASTEXITCODE" }
}

# --- Preflight: is Docker usable? ---
$dockerOk = $false
try {
  & docker info *> $null
  if ($LASTEXITCODE -eq 0) { $dockerOk = $true }
} catch { $dockerOk = $false }

New-Item -ItemType Directory -Force $Work | Out-Null
$WorkAbs = (Resolve-Path $Work).Path
$WorkVol = $WorkAbs -replace '\\', '/'

if (-not $dockerOk) {
  Write-Warning "Docker is not available. The KLayout/gdspy comparison was NOT run."
  Write-Warning "To run it: docker run --rm -v ${WorkVol}:/work -v ${harness}:/harness $Image --skip python3 /harness/interop.py ..."
  exit 3
}

# --- 1. Fixtures (container). ---
Invoke-Docker @("run", "--rm", "-v", "${WorkVol}:/work", "-v", "${harness}:/harness",
  $Image, "--skip", "python3", "/harness/interop.py", "fixtures", "/work")

# --- 2. Reticle round-trip (host). Reuse the lane target dir for a warm build. ---
$env:CARGO_TARGET_DIR = "E:/dev/reticle-target-v8-5d-interop-pdk"
Push-Location "$PSScriptRoot/reticle-roundtrip"
try {
  & cargo build --release --quiet
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally { Pop-Location }
$bin = "E:/dev/reticle-target-v8-5d-interop-pdk/release/reticle-roundtrip.exe"

foreach ($fx in @("clean", "odd")) {
  & $bin gds "$WorkAbs/$fx.gds" "$WorkAbs/$fx.reticle.gds"
  if ($LASTEXITCODE -ne 0) { throw "reticle gds round-trip failed for $fx" }
  # Conformant-OASIS export for the KLayout read test.
  & $bin oasis-std "$WorkAbs/$fx.gds" "$WorkAbs/$fx.reticle.oas"
  if ($LASTEXITCODE -ne 0) { throw "reticle oasis-std export failed for $fx" }
}

# --- 3+4. Tool round-trips, normalize, report, and the OASIS read test (container). ---
$script = @'
set -e
for fx in clean odd; do
  python3 /harness/interop.py roundtrip klayout /work/$fx.gds /work/$fx.klayout.gds
  python3 /harness/interop.py roundtrip gdspy   /work/$fx.gds /work/$fx.gdspy.gds
done
python3 /harness/interop.py report /work /work/report.md
echo "=== OASIS read test (KLayout reading Reticle's conformant-OASIS writer) ==="
for fx in clean odd; do
  python3 /harness/interop.py oasis-check /work/$fx.reticle.oas || true
done
'@
Invoke-Docker @("run", "--rm", "-v", "${WorkVol}:/work", "-v", "${harness}:/harness",
  $Image, "--skip", "bash", "-c", $script)

# --- 5. Publish the report. ---
New-Item -ItemType Directory -Force (Split-Path $Report) | Out-Null
Copy-Item "$WorkAbs/report.md" $Report -Force
if (Test-Path "$WorkAbs/report.md.json") { Copy-Item "$WorkAbs/report.md.json" "$Report.json" -Force }
Write-Host "Report written to $Report" -ForegroundColor Green

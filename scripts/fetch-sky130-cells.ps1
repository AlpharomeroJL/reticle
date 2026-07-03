# Fetches a handful of real sky130_fd_sc_hd standard cells for corpus work.
#
# Downloads the drive-1 GDS of each named cell from the Google SkyWater PDK
# standard-cell library (Apache-2.0), plus the library LICENSE for attribution,
# into a destination directory (default: scratch/cells). Each download is
# verified to start with a GDSII HEADER record (00 06 00 02) so a proxy error
# page can never masquerade as a cell.
#
# The committed corpus at crates/reticle-io/tests/corpus/sky130/ holds a
# minimized subset of these files; this script exists so the full set is
# reproducible without hunting for URLs.
#
# Usage: powershell -File scripts/fetch-sky130-cells.ps1 [-Dest <dir>]

param(
    [string]$Dest = 'scratch/cells'
)

$ErrorActionPreference = 'Stop'
$repoRaw = 'https://raw.githubusercontent.com/google/skywater-pdk-libs-sky130_fd_sc_hd/main'
$cells = 'inv', 'nand2', 'dfxtp', 'fill', 'tap'

New-Item -ItemType Directory -Force $Dest | Out-Null

$failed = $false
foreach ($cell in $cells) {
    $name = "sky130_fd_sc_hd__${cell}_1.gds"
    $url = "$repoRaw/cells/$cell/$name"
    $out = Join-Path $Dest $name
    curl.exe -sSfL --retry 2 -o $out $url
    if ($LASTEXITCODE -ne 0) {
        Write-Output "FAIL  $name (download error from $url)"
        $failed = $true
        continue
    }
    # A GDSII stream begins with a HEADER record: length 6, type 0x00, data
    # type 0x02 (two-byte signed int). Anything else is not GDS.
    $bytes = [System.IO.File]::ReadAllBytes($out)
    if ($bytes.Length -lt 6 -or $bytes[0] -ne 0 -or $bytes[1] -ne 6 -or
        $bytes[2] -ne 0 -or $bytes[3] -ne 2) {
        Write-Output "FAIL  $name (no GDSII HEADER record; got $($bytes.Length) bytes)"
        $failed = $true
        continue
    }
    Write-Output ("OK    {0}  {1} bytes" -f $name, $bytes.Length)
}

# The library license, for the corpus attribution note.
curl.exe -sSfL -o (Join-Path $Dest 'LICENSE') "$repoRaw/LICENSE"
if ($LASTEXITCODE -ne 0) {
    Write-Output 'FAIL  LICENSE'
    $failed = $true
} else {
    Write-Output 'OK    LICENSE (Apache-2.0)'
}

if ($failed) { exit 1 }
Write-Output "fetch-sky130-cells: all downloads verified into $Dest"

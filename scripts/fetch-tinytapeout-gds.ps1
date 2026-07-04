# Fetches a handful of real, published Tiny Tapeout submitted-design GDS files.
#
# Tiny Tapeout is a community shuttle that fabricates many small open-source
# designs on one chip. Each shuttle repo publishes every submitted project's
# hardened layout as a gzip-compressed GDSII stream under `gds/<project>.gds.gz`.
# Those files are Apache-2.0 licensed (the shuttle repo carries the LICENSE).
#
# This script downloads a small, named set of those real designs from the
# `TinyTapeout/tinytapeout-03` shuttle (discovered from the repo's `gds/`
# directory), decompresses each `.gz`, and verifies the result begins with a
# GDSII HEADER record (00 06 00 02) so a proxy error page can never masquerade as
# a design. It writes a NOTICE file recording the exact source URL, shuttle, and
# license per file.
#
# The full designs are large (0.5 to 1.3 MB uncompressed): a whole shuttle tile,
# not a single cell. They are NOT committed. The committed corpus under
# `corpus/tinytapeout/` holds only minimized real samples plus synthesized
# malformed files, produced reproducibly by the in-repo generator
# (`cargo run -p reticle-io --example gen_tinytapeout_corpus`, gated behind the
# `corpus-tools` feature). This script exists so the full real set is
# reproducible without hunting for URLs, and so the provenance is auditable.
#
# Usage: powershell -File scripts/fetch-tinytapeout-gds.ps1 [-Dest <dir>]

param(
    [string]$Dest = 'scratch/tinytapeout'
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

# The Tiny Tapeout 03 shuttle. Its `gds/` directory holds one `*.gds.gz` per
# submitted project; these three are among the smaller, self-contained designs.
$shuttle = 'tinytapeout-03'
$repoRaw = "https://raw.githubusercontent.com/TinyTapeout/$shuttle/main"
$projects = 'adder', 'azdle_binary_clock', 'aramsey118_freq_counter'

New-Item -ItemType Directory -Force $Dest | Out-Null

# Decompress a gzip file to a plain output path, without extra dependencies.
Add-Type -AssemblyName System.IO.Compression.FileSystem
function Expand-GzipFile {
    param([string]$InPath, [string]$OutPath)
    $ins = [System.IO.File]::OpenRead($InPath)
    try {
        $gz = New-Object System.IO.Compression.GZipStream($ins, [System.IO.Compression.CompressionMode]::Decompress)
        try {
            $outs = [System.IO.File]::Create($OutPath)
            try { $gz.CopyTo($outs) } finally { $outs.Close() }
        } finally { $gz.Close() }
    } finally { $ins.Close() }
}

$notice = New-Object System.Collections.Generic.List[string]
$notice.Add('# Tiny Tapeout GDS provenance')
$notice.Add('')
$notice.Add('Real submitted-design GDSII fetched from the Tiny Tapeout 03 shuttle.')
$notice.Add("Shuttle repo: https://github.com/TinyTapeout/$shuttle")
$notice.Add('License: Apache-2.0 (see the shuttle repo LICENSE).')
$notice.Add('')
$notice.Add('| project | source URL | uncompressed bytes | GDSII header |')
$notice.Add('|---------|------------|--------------------|--------------|')

$failed = $false
foreach ($proj in $projects) {
    $url = "$repoRaw/gds/$proj.gds.gz"
    $gz = Join-Path $Dest "$proj.gds.gz"
    $raw = Join-Path $Dest "$proj.gds"
    curl.exe -sSfL --retry 2 -o $gz $url
    if ($LASTEXITCODE -ne 0) {
        Write-Output "FAIL  $proj (download error from $url)"
        $notice.Add("| $proj | $url | (download failed) | (n/a) |")
        $failed = $true
        continue
    }
    Expand-GzipFile -InPath $gz -OutPath $raw
    $bytes = [System.IO.File]::ReadAllBytes($raw)
    # A GDSII stream begins with a HEADER record: length 6, type 0x00, data type
    # 0x02 (two-byte signed int). Anything else is not GDS.
    $isGds = ($bytes.Length -ge 6 -and $bytes[0] -eq 0 -and $bytes[1] -eq 6 -and
        $bytes[2] -eq 0 -and $bytes[3] -eq 2)
    $hdr = if ($bytes.Length -ge 4) { '{0:x2} {1:x2} {2:x2} {3:x2}' -f $bytes[0], $bytes[1], $bytes[2], $bytes[3] } else { 'short' }
    if (-not $isGds) {
        Write-Output "FAIL  $proj (no GDSII HEADER record; got $($bytes.Length) bytes)"
        $notice.Add("| $proj | $url | $($bytes.Length) | $hdr (NOT GDS) |")
        $failed = $true
        continue
    }
    Write-Output ("OK    {0}  {1} bytes uncompressed" -f $proj, $bytes.Length)
    $notice.Add("| $proj | $url | $($bytes.Length) | $hdr |")
}

# The shuttle license, for the provenance record.
$licenseOut = Join-Path $Dest 'LICENSE'
curl.exe -sSfL -o $licenseOut "$repoRaw/LICENSE"
if ($LASTEXITCODE -ne 0) {
    Write-Output 'WARN  LICENSE (download failed; record it manually)'
    $notice.Add('')
    $notice.Add('LICENSE: download failed; the shuttle is Apache-2.0.')
} else {
    Write-Output 'OK    LICENSE (Apache-2.0)'
}

$notice.Add('')
$notice.Add('These full-tile designs are intentionally not committed (size and')
$notice.Add('third-party-license hygiene). The committed corpus keeps only minimized')
$notice.Add('real samples derived from them plus synthesized malformed files.')
[System.IO.File]::WriteAllLines((Join-Path $Dest 'NOTICE.md'), $notice)

if ($failed) {
    Write-Output ''
    Write-Output 'fetch-tinytapeout-gds: some downloads failed. The malformed corpus is'
    Write-Output 'still generated in-repo, so hardening remains fully provable offline.'
    exit 1
}
Write-Output "fetch-tinytapeout-gds: all downloads verified into $Dest"

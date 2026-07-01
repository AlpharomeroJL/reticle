# Voice rule gate: the tree must contain no em-dash (U+2014).
#
# The repository is written to read as human-authored prose, which for this
# project means no em-dashes anywhere in docs, README, comments, or doc-comments.
# This script scans every tracked text file (read as UTF-8 so multi-byte
# characters are seen correctly on Windows PowerShell 5.1) and fails with a file
# and line list if any em-dash is found. Wired into `just ci`.

$ErrorActionPreference = 'Stop'
$dash = [char]0x2014
$exts = '*.rs', '*.md', '*.toml', '*.wgsl', '*.html', '*.json', '*.proto',
        '*.css', '*.js', '*.yml', '*.yaml', '*.sh', '*.ps1', '*.txt'
$files = git ls-files -- $exts | Where-Object { $_ -ne 'Cargo.lock' }

$bad = New-Object System.Collections.Generic.List[string]
foreach ($f in $files) {
    $full = Join-Path (Get-Location) $f
    if (-not (Test-Path -LiteralPath $full)) { continue }
    $lines = [System.IO.File]::ReadAllLines($full, [System.Text.Encoding]::UTF8)
    for ($i = 0; $i -lt $lines.Length; $i++) {
        if ($lines[$i].IndexOf($dash) -ge 0) {
            $bad.Add("${f}:$($i + 1)")
        }
    }
}

if ($bad.Count -gt 0) {
    Write-Output "check-style: em-dash (U+2014) found in $($bad.Count) place(s); the voice rule forbids em-dashes:"
    $bad | Select-Object -First 80 | ForEach-Object { Write-Output "  $_" }
    exit 1
}
Write-Output 'check-style: OK (no em-dashes)'

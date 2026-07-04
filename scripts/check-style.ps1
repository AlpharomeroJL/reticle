# Voice-rule gate: no em-dashes anywhere, and no marketing adjectives in README.md.
#
# The repository is written to read as human-authored prose. For this project that
# means two things this script enforces, both wired into `just ci`:
#
#   1. No em-dash (U+2014) in any tracked text file (docs, README, comments).
#   2. No marketing adjective in README.md. The README leads with facts and numbers,
#      not adjectives, so a small banned-word list keeps the voice honest: a claim
#      should carry a measurement or a link, never a word like "powerful".
#
# Files are read as UTF-8 so multi-byte characters are seen correctly on Windows
# PowerShell 5.1. The script prints a file-and-line list for every violation and
# exits non-zero if either check fails.

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

# Marketing adjectives banned from README.md. A feature bullet names what you can do
# and every claim carries a number or a link, so these words never earn their place.
$banned = @(
    'powerful', 'blazing', 'blazingly', 'seamless', 'seamlessly', 'robust',
    'comprehensive', 'cutting-edge', 'state-of-the-art', 'revolutionary',
    'world-class', 'best-in-class', 'effortless', 'effortlessly', 'game-changing',
    'turnkey', 'unparalleled', 'next-generation', 'industry-leading'
)
$readme = Join-Path (Get-Location) 'README.md'
$bannedHits = New-Object System.Collections.Generic.List[string]
if (Test-Path -LiteralPath $readme) {
    $lines = [System.IO.File]::ReadAllLines($readme, [System.Text.Encoding]::UTF8)
    for ($i = 0; $i -lt $lines.Length; $i++) {
        foreach ($word in $banned) {
            $pattern = '(?i)\b' + [regex]::Escape($word) + '\b'
            if ([regex]::IsMatch($lines[$i], $pattern)) {
                $bannedHits.Add("README.md:$($i + 1): $word")
            }
        }
    }
}

$failed = $false
if ($bad.Count -gt 0) {
    Write-Output "check-style: em-dash (U+2014) found in $($bad.Count) place(s); the voice rule forbids em-dashes:"
    $bad | Select-Object -First 80 | ForEach-Object { Write-Output "  $_" }
    $failed = $true
}
if ($bannedHits.Count -gt 0) {
    Write-Output "check-style: banned marketing word(s) in README.md; lead with a fact, not an adjective:"
    $bannedHits | Select-Object -First 80 | ForEach-Object { Write-Output "  $_" }
    $failed = $true
}
if ($failed) {
    exit 1
}
Write-Output 'check-style: OK (no em-dashes; no banned words in README.md)'

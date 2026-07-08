# Voice-rule gate: no em-dashes anywhere, no marketing adjectives in README.md,
# and no hard-coded UI colors or font sizes outside the theme module.
#
# The repository is written to read as human-authored prose. For this project that
# means three things this script enforces, all wired into `just ci`:
#
#   1. No em-dash (U+2014) in any tracked text file (docs, README, comments).
#   2. No marketing adjective in README.md. The README leads with facts and numbers,
#      not adjectives, so a small banned-word list keeps the voice honest: a claim
#      should carry a measurement or a link, never a word like "powerful".
#   3. No raw Color32 constructor or FontId/RichText size literal in UI code
#      outside crates/reticle-app/src/theme/. Colors and type sizes belong to the
#      theme so light/dark and density changes happen in one place. Existing
#      violations are grandfathered in scripts/style-baseline.json, a ratchet:
#      counts may only fall. Run with -Ratchet to tighten the baseline to the
#      current counts (never looser); when every count reaches zero the ratchet
#      deletes the baseline file and the check becomes absolute.
#
# Files are read as UTF-8 so multi-byte characters are seen correctly on Windows
# PowerShell 5.1. The script prints a file-and-line list for every violation and
# exits non-zero if any check fails.
param(
    [switch]$Ratchet
)

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

# UI style lint: raw colors and font sizes are banned outside the theme module.
# Anything matching these constructors bypasses the theme, so a palette or density
# change silently misses it; the fix is a theme token, not a literal. Comment lines
# are skipped because prose ABOUT the API is not a use of it.
$uiPatterns = @(
    'Color32::from_rgb\(',
    'Color32::from_rgba_unmultiplied\(',
    'Color32::from_rgba_premultiplied\(',
    'Color32::from_gray\(',
    'RichText::new\([^)]*\)\.size\(',
    'FontId::new\(',
    'FontId::monospace\(',
    'FontId::proportional\('
)
$uiRegex = $uiPatterns -join '|'
$uiFiles = git ls-files -- 'crates/reticle-app/src/*.rs' 'crates/web/src/*.rs' |
    Where-Object { $_ -notlike 'crates/reticle-app/src/theme/*' }

$uiCounts = @{}
$uiLines = @{}
foreach ($f in $uiFiles) {
    $full = Join-Path (Get-Location) $f
    if (-not (Test-Path -LiteralPath $full)) { continue }
    $lines = [System.IO.File]::ReadAllLines($full, [System.Text.Encoding]::UTF8)
    $hits = New-Object System.Collections.Generic.List[string]
    for ($i = 0; $i -lt $lines.Length; $i++) {
        if ($lines[$i].TrimStart().StartsWith('//')) { continue }
        if ([regex]::IsMatch($lines[$i], $uiRegex)) {
            $hits.Add("${f}:$($i + 1)")
        }
    }
    if ($hits.Count -gt 0) {
        $uiCounts[$f] = $hits.Count
        $uiLines[$f] = $hits
    }
}

# The grandfather baseline: a flat map of repo-relative path -> allowed count.
# A missing file means no debt is tolerated anywhere (the ratchet's end state).
$baselinePath = Join-Path (Get-Location) 'scripts/style-baseline.json'
$baseline = @{}
$baselineExists = Test-Path -LiteralPath $baselinePath
if ($baselineExists) {
    $parsed = [System.IO.File]::ReadAllText($baselinePath, [System.Text.Encoding]::UTF8) |
        ConvertFrom-Json
    foreach ($prop in $parsed.PSObject.Properties) {
        $baseline[$prop.Name] = [int]$prop.Value
    }
}

if ($Ratchet) {
    # Rewrite the baseline to the current counts, but never above the existing
    # baseline: the ratchet only tightens, so a new violation can never be
    # grandfathered by re-running it. Zero-count entries are dropped entirely.
    $next = @{}
    foreach ($f in $uiCounts.Keys) {
        $count = $uiCounts[$f]
        if ($baselineExists) {
            $cap = 0
            if ($baseline.ContainsKey($f)) { $cap = $baseline[$f] }
            $count = [Math]::Min($count, $cap)
        }
        if ($count -gt 0) { $next[$f] = $count }
    }
    if ($next.Count -eq 0) {
        if ($baselineExists) {
            Remove-Item -LiteralPath $baselinePath -Force
        }
        Write-Output 'check-style: ratchet complete; zero UI style violations, baseline file removed.'
        exit 0
    }
    $keys = @($next.Keys | Sort-Object)
    $sb = New-Object System.Text.StringBuilder
    [void]$sb.Append("{`n")
    for ($i = 0; $i -lt $keys.Count; $i++) {
        $comma = ','
        if ($i -eq $keys.Count - 1) { $comma = '' }
        [void]$sb.Append("  `"$($keys[$i])`": $($next[$keys[$i]])$comma`n")
    }
    [void]$sb.Append("}`n")
    # UTF-8 without BOM: ConvertTo-Json is avoided because PS 5.1 hard-codes its
    # indentation and Out-File would prepend a BOM.
    [System.IO.File]::WriteAllText($baselinePath, $sb.ToString(),
        (New-Object System.Text.UTF8Encoding($false)))
    Write-Output "check-style: baseline written to scripts/style-baseline.json ($($keys.Count) file(s))."
    foreach ($k in $keys) { Write-Output "  $($k): $($next[$k])" }
    exit 0
}

# Gate the UI counts against the baseline. Over baseline fails with the full line
# list; under baseline only warns, so someone who pays down debt is nudged to run
# `just style-ratchet` and lock the improvement in.
$uiFailures = New-Object System.Collections.Generic.List[string]
$uiWarnings = New-Object System.Collections.Generic.List[string]
$uiAllFiles = @($uiCounts.Keys) + @($baseline.Keys) | Sort-Object -Unique
foreach ($f in $uiAllFiles) {
    $count = 0
    if ($uiCounts.ContainsKey($f)) { $count = $uiCounts[$f] }
    $base = 0
    if ($baseline.ContainsKey($f)) { $base = $baseline[$f] }
    if ($count -gt $base) {
        foreach ($hit in $uiLines[$f]) { $uiFailures.Add($hit) }
        $uiFailures.Add("${f}: $count violations, baseline $base")
    } elseif ($count -lt $base) {
        $uiWarnings.Add("ratchet available: $f $count < baseline $base")
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
if ($uiFailures.Count -gt 0) {
    Write-Output 'check-style: hard-coded UI color/size above baseline; use crates/reticle-app/src/theme tokens:'
    $uiFailures | ForEach-Object { Write-Output "  $_" }
    $failed = $true
}
if ($uiWarnings.Count -gt 0) {
    $uiWarnings | ForEach-Object { Write-Output "check-style: $_ (run 'just style-ratchet' to lock it in)" }
}
if ($failed) {
    exit 1
}
Write-Output 'check-style: OK (no em-dashes; no banned words in README.md; UI style within baseline)'

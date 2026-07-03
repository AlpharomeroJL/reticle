# Secret scan gate: fail if a leaked credential is committed to the working tree.
#
# The real Anthropic API key must ONLY ever come from the environment
# (ANTHROPIC_API_KEY); it must never appear in a tracked file. This script scans
# every tracked text file for:
#
#   1. the Anthropic key prefix `sk-ant-` followed by a real-looking token;
#   2. generic `api_key` / `apikey` / `secret` / `token` assignments to a
#      quoted literal that looks like a real credential;
#   3. long, high-entropy strings (Base64/hex) that resemble a raw secret.
#
# It excludes the build target directory and node_modules (via `git ls-files`,
# which already respects .gitignore, plus an explicit path filter). It exits
# non-zero on any hit so it can gate a release.
#
# Run with `just check-keys`. Pass `-History` to also walk the full git history
# (every blob ever committed), which is slower but catches a secret that was
# committed and later removed from the tip.
#
# Known test fixtures: the agent tests deliberately use obviously-fake keys such
# as `sk-ant-test-secret` and `sk-ant-LEAKTEST-...` to prove the redaction path.
# Those are placeholders, not credentials, and are allow-listed by the markers in
# $TestKeyMarkers below. A genuinely real key (which never contains those markers)
# is still caught.

[CmdletBinding()]
param(
    # Also scan the full git history, not just the working tree.
    [switch]$History
)

$ErrorActionPreference = 'Stop'

# Substrings that mark a token as a known test placeholder, not a real secret.
# Keep this list short and specific; every entry is an obviously-fake value used
# in a test that exercises key handling.
$TestKeyMarkers = @(
    'LEAKTEST', 'test-secret', 'leak-me', 'sk-ant-hidden', 'sk-ant-test',
    'sk-ant-x', 'sk-ant-api-key-goes-here', 'example', 'placeholder', 'REDACTED',
    'xxxx', 'your-key-here', 'YOUR_KEY', 'SENTINEL',
    # The standard RFC 4648 base64 alphabet, a public constant, not a secret.
    'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz'
)

# Paths never scanned (belt and suspenders on top of .gitignore).
$ExcludeDirs = @('target/', 'node_modules/', 'dist/', 'D:/dev/reticle-target')

# A real Anthropic key: the prefix, an api-version segment, then a long token.
$AnthropicReal = 'sk-ant-(?:api|sid)\w{0,4}-[A-Za-z0-9_\-]{24,}'
# A looser `sk-ant-` catch for anything with a long token after the dash.
$AnthropicLoose = 'sk-ant-[A-Za-z0-9_\-]{20,}'
# Generic credential assignment: key/secret/token = "<long value>".
# The value must be quoted and reasonably long, so `api_key` used as a field name
# or a short placeholder does not trip.
$AssignPattern = '(?i)(api[_-]?key|secret|access[_-]?token|auth[_-]?token|client[_-]?secret|password)\s*[:=]\s*["''][A-Za-z0-9_\-\.\/\+]{20,}["'']'
# A bare long high-entropy Base64/hex run (a raw secret dumped without a name).
$HighEntropy = '["''`][A-Za-z0-9+/]{40,}={0,2}["''`]'

# Returns true if the matched text is a known test placeholder to be ignored.
function Test-IsAllowlisted([string]$text) {
    foreach ($m in $TestKeyMarkers) {
        if ($text.ToLowerInvariant().Contains($m.ToLowerInvariant())) { return $true }
    }
    return $false
}

# Returns true if a Base64/hex run is high-entropy enough to look like a secret,
# filtering out low-entropy repetitive strings (e.g. a run of one character) and
# obvious non-secrets. A simple distinct-character-ratio heuristic.
function Test-LooksHighEntropy([string]$token) {
    $inner = $token.Trim('"', "'", '`')
    if ($inner.Length -lt 40) { return $false }
    $distinct = ($inner.ToCharArray() | Sort-Object -Unique).Count
    # A real Base64 secret uses a wide alphabet; require a healthy distinct ratio.
    $ratio = $distinct / [double]$inner.Length
    return ($distinct -ge 20 -and $ratio -ge 0.30)
}

$exts = '*.rs', '*.md', '*.toml', '*.wgsl', '*.html', '*.json', '*.proto',
        '*.css', '*.js', '*.ts', '*.yml', '*.yaml', '*.sh', '*.ps1', '*.txt',
        '*.env', '*.cfg', '*.ini', '*.tech', 'Dockerfile', '*.dockerfile'

$hits = New-Object System.Collections.Generic.List[string]

# Scans one text blob (an array of lines) tagged with a source label.
function Scan-Lines([string]$label, [string[]]$lines) {
    for ($i = 0; $i -lt $lines.Length; $i++) {
        $line = $lines[$i]
        if ([string]::IsNullOrEmpty($line)) { continue }

        foreach ($pat in @($script:AnthropicReal, $script:AnthropicLoose, $script:AssignPattern)) {
            foreach ($m in [regex]::Matches($line, $pat)) {
                if (-not (Test-IsAllowlisted $m.Value)) {
                    $script:hits.Add("${label}:$($i + 1): $($m.Value)")
                }
            }
        }
        foreach ($m in [regex]::Matches($line, $script:HighEntropy)) {
            if ((Test-LooksHighEntropy $m.Value) -and -not (Test-IsAllowlisted $m.Value)) {
                $script:hits.Add("${label}:$($i + 1): high-entropy $($m.Value.Substring(0, [Math]::Min(24, $m.Value.Length)))...")
            }
        }
    }
}

# --- Working-tree scan ---------------------------------------------------------
# Minified/vendored bundles (e.g. docs/mermaid.min.js) are third-party assets, not
# first-party source; their long tokens are code, not credentials, so they are not
# scanned (the same rationale as excluding node_modules).
$files = git ls-files -- $exts | Where-Object {
    $f = $_
    ($f -ne 'Cargo.lock') -and
    ($f -ne 'scripts/check-keys.ps1') -and
    ($f -notlike '*.min.js') -and
    (-not ($ExcludeDirs | Where-Object { $f -like "$_*" }))
}

foreach ($f in $files) {
    $full = Join-Path (Get-Location) $f
    if (-not (Test-Path -LiteralPath $full)) { continue }
    $lines = [System.IO.File]::ReadAllLines($full, [System.Text.Encoding]::UTF8)
    Scan-Lines $f $lines
}

# --- Optional full-history scan ------------------------------------------------
if ($History) {
    Write-Output 'check-keys: scanning full git history (this can be slow)...'
    # Every blob object ever committed. For each, scan its content.
    $blobLines = git rev-list --objects --all | ForEach-Object { ($_ -split ' ')[0] }
    $seen = @{}
    foreach ($sha in $blobLines) {
        if ([string]::IsNullOrWhiteSpace($sha) -or $seen.ContainsKey($sha)) { continue }
        $seen[$sha] = $true
        $type = (git cat-file -t $sha 2>$null)
        if ($type -ne 'blob') { continue }
        $content = git cat-file -p $sha 2>$null
        if ($null -eq $content) { continue }
        Scan-Lines "history:$sha" @($content)
    }
}

if ($hits.Count -gt 0) {
    Write-Output "check-keys: found $($hits.Count) possible secret(s); credentials must never be committed:"
    $hits | Select-Object -First 80 | ForEach-Object { Write-Output "  $_" }
    exit 1
}
Write-Output 'check-keys: OK (no leaked secrets found)'

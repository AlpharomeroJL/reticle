# Deployed-URL smoke test for the GitHub Pages site.
#
# Fetches the deployed index.html, extracts every script/module/preload asset it
# references, resolves each against the page URL, and asserts:
#   * the base URL returns HTTP 200,
#   * every referenced asset returns HTTP 200, and
#   * every referenced asset lives under the `/reticle/` path prefix.
#
# This exists to catch the base-path regression that broke the front door: assets
# emitted at absolute root (`/web-<hash>.js`) 404 under the `/reticle/` subpath and
# the page hangs on the loading spinner. Run it AFTER a deploy:
#
#     just smoke-pages
#     just smoke-pages https://alpharomerojl.github.io/reticle/
#
# It is a live check against the deployed site, so it only passes once the correct
# artifact is published. Against the currently-broken live site it is expected to
# fail, and it says exactly why.

[CmdletBinding()]
param(
    [string]$BaseUrl = 'https://alpharomerojl.github.io/reticle/'
)

$ErrorActionPreference = 'Stop'

# The path prefix every asset must sit under. Derived from the base URL's path so
# the check tracks the deployment subpath rather than hard-coding it.
$baseUri = [System.Uri]$BaseUrl
$prefix = $baseUri.AbsolutePath
if (-not $prefix.EndsWith('/')) { $prefix += '/' }

Write-Output "smoke-pages: base = $BaseUrl (asset prefix '$prefix')"

function Get-Url {
    param([string]$Url)
    # -UseBasicParsing avoids the legacy IE DOM engine; we parse the HTML ourselves.
    return Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 30
}

# 1) The base URL itself must be 200.
try {
    $index = Get-Url -Url $BaseUrl
} catch {
    Write-Output "smoke-pages: FAIL - base URL did not return 200: $($_.Exception.Message)"
    exit 1
}
if ($index.StatusCode -ne 200) {
    Write-Output "smoke-pages: FAIL - base URL returned $($index.StatusCode)"
    exit 1
}
Write-Output "smoke-pages: base URL 200 OK"

# 2) Extract asset URLs the page depends on: ES-module imports inside
#    <script type=module>, and href/src on <script>/<link> tags (modulepreload,
#    preload, stylesheet). Regex parsing is sufficient for the Trunk-generated
#    markup and keeps this dependency-free.
$html = $index.Content
$assets = New-Object System.Collections.Generic.List[string]

# import ... from '<url>'  and  import('<url>')  and  module_or_path: '<url>'
$patterns = @(
    "from\s+['""]([^'""]+)['""]",
    "import\(\s*['""]([^'""]+)['""]\s*\)",
    "module_or_path\s*:\s*['""]([^'""]+)['""]",
    "<script[^>]*\ssrc=['""]([^'""]+)['""]",
    "<link[^>]*\shref=['""]([^'""]+)['""]"
)
foreach ($pat in $patterns) {
    foreach ($m in [System.Text.RegularExpressions.Regex]::Matches($html, $pat)) {
        $ref = $m.Groups[1].Value
        # Only interested in first-party JS/WASM assets; skip data URIs and anchors.
        if ($ref -match '^(data:|#|mailto:)') { continue }
        [void]$assets.Add($ref)
    }
}

# De-duplicate while preserving order.
$seen = @{}
$unique = New-Object System.Collections.Generic.List[string]
foreach ($a in $assets) {
    if (-not $seen.ContainsKey($a)) { $seen[$a] = $true; [void]$unique.Add($a) }
}

if ($unique.Count -eq 0) {
    Write-Output "smoke-pages: FAIL - the page referenced no script/module assets; markup may be broken"
    exit 1
}

Write-Output "smoke-pages: checking $($unique.Count) referenced asset(s)"

$failed = $false
foreach ($ref in $unique) {
    # Resolve relative or absolute-root refs against the base URL.
    $resolved = [System.Uri]::new($baseUri, $ref)
    $path = $resolved.AbsolutePath

    $underPrefix = $path.StartsWith($prefix)
    $prefixNote = if ($underPrefix) { 'under prefix' } else { "NOT under '$prefix'" }

    $code = 0
    try {
        $resp = Get-Url -Url $resolved.AbsoluteUri
        $code = [int]$resp.StatusCode
    } catch {
        # Invoke-WebRequest throws on 4xx/5xx; recover the status code if present.
        if ($_.Exception.Response) {
            $code = [int]$_.Exception.Response.StatusCode
        } else {
            $code = -1
        }
    }

    $ok = ($code -eq 200) -and $underPrefix
    $mark = if ($ok) { 'OK  ' } else { 'FAIL' }
    Write-Output ("  [{0}] {1}  ({2}, {3})" -f $mark, $resolved.AbsoluteUri, $code, $prefixNote)
    if (-not $ok) { $failed = $true }
}

if ($failed) {
    Write-Output "smoke-pages: FAIL - one or more assets 404'd or fell outside '$prefix'."
    Write-Output "smoke-pages: this is the base-path regression. Rebuild with 'just deploy-pages' and redeploy."
    exit 1
}

Write-Output "smoke-pages: PASS - base URL and all referenced assets are 200 and under '$prefix'."
exit 0

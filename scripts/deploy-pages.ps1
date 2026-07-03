# Build and assemble the full GitHub Pages artifact into scratch/pages/.
#
# The site is served under the subpath https://alpharomerojl.github.io/reticle/,
# so Trunk MUST emit assets under `/reticle/` rather than at absolute root. This
# script:
#   1. runs `trunk build index.html --release --public-url /reticle/` in crates/web,
#   2. builds the mdbook (`mdbook build docs`),
#   3. assembles a fresh scratch/pages/ staging dir with the emitted web bundle,
#      a `.nojekyll` marker, and the built book under scratch/pages/book/, and
#   4. asserts the emitted index.html references `/reticle/`-prefixed assets and
#      that NO bare `'/web-` absolute-root reference survives.
#
# It never touches git; the orchestrator publishes scratch/pages/ to gh-pages.
# scratch/ is gitignored. Fails (non-zero exit) if the base-path assertion fails.
#
# Invoked by `just deploy-pages`. Run standalone with:
#     powershell -File scripts/deploy-pages.ps1

[CmdletBinding()]
param(
    [string]$PublicUrl = '/reticle/'
)

$ErrorActionPreference = 'Stop'

# Resolve repo root from this script's location so it works from any CWD.
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$webDir = Join-Path $repoRoot 'crates/web'
$distDir = Join-Path $webDir 'dist'
$stageDir = Join-Path $repoRoot 'scratch/pages'
$bookSrc = Join-Path $repoRoot 'docs'
$bookOut = Join-Path $stageDir 'book'

Write-Output "deploy-pages: repo root $repoRoot"
Write-Output "deploy-pages: public-url $PublicUrl"

# --- 1) Trunk build with the subpath baked in. ---------------------------------
Write-Output 'deploy-pages: building the web bundle (trunk build --release)...'
Push-Location $webDir
try {
    & trunk build index.html --release --public-url $PublicUrl
    if ($LASTEXITCODE -ne 0) {
        throw "trunk build failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

# --- 2) Build the mdbook. ------------------------------------------------------
Write-Output 'deploy-pages: building the book (mdbook build docs)...'
& mdbook build $bookSrc
if ($LASTEXITCODE -ne 0) {
    throw "mdbook build failed with exit code $LASTEXITCODE"
}
# mdbook writes to docs/book by default (book.toml build.build-dir).
$mdbookBuilt = Join-Path $bookSrc 'book'
if (-not (Test-Path -LiteralPath $mdbookBuilt)) {
    throw "expected mdbook output at $mdbookBuilt but it does not exist"
}

# --- 3) Assemble a fresh scratch/pages/ staging dir. ---------------------------
Write-Output "deploy-pages: assembling $stageDir ..."
if (Test-Path -LiteralPath $stageDir) {
    Remove-Item -LiteralPath $stageDir -Recurse -Force
}
New-Item -ItemType Directory -Path $stageDir -Force | Out-Null

# Copy the emitted web bundle (index.html, web-*.js, web-*_bg.wasm, preloads).
Copy-Item -Path (Join-Path $distDir '*') -Destination $stageDir -Recurse -Force

# GitHub Pages: `.nojekyll` disables Jekyll so files/dirs starting with `_` and
# the raw wasm/js are served verbatim.
$noJekyll = Join-Path $stageDir '.nojekyll'
if (-not (Test-Path -LiteralPath $noJekyll)) {
    New-Item -ItemType File -Path $noJekyll | Out-Null
}

# Copy the built book under /book.
New-Item -ItemType Directory -Path $bookOut -Force | Out-Null
Copy-Item -Path (Join-Path $mdbookBuilt '*') -Destination $bookOut -Recurse -Force

# --- 4) Assert the base path is correct in the emitted index.html. -------------
$indexPath = Join-Path $stageDir 'index.html'
if (-not (Test-Path -LiteralPath $indexPath)) {
    throw "expected $indexPath but it does not exist"
}
$indexHtml = Get-Content -LiteralPath $indexPath -Raw

# 4a) It must reference the js and wasm under the /reticle/ prefix.
$hasReticleJs = $indexHtml -match '/reticle/web-[0-9a-fA-F]+\.js'
$hasReticleWasm = $indexHtml -match '/reticle/web-[0-9a-fA-F]+_bg\.wasm'

# 4b) There must be NO bare absolute-root asset reference like '/web-....js' or
#     "/web-..._bg.wasm" (i.e. a `/web-` NOT preceded by `reticle`). This is the
#     exact regression that 404s under the subpath. Look for quote + /web- .
$bareRootRef = [System.Text.RegularExpressions.Regex]::Match(
    $indexHtml, "['""]/web-[0-9a-fA-F]+(?:\.js|_bg\.wasm)")

Write-Output "deploy-pages: assertion - /reticle/ js ref present: $hasReticleJs"
Write-Output "deploy-pages: assertion - /reticle/ wasm ref present: $hasReticleWasm"
Write-Output "deploy-pages: assertion - bare '/web-' absolute-root ref present: $($bareRootRef.Success)"

if (-not $hasReticleJs -or -not $hasReticleWasm -or $bareRootRef.Success) {
    Write-Output 'deploy-pages: FAIL - emitted index.html does not reference /reticle/-prefixed assets,'
    Write-Output '                or a bare absolute-root /web- reference survives. This would 404 under'
    Write-Output '                the subpath and hang the page. Aborting.'
    if ($bareRootRef.Success) {
        Write-Output "deploy-pages: offending reference: $($bareRootRef.Value)"
    }
    exit 1
}

# Show the proof lines (the emitted references) for the operator/log.
Write-Output 'deploy-pages: /reticle/ references in emitted index.html:'
foreach ($m in [System.Text.RegularExpressions.Regex]::Matches($indexHtml, '/reticle/web-[0-9a-fA-F]+(?:\.js|_bg\.wasm)')) {
    Write-Output "  $($m.Value)"
}

Write-Output "deploy-pages: PASS - artifact assembled at $stageDir"
Write-Output "deploy-pages: contents:"
Get-ChildItem -LiteralPath $stageDir -Force | ForEach-Object { Write-Output "  $($_.Name)" }
Write-Output 'deploy-pages: publish scratch/pages/ to the gh-pages branch, then run: just smoke-pages'
exit 0

# Subset the embedded UI/mono/icon faces and regenerate theme/icons.rs (ADR 0097).
#
# A DEV-TIME tool only: it is never invoked by the build, the app, or CI. It
# regenerates the four committed subset TTFs under
# crates/reticle-app/assets/fonts/ and the generated glyph-constant module
# crates/reticle-app/src/theme/icons.rs. Rerunning it on the same pinned sources
# with the same fonttools version produces byte-identical outputs (SOURCE_DATE_EPOCH
# is pinned below so the head-table timestamp is stable).
#
# Prerequisites (zero spend, all free licenses):
#   python -m pip install --user fonttools==4.61.1
#   (any 4.x works; 4.61.1 is the version these committed subsets were built with.)
#
# Pinned source releases (SIL OFL 1.1 for the text faces, ISC for Lucide):
#   Inter        v4.1     https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip
#   JetBrainsMono v2.304  https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip
#   Lucide font  1.23.0   https://github.com/lucide-icons/lucide/releases/download/1.23.0/lucide-font-1.23.0.zip
#
# Populate the sources once (they live under scratch/, which is gitignored, so
# only the subsets and licenses are committed):
#   ./scripts/subset-fonts.ps1 -Download
# then regenerate:
#   ./scripts/subset-fonts.ps1
# or do both in one call:
#   ./scripts/subset-fonts.ps1 -Download
#
# The subset unicode range (ADR 0097): Basic Latin, Latin-1 Supplement (micro
# sign, degree, plus/minus, multiply), General Punctuation (en/em dash, curly
# quotes, bullet, ellipsis, prime marks), arrows, a math-operator set for DRC and
# boolean readouts, unit letters (mu, pi, ohm, angstrom), Geometric Shapes
# (swatches and bullets), and check/cross marks. Layout features kept:
# kern, liga, tnum (tabular figures survive so status-bar numerals stay aligned).
# The Lucide subset is exactly the glyphs named in scripts/lucide-glyphs.txt.
[CmdletBinding()]
param(
    [string]$SourceDir = (Join-Path $PSScriptRoot '..\scratch\fonts-src'),
    [string]$OutDir    = (Join-Path $PSScriptRoot '..\crates\reticle-app\assets\fonts'),
    [string]$GlyphList = (Join-Path $PSScriptRoot 'lucide-glyphs.txt'),
    [string]$IconsRs   = (Join-Path $PSScriptRoot '..\crates\reticle-app\src\theme\icons.rs'),
    [switch]$Download
)

$ErrorActionPreference = 'Stop'
# Pinned so fontTools writes a stable head.modified timestamp (reproducible output).
$env:SOURCE_DATE_EPOCH = '1580000000'

# --- Pinned sources -------------------------------------------------------
$sources = @(
    @{ Name = 'Inter-4.1';           Url = 'https://github.com/rsms/inter/releases/download/v4.1/Inter-4.1.zip' }
    @{ Name = 'JetBrainsMono-2.304'; Url = 'https://github.com/JetBrains/JetBrainsMono/releases/download/v2.304/JetBrainsMono-2.304.zip' }
    @{ Name = 'lucide-font-1.23.0';  Url = 'https://github.com/lucide-icons/lucide/releases/download/1.23.0/lucide-font-1.23.0.zip' }
)
$lucideLicenseUrl = 'https://raw.githubusercontent.com/lucide-icons/lucide/1.23.0/LICENSE'

if ($Download) {
    New-Item -ItemType Directory -Force -Path $SourceDir | Out-Null
    $ProgressPreference = 'SilentlyContinue'
    foreach ($s in $sources) {
        $zip = Join-Path $SourceDir "$($s.Name).zip"
        $dir = Join-Path $SourceDir $s.Name
        Write-Output "downloading $($s.Name)"
        Invoke-WebRequest $s.Url -OutFile $zip -UseBasicParsing -TimeoutSec 180
        if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
        Expand-Archive -Path $zip -DestinationPath $dir -Force
    }
    Invoke-WebRequest $lucideLicenseUrl -OutFile (Join-Path $SourceDir 'lucide-LICENSE.txt') -UseBasicParsing -TimeoutSec 60
}

# --- Locate the source faces ---------------------------------------------
$interRegular = Join-Path $SourceDir 'Inter-4.1\extras\ttf\Inter-Regular.ttf'
$interMedium  = Join-Path $SourceDir 'Inter-4.1\extras\ttf\Inter-Medium.ttf'
$jbMono       = Join-Path $SourceDir 'JetBrainsMono-2.304\fonts\ttf\JetBrainsMono-Regular.ttf'
$lucide       = Join-Path $SourceDir 'lucide-font-1.23.0\lucide-font\lucide.ttf'
$codepoints   = Join-Path $SourceDir 'lucide-font-1.23.0\lucide-font\codepoints.json'
$interLicense = Join-Path $SourceDir 'Inter-4.1\LICENSE.txt'
$jbLicense    = Join-Path $SourceDir 'JetBrainsMono-2.304\OFL.txt'
$lucideLicense= Join-Path $SourceDir 'lucide-LICENSE.txt'

foreach ($p in @($interRegular, $interMedium, $jbMono, $lucide, $codepoints)) {
    if (-not (Test-Path -LiteralPath $p)) {
        throw "missing source: $p. Run './scripts/subset-fonts.ps1 -Download' to fetch the pinned releases."
    }
}

# fontTools must be importable; pyftsubset's console script is not always on PATH,
# so the module form is used throughout.
& python -c "import fontTools" 2>$null
if ($LASTEXITCODE -ne 0) {
    throw "fontTools is not installed. Run: python -m pip install --user fonttools==4.61.1"
}
$ftVersion = (& python -c "import fontTools; print(fontTools.version)").Trim()
Write-Output "fonttools $ftVersion"

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

# --- Text-face unicode coverage (ADR 0097) --------------------------------
$textUnicodes = @(
    'U+0020-007E'                     # Basic Latin
    'U+00A0-00FF'                     # Latin-1 Supplement (micro sign, degree, +/-, x, /)
    'U+0131,U+0152-0153,U+0192'       # dotless i, OE/oe, florin
    'U+2010-2027'                     # hyphens, dashes, quotes, dagger, bullet, ellipsis
    'U+2030,U+2032-2033,U+2039-203A,U+2044' # per-mille, prime/double-prime, guillemets, fraction slash
    'U+2070,U+2074-2079,U+207F,U+2080-2089' # super/subscript digits for units
    'U+20AC,U+2122'                   # euro, trademark
    'U+2190-21FF'                     # arrows
    'U+2202,U+2205,U+2212,U+2215,U+221A,U+221E,U+2229,U+222A' # partial, empty set, minus, division slash, root, infinity, intersection, union
    'U+2248,U+2260,U+2264-2265'       # approx, not-equal, <=, >=
    'U+03A9,U+03BC,U+03C0,U+2126,U+212B' # omega, mu, pi, ohm, angstrom (units)
    'U+25A0-25FF'                     # geometric shapes (swatches, bullets)
    'U+2605-2606'                     # star (favorites/bookmarks)
    'U+2612,U+2713-2718'              # ballot box with X, check/cross marks
) -join ','

function Invoke-Subset {
    param([string]$In, [string]$Out, [string]$Unicodes, [string]$Features)
    & python -m fontTools.subset $In `
        "--output-file=$Out" `
        "--unicodes=$Unicodes" `
        "--layout-features=$Features" `
        '--no-hinting' `
        '--desubroutinize' `
        '--no-glyph-names' `
        '--drop-tables+=DSIG'
    if ($LASTEXITCODE -ne 0) { throw "pyftsubset failed for $In" }
}

Write-Output 'subsetting text faces'
Invoke-Subset -In $interRegular -Out (Join-Path $OutDir 'inter-regular.subset.ttf')       -Unicodes $textUnicodes -Features 'kern,liga,tnum'
Invoke-Subset -In $interMedium  -Out (Join-Path $OutDir 'inter-medium.subset.ttf')        -Unicodes $textUnicodes -Features 'kern,liga,tnum'
Invoke-Subset -In $jbMono       -Out (Join-Path $OutDir 'jetbrains-mono-regular.subset.ttf') -Unicodes $textUnicodes -Features 'kern,liga,tnum'

# --- Lucide subset (driven by the glyph list) -----------------------------
$map = @{}
foreach ($p in ((Get-Content $codepoints -Raw | ConvertFrom-Json).PSObject.Properties)) {
    $map[$p.Name] = [int]$p.Value
}
$names = Get-Content $GlyphList |
    ForEach-Object { $_.Trim() } |
    Where-Object { $_ -and -not $_.StartsWith('#') } |
    Select-Object -Unique
$missing = $names | Where-Object { -not $map.ContainsKey($_) }
if ($missing) { throw "glyph names not in the Lucide codepoint map: $($missing -join ', ')" }

$lucideUnicodes = ($names | ForEach-Object { 'U+{0:X4}' -f $map[$_] }) -join ','
Write-Output "subsetting Lucide ($($names.Count) glyphs)"
Invoke-Subset -In $lucide -Out (Join-Path $OutDir 'lucide.subset.ttf') -Unicodes $lucideUnicodes -Features ''

# --- License files (OFL 1.1 for the text faces, ISC for Lucide) -----------
if (Test-Path $interLicense) { Copy-Item $interLicense (Join-Path $OutDir 'Inter-OFL.txt') -Force }
if (Test-Path $jbLicense)    { Copy-Item $jbLicense    (Join-Path $OutDir 'JetBrainsMono-OFL.txt') -Force }
if (Test-Path $lucideLicense){ Copy-Item $lucideLicense (Join-Path $OutDir 'Lucide-LICENSE.txt') -Force }

# --- Generate theme/icons.rs ----------------------------------------------
# One `pub const` per glyph, sorted by constant name, with a do-not-edit header.
function ConvertTo-ConstName {
    param([string]$Kebab)
    ($Kebab -replace '-', '_').ToUpperInvariant()
}

$consts = $names |
    ForEach-Object { [pscustomobject]@{ Const = (ConvertTo-ConstName $_); Name = $_; Cp = $map[$_] } } |
    Sort-Object Const

# Single-quoted here-string: backticks and $ stay literal so the doc code-spans
# and intra-doc link survive into the generated Rust.
$header = @'
//! Generated Lucide glyph constants. DO NOT EDIT BY HAND.
//!
//! Regenerate with `scripts/subset-fonts.ps1` (ADR 0097): it reads the glyph
//! names from `scripts/lucide-glyphs.txt` and the Lucide codepoint map, then
//! writes one `char` constant per glyph, sorted by name. The matching subset
//! `lucide.subset.ttf` is installed as an icon fallback family by
//! [`crate::theme::fonts`], so any label can inline one of these constants.
//!
//! Lucide is ISC licensed (crates/reticle-app/assets/fonts/Lucide-LICENSE.txt).

'@

$sb = New-Object System.Text.StringBuilder
[void]$sb.Append($header)
foreach ($c in $consts) {
    $hex = '{0:x4}' -f $c.Cp
    [void]$sb.Append("/// Lucide ``$($c.Name)`` glyph.`n")
    [void]$sb.Append("pub const $($c.Const): char = '\u{$hex}';`n")
}

# Write LF line endings, UTF-8 without BOM, so the file matches rustfmt output
# and is byte-identical on every OS.
$text = ($sb.ToString() -replace "`r`n", "`n")
[System.IO.File]::WriteAllText($IconsRs, $text, (New-Object System.Text.UTF8Encoding($false)))

Write-Output "wrote $IconsRs ($($consts.Count) glyphs)"
Write-Output 'subset sizes:'
Get-ChildItem $OutDir -Filter '*.subset.ttf' | ForEach-Object { Write-Output ("  {0,-34} {1,8} bytes" -f $_.Name, $_.Length) }

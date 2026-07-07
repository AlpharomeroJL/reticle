# Measure a child process's wall time and peak working set (memory).
#
# Runs $Exe with $CliArgs and prints a single MEASURE line plus the child's stdout.
# PeakWorkingSet64 is a monotonic kernel high-water mark; it reads 0 once the
# process has exited, so this polls it while the process runs and keeps the max.
# For a run of more than a few hundred milliseconds (the scale runs this measures)
# the polled maximum is the true peak. stdout/stderr are drained asynchronously so
# a full pipe buffer cannot deadlock the poll loop.
#
# A .NET child process started with UseShellExecute=false inherits its working
# directory from [Environment]::CurrentDirectory, which PowerShell's Set-Location does
# NOT update. So a relative path in $CliArgs (for example scratch\gen.gds) would resolve
# against a stale directory and could read a different file than the one at the shell's
# current location. $WorkingDirectory (default: the caller's current location) is set on
# the child explicitly so relative arguments resolve where the caller expects. Prefer
# passing designs by absolute path regardless; this closes the ambiguity either way.
#
# Usage (call directly so the array binds; do not use powershell -File):
#   & scripts/measure-run.ps1 -Label import -Exe path\to\reticle.exe -CliArgs @("import","scratch\scale.gds")
param(
    [Parameter(Mandatory = $true)][string]$Label,
    [Parameter(Mandatory = $true)][string]$Exe,
    [string[]]$CliArgs = @(),
    [string]$WorkingDirectory = (Get-Location).Path
)

$ErrorActionPreference = 'Stop'

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.Arguments = ($CliArgs -join ' ')
$psi.WorkingDirectory = $WorkingDirectory
$psi.UseShellExecute = $false
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$proc = [System.Diagnostics.Process]::Start($psi)
$stdoutTask = $proc.StandardOutput.ReadToEndAsync()
$stderrTask = $proc.StandardError.ReadToEndAsync()

$peakBytes = 0
while (-not $proc.HasExited) {
    try {
        $proc.Refresh()
        if ($proc.PeakWorkingSet64 -gt $peakBytes) { $peakBytes = $proc.PeakWorkingSet64 }
    } catch { }
    Start-Sleep -Milliseconds 15
}
$sw.Stop()

$stdout = $stdoutTask.Result
$stderr = $stderrTask.Result
$peakMb = [math]::Round($peakBytes / 1MB, 1)
$wallMs = [math]::Round($sw.Elapsed.TotalMilliseconds, 0)
$exitCode = $proc.ExitCode

Write-Output "MEASURE|$Label|wall_ms=$wallMs|peak_mb=$peakMb|exit=$exitCode"
if ($stdout) { Write-Output $stdout.TrimEnd() }
if ($stderr) { Write-Output "--- stderr ---"; Write-Output $stderr.TrimEnd() }
# Note: no `exit` here, so callers can chain several measurements in one session.
# The exit code is reported on the MEASURE line above.

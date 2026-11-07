[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7 -or $env:OS -ne 'Windows_NT') {
    throw 'This scheduler contract requires Windows and PowerShell 7.'
}
# This test never loads run-all.ps1, discovers ShotKit, or accesses the desktop,
# clipboard, user profiles, or application settings. All processes are fixtures.
$pwsh = (Get-Process -Id $PID).Path
$root = Join-Path ([IO.Path]::GetTempPath()) ('shotkit-suite-budget-' + [Guid]::NewGuid().ToString('N'))
[void] (New-Item -ItemType Directory -Path $root)
$harnessProcess = $null
try {
    @'
param([string] $Root, [switch] $Leaf)
if ($Leaf) { Start-Sleep -Seconds 120; exit 0 }
$start = [Diagnostics.ProcessStartInfo]::new((Get-Process -Id $PID).Path)
$start.UseShellExecute = $false
foreach ($arg in @('-NoProfile', '-NonInteractive', '-File', $PSCommandPath, '-Leaf')) { [void] $start.ArgumentList.Add($arg) }
$child = [Diagnostics.Process]::Start($start)
@(
    @{ Id = $PID; StartTicks = (Get-Process -Id $PID).StartTime.ToUniversalTime().Ticks },
    @{ Id = $child.Id; StartTicks = $child.StartTime.ToUniversalTime().Ticks }
) | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Root 'owned-processes.json')
# No newline: proves incremental capture survives forced termination.
[Console]::Out.Write('BUDGET_STDOUT_PRESERVED')
[Console]::Error.Write('BUDGET_STDERR_PRESERVED')
Start-Sleep -Seconds 120
'@ | Set-Content -LiteralPath (Join-Path $root 'active.ps1')
    @'
param([string] $Root)
Set-Content -LiteralPath (Join-Path $Root 'must-not-run') -Value 'unexpected'
'@ | Set-Content -LiteralPath (Join-Path $root 'pending.ps1')
    @'
param([string] $Helper, [string] $Root)
$ErrorActionPreference = 'Stop'
. $Helper
$cases = @(
    @{ Name = 'active optional fixture'; Script = 'active.ps1'; Extra = @('-Root', $Root); TimeoutSeconds = 60; Toolbar = $true },
    @{ Name = 'pending required fixture'; Script = 'pending.ps1'; Extra = @('-Root', $Root); TimeoutSeconds = 60; Toolbar = $false },
    @{ Name = 'pending optional fixture'; Script = 'pending.ps1'; Extra = @('-Root', $Root); TimeoutSeconds = 60; Toolbar = $true }
)
Invoke-ShotKitSmokeSuite -Cases $cases -ScriptRoot $Root -Pwsh (Get-Process -Id $PID).Path `
    -Clock ([Diagnostics.Stopwatch]::StartNew()) -BudgetSeconds 8 -AllowToolbarFailure -IsCi
'@ | Set-Content -LiteralPath (Join-Path $root 'harness.ps1')
    $start = [Diagnostics.ProcessStartInfo]::new($pwsh)
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($arg in @('-NoProfile', '-NonInteractive', '-File', (Join-Path $root 'harness.ps1'), '-Helper', (Join-Path $PSScriptRoot 'suite-budget-contract.ps1'), '-Root', $root)) {
        [void] $start.ArgumentList.Add($arg)
    }
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $harnessProcess = [Diagnostics.Process]::Start($start)
    $stdout = $harnessProcess.StandardOutput.ReadToEndAsync()
    $stderr = $harnessProcess.StandardError.ReadToEndAsync()
    if (-not $harnessProcess.WaitForExit(30000)) { throw 'Harness did not stop within 30 seconds.' }
    if (-not [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]] @($stdout, $stderr), 5000)) { throw 'Harness output did not close.' }
    $output = $stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult()
    Write-Host $output.TrimEnd()
    if ($harnessProcess.ExitCode -eq 0) { throw 'Budget exhaustion returned success.' }
    if ($timer.Elapsed.TotalSeconds -ge 35) { throw 'Suite did not terminate within its bounded cleanup allowance.' }
    foreach ($expected in @(
        'BUDGET_STDOUT_PRESERVED', 'BUDGET_STDERR_PRESERVED',
        'FAILURE: active optional fixture: SUITE_BUDGET_EXHAUSTED',
        'FAILURE: pending required fixture: NOT RUN: SUITE_BUDGET_EXHAUSTED',
        'FAILURE: pending optional fixture: NOT RUN: SUITE_BUDGET_EXHAUSTED',
        'Smoke test summary: 3 scenarios, 3 failures.',
        'active optional fixture (non-blocking=False)',
        'pending required fixture (non-blocking=False)',
        'pending optional fixture (non-blocking=False)',
        '3 blocking smoke test(s) failed.'
    )) {
        if (-not $output.Contains($expected)) { throw "Missing scheduler evidence: $expected" }
    }
    if (Test-Path -LiteralPath (Join-Path $root 'must-not-run')) { throw 'A pending fixture was started after the deadline.' }
    $owned = @(Get-Content -Raw -LiteralPath (Join-Path $root 'owned-processes.json') | ConvertFrom-Json)
    if ($owned.Count -ne 2) { throw 'Fixture parent and child were not both recorded.' }
    foreach ($entry in $owned) {
        $process = Get-Process -Id $entry.Id -ErrorAction SilentlyContinue
        if ($process -and $process.StartTime.ToUniversalTime().Ticks -eq $entry.StartTicks) {
            if (-not $process.WaitForExit(2000)) { throw "Fixture process $($entry.Id) survived suite deadline cleanup." }
        }
    }
    Write-Host 'PASS: suite deadline stops owned parent/child, preserves stdout/stderr, lists every pending case, and exits nonzero even with optional toolbar failures.'
}
finally {
    if ($harnessProcess) {
        if (-not $harnessProcess.HasExited) { $harnessProcess.Kill($true); [void] $harnessProcess.WaitForExit(5000) }
        $harnessProcess.Dispose()
    }
    # Cleanup is restricted to identities recorded by this fixture (PID + start time).
    $identities = Join-Path $root 'owned-processes.json'
    if (Test-Path -LiteralPath $identities) {
        foreach ($entry in @(Get-Content -Raw -LiteralPath $identities | ConvertFrom-Json)) {
            $process = Get-Process -Id $entry.Id -ErrorAction SilentlyContinue
            if ($process -and $process.StartTime.ToUniversalTime().Ticks -eq $entry.StartTicks) {
                $process.Kill($true)
                [void] $process.WaitForExit(5000)
            }
        }
    }
    Remove-Item -LiteralPath $root -Recurse -Force
}

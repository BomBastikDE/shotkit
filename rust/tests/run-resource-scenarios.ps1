[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateSet('All','1','2','3','4','5')][string] $Scenario = 'All'
)
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7 -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'INTERACTIVE_WINDOWS_REQUIRED: PowerShell 7 and a dedicated interactive Windows test desktop are required.'
}
if ($env:GITHUB_ACTIONS -cne 'true') { throw 'EPHEMERAL_CI_REQUIRED: import-boundary scenarios are restricted to an ephemeral Windows runner.' }
$ExePath=(Resolve-Path -LiteralPath $ExePath).Path
$imageName=[IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) { throw 'PREEXISTING_SHOTKIT_PROCESS.' }
$cases=@(
    @{Id='1'; Script='windows-png-export-memory-smoke.ps1'; Extra=@('-ScrollHeight'); Scope='PARTIAL: 30000-row synthetic result export; actual maximum scroll acquisition remains unverified'},
    @{Id='2'; Script='windows-import-budget-smoke.ps1'; Extra=@('-Fixture','Boundary'); Scope='PARTIAL: exact 512-MiB RGBA PNG rejection; opening, redaction, blur and save at the boundary remain unverified'},
    @{Id='3'; Script='windows-import-budget-smoke.ps1'; Extra=@('-Fixture','JustOver'); Scope='exactly one 16384-pixel row over the 512-MiB raster boundary'},
    @{Id='4'; Script='windows-png-export-memory-smoke.ps1'; Extra=@('-DocumentCount','5'); Scope='five simultaneous 6000x4000 documents, one GUI Save As, source hashes and HWNDs preserved'},
    @{Id='5'; Script='windows-crop-history-confirmation-smoke.ps1'; Extra=@(); Scope='real 128-MiB history overflow, decline/accept, pixel fingerprints and Undo/Redo'},
    @{Id='5'; Script='windows-crop-history-confirmation-smoke.ps1'; Extra=@('-WithoutUndo'); Scope='history snapshot larger than 128 MiB, decline/accept and later Undo/Redo'}
)
$pwsh=(Get-Process -Id $PID).Path
foreach ($case in $cases) {
    if ($Scenario -ne 'All' -and $Scenario -ne $case.Id) { continue }
    Write-Host "RESOURCE_SCENARIO_START: id=$($case.Id) scope=$($case.Scope)"
    $start=[Diagnostics.ProcessStartInfo]::new($pwsh)
    $start.UseShellExecute=$false
    foreach ($arg in @('-NoProfile','-File',(Join-Path $PSScriptRoot $case.Script),'-ExePath',$ExePath)+$case.Extra) { [void]$start.ArgumentList.Add($arg) }
    $child=$null; $peaks=@{}; $clock=[Diagnostics.Stopwatch]::StartNew()
    $startUtc=[DateTime]::UtcNow
    try {
        $child=[Diagnostics.Process]::Start($start)
        while (-not $child.WaitForExit(50)) {
            if ($clock.Elapsed.TotalSeconds -gt 300) { throw "RESOURCE_SCENARIO_TIMEOUT: id=$($case.Id)" }
            foreach ($app in @(Get-Process -Name $imageName -ErrorAction SilentlyContinue)) {
                try {
                    # Observe only newly launched copies of the exact test executable.
                    if ($app.StartTime.ToUniversalTime() -lt $startUtc -or $app.MainModule.FileName -ine $ExePath) { continue }
                    $key="$($app.Id):$($app.StartTime.ToUniversalTime().Ticks)"
                    if (-not $peaks.ContainsKey($key)) { $peaks[$key]=@{WorkingSet=0L;Private=0L;Samples=0L} }
                    $stat=$peaks[$key]; $app.Refresh()
                    $stat.WorkingSet=[Math]::Max($stat.WorkingSet,$app.PeakWorkingSet64)
                    $stat.Private=[Math]::Max($stat.Private,$app.PrivateMemorySize64)
                    $stat.Samples++
                } catch [InvalidOperationException] { } # Process may exit between enumeration and read.
                finally { $app.Dispose() }
            }
        }
        if ($child.ExitCode -ne 0) { throw "RESOURCE_SCENARIO_FAILED: id=$($case.Id) exit=$($child.ExitCode)" }
        if ($peaks.Count -eq 0) { throw "NO_MEMORY_SAMPLES: id=$($case.Id)" }
        Write-Host "RESOURCE_SCENARIO_CHECKS_PASSED: id=$($case.Id) scope=$($case.Scope)"
    }
    finally {
        foreach ($key in $peaks.Keys) {
            $stat=$peaks[$key]
            Write-Host "RESOURCE_MEMORY: scenario=$($case.Id) process=$key samples=$($stat.Samples) peak_working_set_bytes=$($stat.WorkingSet) max_sampled_private_bytes=$($stat.Private)"
        }
        if ($null -ne $child) {
            if (-not $child.HasExited) { $child.Kill($true); [void]$child.WaitForExit(5000) }
            $child.Dispose()
        }
    }
}
Write-Host 'Memory observations have no pass/fail threshold; private-byte peaks between samples can be missed. Existing scenario safety guards remain in force.'
Write-Host 'ISSUE_65_ACCEPTANCE_INCOMPLETE: scenarios 1 and 2 have explicit coverage gaps. Passing these automated checks does not close #65.'

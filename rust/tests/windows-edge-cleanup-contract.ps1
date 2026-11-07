[CmdletBinding()]
param([Parameter(Mandatory = $true)][string] $ExePath)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7 -or $env:OS -ne 'Windows_NT') { throw 'PowerShell 7 on Windows is required.' }
. (Join-Path $PSScriptRoot 'edge-profile-cleanup.ps1')

$sandbox = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-EdgeCleanup-' + [Guid]::NewGuid().ToString('N'))
$transient = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Browser-Scroll-' + [Guid]::NewGuid().ToString('N'))
$persistent = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Browser-Scroll-' + [Guid]::NewGuid().ToString('N'))
$sentinel = Join-Path $sandbox 'outside.txt'
$script:heldFile = $null
$junction = $null
$testFailure = $null
$cleanupFailures = [Collections.Generic.List[string]]::new()
try {
    New-Item -ItemType Directory -Path $sandbox, $transient, $persistent -Force | Out-Null
    [IO.File]::WriteAllText($sentinel, 'keep')
    $outsideRejected = $false
    try { Remove-EdgeTestTree -Path $sandbox -MaxAttempts 1 }
    catch { $outsideRejected = $true }
    if (-not $outsideRejected -or [IO.File]::ReadAllText($sentinel) -cne 'keep') { throw 'Cleanup did not reject an unrelated temporary directory.' }
    $transientFile = Join-Path $transient 'cache.dat'
    [IO.File]::WriteAllText($transientFile, 'fixture')
    $script:heldFile = [IO.File]::Open($transientFile, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    $script:retryCount = 0
    Remove-EdgeTestTree -Path $transient -MaxAttempts 3 -DelayMilliseconds 0 -OnRetry {
        param($attempt)
        $script:retryCount++
        $script:heldFile.Dispose()
        $script:heldFile = $null
    }
    if ($script:retryCount -ne 1 -or (Test-Path -LiteralPath $transient)) { throw 'Transient lock was not retried and removed as expected.' }

    $persistentFile = Join-Path $persistent 'cache.dat'
    [IO.File]::WriteAllText($persistentFile, 'fixture')
    $script:heldFile = [IO.File]::Open($persistentFile, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    $script:retryCount = 0
    $failed = $false
    try {
        Remove-EdgeTestTree -Path $persistent -MaxAttempts 3 -DelayMilliseconds 0 -OnRetry { param($attempt) $script:retryCount++ }
    }
    catch { $failed = $true }
    if (-not $failed -or $script:retryCount -ne 2 -or -not (Test-Path -LiteralPath $persistentFile)) { throw 'Persistent lock did not remain a cleanup failure.' }
    if ([IO.File]::ReadAllText($sentinel) -cne 'keep') { throw 'File outside the test profile was changed.' }
    $script:heldFile.Dispose()
    $script:heldFile = $null
    $junction = Join-Path $persistent 'outside-link'
    New-Item -ItemType Junction -Path $junction -Target $sandbox | Out-Null
    $linkRejected = $false
    try { Remove-EdgeTestTree -Path $persistent -MaxAttempts 1 }
    catch { $linkRejected = $true }
    if (-not $linkRejected -or [IO.File]::ReadAllText($sentinel) -cne 'keep') { throw 'Cleanup did not reject a junction to an outside directory.' }
    [IO.Directory]::Delete($junction, $false)
    if ([IO.File]::ReadAllText($sentinel) -cne 'keep') { throw 'Removing the junction changed its target.' }
    Remove-EdgeTestTree -Path $persistent -MaxAttempts 1
}
catch { $testFailure = $_ }
finally {
    try { if ($null -ne $script:heldFile) { $script:heldFile.Dispose() } }
    catch { [void]$cleanupFailures.Add("Could not close fixture handle: $($_.Exception.Message)") }
    try {
        if ($junction -and (Test-Path -LiteralPath $junction)) {
            $entry = Get-Item -LiteralPath $junction -Force -ErrorAction Stop
            if (-not ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Fixture junction was replaced by a non-reparse entry.' }
            [IO.Directory]::Delete($junction, $false)
        }
    }
    catch { [void]$cleanupFailures.Add("Could not unlink fixture junction: $($_.Exception.Message)") }
    foreach ($ownRoot in @($transient, $persistent)) {
        try { if (Test-Path -LiteralPath $ownRoot) { Remove-EdgeTestTree -Path $ownRoot -MaxAttempts 1 } }
        catch { [void]$cleanupFailures.Add("Could not remove fixture profile: $($_.Exception.Message)") }
    }
    try { if (Test-Path -LiteralPath $sentinel) { [IO.File]::Delete($sentinel) } }
    catch { [void]$cleanupFailures.Add("Could not remove fixture sentinel: $($_.Exception.Message)") }
    try { if (Test-Path -LiteralPath $sandbox) { [IO.Directory]::Delete($sandbox, $false) } }
    catch { [void]$cleanupFailures.Add("Could not remove fixture sandbox: $($_.Exception.Message)") }
}
if ($null -ne $testFailure -or $cleanupFailures.Count -gt 0) {
    throw "Fixture failure: $($testFailure.Exception.Message); cleanup failures: $($cleanupFailures -join '; ')"
}
Write-Output 'PASS: transient profile lock retried; persistent lock and outside junction rejected; outside sentinel remained unchanged.'

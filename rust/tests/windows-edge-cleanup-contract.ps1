[CmdletBinding()]
param([Parameter(Mandatory = $true)][string] $ExePath)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7 -or $env:OS -ne 'Windows_NT') { throw 'PowerShell 7 on Windows is required.' }
. (Join-Path $PSScriptRoot 'edge-profile-cleanup.ps1')

# Native Windows parser coverage, including quoting/argument-boundary traps.
$profileArgumentCases = @(
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile'; Profile='C:\owned-test-profile'; Expected=$true },
    @{ Line='"C:\Program Files\Edge\msedge.exe" --user-data-dir="C:\owned profile" --no-first-run'; Profile='C:\owned profile'; Expected=$true },
    @{ Line='msedge.exe "--user-data-dir=C:\owned profile"'; Profile='C:\owned profile'; Expected=$true },
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile-suffix'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\prefix-C:\owned-test-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile\child'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --other=C:\owned-test-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --other="--user-data-dir=C:\owned-test-profile"'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe "--other=note --user-data-dir=C:\owned-test-profile"'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\foreign-profile --other=C:\owned-test-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile --user-data-dir=C:\foreign-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile --user-data-dir=C:\owned-test-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile -user-data-dir=C:\foreign-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile /user-data-dir=C:\foreign-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir C:\owned-test-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe -- --user-data-dir=C:\owned-test-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\OWNED-TEST-PROFILE'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir="C:\owned profile suffix"'; Profile='C:\owned profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile\..\foreign'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile " --user-data-dir=C:\foreign-profile "'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --single-argument --user-data-dir=C:\owned-test-profile'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line='msedge.exe --user-data-dir=C:\owned-test-profile --single-argument ignored'; Profile='C:\owned-test-profile'; Expected=$false },
    @{ Line=''; Profile='C:\owned-test-profile'; Expected=$false }
)
foreach ($argumentCase in $profileArgumentCases) {
    $actual = [ShotKitEdgeCleanup.CommandLine]::HasExactProfile($argumentCase.Line, $argumentCase.Profile)
    if ($actual -ne $argumentCase.Expected) { throw "Unexpected exact profile argument result: '$($argumentCase.Line)'." }
}

# Deterministic process lifecycle tests: no real processes are opened or killed.
function Test-EdgeDiscoveryRaces {
    $edgeProfile = 'C:\owned-test-profile'
    $created = [DateTime]::UtcNow
    $candidate = [pscustomobject]@{ ProcessId=123; CreationDate=$created; Name='msedge.exe'; CommandLine='msedge.exe --user-data-dir=C:\owned-test-profile' }
    function Get-CimInstance {
        [CmdletBinding()]
        param([string] $ClassName, [string] $Filter)
        if ($case -eq 'cim-error') { throw [UnauthorizedAccessException]::new('CIM failure') }
        if ($Filter -eq "Name='msedge.exe'") { return $candidate }
        if ($case -ne 'access-denied' -and $null -ne $fake -and -not $fake.Opened) { throw 'Native handle was not opened before PID revalidation.' }
        if ($case -eq 'gone-after-open') { return $null }
        if ($case -eq 'reused-pid') {
            return [pscustomobject]@{ ProcessId=123; CreationDate=$created.AddSeconds(1); Name='msedge.exe'; CommandLine=$candidate.CommandLine }
        }
        if ($case -eq 'foreign-profile') {
            return [pscustomobject]@{ ProcessId=123; CreationDate=$created; Name='msedge.exe'; CommandLine='msedge.exe --user-data-dir=C:\foreign-profile' }
        }
        if ($case -in @('profile-prefix-after-open', 'other-argument-after-open')) {
            $line = if ($case -eq 'profile-prefix-after-open') { 'msedge.exe --user-data-dir=C:\owned-test-profile-suffix' } else { 'msedge.exe --other=C:\owned-test-profile' }
            return [pscustomobject]@{ ProcessId=123; CreationDate=$created; Name='msedge.exe'; CommandLine=$line }
        }
        if ($case -eq 'foreign-name') {
            return [pscustomobject]@{ ProcessId=123; CreationDate=$created; Name='other.exe'; CommandLine=$candidate.CommandLine }
        }
        return $candidate # deliberately stale even when GetProcessById reports gone
    }
    foreach ($argumentCase in @($profileArgumentCases | Where-Object { -not $_.Expected })) {
        $edgeOwnedHandles = [Collections.Generic.List[object]]::new()
        $case = 'initial-foreign'
        $candidate.CommandLine = $argumentCase.Line
        $snapshots = @(Get-ProfileEdgeProcesses -OpenProcess { throw 'Foreign argument must not open any process.' } -IncludeUnverifiedSnapshots)
        if ($snapshots.Count -ne 0 -or $edgeOwnedHandles.Count -ne 0) { throw 'Foreign argument established process ownership.' }
    }
    $candidate.CommandLine = 'msedge.exe --user-data-dir=C:\owned-test-profile'
    foreach ($case in @('vanished', 'access-denied', 'cim-error', 'exited-before-handle', 'exited-after-open', 'gone-after-open', 'reused-pid', 'foreign-profile', 'foreign-name', 'profile-prefix-after-open', 'other-argument-after-open', 'unexpected-name', 'valid')) {
        $edgeOwnedHandles = [Collections.Generic.List[object]]::new()
        $fake = [pscustomobject]@{ HasExited=($case -eq 'exited-after-open'); ProcessName='msedge'; Disposed=$false; Opened=$false }
        if ($case -eq 'unexpected-name') { $fake.ProcessName = 'other' }
        $fake | Add-Member -MemberType ScriptProperty -Name Handle -Value {
            if ($case -eq 'exited-before-handle') { $this.HasExited = $true; throw [InvalidOperationException]::new('Process exited.') }
            $this.Opened = $true
            return [IntPtr]123
        }
        $fake | Add-Member -MemberType ScriptMethod -Name Dispose -Value { $this.Disposed = $true }
        $open = {
            param($ProcessId)
            if ($ProcessId -ne 123) { throw 'Unexpected process target.' }
            if ($case -eq 'vanished') { throw [ArgumentException]::new('Process is no longer running.') }
            if ($case -eq 'access-denied') { throw [UnauthorizedAccessException]::new('Access denied.') }
            return $fake
        }
        $failed = $false
        $snapshots = @()
        try { $snapshots = @(Get-ProfileEdgeProcesses -OpenProcess $open) }
        catch { $failed = $true }
        $mustFail = $case -in @('access-denied', 'cim-error', 'unexpected-name')
        if ($failed -ne $mustFail) { throw "Unexpected discovery error result in case '$case': failed=$failed." }
        $expectedHandles = if ($case -eq 'valid') { 1 } else { 0 }
        if ($edgeOwnedHandles.Count -ne $expectedHandles) { throw "Unverified kill target retained in case '$case'." }
        if ($case -eq 'vanished') {
            if ($snapshots.Count -ne 0) { throw 'Stale CIM row must not target window inspection or focus.' }
            $unverified = @(Get-ProfileEdgeProcesses -OpenProcess $open -IncludeUnverifiedSnapshots)
            if ($unverified.Count -ne 1 -or $edgeOwnedHandles.Count -ne 0) { throw 'Stale CIM row must remain visible only for mandatory final cleanup verification.' }
        }
        if ($case -in @('exited-before-handle', 'exited-after-open', 'gone-after-open', 'reused-pid', 'foreign-profile', 'foreign-name', 'profile-prefix-after-open', 'other-argument-after-open', 'unexpected-name') -and -not $fake.Disposed) {
            throw "Rejected process handle leaked in case '$case'."
        }
        if ($case -eq 'valid') {
            [void](Get-ProfileEdgeProcesses -OpenProcess { throw 'Known identity must not be reopened.' })
            if ($edgeOwnedHandles.Count -ne 1 -or $fake.Disposed) { throw 'Verified process handle was not retained exactly once.' }
            $fake.Dispose()
        }
    }
}
Test-EdgeDiscoveryRaces

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
Write-Output 'PASS: native Windows command-line parsing accepted exact quoted profile arguments and rejected substring/duplicate/other-argument traps; process discovery rejected stale, reused and foreign identities, propagated real access/CIM errors; transient profile lock retried; persistent lock and outside junction rejected; outside sentinel remained unchanged.'

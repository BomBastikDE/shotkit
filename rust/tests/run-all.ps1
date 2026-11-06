[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [string] $ScreenshotPath = 'target/shotkit-toolbar-test.png',
    [switch] $AllowToolbarFailure,
    [switch] $AllowLocalInteractive
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if ($env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'NO_INTERACTIVE_DESKTOP: an interactive Windows desktop is required.'
}
$isCi = $env:GITHUB_ACTIONS -eq 'true'
if (-not $isCi -and -not $AllowLocalInteractive) {
    throw 'LOCAL_OPT_IN_REQUIRED: use -AllowLocalInteractive only in a dedicated test session; tests control mouse and keyboard and replace clipboard contents.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw 'ShotKit EXE not found.' }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$ScreenshotPath = [IO.Path]::GetFullPath($ScreenshotPath)
$PreviewScreenshotPath = Join-Path (Split-Path -Parent $ScreenshotPath) ('shotkit-held-preview-' + [Guid]::NewGuid().ToString('N') + '.png')
# A running instance could receive input or single-instance messages intended
# for the test. Refuse the run rather than interact with an existing process.
$processNames = @([IO.Path]::GetFileNameWithoutExtension($ExePath), 'ShotKit', 'shotkit-rust-prototype') | Select-Object -Unique
foreach ($name in $processNames) {
    if (@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count -gt 0) {
        throw "PREEXISTING_PROCESS: '$name' is already running."
    }
}
$pwsh = (Get-Process -Id $PID).Path
$cases = @(
    @{ Name = 'Interactive Windows About dialog smoke test'; Script = 'windows-about-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows delayed capture smoke test'; Script = 'windows-delayed-capture-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows storage folder open smoke test'; Script = 'windows-storage-folder-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows toolbar and icon visual smoke test'; Script = 'windows-toolbar-visual-smoke.ps1'; Extra = @('-ScreenshotPath', $ScreenshotPath); TimeoutSeconds = 300; Toolbar = $true },
    @{ Name = 'Required interactive Windows responsive toolbar smoke test'; Script = 'windows-toolbar-visual-smoke.ps1'; Extra = @('-ScreenshotPath', $ScreenshotPath, '-ResponsiveOnly'); TimeoutSeconds = 180; Toolbar = $false },
    @{ Name = 'Required interactive Windows editor popup smoke test'; Script = 'windows-toolbar-visual-smoke.ps1'; Extra = @('-ScreenshotPath', $ScreenshotPath, '-PopupOnly'); TimeoutSeconds = 180; Toolbar = $false },
    @{ Name = 'Interactive Windows step number visibility smoke test'; Script = 'windows-step-visibility-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows clipboard alpha round-trip smoke test'; Script = 'windows-clipboard-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows clipboard history enabled smoke test'; Script = 'windows-clipboard-smoke.ps1'; Extra = @('-AllowHistory'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows capture smoke test'; Script = 'windows-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows capture output settings matrix smoke test'; Script = 'windows-output-matrix-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows capture output recovery smoke test'; Script = 'windows-output-failure-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows magnifier smoke test'; Script = 'windows-magnifier-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows active-window capture parity smoke test'; Script = 'windows-active-window-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows hotkey conflict smoke test'; Script = 'windows-hotkey-conflict-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows F10 hotkey conflict smoke test'; Script = 'windows-f10-conflict-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows zoom input smoke test'; Script = 'windows-zoom-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows Ctrl+0 exact 10x10 image-pixel smoke test'; Script = 'windows-zoom-10x10-smoke.ps1'; Extra = @(); TimeoutSeconds = 180; Toolbar = $false },
    @{ Name = 'Interactive Windows keyboard selection smoke test'; Script = 'windows-keyboard-selection-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows last-region repeat smoke test'; Script = 'windows-last-region-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows last-region clipboard repeat smoke test'; Script = 'windows-last-region-clipboard-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows scroll capture smoke test'; Script = 'windows-scroll-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows scroll diagnostics smoke test'; Script = 'windows-scroll-diagnostics-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows scroll cancellation smoke test'; Script = 'windows-scroll-abort-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows scroll mouse cancellation smoke test'; Script = 'windows-scroll-abort-smoke.ps1'; Extra = @('-MouseAbort'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows horizontal scroll smoke test'; Script = 'windows-horizontal-scroll-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows quit message security smoke test'; Script = 'windows-quit-security-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows redaction export smoke test'; Script = 'windows-redaction-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows soft blur export smoke test'; Script = 'windows-gaussian-blur-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows canvas resize smoke test'; Script = 'windows-canvas-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows stamp editor smoke test'; Script = 'windows-stamp-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows single-instance smoke test'; Script = 'windows-instance-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows hover suggestion smoke test'; Script = 'windows-hover-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Edge browser hover smoke test'; Script = 'windows-browser-hover-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Chrome browser hover smoke test'; Script = 'windows-browser-hover-smoke.ps1'; Extra = @('-Browser', 'Chrome'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Edge browser hover keyboard adjustment smoke test'; Script = 'windows-browser-hover-smoke.ps1'; Extra = @('-KeyboardAdjustment'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Edge browser unchanged hover Enter acceptance smoke test'; Script = 'windows-browser-hover-smoke.ps1'; Extra = @('-EnterAccept'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows editor Shift constraints and pixel smoke test'; Script = 'windows-editor-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows editor pixelation and undo smoke test'; Script = 'windows-editor-pixelate-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows frame and shadow smoke test'; Script = 'windows-frame-shadow-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows editor selected-handle parity smoke test'; Script = 'windows-editor-handles-smoke.ps1'; Extra = @('-PreviewScreenshotPath', $PreviewScreenshotPath); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows editor text edit smoke test'; Script = 'windows-editor-text-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows original image save protection smoke test'; Script = 'windows-save-original-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows saved-state undo and redo close smoke test'; Script = 'windows-save-undo-redo-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows image formats smoke test'; Script = 'windows-formats-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows rejected oversized image import preservation smoke test'; Script = 'windows-large-image-import-smoke.ps1'; Extra = @(); TimeoutSeconds = 180; Toolbar = $false },
    @{ Name = 'Interactive Edge browser scroll smoke test'; Script = 'windows-browser-scroll-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Edge browser scroll smoke test with fixed header and footer'; Script = 'windows-browser-scroll-smoke.ps1'; Extra = @('-FixedHeaderFooter'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows multi-editor lifecycle smoke test'; Script = 'windows-multi-editor-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false }
)
$failures = [Collections.Generic.List[object]]::new()
foreach ($case in $cases) {
    Write-Host ("--- {0} ---" -f $case.Name)
    $path = Join-Path $PSScriptRoot $case.Script
    $process = $null
    try {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Test script missing: $($case.Script)" }
        $start = [Diagnostics.ProcessStartInfo]::new()
        $start.FileName = $pwsh
        $start.UseShellExecute = $false
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        $start.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
        $start.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
        foreach ($arg in @('-NoProfile', '-NonInteractive', '-File', $path, '-ExePath', $ExePath) + $case.Extra) {
            [void] $start.ArgumentList.Add([string]$arg)
        }
        if (-not $isCi -and $AllowLocalInteractive) { $start.Environment['SHOTKIT_LOCAL_SMOKE_TEST'] = '1' }
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $start
        if (-not $process.Start()) { throw 'Could not start the test process.' }
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit([int]$case.TimeoutSeconds * 1000)) {
            if (-not $process.HasExited) { $process.Kill($true) }
            [void] $process.WaitForExit(5000)
            if ($stdout.IsCompleted) { $partial = $stdout.GetAwaiter().GetResult(); if ($partial) { Write-Host ($partial.TrimEnd()) } }
            if ($stderr.IsCompleted) { $partial = $stderr.GetAwaiter().GetResult(); if ($partial) { Write-Host ($partial.TrimEnd()) } }
            throw "TIMEOUT: test exceeded $($case.TimeoutSeconds) seconds; its process tree was stopped."
        }
        $outText = $stdout.GetAwaiter().GetResult()
        $errText = $stderr.GetAwaiter().GetResult()
        if ($outText) { Write-Host ($outText.TrimEnd()) }
        if ($errText) { Write-Host ($errText.TrimEnd()) }
        if ($process.ExitCode -ne 0) { throw "Test process exited with code $($process.ExitCode)." }
        Write-Host ("PASS: {0}" -f $case.Name)
    }
    catch {
        $optional = [bool]($case.Toolbar -and $AllowToolbarFailure)
        $failures.Add([pscustomobject]@{ Name = $case.Name; Message = $_.Exception.Message; NonBlocking = $optional })
        $kind = if ($optional) { 'NON-BLOCKING FAILURE' } else { 'FAILURE' }
        Write-Host ("{0}: {1}: {2}" -f $kind, $case.Name, $_.Exception.Message)
        if ($optional -and $isCi) { Write-Host ("::warning::Non-blocking toolbar smoke failed: {0}" -f $_.Exception.Message) }
    }
    finally { if ($process) { $process.Dispose() } }
}
Write-Host ("Smoke test summary: {0} scenarios, {1} failures." -f $cases.Count, $failures.Count)
foreach ($failure in $failures) {
    Write-Host (" - {0} (non-blocking={1}): {2}" -f $failure.Name, $failure.NonBlocking, $failure.Message)
}
$blocking = @($failures | Where-Object { -not $_.NonBlocking })
if ($blocking.Count -gt 0) { throw ("{0} blocking smoke test(s) failed." -f $blocking.Count) }

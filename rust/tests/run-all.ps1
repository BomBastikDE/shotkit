[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [string] $ScreenshotPath = 'target/shotkit-toolbar-test.png',
    [switch] $AllowToolbarFailure,
    [switch] $AllowLocalInteractive
)

$ErrorActionPreference = 'Stop'
# Include preflight and helper loading in the monotonic CI deadline. Leave three
# minutes before the workflow's 50-minute step cap for bounded cleanup/summary.
$suiteClock = [Diagnostics.Stopwatch]::StartNew()
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if ($env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'NO_INTERACTIVE_DESKTOP: an interactive Windows desktop is required.'
}
$isCi = $env:GITHUB_ACTIONS -eq 'true'
if (-not $isCi -and -not $AllowLocalInteractive) {
    throw 'LOCAL_OPT_IN_REQUIRED: use -AllowLocalInteractive only in a dedicated test session; tests control mouse and keyboard and replace clipboard contents.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw 'NOT RUN: ShotKit EXE not found; the interactive smoke suite could not start.' }
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
    @{ Name = 'Interactive Windows first-start notice smoke test'; Script = 'windows-first-start-smoke.ps1'; Extra = @(); TimeoutSeconds = 180; Toolbar = $false },
    @{ Name = 'Interactive Windows pinned image smoke test'; Script = 'windows-pin-smoke.ps1'; Extra = @(); TimeoutSeconds = 180; Toolbar = $false },
    @{ Name = 'Windows shared live-log reader contract'; Script = 'windows-shared-log-reader-contract.ps1'; Extra = @(); TimeoutSeconds = 60; Toolbar = $false },
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
    @{ Name = 'Interactive Windows clamped cursor scroll capture smoke test'; Script = 'windows-scroll-smoke.ps1'; Extra = @('-CursorClip', 'Inside'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows outside cursor clip partial capture smoke test'; Script = 'windows-scroll-smoke.ps1'; Extra = @('-CursorClip', 'Outside'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows scroll diagnostics smoke test'; Script = 'windows-scroll-diagnostics-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows scroll cancellation smoke test'; Script = 'windows-scroll-abort-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows scroll mouse cancellation smoke test'; Script = 'windows-scroll-abort-smoke.ps1'; Extra = @('-MouseAbort'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows horizontal scroll smoke test'; Script = 'windows-horizontal-scroll-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows quit message security smoke test'; Script = 'windows-quit-security-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows redaction export smoke test'; Script = 'windows-redaction-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows soft blur export smoke test'; Script = 'windows-gaussian-blur-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows canvas resize smoke test'; Script = 'windows-canvas-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows stamp editor smoke test'; Script = 'windows-stamp-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows callout creation and history smoke test'; Script = 'windows-callout-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows form style editor smoke test'; Script = 'windows-form-style-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows spotlight and image magnifier effects smoke test'; Script = 'windows-effects-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
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
    @{ Name = 'Interactive Windows import budget preservation smoke test'; Script = 'windows-import-budget-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows crop history eviction confirmation smoke test'; Script = 'windows-crop-history-confirmation-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows crop without Undo confirmation smoke test'; Script = 'windows-crop-history-confirmation-smoke.ps1'; Extra = @('-WithoutUndo'); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows large image memory measurement'; Script = 'windows-large-image-memory-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows two large documents memory measurement'; Script = 'windows-large-image-memory-smoke.ps1'; Extra = @('-DocumentCount', '2'); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows RGBA PNG memory measurement'; Script = 'windows-large-image-memory-smoke.ps1'; Extra = @('-FixtureFormat', 'RGBA-PNG'); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows PNG export pixels and memory measurement'; Script = 'windows-png-export-memory-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Windows settings recovery filesystem contract'; Script = 'windows-settings-recovery-contract.ps1'; Extra = @(); TimeoutSeconds = 60; Toolbar = $false },
    @{ Name = 'Interactive Windows mapped network drive warning'; Script = 'windows-mapped-drive-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Interactive Windows temporary-file recovery on a network save folder'; Script = 'windows-temp-recovery-network-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false },
    @{ Name = 'Windows Edge profile cleanup contract'; Script = 'windows-edge-cleanup-contract.ps1'; Extra = @(); TimeoutSeconds = 120; Toolbar = $false },
    @{ Name = 'Interactive Edge browser scroll smoke test'; Script = 'windows-browser-scroll-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Edge browser scroll smoke test with fixed header and footer'; Script = 'windows-browser-scroll-smoke.ps1'; Extra = @('-FixedHeaderFooter'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Edge browser scroll smoke test stopping halfway'; Script = 'windows-browser-scroll-smoke.ps1'; Extra = @('-StopHalfway'); TimeoutSeconds = 600; Toolbar = $false },
    @{ Name = 'Interactive Windows multi-editor lifecycle smoke test'; Script = 'windows-multi-editor-smoke.ps1'; Extra = @(); TimeoutSeconds = 600; Toolbar = $false }
)
# The reset fixture drives the real tray menu and deliberately has no local
# owner-session opt-in. Existing local suite behavior stays unchanged.
if ($isCi) {
    $cases = @(@{ Name = 'Interactive Windows explicit settings reset UI smoke test'; Script = 'windows-settings-reset-smoke.ps1'; Extra = @(); TimeoutSeconds = 300; Toolbar = $false }) + $cases
}
# There is deliberately no production CLI/environment override for the CI budget.
. (Join-Path $PSScriptRoot 'suite-budget-contract.ps1')
$budgetSeconds = if ($isCi) { 47 * 60 } else { 0 }
Invoke-ShotKitSmokeSuite -Cases $cases -ScriptRoot $PSScriptRoot -Pwsh $pwsh `
    -CommonArguments @('-ExePath', $ExePath) -Clock $suiteClock -BudgetSeconds $budgetSeconds `
    -AllowToolbarFailure:$AllowToolbarFailure -IsCi:$isCi `
    -LocalInteractive:([bool](-not $isCi -and $AllowLocalInteractive))

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [Parameter(Mandatory = $true)][ValidateSet('output-matrix', 'edge-scroll', 'edge-scroll-fixed-header-footer')][string] $Scenario,
    [ValidatePattern('^(?:[1-9]|1[0-9]|20)$')][string] $RepeatCount = '10'
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7 -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'INTERACTIVE_WINDOWS_REQUIRED: scenario repeats require PowerShell 7 on an interactive Windows desktop.'
}
if ($env:GITHUB_ACTIONS -cne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') {
    throw 'LOCAL_OPT_IN_REQUIRED: use run-all.ps1 -AllowLocalInteractive in a dedicated test session.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$extra = @()
$scriptPath = switch ($Scenario) {
    'output-matrix' { Join-Path $PSScriptRoot 'windows-output-matrix-smoke.ps1' }
    'edge-scroll' { Join-Path $PSScriptRoot 'windows-browser-scroll-smoke.ps1' }
    'edge-scroll-fixed-header-footer' {
        $extra = @('-FixedHeaderFooter')
        Join-Path $PSScriptRoot 'windows-browser-scroll-smoke.ps1'
    }
}
$pwsh = (Get-Process -Id $PID).Path
$repeatTotal = [int]$RepeatCount
for ($iteration = 1; $iteration -le $repeatTotal; $iteration++) {
    Write-Output "SCENARIO_REPEAT_START: scenario=$Scenario iteration=$iteration/$repeatTotal"
    & $pwsh -NoProfile -File $scriptPath -ExePath $ExePath @extra
    if ($LASTEXITCODE -ne 0) { throw "SCENARIO_REPEAT_FAILED: scenario=$Scenario iteration=$iteration exit=$LASTEXITCODE" }
    Write-Output "SCENARIO_REPEAT_PASS: scenario=$Scenario iteration=$iteration/$repeatTotal"
}
Write-Output "SCENARIO_REPEAT_SUMMARY: scenario=$Scenario passed=$repeatTotal/$repeatTotal"

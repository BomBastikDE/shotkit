[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateSet('Edge', 'Chrome')]
    [string] $Browser = 'Edge',

    [switch] $KeyboardAdjustment,

    [switch] $EnterAccept,

    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 30,

    [ValidateRange(15, 180)]
    [int] $CaptureTimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
if ($KeyboardAdjustment -and $EnterAccept) { throw 'INVALID_OPTIONS: -KeyboardAdjustment and -EnterAccept are separate scenarios.' }
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') {
    throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'
}
if (-not [Environment]::UserInteractive) {
    throw 'NO_INTERACTIVE_DESKTOP: the GitHub runner is not interactive.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "ShotKit EXE not found: $ExePath"
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

if (-not ('ShotKitBrowserHoverSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace ShotKitBrowserHoverSmoke {
    public sealed class WindowInfo {
        public IntPtr Handle;
        public string ClassName;
        public string Title;
        public bool Visible;
    }
    public sealed class ButtonInfo {
        public IntPtr Handle;
        public int ControlId;
        public string Text;
    }
    public static class Native {
        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);
        private delegate bool EnumChildProc(IntPtr hwnd, IntPtr lParam);
        [DllImport("user32.dll", SetLastError=true)]
        private static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll", SetLastError=true)]
        private static extern bool CloseDesktop(IntPtr desktop);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder info, uint length, out uint needed);
        [DllImport("user32.dll")]
        private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")]
        private static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr lParam);
        [DllImport("user32.dll")]
        private static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern int SendMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")]
        private static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")]
        public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")]
        private static extern IntPtr GetForegroundWindow();
        public static IntPtr ForegroundWindowHandle() { return GetForegroundWindow(); }
        public static uint ProcessIdForWindow(IntPtr hwnd) {
            uint processId;
            GetWindowThreadProcessId(hwnd, out processId);
            return processId;
        }
        [StructLayout(LayoutKind.Sequential)]
        private struct ScreenPoint { public int X; public int Y; }
        [DllImport("user32.dll", SetLastError=true)]
        private static extern bool GetCursorPos(out ScreenPoint point);
        public static bool CursorPosition(out int x, out int y) {
            ScreenPoint point;
            bool ok = GetCursorPos(out point);
            x = point.X;
            y = point.Y;
            return ok;
        }
        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")]
        public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
        [DllImport("user32.dll")]
        public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);
        [DllImport("user32.dll")]
        public static extern int GetSystemMetrics(int index);
        public static string InputDesktopName() {
            IntPtr desktop = OpenInputDesktop(0, false, 0x0001);
            if (desktop == IntPtr.Zero) return null;
            try {
                StringBuilder name = new StringBuilder(256);
                uint needed;
                if (!GetUserObjectInformationW(desktop, 2, name, (uint)(name.Capacity * 2), out needed)) return null;
                return name.ToString();
            } finally { CloseDesktop(desktop); }
        }
        private static string Text(IntPtr hwnd) {
            StringBuilder text = new StringBuilder(512);
            GetWindowTextW(hwnd, text, text.Capacity);
            return text.ToString();
        }
        public static WindowInfo[] WindowsForProcess(int processId) {
            List<WindowInfo> windows = new List<WindowInfo>();
            EnumWindows(delegate(IntPtr hwnd, IntPtr ignored) {
                uint owner;
                GetWindowThreadProcessId(hwnd, out owner);
                if (owner != (uint)processId) return true;
                StringBuilder cls = new StringBuilder(256);
                GetClassNameW(hwnd, cls, cls.Capacity);
                windows.Add(new WindowInfo { Handle=hwnd, ClassName=cls.ToString(), Title=Text(hwnd), Visible=IsWindowVisible(hwnd) });
                return true;
            }, IntPtr.Zero);
            return windows.ToArray();
        }
        public static ButtonInfo[] VisibleButtons(IntPtr dialog) {
            List<ButtonInfo> buttons = new List<ButtonInfo>();
            EnumChildProc callback = delegate(IntPtr child, IntPtr ignored) {
                if (!IsWindowVisible(child)) return true;
                StringBuilder cls = new StringBuilder(128);
                GetClassNameW(child, cls, cls.Capacity);
                if (!String.Equals(cls.ToString(), "Button", StringComparison.OrdinalIgnoreCase)) return true;
                StringBuilder text = new StringBuilder(256);
                GetWindowTextW(child, text, text.Capacity);
                buttons.Add(new ButtonInfo { Handle=child, ControlId=GetDlgCtrlID(child), Text=text.ToString() });
                return true;
            };
            EnumChildWindows(dialog, callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            return buttons.ToArray();
        }
        public static void ClickButton(IntPtr button) {
            const uint BM_CLICK = 0x00F5;
            if (button == IntPtr.Zero) throw new ArgumentException("Dialog button handle is null.");
            SendMessageW(button, BM_CLICK, IntPtr.Zero, IntPtr.Zero);
        }
    }
}
'@
}

$inputDesktop = [ShotKitBrowserHoverSmoke.Native]::InputDesktopName()
if ([string]::IsNullOrWhiteSpace($inputDesktop)) {
    throw 'NO_INTERACTIVE_DESKTOP: Windows could not open the input desktop.'
}
try {
    Add-Type -AssemblyName System.Drawing
}
catch {
    throw "IMAGE_RUNTIME_UNAVAILABLE: System.Drawing could not be loaded: $($_.Exception.Message)"
}
$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
if (-not (Test-Path -LiteralPath $windowsPowerShell -PathType Leaf)) {
    throw "Windows PowerShell 5.1 was not found at '$windowsPowerShell'."
}

$browserConfig = @{
    Edge = @{
        ProcessName = 'msedge.exe'
        Candidates = @(
            (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'),
            (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe')
        )
    }
    Chrome = @{
        ProcessName = 'chrome.exe'
        Candidates = @(
            (Join-Path ${env:ProgramFiles(x86)} 'Google\Chrome\Application\chrome.exe'),
            (Join-Path $env:ProgramFiles 'Google\Chrome\Application\chrome.exe')
        )
    }
}[$Browser]
$browserCandidates = @($browserConfig.Candidates | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) })
if ($browserCandidates.Count -eq 0) {
    throw "$($Browser.ToUpperInvariant())_NOT_INSTALLED: $Browser was not found in the runner image."
}
$browserPath = $browserCandidates[0]
$browserProcessName = $browserConfig.ProcessName

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Browser-Hover-' + [Guid]::NewGuid().ToString('N'))
$browserProfile = Join-Path $testRoot ($Browser.ToLowerInvariant() + '-profile')
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$saveFolder = Join-Path $testRoot 'captures'
$htmlFile = Join-Path $testRoot 'surface.html'
$uiaProbeFile = Join-Path $testRoot 'uia-probe.ps1'
$uiaProbeReady = Join-Path $testRoot 'uia-ready.txt'
$uiaProbeResult = Join-Path $testRoot 'uia-result.json'
$uiaProbeStdout = Join-Path $testRoot 'uia-probe.stdout.log'
$uiaProbeStderr = Join-Path $testRoot 'uia-probe.stderr.log'
$diagnosticsFile = Join-Path $testRoot 'uia-selection-diagnostics.log'
$shotKit = $null
$browserProcess = $null
$uiaProbe = $null
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$oldDiagnosticsPath = $env:SHOTKIT_UIA_DIAGNOSTICS_PATH
$passMessage = $null
$testFailure = $null
$cleanupFailures = @()
$mouseDown = $false

function Get-ProcessWindows {
    param([System.Diagnostics.Process] $OwnedProcess)
    [ShotKitBrowserHoverSmoke.Native]::WindowsForProcess($OwnedProcess.Id)
}
function Get-ProcessDialogs {
    param([System.Diagnostics.Process] $OwnedProcess)
    Get-ProcessWindows $OwnedProcess | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' }
}
function Assert-NoUnexpectedDialog {
    param([System.Diagnostics.Process] $OwnedProcess, [string[]] $AllowedTitles = @())
    $dialog = Get-ProcessDialogs $OwnedProcess |
        Where-Object { $AllowedTitles -notcontains $_.Title -and $_.Title -notlike '*Prototyp*' } |
        Select-Object -First 1
    if ($null -ne $dialog) {
        throw "Unexpected ShotKit dialog. Title='$($dialog.Title)' HWND=$($dialog.Handle)."
    }
}
function Wait-ForCondition {
    param(
        [scriptblock] $Condition,
        [int] $TimeoutSeconds,
        [string] $Description,
        [System.Diagnostics.Process] $OwnedProcess,
        [string[]] $AllowedDialogTitles = @()
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($OwnedProcess.HasExited) {
            throw "Owned process '$($OwnedProcess.ProcessName)' exited before $Description (exit $($OwnedProcess.ExitCode))."
        }
        if ($OwnedProcess.ProcessName -like '*shotkit*') {
            Assert-NoUnexpectedDialog $OwnedProcess $AllowedDialogTitles
        }
        $value = & $Condition
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out after ${TimeoutSeconds}s waiting for $Description."
}
function Test-FullyDecodablePng {
    param([Parameter(Mandatory = $true)][string] $Path)

    $memory = $null
    $bitmap = $null
    $locked = $null
    try {
        $bytes = [IO.File]::ReadAllBytes($Path)
        $signature = [byte[]](137, 80, 78, 71, 13, 10, 26, 10)
        if ($bytes.Length -lt 20) {
            $script:lastPngReadinessError = "file is only $($bytes.Length) bytes; minimum PNG header/IEND length is 20"
            return $false
        }
        for ($i = 0; $i -lt $signature.Length; $i++) {
            if ($bytes[$i] -ne $signature[$i]) {
                $script:lastPngReadinessError = 'PNG signature is incomplete or invalid'
                return $false
            }
        }
        $iend = $bytes.Length - 12
        if ($bytes[$iend] -ne 0 -or $bytes[$iend + 1] -ne 0 -or
            $bytes[$iend + 2] -ne 0 -or $bytes[$iend + 3] -ne 0 -or
            $bytes[$iend + 4] -ne 73 -or $bytes[$iend + 5] -ne 69 -or
            $bytes[$iend + 6] -ne 78 -or $bytes[$iend + 7] -ne 68 -or
            $bytes[$iend + 8] -ne 174 -or $bytes[$iend + 9] -ne 66 -or
            $bytes[$iend + 10] -ne 96 -or $bytes[$iend + 11] -ne 130) {
            $script:lastPngReadinessError = 'PNG does not yet end with a complete IEND chunk'
            return $false
        }

        $memory = [IO.MemoryStream]::new($bytes, $false)
        $bitmap = [Drawing.Bitmap]::new($memory)
        if ($bitmap.Width -le 0 -or $bitmap.Height -le 0) {
            $script:lastPngReadinessError = "decoder returned invalid dimensions $($bitmap.Width)x$($bitmap.Height)"
            return $false
        }
        $rectangle = [Drawing.Rectangle]::new(0, 0, $bitmap.Width, $bitmap.Height)
        $locked = $bitmap.LockBits(
            $rectangle,
            [Drawing.Imaging.ImageLockMode]::ReadOnly,
            [Drawing.Imaging.PixelFormat]::Format32bppArgb
        )
        $rowBytes = $bitmap.Width * 4
        $decodedRow = [byte[]]::new($rowBytes)
        for ($y = 0; $y -lt $bitmap.Height; $y++) {
            [Runtime.InteropServices.Marshal]::Copy(
                [IntPtr]::Add($locked.Scan0, $y * $locked.Stride),
                $decodedRow,
                0,
                $rowBytes
            )
        }
        $script:lastPngReadinessError = $null
        return $true
    }
    catch {
        $script:lastPngReadinessError = $_.Exception.ToString()
        return $false
    }
    finally {
        if ($null -ne $locked -and $null -ne $bitmap) {
            $bitmap.UnlockBits($locked)
        }
        if ($null -ne $bitmap) { $bitmap.Dispose() }
        if ($null -ne $memory) { $memory.Dispose() }
    }
}
function Click-DialogButton {
    param([System.Diagnostics.Process] $OwnedProcess, $Dialog, [string[]] $ExpectedTexts, [string] $Description)
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    $buttons = @()
    while ([DateTime]::UtcNow -lt $deadline -and -not $OwnedProcess.HasExited) {
        $buttons = @([ShotKitBrowserHoverSmoke.Native]::VisibleButtons($Dialog.Handle))
        $button = $buttons | Where-Object {
            $ExpectedTexts -contains (($_.Text -replace '&', '').Trim())
        } | Select-Object -First 1
        if ($null -ne $button) {
            [ShotKitBrowserHoverSmoke.Native]::ClickButton($button.Handle)
            return $button
        }
        Start-Sleep -Milliseconds 100
    }
    $detail = ($buttons | ForEach-Object { "HWND=$($_.Handle) ID=$($_.ControlId) Text='$($_.Text)'" }) -join '; '
    if (-not $detail) { $detail = 'no visible Button-class children' }
    throw "BUTTON_TIMEOUT: $Description on '$($Dialog.Title)' HWND=$($Dialog.Handle); buttons=$detail"
}
function Wait-ForDialogClosed {
    param([System.Diagnostics.Process] $OwnedProcess, [IntPtr] $DialogHandle, [string] $Description)
    Wait-ForCondition -TimeoutSeconds 5 -Description $Description -OwnedProcess $OwnedProcess -Condition {
        @((Get-ProcessDialogs $OwnedProcess | Where-Object { $_.Handle -eq $DialogHandle })).Count -eq 0
    } | Out-Null
}
function Get-LoggedSelectionRect {
    param([Parameter(Mandatory = $true)][string] $Path, [Parameter(Mandatory = $true)][string] $Phase)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "SELECTION_DIAGNOSTICS_MISSING: no selection trace was written."
    }
    $lines = @(Get-Content -LiteralPath $Path)
    if ($Phase -eq 'mouse_down') {
        $line = $lines | Where-Object { $_ -match 'selection mouse_down point=' } | Select-Object -Last 1
        $pattern = 'selection mouse_down point=\((?<x>-?\d+), (?<y>-?\d+)\).*?frozen_rect=Some\(\((?<left>-?\d+), (?<top>-?\d+), (?<right>-?\d+), (?<bottom>-?\d+)\)\)'
    } elseif ($Phase -eq 'uia_response') {
        $line = $lines | Where-Object { $_ -match 'selection uia_response ' } | Select-Object -Last 1
        $pattern = 'selection uia_response .*?accepted_rect=Some\(\((?<left>-?\d+), (?<top>-?\d+), (?<right>-?\d+), (?<bottom>-?\d+)\)\)'
    } else {
        $line = $lines | Where-Object { $_ -match 'selection mouse_up point=' } | Select-Object -Last 1
        $pattern = 'selection mouse_up point=\((?<x>-?\d+), (?<y>-?\d+)\).*?result=Some\(\((?<left>-?\d+), (?<top>-?\d+), (?<right>-?\d+), (?<bottom>-?\d+)\)\)'
    }
    $match = [Regex]::Match([string]$line, $pattern)
    if (-not $match.Success) {
        throw "SELECTION_DIAGNOSTICS_INVALID: could not read the $Phase point and selected rectangle."
    }
    $pointX = 0
    $pointY = 0
    if ($match.Groups['x'].Success) { $pointX = [int]$match.Groups['x'].Value }
    if ($match.Groups['y'].Success) { $pointY = [int]$match.Groups['y'].Value }
    return [pscustomobject]@{
        X = $pointX
        Y = $pointY
        Left = [int]$match.Groups['left'].Value
        Top = [int]$match.Groups['top'].Value
        Right = [int]$match.Groups['right'].Value
        Bottom = [int]$match.Groups['bottom'].Value
    }
}
function Get-BrowserProcessById {
    param([int] $ProcessId)
    Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId" -ErrorAction Stop
}
function Get-ProfileBrowserProcesses {
    param([string] $ProfilePath)
    $escapedProfile = [Regex]::Escape($ProfilePath)
    @(Get-CimInstance Win32_Process -Filter "Name='$browserProcessName'" -ErrorAction Stop |
        Where-Object { $_.CommandLine -and $_.CommandLine -match $escapedProfile })
}
function Write-BrowserStartupDiagnostics {
    param([System.Diagnostics.Process] $Launcher, [string] $ProfilePath)
    Write-Output "DIAGNOSTICS: $Browser startup timed out; collecting test-owned process/window metadata before cleanup."
    try {
        $launcherCurrent = Get-BrowserProcessById $Launcher.Id
        $launcherProfileMatch = [bool]($null -ne $launcherCurrent -and
            $launcherCurrent.Name -eq $browserProcessName -and $launcherCurrent.CommandLine -and
            $launcherCurrent.CommandLine -match [Regex]::Escape($ProfilePath))
        Write-Output "DIAGNOSTICS: launcher PID=$($Launcher.Id) hasExited=$($Launcher.HasExited) currentProfileMatch=$launcherProfileMatch."
        $processes = @(Get-ProfileBrowserProcesses $ProfilePath)
        Write-Output "DIAGNOSTICS: profile-owned process count=$($processes.Count)."
        foreach ($snapshot in $processes) {
            $current = Get-BrowserProcessById ([int]$snapshot.ProcessId)
            # Process enumeration and lookup can race with exit/PID reuse.
            # Only inspect windows after confirming the same owned identity.
            if ($null -eq $current -or $current.Name -ne $browserProcessName -or
                -not $current.CommandLine -or $current.CommandLine -notmatch [Regex]::Escape($ProfilePath) -or
                $null -eq $snapshot.CreationDate -or $snapshot.CreationDate -ne $current.CreationDate) {
                Write-Output "DIAGNOSTICS: profile-owned PID=$($snapshot.ProcessId) disappeared or changed identity before window inspection."
                continue
            }
            $created = $current.CreationDate.ToUniversalTime().ToString('o')
            $isLauncher = [int]$current.ProcessId -eq $Launcher.Id
            Write-Output "DIAGNOSTICS: PID=$($current.ProcessId) parent=$($current.ParentProcessId) session=$($current.SessionId) created=$created profileMatch=True launcher=$isLauncher."
            $windows = @([ShotKitBrowserHoverSmoke.Native]::WindowsForProcess([int]$current.ProcessId))
            Write-Output "DIAGNOSTICS: PID=$($current.ProcessId) window count=$($windows.Count)."
            foreach ($window in $windows) {
                $class = if ($window.ClassName -match '^[#A-Za-z0-9_.-]{1,80}$') { $window.ClassName } else { '<redacted>' }
                Write-Output "DIAGNOSTICS: PID=$($current.ProcessId) HWND=$($window.Handle) class=$class visible=$($window.Visible)."
            }
        }
    }
    catch {
        # Exception messages can contain profile paths; retain only the type.
        Write-Output "DIAGNOSTICS: browser startup metadata collection failed ($($_.Exception.GetType().Name))."
    }
}
function Stop-OwnedBrowserProcess {
    param($ProcessSnapshot, [string] $ProfilePath)
    $ProcessId = [int]$ProcessSnapshot.ProcessId
    $current = Get-BrowserProcessById $ProcessId
    if ($null -eq $current) { return }
    $snapshotCreated = $ProcessSnapshot.CreationDate
    $currentCreated = $current.CreationDate
    if ($current.Name -ne $browserProcessName -or -not $current.CommandLine -or
        $current.CommandLine -notmatch [Regex]::Escape($ProfilePath) -or
        $null -eq $snapshotCreated -or $null -eq $currentCreated -or
        $snapshotCreated -ne $currentCreated) {
        # The profile-scoped CIM enumeration and this PID lookup are separate
        # snapshots. Chrome children can exit between them, allowing Windows
        # to reuse the PID. Never act on that replacement; the fresh final
        # profile scan below determines whether any test-owned process remains.
        $snapshotCreatedText = if ($null -ne $snapshotCreated) { $snapshotCreated.ToString('o') } else { 'unknown' }
        $currentCreatedText = if ($null -ne $currentCreated) { $currentCreated.ToString('o') } else { 'unknown' }
        $currentProfileMatch = [bool]($current.CommandLine -and $current.CommandLine -match [Regex]::Escape($ProfilePath))
        Write-Output "CLEANUP: stale Chrome PID snapshot $ProcessId (enumerated=$snapshotCreatedText, current=$currentCreatedText, name='$($current.Name)', profileMatch=$currentProfileMatch); current process left untouched and profile will be rescanned."
        return
    }
    $processHandle = $null
    try { $processHandle = [Diagnostics.Process]::GetProcessById($ProcessId) }
    catch {
        # GetProcessById itself confirms the PID vanished, even if CIM still
        # exposes a stale snapshot. The final profile scan remains mandatory.
        if ($_.Exception.InnerException -is [System.ArgumentException] -or $null -eq (Get-BrowserProcessById $ProcessId)) { return }
        throw
    }
    try {
        $confirmed = Get-BrowserProcessById $ProcessId
        if ($null -eq $confirmed) { return }
        if ($confirmed.Name -ne $browserProcessName -or -not $confirmed.CommandLine -or
            $confirmed.CommandLine -notmatch [Regex]::Escape($ProfilePath) -or
            $null -eq $snapshotCreated -or $null -eq $confirmed.CreationDate -or
            $confirmed.CreationDate -ne $snapshotCreated) {
            throw "PID $ProcessId changed identity during cleanup; refusing to stop the replacement process."
        }
        try { $processHandle.Kill() }
        catch {
            if ($processHandle.HasExited -or $null -eq (Get-BrowserProcessById $ProcessId)) { return }
            throw
        }
        if (-not $processHandle.WaitForExit(3000) -and -not $processHandle.HasExited) {
            throw "Test-owned $Browser PID $ProcessId remains after termination was requested."
        }
    }
    finally { $processHandle.Dispose() }
}
try {
    New-Item -ItemType Directory -Path $testRoot, $browserProfile, $appData, $localAppData, $saveFolder -Force | Out-Null
    $html = @'
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>ShotKit Browser UIA Surface</title>
<style>
  * { box-sizing: border-box; }
  html, body { margin: 0; width: 100%; height: 100%; overflow: hidden; background: #eeeeee; }
  body { display: grid; place-items: center; }
  #target {
    display: grid; grid-template-columns: 1fr 1fr; grid-template-rows: 1fr 1fr;
    width: 280px; height: 180px; margin: 0; padding: 0; border: 0;
    border-radius: 0; appearance: none; background: transparent; position: relative;
  }
  #target::before, #target::after { content: ""; position: absolute; pointer-events: none; }
  #target::before { left: 0; right: 0; bottom: 0; height: 1px; background: rgb(0,255,255); }
  #target::after { top: 0; right: 0; bottom: 0; width: 1px; background: rgb(255,0,255); }
  #target > span { display: block; width: 100%; height: 100%; }
</style>
</head>
<body>
  <button id="target" type="button" aria-label="ShotKit UIA hover target">
    <span aria-hidden="true" style="background:rgb(220,35,45)"></span>
    <span aria-hidden="true" style="background:rgb(30,190,65)"></span>
    <span aria-hidden="true" style="background:rgb(35,70,220)"></span>
    <span aria-hidden="true" style="background:rgb(235,205,35)"></span>
  </button>
</body>
</html>
'@
    [IO.File]::WriteAllText($htmlFile, $html, [Text.UTF8Encoding]::new($false))
    $uiaProbeScript = @'
$ErrorActionPreference = 'Stop'
try {
    [Console]::Error.WriteLine('UIA phase: loading UIAutomationClient')
    Add-Type -AssemblyName UIAutomationClient
    [Console]::Error.WriteLine('UIA phase: loading UIAutomationTypes')
    Add-Type -AssemblyName UIAutomationTypes
    [Console]::Error.WriteLine('UIA phase: preparing target search')
    $windowHandle = [IntPtr]([Int64]$args[0])
    $condition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty,
        'ShotKit UIA hover target'
    )
    # Publish readiness atomically so cold PowerShell/assembly startup does not
    # consume the separate UIA operation budget enforced by the parent.
    $readyAt = [DateTime]::UtcNow
    [IO.File]::WriteAllText(($args[2] + '.tmp'), $readyAt.ToString('o'), [Text.UTF8Encoding]::new($false))
    [IO.File]::Move(($args[2] + '.tmp'), $args[2])
    [Console]::Error.WriteLine("UIA phase: ready at $($readyAt.ToString('o'))")
    # The renderer and UIA provider can become available after the top-level
    # HWND. Retry only this transient absence; the parent enforces a hard 15s
    # UIA limit from readiness, including property reads and result publication.
    $deadline = [DateTime]::UtcNow.AddSeconds(12)
    $target = $null
    do {
        [Console]::Error.WriteLine("UIA phase: FromHandle starting at $([DateTime]::UtcNow.ToString('o'))")
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($windowHandle)
        [Console]::Error.WriteLine("UIA phase: FromHandle completed at $([DateTime]::UtcNow.ToString('o'))")
        if ($null -ne $root) {
            [Console]::Error.WriteLine("UIA phase: FindFirst starting at $([DateTime]::UtcNow.ToString('o'))")
            $target = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
            [Console]::Error.WriteLine("UIA phase: FindFirst completed at $([DateTime]::UtcNow.ToString('o'))")
        }
        if ($null -eq $target -and [DateTime]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 200
        }
    } while ($null -eq $target -and [DateTime]::UtcNow -lt $deadline)
    if ($null -eq $root) { throw 'UIA ElementFromHandle returned null throughout provider initialization.' }
    if ($null -eq $target) { throw "Named target remained absent from $Browser UIA Control View after provider initialization wait." }
    [Console]::Error.WriteLine("UIA phase: Current starting at $([DateTime]::UtcNow.ToString('o'))")
    $current = $target.Current
    $bounds = $current.BoundingRectangle
    [Console]::Error.WriteLine("UIA phase: Current completed at $([DateTime]::UtcNow.ToString('o'))")
    if ($current.IsOffscreen) { throw 'Named target is marked offscreen.' }
    if ($current.ControlType.ProgrammaticName -ne 'ControlType.Button') {
        throw "Named target is not exposed as a UIA button (type '$($current.ControlType.ProgrammaticName)')."
    }
    $result = @{
        Name = $current.Name
        ControlType = $current.ControlType.ProgrammaticName
        X = $bounds.X
        Y = $bounds.Y
        Width = $bounds.Width
        Height = $bounds.Height
        IsOffscreen = $current.IsOffscreen
    } | ConvertTo-Json -Compress
    # Publish only a closed, complete JSON file. Its last-write timestamp is
    # the completion time used by the parent even if its next poll is delayed.
    [IO.File]::WriteAllText(($args[1] + '.tmp'), $result, [Text.UTF8Encoding]::new($false))
    [IO.File]::Move(($args[1] + '.tmp'), $args[1])
}
catch {
    [Console]::Error.WriteLine($_.Exception.ToString())
    exit 2
}
'@
    [IO.File]::WriteAllText($uiaProbeFile, $uiaProbeScript, [Text.UTF8Encoding]::new($false))

    $desktopLeft = [ShotKitBrowserHoverSmoke.Native]::GetSystemMetrics(76)
    $desktopTop = [ShotKitBrowserHoverSmoke.Native]::GetSystemMetrics(77)
    $desktopWidth = [ShotKitBrowserHoverSmoke.Native]::GetSystemMetrics(78)
    $desktopHeight = [ShotKitBrowserHoverSmoke.Native]::GetSystemMetrics(79)
    if ($desktopWidth -lt 800 -or $desktopHeight -lt 600) {
        throw "INTERACTIVE_DESKTOP_TOO_SMALL: ${desktopWidth}x${desktopHeight}."
    }

    $fileUri = ([Uri]::new($htmlFile)).AbsoluteUri
    $browserArgs = @(
        "--user-data-dir=`"$browserProfile`"",
        '--no-first-run',
        '--disable-background-networking',
        '--disable-sync',
        '--disable-extensions',
        '--disable-component-update',
        '--force-device-scale-factor=1',
        "--app=`"$fileUri`"",
        '--window-size=1000,720'
    )
    if ($Browser -eq 'Edge') { $browserArgs += '--disable-features=msEdgeSidebarV2,msEdgeDiscover,msSmartScreenProtection' }
    $browserArgs = $browserArgs -join ' '
    $browserProcess = Start-Process -FilePath $browserPath -ArgumentList $browserArgs -WorkingDirectory $testRoot -PassThru

    try {
        $browserWindow = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description "the owned $Browser app window" -OwnedProcess $browserProcess -Condition {
            Get-ProcessWindows $browserProcess | Where-Object { $_.Visible -and $_.ClassName -eq 'Chrome_WidgetWin_1' } | Select-Object -First 1
        }
    }
    catch {
        if ($_.Exception.Message -eq "Timed out after $($StartupTimeoutSeconds)s waiting for the owned $Browser app window.") {
            Write-BrowserStartupDiagnostics -Launcher $browserProcess -ProfilePath $browserProfile
        }
        throw
    }
    if (-not [ShotKitBrowserHoverSmoke.Native]::SetForegroundWindow($browserWindow.Handle)) {
        throw "Could not foreground the test-owned $Browser app window."
    }

    $targetName = 'ShotKit UIA hover target'
    $probeArgs = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" "{1}" "{2}" "{3}"' -f `
        $uiaProbeFile, $browserWindow.Handle.ToInt64(), $uiaProbeResult, $uiaProbeReady
    $probeStartupDeadline = [DateTime]::UtcNow.AddSeconds(15)
    $uiaProbe = Start-Process -FilePath $windowsPowerShell -ArgumentList $probeArgs -WorkingDirectory $testRoot `
        -WindowStyle Hidden -RedirectStandardOutput $uiaProbeStdout -RedirectStandardError $uiaProbeStderr -PassThru
    $probeDeadline = $null
    while ($true) {
        # Check every deadline before accepting an available result, including
        # when the child has already exited between polls.
        $probeCompleted = Test-Path -LiteralPath $uiaProbeResult
        if ($null -eq $probeDeadline -and (Test-Path -LiteralPath $uiaProbeReady)) {
            $readyAt = [DateTime]::Parse(
                [IO.File]::ReadAllText($uiaProbeReady),
                [Globalization.CultureInfo]::InvariantCulture,
                [Globalization.DateTimeStyles]::RoundtripKind
            )
            $probeDeadline = $readyAt.AddSeconds(15)
        }
        if ($probeCompleted -and $null -eq $probeDeadline) {
            throw 'UIA_PRECHECK_FAILED: the probe published a result without a readiness marker.'
        }
        $phaseTime = if ($probeCompleted) {
            [IO.File]::GetLastWriteTimeUtc($uiaProbeResult)
        } else { [DateTime]::UtcNow }
        $startupTimedOut = ($null -eq $probeDeadline -and $phaseTime -ge $probeStartupDeadline) -or
            ($null -ne $probeDeadline -and $readyAt -gt $probeStartupDeadline)
        $uiaTimedOut = $null -ne $probeDeadline -and $phaseTime -ge $probeDeadline
        if ($startupTimedOut -or $uiaTimedOut) {
            # Reobserve publication that raced this poll before rejecting it;
            # the next iteration validates its recorded completion time.
            if (-not $probeCompleted -and (Test-Path -LiteralPath $uiaProbeResult)) { continue }
            if ($null -eq $probeDeadline -and (Test-Path -LiteralPath $uiaProbeReady)) { continue }
            try { $uiaProbe.Kill(); [void]$uiaProbe.WaitForExit(3000) } catch { }
            $probeTrace = if (Test-Path -LiteralPath $uiaProbeStderr) { [IO.File]::ReadAllText($uiaProbeStderr).Trim() } else { 'No probe diagnostics available.' }
            if ($startupTimedOut) {
                throw "UIA_PROBE_STARTUP_TIMEOUT: $Browser probe did not finish PowerShell/assembly initialization within 15 seconds. Probe trace: $probeTrace"
            }
            throw "UIA_PRECHECK_TIMEOUT: $Browser UI Automation did not return within 15 seconds after probe readiness. Probe trace: $probeTrace"
        }
        if ($probeCompleted) { break }
        if ($uiaProbe.HasExited) {
            # The result may have been published after this iteration's first
            # observation but before process exit. Recheck it with its deadline.
            if (Test-Path -LiteralPath $uiaProbeResult) { continue }
            $probeError = if (Test-Path -LiteralPath $uiaProbeStderr) { [IO.File]::ReadAllText($uiaProbeStderr).Trim() } else { '' }
            if ([string]::IsNullOrWhiteSpace($probeError)) { $probeError = 'No UIA result was written.' }
            throw "UIA_PRECHECK_FAILED: $probeError"
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $uiaProbe.HasExited) { [void]$uiaProbe.WaitForExit(1000) }
    if (-not $uiaProbe.HasExited -or $uiaProbe.ExitCode -ne 0) {
        throw 'UIA_PRECHECK_FAILED: the isolated UIA probe did not exit successfully.'
    }
    $uia = Get-Content -LiteralPath $uiaProbeResult -Raw | ConvertFrom-Json
    if ($uia.Name -ne $targetName -or $uia.ControlType -ne 'ControlType.Button' -or $uia.IsOffscreen) {
        throw "UIA_PRECHECK_FAILED: expected visible named button '$targetName', received '$($uia.Name)' / '$($uia.ControlType)'."
    }
    $bounds = [pscustomobject]@{ X = [double]$uia.X; Y = [double]$uia.Y; Width = [double]$uia.Width; Height = [double]$uia.Height }
    if ([double]::IsNaN($bounds.X) -or [double]::IsNaN($bounds.Y) -or
        [double]::IsInfinity($bounds.X) -or [double]::IsInfinity($bounds.Y) -or
        [double]::IsNaN($bounds.Width) -or [double]::IsNaN($bounds.Height) -or
        $bounds.Width -lt 100 -or $bounds.Height -lt 80) {
        throw "UIA_PRECHECK_FAILED: $Browser returned invalid or undersized bounds ($($bounds.X),$($bounds.Y),$($bounds.Width),$($bounds.Height))."
    }
    if ([Math]::Abs($bounds.Width - 280) -gt 1 -or [Math]::Abs($bounds.Height - 180) -gt 1) {
        throw "UIA_PRECHECK_FAILED: expected 280x180 CSS px at scale factor 1, got $([Math]::Round($bounds.Width,2))x$([Math]::Round($bounds.Height,2)) UIA px."
    }
    $targetLeft = [int][Math]::Floor($bounds.X)
    $targetTop = [int][Math]::Floor($bounds.Y)
    $targetRight = [int][Math]::Ceiling($bounds.X + $bounds.Width)
    $targetBottom = [int][Math]::Ceiling($bounds.Y + $bounds.Height)
    $targetWidth = $targetRight - $targetLeft
    $targetHeight = $targetBottom - $targetTop
    if ($targetLeft -lt $desktopLeft -or $targetTop -lt $desktopTop -or
        $targetRight -gt $desktopLeft + $desktopWidth -or
        $targetBottom -gt $desktopTop + $desktopHeight) {
        throw 'UIA_PRECHECK_FAILED: target bounds extend outside the interactive desktop.'
    }
    Write-Output "MILESTONE: UIA named button confirmed at ${targetWidth}x${targetHeight} screen pixels."
    $centerX = [int][Math]::Floor($bounds.X + ($bounds.Width / 2))
    $centerY = [int][Math]::Floor($bounds.Y + ($bounds.Height / 2))
    if ($centerX -lt $bounds.X -or $centerX -ge ($bounds.X + $bounds.Width) -or
        $centerY -lt $bounds.Y -or $centerY -ge ($bounds.Y + $bounds.Height)) {
        throw 'UIA_PRECHECK_FAILED: integer pointer center falls outside the fractional UIA target bounds.'
    }

    $settingsPath = Join-Path (Join-Path $appData 'ShotKitRust') 'settings.json'
    New-Item -ItemType Directory -Path (Split-Path -Parent $settingsPath) -Force | Out-Null
    $settings = @{
        version = 1
        save_folder = $saveFolder
        open_editor = $true
        copy_to_clipboard = $false
        auto_save = $true
        scroll_direction = 'down'
    } | ConvertTo-Json
    [IO.File]::WriteAllText($settingsPath, $settings, [Text.UTF8Encoding]::new($false))

    try {
        $env:APPDATA = $appData
        $env:LOCALAPPDATA = $localAppData
        $env:SHOTKIT_UIA_DIAGNOSTICS_PATH = $diagnosticsFile
        $shotKit = Start-Process -FilePath $ExePath -WorkingDirectory $testRoot -PassThru
    }
    finally {
        $env:APPDATA = $oldAppData
        $env:LOCALAPPDATA = $oldLocalAppData
        $env:SHOTKIT_UIA_DIAGNOSTICS_PATH = $oldDiagnosticsPath
    }

    $mainWindow = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the ShotKit main window' -OwnedProcess $shotKit -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
    }
    $welcome = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the ShotKit startup dialog' -OwnedProcess $shotKit -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -like '*Prototyp*' } | Select-Object -First 1
    }
    [void](Click-DialogButton -OwnedProcess $shotKit -Dialog $welcome -ExpectedTexts @('OK') -Description 'startup acknowledgement')
    Wait-ForDialogClosed -OwnedProcess $shotKit -DialogHandle $welcome.Handle -Description 'the startup dialog to close'
    if ($shotKit.HasExited) { throw 'ShotKit exited after its startup dialog was closed.' }
    Assert-NoUnexpectedDialog $shotKit

    if (-not [ShotKitBrowserHoverSmoke.Native]::SetForegroundWindow($browserWindow.Handle)) {
        throw "Could not activate the owned $Browser app before requesting selection."
    }
    [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x7A, 0, 0, [UIntPtr]::Zero)
    [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x7A, 0, 0x0002, [UIntPtr]::Zero)
    $overlay = Wait-ForCondition -TimeoutSeconds 10 -Description 'ShotKit selection overlay' -OwnedProcess $shotKit -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible } | Select-Object -First 1
    }
    if (-not [ShotKitBrowserHoverSmoke.Native]::SetForegroundWindow($overlay.Handle)) {
        throw 'Could not activate ShotKit selection overlay.'
    }
    [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x11, 0, 0x0002, [UIntPtr]::Zero)
    $foregroundWindow = [ShotKitBrowserHoverSmoke.Native]::ForegroundWindowHandle()
    $foregroundPid = [ShotKitBrowserHoverSmoke.Native]::ProcessIdForWindow($foregroundWindow)
    if ($foregroundPid -ne [uint]$shotKit.Id) {
        throw "TEST_FOCUS_MISMATCH: foreground PID is $foregroundPid, expected ShotKit PID $($shotKit.Id)."
    }

    # Use the measured UIA screen rectangle and real system pointer input. The
    # click is stationary; a manual rectangle drag cannot satisfy the checks.
    if (-not [ShotKitBrowserHoverSmoke.Native]::SetCursorPos($centerX, $centerY)) {
        throw 'Could not move the system pointer to the UIA target center.'
    }
    Start-Sleep -Milliseconds 1800
    $actualX = 0
    $actualY = 0
    if (-not [ShotKitBrowserHoverSmoke.Native]::CursorPosition([ref]$actualX, [ref]$actualY) -or
        $actualX -ne $centerX -or $actualY -ne $centerY) {
        throw "TEST_CURSOR_MISMATCH: pointer is at ($actualX,$actualY), expected ($centerX,$centerY)."
    }
    $foregroundWindow = [ShotKitBrowserHoverSmoke.Native]::ForegroundWindowHandle()
    $foregroundPid = [ShotKitBrowserHoverSmoke.Native]::ProcessIdForWindow($foregroundWindow)
    if ($foregroundPid -ne [uint]$shotKit.Id) {
        throw "TEST_FOCUS_MISMATCH: foreground PID is $foregroundPid at click time, expected ShotKit PID $($shotKit.Id)."
    }
    Write-Output "MILESTONE: cursor=$actualX,$actualY and foreground PID=$foregroundPid verified before hover click."
    if ($KeyboardAdjustment -or $EnterAccept) {
        # Preserve the real browser hover suggestion. The adjustment scenario
        # edits it with Right and Shift+Down; EnterAccept submits it unchanged.
        Start-Sleep -Milliseconds 900
        if ($KeyboardAdjustment) {
            [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x27, 0, 0, [UIntPtr]::Zero)
            [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x27, 0, 0x0002, [UIntPtr]::Zero)
            [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
            [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x28, 0, 0, [UIntPtr]::Zero)
            [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x28, 0, 0x0002, [UIntPtr]::Zero)
            [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x10, 0, 0x0002, [UIntPtr]::Zero)
        }
        Start-Sleep -Milliseconds 250
        [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x0D, 0, 0, [UIntPtr]::Zero)
        [ShotKitBrowserHoverSmoke.Native]::keybd_event(0x0D, 0, 0x0002, [UIntPtr]::Zero)
        if ($EnterAccept) {
            $cursorAfterEnterX = 0; $cursorAfterEnterY = 0
            if (-not [ShotKitBrowserHoverSmoke.Native]::CursorPosition([ref]$cursorAfterEnterX, [ref]$cursorAfterEnterY) -or
                $cursorAfterEnterX -ne $actualX -or $cursorAfterEnterY -ne $actualY) {
                throw "ENTER_MOVED_CURSOR: cursor was ($actualX,$actualY) before Enter and ($cursorAfterEnterX,$cursorAfterEnterY) after Enter."
            }
        }
    }
    else {
        [ShotKitBrowserHoverSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
        $mouseDown = $true
        Start-Sleep -Milliseconds 120
        [ShotKitBrowserHoverSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
        $mouseDown = $false
    }

    $script:lastPngReadinessError = 'PNG file not present yet'
    try {
        $png = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'a completely written, decodable auto-saved browser target PNG' -OwnedProcess $shotKit -Condition {
            $candidate = Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Select-Object -First 1
            if ($null -ne $candidate -and (Test-FullyDecodablePng -Path $candidate.FullName)) {
                return $candidate
            }
            return $null
        }
    }
    catch {
        if ($_.Exception.Message -like 'Timed out after * waiting for a completely written, decodable auto-saved browser target PNG.*') {
            $fileDiagnostic = 'no PNG file exists'
            $latestFile = Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File -ErrorAction SilentlyContinue |
                Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
            if ($null -ne $latestFile) {
                $fileDiagnostic = "latest file='$($latestFile.FullName)' length=$($latestFile.Length) lastWriteUtc=$($latestFile.LastWriteTimeUtc.ToString('o'))"
            }
            throw "Timed out waiting for a fully decodable PNG. Last decoder/readiness result: $script:lastPngReadinessError; $fileDiagnostic"
        }
        throw
    }
    $pngs = @(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File)
    if ($pngs.Count -ne 1) { throw "Expected exactly one captured PNG, found $($pngs.Count)." }

    $bitmap = [Drawing.Bitmap]::new($png.FullName)
    try {
        $uiaSelection = Get-LoggedSelectionRect -Path $diagnosticsFile -Phase 'uia_response'
        if ($EnterAccept) {
            $uiaWidth = $uiaSelection.Right - $uiaSelection.Left
            $uiaHeight = $uiaSelection.Bottom - $uiaSelection.Top
            $selectionWidth = $uiaWidth
            $selectionHeight = $uiaHeight
            if ([Math]::Abs($uiaWidth - 280) -gt 1 -or [Math]::Abs($uiaHeight - 180) -gt 1 -or
                [Math]::Abs($uiaSelection.Left - $bounds.X) -gt 1 -or
                [Math]::Abs($uiaSelection.Top - $bounds.Y) -gt 1 -or
                [Math]::Abs($uiaSelection.Right - ($bounds.X + $bounds.Width)) -gt 1 -or
                [Math]::Abs($uiaSelection.Bottom - ($bounds.Y + $bounds.Height)) -gt 1) {
                throw "UIA_BASE_BOUNDS_MISMATCH: UIA suggestion $($uiaWidth)x$($uiaHeight) at ($($uiaSelection.Left),$($uiaSelection.Top),$($uiaSelection.Right),$($uiaSelection.Bottom)) does not match fractional bounds ($($bounds.X),$($bounds.Y),$($bounds.Width),$($bounds.Height)) within one pixel."
            }
            $trace = [IO.File]::ReadAllText($diagnosticsFile)
            if ($trace -match 'selection mouse_(down|up)') {
                throw 'ENTER_PATH_MOUSE_GESTURE: Enter acceptance unexpectedly recorded a mouse selection gesture.'
            }
            if ($bitmap.Width -ne $uiaWidth -or $bitmap.Height -ne $uiaHeight) {
                throw "ENTER_BOUNDS_MISMATCH: PNG is $($bitmap.Width)x$($bitmap.Height), logged UIA rectangle is ${uiaWidth}x${uiaHeight}."
            }
            # Fractional UIA bounds can round at either edge. Check the edge
            # landmarks at their measured positions inside the logged crop;
            # there is no following outside pixel when the crop is unchanged.
            $rightMarkerX = [int][Math]::Floor($bounds.X + $bounds.Width) - 1 - $uiaSelection.Left
            $bottomMarkerY = [int][Math]::Floor($bounds.Y + $bounds.Height) - 1 - $uiaSelection.Top
            $sampleX = [int][Math]::Floor($bitmap.Width / 2.0)
            $sampleY = [int][Math]::Floor($bitmap.Height / 2.0)
            if ($rightMarkerX -lt 0 -or $rightMarkerX -ge $bitmap.Width -or
                $bottomMarkerY -lt 0 -or $bottomMarkerY -ge $bitmap.Height) {
                throw "ENTER_PIXEL_INDEX_MISMATCH: measured UIA edge markers fall outside the logged crop (right=$rightMarkerX, bottom=$bottomMarkerY, PNG=$($bitmap.Width)x$($bitmap.Height))."
            }
            $rightMarker = $bitmap.GetPixel($rightMarkerX, $sampleY)
            $bottomMarker = $bitmap.GetPixel($sampleX, $bottomMarkerY)
            if ($rightMarker.R -lt 250 -or $rightMarker.G -gt 5 -or $rightMarker.B -lt 250 -or
                $bottomMarker.R -gt 5 -or $bottomMarker.G -lt 250 -or $bottomMarker.B -lt 250) {
                throw "ENTER_PIXEL_POSITION_MISMATCH: expected magenta right-edge marker at ($rightMarkerX,$sampleY) and cyan bottom marker at ($sampleX,$bottomMarkerY); got RGB right=$($rightMarker.R),$($rightMarker.G),$($rightMarker.B) bottom=$($bottomMarker.R),$($bottomMarker.G),$($bottomMarker.B)."
            }
            $finalRect = [pscustomobject]@{ Left=$uiaSelection.Left; Top=$uiaSelection.Top; Right=$uiaSelection.Right; Bottom=$uiaSelection.Bottom }
        }
        elseif ($KeyboardAdjustment) {
            $keyboardRect = [pscustomobject]@{
                Left=$uiaSelection.Left + 1; Top=$uiaSelection.Top
                Right=$uiaSelection.Right + 1; Bottom=$uiaSelection.Bottom + 1
            }
            $baseWidth = $uiaSelection.Right - $uiaSelection.Left
            $baseHeight = $uiaSelection.Bottom - $uiaSelection.Top
            if ([Math]::Abs($baseWidth - 280) -gt 1 -or [Math]::Abs($baseHeight - 180) -gt 1 -or
                [Math]::Abs($uiaSelection.Left - $bounds.X) -gt 1 -or
                [Math]::Abs($uiaSelection.Top - $bounds.Y) -gt 1 -or
                [Math]::Abs($uiaSelection.Right - ($bounds.X + $bounds.Width)) -gt 1 -or
                [Math]::Abs($uiaSelection.Bottom - ($bounds.Y + $bounds.Height)) -gt 1) {
                throw "UIA_BASE_BOUNDS_MISMATCH: UIA suggestion $($baseWidth)x$($baseHeight) at ($($uiaSelection.Left),$($uiaSelection.Top),$($uiaSelection.Right),$($uiaSelection.Bottom)) does not match fractional bounds ($($bounds.X),$($bounds.Y),$($bounds.Width),$($bounds.Height)) within one pixel."
            }
            $trace = [IO.File]::ReadAllText($diagnosticsFile)
            if ($trace -match 'selection mouse_(down|up)') {
                throw 'KEYBOARD_PATH_MOUSE_GESTURE: keyboard adjustment unexpectedly recorded a mouse selection gesture.'
            }
            $selectionWidth = $keyboardRect.Right - $keyboardRect.Left
            $selectionHeight = $keyboardRect.Bottom - $keyboardRect.Top
            if ($selectionWidth -ne $baseWidth -or $selectionHeight -ne ($baseHeight + 1)) {
                throw "KEYBOARD_BOUNDS_MISMATCH: Right must preserve the UIA width ($baseWidth) and Shift+Down must add one pixel to its height ($($baseHeight + 1)); got ${selectionWidth}x${selectionHeight}."
            }
            if ([Math]::Abs($keyboardRect.Left - ($bounds.X + 1)) -gt 1 -or
                [Math]::Abs($keyboardRect.Top - $bounds.Y) -gt 1 -or
                [Math]::Abs($keyboardRect.Right - ($bounds.X + $bounds.Width + 1)) -gt 1 -or
                [Math]::Abs($keyboardRect.Bottom - ($bounds.Y + $bounds.Height + 1)) -gt 1) {
                throw "KEYBOARD_RECT_MISMATCH: expected UIA rectangle translated +1 X and resized +1 bottom, got ($($keyboardRect.Left),$($keyboardRect.Top),$($keyboardRect.Right),$($keyboardRect.Bottom))."
            }
            # UIA screen bounds are fractional while the accepted selection is
            # integer, half-open physical pixels. Derive landmark indices from
            # the measured bounds and accepted crop origin instead of assuming
            # the fractional edge always rounds in the same direction.
            $rightMarkerX = [int][Math]::Floor($bounds.X + $bounds.Width) - 1 - $keyboardRect.Left
            $bottomMarkerY = [int][Math]::Floor($bounds.Y + $bounds.Height) - 1 - $keyboardRect.Top
            $sampleX = [int][Math]::Floor($bitmap.Width / 2.0)
            $sampleY = [int][Math]::Floor($bitmap.Height / 2.0)
            if ($rightMarkerX -lt 0 -or $rightMarkerX -ge $bitmap.Width - 1 -or
                $bottomMarkerY -lt 0 -or $bottomMarkerY -ge $bitmap.Height - 1) {
                throw "KEYBOARD_PIXEL_INDEX_MISMATCH: fractional UIA edges map outside the expected marker samples (right=$rightMarkerX, bottom=$bottomMarkerY, PNG=$($bitmap.Width)x$($bitmap.Height))."
            }
            $rightMarker = $bitmap.GetPixel($rightMarkerX, $sampleY)
            $outsideTarget = $bitmap.GetPixel($rightMarkerX + 1, $sampleY)
            $bottomMarker = $bitmap.GetPixel($sampleX, $bottomMarkerY)
            $outsideBottom = $bitmap.GetPixel($sampleX, $bottomMarkerY + 1)
            if ($rightMarker.R -lt 250 -or $rightMarker.G -gt 5 -or $rightMarker.B -lt 250 -or
                $outsideTarget.R -lt 230 -or $outsideTarget.R -gt 245 -or
                $outsideTarget.G -lt 230 -or $outsideTarget.G -gt 245 -or
                $outsideTarget.B -lt 230 -or $outsideTarget.B -gt 245 -or
                $bottomMarker.R -gt 5 -or $bottomMarker.G -lt 250 -or $bottomMarker.B -lt 250 -or
                $outsideBottom.R -lt 230 -or $outsideBottom.R -gt 245 -or
                $outsideBottom.G -lt 230 -or $outsideBottom.G -gt 245 -or
                $outsideBottom.B -lt 230 -or $outsideBottom.B -gt 245) {
                throw "KEYBOARD_PIXEL_POSITION_MISMATCH: expected magenta right-edge marker at ($rightMarkerX,$sampleY), page background after it, cyan bottom marker at ($sampleX,$bottomMarkerY), and page background after it; got RGB right=$($rightMarker.R),$($rightMarker.G),$($rightMarker.B) rightOutside=$($outsideTarget.R),$($outsideTarget.G),$($outsideTarget.B) bottom=$($bottomMarker.R),$($bottomMarker.G),$($bottomMarker.B) bottomOutside=$($outsideBottom.R),$($outsideBottom.G),$($outsideBottom.B)."
            }
            $finalRect = $keyboardRect
        }
        else {
            $mouseDownSelection = Get-LoggedSelectionRect -Path $diagnosticsFile -Phase 'mouse_down'
            $mouseUpSelection = Get-LoggedSelectionRect -Path $diagnosticsFile -Phase 'mouse_up'
            if ($mouseDownSelection.X -ne $centerX -or $mouseDownSelection.Y -ne $centerY -or
                $mouseUpSelection.X -ne $centerX -or $mouseUpSelection.Y -ne $centerY) {
                throw "SELECTION_COORDINATE_MISMATCH: logged down/up points were ($($mouseDownSelection.X),$($mouseDownSelection.Y)) / ($($mouseUpSelection.X),$($mouseUpSelection.Y)); expected ($centerX,$centerY)."
            }
            if ($mouseDownSelection.Left -ne $mouseUpSelection.Left -or $mouseDownSelection.Top -ne $mouseUpSelection.Top -or
                $mouseDownSelection.Right -ne $mouseUpSelection.Right -or $mouseDownSelection.Bottom -ne $mouseUpSelection.Bottom) {
                throw 'SELECTION_RECT_MISMATCH: the rectangle frozen on mouse-down changed before mouse-up.'
            }
            if ($uiaSelection.Left -ne $mouseDownSelection.Left -or $uiaSelection.Top -ne $mouseDownSelection.Top -or
                $uiaSelection.Right -ne $mouseDownSelection.Right -or $uiaSelection.Bottom -ne $mouseDownSelection.Bottom) {
                throw 'SELECTION_UIA_MISMATCH: the frozen selection differs from the latest accepted UIA rectangle.'
            }
            $selectionWidth = $mouseUpSelection.Right - $mouseUpSelection.Left
            $selectionHeight = $mouseUpSelection.Bottom - $mouseUpSelection.Top
            if ([Math]::Abs($selectionWidth - 280) -gt 1 -or [Math]::Abs($selectionHeight - 180) -gt 1 -or
                [Math]::Abs($mouseUpSelection.Left - $bounds.X) -gt 1 -or
                [Math]::Abs($mouseUpSelection.Top - $bounds.Y) -gt 1 -or
                [Math]::Abs($mouseUpSelection.Right - ($bounds.X + $bounds.Width)) -gt 1 -or
                [Math]::Abs($mouseUpSelection.Bottom - ($bounds.Y + $bounds.Height)) -gt 1) {
                throw "SELECTION_BOUNDS_MISMATCH: selected RECT ($($mouseUpSelection.Left),$($mouseUpSelection.Top),$($mouseUpSelection.Right),$($mouseUpSelection.Bottom)) does not match the fractional UIA target within one pixel."
            }
            $finalRect = $mouseUpSelection
        }
        if ($bitmap.Width -ne $selectionWidth -or $bitmap.Height -ne $selectionHeight) {
            throw "CAPTURE_BOUNDS_MISMATCH: PNG is $($bitmap.Width)x$($bitmap.Height), logged selection is ${selectionWidth}x${selectionHeight}."
        }
        $samples = @(
            @{ X = [int][Math]::Floor($bitmap.Width * 0.25); Y = [int][Math]::Floor($bitmap.Height * 0.25); R = 220; G = 35; B = 45; Name = 'top-left red' },
            @{ X = [int][Math]::Floor($bitmap.Width * 0.75); Y = [int][Math]::Floor($bitmap.Height * 0.25); R = 30; G = 190; B = 65; Name = 'top-right green' },
            @{ X = [int][Math]::Floor($bitmap.Width * 0.25); Y = [int][Math]::Floor($bitmap.Height * 0.75); R = 35; G = 70; B = 220; Name = 'bottom-left blue' },
            @{ X = [int][Math]::Floor($bitmap.Width * 0.75); Y = [int][Math]::Floor($bitmap.Height * 0.75); R = 235; G = 205; B = 35; Name = 'bottom-right yellow' }
        )
        foreach ($sample in $samples) {
            $actual = $bitmap.GetPixel($sample.X, $sample.Y)
            if ([Math]::Abs($actual.R - $sample.R) -gt 3 -or
                [Math]::Abs($actual.G - $sample.G) -gt 3 -or
                [Math]::Abs($actual.B - $sample.B) -gt 3) {
                throw "CAPTURE_PIXEL_MISMATCH: '$($sample.Name)' at ($($sample.X),$($sample.Y)) was RGB=$($actual.R),$($actual.G),$($actual.B), expected $($sample.R),$($sample.G),$($sample.B)."
            }
        }
    }
    finally { $bitmap.Dispose() }
    Write-Output 'MILESTONE: capture dimensions and all four expected pixels validated.'

    Assert-NoUnexpectedDialog $shotKit
    $interaction = if ($KeyboardAdjustment) { 'hover + Right + Shift+Down + Enter' } elseif ($EnterAccept) { 'unchanged hover + Enter, cursor stationary' } else { 'hover-click' }
    $location = if ($EnterAccept) { "logged UIA rectangle=($($finalRect.Left),$($finalRect.Top),$($finalRect.Right - $finalRect.Left)x$($finalRect.Bottom - $finalRect.Top))" } else { "screen=($($finalRect.Left),$($finalRect.Top),$($finalRect.Right - $finalRect.Left)x$($finalRect.Bottom - $finalRect.Top))" }
    $passMessage = "PASS: $Browser UIA name='$targetName', HWND=$($browserWindow.Handle), $location; $interaction captured matching dimensions and expected pixels."
}
catch {
    $testFailure = $_
}
finally {
    if ($mouseDown) {
        try { [ShotKitBrowserHoverSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) }
        catch { $cleanupFailures += "Could not release test-owned mouse button: $($_.Exception.Message)" }
    }
    if ($null -ne $shotKit) {
        try {
            if (-not $shotKit.HasExited) {
                $shotKit.Kill()
                if (-not $shotKit.WaitForExit(5000) -or -not $shotKit.HasExited) {
                    throw 'ShotKit did not exit after termination was requested.'
                }
            }
        }
        catch { $cleanupFailures += "Could not stop test-owned ShotKit process: $($_.Exception.Message)" }
        $shotKit.Dispose()
    }
    if ($null -ne $uiaProbe) {
        try {
            if (-not $uiaProbe.HasExited) {
                $uiaProbe.Kill()
                if (-not $uiaProbe.WaitForExit(5000) -or -not $uiaProbe.HasExited) {
                    throw 'The isolated UIA probe did not exit after termination was requested.'
                }
            }
        }
        catch { $cleanupFailures += "Could not stop isolated UIA probe: $($_.Exception.Message)" }
        $uiaProbe.Dispose()
    }
    if ($null -ne $browserProcess) {
        try {
            $ownedBrowser = Get-BrowserProcessById $browserProcess.Id
            if ($null -eq $ownedBrowser) {
                Write-Output "CLEANUP: $Browser launcher PID $($browserProcess.Id) is verified absent."
            } elseif ($ownedBrowser.Name -eq $browserProcessName -and $ownedBrowser.CommandLine -and
                $ownedBrowser.CommandLine -match [Regex]::Escape($browserProfile)) {
                # Start-Process returned a Process object for this exact process;
                # Kill uses its process handle instead of re-opening a bare PID.
                if (-not $browserProcess.HasExited) {
                    $browserProcess.Kill()
                    if (-not $browserProcess.WaitForExit(3000) -and -not $browserProcess.HasExited) {
                        throw "Owned $Browser launcher PID $($browserProcess.Id) remains after termination was requested."
                    }
                }
            } else {
                Write-Output "CLEANUP: $Browser launcher PID $($browserProcess.Id) now belongs to another process; it was left untouched."
            }
        }
        catch { $cleanupFailures += "Could not stop test-owned $Browser process tree: $($_.Exception.Message)" }
        $browserProcess.Dispose()
    }
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    $env:SHOTKIT_UIA_DIAGNOSTICS_PATH = $oldDiagnosticsPath

    # Only stop processes whose current identity still matches this unique
    # profile. A process disappearing between enumeration and Stop-Process is
    # accepted only after a fresh PID query confirms absence.
    try {
        $stragglers = Get-ProfileBrowserProcesses $browserProfile
        foreach ($process in $stragglers) {
            try {
                Stop-OwnedBrowserProcess -ProcessSnapshot $process -ProfilePath $browserProfile
            }
            catch { $cleanupFailures += "Could not stop owned $Browser profile process $($process.ProcessId): $($_.Exception.Message)" }
        }
        Start-Sleep -Milliseconds 200
        $remaining = Get-ProfileBrowserProcesses $browserProfile
        if ($remaining.Count -gt 0) {
            $cleanupFailures += "Owned $Browser processes remain for the test profile: $(@($remaining | ForEach-Object ProcessId) -join ', ')."
        }
    }
    catch { $cleanupFailures += "Could not verify owned $Browser process cleanup: $($_.Exception.Message)" }

    if ($null -ne $testFailure -and (Test-Path -LiteralPath $diagnosticsFile)) {
        Write-Output 'DIAGNOSTICS: UIA and selection trace from failed browser hover test:'
        Get-Content -LiteralPath $diagnosticsFile
    }
    if (Test-Path -LiteralPath $testRoot) {
        try { Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailures += "Could not remove test-owned files/profile '$testRoot': $($_.Exception.Message)" }
    }
}
if ($null -ne $testFailure -or $cleanupFailures.Count -gt 0) {
    $failureParts = @()
    if ($null -ne $testFailure) { $failureParts += "TEST FAILURE: $($testFailure.Exception.Message)" }
    if ($cleanupFailures.Count -gt 0) { $failureParts += "CLEANUP FAILURE: $($cleanupFailures -join [Environment]::NewLine)" }
    throw ($failureParts -join [Environment]::NewLine)
}
Write-Output $passMessage

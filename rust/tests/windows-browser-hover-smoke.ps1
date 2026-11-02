[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 30,

    [ValidateRange(15, 180)]
    [int] $CaptureTimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') {
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

$edgeCandidates = @(@(
    (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'),
    (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe')
) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) })
if ($edgeCandidates.Count -eq 0) {
    throw 'EDGE_NOT_INSTALLED: Microsoft Edge was not found in the runner image.'
}
$edgePath = $edgeCandidates[0]

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Browser-Hover-' + [Guid]::NewGuid().ToString('N'))
$edgeProfile = Join-Path $testRoot 'edge-profile'
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$saveFolder = Join-Path $testRoot 'captures'
$htmlFile = Join-Path $testRoot 'surface.html'
$uiaProbeFile = Join-Path $testRoot 'uia-probe.ps1'
$uiaProbeResult = Join-Path $testRoot 'uia-result.json'
$uiaProbeStdout = Join-Path $testRoot 'uia-probe.stdout.log'
$uiaProbeStderr = Join-Path $testRoot 'uia-probe.stderr.log'
$diagnosticsFile = Join-Path $testRoot 'uia-selection-diagnostics.log'
$shotKit = $null
$edge = $null
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
function Get-EdgeProcessById {
    param([int] $ProcessId)
    Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId" -ErrorAction Stop
}
function Get-ProfileEdgeProcesses {
    param([string] $ProfilePath)
    $escapedProfile = [Regex]::Escape($ProfilePath)
    @(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction Stop |
        Where-Object { $_.CommandLine -and $_.CommandLine -match $escapedProfile })
}
function Stop-OwnedEdgeProcess {
    param([int] $ProcessId, [string] $ProfilePath)
    $current = Get-EdgeProcessById $ProcessId
    if ($null -eq $current) { return }
    if ($current.Name -ne 'msedge.exe' -or -not $current.CommandLine -or
        $current.CommandLine -notmatch [Regex]::Escape($ProfilePath)) {
        throw "Refusing to stop PID ${ProcessId}: current process identity does not match the test-owned Edge profile."
    }
    $processHandle = $null
    try { $processHandle = [Diagnostics.Process]::GetProcessById($ProcessId) }
    catch {
        if ($null -eq (Get-EdgeProcessById $ProcessId)) { return }
        throw
    }
    try {
        $confirmed = Get-EdgeProcessById $ProcessId
        if ($null -eq $confirmed) { return }
        if ($confirmed.Name -ne 'msedge.exe' -or -not $confirmed.CommandLine -or
            $confirmed.CommandLine -notmatch [Regex]::Escape($ProfilePath) -or
            $confirmed.CreationDate -ne $current.CreationDate) {
            throw "PID $ProcessId changed identity during cleanup; refusing to stop the replacement process."
        }
        try { $processHandle.Kill() }
        catch {
            if ($processHandle.HasExited -or $null -eq (Get-EdgeProcessById $ProcessId)) { return }
            throw
        }
        if (-not $processHandle.WaitForExit(3000) -and -not $processHandle.HasExited) {
            throw "Test-owned Edge PID $ProcessId remains after termination was requested."
        }
    }
    finally { $processHandle.Dispose() }
}
try {
    New-Item -ItemType Directory -Path $testRoot, $edgeProfile, $appData, $localAppData, $saveFolder -Force | Out-Null
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
    border-radius: 0; appearance: none; background: transparent;
  }
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
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $windowHandle = [IntPtr]([Int64]$args[0])
    $condition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty,
        'ShotKit UIA hover target'
    )
    # Accessibility is requested on Edge's command line, but its renderer and
    # UIA provider can become available after the top-level HWND. Retry only
    # this transient absence; the parent process enforces the hard 15s limit.
    $deadline = [DateTime]::UtcNow.AddSeconds(12)
    $target = $null
    do {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($windowHandle)
        if ($null -ne $root) {
            $target = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
        }
        if ($null -eq $target -and [DateTime]::UtcNow -lt $deadline) {
            Start-Sleep -Milliseconds 200
        }
    } while ($null -eq $target -and [DateTime]::UtcNow -lt $deadline)
    if ($null -eq $root) { throw 'UIA ElementFromHandle returned null throughout provider initialization.' }
    if ($null -eq $target) { throw 'Named target remained absent from Edge UIA Control View after provider initialization wait.' }
    $current = $target.Current
    $bounds = $current.BoundingRectangle
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
    [IO.File]::WriteAllText($args[1], $result, [Text.UTF8Encoding]::new($false))
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
    $edgeArgs = @(
        "--user-data-dir=`"$edgeProfile`"",
        '--no-first-run',
        '--disable-background-networking',
        '--disable-sync',
        '--disable-extensions',
        '--disable-component-update',
        '--disable-features=msEdgeSidebarV2,msEdgeDiscover,msSmartScreenProtection',
        '--force-renderer-accessibility',
        '--force-device-scale-factor=1',
        "--app=`"$fileUri`"",
        '--window-size=1000,720'
    ) -join ' '
    $edge = Start-Process -FilePath $edgePath -ArgumentList $edgeArgs -WorkingDirectory $testRoot -PassThru

    $browserWindow = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the owned Edge app window' -OwnedProcess $edge -Condition {
        Get-ProcessWindows $edge | Where-Object { $_.Visible -and $_.ClassName -eq 'Chrome_WidgetWin_1' } | Select-Object -First 1
    }
    if (-not [ShotKitBrowserHoverSmoke.Native]::SetForegroundWindow($browserWindow.Handle)) {
        throw 'Could not foreground the test-owned Edge app window.'
    }

    $targetName = 'ShotKit UIA hover target'
    $probeArgs = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" "{1}" "{2}"' -f `
        $uiaProbeFile, $browserWindow.Handle.ToInt64(), $uiaProbeResult
    $uiaProbe = Start-Process -FilePath $windowsPowerShell -ArgumentList $probeArgs -WorkingDirectory $testRoot `
        -WindowStyle Hidden -RedirectStandardOutput $uiaProbeStdout -RedirectStandardError $uiaProbeStderr -PassThru
    $probeDeadline = [DateTime]::UtcNow.AddSeconds(15)
    while (-not (Test-Path -LiteralPath $uiaProbeResult)) {
        if ($uiaProbe.HasExited) {
            $probeError = if (Test-Path -LiteralPath $uiaProbeStderr) { [IO.File]::ReadAllText($uiaProbeStderr).Trim() } else { '' }
            if ([string]::IsNullOrWhiteSpace($probeError)) { $probeError = 'No UIA result was written.' }
            throw "UIA_PRECHECK_FAILED: $probeError"
        }
        if ([DateTime]::UtcNow -ge $probeDeadline) {
            try { $uiaProbe.Kill(); [void]$uiaProbe.WaitForExit(3000) } catch { }
            throw 'UIA_PRECHECK_TIMEOUT: Edge UI Automation did not return within 15 seconds.'
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
        throw "UIA_PRECHECK_FAILED: Edge returned invalid or undersized bounds ($($bounds.X),$($bounds.Y),$($bounds.Width),$($bounds.Height))."
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
    if ($targetWidth -ne 280 -or $targetHeight -ne 180) {
        throw "UIA_PRECHECK_FAILED: rounded screen bounds are ${targetWidth}x${targetHeight}; strict test requires 280x180."
    }
    if ($targetLeft -lt $desktopLeft -or $targetTop -lt $desktopTop -or
        $targetRight -gt $desktopLeft + $desktopWidth -or
        $targetBottom -gt $desktopTop + $desktopHeight) {
        throw 'UIA_PRECHECK_FAILED: target bounds extend outside the interactive desktop.'
    }
    Write-Output "MILESTONE: UIA named button confirmed at ${targetWidth}x${targetHeight} screen pixels."
    $centerX = $targetLeft + [int]($targetWidth / 2)
    $centerY = $targetTop + [int]($targetHeight / 2)

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
        throw 'Could not activate the owned Edge app before requesting selection.'
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
    [ShotKitBrowserHoverSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    $mouseDown = $true
    Start-Sleep -Milliseconds 120
    [ShotKitBrowserHoverSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    $mouseDown = $false

    $png = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'the auto-saved browser target PNG' -OwnedProcess $shotKit -Condition {
        Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Select-Object -First 1
    }
    $pngs = @(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File)
    if ($pngs.Count -ne 1) { throw "Expected exactly one captured PNG, found $($pngs.Count)." }

    $bitmap = [Drawing.Bitmap]::new($png.FullName)
    try {
        if ($bitmap.Width -ne $targetWidth -or $bitmap.Height -ne $targetHeight) {
            throw "CAPTURE_BOUNDS_MISMATCH: got $($bitmap.Width)x$($bitmap.Height), UIA target is ${targetWidth}x${targetHeight}."
        }
        $samples = @(
            @{ X = 70; Y = 45; R = 220; G = 35; B = 45; Name = 'top-left red' },
            @{ X = 210; Y = 45; R = 30; G = 190; B = 65; Name = 'top-right green' },
            @{ X = 70; Y = 135; R = 35; G = 70; B = 220; Name = 'bottom-left blue' },
            @{ X = 210; Y = 135; R = 235; G = 205; B = 35; Name = 'bottom-right yellow' }
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
    $passMessage = "PASS: Edge UIA name='$targetName', HWND=$($browserWindow.Handle), screen=($targetLeft,$targetTop,${targetWidth}x${targetHeight}); real hover-click captured exact dimensions and four expected pixels."
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
    if ($null -ne $edge) {
        try {
            $ownedEdge = Get-EdgeProcessById $edge.Id
            if ($null -eq $ownedEdge) {
                Write-Output "CLEANUP: Edge launcher PID $($edge.Id) is verified absent."
            } elseif ($ownedEdge.Name -eq 'msedge.exe' -and $ownedEdge.CommandLine -and
                $ownedEdge.CommandLine -match [Regex]::Escape($edgeProfile)) {
                # Start-Process returned a Process object for this exact process;
                # Kill uses its process handle instead of re-opening a bare PID.
                if (-not $edge.HasExited) {
                    $edge.Kill()
                    if (-not $edge.WaitForExit(3000) -and -not $edge.HasExited) {
                        throw "Owned Edge launcher PID $($edge.Id) remains after termination was requested."
                    }
                }
            } else {
                Write-Output "CLEANUP: Edge launcher PID $($edge.Id) now belongs to another process; it was left untouched."
            }
        }
        catch { $cleanupFailures += "Could not stop test-owned Edge process tree: $($_.Exception.Message)" }
        $edge.Dispose()
    }
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    $env:SHOTKIT_UIA_DIAGNOSTICS_PATH = $oldDiagnosticsPath

    # Only stop processes whose current identity still matches this unique
    # profile. A process disappearing between enumeration and Stop-Process is
    # accepted only after a fresh PID query confirms absence.
    try {
        $stragglers = Get-ProfileEdgeProcesses $edgeProfile
        foreach ($process in $stragglers) {
            try {
                Stop-OwnedEdgeProcess -ProcessId $process.ProcessId -ProfilePath $edgeProfile
            }
            catch { $cleanupFailures += "Could not stop owned Edge profile process $($process.ProcessId): $($_.Exception.Message)" }
        }
        Start-Sleep -Milliseconds 200
        $remaining = Get-ProfileEdgeProcesses $edgeProfile
        if ($remaining.Count -gt 0) {
            $cleanupFailures += "Owned Edge processes remain for the test profile: $(@($remaining | ForEach-Object ProcessId) -join ', ')."
        }
    }
    catch { $cleanupFailures += "Could not verify owned Edge process cleanup: $($_.Exception.Message)" }

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

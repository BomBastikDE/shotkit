[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 30,

    [ValidateRange(30, 180)]
    [int] $CaptureTimeoutSeconds = 90
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.' }
if (-not [Environment]::UserInteractive) { throw 'NO_INTERACTIVE_DESKTOP: the GitHub runner is not interactive.' }
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

if (-not ('ShotKitBrowserScrollSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace ShotKitBrowserScrollSmoke {
    public struct Point { public int X; public int Y; }
    public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }
    public sealed class WindowInfo {
        public IntPtr Handle;
        public string ClassName;
        public string Title;
        public bool Visible;
        public uint ProcessId;
        public int[] ClientBounds;
    }
    public sealed class ButtonInfo {
        public IntPtr Handle;
        public int ControlId;
        public string Text;
    }
    public static class Native {
        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);
        private delegate bool EnumChildProc(IntPtr hwnd, IntPtr lParam);
        [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool CloseDesktop(IntPtr desktop);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder info, uint length, out uint needed);
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern int SendMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr lParam);
        [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
        [DllImport("user32.dll")] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
        public static IntPtr ForegroundWindowHandle() { return GetForegroundWindow(); }
        public static uint ProcessIdForWindow(IntPtr hwnd) { uint pid; GetWindowThreadProcessId(hwnd, out pid); return pid; }
        public static IntPtr EnterPhysicalDpiContext() { return SetThreadDpiAwarenessContext(new IntPtr(-4)); }
        public static void RestoreDpiContext(IntPtr previous) { if (previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous); }
        [DllImport("user32.dll", SetLastError=true)] public static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll", SetLastError=true)] public static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
        [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll", SetLastError=true)] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool GetCursorPos(out Point point);
        public static bool CursorPosition(out int x, out int y) {
            Point point; bool ok = GetCursorPos(out point); x = point.X; y = point.Y; return ok;
        }
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);
        [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
        public static string InputDesktopName() {
            IntPtr desktop = OpenInputDesktop(0, false, 0x0001);
            if (desktop == IntPtr.Zero) return null;
            try {
                StringBuilder name = new StringBuilder(256); uint needed;
                if (!GetUserObjectInformationW(desktop, 2, name, (uint)(name.Capacity * 2), out needed)) return null;
                return name.ToString();
            } finally { CloseDesktop(desktop); }
        }
        private static string Text(IntPtr hwnd) {
            StringBuilder text = new StringBuilder(512); GetWindowTextW(hwnd, text, text.Capacity); return text.ToString();
        }
        public static string WindowText(IntPtr hwnd) {
            StringBuilder all = new StringBuilder(Text(hwnd));
            EnumChildProc callback = delegate(IntPtr child, IntPtr ignored) {
                string text = Text(child);
                if (!String.IsNullOrWhiteSpace(text)) all.Append(" ").Append(text);
                return true;
            };
            EnumChildWindows(hwnd, callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            return all.ToString();
        }
        public static ButtonInfo[] VisibleButtons(IntPtr dialog) {
            List<ButtonInfo> buttons = new List<ButtonInfo>();
            EnumChildProc callback = delegate(IntPtr child, IntPtr ignored) {
                if (!IsWindowVisible(child)) return true;
                StringBuilder cls = new StringBuilder(128); GetClassNameW(child, cls, cls.Capacity);
                if (!String.Equals(cls.ToString(), "Button", StringComparison.OrdinalIgnoreCase)) return true;
                StringBuilder text = new StringBuilder(256); GetWindowTextW(child, text, text.Capacity);
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
        public static WindowInfo[] WindowsForProcess(int processId) {
            List<WindowInfo> windows = new List<WindowInfo>();
            EnumWindows(delegate(IntPtr hwnd, IntPtr ignored) {
                uint owner; GetWindowThreadProcessId(hwnd, out owner);
                if (owner != (uint)processId) return true;
                StringBuilder cls = new StringBuilder(256); GetClassNameW(hwnd, cls, cls.Capacity);
                windows.Add(new WindowInfo { Handle=hwnd, ClassName=cls.ToString(), Title=Text(hwnd), Visible=IsWindowVisible(hwnd), ProcessId=owner });
                return true;
            }, IntPtr.Zero);
            return windows.ToArray();
        }
        public static int[] ClientBounds(IntPtr hwnd) {
            Rect rect;
            if (!GetClientRect(hwnd, out rect)) throw new InvalidOperationException("GetClientRect failed.");
            Point origin = new Point { X=rect.Left, Y=rect.Top };
            Point end = new Point { X=rect.Right, Y=rect.Bottom };
            if (!ClientToScreen(hwnd, ref origin) || !ClientToScreen(hwnd, ref end))
                throw new InvalidOperationException("ClientToScreen failed.");
            return new int[] { origin.X, origin.Y, end.X, end.Y };
        }
        public static WindowInfo[] ChildWindows(IntPtr parent) {
            List<WindowInfo> windows = new List<WindowInfo>();
            EnumChildProc callback = delegate(IntPtr hwnd, IntPtr ignored) {
                try {
                    uint owner; GetWindowThreadProcessId(hwnd, out owner);
                    StringBuilder cls = new StringBuilder(256); GetClassNameW(hwnd, cls, cls.Capacity);
                    windows.Add(new WindowInfo {
                        Handle=hwnd, ClassName=cls.ToString(), Title=Text(hwnd),
                        Visible=IsWindowVisible(hwnd), ProcessId=owner, ClientBounds=ClientBounds(hwnd)
                    });
                } catch { }
                return true;
            };
            EnumChildWindows(parent, callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            return windows.ToArray();
        }
    }
}
'@
}

$inputDesktop = [ShotKitBrowserScrollSmoke.Native]::InputDesktopName()
if ([string]::IsNullOrWhiteSpace($inputDesktop)) { throw 'NO_INTERACTIVE_DESKTOP: Windows could not open the input desktop.' }
try { Add-Type -AssemblyName System.Drawing }
catch { throw "IMAGE_RUNTIME_UNAVAILABLE: System.Drawing could not be loaded: $($_.Exception.Message)" }

$edgeCandidates = @(@(
    (Join-Path ${env:ProgramFiles(x86)} 'Microsoft\Edge\Application\msedge.exe'),
    (Join-Path $env:ProgramFiles 'Microsoft\Edge\Application\msedge.exe')
) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) })
if ($edgeCandidates.Count -eq 0) { throw 'EDGE_NOT_INSTALLED: Microsoft Edge was not found.' }
$edgePath = $edgeCandidates[0]
$powershellExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
if (-not (Test-Path -LiteralPath $powershellExe -PathType Leaf)) { throw 'Windows PowerShell 5.1 was not found.' }

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Browser-Scroll-' + [Guid]::NewGuid().ToString('N'))
$edgeProfile = Join-Path $testRoot 'edge-profile'
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$saveFolder = Join-Path $testRoot 'captures'
$htmlFile = Join-Path $testRoot 'scroll-surface.html'
$shotKit = $null
$edge = $null
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$passMessage = $null
$testFailure = $null
$cleanupFailures = @()
$mouseDown = $false
$originalCursorX = 0
$originalCursorY = 0
$previousDpiContext = [ShotKitBrowserScrollSmoke.Native]::EnterPhysicalDpiContext()
if ($previousDpiContext -eq [IntPtr]::Zero) { throw 'Could not enable per-monitor DPI awareness for screen-coordinate measurements.' }
$haveOriginalCursor = [ShotKitBrowserScrollSmoke.Native]::CursorPosition([ref]$originalCursorX, [ref]$originalCursorY)
if (-not $haveOriginalCursor) {
    [ShotKitBrowserScrollSmoke.Native]::RestoreDpiContext($previousDpiContext)
    throw 'Could not save the original system cursor position.'
}

function Get-ProcessWindows {
    param([System.Diagnostics.Process] $OwnedProcess)
    [ShotKitBrowserScrollSmoke.Native]::WindowsForProcess($OwnedProcess.Id)
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
    if ($null -ne $dialog) { throw "Unexpected ShotKit dialog. Title='$($dialog.Title)' HWND=$($dialog.Handle)." }
}
function Wait-ForCondition {
    param([scriptblock] $Condition, [int] $TimeoutSeconds, [string] $Description,
        [System.Diagnostics.Process] $OwnedProcess, [string[]] $AllowedDialogTitles = @())
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($null -ne $OwnedProcess) {
            if ($OwnedProcess.HasExited) { throw "Owned process '$($OwnedProcess.ProcessName)' exited before $Description (exit $($OwnedProcess.ExitCode))." }
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
        $buttons = @([ShotKitBrowserScrollSmoke.Native]::VisibleButtons($Dialog.Handle))
        $button = $buttons | Where-Object { $ExpectedTexts -contains (($_.Text -replace '&', '').Trim()) } | Select-Object -First 1
        if ($null -ne $button) {
            [ShotKitBrowserScrollSmoke.Native]::ClickButton($button.Handle)
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
function Get-ProfileEdgeProcesses {
    $escapedProfile = [Regex]::Escape($edgeProfile)
    @(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction Stop |
        Where-Object { $_.CommandLine -and $_.CommandLine -match $escapedProfile })
}
function Stop-ProfileEdgeProcess {
    param($ProcessIdentity)
    $ProcessId = [int]$ProcessIdentity.ProcessId
    $currentIdentity = Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId" -ErrorAction Stop
    if ($null -eq $currentIdentity) { return }
    # The original process may have exited and Windows may already have reused its PID.
    # Treat that as successful cleanup, without ever touching the replacement.
    if ($currentIdentity.CreationDate -ne $ProcessIdentity.CreationDate) { return }
    if ($currentIdentity.Name -ne 'msedge.exe' -or -not $currentIdentity.CommandLine -or
        $currentIdentity.CommandLine -notmatch [Regex]::Escape($edgeProfile)) {
        throw "Refusing to stop PID ${ProcessId}: it no longer identifies the test Edge profile."
    }
    $process = $null
    try { $process = [Diagnostics.Process]::GetProcessById($ProcessId) }
    catch {
        $latest = Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId" -ErrorAction Stop
        if ($null -eq $latest -or $latest.CreationDate -ne $ProcessIdentity.CreationDate) { return }
        throw
    }
    try {
        $confirmed = Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId" -ErrorAction Stop
        if ($null -eq $confirmed) { return }
        if ($confirmed.CreationDate -ne $ProcessIdentity.CreationDate) { return }
        if ($confirmed.Name -ne 'msedge.exe' -or
            -not $confirmed.CommandLine -or $confirmed.CommandLine -notmatch [Regex]::Escape($edgeProfile)) {
            throw "Refusing to stop PID ${ProcessId}: the original creation identity does not match the test Edge profile."
        }
        try { $process.Kill() }
        catch {
            $latest = Get-CimInstance Win32_Process -Filter "ProcessId=$ProcessId" -ErrorAction Stop
            if ($process.HasExited -or $null -eq $latest -or $latest.CreationDate -ne $ProcessIdentity.CreationDate) { return }
            throw
        }
        if (-not $process.WaitForExit(3000) -and -not $process.HasExited) { throw "Owned Edge PID $ProcessId did not exit." }
    }
    finally { $process.Dispose() }
}

try {
    New-Item -ItemType Directory -Path $testRoot, $edgeProfile, $appData, $localAppData, $saveFolder -Force | Out-Null
    $pageHeight = 2400
    $pageWidth = 2000
    $html = @'
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>ShotKit Browser Scroll Surface Loading</title>
<style>html,body{margin:0;padding:0}html{overflow-x:hidden}body{width:2000px;height:2400px}canvas{display:block;width:2000px;height:2400px}</style></head>
<body><canvas id="surface" width="2000" height="2400"></canvas>
<script>
(() => {
  const width = 2000, height = 2400;
  const canvas = document.getElementById('surface');
  const ctx = canvas.getContext('2d', { alpha: false, willReadFrequently: false });
  const image = ctx.createImageData(width, height);
  const p = image.data;
  for (let y = 0; y < height; y++) {
    const bandR = Math.floor(y / 19) * 53;
    const bandG = Math.floor(y / 23) * 97;
    const bandB = Math.floor(y / 11) * 41;
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      p[i] = (x * 29 + y * 17 + bandR) & 255;
      p[i + 1] = (x * 7 + y * 31 + bandG) & 255;
      p[i + 2] = (x * 19 + y * 11 + bandB) & 255;
      p[i + 3] = 255;
    }
  }
  ctx.putImageData(image, 0, 0);
  document.title = `ShotKit Browser Scroll Surface READY inner=${innerWidth}x${innerHeight} scroll=${document.scrollingElement.scrollHeight} dpr=${devicePixelRatio} offset=${scrollX},${scrollY}`;
})();
</script></body></html>
'@
    [IO.File]::WriteAllText($htmlFile, $html, [Text.UTF8Encoding]::new($false))

    $desktopWidth = [ShotKitBrowserScrollSmoke.Native]::GetSystemMetrics(78)
    $desktopHeight = [ShotKitBrowserScrollSmoke.Native]::GetSystemMetrics(79)
    if ($desktopWidth -lt 1000 -or $desktopHeight -lt 720) { throw "INTERACTIVE_DESKTOP_TOO_SMALL: ${desktopWidth}x${desktopHeight}." }

    $fileUri = ([Uri]::new($htmlFile)).AbsoluteUri
    $edgeArgs = @(
        "--user-data-dir=`"$edgeProfile`"", '--no-first-run', '--disable-background-networking',
        '--disable-sync', '--disable-extensions', '--disable-component-update',
        '--disable-features=msEdgeSidebarV2,msEdgeDiscover,msSmartScreenProtection',
        '--disable-gpu', '--force-device-scale-factor=1', "--app=`"$fileUri`"", '--window-size=1000,720'
    ) -join ' '
    $edge = Start-Process -FilePath $edgePath -ArgumentList $edgeArgs -WorkingDirectory $testRoot -PassThru

    $browserWindow = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'owned Edge local scroll surface' -OwnedProcess $null -Condition {
        $owned = Get-ProfileEdgeProcesses
        foreach ($process in $owned) {
            [ShotKitBrowserScrollSmoke.Native]::WindowsForProcess([int]$process.ProcessId) |
                Where-Object { $_.Visible -and $_.ClassName -eq 'Chrome_WidgetWin_1' -and $_.Title -like '*READY inner=*scroll=2400*' } |
                Select-Object -First 1
        }
    }
    if (-not [ShotKitBrowserScrollSmoke.Native]::SetForegroundWindow($browserWindow.Handle)) { throw 'Could not foreground the owned Edge surface.' }
    $foreground = Wait-ForCondition -TimeoutSeconds 3 -Description 'owned Edge foreground HWND activation' -OwnedProcess $null -Condition {
        $foregroundHwnd = [ShotKitBrowserScrollSmoke.Native]::ForegroundWindowHandle()
        if ($foregroundHwnd -eq $browserWindow.Handle -and
            [ShotKitBrowserScrollSmoke.Native]::ProcessIdForWindow($foregroundHwnd) -eq $browserWindow.ProcessId) { $true } else { $false }
    }
    if (-not $foreground) { throw 'EDGE_FOCUS_MISMATCH: foreground HWND is not the activated owned browser surface.' }
    $browserClient = [ShotKitBrowserScrollSmoke.Native]::ClientBounds($browserWindow.Handle)
    if ($browserWindow.Title -match 'inner=(\d+)x(\d+) scroll=(\d+) dpr=([\d.]+) offset=(-?\d+),(-?\d+)') {
        $innerWidth = [int]$Matches[1]
        $innerHeight = [int]$Matches[2]
        $scrollHeight = [int]$Matches[3]
        $deviceScale = [double]$Matches[4]
        $initialScrollX = [int]$Matches[5]
        $initialScrollY = [int]$Matches[6]
    } else {
        throw "EDGE_PAGE_NOT_READY: unexpected title '$($browserWindow.Title)'."
    }
    if ($innerWidth -ne 984 -or $innerHeight -ne 636) {
        throw "EDGE_VIEWPORT_SIZE_MISMATCH: this runner contract expects an exact 984x636 web viewport, got ${innerWidth}x${innerHeight}."
    }
    if ($scrollHeight -ne $pageHeight) { throw "EDGE_PAGE_GEOMETRY_MISMATCH: expected scrollHeight=$pageHeight, got $scrollHeight." }
    if ($deviceScale -ne 1.0 -or $initialScrollX -ne 0 -or $initialScrollY -ne 0) {
        throw "EDGE_COORDINATE_MISMATCH: deviceScale=$deviceScale initialScroll=($initialScrollX,$initialScrollY)."
    }
    $profilePids = @((Get-ProfileEdgeProcesses | ForEach-Object { [uint]$_.ProcessId }))
    $edgeChildWindows = @([ShotKitBrowserScrollSmoke.Native]::ChildWindows($browserWindow.Handle))
    $rendererCandidates = @($edgeChildWindows |
        Where-Object {
            $_.Visible -and $_.ClassName -eq 'Chrome_RenderWidgetHostHWND' -and
            $profilePids -contains $_.ProcessId -and $_.ClientBounds -and
            ($_.ClientBounds[2] - $_.ClientBounds[0]) -eq $innerWidth -and
            ($_.ClientBounds[3] - $_.ClientBounds[1]) -eq $innerHeight
        })
    $rendererCandidateSummary = @($edgeChildWindows |
        Where-Object { $_.ClassName -eq 'Chrome_RenderWidgetHostHWND' } |
        ForEach-Object {
            $width = if ($_.ClientBounds) { $_.ClientBounds[2] - $_.ClientBounds[0] } else { -1 }
            $height = if ($_.ClientBounds) { $_.ClientBounds[3] - $_.ClientBounds[1] } else { -1 }
            "hwnd=0x$($_.Handle.ToInt64().ToString('X')) pid=$($_.ProcessId) visible=$($_.Visible) client=${width}x${height}"
        })
    if ($rendererCandidates.Count -ne 1) {
        throw "EDGE_RENDERER_BOUNDS_UNVERIFIED: expected one visible profile-owned Chrome_RenderWidgetHostHWND at ${innerWidth}x${innerHeight}; found $($rendererCandidates.Count). Candidates: $($rendererCandidateSummary -join '; ')"
    }
    $rendererWindow = $rendererCandidates[0]
    $client = $rendererWindow.ClientBounds
    $clientWidth = $client[2] - $client[0]
    $clientHeight = $client[3] - $client[1]
    if ($clientWidth -ne $innerWidth -or $clientHeight -ne $innerHeight) {
        throw "EDGE_RENDERER_BOUNDS_MISMATCH: child client ${clientWidth}x${clientHeight}, DOM viewport ${innerWidth}x${innerHeight}."
    }
    if ($client[0] -lt $browserClient[0] -or $client[1] -lt $browserClient[1] -or
        $client[2] -gt $browserClient[2] -or $client[3] -gt $browserClient[3]) {
        throw "EDGE_RENDERER_OUTSIDE_ROOT_CLIENT: child HWND client rect $($client -join ',') is outside root client rect $($browserClient -join ',')."
    }
    $contentOffsetX = $client[0] - $browserClient[0]
    $contentOffsetY = $client[1] - $browserClient[1]
    Write-Output "MILESTONE: verified Edge renderer HWND=0x$($rendererWindow.Handle.ToInt64().ToString('X')) PID=$($rendererWindow.ProcessId), content origin=($($client[0]),$($client[1])) root-client offset=($contentOffsetX,$contentOffsetY), viewport=984x636; root client is $($browserClient[2]-$browserClient[0])x$($browserClient[3]-$browserClient[1])."

    $settingsPath = Join-Path (Join-Path $appData 'ShotKitRust') 'settings.json'
    New-Item -ItemType Directory -Path (Split-Path -Parent $settingsPath) -Force | Out-Null
    $settings = @{ version=1; save_folder=$saveFolder; open_editor=$false; copy_to_clipboard=$false; auto_save=$true; scroll_direction='down' } | ConvertTo-Json
    [IO.File]::WriteAllText($settingsPath, $settings, [Text.UTF8Encoding]::new($false))
    try {
        $env:APPDATA = $appData
        $env:LOCALAPPDATA = $localAppData
        $shotKit = Start-Process -FilePath $ExePath -WorkingDirectory $testRoot -PassThru
    }
    finally { $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData }

    $mainWindow = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'ShotKit main window' -OwnedProcess $shotKit -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
    }
    $welcome = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'ShotKit startup dialog' -OwnedProcess $shotKit -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -like '*Prototyp*' } | Select-Object -First 1
    }
    Click-DialogButton -OwnedProcess $shotKit -Dialog $welcome -ExpectedTexts @('OK') -Description 'startup acknowledgement' | Out-Null
    Wait-ForDialogClosed -OwnedProcess $shotKit -DialogHandle $welcome.Handle -Description 'startup dialog closure after its OK button was clicked'
    Assert-NoUnexpectedDialog $shotKit

    # Reacquire the actual top-level browser window after ShotKit's dialog has
    # closed; do not assume the earlier HWND remained the active foreground target.
    $browserWindow = Wait-ForCondition -TimeoutSeconds 5 -Description 'owned Edge window after startup dialog closure' -OwnedProcess $null -Condition {
        $owned = Get-ProfileEdgeProcesses
        foreach ($process in $owned) {
            [ShotKitBrowserScrollSmoke.Native]::WindowsForProcess([int]$process.ProcessId) |
                Where-Object { $_.Visible -and $_.ClassName -eq 'Chrome_WidgetWin_1' -and $_.Title -like '*READY inner=*scroll=2400*' } |
                Select-Object -First 1
        }
    }

    if (-not [ShotKitBrowserScrollSmoke.Native]::SetForegroundWindow($browserWindow.Handle)) { throw 'Could not restore Edge foreground before sending the scroll hotkey.' }
    $foreground = Wait-ForCondition -TimeoutSeconds 3 -Description 'Edge foreground HWND before F10' -OwnedProcess $null -Condition {
        $foregroundHwnd = [ShotKitBrowserScrollSmoke.Native]::ForegroundWindowHandle()
        if ($foregroundHwnd -eq $browserWindow.Handle -and
            [ShotKitBrowserScrollSmoke.Native]::ProcessIdForWindow($foregroundHwnd) -eq $browserWindow.ProcessId) { $true } else { $false }
    }
    if (-not $foreground) { throw 'EDGE_FOCUS_MISMATCH: foreground HWND changed before F10.' }
    [ShotKitBrowserScrollSmoke.Native]::keybd_event(0x11, 0, 0x0002, [UIntPtr]::Zero) # release stale Ctrl
    [ShotKitBrowserScrollSmoke.Native]::keybd_event(0x79, 0, 0, [UIntPtr]::Zero) # F10 down
    [ShotKitBrowserScrollSmoke.Native]::keybd_event(0x79, 0, 0x0002, [UIntPtr]::Zero) # F10 up
    $directionDialog = Wait-ForCondition -TimeoutSeconds 10 -Description 'scroll direction prompt' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Richtung' } | Select-Object -First 1
    }
    if (-not [ShotKitBrowserScrollSmoke.Native]::PostMessageW($directionDialog.Handle, 0x0111, [IntPtr]6, [IntPtr]::Zero)) { throw 'Could not select vertical downward scrolling.' }
    $overlay = Wait-ForCondition -TimeoutSeconds 10 -Description 'scroll selection overlay' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible } | Select-Object -First 1
    }
    if (-not [ShotKitBrowserScrollSmoke.Native]::SetForegroundWindow($overlay.Handle)) { throw 'Could not foreground selection overlay.' }
    if ([ShotKitBrowserScrollSmoke.Native]::ProcessIdForWindow($overlay.Handle) -ne [uint]$shotKit.Id) {
        throw 'SHOTKIT_FOCUS_MISMATCH: selection overlay is not owned by the test ShotKit process.'
    }
    $x0 = $client[0] + 100
    $y0 = $client[1] + 10
    $x1 = $x0 + 600
    $y1 = $y0 + 400
    if ($x1 -gt $client[2] -or $y1 -gt $client[3]) { throw 'Selected browser scroll rectangle does not fit in the client area.' }
    $desktopLeft = [ShotKitBrowserScrollSmoke.Native]::GetSystemMetrics(76)
    $desktopTop = [ShotKitBrowserScrollSmoke.Native]::GetSystemMetrics(77)
    if ($x0 -lt $desktopLeft -or $y0 -lt $desktopTop -or
        $x1 -gt $desktopLeft + $desktopWidth -or $y1 -gt $desktopTop + $desktopHeight) {
        throw 'Selected browser scroll rectangle extends outside the interactive desktop.'
    }
    if (-not [ShotKitBrowserScrollSmoke.Native]::SetCursorPos($x0, $y0)) { throw 'Could not position selection start.' }
    Start-Sleep -Milliseconds 150
    [ShotKitBrowserScrollSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    $mouseDown = $true
    if (-not [ShotKitBrowserScrollSmoke.Native]::SetCursorPos($x1, $y1)) { throw 'Could not position selection end.' }
    Start-Sleep -Milliseconds 200
    [ShotKitBrowserScrollSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    $mouseDown = $false

    $png = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'auto-saved browser scroll PNG' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Select-Object -First 1
    }
    $completion = Wait-ForCondition -TimeoutSeconds 20 -Description 'normal end-of-scroll notification' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Aufnahme beendet' } | Select-Object -First 1
    }
    $completionText = [ShotKitBrowserScrollSmoke.Native]::WindowText($completion.Handle)
    if ($completionText -notlike '*Ende des scrollbaren Bereichs erreicht.*') {
        throw "Scroll did not report reaching the actual end of the browser page. Dialog='$completionText'"
    }
    $pngs = @(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File)
    if ($pngs.Count -ne 1) { throw "Expected one auto-saved PNG, found $($pngs.Count)." }

    $bitmap = [Drawing.Bitmap]::new($png.FullName)
    try {
        $selectedWidth = 600
        $selectedHeight = 400
        $expectedHeight = $scrollHeight - $innerHeight + $selectedHeight
        if ($bitmap.Width -ne $selectedWidth -or $bitmap.Height -ne $expectedHeight) {
            throw "SCROLL_BOUNDS_MISMATCH: got $($bitmap.Width)x$($bitmap.Height); expected ${selectedWidth}x${expectedHeight} from viewport/document geometry."
        }
        foreach ($y in 0..($expectedHeight - 1)) {
            $sourceY = $y + 10
            foreach ($x in @(10, 97, 299, 588)) {
                $sourceX = $x + 100
                $expectedR = ($sourceX * 29 + $sourceY * 17 + [int][Math]::Floor($sourceY / 19) * 53) % 256
                $expectedG = ($sourceX * 7 + $sourceY * 31 + [int][Math]::Floor($sourceY / 23) * 97) % 256
                $expectedB = ($sourceX * 19 + $sourceY * 11 + [int][Math]::Floor($sourceY / 11) * 41) % 256
                $actual = $bitmap.GetPixel($x, $y)
                if ($actual.R -ne $expectedR -or $actual.G -ne $expectedG -or $actual.B -ne $expectedB) {
                    throw "SCROLL_PIXEL_MISMATCH: output ($x,$y) maps to page ($sourceX,$sourceY), got $($actual.R),$($actual.G),$($actual.B), expected $expectedR,$expectedG,$expectedB."
                }
            }
        }
    }
    finally { $bitmap.Dispose() }
    Write-Output "MILESTONE: saved scroll image validated at $($png.Length) bytes with $expectedHeight deterministic rows and four pixel tracks."
    $passMessage = "PASS: real Edge page scrolled to bottom and auto-saved ${selectedWidth}x${expectedHeight}; every sampled pixel matches its deterministic source coordinate."
}
catch { $testFailure = $_ }
finally {
    if ($mouseDown) {
        try { [ShotKitBrowserScrollSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) }
        catch { $cleanupFailures += "Could not release test-owned mouse button: $($_.Exception.Message)" }
    }
    if ($haveOriginalCursor -and -not [ShotKitBrowserScrollSmoke.Native]::SetCursorPos($originalCursorX, $originalCursorY)) {
        $cleanupFailures += "Could not restore cursor position to ($originalCursorX,$originalCursorY)."
    }
    if ($null -ne $shotKit) {
        try {
            if (-not $shotKit.HasExited) {
                $shotKit.Kill()
                if (-not $shotKit.WaitForExit(5000) -or -not $shotKit.HasExited) { throw 'ShotKit did not exit after termination was requested.' }
            }
        }
        catch { $cleanupFailures += "Could not stop test-owned ShotKit process: $($_.Exception.Message)" }
        $shotKit.Dispose()
    }
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(8)
        do {
            $owned = Get-ProfileEdgeProcesses
            foreach ($process in $owned) {
                try { Stop-ProfileEdgeProcess -ProcessIdentity $process }
                catch { $cleanupFailures += "Could not stop owned Edge PID $($process.ProcessId): $($_.Exception.Message)" }
            }
            Start-Sleep -Milliseconds 200
        } while ((Get-ProfileEdgeProcesses).Count -gt 0 -and [DateTime]::UtcNow -lt $deadline)
        $remaining = Get-ProfileEdgeProcesses
        if ($remaining.Count -gt 0) { $cleanupFailures += "Owned Edge profile processes remain: $(@($remaining | ForEach-Object ProcessId) -join ', ')." }
    }
    catch { $cleanupFailures += "Could not verify owned Edge process cleanup: $($_.Exception.Message)" }
    if ($null -ne $edge) { $edge.Dispose() }
    if (Test-Path -LiteralPath $testRoot) {
        try { Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailures += "Could not remove test-owned files/profile '$testRoot': $($_.Exception.Message)" }
    }
    [ShotKitBrowserScrollSmoke.Native]::RestoreDpiContext($previousDpiContext)
}
if ($null -ne $testFailure -or $cleanupFailures.Count -gt 0) {
    $parts = @()
    if ($null -ne $testFailure) { $parts += "TEST FAILURE: $($testFailure.Exception.Message)" }
    if ($cleanupFailures.Count -gt 0) { $parts += "CLEANUP FAILURE: $($cleanupFailures -join [Environment]::NewLine)" }
    throw ($parts -join [Environment]::NewLine)
}
Write-Output $passMessage

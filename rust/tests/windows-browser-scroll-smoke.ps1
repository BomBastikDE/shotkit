[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 30,

    [ValidateRange(30, 180)]
    [int] $CaptureTimeoutSeconds = 90,

    [switch] $FixedHeaderFooter
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
. (Join-Path $PSScriptRoot 'edge-profile-cleanup.ps1')
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') { throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.' }
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
$edgeOwnedHandles = New-Object System.Collections.ArrayList
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
function Format-DialogDetails {
    param($Dialog)
    $text = [ShotKitBrowserScrollSmoke.Native]::WindowText($Dialog.Handle)
    "Title='$($Dialog.Title)' HWND=0x$($Dialog.Handle.ToInt64().ToString('X')) PID=$($Dialog.ProcessId) Class='$($Dialog.ClassName)' Text='$text'"
}
function Assert-NoUnexpectedDialog {
    param([System.Diagnostics.Process] $OwnedProcess, [string[]] $AllowedTitles = @())
    $dialog = Get-ProcessDialogs $OwnedProcess |
        Where-Object { $AllowedTitles -notcontains $_.Title } |
        Select-Object -First 1
    if ($null -ne $dialog) { throw "Unexpected ShotKit dialog: $(Format-DialogDetails $dialog)" }
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
    $candidates = @(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction Stop |
        Where-Object { $_.CommandLine -and $_.CommandLine -match $escapedProfile })
    foreach ($candidate in $candidates) {
        $known = $edgeOwnedHandles | Where-Object {
            $_.ProcessId -eq [int]$candidate.ProcessId -and $_.CreationDate -eq $candidate.CreationDate
        } | Select-Object -First 1
        if ($null -ne $known) { continue }
        $handle = $null
        try { $handle = [Diagnostics.Process]::GetProcessById([int]$candidate.ProcessId) }
        catch {
            $latest = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.ProcessId)" -ErrorAction Stop
            if ($null -eq $latest -or $latest.CreationDate -ne $candidate.CreationDate) { continue }
            throw
        }
        try {
            $confirmed = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.ProcessId)" -ErrorAction Stop
            if ($null -eq $confirmed -or $confirmed.CreationDate -ne $candidate.CreationDate -or
                $confirmed.Name -ne 'msedge.exe' -or -not $confirmed.CommandLine -or
                $confirmed.CommandLine -notmatch $escapedProfile) { continue }
            if ($handle.ProcessName -ne 'msedge') { throw "Owned Edge process identity mismatch for PID $($candidate.ProcessId)." }
            [void]$edgeOwnedHandles.Add([pscustomobject]@{
                ProcessId=[int]$candidate.ProcessId; CreationDate=$candidate.CreationDate; Handle=$handle
            })
            $handle = $null
        }
        finally { if ($null -ne $handle) { $handle.Dispose() } }
    }
    return $candidates
}
function Stop-ProfileEdgeProcess {
    param($OwnedHandle)
    $process = $OwnedHandle.Handle
    $ProcessId = $OwnedHandle.ProcessId
    # This held Process handle was tied to the profile-verified process at discovery.
    # Killing through it cannot target a later process that reuses the numeric PID.
    if ($process.HasExited) { return }
    if ($process.ProcessName -ne 'msedge') { throw "Verified Edge handle for PID $ProcessId reports unexpected process name '$($process.ProcessName)'." }
    try { $process.Kill() }
    catch {
        if ($process.HasExited) { return }
        throw
    }
    if (-not $process.WaitForExit(3000) -and -not $process.HasExited) { throw "Owned Edge PID $ProcessId did not exit." }
}
function Write-ProfileEdgeCleanupDiagnostics {
    # Read-only snapshot: do not discover more kill targets or widen cleanup ownership.
    try {
        $escapedProfile = [Regex]::Escape($edgeProfile)
        $processes = @(Get-CimInstance Win32_Process -ErrorAction Stop)
        $related = @($processes | Where-Object {
            $candidate = $_
            ($candidate.CommandLine -and $candidate.CommandLine -match $escapedProfile) -or
                @($edgeOwnedHandles | Where-Object {
                    $_.ProcessId -eq [int]$candidate.ProcessId -and $_.CreationDate -eq $candidate.CreationDate
                }).Count -gt 0
        } | Select-Object Name, ProcessId, ParentProcessId, CreationDate, ExecutablePath, CommandLine)
        $known = @($edgeOwnedHandles | Select-Object ProcessId, CreationDate)
        Write-Output "DIAGNOSTIC: Edge cleanup snapshot for test-owned profile '$edgeProfile'; related processes match the exact profile path or a previously verified PID/creation date."
        Write-Output ([pscustomobject]@{
            CapturedAtUtc = [DateTime]::UtcNow.ToString('o')
            RemainingProfileRelatedProcesses = $related
            PreviouslyVerifiedProcessIdentities = $known
        } | ConvertTo-Json -Depth 4)
    }
    catch { Write-Output "DIAGNOSTIC: Could not inspect remaining profile-related processes: $($_.Exception.ToString())" }
}

$script:edgeTrace = [Collections.Generic.List[string]]::new()
$script:edgeTraceLast = ''
$script:edgeTraceReadFailed = $false
$script:edgeTraceClock = [Diagnostics.Stopwatch]::StartNew()
function Record-EdgeScrollTrace {
    # Diagnostic only: read the original profile-owned window's title without
    # moving focus, changing input, or returning data to Wait-ForCondition.
    try {
        $window = [ShotKitBrowserScrollSmoke.Native]::WindowsForProcess([int]$browserWindow.ProcessId) |
            Where-Object { $_.Handle -eq $browserWindow.Handle -and $_.Visible } | Select-Object -First 1
        if ($null -ne $window -and $window.Title -match 'offset=(-?\d+),(-?\d+) wheel=(\d+) scrollEvents=(\d+)') {
            $sample = "offset=$($Matches[1]),$($Matches[2]) wheel=$($Matches[3]) scrollEvents=$($Matches[4])"
            if ($sample -cne $script:edgeTraceLast -and $script:edgeTrace.Count -lt 256) {
                [void]$script:edgeTrace.Add("t=$($script:edgeTraceClock.ElapsedMilliseconds)ms $sample")
                $script:edgeTraceLast = $sample
            }
        }
    }
    catch { $script:edgeTraceReadFailed = $true }
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
  let wheelEvents = 0, scrollEvents = 0;
  const updateTitle = () => {
    document.title = `ShotKit Browser Scroll Surface READY inner=${innerWidth}x${innerHeight} scroll=${document.scrollingElement.scrollHeight} dpr=${devicePixelRatio} offset=${scrollX},${scrollY} wheel=${wheelEvents} scrollEvents=${scrollEvents}`;
  };
  window.addEventListener('resize', updateTitle, { passive: true });
  window.addEventListener('wheel', () => { wheelEvents++; updateTitle(); }, { passive: true, capture: true });
  window.addEventListener('scroll', () => { scrollEvents++; updateTitle(); }, { passive: true });
  updateTitle();
})();
</script></body></html>
'@
    if ($FixedHeaderFooter) {
        # Fixed bars are deliberately patterned horizontally so the production
        # matcher can identify them as stationary rows, like the unit fixture.
        $html = $html.Replace('</style>', 'header,footer{position:fixed;left:0;width:100%;height:40px;z-index:2}header{top:0;background:repeating-linear-gradient(90deg,#e81329 0 100px,#ffcc00 100px 200px)}footer{bottom:0;background:repeating-linear-gradient(90deg,#0876d1 0 100px,#17b978 100px 200px)}</style>')
        $html = $html.Replace('</body>', '<header></header><footer></footer></body>')
    }
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
    $settingsPath = Join-Path (Join-Path $appData 'ShotKitRust') 'settings.json'
    New-Item -ItemType Directory -Path (Split-Path -Parent $settingsPath) -Force | Out-Null
    $settings = @{ version=1; save_folder=$saveFolder; open_editor=$true; copy_to_clipboard=$false; auto_save=$true; scroll_direction='down' } | ConvertTo-Json
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
    Assert-NoStartupDialogs -TimeoutSeconds $StartupTimeoutSeconds -OwnedProcess $shotKit -GetWindows { Get-ProcessWindows $shotKit }
    Assert-NoUnexpectedDialog $shotKit

    # Reacquire the actual top-level browser window after ShotKit startup;
    # do not assume the earlier HWND remained the active foreground target.
    $browserWindow = Wait-ForCondition -TimeoutSeconds 5 -Description 'owned Edge window after ShotKit startup' -OwnedProcess $null -Condition {
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
    Write-Output 'MILESTONE: Edge activated after ShotKit startup; polling current DOM viewport against the owned renderer HWND.'

    $geometryTimeoutSeconds = [Math]::Min($StartupTimeoutSeconds, 15)
    $geometryDeadline = [DateTime]::UtcNow.AddSeconds($geometryTimeoutSeconds)
    $previousGeometryKey = $null
    $stableGeometrySamples = 0
    $lastGeometryDetail = 'no READY browser window observed'
    $geometryReady = $false
    while ([DateTime]::UtcNow -lt $geometryDeadline) {
        $candidateWindow = $null
        foreach ($process in (Get-ProfileEdgeProcesses)) {
            $candidateWindow = [ShotKitBrowserScrollSmoke.Native]::WindowsForProcess([int]$process.ProcessId) |
                Where-Object { $_.Visible -and $_.ClassName -eq 'Chrome_WidgetWin_1' -and $_.Title -like '*READY inner=*scroll=2400*' } |
                Select-Object -First 1
            if ($null -ne $candidateWindow) { break }
        }
        if ($null -ne $candidateWindow -and
            $candidateWindow.Title -match 'inner=(\d+)x(\d+) scroll=(\d+) dpr=([\d.]+) offset=(-?\d+),(-?\d+)') {
            $sampleInnerWidth = [int]$Matches[1]
            $sampleInnerHeight = [int]$Matches[2]
            $sampleScrollHeight = [int]$Matches[3]
            $sampleDeviceScale = [double]$Matches[4]
            $sampleScrollX = [int]$Matches[5]
            $sampleScrollY = [int]$Matches[6]
            $sampleRootClient = [ShotKitBrowserScrollSmoke.Native]::ClientBounds($candidateWindow.Handle)
            $sampleProfilePids = @((Get-ProfileEdgeProcesses | ForEach-Object { [uint]$_.ProcessId }))
            $sampleChildren = @([ShotKitBrowserScrollSmoke.Native]::ChildWindows($candidateWindow.Handle))
            $sampleRenderers = @($sampleChildren | Where-Object {
                $_.Visible -and $_.ClassName -eq 'Chrome_RenderWidgetHostHWND' -and
                $sampleProfilePids -contains $_.ProcessId -and $_.ClientBounds
            })
            $sampleSummary = @($sampleChildren | Where-Object { $_.ClassName -eq 'Chrome_RenderWidgetHostHWND' } | ForEach-Object {
                $w = if ($_.ClientBounds) { $_.ClientBounds[2] - $_.ClientBounds[0] } else { -1 }
                $h = if ($_.ClientBounds) { $_.ClientBounds[3] - $_.ClientBounds[1] } else { -1 }
                "hwnd=0x$($_.Handle.ToInt64().ToString('X')) pid=$($_.ProcessId) visible=$($_.Visible) client=${w}x${h}"
            }) -join '; '
            $exactRenderers = @($sampleRenderers | Where-Object {
                ($_.ClientBounds[2] - $_.ClientBounds[0]) -eq $sampleInnerWidth -and
                ($_.ClientBounds[3] - $_.ClientBounds[1]) -eq $sampleInnerHeight
            })
            $lastGeometryDetail = "DOM=${sampleInnerWidth}x${sampleInnerHeight} scroll=$sampleScrollHeight dpr=$sampleDeviceScale offset=($sampleScrollX,$sampleScrollY); profile-owned renderers: $sampleSummary"
            if ($exactRenderers.Count -eq 1) {
                $sampleRenderer = $exactRenderers[0]
                $sampleClient = $sampleRenderer.ClientBounds
                $insideRootClient = $sampleClient[0] -ge $sampleRootClient[0] -and
                    $sampleClient[1] -ge $sampleRootClient[1] -and
                    $sampleClient[2] -le $sampleRootClient[2] -and
                    $sampleClient[3] -le $sampleRootClient[3]
                if ($insideRootClient -and $sampleInnerWidth -ge 720 -and $sampleInnerHeight -ge 450 -and
                    $sampleScrollHeight -eq $pageHeight -and $sampleDeviceScale -eq 1.0 -and
                    $sampleScrollX -eq 0 -and $sampleScrollY -eq 0) {
                    $sampleKey = "{0}:{1}:{2}:{3}:{4}:{5}:{6}:{7}" -f `
                        $candidateWindow.Handle.ToInt64(), $sampleRenderer.Handle.ToInt64(),
                        $sampleRenderer.ProcessId, $sampleClient[0], $sampleClient[1],
                        $sampleClient[2], $sampleClient[3], $candidateWindow.Title
                    if ($sampleKey -eq $previousGeometryKey) { $stableGeometrySamples++ }
                    else { $previousGeometryKey = $sampleKey; $stableGeometrySamples = 1 }
                    if ($stableGeometrySamples -ge 3) {
                        $browserWindow = $candidateWindow
                        $rendererWindow = $sampleRenderer
                        $client = $sampleClient
                        $browserClient = $sampleRootClient
                        $innerWidth = $sampleInnerWidth
                        $innerHeight = $sampleInnerHeight
                        $scrollHeight = $sampleScrollHeight
                        $deviceScale = $sampleDeviceScale
                        $initialScrollX = $sampleScrollX
                        $initialScrollY = $sampleScrollY
                        $geometryReady = $true
                        break
                    }
                } else {
                    $stableGeometrySamples = 0
                    $previousGeometryKey = $null
                }
            } else {
                $stableGeometrySamples = 0
                $previousGeometryKey = $null
            }
        } else {
            $stableGeometrySamples = 0
            $previousGeometryKey = $null
            if ($null -ne $candidateWindow) { $lastGeometryDetail = "READY title not parseable: '$($candidateWindow.Title)'" }
        }
        Start-Sleep -Milliseconds 150
    }
    if (-not $geometryReady) {
        throw "EDGE_RENDERER_BOUNDS_UNVERIFIED: DOM viewport and profile-owned renderer did not match stably within ${geometryTimeoutSeconds}s. Last observation: $lastGeometryDetail"
    }
    $clientWidth = $client[2] - $client[0]
    $clientHeight = $client[3] - $client[1]
    if ($clientWidth -ne $innerWidth -or $clientHeight -ne $innerHeight) {
        throw "EDGE_RENDERER_BOUNDS_MISMATCH: child client ${clientWidth}x${clientHeight}, DOM viewport ${innerWidth}x${innerHeight}."
    }
    $contentOffsetX = $client[0] - $browserClient[0]
    $contentOffsetY = $client[1] - $browserClient[1]
    Write-Output "MILESTONE: stable DOM/renderer geometry verified for three consecutive samples: renderer HWND=0x$($rendererWindow.Handle.ToInt64().ToString('X')) PID=$($rendererWindow.ProcessId), content origin=($($client[0]),$($client[1])) root-client offset=($contentOffsetX,$contentOffsetY), viewport=${innerWidth}x${innerHeight}; root client is $($browserClient[2]-$browserClient[0])x$($browserClient[3]-$browserClient[1])."
    if (-not [ShotKitBrowserScrollSmoke.Native]::SetForegroundWindow($browserWindow.Handle)) { throw 'Could not reactivate Edge after waiting for viewport geometry.' }
    $foreground = Wait-ForCondition -TimeoutSeconds 3 -Description 'Edge foreground HWND after geometry stabilization' -OwnedProcess $null -Condition {
        $foregroundHwnd = [ShotKitBrowserScrollSmoke.Native]::ForegroundWindowHandle()
        if ($foregroundHwnd -eq $browserWindow.Handle -and
            [ShotKitBrowserScrollSmoke.Native]::ProcessIdForWindow($foregroundHwnd) -eq $browserWindow.ProcessId) { $true } else { $false }
    }
    if (-not $foreground) { throw 'EDGE_FOCUS_MISMATCH: foreground HWND changed after geometry stabilization.' }
    [ShotKitBrowserScrollSmoke.Native]::keybd_event(0x11, 0, 0x0002, [UIntPtr]::Zero) # release stale Ctrl
    [ShotKitBrowserScrollSmoke.Native]::keybd_event(0x79, 0, 0, [UIntPtr]::Zero) # F10 down
    [ShotKitBrowserScrollSmoke.Native]::keybd_event(0x79, 0, 0x0002, [UIntPtr]::Zero) # F10 up
    Write-Output 'MILESTONE: F10 key-up sent to verified foreground Edge HWND; waiting for scroll-direction dialog.'
    $directionDialog = Wait-ForCondition -TimeoutSeconds 10 -Description 'scroll direction prompt' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Richtung' } | Select-Object -First 1
    }
    Write-Output "MILESTONE: scroll-direction dialog appeared: $(Format-DialogDetails $directionDialog)"
    if (-not [ShotKitBrowserScrollSmoke.Native]::PostMessageW($directionDialog.Handle, 0x0111, [IntPtr]6, [IntPtr]::Zero)) { throw 'Could not select vertical downward scrolling.' }
    $overlay = Wait-ForCondition -TimeoutSeconds 10 -Description 'scroll selection overlay' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible } | Select-Object -First 1
    }
    Write-Output "MILESTONE: selection overlay visible HWND=0x$($overlay.Handle.ToInt64().ToString('X')); selected drag target is measured from renderer client bounds."
    if (-not [ShotKitBrowserScrollSmoke.Native]::SetForegroundWindow($overlay.Handle)) { throw 'Could not foreground selection overlay.' }
    if ([ShotKitBrowserScrollSmoke.Native]::ProcessIdForWindow($overlay.Handle) -ne [uint]$shotKit.Id) {
        throw 'SHOTKIT_FOCUS_MISMATCH: selection overlay is not owned by the test ShotKit process.'
    }
    $x0 = $client[0] + 100
    $y0 = if ($FixedHeaderFooter) { $client[1] } else { $client[1] + 10 }
    $x1 = $x0 + 600
    $y1 = if ($FixedHeaderFooter) { $client[3] } else { $y0 + 400 }
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

    Record-EdgeScrollTrace
    $png = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'auto-saved browser scroll PNG' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Record-EdgeScrollTrace
        Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Select-Object -First 1
    }
    Write-Output "MILESTONE: auto-saved capture observed at '$($png.FullName)' ($($png.Length) bytes); checking completion dialog and image geometry."
    $completion = Wait-ForCondition -TimeoutSeconds 20 -Description 'normal end-of-scroll notification' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Aufnahme beendet' } | Select-Object -First 1
    }
    $completionText = [ShotKitBrowserScrollSmoke.Native]::WindowText($completion.Handle)
    Record-EdgeScrollTrace
    try {
        # Re-enumerate the live profile-owned window rather than reuse its startup title.
        $escapedProfile = [Regex]::Escape($edgeProfile)
        $completionBrowserWindows = @(
            foreach ($process in (Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction Stop |
                Where-Object { $_.CommandLine -and $_.CommandLine -match $escapedProfile })) {
                [ShotKitBrowserScrollSmoke.Native]::WindowsForProcess([int]$process.ProcessId) |
                    Where-Object { $_.Visible -and $_.ClassName -eq 'Chrome_WidgetWin_1' }
            }
        )
        if ($completionBrowserWindows.Count -eq 0) {
            Write-Output 'DIAGNOSTIC: No live profile-owned Edge browser window found after scroll completion.'
        }
        foreach ($window in $completionBrowserWindows) {
            Write-Output "DIAGNOSTIC: Edge after scroll completion: HWND=0x$($window.Handle.ToInt64().ToString('X')) PID=$($window.ProcessId) Title='$($window.Title)'"
        }
    }
    catch { Write-Output "DIAGNOSTIC: Could not read live Edge DOM-offset title after scroll completion: $($_.Exception.ToString())" }
    if ($completionText -notlike '*Ende des scrollbaren Bereichs erreicht.*') {
        throw "Scroll did not report reaching the actual end of the browser page. Dialog='$completionText'"
    }
    $pngs = @(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File)
    if ($pngs.Count -ne 1) { throw "Expected one auto-saved PNG, found $($pngs.Count)." }

    $bitmap = [Drawing.Bitmap]::new($png.FullName)
    try {
        $selectedWidth = 600
        $selectedHeight = $y1 - $y0
        $expectedHeight = $scrollHeight - $innerHeight + $selectedHeight
        if ($bitmap.Width -ne $selectedWidth -or $bitmap.Height -ne $expectedHeight) {
            throw "SCROLL_BOUNDS_MISMATCH: got $($bitmap.Width)x$($bitmap.Height); expected ${selectedWidth}x${expectedHeight} from viewport/document geometry."
        }
        foreach ($y in 0..($expectedHeight - 1)) {
            $sourceY = if ($FixedHeaderFooter) { $y } else { $y + 10 }
            foreach ($x in @(10, 97, 299, 588)) {
                $sourceX = $x + 100
                if ($FixedHeaderFooter -and $y -lt 40) {
                    if ([int][Math]::Floor($sourceX / 100) % 2 -eq 0) { $expectedR = 232; $expectedG = 19; $expectedB = 41 }
                    else { $expectedR = 255; $expectedG = 204; $expectedB = 0 }
                } elseif ($FixedHeaderFooter -and $y -ge $expectedHeight - 40) {
                    if ([int][Math]::Floor($sourceX / 100) % 2 -eq 0) { $expectedR = 8; $expectedG = 118; $expectedB = 209 }
                    else { $expectedR = 23; $expectedG = 185; $expectedB = 120 }
                } else {
                    $expectedR = ($sourceX * 29 + $sourceY * 17 + [int][Math]::Floor($sourceY / 19) * 53) % 256
                    $expectedG = ($sourceX * 7 + $sourceY * 31 + [int][Math]::Floor($sourceY / 23) * 97) % 256
                    $expectedB = ($sourceX * 19 + $sourceY * 11 + [int][Math]::Floor($sourceY / 11) * 41) % 256
                }
                $actual = $bitmap.GetPixel($x, $y)
                if ($actual.R -ne $expectedR -or $actual.G -ne $expectedG -or $actual.B -ne $expectedB) {
                    throw "SCROLL_PIXEL_MISMATCH: output ($x,$y) maps to page ($sourceX,$sourceY), got $($actual.R),$($actual.G),$($actual.B), expected $expectedR,$expectedG,$expectedB."
                }
            }
        }
    }
    finally { $bitmap.Dispose() }
    Write-Output "MILESTONE: saved scroll image validated at $($png.Length) bytes with $expectedHeight deterministic rows and four pixel tracks."
    $fixture = if ($FixedHeaderFooter) { ' with fixed header/footer' } else { '' }
    $passMessage = "PASS: real Edge page$fixture scrolled to bottom and auto-saved ${selectedWidth}x${expectedHeight}; every sampled pixel matches its deterministic source coordinate."
}
catch { $testFailure = $_ }
finally {
    if ($script:edgeTrace.Count -eq 0) { Write-Output 'DIAGNOSTIC: Edge scroll trace unavailable.' }
    else { foreach ($entry in $script:edgeTrace) { Write-Output "DIAGNOSTIC: Edge scroll trace $entry" } }
    if ($script:edgeTraceReadFailed) { Write-Output 'DIAGNOSTIC: At least one Edge scroll trace read failed.' }
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
            [void](Get-ProfileEdgeProcesses) # discover any newly spawned profile children and retain verified handles
            $activeOwnedHandles = @($edgeOwnedHandles | Where-Object { -not $_.Handle.HasExited })
            foreach ($ownedHandle in $activeOwnedHandles) {
                try { Stop-ProfileEdgeProcess -OwnedHandle $ownedHandle }
                catch { $cleanupFailures += "Could not stop owned Edge PID $($ownedHandle.ProcessId): $($_.Exception.Message)" }
            }
            Start-Sleep -Milliseconds 200
            [void](Get-ProfileEdgeProcesses)
            $remaining = @($edgeOwnedHandles | Where-Object { -not $_.Handle.HasExited })
        } while ($remaining.Count -gt 0 -and [DateTime]::UtcNow -lt $deadline)
        if ($remaining.Count -gt 0) { $cleanupFailures += "Owned Edge profile processes remain: $(@($remaining | ForEach-Object ProcessId) -join ', ')." }
    }
    catch { $cleanupFailures += "Could not verify owned Edge process cleanup: $($_.Exception.Message)" }
    foreach ($ownedHandle in @($edgeOwnedHandles)) { try { $ownedHandle.Handle.Dispose() } catch { $cleanupFailures += "Could not release Edge process handle $($ownedHandle.ProcessId): $($_.Exception.Message)" } }
    if ($null -ne $edge) { $edge.Dispose() }
    if (Test-Path -LiteralPath $testRoot) {
        try { Remove-EdgeTestTree -Path $testRoot }
        catch {
            $cleanupFailures += "Could not remove test-owned files/profile '$testRoot': $($_.Exception.Message)"
            Write-Output "DIAGNOSTIC: Profile removal exception: $($_.Exception.ToString())"
            Write-Output ($_ | Format-List * -Force | Out-String -Width 4096)
        }
    }
    if ($cleanupFailures.Count -gt 0) { Write-ProfileEdgeCleanupDiagnostics }
    [ShotKitBrowserScrollSmoke.Native]::RestoreDpiContext($previousDpiContext)
}
if ($null -ne $testFailure -or $cleanupFailures.Count -gt 0) {
    $parts = @()
    if ($null -ne $testFailure) { $parts += "TEST FAILURE: $($testFailure.Exception.Message)" }
    if ($cleanupFailures.Count -gt 0) { $parts += "CLEANUP FAILURE: $($cleanupFailures -join [Environment]::NewLine)" }
    throw ($parts -join [Environment]::NewLine)
}
Write-Output $passMessage

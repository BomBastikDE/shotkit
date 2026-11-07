[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,
    [ValidateRange(10, 120)]
    [int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'shared-log-reader.ps1')
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') { throw 'LOCAL_OPT_IN_REQUIRED: use run-all.ps1 -AllowLocalInteractive in a dedicated test session.' }
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'This smoke test requires Windows.'
}
if (-not [Environment]::UserInteractive) {
    throw 'Desktop session unavailable: magnifier smoke requires an interactive Windows desktop.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$processNames = @([IO.Path]::GetFileNameWithoutExtension($ExePath), 'ShotKit', 'shotkit-rust-prototype') | Select-Object -Unique
foreach ($name in $processNames) {
    if (@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count -gt 0) {
        throw "PREEXISTING_PROCESS: refusing to start or affect '$name'."
    }
}

if (-not ('ShotKitMagnifierSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace ShotKitMagnifierSmoke {
    public struct Point { public int X; public int Y; }
    public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }
    public sealed class WindowInfo {
        public IntPtr Handle; public int ProcessId; public string ClassName; public string Title; public bool Visible;
        public Rect Bounds;
    }
    public static class Native {
        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
        [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
        [DllImport("user32.dll")] private static extern IntPtr GetWindowDC(IntPtr hwnd);
        [DllImport("gdi32.dll")] private static extern uint GetPixel(IntPtr dc, int x, int y);
        [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr hwnd, IntPtr dc);
        [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool CloseDesktop(IntPtr hwnd);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool GetUserObjectInformationW(IntPtr h, int index, StringBuilder text, uint length, out uint needed);
        [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
        public static WindowInfo[] WindowsForProcess(int pid) {
            List<WindowInfo> found = new List<WindowInfo>();
            EnumWindows(delegate(IntPtr hwnd, IntPtr ignored) {
                uint owner; GetWindowThreadProcessId(hwnd, out owner);
                if (owner != (uint)pid) return true;
                StringBuilder cls = new StringBuilder(256), title = new StringBuilder(512);
                GetClassNameW(hwnd, cls, cls.Capacity); GetWindowTextW(hwnd, title, title.Capacity);
                Rect r; GetWindowRect(hwnd, out r);
                found.Add(new WindowInfo { Handle=hwnd, ProcessId=(int)owner, ClassName=cls.ToString(),
                    Title=title.ToString(), Visible=IsWindowVisible(hwnd), Bounds=r });
                return true;
            }, IntPtr.Zero);
            return found.ToArray();
        }
        public static int Owner(IntPtr hwnd) { uint pid; GetWindowThreadProcessId(hwnd,out pid); return (int)pid; }
        // COLORREF, read from the owned top-level window's own DC (not the desktop beneath it).
        public static uint WindowPixel(IntPtr hwnd, int x, int y) {
            IntPtr dc = GetWindowDC(hwnd); if (dc == IntPtr.Zero) return 0xffffffff;
            try { return GetPixel(dc, x, y); } finally { ReleaseDC(hwnd, dc); }
        }
        public static uint ScreenPixel(int x,int y) { IntPtr dc=GetDC(IntPtr.Zero);if(dc==IntPtr.Zero)return 0xffffffff;try{return GetPixel(dc,x,y);}finally{ReleaseDC(IntPtr.Zero,dc);} }
        public static IntPtr Foreground { get { return GetForegroundWindow(); } }
        public static bool ForegroundWindow(IntPtr hwnd) { return SetForegroundWindow(hwnd); }
        public static string InputDesktopName() { IntPtr d=OpenInputDesktop(0,false,1);if(d==IntPtr.Zero)return null;try{StringBuilder s=new StringBuilder(256);uint n;if(!GetUserObjectInformationW(d,2,s,(uint)(s.Capacity*2),out n))return null;return s.ToString();}finally{CloseDesktop(d);} }
    }
}
'@
}

$inputDesktop = [ShotKitMagnifierSmoke.Native]::InputDesktopName()
if ($inputDesktop -cne 'Default') { throw "INTERACTIVE_DEFAULT_DESKTOP_REQUIRED: current input desktop is '$inputDesktop'." }

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Magnifier-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'
$localAppData = Join-Path $root 'localappdata'
$readyFile = Join-Path $root 'surface.ready'
$helperPath = Join-Path $root 'surface.ps1'
$stdoutPath = Join-Path $root 'surface.stdout.log'
$stderrPath = Join-Path $root 'surface.stderr.log'
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$shotKit = $null
$surface = $null

function Get-OwnedWindows([System.Diagnostics.Process] $Process) {
    [ShotKitMagnifierSmoke.Native]::WindowsForProcess($Process.Id)
}
function Wait-Until([scriptblock] $Condition, [string] $What) {
    $until = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $until) {
        if ($shotKit -and $shotKit.HasExited) { throw "ShotKit exited before $What (code $($shotKit.ExitCode))." }
        if ($surface -and $surface.HasExited) {
            $stdout = Read-SharedText -Path $stdoutPath
            $stderr = Read-SharedText -Path $stderrPath
            throw "SURFACE_HELPER_EXITED before $What (code $($surface.ExitCode)); stdout='$stdout'; stderr='$stderr'"
        }
        $value = & $Condition
        if ($value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out waiting for $What."
}
function Get-OwnedDialogs([System.Diagnostics.Process] $Process) {
    @(Get-OwnedWindows $Process | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' })
}
function Close-StartupDialog($Dialog) {
    if ($Dialog.ProcessId -ne $shotKit.Id -or $Dialog.ClassName -ne '#32770' -or $Dialog.Title -cne 'Willkommen bei ShotKit') {
        throw "REFUSING_DIALOG_CLOSE: not the owned ShotKit startup notice (pid=$($Dialog.ProcessId), title='$($Dialog.Title)')."
    }
    if (-not [ShotKitMagnifierSmoke.Native]::PostMessageW($Dialog.Handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)) {
        throw 'Could not close the owned ShotKit startup notice with WM_CLOSE.'
    }
    [void](Wait-Until { -not [ShotKitMagnifierSmoke.Native]::IsWindow($Dialog.Handle) } 'ShotKit startup notice to close')
}

try {
    New-Item -ItemType Directory -Path $appData, $localAppData | Out-Null
    # A separate, owned WinForms process paints a single exact-color surface.
    # The ready file contains the actual client origin after Windows lays out the form.
    @'
param([string]$ReadyFile)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies @('System.dll','System.Windows.Forms.dll','System.Drawing.dll') -TypeDefinition @"
using System;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Forms;
namespace ShotKitMagnifierSurface {
 public class Surface : Form {
  [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr value);
  public Surface(string ready) {
   try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch {}
   Text="ShotKit Magnifier Test Surface"; StartPosition=FormStartPosition.Manual; Location=new Point(50,50);
   ClientSize=new Size(720,520); FormBorderStyle=FormBorderStyle.FixedSingle; ShowInTaskbar=false;
   TopMost=true; MaximizeBox=false; MinimizeBox=false; BackColor=Color.FromArgb(0x12,0xA4,0xE7);
   Shown += delegate { Point p=PointToScreen(new Point(0,0)); File.WriteAllText(ready,p.X+","+p.Y); };
  }
 }
 public static class Start { [STAThread] public static void Run(string ready) { Application.EnableVisualStyles(); Application.Run(new Surface(ready)); } }
}
"@
[ShotKitMagnifierSurface.Start]::Run($ReadyFile)
'@ | Set-Content -LiteralPath $helperPath -Encoding UTF8

$powershellExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (-not (Test-Path -LiteralPath $powershellExe -PathType Leaf)) { throw 'Windows PowerShell 5.1 is required for the WinForms surface helper.' }
    $surface = Start-Process -FilePath $powershellExe -ArgumentList @('-NoProfile','-STA','-ExecutionPolicy','Bypass','-File',('"' + $helperPath + '"'),('"' + $readyFile + '"')) -PassThru -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    try { $ready = Wait-Until { Test-Path -LiteralPath $readyFile } }
    catch {
        $stdout = Read-SharedText -Path $stdoutPath
        $stderr = Read-SharedText -Path $stderrPath
        throw "SURFACE_HELPER_START_FAILED: $($_.Exception.Message); stdout='$stdout'; stderr='$stderr'"
    }
    $origin = [IO.File]::ReadAllText($readyFile).Split(',')
    $targetX = [int]$origin[0] + 360
    $targetY = [int]$origin[1] + 260

    try {
        $env:APPDATA = $appData
        $env:LOCALAPPDATA = $localAppData
        $shotKit = Start-Process -FilePath $ExePath -WorkingDirectory $root -PassThru
    } finally {
        $env:APPDATA = $oldAppData
        $env:LOCALAPPDATA = $oldLocalAppData
    }
    $mainWindow = Wait-Until { (Get-OwnedWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1) } 'the owned ShotKit primary window'
    if ($mainWindow.ProcessId -ne $shotKit.Id) { throw "MAIN_WINDOW_OWNER_MISMATCH: HWND=$($mainWindow.Handle), expected PID=$($shotKit.Id)." }
    $startup = Wait-Until { Get-OwnedDialogs $shotKit | Where-Object { $_.Title -ceq 'Willkommen bei ShotKit' } | Select-Object -First 1 } 'ShotKit startup notice'
    Close-StartupDialog $startup
    $remainingDialogs = @(Get-OwnedDialogs $shotKit)
    if ($remainingDialogs.Count -gt 0) { throw "UNEXPECTED_SHOTKIT_DIALOG: title='$($remainingDialogs[0].Title)'." }

    $surfaceWindow = Wait-Until { Get-OwnedWindows $surface | Where-Object { $_.Visible -and $_.Title -eq 'ShotKit Magnifier Test Surface' } | Select-Object -First 1 } 'the owned visible color surface'
    if ($surfaceWindow.ProcessId -ne $surface.Id -or -not $surfaceWindow.Visible) { throw 'SURFACE_WINDOW_OWNERSHIP_OR_VISIBILITY_FAILED.' }
    $sourcePixel = [ShotKitMagnifierSmoke.Native]::ScreenPixel($targetX,$targetY)
    if ($sourcePixel -ne 0x00E7A412) { throw ('VISIBLE_SOURCE_PIXEL_MISMATCH: at ({0},{1}) expected #12A4E7, got COLORREF 0x{2:X8}.' -f $targetX,$targetY,$sourcePixel) }
    if (-not [ShotKitMagnifierSmoke.Native]::ForegroundWindow($surfaceWindow.Handle)) { throw 'Could not foreground the owned color surface before input.' }
    if ([ShotKitMagnifierSmoke.Native]::Foreground -ne $surfaceWindow.Handle) { throw 'FOREGROUND_IDENTITY_CHECK_FAILED before cursor movement.' }
    if (-not [ShotKitMagnifierSmoke.Native]::SetCursorPos($targetX,$targetY)) { throw 'Could not position cursor on the test color field.' }
    Start-Sleep -Milliseconds 250
    if ([ShotKitMagnifierSmoke.Native]::Foreground -ne $surfaceWindow.Handle -or [ShotKitMagnifierSmoke.Native]::Owner([ShotKitMagnifierSmoke.Native]::Foreground) -ne $surface.Id) { throw 'FOREGROUND_OWNERSHIP_CHECK_FAILED before sending F11.' }
    # F11 is ShotKit's registered region-capture hotkey. Keep the pointer still so
    # the selection magnifier remains at the known coordinate.
    [ShotKitMagnifierSmoke.Native]::keybd_event(0x7A,0,0,[UIntPtr]::Zero)
    [ShotKitMagnifierSmoke.Native]::keybd_event(0x7A,0,2,[UIntPtr]::Zero)
    $mag = Wait-Until {
        Get-OwnedWindows $shotKit | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitSelectionMagnifier' } | Select-Object -First 1
    } 'the ShotKit-owned magnifier window'
    if ($mag.ProcessId -ne $shotKit.Id) { throw "MAGNIFIER_OWNER_MISMATCH: HWND=$($mag.Handle), expected PID=$($shotKit.Id)." }
    $magWidth = $mag.Bounds.Right - $mag.Bounds.Left
    $magHeight = $mag.Bounds.Bottom - $mag.Bounds.Top
    if ($magWidth -ne 120 -or $magHeight -ne 146) { throw "MAGNIFIER_BOUNDS_MISMATCH: expected 120x146 window, got ${magWidth}x${magHeight}." }
    Start-Sleep -Milliseconds 400

    # Magnifier is 120x120 with the label below. Find exact RGB field pixels in
    # its zoomed image; sample away from the expected cursor center and frame.
    $matches = 0
    $observedColors = @{}
    for ($y=10; $y -lt 110; $y+=4) {
        for ($x=10; $x -lt 110; $x+=4) {
            $color = [ShotKitMagnifierSmoke.Native]::WindowPixel($mag.Handle,$x,$y)
            if ($color -eq 0x00E7A412) { $matches++ }
            $key = '{0:X8}' -f $color
            if (-not $observedColors.ContainsKey($key)) { $observedColors[$key] = 0 }
            $observedColors[$key]++
        }
    }
    if ($matches -lt 8) {
        $top = @($observedColors.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 8 | ForEach-Object { "$($_.Key):$($_.Value)" }) -join ', '
        $screenCenter = [ShotKitMagnifierSmoke.Native]::ScreenPixel($mag.Bounds.Left + 60, $mag.Bounds.Top + 60)
        $targetNow = [ShotKitMagnifierSmoke.Native]::ScreenPixel($targetX, $targetY)
        throw ("Magnifier image did not contain expected #12A4E7 (exact-color samples={0}; window COLORREF counts={1}; desktop magnifier center=0x{2:X8}; desktop source point=0x{3:X8}; magnifier bounds=({4},{5})-({6},{7}); cursor target=({8},{9}))." -f $matches,$top,$screenCenter,$targetNow,$mag.Bounds.Left,$mag.Bounds.Top,$mag.Bounds.Right,$mag.Bounds.Bottom,$targetX,$targetY)
    }

    $labelWhitePixels = 0
    for ($y=124; $y -lt 144; $y++) {
        for ($x=2; $x -lt 118; $x++) {
            if ([ShotKitMagnifierSmoke.Native]::WindowPixel($mag.Handle,$x,$y) -eq 0x00FFFFFF) { $labelWhitePixels++ }
        }
    }
    if ($labelWhitePixels -lt 4) { throw "Magnifier label strip contained no detectable white text pixels ($labelWhitePixels)." }

    # Deliberately no OCR: this proves that the text strip is painted, not that
    # its glyphs semantically encode the expected coordinates or RGB string.
    Write-Output ("PASS magnifier hwnd=0x{0:X} ownerPid={1} target=({2},{3}) expectedColor=#12A4E7 exactImageSamples={4} labelStripWhitePixels={5} labelTextSemanticVerification=UNPROVEN" -f $mag.Handle.ToInt64(),$shotKit.Id,$targetX,$targetY,$matches,$labelWhitePixels)
}
finally {
    $cleanupErrors = @()
    if ($shotKit -and -not $shotKit.HasExited) {
        # Escape cancels only the selection overlay started by this test.
        $overlay = Get-OwnedWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' } | Select-Object -First 1
        if ($overlay -and $overlay.ProcessId -eq $shotKit.Id) { [void][ShotKitMagnifierSmoke.Native]::PostMessageW($overlay.Handle,0x0100,[IntPtr]0x1B,[IntPtr]::Zero) }
        try {
            $live = Get-Process -Id $shotKit.Id -ErrorAction Stop
            if ($live.Path -ne $ExePath) { throw "Refusing to stop PID $($shotKit.Id): executable path changed to '$($live.Path)'" }
            if (-not $shotKit.WaitForExit(3000)) { $shotKit.Kill(); if (-not $shotKit.WaitForExit(5000)) { throw 'Owned ShotKit process did not exit after bounded kill.' } }
        } catch { $cleanupErrors += $_.Exception.Message }
    }
    if ($surface -and -not $surface.HasExited) {
        try {
            $surfaceWindow = Get-OwnedWindows $surface | Where-Object { $_.Title -eq 'ShotKit Magnifier Test Surface' } | Select-Object -First 1
            if ($surfaceWindow -and $surfaceWindow.ProcessId -eq $surface.Id) { [void][ShotKitMagnifierSmoke.Native]::PostMessageW($surfaceWindow.Handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) }
            if (-not $surface.WaitForExit(3000)) {
                $live = Get-Process -Id $surface.Id -ErrorAction Stop
                if ($live.ProcessName -ne 'powershell') { throw "Refusing to stop unexpected helper process '$($live.ProcessName)' PID $($surface.Id)." }
                $surface.Kill(); if (-not $surface.WaitForExit(5000)) { throw 'Owned WinForms helper did not exit after bounded kill.' }
            }
        } catch { $cleanupErrors += $_.Exception.Message }
    }
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    $tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]@([IO.Path]::DirectorySeparatorChar,[IO.Path]::AltDirectorySeparatorChar))
    $safeRoot = ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($root)) -eq $tempParent) -and ([IO.Path]::GetFileName($root) -like 'ShotKit-Magnifier-Smoke-*')
    if (($shotKit -eq $null -or $shotKit.HasExited) -and ($surface -eq $null -or $surface.HasExited) -and $safeRoot -and (Test-Path -LiteralPath $root)) {
        Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
    } else { $cleanupErrors += "Temp directory retained because process exit or path validation failed: $root" }
    if ($cleanupErrors.Count) { throw ('CLEANUP_FAILED: ' + ($cleanupErrors -join '; ')) }
}

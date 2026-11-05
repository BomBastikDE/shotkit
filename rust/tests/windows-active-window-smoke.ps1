[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 25,

    [ValidateRange(15, 180)]
    [int] $CaptureTimeoutSeconds = 60
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') {
    throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'
}
if (-not [Environment]::UserInteractive) {
    throw 'Desktop session unavailable: this Windows runner is not interactive.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "EXE not found: $ExePath"
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

if (-not ('ShotKitActiveWindowSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace ShotKitActiveWindowSmoke {
    public sealed class WindowInfo {
        public IntPtr Handle;
        public string ClassName;
        public string Title;
        public bool Visible;
        public bool Minimized;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct NativeRect { public int Left; public int Top; public int Right; public int Bottom; }

    public static class Native {
        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);
        private delegate bool EnumChildProc(IntPtr hwnd, IntPtr lParam);
        [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool CloseDesktop(IntPtr desktop);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder info, uint length, out uint needed);
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
        [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr lParam);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern bool IsIconic(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
        [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
        [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr hwnd, int command);
        [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);
        [DllImport("user32.dll")] private static extern int GetSystemMetrics(int index);
        [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("dwmapi.dll", PreserveSig=true)] private static extern int DwmGetWindowAttribute(IntPtr hwnd, uint attribute, out NativeRect value, uint size);
        [DllImport("user32.dll", SetLastError=true)] private static extern bool ClientToScreen(IntPtr hwnd, ref NativePoint point);
        [StructLayout(LayoutKind.Sequential)] private struct NativePoint { public int X; public int Y; }

        public static IntPtr EnablePerMonitorDpi() { return SetThreadDpiAwarenessContext(new IntPtr(-4)); }
        public static void RestoreDpi(IntPtr previous) { if (previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous); }
        public static string InputDesktopName() {
            IntPtr desktop = OpenInputDesktop(0, false, 0x0001);
            if (desktop == IntPtr.Zero) return null;
            try {
                StringBuilder name = new StringBuilder(256); uint needed;
                if (!GetUserObjectInformationW(desktop, 2, name, (uint)(name.Capacity * 2), out needed)) return null;
                return name.ToString();
            } finally { CloseDesktop(desktop); }
        }
        private static string Text(IntPtr hwnd) { StringBuilder text = new StringBuilder(512); GetWindowTextW(hwnd, text, text.Capacity); return text.ToString(); }
        private static string Class(IntPtr hwnd) { StringBuilder text = new StringBuilder(256); GetClassNameW(hwnd, text, text.Capacity); return text.ToString(); }
        public static string WindowText(IntPtr hwnd) {
            StringBuilder all = new StringBuilder(Text(hwnd));
            EnumChildProc callback = delegate(IntPtr child, IntPtr ignored) {
                string text = Text(child); if (!String.IsNullOrWhiteSpace(text)) all.Append(" ").Append(text); return true;
            };
            EnumChildWindows(hwnd, callback, IntPtr.Zero); GC.KeepAlive(callback); return all.ToString();
        }
        public static WindowInfo[] WindowsForProcess(int processId) {
            List<WindowInfo> windows = new List<WindowInfo>();
            EnumWindowsProc callback = delegate(IntPtr hwnd, IntPtr ignored) {
                uint owner; GetWindowThreadProcessId(hwnd, out owner);
                if (owner == (uint)processId) windows.Add(new WindowInfo { Handle=hwnd, ClassName=Class(hwnd), Title=Text(hwnd), Visible=IsWindowVisible(hwnd), Minimized=IsIconic(hwnd) });
                return true;
            };
            EnumWindows(callback, IntPtr.Zero); GC.KeepAlive(callback); return windows.ToArray();
        }
        public static bool IsForeground(IntPtr hwnd) { return GetForegroundWindow() == hwnd; }
        public static bool Activate(IntPtr hwnd) { ShowWindow(hwnd, 9); return SetForegroundWindow(hwnd); }
        public static IntPtr Root(IntPtr hwnd) { return GetAncestor(hwnd, 2); }
        public static uint OwnerProcessId(IntPtr hwnd) { uint owner; GetWindowThreadProcessId(hwnd, out owner); return owner; }
        public static bool DwmFrame(IntPtr hwnd, out NativeRect rect) { return DwmGetWindowAttribute(hwnd, 9, out rect, (uint)Marshal.SizeOf(typeof(NativeRect))) == 0; }
        public static int[] VirtualDesktop() { return new int[] { GetSystemMetrics(76), GetSystemMetrics(77), GetSystemMetrics(78), GetSystemMetrics(79) }; }
        public static int[] ClientOrigin(IntPtr hwnd) { NativePoint p = new NativePoint(); if (!ClientToScreen(hwnd, ref p)) throw new System.ComponentModel.Win32Exception(); return new int[] { p.X, p.Y }; }
    }
}
'@
}

$previousDpi = [ShotKitActiveWindowSmoke.Native]::EnablePerMonitorDpi()
if ($previousDpi -eq [IntPtr]::Zero) {
    throw 'Could not enable per-monitor DPI awareness for physical screen-coordinate checks.'
}
$inputDesktop = [ShotKitActiveWindowSmoke.Native]::InputDesktopName()
if ([string]::IsNullOrWhiteSpace($inputDesktop)) {
    throw 'Desktop session unavailable: Windows could not open the interactive input desktop.'
}

# Never let this script route input to an already-running personal instance.
$processNames = @([IO.Path]::GetFileNameWithoutExtension($ExePath), 'ShotKit', 'shotkit-rust-prototype') | Select-Object -Unique
foreach ($name in $processNames) {
    if (@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count -gt 0) {
        [ShotKitActiveWindowSmoke.Native]::RestoreDpi($previousDpi)
        throw "PREEXISTING_PROCESS: '$name' is already running."
    }
}

$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
if (-not (Test-Path -LiteralPath $windowsPowerShell -PathType Leaf)) {
    [ShotKitActiveWindowSmoke.Native]::RestoreDpi($previousDpi)
    throw "Windows PowerShell 5.1 was not found at '$windowsPowerShell'."
}
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Active-Window-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$saveFolder = Join-Path $testRoot 'captures'
$helperFile = Join-Path $testRoot 'surface.ps1'
$readyFile = Join-Path $testRoot 'surface.ready'
$helperStdout = Join-Path $testRoot 'surface.stdout.log'
$helperStderr = Join-Path $testRoot 'surface.stderr.log'
$shotKit = $null
$surface = $null
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$passMessage = $null

function Get-OwnWindows {
    param([System.Diagnostics.Process] $Process)
    [ShotKitActiveWindowSmoke.Native]::WindowsForProcess($Process.Id)
}
function Get-OwnDialogs {
    param([System.Diagnostics.Process] $Process)
    Get-OwnWindows $Process | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' }
}
function Assert-NoUnexpectedDialog {
    param([System.Diagnostics.Process] $Process, [string[]] $AllowedTitles = @())
    foreach ($dialog in @(Get-OwnDialogs $Process)) {
        if (@($AllowedTitles | Where-Object { $dialog.Title -like $_ }).Count -gt 0) { continue }
        $details = [ShotKitActiveWindowSmoke.Native]::WindowText($dialog.Handle)
        throw "ShotKit displayed an unexpected dialog. Title='$($dialog.Title)' Text='$details'"
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
        if ($OwnedProcess.HasExited) { throw "Owned process '$($OwnedProcess.ProcessName)' exited before $Description (code $($OwnedProcess.ExitCode))." }
        if ($OwnedProcess -eq $shotKit) { Assert-NoUnexpectedDialog $OwnedProcess $AllowedDialogTitles }
        $value = & $Condition
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out after ${TimeoutSeconds}s waiting for $Description."
}

try {
    New-Item -ItemType Directory -Path $appData, $localAppData, $saveFolder -Force | Out-Null
    $desktop = [ShotKitActiveWindowSmoke.Native]::VirtualDesktop()
    if ($desktop[2] -lt 600 -or $desktop[3] -lt 480) {
        throw "Interactive desktop $($desktop[2])x$($desktop[3]) is too small for the synthetic target."
    }

$helperSource = @'
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Forms;
namespace ShotKitActiveWindowSurface {
    public static class Dpi {
        [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        public static void Enable() { if (SetThreadDpiAwarenessContext(new IntPtr(-4)) == IntPtr.Zero) throw new System.ComponentModel.Win32Exception(); }
    }
    public sealed class Surface : Form {
        private readonly string readyPath;
        private const int SurfaceWidth = 480, SurfaceHeight = 320;
        public Surface(string readyPath, int left, int top) {
            this.readyPath = readyPath;
            Text = "ShotKit Active Window Synthetic Target";
            FormBorderStyle = FormBorderStyle.None;
            StartPosition = FormStartPosition.Manual;
            Location = new Point(left, top);
            ClientSize = new Size(SurfaceWidth, SurfaceHeight);
            ShowInTaskbar = true;
            TopMost = false;
            DoubleBuffered = true;
            SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer, true);
        }
        protected override void OnPaint(PaintEventArgs e) {
            int midX = ClientSize.Width / 2, midY = ClientSize.Height / 2;
            using (SolidBrush red = new SolidBrush(Color.FromArgb(220, 35, 45)))
            using (SolidBrush green = new SolidBrush(Color.FromArgb(30, 190, 65)))
            using (SolidBrush blue = new SolidBrush(Color.FromArgb(35, 70, 220)))
            using (SolidBrush yellow = new SolidBrush(Color.FromArgb(235, 205, 35))) {
                e.Graphics.FillRectangle(red, 0, 0, midX, midY);
                e.Graphics.FillRectangle(green, midX, 0, ClientSize.Width-midX, midY);
                e.Graphics.FillRectangle(blue, 0, midY, midX, ClientSize.Height-midY);
                e.Graphics.FillRectangle(yellow, midX, midY, ClientSize.Width-midX, ClientSize.Height-midY);
            }
            base.OnPaint(e);
        }
        protected override void OnShown(EventArgs e) {
            base.OnShown(e); Update();
            Point origin = PointToScreen(Point.Empty);
            File.WriteAllText(readyPath, String.Join(",", new string[] {
                Handle.ToInt64().ToString(), origin.X.ToString(), origin.Y.ToString(),
                ClientSize.Width.ToString(), ClientSize.Height.ToString()
            }));
        }
    }
}
"@
[ShotKitActiveWindowSurface.Dpi]::Enable()
$desktopLeft = [System.Windows.Forms.Screen]::AllScreens | ForEach-Object { $_.Bounds.Left } | Measure-Object -Minimum | Select-Object -ExpandProperty Minimum
$desktopTop = [System.Windows.Forms.Screen]::AllScreens | ForEach-Object { $_.Bounds.Top } | Measure-Object -Minimum | Select-Object -ExpandProperty Minimum
$surface = [ShotKitActiveWindowSurface.Surface]::new($args[0], ([int]$desktopLeft + 60), ([int]$desktopTop + 60))
try { [System.Windows.Forms.Application]::Run($surface) }
finally { $surface.Dispose() }
'@
    [IO.File]::WriteAllText($helperFile, $helperSource, [Text.UTF8Encoding]::new($false))

    $helperArgs = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" "{1}"' -f $helperFile, $readyFile
    $surface = Start-Process -FilePath $windowsPowerShell -ArgumentList $helperArgs -WorkingDirectory $testRoot -WindowStyle Hidden -RedirectStandardOutput $helperStdout -RedirectStandardError $helperStderr -PassThru
    $surfaceInfo = Wait-ForCondition -TimeoutSeconds 20 -Description 'the synthetic foreground window geometry' -OwnedProcess $surface -Condition {
        if (Test-Path -LiteralPath $readyFile) { Get-Content -LiteralPath $readyFile -Raw }
    }
    $values = $surfaceInfo.Trim().Split(',') | ForEach-Object { [int64]$_ }
    if ($values.Count -ne 5) { throw "Synthetic window returned invalid geometry: '$surfaceInfo'." }
    $target = [IntPtr]$values[0]
    $clientLeft = [int]$values[1]
    $clientTop = [int]$values[2]
    $clientWidth = [int]$values[3]
    $clientHeight = [int]$values[4]
    if ($clientWidth -ne 480 -or $clientHeight -ne 320) { throw "Unexpected synthetic client size ${clientWidth}x${clientHeight}." }
    $root = [ShotKitActiveWindowSmoke.Native]::Root($target)
    if ($root -eq [IntPtr]::Zero -or $root -ne $target) { throw "Synthetic target HWND $target is not its own top-level root window." }
    if ([ShotKitActiveWindowSmoke.Native]::OwnerProcessId($root) -ne [uint32]$surface.Id) { throw 'Synthetic target window is not owned by the helper process.' }
    $null = Wait-ForCondition -TimeoutSeconds 5 -Description 'the visible synthetic target window' -OwnedProcess $surface -Condition {
        Get-OwnWindows $surface | Where-Object { $_.Handle -eq $root -and $_.Visible -and -not $_.Minimized -and $_.Title -eq 'ShotKit Active Window Synthetic Target' } | Select-Object -First 1
    }

    $frame = [ShotKitActiveWindowSmoke.NativeRect]::new()
    if (-not [ShotKitActiveWindowSmoke.Native]::DwmFrame($root, [ref]$frame)) { throw 'DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS) failed for the synthetic window.' }
    $frameWidth = $frame.Right - $frame.Left
    $frameHeight = $frame.Bottom - $frame.Top
    if ($frameWidth -ne $clientWidth -or $frameHeight -ne $clientHeight) {
        throw "Borderless synthetic DWM frame is ${frameWidth}x${frameHeight}, expected client ${clientWidth}x${clientHeight}."
    }
    if ($frame.Left -lt $desktop[0] -or $frame.Top -lt $desktop[1] -or
        $frame.Right -gt ($desktop[0] + $desktop[2]) -or $frame.Bottom -gt ($desktop[1] + $desktop[3])) {
        throw 'Synthetic window is not fully inside the virtual desktop; refusing a clipped-window assertion.'
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
        $shotKit = Start-Process -FilePath $ExePath -WorkingDirectory $testRoot -PassThru
    }
    finally { $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData }

    $mainWindow = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the ShotKit main window' -OwnedProcess $shotKit -AllowedDialogTitles @('*Prototyp*') -Condition {
        Get-OwnWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
    }
    $welcome = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the ShotKit startup dialog' -OwnedProcess $shotKit -AllowedDialogTitles @('*Prototyp*') -Condition {
        Get-OwnDialogs $shotKit | Where-Object { $_.Title -like '*Prototyp*' } | Select-Object -First 1
    }
    if (-not [ShotKitActiveWindowSmoke.Native]::PostMessageW($welcome.Handle, 0x0111, [IntPtr]1, [IntPtr]::Zero)) {
        throw 'Could not close the known startup dialog with its default OK command.'
    }
    $null = Wait-ForCondition -TimeoutSeconds 5 -Description 'the startup dialog to close' -OwnedProcess $shotKit -AllowedDialogTitles @($welcome.Title) -Condition {
        -not (Get-OwnDialogs $shotKit | Where-Object { $_.Handle -eq $welcome.Handle })
    }

    if (-not [ShotKitActiveWindowSmoke.Native]::Activate($root)) { throw 'SetForegroundWindow failed for the synthetic target.' }
    $null = Wait-ForCondition -TimeoutSeconds 5 -Description 'the synthetic window to become foreground' -OwnedProcess $surface -Condition {
        [ShotKitActiveWindowSmoke.Native]::IsForeground($root)
    }
    $surfaceWindow = Get-OwnWindows $surface | Where-Object { $_.Handle -eq $root -and $_.Visible -and -not $_.Minimized } | Select-Object -First 1
    if ($null -eq $surfaceWindow -or $surfaceWindow.Title -ne 'ShotKit Active Window Synthetic Target') {
        throw 'Foreground HWND did not resolve to the expected visible, restored helper window.'
    }

    # C# parity shortcut: Shift+PrintScreen. The test verifies foreground again
    # immediately before injection so no different window can be accepted as a pass.
    if (-not [ShotKitActiveWindowSmoke.Native]::IsForeground($root)) { throw 'Foreground changed before Shift+PrintScreen injection.' }
    [ShotKitActiveWindowSmoke.Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
    [ShotKitActiveWindowSmoke.Native]::keybd_event(0x2c, 0, 0, [UIntPtr]::Zero)
    [ShotKitActiveWindowSmoke.Native]::keybd_event(0x2c, 0, 0x0002, [UIntPtr]::Zero)
    [ShotKitActiveWindowSmoke.Native]::keybd_event(0x10, 0, 0x0002, [UIntPtr]::Zero)

    $png = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'the active-window capture PNG' -OwnedProcess $shotKit -Condition {
        Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Select-Object -First 1
    }
    $editor = Wait-ForCondition -TimeoutSeconds 20 -Description 'the editor after active-window capture' -OwnedProcess $shotKit -Condition {
        Get-OwnWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible } | Select-Object -First 1
    }

    Add-Type -AssemblyName System.Drawing
    $bitmap = [Drawing.Bitmap]::new($png.FullName)
    try {
        if ($bitmap.Width -ne $frameWidth -or $bitmap.Height -ne $frameHeight) {
            throw "Captured PNG is $($bitmap.Width)x$($bitmap.Height); expected DWM frame ${frameWidth}x${frameHeight}."
        }
        $samples = @(
            @{ X = [int]($clientWidth * 0.25); Y = [int]($clientHeight * 0.25); R = 220; G = 35; B = 45; Name = 'top-left red' },
            @{ X = [int]($clientWidth * 0.75); Y = [int]($clientHeight * 0.25); R = 30; G = 190; B = 65; Name = 'top-right green' },
            @{ X = [int]($clientWidth * 0.25); Y = [int]($clientHeight * 0.75); R = 35; G = 70; B = 220; Name = 'bottom-left blue' },
            @{ X = [int]($clientWidth * 0.75); Y = [int]($clientHeight * 0.75); R = 235; G = 205; B = 35; Name = 'bottom-right yellow' }
        )
        foreach ($sample in $samples) {
            $screenX = $clientLeft + $sample.X
            $screenY = $clientTop + $sample.Y
            $imageX = $screenX - $frame.Left
            $imageY = $screenY - $frame.Top
            if ($imageX -lt 0 -or $imageY -lt 0 -or $imageX -ge $bitmap.Width -or $imageY -ge $bitmap.Height) {
                throw "Sample '$($sample.Name)' falls outside the DWM frame at image coordinate ($imageX,$imageY)."
            }
            $actual = $bitmap.GetPixel($imageX, $imageY)
            if ([Math]::Abs($actual.R - $sample.R) -gt 3 -or
                [Math]::Abs($actual.G - $sample.G) -gt 3 -or
                [Math]::Abs($actual.B - $sample.B) -gt 3) {
                throw "Active-window pixel '$($sample.Name)' at ($imageX,$imageY) was RGB=$($actual.R),$($actual.G),$($actual.B), expected $($sample.R),$($sample.G),$($sample.B)."
            }
        }
    }
    finally { $bitmap.Dispose() }
    Assert-NoUnexpectedDialog $shotKit
    $passMessage = "PASS: Shift+PrintScreen captured foreground HWND $($root.ToInt64()) at ${frameWidth}x${frameHeight}; DWM frame and four deterministic pixels matched."
}
catch {
    foreach ($log in @($helperStdout, $helperStderr)) {
        if (Test-Path -LiteralPath $log -PathType Leaf) {
            $contents = Get-Content -LiteralPath $log -Raw
            if ($contents) { Write-Host ("Helper log {0}:`n{1}" -f (Split-Path -Leaf $log), $contents) }
        }
    }
    throw
}
finally {
    foreach ($owned in @($shotKit, $surface)) {
        if ($null -ne $owned) {
            try {
                if (-not $owned.HasExited) {
                    $owned.Kill()
                    if (-not $owned.WaitForExit(5000) -or -not $owned.HasExited) {
                        throw "Owned process '$($owned.ProcessName)' did not exit after termination was requested."
                    }
                }
            }
            catch { Write-Warning "Could not stop owned process '$($owned.ProcessName)': $($_.Exception.Message)" }
            $owned.Dispose()
        }
    }
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    [ShotKitActiveWindowSmoke.Native]::RestoreDpi($previousDpi)
    if (Test-Path -LiteralPath $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue }
}
Write-Output $passMessage

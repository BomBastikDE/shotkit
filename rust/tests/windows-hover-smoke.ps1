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
if ($env:GITHUB_ACTIONS -ne 'true') {
    throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'
}
if (-not [Environment]::UserInteractive) {
    throw 'Desktop session unavailable: this Windows runner is not interactive.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "EXE not found: $ExePath"
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

if (-not ('ShotKitHoverSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace ShotKitHoverSmoke {
    public sealed class WindowInfo {
        public IntPtr Handle;
        public string ClassName;
        public string Title;
        public bool Visible;
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
        [DllImport("user32.dll")]
        public static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
        [DllImport("user32.dll")]
        private static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
        [DllImport("user32.dll")]
        private static extern bool ShowWindow(IntPtr hwnd, int command);
        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")]
        private static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr lParam);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", SetLastError=true)]
        public static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll")]
        public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")]
        public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")]
        public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
        [DllImport("user32.dll")]
        public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);
        [DllImport("user32.dll")]
        public static extern int GetSystemMetrics(int index);
        [DllImport("user32.dll")]
        public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("user32.dll", SetLastError=true)]
        private static extern IntPtr SetProcessDpiAwarenessContext(IntPtr context);

        public static bool EnablePerMonitorDpi() {
            try { return SetProcessDpiAwarenessContext(new IntPtr(-4)) != IntPtr.Zero; }
            catch { return false; }
        }
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
        public static string WindowText(IntPtr hwnd) {
            StringBuilder all = new StringBuilder(Text(hwnd));
            EnumChildWindows(hwnd, delegate(IntPtr child, IntPtr ignored) {
                string text = Text(child);
                if (!String.IsNullOrWhiteSpace(text)) all.Append(" ").Append(text);
                return true;
            }, IntPtr.Zero);
            return all.ToString();
        }
        public static IntPtr RootWindow(IntPtr hwnd) { return GetAncestor(hwnd, 2); }
        public static uint OwnerProcessId(IntPtr hwnd) {
            uint processId;
            GetWindowThreadProcessId(hwnd, out processId);
            return processId;
        }
        public static string WindowTitle(IntPtr hwnd) { return Text(hwnd); }
        public static void ShowNormal(IntPtr hwnd) { ShowWindow(hwnd, 1); }
        public static IntPtr FindVisibleOkButton(IntPtr dialog, out string details) {
            IntPtr found = IntPtr.Zero;
            List<string> buttons = new List<string>();
            EnumChildProc callback = delegate(IntPtr child, IntPtr ignored) {
                if (!IsWindowVisible(child)) return true;
                StringBuilder cls = new StringBuilder(128);
                StringBuilder title = new StringBuilder(256);
                GetClassNameW(child, cls, cls.Capacity);
                GetWindowTextW(child, title, title.Capacity);
                if (String.Equals(cls.ToString(), "Button", StringComparison.OrdinalIgnoreCase)) {
                    int id = GetDlgCtrlID(child);
                    buttons.Add(String.Format("HWND=0x{0:X} ID={1} Text='{2}'", child.ToInt64(), id, title));
                    if (String.Equals(title.ToString().Trim(), "OK", StringComparison.OrdinalIgnoreCase)) {
                        found = child;
                        return false;
                    }
                }
                return true;
            };
            EnumChildWindows(dialog, callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            details = buttons.Count == 0 ? "no visible Button-class children" : String.Join("; ", buttons.ToArray());
            return found;
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
    }
}
'@
}

$inputDesktop = [ShotKitHoverSmoke.Native]::InputDesktopName()
if ([string]::IsNullOrWhiteSpace($inputDesktop)) {
    throw 'Desktop session unavailable: Windows could not open the interactive input desktop.'
}
$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
if (-not (Test-Path -LiteralPath $windowsPowerShell -PathType Leaf)) {
    throw "Windows PowerShell 5.1 was not found at '$windowsPowerShell'."
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Hover-Smoke-' + [Guid]::NewGuid().ToString('N'))
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
$previousDpiContext = [IntPtr]::Zero
try { $previousDpiContext = [ShotKitHoverSmoke.Native]::SetThreadDpiAwarenessContext([IntPtr](-4)) } catch { }
$passMessage = $null
$cleanupFailures = @()

function Get-ProcessWindows {
    param([System.Diagnostics.Process] $OwnedProcess)
    [ShotKitHoverSmoke.Native]::WindowsForProcess($OwnedProcess.Id)
}
function Get-ProcessDialogs {
    param([System.Diagnostics.Process] $OwnedProcess)
    Get-ProcessWindows $OwnedProcess | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' }
}
function Assert-NoUnexpectedDialog {
    param(
        [System.Diagnostics.Process] $OwnedProcess,
        [string[]] $AllowedTitles = @()
    )
    $dialog = Get-ProcessDialogs $OwnedProcess |
        Where-Object { $AllowedTitles -notcontains $_.Title -and $_.Title -notlike '*Prototyp*' } |
        Select-Object -First 1
    if ($null -ne $dialog) {
        throw "Unexpected ShotKit dialog. Title='$($dialog.Title)' Text='$([ShotKitHoverSmoke.Native]::WindowText($dialog.Handle))'"
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
            if ($OwnedProcess.Id -eq $surface.Id) {
                $stdout = if (Test-Path -LiteralPath $helperStdout) { [IO.File]::ReadAllText($helperStdout).Trim() } else { '' }
                $stderr = if (Test-Path -LiteralPath $helperStderr) { [IO.File]::ReadAllText($helperStderr).Trim() } else { '' }
                throw "Synthetic surface exited before $Description (exit $($OwnedProcess.ExitCode)).`nstdout:`n$stdout`nstderr:`n$stderr"
            }
            throw "ShotKit exited before $Description (exit code $($OwnedProcess.ExitCode))."
        }
        Assert-NoUnexpectedDialog $OwnedProcess $AllowedDialogTitles
        $value = & $Condition
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out after ${TimeoutSeconds}s waiting for $Description."
}

try {
    New-Item -ItemType Directory -Path $appData, $localAppData, $saveFolder -Force | Out-Null

    # This test-owned WinForms control has its own HWND and a deterministic,
    # four-color client area. Selecting it by a stationary hover + click must
    # yield its exact bounds; a missed suggestion falls back to the larger form
    # and fails the output dimension assertion below.
    $helperSource = @'
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$formsAssembly = [System.Windows.Forms.Form].Assembly.Location
$drawingAssembly = [System.Drawing.Bitmap].Assembly.Location
$typeDefinition = @"
using System;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Forms;
namespace ShotKitHoverSurface {
    public static class Dpi {
        [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        private static IntPtr previous;
        public static void Enable() { try { previous = SetThreadDpiAwarenessContext(new IntPtr(-4)); } catch { } }
        public static void Restore() { try { if (previous != IntPtr.Zero) SetThreadDpiAwarenessContext(previous); } catch { } }
    }
    public struct NativeRect { public int Left; public int Top; public int Right; public int Bottom; }
    public static class Native {
        [DllImport("user32.dll", SetLastError=true)] public static extern bool GetWindowRect(IntPtr hwnd, out NativeRect rect);
    }
    public sealed class PatternControl : Control {
        public PatternControl() {
            SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer | ControlStyles.Opaque, true);
            Size = new Size(280, 180);
            Location = new Point(100, 90);
            TabStop = false;
        }
        protected override void OnPaint(PaintEventArgs e) {
            int halfW = ClientSize.Width / 2;
            int halfH = ClientSize.Height / 2;
            using (Brush b = new SolidBrush(Color.FromArgb(220, 35, 45))) e.Graphics.FillRectangle(b, 0, 0, halfW, halfH);
            using (Brush b = new SolidBrush(Color.FromArgb(30, 190, 65))) e.Graphics.FillRectangle(b, halfW, 0, ClientSize.Width-halfW, halfH);
            using (Brush b = new SolidBrush(Color.FromArgb(35, 70, 220))) e.Graphics.FillRectangle(b, 0, halfH, halfW, ClientSize.Height-halfH);
            using (Brush b = new SolidBrush(Color.FromArgb(235, 205, 35))) e.Graphics.FillRectangle(b, halfW, halfH, ClientSize.Width-halfW, ClientSize.Height-halfH);
            base.OnPaint(e);
        }
    }
    public sealed class Surface : Form {
        private readonly string readyFile;
        private readonly PatternControl pattern;
        public Surface(string path, int left, int top) {
            readyFile = path;
            Text = "ShotKit Hover Synthetic Surface";
            StartPosition = FormStartPosition.Manual;
            Location = new Point(left, top);
            ClientSize = new Size(520, 380);
            FormBorderStyle = FormBorderStyle.FixedSingle;
            MaximizeBox = false;
            MinimizeBox = false;
            ShowInTaskbar = true;
            pattern = new PatternControl();
            Controls.Add(pattern);
        }
        protected override void OnShown(EventArgs e) {
            base.OnShown(e);
            IntPtr targetHandle = pattern.Handle;
            pattern.Invalidate();
            Update();
            NativeRect rect;
            if (!Native.GetWindowRect(targetHandle, out rect)) throw new System.ComponentModel.Win32Exception();
            string geometry = String.Format("{0},{1},{2},{3},{4}", rect.Left, rect.Top, rect.Right-rect.Left, rect.Bottom-rect.Top, targetHandle.ToInt64());
            File.WriteAllText(readyFile, geometry);
        }
    }
}
"@
Add-Type -TypeDefinition $typeDefinition -ReferencedAssemblies @($formsAssembly, $drawingAssembly)
[ShotKitHoverSurface.Dpi]::Enable()
$left = [int]$args[0]
$top = [int]$args[1]
$surfaceForm = [ShotKitHoverSurface.Surface]::new($args[2], $left, $top)
try { [System.Windows.Forms.Application]::Run($surfaceForm) }
finally { [ShotKitHoverSurface.Dpi]::Restore() }
'@
    [IO.File]::WriteAllText($helperFile, $helperSource, [Text.UTF8Encoding]::new($false))

    $desktopLeft = [ShotKitHoverSmoke.Native]::GetSystemMetrics(76)
    $desktopTop = [ShotKitHoverSmoke.Native]::GetSystemMetrics(77)
    $desktopWidth = [ShotKitHoverSmoke.Native]::GetSystemMetrics(78)
    $desktopHeight = [ShotKitHoverSmoke.Native]::GetSystemMetrics(79)
    if ($desktopWidth -lt 600 -or $desktopHeight -lt 480) {
        throw "Interactive desktop ${desktopWidth}x${desktopHeight} is too small for the synthetic surface."
    }
    $formLeft = $desktopLeft + 40
    $formTop = $desktopTop + 40
    $helperArgs = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" {1} {2} "{3}"' -f $helperFile, $formLeft, $formTop, $readyFile
    $surface = Start-Process -FilePath $windowsPowerShell -ArgumentList $helperArgs -WorkingDirectory $testRoot -WindowStyle Hidden -RedirectStandardOutput $helperStdout -RedirectStandardError $helperStderr -PassThru

    $geometryText = Wait-ForCondition -TimeoutSeconds 20 -Description 'the synthetic control geometry' -OwnedProcess $surface -Condition {
        if (Test-Path -LiteralPath $readyFile) { Get-Content -LiteralPath $readyFile -Raw }
    }
    $geometry = $geometryText.Trim().Split(',') | ForEach-Object { [int64]$_ }
    if ($geometry.Count -ne 5 -or $geometry[2] -lt 100 -or $geometry[3] -lt 100 -or $geometry[4] -eq 0) {
        throw "Synthetic control reported invalid geometry or HWND: '$geometryText'."
    }
    $targetX = [int]$geometry[0]
    $targetY = [int]$geometry[1]
    $targetWidth = [int]$geometry[2]
    $targetHeight = [int]$geometry[3]
    if ($targetX -lt $desktopLeft -or $targetY -lt $desktopTop -or
        $targetX + $targetWidth -gt $desktopLeft + $desktopWidth -or
        $targetY + $targetHeight -gt $desktopTop + $desktopHeight) {
        throw "Synthetic control is outside the interactive desktop: '$geometryText'."
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
    finally {
        $env:APPDATA = $oldAppData
        $env:LOCALAPPDATA = $oldLocalAppData
    }

    $mainWindow = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the ShotKit main window' -OwnedProcess $shotKit -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
    }
    $welcome = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the ShotKit startup dialog' -OwnedProcess $shotKit -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -like '*Prototyp*' } | Select-Object -First 1
    }
    $okButton = [IntPtr]::Zero
    $buttonDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $buttonDeadline -and -not $shotKit.HasExited) {
        $buttonDetails = ''
        $okButton = [ShotKitHoverSmoke.Native]::FindVisibleOkButton($welcome.Handle, [ref]$buttonDetails)
        if ($okButton -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($okButton -eq [IntPtr]::Zero) {
        $dialogTitle = $welcome.Title
        $dialogText = [ShotKitHoverSmoke.Native]::WindowText($welcome.Handle)
        $exitDetail = if ($shotKit.HasExited) { " process exited with code $($shotKit.ExitCode)." } else { '' }
        throw "Timed out waiting for a visible Button-class child titled OK (dialog HWND $($welcome.Handle)). Dialog title='$dialogTitle'; text='$dialogText'; buttons='$buttonDetails'$exitDetail"
    }
    $okControlId = [ShotKitHoverSmoke.Native]::GetDlgCtrlID($okButton)
    if (-not [ShotKitHoverSmoke.Native]::PostMessageW($okButton, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw "Could not post BM_CLICK to startup OK button HWND=$($okButton.ToInt64()) control ID=$okControlId."
    }
    try {
        $welcomeClosed = Wait-ForCondition -TimeoutSeconds 5 -Description 'the ShotKit startup dialog to close' -OwnedProcess $shotKit -AllowedDialogTitles @($welcome.Title) -Condition {
            -not (Get-ProcessDialogs $shotKit | Where-Object { $_.Handle -eq $welcome.Handle })
        }
    }
    catch {
        $dialogText = [ShotKitHoverSmoke.Native]::WindowText($welcome.Handle)
        throw "Startup dialog remained after BM_CLICK (button HWND=$($okButton.ToInt64()), control ID=$okControlId). Title='$($welcome.Title)'; text='$dialogText'. $($_.Exception.Message)"
    }
    Assert-NoUnexpectedDialog $shotKit

    $controlHandle = [IntPtr]([int64]$geometry[4])
    $surfaceRoot = [ShotKitHoverSmoke.Native]::RootWindow($controlHandle)
    if ($surfaceRoot -eq [IntPtr]::Zero) {
        throw "The synthetic control HWND $controlHandle has no GA_ROOT window."
    }
    $rootOwner = [ShotKitHoverSmoke.Native]::OwnerProcessId($surfaceRoot)
    if ($rootOwner -ne [uint32]$surface.Id) {
        throw "Refusing to show a window not owned by the helper process: root HWND=$surfaceRoot owner PID=$rootOwner expected PID=$($surface.Id)."
    }
    $rootTitle = [ShotKitHoverSmoke.Native]::WindowTitle($surfaceRoot)
    if ($rootTitle -ne 'ShotKit Hover Synthetic Surface') {
        throw "Synthetic root HWND=$surfaceRoot has unexpected title '$rootTitle'."
    }
    [ShotKitHoverSmoke.Native]::ShowNormal($surfaceRoot)
    $surfaceWindow = Wait-ForCondition -TimeoutSeconds 10 -Description 'the visible synthetic form from the stored control HWND' -OwnedProcess $surface -Condition {
        $rootOwner = [ShotKitHoverSmoke.Native]::OwnerProcessId($surfaceRoot)
        $rootTitle = [ShotKitHoverSmoke.Native]::WindowTitle($surfaceRoot)
        if ($rootOwner -eq [uint32]$surface.Id -and
            $rootTitle -eq 'ShotKit Hover Synthetic Surface' -and
            [ShotKitHoverSmoke.Native]::IsWindowVisible($surfaceRoot)) {
            [pscustomobject]@{ Handle = $surfaceRoot; Title = $rootTitle; Visible = $true }
        }
    }
    if (-not [ShotKitHoverSmoke.Native]::SetForegroundWindow($surfaceRoot)) {
        throw 'Could not activate the synthetic surface.'
    }
    [ShotKitHoverSmoke.Native]::keybd_event(0x7A, 0, 0, [UIntPtr]::Zero)
    [ShotKitHoverSmoke.Native]::keybd_event(0x7A, 0, 0x0002, [UIntPtr]::Zero)
    $overlay = Wait-ForCondition -TimeoutSeconds 10 -Description 'the F11 selection overlay' -OwnedProcess $shotKit -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible } | Select-Object -First 1
    }

    # Let sequenced HWND and UIA hover scans settle on the own control, then
    # perform a click with no drag. A rectangle drag cannot satisfy the exact
    # physical control bounds and four-quadrant pixel checks below.
    if (-not [ShotKitHoverSmoke.Native]::SetForegroundWindow($overlay.Handle)) {
        throw 'Could not activate the selection overlay.'
    }
    $pointX = $targetX + [int]($targetWidth / 2)
    $pointY = $targetY + [int]($targetHeight / 2)
    if (-not [ShotKitHoverSmoke.Native]::SetCursorPos($pointX, $pointY)) { throw 'Could not move the pointer over the synthetic control.' }
    Start-Sleep -Milliseconds 1500
    [ShotKitHoverSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 180
    [ShotKitHoverSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)

    $png = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'the PNG of the hovered control' -OwnedProcess $shotKit -Condition {
        Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Select-Object -First 1
    }
    $editor = Wait-ForCondition -TimeoutSeconds 15 -Description 'the visible editor after capture' -OwnedProcess $shotKit -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible -and $_.Title -like '*speichern*' } | Select-Object -First 1
    }

    Add-Type -AssemblyName System.Drawing
    $bitmap = [Drawing.Bitmap]::new($png.FullName)
    try {
        if ($bitmap.Width -ne $targetWidth -or $bitmap.Height -ne $targetHeight) {
            throw "Selection produced $($bitmap.Width)x$($bitmap.Height), expected hovered control $targetWidth`x$targetHeight. This indicates a different candidate or rectangle fallback."
        }
        $samples = @(
            @{ X = [int]($targetWidth * 0.25); Y = [int]($targetHeight * 0.25); R = 220; G = 35; B = 45; Name = 'top-left red' },
            @{ X = [int]($targetWidth * 0.75); Y = [int]($targetHeight * 0.25); R = 30; G = 190; B = 65; Name = 'top-right green' },
            @{ X = [int]($targetWidth * 0.25); Y = [int]($targetHeight * 0.75); R = 35; G = 70; B = 220; Name = 'bottom-left blue' },
            @{ X = [int]($targetWidth * 0.75); Y = [int]($targetHeight * 0.75); R = 235; G = 205; B = 35; Name = 'bottom-right yellow' }
        )
        foreach ($sample in $samples) {
            $actual = $bitmap.GetPixel($sample.X, $sample.Y)
            if ([Math]::Abs($actual.R - $sample.R) -gt 3 -or
                [Math]::Abs($actual.G - $sample.G) -gt 3 -or
                [Math]::Abs($actual.B - $sample.B) -gt 3) {
                throw "Hovered-control pixel '$($sample.Name)' at ($($sample.X),$($sample.Y)) was RGB=$($actual.R),$($actual.G),$($actual.B), expected $($sample.R),$($sample.G),$($sample.B)."
            }
        }
    }
    finally { $bitmap.Dispose() }

    Assert-NoUnexpectedDialog $shotKit
    $passMessage = "PASS: F11 hover-click selected synthetic control HWND $($geometry[4]) at $targetWidth`x$targetHeight and captured its four deterministic color quadrants."
}
finally {
    foreach ($owned in @($shotKit, $surface)) {
        if ($null -ne $owned) {
            try {
                if (-not $owned.HasExited) {
                    $owned.Kill()
                    if (-not $owned.WaitForExit(5000) -or -not $owned.HasExited) {
                        throw "Process '$($owned.ProcessName)' did not exit after termination was requested."
                    }
                }
            }
            catch { $cleanupFailures += "Could not stop owned process '$($owned.ProcessName)': $($_.Exception.Message)" }
            $owned.Dispose()
        }
    }
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    if ($previousDpiContext -ne [IntPtr]::Zero) {
        [void][ShotKitHoverSmoke.Native]::SetThreadDpiAwarenessContext($previousDpiContext)
    }
    if (Test-Path -LiteralPath $testRoot) {
        try { Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailures += "Could not remove smoke-test data '$testRoot': $($_.Exception.Message)" }
    }
    if ($cleanupFailures.Count -gt 0) {
        throw ($cleanupFailures -join [Environment]::NewLine)
    }
}
Write-Output $passMessage

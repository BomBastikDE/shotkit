[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 25,

    [ValidateRange(20, 180)]
    [int] $CaptureTimeoutSeconds = 75,

    [ValidateSet('None', 'Inside', 'Outside')]
    [string] $CursorClip = 'None'
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
. (Join-Path $PSScriptRoot 'shared-log-reader.ps1')
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

if (-not ('ShotKitScrollSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace ShotKitScrollSmoke {
    public struct Point { public int X; public int Y; }
    public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }
    public sealed class WindowInfo {
        public IntPtr Handle;
        public string ClassName;
        public string Title;
        public bool Visible;
    }
    public static class Native {
        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr lParam);
        private delegate bool EnumChildProc(IntPtr hwnd, IntPtr lParam);
        [DllImport("user32.dll", SetLastError = true)]
        private static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll", SetLastError = true)]
        private static extern bool CloseDesktop(IntPtr desktop);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder info, uint length, out uint needed);
        [DllImport("user32.dll")]
        private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr lParam);
        [DllImport("user32.dll")]
        private static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr lParam);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")]
        private static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")]
        public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")]
        public static extern IntPtr GetWindowDpiAwarenessContext(IntPtr hwnd);
        [DllImport("user32.dll")]
        public static extern bool AreDpiAwarenessContextsEqual(IntPtr first, IntPtr second);
        [DllImport("user32.dll")]
        public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool GetCursorPos(out Point point);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool GetClipCursor(out Rect rect);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool ClipCursor(ref Rect rect);
        [DllImport("user32.dll")]
        public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
        [DllImport("user32.dll")]
        public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
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
        private static string GetText(IntPtr hwnd) {
            StringBuilder text = new StringBuilder(512);
            GetWindowTextW(hwnd, text, text.Capacity);
            return text.ToString();
        }
        public static string WindowText(IntPtr hwnd) {
            StringBuilder all = new StringBuilder(GetText(hwnd));
            EnumChildWindows(hwnd, delegate(IntPtr child, IntPtr ignored) {
                string text = GetText(child);
                if (!String.IsNullOrWhiteSpace(text)) all.Append(" ").Append(text);
                return true;
            }, IntPtr.Zero);
            return all.ToString();
        }
        public static WindowInfo[] WindowsForProcess(int processId) {
            List<WindowInfo> windows = new List<WindowInfo>();
            EnumWindows(delegate(IntPtr hwnd, IntPtr ignored) {
                uint owner;
                GetWindowThreadProcessId(hwnd, out owner);
                if (owner != (uint)processId) return true;
                StringBuilder className = new StringBuilder(256);
                GetClassNameW(hwnd, className, className.Capacity);
                windows.Add(new WindowInfo { Handle = hwnd, ClassName = className.ToString(),
                    Title = GetText(hwnd), Visible = IsWindowVisible(hwnd) });
                return true;
            }, IntPtr.Zero);
            return windows.ToArray();
        }
    }
}
'@
}

$inputDesktop = [ShotKitScrollSmoke.Native]::InputDesktopName()
if ([string]::IsNullOrWhiteSpace($inputDesktop)) {
    throw 'Desktop session unavailable: Windows could not open the interactive input desktop.'
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Scroll-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$saveFolder = Join-Path $testRoot 'captures'
$readyFile = Join-Path $testRoot 'surface.ready'
$helperFile = Join-Path $testRoot 'surface.ps1'
$clipArmFile = Join-Path $testRoot 'cursor-clip.arm'
$clipProofFile = Join-Path $testRoot 'cursor-clip.proof'
$surfaceStdout = Join-Path $testRoot 'surface.stdout.log'
$surfaceStderr = Join-Path $testRoot 'surface.stderr.log'
$shotKit = $null
$surface = $null
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$passMessage = $null
$cleanupFailures = @()
$clipChanged = $false
$selectionButtonDown = $false
$originalClip = [ShotKitScrollSmoke.Rect]::new()

function Get-ProcessWindows {
    param([System.Diagnostics.Process] $OwnedProcess)
    [ShotKitScrollSmoke.Native]::WindowsForProcess($OwnedProcess.Id)
}
function Get-ProcessOutput {
    param([string] $Path)
    if (Test-Path -LiteralPath $Path -PathType Leaf) {
        $content = Read-SharedText -Path $Path
        return $content.Trim()
    }
    return ''
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
        Where-Object { $AllowedTitles -notcontains $_.Title } |
        Select-Object -First 1
    if ($null -ne $dialog) {
        $text = [ShotKitScrollSmoke.Native]::WindowText($dialog.Handle)
        throw "Unexpected ShotKit dialog. Title='$($dialog.Title)' Text='$text'"
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
            $stdout = Get-ProcessOutput $surfaceStdout
            $stderr = Get-ProcessOutput $surfaceStderr
            throw "Process '$($OwnedProcess.ProcessName)' exited before $Description (exit code $($OwnedProcess.ExitCode)).`nstdout:`n$stdout`nstderr:`n$stderr"
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

    # The synthetic surface exists only for this test. It draws a fixed page and
    # implements its own WM_MOUSEWHEEL scroll model; no external app/data is used.
    $helperSource = @'
param(
    [Parameter(Mandatory=$true)][string] $ReadyFile,
    [Parameter(Mandatory=$true)][string] $ClipArmFile,
    [Parameter(Mandatory=$true)][string] $ClipProofFile
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies 'System.Windows.Forms.dll', 'System.Drawing.dll' -TypeDefinition @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Forms;

namespace ShotKitScrollSurface {
    public sealed class Surface : Form {
        private const int PageWidth = 860;
        private const int PageHeight = 2300;
        private const int WheelMessage = 0x020A;
        private readonly Bitmap page;
        private readonly string readyFile;
        private readonly string clipArmFile;
        private readonly string clipProofFile;
        private bool clipApplied;
        private int offset;
        [StructLayout(LayoutKind.Sequential)]
        private struct ClipRect { public int Left, Top, Right, Bottom; }
        [StructLayout(LayoutKind.Sequential)]
        private struct CursorPoint { public int X, Y; }
        [DllImport("user32.dll", SetLastError = true)] private static extern bool ClipCursor(ref ClipRect rect);
        [DllImport("user32.dll", SetLastError = true)] private static extern bool GetClipCursor(out ClipRect rect);
        [DllImport("user32.dll", SetLastError = true)] private static extern bool GetCursorPos(out CursorPoint point);
        [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] private static extern bool SetProcessDpiAwarenessContext(IntPtr value);

        public Surface(string path, string armPath, string proofPath) {
            try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            readyFile = path;
            clipArmFile = armPath;
            clipProofFile = proofPath;
            Text = "ShotKit Synthetic Scroll Surface";
            StartPosition = FormStartPosition.Manual;
            Location = new Point(20, 20);
            ClientSize = new Size(900, 650);
            FormBorderStyle = FormBorderStyle.FixedSingle;
            MaximizeBox = false;
            MinimizeBox = false;
            ShowInTaskbar = false;
            DoubleBuffered = true;
            SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer, true);
            page = new Bitmap(PageWidth, PageHeight, PixelFormat.Format32bppArgb);
            FillPage();
        }

        private void FillPage() {
            Rectangle bounds = new Rectangle(0, 0, page.Width, page.Height);
            BitmapData data = page.LockBits(bounds, ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
            try {
                byte[] bytes = new byte[data.Stride * page.Height];
                for (int y = 0; y < page.Height; y++) {
                    for (int x = 0; x < page.Width; x++) {
                        int i = y * data.Stride + x * 4;
                        bytes[i] = (byte)((x * 19 + y * 11 + (y / 11) * 41) & 255);     // B
                        bytes[i + 1] = (byte)((x * 7 + y * 31 + (y / 23) * 97) & 255); // G
                        bytes[i + 2] = (byte)((x * 29 + y * 17 + (y / 19) * 53) & 255);// R
                        bytes[i + 3] = 255;
                    }
                }
                Marshal.Copy(bytes, 0, data.Scan0, bytes.Length);
            } finally { page.UnlockBits(data); }
        }

        protected override void OnShown(EventArgs e) {
            base.OnShown(e);
            Point origin = PointToScreen(Point.Empty);
            File.WriteAllText(readyFile, String.Format("{0},{1},{2},{3}", origin.X, origin.Y, ClientSize.Width, ClientSize.Height));
        }

        protected override void OnActivated(EventArgs e) {
            base.OnActivated(e);
            if (clipApplied || String.IsNullOrEmpty(clipArmFile) ||
                !File.Exists(clipArmFile) || GetForegroundWindow() != Handle) return;
            string mode = File.ReadAllText(clipArmFile).Trim();
            if (mode != "Inside" && mode != "Outside") throw new InvalidOperationException("Invalid cursor clip mode.");
            // Arm only after the selection endpoint is established. Apply here,
            // after the overlay gives foreground back to this owned surface.
            // ShotKit's existing activation-message barrier runs before its
            // capture worker starts, so no timer/poll races are needed here.
            Point origin = PointToScreen(Point.Empty);
            ClipRect expected = new ClipRect {
                Left = origin.X + (mode == "Inside" ? 676 : 680),
                Top = origin.Y + 60, Right = origin.X + 681, Bottom = origin.Y + 461
            };
            clipApplied = true;
            if (!ClipCursor(ref expected)) throw new InvalidOperationException("Foreground ClipCursor failed.");
            File.WriteAllText(clipProofFile, ClipEvidence("active") + Environment.NewLine);
        }

        private string ClipEvidence(string stage) {
            ClipRect rect;
            CursorPoint point;
            if (!GetClipCursor(out rect) || !GetCursorPos(out point))
                throw new InvalidOperationException("Could not read active cursor clipping.");
            return String.Format("{0},{1},{2},{3},{4},{5},{6}",
                stage, rect.Left, rect.Top, rect.Right, rect.Bottom, point.X, point.Y);
        }

        protected override void WndProc(ref Message m) {
            if (m.Msg == WheelMessage) {
                if (clipApplied)
                    File.AppendAllText(clipProofFile, ClipEvidence("wheel") + Environment.NewLine);
                int delta = unchecked((short)((m.WParam.ToInt64() >> 16) & 0xffff));
                int max = Math.Max(0, PageHeight - ClientSize.Height);
                offset = Math.Max(0, Math.Min(max, offset - delta));
                Invalidate();
                m.Result = IntPtr.Zero;
                return;
            }
            base.WndProc(ref m);
        }

        protected override void OnPaint(PaintEventArgs e) {
            e.Graphics.DrawImage(page, new Rectangle(0, 0, PageWidth, ClientSize.Height),
                new Rectangle(0, offset, PageWidth, ClientSize.Height), GraphicsUnit.Pixel);
            using (Brush track = new SolidBrush(Color.FromArgb(220, 220, 220)))
            using (Brush thumb = new SolidBrush(Color.FromArgb(80, 80, 80))) {
                e.Graphics.FillRectangle(track, 878, 0, 14, ClientSize.Height);
                int thumbHeight = Math.Max(40, ClientSize.Height * ClientSize.Height / PageHeight);
                int max = Math.Max(1, PageHeight - ClientSize.Height);
                int thumbTop = (ClientSize.Height - thumbHeight) * offset / max;
                e.Graphics.FillRectangle(thumb, 880, thumbTop, 10, thumbHeight);
            }
            base.OnPaint(e);
        }

        protected override void Dispose(bool disposing) {
            if (disposing) page.Dispose();
            base.Dispose(disposing);
        }
    }
}
"@
$surfaceForm = [ShotKitScrollSurface.Surface]::new($ReadyFile, $ClipArmFile, $ClipProofFile)
[System.Windows.Forms.Application]::Run($surfaceForm)
'@
    [IO.File]::WriteAllText($helperFile, $helperSource, [System.Text.UTF8Encoding]::new($false))

    # Run the WinForms helper under Windows PowerShell 5.1. The CI step may
    # itself use pwsh; using that host for the .NET Framework WinForms helper
    # can fail before the form is created, and previously hid the diagnostic.
    $powershellExe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (-not (Test-Path -LiteralPath $powershellExe -PathType Leaf)) {
        throw "Windows PowerShell 5.1 was not found at '$powershellExe'."
    }
    $helperArgs = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -ReadyFile "{1}" -ClipArmFile "{2}" -ClipProofFile "{3}"' -f $helperFile, $readyFile, $clipArmFile, $clipProofFile
    $surface = Start-Process -FilePath $powershellExe -ArgumentList $helperArgs -WorkingDirectory $testRoot -WindowStyle Hidden -RedirectStandardOutput $surfaceStdout -RedirectStandardError $surfaceStderr -PassThru

    $surfaceWindow = Wait-ForCondition -TimeoutSeconds 20 -Description 'the synthetic scroll surface' -OwnedProcess $surface -Condition {
        Get-ProcessWindows $surface | Where-Object { $_.Title -eq 'ShotKit Synthetic Scroll Surface' -and $_.Visible } | Select-Object -First 1
    }
    $ready = Wait-ForCondition -TimeoutSeconds 10 -Description 'the synthetic surface geometry' -OwnedProcess $surface -Condition {
        if (Test-Path -LiteralPath $readyFile) { Get-Content -LiteralPath $readyFile -Raw }
    }
    $geometry = $ready.Trim().Split(',') | ForEach-Object { [int]$_ }
    if ($geometry.Count -ne 4 -or $geometry[2] -ne 900 -or $geometry[3] -ne 650) {
        throw "Synthetic surface reported unexpected client geometry: '$ready'."
    }
    $desktopWidth = [ShotKitScrollSmoke.Native]::GetSystemMetrics(78)
    $desktopHeight = [ShotKitScrollSmoke.Native]::GetSystemMetrics(79)
    if ($desktopWidth -lt 940 -or $desktopHeight -lt 720 -or
        $geometry[0] + 680 -gt $desktopWidth -or $geometry[1] + 460 -gt $desktopHeight) {
        throw "Interactive desktop ${desktopWidth}x${desktopHeight} cannot fit the controlled test window and selection."
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
    [IO.File]::WriteAllText($settingsPath, $settings, [System.Text.UTF8Encoding]::new($false))
    # An unchanged direction must not rewrite the settings file. Use a past
    # timestamp as well as exact bytes so even an identical rewrite is caught.
    [IO.File]::SetLastWriteTimeUtc($settingsPath, [DateTime]::UtcNow.AddDays(-1))
    $initialSettingsBytes = [Convert]::ToBase64String([IO.File]::ReadAllBytes($settingsPath))
    $initialSettingsWriteTicks = [IO.File]::GetLastWriteTimeUtc($settingsPath).Ticks

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
    $dpiContext = [ShotKitScrollSmoke.Native]::GetWindowDpiAwarenessContext($mainWindow.Handle)
    if (-not [ShotKitScrollSmoke.Native]::AreDpiAwarenessContextsEqual($dpiContext, [IntPtr](-4))) {
        throw "ShotKit window did not start in the required PerMonitorV2 context: $dpiContext."
    }
    Assert-NoStartupDialogs -TimeoutSeconds $StartupTimeoutSeconds -OwnedProcess $shotKit -GetWindows { Get-ProcessWindows $shotKit }
    Assert-NoUnexpectedDialog $shotKit

    # Place the synthetic form in front and invoke ShotKit's registered F10
    # scroll-capture hotkey. This never targets windows outside this test.
    if (-not [ShotKitScrollSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)) {
        throw 'Could not activate the synthetic test surface.'
    }
    [ShotKitScrollSmoke.Native]::keybd_event(0x79, 0, 0, [UIntPtr]::Zero)
    [ShotKitScrollSmoke.Native]::keybd_event(0x79, 0, 0x0002, [UIntPtr]::Zero)
    $directionDialog = Wait-ForCondition -TimeoutSeconds 10 -Description 'the scroll direction prompt' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Richtung' } | Select-Object -First 1
    }
    # MessageBox IDYES selects the vertical downward direction.
    if (-not [ShotKitScrollSmoke.Native]::PostMessageW($directionDialog.Handle, 0x0111, [IntPtr]6, [IntPtr]::Zero)) {
        throw 'Could not select downward scrolling.'
    }

    $overlay = Wait-ForCondition -TimeoutSeconds 10 -Description 'the scroll-selection overlay' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible } | Select-Object -First 1
    }
    if ([Convert]::ToBase64String([IO.File]::ReadAllBytes($settingsPath)) -cne $initialSettingsBytes -or
        [IO.File]::GetLastWriteTimeUtc($settingsPath).Ticks -ne $initialSettingsWriteTicks) {
        throw 'Unchanged scroll direction unexpectedly rewrote the settings file.'
    }
    $originX = $geometry[0]
    $originY = $geometry[1]
    $startX = $originX + 80
    $startY = $originY + 60
    $endX = $startX + 600
    $endY = $startY + 400
    [void][ShotKitScrollSmoke.Native]::SetForegroundWindow($overlay.Handle)
    if (-not [ShotKitScrollSmoke.Native]::SetCursorPos($startX, $startY)) { throw 'Could not position the selection start.' }
    Start-Sleep -Milliseconds 150
    [ShotKitScrollSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    $selectionButtonDown = $true
    [void][ShotKitScrollSmoke.Native]::SetCursorPos($endX, $endY)
    Start-Sleep -Milliseconds 200
    if ($CursorClip -ne 'None') {
        if (-not [ShotKitScrollSmoke.Native]::GetClipCursor([ref]$originalClip)) {
            throw 'Could not save the original cursor clip rectangle.'
        }
        # Let the owned target apply clipping when it regains foreground.
        # A pre-mouse-up probe did not establish the condition during scrolling:
        # the previous Outside CI run incorrectly reached the real page end.
        $clipChanged = $true
        [IO.File]::WriteAllText($clipArmFile, $CursorClip, [System.Text.UTF8Encoding]::new($false))
    }
    [ShotKitScrollSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    $selectionButtonDown = $false
    if ($CursorClip -ne 'None') {
        $activeClip = Wait-ForCondition -TimeoutSeconds 10 -Description 'cursor clipping applied by the foreground test surface' -OwnedProcess $surface -Condition {
            if (Test-Path -LiteralPath $clipProofFile -PathType Leaf) {
                $content = Read-SharedText -Path $clipProofFile
                if ($content.Contains("`n")) {
                    $lines = @($content.Trim() -split '\r?\n')
                    if ($lines[0].StartsWith('active,')) { $lines[0] }
                }
            }
        }
        $clipLeft = if ($CursorClip -eq 'Inside') { $endX - 4 } else { $endX }
        $clipBounds = '{0},{1},{2},{3}' -f $clipLeft, $startY, ($endX + 1), ($endY + 1)
        $expectedActiveClip = 'active,{0},{1},{2}' -f $clipBounds, $endX, $endY
        if ($activeClip -cne $expectedActiveClip) {
            throw "Foreground cursor clip mismatch. Expected='$expectedActiveClip' Actual='$activeClip'"
        }
        Write-Output "MILESTONE: foreground surface established $activeClip."
    }

    $png = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'an auto-saved scroll-capture PNG' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Select-Object -First 1
    }
    $completion = Wait-ForCondition -TimeoutSeconds 15 -Description 'the normal scroll-stop notification' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Aufnahme beendet' } | Select-Object -First 1
    }
    $completionText = [ShotKitScrollSmoke.Native]::WindowText($completion.Handle)
    if ($completionText -notlike '*Möglicherweise unvollständig: Das Seitenende wurde nicht bestätigt.*') {
        throw "Scroll did not warn that the document end is unconfirmed. Dialog='$completionText'"
    }
    if ($CursorClip -ne 'None') {
        $clipEvidence = @((Read-SharedText -Path $clipProofFile).Trim() -split '\r?\n')
        $wheelEvidence = @($clipEvidence | Where-Object { $_.StartsWith('wheel,') })
        Write-Output "DIAGNOSTIC: cursor clipping evidence: $($clipEvidence -join '; ')"
    }
    $expectedReason = if ($CursorClip -eq 'Outside') {
        'Mauszeiger kann nicht im Aufnahmebereich positioniert werden'
    } else { 'Keine weitere Scrollbewegung erkennbar.' }
    if ($completionText -notlike "*$expectedReason*") {
        throw "Scroll completion mismatch. Expected='$expectedReason' Dialog='$completionText'"
    }

    if ($CursorClip -ne 'None') {
        if ($CursorClip -eq 'Outside') {
            if ($wheelEvidence.Count -ne 0) {
                throw "Outside clipping must reject scrolling before any wheel input: $($wheelEvidence -join '; ')"
            }
        }
        else {
            $expectedWheel = 'wheel,{0},{1},{2}' -f $clipBounds, $clipLeft, ($startY + 200)
            $invalidWheelEvidence = @($wheelEvidence | Where-Object { $_ -cne $expectedWheel })
            if ($wheelEvidence.Count -eq 0 -or $invalidWheelEvidence.Count -ne 0) {
                throw "Wheel input did not prove actual cursor clamping. Expected='$expectedWheel' Actual='$($wheelEvidence -join '; ')'"
            }
            Write-Output "MILESTONE: all $($wheelEvidence.Count) wheel events proved the clamped cursor: $expectedWheel."
        }
    }

    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::new($png.FullName)
    try {
        $expectedWidth = 600
        # Outside clipping must preserve the first viewport; inside clipping
        # must still reconstruct every row through the real bottom of the page.
        $expectedHeight = if ($CursorClip -eq 'Outside') { 400 } else { 2050 }
        if ($bitmap.Width -ne $expectedWidth -or $bitmap.Height -ne $expectedHeight) {
            throw "Captured image dimensions were $($bitmap.Width)x$($bitmap.Height); expected ${expectedWidth}x${expectedHeight}."
        }
        # Verify every output row at multiple horizontal positions against the
        # deterministic source page. This catches missing/repeated seams and an
        # early or late stop while keeping the check quick on hosted runners.
        foreach ($y in 0..($expectedHeight - 1)) {
            $sourceY = $y + 60
            foreach ($x in @(0, 73, 211, 389, 599)) {
                $sourceX = $x + 80
                $expectedR = ($sourceX * 29 + $sourceY * 17 + [int][Math]::Floor($sourceY / 19) * 53) % 256
                $expectedG = ($sourceX * 7 + $sourceY * 31 + [int][Math]::Floor($sourceY / 23) * 97) % 256
                $expectedB = ($sourceX * 19 + $sourceY * 11 + [int][Math]::Floor($sourceY / 11) * 41) % 256
                $actual = $bitmap.GetPixel($x, $y)
                if ($actual.R -ne $expectedR -or $actual.G -ne $expectedG -or $actual.B -ne $expectedB) {
                    throw "Pixel sequence mismatch at output ($x,$y), source ($sourceX,$sourceY): actual RGB=$($actual.R),$($actual.G),$($actual.B), expected=$expectedR,$expectedG,$expectedB."
                }
            }
        }
    }
    finally { $bitmap.Dispose() }

    # The modal completion notice may remain open until the owned process is
    # stopped. Its text and the saved pixels have already been verified; check
    # the editor behind it without depending on dialog-dismissal timing.
    $editor = Wait-ForCondition -TimeoutSeconds 15 -Description 'the visible editor window' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ProcessWindows $shotKit | Where-Object {
            $_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible -and $_.Title -like '*speichern*'
        } | Select-Object -First 1
    }
    Assert-NoUnexpectedDialog $shotKit @('Scroll-Aufnahme beendet')
    $passMessage = "PASS: cursor clip=$CursorClip; reason=$expectedReason; verified all $expectedHeight deterministic rows in $($png.Name)."
}
finally {
    # A failed fixture setup can happen during the owned selection drag. Release
    # only the button this test pressed, and still restore clipping if it fails.
    if ($selectionButtonDown) {
        try { [ShotKitScrollSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) }
        catch { $cleanupFailures += "Could not release the selection button: $($_.Exception.Message)" }
    }
    # Disarm before stopping either actor; a surface not yet activated must
    # not acquire clipping because terminating ShotKit brings it to the front.
    if (Test-Path -LiteralPath $clipArmFile -PathType Leaf) {
        try { Remove-Item -LiteralPath $clipArmFile -Force -ErrorAction Stop }
        catch { $cleanupFailures += "Could not disarm cursor clipping: $($_.Exception.Message)" }
    }
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
    # Stop both owned actors first, so no delayed activation can overwrite
    # the restored global desktop state. Restoration failure fails the test.
    if ($clipChanged) {
        try {
            if (-not [ShotKitScrollSmoke.Native]::ClipCursor([ref]$originalClip)) {
                throw 'Could not restore the original cursor clip rectangle.'
            }
        }
        catch { $cleanupFailures += $_.Exception.Message }
    }
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    if (Test-Path -LiteralPath $testRoot) {
        try { Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailures += "Could not remove smoke-test data: $($_.Exception.Message)" }
    }
    if ($cleanupFailures.Count -gt 0) {
        throw ($cleanupFailures -join [Environment]::NewLine)
    }
}
Write-Output $passMessage

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 25,

    [ValidateRange(20, 180)]
    [int] $CaptureTimeoutSeconds = 75,

    [switch] $MouseAbort
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
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: refusing to target or change '$imageName'."
}

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
        public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")]
        public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")]
        public static extern bool GetCursorPos(out Point point);
        public static Point CursorPosition() {
            Point point;
            if (!GetCursorPos(out point)) throw new InvalidOperationException("GetCursorPos failed.");
            return point;
        }
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

        public static int WindowOwner(IntPtr hwnd) {
            uint owner;
            GetWindowThreadProcessId(hwnd, out owner);
            return (int)owner;
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
$eventFile = Join-Path $testRoot 'surface.events'
$helperFile = Join-Path $testRoot 'surface.ps1'
$surfaceStdout = Join-Path $testRoot 'surface.stdout.log'
$surfaceStderr = Join-Path $testRoot 'surface.stderr.log'
$shotKit = $null
$surface = $null
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$passMessage = $null
$cleanupFailures = @()

function Get-ProcessWindows {
    param([System.Diagnostics.Process] $OwnedProcess)
    [ShotKitScrollSmoke.Native]::WindowsForProcess($OwnedProcess.Id)
}
function Get-ProcessOutput {
    param([string] $Path)
    if (Test-Path -LiteralPath $Path -PathType Leaf) {
        $content = [IO.File]::ReadAllText($Path)
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
        Where-Object { $AllowedTitles -notcontains $_.Title -and $_.Title -notlike '*Prototyp*' } |
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
param([Parameter(Mandatory=$true)][string] $ReadyFile, [Parameter(Mandatory=$true)][string] $EventFile)
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
        private readonly string eventFile;
        private int offset;
        private int wheelEvents;
        [DllImport("user32.dll")] private static extern bool SetProcessDpiAwarenessContext(IntPtr value);

        public Surface(string path, string events) {
            try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            readyFile = path;
            eventFile = events;
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

        protected override void WndProc(ref Message m) {
            if (m.Msg == WheelMessage) {
                int delta = unchecked((short)((m.WParam.ToInt64() >> 16) & 0xffff));
                int max = Math.Max(0, PageHeight - ClientSize.Height);
                offset = Math.Max(0, Math.Min(max, offset - delta));
                wheelEvents++;
                File.WriteAllText(eventFile, String.Format("{0},{1}", wheelEvents, offset));
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
$surfaceForm = [ShotKitScrollSurface.Surface]::new($ReadyFile, $EventFile)
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
    $helperArgs = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -ReadyFile "{1}" -EventFile "{2}"' -f $helperFile, $readyFile, $eventFile
    $surface = Start-Process -FilePath $powershellExe -ArgumentList $helperArgs -WorkingDirectory $testRoot -WindowStyle Hidden -RedirectStandardOutput $surfaceStdout -RedirectStandardError $surfaceStderr -PassThru

    $surfaceWindow = Wait-ForCondition -TimeoutSeconds 20 -Description 'the synthetic scroll surface' -OwnedProcess $surface -Condition {
        Get-ProcessWindows $surface | Where-Object { $_.Title -eq 'ShotKit Synthetic Scroll Surface' -and $_.Visible } | Select-Object -First 1
    }
    if ([ShotKitScrollSmoke.Native]::WindowOwner($surfaceWindow.Handle) -ne $surface.Id) {
        throw "SURFACE_OWNER_MISMATCH: HWND=$($surfaceWindow.Handle), expected PID=$($surface.Id)."
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
    if ([ShotKitScrollSmoke.Native]::WindowOwner($mainWindow.Handle) -ne $shotKit.Id) {
        throw "SHOTKIT_WINDOW_OWNER_MISMATCH: HWND=$($mainWindow.Handle), expected PID=$($shotKit.Id)."
    }
    $welcome = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the ShotKit startup dialog' -OwnedProcess $shotKit -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -like '*Prototyp*' } | Select-Object -First 1
    }
    if (-not [ShotKitScrollSmoke.Native]::PostMessageW($welcome.Handle, 0x0111, [IntPtr]1, [IntPtr]::Zero)) {
        throw 'Could not close the ShotKit startup dialog.'
    }
    Start-Sleep -Milliseconds 250
    Assert-NoUnexpectedDialog $shotKit

    # Place the synthetic form in front and invoke ShotKit's registered F10
    # scroll-capture hotkey. This never targets windows outside this test.
    if (-not [ShotKitScrollSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)) {
        throw 'Could not activate the synthetic test surface.'
    }
    if ([ShotKitScrollSmoke.Native]::GetForegroundWindow() -ne $surfaceWindow.Handle) {
        throw "SURFACE_NOT_FOREGROUND_BEFORE_HOTKEY: expected HWND=$($surfaceWindow.Handle), actual=$([ShotKitScrollSmoke.Native]::GetForegroundWindow())."
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
    if ([ShotKitScrollSmoke.Native]::WindowOwner($overlay.Handle) -ne $shotKit.Id) {
        throw "OVERLAY_OWNER_MISMATCH: HWND=$($overlay.Handle), expected PID=$($shotKit.Id)."
    }
    $originX = $geometry[0]
    $originY = $geometry[1]
    $startX = $originX + 80
    $startY = $originY + 60
    $endX = $startX + 600
    $endY = $startY + 400
    [void][ShotKitScrollSmoke.Native]::SetForegroundWindow($overlay.Handle)
    if ([ShotKitScrollSmoke.Native]::GetForegroundWindow() -ne $overlay.Handle) {
        throw "OVERLAY_NOT_FOREGROUND_FOR_SELECTION: expected HWND=$($overlay.Handle), actual=$([ShotKitScrollSmoke.Native]::GetForegroundWindow())."
    }
    if (-not [ShotKitScrollSmoke.Native]::SetCursorPos($startX, $startY)) { throw 'Could not position the selection start.' }
    Start-Sleep -Milliseconds 150
    [ShotKitScrollSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    [void][ShotKitScrollSmoke.Native]::SetCursorPos($endX, $endY)
    Start-Sleep -Milliseconds 200
    [ShotKitScrollSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)

    # The second observed wheel event can only be issued after ShotKit has
    # captured, matched, and stitched the first scrolled viewport. The default
    # Escape path remains unchanged; -MouseAbort moves the cursor while the
    # second movement is pending, before it can be captured or stitched.
    $secondWheel = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'a second observable wheel event after the first stitched step' -OwnedProcess $shotKit -Condition {
        if (Test-Path -LiteralPath $eventFile) {
            $eventState = [IO.File]::ReadAllText($eventFile).Trim()
            if ($eventState -match '^(\d+),(\d+)$' -and [int]$Matches[1] -ge 2 -and [int]$Matches[2] -gt 0) { @($Matches[1], $Matches[2]) }
        }
    }
    $firstStepOffset = [int]$secondWheel[1] / 2
    if ($firstStepOffset -le 0) { throw "Synthetic surface did not demonstrate a positive first scroll offset: '$secondWheel'." }
    if ($MouseAbort) {
        if ([int]$secondWheel[0] -ne 2 -or [int]$secondWheel[1] -ne 240) {
            throw "MOUSE_ABORT_STEP_GATE_FAILED: expected exactly two 120px positive wheel events before input, observed count=$($secondWheel[0]) offset=$($secondWheel[1])."
        }
        if ([ShotKitScrollSmoke.Native]::InputDesktopName() -ne $inputDesktop) {
            throw 'INPUT_DESKTOP_CHANGED before mouse-abort trigger.'
        }
        if ([ShotKitScrollSmoke.Native]::GetForegroundWindow() -ne $surfaceWindow.Handle) {
            throw "TARGET_NOT_FOREGROUND_BEFORE_MOUSE_ABORT: expected surface HWND=$($surfaceWindow.Handle), actual=$([ShotKitScrollSmoke.Native]::GetForegroundWindow())."
        }

        # WindowsScrollIo uses the right edge and vertical midpoint of the
        # 600x400 selection as its expected cursor position for downward scroll.
        $expectedCursorX = $startX + 600 - 8
        $expectedCursorY = $startY + 200
        $cursorBefore = [ShotKitScrollSmoke.Native]::CursorPosition()
        if ($cursorBefore.X -ne $expectedCursorX -or $cursorBefore.Y -ne $expectedCursorY) {
            throw "SCROLL_CURSOR_IDENTITY_MISMATCH: observed ($($cursorBefore.X),$($cursorBefore.Y)), expected ($expectedCursorX,$expectedCursorY)."
        }

        # Move only within the owned synthetic surface, outside the selected
        # capture rectangle, without clicking or sending Escape.
        $moveX = $originX + 20
        $moveY = $originY + 20
        if ($moveX -lt $originX -or $moveY -lt $originY -or
            $moveX -ge ($originX + $geometry[2]) -or $moveY -ge ($originY + $geometry[3]) -or
            ($moveX -ge $startX -and $moveX -lt $endX -and $moveY -ge $startY -and $moveY -lt $endY)) {
            throw "MOUSE_ABORT_TARGET_OUTSIDE_SAFE_SURFACE: point=($moveX,$moveY), surface=($originX,$originY,$($geometry[2]),$($geometry[3]))."
        }
        if (-not [ShotKitScrollSmoke.Native]::SetCursorPos($moveX, $moveY)) {
            throw 'Could not move the cursor within the owned synthetic surface.'
        }
        $cursorAfter = [ShotKitScrollSmoke.Native]::CursorPosition()
        if ($cursorAfter.X -ne $moveX -or $cursorAfter.Y -ne $moveY) {
            throw "MOUSE_ABORT_CURSOR_MOVE_MISMATCH: requested ($moveX,$moveY), observed ($($cursorAfter.X),$($cursorAfter.Y))."
        }
        if ([ShotKitScrollSmoke.Native]::GetForegroundWindow() -ne $surfaceWindow.Handle) {
            throw "TARGET_FOREGROUND_CHANGED_DURING_MOUSE_ABORT: expected surface HWND=$($surfaceWindow.Handle), actual=$([ShotKitScrollSmoke.Native]::GetForegroundWindow())."
        }
        Write-Output "MILESTONE: first 120px scroll is committed; second wheel is pending; moved cursor within owned test surface from ($($cursorBefore.X),$($cursorBefore.Y)) to ($moveX,$moveY) without click/Escape."
    }
    else {
        Write-Output "MILESTONE: observed two real wheel events; first-step offset=$firstStepOffset proves the first viewport was followed by another scroll request. Sending Escape now."
        [ShotKitScrollSmoke.Native]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 120
        [ShotKitScrollSmoke.Native]::keybd_event(0x1B, 0, 0x0002, [UIntPtr]::Zero)
    }

    $expectedReason = if ($MouseAbort) { 'Durch Mausbewegung beendet.' } else { 'Mit Esc beendet.' }
    $completion = Wait-ForCondition -TimeoutSeconds 20 -Description 'the scroll cancellation notification' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Aufnahme beendet' } | Select-Object -First 1
    }
    if ([ShotKitScrollSmoke.Native]::WindowOwner($completion.Handle) -ne $shotKit.Id) {
        throw "COMPLETION_DIALOG_OWNER_MISMATCH: HWND=$($completion.Handle), expected PID=$($shotKit.Id)."
    }
    $completionText = [ShotKitScrollSmoke.Native]::WindowText($completion.Handle)
    if ($completionText -notlike "*$expectedReason*") {
        throw "Scroll cancellation reason mismatch. Expected='$expectedReason' Dialog='$completionText'"
    }
    if ($MouseAbort) {
        $finalEvent = [IO.File]::ReadAllText($eventFile).Trim()
        if ($finalEvent -cne '2,240') {
            throw "MOUSE_ABORT_STEPPED_AFTER_TRIGGER: expected event state 2,240, found '$finalEvent'."
        }
    }
    $pngs = @(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File)
    if ($pngs.Count -ne 1) { throw "Expected one auto-saved partial PNG, found $($pngs.Count)." }
    $png = $pngs[0]

    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::new($png.FullName)
    try {
        $expectedWidth = 600
        # The first viewport (400 rows) plus the successfully matched 120px
        # scroll step must be present; the later wheel event is cancelled.
        $expectedHeight = 520
        if ($bitmap.Width -ne $expectedWidth -or $bitmap.Height -ne $expectedHeight) {
            throw "Partial image dimensions were $($bitmap.Width)x$($bitmap.Height); expected ${expectedWidth}x${expectedHeight}."
        }
        foreach ($y in 0..($expectedHeight - 1)) {
            $sourceY = $y + 60
            foreach ($x in @(0, 73, 211, 389, 599)) {
                $sourceX = $x + 80
                $expectedR = ($sourceX * 29 + $sourceY * 17 + [int][Math]::Floor($sourceY / 19) * 53) % 256
                $expectedG = ($sourceX * 7 + $sourceY * 31 + [int][Math]::Floor($sourceY / 23) * 97) % 256
                $expectedB = ($sourceX * 19 + $sourceY * 11 + [int][Math]::Floor($sourceY / 11) * 41) % 256
                $actual = $bitmap.GetPixel($x, $y)
                if ($actual.R -ne $expectedR -or $actual.G -ne $expectedG -or $actual.B -ne $expectedB) {
                    throw "Partial pixel mismatch at output ($x,$y), source ($sourceX,$sourceY): actual RGB=$($actual.R),$($actual.G),$($actual.B), expected=$expectedR,$expectedG,$expectedB."
                }
            }
        }
    }
    finally { $bitmap.Dispose() }

    $editor = Wait-ForCondition -TimeoutSeconds 15 -Description 'the visible editor window containing the partial capture' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ProcessWindows $shotKit | Where-Object {
            $_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible -and $_.Title -like '*speichern*'
        } | Select-Object -First 1
    }
    Assert-NoUnexpectedDialog $shotKit @('Scroll-Aufnahme beendet')
    if ($MouseAbort) {
        $passMessage = "PASS: cursor movement inside the owned synthetic surface produced '$expectedReason'; saved $($png.Name) as a pixel-exact ${expectedWidth}x${expectedHeight} prefix after exactly one committed 120px step."
    }
    else {
        $passMessage = "PASS: Escape produced the expected '$expectedReason' outcome and saved $($png.Name) as a pixel-exact ${expectedWidth}x${expectedHeight} stitched prefix after an observed successful scroll step."
    }
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
    if (Test-Path -LiteralPath $testRoot) {
        try { Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailures += "Could not remove smoke-test data: $($_.Exception.Message)" }
    }
    if ($cleanupFailures.Count -gt 0) {
        throw ($cleanupFailures -join [Environment]::NewLine)
    }
}
Write-Output $passMessage

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 25,

    [ValidateRange(20, 180)]
    [int] $CaptureTimeoutSeconds = 75
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
. (Join-Path $PSScriptRoot 'shared-log-reader.ps1')
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') {
    throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'
}
if (($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT') {
    throw 'This smoke test requires an ephemeral GitHub-hosted Windows runner.'
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
    throw "PREEXISTING_PROCESS: A process named '$imageName' is already running; no existing process will be used or changed."
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
    public sealed class ButtonInfo {
        public IntPtr Handle;
        public int ControlId;
        public string Text;
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
        [DllImport("user32.dll", SetLastError = true)]
        private static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern int SendMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")]
        public static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
        [DllImport("user32.dll", SetLastError = true)]
        public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")]
        public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")]
        private static extern bool AttachThreadInput(uint attach, uint attachTo, bool attachState);
        [DllImport("user32.dll")]
        private static extern IntPtr SetFocus(IntPtr hwnd);
        [DllImport("kernel32.dll")]
        private static extern uint GetCurrentThreadId();
        [DllImport("user32.dll")]
        public static extern bool ShowWindow(IntPtr hwnd, int command);
        [DllImport("user32.dll")]
        public static extern bool BringWindowToTop(IntPtr hwnd);
        [DllImport("user32.dll")]
        public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")]
        public static extern bool SetCursorPos(int x, int y);
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
        public static ButtonInfo[] VisibleButtons(IntPtr dialog) {
            List<ButtonInfo> buttons = new List<ButtonInfo>();
            EnumChildProc callback = delegate(IntPtr child, IntPtr ignored) {
                if (!IsWindowVisible(child)) return true;
                StringBuilder cls = new StringBuilder(128);
                GetClassNameW(child, cls, cls.Capacity);
                if (!String.Equals(cls.ToString(), "Button", StringComparison.OrdinalIgnoreCase)) return true;
                StringBuilder text = new StringBuilder(256);
                GetWindowTextW(child, text, text.Capacity);
                buttons.Add(new ButtonInfo { Handle = child, ControlId = GetDlgCtrlID(child), Text = text.ToString() });
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
        public static uint OwnerProcessId(IntPtr hwnd) {
            uint processId;
            GetWindowThreadProcessId(hwnd, out processId);
            return processId;
        }
        public static void ShowNormal(IntPtr hwnd) { ShowWindow(hwnd, 1); }
        public static string WindowTitle(IntPtr hwnd) { return GetText(hwnd); }
        public static string WindowClass(IntPtr hwnd) {
            StringBuilder className = new StringBuilder(256);
            GetClassNameW(hwnd, className, className.Capacity);
            return className.ToString();
        }
        public static bool ActivateOwnedWindow(IntPtr hwnd, uint expectedProcessId) {
            if (hwnd == IntPtr.Zero || OwnerProcessId(hwnd) != expectedProcessId) return false;
            ShowWindow(hwnd, 1);
            uint currentThread = GetCurrentThreadId();
            uint ignoredProcessId;
            uint targetThread = GetWindowThreadProcessId(hwnd, out ignoredProcessId);
            IntPtr foreground = GetForegroundWindow();
            uint foregroundThread = 0;
            if (foreground != IntPtr.Zero)
                foregroundThread = GetWindowThreadProcessId(foreground, out ignoredProcessId);
            bool attachedForeground = false;
            bool attachedTarget = false;
            try {
                if (foregroundThread != 0 && foregroundThread != currentThread)
                    attachedForeground = AttachThreadInput(currentThread, foregroundThread, true);
                if (targetThread != 0 && targetThread != currentThread && targetThread != foregroundThread)
                    attachedTarget = AttachThreadInput(currentThread, targetThread, true);
                BringWindowToTop(hwnd);
                SetForegroundWindow(hwnd);
                SetFocus(hwnd);
                return OwnerProcessId(GetForegroundWindow()) == expectedProcessId;
            } finally {
                if (attachedTarget) AttachThreadInput(currentThread, targetThread, false);
                if (attachedForeground) AttachThreadInput(currentThread, foregroundThread, false);
            }
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
function Click-DialogButton {
    param(
        [System.Diagnostics.Process] $OwnedProcess,
        $Dialog,
        [string[]] $ExpectedTexts,
        [string] $Description
    )
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    $buttons = @()
    while ([DateTime]::UtcNow -lt $deadline -and -not $OwnedProcess.HasExited) {
        $buttons = @([ShotKitScrollSmoke.Native]::VisibleButtons($Dialog.Handle))
        $button = $buttons | Where-Object {
            $ExpectedTexts -contains (($_.Text -replace '&', '').Trim())
        } | Select-Object -First 1
        if ($null -ne $button) {
            [ShotKitScrollSmoke.Native]::ClickButton($button.Handle)
            return $button
        }
        Start-Sleep -Milliseconds 100
    }
    $details = ($buttons | ForEach-Object {
        "HWND=0x$($_.Handle.ToInt64().ToString('X')) ID=$($_.ControlId) Text='$($_.Text)'"
    }) -join '; '
    if (-not $details) { $details = 'no visible Button-class children' }
    $dialogText = [ShotKitScrollSmoke.Native]::WindowText($Dialog.Handle)
    $exitDetail = if ($OwnedProcess.HasExited) { " Process exited with code $($OwnedProcess.ExitCode)." } else { '' }
    throw "BUTTON_TIMEOUT: $Description on dialog '$($Dialog.Title)' HWND=$($Dialog.Handle); text='$dialogText'; buttons=$details.$exitDetail"
}
function Wait-ForDialogClosed {
    param([System.Diagnostics.Process] $OwnedProcess, [IntPtr] $DialogHandle, [string] $Description, [string[]] $AllowedDialogTitles = @())
    Wait-ForCondition -TimeoutSeconds 5 -Description $Description -OwnedProcess $OwnedProcess -AllowedDialogTitles $AllowedDialogTitles -Condition {
        @((Get-ProcessDialogs $OwnedProcess | Where-Object { $_.Handle -eq $DialogHandle })).Count -eq 0
    } | Out-Null
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
    # implements its own WM_MOUSEHWHEEL scroll model; no external app/data is used.
    $helperSource = @'
param([Parameter(Mandatory=$true)][string] $ReadyFile, [int] $AtRight)
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
        private const int PageWidth = 2600;
        private const int PageHeight = 520;
        private const int HorizontalWheelMessage = 0x020E;
        private readonly Bitmap page;
        private readonly string readyFile;
        private int offset;
        [DllImport("user32.dll")] private static extern bool SetProcessDpiAwarenessContext(IntPtr value);

        public Surface(string path, bool atRight) {
            try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            readyFile = path;
            Text = "ShotKit Synthetic Horizontal Surface";
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
            offset = atRight ? Math.Max(0, PageWidth - ClientSize.Width) : 0;
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
                        bytes[i] = (byte)((x * 19 + y * 11 + (x / 11) * 41) & 255);      // B
                        bytes[i + 1] = (byte)((x * 7 + y * 31 + (x / 23) * 97) & 255);  // G
                        bytes[i + 2] = (byte)((x * 29 + y * 17 + (x / 19) * 53) & 255); // R
                        bytes[i + 3] = 255;
                    }
                }
                Marshal.Copy(bytes, 0, data.Scan0, bytes.Length);
            } finally { page.UnlockBits(data); }
        }

        protected override void OnShown(EventArgs e) {
            base.OnShown(e);
            Point origin = PointToScreen(Point.Empty);
            File.WriteAllText(readyFile, String.Format("{0},{1},{2},{3},{4}", origin.X, origin.Y, ClientSize.Width, ClientSize.Height, offset));
        }

        protected override void WndProc(ref Message m) {
            if (m.Msg == HorizontalWheelMessage) {
                int delta = unchecked((short)((m.WParam.ToInt64() >> 16) & 0xffff));
                int max = Math.Max(0, PageWidth - ClientSize.Width);
                offset = Math.Max(0, Math.Min(max, offset + delta));
                Invalidate();
                m.Result = IntPtr.Zero;
                return;
            }
            base.WndProc(ref m);
        }

        protected override void OnPaint(PaintEventArgs e) {
            e.Graphics.DrawImage(page, new Rectangle(0, 0, ClientSize.Width, PageHeight),
                new Rectangle(offset, 0, ClientSize.Width, PageHeight), GraphicsUnit.Pixel);
            using (Brush track = new SolidBrush(Color.FromArgb(220, 220, 220)))
            using (Brush thumb = new SolidBrush(Color.FromArgb(80, 80, 80))) {
                int trackY = ClientSize.Height - 18;
                e.Graphics.FillRectangle(track, 0, trackY, ClientSize.Width, 12);
                int thumbWidth = Math.Max(40, ClientSize.Width * ClientSize.Width / PageWidth);
                int max = Math.Max(1, PageWidth - ClientSize.Width);
                int thumbLeft = (ClientSize.Width - thumbWidth) * offset / max;
                e.Graphics.FillRectangle(thumb, thumbLeft, trackY + 1, thumbWidth, 10);
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
$surfaceForm = [ShotKitScrollSurface.Surface]::new($ReadyFile, ($AtRight -ne 0))
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
    $settingsPath = Join-Path (Join-Path $appData 'ShotKitRust') 'settings.json'
    New-Item -ItemType Directory -Path (Split-Path -Parent $settingsPath) -Force | Out-Null
    $settings = @{
        version = 1
        save_folder = $saveFolder
        open_editor = $true
        copy_to_clipboard = $false
        auto_save = $true
        scroll_direction = 'right'
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
    Assert-NoStartupDialogs -TimeoutSeconds $StartupTimeoutSeconds -OwnedProcess $shotKit -GetWindows { Get-ProcessWindows $shotKit }
    Assert-NoUnexpectedDialog $shotKit

    $cases = @(
        @{ Name = 'right'; AtRight = 0; DirectionChoice = 6 },
        @{ Name = 'left';  AtRight = 1; DirectionChoice = 7 }
    )
    $passMessages = @()
    foreach ($case in $cases) {
        $readyFile = Join-Path $testRoot ("surface.$($case.Name).ready")
        $surfaceStdout = Join-Path $testRoot ("surface.$($case.Name).stdout.log")
        $surfaceStderr = Join-Path $testRoot ("surface.$($case.Name).stderr.log")
        $helperArgs = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -ReadyFile "{1}" -AtRight {2}' -f $helperFile, $readyFile, $case.AtRight
        $surface = Start-Process -FilePath $powershellExe -ArgumentList $helperArgs -WorkingDirectory $testRoot -WindowStyle Hidden -RedirectStandardOutput $surfaceStdout -RedirectStandardError $surfaceStderr -PassThru

        $surfaceWindow = Wait-ForCondition -TimeoutSeconds 20 -Description "the synthetic horizontal surface ($($case.Name))" -OwnedProcess $surface -Condition {
            Get-ProcessWindows $surface | Where-Object { $_.Title -eq 'ShotKit Synthetic Horizontal Surface' -and $_.Visible } | Select-Object -First 1
        }
        $ready = Wait-ForCondition -TimeoutSeconds 10 -Description 'the synthetic surface geometry' -OwnedProcess $surface -Condition {
            if (Test-Path -LiteralPath $readyFile) { Get-Content -LiteralPath $readyFile -Raw }
        }
        $geometry = $ready.Trim().Split(',') | ForEach-Object { [int]$_ }
        $expectedInitialOffset = if ($case.AtRight -eq 0) { 0 } else { 1700 }
        if ($geometry.Count -ne 5 -or $geometry[2] -ne 900 -or $geometry[3] -ne 650 -or $geometry[4] -ne $expectedInitialOffset) {
            throw "Synthetic surface reported unexpected client geometry: '$ready'."
        }
        $desktopWidth = [ShotKitScrollSmoke.Native]::GetSystemMetrics(78)
        $desktopHeight = [ShotKitScrollSmoke.Native]::GetSystemMetrics(79)
        if ($desktopWidth -lt 940 -or $desktopHeight -lt 720 -or
            $geometry[0] + 900 -gt $desktopWidth -or $geometry[1] + 460 -gt $desktopHeight) {
            throw "Interactive desktop ${desktopWidth}x${desktopHeight} cannot fit the controlled horizontal test window and selection."
        }

        $surfaceRoot = $surfaceWindow.Handle
        $surfaceOwner = [ShotKitScrollSmoke.Native]::OwnerProcessId($surfaceRoot)
        if ($surfaceOwner -ne [uint32]$surface.Id) {
            throw "Refusing to activate an HWND not owned by the helper process: HWND=$surfaceRoot owner PID=$surfaceOwner expected PID=$($surface.Id)."
        }
        if ([ShotKitScrollSmoke.Native]::WindowTitle($surfaceRoot) -ne 'ShotKit Synthetic Horizontal Surface') {
            throw "Helper-owned HWND $surfaceRoot has an unexpected title."
        }
        [ShotKitScrollSmoke.Native]::ShowNormal($surfaceRoot)
        $surfaceWindow = Wait-ForCondition -TimeoutSeconds 5 -Description 'the visible helper-owned horizontal surface' -OwnedProcess $surface -Condition {
            if ([ShotKitScrollSmoke.Native]::OwnerProcessId($surfaceRoot) -eq [uint32]$surface.Id -and
                [ShotKitScrollSmoke.Native]::IsWindowVisible($surfaceRoot)) {
                Get-ProcessWindows $surface | Where-Object { $_.Handle -eq $surfaceRoot } | Select-Object -First 1
            }
        }
        $blockingDialogs = @(Get-ProcessDialogs $shotKit)
        if ($blockingDialogs.Count -gt 0) {
            $details = $blockingDialogs | ForEach-Object {
                "Title='$($_.Title)' Text='$([ShotKitScrollSmoke.Native]::WindowText($_.Handle))'"
            }
            throw "ACTIVATION_BLOCKED_BY_SHOTKIT_DIALOG: $($details -join '; ')"
        }
        $activationApiResult = [ShotKitScrollSmoke.Native]::ActivateOwnedWindow($surfaceRoot, [uint32]$surface.Id)
        $foreground = [ShotKitScrollSmoke.Native]::GetForegroundWindow()
        $foregroundPid = [ShotKitScrollSmoke.Native]::OwnerProcessId($foreground)
        if ($foregroundPid -ne [uint32]$surface.Id) {
            $foregroundClass = if ($foreground -eq [IntPtr]::Zero) { '<none>' } else { [ShotKitScrollSmoke.Native]::WindowClass($foreground) }
            $foregroundTitle = if ($foregroundPid -eq [uint32]$shotKit.Id -or $foregroundPid -eq [uint32]$surface.Id) {
                [ShotKitScrollSmoke.Native]::WindowTitle($foreground)
            } else { '<suppressed: outside test processes>' }
            $appDialogs = @(Get-ProcessDialogs $shotKit | ForEach-Object {
                "Title='$($_.Title)' Text='$([ShotKitScrollSmoke.Native]::WindowText($_.Handle))'"
            })
            $dialogInfo = if ($appDialogs.Count -gt 0) { $appDialogs -join '; ' } else { '<none owned by ShotKit>' }
            throw "SURFACE_ACTIVATION_FAILED: own HWND=$surfaceRoot PID=$surfaceOwner visible=$([ShotKitScrollSmoke.Native]::IsWindowVisible($surfaceRoot)); activation API returned $activationApiResult. Foreground HWND=$foreground PID=$foregroundPid class='$foregroundClass' title='$foregroundTitle'; ShotKit dialogs=$dialogInfo. Likely Windows foreground-lock/another foreground process."
        }
        Start-Sleep -Milliseconds 150
        $beforePngs = @(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | ForEach-Object { $_.FullName })
        [ShotKitScrollSmoke.Native]::keybd_event(0x79, 0, 0, [UIntPtr]::Zero)
        [ShotKitScrollSmoke.Native]::keybd_event(0x79, 0, 0x0002, [UIntPtr]::Zero)
        $axisDialog = Wait-ForCondition -TimeoutSeconds 10 -Description 'the scroll-axis prompt' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
            Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Richtung' } | Select-Object -First 1
        }
        # IDNO selects horizontal scrolling.
        [void](Click-DialogButton -OwnedProcess $shotKit -Dialog $axisDialog -ExpectedTexts @('No', 'Nein') -Description 'horizontal axis choice')
        Wait-ForDialogClosed -OwnedProcess $shotKit -DialogHandle $axisDialog.Handle -Description 'the horizontal-axis prompt to close' -AllowedDialogTitles @('Scroll-Richtung')
        $directionDialog = Wait-ForCondition -TimeoutSeconds 10 -Description "the $($case.Name) direction prompt" -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
            Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Richtung' } | Select-Object -First 1
        }
        $directionButton = if ($case.DirectionChoice -eq 6) { @('Yes', 'Ja') } else { @('No', 'Nein') }
        [void](Click-DialogButton -OwnedProcess $shotKit -Dialog $directionDialog -ExpectedTexts $directionButton -Description "$($case.Name) direction choice")
        Wait-ForDialogClosed -OwnedProcess $shotKit -DialogHandle $directionDialog.Handle -Description "the $($case.Name) direction prompt to close" -AllowedDialogTitles @('Scroll-Richtung')

        $overlay = Wait-ForCondition -TimeoutSeconds 10 -Description 'the horizontal scroll-selection overlay' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
            Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible } | Select-Object -First 1
        }
        $persistedSettings = [IO.File]::ReadAllText($settingsPath) | ConvertFrom-Json
        if ($persistedSettings.scroll_direction -cne $case.Name) {
            throw "Scroll direction was not persisted: expected '$($case.Name)', got '$($persistedSettings.scroll_direction)'."
        }
        $startX = $geometry[0]
        $startY = $geometry[1] + 60
        $endX = $startX + 900
        $endY = $startY + 400
        if (-not [ShotKitScrollSmoke.Native]::SetForegroundWindow($overlay.Handle)) { throw 'Could not activate the selection overlay.' }
        if (-not [ShotKitScrollSmoke.Native]::SetCursorPos($startX, $startY)) { throw 'Could not position the selection start.' }
        Start-Sleep -Milliseconds 150
        [ShotKitScrollSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
        if (-not [ShotKitScrollSmoke.Native]::SetCursorPos($endX, $endY)) { throw 'Could not position the selection end.' }
        Start-Sleep -Milliseconds 200
        [ShotKitScrollSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)

        $png = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description "the $($case.Name)-capture PNG" -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
            Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Where-Object { $beforePngs -notcontains $_.FullName } | Select-Object -First 1
        }
        $completion = Wait-ForCondition -TimeoutSeconds 20 -Description "the $($case.Name) scroll-stop notification" -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
            Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Aufnahme beendet' } | Select-Object -First 1
        }
        $completionText = [ShotKitScrollSmoke.Native]::WindowText($completion.Handle)
        if ($completionText -notlike '*Ende des scrollbaren Bereichs erreicht.*') {
            throw "Horizontal scroll ($($case.Name)) did not report reaching the real page edge. Dialog='$completionText'"
        }

        $bitmap = [System.Drawing.Bitmap]::new($png.FullName)
        try {
            $expectedWidth = 2600 # 1700 scrollable pixels plus the full 900-pixel viewport
            $expectedHeight = 400
            if ($bitmap.Width -ne $expectedWidth -or $bitmap.Height -ne $expectedHeight) {
                throw "Horizontal $($case.Name) image dimensions were $($bitmap.Width)x$($bitmap.Height); expected ${expectedWidth}x${expectedHeight}."
            }
            # Check each reconstructed column at several rows. This verifies the
            # full horizontal extent, correct orientation, and every seam.
            foreach ($x in 0..($expectedWidth - 1)) {
                $sourceX = $x
                foreach ($y in @(0, 47, 179, 399)) {
                    $sourceY = $y + 60
                    $expectedR = ($sourceX * 29 + $sourceY * 17 + [int][Math]::Floor($sourceX / 19) * 53) % 256
                    $expectedG = ($sourceX * 7 + $sourceY * 31 + [int][Math]::Floor($sourceX / 23) * 97) % 256
                    $expectedB = ($sourceX * 19 + $sourceY * 11 + [int][Math]::Floor($sourceX / 11) * 41) % 256
                    $actual = $bitmap.GetPixel($x, $y)
                    if ($actual.R -ne $expectedR -or $actual.G -ne $expectedG -or $actual.B -ne $expectedB) {
                        throw "Horizontal $($case.Name) pixel mismatch at output ($x,$y), source ($sourceX,$sourceY): actual RGB=$($actual.R),$($actual.G),$($actual.B), expected=$expectedR,$expectedG,$expectedB."
                    }
                }
            }
        }
        finally { $bitmap.Dispose() }

        $editor = Wait-ForCondition -TimeoutSeconds 15 -Description 'the visible editor window' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
            Get-ProcessWindows $shotKit | Where-Object {
                $_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible -and $_.Title -like '*speichern*'
            } | Select-Object -First 1
        }
        Assert-NoUnexpectedDialog $shotKit @('Scroll-Aufnahme beendet')
        $passMessages += "$($case.Name): reached the horizontal page edge and reconstructed $expectedWidth columns in $($png.Name)."
        [void](Click-DialogButton -OwnedProcess $shotKit -Dialog $completion -ExpectedTexts @('OK') -Description "$($case.Name) completion acknowledgement")
        Wait-ForDialogClosed -OwnedProcess $shotKit -DialogHandle $completion.Handle -Description "the $($case.Name) completion notice to close" -AllowedDialogTitles @('Scroll-Aufnahme beendet')

        if (-not $surface.HasExited) {
            $surface.Kill()
            if (-not $surface.WaitForExit(5000) -or -not $surface.HasExited) {
                throw "Synthetic $($case.Name) surface did not stop after termination was requested."
            }
        }
        $surface.Dispose()
        $surface = $null
    }
    $passMessage = 'PASS: ' + ($passMessages -join ' | ')
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

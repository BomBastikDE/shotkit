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

if (-not ('ShotKitScrollDiagnosticsSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace ShotKitScrollDiagnosticsSmoke {
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

$inputDesktop = [ShotKitScrollDiagnosticsSmoke.Native]::InputDesktopName()
if ([string]::IsNullOrWhiteSpace($inputDesktop)) {
    throw 'Desktop session unavailable: Windows could not open the interactive input desktop.'
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Scroll-Diagnostics-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$saveFolder = Join-Path $testRoot 'captures'
$readyFile = Join-Path $testRoot 'surface.ready'
$wheelFile = Join-Path $testRoot 'surface.wheel'
$helperFile = Join-Path $testRoot 'surface.ps1'
$surfaceStdout = Join-Path $testRoot 'surface.stdout.log'
$surfaceStderr = Join-Path $testRoot 'surface.stderr.log'
$shotKit = $null
$surface = $null
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$passMessage = $null
$cleanupFailures = @()

function Assert-Foreground {
    param([IntPtr] $Expected, [string] $Action)
    $actual = [ShotKitScrollDiagnosticsSmoke.Native]::GetForegroundWindow()
    if ($actual -ne $Expected) {
        throw "Foreground changed before ${Action}: expected $Expected, got $actual. No input was sent."
    }
}
function Assert-NoExistingShotKit {
    # Avoid the application's single-instance forwarding path and never send a
    # hotkey to a pre-existing user's process. Compare both common names and
    # resolved executable paths, including renamed copies of the test binary.
    foreach ($process in @(Get-Process)) {
        try {
            $matchingName = $process.ProcessName -match '(?i)^shotkit'
            $matchingPath = $false
            try {
                $path = $process.Path
                if (-not [string]::IsNullOrEmpty($path)) {
                    $matchingPath = [string]::Equals($path, $ExePath, [StringComparison]::OrdinalIgnoreCase)
                }
            } catch {
                if ($matchingName) { throw }
            }
            if ($matchingName -or $matchingPath) {
                throw "Existing ShotKit process detected (PID $($process.Id), name '$($process.ProcessName)'). Close it before running this test."
            }
        } finally { $process.Dispose() }
    }
}
function Get-ProcessWindows {
    param([System.Diagnostics.Process] $OwnedProcess)
    [ShotKitScrollDiagnosticsSmoke.Native]::WindowsForProcess($OwnedProcess.Id)
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
        Where-Object { $AllowedTitles -notcontains $_.Title -and $_.Title -notlike '*Prototyp*' } |
        Select-Object -First 1
    if ($null -ne $dialog) {
        $text = [ShotKitScrollDiagnosticsSmoke.Native]::WindowText($dialog.Handle)
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

    # The synthetic surface changes from one flat color to another on its first
    # WM_MOUSEWHEEL; no external app or real content is used.
    $helperSource = @'
param([Parameter(Mandatory=$true)][string] $ReadyFile, [Parameter(Mandatory=$true)][string] $WheelFile)
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

namespace ShotKitScrollDiagnosticsSurface {
    public sealed class Surface : Form {
        private const int WheelMessage = 0x020A;
        private readonly string readyFile;
        private readonly string wheelFile;
        private int wheels;
        [DllImport("user32.dll")] private static extern bool SetProcessDpiAwarenessContext(IntPtr value);

        public Surface(string path, string wheelPath) {
            try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { }
            readyFile = path;
            wheelFile = wheelPath;
            Text = "ShotKit Synthetic Diagnostics Surface";
            StartPosition = FormStartPosition.Manual;
            Location = new Point(20, 20);
            ClientSize = new Size(900, 650);
            FormBorderStyle = FormBorderStyle.FixedSingle;
            MaximizeBox = false;
            MinimizeBox = false;
            ShowInTaskbar = false;
            DoubleBuffered = true;
            SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer, true);
        }

        protected override void OnShown(EventArgs e) {
            base.OnShown(e);
            Point origin = PointToScreen(Point.Empty);
            File.WriteAllText(readyFile, String.Format("{0},{1},{2},{3}", origin.X, origin.Y, ClientSize.Width, ClientSize.Height));
        }

        protected override void WndProc(ref Message m) {
            if (m.Msg == WheelMessage) {
                wheels++;
                Invalidate();
                Update();
                File.WriteAllText(wheelFile, wheels.ToString());
                m.Result = IntPtr.Zero;
                return;
            }
            base.WndProc(ref m);
        }

        protected override void OnPaint(PaintEventArgs e) {
            // Constant but distinct colors guarantee active matcher columns and
            // no structured rows. No displacement can gain a match vote.
            e.Graphics.Clear(wheels == 0 ? Color.FromArgb(32, 48, 64) : Color.FromArgb(208, 224, 240));
            base.OnPaint(e);
        }

        protected override void Dispose(bool disposing) {
            base.Dispose(disposing);
        }
    }
}
"@
$surfaceForm = [ShotKitScrollDiagnosticsSurface.Surface]::new($ReadyFile, $WheelFile)
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
    $helperArgs = '-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -ReadyFile "{1}" -WheelFile "{2}"' -f $helperFile, $readyFile, $wheelFile
    $surface = Start-Process -FilePath $powershellExe -ArgumentList $helperArgs -WorkingDirectory $testRoot -WindowStyle Hidden -RedirectStandardOutput $surfaceStdout -RedirectStandardError $surfaceStderr -PassThru

    $surfaceWindow = Wait-ForCondition -TimeoutSeconds 20 -Description 'the synthetic scroll surface' -OwnedProcess $surface -Condition {
        Get-ProcessWindows $surface | Where-Object { $_.Title -eq 'ShotKit Synthetic Diagnostics Surface' -and $_.Visible } | Select-Object -First 1
    }
    $ready = Wait-ForCondition -TimeoutSeconds 10 -Description 'the synthetic surface geometry' -OwnedProcess $surface -Condition {
        if (Test-Path -LiteralPath $readyFile) { Get-Content -LiteralPath $readyFile -Raw }
    }
    $geometry = $ready.Trim().Split(',') | ForEach-Object { [int]$_ }
    if ($geometry.Count -ne 4 -or $geometry[2] -ne 900 -or $geometry[3] -ne 650) {
        throw "Synthetic surface reported unexpected client geometry: '$ready'."
    }
    $desktopLeft = [ShotKitScrollDiagnosticsSmoke.Native]::GetSystemMetrics(76)
    $desktopTop = [ShotKitScrollDiagnosticsSmoke.Native]::GetSystemMetrics(77)
    $desktopWidth = [ShotKitScrollDiagnosticsSmoke.Native]::GetSystemMetrics(78)
    $desktopHeight = [ShotKitScrollDiagnosticsSmoke.Native]::GetSystemMetrics(79)
    $desktopRight = $desktopLeft + $desktopWidth
    $desktopBottom = $desktopTop + $desktopHeight
    if ($desktopWidth -lt 940 -or $desktopHeight -lt 720 -or
        $geometry[0] -lt $desktopLeft -or $geometry[1] -lt $desktopTop -or
        $geometry[0] + 900 -gt $desktopRight -or $geometry[1] + 650 -gt $desktopBottom) {
        throw "Virtual desktop ($desktopLeft,$desktopTop) ${desktopWidth}x${desktopHeight} cannot fit the controlled test surface."
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

    Assert-NoExistingShotKit
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
    if (-not [ShotKitScrollDiagnosticsSmoke.Native]::PostMessageW($welcome.Handle, 0x0111, [IntPtr]1, [IntPtr]::Zero)) {
        throw 'Could not close the ShotKit startup dialog.'
    }
    Start-Sleep -Milliseconds 250
    Assert-NoUnexpectedDialog $shotKit

    # Place the synthetic form in front and invoke ShotKit's registered F10
    # scroll-capture hotkey. This never targets windows outside this test.
    if (-not [ShotKitScrollDiagnosticsSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)) {
        throw 'Could not activate the synthetic test surface.'
    }
    Assert-Foreground $surfaceWindow.Handle 'F10 key down'
    [ShotKitScrollDiagnosticsSmoke.Native]::keybd_event(0x79, 0, 0, [UIntPtr]::Zero)
    # Always release the pressed key, even if F10 itself transfers focus.
    [ShotKitScrollDiagnosticsSmoke.Native]::keybd_event(0x79, 0, 0x0002, [UIntPtr]::Zero)
    $directionDialog = Wait-ForCondition -TimeoutSeconds 10 -Description 'the scroll direction prompt' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Richtung' } | Select-Object -First 1
    }
    # MessageBox IDYES selects the vertical downward direction.
    if (-not [ShotKitScrollDiagnosticsSmoke.Native]::PostMessageW($directionDialog.Handle, 0x0111, [IntPtr]6, [IntPtr]::Zero)) {
        throw 'Could not select downward scrolling.'
    }

    $overlay = Wait-ForCondition -TimeoutSeconds 10 -Description 'the scroll-selection overlay' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Richtung') -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible } | Select-Object -First 1
    }
    $originX = $geometry[0]
    $originY = $geometry[1]
    $startX = $originX + 80
    $startY = $originY + 60
    $endX = $startX + 600
    $endY = $startY + 400
    if (-not [ShotKitScrollDiagnosticsSmoke.Native]::SetForegroundWindow($overlay.Handle)) { throw 'Could not activate the selection overlay.' }
    Assert-Foreground $overlay.Handle 'positioning the selection start'
    if (-not [ShotKitScrollDiagnosticsSmoke.Native]::SetCursorPos($startX, $startY)) { throw 'Could not position the selection start.' }
    Start-Sleep -Milliseconds 150
    Assert-Foreground $overlay.Handle 'selection mouse down'
    [ShotKitScrollDiagnosticsSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    $mouseDown = $true
    try {
        Assert-Foreground $overlay.Handle 'positioning the selection end'
        if (-not [ShotKitScrollDiagnosticsSmoke.Native]::SetCursorPos($endX, $endY)) { throw 'Could not position the selection end.' }
        Start-Sleep -Milliseconds 200
        Assert-Foreground $overlay.Handle 'selection mouse up'
        [ShotKitScrollDiagnosticsSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
        $mouseDown = $false
    } finally {
        # Do not leave the physical button logically pressed if focus changed.
        if ($mouseDown) { [ShotKitScrollDiagnosticsSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) }
    }

    $completion = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'the ambiguous-match completion dialog' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ProcessDialogs $shotKit | Where-Object { $_.Title -eq 'Scroll-Aufnahme beendet' } | Select-Object -First 1
    }
    $completionText = [ShotKitScrollDiagnosticsSmoke.Native]::WindowText($completion.Handle)
    if ($completionText -notmatch 'Bildinhalt' -or $completionText -notmatch 'Diagnose gespeichert:') {
        throw "Completion did not report an ambiguous match and a saved diagnosis: '$completionText'"
    }
    if (-not (Test-Path -LiteralPath $wheelFile -PathType Leaf) -or ((Read-SharedText -Path $wheelFile).Trim() -cne '1')) {
        throw 'Expected exactly one wheel event on the synthetic surface.'
    }
    $pngs = @(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File)
    if ($pngs.Count -ne 1) { throw "Expected one saved partial PNG, found $($pngs.Count)." }
    $diagnosticBase = Join-Path $saveFolder 'Diagnose'
    if (-not (Test-Path -LiteralPath $diagnosticBase -PathType Container)) { throw 'Diagnosis directory missing.' }
    $bundles = @(Get-ChildItem -LiteralPath $diagnosticBase -Directory)
    if ($bundles.Count -ne 1 -or $bundles[0].Name -notmatch '^Scroll_[0-9]+_[0-9]+$') {
        throw "Expected exactly one scroll diagnostic bundle, found $($bundles.Count)."
    }
    $bundle = $bundles[0].FullName
    if (-not $completionText.Contains($bundle)) { throw "Completion dialog did not identify the actual diagnostic bundle: '$completionText'" }
    $entries = @(Get-ChildItem -LiteralPath $bundle -Force)
    $names = @($entries | ForEach-Object { $_.Name } | Sort-Object)
    if ($entries.Count -ne 3 -or (@($entries | Where-Object { $_.PSIsContainer }).Count -ne 0) -or
        (($names -join ',') -cne 'info.txt,nachher.png,vorher.png')) {
        throw "Diagnostic bundle has unexpected entries: $($names -join ', ')"
    }

    Add-Type -AssemblyName System.Drawing
    # Decode the saved partial and both diagnostics independently. Pixel probes
    # cover the interior and edges, so accidental desktop capture fails loudly.
    foreach ($check in @(
        @{ Path = $pngs[0].FullName; R = 32; G = 48; B = 64; Label = 'partial' },
        @{ Path = (Join-Path $bundle 'vorher.png'); R = 32; G = 48; B = 64; Label = 'before' },
        @{ Path = (Join-Path $bundle 'nachher.png'); R = 208; G = 224; B = 240; Label = 'after' }
    )) {
        $bitmap = [System.Drawing.Bitmap]::new($check.Path)
        try {
            if ($bitmap.RawFormat.Guid -ne [System.Drawing.Imaging.ImageFormat]::Png.Guid -or
                $bitmap.Width -ne 600 -or $bitmap.Height -ne 400) {
                throw "$($check.Label) image is not a 600x400 PNG (got $($bitmap.Width)x$($bitmap.Height))."
            }
            foreach ($y in @(0, 1, 60, 200, 338, 398, 399)) {
                foreach ($x in @(0, 1, 80, 300, 518, 598, 599)) {
                    $pixel = $bitmap.GetPixel($x, $y)
                    if ($pixel.R -ne $check.R -or $pixel.G -ne $check.G -or $pixel.B -ne $check.B) {
                        throw "$($check.Label) unexpected RGB at ($x,$y): $($pixel.R),$($pixel.G),$($pixel.B)."
                    }
                }
            }
        } finally { $bitmap.Dispose() }
    }
    $info = [IO.File]::ReadAllText((Join-Path $bundle 'info.txt'), [Text.Encoding]::UTF8)
    $requiredInfo = @(
        "x=$startX, y=$startY, Breite=600",
        'Bildschirm-DPI X:',
        'Richtung: Down',
        'Erwartete Verschiebung: 133 px',
        'Scrollrasten: 1',
        'Gemessene Pixel pro Raste: 0',
        'Letzte feste',
        'Aufeinanderfolgende Ersatzabgleiche: 0',
        'Abgleich: kein eindeutiges Ergebnis nach erneutem Versuch',
        'Bilder: Matcher-Ausrichtung'
    )
    foreach ($expected in $requiredInfo) {
        if (-not $info.Contains($expected)) { throw "Diagnostic info missing '$expected': $info" }
    }
    $editor = Wait-ForCondition -TimeoutSeconds 15 -Description 'the visible partial-image editor' -OwnedProcess $shotKit -AllowedDialogTitles @('Scroll-Aufnahme beendet') -Condition {
        Get-ProcessWindows $shotKit | Where-Object { $_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible -and $_.Title -like '*speichern*' } | Select-Object -First 1
    }
    Assert-NoUnexpectedDialog $shotKit @('Scroll-Aufnahme beendet')
    $passMessage = 'PASS: first wheel event caused a final ambiguous match; the partial image and both diagnostic PNGs and info were verified.'

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

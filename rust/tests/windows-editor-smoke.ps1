[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 30,

    [ValidateRange(5, 60)]
    [int] $ActionTimeoutSeconds = 20
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') {
    throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'
}
if (-not [Environment]::UserInteractive) {
    throw 'NO_INTERACTIVE_DESKTOP: the Windows runner is not interactive.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "ShotKit EXE not found: $ExePath"
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: a process named '$imageName' is already running; the test will not use or change it."
}

if (-not ('ShotKitEditorSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace ShotKitEditorSmoke {
    public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }
    public struct Point { public int X; public int Y; }
    public sealed class WindowInfo {
        public IntPtr Handle;
        public string ClassName;
        public string Title;
        public bool Visible;
    }
    public sealed class ButtonInfo {
        public IntPtr Handle;
        public int Id;
        public string Text;
    }
    public static class Native {
        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr data);
        private delegate bool EnumChildProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr data);
        [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] private static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern bool IsWindowEnabled(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] private static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
        [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] private static extern bool ScreenToClient(IntPtr hwnd, ref Point point);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
        [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
        [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);

        private static string Text(IntPtr hwnd) {
            StringBuilder text = new StringBuilder(512);
            GetWindowTextW(hwnd, text, text.Capacity);
            return text.ToString();
        }
        private static string Class(IntPtr hwnd) {
            StringBuilder text = new StringBuilder(256);
            GetClassNameW(hwnd, text, text.Capacity);
            return text.ToString();
        }
        public static WindowInfo[] WindowsForProcess(int processId) {
            List<WindowInfo> result = new List<WindowInfo>();
            EnumWindowsProc callback = delegate(IntPtr hwnd, IntPtr data) {
                uint pid;
                GetWindowThreadProcessId(hwnd, out pid);
                if (pid == (uint)processId) result.Add(new WindowInfo {
                    Handle = hwnd, ClassName = Class(hwnd), Title = Text(hwnd), Visible = IsWindowVisible(hwnd)
                });
                return true;
            };
            EnumWindows(callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            return result.ToArray();
        }
        public static WindowInfo[] ChildWindows(IntPtr parent) {
            List<WindowInfo> result = new List<WindowInfo>();
            EnumChildProc callback = delegate(IntPtr hwnd, IntPtr data) {
                result.Add(new WindowInfo {
                    Handle = hwnd, ClassName = Class(hwnd), Title = Text(hwnd), Visible = IsWindowVisible(hwnd)
                });
                return true;
            };
            EnumChildWindows(parent, callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            return result.ToArray();
        }
        public static bool GetChildRectInClient(IntPtr parent, IntPtr child, out Rect rect) {
            rect = new Rect();
            if (!GetWindowRect(child, out rect)) return false;
            Point topLeft = new Point { X = rect.Left, Y = rect.Top };
            Point bottomRight = new Point { X = rect.Right, Y = rect.Bottom };
            if (!ScreenToClient(parent, ref topLeft) || !ScreenToClient(parent, ref bottomRight)) return false;
            rect = new Rect { Left = topLeft.X, Top = topLeft.Y, Right = bottomRight.X, Bottom = bottomRight.Y };
            return true;
        }
        public static ButtonInfo[] VisibleButtons(IntPtr parent) {
            List<ButtonInfo> result = new List<ButtonInfo>();
            EnumChildProc callback = delegate(IntPtr child, IntPtr data) {
                if (IsWindowVisible(child) && IsWindowEnabled(child) && String.Equals(Class(child), "Button", StringComparison.OrdinalIgnoreCase))
                    result.Add(new ButtonInfo { Handle=child, Id=GetDlgCtrlID(child), Text=Text(child) });
                return true;
            };
            EnumChildWindows(parent, callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            return result.ToArray();
        }
        public static uint OwnerPid(IntPtr hwnd) {
            uint pid;
            GetWindowThreadProcessId(hwnd, out pid);
            return pid;
        }
        public static string WindowClass(IntPtr hwnd) { return Class(hwnd); }
        public static string WindowTitle(IntPtr hwnd) { return Text(hwnd); }
        public static string WindowText(IntPtr hwnd) {
            StringBuilder all = new StringBuilder(Text(hwnd));
            EnumChildProc callback = delegate(IntPtr child, IntPtr data) {
                string text = Text(child);
                if (!String.IsNullOrWhiteSpace(text)) all.Append(" ").Append(text);
                return true;
            };
            EnumChildWindows(hwnd, callback, IntPtr.Zero);
            GC.KeepAlive(callback);
            return all.ToString();
        }
        public static void Click(IntPtr button) {
            if (button == IntPtr.Zero) throw new ArgumentException("Button HWND is null.");
            SendMessageW(button, 0x00F5, IntPtr.Zero, IntPtr.Zero); // BM_CLICK
        }
        public static bool PostButtonClick(IntPtr button) {
            if (button == IntPtr.Zero) return false;
            return PostMessageW(button, 0x00F5, IntPtr.Zero, IntPtr.Zero); // BM_CLICK
        }
        public static bool IsLiveWindow(IntPtr hwnd) { return IsWindow(hwnd); }
    }
}
'@
}
try { Add-Type -AssemblyName System.Drawing }
catch { throw "IMAGE_RUNTIME_UNAVAILABLE: $($_.Exception.Message)" }

$desktopWidth = [ShotKitEditorSmoke.Native]::GetSystemMetrics(78)
$desktopHeight = [ShotKitEditorSmoke.Native]::GetSystemMetrics(79)
if ($desktopWidth -lt 1024 -or $desktopHeight -lt 768) {
    throw "INTERACTIVE_DESKTOP_TOO_SMALL: ${desktopWidth}x${desktopHeight}; editor test requires at least 1024x768."
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Editor-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$inputPath = Join-Path $testRoot 'transparent-input.png'
$process = $null
$processStartTicks = $null
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$mouseDown = $false
$shiftDown = $false
$editorStopped = $true
$testFailure = $null
$cleanupFailures = @()

function Get-OwnedWindows {
    if ($null -eq $process -or $process.HasExited) { return @() }
    @([ShotKitEditorSmoke.Native]::WindowsForProcess($process.Id))
}
function Get-OwnedDialogs {
    Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' }
}
function Assert-NoUnexpectedDialog {
    param([string[]] $AllowedTitles = @())
    $dialog = Get-OwnedDialogs | Where-Object {
        $title = $_.Title
        -not ($AllowedTitles | Where-Object { $title -like $_ } | Select-Object -First 1)
    } | Select-Object -First 1
    if ($null -ne $dialog) {
        throw "UNEXPECTED_DIALOG: '$($dialog.Title)' HWND=$($dialog.Handle), text='$([ShotKitEditorSmoke.Native]::WindowText($dialog.Handle))'."
    }
}
function Wait-ForValue {
    param([scriptblock] $Probe, [int] $Seconds, [string] $Description, [string[]] $AllowedTitles = @())
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($null -ne $process -and $process.HasExited) {
            throw "PROCESS_EXITED: ShotKit exited before $Description (code $($process.ExitCode))."
        }
        Assert-NoUnexpectedDialog $AllowedTitles
        $value = & $Probe
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    throw "TIMEOUT: waiting for $Description."
}
function Click-VisibleButton {
    param([IntPtr] $Parent, [string[]] $ExpectedTexts, [string] $Description)
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    $buttons = @()
    while ([DateTime]::UtcNow -lt $deadline) {
        $buttons = @([ShotKitEditorSmoke.Native]::VisibleButtons($Parent))
        $button = $buttons | Where-Object { $ExpectedTexts -contains (($_.Text -replace '&', '').Trim()) } | Select-Object -First 1
        if ($null -ne $button) {
            [ShotKitEditorSmoke.Native]::Click($button.Handle)
            return $button
        }
        Start-Sleep -Milliseconds 100
    }
    $details = ($buttons | ForEach-Object { "ID=$($_.Id) Text='$($_.Text)' HWND=$($_.Handle)" }) -join '; '
    throw "BUTTON_TIMEOUT: $Description; buttons=$details"
}
function Dismiss-OwnedDialog {
    param($Dialog, [string[]] $ExpectedTexts, [string] $ExpectedTitlePattern, [string] $Description)
    $handle = [IntPtr]$Dialog.Handle
    $dialogPid = [ShotKitEditorSmoke.Native]::OwnerPid($handle)
    $dialogClass = [ShotKitEditorSmoke.Native]::WindowClass($handle)
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    $lastButtons = @()
    $clickAttempts = 0
    if ($dialogPid -ne [uint32]$process.Id -or $dialogClass -ne '#32770' -or
        $Dialog.Title -notlike $ExpectedTitlePattern) {
        throw "REFUSING_DIALOG_CLICK: $Description HWND=$handle PID=$dialogPid class='$dialogClass' title='$($Dialog.Title)' does not match the owned expected dialog."
    }
    while ([DateTime]::UtcNow -lt $deadline) {
        if (-not [ShotKitEditorSmoke.Native]::IsLiveWindow($handle)) { return }
        if ([ShotKitEditorSmoke.Native]::OwnerPid($handle) -ne [uint32]$process.Id -or
            [ShotKitEditorSmoke.Native]::WindowClass($handle) -ne '#32770' -or
            [ShotKitEditorSmoke.Native]::WindowTitle($handle) -notlike $ExpectedTitlePattern) {
            throw "DIALOG_IDENTITY_CHANGED: refusing another click for $Description HWND=$handle."
        }
        $lastButtons = @([ShotKitEditorSmoke.Native]::VisibleButtons($handle))
        $button = $lastButtons | Where-Object { $ExpectedTexts -contains (($_.Text -replace '&', '').Trim()) } | Select-Object -First 1
        if ($null -ne $button) {
            if (-not [ShotKitEditorSmoke.Native]::PostButtonClick($button.Handle)) {
                throw "BM_CLICK_POST_FAILED: $Description HWND=$handle button HWND=$($button.Handle) ID=$($button.Id) Text='$($button.Text)'."
            }
            $clickAttempts++
        }
        Start-Sleep -Milliseconds 200
    }
    $currentDialogs = @(Get-OwnedDialogs | ForEach-Object {
        $buttons = @([ShotKitEditorSmoke.Native]::VisibleButtons($_.Handle))
        $buttonText = ($buttons | ForEach-Object { "HWND=$($_.Handle) ID=$($_.Id) Text='$($_.Text)'" }) -join '; '
        if (-not $buttonText) { $buttonText = 'no visible buttons' }
        "HWND=$($_.Handle) PID=$([ShotKitEditorSmoke.Native]::OwnerPid($_.Handle)) class='$($_.ClassName)' title='$($_.Title)' text='$([ShotKitEditorSmoke.Native]::WindowText($_.Handle))' buttons=[$buttonText]"
    }) -join ' | '
    $lastButtonText = ($lastButtons | ForEach-Object { "HWND=$($_.Handle) ID=$($_.Id) Text='$($_.Text)'" }) -join '; '
    if (-not $lastButtonText) { $lastButtonText = 'no visible Button-class children' }
    if (-not $currentDialogs) { $currentDialogs = 'none found in latest top-level enumeration' }
    throw "DIALOG_CLOSE_TIMEOUT: $Description HWND=$handle remains live=$([ShotKitEditorSmoke.Native]::IsLiveWindow($handle)) after $clickAttempts BM_CLICK attempts. Last original-dialog buttons=[$lastButtonText]. Current owned dialogs=$currentDialogs"
}
function Get-MainWindow {
    Get-OwnedWindows | Where-Object { $_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible } | Select-Object -First 1
}
function Get-ImagePixels {
    param([string] $Path)
    $bitmap = [Drawing.Bitmap]::new($Path)
    try {
        $pixels = [Array]::CreateInstance([Drawing.Color], $bitmap.Width * $bitmap.Height)
        for ($y = 0; $y -lt $bitmap.Height; $y++) {
            for ($x = 0; $x -lt $bitmap.Width; $x++) {
                $pixels[$y * $bitmap.Width + $x] = $bitmap.GetPixel($x, $y)
            }
        }
        return [pscustomobject]@{ Width = $bitmap.Width; Height = $bitmap.Height; Pixels = $pixels }
    }
    finally { $bitmap.Dispose() }
}
function Wait-ForImageState {
    param([string] $Path, [scriptblock] $Predicate, [string] $Description)
    $deadline = [DateTime]::UtcNow.AddSeconds($ActionTimeoutSeconds)
    $lastError = ''
    while ([DateTime]::UtcNow -lt $deadline) {
        Assert-NoUnexpectedDialog
        if ($process.HasExited) { throw "PROCESS_EXITED while waiting for $Description (code $($process.ExitCode))." }
        try {
            $image = Get-ImagePixels $Path
            if (& $Predicate $image) { return $image }
        }
        catch { $lastError = $_.Exception.Message }
        Start-Sleep -Milliseconds 150
    }
    throw "IMAGE_STATE_TIMEOUT: $Description. Last decode/probe error: $lastError"
}
function Assert-ToolbarIconsVisible {
    param([IntPtr] $MainHandle, [string] $Context)
    $origin = [Activator]::CreateInstance([ShotKitEditorSmoke.Point])
    if (-not [ShotKitEditorSmoke.Native]::ClientToScreen($MainHandle, [ref]$origin)) { throw "${Context}: client origin unavailable" }
    $buttons = @([ShotKitEditorSmoke.Native]::VisibleButtons($MainHandle))
    foreach ($name in @('Pfeil (A)', 'Linie (L)', 'Rechteck (R)', 'Ellipse (E)', 'Text (T)')) {
        $button = $buttons | Where-Object { $_.Text -ceq $name } | Select-Object -First 1
        if (-not $button) { throw "${Context}: toolbar button missing: $name" }
        $rect = [Activator]::CreateInstance([ShotKitEditorSmoke.Rect])
        if (-not [ShotKitEditorSmoke.Native]::GetChildRectInClient($MainHandle, $button.Handle, [ref]$rect)) { throw "${Context}: button bounds unavailable: $name" }
        $width = $rect.Right - $rect.Left; $height = $rect.Bottom - $rect.Top
        if ($width -lt 20 -or $height -lt 20) { throw "${Context}: invalid button bounds: $name" }
        $bitmap = [Drawing.Bitmap]::new($width, $height)
        try {
            $graphics = [Drawing.Graphics]::FromImage($bitmap)
            try { $graphics.CopyFromScreen($origin.X + $rect.Left, $origin.Y + $rect.Top, 0, 0, $bitmap.Size) }
            finally { $graphics.Dispose() }
            $background = $bitmap.GetPixel(2, 2)
            $different = 0
            for ($y = 6; $y -lt $height - 6; $y++) {
                for ($x = 6; $x -lt $width - 6; $x++) {
                    $pixel = $bitmap.GetPixel($x, $y)
                    if ([Math]::Abs([int]$pixel.R - [int]$background.R) + [Math]::Abs([int]$pixel.G - [int]$background.G) + [Math]::Abs([int]$pixel.B - [int]$background.B) -gt 90) { $different++ }
                }
            }
            if ($different -lt 5) { throw "${Context}: toolbar icon is missing or overpainted: $name ($different contrasting pixels)" }
        } finally { $bitmap.Dispose() }
    }
}

function Save-FromToolbar {
    param([IntPtr] $MainHandle)
    [void](Click-VisibleButton -Parent $MainHandle -ExpectedTexts @('Speichern') -Description 'editor Save button')
}
function Get-CoordinateForImagePoint {
    param([int] $ImagePoint, [int] $ImageLength, [int] $ViewOrigin, [int] $ViewLength)
    [int][Math]::Round($ViewOrigin + ($ImagePoint + 0.5) * $ViewLength / $ImageLength, [MidpointRounding]::AwayFromZero)
}
function Invoke-CanvasDrag {
    param([int] $FromX, [int] $FromY, [int] $ToX, [int] $ToY, [bool] $WithShift)
    $origin = [Activator]::CreateInstance([ShotKitEditorSmoke.Point])
    if (-not [ShotKitEditorSmoke.Native]::ClientToScreen($main.Handle, [ref]$origin)) { throw 'Could not map editor client origin for Shift drag.' }
    $x1 = Get-CoordinateForImagePoint $FromX 160 $viewX $displayWidth
    $y1 = Get-CoordinateForImagePoint $FromY 120 $viewY $displayHeight
    $x2 = Get-CoordinateForImagePoint $ToX 160 $viewX $displayWidth
    $y2 = Get-CoordinateForImagePoint $ToY 120 $viewY $displayHeight
    if ($WithShift) {
        [ShotKitEditorSmoke.Native]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero)
        $script:shiftDown = $true
        Start-Sleep -Milliseconds 100
    }
    if (-not [ShotKitEditorSmoke.Native]::SetCursorPos($origin.X + $x1, $origin.Y + $y1)) { throw 'Could not position pointer at Shift-drag start.' }
    $actual = [Activator]::CreateInstance([ShotKitEditorSmoke.Point])
    if (-not [ShotKitEditorSmoke.Native]::GetCursorPos([ref]$actual) -or $actual.X -ne ($origin.X + $x1) -or $actual.Y -ne ($origin.Y + $y1)) { throw 'Shift-drag start cursor did not reach its requested position.' }
    if ([ShotKitEditorSmoke.Native]::OwnerPid([ShotKitEditorSmoke.Native]::WindowFromPoint($actual)) -ne [uint32]$process.Id) { throw 'Shift-drag start is not over the owned editor.' }
    [ShotKitEditorSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    $script:mouseDown = $true
    Start-Sleep -Milliseconds 100
    if (-not [ShotKitEditorSmoke.Native]::SetCursorPos($origin.X + $x2, $origin.Y + $y2)) { throw 'Could not position pointer at Shift-drag end.' }
    Start-Sleep -Milliseconds 200
    [ShotKitEditorSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    $script:mouseDown = $false
    if ($WithShift) {
        [ShotKitEditorSmoke.Native]::keybd_event(0x10, 0, 0x0002, [UIntPtr]::Zero)
        $script:shiftDown = $false
        Start-Sleep -Milliseconds 100
    }
}
function Get-RedBounds {
    param($Image)
    $xs = [System.Collections.Generic.List[int]]::new(); $ys = [System.Collections.Generic.List[int]]::new(); $count = 0
    for ($y = 0; $y -lt $Image.Height; $y++) {
        for ($x = 0; $x -lt $Image.Width; $x++) {
            $p = $Image.Pixels[$y * $Image.Width + $x]
            if ($p.A -gt 0 -and $p.R -gt 200 -and $p.G -lt 80 -and $p.B -lt 80) { $xs.Add($x); $ys.Add($y); $count++ }
        }
    }
    if ($count -eq 0) { return $null }
    return [pscustomobject]@{ Count=$count; Left=($xs | Measure-Object -Minimum).Minimum; Right=($xs | Measure-Object -Maximum).Maximum; Top=($ys | Measure-Object -Minimum).Minimum; Bottom=($ys | Measure-Object -Maximum).Maximum }
}
function Test-OriginalImage {
    param($Image)
    if ($Image.Width -ne $expectedBase.Width -or $Image.Height -ne $expectedBase.Height) { return $false }
    for ($i = 0; $i -lt $Image.Pixels.Length; $i++) {
        $a = $Image.Pixels[$i]; $b = $expectedBase.Pixels[$i]
        if ($a.R -ne $b.R -or $a.G -ne $b.G -or $a.B -ne $b.B -or $a.A -ne $b.A) { return $false }
    }
    return $true
}

try {
    New-Item -ItemType Directory -Path $testRoot, $appData, $localAppData -Force | Out-Null
    $fixture = [Drawing.Bitmap]::new(160, 120, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $graphics = [Drawing.Graphics]::FromImage($fixture)
        try { $graphics.Clear([Drawing.Color]::Transparent) }
        finally { $graphics.Dispose() }
        $fixture.SetPixel(12, 105, [Drawing.Color]::FromArgb(91, 20, 120, 220))
        $fixture.SetPixel(146, 104, [Drawing.Color]::FromArgb(255, 10, 200, 60))
        $fixture.Save($inputPath, [Drawing.Imaging.ImageFormat]::Png)
    }
    finally { $fixture.Dispose() }
    $expectedBase = Get-ImagePixels $inputPath
    if ($expectedBase.Width -ne 160 -or $expectedBase.Height -ne 120 -or
        $expectedBase.Pixels[5 * 160 + 5].A -ne 0 -or
        $expectedBase.Pixels[105 * 160 + 12].A -ne 91) {
        throw 'FIXTURE_INVALID: generated PNG did not retain its expected transparent and semitransparent pixels.'
    }

    try {
        $env:APPDATA = $appData
        $env:LOCALAPPDATA = $localAppData
        $quotedInput = '"{0}"' -f $inputPath
        $process = Start-Process -FilePath $ExePath -ArgumentList $quotedInput -WorkingDirectory $testRoot -PassThru
        $editorStopped = $false
        $processStartTicks = $process.StartTime.ToUniversalTime().Ticks
    }
    finally {
        $env:APPDATA = $oldAppData
        $env:LOCALAPPDATA = $oldLocalAppData
    }

    $main = Wait-ForValue -Seconds $StartupTimeoutSeconds -Description 'the owned ShotKit editor window' -Probe { Get-MainWindow }
    [void][ShotKitEditorSmoke.Native]::ShowWindow($main.Handle, 3) # SW_MAXIMIZE
    if (-not [ShotKitEditorSmoke.Native]::SetForegroundWindow($main.Handle)) { throw 'Could not foreground the owned editor window.' }
    $welcome = Wait-ForValue -Seconds $StartupTimeoutSeconds -Description 'the expected ShotKit startup notice' -AllowedTitles @('*Prototyp*') -Probe {
        Get-OwnedDialogs | Where-Object { $_.Title -like '*Prototyp*' } | Select-Object -First 1
    }
    Dismiss-OwnedDialog -Dialog $welcome -ExpectedTexts @('OK') -ExpectedTitlePattern '*Prototyp*' -Description 'startup notice closure'

    $main = Wait-ForValue -Seconds 5 -Description 'the loaded-image editor title' -Probe {
        Get-MainWindow | Where-Object { $_.Title -like '*Strg+S speichern*' } | Select-Object -First 1
    }
    $client = [Activator]::CreateInstance([ShotKitEditorSmoke.Rect])
    if (-not [ShotKitEditorSmoke.Native]::GetClientRect($main.Handle, [ref]$client)) { throw 'Could not read editor client rectangle.' }
    $toolbar = [ShotKitEditorSmoke.Native]::ChildWindows($main.Handle) |
        Where-Object { $_.ClassName -eq 'ShotKitRustEditorToolbar' -and $_.Visible } |
        Select-Object -First 1
    if ($null -eq $toolbar) { throw 'EDITOR_NOT_READY: visible native toolbar was not found.' }
    $toolbarOwner = [ShotKitEditorSmoke.Native]::OwnerPid($toolbar.Handle)
    if ($toolbarOwner -ne [uint32]$process.Id) {
        throw "EDITOR_TOOLBAR_OWNER_MISMATCH: toolbar HWND $($toolbar.Handle) belongs to PID $toolbarOwner, expected $($process.Id)."
    }
    $toolbarRect = [Activator]::CreateInstance([ShotKitEditorSmoke.Rect])
    if (-not [ShotKitEditorSmoke.Native]::GetClientRect($toolbar.Handle, [ref]$toolbarRect)) { throw 'Could not read toolbar height.' }
    $toolbarHeight = $toolbarRect.Bottom - $toolbarRect.Top
    $viewWidth = $client.Right - $client.Left
    $viewHeight = $client.Bottom - $client.Top - $toolbarHeight
    if ($toolbarHeight -le 0 -or $viewWidth -le 0 -or $viewHeight -le 0) { throw 'EDITOR_NOT_READY: invalid toolbar/canvas geometry.' }
    $toolbarBounds = [Activator]::CreateInstance([ShotKitEditorSmoke.Rect])
    if (-not [ShotKitEditorSmoke.Native]::GetChildRectInClient($main.Handle, $toolbar.Handle, [ref]$toolbarBounds) -or
        $toolbarBounds.Left -ne 0 -or $toolbarBounds.Top -ne 0 -or
        $toolbarBounds.Right -ne $viewWidth -or $toolbarBounds.Bottom -ne $toolbarHeight) {
        throw "EDITOR_NOT_READY: toolbar geometry is inconsistent with editor client (toolbar=($($toolbarBounds.Left),$($toolbarBounds.Top),$($toolbarBounds.Right),$($toolbarBounds.Bottom)); client=${viewWidth}x$($client.Bottom - $client.Top))."
    }

    $fit = [Math]::Min($viewWidth / 160.0, $viewHeight / 120.0)
    $displayWidth = [int][Math]::Round(160 * $fit, [MidpointRounding]::AwayFromZero)
    $displayHeight = [int][Math]::Round(120 * $fit, [MidpointRounding]::AwayFromZero)
    $viewX = [int][Math]::Floor(($viewWidth - $displayWidth) / 2.0)
    $viewY = $toolbarHeight + [int][Math]::Floor(($viewHeight - $displayHeight) / 2.0)

    # Select the actual toolbar tool and verify the editor accepted the command.
    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Rechteck (R)') -Description 'rectangle tool')
    [void](Wait-ForValue -Seconds 5 -Description 'rectangle tool activation' -Probe {
        Get-MainWindow | Where-Object { $_.Title -like '*Werkzeug Rectangle*' } | Select-Object -First 1
    })

    # Drag a large rectangle well inside the image; coordinates are derived
    # from the editor's real client and toolbar geometry, not screen guesses.
    $clientOrigin = [Activator]::CreateInstance([ShotKitEditorSmoke.Point])
    if (-not [ShotKitEditorSmoke.Native]::ClientToScreen($main.Handle, [ref]$clientOrigin)) { throw 'Could not map editor client origin to screen.' }
    $startClientX = Get-CoordinateForImagePoint -ImagePoint 25 -ImageLength 160 -ViewOrigin $viewX -ViewLength $displayWidth
    $startClientY = Get-CoordinateForImagePoint -ImagePoint 24 -ImageLength 120 -ViewOrigin $viewY -ViewLength $displayHeight
    $endClientX = Get-CoordinateForImagePoint -ImagePoint 132 -ImageLength 160 -ViewOrigin $viewX -ViewLength $displayWidth
    $endClientY = Get-CoordinateForImagePoint -ImagePoint 94 -ImageLength 120 -ViewOrigin $viewY -ViewLength $displayHeight
    if ($startClientX -lt $viewX -or $startClientX -ge ($viewX + $displayWidth) -or
        $endClientX -lt $viewX -or $endClientX -ge ($viewX + $displayWidth) -or
        $startClientY -lt $viewY -or $startClientY -ge ($viewY + $displayHeight) -or
        $endClientY -lt $viewY -or $endClientY -ge ($viewY + $displayHeight) -or
        $viewX -lt 0 -or $viewY -lt $toolbarHeight -or
        ($viewX + $displayWidth) -gt $viewWidth -or
        ($viewY + $displayHeight) -gt ($toolbarHeight + $viewHeight)) {
        throw "EDITOR_LAYOUT_INVALID: visible canvas ${displayWidth}x${displayHeight} at ($viewX,$viewY) does not contain test drag points ($startClientX,$startClientY)-($endClientX,$endClientY)."
    }
    $startScreenX = $clientOrigin.X + $startClientX
    $startScreenY = $clientOrigin.Y + $startClientY
    $endScreenX = $clientOrigin.X + $endClientX
    $endScreenY = $clientOrigin.Y + $endClientY
    if (-not [ShotKitEditorSmoke.Native]::SetForegroundWindow($main.Handle) -or
        [ShotKitEditorSmoke.Native]::OwnerPid([ShotKitEditorSmoke.Native]::GetForegroundWindow()) -ne [uint32]$process.Id) {
        throw 'EDITOR_FOCUS_FAILED: the test-owned editor is not the foreground process.'
    }
    if (-not [ShotKitEditorSmoke.Native]::SetCursorPos($startScreenX, $startScreenY)) { throw 'Could not position the pointer at rectangle start.' }
    $actualCursor = [Activator]::CreateInstance([ShotKitEditorSmoke.Point])
    if (-not [ShotKitEditorSmoke.Native]::GetCursorPos([ref]$actualCursor) -or
        $actualCursor.X -ne $startScreenX -or $actualCursor.Y -ne $startScreenY) {
        throw "CURSOR_POSITION_MISMATCH: requested ($startScreenX,$startScreenY), actual ($($actualCursor.X),$($actualCursor.Y))."
    }
    $underPointer = [ShotKitEditorSmoke.Native]::WindowFromPoint($actualCursor)
    if ([ShotKitEditorSmoke.Native]::OwnerPid($underPointer) -ne [uint32]$process.Id) {
        throw "INPUT_TARGET_MISMATCH: cursor is over HWND $underPointer not owned by the test editor."
    }
    Start-Sleep -Milliseconds 150
    [ShotKitEditorSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    $mouseDown = $true
    Start-Sleep -Milliseconds 120
    if (-not [ShotKitEditorSmoke.Native]::SetCursorPos($endScreenX, $endScreenY)) { throw 'Could not position the pointer at rectangle end.' }
    $actualCursor = [Activator]::CreateInstance([ShotKitEditorSmoke.Point])
    if (-not [ShotKitEditorSmoke.Native]::GetCursorPos([ref]$actualCursor) -or
        $actualCursor.X -ne $endScreenX -or $actualCursor.Y -ne $endScreenY) {
        throw "CURSOR_POSITION_MISMATCH: requested rectangle end ($endScreenX,$endScreenY), actual ($($actualCursor.X),$($actualCursor.Y))."
    }
    $underPointer = [ShotKitEditorSmoke.Native]::WindowFromPoint($actualCursor)
    if ([ShotKitEditorSmoke.Native]::OwnerPid($underPointer) -ne [uint32]$process.Id) {
        throw "INPUT_TARGET_MISMATCH: rectangle end is over HWND $underPointer not owned by the test editor."
    }
    Start-Sleep -Milliseconds 250
    [ShotKitEditorSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
    $mouseDown = $false

    Save-FromToolbar $main.Handle
    $drawn = Wait-ForImageState -Path $inputPath -Description 'exported red rectangle and preserved transparency' -Predicate {
        param($image)
        if ($image.Width -ne 160 -or $image.Height -ne 120) { return $false }
        $foundRed = 0
        foreach ($pixel in $image.Pixels) {
            if ($pixel.A -gt 0 -and $pixel.R -gt 200 -and $pixel.G -lt 80 -and $pixel.B -lt 80) { $foundRed++ }
        }
        $transparent = $image.Pixels[5 * 160 + 5]
        $semi = $image.Pixels[105 * 160 + 12]
        $opaque = $image.Pixels[104 * 160 + 146]
        return ($foundRed -ge 30 -and $image.Pixels[60 * 160 + 80].A -eq 0 -and
            $transparent.A -eq 0 -and $semi.A -eq 91 -and $semi.R -eq 20 -and $semi.G -eq 120 -and $semi.B -eq 220 -and
            $opaque.A -eq 255 -and $opaque.R -eq 10 -and $opaque.G -eq 200 -and $opaque.B -eq 60)
    }

    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Zurück') -Description 'undo annotation')
    Start-Sleep -Milliseconds 300
    Assert-ToolbarIconsVisible -MainHandle $main.Handle -Context 'UNDO_REPAINT'
    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Wiederholen') -Description 'redo annotation')
    Start-Sleep -Milliseconds 300
    Assert-ToolbarIconsVisible -MainHandle $main.Handle -Context 'REDO_REPAINT'
    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Zurück') -Description 'undo annotation again')
    Start-Sleep -Milliseconds 300
    Assert-ToolbarIconsVisible -MainHandle $main.Handle -Context 'SECOND_UNDO_REPAINT'
    Save-FromToolbar $main.Handle
    [void](Wait-ForImageState -Path $inputPath -Description 'undo restoring the original transparent pixels' -Predicate {
        param($image)
        if ($image.Width -ne $expectedBase.Width -or $image.Height -ne $expectedBase.Height) { return $false }
        for ($i = 0; $i -lt $image.Pixels.Length; $i++) {
            $a = $image.Pixels[$i]; $b = $expectedBase.Pixels[$i]
            if ($a.R -ne $b.R -or $a.G -ne $b.G -or $a.B -ne $b.B -or $a.A -ne $b.A) { return $false }
        }
        return $true
    })

    # Select yellow explicitly so this case isolates marker rendering and compositing.
    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Marker (M)') -Description 'text marker tool')
    [void](Wait-ForValue -Seconds 5 -Description 'text marker tool activation' -Probe {
        Get-MainWindow | Where-Object { $_.Title -like '*Werkzeug Highlight*' } | Select-Object -First 1
    })
    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Gelb') -Description 'yellow marker color')
    Invoke-CanvasDrag 120 80 155 115 $false
    Save-FromToolbar $main.Handle
    $highlighted = Wait-ForImageState -Path $inputPath -Description 'saved translucent yellow marker and preserved pixels' -Predicate {
        param($image)
        if ($image.Width -ne 160 -or $image.Height -ne 120) { return $false }
        $onTransparent = $image.Pixels[90 * 160 + 130]
        $onOpaqueGreen = $image.Pixels[104 * 160 + 146]
        $outside = $image.Pixels[60 * 160 + 100]
        if ([Math]::Abs([int]$onTransparent.R - 255) -gt 3 -or
            [Math]::Abs([int]$onTransparent.G - 214) -gt 3 -or
            [Math]::Abs([int]$onTransparent.B) -gt 3 -or
            [Math]::Abs([int]$onTransparent.A - 105) -gt 3) {
            throw "Transparent marker pixel was RGBA=$($onTransparent.R),$($onTransparent.G),$($onTransparent.B),$($onTransparent.A); expected about 255,214,0,105."
        }
        if ([Math]::Abs([int]$onOpaqueGreen.R - 111) -gt 3 -or
            [Math]::Abs([int]$onOpaqueGreen.G - 206) -gt 3 -or
            [Math]::Abs([int]$onOpaqueGreen.B - 35) -gt 3 -or
            $onOpaqueGreen.A -ne 255) {
            throw "Marker over opaque green was RGBA=$($onOpaqueGreen.R),$($onOpaqueGreen.G),$($onOpaqueGreen.B),$($onOpaqueGreen.A); expected about 111,206,35,255."
        }
        $baseOutside = $expectedBase.Pixels[60 * 160 + 100]
        if ($outside.R -ne $baseOutside.R -or $outside.G -ne $baseOutside.G -or
            $outside.B -ne $baseOutside.B -or $outside.A -ne $baseOutside.A) {
            throw "Pixel outside marker changed from RGBA=$($baseOutside.R),$($baseOutside.G),$($baseOutside.B),$($baseOutside.A) to RGBA=$($outside.R),$($outside.G),$($outside.B),$($outside.A)."
        }
        return $image
    }

    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Zurück') -Description 'undo text marker')
    Save-FromToolbar $main.Handle
    [void](Wait-ForImageState -Path $inputPath -Description 'undo restoring the original image after text marker' -Predicate {
        param($image) Test-OriginalImage $image
    })

    # Shift + rectangle must produce equal side lengths in the saved image.
    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Rechteck (R)') -Description 'Shift rectangle tool')
    Invoke-CanvasDrag 30 20 70 40 $true
    Save-FromToolbar $main.Handle
    $square = Wait-ForImageState -Path $inputPath -Description 'saved Shift-constrained square' -Predicate {
        param($image)
        $bounds = Get-RedBounds $image
        return ($null -ne $bounds -and $bounds.Count -ge 60 -and [Math]::Abs((($bounds.Right-$bounds.Left)-($bounds.Bottom-$bounds.Top))) -le 3)
    }
    $squareBounds = Get-RedBounds $square
    if ([Math]::Abs((($squareBounds.Right-$squareBounds.Left)-($squareBounds.Bottom-$squareBounds.Top))) -gt 3) { throw "SHIFT_RECTANGLE_NOT_SQUARE: red bounds $($squareBounds.Left),$($squareBounds.Top)-$($squareBounds.Right),$($squareBounds.Bottom)." }
    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Zurück') -Description 'undo Shift square')
    Save-FromToolbar $main.Handle
    [void](Wait-ForImageState -Path $inputPath -Description 'undo restoring image before line snap' -Predicate { param($image) Test-OriginalImage $image })

    # A 35-by-30 drag snaps to the 45 degree diagonal. Verify saved red pixels
    # stay on that diagonal, rather than just counting a non-empty annotation.
    foreach ($toolSpec in @(@{ Label='Linie (L)'; Name='line' }, @{ Label='Pfeil (A)'; Name='arrow' })) {
        [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @($toolSpec.Label) -Description "Shift $($toolSpec.Name) tool")
        Invoke-CanvasDrag 35 35 70 65 $true
        Save-FromToolbar $main.Handle
        $snapped = Wait-ForImageState -Path $inputPath -Description "saved Shift-snapped $($toolSpec.Name)" -Predicate {
            param($image)
            $bounds = Get-RedBounds $image
            if ($null -eq $bounds -or $bounds.Count -lt 8) { return $false }
            $diagonal = 0
            for ($y=$bounds.Top; $y -le $bounds.Bottom; $y++) { for ($x=$bounds.Left; $x -le $bounds.Right; $x++) {
                $p=$image.Pixels[$y*$image.Width+$x]
                if ($p.A -gt 0 -and $p.R -gt 200 -and $p.G -lt 80 -and $p.B -lt 80 -and [Math]::Abs($x-$y) -le 3) { $diagonal++ }
            }}
            return ($diagonal -ge 8 -and [Math]::Abs(($bounds.Right-$bounds.Left)-($bounds.Bottom-$bounds.Top)) -le 12)
        }
        $bounds = Get-RedBounds $snapped
        if ([Math]::Abs(($bounds.Right-$bounds.Left)-($bounds.Bottom-$bounds.Top)) -gt 12) { throw "SHIFT_$($toolSpec.Name.ToUpper())_NOT_45_DEGREES: red bounds ($($bounds.Right-$bounds.Left))x($($bounds.Bottom-$bounds.Top))." }
        [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Zurück') -Description "undo Shift $($toolSpec.Name)")
        Save-FromToolbar $main.Handle
        [void](Wait-ForImageState -Path $inputPath -Description "undo restoring image after Shift $($toolSpec.Name)" -Predicate { param($image) Test-OriginalImage $image })
    }

    # A too-small crop keeps Crop active; a committed crop switches to Select.
    [void](Click-VisibleButton -Parent $main.Handle -ExpectedTexts @('Zuschneiden (C)') -Description 'crop tool')
    [void](Wait-ForValue -Seconds 5 -Description 'crop tool activation' -Probe {
        Get-MainWindow | Where-Object { $_.Title -like '*Werkzeug Crop*' } | Select-Object -First 1
    })
    Invoke-CanvasDrag 30 30 30 30 $false
    [void](Wait-ForValue -Seconds 2 -Description 'small crop leaving Crop active' -Probe {
        Get-MainWindow | Where-Object { $_.Title -like '*Werkzeug Crop*' } | Select-Object -First 1
    })
    Invoke-CanvasDrag 30 20 130 100 $false
    [void](Wait-ForValue -Seconds 5 -Description 'successful crop switching to Select' -Probe {
        Get-MainWindow | Where-Object { $_.Title -like '*Werkzeug Select*' } | Select-Object -First 1
    })
    Save-FromToolbar $main.Handle
    [void](Wait-ForImageState -Path $inputPath -Description 'saved cropped image dimensions' -Predicate {
        param($image)
        return ($image.Width -eq 101 -and $image.Height -eq 81)
    })
    $passMessage = 'PASS: verified Shift-constrained rectangle, line, and arrow behavior with undo; a too-small crop kept Crop active, while a committed crop saved at 101x81 and switched to Select.'
}
catch {
    $testFailure = $_
}
finally {
    if ($mouseDown) {
        try { [ShotKitEditorSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero) }
        catch { $cleanupFailures += "Could not release test mouse button: $($_.Exception.Message)" }
    }
    if ($shiftDown) {
        try { [ShotKitEditorSmoke.Native]::keybd_event(0x10, 0, 0x0002, [UIntPtr]::Zero) }
        catch { $cleanupFailures += "Could not release test Shift key: $($_.Exception.Message)" }
    }
    if ($null -ne $process) {
        try {
            if (-not $process.HasExited) {
                $actualPath = $process.MainModule.FileName
                $actualTicks = $process.StartTime.ToUniversalTime().Ticks
                if (-not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($actualPath), [IO.Path]::GetFullPath($ExePath)) -or
                    $actualTicks -ne $processStartTicks) {
                    throw 'Refusing cleanup: process identity no longer matches the test-owned executable and start time.'
                }
                $process.Kill()
                if (-not $process.WaitForExit(5000) -and -not $process.HasExited) { throw 'Test-owned editor did not exit after termination.' }
            }
            $editorStopped = $process.HasExited
        }
        catch { $cleanupFailures += "Editor cleanup failed: $($_.Exception.Message)" }
        finally { $process.Dispose() }
    }
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    if ($editorStopped -and (Test-Path -LiteralPath $testRoot)) {
        try { Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction Stop }
        catch { $cleanupFailures += "Temporary directory cleanup failed: $($_.Exception.Message)" }
    } elseif (Test-Path -LiteralPath $testRoot) {
        $cleanupFailures += "Temporary directory retained because the test-owned editor may still be running: $testRoot"
    }
}
if ($null -ne $testFailure) { throw $testFailure }
if ($cleanupFailures.Count -gt 0) { throw ('CLEANUP_FAILURE: ' + ($cleanupFailures -join '; ')) }
Write-Output $passMessage

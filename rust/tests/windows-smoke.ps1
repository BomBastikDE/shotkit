[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(5, 120)]
    [int] $StartupTimeoutSeconds = 20,

    [ValidateRange(5, 180)]
    [int] $CaptureTimeoutSeconds = 45
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) {
    throw "EXE not found: $ExePath"
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path

# The smoke test must run on a real interactive desktop. In a headless Actions
# service session this fails explicitly instead of reporting a false pass.
if ($env:GITHUB_ACTIONS -ne 'true') {
    throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'
}
if (-not [Environment]::UserInteractive) {
    throw 'Desktop session unavailable: this Windows runner is not interactive.'
}

if (-not ('ShotKitSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

namespace ShotKitSmoke {
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
        public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")]
        public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);
        [DllImport("user32.dll")]
        public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);
        [DllImport("user32.dll")]
        public static extern int GetSystemMetrics(int index);

        public static string InputDesktopName() {
            // DESKTOP_READOBJECTS; UOI_NAME
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
                windows.Add(new WindowInfo {
                    Handle = hwnd,
                    ClassName = className.ToString(),
                    Title = GetText(hwnd),
                    Visible = IsWindowVisible(hwnd)
                });
                return true;
            }, IntPtr.Zero);
            return windows.ToArray();
        }
    }
}
'@
}

$inputDesktop = [ShotKitSmoke.Native]::InputDesktopName()
if ([string]::IsNullOrWhiteSpace($inputDesktop)) {
    throw 'Desktop session unavailable: Windows could not open the interactive input desktop.'
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Windows-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$saveFolder = Join-Path $testRoot 'captures'
$process = $null
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA

function Get-OwnWindows {
    param([System.Diagnostics.Process] $OwnedProcess)
    [ShotKitSmoke.Native]::WindowsForProcess($OwnedProcess.Id)
}

function Get-OwnErrorDialog {
    param([System.Diagnostics.Process] $OwnedProcess)
    Get-OwnWindows $OwnedProcess |
        Where-Object { $_.Visible -and $_.ClassName -eq '#32770' }
}

function Assert-NoOwnErrorDialog {
    param([System.Diagnostics.Process] $OwnedProcess)
    $dialog = Get-OwnErrorDialog $OwnedProcess |
        Where-Object { $_.Title -ne 'ShotKit Rust – Prototyp' } |
        Select-Object -First 1
    if ($null -ne $dialog) {
        $details = [ShotKitSmoke.Native]::WindowText($dialog.Handle)
        throw "ShotKit displayed a dialog during the smoke test. Title='$($dialog.Title)' Text='$details'"
    }
}

function Wait-ForCondition {
    param(
        [scriptblock] $Condition,
        [int] $TimeoutSeconds,
        [string] $Description,
        [System.Diagnostics.Process] $OwnedProcess
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($OwnedProcess.HasExited) {
            throw "ShotKit exited before $Description (exit code $($OwnedProcess.ExitCode))."
        }
        Assert-NoOwnErrorDialog $OwnedProcess
        $value = & $Condition
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out after ${TimeoutSeconds}s waiting for $Description."
}

try {
    New-Item -ItemType Directory -Path $appData, $localAppData, $saveFolder -Force | Out-Null
    $settingsPath = Join-Path (Join-Path $appData 'ShotKitRust') 'settings.json'
    New-Item -ItemType Directory -Path (Split-Path -Parent $settingsPath) -Force | Out-Null
    # Isolate all persistent preferences from the runner profile and explicitly
    # disable automatic save/clipboard outputs. The capture remains in memory.
    $settings = @{
        version = 1
        save_folder = $saveFolder
        open_editor = $true
        copy_to_clipboard = $false
        auto_save = $false
        scroll_direction = 'down'
    } | ConvertTo-Json
    [IO.File]::WriteAllText($settingsPath, $settings, [System.Text.UTF8Encoding]::new($false))

    try {
        $env:APPDATA = $appData
        $env:LOCALAPPDATA = $localAppData
        $process = Start-Process -FilePath $ExePath -WorkingDirectory $testRoot -PassThru
    }
    finally {
        $env:APPDATA = $oldAppData
        $env:LOCALAPPDATA = $oldLocalAppData
    }

    $mainWindow = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the ShotKit main window' -OwnedProcess $process -Condition {
        Get-OwnWindows $process | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
    }

    # main() deliberately blocks on this informational MessageBox before its
    # message loop. Close only that known dialog belonging to this process.
    $welcome = Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'the ShotKit startup dialog' -OwnedProcess $process -Condition {
        Get-OwnErrorDialog $process | Where-Object { $_.Title -like '*Prototyp*' } | Select-Object -First 1
    }
    if (-not [ShotKitSmoke.Native]::PostMessageW($welcome.Handle, 0x0111, [IntPtr]1, [IntPtr]::Zero)) {
        throw 'Could not safely close the ShotKit startup dialog with its default OK action.'
    }
    Start-Sleep -Milliseconds 300
    Assert-NoOwnErrorDialog $process

    # F11 is the app's registered area-capture hotkey. The test profile disables
    # auto-save and clipboard, so the selected 200x200 synthetic runner region is
    # only held in memory and displayed in the editor.
    [ShotKitSmoke.Native]::keybd_event(0x7A, 0, 0, [UIntPtr]::Zero)
    [ShotKitSmoke.Native]::keybd_event(0x7A, 0, 0x0002, [UIntPtr]::Zero)
    $overlay = Wait-ForCondition -TimeoutSeconds 10 -Description 'the area-selection overlay after F11' -OwnedProcess $process -Condition {
        Get-OwnWindows $process | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible } | Select-Object -First 1
    }

    $left = [ShotKitSmoke.Native]::GetSystemMetrics(76)
    $top = [ShotKitSmoke.Native]::GetSystemMetrics(77)
    $width = [ShotKitSmoke.Native]::GetSystemMetrics(78)
    $height = [ShotKitSmoke.Native]::GetSystemMetrics(79)
    if ($width -lt 240 -or $height -lt 240) {
        throw "Interactive desktop is too small for a safe 200x200 smoke selection (${width}x${height})."
    }
    $x1 = $left + [int]($width / 2) - 100
    $y1 = $top + [int]($height / 2) - 100
    $x2 = $x1 + 200
    $y2 = $y1 + 200
    [void][ShotKitSmoke.Native]::SetForegroundWindow($overlay.Handle)
    if (-not [ShotKitSmoke.Native]::SetCursorPos($x1, $y1)) { throw 'Could not position the cursor for the selection start.' }
    Start-Sleep -Milliseconds 150
    [ShotKitSmoke.Native]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
    [void][ShotKitSmoke.Native]::SetCursorPos($x2, $y2)
    Start-Sleep -Milliseconds 200
    [ShotKitSmoke.Native]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)

    $editor = Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'a visible editor window after area capture' -OwnedProcess $process -Condition {
        $window = Get-OwnWindows $process | Where-Object {
            $_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible -and $_.Title -like '*speichern*'
        } | Select-Object -First 1
        if ($null -ne $window) { $window }
    }

    Write-Output "PASS: ShotKit area capture opened the editor on interactive desktop '$inputDesktop'."
}
finally {
    # Kill only the process handle created above. Do not search or terminate by
    # executable name, which could affect another user's ShotKit instance.
    if ($null -ne $process) {
        try {
            if (-not $process.HasExited) {
                $process.Kill()
                if (-not $process.WaitForExit(5000) -or -not $process.HasExited) {
                    throw 'The smoke-test process did not exit after termination was requested.'
                }
            }
        }
        catch { Write-Warning "Could not stop the smoke-test process: $($_.Exception.Message)" }
        $process.Dispose()
    }
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}

param(
    [Parameter(Mandatory = $true)]
    [string] $ExePath,
    [int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'

if ($TimeoutSeconds -lt 5 -or $TimeoutSeconds -gt 180) {
    throw 'TimeoutSeconds muss zwischen 5 und 180 liegen.'
}

$resolvedExe = (Resolve-Path -LiteralPath $ExePath).Path
if ([IO.Path]::GetExtension($resolvedExe) -ne '.exe') {
    throw "Erwartet wurde eine ShotKit-EXE: $resolvedExe"
}

$interopSource = @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class ShotKitInstanceSmokeNative {
    public delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr state);
    public delegate bool EnumChildProc(IntPtr hwnd, IntPtr state);

    public sealed class WindowInfo {
        public IntPtr Handle;
        public uint ProcessId;
        public string ClassName;
        public string Title;
        public bool Visible;
    }
    public sealed class ButtonInfo {
        public IntPtr Handle;
        public int ControlId;
        public string Text;
    }

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr state);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int maxCount);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern int GetClassName(IntPtr hwnd, StringBuilder text, int maxCount);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
    [DllImport("user32.dll", SetLastError = true)]
    private static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);
    [DllImport("user32.dll")]
    private static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr state);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern int GetDlgCtrlID(IntPtr hwnd);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool CloseDesktop(IntPtr desktop);

    public static WindowInfo[] GetWindows(uint wantedProcessId) {
        var found = new List<WindowInfo>();
        EnumWindowsProc callback = delegate(IntPtr hwnd, IntPtr state) {
            uint pid;
            GetWindowThreadProcessId(hwnd, out pid);
            if (pid == wantedProcessId) {
                var title = new StringBuilder(1024);
                var className = new StringBuilder(256);
                GetWindowText(hwnd, title, title.Capacity);
                GetClassName(hwnd, className, className.Capacity);
                found.Add(new WindowInfo {
                    Handle = hwnd,
                    ProcessId = pid,
                    ClassName = className.ToString(),
                    Title = title.ToString(),
                    Visible = IsWindowVisible(hwnd)
                });
            }
            return true;
        };
        EnumWindows(callback, IntPtr.Zero);
        GC.KeepAlive(callback);
        return found.ToArray();
    }

    public static bool HasInteractiveInputDesktop() {
        const uint DESKTOP_READOBJECTS = 0x0001;
        IntPtr desktop = OpenInputDesktop(0, false, DESKTOP_READOBJECTS);
        if (desktop == IntPtr.Zero) return false;
        CloseDesktop(desktop);
        return true;
    }

    public static ButtonInfo[] GetVisibleButtons(IntPtr dialog) {
        var found = new List<ButtonInfo>();
        EnumChildProc callback = delegate(IntPtr child, IntPtr state) {
            if (!IsWindowVisible(child)) return true;
            var className = new StringBuilder(128);
            GetClassName(child, className, className.Capacity);
            if (!String.Equals(className.ToString(), "Button", StringComparison.OrdinalIgnoreCase)) return true;
            var title = new StringBuilder(256);
            GetWindowText(child, title, title.Capacity);
            found.Add(new ButtonInfo { Handle = child, ControlId = GetDlgCtrlID(child), Text = title.ToString() });
            return true;
        };
        EnumChildWindows(dialog, callback, IntPtr.Zero);
        GC.KeepAlive(callback);
        return found.ToArray();
    }

    public static void ClickButton(IntPtr button) {
        const uint BM_CLICK = 0x00F5;
        if (button == IntPtr.Zero) throw new InvalidOperationException("Dialog-Schaltfläche ist null.");
        SendMessage(button, BM_CLICK, IntPtr.Zero, IntPtr.Zero);
    }
}
'@

Add-Type -TypeDefinition $interopSource -Language CSharp

function Wait-ForCondition {
    param([scriptblock] $Condition, [int] $Seconds, [string] $Failure)
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 150
    }
    throw $Failure
}

function Get-ProcessWindows {
    param([Diagnostics.Process] $Process)
    if ($Process.HasExited) { return @() }
    return @([ShotKitInstanceSmokeNative]::GetWindows([uint32] $Process.Id))
}

function Get-Dialog {
    param([Diagnostics.Process] $Process, [string] $TitlePattern)
    return (Get-ProcessWindows $Process | Where-Object {
        $_.ClassName -eq '#32770' -and $_.Title -like $TitlePattern
    } | Select-Object -First 1)
}

function Wait-ForDialogButton {
    param(
        [Diagnostics.Process] $Process,
        $Dialog,
        [string[]] $ExpectedTexts,
        [string] $Description
    )
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    $buttons = @()
    while ([DateTime]::UtcNow -lt $deadline -and -not $Process.HasExited) {
        $buttons = @([ShotKitInstanceSmokeNative]::GetVisibleButtons($Dialog.Handle))
        $match = $buttons | Where-Object {
            $caption = ($_.Text -replace '&', '').Trim()
            $ExpectedTexts -contains $caption
        } | Select-Object -First 1
        if ($null -ne $match) {
            [ShotKitInstanceSmokeNative]::ClickButton($match.Handle)
            return $match
        }
        Start-Sleep -Milliseconds 100
    }
    $buttonDetails = ($buttons | ForEach-Object {
        "HWND=0x$($_.Handle.ToInt64().ToString('X')) ID=$($_.ControlId) Text='$($_.Text)'"
    }) -join '; '
    if (-not $buttonDetails) { $buttonDetails = 'no visible Button-class children' }
    $windowText = [ShotKitInstanceSmokeNative]::GetWindowText($Dialog.Handle)
    $exitDetail = if ($Process.HasExited) { " Process exited with code $($Process.ExitCode)." } else { '' }
    throw "BUTTON_TIMEOUT: $Description on dialog '$($Dialog.Title)' (HWND $($Dialog.Handle)); text='$windowText'; buttons=$buttonDetails.$exitDetail"
}

function Wait-ForDialogClosed {
    param([Diagnostics.Process] $Process, $Dialog, [string] $Description)
    $null = Wait-ForCondition {
        $Process.HasExited -or $null -eq (Get-ProcessWindows $Process | Where-Object { $_.Handle -eq $Dialog.Handle })
    } 5 $Description
}

if (-not [Environment]::UserInteractive -or -not [ShotKitInstanceSmokeNative]::HasInteractiveInputDesktop()) {
    throw 'NO_INTERACTIVE_DESKTOP: Dieser Smoke-Test benötigt eine interaktive Windows-Desktop-Sitzung; der Test wird nicht als bestanden gewertet.'
}
$imageName = [IO.Path]::GetFileNameWithoutExtension($resolvedExe)
$existing = @(Get-Process -Name $imageName -ErrorAction SilentlyContinue)
if ($existing.Count -gt 0) {
    throw "PREEXISTING_PROCESS: Es läuft bereits ein Prozess namens '$imageName'. Für einen isolierten Smoke-Test muss der Runner sauber sein; bestehende Prozesse wurden nicht verändert."
}

$root = Join-Path ([IO.Path]::GetTempPath()) ("shotkit-instance-smoke-" + [Guid]::NewGuid().ToString('N'))
$profile = Join-Path $root 'profile'
$localProfile = Join-Path $root 'local-profile'
$tempFiles = Join-Path $root 'temp'
$probeFile = Join-Path $root 'second-instance-probe.txt'
$primary = $null
$secondActivate = $null
$secondFile = $null
$testFailure = $null
$cleanupErrors = @()

function New-ShotKitProcess {
    param([string[]] $Arguments)
    $start = New-Object Diagnostics.ProcessStartInfo
    $start.FileName = $resolvedExe
    $start.WorkingDirectory = $root
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.EnvironmentVariables['APPDATA'] = $profile
    $start.EnvironmentVariables['LOCALAPPDATA'] = $localProfile
    $start.EnvironmentVariables['TEMP'] = $tempFiles
    $start.EnvironmentVariables['TMP'] = $tempFiles
    if ($Arguments.Count -gt 0) {
        # Generated probe path is a normal Windows path and cannot contain a quote.
        $start.Arguments = '"' + ($Arguments -join '" "') + '"'
    }
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $start
    if (-not $process.Start()) { throw 'ShotKit-Prozess konnte nicht gestartet werden.' }
    return $process
}

try {
    New-Item -ItemType Directory -Path $profile, $localProfile, $tempFiles -Force | Out-Null
    Set-Content -LiteralPath $probeFile -Value 'ShotKit single-instance smoke probe; safe to discard.' -Encoding ASCII

    $primary = New-ShotKitProcess @()
    $welcomeTitle = '*Prototyp*'
    Wait-ForCondition {
        -not $primary.HasExited -and $null -ne (Get-Dialog $primary $welcomeTitle)
    } $TimeoutSeconds 'STARTUP_DIALOG_TIMEOUT: Startdialog der ersten Instanz wurde nicht gefunden.'
    $welcome = Get-Dialog $primary $welcomeTitle
    $welcomeButton = Wait-ForDialogButton $primary $welcome @('OK') 'IDOK'
    Wait-ForDialogClosed $primary $welcome 'STARTUP_DIALOG_CLOSE_TIMEOUT: Startdialog blieb nach IDOK geöffnet.'

    Wait-ForCondition {
        -not $primary.HasExited -and @((Get-ProcessWindows $primary | Where-Object {
            $_.ClassName -eq 'ShotKitRustPrototype'
        })).Count -gt 0
    } $TimeoutSeconds 'PRIMARY_WINDOW_TIMEOUT: Hauptfenster der ersten Instanz wurde nicht erstellt.'

    $secondActivate = New-ShotKitProcess @()
    if (-not $secondActivate.WaitForExit($TimeoutSeconds * 1000)) {
        throw 'ACTIVATION_TIMEOUT: Zweitstart ohne Argumente hat sich nicht kontrolliert beendet.'
    }
    if ($secondActivate.ExitCode -ne 0) {
        throw "ACTIVATION_FAILED: Zweitstart endete mit Exitcode $($secondActivate.ExitCode)."
    }
    Wait-ForCondition {
        -not $primary.HasExited -and @((Get-ProcessWindows $primary | Where-Object {
            $_.ClassName -eq 'ShotKitRustPrototype' -and $_.Visible
        })).Count -gt 0
    } $TimeoutSeconds 'ACTIVATION_NOT_VISIBLE: Die primäre ShotKit-Instanz wurde nicht sichtbar aktiviert.'

    $secondFile = New-ShotKitProcess @($probeFile)
    $confirmTitle = '*Datei aus zweiter Instanz*'
    Wait-ForCondition {
        -not $primary.HasExited -and $null -ne (Get-Dialog $primary $confirmTitle)
    } $TimeoutSeconds 'FILE_PROMPT_TIMEOUT: Der Dateibestätigungsdialog der ersten Instanz wurde nicht ausgelöst.'
    $confirmation = Get-Dialog $primary $confirmTitle
    # Select the visible localized No button to ensure the second process reports cancellation.
    $declineButton = Wait-ForDialogButton $primary $confirmation @('No', 'Nein') 'IDNO'
    Wait-ForDialogClosed $primary $confirmation 'FILE_PROMPT_CLOSE_TIMEOUT: Bestätigungsdialog blieb nach Ablehnung geöffnet.'

    $declinedTitle = '*Datei nicht geöffnet*'
    Wait-ForCondition {
        -not $secondFile.HasExited -and $null -ne (Get-Dialog $secondFile $declinedTitle)
    } $TimeoutSeconds 'DECLINE_RESULT_TIMEOUT: Zweitprozess hat die Ablehnung nicht sichtbar gemeldet.'
    $declined = Get-Dialog $secondFile $declinedTitle
    $declinedButton = Wait-ForDialogButton $secondFile $declined @('OK') 'IDOK'
    Wait-ForDialogClosed $secondFile $declined 'DECLINE_DIALOG_CLOSE_TIMEOUT: Ablehnungsdialog blieb nach OK geöffnet.'

    if (-not $secondFile.WaitForExit($TimeoutSeconds * 1000)) {
        throw 'DECLINE_TIMEOUT: Zweitstart mit Dateipfad beendete sich nach Ablehnung nicht.'
    }
    if ($secondFile.ExitCode -ne 0) {
        throw "DECLINE_NOT_CONTROLLED: Zweitstart endete nach Ablehnung mit Exitcode $($secondFile.ExitCode)."
    }
    if ($primary.HasExited) {
        throw 'PRIMARY_EXITED: Die erste Instanz wurde während der Ablehnung beendet.'
    }

} catch {
    $testFailure = $_.Exception.Message
} finally {
    foreach ($owned in @($secondFile, $secondActivate, $primary)) {
        if ($null -ne $owned) {
            try {
                if (-not $owned.HasExited) {
                    $owned.Kill()
                    if (-not $owned.WaitForExit(5000)) {
                        throw "Prozess $($owned.Id) blieb nach Kill aktiv."
                    }
                } elseif (-not $owned.WaitForExit(5000)) {
                    throw "Exit von Prozess $($owned.Id) konnte nicht bestätigt werden."
                }
            } catch {
                $cleanupErrors += "Prozess $($owned.Id): $($_.Exception.Message)"
            } finally {
                try { $owned.Dispose() }
                catch { $cleanupErrors += "Prozessobjekt $($owned.Id): $($_.Exception.Message)" }
            }
        }
    }
    try {
        if (Test-Path -LiteralPath $root) {
            Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop
        }
        if (Test-Path -LiteralPath $root) {
            throw 'Temporäres Testverzeichnis ist nach dem Entfernen noch vorhanden.'
        }
    } catch {
        $cleanupErrors += "Temporäre Daten: $($_.Exception.Message)"
    }
}

if ($cleanupErrors.Count -gt 0) {
    $details = $cleanupErrors -join '; '
    if ($testFailure) { throw "TEST_AND_CLEANUP_FAILED: $testFailure Cleanup: $details" }
    throw "CLEANUP_FAILED: $details"
}
if ($testFailure) { throw $testFailure }

Write-Output 'PASS: Zweitstart aktiviert die primäre Instanz; Dateianfrage wird angezeigt und Ablehnung kontrolliert zurückgemeldet. Cleanup bestätigt.'

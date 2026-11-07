[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 is required.' }
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'EPHEMERAL_CI_ONLY: first-start UI smoke requires an interactive GitHub Actions Windows runner.'
}
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
if ([IO.Path]::GetExtension($ExePath) -ine '.exe') { throw 'A ShotKit executable is required.' }

if (-not ('ShotKitFirstStartSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitFirstStartSmoke {
    public sealed class WindowInfo {
        public IntPtr Handle;
        public string ClassName;
        public string Title;
        public string Text;
        public bool Visible;
    }
    public static class Native {
        private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
        private delegate bool ChildProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr data);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, ChildProc cb, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
        static string Text(IntPtr hwnd) { var b=new StringBuilder(2048); GetWindowTextW(hwnd,b,b.Capacity); return b.ToString(); }
        static string AllText(IntPtr hwnd) {
            var b=new StringBuilder(Text(hwnd));
            EnumChildWindows(hwnd, delegate(IntPtr child, IntPtr ignored) {
                string t=Text(child); if(!String.IsNullOrWhiteSpace(t)) b.Append(" ").Append(t); return true;
            }, IntPtr.Zero);
            return b.ToString();
        }
        public static WindowInfo[] Windows(int processId) {
            var result=new List<WindowInfo>();
            EnumProc callback=delegate(IntPtr hwnd, IntPtr ignored) {
                uint pid; GetWindowThreadProcessId(hwnd,out pid); if(pid!=(uint)processId) return true;
                var c=new StringBuilder(256); GetClassNameW(hwnd,c,c.Capacity);
                result.Add(new WindowInfo { Handle=hwnd, ClassName=c.ToString(), Title=Text(hwnd), Text=AllText(hwnd), Visible=IsWindowVisible(hwnd) });
                return true;
            };
            EnumWindows(callback,IntPtr.Zero); GC.KeepAlive(callback); return result.ToArray();
        }
    }
}
'@
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-FirstStart-' + [Guid]::NewGuid().ToString('N'))
$profiles = @{
    first = Join-Path $testRoot 'first'
    existing = Join-Path $testRoot 'existing'
    argument = Join-Path $testRoot 'argument'
}
$processes = [Collections.Generic.List[Diagnostics.Process]]::new()
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA

function Set-TestProfile([string] $Name) {
    $base = $profiles[$Name]
    $env:APPDATA = Join-Path $base 'appdata'
    $env:LOCALAPPDATA = Join-Path $base 'localappdata'
    [IO.Directory]::CreateDirectory($env:APPDATA) | Out-Null
    [IO.Directory]::CreateDirectory($env:LOCALAPPDATA) | Out-Null
}

function Get-ProcessWindows([Diagnostics.Process] $Process) {
    [ShotKitFirstStartSmoke.Native]::Windows($Process.Id)
}

function Wait-ForMainWindow([Diagnostics.Process] $Process) {
    $deadline=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while([DateTime]::UtcNow -lt $deadline) {
        $Process.Refresh()
        if($Process.HasExited) { throw "ShotKit exited during startup (code $($Process.ExitCode))." }
        # The tray host is deliberately created hidden; identify it by its
        # stable internal class and owning PID rather than visibility/title.
        $main=Get-ProcessWindows $Process | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
        if($null -ne $main) { return $main }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out waiting for the app's main window (PID $($Process.Id))."
}

function Get-ModalDialogs([Diagnostics.Process] $Process) {
    @(Get-ProcessWindows $Process | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' })
}

function Wait-ForNotice([Diagnostics.Process] $Process) {
    $deadline=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while([DateTime]::UtcNow -lt $deadline) {
        $Process.Refresh()
        if($Process.HasExited) { throw "ShotKit exited before first-start notice (code $($Process.ExitCode))." }
        $dialogs=Get-ModalDialogs $Process
        if($dialogs.Count -gt 1) { throw "Multiple dialogs appeared at first start: $(( $dialogs | ForEach-Object { $_.Title + ': ' + $_.Text } ) -join '; ')" }
        if($dialogs.Count -eq 1) {
            $dialog=$dialogs[0]
            if($dialog.Text -notmatch 'ShotKit läuft im Infobereich') {
                throw "Unexpected dialog at first start. Title='$($dialog.Title)' Text='$($dialog.Text)'"
            }
            return $dialog
        }
        Start-Sleep -Milliseconds 100
    }
    throw 'Timed out waiting for the first-start notice.'
}

function Assert-NoDialog([Diagnostics.Process] $Process, [string] $Scenario) {
    # Wait briefly after the main window appears so a delayed startup modal cannot pass unnoticed.
    $deadline=[DateTime]::UtcNow.AddSeconds(2)
    do {
        $Process.Refresh()
        if($Process.HasExited) { throw "ShotKit exited in '$Scenario' (code $($Process.ExitCode))." }
        $dialogs=Get-ModalDialogs $Process
        if($dialogs.Count -gt 0) {
            $details=$dialogs | ForEach-Object { "'$($_.Title)': $($_.Text)" }
            throw "Unexpected dialog in '$Scenario': $($details -join '; ')"
        }
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $deadline)
}

function Start-ShotKit([string[]] $Arguments = @()) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $ExePath
    $start.UseShellExecute = $false
    foreach($arg in $Arguments) { [void]$start.ArgumentList.Add($arg) }
    $p=[Diagnostics.Process]::Start($start)
    if($null -eq $p) { throw 'Could not start ShotKit.' }
    $processes.Add($p)
    [void](Wait-ForMainWindow $p)
    return $p
}

function Stop-ShotKit([Diagnostics.Process] $Process) {
    if($null -eq $Process) { return }
    $Process.Refresh()
    if(-not $Process.HasExited) {
        try { Stop-Process -Id $Process.Id -Force -ErrorAction Stop } catch {
            $Process.Refresh(); if(-not $Process.HasExited) { throw }
        }
        if(-not $Process.WaitForExit(10000)) { throw "ShotKit PID $($Process.Id) did not stop." }
    }
}

try {
    foreach($path in $profiles.Values) { [IO.Directory]::CreateDirectory($path) | Out-Null }

    # A: Missing settings -> show and acknowledge notice -> restart -> no notice.
    Set-TestProfile 'first'
    $settingsA=Join-Path (Join-Path $env:APPDATA 'ShotKitRust') 'settings.json'
    if(Test-Path -LiteralPath $settingsA) { throw 'First-start fixture unexpectedly contains settings.' }
    $first=Start-ShotKit
    $notice=Wait-ForNotice $first
    Write-Host "PASS first-start notice shown: PID=$($first.Id), title='$($notice.Title)'"
    # IDOK (1) closes only this process-owned modal dialog; no global keystrokes are sent.
    if(-not [ShotKitFirstStartSmoke.Native]::PostMessageW($notice.Handle,0x0111,[IntPtr]1,[IntPtr]::Zero)) {
        throw 'Could not acknowledge the first-start notice with WM_COMMAND/IDOK.'
    }
    $deadline=[DateTime]::UtcNow.AddSeconds(5)
    while([DateTime]::UtcNow -lt $deadline -and (Get-ModalDialogs $first).Count -gt 0) { Start-Sleep -Milliseconds 100 }
    if((Get-ModalDialogs $first).Count -gt 0) { throw 'First-start notice remained open after acknowledgment.' }
    $deadline=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while([DateTime]::UtcNow -lt $deadline -and -not [IO.File]::Exists($settingsA)) { Start-Sleep -Milliseconds 100 }
    if(-not [IO.File]::Exists($settingsA)) { throw 'Acknowledgment did not persist first-start settings.' }
    Stop-ShotKit $first

    $restart=Start-ShotKit
    Assert-NoDialog $restart 'restart after acknowledging first-start notice'
    Write-Host "PASS restart after acknowledgment has no dialog: PID=$($restart.Id)"
    Stop-ShotKit $restart

    # B: A valid, pre-existing settings file suppresses the first-start notice.
    Set-TestProfile 'existing'
    $settingsDir=Join-Path $env:APPDATA 'ShotKitRust'
    [IO.Directory]::CreateDirectory($settingsDir) | Out-Null
    $settingsB=Join-Path $settingsDir 'settings.json'
    [IO.File]::WriteAllText($settingsB,'{"version":1,"open_editor":true,"copy_to_clipboard":false,"exclude_from_clipboard_history":true,"auto_save":false,"scroll_direction":"down"}',[Text.UTF8Encoding]::new($false))
    $existing=Start-ShotKit
    Assert-NoDialog $existing 'existing settings'
    Write-Host "PASS existing settings suppress notice: PID=$($existing.Id)"
    Stop-ShotKit $existing

    # C: File argument on a genuinely fresh profile suppresses the notice.
    Set-TestProfile 'argument'
    $png=Join-Path $testRoot 'argument.png'
    [IO.File]::WriteAllBytes($png,[Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p0sAAAAASUVORK5CYII='))
    $argument=Start-ShotKit @($png)
    Assert-NoDialog $argument 'fresh start with image argument'
    $editorDeadline=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $editor=$null
    do {
        $editor=Get-ProcessWindows $argument | Where-Object { $_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible } | Select-Object -First 1
        if($null -ne $editor) { break }
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $editorDeadline)
    if($null -eq $editor) { throw 'File-argument scenario opened no visible ShotKit editor; the argument may not have been consumed.' }
    Write-Host "PASS file argument opens editor without notice: PID=$($argument.Id)"
    Stop-ShotKit $argument

    Write-Host 'PASS all four first-start scenarios.'
}
finally {
    foreach($process in $processes) { Stop-ShotKit $process }
    $env:APPDATA=$oldAppData
    $env:LOCALAPPDATA=$oldLocalAppData

    # Delete only the uniquely generated fixture root. Refuse traversal if any
    # reparse point appeared anywhere in it; this prevents cleanup from following links.
    if(Test-Path -LiteralPath $testRoot -PathType Container) {
        $rootInfo=[IO.DirectoryInfo]::new($testRoot)
        if(($rootInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw "Refusing cleanup: generated test root became a reparse point: $testRoot"
        }
        $reparse=Get-ChildItem -LiteralPath $testRoot -Force -Recurse -ErrorAction Stop | Where-Object { ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 } | Select-Object -First 1
        if($null -ne $reparse) { throw "Refusing cleanup: reparse point found in generated fixture: $($reparse.FullName)" }
        Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction Stop
    }
}

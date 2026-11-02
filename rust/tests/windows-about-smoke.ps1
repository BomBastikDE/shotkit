[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,

    [ValidateRange(10, 120)]
    [int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') {
    throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'
}
if (-not [Environment]::UserInteractive) {
    throw 'NO_INTERACTIVE_DESKTOP: the About dialog smoke test needs an interactive Windows desktop.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\Cargo.toml') -Raw
if ($manifest -notmatch '(?ms)^\[package\].*?^version\s*=\s*"([^"]+)"') {
    throw 'Could not read [package].version from Cargo.toml.'
}
$expectedVersion = $Matches[1]
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: '$imageName' is already running; refusing to interact with it."
}

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitAboutSmoke {
    public sealed class WindowInfo { public IntPtr Handle; public int Pid; public string ClassName; public string Title; public bool Visible; }
    public static class Native {
        private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc cb, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        public static WindowInfo[] Windows(int wantedPid) {
            var list = new List<WindowInfo>();
            EnumProc cb = delegate(IntPtr h, IntPtr d) { uint pid; GetWindowThreadProcessId(h, out pid); if (wantedPid == 0 || pid == (uint)wantedPid) {
                var c = new StringBuilder(256); var t = new StringBuilder(1024); GetClassNameW(h,c,c.Capacity); GetWindowTextW(h,t,t.Capacity);
                list.Add(new WindowInfo { Handle=h, Pid=(int)pid, ClassName=c.ToString(), Title=t.ToString(), Visible=IsWindowVisible(h) }); } return true; };
            EnumWindows(cb, IntPtr.Zero); GC.KeepAlive(cb); return list.ToArray();
        }
        public static string WindowText(IntPtr h) {
            var b = new StringBuilder();
            EnumProc cb = delegate(IntPtr child, IntPtr d) { var t = new StringBuilder(1024); GetWindowTextW(child,t,t.Capacity); if (t.Length > 0) b.Append(' ').Append(t); return true; };
            EnumChildWindows(h,cb,IntPtr.Zero); GC.KeepAlive(cb); return b.ToString();
        }
    }
}
'@
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-About-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'
$localAppData = Join-Path $root 'localappdata'
$process = $null
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$failure = $null
try {
    New-Item -ItemType Directory -Path $appData, $localAppData -Force | Out-Null
    try {
        $env:APPDATA = $appData
        $env:LOCALAPPDATA = $localAppData
        $process = Start-Process -FilePath $ExePath -WorkingDirectory $root -PassThru
    } finally { $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData }

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $main = $null
    $welcome = $null
    while ([DateTime]::UtcNow -lt $deadline -and -not $process.HasExited) {
        $windows = @([ShotKitAboutSmoke.Native]::Windows($process.Id))
        $main = $windows | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
        $welcome = $windows | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*' } | Select-Object -First 1
        if ($main -and $welcome) { break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $main -or -not $welcome) { throw 'STARTUP_TIMEOUT: did not find ShotKit main window and its expected startup dialog.' }
    if (-not [ShotKitAboutSmoke.Native]::PostMessageW($welcome.Handle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'STARTUP_DIALOG_CLOSE_POST_FAILED: could not close the startup dialog.' }
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    while ([DateTime]::UtcNow -lt $deadline -and [ShotKitAboutSmoke.Native]::IsWindow($welcome.Handle)) { Start-Sleep -Milliseconds 100 }
    if ([ShotKitAboutSmoke.Native]::IsWindow($welcome.Handle)) { throw 'STARTUP_DIALOG_CLOSE_TIMEOUT: startup dialog did not close.' }

    # Exercise the same app window callback emitted by the registered tray icon for WM_RBUTTONUP.
    # It opens the real TrackPopupMenu; selection below is against the visible native menu item.
    if (-not [ShotKitAboutSmoke.Native]::PostMessageW($main.Handle, 0x8001, [IntPtr]1, [IntPtr]0x0205)) {
        throw 'TRAY_CALLBACK_POST_FAILED: could not post the tray right-click notification.'
    }
    # TrackPopupMenu creates a native #32768 popup window. Scope UIA to that
    # visible menu HWND instead of the desktop tree: menu items are hosted by
    # the popup window and may not report the app PID through UI Automation.
    $popup = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    while ([DateTime]::UtcNow -lt $deadline) {
        $popup = @([ShotKitAboutSmoke.Native]::Windows($process.Id) | Where-Object { $_.Visible -and $_.ClassName -eq '#32768' } | Select-Object -First 1)
        if ($popup.Count -gt 0) { $popup = $popup[0]; break }
        if ($process.HasExited) { throw "PROCESS_EXITED before native tray popup appeared (code $($process.ExitCode))." }
        Start-Sleep -Milliseconds 100
    }
    if (-not $popup) { throw 'TRAY_MENU_TIMEOUT: the app process did not create a visible native #32768 popup after its tray callback.' }
    $menuRoot = [System.Windows.Automation.AutomationElement]::FromHandle($popup.Handle)
    $menuItemCondition = [System.Windows.Automation.AndCondition]::new(@(
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, 'Über ShotKit'),
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::MenuItem)
    ))
    $menuItem = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $deadline) {
        $menuItem = $menuRoot.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $menuItemCondition)
        if ($menuItem) { break }
        if ($process.HasExited) { throw "PROCESS_EXITED before tray menu opened (code $($process.ExitCode))." }
        Start-Sleep -Milliseconds 150
    }
    if (-not $menuItem) {
        $names = @()
        try {
            $children = $menuRoot.FindAll([System.Windows.Automation.TreeScope]::Children, [System.Windows.Automation.Condition]::TrueCondition)
            foreach ($child in $children) { if ($child.Current.Name) { $names += $child.Current.Name } }
        } catch { $names += "UIA enumeration error: $($_.Exception.Message)" }
        throw "TRAY_MENU_ITEM_TIMEOUT: visible native #32768 menu did not expose the exact 'Über ShotKit' item. Items='$($names -join ' | ')'"
    }
    $invoke = $menuItem.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
    $invoke.Invoke()

    $about = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ([DateTime]::UtcNow -lt $deadline) {
        $about = @([ShotKitAboutSmoke.Native]::Windows($process.Id) | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustAboutWindow' -and $_.Title -eq 'Über ShotKit' }) | Select-Object -First 1
        if ($about) { break }
        if ($process.HasExited) { throw "PROCESS_EXITED before About dialog appeared (code $($process.ExitCode))." }
        Start-Sleep -Milliseconds 100
    }
    if (-not $about) { throw 'ABOUT_WINDOW_TIMEOUT: selecting the tray menu item did not open the expected ShotKitRustAboutWindow.' }

    $dialogElement = [System.Windows.Automation.AutomationElement]::FromHandle($about.Handle)
    $allText = [ShotKitAboutSmoke.Native]::WindowText($about.Handle)
    try {
        $textCondition = [System.Windows.Automation.Condition]::TrueCondition
        $descendants = $dialogElement.FindAll([System.Windows.Automation.TreeScope]::Descendants, $textCondition)
        foreach ($element in $descendants) { if ($element.Current.Name) { $allText += ' ' + $element.Current.Name } }
    } catch { throw "UIA_CONTENT_READ_FAILED: $($_.Exception.Message)" }
    foreach ($expectedText in @('ShotKit Rust', "Version: $expectedVersion", 'Entwickler: Oliver Muenk', 'Website öffnen: alleswasbewegt.de')) {
        if ($allText -notmatch [regex]::Escape($expectedText)) {
            throw "ABOUT_CONTENT_MISSING: expected '$expectedText' in the native About window. Content='$allText'"
        }
    }
    # Never activate the website button; close only this test-owned About window.
    if (-not [ShotKitAboutSmoke.Native]::PostMessageW($about.Handle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw 'ABOUT_CLOSE_POST_FAILED: could not post WM_CLOSE to the About window.'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    while ([DateTime]::UtcNow -lt $deadline -and [ShotKitAboutSmoke.Native]::IsWindow($about.Handle)) { Start-Sleep -Milliseconds 100 }
    if ([ShotKitAboutSmoke.Native]::IsWindow($about.Handle)) { throw 'ABOUT_CLOSE_TIMEOUT: About window remained open after WM_CLOSE.' }
    Write-Host "PASS: tray callback opened About window; name, version $expectedVersion, developer and website button text verified; WM_CLOSE closed it. Website was not opened."
} catch { $failure = $_ }
finally {
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    if ($process -and -not $process.HasExited) {
        try {
            # WM_CLOSE targets only the process created by this test.
            $owned = @([ShotKitAboutSmoke.Native]::Windows($process.Id))
            foreach ($window in $owned) { [void][ShotKitAboutSmoke.Native]::PostMessageW($window.Handle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
            if (-not $process.WaitForExit(5000)) { $process.Kill(); $process.WaitForExit() }
        } catch { if (-not $failure) { $failure = $_ } }
    }
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
}
if ($failure) { throw $failure }

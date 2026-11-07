[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(20, 120)][int] $StartupTimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or
    $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'INTERACTIVE_WINDOWS_REQUIRED: run only on an ephemeral interactive GitHub Actions runner or with SHOTKIT_LOCAL_SMOKE_TEST=1 in a dedicated session.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: '$imageName' is already running; refusing to interact with it."
}

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitDelayedCaptureSmoke {
    public sealed class WindowInfo {
        public IntPtr Handle; public int Pid; public string ClassName; public string Title; public bool Visible;
    }
    public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }
    public static class Native {
        private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll", SetLastError=true)] private static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll")] private static extern bool CloseDesktop(IntPtr desktop);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] private static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder info, uint length, out uint needed);
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc cb, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] private static extern int GetMenuItemCount(IntPtr menu);
        [DllImport("user32.dll")] private static extern uint GetMenuItemID(IntPtr menu, uint position);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern uint GetMenuStringW(IntPtr menu, uint position, StringBuilder text, int max, uint flags);
        [DllImport("user32.dll")] private static extern bool GetMenuItemRect(IntPtr hwnd, IntPtr menu, uint position, out Rect rect);

        public static string InputDesktopName() {
            IntPtr desktop=OpenInputDesktop(0,false,0x0001);
            if(desktop==IntPtr.Zero) return null;
            try {
                var name=new StringBuilder(256); uint needed;
                if(!GetUserObjectInformationW(desktop,2,name,(uint)(name.Capacity*2),out needed)) return null;
                return name.ToString();
            } finally { CloseDesktop(desktop); }
        }
        private static string Read(IntPtr hwnd, bool cls) {
            var b=new StringBuilder(cls?256:1024);
            if(cls) GetClassNameW(hwnd,b,b.Capacity); else GetWindowTextW(hwnd,b,b.Capacity);
            return b.ToString();
        }
        public static WindowInfo[] Windows(int wantedPid) {
            var result=new List<WindowInfo>();
            EnumProc cb=delegate(IntPtr h,IntPtr d) {
                uint pid; GetWindowThreadProcessId(h,out pid);
                if(pid==(uint)wantedPid) result.Add(new WindowInfo {Handle=h,Pid=(int)pid,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});
                return true;
            };
            EnumWindows(cb,IntPtr.Zero); GC.KeepAlive(cb); return result.ToArray();
        }
        public static int OwnerPid(IntPtr hwnd) { uint pid; GetWindowThreadProcessId(hwnd,out pid); return (int)pid; }
        public static string WindowClass(IntPtr hwnd) { return Read(hwnd,true); }
        public static string WindowTitle(IntPtr hwnd) { return Read(hwnd,false); }
        public static string ClickTrayItem(IntPtr popup, int expectedPid, uint expectedId, string expectedText) {
            if(OwnerPid(popup)!=expectedPid || WindowClass(popup)!="#32768" || !IsWindowVisible(popup))
                throw new InvalidOperationException("Refusing menu click: popup is not a visible #32768 window owned by the test process.");
            const uint MN_GETHMENU=0x01E1, MF_BYPOSITION=0x0400;
            IntPtr menu=SendMessageW(popup,MN_GETHMENU,IntPtr.Zero,IntPtr.Zero);
            if(menu==IntPtr.Zero) throw new InvalidOperationException("MN_GETHMENU returned null.");
            int count=GetMenuItemCount(menu), match=-1;
            if(count<0) throw new InvalidOperationException("GetMenuItemCount failed.");
            var entries=new StringBuilder();
            for(uint i=0;i<(uint)count;i++) {
                uint id=GetMenuItemID(menu,i); var label=new StringBuilder(512);
                GetMenuStringW(menu,i,label,label.Capacity,MF_BYPOSITION);
                entries.Append(" [").Append(id).Append(":'").Append(label).Append("']");
                if(id==expectedId && String.Equals(label.ToString(),expectedText,StringComparison.Ordinal)) {
                    if(match>=0) throw new InvalidOperationException("Exact delay menu item is duplicated.");
                    match=(int)i;
                }
            }
            if(match<0) throw new InvalidOperationException("Exact menu item missing; entries="+entries);
            Rect rect;
            if(!GetMenuItemRect(IntPtr.Zero,menu,(uint)match,out rect) || rect.Right<=rect.Left || rect.Bottom<=rect.Top)
                throw new InvalidOperationException("GetMenuItemRect failed for the verified menu item.");
            int x=rect.Left+(rect.Right-rect.Left)/2, y=rect.Top+(rect.Bottom-rect.Top)/2;
            if(!SetCursorPos(x,y)) throw new InvalidOperationException("SetCursorPos failed for the verified menu item.");
            mouse_event(0x0002,0,0,0,UIntPtr.Zero); mouse_event(0x0004,0,0,0,UIntPtr.Zero);
            return "menu id="+expectedId+" label='"+expectedText+"' rect="+rect.Left+","+rect.Top+","+rect.Right+","+rect.Bottom;
        }
    }
}
'@

$inputDesktop = [ShotKitDelayedCaptureSmoke.Native]::InputDesktopName()
if ([string]::IsNullOrWhiteSpace($inputDesktop)) {
    throw 'INPUT_DESKTOP_UNAVAILABLE: Windows could not open the interactive input desktop.'
}
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Delayed-Capture-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$settingsDir = Join-Path $appData 'ShotKitRust'
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$process = $null
$failure = $null
$heldKey = $false
$delayCases = @(
    @{ Seconds=3; Id=9; Label='Bereich nach 3 Sekunden auswählen' },
    @{ Seconds=5; Id=10; Label='Bereich nach 5 Sekunden auswählen' },
    @{ Seconds=10; Id=11; Label='Bereich nach 10 Sekunden auswählen' }
)

function Get-OwnedWindows {
    if ($null -eq $process -or $process.HasExited) { return @() }
    return @([ShotKitDelayedCaptureSmoke.Native]::Windows($process.Id))
}
function Get-OwnedDialogs {
    @(Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' })
}
function Wait-For([scriptblock] $Probe, [string] $Description, [int] $Seconds = 10) {
    $deadline=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $deadline) {
        if($process.HasExited) { throw "PROCESS_EXITED while waiting for $Description (code $($process.ExitCode))." }
        $value=& $Probe
        if($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 50
    }
    $windows=(Get-OwnedWindows | ForEach-Object { "HWND=$($_.Handle) class='$($_.ClassName)' title='$($_.Title)' visible=$($_.Visible)" }) -join '; '
    throw "TIMEOUT waiting for $Description; owned windows=$windows"
}
function Open-OwnedTrayMenu {
    $main=Get-OwnedWindows | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
    if($null -eq $main) { throw 'MAIN_WINDOW_MISSING: cannot open tray menu without the owned ShotKit main window.' }
    if(-not [ShotKitDelayedCaptureSmoke.Native]::PostMessageW($main.Handle,0x8001,[IntPtr]1,[IntPtr]0x0205)) {
        throw 'TRAY_CALLBACK_POST_FAILED: could not request the owned tray right-click callback.'
    }
    return Wait-For -Seconds 8 -Description 'visible owned native tray popup' -Probe {
        Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq '#32768' } | Select-Object -First 1
    }
}
function Invoke-DelayCase($Case) {
    if(@(Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitSelectionOverlay' }).Count -gt 0) {
        throw "STALE_OVERLAY_BEFORE_DELAY: selection overlay already visible before $($Case.Seconds)-second case."
    }
    if(@(Get-OwnedDialogs).Count -gt 0) {
        $details=(Get-OwnedDialogs | ForEach-Object { "HWND=$($_.Handle) title='$($_.Title)'" }) -join '; '
        throw "UNEXPECTED_DIALOG_BEFORE_DELAY: $details"
    }
    $popup=Open-OwnedTrayMenu
    $clock=[Diagnostics.Stopwatch]::StartNew()
    try {
        $menuEvidence=[ShotKitDelayedCaptureSmoke.Native]::ClickTrayItem($popup.Handle,$process.Id,[uint32]$Case.Id,[string]$Case.Label)
    } catch { throw "DELAY_MENU_SELECTION_FAILED for $($Case.Seconds)s: $($_.Exception.Message)" }

    $expected=[TimeSpan]::FromSeconds([double]$Case.Seconds)
    $earlyTolerance=[TimeSpan]::FromMilliseconds(300)
    $latest=$expected.Add([TimeSpan]::FromSeconds(3))
    $overlay=$null
    while($clock.Elapsed -lt $latest) {
        if($process.HasExited) { throw "PROCESS_EXITED during $($Case.Seconds)s delay (code $($process.ExitCode))." }
        $dialogs=@(Get-OwnedDialogs)
        if($dialogs.Count -gt 0) {
            $details=($dialogs | ForEach-Object { "HWND=$($_.Handle) title='$($_.Title)'" }) -join '; '
            throw "UNEXPECTED_DIALOG_DURING_DELAY: $details"
        }
        $overlays=@(Get-OwnedWindows | Where-Object { $_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible })
        if($overlays.Count -gt 1) { throw "MULTIPLE_OVERLAYS: expected one owned selection overlay, found $($overlays.Count)." }
        if($overlays.Count -eq 1) {
            $overlay=$overlays[0]
            if($clock.Elapsed -lt $expected.Subtract($earlyTolerance)) {
                throw "DELAY_TOO_EARLY: $($Case.Seconds)s menu item opened the overlay after $([Math]::Round($clock.Elapsed.TotalMilliseconds)) ms."
            }
            break
        }
        Start-Sleep -Milliseconds 25
    }
    if($null -eq $overlay) {
        throw "DELAY_OVERLAY_TIMEOUT: $($Case.Seconds)s menu item did not open an owned selection overlay within $([Math]::Round($latest.TotalSeconds,1))s. $menuEvidence"
    }
    if($overlay.Pid -ne $process.Id -or [ShotKitDelayedCaptureSmoke.Native]::OwnerPid($overlay.Handle) -ne $process.Id) {
        throw "OVERLAY_OWNER_MISMATCH: HWND=$($overlay.Handle) window PID=$($overlay.Pid), API PID=$([ShotKitDelayedCaptureSmoke.Native]::OwnerPid($overlay.Handle)), expected $($process.Id)."
    }
    if([ShotKitDelayedCaptureSmoke.Native]::GetForegroundWindow() -ne $overlay.Handle) {
        if(-not [ShotKitDelayedCaptureSmoke.Native]::SetForegroundWindow($overlay.Handle) -or
            [ShotKitDelayedCaptureSmoke.Native]::GetForegroundWindow() -ne $overlay.Handle) {
            throw "OVERLAY_FOCUS_FAILED: cannot confirm test-owned overlay foreground; HWND=$($overlay.Handle), foreground=$([ShotKitDelayedCaptureSmoke.Native]::GetForegroundWindow())."
        }
    }
    $elapsedMs=[Math]::Round($clock.Elapsed.TotalMilliseconds)
    [ShotKitDelayedCaptureSmoke.Native]::keybd_event(0x1B,0,0,[UIntPtr]::Zero)
    $script:heldKey=$true
    Start-Sleep -Milliseconds 60
    [ShotKitDelayedCaptureSmoke.Native]::keybd_event(0x1B,0,0x0002,[UIntPtr]::Zero)
    $script:heldKey=$false
    $closeDeadline=[DateTime]::UtcNow.AddSeconds(8)
    while([DateTime]::UtcNow -lt $closeDeadline -and [ShotKitDelayedCaptureSmoke.Native]::IsWindowVisible($overlay.Handle)) { Start-Sleep -Milliseconds 50 }
    if([ShotKitDelayedCaptureSmoke.Native]::IsWindowVisible($overlay.Handle)) {
        throw "OVERLAY_CANCEL_TIMEOUT: Escape did not dismiss test-owned overlay HWND=$($overlay.Handle), title='$($overlay.Title)'."
    }
    # Allow the modal capture call and its ticket guard to unwind before the next request.
    Start-Sleep -Milliseconds 200
    if(@(Get-OwnedDialogs).Count -gt 0) { throw "UNEXPECTED_DIALOG_AFTER_CANCEL: $((Get-OwnedDialogs | ForEach-Object Title) -join '; ')" }
    Write-Host "PASS: $menuEvidence; owned ShotKitSelectionOverlay appeared after ${elapsedMs}ms for requested $($Case.Seconds)s and closed on Escape."
}

try {
    New-Item -ItemType Directory -Path $testRoot,$appData,$localAppData,$settingsDir -Force | Out-Null
    $settings=@{
        version=1; save_folder=$null; open_editor=$false
        copy_to_clipboard=$false; auto_save=$false; scroll_direction='down'
    } | ConvertTo-Json -Compress
    Set-Content -LiteralPath (Join-Path $settingsDir 'settings.json') -Value $settings -Encoding utf8NoBOM
    try {
        $env:APPDATA=$appData; $env:LOCALAPPDATA=$localAppData
        $process=Start-Process -FilePath $ExePath -WorkingDirectory $testRoot -PassThru
    } finally { $env:APPDATA=$oldAppData; $env:LOCALAPPDATA=$oldLocalAppData }

    $main=Wait-For -Seconds $StartupTimeoutSeconds -Description 'owned ShotKit main window' -Probe {
        Get-OwnedWindows | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' } | Select-Object -First 1
    }
    if($main.Pid -ne $process.Id -or [ShotKitDelayedCaptureSmoke.Native]::OwnerPid($main.Handle) -ne $process.Id) {
        throw "MAIN_WINDOW_OWNER_MISMATCH: HWND=$($main.Handle), window PID=$($main.Pid), expected $($process.Id)."
    }
    Assert-NoStartupDialogs -TimeoutSeconds $StartupTimeoutSeconds -OwnedProcess $process -GetWindows { Get-OwnedWindows }
    foreach($case in $delayCases) { Invoke-DelayCase $case }
    Write-Host "PASS: all 3/5/10-second delayed tray selections opened the owned overlay in the measured interval and were safely cancelled on input desktop '$inputDesktop'."
} catch { $failure=$_ }
finally {
    if($heldKey) {
        try { [ShotKitDelayedCaptureSmoke.Native]::keybd_event(0x1B,0,0x0002,[UIntPtr]::Zero) }
        catch { if(-not $failure) { $failure="Could not release Escape: $($_.Exception.Message)" } }
    }
    if($null -ne $process -and -not $process.HasExited) {
        try {
            $owned=@([ShotKitDelayedCaptureSmoke.Native]::Windows($process.Id))
            foreach($window in $owned) { [void][ShotKitDelayedCaptureSmoke.Native]::PostMessageW($window.Handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) }
            if(-not $process.WaitForExit(5000)) { $process.Kill(); [void]$process.WaitForExit(5000) }
        } catch { if(-not $failure) { $failure="Could not stop owned ShotKit process: $($_.Exception.Message)" } }
        $process.Dispose()
    }
    $env:APPDATA=$oldAppData; $env:LOCALAPPDATA=$oldLocalAppData
    if(Test-Path -LiteralPath $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue }
}
if($failure) { throw $failure }

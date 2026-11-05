[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') {
    throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'
}
if ($env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'NO_INTERACTIVE_DESKTOP: opening and verifying an Explorer folder window requires an interactive Windows desktop.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$appName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $appName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: '$appName' is already running; refusing to interact with it."
}

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitStorageFolderSmoke {
    public sealed class WindowInfo { public IntPtr Handle; public int Pid; public string ClassName; public string Title; public bool Visible; }
    public sealed class ButtonInfo { public IntPtr Handle; public int Id; public string Text; }
    public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }
    public static class Native {
        private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc cb, IntPtr data);
        [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll")] private static extern int GetMenuItemCount(IntPtr menu);
        [DllImport("user32.dll")] private static extern uint GetMenuItemID(IntPtr menu, uint position);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern uint GetMenuStringW(IntPtr menu, uint position, StringBuilder text, int max, uint flags);
        [DllImport("user32.dll")] private static extern bool GetMenuItemRect(IntPtr hwnd, IntPtr menu, uint position, out Rect rect);
        [DllImport("user32.dll")] private static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] private static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);

        private static string Text(IntPtr hwnd, bool className) {
            var b = new StringBuilder(className ? 256 : 1024);
            if (className) GetClassNameW(hwnd, b, b.Capacity); else GetWindowTextW(hwnd, b, b.Capacity);
            return b.ToString();
        }
        public static WindowInfo[] Windows(int wantedPid) {
            var result = new List<WindowInfo>();
            EnumProc cb = delegate(IntPtr h, IntPtr d) {
                uint pid; GetWindowThreadProcessId(h, out pid);
                if (wantedPid == 0 || pid == (uint)wantedPid) result.Add(new WindowInfo {
                    Handle=h, Pid=(int)pid, ClassName=Text(h,true), Title=Text(h,false), Visible=IsWindowVisible(h)
                });
                return true;
            };
            EnumWindows(cb, IntPtr.Zero); GC.KeepAlive(cb); return result.ToArray();
        }
        public static ButtonInfo[] Buttons(IntPtr parent) {
            var result = new List<ButtonInfo>();
            EnumProc cb = delegate(IntPtr h, IntPtr d) {
                if (String.Equals(Text(h,true), "Button", StringComparison.OrdinalIgnoreCase))
                    result.Add(new ButtonInfo { Handle=h, Id=GetDlgCtrlID(h), Text=Text(h,false) });
                return true;
            };
            EnumChildWindows(parent, cb, IntPtr.Zero); GC.KeepAlive(cb); return result.ToArray();
        }
        public static uint OwnerPid(IntPtr hwnd) { uint pid; GetWindowThreadProcessId(hwnd, out pid); return pid; }
        public static string WindowClass(IntPtr hwnd) { return Text(hwnd,true); }
        public static string ClickStorageMenuItem(IntPtr popup, uint expectedId, string expectedText) {
            const uint MN_GETHMENU=0x01E1, MF_BYPOSITION=0x0400;
            IntPtr menu=SendMessageW(popup,MN_GETHMENU,IntPtr.Zero,IntPtr.Zero);
            if (menu==IntPtr.Zero) throw new InvalidOperationException("MN_GETHMENU returned a null HMENU.");
            int count=GetMenuItemCount(menu), match=-1;
            if (count<0) throw new InvalidOperationException("GetMenuItemCount failed.");
            var details=new StringBuilder();
            for(uint i=0;i<(uint)count;i++) {
                uint id=GetMenuItemID(menu,i); var label=new StringBuilder(512);
                GetMenuStringW(menu,i,label,label.Capacity,MF_BYPOSITION);
                details.Append(" [").Append(id).Append(":'").Append(label).Append("']");
                if(id==expectedId && String.Equals(label.ToString(),expectedText,StringComparison.Ordinal)) {
                    if(match>=0) throw new InvalidOperationException("Target menu item is duplicated.");
                    match=(int)i;
                }
            }
            if(match<0) throw new InvalidOperationException("Exact storage menu item missing; entries="+details);
            Rect rect;
            if(!GetMenuItemRect(IntPtr.Zero,menu,(uint)match,out rect) || rect.Right<=rect.Left || rect.Bottom<=rect.Top)
                throw new InvalidOperationException("GetMenuItemRect failed for the verified storage menu item.");
            int x=rect.Left+(rect.Right-rect.Left)/2, y=rect.Top+(rect.Bottom-rect.Top)/2;
            if(!SetCursorPos(x,y)) throw new InvalidOperationException("SetCursorPos failed for the verified menu item.");
            mouse_event(0x0002,0,0,0,UIntPtr.Zero); mouse_event(0x0004,0,0,0,UIntPtr.Zero);
            return "HMENU item id="+expectedId+" text='"+expectedText+"' rect="+rect.Left+","+rect.Top+","+rect.Right+","+rect.Bottom;
        }
    }
}
'@

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Storage-Folder-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $testRoot 'appdata'
$localAppData = Join-Path $testRoot 'localappdata'
$saveFolder = Join-Path $testRoot 'captures'
$inputPath = Join-Path $testRoot 'fixture.png'
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$process = $null
$baselineExplorer = @()
$openedExplorer = $null
$failure = $null
$cleanupFailure = $null
$preserveTestRoot = $false

function Normalize-FolderPath([string] $Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $root = [IO.Path]::GetPathRoot($full)
    if ($full.Length -gt $root.Length) { return $full.TrimEnd([char[]]@('\','/')) }
    return $full
}
function Get-ExplorerFolderWindows {
    $shell = $null
    $collection = $null
    $result = [Collections.Generic.List[object]]::new()
    try {
        $shell = New-Object -ComObject Shell.Application
        $collection = $shell.Windows()
        foreach ($window in $collection) {
            try {
                $fullName = [string]$window.FullName
                if ([IO.Path]::GetFileName($fullName) -ine 'explorer.exe') { continue }
                $url = [string]$window.LocationURL
                if ([string]::IsNullOrWhiteSpace($url)) { continue }
                $uri = [Uri]$url
                if (-not $uri.IsFile) { continue }
                $hwnd = [IntPtr]([long]$window.HWND)
                $result.Add([pscustomobject]@{
                    Handle=$hwnd; Url=$url; Path=(Normalize-FolderPath $uri.LocalPath)
                    Pid=[ShotKitStorageFolderSmoke.Native]::OwnerPid($hwnd)
                    ClassName=[ShotKitStorageFolderSmoke.Native]::WindowClass($hwnd)
                })
            } catch {
                throw "Could not inspect ShellWindows folder entry: $($_.Exception.Message)"
            }
        }
        return @($result.ToArray())
    } finally {
        if ($null -ne $collection -and [Runtime.InteropServices.Marshal]::IsComObject($collection)) {
            [void][Runtime.InteropServices.Marshal]::ReleaseComObject($collection)
        }
        if ($null -ne $shell -and [Runtime.InteropServices.Marshal]::IsComObject($shell)) {
            [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell)
        }
    }
}
function Wait-For([scriptblock] $Probe, [string] $Description, [int] $Seconds = 10) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    do {
        if ($process -and $process.HasExited) { throw "PROCESS_EXITED while waiting for $Description (code $($process.ExitCode))." }
        $value = & $Probe
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "TIMEOUT waiting for $Description."
}

try {
    # Closing or navigating an existing Explorer window is never permitted.
    $baselineExplorer = @(Get-ExplorerFolderWindows)
    if ($baselineExplorer.Count -gt 0) {
        $details = ($baselineExplorer | ForEach-Object { "HWND=$($_.Handle) PID=$($_.Pid) path='$($_.Path)'" }) -join '; '
        throw "PREEXISTING_EXPLORER_WINDOWS: close all Explorer folder windows before this isolated smoke; found $details"
    }

    New-Item -ItemType Directory -Path $appData, $localAppData, $saveFolder -Force | Out-Null
    $bitmap = [Drawing.Bitmap]::new(24, 24, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try { $bitmap.SetPixel(8, 8, [Drawing.Color]::Red); $bitmap.Save($inputPath, [Drawing.Imaging.ImageFormat]::Png) }
    finally { $bitmap.Dispose() }
    $settings = @{
        version=1; save_folder=$saveFolder; open_editor=$true
        copy_to_clipboard=$false; auto_save=$false; scroll_direction='down'
    } | ConvertTo-Json -Compress
    $settingsDir = Join-Path $appData 'ShotKitRust'
    New-Item -ItemType Directory -Path $settingsDir -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $settingsDir 'settings.json') -Value $settings -Encoding utf8NoBOM

    try {
        $env:APPDATA = $appData
        $env:LOCALAPPDATA = $localAppData
        $quotedInput = '"{0}"' -f $inputPath
        $process = Start-Process -FilePath $ExePath -ArgumentList $quotedInput -WorkingDirectory $testRoot -PassThru
    } finally { $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData }

    $main = Wait-For -Seconds $TimeoutSeconds -Description 'owned editor window' -Probe {
        [ShotKitStorageFolderSmoke.Native]::Windows($process.Id) |
            Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' -and $_.Visible } | Select-Object -First 1
    }
    $welcome = Wait-For -Seconds $TimeoutSeconds -Description 'owned startup notice' -Probe {
        [ShotKitStorageFolderSmoke.Native]::Windows($process.Id) |
            Where-Object { $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*' -and $_.Visible } | Select-Object -First 1
    }
    if (-not [ShotKitStorageFolderSmoke.Native]::PostMessageW($welcome.Handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)) {
        throw 'STARTUP_DIALOG_CLOSE_FAILED: could not close the test-owned ShotKit startup notice.'
    }
    $main = Wait-For -Seconds 10 -Description 'loaded editor window' -Probe {
        [ShotKitStorageFolderSmoke.Native]::Windows($process.Id) |
            Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' -and $_.Title -like '*Strg+S speichern*' } | Select-Object -First 1
    }

    # Ensure no folder window appeared concurrently before issuing the UI action.
    $beforeClick = @(Get-ExplorerFolderWindows)
    if ($beforeClick.Count -gt 0) {
        $details = ($beforeClick | ForEach-Object { "HWND=$($_.Handle) PID=$($_.Pid) path='$($_.Path)'" }) -join '; '
        throw "EXPLORER_WINDOW_RACE: Explorer windows appeared before the click; refusing to use or close them: $details"
    }
    $fileButton = [ShotKitStorageFolderSmoke.Native]::Buttons($main.Handle) |
        Where-Object { $_.Id -eq 0x5131 } | Select-Object -First 1
    if ($null -eq $fileButton) { throw 'FILE_MENU_BUTTON_MISSING: editor child button ID 0x5131 was not found.' }
    [ShotKitStorageFolderSmoke.Native]::Click($fileButton.Handle)
    $popup = Wait-For -Seconds 8 -Description 'owned native File popup' -Probe {
        [ShotKitStorageFolderSmoke.Native]::Windows($process.Id) |
            Where-Object { $_.ClassName -eq '#32768' -and $_.Visible } | Select-Object -First 1
    }
    $menuClick = [ShotKitStorageFolderSmoke.Native]::ClickStorageMenuItem($popup.Handle,0x5143,'Speicherordner öffnen')

    $targetPath = Normalize-FolderPath $saveFolder
    $opened = Wait-For -Seconds $TimeoutSeconds -Description "new Explorer window at exact test path '$targetPath'" -Probe {
        $all = @(Get-ExplorerFolderWindows)
        $matches = @($all | Where-Object { [string]::Equals($_.Path,$targetPath,[StringComparison]::OrdinalIgnoreCase) })
        if ($matches.Count -gt 1) {
            $script:preserveTestRoot = $true
            throw "AMBIGUOUS_TEST_FOLDER_WINDOWS: $($matches.Count) ShellWindows entries point at '$targetPath'; preserving the directory because no single window can be safely closed."
        }
        if ($matches.Count -eq 1) {
            if (@($baselineExplorer | Where-Object { $_.Handle -eq $matches[0].Handle }).Count -gt 0) {
                throw "REUSED_BASELINE_EXPLORER_WINDOW: target path appeared in preexisting HWND $($matches[0].Handle); refusing to close it."
            }
            return $matches[0]
        }
        return $null
    }
    $openedExplorer = $opened
    if ($opened.Pid -le 0 -or $opened.ClassName -notin @('CabinetWClass','ExploreWClass')) {
        throw "EXPLORER_WINDOW_IDENTITY_INVALID: target=$targetPath HWND=$($opened.Handle) PID=$($opened.Pid) class='$($opened.ClassName)'."
    }
    Write-Host "OBSERVED: ShellWindows HWND=$($opened.Handle) PID=$($opened.Pid) class=$($opened.ClassName) URL='$($opened.Url)' path='$($opened.Path)'"

    # Close only the new exact-path folder window, and only while it still
    # resolves to this test's unique directory. Never terminate explorer.exe.
    $current = @(Get-ExplorerFolderWindows | Where-Object { $_.Handle -eq $openedExplorer.Handle })
    if ($current.Count -ne 1 -or
        -not [string]::Equals($current[0].Path,$targetPath,[StringComparison]::OrdinalIgnoreCase) -or
        $current[0].Pid -ne $openedExplorer.Pid -or
        $current[0].ClassName -notin @('CabinetWClass','ExploreWClass') -or
        @($baselineExplorer | Where-Object { $_.Handle -eq $openedExplorer.Handle }).Count -gt 0) {
        throw "EXPLORER_CLEANUP_IDENTITY_CHANGED: refusing to close HWND=$($openedExplorer.Handle); current identity no longer matches the new exact test folder."
    }
    if (-not [ShotKitStorageFolderSmoke.Native]::PostMessageW($openedExplorer.Handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)) {
        throw "EXPLORER_CLOSE_POST_FAILED: could not post WM_CLOSE to verified test folder HWND=$($openedExplorer.Handle)."
    }
    $closeDeadline = [DateTime]::UtcNow.AddSeconds(8)
    while ([DateTime]::UtcNow -lt $closeDeadline -and [ShotKitStorageFolderSmoke.Native]::IsWindow($openedExplorer.Handle)) { Start-Sleep -Milliseconds 100 }
    if ([ShotKitStorageFolderSmoke.Native]::IsWindow($openedExplorer.Handle)) {
        throw "EXPLORER_CLOSE_TIMEOUT: verified test folder HWND=$($openedExplorer.Handle) remained open."
    }
    $openedExplorer = $null
    Write-Host "PASS: $menuClick; ShellWindows matched exact isolated path '$targetPath'; only the newly created matching folder HWND was closed."
} catch { $failure = $_ }
finally {
    $env:APPDATA = $oldAppData
    $env:LOCALAPPDATA = $oldLocalAppData
    if ($null -eq $openedExplorer -and (Test-Path -LiteralPath $saveFolder)) {
        try {
            $targetPath = Normalize-FolderPath $saveFolder
            $lateMatches = @(Get-ExplorerFolderWindows | Where-Object {
                [string]::Equals($_.Path,$targetPath,[StringComparison]::OrdinalIgnoreCase)
            })
            if ($lateMatches.Count -eq 1 -and
                @($baselineExplorer | Where-Object { $_.Handle -eq $lateMatches[0].Handle }).Count -eq 0 -and
                $lateMatches[0].Pid -gt 0 -and $lateMatches[0].ClassName -in @('CabinetWClass','ExploreWClass')) {
                $openedExplorer = $lateMatches[0]
            } elseif ($lateMatches.Count -gt 0) {
                $preserveTestRoot = $true
                throw "Could not safely identify a single new test Explorer window during fallback cleanup; matches=$($lateMatches.Count)."
            }
        } catch { if (-not $cleanupFailure) { $cleanupFailure = $_ }; $preserveTestRoot = $true }
    }
    if ($null -ne $openedExplorer) {
        try {
            $targetPath = Normalize-FolderPath $saveFolder
            $current = @(Get-ExplorerFolderWindows | Where-Object { $_.Handle -eq $openedExplorer.Handle })
            $isBaseline = @($baselineExplorer | Where-Object { $_.Handle -eq $openedExplorer.Handle }).Count -gt 0
            if (-not $isBaseline -and $current.Count -eq 1 -and
                [string]::Equals($current[0].Path,$targetPath,[StringComparison]::OrdinalIgnoreCase) -and
                $current[0].Pid -eq $openedExplorer.Pid -and
                $current[0].ClassName -in @('CabinetWClass','ExploreWClass')) {
                [void][ShotKitStorageFolderSmoke.Native]::PostMessageW($openedExplorer.Handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)
                $deadline = [DateTime]::UtcNow.AddSeconds(8)
                while ([DateTime]::UtcNow -lt $deadline -and [ShotKitStorageFolderSmoke.Native]::IsWindow($openedExplorer.Handle)) { Start-Sleep -Milliseconds 100 }
                if ([ShotKitStorageFolderSmoke.Native]::IsWindow($openedExplorer.Handle)) { throw "Verified test Explorer window HWND=$($openedExplorer.Handle) did not close during cleanup." }
            } else {
                throw "Refused Explorer cleanup because HWND=$($openedExplorer.Handle) no longer uniquely identifies the new test folder window."
            }
        } catch { $cleanupFailure = $_; $preserveTestRoot = $true }
    }
    if ($process -and -not $process.HasExited) {
        try {
            foreach ($window in [ShotKitStorageFolderSmoke.Native]::Windows($process.Id)) {
                [void][ShotKitStorageFolderSmoke.Native]::PostMessageW($window.Handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)
            }
            if (-not $process.WaitForExit(5000)) { $process.Kill(); $process.WaitForExit() }
        } catch { if (-not $cleanupFailure) { $cleanupFailure = $_ } }
    }
    if (-not $preserveTestRoot -and (Test-Path -LiteralPath $testRoot)) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
    }
}
if ($cleanupFailure) { throw "CLEANUP_FAILED: $cleanupFailure" }
if ($failure) { throw $failure }

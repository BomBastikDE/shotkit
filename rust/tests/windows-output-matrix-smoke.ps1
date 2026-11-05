[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,
    [ValidateRange(10, 120)]
    [int] $TimeoutSeconds = 30,
    [ValidateRange(5, 180)]
    [int] $CaptureTimeoutSeconds = 45
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if ($env:OS -ne 'Windows_NT') { throw 'Windows is required.' }
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') { throw 'LOCAL_OPT_IN_REQUIRED: use run-all.ps1 -AllowLocalInteractive in a dedicated test session.' }
$processNames=@([IO.Path]::GetFileNameWithoutExtension($ExePath),'ShotKit','shotkit-rust-prototype') | Select-Object -Unique
foreach($name in $processNames){if(@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count -gt 0){throw "PREEXISTING_PROCESS: refusing to affect '$name'."}}
if (-not [Environment]::UserInteractive) { throw 'Desktop session unavailable: this smoke test requires the interactive Windows desktop.' }

if (-not ('ShotKitOutputMatrixSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitOutputMatrixSmoke {
    public sealed class WindowInfo { public IntPtr Handle; public string ClassName; public string Title; public bool Visible; }
    public struct Point { public int X; public int Y; public Point(int x, int y) { X=x; Y=y; } }
    public static class Native {
        private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
        private delegate bool EnumChildProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll", SetLastError=true)] static extern bool CloseDesktop(IntPtr desktop);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder text, uint length, out uint needed);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr data);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
        public static bool WindowVisible(IntPtr hwnd) { return IsWindowVisible(hwnd); }
        [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out UIntPtr result);
        public static bool UiResponsive(IntPtr hwnd) { UIntPtr result; return SendMessageTimeoutW(hwnd, 0, IntPtr.Zero, IntPtr.Zero, 2, 5000, out result)!=IntPtr.Zero; }
        public static bool CloseDialog(IntPtr hwnd) { return PostMessageW(hwnd,0x0010,IntPtr.Zero,IntPtr.Zero); }
        [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
        [DllImport("user32.dll", SetLastError=true)] static extern bool OpenClipboard(IntPtr owner);
        [DllImport("user32.dll", SetLastError=true)] static extern bool CloseClipboard();
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr GetClipboardData(uint format);
        [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr memory);
        [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr GlobalLock(IntPtr memory);
        [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr memory);
        public static uint ClipboardSequence() { return GetClipboardSequenceNumber(); }
        public static byte[] ReadClipboardDibV5() {
            // Another process may still be publishing the data after the sequence changes.
            if(!OpenClipboard(IntPtr.Zero))return null;
            IntPtr memory=IntPtr.Zero, ptr=IntPtr.Zero;
            try {
                memory=GetClipboardData(17); if(memory==IntPtr.Zero)throw new InvalidOperationException("CF_DIBV5 is absent.");
                ulong size=GlobalSize(memory).ToUInt64(); if(size<124 || size>67108864)throw new InvalidOperationException("CF_DIBV5 block size is invalid: "+size+".");
                ptr=GlobalLock(memory); if(ptr==IntPtr.Zero)throw new InvalidOperationException("GlobalLock failed for CF_DIBV5: "+Marshal.GetLastWin32Error());
                byte[] bytes=new byte[(int)size]; Marshal.Copy(ptr,bytes,0,bytes.Length); return bytes;
            } finally { if(ptr!=IntPtr.Zero)GlobalUnlock(memory); if(!CloseClipboard())throw new InvalidOperationException("CloseClipboard failed: "+Marshal.GetLastWin32Error()); }
        }
        public struct Rect { public int Left, Top, Right, Bottom; }
        [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr m);
        [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr m, int pos);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern uint GetMenuStringW(IntPtr m, uint pos, StringBuilder text, int max, uint flags);
        [DllImport("user32.dll")] static extern IntPtr GetSubMenu(IntPtr m, int pos);
        [DllImport("user32.dll")] static extern uint GetMenuState(IntPtr m, uint id, uint flags);
        [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr h, IntPtr m, uint pos, out Rect r);
        public static IntPtr PopupMenu(IntPtr popup) {
            UIntPtr result;
            if(SendMessageTimeoutW(popup,0x01e1,IntPtr.Zero,IntPtr.Zero,2,2000,out result)==IntPtr.Zero)
                throw new InvalidOperationException("Popup menu query failed or timed out.");
            return new IntPtr(unchecked((long)result.ToUInt64()));
        }
        static int FindItem(IntPtr m, string expected, uint? id) {
            int found=-1, count=GetMenuItemCount(m);
            if(count<0) throw new InvalidOperationException("Cannot inspect menu.");
            for(int i=0;i<count;i++) {
                var s=new StringBuilder(256); GetMenuStringW(m,(uint)i,s,s.Capacity,0x400);
                if(s.ToString()==expected && (!id.HasValue || GetMenuItemID(m,i)==id.Value)) {
                    if(found>=0) throw new InvalidOperationException("Duplicate menu entry: "+expected);
                    found=i;
                }
            }
            if(found<0) throw new InvalidOperationException("Missing menu entry: "+expected);
            return found;
        }
        static void MoveToItem(IntPtr popup, IntPtr m, int pos, int expectedPid) {
            uint pid; GetWindowThreadProcessId(popup,out pid);
            if(pid!=(uint)expectedPid || !IsWindow(popup) || !IsWindowVisible(popup) || PopupMenu(popup)!=m)
                throw new InvalidOperationException("Popup no longer identifies the expected visible menu.");
            Rect r; if(!GetMenuItemRect(IntPtr.Zero,m,(uint)pos,out r) || r.Right<=r.Left || r.Bottom<=r.Top)
                throw new InvalidOperationException("Menu entry has no visible screen rectangle.");
            int x=r.Left+(r.Right-r.Left)/2, y=r.Top+(r.Bottom-r.Top)/2;
            if(!SetCursorPos(x,y)) throw new InvalidOperationException("Cannot move cursor to menu entry.");
            if(WindowFromPoint(new Point(x,y))!=popup)
                throw new InvalidOperationException("Menu entry is obscured or no longer belongs to the expected popup.");
        }
        public static IntPtr OpenSettingsSubmenu(IntPtr popup, int expectedPid) {
            IntPtr m=PopupMenu(popup); if(m==IntPtr.Zero) throw new InvalidOperationException("No root menu.");
            int pos=FindItem(m,"Einstellungen",null); IntPtr sub=GetSubMenu(m,pos);
            if(sub==IntPtr.Zero) throw new InvalidOperationException("No settings submenu.");
            MoveToItem(popup,m,pos,expectedPid); return sub;
        }
        public static void ClickSetting(IntPtr popup, IntPtr expectedMenu, uint id, string label, bool checkedBefore, int expectedPid) {
            IntPtr m=PopupMenu(popup); if(m!=expectedMenu) throw new InvalidOperationException("Wrong submenu.");
            int pos=FindItem(m,label,id); uint state=GetMenuState(m,id,0);
            if(state==0xffffffff || ((state & 8)!=0)!=checkedBefore)
                throw new InvalidOperationException("Unexpected checked state for "+label+": "+state);
            MoveToItem(popup,m,pos,expectedPid);
            try { mouse_event(2,0,0,0,UIntPtr.Zero); }
            finally { mouse_event(4,0,0,0,UIntPtr.Zero); }
        }
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
        public static string InputDesktopName() {
            IntPtr d=OpenInputDesktop(0,false,1); if(d==IntPtr.Zero)return null;
            try { var s=new StringBuilder(256); uint n; return GetUserObjectInformationW(d,2,s,(uint)(s.Capacity*2),out n)?s.ToString():null; }
            finally { CloseDesktop(d); }
        }
        static string Text(IntPtr h) { var s=new StringBuilder(512); GetWindowTextW(h,s,s.Capacity); return s.ToString(); }
        public static string WindowText(IntPtr h) {
            var s=new StringBuilder(Text(h)); EnumChildWindows(h,delegate(IntPtr c,IntPtr d){string t=Text(c);if(!String.IsNullOrWhiteSpace(t))s.Append(" ").Append(t);return true;},IntPtr.Zero);return s.ToString();
        }
        public static WindowInfo[] Windows(int pid) {
            var list=new List<WindowInfo>(); EnumProc cb=delegate(IntPtr h,IntPtr d){uint p;GetWindowThreadProcessId(h,out p);if(p!=(uint)pid)return true;
                var c=new StringBuilder(256);GetClassNameW(h,c,c.Capacity);list.Add(new WindowInfo{Handle=h,ClassName=c.ToString(),Title=Text(h),Visible=IsWindowVisible(h)});return true;};
            EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return list.ToArray();
        }

    }
}
'@
}

$inputDesktop=[ShotKitOutputMatrixSmoke.Native]::InputDesktopName()
if($inputDesktop -cne 'Default'){throw "INTERACTIVE_DEFAULT_DESKTOP_REQUIRED: current input desktop is '$inputDesktop'."}
$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Output-Matrix-' + [Guid]::NewGuid().ToString('N'))

$surfaceScript=Join-Path $root 'surface.ps1'; $readyFile=Join-Path $root 'surface-ready.txt'
$surfaceStdout=Join-Path $root 'surface-stdout.txt'; $surfaceStderr=Join-Path $root 'surface-stderr.txt'
$shotKit=$null; $surface=$null; $oldAppData=$env:APPDATA; $oldLocalAppData=$env:LOCALAPPDATA; $failure=$null; $mouseDown=$false; $allExited=$true

function Get-OwnWindows([Diagnostics.Process] $p) { [ShotKitOutputMatrixSmoke.Native]::Windows($p.Id) }
function Wait-ForCondition([scriptblock] $Condition,[int] $Seconds,[string] $Description,[Diagnostics.Process] $OwnedProcess) {
    $deadline=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $deadline) {
        if($OwnedProcess.HasExited){throw "Owned process exited before $Description (code $($OwnedProcess.ExitCode))."}
        $unexpected=Get-OwnWindows $OwnedProcess | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -notlike '*Prototyp*' -and [ShotKitOutputMatrixSmoke.Native]::WindowText($_.Handle) -notlike '*Aufnahme gespeichert:*' -and [ShotKitOutputMatrixSmoke.Native]::WindowText($_.Handle) -notlike '*Aufnahme in die Zwischenablage kopiert*'} | Select-Object -First 1
        if($null -ne $unexpected){throw "Unexpected dialog '$($unexpected.Title)': $([ShotKitOutputMatrixSmoke.Native]::WindowText($unexpected.Handle))"}
        $v=& $Condition; if($null -ne $v -and $v){return $v}; Start-Sleep -Milliseconds 100
    }
    throw "Timed out waiting for $Description."
}
# This path intentionally reads the ephemeral runner's autostart status when the
# native menu is built; it never selects or writes the autostart entry.
function Set-TrayOutputOption([Diagnostics.Process] $Process, [IntPtr] $Main, [uint32] $Id, [string] $Label, [bool] $Before) {
    if ($env:GITHUB_ACTIONS -cne 'true') { throw 'TRAY_SETTINGS_CI_ONLY: refusing to open the menu outside GitHub Actions.' }
    if (-not [ShotKitOutputMatrixSmoke.Native]::PostMessageW($Main,0x8001,[IntPtr]1,[IntPtr]0x0205)) { throw 'Could not post tray callback.' }
    $popup=Wait-ForCondition -Seconds 10 -Description 'native tray menu' -OwnedProcess $Process -Condition {
        Get-OwnWindows $Process | Where-Object {$_.Visible -and $_.ClassName -eq '#32768'} | Select-Object -First 1
    }
    $submenu=[ShotKitOutputMatrixSmoke.Native]::OpenSettingsSubmenu($popup.Handle,$Process.Id)
    $child=Wait-ForCondition -Seconds 10 -Description 'native settings submenu' -OwnedProcess $Process -Condition {
        Get-OwnWindows $Process | Where-Object {$_.Visible -and $_.ClassName -eq '#32768' -and $_.Handle -ne $popup.Handle -and [ShotKitOutputMatrixSmoke.Native]::PopupMenu($_.Handle) -eq $submenu} | Select-Object -First 1
    }
    [ShotKitOutputMatrixSmoke.Native]::ClickSetting($child.Handle,$submenu,$Id,$Label,$Before,$Process.Id)
    [void](Wait-ForCondition -Seconds 10 -Description 'tray menu dismissal' -OwnedProcess $Process -Condition {-not [ShotKitOutputMatrixSmoke.Native]::WindowVisible($popup.Handle)})
}
function Assert-TraySettings([string] $Path, [bool] $Editor, [bool] $Copy, [bool] $Save, [string] $Folder) {
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    do {
        if($shotKit.HasExited){throw 'ShotKit exited while waiting for persisted settings.'}
        $raw=Get-Content -LiteralPath $Path -Raw
        $value=$raw | ConvertFrom-Json
        if ($value.version -eq 1 -and $value.open_editor -ceq $Editor -and $value.copy_to_clipboard -ceq $Copy -and $value.auto_save -ceq $Save -and $value.save_folder -ceq $Folder -and $value.scroll_direction -ceq 'down') { return }
        Start-Sleep -Milliseconds 100
    } while([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for expected persisted settings; last value: $raw"
}
function Send-Key([byte] $Key,[bool] $Down) { $flags=if($Down){0}else{2}; [ShotKitOutputMatrixSmoke.Native]::keybd_event($Key,0,$flags,[UIntPtr]::Zero) }
function Assert-PatternPng([IO.FileInfo] $File,[int] $Width,[int] $Height,[int] $SourceX,[int] $SourceY,[string] $Description) {
    $bytes=[IO.File]::ReadAllBytes($File.FullName)
    $pngSignature=[byte[]](137,80,78,71,13,10,26,10)
    if($bytes.Length -lt 8){throw "${Description}: file is too short for a PNG signature."}
    for($i=0;$i -lt $pngSignature.Length;$i++){if($bytes[$i] -ne $pngSignature[$i]){throw "${Description}: invalid PNG signature."}}
    $bitmap=[Drawing.Bitmap]::new($File.FullName)
    try {
        if($bitmap.RawFormat.Guid -ne [Drawing.Imaging.ImageFormat]::Png.Guid){throw "${Description}: decoded image is not PNG."}
        if($bitmap.Width -ne $Width -or $bitmap.Height -ne $Height){throw "$Description dimensions are $($bitmap.Width)x$($bitmap.Height); expected ${Width}x${Height}."}
        foreach($y in 0..($Height-1)) { foreach($x in @(0,37,[int]($Width/2),($Width-1))) {
            $sx=$SourceX+$x; $sy=$SourceY+$y
            $er=($sx*29+$sy*17+[int][Math]::Floor($sy/19)*53)%256
            $eg=($sx*7+$sy*31+[int][Math]::Floor($sy/23)*97)%256
            $eb=($sx*19+$sy*11+[int][Math]::Floor($sy/11)*41)%256
            $actual=$bitmap.GetPixel($x,$y)
            if($actual.R -ne $er -or $actual.G -ne $eg -or $actual.B -ne $eb -or $actual.A -ne 255){throw "$Description pixel ($x,$y), source ($sx,$sy): actual RGBA=$($actual.R),$($actual.G),$($actual.B),$($actual.A), expected RGBA=$er,$eg,$eb,255."}
        }}
    } finally {$bitmap.Dispose()}
}
function Assert-ClipboardDibV5([byte[]] $Dib,[int] $Width,[int] $Height,[int] $SourceX,[int] $SourceY) {
    if($Dib.Length -lt 124){throw "CF_DIBV5 block is too short: $($Dib.Length) bytes."}
    $header=[BitConverter]::ToUInt32($Dib,0); $w=[BitConverter]::ToInt32($Dib,4); $signedHeight=[BitConverter]::ToInt32($Dib,8)
    $planes=[BitConverter]::ToUInt16($Dib,12); $bpp=[BitConverter]::ToUInt16($Dib,14); $compression=[BitConverter]::ToUInt32($Dib,16)
    if($header -ne 124 -or $w -ne $Width -or [Math]::Abs([long]$signedHeight) -ne $Height -or $planes -ne 1 -or $bpp -ne 32 -or $compression -ne 3){
        throw "CF_DIBV5 header mismatch: header=$header size=${w}x${signedHeight} planes=$planes bpp=$bpp compression=$compression."
    }
    if([BitConverter]::ToUInt32($Dib,40) -ne 16711680 -or [BitConverter]::ToUInt32($Dib,44) -ne 65280 -or [BitConverter]::ToUInt32($Dib,48) -ne 255 -or [BitConverter]::ToUInt32($Dib,52) -ne 4278190080){throw 'CF_DIBV5 channel masks are unexpected.'}
    $stride=[long]$Width*4; if($Dib.Length -lt 124+$stride*$Height){throw 'CF_DIBV5 pixel data is truncated.'}
    foreach($y in @(0,1,37,74,112,148,149)){foreach($x in @(0,37,100,199)){
        $row=if($signedHeight -lt 0){$y}else{$Height-1-$y}; $offset=[int](124+$row*$stride+$x*4); $sx=$SourceX+$x; $sy=$SourceY+$y
        $er=($sx*29+$sy*17+[int][Math]::Floor($sy/19)*53)%256; $eg=($sx*7+$sy*31+[int][Math]::Floor($sy/23)*97)%256; $eb=($sx*19+$sy*11+[int][Math]::Floor($sy/11)*41)%256
        $blue=$Dib[$offset];$green=$Dib[$offset+1];$red=$Dib[$offset+2];$alpha=$Dib[$offset+3]
        if($red -ne $er -or $green -ne $eg -or $blue -ne $eb -or $alpha -ne 255){throw "Clipboard pixel ($x,$y) mismatch: got RGBA=$red,$green,$blue,$alpha expected RGBA=$er,$eg,$eb,255."}
    }}
}
function Wait-ClipboardDibV5([uint32] $PreviousSequence,[Diagnostics.Process] $Process,[string] $Description) {
    $deadline=[DateTime]::UtcNow.AddSeconds($CaptureTimeoutSeconds)
    while([DateTime]::UtcNow -lt $deadline) {
        if($Process.HasExited){throw "Owned process exited before $Description."}
        if([ShotKitOutputMatrixSmoke.Native]::ClipboardSequence() -ne $PreviousSequence) {
            # Only OpenClipboard contention is retried. Missing formats, malformed
            # data, and pixel mismatches after a complete read are hard failures.
            $dib=[ShotKitOutputMatrixSmoke.Native]::ReadClipboardDibV5()
            if($null -ne $dib){return ,$dib}
        }
        Start-Sleep -Milliseconds 100
    }
    throw "Timed out waiting for a complete clipboard read: $Description."
}

try {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    Add-Type -AssemblyName System.Drawing
    $surfaceSource=@'
param([string]$ReadyFile)
$ErrorActionPreference='Stop'
Write-Output 'FIXTURE_STAGE: loading assemblies'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Write-Output 'FIXTURE_STAGE: compiling surface'
Add-Type -ReferencedAssemblies 'System.Windows.Forms.dll','System.Drawing.dll' -TypeDefinition @"
using System;
using System.Drawing;
using System.Windows.Forms;
using System.IO;
using System.Runtime.InteropServices;
namespace ShotKitOutputMatrixSurface {
 public sealed class Surface : Form {
  Bitmap image; string ready;
  [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  public Surface(string path) {
   try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch {}
   ready=path; Text="ShotKit Output Matrix Fixture"; StartPosition=FormStartPosition.Manual; Location=new Point(20,20);
   ClientSize=new Size(900,650); FormBorderStyle=FormBorderStyle.FixedSingle; MaximizeBox=false; MinimizeBox=false; ShowInTaskbar=false; DoubleBuffered=true; TopMost=true;
   image=new Bitmap(900,650);
   for(int y=0;y<650;y++) for(int x=0;x<900;x++) image.SetPixel(x,y,Color.FromArgb((x*29+y*17+(y/19)*53)&255,(x*7+y*31+(y/23)*97)&255,(x*19+y*11+(y/11)*41)&255));
  }
  protected override void OnShown(EventArgs e){base.OnShown(e);Point p=PointToScreen(Point.Empty);File.WriteAllText(ready,String.Format("{0},{1},{2},{3}",p.X,p.Y,ClientSize.Width,ClientSize.Height));}
  protected override void OnPaint(PaintEventArgs e){e.Graphics.DrawImageUnscaled(image,0,0);}
  protected override void Dispose(bool d){if(d&&image!=null)image.Dispose();base.Dispose(d);}
 }
}
"@
Write-Output 'FIXTURE_STAGE: constructing surface and entering message loop'
[System.Windows.Forms.Application]::Run([ShotKitOutputMatrixSurface.Surface]::new($ReadyFile))
'@
    [IO.File]::WriteAllText($surfaceScript,$surfaceSource,[Text.UTF8Encoding]::new($false))
    $powershellExe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (-not (Test-Path -LiteralPath $powershellExe -PathType Leaf)){throw 'Windows PowerShell 5.1 is required for the isolated fixture window.'}
    $surfaceArgs='-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -ReadyFile "{1}"' -f $surfaceScript,$readyFile
    $surface=Start-Process -FilePath $powershellExe -ArgumentList $surfaceArgs -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput $surfaceStdout -RedirectStandardError $surfaceStderr -PassThru
    try {
        $fixtureWindow=Wait-ForCondition -Seconds 20 -Description 'the deterministic fixture surface' -OwnedProcess $surface -Condition {
            [ShotKitOutputMatrixSmoke.Native]::Windows($surface.Id) | Where-Object {$_.Title -eq 'ShotKit Output Matrix Fixture' -and $_.Visible} | Select-Object -First 1
        }
    } catch {
        $startupFailure=$_
        try {
            $windows=@([ShotKitOutputMatrixSmoke.Native]::Windows($surface.Id) | ForEach-Object {"handle=$($_.Handle) class='$($_.ClassName)' title='$($_.Title)' visible=$($_.Visible)"}) -join '; '
            Write-Output "FIXTURE_DIAGNOSTIC: pid=$($surface.Id) exited=$($surface.HasExited) readyFile=$(Test-Path -LiteralPath $readyFile) windows=[$windows]"
        } catch { Write-Output 'FIXTURE_DIAGNOSTIC: window inspection failed.' }
        throw $startupFailure
    }
    $ready=Wait-ForCondition -Seconds 10 -Description 'complete fixture client coordinates' -OwnedProcess $surface -Condition {
        if(Test-Path -LiteralPath $readyFile){
            $value=Get-Content -LiteralPath $readyFile -Raw
            if($value -match '^\s*-?\d+,-?\d+,900,650\s*$'){$value}
        }
    }
    $g=$ready.Trim().Split(',') | ForEach-Object {[int]$_}
    if($g.Count -ne 4 -or $g[2] -ne 900 -or $g[3] -ne 650){throw "Unexpected fixture geometry '$ready'."}

    # Fresh settings, directories and app process per case; desktop and fixture are shared serially.
    foreach ($openEditor in @($false,$true)) { foreach ($copy in @($false,$true)) { foreach ($save in @($false,$true)) {
    $caseName="editor=$openEditor copy=$copy save=$save"
    $caseRoot=Join-Path $root ("e{0}-c{1}-s{2}" -f [int]$openEditor,[int]$copy,[int]$save)
    $appData=Join-Path $caseRoot 'appdata'; $localAppData=Join-Path $caseRoot 'localappdata'; $saveFolder=Join-Path $caseRoot 'captures'
    New-Item -ItemType Directory -Path $appData,$localAppData,$saveFolder -Force | Out-Null
    $settingsPath=Join-Path (Join-Path $appData 'ShotKitRust') 'settings.json'
    New-Item -ItemType Directory -Path (Split-Path -Parent $settingsPath) -Force | Out-Null
    # One matrix case derives every output option from actual native tray clicks.
    # Local opt-in matrix runs remain unchanged; the tray path is CI-only.
    $trayCase=($env:GITHUB_ACTIONS -ceq 'true' -and -not $openEditor -and -not $copy -and $save)
    $initialEditor=if($trayCase){-not $openEditor}else{$openEditor}
    $initialCopy=if($trayCase){-not $copy}else{$copy}
    $initialSave=if($trayCase){-not $save}else{$save}
    $settings=@{version=1;save_folder=$saveFolder;open_editor=$initialEditor;copy_to_clipboard=$initialCopy;auto_save=$initialSave;scroll_direction='down'} | ConvertTo-Json
    [IO.File]::WriteAllText($settingsPath,$settings,[Text.UTF8Encoding]::new($false))
    try {$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$shotKit=Start-Process -FilePath $ExePath -WorkingDirectory $caseRoot -PassThru}
    finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    [void](Wait-ForCondition -Seconds $TimeoutSeconds -Description 'ShotKit main window' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustPrototype'} | Select-Object -First 1})
    $welcome=Wait-ForCondition -Seconds $TimeoutSeconds -Description 'ShotKit startup dialog' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1}
    if (-not [ShotKitOutputMatrixSmoke.Native]::PostMessageW($welcome.Handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'Could not close the owned startup dialog.'}
    [void](Wait-ForCondition -Seconds 8 -Description 'startup dialog to close' -OwnedProcess $shotKit -Condition {-not [ShotKitOutputMatrixSmoke.Native]::IsWindow($welcome.Handle)})
    if($trayCase) {
        $owner=Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustPrototype'} | Select-Object -First 1
        if($null -eq $owner){throw 'ShotKit owner window missing.'}
        Set-TrayOutputOption $shotKit $owner.Handle 101 'Nach Aufnahme Editor öffnen' $true
        Assert-TraySettings $settingsPath $false $true $false $saveFolder
        Set-TrayOutputOption $shotKit $owner.Handle 102 'In Zwischenablage kopieren' $true
        Assert-TraySettings $settingsPath $false $false $false $saveFolder
        Set-TrayOutputOption $shotKit $owner.Handle 103 'Automatisch als PNG speichern' $false
        Assert-TraySettings $settingsPath $false $false $true $saveFolder
        $shotKit.Kill()
        if(-not $shotKit.WaitForExit(5000) -or -not $shotKit.HasExited){throw 'ShotKit did not exit before persistence restart.'}
        $shotKit.Dispose(); $shotKit=$null
        try {$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$shotKit=Start-Process -FilePath $ExePath -WorkingDirectory $caseRoot -PassThru}
        finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
        [void](Wait-ForCondition -Seconds $TimeoutSeconds -Description 'restarted ShotKit main window' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustPrototype'} | Select-Object -First 1})
        $welcome=Wait-ForCondition -Seconds $TimeoutSeconds -Description 'restarted ShotKit startup dialog' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1}
        if(-not [ShotKitOutputMatrixSmoke.Native]::PostMessageW($welcome.Handle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'Could not close restarted startup dialog.'}
        [void](Wait-ForCondition -Seconds 8 -Description 'restarted startup dialog to close' -OwnedProcess $shotKit -Condition {-not [ShotKitOutputMatrixSmoke.Native]::IsWindow($welcome.Handle)})
        Assert-TraySettings $settingsPath $false $false $true $saveFolder
        Write-Output 'PASS: native tray settings selected and persisted across process restart; checking capture effects next.'
    }

    [void][ShotKitOutputMatrixSmoke.Native]::SetForegroundWindow($fixtureWindow.Handle)
    [void](Wait-ForCondition -Seconds 5 -Description 'owned fixture foreground' -OwnedProcess $surface -Condition {[ShotKitOutputMatrixSmoke.Native]::GetForegroundWindow() -eq $fixtureWindow.Handle})

    # Assert the selected area belongs to our fixture just before triggering capture.
    # The dedicated desktop must remain free of unrelated windows throughout.
    foreach($x in @(80,180,279)){foreach($y in @(60,135,209)){
        $point=[ShotKitOutputMatrixSmoke.Point]::new(($g[0]+$x),($g[1]+$y))
        if([ShotKitOutputMatrixSmoke.Native]::WindowFromPoint($point) -ne $fixtureWindow.Handle){throw "Fixture is obscured at ($x,$y) for $caseName."}
    }}
    # A unique sentinel detects writes even when copying is disabled. This test
    # deliberately controls the clipboard and is limited to a dedicated desktop.
    $sentinel='ShotKit-output-matrix-' + [Guid]::NewGuid().ToString('N')
    Set-Clipboard -Value $sentinel
    if ((Get-Clipboard -Raw).TrimEnd("`r","`n") -cne $sentinel) { throw "Could not establish clipboard sentinel for $caseName." }
    $initialClipboardSequence=[ShotKitOutputMatrixSmoke.Native]::ClipboardSequence()
    if(-not [ShotKitOutputMatrixSmoke.Native]::SetCursorPos(($g[0]+80),($g[1]+60))){throw 'Could not position selection start.'}
    Send-Key 0x7A $true; try {} finally {Send-Key 0x7A $false}
    $overlay=Wait-ForCondition -Seconds 10 -Description 'the first area-selection overlay' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}
    [void][ShotKitOutputMatrixSmoke.Native]::SetForegroundWindow($overlay.Handle)
    [void](Wait-ForCondition -Seconds 5 -Description 'owned overlay foreground' -OwnedProcess $shotKit -Condition {[ShotKitOutputMatrixSmoke.Native]::GetForegroundWindow() -eq $overlay.Handle})
    if(-not [ShotKitOutputMatrixSmoke.Native]::SetCursorPos(($g[0]+80),($g[1]+60))){throw 'Could not position selection start.'}; Start-Sleep -Milliseconds 100
    [ShotKitOutputMatrixSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero); $mouseDown=$true
    try {
        if(-not [ShotKitOutputMatrixSmoke.Native]::SetCursorPos(($g[0]+280),($g[1]+210))){throw 'Could not position selection endpoint.'}
        Start-Sleep -Milliseconds 150
    } finally {
        [ShotKitOutputMatrixSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero); $mouseDown=$false
    }
    $expectEditor=$openEditor -or (-not $copy -and -not $save)
    if ($expectEditor) {
        $editor=Wait-ForCondition -Seconds $CaptureTimeoutSeconds -Description "editor for $caseName" -OwnedProcess $shotKit -Condition {
            Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible} | Select-Object -First 1
        }
    } else {
        $expectedText=if($save){'*Aufnahme gespeichert:*'}else{'*Aufnahme in die Zwischenablage kopiert*'}
        $dialog=Wait-ForCondition -Seconds $CaptureTimeoutSeconds -Description "output completion for $caseName" -OwnedProcess $shotKit -Condition {
            Get-OwnWindows $shotKit | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and [ShotKitOutputMatrixSmoke.Native]::WindowText($_.Handle) -like $expectedText} | Select-Object -First 1
        }
        if(-not [ShotKitOutputMatrixSmoke.Native]::CloseDialog($dialog.Handle)){throw "Could not close completion dialog for $caseName."}
        [void](Wait-ForCondition -Seconds 5 -Description 'completion dialog to close' -OwnedProcess $shotKit -Condition {-not [ShotKitOutputMatrixSmoke.Native]::WindowVisible($dialog.Handle)})
    }
    # Save and clipboard actions precede editor creation or the completion dialog.
    # A bounded WM_NULL confirms the UI thread responds; Win32 may dispatch sent
    # messages reentrantly, so this is not proof that handle_capture has returned.
    $owner=Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustPrototype'} | Select-Object -First 1
    if($null -eq $owner -or -not [ShotKitOutputMatrixSmoke.Native]::UiResponsive($owner.Handle)){throw "UI did not respond for $caseName."}
    if ($shotKit.HasExited) { throw "ShotKit exited unexpectedly for $caseName." }
    $unexpected=Get-OwnWindows $shotKit | Where-Object {$_.Visible -and $_.ClassName -eq '#32770'} | Select-Object -First 1
    if ($unexpected) { throw "Unexpected dialog for ${caseName}: $([ShotKitOutputMatrixSmoke.Native]::WindowText($unexpected.Handle))" }
    $editors=@(Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible})
    if ($editors.Count -ne [int]$expectEditor) { throw "Expected $([int]$expectEditor) editor(s) for $caseName, found $($editors.Count)." }
    $files=@(Get-ChildItem -LiteralPath $saveFolder -Force)
    if ($files.Count -ne [int]$save) { throw "Expected $([int]$save) output file(s) for $caseName, found $($files.Count)." }
    if ($save) {
        if ($files[0].PSIsContainer -or $files[0].Extension -cne '.png') { throw "Output is not PNG for $caseName." }
        Assert-PatternPng -File $files[0] -Width 200 -Height 150 -SourceX 80 -SourceY 60 -Description $caseName
    }
    $finalSequence=[ShotKitOutputMatrixSmoke.Native]::ClipboardSequence()
    if ($copy) {
        $captureDib=Wait-ClipboardDibV5 -PreviousSequence $initialClipboardSequence -Process $shotKit -Description "capture output for $caseName"
        Assert-ClipboardDibV5 -Dib $captureDib -Width 200 -Height 150 -SourceX 80 -SourceY 60
    } else {
        if ($finalSequence -ne $initialClipboardSequence -or (Get-Clipboard -Raw).TrimEnd("`r","`n") -cne $sentinel) { throw "Clipboard changed unexpectedly for $caseName." }
    }
    if ($expectEditor) {
        # Do this only after automatic clipboard checks: it intentionally writes
        # to the clipboard and independently verifies the editor's source image.
        if(-not [ShotKitOutputMatrixSmoke.Native]::SetForegroundWindow($editor.Handle)){throw "Could not foreground editor for $caseName."}
        [void](Wait-ForCondition -Seconds 5 -Description 'editor foreground' -OwnedProcess $shotKit -Condition {[ShotKitOutputMatrixSmoke.Native]::GetForegroundWindow() -eq $editor.Handle})
        $beforeEditorCopy=[ShotKitOutputMatrixSmoke.Native]::ClipboardSequence()
        Send-Key 0x11 $true
        try {
            Start-Sleep -Milliseconds 30
            Send-Key 0x43 $true
            try { Start-Sleep -Milliseconds 80 } finally { Send-Key 0x43 $false }
        } finally { Send-Key 0x11 $false }
        $editorDib=Wait-ClipboardDibV5 -PreviousSequence $beforeEditorCopy -Process $shotKit -Description "editor export for $caseName"
        Assert-ClipboardDibV5 -Dib $editorDib -Width 200 -Height 150 -SourceX 80 -SourceY 60
    }
    Write-Output "PASS: $caseName; file, clipboard, and editor effects checked on '$inputDesktop'."
    $shotKit.Kill()
    if (-not $shotKit.WaitForExit(5000) -or -not $shotKit.HasExited) { throw "ShotKit did not exit after $caseName." }
    $shotKit.Dispose(); $shotKit=$null
    } } }

}
catch {$failure=$_}
finally {
    if($mouseDown){[ShotKitOutputMatrixSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero);$mouseDown=$false}
    $env:APPDATA=$oldAppData; $env:LOCALAPPDATA=$oldLocalAppData
    foreach($owned in @($shotKit,$surface)) {
        if($null -ne $owned) {
            try {if (-not $owned.HasExited){$owned.Kill();if (-not $owned.WaitForExit(5000) -or -not $owned.HasExited){throw 'Owned process did not exit after termination request.'}}}
            catch {$allExited=$false;if (-not $failure){$failure=$_}}
            $owned.Dispose()
        }
    }
    if($failure -and $null -ne $surface) {
        foreach($log in @($surfaceStdout,$surfaceStderr)) {
            try {
                if(Test-Path -LiteralPath $log) {
                    $content=Get-Content -LiteralPath $log -Raw
                    if($env:GITHUB_ACTIONS -ceq 'true') {
                        Write-Output "FIXTURE_DIAGNOSTIC: $([IO.Path]::GetFileName($log)): $content"
                    } elseif($log -eq $surfaceStdout) {
                        $phases=@('FIXTURE_STAGE: loading assemblies','FIXTURE_STAGE: compiling surface','FIXTURE_STAGE: constructing surface and entering message loop')
                        foreach($line in @($content -split "`r?`n")) { if($line -cin $phases) { Write-Output $line } }
                    } else {
                        Write-Output "FIXTURE_DIAGNOSTIC: stderr nonempty=$(-not [string]::IsNullOrWhiteSpace($content))"
                    }
                }
            }
            catch { Write-Output "FIXTURE_DIAGNOSTIC: could not read $([IO.Path]::GetFileName($log))." }
        }
    }
    if($allExited -and (Test-Path -LiteralPath $root)){
        try {Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop}
        catch {if (-not $failure){$failure=$_}}
    }
}
if($failure){throw $failure}

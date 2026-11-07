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
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if ($env:OS -ne 'Windows_NT') { throw 'Windows is required.' }
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') { throw 'LOCAL_OPT_IN_REQUIRED: use run-all.ps1 -AllowLocalInteractive in a dedicated test session.' }
$processNames=@([IO.Path]::GetFileNameWithoutExtension($ExePath),'ShotKit','shotkit-rust-prototype') | Select-Object -Unique
foreach($name in $processNames){if(@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count -gt 0){throw "PREEXISTING_PROCESS: refusing to affect '$name'."}}
if (-not [Environment]::UserInteractive) { throw 'Desktop session unavailable: this smoke test requires the interactive Windows desktop.' }

if (-not ('ShotKitOutputFailureSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitOutputFailureSmoke {
    public sealed class WindowInfo { public IntPtr Handle; public string ClassName; public string Title; public bool Visible; }
    public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }
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
        public static bool CloseDialog(IntPtr hwnd) { return PostMessageW(hwnd,0x0010,IntPtr.Zero,IntPtr.Zero); }
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageW(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
        [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr menu, uint position);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern uint GetMenuStringW(IntPtr menu, uint position, StringBuilder text, int max, uint flags);
        [DllImport("user32.dll")] static extern int GetMenuState(IntPtr menu, uint item, uint flags);
        [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr hwnd, IntPtr menu, uint position, out Rect rect);
        [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
        [DllImport("user32.dll", SetLastError=true)] static extern bool OpenClipboard(IntPtr owner);
        [DllImport("user32.dll", SetLastError=true)] static extern bool CloseClipboard();
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr GetClipboardData(uint format);
        [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr memory);
        [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr GlobalLock(IntPtr memory);
        [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr memory);
        public static uint ClipboardSequence() { return GetClipboardSequenceNumber(); }
        public static byte[] ReadClipboardDibV5() {
            if(!OpenClipboard(IntPtr.Zero))throw new InvalidOperationException("OpenClipboard failed: "+Marshal.GetLastWin32Error());
            IntPtr memory=IntPtr.Zero, ptr=IntPtr.Zero;
            try {
                memory=GetClipboardData(17); if(memory==IntPtr.Zero)throw new InvalidOperationException("CF_DIBV5 is absent.");
                ulong size=GlobalSize(memory).ToUInt64(); if(size<124 || size>67108864)throw new InvalidOperationException("CF_DIBV5 block size is invalid: "+size+".");
                ptr=GlobalLock(memory); if(ptr==IntPtr.Zero)throw new InvalidOperationException("GlobalLock failed for CF_DIBV5: "+Marshal.GetLastWin32Error());
                byte[] bytes=new byte[(int)size]; Marshal.Copy(ptr,bytes,0,bytes.Length); return bytes;
            } finally { if(ptr!=IntPtr.Zero)GlobalUnlock(memory); if(!CloseClipboard())throw new InvalidOperationException("CloseClipboard failed: "+Marshal.GetLastWin32Error()); }
        }
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
        [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
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
        public static string VerifyAndClickLastRegion(IntPtr popup) {
            const uint MN_GETHMENU=0x01E1, MF_BYPOSITION=0x0400, MF_BYCOMMAND=0;
            const uint command=16; const string label="Letzten Bereich erneut aufnehmen";
            IntPtr menu=SendMessageW(popup,MN_GETHMENU,IntPtr.Zero,IntPtr.Zero);
            if(menu==IntPtr.Zero)throw new InvalidOperationException("MN_GETHMENU returned a null HMENU.");
            int count=GetMenuItemCount(menu); if(count<0)throw new InvalidOperationException("GetMenuItemCount failed.");
            int match=-1;
            for(uint i=0;i<(uint)count;i++) {
                uint id=GetMenuItemID(menu,i); var text=new StringBuilder(512); GetMenuStringW(menu,i,text,text.Capacity,MF_BYPOSITION);
                if(id==command && String.Equals(text.ToString(),label,StringComparison.Ordinal)) { if(match>=0)throw new InvalidOperationException("Duplicate last-region item.");match=(int)i; }
            }
            if(match<0)throw new InvalidOperationException("Visible tray popup lacks exact ID 16 item '"+label+"'.");
            int state=GetMenuState(menu,command,MF_BYCOMMAND);
            if(state<0 || (state&0x0003)!=0)throw new InvalidOperationException("Last-region menu item is disabled (or its state could not be read): "+state+".");
            Rect r; if(!GetMenuItemRect(IntPtr.Zero,menu,(uint)match,out r)||r.Right<=r.Left||r.Bottom<=r.Top)throw new InvalidOperationException("GetMenuItemRect failed for the verified last-region item.");
            int x=r.Left+(r.Right-r.Left)/2,y=r.Top+(r.Bottom-r.Top)/2;
            if(!SetCursorPos(x,y))throw new InvalidOperationException("Could not position cursor over the verified tray item.");
            mouse_event(0x0002,0,0,0,UIntPtr.Zero); mouse_event(0x0004,0,0,0,UIntPtr.Zero);
            return "native HMENU item '"+label+"' ID=16 rect="+r.Left+","+r.Top+","+r.Right+","+r.Bottom;
        }
    }
}
'@
}

$inputDesktop=[ShotKitOutputFailureSmoke.Native]::InputDesktopName()
if($inputDesktop -cne 'Default'){throw "INTERACTIVE_DEFAULT_DESKTOP_REQUIRED: current input desktop is '$inputDesktop'."}
$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Output-Failure-' + [Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata'; $localAppData=Join-Path $root 'localappdata'; $saveFolder=Join-Path $root 'blocked-save-folder'
$surfaceScript=Join-Path $root 'surface.ps1'; $readyFile=Join-Path $root 'surface-ready.txt'
$shotKit=$null; $surface=$null; $oldAppData=$env:APPDATA; $oldLocalAppData=$env:LOCALAPPDATA; $failure=$null; $mouseDown=$false; $allExited=$true

function Get-OwnWindows([Diagnostics.Process] $p) { [ShotKitOutputFailureSmoke.Native]::Windows($p.Id) }
function Wait-ForCondition([scriptblock] $Condition,[int] $Seconds,[string] $Description,[Diagnostics.Process] $OwnedProcess) {
    $deadline=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $deadline) {
        if($OwnedProcess.HasExited){throw "Owned process exited before $Description (code $($OwnedProcess.ExitCode))."}
        $unexpected=Get-OwnWindows $OwnedProcess | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and [ShotKitOutputFailureSmoke.Native]::WindowText($_.Handle) -notlike '*Ein Teil der Ausgabe ist fehlgeschlagen:*'} | Select-Object -First 1
        if($null -ne $unexpected){throw "Unexpected dialog '$($unexpected.Title)': $([ShotKitOutputFailureSmoke.Native]::WindowText($unexpected.Handle))"}
        $v=& $Condition; if($null -ne $v -and $v){return $v}; Start-Sleep -Milliseconds 100
    }
    throw "Timed out waiting for $Description."
}
function Send-Key([byte] $Key,[bool] $Down) { $flags=if($Down){0}else{2}; [ShotKitOutputFailureSmoke.Native]::keybd_event($Key,0,$flags,[UIntPtr]::Zero) }
function Close-ClipboardConfirmation([Diagnostics.Process] $Process) {
    $dialog=Get-OwnWindows $Process | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and [ShotKitOutputFailureSmoke.Native]::WindowText($_.Handle) -like '*Aufnahme in die Zwischenablage kopiert*'} | Select-Object -First 1
    if($null -ne $dialog){
        if(-not [ShotKitOutputFailureSmoke.Native]::CloseDialog($dialog.Handle)){throw 'Could not close the owned clipboard dialog.'}
        [void](Wait-ForCondition -Seconds 5 -Description 'clipboard confirmation dialog to close' -OwnedProcess $Process -Condition {if(-not [ShotKitOutputFailureSmoke.Native]::WindowVisible($dialog.Handle)){$true}})
    }
}
function Assert-PatternPng([IO.FileInfo] $File,[int] $Width,[int] $Height,[int] $SourceX,[int] $SourceY,[string] $Description) {
    $bitmap=[Drawing.Bitmap]::new($File.FullName)
    try {
        if($bitmap.Width -ne $Width -or $bitmap.Height -ne $Height){throw "$Description dimensions are $($bitmap.Width)x$($bitmap.Height); expected ${Width}x${Height}."}
        foreach($y in 0..($Height-1)) { foreach($x in @(0,37,[int]($Width/2),($Width-1))) {
            $sx=$SourceX+$x; $sy=$SourceY+$y
            $er=($sx*29+$sy*17+[int][Math]::Floor($sy/19)*53)%256
            $eg=($sx*7+$sy*31+[int][Math]::Floor($sy/23)*97)%256
            $eb=($sx*19+$sy*11+[int][Math]::Floor($sy/11)*41)%256
            $actual=$bitmap.GetPixel($x,$y)
            if($actual.R -ne $er -or $actual.G -ne $eg -or $actual.B -ne $eb){throw "$Description pixel ($x,$y), source ($sx,$sy): actual RGB=$($actual.R),$($actual.G),$($actual.B), expected RGB=$er,$eg,$eb."}
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
    if([BitConverter]::ToUInt32($Dib,40) -ne 16711680 -or [BitConverter]::ToUInt32($Dib,44) -ne 65280 -or [BitConverter]::ToUInt32($Dib,48) -ne 255){throw 'CF_DIBV5 channel masks are unexpected.'}
    $stride=[long]$Width*4; if($Dib.Length -lt 124+$stride*$Height){throw 'CF_DIBV5 pixel data is truncated.'}
    foreach($y in @(0,1,37,74,112,148,149)){foreach($x in @(0,37,100,199)){
        $row=if($signedHeight -lt 0){$y}else{$Height-1-$y}; $offset=[int](124+$row*$stride+$x*4); $sx=$SourceX+$x; $sy=$SourceY+$y
        $er=($sx*29+$sy*17+[int][Math]::Floor($sy/19)*53)%256; $eg=($sx*7+$sy*31+[int][Math]::Floor($sy/23)*97)%256; $eb=($sx*19+$sy*11+[int][Math]::Floor($sy/11)*41)%256
        $blue=$Dib[$offset];$green=$Dib[$offset+1];$red=$Dib[$offset+2]
        if($red -ne $er -or $green -ne $eg -or $blue -ne $eb){throw "Clipboard pixel ($x,$y) mismatch: got RGB=$red,$green,$blue expected RGB=$er,$eg,$eb."}
    }}
}

try {
    New-Item -ItemType Directory -Path $appData,$localAppData -Force | Out-Null
    $blockerBytes=[Text.Encoding]::ASCII.GetBytes('ShotKit output failure fixture: do not alter')
    [IO.File]::WriteAllBytes($saveFolder,$blockerBytes)
    $surfaceSource=@'
param([string]$ReadyFile)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies 'System.Windows.Forms.dll','System.Drawing.dll' -TypeDefinition @"
using System;
using System.Drawing;
using System.Windows.Forms;
using System.IO;
using System.Runtime.InteropServices;
namespace ShotKitOutputFailureSurface {
 public sealed class Surface : Form {
  Bitmap image; string ready;
  [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  public Surface(string path) {
   try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch {}
   ready=path; Text="ShotKit Output Failure Fixture"; StartPosition=FormStartPosition.Manual; Location=new Point(20,20);
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
[System.Windows.Forms.Application]::Run([ShotKitOutputFailureSurface.Surface]::new($ReadyFile))
'@
    [IO.File]::WriteAllText($surfaceScript,$surfaceSource,[Text.UTF8Encoding]::new($false))
    $powershellExe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (-not (Test-Path -LiteralPath $powershellExe -PathType Leaf)){throw 'Windows PowerShell 5.1 is required for the isolated fixture window.'}
    $surfaceArgs='-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -ReadyFile "{1}"' -f $surfaceScript,$readyFile
    $surface=Start-Process -FilePath $powershellExe -ArgumentList $surfaceArgs -WorkingDirectory $root -WindowStyle Hidden -PassThru
    $fixtureWindow=Wait-ForCondition -Seconds 20 -Description 'the deterministic fixture surface' -OwnedProcess $surface -Condition {
        [ShotKitOutputFailureSmoke.Native]::Windows($surface.Id) | Where-Object {$_.Title -eq 'ShotKit Output Failure Fixture' -and $_.Visible} | Select-Object -First 1
    }
    $ready=Wait-ForCondition -Seconds 10 -Description 'fixture client coordinates' -OwnedProcess $surface -Condition {if(Test-Path -LiteralPath $readyFile){Get-Content -LiteralPath $readyFile -Raw}}
    $g=$ready.Trim().Split(',') | ForEach-Object {[int]$_}
    if($g.Count -ne 4 -or $g[2] -ne 900 -or $g[3] -ne 650){throw "Unexpected fixture geometry '$ready'."}

    $settingsPath=Join-Path (Join-Path $appData 'ShotKitRust') 'settings.json'
    New-Item -ItemType Directory -Path (Split-Path -Parent $settingsPath) -Force | Out-Null
    $settings=@{version=1;save_folder=$saveFolder;open_editor=$false;copy_to_clipboard=$true;auto_save=$true;scroll_direction='down'} | ConvertTo-Json
    [IO.File]::WriteAllText($settingsPath,$settings,[Text.UTF8Encoding]::new($false))
    try {$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$shotKit=Start-Process -FilePath $ExePath -WorkingDirectory $root -PassThru}
    finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    [void](Wait-ForCondition -Seconds $TimeoutSeconds -Description 'ShotKit main window' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustPrototype'} | Select-Object -First 1})
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $shotKit -GetWindows { Get-OwnWindows $shotKit }
    [void][ShotKitOutputFailureSmoke.Native]::SetForegroundWindow($fixtureWindow.Handle)
    [void](Wait-ForCondition -Seconds 5 -Description 'owned fixture foreground' -OwnedProcess $surface -Condition {[ShotKitOutputFailureSmoke.Native]::GetForegroundWindow() -eq $fixtureWindow.Handle})

    # First successful manual selection establishes LAST_REGION and copies a
    # pixel-checked image from the known fixture coordinates (80,60)-(280,210).
    $initialClipboardSequence=[ShotKitOutputFailureSmoke.Native]::ClipboardSequence()
    if(-not [ShotKitOutputFailureSmoke.Native]::SetCursorPos(($g[0]+80),($g[1]+60))){throw 'Could not position selection start.'}
    Send-Key 0x7A $true; try {} finally {Send-Key 0x7A $false}
    $overlay=Wait-ForCondition -Seconds 10 -Description 'the first area-selection overlay' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}
    [void][ShotKitOutputFailureSmoke.Native]::SetForegroundWindow($overlay.Handle)
    [void](Wait-ForCondition -Seconds 5 -Description 'owned overlay foreground' -OwnedProcess $shotKit -Condition {[ShotKitOutputFailureSmoke.Native]::GetForegroundWindow() -eq $overlay.Handle})
    if(-not [ShotKitOutputFailureSmoke.Native]::SetCursorPos(($g[0]+80),($g[1]+60))){throw 'Could not position selection start.'}; Start-Sleep -Milliseconds 100
    [ShotKitOutputFailureSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero); $mouseDown=$true
    try {
        if(-not [ShotKitOutputFailureSmoke.Native]::SetCursorPos(($g[0]+280),($g[1]+210))){throw 'Could not position selection endpoint.'}
        Start-Sleep -Milliseconds 150
    } finally {
        [ShotKitOutputFailureSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero); $mouseDown=$false
    }
    $firstSequence=Wait-ForCondition -Seconds $CaptureTimeoutSeconds -Description 'clipboard update from the first manual selection' -OwnedProcess $shotKit -Condition {
        $current=[ShotKitOutputFailureSmoke.Native]::ClipboardSequence(); if($current -ne $initialClipboardSequence){$current}
    }
    $firstDib=[ShotKitOutputFailureSmoke.Native]::ReadClipboardDibV5()
    Assert-ClipboardDibV5 -Dib $firstDib -Width 200 -Height 150 -SourceX 80 -SourceY 60
    $editor=Wait-ForCondition -Seconds $CaptureTimeoutSeconds -Description 'fallback editor after autosave failure' -OwnedProcess $shotKit -Condition {
        Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible} | Select-Object -First 1
    }
    $failureDialog=Wait-ForCondition -Seconds $TimeoutSeconds -Description 'autosave failure dialog' -OwnedProcess $shotKit -Condition {
        Get-OwnWindows $shotKit | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and [ShotKitOutputFailureSmoke.Native]::WindowText($_.Handle) -like '*Ein Teil der Ausgabe ist fehlgeschlagen:*'} | Select-Object -First 1
    }
    $failureText=[ShotKitOutputFailureSmoke.Native]::WindowText($failureDialog.Handle)
    if ($failureText -notlike '*AutoSave: PNG konnte nicht gespeichert werden:*') {throw "Missing concrete AutoSave error in dialog: $failureText"}
    if (-not (Test-Path -LiteralPath $saveFolder -PathType Leaf)) {throw 'The regular file at save_folder disappeared or changed type.'}
    $actualBytes=[IO.File]::ReadAllBytes($saveFolder)
    if ([Convert]::ToBase64String($actualBytes) -ne [Convert]::ToBase64String($blockerBytes)) {throw 'The regular file at save_folder was modified.'}
    if(-not [ShotKitOutputFailureSmoke.Native]::CloseDialog($failureDialog.Handle)){throw 'Could not close the owned output error dialog.'}
    [void](Wait-ForCondition -Seconds 5 -Description 'autosave error dialog to close' -OwnedProcess $shotKit -Condition {if(-not [ShotKitOutputFailureSmoke.Native]::WindowVisible($failureDialog.Handle)){$true}})
    if (-not [ShotKitOutputFailureSmoke.Native]::WindowVisible($editor.Handle)) {throw 'Fallback editor closed unexpectedly.'}
    Write-Output "PASS: autosave failed on an unchanged fixture file, clipboard contained verified 200x150 CF_DIBV5 color pixels, fallback editor opened, and the AutoSave error was shown on '$inputDesktop'."
}
catch {$failure=$_}
finally {
    if($mouseDown){[ShotKitOutputFailureSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero);$mouseDown=$false}
    $env:APPDATA=$oldAppData; $env:LOCALAPPDATA=$oldLocalAppData
    foreach($owned in @($shotKit,$surface)) {
        if($null -ne $owned) {
            try {if (-not $owned.HasExited){$owned.Kill();if (-not $owned.WaitForExit(5000) -or -not $owned.HasExited){throw 'Owned process did not exit after termination request.'}}}
            catch {$allExited=$false;if (-not $failure){$failure=$_}}
            $owned.Dispose()
        }
    }
    if($allExited -and (Test-Path -LiteralPath $root)){
        try {Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop}
        catch {if (-not $failure){$failure=$_}}
    }
}
if($failure){throw $failure}

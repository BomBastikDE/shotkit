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
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') { throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.' }
if (-not [Environment]::UserInteractive) { throw 'Desktop session unavailable: this smoke test requires the interactive Windows desktop.' }

if (-not ('ShotKitLastRegionSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitLastRegionSmoke {
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
        [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hwnd);
        [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageW(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
        [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr menu, uint position);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern uint GetMenuStringW(IntPtr menu, uint position, StringBuilder text, int max, uint flags);
        [DllImport("user32.dll")] static extern int GetMenuState(IntPtr menu, uint item, uint flags);
        [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr hwnd, IntPtr menu, uint position, out Rect rect);
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

$inputDesktop=[ShotKitLastRegionSmoke.Native]::InputDesktopName()
if([string]::IsNullOrWhiteSpace($inputDesktop)){throw 'Could not open the interactive input desktop.'}
$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Last-Region-' + [Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata'; $localAppData=Join-Path $root 'localappdata'; $saveFolder=Join-Path $root 'captures'
$surfaceScript=Join-Path $root 'surface.ps1'; $readyFile=Join-Path $root 'surface-ready.txt'
$shotKit=$null; $surface=$null; $oldAppData=$env:APPDATA; $oldLocalAppData=$env:LOCALAPPDATA; $failure=$null

function Get-OwnWindows([Diagnostics.Process] $p) { [ShotKitLastRegionSmoke.Native]::Windows($p.Id) }
function Wait-ForCondition([scriptblock] $Condition,[int] $Seconds,[string] $Description,[Diagnostics.Process] $OwnedProcess) {
    $deadline=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $deadline) {
        if($OwnedProcess.HasExited){throw "Owned process exited before $Description (code $($OwnedProcess.ExitCode))."}
        $unexpected=Get-OwnWindows $OwnedProcess | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -notlike '*Prototyp*'} | Select-Object -First 1
        if($null -ne $unexpected){throw "Unexpected dialog '$($unexpected.Title)': $([ShotKitLastRegionSmoke.Native]::WindowText($unexpected.Handle))"}
        $v=& $Condition; if($null -ne $v -and $v){return $v}; Start-Sleep -Milliseconds 100
    }
    throw "Timed out waiting for $Description."
}
function Send-Key([byte] $Key,[bool] $Down) { $flags=if($Down){0}else{2}; [ShotKitLastRegionSmoke.Native]::keybd_event($Key,0,$flags,[UIntPtr]::Zero) }
function Test-PngReady([IO.FileInfo] $File) {
    if($null -eq $File){return $false}
    try {$bitmap=[Drawing.Bitmap]::new($File.FullName);try {[void]$bitmap.GetPixel(0,0);return $true} finally {$bitmap.Dispose()}}
    catch {
        $cause=$_.Exception.InnerException
        if($cause -is [System.ArgumentException] -or $cause -is [System.Runtime.InteropServices.ExternalException] -or $cause -is [System.IO.IOException]){return $false}
        throw
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

try {
    New-Item -ItemType Directory -Path $appData,$localAppData,$saveFolder -Force | Out-Null
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
namespace ShotKitLastRegionSurface {
 public sealed class Surface : Form {
  Bitmap image; string ready;
  [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  public Surface(string path) {
   try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch {}
   ready=path; Text="ShotKit Last Region Fixture"; StartPosition=FormStartPosition.Manual; Location=new Point(20,20);
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
[System.Windows.Forms.Application]::Run([ShotKitLastRegionSurface.Surface]::new($ReadyFile))
'@
    [IO.File]::WriteAllText($surfaceScript,$surfaceSource,[Text.UTF8Encoding]::new($false))
    $powershellExe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (-not (Test-Path -LiteralPath $powershellExe -PathType Leaf)){throw 'Windows PowerShell 5.1 is required for the isolated fixture window.'}
    $surfaceArgs='-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -ReadyFile "{1}"' -f $surfaceScript,$readyFile
    $surface=Start-Process -FilePath $powershellExe -ArgumentList $surfaceArgs -WorkingDirectory $root -WindowStyle Hidden -PassThru
    $fixtureWindow=Wait-ForCondition -Seconds 20 -Description 'the deterministic fixture surface' -OwnedProcess $surface -Condition {
        [ShotKitLastRegionSmoke.Native]::Windows($surface.Id) | Where-Object {$_.Title -eq 'ShotKit Last Region Fixture' -and $_.Visible} | Select-Object -First 1
    }
    $ready=Wait-ForCondition -Seconds 10 -Description 'fixture client coordinates' -OwnedProcess $surface -Condition {if(Test-Path -LiteralPath $readyFile){Get-Content -LiteralPath $readyFile -Raw}}
    $g=$ready.Trim().Split(',') | ForEach-Object {[int]$_}
    if($g.Count -ne 4 -or $g[2] -ne 900 -or $g[3] -ne 650){throw "Unexpected fixture geometry '$ready'."}

    $settingsPath=Join-Path (Join-Path $appData 'ShotKitRust') 'settings.json'
    New-Item -ItemType Directory -Path (Split-Path -Parent $settingsPath) -Force | Out-Null
    $settings=@{version=1;save_folder=$saveFolder;open_editor=$true;copy_to_clipboard=$false;auto_save=$true;scroll_direction='down'} | ConvertTo-Json
    [IO.File]::WriteAllText($settingsPath,$settings,[Text.UTF8Encoding]::new($false))
    try {$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$shotKit=Start-Process -FilePath $ExePath -WorkingDirectory $root -PassThru}
    finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    [void](Wait-ForCondition -Seconds $TimeoutSeconds -Description 'ShotKit main window' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustPrototype'} | Select-Object -First 1})
    $welcome=Wait-ForCondition -Seconds $TimeoutSeconds -Description 'ShotKit startup dialog' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1}
    if (-not [ShotKitLastRegionSmoke.Native]::PostMessageW($welcome.Handle,0x0111,[IntPtr]1,[IntPtr]::Zero)){throw 'Could not close the owned startup dialog.'}
    Start-Sleep -Milliseconds 250

    # First successful manual selection establishes LAST_REGION and creates a
    # pixel-checked image from the known fixture coordinates (80,60)-(280,210).
    [void][ShotKitLastRegionSmoke.Native]::SetCursorPos(($g[0]+80),($g[1]+60))
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $overlay=Wait-ForCondition -Seconds 10 -Description 'the first area-selection overlay' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}
    [void][ShotKitLastRegionSmoke.Native]::SetCursorPos(($g[0]+80),($g[1]+60)); Start-Sleep -Milliseconds 100
    [ShotKitLastRegionSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero)
    [void][ShotKitLastRegionSmoke.Native]::SetCursorPos(($g[0]+280),($g[1]+210)); Start-Sleep -Milliseconds 150
    [ShotKitLastRegionSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero)
    Add-Type -AssemblyName System.Drawing
    $first=Wait-ForCondition -Seconds $CaptureTimeoutSeconds -Description 'the first manually selected complete PNG' -OwnedProcess $shotKit -Condition {$file=Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Select-Object -First 1;if(Test-PngReady $file){$file}}
    Assert-PatternPng -File $first -Width 200 -Height 150 -SourceX 80 -SourceY 60 -Description 'First manual capture'

    # Optional abort check: beginning and cancelling another manual selection
    # must leave the previously successful rectangle available for repetition.
    [void][ShotKitLastRegionSmoke.Native]::SetCursorPos(($g[0]+500),($g[1]+400))
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $cancelOverlay=Wait-ForCondition -Seconds 10 -Description 'selection overlay for cancellation check' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}
    [void][ShotKitLastRegionSmoke.Native]::SetForegroundWindow($cancelOverlay.Handle)
    Send-Key 0x1B $true; Start-Sleep -Milliseconds 80; Send-Key 0x1B $false
    [void](Wait-ForCondition -Seconds 10 -Description 'Escape to close the cancelled selection' -OwnedProcess $shotKit -Condition {if (-not (Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible})){$true}})

    # Move the pointer well away from the saved rectangle. Then open ShotKit's
    # real tray popup and select its enabled exact-text native menu entry.
    $desktopW=[ShotKitLastRegionSmoke.Native]::GetSystemMetrics(0); $desktopH=[ShotKitLastRegionSmoke.Native]::GetSystemMetrics(1)
    $pointerX=$desktopW-40; $pointerY=$desktopH-80
    if($pointerX -ge $g[0]+80 -and $pointerX -lt $g[0]+280 -and $pointerY -ge $g[1]+60 -and $pointerY -lt $g[1]+210){$pointerX=$g[0]+700;$pointerY=$g[1]+500}
    if (-not [ShotKitLastRegionSmoke.Native]::SetCursorPos($pointerX,$pointerY)){throw 'Could not move pointer away from the remembered capture area.'}
    $main=Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustPrototype'} | Select-Object -First 1
    if($null -eq $main){throw 'ShotKit main/tray owner window disappeared before the repeat command.'}
    # Same callback message that the registered tray icon receives on right-click.
    if (-not [ShotKitLastRegionSmoke.Native]::PostMessageW($main.Handle,0x8001,[IntPtr]1,[IntPtr]0x0205)){throw 'Could not post the tray right-click callback to the owned ShotKit window.'}
    $popup=Wait-ForCondition -Seconds 10 -Description 'the visible native tray popup' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.Visible -and $_.ClassName -eq '#32768'} | Select-Object -First 1}
    $menuEvidence=[ShotKitLastRegionSmoke.Native]::VerifyAndClickLastRegion($popup.Handle)
    $allPngs=Wait-ForCondition -Seconds $CaptureTimeoutSeconds -Description 'the second PNG from the remembered area' -OwnedProcess $shotKit -Condition {
        $files=@(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File)
        $newFile=$files | Where-Object {$_.FullName -ne $first.FullName} | Select-Object -First 1
        if(Test-PngReady $newFile){$files}
    }
    $second=@($allPngs | Where-Object {$_.FullName -ne $first.FullName}) | Select-Object -First 1
    if($null -eq $second){throw 'The tray repeat command did not create a separate second PNG.'}
    Assert-PatternPng -File $second -Width 200 -Height 150 -SourceX 80 -SourceY 60 -Description 'Repeated last-region capture after pointer movement'
    Write-Output "PASS: first manual rectangle (80,60)-(280,210) saved 200x150 with verified coordinate/color pixels; Escape left it intact; after pointer moved to ($pointerX,$pointerY), $menuEvidence produced a second 200x150 PNG with the same checked source pixels on '$inputDesktop'."
}
catch {$failure=$_}
finally {
    $env:APPDATA=$oldAppData; $env:LOCALAPPDATA=$oldLocalAppData
    foreach($owned in @($shotKit,$surface)) {
        if($null -ne $owned) {
            try {if (-not $owned.HasExited){$owned.Kill();if (-not $owned.WaitForExit(5000) -or -not $owned.HasExited){throw 'Owned process did not exit after termination request.'}}}
            catch {if (-not $failure){$failure=$_}}
            $owned.Dispose()
        }
    }
    if(Test-Path -LiteralPath $root){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($failure){throw $failure}

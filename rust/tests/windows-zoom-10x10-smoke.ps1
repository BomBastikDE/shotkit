[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$ExePath,
    [ValidateRange(10,120)][int]$StartupTimeoutSeconds=30
)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if($PSVersionTable.PSVersion.Major -lt 7){throw 'PowerShell 7 (pwsh) is required.'}
if(($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive){throw 'INTERACTIVE_WINDOWS_REQUIRED: ephemeral interactive runner or dedicated opt-in session required.'}
if(-not(Test-Path -LiteralPath $ExePath -PathType Leaf)){throw "ShotKit EXE not found: $ExePath"}
$ExePath=(Resolve-Path -LiteralPath $ExePath).Path
$imageName=[IO.Path]::GetFileNameWithoutExtension($ExePath)
$processNames=@($imageName,'ShotKit','shotkit-rust-prototype')|Select-Object -Unique
foreach($name in $processNames){if(@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count){throw "PREEXISTING_PROCESS: refusing to use/change '$name'."}}
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitZoomTenByTenSmoke {
 public struct Rect { public int Left,Top,Right,Bottom; }
 public sealed class Win { public IntPtr H; public string ClassName,Title; public bool Visible; }
 public static class N {
  delegate bool EnumProc(IntPtr h,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr d);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int cmd);
  [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] static extern IntPtr OpenInputDesktop(uint f,bool inherit,uint access);
  [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr h);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetUserObjectInformationW(IntPtr h,int i,StringBuilder s,uint n,out uint needed);
  public static string InputDesktop(){IntPtr h=OpenInputDesktop(0,false,1);if(h==IntPtr.Zero)return null;try{var s=new StringBuilder(256);uint n;return GetUserObjectInformationW(h,2,s,512,out n)?s.ToString():null;}finally{CloseDesktop(h);}}
  static string Read(IntPtr h,bool c){var s=new StringBuilder(512);if(c)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString();}
  public static Win[] Windows(int pid){var a=new List<Win>();EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)pid)a.Add(new Win{H=h,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static int Owner(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return (int)p;}
  public static void CtrlZero(){try{keybd_event(0x11,0,0,UIntPtr.Zero);keybd_event(0x30,0,0,UIntPtr.Zero);}finally{keybd_event(0x30,0,2,UIntPtr.Zero);keybd_event(0x11,0,2,UIntPtr.Zero);}}
 }
}
'@
$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Zoom-10x10-Smoke-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$fixture=Join-Path $root 'fixture-10x10.png'
$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA;$proc=$null;$failure=$null
function Owned-Windows {if(!$proc -or $proc.HasExited){return @()};@([ShotKitZoomTenByTenSmoke.N]::Windows($proc.Id))}
function Wait-For([scriptblock]$Probe,[string]$What,[int]$Seconds){$end=[DateTime]::UtcNow.AddSeconds($Seconds);while([DateTime]::UtcNow -lt $end){if($proc -and $proc.HasExited){throw "PROCESS_EXITED waiting for $What (code $($proc.ExitCode))."};$v=&$Probe;if($null -ne $v -and $v){return $v};Start-Sleep -Milliseconds 100};throw "TIMEOUT waiting for $What."}
function Focus-Editor($H){if([ShotKitZoomTenByTenSmoke.N]::Owner($H) -ne $proc.Id){throw 'EDITOR_OWNER_MISMATCH'};[void][ShotKitZoomTenByTenSmoke.N]::SetForegroundWindow($H);[void](Wait-For {[ShotKitZoomTenByTenSmoke.N]::GetForegroundWindow() -eq $H} 'exact editor foreground' 5)}
function New-Fixture {
    $b=[Drawing.Bitmap]::new(10,10,[Drawing.Imaging.PixelFormat]::Format24bppRgb)
    try { for($y=0;$y -lt 10;$y++){for($x=0;$x -lt 10;$x++){$b.SetPixel($x,$y,[Drawing.Color]::FromArgb(20+20*$x,20+20*$y,73))} };$b.Save($fixture,[Drawing.Imaging.ImageFormat]::Png) }
    finally {$b.Dispose()}
}
function Assert-ExactImagePixels($H) {
    $r=[Activator]::CreateInstance([ShotKitZoomTenByTenSmoke.Rect]);if(-not[ShotKitZoomTenByTenSmoke.N]::GetWindowRect($H,[ref]$r)){throw 'WINDOW_RECT_FAILED'}
    if(($r.Right-$r.Left) -lt 800 -or ($r.Bottom-$r.Top) -lt 600){throw "EDITOR_NOT_LARGE: window is $($r.Right-$r.Left)x$($r.Bottom-$r.Top), expected at least 800x600."}
    $x0=[Math]::Max(0,$r.Left);$y0=[Math]::Max(0,$r.Top);$x1=$r.Right;$y1=$r.Bottom
    $bmp=[Drawing.Bitmap]::new($x1-$x0,$y1-$y0);$g=[Drawing.Graphics]::FromImage($bmp)
    try {$g.CopyFromScreen($x0,$y0,0,0,$bmp.Size)}finally{$g.Dispose()}
    try {
        $expected=@{};$observed=@{}
        for($y=0;$y -lt 10;$y++){for($x=0;$x -lt 10;$x++){$key=([Drawing.Color]::FromArgb(20+20*$x,20+20*$y,73)).ToArgb();$expected[$key]="$x,$y";$observed[$key]=[Collections.Generic.List[object]]::new()}}
        for($py=0;$py -lt $bmp.Height;$py++){for($px=0;$px -lt $bmp.Width;$px++){$key=$bmp.GetPixel($px,$py).ToArgb();if($expected.ContainsKey($key)){$observed[$key].Add([pscustomobject]@{X=$px;Y=$py})}}}
        $origin=$null
        foreach($y in 0..9){foreach($x in 0..9){$key=([Drawing.Color]::FromArgb(20+20*$x,20+20*$y,73)).ToArgb();$hits=$observed[$key];if($hits.Count -ne 1){throw "PIXEL_COUNT_MISMATCH: fixture pixel ($x,$y) appeared $($hits.Count) times; expected exactly once. This can indicate zoom or display scaling."};$p=$hits[0];if($x -eq 0 -and $y -eq 0){$origin=$p};if($p.X -ne $origin.X+$x -or $p.Y -ne $origin.Y+$y){throw "PIXEL_GRID_MISMATCH: source pixel ($x,$y) is at screen-window ($($p.X),$($p.Y)); expected ($($origin.X+$x),$($origin.Y+$y))."}}}
        Write-Output ("PASS_ACTION Ctrl+0: visible 10x10 fixture matched all 100 unique source pixels at 1 screen pixel per image pixel; origin=({0},{1}), editor={2}x{3}." -f ($origin.X+$x0),($origin.Y+$y0),($r.Right-$r.Left),($r.Bottom-$r.Top))
    } finally {$bmp.Dispose()}
}
try {
    if([ShotKitZoomTenByTenSmoke.N]::InputDesktop() -ne 'Default'){throw "NO_INTERACTIVE_INPUT_DESKTOP: $([ShotKitZoomTenByTenSmoke.N]::InputDesktop())"}
    New-Item -ItemType Directory -Path $appData,$localAppData -Force|Out-Null;New-Fixture
    $env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData
    try {$proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $fixture) -WorkingDirectory $root -PassThru}
    finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    Assert-NoStartupDialogs -TimeoutSeconds $StartupTimeoutSeconds -OwnedProcess $proc -GetWindows { Owned-Windows }
    $main=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} 'owned editor window' $StartupTimeoutSeconds
    if([ShotKitZoomTenByTenSmoke.N]::Owner($main.H) -ne $proc.Id){throw 'EDITOR_OWNER_MISMATCH'}
    [void][ShotKitZoomTenByTenSmoke.N]::ShowWindow($main.H,3) # SW_MAXIMIZE
    Start-Sleep -Milliseconds 500;Focus-Editor $main.H
    [ShotKitZoomTenByTenSmoke.N]::CtrlZero();Start-Sleep -Milliseconds 500
    Assert-ExactImagePixels $main.H
} catch {$failure=$_} finally {
    if($proc -and -not $proc.HasExited){try{foreach($w in [ShotKitZoomTenByTenSmoke.N]::Windows($proc.Id)){[void][ShotKitZoomTenByTenSmoke.N]::PostMessageW($w.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)};if(-not $proc.WaitForExit(5000)){$proc.Kill();if(-not $proc.WaitForExit(5000)){throw 'OWNED_PROCESS_CLEANUP_FAILED: process remained alive after close and bounded kill wait.'}}}catch{if($null -eq $failure){$failure=$_}}}
    $mayRemoveRoot=($null -eq $proc -or $proc.HasExited);if($proc){$proc.Dispose()};$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
    if($mayRemoveRoot -and (Test-Path -LiteralPath $root)){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}
Write-Output 'PASS: Ctrl+0 displayed the 10x10 image at native 1:1 screen-pixel size.'

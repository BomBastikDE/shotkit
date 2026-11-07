[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,
    [ValidateRange(10, 120)]
    [int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') { throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.' }
if (-not [Environment]::UserInteractive) { throw 'NO_INTERACTIVE_DESKTOP: the Windows runner is not interactive.' }
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) { throw "PREEXISTING_PROCESS: '$imageName' is already running." }

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitRedactionSmoke {
 public struct Rect { public int Left, Top, Right, Bottom; }
 public struct Point { public int X, Y; }
 public sealed class Win { public IntPtr H; public string ClassName, Title; public bool Visible; }
 public sealed class Btn { public IntPtr H; public string Text; }
 public static class N {
  delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h, EnumProc cb, IntPtr p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out Rect r);
  [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr h, ref Point p);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, UIntPtr e);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  static string Txt(IntPtr h) { var b=new StringBuilder(512); GetWindowTextW(h,b,b.Capacity); return b.ToString(); }
  static string Cls(IntPtr h) { var b=new StringBuilder(256); GetClassNameW(h,b,b.Capacity); return b.ToString(); }
  public static uint Pid(IntPtr h) { uint p; GetWindowThreadProcessId(h,out p); return p; }
  public static Win[] Windows(int pid) { var a=new List<Win>(); EnumProc cb=(h,p)=>{ if(Pid(h)==(uint)pid) a.Add(new Win{H=h,ClassName=Cls(h),Title=Txt(h),Visible=IsWindowVisible(h)}); return true; }; EnumWindows(cb,IntPtr.Zero); GC.KeepAlive(cb); return a.ToArray(); }
  public static Win[] Children(IntPtr parent) { var a=new List<Win>(); EnumProc cb=(h,p)=>{ a.Add(new Win{H=h,ClassName=Cls(h),Title=Txt(h),Visible=IsWindowVisible(h)}); return true; }; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return a.ToArray(); }
  public static Btn[] Buttons(IntPtr parent) { var a=new List<Btn>(); EnumProc cb=(h,p)=>{ if(IsWindowVisible(h) && Cls(h).Equals("Button",StringComparison.OrdinalIgnoreCase)) a.Add(new Btn{H=h,Text=Txt(h)}); return true; }; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return a.ToArray(); }
  public static void Click(IntPtr h) { if(h==IntPtr.Zero) throw new Exception("Null button HWND"); SendMessageW(h,0x00F5,IntPtr.Zero,IntPtr.Zero); }
  public static bool ChildRect(IntPtr parent,IntPtr child,out Rect r) { r=new Rect(); if(!GetWindowRect(child,out r))return false; var a=new Point{X=r.Left,Y=r.Top}; var b=new Point{X=r.Right,Y=r.Bottom}; if(!ScreenToClient(parent,ref a)||!ScreenToClient(parent,ref b))return false; r=new Rect{Left=a.X,Top=a.Y,Right=b.X,Bottom=b.Y}; return true; }
 }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Redaction-Smoke-' + [Guid]::NewGuid().ToString('N'))
$oldAppData=$env:APPDATA; $oldLocalAppData=$env:LOCALAPPDATA
$process=$null; $mouseDown=$false; $failure=$null
$inputPath=Join-Path $root 'fixture.png'
$envApp=Join-Path $root 'appdata'; $envLocal=Join-Path $root 'localappdata'

function Get-Main {
    if ($null -eq $process -or $process.HasExited) { return $null }
    [ShotKitRedactionSmoke.N]::Windows($process.Id) | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' } | Select-Object -First 1
}
function Wait-Value([scriptblock]$Probe,[string]$What,[int]$Seconds=$TimeoutSeconds) {
    $end=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $end) { if($process -and $process.HasExited){throw "PROCESS_EXITED before $What (code $($process.ExitCode))."}; $v=&$Probe; if($null -ne $v -and $v){return $v}; Start-Sleep -Milliseconds 100 }
    throw "TIMEOUT waiting for $What"
}
function Click-Button([IntPtr]$Parent,[string]$Text) {
    $b=[ShotKitRedactionSmoke.N]::Buttons($Parent) | Where-Object { ($_.Text -replace '&','').Trim() -eq $Text } | Select-Object -First 1
    if($null -eq $b){$all=([ShotKitRedactionSmoke.N]::Buttons($Parent)|%{$_.Text}) -join ', '; throw "BUTTON_NOT_FOUND '$Text'; found [$all]"}
    [ShotKitRedactionSmoke.N]::Click($b.H)
}
function Save-Image([string]$Description) {
    Click-Button $main.H 'Speichern'
    $end=[DateTime]::UtcNow.AddSeconds(15)
    while([DateTime]::UtcNow -lt $end) { try { $b=[Drawing.Bitmap]::new($inputPath); $b.Dispose(); return } catch { Start-Sleep -Milliseconds 120 } }
    throw "SAVE_TIMEOUT: $Description"
}
function Read-Image {
    $b=[Drawing.Bitmap]::new($inputPath)
    try { $pixels=[Array]::CreateInstance([Drawing.Color],$b.Width*$b.Height); for($y=0;$y -lt $b.Height;$y++){for($x=0;$x -lt $b.Width;$x++){$pixels[$y*$b.Width+$x]=$b.GetPixel($x,$y)}}; return [pscustomobject]@{Width=$b.Width;Height=$b.Height;Pixels=$pixels} }
    finally {$b.Dispose()}
}
function Assert-Original($im,[string]$phase) {
    if($im.Width -ne 160 -or $im.Height -ne 120){throw "$phase dimensions changed to $($im.Width)x$($im.Height)"}
    for($i=0;$i -lt $original.Pixels.Length;$i++){ $a=$im.Pixels[$i];$b=$original.Pixels[$i]; if($a.ToArgb() -ne $b.ToArgb()){throw "$phase mismatch at pixel ($($i%160),$([int]($i/160))): got $a expected $b"} }
}
function Assert-Redacted($im,[string]$phase,[bool]$withPixelation) {
    if($im.Width -ne 160 -or $im.Height -ne 120){throw "$phase dimensions changed"}
    $pixelated=0
    for($y=0;$y -lt 120;$y++){for($x=0;$x -lt 160;$x++){
        $i=$y*160+$x; $a=$im.Pixels[$i]; $b=$original.Pixels[$i]
        # Allow a small boundary margin for screen-to-image coordinate rounding.
        if($x -ge 26 -and $x -le 88 -and $y -ge 22 -and $y -le 78) {
            if($a.A -ne 255 -or $a.R -ne 0 -or $a.G -ne 0 -or $a.B -ne 0){throw "$phase redaction not opaque black at ($x,$y): $a"}
        }
        if(($x -lt 22 -or $x -gt 92 -or $y -lt 18 -or $y -gt 82) -and
           (-not $withPixelation -or $x -lt 58 -or $x -gt 132 -or $y -lt 38 -or $y -gt 102)) {
            if($a.ToArgb() -ne $b.ToArgb()){throw "$phase changed outside both regions at ($x,$y)"}
        }
        if($withPixelation -and $x -ge 98 -and $x -le 126 -and $y -ge 44 -and $y -le 96 -and $a.ToArgb() -ne $b.ToArgb()){$pixelated++}
    }}
    if($withPixelation -and $pixelated -lt 10){throw "$phase did not pixelate outside the redaction (changed=$pixelated)"}
}
function Drag-Image([int]$x1,[int]$y1,[int]$x2,[int]$y2) {
    $pt=[Activator]::CreateInstance([ShotKitRedactionSmoke.Point]); if(-not [ShotKitRedactionSmoke.N]::ClientToScreen($main.H,[ref]$pt)){throw 'ClientToScreen failed'}
    $sx=$pt.X+[int][Math]::Round($viewX+($x1+0.5)*$displayW/160,[MidpointRounding]::AwayFromZero)
    $sy=$pt.Y+[int][Math]::Round($viewY+($y1+0.5)*$displayH/120,[MidpointRounding]::AwayFromZero)
    $ex=$pt.X+[int][Math]::Round($viewX+($x2+0.5)*$displayW/160,[MidpointRounding]::AwayFromZero)
    $ey=$pt.Y+[int][Math]::Round($viewY+($y2+0.5)*$displayH/120,[MidpointRounding]::AwayFromZero)
    if(-not [ShotKitRedactionSmoke.N]::SetForegroundWindow($main.H)){throw 'EDITOR_FOCUS_FAILED'}
    if(-not [ShotKitRedactionSmoke.N]::SetCursorPos($sx,$sy)){throw 'Could not position drag start'}
    $p=[Activator]::CreateInstance([ShotKitRedactionSmoke.Point]);[void][ShotKitRedactionSmoke.N]::GetCursorPos([ref]$p)
    if([ShotKitRedactionSmoke.N]::Pid([ShotKitRedactionSmoke.N]::WindowFromPoint($p)) -ne [uint32]$process.Id){throw 'INPUT_TARGET_MISMATCH at drag start'}
    [ShotKitRedactionSmoke.N]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true;Start-Sleep -Milliseconds 100
    [void][ShotKitRedactionSmoke.N]::SetCursorPos($ex,$ey);Start-Sleep -Milliseconds 200
    [ShotKitRedactionSmoke.N]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
}

try {
    New-Item -ItemType Directory -Path $root,$envApp,$envLocal -Force | Out-Null
    $fixture=[Drawing.Bitmap]::new(160,120,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try { for($y=0;$y -lt 120;$y++){for($x=0;$x -lt 160;$x++){ $c=if((( [int]($x/4)+[int]($y/4))%2)-eq 0){[Drawing.Color]::FromArgb(255,240,20,30)}else{[Drawing.Color]::FromArgb(255,15,40,230)}; $fixture.SetPixel($x,$y,$c) }}; $fixture.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png) } finally {$fixture.Dispose()}
    $original=Read-Image
    $env:APPDATA=$envApp;$env:LOCALAPPDATA=$envLocal
    try {$process=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru} finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    $main=Wait-Value {Get-Main} 'owned editor window'
    [void][ShotKitRedactionSmoke.N]::ShowWindow($main.H,3);[void][ShotKitRedactionSmoke.N]::SetForegroundWindow($main.H)
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $process -GetWindows { [ShotKitRedactionSmoke.N]::Windows($process.Id) }
    $main=Wait-Value {Get-Main} 'ready editor'
    $client=[Activator]::CreateInstance([ShotKitRedactionSmoke.Rect]);[void][ShotKitRedactionSmoke.N]::GetClientRect($main.H,[ref]$client)
    $toolbar=[ShotKitRedactionSmoke.N]::Children($main.H)|?{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'}|select -First 1
    if($null -eq $toolbar){throw 'EDITOR_TOOLBAR_MISSING'}
    $tr=[Activator]::CreateInstance([ShotKitRedactionSmoke.Rect]);[void][ShotKitRedactionSmoke.N]::GetClientRect($toolbar.H,[ref]$tr)
    $vw=$client.Right;$vh=$client.Bottom-$tr.Bottom
    $fit=[Math]::Min($vw/160.0,$vh/120.0);$displayW=[int][Math]::Round(160*$fit);$displayH=[int][Math]::Round(120*$fit)
    $viewX=[int][Math]::Floor(($vw-$displayW)/2.0);$viewY=$tr.Bottom+[int][Math]::Floor(($vh-$displayH)/2.0)
    if(-not [ShotKitRedactionSmoke.N]::SetForegroundWindow($main.H)){throw 'EDITOR_FOCUS_FAILED'}
    [ShotKitRedactionSmoke.N]::keybd_event(0x53,0,0,[UIntPtr]::Zero)
    [ShotKitRedactionSmoke.N]::keybd_event(0x53,0,2,[UIntPtr]::Zero)
    [void](Wait-Value {Get-Main|?{$_.Title -like '*Werkzeug Redact*'}} 'Redact tool activation by S')
    Drag-Image 24 20 90 80
    Save-Image 'redaction export'; Assert-Redacted (Read-Image) 'redaction export' $false
    Click-Button $main.H 'Verpixeln (B) – keine sichere Schwärzung'
    [void](Wait-Value {Get-Main|?{$_.Title -like '*Werkzeug Blur*'}} 'Blur tool activation')
    Drag-Image 60 40 130 100
    Save-Image 'overlapping pixelation export'; Assert-Redacted (Read-Image) 'overlapping pixelation export' $true
    Click-Button $main.H 'Zurück'; Save-Image 'undo pixelation export'; Assert-Redacted (Read-Image) 'undo pixelation export' $false
    Click-Button $main.H 'Zurück'; Save-Image 'undo redaction export'; Assert-Original (Read-Image) 'undo redaction export'
    Click-Button $main.H 'Wiederholen'; Save-Image 'redo redaction export'; Assert-Redacted (Read-Image) 'redo redaction export' $false
    Click-Button $main.H 'Wiederholen'; Save-Image 'redo pixelation export'; Assert-Redacted (Read-Image) 'redo pixelation export' $true
    Write-Output 'PASS: S activates redaction; PNG exports preserve opaque black through overlapping pixelation and undo/redo, with unrelated pixels intact.'

}
catch { $failure=$_ }
finally {
    if($mouseDown){try{[ShotKitRedactionSmoke.N]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($process -and -not $process.HasExited){try{$process.Kill();$process.WaitForExit()}catch{}}
    $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
    if(Test-Path -LiteralPath $root){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}

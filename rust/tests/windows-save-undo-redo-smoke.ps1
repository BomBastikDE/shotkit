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
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) { throw 'This smoke test requires an interactive ephemeral GitHub Actions Windows runner or an explicitly opted-in local interactive session.' }
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
namespace ShotKitSaveHistorySmoke {
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
  [DllImport("user32.dll")] static extern bool SendMessageTimeoutW(IntPtr h, uint m, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll", SetLastError=true)] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
  [DllImport("user32.dll", SetLastError=true)] static extern bool CloseDesktop(IntPtr desktop);
  [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool GetUserObjectInformationW(IntPtr obj, int index, StringBuilder info, uint length, out uint needed);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out Rect r);
  [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr h, ref Point p);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  static string Txt(IntPtr h) { var b=new StringBuilder(512); GetWindowTextW(h,b,b.Capacity); return b.ToString(); }
  static string Cls(IntPtr h) { var b=new StringBuilder(256); GetClassNameW(h,b,b.Capacity); return b.ToString(); }
  public static uint Pid(IntPtr h) { uint p; GetWindowThreadProcessId(h,out p); return p; }
  public static Win[] Windows(int pid) { var a=new List<Win>(); EnumProc cb=(h,p)=>{ if(Pid(h)==(uint)pid) a.Add(new Win{H=h,ClassName=Cls(h),Title=Txt(h),Visible=IsWindowVisible(h)}); return true; }; EnumWindows(cb,IntPtr.Zero); GC.KeepAlive(cb); return a.ToArray(); }
  public static Win[] Children(IntPtr parent) { var a=new List<Win>(); EnumProc cb=(h,p)=>{ a.Add(new Win{H=h,ClassName=Cls(h),Title=Txt(h),Visible=IsWindowVisible(h)}); return true; }; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return a.ToArray(); }
  public static Btn[] Buttons(IntPtr parent) { var a=new List<Btn>(); EnumProc cb=(h,p)=>{ if(IsWindowVisible(h) && Cls(h).Equals("Button",StringComparison.OrdinalIgnoreCase)) a.Add(new Btn{H=h,Text=Txt(h)}); return true; }; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return a.ToArray(); }
  public static void Click(IntPtr h) { if(h==IntPtr.Zero) throw new Exception("Null button HWND"); IntPtr result; if(!SendMessageTimeoutW(h,0x00F5,IntPtr.Zero,IntPtr.Zero,0x0002,3000,out result)) throw new TimeoutException("BM_CLICK did not complete within 3 seconds."); }
  public static string InputDesktopName() { IntPtr d=OpenInputDesktop(0,false,0x0001); if(d==IntPtr.Zero)return null; try { var s=new StringBuilder(256);uint needed;if(!GetUserObjectInformationW(d,2,s,(uint)(s.Capacity*2),out needed))return null;return s.ToString(); } finally { CloseDesktop(d); } }
  public static bool ChildRect(IntPtr parent,IntPtr child,out Rect r) { r=new Rect(); if(!GetWindowRect(child,out r))return false; var a=new Point{X=r.Left,Y=r.Top}; var b=new Point{X=r.Right,Y=r.Bottom}; if(!ScreenToClient(parent,ref a)||!ScreenToClient(parent,ref b))return false; r=new Rect{Left=a.X,Top=a.Y,Right=b.X,Bottom=b.Y}; return true; }
 }
}
'@

    $root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Save-Undo-Redo-Smoke-' + [Guid]::NewGuid().ToString('N'))
$oldAppData=$env:APPDATA; $oldLocalAppData=$env:LOCALAPPDATA
$process=$null; $mouseDown=$false; $failure=$null; $cleanupFailure=$null; $processExited=$true
$inputPath=Join-Path $root 'fixture.png'
$envApp=Join-Path $root 'appdata'; $envLocal=Join-Path $root 'localappdata'

function Get-Main {
    if ($null -eq $process -or $process.HasExited) { return $null }
    [ShotKitSaveHistorySmoke.N]::Windows($process.Id) | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' } | Select-Object -First 1
}
function Wait-Value([scriptblock]$Probe,[string]$What,[int]$Seconds=$TimeoutSeconds) {
    $end=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $end) { if($process -and $process.HasExited){throw "PROCESS_EXITED before $What (code $($process.ExitCode))."}; $v=&$Probe; if($null -ne $v -and $v){return $v}; Start-Sleep -Milliseconds 100 }
    throw "TIMEOUT waiting for $What"
}
function Click-Button([IntPtr]$Parent,[string]$Text) {
    $b=[ShotKitSaveHistorySmoke.N]::Buttons($Parent) | Where-Object { ($_.Text -replace '&','').Trim() -eq $Text } | Select-Object -First 1
    if($null -eq $b){$all=([ShotKitSaveHistorySmoke.N]::Buttons($Parent)|%{$_.Text}) -join ', '; throw "BUTTON_NOT_FOUND '$Text'; found [$all]"}
    [ShotKitSaveHistorySmoke.N]::Click($b.H)
}
function Assert-EditorTitle([string]$Tool,[bool]$Dirty,[string]$Phase) {
    $prefix=if($Dirty){'* '}else{''}
    $expected=$prefix+$inputPath+' – ShotKit · Werkzeug: '+$Tool+' · Strg+S speichern'
    [void](Wait-Value {Get-Main|Where-Object {$_.Title -ceq $expected}} "$Phase; expected title '$expected'")
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
function Assert-Pixelated($im,[string]$phase) {
    if($im.Width -ne 160 -or $im.Height -ne 120){throw "$phase dimensions changed"}
    $changed=0
    for($y=0;$y -lt 120;$y++){for($x=0;$x -lt 160;$x++){
        $i=$y*160+$x;$a=$im.Pixels[$i];$b=$original.Pixels[$i]
        if($x -lt 22 -or $x -gt 122 -or $y -lt 18 -or $y -gt 92) { if($a.ToArgb() -ne $b.ToArgb()){throw "$phase changed outside selected region at ($x,$y)"} }
        elseif($x -ge 26 -and $x -le 118 -and $y -ge 22 -and $y -le 88 -and $a.ToArgb() -ne $b.ToArgb()){$changed++}
    }}
    if($changed -lt 10){throw "$phase did not change enough selected pixels (changed=$changed)"}
    $probe=$im.Pixels[55*160+70];$source=$original.Pixels[55*160+70]
    if($probe.ToArgb() -eq $source.ToArgb()){throw "$phase center probe was unchanged"}
}
function Drag-Image([int]$x1,[int]$y1,[int]$x2,[int]$y2) {
    $pt=[Activator]::CreateInstance([ShotKitSaveHistorySmoke.Point]); if(-not [ShotKitSaveHistorySmoke.N]::ClientToScreen($main.H,[ref]$pt)){throw 'ClientToScreen failed'}
    $sx=$pt.X+[int][Math]::Round($viewX+($x1+0.5)*$displayW/160,[MidpointRounding]::AwayFromZero)
    $sy=$pt.Y+[int][Math]::Round($viewY+($y1+0.5)*$displayH/120,[MidpointRounding]::AwayFromZero)
    $ex=$pt.X+[int][Math]::Round($viewX+($x2+0.5)*$displayW/160,[MidpointRounding]::AwayFromZero)
    $ey=$pt.Y+[int][Math]::Round($viewY+($y2+0.5)*$displayH/120,[MidpointRounding]::AwayFromZero)
    if(-not [ShotKitSaveHistorySmoke.N]::SetForegroundWindow($main.H)){throw 'EDITOR_FOCUS_FAILED'}
    if(-not [ShotKitSaveHistorySmoke.N]::SetCursorPos($sx,$sy)){throw 'Could not position drag start'}
    $p=[Activator]::CreateInstance([ShotKitSaveHistorySmoke.Point]);[void][ShotKitSaveHistorySmoke.N]::GetCursorPos([ref]$p)
    if([ShotKitSaveHistorySmoke.N]::Pid([ShotKitSaveHistorySmoke.N]::WindowFromPoint($p)) -ne [uint32]$process.Id){throw 'INPUT_TARGET_MISMATCH at drag start'}
    [ShotKitSaveHistorySmoke.N]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true;Start-Sleep -Milliseconds 100
    [void][ShotKitSaveHistorySmoke.N]::SetCursorPos($ex,$ey);Start-Sleep -Milliseconds 200
    [ShotKitSaveHistorySmoke.N]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
}

try {
    $inputDesktop=[ShotKitSaveHistorySmoke.N]::InputDesktopName()
    if($inputDesktop -cne 'Default'){throw "DEFAULT_DESKTOP_REQUIRED: active input desktop is '$inputDesktop'."}
    New-Item -ItemType Directory -Path $root,$envApp,$envLocal -Force | Out-Null
    $fixture=[Drawing.Bitmap]::new(160,120,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try { for($y=0;$y -lt 120;$y++){for($x=0;$x -lt 160;$x++){ $c=if((( [int]($x/4)+[int]($y/4))%2)-eq 0){[Drawing.Color]::FromArgb(255,240,20,30)}else{[Drawing.Color]::FromArgb(255,15,40,230)}; $fixture.SetPixel($x,$y,$c) }}; $fixture.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png) } finally {$fixture.Dispose()}
    $original=Read-Image
    $originalHash=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash
    $env:APPDATA=$envApp;$env:LOCALAPPDATA=$envLocal
    try {$process=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru} finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    $main=Wait-Value {Get-Main} 'owned editor window'
    [void][ShotKitSaveHistorySmoke.N]::ShowWindow($main.H,3);[void][ShotKitSaveHistorySmoke.N]::SetForegroundWindow($main.H)
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $process -GetWindows { [ShotKitSaveHistorySmoke.N]::Windows($process.Id) }
    $main=Wait-Value {Get-Main} 'ready editor'
    Assert-EditorTitle 'Auswahl' $false 'opened filename and initial German tool'
    $client=[Activator]::CreateInstance([ShotKitSaveHistorySmoke.Rect]);[void][ShotKitSaveHistorySmoke.N]::GetClientRect($main.H,[ref]$client)
    $toolbar=[ShotKitSaveHistorySmoke.N]::Children($main.H)|?{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'}|select -First 1
    if($null -eq $toolbar){throw 'EDITOR_TOOLBAR_MISSING'}
    $tr=[Activator]::CreateInstance([ShotKitSaveHistorySmoke.Rect]);[void][ShotKitSaveHistorySmoke.N]::GetClientRect($toolbar.H,[ref]$tr)
    $vw=$client.Right;$vh=$client.Bottom-$tr.Bottom
    $fit=[Math]::Min($vw/160.0,$vh/120.0);$displayW=[int][Math]::Round(160*$fit);$displayH=[int][Math]::Round(120*$fit)
    $viewX=[int][Math]::Floor(($vw-$displayW)/2.0);$viewY=$tr.Bottom+[int][Math]::Floor(($vh-$displayH)/2.0)
    Click-Button $main.H 'Verpixeln (B) – keine sichere Schwärzung'
    [void](Wait-Value {Get-Main|?{$_.Title -like '*Werkzeug: Verpixeln*'}} 'Blur tool activation')
    Assert-EditorTitle 'Verpixeln' $false 'filename and clean state after tool change'
    Drag-Image 24 20 120 90
    Assert-EditorTitle 'Verpixeln' $true 'filename and dirty state after edit'
    Click-Button $main.H 'Auswahl (V)'
    Assert-EditorTitle 'Auswahl' $true 'filename and dirty marker retained after tool change'
    Click-Button $main.H 'Weichzeichnen (W) – keine sichere Schwärzung'
    Assert-EditorTitle 'Weichzeichnen' $true 'German blur name and dirty marker after second tool change'
    Save-Image 'pixelation export'
    Assert-Pixelated (Read-Image) 'pixelation export'
    $savedHash=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash
    if($savedHash -ceq $originalHash){throw 'SAVE_DID_NOT_CHANGE_IMAGE: output hash still matches the untouched fixture.'}
    $main=Wait-Value {Get-Main|Where-Object {-not $_.Title.StartsWith('* ')}} 'clean title after successful save'
    Click-Button $main.H 'Verpixeln (B) – keine sichere Schwärzung'
    Assert-EditorTitle 'Verpixeln' $false 'filename and saved clean state retained after tool change'
    Click-Button $main.H 'Zurück'
    [void](Wait-Value {Get-Main|Where-Object {$_.Title.StartsWith('* ')}} 'dirty title after undo')
    Click-Button $main.H 'Wiederholen'
    $main=Wait-Value {Get-Main|Where-Object {-not $_.Title.StartsWith('* ')}} 'clean title after redo'
    if((Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash -cne $savedHash){throw 'REDO_CHANGED_SAVED_FILE: redo to saved history state altered the already saved file.'}
    $editorHandle=$main.H
    if(-not [ShotKitSaveHistorySmoke.N]::PostMessageW($editorHandle,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'EDITOR_CLOSE_POST_FAILED: WM_CLOSE could not be posted to the test-owned editor.'}
    $exitDeadline=[DateTime]::UtcNow.AddSeconds(8)
    while([DateTime]::UtcNow -lt $exitDeadline -and [ShotKitSaveHistorySmoke.N]::IsWindow($editorHandle)){
        $dialogs=@([ShotKitSaveHistorySmoke.N]::Windows($process.Id)|Where-Object {$_.Visible -and $_.ClassName -eq '#32770'})
        if($dialogs.Count -gt 0){$startup=@($dialogs|Where-Object {$_.Title -eq 'Willkommen bei ShotKit'});if($startup.Count -gt 0){throw "STARTUP_NOTICE_REAPPEARED_DURING_CLOSE: $($startup.Title -join ', ')"};throw "UNEXPECTED_CLOSE_DIALOG: $($dialogs.Title -join ', ')"}
        Start-Sleep -Milliseconds 100
    }
    if([ShotKitSaveHistorySmoke.N]::IsWindow($editorHandle)){throw 'EDITOR_CLOSE_TIMEOUT: the test-owned editor HWND remained after WM_CLOSE.'}
    $remainingDialogs=@([ShotKitSaveHistorySmoke.N]::Windows($process.Id)|Where-Object {$_.Visible -and $_.ClassName -eq '#32770'})
    if($remainingDialogs.Count -gt 0){$startup=@($remainingDialogs|Where-Object {$_.Title -eq 'Willkommen bei ShotKit'});if($startup.Count -gt 0){throw "STARTUP_NOTICE_REAPPEARED_DURING_CLOSE: $($startup.Title -join ', ')"};throw "UNEXPECTED_CLOSE_DIALOG: $($remainingDialogs.Title -join ', ')"}
    Write-Output "PASS: opened filename, German tool names and clean/dirty marker survived tool changes; edit changed and saved the opened image; undo showed the title star; redo returned to the clean saved state without a star; the editor HWND closed without an owned dialog (saved SHA256 $savedHash)."
}
catch { $failure=$_ }
finally {
    if($mouseDown){try{[ShotKitSaveHistorySmoke.N]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($process -and -not $process.HasExited){try{$process.Kill();if(-not $process.WaitForExit(5000)){$cleanupFailure='Owned ShotKit process remained alive after bounded kill wait.'}}catch{$cleanupFailure=$_.Exception.Message}}
    if($null -ne $process){try{$processExited=$process.HasExited}catch{$processExited=$false};$process.Dispose()}
    $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
    if($processExited -and (Test-Path -LiteralPath $root)){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($cleanupFailure){throw "CLEANUP_FAILED: $cleanupFailure. Temporary evidence retained at '$root'."}
if($null -ne $failure){throw $failure}

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
namespace ShotKitQuitSecuritySmoke {
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
  [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint command);
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
  public static uint ThreadId(IntPtr h) { uint p; return GetWindowThreadProcessId(h,out p); }
  public static IntPtr Owner(IntPtr h) { return GetWindow(h,4); }
  public static Win[] Windows(int pid) { var a=new List<Win>(); EnumProc cb=(h,p)=>{ if(Pid(h)==(uint)pid) a.Add(new Win{H=h,ClassName=Cls(h),Title=Txt(h),Visible=IsWindowVisible(h)}); return true; }; EnumWindows(cb,IntPtr.Zero); GC.KeepAlive(cb); return a.ToArray(); }
  public static Win[] Children(IntPtr parent) { var a=new List<Win>(); EnumProc cb=(h,p)=>{ a.Add(new Win{H=h,ClassName=Cls(h),Title=Txt(h),Visible=IsWindowVisible(h)}); return true; }; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return a.ToArray(); }
  public static Btn[] Buttons(IntPtr parent) { var a=new List<Btn>(); EnumProc cb=(h,p)=>{ if(IsWindowVisible(h) && Cls(h).Equals("Button",StringComparison.OrdinalIgnoreCase)) a.Add(new Btn{H=h,Text=Txt(h)}); return true; }; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return a.ToArray(); }
  public static void SendBarrier(IntPtr h, uint message) { IntPtr result; if(!SendMessageTimeoutW(h,message,IntPtr.Zero,IntPtr.Zero,0x0002,3000,out result)) throw new TimeoutException("Message delivery failed or timed out: " + message); }
  public static void Click(IntPtr h) { if(h==IntPtr.Zero) throw new Exception("Null button HWND"); IntPtr result; if(!SendMessageTimeoutW(h,0x00F5,IntPtr.Zero,IntPtr.Zero,0x0002,3000,out result)) throw new TimeoutException("BM_CLICK did not complete within 3 seconds."); }
  public static string InputDesktopName() { IntPtr d=OpenInputDesktop(0,false,0x0001); if(d==IntPtr.Zero)return null; try { var s=new StringBuilder(256);uint needed;if(!GetUserObjectInformationW(d,2,s,(uint)(s.Capacity*2),out needed))return null;return s.ToString(); } finally { CloseDesktop(d); } }
  public static bool ChildRect(IntPtr parent,IntPtr child,out Rect r) { r=new Rect(); if(!GetWindowRect(child,out r))return false; var a=new Point{X=r.Left,Y=r.Top}; var b=new Point{X=r.Right,Y=r.Bottom}; if(!ScreenToClient(parent,ref a)||!ScreenToClient(parent,ref b))return false; r=new Rect{Left=a.X,Top=a.Y,Right=b.X,Bottom=b.Y}; return true; }
 }
}
'@

    $root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Quit-Security-Smoke-' + [Guid]::NewGuid().ToString('N'))
$oldAppData=$env:APPDATA; $oldLocalAppData=$env:LOCALAPPDATA
$process=$null; $mouseDown=$false; $failure=$null; $cleanupFailure=$null; $processExited=$true
$inputPath=Join-Path $root 'fixture.png'
$envApp=Join-Path $root 'appdata'; $envLocal=Join-Path $root 'localappdata'

function Get-Main {
    if ($null -eq $process -or $process.HasExited) { return $null }
    [ShotKitQuitSecuritySmoke.N]::Windows($process.Id) | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' } | Select-Object -First 1
}
function Wait-Value([scriptblock]$Probe,[string]$What,[int]$Seconds=$TimeoutSeconds) {
    $end=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $end) { if($process -and $process.HasExited){throw "PROCESS_EXITED before $What (code $($process.ExitCode))."}; $v=&$Probe; if($null -ne $v -and $v){return $v}; Start-Sleep -Milliseconds 100 }
    throw "TIMEOUT waiting for $What"
}
function Click-Button([IntPtr]$Parent,[string]$Text) {
    $b=[ShotKitQuitSecuritySmoke.N]::Buttons($Parent) | Where-Object { ($_.Text -replace '&','').Trim() -eq $Text } | Select-Object -First 1
    if($null -eq $b){$all=([ShotKitQuitSecuritySmoke.N]::Buttons($Parent)|%{$_.Text}) -join ', '; throw "BUTTON_NOT_FOUND '$Text'; found [$all]"}
    [ShotKitQuitSecuritySmoke.N]::Click($b.H)
}
function Assert-QuitDialog([IntPtr]$DialogHandle) {
    # quit_application(PRIMARY) calls confirm_replace(editor), whose
    # MessageBoxW owner and ModalGuard are the dirty editor, not PRIMARY.
    $owner=[ShotKitQuitSecuritySmoke.N]::Owner($DialogHandle)
    if($owner -ne $editorHandle){throw "UNSAVED_DIALOG_NOT_OWNED_BY_EDITOR: owner=$owner editor=$editorHandle primary=$($primary.H)"}
    $thread=[ShotKitQuitSecuritySmoke.N]::ThreadId($primary.H)
    if($thread -eq 0 -or [ShotKitQuitSecuritySmoke.N]::ThreadId($editorHandle) -ne $thread -or [ShotKitQuitSecuritySmoke.N]::ThreadId($DialogHandle) -ne $thread){throw 'QUIT_DIALOG_NOT_ON_PRIMARY_UI_THREAD'}
    if([ShotKitQuitSecuritySmoke.N]::IsWindowEnabled($editorHandle) -or -not [ShotKitQuitSecuritySmoke.N]::IsWindowEnabled($DialogHandle)){throw 'QUIT_DIALOG_NOT_MODAL: expected disabled editor and enabled confirmation'}
}
function Drag-Image([int]$x1,[int]$y1,[int]$x2,[int]$y2) {
    $pt=[Activator]::CreateInstance([ShotKitQuitSecuritySmoke.Point]); if(-not [ShotKitQuitSecuritySmoke.N]::ClientToScreen($main.H,[ref]$pt)){throw 'ClientToScreen failed'}
    $sx=$pt.X+[int][Math]::Round($viewX+($x1+0.5)*$displayW/160,[MidpointRounding]::AwayFromZero)
    $sy=$pt.Y+[int][Math]::Round($viewY+($y1+0.5)*$displayH/120,[MidpointRounding]::AwayFromZero)
    $ex=$pt.X+[int][Math]::Round($viewX+($x2+0.5)*$displayW/160,[MidpointRounding]::AwayFromZero)
    $ey=$pt.Y+[int][Math]::Round($viewY+($y2+0.5)*$displayH/120,[MidpointRounding]::AwayFromZero)
    if(-not [ShotKitQuitSecuritySmoke.N]::SetForegroundWindow($main.H)){throw 'EDITOR_FOCUS_FAILED'}
    if(-not [ShotKitQuitSecuritySmoke.N]::SetCursorPos($sx,$sy)){throw 'Could not position drag start'}
    $p=[Activator]::CreateInstance([ShotKitQuitSecuritySmoke.Point]);[void][ShotKitQuitSecuritySmoke.N]::GetCursorPos([ref]$p)
    if([ShotKitQuitSecuritySmoke.N]::Pid([ShotKitQuitSecuritySmoke.N]::WindowFromPoint($p)) -ne [uint32]$process.Id){throw 'INPUT_TARGET_MISMATCH at drag start'}
    [ShotKitQuitSecuritySmoke.N]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true;Start-Sleep -Milliseconds 100
    [void][ShotKitQuitSecuritySmoke.N]::SetCursorPos($ex,$ey);Start-Sleep -Milliseconds 200
    [ShotKitQuitSecuritySmoke.N]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
}

try {
    $inputDesktop=[ShotKitQuitSecuritySmoke.N]::InputDesktopName()
    if($inputDesktop -cne 'Default'){throw "DEFAULT_DESKTOP_REQUIRED: active input desktop is '$inputDesktop'."}
    New-Item -ItemType Directory -Path $root,$envApp,$envLocal -Force | Out-Null
    $fixture=[Drawing.Bitmap]::new(160,120,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try { for($y=0;$y -lt 120;$y++){for($x=0;$x -lt 160;$x++){ $c=if((( [int]($x/4)+[int]($y/4))%2)-eq 0){[Drawing.Color]::FromArgb(255,240,20,30)}else{[Drawing.Color]::FromArgb(255,15,40,230)}; $fixture.SetPixel($x,$y,$c) }}; $fixture.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png) } finally {$fixture.Dispose()}
    $env:APPDATA=$envApp;$env:LOCALAPPDATA=$envLocal
    try {$process=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru} finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    $main=Wait-Value {Get-Main} 'owned editor window'
    [void][ShotKitQuitSecuritySmoke.N]::ShowWindow($main.H,3);[void][ShotKitQuitSecuritySmoke.N]::SetForegroundWindow($main.H)
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $process -GetWindows { [ShotKitQuitSecuritySmoke.N]::Windows($process.Id) }
    $main=Wait-Value {Get-Main} 'ready editor'
    $client=[Activator]::CreateInstance([ShotKitQuitSecuritySmoke.Rect]);[void][ShotKitQuitSecuritySmoke.N]::GetClientRect($main.H,[ref]$client)
    $toolbar=[ShotKitQuitSecuritySmoke.N]::Children($main.H)|?{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'}|select -First 1
    if($null -eq $toolbar){throw 'EDITOR_TOOLBAR_MISSING'}
    $tr=[Activator]::CreateInstance([ShotKitQuitSecuritySmoke.Rect]);[void][ShotKitQuitSecuritySmoke.N]::GetClientRect($toolbar.H,[ref]$tr)
    $vw=$client.Right;$vh=$client.Bottom-$tr.Bottom
    $fit=[Math]::Min($vw/160.0,$vh/120.0);$displayW=[int][Math]::Round(160*$fit);$displayH=[int][Math]::Round(120*$fit)
    $viewX=[int][Math]::Floor(($vw-$displayW)/2.0);$viewY=$tr.Bottom+[int][Math]::Floor(($vh-$displayH)/2.0)
    Click-Button $main.H 'Verpixeln (B) – keine sichere Schwärzung'
    [void](Wait-Value {Get-Main|?{$_.Title -like '*Werkzeug: Verpixeln*'}} 'Blur tool activation')
    Drag-Image 24 20 120 90
    $main=Wait-Value {Get-Main|Where-Object {$_.Title.StartsWith('* ')}} 'dirty editor title'
    $editorHandle=$main.H
    $primary=Wait-Value { [ShotKitQuitSecuritySmoke.N]::Windows($process.Id)|Where-Object {$_.ClassName -eq 'ShotKitRustPrototype'}|Select-Object -First 1 } 'owned primary window'
    # Synchronous delivery from this separate PowerShell process proves the
    # foreign private message reached the UI thread before assertions run.
    [ShotKitQuitSecuritySmoke.N]::SendBarrier($primary.H, 0x8005)
    Start-Sleep -Milliseconds 500
    if($process.HasExited -or -not [ShotKitQuitSecuritySmoke.N]::IsWindow($primary.H) -or -not [ShotKitQuitSecuritySmoke.N]::IsWindow($editorHandle)){throw 'FOREIGN_QUIT_DESTROYED_WINDOW: unsolicited WM_FINISH_QUIT ended the process or destroyed a window.'}
    $main=Get-Main
    if($null -eq $main -or -not $main.Title.StartsWith('* ')){throw 'DIRTY_DOCUMENT_NOT_PRESERVED after foreign WM_FINISH_QUIT.'}
    if(@([ShotKitQuitSecuritySmoke.N]::Windows($process.Id)|Where-Object {$_.Visible -and $_.ClassName -eq '#32770'}).Count -ne 0){throw 'UNEXPECTED_DIALOG after foreign WM_FINISH_QUIT.'}

    # The normal primary-window close must still ask about unsaved edits.
    if(-not [ShotKitQuitSecuritySmoke.N]::PostMessageW($primary.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'QUIT_POST_FAILED'}
    $dialog=Wait-Value { [ShotKitQuitSecuritySmoke.N]::Windows($process.Id)|Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq 'ShotKit'}|Select-Object -First 1 } 'unsaved changes confirmation'
    Assert-QuitDialog $dialog.H
    # The editor-owned MessageBox runs a nested loop on PRIMARY's UI thread.
    # Send to PRIMARY itself, then recheck modality: the foreign synchronous
    # barrier must complete while the real unsaved confirmation is still open.
    # On this WM_CLOSE path PRIMARY has no ModalGuard; FINISH_QUIT_READY=false
    # must prevent the unsolicited completion message from destroying it.
    [ShotKitQuitSecuritySmoke.N]::SendBarrier($primary.H, 0x8005)
    Start-Sleep -Milliseconds 300
    if($process.HasExited -or -not [ShotKitQuitSecuritySmoke.N]::IsWindow($primary.H) -or -not [ShotKitQuitSecuritySmoke.N]::IsWindow($editorHandle) -or -not [ShotKitQuitSecuritySmoke.N]::IsWindow($dialog.H)){throw 'MODAL_FOREIGN_QUIT_DESTROYED_WINDOW: foreign WM_FINISH_QUIT ended the process or destroyed PRIMARY, editor, or its modal dialog.'}
    Assert-QuitDialog $dialog.H
    $main=Get-Main
    if($null -eq $main -or -not $main.Title.StartsWith('* ')){throw 'DIRTY_DOCUMENT_NOT_PRESERVED after modal foreign WM_FINISH_QUIT.'}
    $cancel=[ShotKitQuitSecuritySmoke.N]::Buttons($dialog.H)|Where-Object {($_.Text -replace '&','').Trim() -in @('Cancel','Abbrechen')}|Select-Object -First 1
    if($null -eq $cancel){throw 'CANCEL_BUTTON_NOT_FOUND'}
    [ShotKitQuitSecuritySmoke.N]::Click($cancel.H)
    $main=Wait-Value {Get-Main|Where-Object {$_.Title.StartsWith('* ')}} 'dirty document after cancel'
    if(-not [ShotKitQuitSecuritySmoke.N]::IsWindow($primary.H)){throw 'PRIMARY_LOST_AFTER_CANCEL'}
    if(-not [ShotKitQuitSecuritySmoke.N]::PostMessageW($primary.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'SECOND_QUIT_POST_FAILED'}
    $dialog=Wait-Value { [ShotKitQuitSecuritySmoke.N]::Windows($process.Id)|Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq 'ShotKit'}|Select-Object -First 1 } 'second unsaved changes confirmation'
    Assert-QuitDialog $dialog.H
    $discard=[ShotKitQuitSecuritySmoke.N]::Buttons($dialog.H)|Where-Object {($_.Text -replace '&','').Trim() -in @('No','Nein')}|Select-Object -First 1
    if($null -eq $discard){throw 'DISCARD_BUTTON_NOT_FOUND'}
    [ShotKitQuitSecuritySmoke.N]::Click($discard.H)
    if(-not $process.WaitForExit($TimeoutSeconds*1000)){throw 'LEGITIMATE_QUIT_TIMEOUT'}
    if($process.ExitCode -ne 0){throw "LEGITIMATE_QUIT_FAILED: exit code $($process.ExitCode)"}
    Write-Output 'PASS: foreign WM_FINISH_QUIT preserved the dirty document both normally and during editor-owned modal confirmation on PRIMARY''s UI thread; cancel preserved it, and discard exited successfully.'

}
catch { $failure=$_ }
finally {
    if($mouseDown){try{[ShotKitQuitSecuritySmoke.N]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($process -and -not $process.HasExited){try{$process.Kill();if(-not $process.WaitForExit(5000)){$cleanupFailure='Owned ShotKit process remained alive after bounded kill wait.'}}catch{$cleanupFailure=$_.Exception.Message}}
    if($null -ne $process){try{$processExited=$process.HasExited}catch{$processExited=$false};$process.Dispose()}
    $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
    if($processExited -and (Test-Path -LiteralPath $root)){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($cleanupFailure){throw "CLEANUP_FAILED: $cleanupFailure. Temporary evidence retained at '$root'."}
if($null -ne $failure){throw $failure}

[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$ExePath,
    [ValidateRange(10,120)][int]$StartupTimeoutSeconds=30,
    [ValidateRange(5,60)][int]$ActionTimeoutSeconds=15
)
$ErrorActionPreference='Stop'
if($PSVersionTable.PSVersion.Major -lt 7){throw 'PowerShell 7 (pwsh) is required.'}
if(($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive){throw 'INTERACTIVE_WINDOWS_REQUIRED: ephemeral interactive runner or dedicated opt-in session required.'}
if(-not(Test-Path -LiteralPath $ExePath -PathType Leaf)){throw "ShotKit EXE not found: $ExePath"}
$ExePath=(Resolve-Path -LiteralPath $ExePath).Path
$imageName=[IO.Path]::GetFileNameWithoutExtension($ExePath)
if(@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count){throw "PREEXISTING_PROCESS: refusing to use/change '$imageName'."}
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitZoomSmoke {
 public struct Rect { public int Left,Top,Right,Bottom; }
 public struct Point { public int X,Y; }
 public sealed class Win { public IntPtr H; public int Pid; public string ClassName,Title; public bool Visible; }
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
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,uint x,uint y,uint data,UIntPtr extra);
  [DllImport("user32.dll")] public static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll",SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,UIntPtr w,IntPtr l,uint flags,uint timeout,out UIntPtr result);
  [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
  [DllImport("user32.dll")] static extern IntPtr OpenInputDesktop(uint f,bool inherit,uint access);
  [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr h);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern bool GetUserObjectInformationW(IntPtr h,int i,StringBuilder s,uint n,out uint needed);
  public static string InputDesktop(){IntPtr h=OpenInputDesktop(0,false,1);if(h==IntPtr.Zero)return null;try{var s=new StringBuilder(256);uint n;return GetUserObjectInformationW(h,2,s,512,out n)?s.ToString():null;}finally{CloseDesktop(h);}}
  static string Read(IntPtr h,bool c){var s=new StringBuilder(512);if(c)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString();}
  public static Win[] Windows(int pid){var a=new List<Win>();EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)pid)a.Add(new Win{H=h,Pid=(int)p,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static int Owner(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return (int)p;}
  public static void Key(byte k){keybd_event(k,0,0,UIntPtr.Zero);keybd_event(k,0,2,UIntPtr.Zero);}
  public static void CtrlKey(byte k){try{keybd_event(0x11,0,0,UIntPtr.Zero);keybd_event(k,0,0,UIntPtr.Zero);}finally{keybd_event(k,0,2,UIntPtr.Zero);keybd_event(0x11,0,2,UIntPtr.Zero);}}
  public static void CtrlNumpad(byte k){try{keybd_event(0x11,0,0,UIntPtr.Zero);keybd_event(k,0,0,UIntPtr.Zero);}finally{keybd_event(k,0,2,UIntPtr.Zero);keybd_event(0x11,0,2,UIntPtr.Zero);}}
  public static void Wheel(int x,int y,int delta){SetCursorPos(x,y);mouse_event(0x0800,0,0,(uint)delta,UIntPtr.Zero);}
  public static void ShiftWheelMessage(IntPtr h,int x,int y,int delta){
   if(!IsWindow(h))throw new InvalidOperationException("MESSAGE_TARGET_NOT_WINDOW");
   if(delta!=120&&delta!=-120)throw new ArgumentOutOfRangeException("delta");
   if((GetAsyncKeyState(0x10)&0x8000)!=0)throw new InvalidOperationException("SHIFT_PHYSICALLY_DOWN_BEFORE_MESSAGE");
   long packed=((long)(ushort)(short)y<<16)|(ushort)(short)x;
   ulong wp=((ulong)(ushort)(short)delta<<16)|0x0004UL; // MK_SHIFT in low word; signed delta bits in high word.
   UIntPtr result;UIntPtr wparam=UIntPtr.Size==8?new UIntPtr(wp):new UIntPtr((uint)wp);IntPtr lparam=IntPtr.Size==8?new IntPtr(packed):new IntPtr(unchecked((int)packed));
   IntPtr sent=SendMessageTimeoutW(h,0x020A,wparam,lparam,0x0001|0x0002|0x0020,1000,out result);
   if(sent==IntPtr.Zero)throw new InvalidOperationException("SHIFT_WHEEL_SENDMESSAGE_TIMEOUT_OR_FAILURE");
   if((GetAsyncKeyState(0x10)&0x8000)!=0)throw new InvalidOperationException("SHIFT_PHYSICALLY_DOWN_AFTER_MESSAGE");
  }
 }
}
'@
$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Zoom-Smoke-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$fixture=Join-Path $root 'zoom-fixture.png'
$diagnosticLog=Join-Path $root 'wheel-diagnostics.log'
$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA;$oldDiagnostic=$env:SHOTKIT_UIA_DIAGNOSTICS_PATH;$proc=$null;$failure=$null;$mouseDown=$false
function Owned-Windows {if(!$proc -or $proc.HasExited){return @()};@([ShotKitZoomSmoke.N]::Windows($proc.Id))}
function Wait-For([scriptblock]$Probe,[string]$What,[int]$Seconds=$ActionTimeoutSeconds){$end=[DateTime]::UtcNow.AddSeconds($Seconds);while([DateTime]::UtcNow -lt $end){if($proc -and $proc.HasExited){throw "PROCESS_EXITED waiting for $What (code $($proc.ExitCode))."};$v=&$Probe;if($null -ne $v -and $v){return $v};Start-Sleep -Milliseconds 100};throw "TIMEOUT waiting for $What."}
function Focus-Editor($H){if([ShotKitZoomSmoke.N]::Owner($H) -ne $proc.Id){throw 'EDITOR_OWNER_MISMATCH'};[void][ShotKitZoomSmoke.N]::SetForegroundWindow($H);[void](Wait-For {[ShotKitZoomSmoke.N]::GetForegroundWindow() -eq $H} 'exact editor foreground' 5)}
function New-Fixture { $b=[Drawing.Bitmap]::new(320,240,[Drawing.Imaging.PixelFormat]::Format24bppRgb);try{$g=[Drawing.Graphics]::FromImage($b);try{$g.Clear([Drawing.Color]::FromArgb(12,12,12));$g.FillRectangle([Drawing.Brushes]::Magenta,20,20,280,200);$g.FillRectangle([Drawing.Brushes]::Lime,35,35,35,35);$g.FillRectangle([Drawing.Brushes]::Cyan,250,170,30,30)}finally{$g.Dispose()};$b.Save($fixture,[Drawing.Imaging.ImageFormat]::Png)}finally{$b.Dispose()}}
function Get-VisibleBounds($H){$r=[Activator]::CreateInstance([ShotKitZoomSmoke.Rect]);if(-not[ShotKitZoomSmoke.N]::GetWindowRect($H,[ref]$r)){throw 'WINDOW_RECT_FAILED'};$vx=[ShotKitZoomSmoke.N]::GetSystemMetrics(76);$vy=[ShotKitZoomSmoke.N]::GetSystemMetrics(77);$x0=[Math]::Max($vx,$r.Left);$y0=[Math]::Max($vy,$r.Top);$x1=[Math]::Min($vx+[ShotKitZoomSmoke.N]::GetSystemMetrics(78),$r.Right);$y1=[Math]::Min($vy+[ShotKitZoomSmoke.N]::GetSystemMetrics(79),$r.Bottom);if($x1 -le $x0 -or $y1 -le $y0){throw "EDITOR_OFFSCREEN: $($r.Left),$($r.Top),$($r.Right),$($r.Bottom)"};$bmp=[Drawing.Bitmap]::new($x1-$x0,$y1-$y0);$g=[Drawing.Graphics]::FromImage($bmp);try{$g.CopyFromScreen($x0,$y0,0,0,$bmp.Size)}finally{$g.Dispose()};try{$minX=$bmp.Width;$minY=$bmp.Height;$maxX=-1;$maxY=-1;$count=0;for($y=0;$y -lt $bmp.Height;$y++){for($x=0;$x -lt $bmp.Width;$x++){$c=$bmp.GetPixel($x,$y);if($c.R -gt 180 -and $c.B -gt 180 -and $c.G -lt 100){$minX=[Math]::Min($minX,$x);$minY=[Math]::Min($minY,$y);$maxX=[Math]::Max($maxX,$x);$maxY=[Math]::Max($maxY,$y);$count++}}};if($count -lt 1000){throw "VISIBLE_IMAGE_NOT_FOUND: magenta fixture pixels=$count; ensure unobscured interactive desktop."};return [pscustomobject]@{Width=($maxX-$minX+1);Height=($maxY-$minY+1);Pixels=$count;X=$minX+$x0;Y=$minY+$y0}}finally{$bmp.Dispose()}}
function Get-LimeMarker($H){$r=[Activator]::CreateInstance([ShotKitZoomSmoke.Rect]);if(-not[ShotKitZoomSmoke.N]::GetWindowRect($H,[ref]$r)){throw 'WINDOW_RECT_FAILED'};$vx=[ShotKitZoomSmoke.N]::GetSystemMetrics(76);$vy=[ShotKitZoomSmoke.N]::GetSystemMetrics(77);$x0=[Math]::Max($vx,$r.Left);$y0=[Math]::Max($vy,$r.Top);$x1=[Math]::Min($vx+[ShotKitZoomSmoke.N]::GetSystemMetrics(78),$r.Right);$y1=[Math]::Min($vy+[ShotKitZoomSmoke.N]::GetSystemMetrics(79),$r.Bottom);$bmp=[Drawing.Bitmap]::new($x1-$x0,$y1-$y0);$g=[Drawing.Graphics]::FromImage($bmp);try{$g.CopyFromScreen($x0,$y0,0,0,$bmp.Size)}finally{$g.Dispose()};try{$sx=0;$sy=0;$n=0;for($y=0;$y -lt $bmp.Height;$y++){for($x=0;$x -lt $bmp.Width;$x++){$c=$bmp.GetPixel($x,$y);if($c.G -gt 190 -and $c.R -lt 100 -and $c.B -lt 100){$sx+=$x;$sy+=$y;$n++}}};if($n -lt 100){throw "FIXTURE_LIME_MARKER_NOT_VISIBLE: pixels=$n"};return [pscustomobject]@{X=[int][Math]::Round($sx/$n)+$x0;Y=[int][Math]::Round($sy/$n)+$y0;Pixels=$n}}finally{$bmp.Dispose()}}
function Assert-OwnedInputTarget($H,$X,$Y){if([ShotKitZoomSmoke.N]::Owner($H) -ne $proc.Id -or [ShotKitZoomSmoke.N]::GetForegroundWindow() -ne $H){throw 'INPUT_TARGET_NOT_OWNED_FOREGROUND'};$r=[Activator]::CreateInstance([ShotKitZoomSmoke.Rect]);if(-not[ShotKitZoomSmoke.N]::GetWindowRect($H,[ref]$r) -or $X -lt $r.Left -or $X -ge $r.Right -or $Y -lt $r.Top -or $Y -ge $r.Bottom){throw 'INPUT_CURSOR_OUTSIDE_OWNED_EDITOR'};if(-not[ShotKitZoomSmoke.N]::SetCursorPos($X,$Y)){throw 'CURSOR_POSITION_FAILED'};$p=[Activator]::CreateInstance([ShotKitZoomSmoke.Point]);$p.X=$X;$p.Y=$Y;if([ShotKitZoomSmoke.N]::WindowFromPoint($p) -ne $H){throw 'INPUT_CURSOR_TARGET_NOT_OWNED_EDITOR'}}
# Keep the modifier down until the editor has processed the real OS wheel input.
# The diagnostic is emitted inside the WM_MOUSEWHEEL handler after applying the action;
# the independent on-screen pixel checks below still determine whether it worked.
function Send-ModifiedWheel($H,[int]$X,[int]$Y,[int]$Delta,[byte]$Key,[int]$Flag){
 Focus-Editor $H;Assert-OwnedInputTarget $H $X $Y
 if(([ShotKitZoomSmoke.N]::GetAsyncKeyState($Key) -band 0x8000) -ne 0){throw "MODIFIER_ALREADY_DOWN: key=$Key"}
 $before=@(if(Test-Path -LiteralPath $diagnosticLog){Get-Content -LiteralPath $diagnosticLog | Where-Object {$_ -match '^wheel hwnd=\d+ flags=\d+ delta=-?\d+ mode=[012] '}}).Count
 try {
  [ShotKitZoomSmoke.N]::keybd_event($Key,0,0,[UIntPtr]::Zero)
  [void](Wait-For {([ShotKitZoomSmoke.N]::GetAsyncKeyState($Key) -band 0x8000) -ne 0} 'modifier key down' 5)
  Assert-OwnedInputTarget $H $X $Y
  [ShotKitZoomSmoke.N]::Wheel($X,$Y,$Delta)
  $line=Wait-For { $lines=@(if(Test-Path -LiteralPath $diagnosticLog){Get-Content -LiteralPath $diagnosticLog | Where-Object {$_ -match '^wheel hwnd=\d+ flags=\d+ delta=-?\d+ mode=[012] '}});if($lines.Count -gt $before){$lines[$before]} } 'wheel event processed by editor' 5
  $script:lastPanWheelDiagnostic=$line
  if($line -notmatch '^wheel hwnd=(\d+) flags=(\d+) delta=(-?\d+) mode=([012]) '){throw "WHEEL_DIAGNOSTIC_INVALID: $line"}
  $expectedMode=if(($Flag -band 0x0004) -ne 0){1}else{0}
  if([UInt64]$Matches[1] -ne [UInt64]($H.ToInt64()) -or [int]$Matches[3] -ne $Delta -or ([int]$Matches[2] -band $Flag) -ne $Flag -or [int]$Matches[4] -ne $expectedMode){throw "MODIFIED_WHEEL_DELIVERY_MISMATCH: $line"}
  return $line
 } finally {[ShotKitZoomSmoke.N]::keybd_event($Key,0,2,[UIntPtr]::Zero)}
}
function Get-WheelDiagnosticCount { @(if(Test-Path -LiteralPath $diagnosticLog){Get-Content -LiteralPath $diagnosticLog | Where-Object {$_ -match '^wheel hwnd=\d+ flags=\d+ delta=-?\d+ mode=[012] '}}).Count }
function Wait-WheelDiagnostic([int]$Before,$H,[int]$Delta,[int]$Flag,[int]$Mode){
 $line=Wait-For { $lines=@(if(Test-Path -LiteralPath $diagnosticLog){Get-Content -LiteralPath $diagnosticLog | Where-Object {$_ -match '^wheel hwnd=\d+ flags=\d+ delta=-?\d+ mode=[012] '}});if($lines.Count -gt $Before){$lines[$Before]} } 'wheel event processed by editor' 5
 $script:lastPanWheelDiagnostic=$line
 if($line -notmatch '^wheel hwnd=(\d+) flags=(\d+) delta=(-?\d+) mode=([012]) '){throw "WHEEL_DIAGNOSTIC_INVALID: $line"}
 if([UInt64]$Matches[1] -ne [UInt64]($H.ToInt64()) -or [int]$Matches[3] -ne $Delta -or ([int]$Matches[2] -band $Flag) -ne $Flag -or [int]$Matches[4] -ne $Mode){throw "PAN_WHEEL_DELIVERY_MISMATCH: $line"}
 return $line
}
function Send-PanWheel($H,[int]$Delta,[bool]$Shift,$PairStart,[bool]$ReturnToStart){
 Focus-Editor $H;$m=Get-LimeMarker $H;Assert-OwnedInputTarget $H $m.X $m.Y;$before=Get-WheelDiagnosticCount;$script:lastPanWheelDiagnostic=$null
 if($Shift){$line=Send-ModifiedWheel $H $m.X $m.Y $Delta 0x10 0x0004}
 else {[ShotKitZoomSmoke.N]::Wheel($m.X,$m.Y,$Delta);$line=Wait-WheelDiagnostic $before $H $Delta 0 2}
 $script:panEvidence.Diagnostics+=@($line)
 $axis=if($Shift){'X'}else{'Y'};$target=[int]$PairStart.$axis
 $probe=Wait-For {try{$observed=Get-LimeMarker $H;$script:panEvidence.LastMarker=$observed;if($ReturnToStart){[Math]::Abs([int]$observed.$axis-$target) -le 3}else{[int]$m.$axis-[int]$observed.$axis -ge 3}}catch{$false}} "visible $axis pan marker state after delta $Delta" 3
 return $script:panEvidence.LastMarker
}
function Send-ShiftWheelMessage($H,[int]$Delta,$PairStart,[bool]$ReturnToStart){
 if([ShotKitZoomSmoke.N]::Owner($H) -ne $proc.Id){throw 'MESSAGE_TARGET_OWNER_MISMATCH'};if(-not[ShotKitZoomSmoke.N]::IsWindow($H)){throw 'MESSAGE_TARGET_NOT_WINDOW'}
 $m=Get-LimeMarker $H;$before=Get-WheelDiagnosticCount;$script:lastPanWheelDiagnostic=$null;[ShotKitZoomSmoke.N]::ShiftWheelMessage($H,$m.X,$m.Y,$Delta);$line=Wait-WheelDiagnostic $before $H $Delta 0x0004 1;$script:panEvidence.Diagnostics+=@($line)
 $target=[int]$PairStart.X;$probe=Wait-For {try{$observed=Get-LimeMarker $H;$script:panEvidence.LastMarker=$observed;if($ReturnToStart){[Math]::Abs([int]$observed.X-$target) -le 3}else{[int]$m.X-[int]$observed.X -ge 3}}catch{$false}} "visible X pan marker state after direct message delta $Delta" 3
 return $script:panEvidence.LastMarker
}
function Assert-PanPair([string]$Name,$Start,$Moved,$Returned,[string]$Axis){$primary=if($Axis -eq 'Y'){'Y'}else{'X'};$other=if($Axis -eq 'Y'){'X'}else{'Y'};$step=[int]$Start.$primary-[int]$Moved.$primary;$returnError=[Math]::Abs([int]$Returned.$primary-[int]$Start.$primary);$crossMove=[Math]::Abs([int]$Moved.$other-[int]$Start.$other);$crossReturn=[Math]::Abs([int]$Returned.$other-[int]$Start.$other);if($step -lt 3 -or $returnError -gt 3 -or $crossMove -gt 3 -or $crossReturn -gt 3){throw "$Name visible marker motion failed: start=($($Start.X),$($Start.Y)) moved=($($Moved.X),$($Moved.Y)) returned=($($Returned.X),$($Returned.Y)); expected negative $Axis-only motion on wheel down and return within 3px."};Write-Output ("PASS_ACTION {0}: visible lime marker {1} step {2}px; opposite step returned within {3}px; cross-axis <= {4}px." -f $Name,$Axis,$step,$returnError,[Math]::Max($crossMove,$crossReturn))}
function Invoke-PanPair([string]$Name,$H,[bool]$Shift,[bool]$DirectMessage,[string]$Axis){
 $start=Get-LimeMarker $H;$script:panEvidence=[ordered]@{Name=$Name;Start=$start;Moved=$null;Returned=$null;LastMarker=$start;Diagnostics=@()}
 try {
  if($DirectMessage){$script:panEvidence.Moved=Send-ShiftWheelMessage $H -120 $start $false;$script:panEvidence.Returned=Send-ShiftWheelMessage $H 120 $start $true}
  else {$script:panEvidence.Moved=Send-PanWheel $H -120 $Shift $start $false;$script:panEvidence.Returned=Send-PanWheel $H 120 $Shift $start $true}
  Assert-PanPair $Name $start $script:panEvidence.Moved $script:panEvidence.Returned $Axis
 } catch {
  if($script:lastPanWheelDiagnostic -and $script:panEvidence.Diagnostics -notcontains $script:lastPanWheelDiagnostic){$script:panEvidence.Diagnostics+=@($script:lastPanWheelDiagnostic)}
  Write-Output "PAN_PAIR_FAILURE: $Name"
  Write-Output ("PAN_PAIR_MARKERS: start=({0},{1}) moved={2} returned={3} last={4}" -f $start.X,$start.Y,$(if($null -ne $script:panEvidence.Moved){"($($script:panEvidence.Moved.X),$($script:panEvidence.Moved.Y))"}else{'unavailable'}),$(if($null -ne $script:panEvidence.Returned){"($($script:panEvidence.Returned.X),$($script:panEvidence.Returned.Y))"}else{'unavailable'}),$(if($null -ne $script:panEvidence.LastMarker){"($($script:panEvidence.LastMarker.X),$($script:panEvidence.LastMarker.Y))"}else{'unavailable'}))
  Write-Output 'PAN_PAIR_HANDLER_DIAGNOSTICS:'
  if($script:panEvidence.Diagnostics.Count){$script:panEvidence.Diagnostics|ForEach-Object{Write-Output $_};if($script:panEvidence.Diagnostics.Count -lt 2){Write-Output 'second related handler diagnostic unavailable (event may not have reached handler)'}}else{Write-Output 'both related handler diagnostics unavailable (no wheel event was logged)'}
  throw
 }
}
function Measure-After([string]$Name,[scriptblock]$Action,$Before,$H){Focus-Editor $H;&$Action;Start-Sleep -Milliseconds 350;[void](Wait-For {try{$m=Get-VisibleBounds $H;($m.Width -ne $Before.Width -or $m.Height -ne $Before.Height)}catch{$false}} "$Name visible image extent change");$after=Get-VisibleBounds $H;if($after.Width -le $Before.Width -or $after.Height -le $Before.Height){throw "$Name did not enlarge visible image bounds: before=$($Before.Width)x$($Before.Height), after=$($after.Width)x$($after.Height)."};Write-Output ("PASS_ACTION {0}: visible magenta bounds {1}x{2} -> {3}x{4}" -f $Name,$Before.Width,$Before.Height,$after.Width,$after.Height);return $after}
function Assert-Smaller([string]$Name,$Before,$After){if($After.Width -ge $Before.Width -or $After.Height -ge $Before.Height){throw "$Name did not reduce visible image extent: before=$($Before.Width)x$($Before.Height), after=$($After.Width)x$($After.Height)."};Write-Output ("PASS_ACTION {0}: visible magenta bounds {1}x{2} -> {3}x{4}" -f $Name,$Before.Width,$Before.Height,$After.Width,$After.Height)}
try {
 if([ShotKitZoomSmoke.N]::InputDesktop() -ne 'Default'){throw "NO_INTERACTIVE_INPUT_DESKTOP: $([ShotKitZoomSmoke.N]::InputDesktop())"}
 New-Item -ItemType Directory -Path $appData,$localAppData -Force|Out-Null;New-Fixture
 $env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData
 $env:SHOTKIT_UIA_DIAGNOSTICS_PATH=$diagnosticLog
 try {$proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $fixture) -WorkingDirectory $root -PassThru} finally {$env:SHOTKIT_UIA_DIAGNOSTICS_PATH=$oldDiagnostic}
 $main=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} 'owned editor window' $StartupTimeoutSeconds
 if([ShotKitZoomSmoke.N]::Owner($main.H) -ne $proc.Id){throw 'EDITOR_OWNER_MISMATCH'}
 $notice=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'}|Select-Object -First 1} 'owned startup notice' 12
 if(-not[ShotKitZoomSmoke.N]::PostMessageW($notice.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'STARTUP_NOTICE_CLOSE_FAILED'}
 [void](Wait-For {-not[ShotKitZoomSmoke.N]::IsWindow($notice.H)} 'startup notice close' 8)
 $main=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} 'editor after notice' 8
 Focus-Editor $main.H;Start-Sleep -Milliseconds 500
 $fitBase=Get-VisibleBounds $main.H
 Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlKey(0x39);Start-Sleep -Milliseconds 350;$fitBase=Get-VisibleBounds $main.H
 Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlKey(0x30);Start-Sleep -Milliseconds 350;$original=Get-VisibleBounds $main.H;Assert-Smaller 'Ctrl+0 (top-row 0, original size from fit)' $fitBase $original
 Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlKey(0x39);Start-Sleep -Milliseconds 350;$fitAfter=Get-VisibleBounds $main.H;if($fitAfter.Width -le $original.Width -or $fitAfter.Height -le $original.Height){throw 'Ctrl+9 (top-row 9) did not restore the larger fitted image.'};Write-Output 'PASS_ACTION Ctrl+9 (top-row 9): visible image enlarged back to fit.'
 Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlNumpad(0x60);Start-Sleep -Milliseconds 350;$originalPad=Get-VisibleBounds $main.H;Assert-Smaller 'Ctrl+0 (NumPad 0, original size)' $fitAfter $originalPad
 Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlNumpad(0x69);Start-Sleep -Milliseconds 350;$fitPad=Get-VisibleBounds $main.H;if($fitPad.Width -le $originalPad.Width -or $fitPad.Height -le $originalPad.Height){throw 'Ctrl+9 (NumPad 9) did not enlarge the visible image back to fit.'};Write-Output 'PASS_ACTION Ctrl+9 (NumPad 9): visible image enlarged back to fit.'
 for($i=0;$i -lt 3;$i++){Focus-Editor $main.H;$zoomMarker=Get-LimeMarker $main.H;Assert-OwnedInputTarget $main.H $zoomMarker.X $zoomMarker.Y;[ShotKitZoomSmoke.N]::CtrlKey(0xBB);Start-Sleep -Milliseconds 150}
 Start-Sleep -Milliseconds 350
 Invoke-PanPair 'vertical wheel down/up' $main.H $false $false 'Y'
 # Repeat real Shift+wheel pairs: issue #42 was intermittent, so validate every
 # down/up cycle independently from on-screen marker screenshots and handler diagnostics.
 for($i=1;$i -le 5;$i++){Invoke-PanPair ("Shift+wheel down/up pair $i/5") $main.H $true $false 'X'}
 Invoke-PanPair 'WM_MOUSEWHEEL MK_SHIFT down/up (no Shift key)' $main.H $false $true 'X'
 Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlKey(0x30);Start-Sleep -Milliseconds 350;$zoomBase=Get-VisibleBounds $main.H
 $s=Measure-After 'Ctrl+Plus (OEM plus)' { [ShotKitZoomSmoke.N]::CtrlKey(0xBB) } $zoomBase $main.H
 $smaller=Get-VisibleBounds $main.H;Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlKey(0xBD);Start-Sleep -Milliseconds 350;$after=Get-VisibleBounds $main.H;Assert-Smaller 'Ctrl+Minus (OEM minus)' $smaller $after
 Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlNumpad(0x6B);Start-Sleep -Milliseconds 350;$afterPlus=Get-VisibleBounds $main.H;if($afterPlus.Width -le $after.Width -or $afterPlus.Height -le $after.Height){throw "Ctrl+NumPad Plus did not enlarge visible image: $($after.Width)x$($after.Height) -> $($afterPlus.Width)x$($afterPlus.Height)."};Write-Output "PASS_ACTION Ctrl+Plus (NumPad plus): visible image enlarged."
 Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlNumpad(0x6D);Start-Sleep -Milliseconds 350;$afterMinus=Get-VisibleBounds $main.H;Assert-Smaller 'Ctrl+Minus (NumPad minus)' $afterPlus $afterMinus
 Focus-Editor $main.H;[ShotKitZoomSmoke.N]::CtrlKey(0x30);Start-Sleep -Milliseconds 350;$wheelPoint=Get-VisibleBounds $main.H;Send-ModifiedWheel $main.H ($wheelPoint.X+[int]($wheelPoint.Width/2)) ($wheelPoint.Y+[int]($wheelPoint.Height/2)) 120 0x11 0x0008;Start-Sleep -Milliseconds 350;$wheelIn=Get-VisibleBounds $main.H;if($wheelIn.Width -le $wheelPoint.Width -or $wheelIn.Height -le $wheelPoint.Height){throw "Ctrl+mouse wheel up did not enlarge visible image: $($wheelPoint.Width)x$($wheelPoint.Height) -> $($wheelIn.Width)x$($wheelIn.Height)."};if([Math]::Abs($wheelIn.Width-$wheelPoint.Width*1.25) -gt 4 -or [Math]::Abs($wheelIn.Height-$wheelPoint.Height*1.25) -gt 4){throw "CTRL_WHEEL_STEP_MISMATCH: expected 1.25x, got $($wheelPoint.Width)x$($wheelPoint.Height) -> $($wheelIn.Width)x$($wheelIn.Height)."};Write-Output 'PASS_ACTION Ctrl+mouse wheel up: visible image enlarged by 1.25.'
 Send-ModifiedWheel $main.H ($wheelIn.X+[int]($wheelIn.Width/2)) ($wheelIn.Y+[int]($wheelIn.Height/2)) -120 0x11 0x0008;Start-Sleep -Milliseconds 350;$wheelOut=Get-VisibleBounds $main.H;Assert-Smaller 'Ctrl+mouse wheel down' $wheelIn $wheelOut
 Write-Output 'PASS: zoom shortcuts and wheel changed the fixture measured on-screen pixel bounds.'
} catch {$failure=$_} finally {
 if($proc -and -not $proc.HasExited){try{foreach($w in [ShotKitZoomSmoke.N]::Windows($proc.Id)){[void][ShotKitZoomSmoke.N]::PostMessageW($w.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)};if(-not $proc.WaitForExit(5000)){$proc.Kill();[void]$proc.WaitForExit(5000)}}catch{if($null -eq $failure){$failure=$_}}}
 if($proc -and -not $proc.HasExited -and $null -eq $failure){$failure='OWNED_PROCESS_CLEANUP_FAILED'}
 $mayRemoveRoot=($null -eq $proc -or $proc.HasExited)
 if($proc){$proc.Dispose()};$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData;$env:SHOTKIT_UIA_DIAGNOSTICS_PATH=$oldDiagnostic
 if($null -ne $failure -and (Test-Path -LiteralPath $diagnosticLog -PathType Leaf)){try{Write-Output 'WHEEL_DIAGNOSTICS (last 60 lines):';Get-Content -LiteralPath $diagnosticLog -Tail 60 | Where-Object {$_ -match '^wheel hwnd=\d+ flags=\d+ delta=-?\d+ mode=[012] pan_before=\(-?\d+, -?\d+\) zoom_before=[0-9.eE+-]+ pan_after=\(-?\d+, -?\d+\) zoom_after=[0-9.eE+-]+$'} | ForEach-Object {Write-Output $_}}catch{Write-Output 'WHEEL_DIAGNOSTICS_UNAVAILABLE'}}
 if($mayRemoveRoot -and (Test-Path -LiteralPath $root)){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}

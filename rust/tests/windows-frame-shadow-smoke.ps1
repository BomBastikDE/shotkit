[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$ExePath,
    [ValidateRange(15,120)][int]$StartupTimeoutSeconds=30,
    [ValidateRange(5,60)][int]$ActionTimeoutSeconds=20
)
$ErrorActionPreference='Stop'
if($PSVersionTable.PSVersion.Major -lt 7){throw 'PowerShell 7 (pwsh) is required.'}
if(($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive){throw 'INTERACTIVE_WINDOWS_REQUIRED: ephemeral interactive runner or dedicated opt-in session required.'}
if(-not(Test-Path -LiteralPath $ExePath -PathType Leaf)){throw "ShotKit EXE not found: $ExePath"}
$ExePath=(Resolve-Path -LiteralPath $ExePath).Path
$imageName=[IO.Path]::GetFileNameWithoutExtension($ExePath)
if(@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0){throw "PREEXISTING_PROCESS: '$imageName' is already running."}
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitFrameShadowSmoke {
 public sealed class Win { public IntPtr H; public int Pid; public string ClassName,Title; public bool Visible; }
 public sealed class Btn { public IntPtr H; public int Id; public string Text; public bool Visible; }
 public struct Point { public int X,Y; }
 public struct Rect { public int Left,Top,Right,Bottom; }
 public static class N {
  delegate bool EnumProc(IntPtr h,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc cb,IntPtr d);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int command);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr h,uint i);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern uint GetMenuStringW(IntPtr h,uint i,StringBuilder s,int n,uint flags);
  [DllImport("user32.dll")] static extern bool GetMenuItemRect(IntPtr h,IntPtr menu,uint i,out Rect r);
  [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] static extern void mouse_event(uint f,uint x,uint y,uint data,UIntPtr extra);
  static string Read(IntPtr h,bool cls){var s=new StringBuilder(cls?256:1024);if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString();}
  public static Win[] Windows(int pid){var a=new List<Win>();EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)pid)a.Add(new Win{H=h,Pid=(int)p,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static Btn[] Buttons(IntPtr parent){var a=new List<Btn>();EnumProc cb=(h,d)=>{if(Read(h,true).Equals("Button",StringComparison.OrdinalIgnoreCase))a.Add(new Btn{H=h,Id=GetDlgCtrlID(h),Text=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static int Owner(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return (int)p;}
  public static void ClickButton(IntPtr h){if(h==IntPtr.Zero||!IsWindow(h))throw new ArgumentException("Invalid button HWND.");SendMessageW(h,0xF5,IntPtr.Zero,IntPtr.Zero);}
  public static void ClickMenu(IntPtr popup,int pid,uint wantedId,string wantedText){
   if(Owner(popup)!=pid||Read(popup,true)!="#32768"||!IsWindowVisible(popup))throw new InvalidOperationException("Popup is not visible and owned by test process.");
   const uint MN_GETHMENU=0x1E1,MF_BYPOSITION=0x400;IntPtr menu=SendMessageW(popup,MN_GETHMENU,IntPtr.Zero,IntPtr.Zero);
   if(menu==IntPtr.Zero)throw new InvalidOperationException("MN_GETHMENU returned null.");
   int count=GetMenuItemCount(menu),match=-1;if(count<0)throw new InvalidOperationException("GetMenuItemCount failed.");var entries=new StringBuilder();
   for(uint i=0;i<(uint)count;i++){uint id=GetMenuItemID(menu,i);var s=new StringBuilder(512);GetMenuStringW(menu,i,s,s.Capacity,MF_BYPOSITION);entries.Append(" [").Append(id).Append(":'").Append(s).Append("']");if(id==wantedId&&String.Equals(s.ToString(),wantedText,StringComparison.Ordinal)){if(match>=0)throw new InvalidOperationException("Duplicate exact menu item.");match=(int)i;}}
   if(match<0)throw new InvalidOperationException("Exact menu item missing; entries="+entries);
   Rect r;if(!GetMenuItemRect(IntPtr.Zero,menu,(uint)match,out r)||r.Right<=r.Left||r.Bottom<=r.Top)throw new InvalidOperationException("GetMenuItemRect failed.");
   int x=r.Left+(r.Right-r.Left)/2,y=r.Top+(r.Bottom-r.Top)/2;if(!SetCursorPos(x,y))throw new InvalidOperationException("SetCursorPos failed.");
   IntPtr under=WindowFromPoint(new Point{X=x,Y=y});if(under!=popup||Owner(under)!=pid)throw new InvalidOperationException("Menu point is not over owned popup.");
   mouse_event(2,0,0,0,UIntPtr.Zero);mouse_event(4,0,0,0,UIntPtr.Zero);
  }
 }
}
'@

$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Frame-Shadow-Smoke-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$settingsDir=Join-Path $appData 'ShotKitRust'
$fixturePath=Join-Path $root 'transparent-fixture.png';$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA
$process=$null;$main=$null;$fixture=$null;$failure=$null;$cleanupFailure=$null;$processExited=$true
function Owned-Windows { if($null -eq $process -or $process.HasExited){return @()};return @([ShotKitFrameShadowSmoke.N]::Windows($process.Id)) }
function Owned-Dialogs { @(Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770'}) }
function Wait-For([scriptblock]$Probe,[string]$What,[int]$Seconds=10){
 $end=[DateTime]::UtcNow.AddSeconds($Seconds)
 while([DateTime]::UtcNow -lt $end){if($process -and $process.HasExited){throw "PROCESS_EXITED waiting for $What (code $($process.ExitCode))."};$v=&$Probe;if($null -ne $v -and $v){return $v};Start-Sleep -Milliseconds 100}
 $wins=(Owned-Windows|ForEach-Object{"HWND=$($_.H) PID=$($_.Pid) class='$($_.ClassName)' title='$($_.Title)' visible=$($_.Visible)"}) -join '; '
 throw "TIMEOUT waiting for $What; owned windows=$wins"
}
function Button([IntPtr]$Parent,[int]$Id,[string]$Text){
 $a=@([ShotKitFrameShadowSmoke.N]::Buttons($Parent)|Where-Object{$_.Visible -and $_.Id -eq $Id -and (($_.Text -replace '&','').Trim() -ceq $Text)})
 if($a.Count -ne 1){$all=([ShotKitFrameShadowSmoke.N]::Buttons($Parent)|ForEach-Object{"ID=$($_.Id) text='$($_.Text)' HWND=$($_.H)"}) -join '; ';throw "BUTTON_IDENTITY_MISMATCH ID=$Id text='$Text'; matches=$($a.Count); $all"}
 if([ShotKitFrameShadowSmoke.N]::Owner($a[0].H) -ne $process.Id){throw "BUTTON_OWNER_MISMATCH ID=$Id HWND=$($a[0].H)."}
 return $a[0]
}
function Click-EditorButton([int]$Id,[string]$Text){$b=Button $main.H $Id $Text;[ShotKitFrameShadowSmoke.N]::ClickButton($b.H)}
function Click-EditorButtonAsync([int]$Id,[string]$Text){$b=Button $main.H $Id $Text;if(-not [ShotKitFrameShadowSmoke.N]::PostMessageW($b.H,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)){throw "ASYNC_BUTTON_CLICK_FAILED ID=$Id text='$Text'."}}
function Popup([string]$What){Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32768'}|Select-Object -First 1} $What 8}
function Menu-Item($Menu,[int]$Id,[string]$Text,[string]$What){
 if($Menu.Pid -ne $process.Id -or [ShotKitFrameShadowSmoke.N]::Owner($Menu.H) -ne $process.Id){throw "POPUP_OWNER_MISMATCH for $What."}
 try{[ShotKitFrameShadowSmoke.N]::ClickMenu($Menu.H,$process.Id,[uint32]$Id,$Text)}catch{throw "MENU_CLICK_FAILED for $($What): $($_.Exception.Message)"}
 $end=[DateTime]::UtcNow.AddSeconds(5);while([DateTime]::UtcNow -lt $end){if(@(Owned-Windows|Where-Object{$_.H -eq $Menu.H -and $_.Visible}).Count -eq 0){return};Start-Sleep -Milliseconds 50}
 throw "MENU_CLOSE_TIMEOUT after $What; HWND=$($Menu.H)."
}
function Read-Png([string]$Path){
 $b=[Drawing.Bitmap]::new($Path)
 try{$p=[Array]::CreateInstance([Drawing.Color],$b.Width*$b.Height);for($y=0;$y -lt $b.Height;$y++){for($x=0;$x -lt $b.Width;$x++){$p[$y*$b.Width+$x]=$b.GetPixel($x,$y)}};return [pscustomobject]@{Width=$b.Width;Height=$b.Height;Pixels=$p}}
 finally{$b.Dispose()}
}
function Wait-Png([scriptblock]$Predicate,[string]$What){
 $end=[DateTime]::UtcNow.AddSeconds($ActionTimeoutSeconds);$last=''
 while([DateTime]::UtcNow -lt $end){if($process.HasExited){throw "PROCESS_EXITED waiting for $What (code $($process.ExitCode))."};$d=@(Owned-Dialogs);if($d.Count){throw "UNEXPECTED_DIALOG waiting for $($What): $(($d|ForEach-Object{$_.Title}) -join ', ')"};try{$im=Read-Png $fixturePath;if(& $Predicate $im){return $im}}catch{$last=$_.Exception.Message};Start-Sleep -Milliseconds 100}
 throw "PNG_STATE_TIMEOUT for $What; last result: $last"
}
function Assert-Fixture($Image,[string]$What){
 if($Image.Width -ne $fixture.Width -or $Image.Height -ne $fixture.Height){throw "$What dimensions $($Image.Width)x$($Image.Height), expected $($fixture.Width)x$($fixture.Height)."}
 for($i=0;$i -lt $fixture.Pixels.Length;$i++){if($Image.Pixels[$i].ToArgb() -ne $fixture.Pixels[$i].ToArgb()){throw "$What differs at ($($i%$fixture.Width),$([int]($i/$fixture.Width)))."}}
}
function Save-Editor { Click-EditorButton 0x5110 'Speichern' }

try{
 New-Item -ItemType Directory -Path $root,$appData,$localAppData,$settingsDir -Force|Out-Null
 $b=[Drawing.Bitmap]::new(32,24,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
 try{for($y=0;$y -lt 24;$y++){for($x=0;$x -lt 32;$x++){$b.SetPixel($x,$y,[Drawing.Color]::Transparent)}};$b.SetPixel(7,5,[Drawing.Color]::FromArgb(73,12,123,231));$b.SetPixel(20,14,[Drawing.Color]::FromArgb(255,34,156,87));$b.SetPixel(31,23,[Drawing.Color]::FromArgb(181,201,77,19));$b.Save($fixturePath,[Drawing.Imaging.ImageFormat]::Png)}
 finally{$b.Dispose()}
 $fixture=Read-Png $fixturePath
 if($fixture.Width -ne 32 -or $fixture.Height -ne 24 -or $fixture.Pixels[0].A -ne 0 -or $fixture.Pixels[5*32+7].ToArgb() -ne [Drawing.Color]::FromArgb(73,12,123,231).ToArgb()){throw 'FIXTURE_INVALID: dimensions, alpha, or semitransparent sample differ.'}
 $settings=@{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'}|ConvertTo-Json -Compress
 Set-Content -LiteralPath (Join-Path $settingsDir 'settings.json') -Value $settings -Encoding utf8NoBOM
 try{$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$process=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $fixturePath) -WorkingDirectory $root -PassThru}
 finally{$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
 $main=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} 'owned editor' $StartupTimeoutSeconds
 if($main.Pid -ne $process.Id -or [ShotKitFrameShadowSmoke.N]::Owner($main.H) -ne $process.Id){throw "EDITOR_OWNER_MISMATCH HWND=$($main.H) PID=$($main.Pid), expected $($process.Id)."}
 $welcome=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'}|Select-Object -First 1} 'owned startup notice' $StartupTimeoutSeconds
 if([ShotKitFrameShadowSmoke.N]::Owner($welcome.H) -ne $process.Id){throw 'STARTUP_NOTICE_OWNER_MISMATCH.'}
 if(-not [ShotKitFrameShadowSmoke.N]::PostMessageW($welcome.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'STARTUP_NOTICE_CLOSE_FAILED.'}
 [void](Wait-For {if(-not [ShotKitFrameShadowSmoke.N]::IsWindow($welcome.H)){return $true};return $null} 'startup notice close' 8)
 $main=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*'}|Select-Object -First 1} 'loaded editor title' 10
 if(@(Owned-Dialogs).Count){throw "UNEXPECTED_STARTUP_DIALOG: $((Owned-Dialogs|ForEach-Object{$_.Title}) -join ', ')"}
 # The 28-action toolbar needs 985 logical client pixels; the default 1000px
 # outer window has only 984 at 96 DPI, so Frame correctly enters overflow.
 # These document tests need the full toolbar; retain exact visible identity checks.
 [void][ShotKitFrameShadowSmoke.N]::ShowWindow($main.H,3) # SW_MAXIMIZE
 [void](Wait-For {
  $matches=@([ShotKitFrameShadowSmoke.N]::Buttons($main.H)|Where-Object{$_.Visible -and $_.Id -eq 0x5123 -and $_.Text -ceq 'Rahmen / Schatten / Leinwand…'})
  if($matches.Count -eq 1){return $true}
 } 'maximized editor with visible frame/canvas button' $ActionTimeoutSeconds)
 Click-EditorButton 0x5124 'Rot'
 # TrackPopupMenu runs a modal loop on the UI thread; queue BM_CLICK so this
 # test thread can observe/select the resulting native popup.
 Click-EditorButtonAsync 0x5123 'Rahmen / Schatten / Leinwand…'
 $frameMenu=Popup 'frame/shadow action menu';Menu-Item $frameMenu 3 'Rahmen und Schlagschatten hinzufügen' 'combined frame and shadow action'
 $widthMenu=Popup 'frame width menu';Menu-Item $widthMenu 2 '8' '8-pixel frame width'
 $color=Wait-For {Owned-Dialogs|Select-Object -First 1} 'owned color dialog' 10
 if([ShotKitFrameShadowSmoke.N]::Owner($color.H) -ne $process.Id){throw "COLOR_DIALOG_OWNER_MISMATCH HWND=$($color.H)."}
 $oks=@([ShotKitFrameShadowSmoke.N]::Buttons($color.H)|Where-Object{$_.Visible -and $_.Id -eq 1})
 if($oks.Count -ne 1){$all=([ShotKitFrameShadowSmoke.N]::Buttons($color.H)|ForEach-Object{"ID=$($_.Id) text='$($_.Text)'"})-join '; ';throw "COLOR_DIALOG_IDOK_MISSING: $all"}
 # choose_color initializes COLORREF from current_style; red was set above.
 [ShotKitFrameShadowSmoke.N]::ClickButton($oks[0].H)
 [void](Wait-For {if(-not [ShotKitFrameShadowSmoke.N]::IsWindow($color.H)){return $true};return $null} 'color dialog close' 8)
 if(@(Owned-Dialogs).Count){throw "UNEXPECTED_DIALOG_AFTER_COLOR_ACCEPT: $((Owned-Dialogs|ForEach-Object{$_.Title}) -join ', ')"}
 Start-Sleep -Milliseconds 150
 Save-Editor
 $framed=Wait-Png {param($im)$im.Width -eq 80 -and $im.Height -eq 72} '80x72 combined frame/shadow output'
 $red=[Drawing.Color]::FromArgb(255,230,40,40).ToArgb()
 if($framed.Pixels[12*80+12].ToArgb() -ne $red){throw "FRAME_PIXEL_MISMATCH at (12,12): $($framed.Pixels[12*80+12])."}
 if($framed.Pixels[20*80+20].ToArgb() -ne $fixture.Pixels[0].ToArgb()){throw 'SOURCE_ALPHA_CHANGED at transparent source pixel offset (20,20).'}
 foreach($q in @(@{x=7;y=5},@{x=20;y=14},@{x=31;y=23})){$expected=$fixture.Pixels[$q.y*$fixture.Width+$q.x];$actual=$framed.Pixels[($q.y+20)*$framed.Width+($q.x+20)];if($expected.ToArgb() -ne $actual.ToArgb()){throw "SOURCE_PIXEL_CHANGED at ($($q.x),$($q.y))."}}
 $shadow=$framed.Pixels[71*$framed.Width+79]
 if($shadow.A -le 0 -or $shadow.A -ge 112 -or $shadow.R -ne 0 -or $shadow.G -ne 0 -or $shadow.B -ne 0){throw "SHADOW_PIXEL_MISMATCH at (79,71): $shadow"}
 if($framed.Pixels[0].A -ne 0){throw "OUTER_ALPHA_MISMATCH at (0,0): $($framed.Pixels[0])"}
 Write-Host "OBSERVED: 80x72 PNG has red frame, exact source RGBA, partial black shadow alpha=$($shadow.A), and zero-alpha outer corner."
 Click-EditorButton 0x5112 'Zurück';Save-Editor
 $restored=Wait-Png {param($im)$im.Width -eq $fixture.Width -and $im.Height -eq $fixture.Height} 'Undo restoring fixture dimensions'
 Assert-Fixture $restored 'Undo-restored PNG'
 Write-Host 'PASS: native combined frame/shadow menus applied and saved; Undo restored every fixture pixel.'
}catch{$failure=$_}
finally{
 if($null -ne $process -and -not $process.HasExited){try{foreach($w in [ShotKitFrameShadowSmoke.N]::Windows($process.Id)){[void][ShotKitFrameShadowSmoke.N]::PostMessageW($w.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)};if(-not $process.WaitForExit(5000)){$process.Kill();[void]$process.WaitForExit(5000)}}catch{$cleanupFailure=$_.Exception.Message}}
 if($null -ne $process){try{$processExited=$process.HasExited}catch{$processExited=$false}}
 $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
 if($processExited -and (Test-Path -LiteralPath $root)){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
 if($null -ne $process){$process.Dispose()}
}
if($cleanupFailure){throw "CLEANUP_FAILED: $cleanupFailure"}
if($failure){throw $failure}

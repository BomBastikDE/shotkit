# Standalone interactive test. Usage: pwsh -File windows-canvas-smoke.ps1 -ExePath <shotkit-rust.exe>
# Local sessions must explicitly set SHOTKIT_LOCAL_SMOKE_TEST=1. Uses only disposable fixture/settings.
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
namespace ShotKitCanvasSmoke {
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
  [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")] static extern IntPtr SendTimeoutText(IntPtr h,uint m,IntPtr w,string l,uint flags,uint timeout,out IntPtr result);
  [DllImport("user32.dll",CharSet=CharSet.Unicode,EntryPoint="SendMessageTimeoutW")] static extern IntPtr ReadTimeoutText(IntPtr h,uint m,IntPtr w,StringBuilder l,uint flags,uint timeout,out IntPtr result);
  [DllImport("user32.dll",EntryPoint="SendMessageTimeoutW")] static extern IntPtr SendTimeout(IntPtr h,uint m,IntPtr w,IntPtr l,uint flags,uint timeout,out IntPtr result);
  [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr h,int id);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h,uint relation);
  public static IntPtr Control(IntPtr dialog,int id,string cls,int pid) {
   var h=GetDlgItem(dialog,id);
   if(h==IntPtr.Zero||Owner(h)!=pid||Read(h,true)!=cls)throw new InvalidOperationException("Control identity mismatch ID="+id+" expected "+cls);
   return h;
  }
  static IntPtr SafeSend(IntPtr h,uint m,IntPtr w,IntPtr l) {
   IntPtr result;if(SendTimeout(h,m,w,l,2,3000,out result)==IntPtr.Zero)throw new TimeoutException("Native message timed out: "+m);return result;
  }
  public static void Edit(IntPtr h,string value) {
   IntPtr result;if(SendTimeoutText(h,0xC,IntPtr.Zero,value,2,3000,out result)==IntPtr.Zero||result==IntPtr.Zero)throw new TimeoutException("WM_SETTEXT failed or timed out.");
  }
  public static string EditText(IntPtr h) {
   var s=new StringBuilder(128);IntPtr result;if(ReadTimeoutText(h,0xD,(IntPtr)s.Capacity,s,2,3000,out result)==IntPtr.Zero)throw new TimeoutException("WM_GETTEXT timed out.");return s.ToString();
  }
  public static void Select(IntPtr h,int index) {
   if(SafeSend(h,0x14E,(IntPtr)index,IntPtr.Zero).ToInt32()!=index||SafeSend(h,0x147,IntPtr.Zero,IntPtr.Zero).ToInt32()!=index)throw new InvalidOperationException("Combo selection failed.");
  }
  public static void ClickButton(IntPtr h){if(h==IntPtr.Zero||!IsWindow(h))throw new ArgumentException("Invalid button HWND.");SafeSend(h,0xF5,IntPtr.Zero,IntPtr.Zero);}
  public static void ClickMenu(IntPtr popup,int pid,uint wantedId,string wantedText){
   if(Owner(popup)!=pid||Read(popup,true)!="#32768"||!IsWindowVisible(popup))throw new InvalidOperationException("Popup is not visible and owned by test process.");
   const uint MN_GETHMENU=0x1E1,MF_BYPOSITION=0x400;IntPtr menu=SafeSend(popup,MN_GETHMENU,IntPtr.Zero,IntPtr.Zero);
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

$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Canvas-Smoke-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$settingsDir=Join-Path $appData 'ShotKitRust'
$fixturePath=Join-Path $root 'canvas-fixture.png';$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA
$process=$null;$main=$null;$fixture=$null;$failure=$null;$cleanupFailure=$null;$processExited=$true
function Owned-Windows { if($null -eq $process -or $process.HasExited){return @()};return @([ShotKitCanvasSmoke.N]::Windows($process.Id)) }
function Owned-Dialogs { @(Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770'}) }
function Wait-For([scriptblock]$Probe,[string]$What,[int]$Seconds=10){
 $end=[DateTime]::UtcNow.AddSeconds($Seconds)
 while([DateTime]::UtcNow -lt $end){if($process -and $process.HasExited){throw "PROCESS_EXITED waiting for $What (code $($process.ExitCode))."};$v=&$Probe;if($null -ne $v -and $v){return $v};Start-Sleep -Milliseconds 100}
 $wins=(Owned-Windows|ForEach-Object{"HWND=$($_.H) PID=$($_.Pid) class='$($_.ClassName)' title='$($_.Title)' visible=$($_.Visible)"}) -join '; '
 throw "TIMEOUT waiting for $What; owned windows=$wins"
}
function Button([IntPtr]$Parent,[int]$Id,[string]$Text){
 $a=@([ShotKitCanvasSmoke.N]::Buttons($Parent)|Where-Object{$_.Visible -and $_.Id -eq $Id -and (($_.Text -replace '&','').Trim() -ceq $Text)})
 if($a.Count -ne 1){$all=([ShotKitCanvasSmoke.N]::Buttons($Parent)|ForEach-Object{"ID=$($_.Id) text='$($_.Text)' HWND=$($_.H)"}) -join '; ';throw "BUTTON_IDENTITY_MISMATCH ID=$Id text='$Text'; matches=$($a.Count); $all"}
 if([ShotKitCanvasSmoke.N]::Owner($a[0].H) -ne $process.Id){throw "BUTTON_OWNER_MISMATCH ID=$Id HWND=$($a[0].H)."}
 return $a[0]
}
function Click-EditorButton([int]$Id,[string]$Text){$b=Button $main.H $Id $Text;[ShotKitCanvasSmoke.N]::ClickButton($b.H)}
function Click-EditorButtonAsync([int]$Id,[string]$Text){$b=Button $main.H $Id $Text;if(-not [ShotKitCanvasSmoke.N]::PostMessageW($b.H,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)){throw "ASYNC_BUTTON_CLICK_FAILED ID=$Id text='$Text'."}}
function Popup([string]$What){Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32768'}|Select-Object -First 1} $What 8}
function Menu-Item($Menu,[int]$Id,[string]$Text,[string]$What){
 if($Menu.Pid -ne $process.Id -or [ShotKitCanvasSmoke.N]::Owner($Menu.H) -ne $process.Id){throw "POPUP_OWNER_MISMATCH for $What."}
 try{[ShotKitCanvasSmoke.N]::ClickMenu($Menu.H,$process.Id,[uint32]$Id,$Text)}catch{throw "MENU_CLICK_FAILED for $($What): $($_.Exception.Message)"}
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
function Save-Editor {
 Click-EditorButton 0x5110 'Speichern'
 [void](Wait-For {Owned-Windows|Where-Object{$_.H -eq $main.H -and -not $_.Title.StartsWith('* ')}} 'clean saved editor title' $ActionTimeoutSeconds)
}
function Canvas-Dialog {
 Click-EditorButtonAsync 0x5123 'Rahmen / Schatten / Leinwand…'
 $menu=Popup 'frame menu';Menu-Item $menu 4 'Leinwandgröße ändern…' 'canvas action'
 $d=Wait-For {Owned-Dialogs|Where-Object{$_.Title -ceq 'Leinwandgröße' -and [ShotKitCanvasSmoke.N]::GetWindow($_.H,4) -eq $main.H}|Select-Object -First 1} 'owned canvas dialog' $ActionTimeoutSeconds
 foreach($id in 101,102,106){[void][ShotKitCanvasSmoke.N]::Control($d.H,$id,'Edit',$process.Id)}
 foreach($id in 103,104,105){[void][ShotKitCanvasSmoke.N]::Control($d.H,$id,'ComboBox',$process.Id)}
 return $d
}
function Set-Canvas($Dialog,[string]$Width,[string]$Height,[int]$Fill=0,[string]$Color='#FFFFFF') {
 [ShotKitCanvasSmoke.N]::Edit([ShotKitCanvasSmoke.N]::Control($Dialog.H,101,'Edit',$process.Id),$Width)
 [ShotKitCanvasSmoke.N]::Edit([ShotKitCanvasSmoke.N]::Control($Dialog.H,102,'Edit',$process.Id),$Height)
 [ShotKitCanvasSmoke.N]::Select([ShotKitCanvasSmoke.N]::Control($Dialog.H,103,'ComboBox',$process.Id),1)
 [ShotKitCanvasSmoke.N]::Select([ShotKitCanvasSmoke.N]::Control($Dialog.H,104,'ComboBox',$process.Id),1)
 [ShotKitCanvasSmoke.N]::Select([ShotKitCanvasSmoke.N]::Control($Dialog.H,105,'ComboBox',$process.Id),$Fill)
 [ShotKitCanvasSmoke.N]::Edit([ShotKitCanvasSmoke.N]::Control($Dialog.H,106,'Edit',$process.Id),$Color)
}
function Dialog-Button($Dialog,[int]$Id,[string]$Text) {
 $b=Button $Dialog.H $Id $Text
 if(-not [ShotKitCanvasSmoke.N]::PostMessageW($b.H,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'DIALOG_BUTTON_POST_FAILED'}
}
function Close-Canvas($Dialog,[int]$Id=1,[string]$Text='Übernehmen') {
 Dialog-Button $Dialog $Id $Text
 [void](Wait-For {if(-not [ShotKitCanvasSmoke.N]::IsWindow($Dialog.H)){return $true}} 'canvas dialog close' $ActionTimeoutSeconds)
 if(@(Owned-Dialogs).Count){throw 'UNEXPECTED_DIALOG_AFTER_CANVAS_CLOSE'}
}
function Assert-Canvas($Image,[int]$FillArgb,[string]$What) {
 if($Image.Width -ne 18 -or $Image.Height -ne 14){throw "$What dimensions $($Image.Width)x$($Image.Height), expected 18x14"}
 for($y=0;$y -lt 14;$y++){for($x=0;$x -lt 18;$x++){
  $actual=$Image.Pixels[$y*18+$x]
  if($x -ge 3 -and $x -lt 15 -and $y -ge 3 -and $y -lt 11){
   if($actual.ToArgb() -ne $fixture.Pixels[($y-3)*12+$x-3].ToArgb()){throw "$What source pixel mismatch at ($x,$y)"}
  } elseif($FillArgb -eq 0){if($actual.A -ne 0){throw "$What expected transparent border at ($x,$y)"}}
  elseif($actual.ToArgb() -ne $FillArgb){throw "$What fill mismatch at ($x,$y)"}
 }}
}
function Editor-Title { (Owned-Windows|Where-Object{$_.H -eq $main.H}|Select-Object -First 1).Title }
function Assert-Unchanged([string]$BeforeTitle,[string]$BeforeHash,[string]$What) {
 if((Editor-Title) -cne $BeforeTitle){throw "$What changed editor title/state"}
 if((Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash -cne $BeforeHash){throw "$What changed saved file"}
 Save-Editor
 Assert-Canvas (Wait-Png {param($im)$im.Width -eq 18 -and $im.Height -eq 14} $What) $fillArgb $What
}

try {
 New-Item -ItemType Directory -Path $root,$appData,$localAppData,$settingsDir -Force|Out-Null
 # A tiny opaque, asymmetric fixture makes every source-pixel shift observable.
 $b=[Drawing.Bitmap]::new(12,8,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
 try {
  for($y=0;$y -lt 8;$y++){for($x=0;$x -lt 12;$x++){$b.SetPixel($x,$y,[Drawing.Color]::FromArgb(255,17*$x,29*$y,($x+13*$y)%256))}}
  $b.Save($fixturePath,[Drawing.Imaging.ImageFormat]::Png)
 } finally {$b.Dispose()}
 $fixture=Read-Png $fixturePath
 $settings=@{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'}|ConvertTo-Json -Compress
 Set-Content -LiteralPath (Join-Path $settingsDir 'settings.json') -Value $settings -Encoding utf8NoBOM
 try {
  $env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData
  $process=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $fixturePath) -WorkingDirectory $root -PassThru
 } finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
 $main=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} 'owned editor' $StartupTimeoutSeconds
 $welcome=Wait-For {Owned-Dialogs|Where-Object{$_.Title -like '*Prototyp*'}|Select-Object -First 1} 'owned startup notice' $StartupTimeoutSeconds
 if(-not [ShotKitCanvasSmoke.N]::PostMessageW($welcome.H,0x10,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'STARTUP_NOTICE_CLOSE_FAILED'}
 [void](Wait-For {if(-not [ShotKitCanvasSmoke.N]::IsWindow($welcome.H)){return $true}} 'startup notice close' $ActionTimeoutSeconds)
 $main=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*'}|Select-Object -First 1} 'loaded editor' $ActionTimeoutSeconds
 if(@(Owned-Dialogs).Count){throw 'UNEXPECTED_STARTUP_DIALOG'}
 # The 28-action toolbar needs 985 logical client pixels; the default 1000px
 # outer window has only 984 at 96 DPI, so Frame correctly enters overflow.
 # These document tests need the full toolbar; retain exact visible identity checks.
 [void][ShotKitCanvasSmoke.N]::ShowWindow($main.H,3) # SW_MAXIMIZE
 [void](Wait-For {
  $matches=@([ShotKitCanvasSmoke.N]::Buttons($main.H)|Where-Object{$_.Visible -and $_.Id -eq 0x5123 -and $_.Text -ceq 'Rahmen / Schatten / Leinwand…'})
  if($matches.Count -eq 1){return $true}
 } 'maximized editor with visible frame/canvas button' $ActionTimeoutSeconds)

 $d=Canvas-Dialog
 $initialWidth=[ShotKitCanvasSmoke.N]::EditText([ShotKitCanvasSmoke.N]::Control($d.H,101,'Edit',$process.Id))
 $initialHeight=[ShotKitCanvasSmoke.N]::EditText([ShotKitCanvasSmoke.N]::Control($d.H,102,'Edit',$process.Id))
 if($initialWidth -cne '12' -or $initialHeight -cne '8'){throw "INITIAL_CANVAS_SIZE_MISMATCH: ${initialWidth}x${initialHeight}"}
 Set-Canvas $d '18' '14' 0
 Close-Canvas $d
 Save-Editor
 Assert-Canvas (Wait-Png {param($im)$im.Width -eq 18 -and $im.Height -eq 14} 'transparent expansion') 0 'transparent expansion'

 Click-EditorButton 0x5112 'Zurück'
 Save-Editor
 Assert-Fixture (Wait-Png {param($im)$im.Width -eq 12 -and $im.Height -eq 8} 'Undo original') 'Undo original'
 Click-EditorButton 0x5113 'Wiederholen'
 Save-Editor
 Assert-Canvas (Wait-Png {param($im)$im.Width -eq 18 -and $im.Height -eq 14} 'Redo transparent expansion') 0 'Redo transparent expansion'
 Click-EditorButton 0x5112 'Zurück'
 Save-Editor
 Assert-Fixture (Wait-Png {param($im)$im.Width -eq 12 -and $im.Height -eq 8} 'Undo before color test') 'Undo before color test'

 $d=Canvas-Dialog
 Set-Canvas $d '18' '14' 1 '#1234AB'
 Close-Canvas $d
 Save-Editor
 $fillArgb=[Drawing.Color]::FromArgb(255,18,52,171).ToArgb()
 Assert-Canvas (Wait-Png {param($im)$im.Width -eq 18 -and $im.Height -eq 14} 'opaque expansion') $fillArgb 'opaque expansion'

 # Rejected values must leave the canvas dialog open and document untouched.
 foreach($case in @(
  @{Width='0';Height='14';Fill=0;Color='#FFFFFF';Name='zero width'},
  @{Width='30001';Height='14';Fill=0;Color='#FFFFFF';Name='axis limit'},
  @{Width='30000';Height='30000';Fill=0;Color='#FFFFFF';Name='memory limit'},
  @{Width='1.5';Height='14';Fill=0;Color='#FFFFFF';Name='fractional width'},
  @{Width='20';Height='16';Fill=1;Color='#GGHHII';Name='invalid color'}
 )) {
  $beforeTitle=Editor-Title;$beforeHash=(Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash
  $d=Canvas-Dialog
  Set-Canvas $d $case.Width $case.Height $case.Fill $case.Color
  Dialog-Button $d 1 'Übernehmen'
  $errorDialog=Wait-For {Owned-Dialogs|Where-Object{$_.H -ne $d.H -and $_.Title -ceq 'Leinwandgröße' -and [ShotKitCanvasSmoke.N]::GetWindow($_.H,4) -eq $d.H}|Select-Object -First 1} "validation error: $($case.Name)" $ActionTimeoutSeconds
  if(-not [ShotKitCanvasSmoke.N]::IsWindow($d.H)){throw "INVALID_INPUT_CLOSED_CANVAS: $($case.Name)"}
  # MessageBox IDOK is localized by Windows; assert class/ID/ownership instead of its caption.
  $ok=[ShotKitCanvasSmoke.N]::Control($errorDialog.H,1,'Button',$process.Id)
  if(-not [ShotKitCanvasSmoke.N]::PostMessageW($ok,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'VALIDATION_ERROR_CLOSE_FAILED'}
  [void](Wait-For {if(-not [ShotKitCanvasSmoke.N]::IsWindow($errorDialog.H)){return $true}} 'validation message close' $ActionTimeoutSeconds)
  Close-Canvas $d 2 'Abbrechen'
  Assert-Unchanged $beforeTitle $beforeHash "rejected $($case.Name)"
 }
 $beforeTitle=Editor-Title;$beforeHash=(Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash
 $d=Canvas-Dialog
 Set-Canvas $d '20' '16' 0
 Close-Canvas $d 2 'Abbrechen'
 Assert-Unchanged $beforeTitle $beforeHash 'cancelled valid resize'
 # Cancellation and invalid attempts must not insert history entries.
 Click-EditorButton 0x5112 'Zurück'
 Save-Editor
 Assert-Fixture (Wait-Png {param($im)$im.Width -eq 12 -and $im.Height -eq 8} 'Undo after rejected/cancelled attempts') 'Undo after rejected/cancelled attempts'
 Click-EditorButton 0x5113 'Wiederholen'
 Save-Editor
 Assert-Canvas (Wait-Png {param($im)$im.Width -eq 18 -and $im.Height -eq 14} 'Redo after rejected/cancelled attempts') $fillArgb 'Redo after rejected/cancelled attempts'
 Write-Host 'PASS: frame-menu canvas dialog; exact centered source pixels; transparent/opaque fill; saved dimensions; Undo/Redo; invalid sizes/color and cancel leave document/history unchanged.'
} catch {$failure=$_}
finally {
 if($null -ne $process -and -not $process.HasExited){
  try {
   foreach($w in [ShotKitCanvasSmoke.N]::Windows($process.Id)){[void][ShotKitCanvasSmoke.N]::PostMessageW($w.H,0x10,[IntPtr]::Zero,[IntPtr]::Zero)}
   if(-not $process.WaitForExit(3000)){$process.Kill();if(-not $process.WaitForExit(5000)){throw 'Owned ShotKit process remained alive after bounded kill wait.'}}
  } catch {$cleanupFailure=$_.Exception.Message}
 }
 if($null -ne $process){try{$processExited=$process.HasExited}catch{$processExited=$false}}
 $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
 if($processExited -and (Test-Path -LiteralPath $root)){
  try {Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop} catch {$cleanupFailure=$_.Exception.Message}
 }
 if($null -ne $process){$process.Dispose()}
}
if($cleanupFailure){throw "CLEANUP_FAILED: $cleanupFailure. Check temporary evidence at '$root'. Original test failure: $failure"}
if($failure){throw $failure}

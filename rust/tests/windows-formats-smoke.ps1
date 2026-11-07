[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$ExePath,
    [ValidateRange(10,120)][int]$StartupTimeoutSeconds=30,
    [ValidateRange(10,90)][int]$ActionTimeoutSeconds=25
)
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if($PSVersionTable.PSVersion.Major -lt 7){throw 'PowerShell 7 (pwsh) is required.'}
if(($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive){throw 'INTERACTIVE_WINDOWS_REQUIRED: ephemeral interactive runner or dedicated opt-in session required.'}
if(-not(Test-Path -LiteralPath $ExePath -PathType Leaf)){throw "ShotKit EXE not found: $ExePath"}
$ExePath=(Resolve-Path -LiteralPath $ExePath).Path
$imageName=[IO.Path]::GetFileNameWithoutExtension($ExePath)
if(@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0){throw "PREEXISTING_PROCESS: refusing to use/change '$imageName'."}
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitFormatsSmoke {
 public struct Rect { public int Left,Top,Right,Bottom; }
 public struct Point { public int X,Y; }
 public sealed class Win { public IntPtr H; public int Pid; public string ClassName,Title; public bool Visible; }
 public sealed class Child { public IntPtr H; public int Id; public string ClassName,Text; public bool Visible; }
 public static class N {
  delegate bool EnumProc(IntPtr h,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc cb,IntPtr d);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern IntPtr SendMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] static extern bool IsChild(IntPtr parent,IntPtr child);
  [DllImport("user32.dll")] static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] static extern void mouse_event(uint f,uint x,uint y,uint data,UIntPtr extra);
  [DllImport("user32.dll")] static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
  [DllImport("user32.dll",EntryPoint="SendMessageW",CharSet=CharSet.Unicode)] static extern IntPtr SendText(IntPtr h,uint m,IntPtr w,IntPtr l);
  static string Read(IntPtr h,bool cls){var s=new StringBuilder(512);if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString();}
  public static Win[] Windows(int pid){var a=new List<Win>();EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)pid)a.Add(new Win{H=h,Pid=(int)p,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static int Owner(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return (int)p;}
  public static Child[] Children(IntPtr parent){var a=new List<Child>();EnumProc cb=(h,d)=>{a.Add(new Child{H=h,Id=GetDlgCtrlID(h),ClassName=Read(h,true),Text=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static string ComboText(IntPtr h,int i){int n=SendMessageW(h,0x149,new IntPtr(i),IntPtr.Zero).ToInt32();if(n<0)return null;return ReadComboText(h,i,n);}
  static string ReadComboText(IntPtr h,int i,int n){IntPtr p=Marshal.AllocHGlobal((n+1)*2);try{SendMessageW(h,0x148,new IntPtr(i),p);return Marshal.PtrToStringUni(p)??"";}finally{Marshal.FreeHGlobal(p);}}
  public static int ComboCount(IntPtr h){return SendMessageW(h,0x146,IntPtr.Zero,IntPtr.Zero).ToInt32();}
  public static int ComboSelection(IntPtr h){return SendMessageW(h,0x147,IntPtr.Zero,IntPtr.Zero).ToInt32();}
  static void Key(byte k){keybd_event(k,0,0,UIntPtr.Zero);keybd_event(k,0,2,UIntPtr.Zero);}
  public static void SelectComboByInput(IntPtr h,int target,int pid){Rect r;if(!GetWindowRect(h,out r)||r.Right<=r.Left||r.Bottom<=r.Top)throw new InvalidOperationException("Could not get filter combo rectangle.");int x=r.Left+(r.Right-r.Left)/2,y=r.Top+(r.Bottom-r.Top)/2;if(!SetCursorPos(x,y))throw new InvalidOperationException("SetCursorPos failed for filter combo.");IntPtr under=WindowFromPoint(new Point{X=x,Y=y});if(under==IntPtr.Zero||Owner(under)!=pid||(under!=h&&!IsChild(h,under)))throw new InvalidOperationException("Cursor point is not over the uniquely identified filter combo.");mouse_event(2,0,0,0,UIntPtr.Zero);mouse_event(4,0,0,0,UIntPtr.Zero);Key(0x24);for(int i=0;i<target;i++)Key(0x28);Key(0x0D);}
  public static bool SetEditText(IntPtr h,string s){IntPtr p=Marshal.StringToHGlobalUni(s);try{return SendText(h,0x000C,IntPtr.Zero,p).ToInt64()!=0;}finally{Marshal.FreeHGlobal(p);}}
  public static void Click(IntPtr h){SendMessageW(h,0x00F5,IntPtr.Zero,IntPtr.Zero);}
  public static void ControlShiftS(){try{keybd_event(0x11,0,0,UIntPtr.Zero);keybd_event(0x10,0,0,UIntPtr.Zero);keybd_event(0x53,0,0,UIntPtr.Zero);}finally{keybd_event(0x53,0,2,UIntPtr.Zero);keybd_event(0x10,0,2,UIntPtr.Zero);keybd_event(0x11,0,2,UIntPtr.Zero);}}
  public static void CloseOwned(int pid){foreach(var w in Windows(pid))PostMessageW(w.H,0x10,IntPtr.Zero,IntPtr.Zero);}
 }
}
'@

$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Formats-Smoke-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$settingsDir=Join-Path $appData 'ShotKitRust'
$fixturePath=Join-Path $root 'source.png';$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA
$proc=$null;$failure=$null;$cleanupFailure=$null;$processExited=$true
$formats=@(
 [pscustomobject]@{Name='PNG';Filter='PNG images (*.png)';Index=0;Extension='.png';Raw=[Drawing.Imaging.ImageFormat]::Png;Exact=$true},
 [pscustomobject]@{Name='JPEG';Filter='JPEG images (*.jpg;*.jpeg)';Index=1;Extension='.jpg';Raw=[Drawing.Imaging.ImageFormat]::Jpeg;Exact=$false},
 [pscustomobject]@{Name='BMP';Filter='Bitmap images (*.bmp)';Index=2;Extension='.bmp';Raw=[Drawing.Imaging.ImageFormat]::Bmp;Exact=$true},
 [pscustomobject]@{Name='GIF';Filter='GIF images (*.gif)';Index=3;Extension='.gif';Raw=[Drawing.Imaging.ImageFormat]::Gif;Exact=$true}
)
$expectedLabels=@($formats|ForEach-Object{$_.Filter})
function Owned-Windows {if($null -eq $proc -or $proc.HasExited){return @()};return @([ShotKitFormatsSmoke.N]::Windows($proc.Id))}
function Wait-For([scriptblock]$Probe,[string]$What,[int]$Seconds=$ActionTimeoutSeconds){$end=[DateTime]::UtcNow.AddSeconds($Seconds);while([DateTime]::UtcNow -lt $end){if($proc -and $proc.HasExited){throw "PROCESS_EXITED waiting for $What (code $($proc.ExitCode))."};$v=&$Probe;if($null -ne $v -and $v){return $v};Start-Sleep -Milliseconds 100};$wins=(Owned-Windows|ForEach-Object{"HWND=$($_.H) PID=$($_.Pid) class='$($_.ClassName)' title='$($_.Title)' visible=$($_.Visible)"})-join '; ';throw "TIMEOUT waiting for $What; owned windows=$wins"}
function Get-Buttons([IntPtr]$Parent){@([ShotKitFormatsSmoke.N]::Children($Parent)|Where-Object{$_.Visible -and $_.ClassName -eq 'Button'})}
function Click-Button([IntPtr]$Parent,[int]$Id,[string]$Text){$buttonMatches=@(Get-Buttons $Parent|Where-Object{$_.Id -eq $Id -and (($_.Text -replace '&','').Trim() -ceq $Text)});if($buttonMatches.Count -ne 1){$all=(Get-Buttons $Parent|ForEach-Object{"ID=$($_.Id) '$($_.Text)' HWND=$($_.H)"})-join '; ';throw "BUTTON_IDENTITY_MISMATCH ID=$Id text='$Text' matches=$($buttonMatches.Count); $all"};[ShotKitFormatsSmoke.N]::Click($buttonMatches[0].H)}
function Get-SaveDialog {Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770' -and @([ShotKitFormatsSmoke.N]::Children($_.H)|Where-Object{$_.ClassName -eq 'Edit'}).Count -gt 0}|Select-Object -First 1} 'owned native Save As dialog' 10}
function Get-FilterCombo($Dialog){
 $comboMatches=@( [ShotKitFormatsSmoke.N]::Children($Dialog.H)|Where-Object{$_.Visible -and $_.ClassName -eq 'ComboBox'}|ForEach-Object{$c=$_;$n=[ShotKitFormatsSmoke.N]::ComboCount($c.H);if($n -eq $expectedLabels.Count){$labels=@(for($i=0;$i -lt $n;$i++){[ShotKitFormatsSmoke.N]::ComboText($c.H,$i)});if(($labels -join "`n") -ceq ($expectedLabels -join "`n")){[pscustomobject]@{Child=$c;Labels=$labels}}}})
 if($comboMatches.Count -ne 1){$diagnostic=@([ShotKitFormatsSmoke.N]::Children($Dialog.H)|Where-Object{$_.ClassName -match 'ComboBox'}|ForEach-Object{$c=$_;$n=[ShotKitFormatsSmoke.N]::ComboCount($c.H);$labels=@(for($i=0;$i -lt [Math]::Max(0,$n);$i++){[ShotKitFormatsSmoke.N]::ComboText($c.H,$i)});"class=$($c.ClassName) id=$($c.Id) hwnd=$($c.H) count=$n labels=[$($labels -join '|')]"})-join '; ';throw "FILTER_COMBO_IDENTITY_FAILURE: expected one ComboBox with exact labels [$($expectedLabels -join '|')], found $($comboMatches.Count). $diagnostic"}
 return $comboMatches[0]
}
function Read-PngOrImage([string]$Path){$b=[Drawing.Bitmap]::new($Path);try{$pixels=[Array]::CreateInstance([Drawing.Color],$b.Width*$b.Height);for($y=0;$y -lt $b.Height;$y++){for($x=0;$x -lt $b.Width;$x++){$pixels[$y*$b.Width+$x]=$b.GetPixel($x,$y)}};return [pscustomobject]@{Width=$b.Width;Height=$b.Height;Pixels=$pixels;RawFormat=$b.RawFormat.Guid}}finally{$b.Dispose()}}
function New-Fixture([string]$Path){$b=[Drawing.Bitmap]::new(64,48,[Drawing.Imaging.PixelFormat]::Format32bppArgb);try{for($y=0;$y -lt $b.Height;$y++){for($x=0;$x -lt $b.Width;$x++){$b.SetPixel($x,$y,[Drawing.Color]::Transparent)}};foreach($p in @(@{x=4;y=4;c=[Drawing.Color]::FromArgb(255,220,30,40)},@{x=28;y=4;c=[Drawing.Color]::FromArgb(255,30,200,45)},@{x=4;y=28;c=[Drawing.Color]::FromArgb(255,20,55,220)},@{x=28;y=28;c=[Drawing.Color]::FromArgb(255,12,15,20)})){for($y=$p.y;$y -lt $p.y+12;$y++){for($x=$p.x;$x -lt $p.x+12;$x++){$b.SetPixel($x,$y,$p.c)}}};$b.Save($Path,[Drawing.Imaging.ImageFormat]::Png)}finally{$b.Dispose()}}
function Assert-Sample($Image,[int]$X,[int]$Y,[Drawing.Color]$Expected,[int]$Tolerance,[string]$What){$a=$Image.Pixels[$Y*$Image.Width+$X];foreach($channel in @('R','G','B')){$actualChannel=[int]$a.$channel;$expectedChannel=[int]$Expected.$channel;if([Math]::Abs($actualChannel-$expectedChannel) -gt $Tolerance){throw "$What pixel ($X,$Y) channel $channel=$actualChannel, expected $expectedChannel ±$Tolerance; observed $a"}}}
function Stop-OwnedProcess {if($null -eq $proc){return};if(-not $proc.HasExited){try{[ShotKitFormatsSmoke.N]::CloseOwned($proc.Id);if(-not $proc.WaitForExit(5000)){$proc.Kill();[void]$proc.WaitForExit(5000)}}catch{$script:cleanupFailure=$_.Exception.Message}};try{$script:processExited=$proc.HasExited}catch{$script:processExited=$false};$proc.Dispose();$script:proc=$null;if(-not $script:processExited){if(-not $script:cleanupFailure){$script:cleanupFailure='Started ShotKit process is still alive after close/kill.'};throw "OWNED_PROCESS_CLEANUP_FAILED: $script:cleanupFailure"}}
function Run-Format($Format){
 $sourceHash=(Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash
 $proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $fixturePath) -WorkingDirectory $root -PassThru
 $script:proc=$proc
 try{
  $main=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} 'owned editor' $StartupTimeoutSeconds
  Assert-NoStartupDialogs -TimeoutSeconds $StartupTimeoutSeconds -OwnedProcess $proc -GetWindows { Owned-Windows }
  $main=Wait-For {Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*'}|Select-Object -First 1} 'loaded editor' 12
  [void][ShotKitFormatsSmoke.N]::SetForegroundWindow($main.H)
  [void](Wait-For {[ShotKitFormatsSmoke.N]::GetForegroundWindow() -eq $main.H} 'exact editor foreground before Save As' 5)
  [ShotKitFormatsSmoke.N]::ControlShiftS()
  $dialog=Get-SaveDialog
  if([ShotKitFormatsSmoke.N]::Owner($dialog.H) -ne $proc.Id){throw "SAVE_DIALOG_OWNER_MISMATCH HWND=$($dialog.H)."}
  [void][ShotKitFormatsSmoke.N]::SetForegroundWindow($dialog.H)
  [void](Wait-For {[ShotKitFormatsSmoke.N]::GetForegroundWindow() -eq $dialog.H} 'exact Save As dialog foreground' 5)
  $filterCombo=Get-FilterCombo $dialog
  [ShotKitFormatsSmoke.N]::SelectComboByInput($filterCombo.Child.H,$Format.Index,$proc.Id)
  [void](Wait-For {[ShotKitFormatsSmoke.N]::ComboSelection($filterCombo.Child.H) -eq $Format.Index} "exact $($Format.Name) filter selection" 5)
  $selection=[ShotKitFormatsSmoke.N]::ComboSelection($filterCombo.Child.H)
  $selectedLabel=[ShotKitFormatsSmoke.N]::ComboText($filterCombo.Child.H,$selection)
  if($selection -ne $Format.Index -or $selectedLabel -cne $Format.Filter){throw "FILTER_SELECTION_MISMATCH $($Format.Name): index=$selection label='$selectedLabel' expected index=$($Format.Index) label='$($Format.Filter)'."}
  $edits=@([ShotKitFormatsSmoke.N]::Children($dialog.H)|Where-Object{$_.Visible -and $_.ClassName -eq 'Edit'})
  if($edits.Count -ne 1){throw "SAVE_FILENAME_CONTROL_AMBIGUOUS: expected one visible Edit, found $($edits.Count)."}
  $base='shotkit-format-output'
  $basePath=[IO.Path]::GetFullPath((Join-Path $root $base))
  $expectedDirectory=[IO.Path]::GetFullPath($root).TrimEnd([IO.Path]::DirectorySeparatorChar)+[IO.Path]::DirectorySeparatorChar
  if(-not $basePath.StartsWith($expectedDirectory,[StringComparison]::OrdinalIgnoreCase)){throw "TARGET_PATH_OUTSIDE_ROOT: $basePath"}
  $output=$basePath+$Format.Extension
  if((Split-Path -Parent $output) -ne $root -or (Test-Path -LiteralPath $output)){throw "TARGET_PATH_INVALID_OR_EXISTS: $output"}
  if(-not [ShotKitFormatsSmoke.N]::SetEditText($edits[0].H,$basePath)){throw 'SAVE_FILENAME_SET_FAILED.'}
  Click-Button $dialog.H 1 'Save'
  $image=Wait-For {if(Test-Path -LiteralPath $output -PathType Leaf){try{$im=Read-PngOrImage $output;if($im.Width -eq 64 -and $im.Height -eq 48){return $im}}catch{}};return $null} "$($Format.Name) output file and decode" $ActionTimeoutSeconds
  if($image.RawFormat -ne $Format.Raw.Guid){throw "FORMAT_ID_MISMATCH $($Format.Name): RawFormat=$($image.RawFormat), expected=$($Format.Raw.Guid)."}
  if($image.Width -ne 64 -or $image.Height -ne 48){throw "DIMENSION_MISMATCH $($Format.Name): $($image.Width)x$($image.Height)."}
  $points=@(@{x=9;y=9;c=[Drawing.Color]::FromArgb(255,220,30,40)},@{x=33;y=9;c=[Drawing.Color]::FromArgb(255,30,200,45)},@{x=9;y=33;c=[Drawing.Color]::FromArgb(255,20,55,220)},@{x=33;y=33;c=[Drawing.Color]::FromArgb(255,12,15,20)})
  $tol=if($Format.Name -eq 'JPEG'){30}else{0}
  foreach($p in $points){Assert-Sample $image $p.x $p.y $p.c $tol "$($Format.Name)"}
  if($Format.Name -eq 'PNG' -and $image.Pixels[40*64+58].A -ne 0){throw "PNG_ALPHA_MISMATCH at (58,40): $($image.Pixels[40*64+58])."}
  if($Format.Name -eq 'JPEG'){Assert-Sample $image 58 40 ([Drawing.Color]::White) 10 'JPEG white-composited transparent source'}
  if((Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash -cne $sourceHash){throw "SOURCE_MODIFIED while saving $($Format.Name)."}
  Write-Host "PASS: $($Format.Name) native filter '$selectedLabel' saved $output; format id, 64x48 dimensions, and four decoded color patches verified."
 }finally{Stop-OwnedProcess}
}
try{
 New-Item -ItemType Directory -Path $root,$appData,$localAppData,$settingsDir -Force|Out-Null
 $settings=@{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'}|ConvertTo-Json -Compress
 Set-Content -LiteralPath (Join-Path $settingsDir 'settings.json') -Value $settings -Encoding utf8NoBOM
 $env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData
 New-Fixture $fixturePath
 foreach($format in $formats){Run-Format $format}
 Write-Host 'PASS: all four native Save As format paths were selected and decoded.'
}catch{$failure=$_}finally{
 if($null -ne $proc){try{Stop-OwnedProcess}catch{$cleanupFailure=$_.Exception.Message}}
 $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
 if($processExited -and (Test-Path -LiteralPath $root)){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($cleanupFailure){throw "CLEANUP_FAILED: $cleanupFailure"};if($failure){throw $failure}

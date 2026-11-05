[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$ExePath,
    [ValidateRange(10,120)][int]$StartupTimeoutSeconds=30,
    [ValidateRange(5,60)][int]$ActionTimeoutSeconds=15
)

$ErrorActionPreference='Stop'
if($PSVersionTable.PSVersion.Major -lt 7){throw 'PowerShell 7 (pwsh) is required.'}
if($env:GITHUB_ACTIONS -ne 'true' -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive){throw 'INTERACTIVE_CI_WINDOWS_REQUIRED: this global hotkey-conflict test runs only on an interactive ephemeral Windows CI desktop.'}
if(-not(Test-Path -LiteralPath $ExePath -PathType Leaf)){throw "ShotKit EXE not found: $ExePath"}
$ExePath=(Resolve-Path -LiteralPath $ExePath).Path
$imageName=[IO.Path]::GetFileNameWithoutExtension($ExePath)
if(@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0){throw "PREEXISTING_PROCESS: refusing to target/change '$imageName'."}
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitHotkeyConflictSmoke {
 public sealed class Win { public IntPtr H; public int Pid; public string ClassName,Title; public bool Visible; }
 public static class N {
  delegate bool EnumProc(IntPtr h,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc cb,IntPtr d);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
  [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll",SetLastError=true)] static extern IntPtr OpenInputDesktop(uint flags,bool inherit,uint access);
  [DllImport("user32.dll",SetLastError=true)] static extern bool CloseDesktop(IntPtr h);
  [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool GetUserObjectInformationW(IntPtr h,int index,StringBuilder text,uint length,out uint needed);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] static extern void keybd_event(byte key,byte scan,uint flags,UIntPtr extra);
  static string Read(IntPtr h,bool cls){var s=new StringBuilder(1024);if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString();}
  public static Win[] Windows(int pid){var a=new List<Win>();EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)pid)a.Add(new Win{H=h,Pid=(int)p,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static int Owner(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return (int)p;}
  public static string Text(IntPtr h){var b=new StringBuilder(Read(h,false));EnumChildWindows(h,(c,d)=>{var t=Read(c,false);if(!String.IsNullOrWhiteSpace(t))b.Append(" ").Append(t);return true;},IntPtr.Zero);return b.ToString();}
  public static bool Close(IntPtr h){return PostMessageW(h,0x0010,IntPtr.Zero,IntPtr.Zero);}
  public static bool Exists(IntPtr h){return IsWindow(h);}
  public static string InputDesktopName(){IntPtr d=OpenInputDesktop(0,false,1);if(d==IntPtr.Zero)return null;try{var s=new StringBuilder(256);uint n;if(!GetUserObjectInformationW(d,2,s,(uint)(s.Capacity*2),out n))return null;return s.ToString();}finally{CloseDesktop(d);}}
  public static void SendCtrlShift4(){try{keybd_event(0x11,0,0,UIntPtr.Zero);keybd_event(0x10,0,0,UIntPtr.Zero);keybd_event(0x34,0,0,UIntPtr.Zero);keybd_event(0x34,0,2,UIntPtr.Zero);}finally{keybd_event(0x10,0,2,UIntPtr.Zero);keybd_event(0x11,0,2,UIntPtr.Zero);}}
 }
}
'@

$inputDesktop=[ShotKitHotkeyConflictSmoke.N]::InputDesktopName()
if($inputDesktop -cne 'Default'){throw "INTERACTIVE_DEFAULT_DESKTOP_REQUIRED: current input desktop is '$inputDesktop'."}

$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Hotkey-Conflict-Smoke-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$settingsDir=Join-Path $appData 'ShotKitRust'
$helperFile=Join-Path $root 'reservation-helper.ps1';$readyFile=Join-Path $root 'reservation.ready';$eventFile=Join-Path $root 'reservation.events';$releasedFile=Join-Path $root 'reservation.released'
$helperStdout=Join-Path $root 'helper.stdout';$helperStderr=Join-Path $root 'helper.stderr'
$helperTitle='ShotKit Hotkey Reservation '+[Guid]::NewGuid().ToString('N')
$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA
$helper=$null;$shotKit=$null;$script:helperWindow=$null;$failure=$null;$cleanupFailure=$null;$rootCanBeDeleted=$true

function Owned-Windows($Process){if($null -eq $Process -or $Process.HasExited){return @()};return @([ShotKitHotkeyConflictSmoke.N]::Windows($Process.Id))}
function Owned-Dialogs($Process){@(Owned-Windows $Process|Where-Object{$_.Visible -and $_.ClassName -eq '#32770'})}
function Wait-For([scriptblock]$Probe,[string]$Description,[int]$Seconds=$ActionTimeoutSeconds,$Process=$shotKit){$end=[DateTime]::UtcNow.AddSeconds($Seconds);while([DateTime]::UtcNow -lt $end){if($Process -and $Process.HasExited){throw "PROCESS_EXITED waiting for $Description (code $($Process.ExitCode))."};$v=&$Probe;if($null -ne $v -and $v){return $v};Start-Sleep -Milliseconds 100};$wins=(Owned-Windows $Process|ForEach-Object{"HWND=$($_.H) PID=$($_.Pid) class='$($_.ClassName)' title='$($_.Title)' visible=$($_.Visible)"})-join '; ';throw "TIMEOUT waiting for $Description; owned windows=$wins"}
function Start-ShotKit {try{$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;return Start-Process -FilePath $ExePath -WorkingDirectory $root -PassThru}finally{$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}}
function Close-Dialog($Dialog,[string]$Description){if([ShotKitHotkeyConflictSmoke.N]::Owner($Dialog.H) -ne $shotKit.Id){throw "DIALOG_OWNER_MISMATCH for $Description HWND=$($Dialog.H)."};if(-not [ShotKitHotkeyConflictSmoke.N]::Close($Dialog.H)){throw "DIALOG_CLOSE_POST_FAILED for $Description HWND=$($Dialog.H)."};[void](Wait-For {-not [ShotKitHotkeyConflictSmoke.N]::Exists($Dialog.H)} "$Description close" 8 $shotKit)}
function Set-ReservationForeground {if($null -eq $script:helperWindow -or -not $script:helperWindow.Visible -or [ShotKitHotkeyConflictSmoke.N]::Owner($script:helperWindow.H) -ne $helper.Id){throw 'RESERVATION_WINDOW_IDENTITY_INVALID.'};[void][ShotKitHotkeyConflictSmoke.N]::SetForegroundWindow($script:helperWindow.H);if([ShotKitHotkeyConflictSmoke.N]::GetForegroundWindow() -ne $script:helperWindow.H){throw "RESERVATION_WINDOW_NOT_FOREGROUND: expected HWND=$($script:helperWindow.H), actual=$([ShotKitHotkeyConflictSmoke.N]::GetForegroundWindow())."}}
function Send-ConflictChord {Set-ReservationForeground;[ShotKitHotkeyConflictSmoke.N]::SendCtrlShift4()}
function Wait-EventCount([int]$Expected){Wait-For {if(Test-Path -LiteralPath $eventFile){$v=[IO.File]::ReadAllText($eventFile).Trim();if($v -match '^\d+$' -and [int]$v -ge $Expected){return [int]$v}};return $null} "reservation helper WM_HOTKEY count >= $Expected" 8 $helper}
function Start-AppAndCloseWelcome([switch]$ExpectConflict){
 $script:shotKit=Start-ShotKit
 $main=Wait-For {Owned-Windows $shotKit|Where-Object{$_.ClassName -eq 'ShotKitRustPrototype'}|Select-Object -First 1} 'ShotKit primary window' $StartupTimeoutSeconds $shotKit
 if([ShotKitHotkeyConflictSmoke.N]::Owner($main.H) -ne $shotKit.Id){throw "MAIN_WINDOW_OWNER_MISMATCH HWND=$($main.H)."}
 if($ExpectConflict){
  $conflict=Wait-For {
   $dialogs=Owned-Dialogs $shotKit
   foreach($d in $dialogs){$text=[ShotKitHotkeyConflictSmoke.N]::Text($d.H);if($d.Title -eq 'Tastenkürzel' -and $text -like '*Nicht verfügbare Tastenkürzel: Strg+Umschalt+4*'){return $d};if($d.Title -notlike '*Prototyp*'){throw "UNEXPECTED_REGISTRATION_DIALOG: title='$($d.Title)' text='$text'"}}
   return $null
  } 'exact Ctrl+Shift+4 conflict notification' $StartupTimeoutSeconds $shotKit
  $conflictText=[ShotKitHotkeyConflictSmoke.N]::Text($conflict.H)
  if($conflictText -notlike '*Nicht verfügbare Tastenkürzel: Strg+Umschalt+4*'){throw "CONFLICT_NOTIFICATION_TEXT_MISMATCH: $conflictText"}
  Close-Dialog $conflict 'Ctrl+Shift+4 conflict notification'
 }
 $welcome=Wait-For {
  $dialogs=Owned-Dialogs $shotKit
  $notice=$dialogs|Where-Object{$_.Title -like '*Prototyp*'}|Select-Object -First 1
  if($notice){return $notice}
  if($dialogs.Count){$d=$dialogs[0];throw "UNEXPECTED_REGISTRATION_DIALOG: title='$($d.Title)' text='$([ShotKitHotkeyConflictSmoke.N]::Text($d.H))'"}
  return $null
 } 'ShotKit startup notice' $StartupTimeoutSeconds $shotKit
 Close-Dialog $welcome 'ShotKit startup notice'
 $other=@(Owned-Dialogs $shotKit)
 if($other.Count){$text=[ShotKitHotkeyConflictSmoke.N]::Text($other[0].H);throw "UNEXPECTED_SHOTKIT_DIALOG after startup: title='$($other[0].Title)' text='$text'"}
 Set-ReservationForeground
}
function Stop-ShotKit {if($null -eq $shotKit){return};if(-not $shotKit.HasExited){try{$shotKit.Kill();if(-not $shotKit.WaitForExit(5000)){throw 'ShotKit process did not exit after kill.'}}catch{$script:cleanupFailure=$_.Exception.Message}};try{$ended=$shotKit.HasExited}catch{$ended=$false};$shotKit.Dispose();$script:shotKit=$null;if(-not $ended){$script:rootCanBeDeleted=$false;throw "OWNED_SHOTKIT_CLEANUP_FAILED: $script:cleanupFailure"}}
function Stop-Helper {if($null -eq $helper){return};if(-not $helper.HasExited){try{if($script:helperWindow -and [ShotKitHotkeyConflictSmoke.N]::Owner($script:helperWindow.H) -eq $helper.Id){[void][ShotKitHotkeyConflictSmoke.N]::PostMessageW($script:helperWindow.H,0x8002,[IntPtr]::Zero,[IntPtr]::Zero)};if(-not $helper.WaitForExit(5000)){$helper.Kill();if(-not $helper.WaitForExit(5000)){throw 'Reservation helper did not exit.'}}}catch{$script:cleanupFailure=$_.Exception.Message}};try{$ended=$helper.HasExited}catch{$ended=$false};$helper.Dispose();$script:helper=$null;if(-not $ended){$script:rootCanBeDeleted=$false;throw "OWNED_HELPER_CLEANUP_FAILED: $script:cleanupFailure"}}

try{
 New-Item -ItemType Directory -Path $root,$appData,$localAppData,$settingsDir -Force|Out-Null
 $settings=@{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'}|ConvertTo-Json -Compress
 Set-Content -LiteralPath (Join-Path $settingsDir 'settings.json') -Value $settings -Encoding utf8NoBOM

$helperSource=@'
param([Parameter(Mandatory=$true)][string]$Ready,[Parameter(Mandatory=$true)][string]$Events,[Parameter(Mandatory=$true)][string]$Released,[Parameter(Mandatory=$true)][string]$WindowTitle)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Windows.Forms;
using System.Drawing;
namespace ShotKitHotkeyReservationHelper {
 public sealed class ReservationForm : Form {
  const int HotkeyId=0x4531; const uint HotkeyModifiers=0x4006, HotkeyKey=0x34;
  const int WmHotkey=0x0312,WmRelease=0x8001,WmExit=0x8002;
  [DllImport("user32.dll",SetLastError=true)] static extern bool RegisterHotKey(IntPtr h,int id,uint mods,uint key);
  [DllImport("user32.dll",SetLastError=true)] static extern bool UnregisterHotKey(IntPtr h,int id);
  readonly string ready,events,released; bool registered; int count;
  public ReservationForm(string readyPath,string eventPath,string releasedPath,string title){
   ready=readyPath;events=eventPath;released=releasedPath;Text=title;StartPosition=FormStartPosition.CenterScreen;FormBorderStyle=FormBorderStyle.FixedDialog;ClientSize=new Size(440,120);MaximizeBox=false;MinimizeBox=false;ShowInTaskbar=false;
   var label=new Label();label.Dock=DockStyle.Fill;label.TextAlign=ContentAlignment.MiddleCenter;label.Text="ShotKit hotkey-conflict test window\\r\\nCtrl+Shift+4 is reserved by this test process.";Controls.Add(label);
   Shown+=delegate{registered=RegisterHotKey(Handle,HotkeyId,HotkeyModifiers,HotkeyKey);int error=registered?0:Marshal.GetLastWin32Error();System.IO.File.WriteAllText(ready,String.Format("{0},{1},{2}",System.Diagnostics.Process.GetCurrentProcess().Id,registered?1:0,error));if(registered)System.IO.File.WriteAllText(events,"0");};
  }
  protected override void WndProc(ref Message m){
   if(m.Msg==WmHotkey&&m.WParam.ToInt32()==HotkeyId){count++;System.IO.File.WriteAllText(events,count.ToString());m.Result=IntPtr.Zero;return;}
   if(m.Msg==WmRelease){bool ok=UnregisterHotKey(Handle,HotkeyId);registered=false;System.IO.File.WriteAllText(released,ok?"1":"0");m.Result=IntPtr.Zero;return;}
   if(m.Msg==WmExit){Close();m.Result=IntPtr.Zero;return;}
   base.WndProc(ref m);
  }
  protected override void OnFormClosed(FormClosedEventArgs e){if(registered){UnregisterHotKey(Handle,HotkeyId);registered=false;}base.OnFormClosed(e);}
 }
}
"@
$form=[ShotKitHotkeyReservationHelper.ReservationForm]::new($Ready,$Events,$Released,$WindowTitle)
[System.Windows.Forms.Application]::Run($form)
'@
 Set-Content -LiteralPath $helperFile -Value $helperSource -Encoding utf8NoBOM
 $powershellExe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
 if(-not(Test-Path -LiteralPath $powershellExe -PathType Leaf)){throw "Windows PowerShell 5.1 was not found at '$powershellExe'."}
 $helperArgs='-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -Ready "{1}" -Events "{2}" -Released "{3}" -WindowTitle "{4}"' -f $helperFile,$readyFile,$eventFile,$releasedFile,$helperTitle
 $helper=Start-Process -FilePath $powershellExe -ArgumentList $helperArgs -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput $helperStdout -RedirectStandardError $helperStderr -PassThru
 try {
  $ready=Wait-For {if(Test-Path -LiteralPath $readyFile){[IO.File]::ReadAllText($readyFile).Trim()}} 'reservation helper readiness' 10 $helper
 } catch {
  $stdout=if(Test-Path -LiteralPath $helperStdout){Get-Content -LiteralPath $helperStdout -Raw}else{''}
  $stderr=if(Test-Path -LiteralPath $helperStderr){Get-Content -LiteralPath $helperStderr -Raw}else{''}
  throw "RESERVATION_HELPER_START_FAILED: $($_.Exception.Message); stdout='$stdout'; stderr='$stderr'"
 }
 $readyParts=$ready.Split(',');if($readyParts.Count -ne 3 -or [int]$readyParts[0] -ne $helper.Id){throw "RESERVATION_HELPER_IDENTITY_MISMATCH: '$ready', process PID=$($helper.Id)."}
 if($readyParts[1] -ne '1'){throw "PREEXISTING_HOTKEY_CONFLICT: test helper could not reserve Ctrl+Shift+4 (RegisterHotKey error $($readyParts[2])). No ShotKit process was started."}
 $script:helperWindow=Wait-For {Owned-Windows $helper|Where-Object{$_.Visible -and $_.Title -ceq $helperTitle}|Select-Object -First 1} 'visible reservation helper window' 8 $helper
 if([ShotKitHotkeyConflictSmoke.N]::Owner($script:helperWindow.H) -ne $helper.Id){throw "RESERVATION_WINDOW_OWNER_MISMATCH HWND=$($script:helperWindow.H)."}
 [void](Wait-For {if(Test-Path -LiteralPath $eventFile){[IO.File]::ReadAllText($eventFile).Trim()}} 'reservation event log' 5 $helper)

 Start-AppAndCloseWelcome -ExpectConflict
 foreach($expected in 1..2){Send-ConflictChord;[void](Wait-EventCount $expected);Start-Sleep -Milliseconds 100}
 $reservationCount=[IO.File]::ReadAllText($eventFile).Trim()
 if($reservationCount -cne '2'){throw "RESERVATION_CHANGED_OR_REPEATED: expected exactly two helper WM_HOTKEY messages, got '$reservationCount'."}
 $unexpected=@(Owned-Windows $shotKit|Where-Object{$_.Visible -and (($_.ClassName -eq '#32770' -and $_.Title -eq 'Scroll-Richtung') -or $_.ClassName -eq 'ShotKitSelectionOverlay')})
 if($unexpected.Count){throw "CONFLICTING_SHORTCUT_DISPATCHED_INTO_SHOTKIT: $($unexpected[0].ClassName) '$($unexpected[0].Title)'."}
 Write-Host 'OBSERVED: exact conflict warning; helper received both Ctrl+Shift+4 presses while its RegisterHotKey reservation remained active; ShotKit did not enter scroll capture.'

 Stop-ShotKit
 if([ShotKitHotkeyConflictSmoke.N]::Owner($script:helperWindow.H) -ne $helper.Id -or -not [ShotKitHotkeyConflictSmoke.N]::PostMessageW($script:helperWindow.H,0x8001,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'COULD_NOT_POST_RESERVATION_RELEASE_TO_OWNED_WINDOW.'}
 $released=Wait-For {if(Test-Path -LiteralPath $releasedFile){[IO.File]::ReadAllText($releasedFile).Trim()}} 'helper UnregisterHotKey acknowledgement' 8 $helper
 if($released -cne '1'){throw "HELPER_UNREGISTER_FAILED: ack='$released'."}

 Start-AppAndCloseWelcome
 Send-ConflictChord
 $scrollDialog=Wait-For {Owned-Dialogs $shotKit|Where-Object{$_.Title -eq 'Scroll-Richtung'}|Select-Object -First 1} 'Scroll-Richtung after releasing conflict reservation' $ActionTimeoutSeconds $shotKit
 if([ShotKitHotkeyConflictSmoke.N]::Owner($scrollDialog.H) -ne $shotKit.Id){throw "SCROLL_DIALOG_OWNER_MISMATCH HWND=$($scrollDialog.H), expected PID=$($shotKit.Id)."}
 $scrollText=[ShotKitHotkeyConflictSmoke.N]::Text($scrollDialog.H)
 if($scrollText -notlike '*Ja = nach unten*' -or $scrollText -notlike '*Esc bricht ab*'){throw "SCROLL_DIALOG_TEXT_MISMATCH: $scrollText"}
 Close-Dialog $scrollDialog 'recovered Ctrl+Shift+4 Scroll-Richtung dialog'
 Write-Host 'PASS: after explicit UnregisterHotKey, a fresh ShotKit process registered Ctrl+Shift+4 and opened its owned scroll-direction prompt.'
}catch{$failure=$_}finally{
 if($null -ne $shotKit){try{Stop-ShotKit}catch{$cleanupFailure=$_.Exception.Message}}
 if($null -ne $helper){try{Stop-Helper}catch{$cleanupFailure=$_.Exception.Message}}
 $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
 if($rootCanBeDeleted -and (Test-Path -LiteralPath $root)){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($cleanupFailure){throw "CLEANUP_FAILED: $cleanupFailure"};if($failure){throw $failure}

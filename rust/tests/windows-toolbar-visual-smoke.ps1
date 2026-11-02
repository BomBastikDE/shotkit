[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$ExePath,
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$ScreenshotPath,
    [ValidateRange(10,120)][int]$StartupTimeoutSeconds=30
)
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -ne 'true'){throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'}
if(-not [Environment]::UserInteractive){throw 'NO_INTERACTIVE_DESKTOP: the Windows runner is not interactive.'}
if(-not(Test-Path -LiteralPath $ExePath -PathType Leaf)){throw "ShotKit EXE not found: $ExePath"}
$ExePath=(Resolve-Path -LiteralPath $ExePath).Path
$ScreenshotPath=[IO.Path]::GetFullPath($ScreenshotPath)
$imageName=[IO.Path]::GetFileNameWithoutExtension($ExePath)
if(@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count){throw "PREEXISTING_PROCESS: '$imageName' exists; refusing to connect to or stop it."}
if(-not('ShotKitToolbarVisual.Native' -as [type])) { Add-Type -TypeDefinition @'
using System; using System.Collections.Generic; using System.Runtime.InteropServices; using System.Text;
namespace ShotKitToolbarVisual {
 public struct Rect { public int Left,Top,Right,Bottom; }
 public struct Point { public int X,Y; }
 public sealed class Win { public IntPtr H; public string ClassName,Title; public bool Visible; }
 public sealed class Button { public IntPtr H; public int Id; public string Text; }
 [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct NotifyId { public uint Size; public IntPtr Hwnd; public uint Id; public Guid Guid; }
 [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)] public struct NotifyData {
  public uint Size; public IntPtr Hwnd; public uint Id,Flags,Callback; public IntPtr Icon;
  [MarshalAs(UnmanagedType.ByValTStr,SizeConst=128)] public string Tip;
  public uint State,StateMask;
  [MarshalAs(UnmanagedType.ByValTStr,SizeConst=256)] public string Info;
  public uint Timeout;
  [MarshalAs(UnmanagedType.ByValTStr,SizeConst=64)] public string InfoTitle;
  public uint InfoFlags; public Guid Guid; public IntPtr BalloonIcon;
 }
 public static class Native {
  delegate bool EnumProc(IntPtr h,IntPtr p);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr p);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc cb,IntPtr p);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll",EntryPoint="GetClassLongPtrW")] static extern IntPtr GetClassLongPtrW(IntPtr h,int index);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,uint x,uint y,uint d,UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k,byte s,uint f,UIntPtr e);
  [DllImport("user32.dll")] public static extern IntPtr LoadIconW(IntPtr i,IntPtr name);
  [DllImport("shell32.dll",CharSet=CharSet.Unicode)] public static extern int Shell_NotifyIconGetRect(ref NotifyId id,out Rect rect);
  [DllImport("shell32.dll",CharSet=CharSet.Unicode)] static extern bool Shell_NotifyIconW(uint command,ref NotifyData data);
  public static bool VerifyTrayRegistration(IntPtr host) {
   var n=new NotifyData { Size=(uint)Marshal.SizeOf(typeof(NotifyData)), Hwnd=host, Id=1, Flags=4,
    Tip="ShotKit Rust: F9 = Bildschirm, Klick = Bereich, Rechtsklick = Menü", Info="", InfoTitle="" };
   return Shell_NotifyIconW(1,ref n); // NIM_MODIFY of the existing tooltip; fails for an unknown icon.
  }
  [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr d,int x,int y,int w,int h,IntPtr s,int sx,int sy,int op);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h,IntPtr dc);
  public static Win[] Windows(int pid){var a=new List<Win>(); EnumWindows((h,p)=>{uint q;GetWindowThreadProcessId(h,out q);if(q==(uint)pid){var c=new StringBuilder(256);var t=new StringBuilder(512);GetClassNameW(h,c,c.Capacity);GetWindowTextW(h,t,t.Capacity);a.Add(new Win{H=h,ClassName=c.ToString(),Title=t.ToString(),Visible=IsWindowVisible(h)});}return true;},IntPtr.Zero);return a.ToArray();}
  public static Win[] Children(IntPtr parent){var a=new List<Win>();EnumChildWindows(parent,(h,p)=>{var c=new StringBuilder(256);var t=new StringBuilder(512);GetClassNameW(h,c,c.Capacity);GetWindowTextW(h,t,t.Capacity);a.Add(new Win{H=h,ClassName=c.ToString(),Title=t.ToString(),Visible=IsWindowVisible(h)});return true;},IntPtr.Zero);return a.ToArray();}
  public static Button[] Buttons(IntPtr parent){var a=new List<Button>();EnumChildWindows(parent,(h,p)=>{var c=new StringBuilder(128);GetClassNameW(h,c,c.Capacity);if(IsWindowVisible(h)&&IsWindowEnabled(h)&&String.Equals(c.ToString(),"Button",StringComparison.OrdinalIgnoreCase)){var t=new StringBuilder(512);GetWindowTextW(h,t,t.Capacity);a.Add(new Button{H=h,Id=GetDlgCtrlID(h),Text=t.ToString()});}return true;},IntPtr.Zero);return a.ToArray();}
  public static uint Owner(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return p;}
  public static IntPtr Icon(IntPtr h){var i=SendMessageW(h,0x7f,new IntPtr(1),IntPtr.Zero);if(i==IntPtr.Zero)i=SendMessageW(h,0x7f,IntPtr.Zero,IntPtr.Zero);if(i==IntPtr.Zero)i=GetClassLongPtrW(h,-14);return i;}
 }
}
'@ }
Add-Type -AssemblyName System.Drawing
try { Add-Type -AssemblyName UIAutomationClient; Add-Type -AssemblyName UIAutomationTypes }
catch { throw "UI_AUTOMATION_UNAVAILABLE: cannot read the visible native tooltip safely: $($_.Exception.Message)" }
$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Toolbar-Visual-'+[guid]::NewGuid().ToString('N'))
$app=Join-Path $root 'appdata';$local=Join-Path $root 'localappdata';$fixture=Join-Path $root 'fixture.png'
$oldA=$env:APPDATA;$oldL=$env:LOCALAPPDATA;$p=$null;$ticks=0;$mouse=$false;$controlDown=$false;$stopped=$true;$failure=$null;$cleanup=@()
function Owned-Windows { if($null -eq $script:p -or $script:p.HasExited){return @()} @([ShotKitToolbarVisual.Native]::Windows($script:p.Id)) }
try {
 New-Item -ItemType Directory -Path $root,$app,$local,(Split-Path -Parent $ScreenshotPath) -Force|Out-Null
 $b=[Drawing.Bitmap]::new(640,360);try{$g=[Drawing.Graphics]::FromImage($b);try{$g.Clear([Drawing.Color]::FromArgb(255,248,246,240));$f=[Drawing.Font]::new('Segoe UI',22);try{$g.DrawString('ShotKit visual smoke fixture',$f,[Drawing.Brushes]::DarkSlateBlue,28,26);$g.DrawRectangle([Drawing.Pens]::DarkOrange,50,100,420,210)}finally{$f.Dispose()}}finally{$g.Dispose()};$b.Save($fixture,[Drawing.Imaging.ImageFormat]::Png)}finally{$b.Dispose()}
 $env:APPDATA=$app;$env:LOCALAPPDATA=$local
 try{$p=Start-Process -FilePath $ExePath -ArgumentList ('"'+$fixture+'"') -WorkingDirectory $root -PassThru;$stopped=$false;$ticks=$p.StartTime.ToUniversalTime().Ticks}finally{$env:APPDATA=$oldA;$env:LOCALAPPDATA=$oldL}
 $deadline=[DateTime]::UtcNow.AddSeconds($StartupTimeoutSeconds);$main=$null
 while([DateTime]::UtcNow -lt $deadline){if($p.HasExited){throw "PROCESS_EXITED: code $($p.ExitCode)"};$main=Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1;if($main){break};Start-Sleep -Milliseconds 150}
 if(-not $main){throw 'EDITOR_NOT_READY: visible owned ShotKitRustEditor window not found.'}
 [void][ShotKitToolbarVisual.Native]::SetForegroundWindow($main.H);Start-Sleep -Milliseconds 400
 # Acknowledge only the app-owned startup notice, if present.
 $deadline=[DateTime]::UtcNow.AddSeconds(12);$welcome=$null
 while([DateTime]::UtcNow -lt $deadline){$welcome=Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'}|Select-Object -First 1;if($welcome){break};Start-Sleep -Milliseconds 100}
 if(-not $welcome){throw 'STARTUP_NOTICE_MISSING: owned prototype notice did not appear.'}
 $ok=[ShotKitToolbarVisual.Native]::Buttons($welcome.H)|Where-Object{$_.Text -eq 'OK'}|Select-Object -First 1
 if(-not $ok){throw 'STARTUP_NOTICE_UNEXPECTED: owned prototype notice lacked exact OK button.'}
 [void][ShotKitToolbarVisual.Native]::SetForegroundWindow($welcome.H)
 [void][ShotKitToolbarVisual.Native]::SendMessageW($ok.H,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)
 $deadline=[DateTime]::UtcNow.AddSeconds(5)
 while([DateTime]::UtcNow -lt $deadline){$still=Owned-Windows|Where-Object{$_.H -eq $welcome.H -and $_.Visible}|Select-Object -First 1;if(-not $still){break};Start-Sleep -Milliseconds 100}
 if($still){throw 'STARTUP_NOTICE_CLOSE_FAILED: the owned notice remained visible after OK.'}
 [void][ShotKitToolbarVisual.Native]::SetForegroundWindow($main.H)
 Start-Sleep -Milliseconds 500
 $toolbar=[ShotKitToolbarVisual.Native]::Children($main.H)|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'}|Select-Object -First 1
 if(-not $toolbar -or [ShotKitToolbarVisual.Native]::Owner($toolbar.H) -ne [uint32]$p.Id){throw 'TOOLBAR_MISSING_OR_WRONG_OWNER'}
 $buttons=@([ShotKitToolbarVisual.Native]::Buttons($toolbar.H));if($buttons.Count -ne 24){throw "TOOLBAR_BUTTONS_INCOMPLETE: expected exactly 24 native visible buttons, got $($buttons.Count)."}
 $expected=@('Auswahl (V)','Pfeil (A)','Linie (L)','Rechteck (R)','Ellipse (E)','Text (T)','Marker (M)','Verpixeln (B)','Nummer (N)','Stift (P)','Zuschneiden (C)','Farbe…','Rot','Schwarz','Gelb','Blau','Linienbreite…','Schriftgröße…','Rahmen / Schatten…','Neue Aufnahme (Strg+N)','Kopieren','Speichern','Zurück','Wiederholen')
 foreach($label in $expected){if(-not($buttons|Where-Object{$_.Text -ceq $label}|Select-Object -First 1)){throw "NATIVE_BUTTON_LABEL_MISSING: '$label'"}}
 # Hover native buttons and read the visible tooltip through UI Automation.
 foreach($tipLabel in @('Rechteck (R)','Rot','Farbe…','Neue Aufnahme (Strg+N)')) {
  $hover=$buttons|Where-Object{$_.Text -ceq $tipLabel}|Select-Object -First 1
  $wr=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect]);if(-not [ShotKitToolbarVisual.Native]::GetWindowRect($hover.H,[ref]$wr)){throw "BUTTON_RECT_FAILED: $tipLabel"}
  $pos=[Activator]::CreateInstance([ShotKitToolbarVisual.Point]);$pos.X=[int](($wr.Left+$wr.Right)/2);$pos.Y=[int](($wr.Top+$wr.Bottom)/2)
  if(-not [ShotKitToolbarVisual.Native]::SetCursorPos($pos.X,$pos.Y)){throw "HOVER_CURSOR_FAILED: $tipLabel"}
  $actual=[Activator]::CreateInstance([ShotKitToolbarVisual.Point]);[void][ShotKitToolbarVisual.Native]::GetCursorPos([ref]$actual)
  $hit=[ShotKitToolbarVisual.Native]::WindowFromPoint($actual)
  if($hit -ne $hover.H){throw "HOVER_TARGET_MISMATCH: expected '$tipLabel' handle $($hover.H), got $hit at $($actual.X),$($actual.Y)."}
  $deadline=[DateTime]::UtcNow.AddSeconds(5);$tip=$null;$tooltipText=''
  # A visible tooltip may still contain the previous hovered button's text.
  # Keep checking until the exact new tooltip appears, or fail on timeout.
  while([DateTime]::UtcNow -lt $deadline){
   $tip=Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'tooltips_class32'}|Select-Object -First 1
   if($tip){$tooltipText=[Windows.Automation.AutomationElement]::FromHandle($tip.H).Current.Name;if($tooltipText -ceq $tipLabel){break}}
   Start-Sleep -Milliseconds 100
  }
  if(-not $tip){$details=(Owned-Windows|ForEach-Object{"$($_.ClassName):visible=$($_.Visible):title=$($_.Title)"}) -join ', ';throw "TOOLTIP_NOT_VISIBLE: native tooltip for '$tipLabel' did not appear after real hover. Owned windows: $details"}
  if($tooltipText -cne $tipLabel){throw "TOOLTIP_TEXT_MISMATCH: expected exact '$tipLabel'; observed '$tooltipText'."}
 }
 # The color action must open its owned native color chooser. Cancel only that
 # verified dialog, then exercise a quick color swatch and retain it in capture.
 $colorButton=$buttons|Where-Object{$_.Text -ceq 'Farbe…'}|Select-Object -First 1
 if(-not [ShotKitToolbarVisual.Native]::PostMessageW($colorButton.H,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'COLOR_CLICK_POST_FAILED'}
 $deadline=[DateTime]::UtcNow.AddSeconds(5);$colorDialog=$null
 while([DateTime]::UtcNow -lt $deadline){$colorDialog=Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770'}|Select-Object -First 1;if($colorDialog){break};Start-Sleep -Milliseconds 100}
 if(-not $colorDialog -or [ShotKitToolbarVisual.Native]::Owner($colorDialog.H) -ne [uint32]$p.Id){throw 'COLOR_CHOOSER_MISSING_OR_WRONG_OWNER'}
 $cancel=[ShotKitToolbarVisual.Native]::Buttons($colorDialog.H)|Where-Object{$_.Text -in @('Cancel','Abbrechen')}|Select-Object -First 1
 if(-not $cancel){throw "COLOR_CHOOSER_CANCEL_MISSING: title='$($colorDialog.Title)' buttons=$(([ShotKitToolbarVisual.Native]::Buttons($colorDialog.H)|ForEach-Object{$_.Text}) -join ', ')"}
 [void][ShotKitToolbarVisual.Native]::SendMessageW($cancel.H,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)
 Start-Sleep -Milliseconds 200
 $red=$buttons|Where-Object{$_.Text -ceq 'Rot'}|Select-Object -First 1
 [void][ShotKitToolbarVisual.Native]::SendMessageW($red.H,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)
 # Verify the toolbar capture action and Ctrl+N shortcut by observing the
 # owned selection overlay, then cancel only that overlay with Escape.
 foreach($captureMode in @('button','shortcut')) {
  if($captureMode -eq 'button') { $capture=$buttons|Where-Object{$_.Text -ceq 'Neue Aufnahme (Strg+N)'}|Select-Object -First 1;if(-not [ShotKitToolbarVisual.Native]::PostMessageW($capture.H,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'CAPTURE_CLICK_POST_FAILED'} }
  else { [void][ShotKitToolbarVisual.Native]::SetForegroundWindow($main.H);[ShotKitToolbarVisual.Native]::keybd_event(0x11,0,0,[UIntPtr]::Zero);$controlDown=$true;[ShotKitToolbarVisual.Native]::keybd_event(0x4e,0,0,[UIntPtr]::Zero);Start-Sleep -Milliseconds 60;[ShotKitToolbarVisual.Native]::keybd_event(0x4e,0,2,[UIntPtr]::Zero);[ShotKitToolbarVisual.Native]::keybd_event(0x11,0,2,[UIntPtr]::Zero);$controlDown=$false }
  $deadline=[DateTime]::UtcNow.AddSeconds(5);$overlay=$null
  while([DateTime]::UtcNow -lt $deadline){$overlay=Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitSelectionOverlay'}|Select-Object -First 1;if($overlay){break};Start-Sleep -Milliseconds 100}
  if(-not $overlay -or [ShotKitToolbarVisual.Native]::Owner($overlay.H) -ne [uint32]$p.Id){throw "CAPTURE_$($captureMode.ToUpperInvariant())_FAILED: owned selection overlay did not appear."}
  [void][ShotKitToolbarVisual.Native]::SendMessageW($overlay.H,0x0100,[IntPtr]0x1b,[IntPtr]::Zero)
  $deadline=[DateTime]::UtcNow.AddSeconds(5);$alive=$null
  do{$alive=Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitSelectionOverlay'}|Select-Object -First 1;if(-not $alive){break};Start-Sleep -Milliseconds 100}while([DateTime]::UtcNow -lt $deadline)
  if($alive){throw "CAPTURE_CANCEL_FAILED: $captureMode overlay remained visible after Escape."}
  $main=Owned-Windows|Where-Object{$_.Visible -and $_.H -eq $main.H}|Select-Object -First 1
  if(-not $main){throw "CAPTURE_CANCEL_FAILED: editor did not return after cancelling $captureMode."}
 }
 # Mouse and keyboard must each change the selected tool, confirmed by the app's live window title.
 $line=$buttons|Where-Object{$_.Text -ceq 'Linie (L)'}|Select-Object -First 1;$lr=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect]);if(-not [ShotKitToolbarVisual.Native]::GetWindowRect($line.H,[ref]$lr)){throw 'LINE_BUTTON_RECT_FAILED'}
 $lp=[Activator]::CreateInstance([ShotKitToolbarVisual.Point]);$lp.X=[int](($lr.Left+$lr.Right)/2);$lp.Y=[int](($lr.Top+$lr.Bottom)/2)
 if(-not [ShotKitToolbarVisual.Native]::SetCursorPos($lp.X,$lp.Y)){throw 'MOUSE_TOOL_CURSOR_FAILED'};$actual=[Activator]::CreateInstance([ShotKitToolbarVisual.Point]);if(-not [ShotKitToolbarVisual.Native]::GetCursorPos([ref]$actual) -or [ShotKitToolbarVisual.Native]::Owner([ShotKitToolbarVisual.Native]::WindowFromPoint($actual)) -ne [uint32]$p.Id){throw 'MOUSE_TOOL_TARGET_MISMATCH'}
 [ShotKitToolbarVisual.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$mouse=$true;Start-Sleep -Milliseconds 80;[ShotKitToolbarVisual.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$mouse=$false;Start-Sleep -Milliseconds 250
 $state=Owned-Windows|Where-Object{$_.H -eq $main.H}|Select-Object -First 1;if($state.Title -notlike '*Werkzeug Line*'){throw "MOUSE_TOOL_SELECTION_FAILED: title='$($state.Title)'"}
 # Send keyboard shortcut V while editor has focus; verify title reports Select.
 [void][ShotKitToolbarVisual.Native]::SetForegroundWindow($main.H);[ShotKitToolbarVisual.Native]::keybd_event(0x56,0,0,[UIntPtr]::Zero);Start-Sleep -Milliseconds 40;[ShotKitToolbarVisual.Native]::keybd_event(0x56,0,2,[UIntPtr]::Zero);Start-Sleep -Milliseconds 250
 $state=Owned-Windows|Where-Object{$_.H -eq $main.H}|Select-Object -First 1;if($state.Title -notlike '*Werkzeug Select*'){throw "KEYBOARD_TOOL_SELECTION_FAILED: title='$($state.Title)'"}
 # Resource tray icon must be registered for the test process, and the editor window must carry an icon.
 $icon=[ShotKitToolbarVisual.Native]::Icon($main.H);if($icon -eq [IntPtr]::Zero){throw 'WINDOW_ICON_MISSING: WM_GETICON returned no icon.'}
 $trayHost=Owned-Windows|Where-Object{$_.ClassName -eq 'ShotKitRustPrototype'}|Select-Object -First 1
 if(-not $trayHost -or [ShotKitToolbarVisual.Native]::Owner($trayHost.H) -ne [uint32]$p.Id){throw 'TRAY_HOST_MISSING_OR_WRONG_OWNER'}
 $nid=[Activator]::CreateInstance([ShotKitToolbarVisual.NotifyId]);$nid.Size=[Runtime.InteropServices.Marshal]::SizeOf([type][ShotKitToolbarVisual.NotifyId]);$nid.Hwnd=$trayHost.H;$nid.Id=1
 if(-not [ShotKitToolbarVisual.Native]::VerifyTrayRegistration($trayHost.H)){throw 'TRAY_ICON_NOT_REGISTERED: Windows rejected NIM_MODIFY for the exact registered host and ID.'}
 $trayRect=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect]);$trayStatus=[ShotKitToolbarVisual.Native]::Shell_NotifyIconGetRect([ref]$nid,[ref]$trayRect)
 if($trayStatus -ne 0 -and $trayStatus -ne 1){throw ('TRAY_ICON_RECT_FAILED: Shell_NotifyIconGetRect returned HRESULT 0x{0:X8}; host={1}; id={2}; identifier-size={3}.' -f ($trayStatus -band 0xFFFFFFFFL),$trayHost.H,$nid.Id,$nid.Size)}
 if($trayStatus -eq 1){Write-Output 'TRAY: icon identity verified by Windows; no visible bounding rectangle (S_FALSE).'}
 $dir=Split-Path -Parent $ScreenshotPath;if(-not(Test-Path -LiteralPath $dir)){New-Item -ItemType Directory -Path $dir -Force|Out-Null}
 $captureRect=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect]);if(-not [ShotKitToolbarVisual.Native]::GetWindowRect($main.H,[ref]$captureRect)){throw 'SCREENSHOT_RECT_FAILED'}
 $captureBitmap=[Drawing.Bitmap]::new(($captureRect.Right-$captureRect.Left),($captureRect.Bottom-$captureRect.Top))
 try { $captureGraphics=[Drawing.Graphics]::FromImage($captureBitmap);try { $sourceDC=[ShotKitToolbarVisual.Native]::GetWindowDC($main.H);if($sourceDC -eq [IntPtr]::Zero){throw 'SCREENSHOT_DC_FAILED'};try {$targetDC=$captureGraphics.GetHdc();try {if(-not [ShotKitToolbarVisual.Native]::BitBlt($targetDC,0,0,$captureBitmap.Width,$captureBitmap.Height,$sourceDC,0,0,0x00CC0020)){throw 'SCREENSHOT_BITBLT_FAILED'}}finally{$captureGraphics.ReleaseHdc($targetDC)}}finally{[void][ShotKitToolbarVisual.Native]::ReleaseDC($main.H,$sourceDC)}}finally{$captureGraphics.Dispose()};$captureBitmap.Save($ScreenshotPath,[Drawing.Imaging.ImageFormat]::Png)}finally{$captureBitmap.Dispose()};if(-not(Test-Path -LiteralPath $ScreenshotPath -PathType Leaf) -or (Get-Item $ScreenshotPath).Length -lt 10000){throw 'SCREENSHOT_INVALID: editor capture missing or implausibly small.'}
 $pass="PASS: owned 24-button native toolbar; exact tool, color, and capture tooltips; color chooser and swatch; mouse/keyboard tool selection; toolbar/shortcut capture; window and tray icons verified. Screenshot: $ScreenshotPath"
} catch {$failure=$_} finally {
 if($mouse){try{[ShotKitToolbarVisual.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{$cleanup+=$_.Exception.Message}}
 if($controlDown){try{[ShotKitToolbarVisual.Native]::keybd_event(17,0,2,[UIntPtr]::Zero)}catch{$cleanup+=$_.Exception.Message}}
 if($p){try{if(-not $p.HasExited){$actual=$p.StartTime.ToUniversalTime().Ticks;$path=$p.MainModule.FileName;if($actual -ne $ticks -or -not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($path),[IO.Path]::GetFullPath($ExePath))){throw 'Refusing to stop process whose identity changed.'};$p.Kill();if(-not $p.WaitForExit(5000)){throw 'Owned app did not stop.'}};$stopped=$p.HasExited}catch{$cleanup+=$_.Exception.Message}finally{$p.Dispose()}}
 $env:APPDATA=$oldA;$env:LOCALAPPDATA=$oldL
 if($stopped -and (Test-Path -LiteralPath $root)){try{Remove-Item -LiteralPath $root -Recurse -Force}catch{$cleanup+=$_.Exception.Message}}
}
if($failure){throw $failure};if($cleanup.Count){throw ('CLEANUP_FAILURE: '+($cleanup -join '; '))};Write-Output $pass

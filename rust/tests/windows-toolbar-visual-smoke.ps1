[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$ExePath,
    [Parameter(Mandatory=$true)][ValidateNotNullOrEmpty()][string]$ScreenshotPath,
    [ValidateRange(10,120)][int]$StartupTimeoutSeconds=30,
    [switch]$ResponsiveOnly,
    [switch]$PopupOnly
)
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1'){throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.'}
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
 public sealed class MenuItem { public int Id,Position; public string Text; public IntPtr Submenu; }
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
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern bool SetWindowTextW(IntPtr h,string text);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int width,int height,uint flags);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] static extern int GetMenuItemCount(IntPtr menu);
  [DllImport("user32.dll")] static extern uint GetMenuItemID(IntPtr menu,int position);
  [DllImport("user32.dll")] static extern IntPtr GetSubMenu(IntPtr menu,int position);
  [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr owner,IntPtr menu,uint position,out Rect rect);
  [DllImport("user32.dll")] public static extern int MenuItemFromPoint(IntPtr owner,IntPtr menu,Point point);
  [DllImport("user32.dll")] public static extern uint GetMenuState(IntPtr menu,uint position,uint flags);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetMenuStringW(IntPtr menu,uint position,StringBuilder text,int count,uint flags);
  public static MenuItem[] MenuItems(IntPtr menu){var a=new List<MenuItem>();for(int n=0;n<GetMenuItemCount(menu);n++){var text=new StringBuilder(512);GetMenuStringW(menu,(uint)n,text,text.Capacity,0x400);a.Add(new MenuItem{Id=unchecked((int)GetMenuItemID(menu,n)),Position=n,Text=text.ToString(),Submenu=GetSubMenu(menu,n)});}return a.ToArray();}
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
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
  [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr h);
  [DllImport("gdi32.dll")] static extern uint GetPixel(IntPtr dc,int x,int y);
  [DllImport("user32.dll")] public static extern uint GetSysColor(int index);
  public static uint ClientPixel(IntPtr h,int x,int y) {
   Rect client;if(!GetClientRect(h,out client)||x<0||y<0||x>=client.Right||y>=client.Bottom)throw new InvalidOperationException("ZOOM_PIXEL_OUTSIDE_CLIENT");
   var dc=GetDC(h);if(dc==IntPtr.Zero)throw new InvalidOperationException("ZOOM_CLIENT_DC_FAILED");
   try { var color=GetPixel(dc,x,y);if(color==0xffffffff)throw new InvalidOperationException("ZOOM_GET_PIXEL_FAILED");return color; }
   finally { ReleaseDC(h,dc); }
  }
  // Read actual painted pixels through the center of the controlled opaque
  // fixture. Its cream base differs from COLOR_WINDOW throughout both axes.
  public static Rect ImageBounds(IntPtr h,int toolbarHeight,out uint center) {
   Rect client;if(!GetClientRect(h,out client)||client.Right<=0||client.Bottom<=toolbarHeight)throw new InvalidOperationException("ZOOM_CLIENT_BOUNDS_INVALID");
   var dc=GetDC(h);if(dc==IntPtr.Zero)throw new InvalidOperationException("ZOOM_CLIENT_DC_FAILED");
   try {
    var background=GetSysColor(5);int cx=client.Right/2,cy=toolbarHeight+(client.Bottom-toolbarHeight)/2;
    var bounds=new Rect {Left=-1,Top=-1,Right=-1,Bottom=-1};
    for(int x=0;x<client.Right;x++){var color=GetPixel(dc,x,cy);if(color==0xffffffff)throw new InvalidOperationException("ZOOM_ROW_READ_FAILED");if(color!=background){if(bounds.Left<0)bounds.Left=x;bounds.Right=x+1;}}
    for(int y=toolbarHeight;y<client.Bottom;y++){var color=GetPixel(dc,cx,y);if(color==0xffffffff)throw new InvalidOperationException("ZOOM_COLUMN_READ_FAILED");if(color!=background){if(bounds.Top<0)bounds.Top=y;bounds.Bottom=y+1;}}
    center=GetPixel(dc,cx,cy);return bounds;
   } finally { ReleaseDC(h,dc); }
  }
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
$lastMenuInteraction='none'
function Owned-Windows { if($null -eq $script:p -or $script:p.HasExited){return @()} @([ShotKitToolbarVisual.Native]::Windows($script:p.Id)) }
function Wait-ToolbarCondition([scriptblock]$Condition,[string]$Failure) {
 $deadline=[DateTime]::UtcNow.AddSeconds(5)
 do { if(& $Condition){return}; if($script:p.HasExited){throw 'PROCESS_EXITED_DURING_RESPONSIVE_TEST'}; Start-Sleep -Milliseconds 50 } while([DateTime]::UtcNow -lt $deadline)
 throw "$Failure; $(Get-ToolbarInteractionDiagnostic)"
}
function Press-ToolbarKey([byte]$Key) {
 [ShotKitToolbarVisual.Native]::keybd_event($Key,0,0,[UIntPtr]::Zero)
 [ShotKitToolbarVisual.Native]::keybd_event($Key,0,2,[UIntPtr]::Zero)
}
function Click-ToolbarButton($Button) {
 $r=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect])
 if(-not [ShotKitToolbarVisual.Native]::GetWindowRect($Button.H,[ref]$r)){throw 'RESPONSIVE_BUTTON_RECT_FAILED'}
 $x=[int](($r.Left+$r.Right)/2);$y=[int](($r.Top+$r.Bottom)/2)
 if(-not [ShotKitToolbarVisual.Native]::SetCursorPos($x,$y)){throw 'RESPONSIVE_CURSOR_FAILED'}
 $point=[Activator]::CreateInstance([ShotKitToolbarVisual.Point]);[void][ShotKitToolbarVisual.Native]::GetCursorPos([ref]$point)
 if([ShotKitToolbarVisual.Native]::WindowFromPoint($point) -ne $Button.H){throw 'RESPONSIVE_CLICK_TARGET_MISMATCH'}
 [ShotKitToolbarVisual.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouse=$true
 [ShotKitToolbarVisual.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouse=$false
}
function Get-ToolbarPopup {
 $popup=Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32768'}|Select-Object -First 1
 if(-not $popup){return $null}
 $menu=[ShotKitToolbarVisual.Native]::SendMessageW($popup.H,0x01e1,[IntPtr]::Zero,[IntPtr]::Zero)
 if($menu -eq [IntPtr]::Zero){throw 'RESPONSIVE_NATIVE_MENU_HANDLE_MISSING'}
 return [pscustomobject]@{H=$popup.H;Menu=$menu;Entries=@([ShotKitToolbarVisual.Native]::MenuItems($menu))}
}
function Read-ToolbarPopup {
 $popup=Get-ToolbarPopup
 if($null -ne $popup){return $popup.Entries}
}
function Get-ToolbarInteractionDiagnostic {
 try {
  $windows=@(Owned-Windows|Where-Object{$_.Visible}|ForEach-Object{
   $detail="HWND=$($_.H) class=$($_.ClassName) title='$($_.Title)'"
   if($_.ClassName -eq '#32768'){
    $menu=[ShotKitToolbarVisual.Native]::SendMessageW($_.H,0x01e1,[IntPtr]::Zero,[IntPtr]::Zero)
    $entries=@([ShotKitToolbarVisual.Native]::MenuItems($menu)|ForEach-Object{
     "pos=$($_.Position) id=$($_.Id) text='$($_.Text)' submenu=$($_.Submenu) state=$([ShotKitToolbarVisual.Native]::GetMenuState($menu,[uint32]$_.Position,0x400))"
    }) -join ' | '
    $detail+=" HMENU=$menu items=[$entries]"
   }
   $detail
  }) -join '; '
  return "foreground=$([ShotKitToolbarVisual.Native]::GetForegroundWindow()); lastMenuInteraction=[$script:lastMenuInteraction]; windows=[$windows]"
 } catch { return "INTERACTION_DIAGNOSTIC_FAILED: $($_.Exception.Message)" }
}
function Click-ToolbarMenuItem([int]$CommandId) {
 try {
  $popup=Get-ToolbarPopup
  if($null -eq $popup){throw 'MENU_NOT_VISIBLE'}
  $menuMatches=@($popup.Entries|Where-Object{$_.Id -eq $CommandId})
  if($menuMatches.Count -ne 1 -or $menuMatches[0].Submenu -ne [IntPtr]::Zero){throw 'MENU_COMMAND_NOT_UNIQUE_LEAF'}
  $item=$menuMatches[0]
  $r=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect])
  if(-not [ShotKitToolbarVisual.Native]::GetMenuItemRect([IntPtr]::Zero,$popup.Menu,[uint32]$item.Position,[ref]$r) -or $r.Right -le $r.Left -or $r.Bottom -le $r.Top){throw 'MENU_COMMAND_RECT_INVALID'}
  $point=[Activator]::CreateInstance([ShotKitToolbarVisual.Point])
  $point.X=[int][Math]::Floor(($r.Left+$r.Right)/2);$point.Y=[int][Math]::Floor(($r.Top+$r.Bottom)/2)
  $menuDescription=@($popup.Entries|ForEach-Object{"$($_.Position):$($_.Id):$($_.Text)"}) -join ' | '
  $script:lastMenuInteraction="owner=$($script:main.H) popup=$($popup.H) HMENU=$($popup.Menu) id=$CommandId position=$($item.Position) rect=$($r.Left),$($r.Top),$($r.Right),$($r.Bottom) point=$($point.X),$($point.Y) items=[$menuDescription]"
  if([ShotKitToolbarVisual.Native]::MenuItemFromPoint([IntPtr]::Zero,$popup.Menu,$point) -ne $item.Position){throw 'MENU_COMMAND_RECT_ID_MISMATCH'}
  if(-not [ShotKitToolbarVisual.Native]::SetCursorPos($point.X,$point.Y)){throw 'MENU_CURSOR_FAILED'}
  # Wait for real mouse hover, then prove both the native hit and highlighted ID
  # before clicking. No command message bypasses TrackPopupMenu's dispatch.
  Wait-ToolbarCondition {
   $actual=[Activator]::CreateInstance([ShotKitToolbarVisual.Point])
   if(-not [ShotKitToolbarVisual.Native]::GetCursorPos([ref]$actual)){return $false}
   $state=[ShotKitToolbarVisual.Native]::GetMenuState($popup.Menu,[uint32]$item.Position,0x400)
   return [ShotKitToolbarVisual.Native]::WindowFromPoint($actual) -eq $popup.H -and
    [ShotKitToolbarVisual.Native]::MenuItemFromPoint([IntPtr]::Zero,$popup.Menu,$actual) -eq $item.Position -and
    $state -ne [uint32]::MaxValue -and ($state -band 0x03) -eq 0 -and ($state -band 0x80) -ne 0
  } "MENU_COMMAND_NOT_HOVERED: id=$CommandId position=$($item.Position) HMENU=$($popup.Menu)"
  $current=Get-ToolbarPopup
  if($null -eq $current -or $current.H -ne $popup.H -or $current.Menu -ne $popup.Menu -or $current.Entries[$item.Position].Id -ne $CommandId){throw 'MENU_CHANGED_BEFORE_CLICK'}
  [ShotKitToolbarVisual.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouse=$true
  [ShotKitToolbarVisual.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouse=$false
 } catch { throw "MENU_CLICK_FAILED: command=$CommandId; $($_.Exception.Message); $(Get-ToolbarInteractionDiagnostic)" }
}
function Click-OutsideToolbarPopup {
 $popup=Get-ToolbarPopup
 if($null -eq $popup){throw 'OUTSIDE_CLICK_MENU_NOT_VISIBLE'}
 $pr=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect]);$ownerRect=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect])
 if(-not [ShotKitToolbarVisual.Native]::GetWindowRect($popup.H,[ref]$pr)){throw 'OUTSIDE_CLICK_POPUP_RECT_FAILED'}
 if(-not [ShotKitToolbarVisual.Native]::GetWindowRect($script:main.H,[ref]$ownerRect)){throw 'OUTSIDE_CLICK_OWNER_RECT_FAILED'}
 $vx=[ShotKitToolbarVisual.Native]::GetSystemMetrics(76);$vy=[ShotKitToolbarVisual.Native]::GetSystemMetrics(77)
 $vr=$vx+[ShotKitToolbarVisual.Native]::GetSystemMetrics(78);$vb=$vy+[ShotKitToolbarVisual.Native]::GetSystemMetrics(79)
 $cx=[int](($ownerRect.Left+$ownerRect.Right)/2);$cy=[int](($ownerRect.Top+$ownerRect.Bottom)/2)
 $candidates=@(
  [pscustomobject]@{X=$ownerRect.Left-8;Y=$cy},[pscustomobject]@{X=$ownerRect.Right+8;Y=$cy},
  [pscustomobject]@{X=$cx;Y=$ownerRect.Top-8},[pscustomobject]@{X=$cx;Y=$ownerRect.Bottom+8},
  [pscustomobject]@{X=$vx+8;Y=$vy+8},[pscustomobject]@{X=$vr-8;Y=$vy+8},
  [pscustomobject]@{X=$vx+8;Y=$vb-8},[pscustomobject]@{X=$vr-8;Y=$vb-8}
 )
 $point=$candidates|Where-Object{
  $_.X -ge $vx -and $_.X -lt $vr -and $_.Y -ge $vy -and $_.Y -lt $vb -and
  -not($_.X -ge $ownerRect.Left -and $_.X -lt $ownerRect.Right -and $_.Y -ge $ownerRect.Top -and $_.Y -lt $ownerRect.Bottom) -and
  -not($_.X -ge $pr.Left -and $_.X -lt $pr.Right -and $_.Y -ge $pr.Top -and $_.Y -lt $pr.Bottom)
 }|Select-Object -First 1
 if(-not $point){throw "OUTSIDE_CLICK_NO_SAFE_POINT: popup=($($pr.Left),$($pr.Top),$($pr.Right),$($pr.Bottom)); owner=($($ownerRect.Left),$($ownerRect.Top),$($ownerRect.Right),$($ownerRect.Bottom)); virtual=($vx,$vy,$vr,$vb)"}
 if(-not [ShotKitToolbarVisual.Native]::SetCursorPos($point.X,$point.Y)){throw 'OUTSIDE_CLICK_CURSOR_FAILED'}
 $actual=[Activator]::CreateInstance([ShotKitToolbarVisual.Point]);if(-not [ShotKitToolbarVisual.Native]::GetCursorPos([ref]$actual) -or $actual.X -ne $point.X -or $actual.Y -ne $point.Y){throw 'OUTSIDE_CLICK_CURSOR_MISMATCH'}
 [ShotKitToolbarVisual.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouse=$true
 [ShotKitToolbarVisual.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouse=$false
 $script:lastMenuInteraction="outside-click=($($point.X),$($point.Y)); popup=($($pr.Left),$($pr.Top),$($pr.Right),$($pr.Bottom)); owner=($($ownerRect.Left),$($ownerRect.Top),$($ownerRect.Right),$($ownerRect.Bottom))"
}
function Assert-ToolbarRow($Toolbar,$Buttons) {
 $r=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect]);[void][ShotKitToolbarVisual.Native]::GetWindowRect($Toolbar.H,[ref]$r)
 $rects=@(foreach($button in $Buttons){$b=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect]);if(-not [ShotKitToolbarVisual.Native]::GetWindowRect($button.H,[ref]$b)){throw 'RESPONSIVE_BUTTON_RECT_FAILED'};[pscustomobject]@{Id=$button.Id;R=$b}})|Sort-Object{$_.R.Left}
 $top=$rects[0].R.Top
 for($i=0;$i -lt $rects.Count;$i++){
  $b=$rects[$i].R
  if($b.Top -ne $top -or $b.Left -lt $r.Left -or $b.Right -gt $r.Right -or $b.Bottom -gt $r.Bottom){throw 'RESPONSIVE_ROW_BOUNDS_OR_WRAP_FAILED'}
  if($i -gt 0 -and $rects[$i-1].R.Right -ge $b.Left){throw 'RESPONSIVE_BUTTON_OVERLAP'}
 }
 $fixed=@(0x5114,0x5111,0x5110,0x5131)
 $right=@($rects|Select-Object -Last 4|ForEach-Object{$_.Id})
 if(($right -join ',') -ne ($fixed -join ',')){throw "RESPONSIVE_FIXED_ACTION_ORDER_FAILED: $($right -join ',')"}
 $dpi=[ShotKitToolbarVisual.Native]::GetDpiForWindow($Toolbar.H)
 $margin=[int][Math]::Round(4*$dpi/96)
 if([Math]::Abs($r.Right-$rects[-1].R.Right-$margin) -gt 1){throw 'RESPONSIVE_ACTIONS_NOT_RIGHT_ALIGNED'}
 return $r.Bottom-$r.Top
}


function Press-ToolbarControlKey([byte]$Key) {
 [ShotKitToolbarVisual.Native]::keybd_event(0x11,0,0,[UIntPtr]::Zero);$script:controlDown=$true
 try { Press-ToolbarKey $Key }
 finally { [ShotKitToolbarVisual.Native]::keybd_event(0x11,0,2,[UIntPtr]::Zero);$script:controlDown=$false }
}
function Invoke-ToolbarViewMenu([int]$CommandId) {
 $view=[ShotKitToolbarVisual.Native]::Buttons($script:toolbar.H)|Where-Object{$_.Id -eq 0x5132}|Select-Object -First 1
 if(-not $view){throw 'ZOOM_VIEW_BUTTON_MISSING'}
 [void][ShotKitToolbarVisual.Native]::SetForegroundWindow($script:main.H)
 Click-ToolbarButton $view
 Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'ZOOM_VIEW_MENU_NOT_VISIBLE'
 Click-ToolbarMenuItem $CommandId
 Wait-ToolbarCondition { $null -eq (Read-ToolbarPopup) } 'ZOOM_VIEW_MENU_DID_NOT_CLOSE'
}
function Get-ExpectedFixtureView([double]$Factor) {
 $client=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect])
 if(-not [ShotKitToolbarVisual.Native]::GetClientRect($script:main.H,[ref]$client)){throw 'ZOOM_CLIENT_RECT_FAILED'}
 $viewportHeight=$client.Bottom-$script:wideHeight
 $width=[int][Math]::Floor(640*$Factor+0.5);$height=[int][Math]::Floor(360*$Factor+0.5)
 # These checks use centered images smaller than the viewport; no pan is needed.
 if($width -gt $client.Right -or $height -gt $viewportHeight){throw 'ZOOM_EXPECTED_VIEW_EXCEEDS_CLIENT'}
 $left=[int][Math]::Floor(($client.Right-$width)/2)
 $top=$script:wideHeight+[int][Math]::Floor(($viewportHeight-$height)/2)
 return [pscustomobject]@{Left=$left;Top=$top;Right=$left+$width;Bottom=$top+$height}
}
function Test-ToolbarFixturePixels($Expected,$ClearedPoints=@()) {
 [uint32]$center=0
 $actual=[ShotKitToolbarVisual.Native]::ImageBounds($script:main.H,$script:wideHeight,[ref]$center)
 if($actual.Left -ne $Expected.Left -or $actual.Top -ne $Expected.Top -or $actual.Right -ne $Expected.Right -or $actual.Bottom -ne $Expected.Bottom -or $center -ne 0x00f0f6f8){return $false}
 $background=[ShotKitToolbarVisual.Native]::GetSysColor(5)
 foreach($point in $ClearedPoints){if([ShotKitToolbarVisual.Native]::ClientPixel($script:main.H,$point.X,$point.Y) -ne $background){return $false}}
 return $true
}
function Get-FixtureShrinkProbes($Before,$After) {
 $cx=[int][Math]::Floor(($After.Left+$After.Right)/2);$cy=[int][Math]::Floor(($After.Top+$After.Bottom)/2)
 $probes=@(
  [pscustomobject]@{X=[int][Math]::Floor(($Before.Left+$After.Left)/2);Y=$cy},
  [pscustomobject]@{X=[int][Math]::Floor(($Before.Right+$After.Right)/2);Y=$cy},
  [pscustomobject]@{X=$cx;Y=[int][Math]::Floor(($Before.Top+$After.Top)/2)},
  [pscustomobject]@{X=$cx;Y=[int][Math]::Floor(($Before.Bottom+$After.Bottom)/2)}
 )
 $background=[ShotKitToolbarVisual.Native]::GetSysColor(5)
 foreach($point in $probes){
  if($point.X -lt $Before.Left -or $point.X -ge $Before.Right -or $point.Y -lt $Before.Top -or $point.Y -ge $Before.Bottom){throw 'ZOOM_SHRINK_PROBE_OUTSIDE_PREVIOUS_IMAGE'}
  if($point.X -ge $After.Left -and $point.X -lt $After.Right -and $point.Y -ge $After.Top -and $point.Y -lt $After.Bottom){throw 'ZOOM_SHRINK_PROBE_INSIDE_NEW_IMAGE'}
  if([ShotKitToolbarVisual.Native]::ClientPixel($script:main.H,$point.X,$point.Y) -eq $background){throw 'ZOOM_SHRINK_PROBE_WAS_NOT_PAINTED_IMAGE'}
 }
 return $probes
}

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

 $dpi=[ShotKitToolbarVisual.Native]::GetDpiForWindow($main.H)
 $wide=[int][Math]::Round(1000*$dpi/96);$tall=[int][Math]::Round(600*$dpi/96)
 if(-not [ShotKitToolbarVisual.Native]::SetWindowPos($main.H,[IntPtr]::Zero,0,0,$wide,$tall,4)){throw 'RESPONSIVE_WIDE_RESIZE_FAILED'}
 Wait-ToolbarCondition { @([ShotKitToolbarVisual.Native]::Buttons($toolbar.H)).Count -eq 26 } 'RESPONSIVE_WIDE_TOOLBAR_NOT_RESTORED'
 $buttons=@([ShotKitToolbarVisual.Native]::Buttons($toolbar.H))
 $expected=@('Auswahl (V)','Pfeil (A)','Linie (L)','Rechteck (R)','Ellipse (E)','Text (T)','Marker (M)','Verpixeln (B)','Nummer (N)','Stift (P)','Zuschneiden (C)','Farbe…','Rot','Schwarz','Gelb','Blau','Linienbreite…','Schriftgröße…','Rahmen / Schatten…','Neue Aufnahme (Strg+N)','Kopieren','Speichern','Zurück','Wiederholen','Ansicht','Datei')
 foreach($label in $expected){if(-not($buttons|Where-Object{$_.Text -ceq $label}|Select-Object -First 1)){throw "NATIVE_BUTTON_LABEL_MISSING: '$label'"}}
 $wideHeight=Assert-ToolbarRow $toolbar $buttons
 if($PopupOnly) {
  # Exercise the two editor-popup paths from actual toolbar mouse input.
  $frameButton=$buttons|Where-Object{$_.Id -eq 0x5123}|Select-Object -First 1
  $widthButton=$buttons|Where-Object{$_.Id -eq 0x5121}|Select-Object -First 1
  if(-not $frameButton -or -not $widthButton){throw 'POPUP_BUTTONS_MISSING: frame or line-width toolbar button not found.'}
  [void][ShotKitToolbarVisual.Native]::SetForegroundWindow($main.H)
  Click-ToolbarButton $frameButton
  Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'POPUP_FRAME_MENU_FIRST_OPEN_FAILED'
  $entries=@(Read-ToolbarPopup)
  if((@($entries|ForEach-Object{$_.Id}) -join ',') -ne '1,2,3' -or (@($entries|ForEach-Object{$_.Text}) -join '|') -ne 'Rahmen hinzufügen|Schlagschatten hinzufügen|Rahmen und Schlagschatten hinzufügen'){throw "POPUP_FRAME_MENU_CONTENT_FAILED: $(@($entries|ForEach-Object{"$($_.Id):$($_.Text)"}) -join ', ')"}
  Click-OutsideToolbarPopup
  Wait-ToolbarCondition { $null -eq (Read-ToolbarPopup) } 'POPUP_FRAME_OUTSIDE_DISMISS_FAILED'

  Click-ToolbarButton $frameButton
  Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'POPUP_FRAME_MENU_REOPEN_FAILED'
  Click-ToolbarMenuItem 1
  Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'POPUP_FRAME_SELECTION_DID_NOT_OPEN_WIDTH_MENU'
  $entries=@(Read-ToolbarPopup)
  if((@($entries|ForEach-Object{$_.Id}) -join ',') -ne '1,2,3,4,5,6' -or (@($entries|ForEach-Object{$_.Text}) -join ',') -ne '4,8,12,16,24,32'){throw "POPUP_FRAME_WIDTH_MENU_CONTENT_FAILED: $(@($entries|ForEach-Object{"$($_.Id):$($_.Text)"}) -join ', ')"}
  Click-OutsideToolbarPopup
  Wait-ToolbarCondition { $null -eq (Read-ToolbarPopup) } 'POPUP_FRAME_WIDTH_OUTSIDE_DISMISS_FAILED'

  # Reopen, select the frame action, then select a width. Reaching the native
  # color dialog proves both TrackPopupMenu return values reached their callers.
  Click-ToolbarButton $frameButton
  Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'POPUP_FRAME_MENU_SECOND_REOPEN_FAILED'
  Click-ToolbarMenuItem 1
  Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'POPUP_FRAME_SELECTION_DID_NOT_OPEN_WIDTH_MENU'
  Click-ToolbarMenuItem 2
  Wait-ToolbarCondition { @(Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770'}).Count -gt 0 } 'POPUP_NUMBER_SELECTION_DID_NOT_OPEN_COLOR_DIALOG'
  $colorDialog=Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770'}|Select-Object -First 1
  if([ShotKitToolbarVisual.Native]::Owner($colorDialog.H) -ne [uint32]$p.Id){throw 'POPUP_FRAME_COLOR_DIALOG_WRONG_OWNER'}
  $cancel=[ShotKitToolbarVisual.Native]::Buttons($colorDialog.H)|Where-Object{$_.Text -in @('Cancel','Abbrechen')}|Select-Object -First 1
  if(-not $cancel){throw "POPUP_FRAME_COLOR_CANCEL_MISSING: buttons=$(([ShotKitToolbarVisual.Native]::Buttons($colorDialog.H)|ForEach-Object{$_.Text}) -join ', ')"}
  [void][ShotKitToolbarVisual.Native]::SendMessageW($cancel.H,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)
  Wait-ToolbarCondition { @(Owned-Windows|Where-Object{$_.Visible -and $_.H -eq $colorDialog.H}).Count -eq 0 } 'POPUP_FRAME_COLOR_DIALOG_CANCEL_FAILED'

  Click-ToolbarButton $widthButton
  Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'POPUP_NUMBER_MENU_FIRST_OPEN_FAILED'
  $entries=@(Read-ToolbarPopup)
  if((@($entries|ForEach-Object{$_.Id}) -join ',') -ne '1,2,3,4,5,6,7,8' -or (@($entries|ForEach-Object{$_.Text}) -join ',') -ne '1,2,3,4,6,8,12,16'){throw "POPUP_NUMBER_MENU_CONTENT_FAILED: $(@($entries|ForEach-Object{"$($_.Id):$($_.Text)"}) -join ', ')"}
  Click-OutsideToolbarPopup
  Wait-ToolbarCondition { $null -eq (Read-ToolbarPopup) } 'POPUP_NUMBER_OUTSIDE_DISMISS_FAILED'
  Click-ToolbarButton $widthButton
  Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'POPUP_NUMBER_MENU_REOPEN_FAILED'
  Click-ToolbarMenuItem 2
  Wait-ToolbarCondition { $null -eq (Read-ToolbarPopup) } 'POPUP_NUMBER_SELECTION_MENU_STILL_OPEN'
  $pass='PASS: editor frame and number popups opened, dismissed by outside mouse click, reopened and selected through native menu input; selected frame action and width reached the owned color dialog.'
 } elseif($ResponsiveOnly) {
  $allOriginalIds=@($buttons|Where-Object{$_.Id -notin @(0x5131,0x5132)}|ForEach-Object{$_.Id})
  # Repeat shrink/grow to catch stale hidden controls and one-way stage state.
  foreach($logicalWidth in @(900,800,700,500,430)) {
   $narrow=[int][Math]::Round($logicalWidth*$dpi/96)
   if(-not [ShotKitToolbarVisual.Native]::SetWindowPos($main.H,[IntPtr]::Zero,0,0,$narrow,$tall,4)){throw 'RESPONSIVE_NARROW_RESIZE_FAILED'}
   Wait-ToolbarCondition { @([ShotKitToolbarVisual.Native]::Buttons($toolbar.H)|Where-Object{$_.Id -eq 0x5130}).Count -eq 1 } 'RESPONSIVE_OVERFLOW_NOT_VISIBLE'
   $narrowButtons=@([ShotKitToolbarVisual.Native]::Buttons($toolbar.H))
   if((Assert-ToolbarRow $toolbar $narrowButtons) -ne $wideHeight){throw 'RESPONSIVE_TOOLBAR_HEIGHT_CHANGED'}
   foreach($id in @(0x5114,0x5111,0x5110,0x5131)){if(-not($narrowButtons|Where-Object{$_.Id -eq $id})){throw "RESPONSIVE_FIXED_ACTION_HIDDEN: $id"}}
   $visibleIds=@($narrowButtons|ForEach-Object{$_.Id})
   if($logicalWidth -eq 900 -and (0x5123 -in $visibleIds -or 0x5132 -in $visibleIds -or 0x5121 -notin $visibleIds)){throw 'RESPONSIVE_STAGE1_GROUPS_FAILED'}
   if($logicalWidth -eq 800 -and (0x5121 -in $visibleIds -or 0x5122 -in $visibleIds -or 0x5112 -in $visibleIds -or 0x5113 -in $visibleIds -or 0x5124 -notin $visibleIds)){throw 'RESPONSIVE_STAGE2_GROUPS_FAILED'}
   if($logicalWidth -eq 700 -and (0x5124 -in $visibleIds -or 0x5125 -in $visibleIds -or 0x5126 -in $visibleIds -or 0x5127 -in $visibleIds)){throw 'RESPONSIVE_STAGE3_GROUPS_FAILED'}
   $more=$narrowButtons|Where-Object{$_.Id -eq 0x5130}|Select-Object -First 1
   [void][ShotKitToolbarVisual.Native]::SetForegroundWindow($main.H)
   Click-ToolbarButton $more
   Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'RESPONSIVE_OVERFLOW_MENU_DID_NOT_OPEN'
   $entries=@(Read-ToolbarPopup)
   $viewEntry=$entries|Where-Object{$_.Text -ceq 'Ansicht'}|Select-Object -First 1
   if(-not $viewEntry -or $viewEntry.Submenu -eq [IntPtr]::Zero){throw 'RESPONSIVE_VIEW_SUBMENU_MISSING'}
   $viewEntries=@([ShotKitToolbarVisual.Native]::MenuItems($viewEntry.Submenu))
   if((@($viewEntries|ForEach-Object{$_.Id}) -join ',') -ne (@(0x5150,0x5151,0x5152,0x5153) -join ',')){throw 'RESPONSIVE_VIEW_ACTIONS_INCOMPLETE'}
   $reachable=@($narrowButtons|ForEach-Object{$_.Id})+@($entries|ForEach-Object{$_.Id})
   foreach($id in $allOriginalIds){if($id -notin $reachable){throw "RESPONSIVE_ACTION_UNREACHABLE: $id at $logicalWidth"}}
   if($logicalWidth -eq 430) {
    # Crop is hidden here. Click the real native item verified by HMENU and ID.
    Click-ToolbarMenuItem 0x510b
    Wait-ToolbarCondition { @(Owned-Windows|Where-Object{$_.H -eq $main.H -and $_.Title -like '*Werkzeug Crop*'}).Count -eq 1 } 'RESPONSIVE_OVERFLOW_CROP_DID_NOT_EXECUTE'
   } else { Press-ToolbarKey 0x1b }
   Wait-ToolbarCondition { $null -eq (Read-ToolbarPopup) } 'RESPONSIVE_OVERFLOW_MENU_STILL_OPEN'
   # Datei must be usable at every narrow width, including a real Open workflow.
   $file=$narrowButtons|Where-Object{$_.Id -eq 0x5131}|Select-Object -First 1
   Click-ToolbarButton $file
   Wait-ToolbarCondition { $null -ne (Read-ToolbarPopup) } 'RESPONSIVE_FILE_MENU_DID_NOT_OPEN'
   $fileEntries=@(Read-ToolbarPopup)
   if((@($fileEntries|ForEach-Object{$_.Id}) -join ',') -ne (@(0x5110,0x5140,0x5141,0x5142,0x5143) -join ',')){throw 'RESPONSIVE_FILE_ACTIONS_INCOMPLETE'}
   if($logicalWidth -eq 430) {
    Click-ToolbarMenuItem 0x5141
    Wait-ToolbarCondition { @(Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770'}).Count -eq 1 } 'RESPONSIVE_FILE_OPEN_DID_NOT_EXECUTE'
    $dialog=Owned-Windows|Where-Object{$_.Visible -and $_.ClassName -eq '#32770'}|Select-Object -First 1
    $cancel=[ShotKitToolbarVisual.Native]::Buttons($dialog.H)|Where-Object{$_.Id -eq 2}|Select-Object -First 1
    if(-not $cancel){throw 'RESPONSIVE_OPEN_CANCEL_MISSING'}
    [void][ShotKitToolbarVisual.Native]::PostMessageW($cancel.H,0xF5,[IntPtr]::Zero,[IntPtr]::Zero)
    Wait-ToolbarCondition { @(Owned-Windows|Where-Object{$_.Visible -and $_.H -eq $dialog.H}).Count -eq 0 } 'RESPONSIVE_OPEN_CANCEL_FAILED'
   } else { Press-ToolbarKey 0x1b }
   Wait-ToolbarCondition { $null -eq (Read-ToolbarPopup) } 'RESPONSIVE_FILE_MENU_STILL_OPEN'
   if(-not [ShotKitToolbarVisual.Native]::SetWindowPos($main.H,[IntPtr]::Zero,0,0,$wide,$tall,4)){throw 'RESPONSIVE_RESTORE_RESIZE_FAILED'}
   Wait-ToolbarCondition { @([ShotKitToolbarVisual.Native]::Buttons($toolbar.H)).Count -eq 26 } 'RESPONSIVE_WIDE_BUTTONS_NOT_RESTORED'
   $restored=@([ShotKitToolbarVisual.Native]::Buttons($toolbar.H))
   foreach($label in $expected){if(-not($restored|Where-Object{$_.Text -ceq $label})){throw "RESPONSIVE_RESTORED_LABEL_MISSING: $label"}}
   if((Assert-ToolbarRow $toolbar $restored) -ne $wideHeight){throw 'RESPONSIVE_RESTORED_HEIGHT_CHANGED'}
  }
  # A controlled 640x360 fixture provides an observable oracle independent of
  # application zoom state: actual client pixels must occupy the expected size.
  [void][ShotKitToolbarVisual.Native]::SetForegroundWindow($main.H)
  $client=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect])
  if(-not [ShotKitToolbarVisual.Native]::GetClientRect($main.H,[ref]$client)){throw 'ZOOM_CLIENT_RECT_FAILED'}
  $fitFactor=[Math]::Min($client.Right/640.0,($client.Bottom-$wideHeight)/360.0)
  if($fitFactor -lt 1.2){throw 'ZOOM_FIXTURE_VIEWPORT_TOO_SMALL_TO_DISTINGUISH_FIT_FROM_ORIGINAL'}
  if([ShotKitToolbarVisual.Native]::GetSysColor(5) -eq 0x00f0f6f8){throw 'ZOOM_FIXTURE_BACKGROUND_NOT_DISTINGUISHABLE'}
  $fitView=Get-ExpectedFixtureView $fitFactor
  $originalView=Get-ExpectedFixtureView 1.0
  # Move off the client before pixel reads so native tooltips cannot obscure it.
  [void][ShotKitToolbarVisual.Native]::SetCursorPos(5,5)
  Press-ToolbarControlKey 0x39
  Wait-ToolbarCondition { Test-ToolbarFixturePixels $fitView } 'ZOOM_CTRL9_FIT_PIXEL_BOUNDS_FAILED'
  $originalProbes=@(Get-FixtureShrinkProbes $fitView $originalView)
  Invoke-ToolbarViewMenu 0x5152
  [void][ShotKitToolbarVisual.Native]::SetCursorPos(5,5)
  Wait-ToolbarCondition { Test-ToolbarFixturePixels $originalView $originalProbes } 'ZOOM_MENU_ORIGINAL_640X360_OR_BACKGROUND_CLEANUP_FAILED'
  Press-ToolbarControlKey 0x39
  Wait-ToolbarCondition { Test-ToolbarFixturePixels $fitView } 'ZOOM_CTRL9_RESTORE_FIT_PIXELS_FAILED'
  $originalProbes=@(Get-FixtureShrinkProbes $fitView $originalView)
  Press-ToolbarControlKey 0x30
  Wait-ToolbarCondition { Test-ToolbarFixturePixels $originalView $originalProbes } 'ZOOM_CTRL0_ORIGINAL_640X360_OR_BACKGROUND_CLEANUP_FAILED'
  # Exercise the Fit and Verkleinern menu actions, including a reduction that
  # must erase pixels previously painted outside the newly smaller image.
  Invoke-ToolbarViewMenu 0x5153
  [void][ShotKitToolbarVisual.Native]::SetCursorPos(5,5)
  Wait-ToolbarCondition { Test-ToolbarFixturePixels $fitView } 'ZOOM_MENU_FIT_PIXEL_BOUNDS_FAILED'
  $reducedView=Get-ExpectedFixtureView ($fitFactor*0.8)
  $reductionProbes=@(Get-FixtureShrinkProbes $fitView $reducedView)
  Invoke-ToolbarViewMenu 0x5151
  [void][ShotKitToolbarVisual.Native]::SetCursorPos(5,5)
  Wait-ToolbarCondition { Test-ToolbarFixturePixels $reducedView $reductionProbes } 'ZOOM_MENU_REDUCE_PIXEL_BOUNDS_OR_BACKGROUND_CLEANUP_FAILED'
  $pass='PASS: actual narrow/wide native resize; fixed right-side Aufnahme/Kopieren/Speichern/Datei; all existing commands reachable; real overflow Crop and Datei/Open activation; Ansicht/Originalgröße/Fit/Verkleinern and Ctrl+9/0 verified through actual image bounds and cleared background pixels; restoration verified.'
 } else {
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
  $editorHandle=$main.H
  $deadline=[DateTime]::UtcNow.AddSeconds(5);$alive=$null;$restored=$null
  do{
   $windows=@(Owned-Windows)
   $alive=$windows|Where-Object{$_.Visible -and $_.ClassName -eq 'ShotKitSelectionOverlay'}|Select-Object -First 1
   $restored=$windows|Where-Object{$_.Visible -and $_.H -eq $editorHandle}|Select-Object -First 1
   if(-not $alive -and $restored){break}
   Start-Sleep -Milliseconds 100
  }while([DateTime]::UtcNow -lt $deadline)
  if($alive -or -not $restored){$details=($windows|ForEach-Object{"$($_.ClassName):hwnd=$($_.H):visible=$($_.Visible):title=$($_.Title)"}) -join ', ';throw "CAPTURE_CANCEL_FAILED: $captureMode overlayVisible=$([bool]$alive), originalEditorVisible=$([bool]$restored), windows: $details"}
  $main=$restored
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
 if(-not [ShotKitToolbarVisual.Native]::SetWindowTextW($main.H,'ShotKit visual smoke fixture')){throw 'SCREENSHOT_TITLE_SANITIZE_FAILED'}
 $captureRect=[Activator]::CreateInstance([ShotKitToolbarVisual.Rect]);if(-not [ShotKitToolbarVisual.Native]::GetWindowRect($main.H,[ref]$captureRect)){throw 'SCREENSHOT_RECT_FAILED'}
 $captureBitmap=[Drawing.Bitmap]::new(($captureRect.Right-$captureRect.Left),($captureRect.Bottom-$captureRect.Top))
 try { $captureGraphics=[Drawing.Graphics]::FromImage($captureBitmap);try { $sourceDC=[ShotKitToolbarVisual.Native]::GetWindowDC($main.H);if($sourceDC -eq [IntPtr]::Zero){throw 'SCREENSHOT_DC_FAILED'};try {$targetDC=$captureGraphics.GetHdc();try {if(-not [ShotKitToolbarVisual.Native]::BitBlt($targetDC,0,0,$captureBitmap.Width,$captureBitmap.Height,$sourceDC,0,0,0x00CC0020)){throw 'SCREENSHOT_BITBLT_FAILED'}}finally{$captureGraphics.ReleaseHdc($targetDC)}}finally{[void][ShotKitToolbarVisual.Native]::ReleaseDC($main.H,$sourceDC)}}finally{$captureGraphics.Dispose()};$captureBitmap.Save($ScreenshotPath,[Drawing.Imaging.ImageFormat]::Png)}finally{$captureBitmap.Dispose()};if(-not(Test-Path -LiteralPath $ScreenshotPath -PathType Leaf) -or (Get-Item $ScreenshotPath).Length -lt 10000){throw 'SCREENSHOT_INVALID: editor capture missing or implausibly small.'}
 # The inactive diagonal line must contain antialiased edge pixels, not only
 # solid foreground and background pixels. This checks actual screen output.
 $verify=[Drawing.Bitmap]::new($ScreenshotPath)
 try {
  $lineLeft=$lr.Left-$captureRect.Left; $lineTop=$lr.Top-$captureRect.Top
  $edgePixels=0
  for($y=$lineTop+8;$y -lt $lineTop+28;$y++) {
   for($x=$lineLeft+8;$x -lt $lineLeft+28;$x++) {
    if($x -lt 0 -or $y -lt 0 -or $x -ge $verify.Width -or $y -ge $verify.Height){throw 'SMOOTH_ICON_BOUNDS_INVALID'}
    $pixel=$verify.GetPixel($x,$y)
    if([Math]::Abs([int]$pixel.R-[int]$pixel.G) -le 1 -and [Math]::Abs([int]$pixel.G-[int]$pixel.B) -le 1 -and $pixel.R -gt 20 -and $pixel.R -lt 200){$edgePixels++}
   }
  }
  if($edgePixels -eq 0){throw 'SMOOTH_ICON_EDGES_MISSING: no intermediate grayscale pixels in inactive line icon.'}
 } finally { $verify.Dispose() }
 $pass="PASS: owned 26-button native toolbar; exact tool, color, and capture tooltips; color chooser and swatch; mouse/keyboard tool selection; toolbar/shortcut capture; window and tray icons verified. Screenshot: $ScreenshotPath"
 }
} catch {$failure=$_} finally {
 if($mouse){try{[ShotKitToolbarVisual.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{$cleanup+=$_.Exception.Message}}
 if($controlDown){try{[ShotKitToolbarVisual.Native]::keybd_event(17,0,2,[UIntPtr]::Zero)}catch{$cleanup+=$_.Exception.Message}}
 if($p){try{if(-not $p.HasExited){$actual=$p.StartTime.ToUniversalTime().Ticks;$path=$p.MainModule.FileName;if($actual -ne $ticks -or -not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($path),[IO.Path]::GetFullPath($ExePath))){throw 'Refusing to stop process whose identity changed.'};$p.Kill();if(-not $p.WaitForExit(5000)){throw 'Owned app did not stop.'}};$stopped=$p.HasExited}catch{$cleanup+=$_.Exception.Message}finally{$p.Dispose()}}
 $env:APPDATA=$oldA;$env:LOCALAPPDATA=$oldL
 if($stopped -and (Test-Path -LiteralPath $root)){try{Remove-Item -LiteralPath $root -Recurse -Force}catch{$cleanup+=$_.Exception.Message}}
}
if($failure){throw $failure};if($cleanup.Count){throw ('CLEANUP_FAILURE: '+($cleanup -join '; '))};Write-Output $pass

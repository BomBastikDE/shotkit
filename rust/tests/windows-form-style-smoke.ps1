[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,
    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'This smoke test requires an interactive ephemeral GitHub Actions Windows runner.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: refusing to use or change '$imageName'."
}

Add-Type -AssemblyName System.Drawing
if (-not ('ShotKitFormStyleSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitFormStyleSmoke {
    public struct Rect { public int Left, Top, Right, Bottom; }
    public struct Point { public int X, Y; }
    public sealed class Window { public IntPtr Handle; public int Pid; public string ClassName, Title; public bool Visible; }
    public sealed class Control { public IntPtr Handle, Parent; public int Id, Pid; public string ClassName, Text; public bool Visible, Enabled; }
    public static class Native {
        private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr data);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder s, int n);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder s, int n);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr hwnd);
        [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr hwnd);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
        [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
        [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr owner, IntPtr menu, uint position, out Rect r);
        [DllImport("user32.dll")] public static extern int MenuItemFromPoint(IntPtr owner, IntPtr menu, Point p);
        [DllImport("user32.dll")] public static extern uint GetMenuItemID(IntPtr menu, int position);
        [DllImport("user32.dll")] public static extern int GetMenuItemCount(IntPtr menu);
        [DllImport("user32.dll")] public static extern uint GetMenuState(IntPtr menu, uint position, uint flags);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetMenuStringW(IntPtr menu, uint position, StringBuilder text, int count, uint flags);
        public static string MenuText(IntPtr menu, uint position) { var text=new StringBuilder(64); GetMenuStringW(menu,position,text,text.Capacity,0x400); return text.ToString(); }
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
        [StructLayout(LayoutKind.Sequential)] struct GuiInfo {
            public uint Size, Flags; public IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret; public Rect CaretRect;
        }
        [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint thread, ref GuiInfo info);
        public static IntPtr Capture(IntPtr hwnd) {
            uint pid; uint thread=GetWindowThreadProcessId(hwnd,out pid);
            var info=new GuiInfo { Size=(uint)Marshal.SizeOf(typeof(GuiInfo)) };
            if(thread==0 || !GetGUIThreadInfo(thread,ref info)) throw new InvalidOperationException("GET_GUI_THREAD_INFO_FAILED.");
            return info.Capture;
        }
        [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
        [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        static string Text(IntPtr h, bool cls) { var s=new StringBuilder(512); if(cls) GetClassNameW(h,s,s.Capacity); else GetWindowTextW(h,s,s.Capacity); return s.ToString(); }
        public static Window[] Windows(int processId) {
            var r=new List<Window>(); EnumProc cb=(h,d)=>{ uint pid; GetWindowThreadProcessId(h,out pid); if(pid==(uint)processId) r.Add(new Window{Handle=h,Pid=(int)pid,ClassName=Text(h,true),Title=Text(h,false),Visible=IsWindowVisible(h)}); return true;}; EnumWindows(cb,IntPtr.Zero); GC.KeepAlive(cb); return r.ToArray();
        }
        public static Control[] Controls(IntPtr parent) {
            var r=new List<Control>(); EnumProc cb=(h,d)=>{ r.Add(new Control{Handle=h,Parent=GetParent(h),Id=GetDlgCtrlID(h),Pid=Owner(h),ClassName=Text(h,true),Text=Text(h,false),Visible=IsWindowVisible(h),Enabled=IsWindowEnabled(h)}); return true; }; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return r.ToArray();
        }
        public static IntPtr FindToolbar(IntPtr parent) {
            IntPtr result=IntPtr.Zero; EnumProc cb=(h,d)=>{ if(IsWindowVisible(h) && Text(h,true)=="ShotKitRustEditorToolbar") { result=h; return false; } return true;}; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return result;
        }
        public static void Click(IntPtr h) { IntPtr result; if(SendMessageTimeoutW(h,0x00F5,IntPtr.Zero,IntPtr.Zero,2,3000,out result)==IntPtr.Zero) throw new TimeoutException("BM_CLICK timed out."); }
        public static IntPtr Menu(IntPtr h) { IntPtr result; return SendMessageTimeoutW(h,0x01e1,IntPtr.Zero,IntPtr.Zero,0x0002,500,out result)==IntPtr.Zero ? IntPtr.Zero : result; }
        [DllImport("user32.dll")] public static extern IntPtr GetSubMenu(IntPtr menu,int position);
        [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h,int x,int y,int w,int height,bool repaint);
        public static int Owner(IntPtr h) { uint pid; GetWindowThreadProcessId(h,out pid); return (int)pid; }
    }
}
'@
}

# Standalone interactive Windows form-style integration test; isolated settings and fixture.
# Run with an interactive Windows desktop, e.g. SHOTKIT_LOCAL_SMOKE_TEST=1.
# Export predicates inspect decoded PNG pixels, never screenshots or selection adorners.
$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-FormStyle-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$inputPath=Join-Path $root 'form-style.png'
$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA
$proc=$null;$mouseDown=$false;$failure=$null
function Get-OwnedWindows {if($null -ne $proc -and -not $proc.HasExited){[ShotKitFormStyleSmoke.Native]::Windows($proc.Id)}}
function Get-TimeoutDiagnostics {
    $details=[Collections.Generic.List[string]]::new()
    try {
        foreach($frame in @(Get-PSCallStack)) {
            $line=$frame.InvocationInfo.Line
            if($null -ne $line){$line=$line.Trim()}
            $details.Add(("CALLSTACK function={0} location={1}:{2} action='{3}' arguments='{4}'" -f $frame.FunctionName,$frame.ScriptName,$frame.ScriptLineNumber,$line,$frame.Arguments))
        }
        $foreground=[ShotKitFormStyleSmoke.Native]::GetForegroundWindow()
        $foregroundRect=[ShotKitFormStyleSmoke.Rect]::new()
        $foregroundRectValid=[ShotKitFormStyleSmoke.Native]::GetWindowRect($foreground,[ref]$foregroundRect)
        $details.Add(("FOREGROUND HWND=0x{0:X} PID={1} rectValid={2} rect={3},{4},{5},{6}" -f $foreground.ToInt64(),[ShotKitFormStyleSmoke.Native]::Owner($foreground),$foregroundRectValid,$foregroundRect.Left,$foregroundRect.Top,$foregroundRect.Right,$foregroundRect.Bottom))
        $cursor=[ShotKitFormStyleSmoke.Point]::new()
        $cursorValid=[ShotKitFormStyleSmoke.Native]::GetCursorPos([ref]$cursor)
        $cursorWindow=[ShotKitFormStyleSmoke.Native]::WindowFromPoint($cursor)
        $details.Add(("CURSOR valid={0} point={1},{2} HWND=0x{3:X} PID={4}" -f $cursorValid,$cursor.X,$cursor.Y,$cursorWindow.ToInt64(),[ShotKitFormStyleSmoke.Native]::Owner($cursorWindow)))
        $popups=@(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'})
        $details.Add(("OWNED_VISIBLE_POPUPS count={0}" -f $popups.Count))
        foreach($popup in $popups) {
            $rect=[ShotKitFormStyleSmoke.Rect]::new()
            $rectValid=[ShotKitFormStyleSmoke.Native]::GetWindowRect($popup.Handle,[ref]$rect)
            $menu=[ShotKitFormStyleSmoke.Native]::Menu($popup.Handle)
            $details.Add(("POPUP HWND=0x{0:X} PID={1} HMENU=0x{2:X} rectValid={3} rect={4},{5},{6},{7}" -f $popup.Handle.ToInt64(),$popup.Pid,$menu.ToInt64(),$rectValid,$rect.Left,$rect.Top,$rect.Right,$rect.Bottom))
        }
    } catch {$details.Add(("DIAGNOSTIC_ERROR: {0}" -f $_.Exception.Message))}
    return $details -join [Environment]::NewLine
}
function Wait-For([scriptblock]$Probe,[int]$Seconds,[string]$What) {
    $end=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $end){
        if($null -ne $proc -and $proc.HasExited){throw "PROCESS_EXITED waiting for $What."}
        $value=&$Probe;if($value){return $value};Start-Sleep -Milliseconds 100
    };throw ("TIMEOUT waiting for $What."+[Environment]::NewLine+(Get-TimeoutDiagnostics))
}
function Button([int]$Id) {
    $items=@([ShotKitFormStyleSmoke.Native]::Controls($main.Handle)|Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and $_.Id -eq $Id -and $_.Pid -eq $proc.Id})
    if($items.Count -ne 1){throw "BUTTON_IDENTITY id=$Id count=$($items.Count)."};return $items[0]
}
function Click-Button([int]$Id) {[ShotKitFormStyleSmoke.Native]::Click((Button $Id).Handle)}
function Pointer([int]$X,[int]$Y) {
    $p=[ShotKitFormStyleSmoke.Point]::new();$p.X=$X;$p.Y=$Y
    if([ShotKitFormStyleSmoke.Native]::Owner([ShotKitFormStyleSmoke.Native]::WindowFromPoint($p)) -ne $proc.Id){throw 'INPUT_OWNER_MISMATCH.'}
    if(-not [ShotKitFormStyleSmoke.Native]::SetCursorPos($X,$Y)){throw 'CURSOR_FAILED.'}
}
function Mouse-Click {
    try {[ShotKitFormStyleSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true}
    finally {[ShotKitFormStyleSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false}
}
function Wait-MenuClosed {
    [void](Wait-For {-not @(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'}).Count} 5 'native popup close')
}
function Cancel-Menu {
    [ShotKitFormStyleSmoke.Native]::keybd_event(0x1B,0,0,[UIntPtr]::Zero)
    [ShotKitFormStyleSmoke.Native]::keybd_event(0x1B,0,2,[UIntPtr]::Zero)
    Wait-MenuClosed
}
function Open-Menu([int]$ButtonId) {
    [void][ShotKitFormStyleSmoke.Native]::SetForegroundWindow($main.Handle)
    [void](Wait-For {[ShotKitFormStyleSmoke.Native]::GetForegroundWindow() -eq $main.Handle} 3 'editor foreground')
    $b=Button $ButtonId;$r=[ShotKitFormStyleSmoke.Rect]::new()
    if(-not [ShotKitFormStyleSmoke.Native]::GetWindowRect($b.Handle,[ref]$r)){throw 'BUTTON_RECT_FAILED.'}
    Pointer ([int](($r.Left+$r.Right)/2)) ([int](($r.Top+$r.Bottom)/2));Mouse-Click
    return Wait-For {
        foreach($popup in @(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'})){
            $m=[ShotKitFormStyleSmoke.Native]::Menu($popup.Handle);if($m -ne [IntPtr]::Zero){return $m}
        }
    } 5 'native popup menu'
}
function Hover-MenuItem([IntPtr]$Menu,[int]$Position) {
    $r=[ShotKitFormStyleSmoke.Rect]::new()
    if(-not [ShotKitFormStyleSmoke.Native]::GetMenuItemRect([IntPtr]::Zero,$Menu,$Position,[ref]$r)){throw 'MENU_RECT_FAILED.'}
    Pointer ([int](($r.Left+$r.Right)/2)) ([int](($r.Top+$r.Bottom)/2))
    [void](Wait-For {
        $p=[ShotKitFormStyleSmoke.Point]::new();[void][ShotKitFormStyleSmoke.Native]::GetCursorPos([ref]$p)
        $s=[ShotKitFormStyleSmoke.Native]::GetMenuState($Menu,$Position,0x400)
        [ShotKitFormStyleSmoke.Native]::MenuItemFromPoint([IntPtr]::Zero,$Menu,$p) -eq $Position -and $s -ne [uint32]::MaxValue -and ($s -band 0x83) -eq 0x80
    } 5 'enabled highlighted menu item')
}
$labels=@('Füllung: Keine','Füllung: Deckend','Füllung: Halbtransparent','Linie: Durchgezogen','Linie: Gestrichelt','Linie: Gepunktet','Rechteck-Ecken: Eckig','Rechteck-Ecken: Abgerundet','Pfeilenden: Einfach','Pfeilenden: Doppelt','Pfeilenden: Punkt','Sprechblase: Füllfarbe …','Sprechblase: Textfarbe …','Sprechblase: Spitze ein / aus','Spotlight: Abdunklung 25 %','Spotlight: Abdunklung 50 %','Spotlight: Abdunklung 75 %','Spotlight: Abdunklung 90 %','Bildlupe: 1,5×','Bildlupe: 2×','Bildlupe: 3×','Bildlupe: 4×','Bildlupe: Rund / oval','Bildlupe: Rechteck')
$script:checked=@(0,3,6,8)
$script:target='Any'
function Style-Group([int]$Index) {if($Index -le 2){return 0};if($Index -le 5){return 1};if($Index -le 7){return 2};return 3}
function Style-Enabled([int]$Index) {
    if($script:target -eq 'Any'){return $true}
    if($Index -le 2){return $script:target -eq 'Rectangle'}
    if($Index -le 5){return $true}
    if($Index -le 7){return $script:target -eq 'Rectangle'}
    if($Index -le 10){return $script:target -eq 'Arrow'}
    if($Index -le 13){return $script:target -eq 'Callout'}
    if($Index -le 17){return $script:target -eq 'Spotlight'}
    return $script:target -eq 'Magnifier'
}
function Open-StyleMenu([switch]$Overflow) {
    if($Overflow){
        if((Button 0x5130).Text -cne '»'){throw 'OVERFLOW_BUTTON_IDENTITY.'}
        $menu=Open-Menu 0x5130;$position=-1
        for($i=0;$i -lt [ShotKitFormStyleSmoke.Native]::GetMenuItemCount($menu);$i++){
            if([ShotKitFormStyleSmoke.Native]::MenuText($menu,$i) -ceq 'Formstil…'){$position=$i;break}
        }
        if($position -lt 0){throw 'OVERFLOW_FORM_STYLE_SUBMENU_MISSING.'}
        $sub=[ShotKitFormStyleSmoke.Native]::GetSubMenu($menu,$position)
        if($sub -eq [IntPtr]::Zero){throw 'OVERFLOW_FORM_STYLE_SUBMENU_INVALID.'}
        Hover-MenuItem $menu $position
        [void](Wait-For {
            foreach($popup in @(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'})){
                if([ShotKitFormStyleSmoke.Native]::Menu($popup.Handle) -eq $sub){return $true}
            }
        } 5 'visible form-style submenu');$menu=$sub
    }else{
        $button=Button 0x5128
        if($button.Text -cne 'Formstil…'){throw 'FORM_STYLE_BUTTON_LABEL.'}
        $menu=Open-Menu 0x5128
    }
    if([ShotKitFormStyleSmoke.Native]::GetMenuItemCount($menu) -ne 24){throw 'FORM_STYLE_MENU_COUNT.'}
    for($i=0;$i -lt 24;$i++){
        $expectedId=if($i -lt 14){0x5170+$i}else{0x5180+$i-14}
        if([ShotKitFormStyleSmoke.Native]::GetMenuItemID($menu,$i) -ne $expectedId -or [ShotKitFormStyleSmoke.Native]::MenuText($menu,$i) -cne $labels[$i]){throw "FORM_STYLE_MENU_IDENTITY position=$i."}
        $state=[ShotKitFormStyleSmoke.Native]::GetMenuState($menu,$i,0x400)
        if($state -eq [uint32]::MaxValue -or (($state -band 8) -ne 0) -ne ($script:checked -contains $i)){throw "FORM_STYLE_CHECKMARK position=$i state=$state."}
        if((($state -band 3) -eq 0) -ne (Style-Enabled $i)){throw "FORM_STYLE_ENABLED position=$i target=$script:target state=$state."}
    }
    return $menu
}
function Choose-Style([int]$Index,[switch]$Overflow) {
    $menu=Open-StyleMenu -Overflow:$Overflow
    if(-not (Style-Enabled $Index)){throw "TEST_INVALID_DISABLED_STYLE position=$Index."}
    Hover-MenuItem $menu $Index;Mouse-Click;Wait-MenuClosed
    $script:checked[(Style-Group $Index)]=$Index
    # Reopen through the same real mouse route: prove native checkmarks changed.
    $null=Open-StyleMenu -Overflow:$Overflow
    if($Overflow){
        # Escape closes the submenu first, then the root overflow menu.
        [ShotKitFormStyleSmoke.Native]::keybd_event(0x1B,0,0,[UIntPtr]::Zero)
        [ShotKitFormStyleSmoke.Native]::keybd_event(0x1B,0,2,[UIntPtr]::Zero)
        [void](Wait-For {@(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'}).Count -eq 1} 5 'overflow submenu dismissed')
    }
    Cancel-Menu
}
function Image-Point([int]$X,[int]$Y) {
    $r=[ShotKitFormStyleSmoke.Rect]::new();$tr=[ShotKitFormStyleSmoke.Rect]::new()
    if(-not [ShotKitFormStyleSmoke.Native]::GetClientRect($main.Handle,[ref]$r)){throw 'CLIENT_RECT_FAILED.'}
    $toolbar=[ShotKitFormStyleSmoke.Native]::FindToolbar($main.Handle)
    if($toolbar -eq [IntPtr]::Zero -or -not [ShotKitFormStyleSmoke.Native]::GetClientRect($toolbar,[ref]$tr)){throw 'TOOLBAR_MISSING.'}
    $vh=$r.Bottom-$tr.Bottom;$fit=[Math]::Min($r.Right/320.0,$vh/240.0)
    $sw=[int][Math]::Round(320*$fit,[MidpointRounding]::AwayFromZero);$sh=[int][Math]::Round(240*$fit,[MidpointRounding]::AwayFromZero)
    $p=[ShotKitFormStyleSmoke.Point]::new();[void][ShotKitFormStyleSmoke.Native]::ClientToScreen($main.Handle,[ref]$p)
    $p.X += [int][Math]::Floor(($r.Right-$sw)/2.0)+[int][Math]::Round(($X+0.5)*$sw/320.0,[MidpointRounding]::AwayFromZero)
    $p.Y += $tr.Bottom+[int][Math]::Floor(($vh-$sh)/2.0)+[int][Math]::Round(($Y+0.5)*$sh/240.0,[MidpointRounding]::AwayFromZero)
    return $p
}
function Drag-Image([int]$X1,[int]$Y1,[int]$X2,[int]$Y2) {
    $p=Image-Point $X1 $Y1;Pointer $p.X $p.Y
    try {
        [ShotKitFormStyleSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true
        [void](Wait-For {[ShotKitFormStyleSmoke.Native]::Capture($main.Handle) -eq $main.Handle} 5 'editor gesture capture')
        $p=Image-Point $X2 $Y2;Pointer $p.X $p.Y
    }finally{[ShotKitFormStyleSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false}
    [void](Wait-For {[ShotKitFormStyleSmoke.Native]::Capture($main.Handle) -eq [IntPtr]::Zero} 5 'completed image gesture')
}
function Select-Tool([int]$Id,[string]$Name) {
    Click-Button $Id
    [void](Wait-For {Get-OwnedWindows|Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like "*Werkzeug $Name*"}} 5 "$Name tool selected")
}
function Maximize-Editor {
    [void][ShotKitFormStyleSmoke.Native]::ShowWindow($main.Handle,3)
    [void](Wait-For {[ShotKitFormStyleSmoke.Native]::IsZoomed($main.Handle) -and @( [ShotKitFormStyleSmoke.Native]::Controls($main.Handle)|Where-Object {$_.Id -eq 0x5128 -and $_.Visible}).Count -eq 1} 5 'maximized form-style toolbar')
}
function Read-Export {
    $b=[Drawing.Bitmap]::new($inputPath)
    try {
        if($b.Width -ne 320 -or $b.Height -ne 240){throw 'EXPORT_DIMENSIONS.'}
        $r=[Drawing.Rectangle]::new(0,0,320,240)
        $locked=$b.LockBits($r,[Drawing.Imaging.ImageLockMode]::ReadOnly,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {$bytes=[byte[]]::new(320*240*4);for($y=0;$y -lt 240;$y++){[Runtime.InteropServices.Marshal]::Copy([IntPtr]::Add($locked.Scan0,$y*$locked.Stride),$bytes,$y*1280,1280)}}finally{$b.UnlockBits($locked)}
        $sha=[Security.Cryptography.SHA256]::Create();try{$hash=[Convert]::ToBase64String($sha.ComputeHash($bytes))}finally{$sha.Dispose()}
        return [pscustomobject]@{Bytes=$bytes;Hash=$hash}
    }finally{$b.Dispose()}
}
function Save-Export([scriptblock]$Predicate,[string]$What) {
    # Every attempt requires a newly written export, even when the expected hash
    # equals the preceding export (cancel/no-op). This rules out a stale file pass.
    Remove-Item -LiteralPath $inputPath -Force
    Click-Button 0x5110
    return Wait-For {
        try {if(Test-Path -LiteralPath $inputPath){$v=Read-Export;if(&$Predicate $v){return $v}}} catch {}
    } 10 $What
}
function Is-Ink($Image,[int]$X,[int]$Y) {
    $o=($Y*320+$X)*4
    return $Image.Bytes[$o+2] -gt 180 -and $Image.Bytes[$o+1] -lt 100 -and $Image.Bytes[$o] -lt 100 -and $Image.Bytes[$o+3] -eq 255
}
function Ink-Count($Image,[int]$X1,[int]$Y1,[int]$X2,[int]$Y2) {
    $n=0;for($y=$Y1;$y -le $Y2;$y++){for($x=$X1;$x -le $X2;$x++){if(Is-Ink $Image $x $y){$n++}}};return $n
}
function Assert-Fill($Image,[string]$Mode,[int]$X=80,[int]$Y=70) {
    # Interior patch is far from the stroke and handles; alpha must be flattened
    # over the white fixture. Both red components distinguish fill from gray UI.
    for($yy=$Y-2;$yy -le $Y+2;$yy++){for($xx=$X-2;$xx -le $X+2;$xx++){
        $o=($yy*320+$xx)*4;$r=$Image.Bytes[$o+2];$g=$Image.Bytes[$o+1];$b=$Image.Bytes[$o]
        $valid=switch($Mode){
            'None' {$r -eq 255 -and $g -eq 255 -and $b -eq 255}
            'Solid' {$r -gt 180 -and $g -lt 100 -and $b -lt 100}
            'Translucent' {$r -gt 180 -and $g -gt 110 -and $g -lt 210 -and $b -gt 110 -and $b -lt 210}
            default {throw "INVALID_FILL_MODE $Mode."}
        }
        if(-not $valid -or $Image.Bytes[$o+3] -ne 255){throw "FILL_PIXELS mode=$Mode at=($xx,$yy) RGB=($r,$g,$b)."}
    }}
}
function Assert-Rounded($Square,$Rounded,[int]$X=40) {
    # Broad corner patch tolerates subpixel mouse mapping; a filled square paints
    # most of it, a radius-12 rectangle removes it. Center/top stay filled.
    $before=Ink-Count $Square 39 39 44 44
    $after=Ink-Count $Rounded ($X-1) 39 ($X+4) 44
    if($before -lt 20 -or $after -ge $before/2){throw "ROUND_CORNER_PIXELS square=$before rounded=$after."}
    if((Ink-Count $Rounded ($X+30) 39 ($X+40) 43) -lt 20){throw 'ROUND_TOP_EDGE_MISSING.'}
}
function Line-Metrics($Image) {
    $ink=0;$gaps=0;$runs=0;$previous=$false
    for($x=60;$x -le 250;$x++){
        $mark=(Ink-Count $Image $x 142 $x 148) -ge 2
        if($mark){$ink++;if(-not $previous){$runs++}}else{$gaps++};$previous=$mark
    }
    return [pscustomobject]@{Ink=$ink;Gaps=$gaps;Runs=$runs}
}
function Assert-Arrow($Image,[string]$Ends) {
    $back=Ink-Count $Image 81 200 84 202
    $front=Ink-Count $Image 237 200 240 202
    $past=Ink-Count $Image 253 204 255 206
    $shaft=Ink-Count $Image 140 204 180 206
    if($shaft -lt 60){throw "ARROW_SHAFT_MISSING ends=$Ends."}
    $valid=switch($Ends){
        'Single' {$back -eq 0 -and $front -ge 3 -and $past -eq 0}
        'Double' {$back -ge 3 -and $front -ge 3 -and $past -eq 0}
        'Dot' {$back -eq 0 -and $front -eq 0 -and $past -ge 3}
    }
    if(-not $valid){throw "ARROW_END_PIXELS ends=$Ends back=$back front=$front past=$past."}
}
try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData -Force|Out-Null
    $b=[Drawing.Bitmap]::new(320,240,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try{$g=[Drawing.Graphics]::FromImage($b);try{$g.Clear([Drawing.Color]::White)}finally{$g.Dispose()};$b.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png)}finally{$b.Dispose()}
    $baseline=Read-Export
    try{$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru}finally{$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    $main=Wait-For {Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} $StartupTimeoutSeconds 'editor'
    Maximize-Editor
    Assert-NoStartupDialogs -TimeoutSeconds $StartupTimeoutSeconds -OwnedProcess $proc -GetWindows { Get-OwnedWindows }
    [void](Wait-For {Get-OwnedWindows|Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like '*Strg+S speichern*'}} 10 'loaded editor')
    # Native menu opens via mouse, validates identity/state, then Escape must
    # preserve pixels on the pristine fixture.
    $null=Open-StyleMenu;Cancel-Menu
    [void](Save-Export {param($v)$v.Hash -eq $baseline.Hash} 'cancel unchanged fixture pixels')

    Select-Tool 0x5104 'Rectangle';Drag-Image 40 40 120 100;$script:target='Rectangle'
    $outline=Save-Export {param($v)$v.Hash -ne $baseline.Hash} 'existing rectangle outline'
    Assert-Fill $outline 'None'
    if((Ink-Count $outline 60 39 100 43) -lt 30){throw 'RECTANGLE_OUTLINE_MISSING.'}
    Choose-Style 1
    $solid=Save-Export {param($v)$v.Hash -ne $outline.Hash} 'existing rectangle solid fill';Assert-Fill $solid 'Solid'
    Click-Button 0x5112;[void](Save-Export {param($v)$v.Hash -eq $outline.Hash} 'undo fill exact original rectangle pixels')
    Click-Button 0x5113;[void](Save-Export {param($v)$v.Hash -eq $solid.Hash} 'redo fill exact solid rectangle pixels')
    Choose-Style 2
    $translucent=Save-Export {param($v)$v.Hash -ne $solid.Hash -and $v.Hash -ne $outline.Hash} 'translucent fill';Assert-Fill $translucent 'Translucent'
    Choose-Style 0
    $none=Save-Export {param($v)$v.Hash -eq $outline.Hash} 'none fill restores white interior';Assert-Fill $none 'None'
    Choose-Style 1
    $square=Save-Export {param($v)$v.Hash -eq $solid.Hash} 'square solid rectangle restored'
    Choose-Style 7
    $rounded=Save-Export {param($v)$v.Hash -ne $square.Hash} 'rounded rectangle';Assert-Fill $rounded 'Solid';Assert-Rounded $square $rounded
    Click-Button 0x5112;[void](Save-Export {param($v)$v.Hash -eq $square.Hash} 'undo rounded corners exact square pixels')
    Click-Button 0x5113;[void](Save-Export {param($v)$v.Hash -eq $rounded.Hash} 'redo rounded corners exact rounded pixels')
    Choose-Style 6
    [void](Save-Export {param($v)$v.Hash -eq $square.Hash} 'explicit square-corner command')
    Choose-Style 7
    [void](Save-Export {param($v)$v.Hash -eq $rounded.Hash} 'explicit rounded-corner command restored')
    # A second, separately drawn rectangle must inherit solid fill and rounding.
    Drag-Image 170 40 270 100
    $inherited=Save-Export {param($v)$v.Hash -ne $rounded.Hash} 'new rectangle inherits current style'
    Assert-Fill $inherited 'Solid' 220 70
    Assert-Rounded $square $inherited 170
    $null=Open-StyleMenu;Cancel-Menu
    [void](Save-Export {param($v)$v.Hash -eq $inherited.Hash} 'cancel preserves existing styled shapes')
    # Cancellation must preserve history, too: one Undo removes only the new shape.
    Click-Button 0x5112;[void](Save-Export {param($v)$v.Hash -eq $rounded.Hash} 'cancel leaves history unchanged')
    Click-Button 0x5113;[void](Save-Export {param($v)$v.Hash -eq $inherited.Hash} 'redo new inherited shape exactly')

    Select-Tool 0x5103 'Line';Drag-Image 40 145 270 145;$script:target='Line'
    $lineSolid=Save-Export {param($v)$v.Hash -ne $inherited.Hash} 'solid line export'
    $sm=Line-Metrics $lineSolid
    if($sm.Gaps -ne 0 -or $sm.Runs -ne 1){throw 'SOLID_LINE_HAS_GAPS.'}
    Choose-Style 4
    $lineDashed=Save-Export {param($v)$v.Hash -ne $lineSolid.Hash} 'dashed line export';$dm=Line-Metrics $lineDashed
    Choose-Style 5
    $lineDotted=Save-Export {param($v)$v.Hash -ne $lineDashed.Hash -and $v.Hash -ne $lineSolid.Hash} 'dotted line export';$pm=Line-Metrics $lineDotted
    if($dm.Gaps -lt 10 -or $dm.Runs -lt 4 -or $pm.Gaps -lt 10 -or $pm.Runs -le $dm.Runs -or $sm.Ink -le $dm.Ink+10 -or $dm.Ink -le $pm.Ink+10){throw "LINE_PATTERN_GEOMETRY solid=$($sm|ConvertTo-Json -Compress) dashed=$($dm|ConvertTo-Json -Compress) dotted=$($pm|ConvertTo-Json -Compress)."}
    Choose-Style 3
    [void](Save-Export {param($v)$v.Hash -eq $lineSolid.Hash} 'solid line restored exactly')

    Select-Tool 0x5102 'Arrow';Drag-Image 70 205 250 205;$script:target='Arrow'
    $single=Save-Export {param($v)$v.Hash -ne $lineSolid.Hash} 'single arrow export';Assert-Arrow $single 'Single'
    Choose-Style 9
    $double=Save-Export {param($v)$v.Hash -ne $single.Hash} 'double arrow export';Assert-Arrow $double 'Double'
    Choose-Style 10
    $dot=Save-Export {param($v)$v.Hash -ne $single.Hash -and $v.Hash -ne $double.Hash} 'dot arrow export';Assert-Arrow $dot 'Dot'
    # Force responsive overflow; style must remain a true native submenu.
    [void][ShotKitFormStyleSmoke.Native]::ShowWindow($main.Handle,9)
    if(-not [ShotKitFormStyleSmoke.Native]::MoveWindow($main.Handle,40,40,420,520,$true)){throw 'NARROW_EDITOR_FAILED.'}
    [void](Wait-For {@([ShotKitFormStyleSmoke.Native]::Controls($main.Handle)|Where-Object {$_.Id -eq 0x5128 -and $_.Visible}).Count -eq 0} 5 'form-style toolbar overflow')
    Choose-Style 8 -Overflow
    Maximize-Editor
    $overflow=Save-Export {param($v)$v.Hash -eq $single.Hash} 'overflow-selected single arrow exact pixels';Assert-Arrow $overflow 'Single'
    Write-Output 'PASS: native Formstil mouse/menu IDs, applicability and checkmarks; cancel preserves pixels/history; rectangle fill Undo/Redo and inherited style; rounded corners, three real line patterns and three arrow ends; narrow native overflow submenu verified.'
}catch{$failure=$_}
finally{
    if($mouseDown){try{[ShotKitFormStyleSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($null -ne $proc -and -not $proc.HasExited){try{$proc.Kill();[void]$proc.WaitForExit(5000)}catch{}}
    $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
    if($null -eq $proc -or $proc.HasExited){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}

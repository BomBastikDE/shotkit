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
if (-not ('ShotKitEffectsSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitEffectsSmoke {
    public struct Rect { public int Left, Top, Right, Bottom; }
    public struct Point { public int X, Y; }
    public sealed class Window { public IntPtr Handle; public int Pid; public string ClassName, Title; public bool Visible; }
    public sealed class Control { public IntPtr Handle, Parent; public int Id, Pid; public string ClassName, Text; public bool Visible, Enabled; }
    public static class Native {

        // Narrow native identity checks for the expected history-loss message box.
        [DllImport("user32.dll", EntryPoint="GetWindow")] static extern IntPtr HistoryWindow(IntPtr h, uint command);
        [DllImport("user32.dll", EntryPoint="IsWindowEnabled")] public static extern bool HistoryEnabled(IntPtr h);
        [DllImport("user32.dll", EntryPoint="IsWindow")] public static extern bool HistoryExists(IntPtr h);
        [DllImport("user32.dll", EntryPoint="GetParent")] static extern IntPtr HistoryParent(IntPtr h);
        [DllImport("user32.dll", EntryPoint="GetDlgCtrlID")] static extern int HistoryId(IntPtr h);
        [DllImport("user32.dll", EntryPoint="PostMessageW")] public static extern bool HistoryPost(IntPtr h, uint m, IntPtr w, IntPtr l);
        static string HistoryText(IntPtr h, bool cls) {
            var text=new StringBuilder(1024);
            if(cls) GetClassNameW(h,text,text.Capacity); else GetWindowTextW(h,text,text.Capacity);
            return text.ToString();
        }
        static bool HistoryPid(IntPtr h, uint expected) { uint pid; GetWindowThreadProcessId(h,out pid); return pid==expected; }
        public static IntPtr HistoryYes(IntPtr dialog, IntPtr editor, uint pid, string expectedText) {
            if(!HistoryExists(dialog) || !HistoryPid(dialog,pid) || !HistoryPid(editor,pid) ||
                HistoryWindow(dialog,4)!=editor || HistoryEnabled(editor) || !HistoryEnabled(dialog) ||
                HistoryText(dialog,true)!="#32770" || HistoryText(dialog,false)!="Rückgängig-Verlauf")
                throw new InvalidOperationException("REDO_CONFIRMATION_OWNER_OR_MODALITY_MISMATCH");
            int messages=0, buttons=0, yesCount=0, noCount=0; IntPtr yes=IntPtr.Zero;
            EnumProc scan=(h,p)=>{
                string cls=HistoryText(h,true);
                if(cls=="Static" && HistoryText(h,false)==expectedText && HistoryParent(h)==dialog && HistoryPid(h,pid)) messages++;
                if(cls=="Button" && IsWindowVisible(h)) {
                    buttons++;
                    if(HistoryParent(h)!=dialog || !HistoryPid(h,pid) || !HistoryEnabled(h)) return true;
                    if(HistoryId(h)==6) { yesCount++; yes=h; }
                    if(HistoryId(h)==7) noCount++;
                }
                return true;
            };
            EnumChildWindows(dialog,scan,IntPtr.Zero); GC.KeepAlive(scan);
            return messages==1 && buttons==2 && yesCount==1 && noCount==1 ? yes : IntPtr.Zero;
        }
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

# Interactive spotlight/image-magnifier integration test; isolated settings and fixture.
# Run with an interactive Windows desktop, e.g. SHOTKIT_LOCAL_SMOKE_TEST=1.
# Export predicates inspect decoded PNG pixels, never screenshots or selection adorners.
$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Effects-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$inputPath=Join-Path $root 'effects.png'
$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA
$proc=$null;$mouseDown=$false;$failure=$null
function Get-OwnedWindows {if($null -ne $proc -and -not $proc.HasExited){[ShotKitEffectsSmoke.Native]::Windows($proc.Id)}}
function Confirm-RedoDiscard([int]$ExpectedRedo) {
    $expectedText="Für diese Bearbeitung werden 0 ältere Rückgängig-Schritte und $ExpectedRedo Wiederholen-Schritte entfernt. Möchten Sie fortfahren?"
    $dialog=Wait-For {
        $dialogs=@(Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770'})
        if(-not $dialogs.Count){return $null}
        if($dialogs.Count -ne 1 -or $dialogs[0].Title -cne 'Rückgängig-Verlauf'){throw 'UNEXPECTED_REDO_CONFIRMATION_DIALOG'}
        return $dialogs[0]
    } 8 'expected owned Redo-discard confirmation'
    $yes=Wait-For {
        $button=[ShotKitEffectsSmoke.Native]::HistoryYes($dialog.Handle,$main.Handle,[uint32]$proc.Id,$expectedText)
        if($button -ne [IntPtr]::Zero){return $button}
    } 8 'exact history-loss text/counts and native IDYES/IDNO'
    if(-not [ShotKitEffectsSmoke.Native]::HistoryPost($yes,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'REDO_CONFIRMATION_YES_FAILED'}
    [void](Wait-For { -not [ShotKitEffectsSmoke.Native]::HistoryExists($dialog.Handle) -and [ShotKitEffectsSmoke.Native]::HistoryEnabled($main.Handle) } 8 'history-loss confirmation completed')
}
function Get-TimeoutDiagnostics {
    $details=[Collections.Generic.List[string]]::new()
    try {
        foreach($frame in @(Get-PSCallStack)) {
            $line=$frame.InvocationInfo.Line
            if($null -ne $line){$line=$line.Trim()}
            $details.Add(("CALLSTACK function={0} location={1}:{2} action='{3}' arguments='{4}'" -f $frame.FunctionName,$frame.ScriptName,$frame.ScriptLineNumber,$line,$frame.Arguments))
        }
        $foreground=[ShotKitEffectsSmoke.Native]::GetForegroundWindow()
        $foregroundRect=[ShotKitEffectsSmoke.Rect]::new()
        $foregroundRectValid=[ShotKitEffectsSmoke.Native]::GetWindowRect($foreground,[ref]$foregroundRect)
        $details.Add(("FOREGROUND HWND=0x{0:X} PID={1} rectValid={2} rect={3},{4},{5},{6}" -f $foreground.ToInt64(),[ShotKitEffectsSmoke.Native]::Owner($foreground),$foregroundRectValid,$foregroundRect.Left,$foregroundRect.Top,$foregroundRect.Right,$foregroundRect.Bottom))
        $cursor=[ShotKitEffectsSmoke.Point]::new()
        $cursorValid=[ShotKitEffectsSmoke.Native]::GetCursorPos([ref]$cursor)
        $cursorWindow=[ShotKitEffectsSmoke.Native]::WindowFromPoint($cursor)
        $details.Add(("CURSOR valid={0} point={1},{2} HWND=0x{3:X} PID={4}" -f $cursorValid,$cursor.X,$cursor.Y,$cursorWindow.ToInt64(),[ShotKitEffectsSmoke.Native]::Owner($cursorWindow)))
        $popups=@(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'})
        $details.Add(("OWNED_VISIBLE_POPUPS count={0}" -f $popups.Count))
        foreach($popup in $popups) {
            $rect=[ShotKitEffectsSmoke.Rect]::new()
            $rectValid=[ShotKitEffectsSmoke.Native]::GetWindowRect($popup.Handle,[ref]$rect)
            $menu=[ShotKitEffectsSmoke.Native]::Menu($popup.Handle)
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
    $items=@([ShotKitEffectsSmoke.Native]::Controls($main.Handle)|Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and $_.Id -eq $Id -and $_.Pid -eq $proc.Id})
    if($items.Count -ne 1){throw "BUTTON_IDENTITY id=$Id count=$($items.Count)."};return $items[0]
}
function Click-Button([int]$Id) {
    [void][ShotKitEffectsSmoke.Native]::SetForegroundWindow($main.Handle)
    [void](Wait-For {[ShotKitEffectsSmoke.Native]::GetForegroundWindow() -eq $main.Handle} 3 'editor foreground')
    $button=Button $Id;$r=[ShotKitEffectsSmoke.Rect]::new()
    if(-not $button.Enabled -or -not [ShotKitEffectsSmoke.Native]::GetWindowRect($button.Handle,[ref]$r)){throw "BUTTON_NOT_READY id=$Id."}
    Pointer ([int](($r.Left+$r.Right)/2)) ([int](($r.Top+$r.Bottom)/2));Mouse-Click
}
function Pointer([int]$X,[int]$Y) {
    $p=[ShotKitEffectsSmoke.Point]::new();$p.X=$X;$p.Y=$Y
    if([ShotKitEffectsSmoke.Native]::Owner([ShotKitEffectsSmoke.Native]::WindowFromPoint($p)) -ne $proc.Id){throw 'INPUT_OWNER_MISMATCH.'}
    if(-not [ShotKitEffectsSmoke.Native]::SetCursorPos($X,$Y)){throw 'CURSOR_FAILED.'}
}
function Mouse-Click {
    try {[ShotKitEffectsSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true}
    finally {[ShotKitEffectsSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false}
}
function Wait-MenuClosed {
    [void](Wait-For {-not @(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'}).Count} 5 'native popup close')
}
function Cancel-Menu {
    [ShotKitEffectsSmoke.Native]::keybd_event(0x1B,0,0,[UIntPtr]::Zero)
    [ShotKitEffectsSmoke.Native]::keybd_event(0x1B,0,2,[UIntPtr]::Zero)
    Wait-MenuClosed
}
function Open-Menu([int]$ButtonId) {
    [void][ShotKitEffectsSmoke.Native]::SetForegroundWindow($main.Handle)
    [void](Wait-For {[ShotKitEffectsSmoke.Native]::GetForegroundWindow() -eq $main.Handle} 3 'editor foreground')
    $b=Button $ButtonId;$r=[ShotKitEffectsSmoke.Rect]::new()
    if(-not [ShotKitEffectsSmoke.Native]::GetWindowRect($b.Handle,[ref]$r)){throw 'BUTTON_RECT_FAILED.'}
    Pointer ([int](($r.Left+$r.Right)/2)) ([int](($r.Top+$r.Bottom)/2));Mouse-Click
    return Wait-For {
        foreach($popup in @(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'})){
            $m=[ShotKitEffectsSmoke.Native]::Menu($popup.Handle);if($m -ne [IntPtr]::Zero){return $m}
        }
    } 5 'native popup menu'
}
function Hover-MenuItem([IntPtr]$Menu,[int]$Position) {
    $r=[ShotKitEffectsSmoke.Rect]::new()
    if(-not [ShotKitEffectsSmoke.Native]::GetMenuItemRect([IntPtr]::Zero,$Menu,$Position,[ref]$r)){throw 'MENU_RECT_FAILED.'}
    Pointer ([int](($r.Left+$r.Right)/2)) ([int](($r.Top+$r.Bottom)/2))
    [void](Wait-For {
        $p=[ShotKitEffectsSmoke.Point]::new();[void][ShotKitEffectsSmoke.Native]::GetCursorPos([ref]$p)
        $s=[ShotKitEffectsSmoke.Native]::GetMenuState($Menu,$Position,0x400)
        [ShotKitEffectsSmoke.Native]::MenuItemFromPoint([IntPtr]::Zero,$Menu,$p) -eq $Position -and $s -ne [uint32]::MaxValue -and ($s -band 0x83) -eq 0x80
    } 5 'enabled highlighted menu item')
}
function Image-Point([int]$X,[int]$Y) {
    $r=[ShotKitEffectsSmoke.Rect]::new();$tr=[ShotKitEffectsSmoke.Rect]::new()
    if(-not [ShotKitEffectsSmoke.Native]::GetClientRect($main.Handle,[ref]$r)){throw 'CLIENT_RECT_FAILED.'}
    $toolbar=[ShotKitEffectsSmoke.Native]::FindToolbar($main.Handle)
    if($toolbar -eq [IntPtr]::Zero -or -not [ShotKitEffectsSmoke.Native]::GetClientRect($toolbar,[ref]$tr)){throw 'TOOLBAR_MISSING.'}
    $vh=$r.Bottom-$tr.Bottom;$fit=[Math]::Min($r.Right/320.0,$vh/240.0)
    $sw=[int][Math]::Round(320*$fit,[MidpointRounding]::AwayFromZero);$sh=[int][Math]::Round(240*$fit,[MidpointRounding]::AwayFromZero)
    $p=[ShotKitEffectsSmoke.Point]::new();[void][ShotKitEffectsSmoke.Native]::ClientToScreen($main.Handle,[ref]$p)
    $p.X += [int][Math]::Floor(($r.Right-$sw)/2.0)+[int][Math]::Round(($X+0.5)*$sw/320.0,[MidpointRounding]::AwayFromZero)
    $p.Y += $tr.Bottom+[int][Math]::Floor(($vh-$sh)/2.0)+[int][Math]::Round(($Y+0.5)*$sh/240.0,[MidpointRounding]::AwayFromZero)
    return $p
}
function Drag-Image([int]$X1,[int]$Y1,[int]$X2,[int]$Y2) {
    $p=Image-Point $X1 $Y1;Pointer $p.X $p.Y
    try {
        [ShotKitEffectsSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true
        [void](Wait-For {[ShotKitEffectsSmoke.Native]::Capture($main.Handle) -eq $main.Handle} 5 'editor gesture capture')
        $p=Image-Point $X2 $Y2;Pointer $p.X $p.Y
    }finally{[ShotKitEffectsSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false}
    [void](Wait-For {[ShotKitEffectsSmoke.Native]::Capture($main.Handle) -eq [IntPtr]::Zero} 5 'completed image gesture')
}
function Select-Tool([int]$Id,[string]$Name) {
    Click-Button $Id
    [void](Wait-For {Get-OwnedWindows|Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like "*Werkzeug: $Name*"}} 5 "$Name tool selected")
}
function Maximize-Editor {
    [void][ShotKitEffectsSmoke.Native]::ShowWindow($main.Handle,3)
    [void](Wait-For {[ShotKitEffectsSmoke.Native]::IsZoomed($main.Handle) -and @( [ShotKitEffectsSmoke.Native]::Controls($main.Handle)|Where-Object {$_.Id -eq 0x5128 -and $_.Visible}).Count -eq 1} 5 'maximized form-style toolbar')
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

function Canvas-Click([int]$X,[int]$Y) {$p=Image-Point $X $Y;Pointer $p.X $p.Y;Mouse-Click}
function Select-Effect([int]$X,[int]$Y) {Select-Tool 0x5101 'Auswahl';Canvas-Click $X $Y}
function Choose-EffectStyle([int]$Id,[string]$Label) {
    if($Id -le 0x5183){Select-Effect 70 100}else{Select-Effect 160 120}
    $menu=Open-Menu 0x5128;$position=-1
    for($i=0;$i -lt [ShotKitEffectsSmoke.Native]::GetMenuItemCount($menu);$i++){
        if([ShotKitEffectsSmoke.Native]::GetMenuItemID($menu,$i) -eq $Id){$position=$i;break}
    }
    if($position -lt 0 -or [ShotKitEffectsSmoke.Native]::MenuText($menu,$position) -cne $Label){throw "EFFECT_MENU_IDENTITY id=$Id."}
    Hover-MenuItem $menu $position;Mouse-Click;Wait-MenuClosed
    # A checkmark is required as well as a changed export: both native UI state
    # and the existing selected object must reflect the menu command.
    $menu=Open-Menu 0x5128
    $state=[ShotKitEffectsSmoke.Native]::GetMenuState($menu,$position,0x400)
    if($state -eq [uint32]::MaxValue -or ($state -band 8) -eq 0 -or ($state -band 3) -ne 0){throw "EFFECT_MENU_CHECKMARK id=$Id state=$state."}
    Cancel-Menu
}
function Assert-Patch($Image,[int]$X1,[int]$Y1,[int]$X2,[int]$Y2,[int]$Gray,[int]$Tolerance=0) {
    for($y=$Y1;$y -le $Y2;$y++){for($x=$X1;$x -le $X2;$x++){
        $o=($y*320+$x)*4
        if($Image.Bytes[$o+3] -ne 255){throw "NONOPAQUE_PIXEL at=($x,$y)."}
        foreach($c in 0..2){if([Math]::Abs([int]$Image.Bytes[$o+$c]-$Gray) -gt $Tolerance){throw "PATCH_PIXEL at=($x,$y) gray=$Gray actual=$($Image.Bytes[$o+$c])."}}
    }}
}
function Assert-UnchangedPatch($Before,$After,[int]$X1,[int]$Y1,[int]$X2,[int]$Y2) {
    for($y=$Y1;$y -le $Y2;$y++){for($x=$X1;$x -le $X2;$x++){
        $o=($y*320+$x)*4;foreach($c in 0..3){if($Before.Bytes[$o+$c] -ne $After.Bytes[$o+$c]){throw "UNCHANGED_PATCH at=($x,$y)."}}
    }}
}
function Stripe-Transitions($Image) {
    $previous=-1;$transitions=0
    # Exclude the magnifier frame and source edge; classify only strong black
    # or white samples, skipping interpolation pixels around stripe boundaries.
    for($x=104;$x -le 216;$x++){
        $o=(120*320+$x)*4;$r=[int]$Image.Bytes[$o+2]
        if($Image.Bytes[$o+3] -ne 255 -or [Math]::Abs($r-[int]$Image.Bytes[$o]) -gt 2 -or [Math]::Abs($r-[int]$Image.Bytes[$o+1]) -gt 2){throw 'MAGNIFIER_NOT_GRAYSCALE.'}
        $v=if($r -lt 40){0}elseif($r -gt 215){1}else{-1}
        if($v -ge 0){if($previous -ge 0 -and $previous -ne $v){$transitions++};$previous=$v}
    };return $transitions
}
function Assert-History($Before,$After,[string]$What) {
    Click-Button 0x5112;[void](Save-Export {param($v)$v.Hash -eq $Before.Hash} "undo $What decoded pixels")
    Click-Button 0x5113;[void](Save-Export {param($v)$v.Hash -eq $After.Hash} "redo $What decoded pixels")
}
try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData -Force|Out-Null
    $b=[Drawing.Bitmap]::new(320,240,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $g=[Drawing.Graphics]::FromImage($b);try{$g.Clear([Drawing.Color]::White)}finally{$g.Dispose()}
        for($y=60;$y -lt 180;$y++){for($x=80;$x -lt 240;$x++){
            $gray=if(([int][Math]::Floor($x/8.0)%2) -eq 0){255}else{0}
            $b.SetPixel($x,$y,[Drawing.Color]::FromArgb(255,$gray,$gray,$gray))
        }}
        # White privacy target away from the horizontal zoom-measurement row.
        # A broad source patch tolerates a one-pixel image/input rounding offset.
        for($y=108;$y -le 116;$y++){for($x=145;$x -le 159;$x++){$b.SetPixel($x,$y,[Drawing.Color]::White)}}
        $b.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png)
    }finally{$b.Dispose()}
    $baseline=Read-Export
    try{$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru}finally{$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    $main=Wait-For {Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} $StartupTimeoutSeconds 'editor'
    Maximize-Editor
    Assert-NoStartupDialogs -TimeoutSeconds $StartupTimeoutSeconds -OwnedProcess $proc -GetWindows { Get-OwnedWindows }
    [void](Wait-For {Get-OwnedWindows|Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like '*Strg+S speichern*'}} 10 'loaded editor')

    [void](Save-Export {param($v)$v.Hash -eq $baseline.Hash} 'pristine fixture decoded pixels')
    Select-Tool 0x5115 'Spotlight';Drag-Image 60 40 260 200
    $spotlight=Save-Export {param($v)$v.Hash -ne $baseline.Hash} 'spotlight creation'
    Assert-History $baseline $spotlight 'spotlight creation'
    Select-Effect 60 100
    $spotlightStates=@($baseline,$spotlight)
    $previous=$spotlight
    foreach($entry in @(
        @{Id=0x5180;Label='Spotlight: Abdunklung 25 %';Gray=191},
        @{Id=0x5181;Label='Spotlight: Abdunklung 50 %';Gray=128},
        @{Id=0x5182;Label='Spotlight: Abdunklung 75 %';Gray=64},
        @{Id=0x5183;Label='Spotlight: Abdunklung 90 %';Gray=26}
    )){
        Choose-EffectStyle $entry.Id $entry.Label
        $next=Save-Export {param($v)$v.Hash -ne $previous.Hash} $entry.Label
        Assert-Patch $next 15 15 25 25 $entry.Gray 2
        Assert-UnchangedPatch $baseline $next 100 80 220 160
        Assert-Patch $next 65 45 75 55 255
        Assert-History $previous $next $entry.Label
        $spotlightStates+=,$next
        $previous=$next
    }
    # Four strength commands plus creation: restore the exact fixture before
    # testing magnification, so dimming cannot masquerade as zoom changes.
    for($i=$spotlightStates.Count-2;$i -ge 0;$i--){
        $expected=$spotlightStates[$i]
        Click-Button 0x5112
        [void](Save-Export {param($v)$v.Hash -eq $expected.Hash} "undo spotlight state $i")
    }

    Select-Tool 0x5116 'Bildlupe';Drag-Image 80 60 240 180
    Confirm-RedoDiscard 5 # creation plus four Spotlight strength changes are undone
    $created=Save-Export {param($v)$v.Hash -ne $baseline.Hash} 'magnifier creation'
    Assert-History $baseline $created 'magnifier creation'
    Select-Effect 160 120
    Choose-EffectStyle 0x5184 'Bildlupe: 1,5×'
    $oneHalf=Save-Export {param($v)$v.Hash -ne $created.Hash} 'magnifier 1.5x'
    Assert-History $created $oneHalf 'magnifier 1.5x'
    $previous=$oneHalf;$previousTransitions=Stripe-Transitions $oneHalf
    if($previousTransitions -lt 8 -or $previousTransitions -gt 11){throw "MAGNIFIER_150_STRIPES count=$previousTransitions."}
    foreach($entry in @(
        @{Id=0x5185;Label='Bildlupe: 2×';Min=6;Max=8},
        @{Id=0x5186;Label='Bildlupe: 3×';Min=4;Max=5},
        @{Id=0x5187;Label='Bildlupe: 4×';Min=3;Max=4}
    )){
        Choose-EffectStyle $entry.Id $entry.Label
        $next=Save-Export {param($v)$v.Hash -ne $previous.Hash} $entry.Label
        $transitions=Stripe-Transitions $next
        if($transitions -lt $entry.Min -or $transitions -gt $entry.Max -or $transitions -ge $previousTransitions){throw "MAGNIFIER_ZOOM_STRIPES label=$($entry.Label) before=$previousTransitions after=$transitions."}
        Assert-UnchangedPatch $baseline $next 15 15 25 25
        Assert-History $previous $next $entry.Label
        $previous=$next;$previousTransitions=$transitions
    }
    $circle=$previous
    Assert-UnchangedPatch $baseline $circle 84 67 89 78
    Choose-EffectStyle 0x5189 'Bildlupe: Rechteck'
    $rectangle=Save-Export {param($v)$v.Hash -ne $circle.Hash} 'rectangle magnifier frame'
    $changed=0
    for($y=67;$y -le 78;$y++){for($x=84;$x -le 89;$x++){
        $o=($y*320+$x)*4;if($rectangle.Bytes[$o] -ne $baseline.Bytes[$o]){$changed++}
    }}
    if($changed -lt 30){throw "MAGNIFIER_RECTANGLE_CORNER_NOT_FILLED changed=$changed."}
    Assert-History $circle $rectangle 'magnifier rectangle frame'
    Choose-EffectStyle 0x5188 'Bildlupe: Rund / oval'
    $roundAgain=Save-Export {param($v)$v.Hash -eq $circle.Hash} 'circle frame restored exactly'
    Assert-History $rectangle $roundAgain 'magnifier circle frame'

    # Add redaction AFTER the magnifier. The source must still be sanitized.
    # At 4x, viewport (120..126,88..94) samples the white privacy target.
    # This lies outside the redaction's own x=144..176 destination, so black
    # here can only be explained by sanitized sampling, not its final overlay.
    Assert-Patch $roundAgain 120 88 126 94 255
    Select-Tool 0x510C 'Schwärzen';Drag-Image 144 104 176 134
    $redacted=Save-Export {param($v)$v.Hash -ne $roundAgain.Hash} 'magnifier source redacted after magnifier creation'
    Assert-Patch $redacted 120 88 126 94 0
    Assert-Patch $redacted 150 114 170 126 0
    Assert-History $roundAgain $redacted 'sanitized magnifier source redaction'
    Write-Output 'PASS: native mouse/menu spotlight 25/50/75/90 percent inside/outside pixels; image magnifier 1.5/2/3/4x stripe geometry and circle/rectangle frame; later source redaction; decoded PNG Undo/Redo for creation and every effect style.'
}catch{$failure=$_}
finally{
    if($mouseDown){try{[ShotKitEffectsSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($null -ne $proc -and -not $proc.HasExited){try{$proc.Kill();[void]$proc.WaitForExit(5000)}catch{}}
    $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
    if($null -eq $proc -or $proc.HasExited){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
    if($null -ne $proc){$proc.Dispose()}
}
if($null -ne $failure){throw $failure}

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
if (-not ('ShotKitStampSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitStampSmoke {
    public struct Rect { public int Left, Top, Right, Bottom; }
    public struct Point { public int X, Y; }
    public sealed class Window { public IntPtr Handle; public int Pid; public string ClassName, Title; public bool Visible; }
    public sealed class Button { public IntPtr Handle; public string Text; }
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
        public static IntPtr HistoryAnswer(IntPtr dialog, IntPtr editor, uint pid, string expectedText, bool approve) {
            if(!HistoryExists(dialog) || !HistoryPid(dialog,pid) || !HistoryPid(editor,pid) ||
                HistoryWindow(dialog,4)!=editor || HistoryEnabled(editor) || !HistoryEnabled(dialog) ||
                HistoryText(dialog,true)!="#32770" || HistoryText(dialog,false)!="Rückgängig-Verlauf")
                throw new InvalidOperationException("REDO_CONFIRMATION_OWNER_OR_MODALITY_MISMATCH");
            int messages=0, buttons=0, yesCount=0, noCount=0; IntPtr yes=IntPtr.Zero, no=IntPtr.Zero;
            EnumProc scan=(h,p)=>{
                string cls=HistoryText(h,true);
                if(cls=="Static" && HistoryText(h,false)==expectedText && HistoryParent(h)==dialog && HistoryPid(h,pid)) messages++;
                if(cls=="Button" && IsWindowVisible(h)) {
                    buttons++;
                    if(HistoryParent(h)!=dialog || !HistoryPid(h,pid) || !HistoryEnabled(h)) return true;
                    if(HistoryId(h)==6) { yesCount++; yes=h; }
                    if(HistoryId(h)==7) { noCount++; no=h; }
                }
                return true;
            };
            EnumChildWindows(dialog,scan,IntPtr.Zero); GC.KeepAlive(scan);
            return messages==1 && buttons==2 && yesCount==1 && noCount==1 ? (approve ? yes : no) : IntPtr.Zero;
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
        [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
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
        [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
        [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        static string Text(IntPtr h, bool cls) { var s=new StringBuilder(512); if(cls) GetClassNameW(h,s,s.Capacity); else GetWindowTextW(h,s,s.Capacity); return s.ToString(); }
        public static Window[] Windows(int processId) {
            var r=new List<Window>(); EnumProc cb=(h,d)=>{ uint pid; GetWindowThreadProcessId(h,out pid); if(pid==(uint)processId) r.Add(new Window{Handle=h,Pid=(int)pid,ClassName=Text(h,true),Title=Text(h,false),Visible=IsWindowVisible(h)}); return true;}; EnumWindows(cb,IntPtr.Zero); GC.KeepAlive(cb); return r.ToArray();
        }
        public static Button[] Buttons(IntPtr parent) {
            var r=new List<Button>(); EnumProc cb=(h,d)=>{ if(IsWindowVisible(h) && Text(h,true)=="Button") r.Add(new Button{Handle=h,Text=Text(h,false)}); return true;}; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return r.ToArray();
        }
        public static Control[] Controls(IntPtr parent) {
            var r=new List<Control>(); EnumProc cb=(h,d)=>{ r.Add(new Control{Handle=h,Parent=GetParent(h),Id=GetDlgCtrlID(h),Pid=Owner(h),ClassName=Text(h,true),Text=Text(h,false),Visible=IsWindowVisible(h),Enabled=IsWindowEnabled(h)}); return true; }; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return r.ToArray();
        }
        [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint msg,IntPtr w,IntPtr l);
        [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
        public static void SetText(IntPtr h,string text) {
            IntPtr p=Marshal.StringToHGlobalUni(text),result;
            try { if(SendMessageTimeoutW(h,0x000c,IntPtr.Zero,p,2,3000,out result)==IntPtr.Zero || result==IntPtr.Zero) throw new InvalidOperationException("WM_SETTEXT failed or timed out."); }
            finally { Marshal.FreeHGlobal(p); }
        }
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint msg, IntPtr w, StringBuilder l, uint flags, uint timeout, out IntPtr result);
        public static string ControlText(IntPtr h) {
            const int capacity=16;
            var buffer=new StringBuilder(capacity); IntPtr result;
            if(SendMessageTimeoutW(h,0x000d,(IntPtr)capacity,buffer,2,3000,out result)==IntPtr.Zero)
                throw new TimeoutException("WM_GETTEXT failed or timed out.");
            long length=result.ToInt64();
            if(length<0 || length>=capacity-1 || length!=buffer.Length)
                throw new InvalidOperationException("WM_GETTEXT returned invalid or truncated text length.");
            return buffer.ToString();
        }
        public static IntPtr FindToolbar(IntPtr parent) {
            IntPtr result=IntPtr.Zero; EnumProc cb=(h,d)=>{ if(IsWindowVisible(h) && Text(h,true)=="ShotKitRustEditorToolbar") { result=h; return false; } return true;}; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return result;
        }
        public static bool ClientRect(IntPtr h,out Rect r) { return GetClientRect(h,out r); }
        public static void Click(IntPtr h) { IntPtr result; if(SendMessageTimeoutW(h,0x00F5,IntPtr.Zero,IntPtr.Zero,2,3000,out result)==IntPtr.Zero) throw new TimeoutException("BM_CLICK timed out."); }
        public static IntPtr Menu(IntPtr h) { IntPtr result; return SendMessageTimeoutW(h,0x01e1,IntPtr.Zero,IntPtr.Zero,0x0002,500,out result)==IntPtr.Zero ? IntPtr.Zero : result; }
        [DllImport("user32.dll")] public static extern IntPtr GetSubMenu(IntPtr menu,int position);
        [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h,int x,int y,int w,int height,bool repaint);
        public static int Owner(IntPtr h) { uint pid; GetWindowThreadProcessId(h,out pid); return (int)pid; }
    }
}
'@
}

# Standalone interactive Windows integration test; no existing user settings or files.
$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Stamps-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$inputPath=Join-Path $root 'stamps.png'
$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA
$proc=$null;$mouseDown=$false;$failure=$null
function Confirm-RedoDiscard([int]$ExpectedRedo,[switch]$Decline) {
    $expectedText="Für diese Bearbeitung werden 0 ältere Rückgängig-Schritte und $ExpectedRedo Wiederholen-Schritte entfernt. Möchten Sie fortfahren?"
    $dialog=Wait-For {
        $dialogs=@(Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770'})
        if(-not $dialogs.Count){return $null}
        if($dialogs.Count -ne 1 -or $dialogs[0].Title -cne 'Rückgängig-Verlauf'){throw 'UNEXPECTED_REDO_CONFIRMATION_DIALOG'}
        return $dialogs[0]
    } 8 'expected owned Redo-discard confirmation'
    $yes=Wait-For {
        $button=[ShotKitStampSmoke.Native]::HistoryAnswer($dialog.Handle,$main.Handle,[uint32]$proc.Id,$expectedText,(-not $Decline))
        if($button -ne [IntPtr]::Zero){return $button}
    } 8 'exact history-loss text/counts and native IDYES/IDNO'
    if(-not [ShotKitStampSmoke.Native]::HistoryPost($yes,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'REDO_CONFIRMATION_ANSWER_FAILED'}
    [void](Wait-For { -not [ShotKitStampSmoke.Native]::HistoryExists($dialog.Handle) -and [ShotKitStampSmoke.Native]::HistoryEnabled($main.Handle) } 8 'history-loss confirmation completed')
}
function Get-OwnedWindows {if($null -ne $proc -and -not $proc.HasExited){[ShotKitStampSmoke.Native]::Windows($proc.Id)}}
function Wait-For([scriptblock]$Probe,[int]$Seconds,[string]$What) {
    $end=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $end){
        if($null -ne $proc -and $proc.HasExited){throw "PROCESS_EXITED waiting for $What."}
        $value=&$Probe;if($value){return $value};Start-Sleep -Milliseconds 100
    };throw "TIMEOUT waiting for $What."
}
function Button([int]$Id) {
    $items=@([ShotKitStampSmoke.Native]::Controls($main.Handle)|Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and $_.Id -eq $Id})
    if($items.Count -ne 1){throw "BUTTON_IDENTITY id=$Id count=$($items.Count)."};return $items[0]
}
function Click-Button([int]$Id) {[ShotKitStampSmoke.Native]::Click((Button $Id).Handle)}
function Pointer([int]$X,[int]$Y) {
    $p=[ShotKitStampSmoke.Point]::new();$p.X=$X;$p.Y=$Y
    if([ShotKitStampSmoke.Native]::Owner([ShotKitStampSmoke.Native]::WindowFromPoint($p)) -ne $proc.Id){throw 'INPUT_OWNER_MISMATCH.'}
    if(-not [ShotKitStampSmoke.Native]::SetCursorPos($X,$Y)){throw 'CURSOR_FAILED.'}
}
function Mouse-Click {
    try {[ShotKitStampSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true;Start-Sleep -Milliseconds 60}
    finally {[ShotKitStampSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false}
    Start-Sleep -Milliseconds 100
}
function Open-Menu([int]$ButtonId) {
    [void][ShotKitStampSmoke.Native]::SetForegroundWindow($main.Handle)
    [void](Wait-For {[ShotKitStampSmoke.Native]::GetForegroundWindow() -eq $main.Handle} 3 'editor foreground')
    $b=Button $ButtonId;$r=[ShotKitStampSmoke.Rect]::new()
    if(-not [ShotKitStampSmoke.Native]::GetWindowRect($b.Handle,[ref]$r)){throw 'BUTTON_RECT_FAILED.'}
    Pointer ([int](($r.Left+$r.Right)/2)) ([int](($r.Top+$r.Bottom)/2));Mouse-Click
    return Wait-For {
        foreach($popup in @(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'})){
            $m=[ShotKitStampSmoke.Native]::Menu($popup.Handle);if($m -ne [IntPtr]::Zero){return $m}
        }
    } 5 'native popup menu'
}
function Hover-MenuItem([IntPtr]$Menu,[int]$Position) {
    $r=[ShotKitStampSmoke.Rect]::new()
    if(-not [ShotKitStampSmoke.Native]::GetMenuItemRect([IntPtr]::Zero,$Menu,$Position,[ref]$r)){throw 'MENU_RECT_FAILED.'}
    Pointer ([int](($r.Left+$r.Right)/2)) ([int](($r.Top+$r.Bottom)/2))
    [void](Wait-For {
        $p=[ShotKitStampSmoke.Point]::new();[void][ShotKitStampSmoke.Native]::GetCursorPos([ref]$p)
        $s=[ShotKitStampSmoke.Native]::GetMenuState($Menu,$Position,0x400)
        [ShotKitStampSmoke.Native]::MenuItemFromPoint([IntPtr]::Zero,$Menu,$p) -eq $Position -and $s -ne [uint32]::MaxValue -and ($s -band 0x83) -eq 0x80
    } 5 'enabled highlighted menu item')
}
$labels=@('Haken','Kreuz','Warnung','Information','Mauszeiger')
function Choose-Stamp([int]$Index,[switch]$Overflow) {
    if($Overflow){
        $menu=Open-Menu 0x5130;$position=-1
        for($i=0;$i -lt [ShotKitStampSmoke.Native]::GetMenuItemCount($menu);$i++){
            if([ShotKitStampSmoke.Native]::MenuText($menu,$i) -eq 'Stempel auswählen…'){$position=$i;break}
        }
        if($position -lt 0){throw 'OVERFLOW_STAMP_SUBMENU_MISSING.'}
        $sub=[ShotKitStampSmoke.Native]::GetSubMenu($menu,$position)
        if($sub -eq [IntPtr]::Zero){throw 'OVERFLOW_STAMP_SUBMENU_INVALID.'}
        Hover-MenuItem $menu $position
        [void](Wait-For {
            foreach($popup in @(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'})){
                if([ShotKitStampSmoke.Native]::Menu($popup.Handle) -eq $sub){return $true}
            }
        } 5 'visible stamp submenu');$menu=$sub
    }else{$menu=Open-Menu 0x510E}
    if([ShotKitStampSmoke.Native]::GetMenuItemCount($menu) -ne 5){throw 'STAMP_MENU_COUNT.'}
    for($i=0;$i -lt 5;$i++){
        if([ShotKitStampSmoke.Native]::GetMenuItemID($menu,$i) -ne (0x5160+$i) -or [ShotKitStampSmoke.Native]::MenuText($menu,$i) -cne $labels[$i]){throw "STAMP_MENU_IDENTITY position=$i."}
    }
    Hover-MenuItem $menu $Index;Mouse-Click
    [void](Wait-For {-not @(Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32768'}).Count} 5 'stamp popup close')
    [void](Wait-For {Get-OwnedWindows|Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like '*Werkzeug Stamp*'}} 5 'Stamp tool selected')
}
function Image-Point([int]$X,[int]$Y) {
    $r=[ShotKitStampSmoke.Rect]::new();$tr=[ShotKitStampSmoke.Rect]::new()
    if(-not [ShotKitStampSmoke.Native]::GetClientRect($main.Handle,[ref]$r)){throw 'CLIENT_RECT_FAILED.'}
    $toolbar=[ShotKitStampSmoke.Native]::FindToolbar($main.Handle)
    if($toolbar -eq [IntPtr]::Zero -or -not [ShotKitStampSmoke.Native]::GetClientRect($toolbar,[ref]$tr)){throw 'TOOLBAR_MISSING.'}
    $vh=$r.Bottom-$tr.Bottom;$fit=[Math]::Min($r.Right/320.0,$vh/240.0)
    $sw=[int][Math]::Round(320*$fit,[MidpointRounding]::AwayFromZero);$sh=[int][Math]::Round(240*$fit,[MidpointRounding]::AwayFromZero)
    $p=[ShotKitStampSmoke.Point]::new();[void][ShotKitStampSmoke.Native]::ClientToScreen($main.Handle,[ref]$p)
    $p.X += [int][Math]::Floor(($r.Right-$sw)/2.0)+[int][Math]::Round(($X+0.5)*$sw/320.0,[MidpointRounding]::AwayFromZero)
    $p.Y += $tr.Bottom+[int][Math]::Floor(($vh-$sh)/2.0)+[int][Math]::Round(($Y+0.5)*$sh/240.0,[MidpointRounding]::AwayFromZero)
    return $p
}
function Click-Image([int]$X,[int]$Y) {$p=Image-Point $X $Y;Pointer $p.X $p.Y;Mouse-Click}
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
    Click-Button 0x5110
    return Wait-For {try{$v=Read-Export;if(&$Predicate $v){return $v}}catch{}} 10 $What
}
# Probe known ink and known empty regions, independently of rasterizer code.
# Each pair is a normalized x/y location in the stamp's square.
$ink=@(
    @(@(.20,.56),@(.40,.71),@(.72,.38)),
    @(@(.28,.28),@(.50,.50),@(.28,.72)),
    @(@(.50,.17),@(.20,.82),@(.50,.50)),
    @(@(.50,.13),@(.50,.30),@(.50,.60)),
    @(@(.25,.27),@(.40,.50),@(.60,.79))
)
$empty=@(@(@(.70,.72),@(.25,.25)),@(@(.50,.22),@(.22,.50)),@(@(.36,.60),@(.64,.60)),@(@(.30,.50),@(.70,.50)),@(@(.75,.25),@(.78,.82)))
function Assert-Symbol($Image,[int]$Index,[int]$Size=28) {
    $red=0;$out=0
    for($y=0;$y -lt 240;$y++){for($x=0;$x -lt 320;$x++){
        $o=($y*320+$x)*4;$b=$Image.Bytes
        if($b[$o+2] -gt 180 -and $b[$o+1] -lt 100 -and $b[$o] -lt 100){$red++;if([Math]::Abs($x-160) -gt ($Size/2+2) -or [Math]::Abs($y-120) -gt ($Size/2+2)){$out++}}
    }}
    if($red -lt 30 -or $out -ne 0){throw "STAMP_EXPORT_REGION $($labels[$Index]): red=$red outside=$out."}
    foreach($kind in @('ink','empty')){
        $probes=if($kind -eq 'ink'){$ink[$Index]}else{$empty[$Index]}
        foreach($q in $probes){
            $x=[int][Math]::Floor(160-$Size/2+$Size*$q[0]);$y=[int][Math]::Floor(120-$Size/2+$Size*$q[1]);$o=($y*320+$x)*4
            $isRed=$Image.Bytes[$o+2] -gt 180 -and $Image.Bytes[$o+1] -lt 130 -and $Image.Bytes[$o] -lt 130
            if(($kind -eq 'ink' -and -not $isRed) -or ($kind -eq 'empty' -and ($Image.Bytes[$o] -lt 240 -or $Image.Bytes[$o+1] -lt 240 -or $Image.Bytes[$o+2] -lt 240))){throw "STAMP_GEOMETRY $($labels[$Index]) $kind probe ($x,$y)."}
        }
    }
    return $red
}
try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData -Force|Out-Null
    $b=[Drawing.Bitmap]::new(320,240,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try{$g=[Drawing.Graphics]::FromImage($b);try{$g.Clear([Drawing.Color]::White)}finally{$g.Dispose()};$b.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png)}finally{$b.Dispose()}
    $baseline=Read-Export
    try{$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru}finally{$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    $main=Wait-For {Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} $StartupTimeoutSeconds 'editor'
    [void][ShotKitStampSmoke.Native]::ShowWindow($main.Handle,3)
    Assert-NoStartupDialogs -TimeoutSeconds $StartupTimeoutSeconds -OwnedProcess $proc -GetWindows { Get-OwnedWindows }
    [void](Wait-For {Get-OwnedWindows|Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like '*Strg+S speichern*'}} 10 'loaded editor')
    $hashes=@()
    for($i=0;$i -lt 5;$i++){
        Choose-Stamp $i;Click-Image 160 120
        if($i -eq 1){
            Confirm-RedoDiscard 1 -Decline
            [void](Save-Export {param($v)$v.Hash -eq $baseline.Hash} 'rejected new stamp preserves original pixels')
            Click-Button 0x5113
            [void](Save-Export {param($v)$v.Hash -eq $hashes[0]} 'rejected new stamp preserves exact previous Redo')
            Click-Button 0x5112
            [void](Save-Export {param($v)$v.Hash -eq $baseline.Hash} 'undo preserved previous stamp after rejection')
            Click-Image 160 120
        }
        if($i -gt 0){Confirm-RedoDiscard 1} # each previous symbol ends undone
        $saved=Save-Export {param($v)$v.Hash -ne $baseline.Hash} 'visible stamp export'
        [void](Assert-Symbol $saved $i)
        if($hashes -contains $saved.Hash){throw "STAMP_SYMBOL_NOT_DISTINCT $($labels[$i])."};$hashes+=$saved.Hash
        Click-Button 0x5112;[void](Save-Export {param($v)$v.Hash -eq $baseline.Hash} 'undo exact original pixels')
        Click-Button 0x5113;[void](Save-Export {param($v)$v.Hash -eq $saved.Hash} 'redo exact symbol pixels')
        Click-Button 0x5112;[void](Save-Export {param($v)$v.Hash -eq $baseline.Hash} 'clear symbol for next selection')
    }
    # The narrow editor must expose the same native stamp submenu in overflow.
    [void][ShotKitStampSmoke.Native]::ShowWindow($main.Handle,9)
    if(-not [ShotKitStampSmoke.Native]::MoveWindow($main.Handle,40,40,420,520,$true)){throw 'NARROW_EDITOR_FAILED.'}
    [void](Wait-For {@([ShotKitStampSmoke.Native]::Controls($main.Handle)|Where-Object {$_.Id -eq 0x510E -and $_.Visible}).Count -eq 0} 5 'stamp toolbar overflow')
    Choose-Stamp 1 -Overflow
    [void][ShotKitStampSmoke.Native]::ShowWindow($main.Handle,3);Start-Sleep -Milliseconds 250
    Click-Image 160 120
    Confirm-RedoDiscard 1 # final loop symbol is undone
    $small=Save-Export {param($v)$v.Hash -ne $baseline.Hash} 'overflow-selected cross export';$smallCount=Assert-Symbol $small 1
    # Mouse-up leaves the stamp selected. Resize its lower-right square handle.
    $p=Image-Point 174 134;Pointer $p.X $p.Y
    try{
        [ShotKitStampSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$mouseDown=$true;Start-Sleep -Milliseconds 100
        $p=Image-Point 188 148;Pointer $p.X $p.Y;Start-Sleep -Milliseconds 200
    }finally{[ShotKitStampSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$mouseDown=$false}
    $large=Save-Export {param($v)$v.Hash -ne $small.Hash} 'resized stamp export';$largeCount=Assert-Symbol $large 1 56
    if($largeCount -lt 3*$smallCount){throw "STAMP_RESIZE_AREA small=$smallCount large=$largeCount."}
    Click-Button 0x5112;[void](Save-Export {param($v)$v.Hash -eq $small.Hash} 'undo exact resize pixels')
    Click-Button 0x5113;[void](Save-Export {param($v)$v.Hash -eq $large.Hash} 'redo exact resize pixels')
    Write-Output 'PASS: five native stamp selections produced distinct geometric PNG symbols; exact pixel Undo/Redo, overflow submenu selection and selected-handle resize verified.'
}catch{$failure=$_}
finally{
    if($mouseDown){try{[ShotKitStampSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($null -ne $proc -and -not $proc.HasExited){try{$proc.Kill();[void]$proc.WaitForExit(5000)}catch{}}
    $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
    if($null -eq $proc -or $proc.HasExited){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}

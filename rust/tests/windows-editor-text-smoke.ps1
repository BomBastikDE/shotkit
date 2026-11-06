[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $StartupTimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
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
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitTextSmoke {
    public struct Rect { public int Left, Top, Right, Bottom; }
    public struct Point { public int X, Y; }
    public sealed class Win { public IntPtr Handle; public int Pid; public string ClassName, Title; public bool Visible; }
    public sealed class Child { public IntPtr Handle; public string ClassName, Text; public bool Visible; }
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
        delegate bool EnumProc(IntPtr h, IntPtr d);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr d);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr d);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
        [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out Rect r);
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint msg, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
        [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
        [DllImport("user32.dll")] public static extern bool GetMenuItemRect(IntPtr owner, IntPtr menu, uint position, out Rect r);
        [DllImport("user32.dll")] public static extern int MenuItemFromPoint(IntPtr owner, IntPtr menu, Point p);
        [DllImport("user32.dll")] public static extern uint GetMenuItemID(IntPtr menu, int position);
        [DllImport("user32.dll")] public static extern int GetMenuItemCount(IntPtr menu);
        [DllImport("user32.dll")] public static extern uint GetMenuState(IntPtr menu, uint position, uint flags);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetMenuStringW(IntPtr menu, uint position, StringBuilder text, int count, uint flags);
        public static string MenuText(IntPtr menu, uint position) { var text=new StringBuilder(64); GetMenuStringW(menu,position,text,text.Capacity,0x400); return text.ToString(); }
        [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern uint GetDoubleClickTime();
        [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
        [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
        [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
        static extern IntPtr SendMessageText(IntPtr h, uint m, IntPtr w, [Out] StringBuilder l);
        [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
        static string Read(IntPtr h, bool cls) { var s=new StringBuilder(512); if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString(); }
        public static string ClassName(IntPtr h) { return Read(h,true); }
        public static string Title(IntPtr h) { return Read(h,false); }
        public static Win[] Windows(int wanted) { var a=new List<Win>(); EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)wanted)a.Add(new Win{Handle=h,Pid=(int)p,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
        public static Child[] Children(IntPtr parent) { var a=new List<Child>();EnumProc cb=(h,d)=>{a.Add(new Child{Handle=h,ClassName=Read(h,true),Text=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
        public static string EditText(IntPtr h) {
            long length=SendMessageW(h,0x000E,IntPtr.Zero,IntPtr.Zero).ToInt64(); // WM_GETTEXTLENGTH
            if(length<0 || length>Int32.MaxValue-1) throw new InvalidOperationException("WM_GETTEXTLENGTH returned an invalid edit length.");
            var text=new StringBuilder((int)length+1);
            long copied=SendMessageText(h,0x000D,(IntPtr)text.Capacity,text).ToInt64(); // WM_GETTEXT
            if(copied<0 || copied>text.Capacity-1) throw new InvalidOperationException("WM_GETTEXT returned an invalid character count.");
            return text.ToString(0,(int)copied);
        }
        public static bool SetEditText(IntPtr h,string s) {
            IntPtr p=Marshal.StringToHGlobalUni(s);
            try { return SendMessageW(h,0x000C,IntPtr.Zero,p).ToInt64()!=0; } // WM_SETTEXT
            finally { Marshal.FreeHGlobal(p); }
        }
        public static void Click(IntPtr h) { SendMessageW(h,0x00F5,IntPtr.Zero,IntPtr.Zero); }
        public static bool Client(IntPtr h,out Rect r) { return GetClientRect(h,out r); }
        public static bool WindowRect(IntPtr h,out Rect r) { return GetWindowRect(h,out r); }
        public static IntPtr Menu(IntPtr h) { IntPtr result; return SendMessageTimeoutW(h,0x01e1,IntPtr.Zero,IntPtr.Zero,0x0002,500,out result)==IntPtr.Zero ? IntPtr.Zero : result; }
        public static int Owner(IntPtr h) { uint p;GetWindowThreadProcessId(h,out p);return (int)p; }
        public static int Id(IntPtr h) { return GetDlgCtrlID(h); }
    }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Text-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'; $inputPath = Join-Path $root 'input.png'
$diagnosticPath = Join-Path $root 'uia-diagnostics.log'
$oldAppData = $env:APPDATA; $oldLocalAppData = $env:LOCALAPPDATA; $oldDiagnosticPath = $env:SHOTKIT_UIA_DIAGNOSTICS_PATH
$proc = $null; $mouseDown = $false; $failure = $null
function Owned-Windows { if ($null -eq $proc -or $proc.HasExited) { return @() }; @([ShotKitTextSmoke.Native]::Windows($proc.Id)) }
function Wait-For([scriptblock]$Probe,[int]$Seconds,[string]$What) {
    $until=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $until) { if($null -ne $proc -and $proc.HasExited){throw "PROCESS_EXITED waiting for $What (code $($proc.ExitCode))."}; $v=&$Probe; if($null -ne $v -and $v){return $v}; Start-Sleep -Milliseconds 100 }
    throw "TIMEOUT waiting for $What."
}
function Button([IntPtr]$Parent,[string]$Label) {
    $b=[ShotKitTextSmoke.Native]::Children($Parent) | Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and (($_.Text -replace '&','').Trim() -eq $Label)} | Select-Object -First 1
    if($null -eq $b){throw "BUTTON_NOT_FOUND: $Label"}; [ShotKitTextSmoke.Native]::Click($b.Handle)
}
function Canvas-Click([int]$ImageX,[int]$ImageY) {
    $o=[Activator]::CreateInstance([ShotKitTextSmoke.Point]); if(-not [ShotKitTextSmoke.Native]::ClientToScreen($main.Handle,[ref]$o)){throw 'Could not map editor origin.'}
    $sx=$o.X+$viewX+[int][Math]::Floor(($ImageX+0.5)*$shownW/160.0); $sy=$o.Y+$viewY+[int][Math]::Floor(($ImageY+0.5)*$shownH/120.0)
    if(-not [ShotKitTextSmoke.Native]::SetForegroundWindow($main.Handle)){throw 'EDITOR_FOCUS_FAILED.'}
    $p=[Activator]::CreateInstance([ShotKitTextSmoke.Point]);$p.X=$sx;$p.Y=$sy
    if(-not [ShotKitTextSmoke.Native]::SetCursorPos($sx,$sy)){throw 'Could not position pointer.'}
    Start-Sleep -Milliseconds 200
    # Resolve ownership after pointer movement and immediately before the
    # mouse event, so hover UI is included in the actual input-target check.
    $target=[ShotKitTextSmoke.Native]::WindowFromPoint($p)
    $targetPid=[ShotKitTextSmoke.Native]::Owner($target)
    $cursor=[Activator]::CreateInstance([ShotKitTextSmoke.Point]);[void][ShotKitTextSmoke.Native]::GetCursorPos([ref]$cursor)
    $targetRect=[Activator]::CreateInstance([ShotKitTextSmoke.Rect]);$rectOk=[ShotKitTextSmoke.Native]::WindowRect($target,[ref]$targetRect)
    if($targetPid -ne $proc.Id){throw "INPUT_TARGET_MISMATCH before click: requested screen=($sx,$sy), cursor=($($cursor.X),$($cursor.Y)), HWND=$target, class='$([ShotKitTextSmoke.Native]::ClassName($target))', title='$([ShotKitTextSmoke.Native]::Title($target))', owner PID=$targetPid, expected test PID=$($proc.Id), target rect valid=$rectOk ($($targetRect.Left),$($targetRect.Top))-($($targetRect.Right),$($targetRect.Bottom)), editor HWND=$($main.Handle), image point=($ImageX,$ImageY), view=($viewX,$viewY ${shownW}x${shownH})."}
    $script:lastCanvasClick="screen=($sx,$sy), cursor=($($cursor.X),$($cursor.Y)), target HWND=$target class='$([ShotKitTextSmoke.Native]::ClassName($target))' title='$([ShotKitTextSmoke.Native]::Title($target))' PID=$targetPid; requested image=($ImageX,$ImageY); test view origin=($viewX,$viewY), size=${shownW}x${shownH}"
    [ShotKitTextSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true;Start-Sleep -Milliseconds 80
    [ShotKitTextSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
}
function Canvas-DoubleClick([int]$ImageX,[int]$ImageY) {
    $o=[Activator]::CreateInstance([ShotKitTextSmoke.Point]); if(-not [ShotKitTextSmoke.Native]::ClientToScreen($main.Handle,[ref]$o)){throw 'Could not map editor origin.'}
    $sx=$o.X+$viewX+[int][Math]::Floor(($ImageX+0.5)*$shownW/160.0); $sy=$o.Y+$viewY+[int][Math]::Floor(($ImageY+0.5)*$shownH/120.0)
    if(-not [ShotKitTextSmoke.Native]::SetForegroundWindow($main.Handle)){throw 'EDITOR_FOCUS_FAILED.'}
    if(-not [ShotKitTextSmoke.Native]::SetCursorPos($sx,$sy)){throw 'Could not position pointer for double-click.'}
    Start-Sleep -Milliseconds 200
    $p=[Activator]::CreateInstance([ShotKitTextSmoke.Point]);$p.X=$sx;$p.Y=$sy
    $target=[ShotKitTextSmoke.Native]::WindowFromPoint($p)
    if([ShotKitTextSmoke.Native]::Owner($target) -ne $proc.Id){throw 'INPUT_TARGET_MISMATCH before double-click.'}
    $limit=[ShotKitTextSmoke.Native]::GetDoubleClickTime()
    [ShotKitTextSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true
    Start-Sleep -Milliseconds 70
    [ShotKitTextSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
    Start-Sleep -Milliseconds ([Math]::Min(120,[Math]::Max(20,[int]$limit-100)))
    [ShotKitTextSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true
    Start-Sleep -Milliseconds 70
    [ShotKitTextSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
}
function Text-Dialog([string]$Description) {
    Wait-For { Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustTextEntryDialog' -and $_.Title -eq 'Text eingeben'} | Select-Object -First 1 } 8 $Description
}
function Check-DialogText([string]$Expected,[string]$Stage) {
    $d=Text-Dialog 'owned text entry dialog';
    if([ShotKitTextSmoke.Native]::Owner($d.Handle) -ne $proc.Id){throw 'TEXT_DIALOG_OWNER_MISMATCH.'}
    $edit=[ShotKitTextSmoke.Native]::Children($d.Handle) | Where-Object {$_.Visible -and $_.ClassName -eq 'Edit'} | Select-Object -First 1
    if($null -eq $edit){throw 'TEXT_DIALOG_EDIT_CONTROL_MISSING.'}
    $actual=[ShotKitTextSmoke.Native]::EditText($edit.Handle)
    if($actual -cne $Expected){
        $clickDiag=@(Get-Content -LiteralPath $diagnosticPath -ErrorAction SilentlyContinue | Where-Object {$_ -like 'text-click *'} | Select-Object -Last 1) -join ''
        throw "TEXT_DIALOG_INITIAL_CONTENT_WRONG at ${Stage}: expected '$Expected', got '$actual'. Last canvas click: $lastCanvasClick. Product text-click diagnostic: '$clickDiag'. Main title='$([ShotKitTextSmoke.Native]::Title($main.Handle))'."
    }
    return $d
}
function Confirm-RedoDiscard([int]$ExpectedRedo) {
    $expectedText="Für diese Bearbeitung werden 0 ältere Rückgängig-Schritte und $ExpectedRedo Wiederholen-Schritte entfernt. Möchten Sie fortfahren?"
    $dialog=Wait-For {
        $dialogs=@(Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770'})
        if(-not $dialogs.Count){return $null}
        if($dialogs.Count -ne 1 -or $dialogs[0].Title -cne 'Rückgängig-Verlauf'){throw 'UNEXPECTED_REDO_CONFIRMATION_DIALOG'}
        return $dialogs[0]
    } 8 'expected owned Redo-discard confirmation'
    $yes=Wait-For {
        $button=[ShotKitTextSmoke.Native]::HistoryYes($dialog.Handle,$main.Handle,[uint32]$proc.Id,$expectedText)
        if($button -ne [IntPtr]::Zero){return $button}
    } 8 'exact history-loss text/counts and native IDYES/IDNO'
    if(-not [ShotKitTextSmoke.Native]::HistoryPost($yes,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'REDO_CONFIRMATION_YES_FAILED'}
    [void](Wait-For { -not [ShotKitTextSmoke.Native]::HistoryExists($dialog.Handle) -and [ShotKitTextSmoke.Native]::HistoryEnabled($main.Handle) } 8 'history-loss confirmation completed')
}
function Choose-FontSize48 {
    $button=[ShotKitTextSmoke.Native]::Children($main.Handle) | Where-Object { $_.Visible -and $_.ClassName -eq 'Button' } | Where-Object { ($_.Text -replace '&','').Trim() -eq 'Schriftgröße…' } | Select-Object -First 1
    if($null -eq $button){throw 'FONT_SIZE_BUTTON_MISSING.'}
    if(-not [ShotKitTextSmoke.Native]::SetForegroundWindow($main.Handle) -or [ShotKitTextSmoke.Native]::GetForegroundWindow() -ne $main.Handle){throw 'STYLE_EDITOR_FOCUS_FAILED.'}
    $rect=[Activator]::CreateInstance([ShotKitTextSmoke.Rect])
    if(-not [ShotKitTextSmoke.Native]::GetWindowRect($button.Handle,[ref]$rect)){throw 'FONT_SIZE_BUTTON_RECT_FAILED.'}
    $x=[int][Math]::Floor(($rect.Left+$rect.Right)/2); $y=[int][Math]::Floor(($rect.Top+$rect.Bottom)/2)
    if(-not [ShotKitTextSmoke.Native]::SetCursorPos($x,$y)){throw 'FONT_SIZE_BUTTON_CURSOR_FAILED.'}
    $point=[Activator]::CreateInstance([ShotKitTextSmoke.Point]); [void][ShotKitTextSmoke.Native]::GetCursorPos([ref]$point)
    if([ShotKitTextSmoke.Native]::WindowFromPoint($point) -ne $button.Handle){throw 'FONT_SIZE_BUTTON_TARGET_MISMATCH.'}
    try {
        [ShotKitTextSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero); $script:mouseDown=$true
    } finally {
        [ShotKitTextSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero); $script:mouseDown=$false
    }
    $popup=Wait-For { Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32768'} | Select-Object -First 1 } 5 'font size popup'
    $menu=Wait-For { $m=[ShotKitTextSmoke.Native]::Menu($popup.Handle); if($m -ne [IntPtr]::Zero){$m} } 5 'font size menu handle'
    # The size-48 entry has command ID 6 and position 5 in this menu.
    if([ShotKitTextSmoke.Native]::GetMenuItemCount($menu) -ne 8 -or [ShotKitTextSmoke.Native]::GetMenuItemID($menu,5) -ne 6 -or [ShotKitTextSmoke.Native]::MenuText($menu,5) -ne '48'){throw 'FONT_SIZE_MENU_IDENTITY_MISMATCH.'}
    if(-not [ShotKitTextSmoke.Native]::GetMenuItemRect([IntPtr]::Zero,$menu,5,[ref]$rect) -or $rect.Right -le $rect.Left -or $rect.Bottom -le $rect.Top){throw 'FONT_SIZE_MENU_RECT_FAILED.'}
    $point.X=[int][Math]::Floor(($rect.Left+$rect.Right)/2); $point.Y=[int][Math]::Floor(($rect.Top+$rect.Bottom)/2)
    if([ShotKitTextSmoke.Native]::MenuItemFromPoint([IntPtr]::Zero,$menu,$point) -ne 5 -or -not [ShotKitTextSmoke.Native]::SetCursorPos($point.X,$point.Y)){throw 'FONT_SIZE_MENU_TARGET_INVALID.'}
    [void](Wait-For {
        $actual=[Activator]::CreateInstance([ShotKitTextSmoke.Point]); [void][ShotKitTextSmoke.Native]::GetCursorPos([ref]$actual)
        $state=[ShotKitTextSmoke.Native]::GetMenuState($menu,5,0x400)
        [ShotKitTextSmoke.Native]::WindowFromPoint($actual) -eq $popup.Handle -and [ShotKitTextSmoke.Native]::MenuItemFromPoint([IntPtr]::Zero,$menu,$actual) -eq 5 -and $state -ne [uint32]::MaxValue -and ($state -band 0x83) -eq 0x80
    } 5 'enabled highlighted font size 48')
    $current=Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32768' -and $_.Handle -eq $popup.Handle} | Select-Object -First 1
    if($null -eq $current -or [ShotKitTextSmoke.Native]::Menu($current.Handle) -ne $menu -or [ShotKitTextSmoke.Native]::GetMenuItemID($menu,5) -ne 6 -or [ShotKitTextSmoke.Native]::MenuText($menu,5) -ne '48'){throw 'FONT_SIZE_MENU_CHANGED.'}
    $actual=[Activator]::CreateInstance([ShotKitTextSmoke.Point])
    if(-not [ShotKitTextSmoke.Native]::GetCursorPos([ref]$actual) -or [ShotKitTextSmoke.Native]::WindowFromPoint($actual) -ne $popup.Handle -or [ShotKitTextSmoke.Native]::MenuItemFromPoint([IntPtr]::Zero,$menu,$actual) -ne 5){throw 'FONT_SIZE_MENU_TARGET_CHANGED.'}
    try {
        [ShotKitTextSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero); $script:mouseDown=$true
    } finally {
        [ShotKitTextSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero); $script:mouseDown=$false
    }
    [void](Wait-For { -not @(Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32768'}).Count } 5 'font size popup close')
}
function Test-SameImage($a,$b) {
    if($a.Width -ne $b.Width -or $a.Height -ne $b.Height){return $false}
    for($i=0;$i -lt $a.Pixels.Length;$i++){if($a.Pixels[$i].ToArgb() -ne $b.Pixels[$i].ToArgb()){return $false}}
    return $true
}
function Save-AndWaitImage([scriptblock]$Predicate,[string]$Description) {
    Button $main.Handle 'Speichern'
    return Wait-For { try {$image=Read-Image $inputPath; if(&$Predicate $image){return $image}} catch {} } 15 $Description
}
function Read-Image([string]$Path) {
    $bmp=[Drawing.Bitmap]::new($Path)
    try { $colors=[Array]::CreateInstance([Drawing.Color],$bmp.Width*$bmp.Height); for($y=0;$y -lt $bmp.Height;$y++){for($x=0;$x -lt $bmp.Width;$x++){$colors[$y*$bmp.Width+$x]=$bmp.GetPixel($x,$y)}}; return [pscustomobject]@{Width=$bmp.Width;Height=$bmp.Height;Pixels=$colors} }
    finally { $bmp.Dispose() }
}
function Wait-ForTextPlacement {
    Wait-For {
        $lines=@(Get-Content -LiteralPath $diagnosticPath -ErrorAction SilentlyContinue)
        $lines | Where-Object {$_ -match '^text-place image=\(30,30\) placed=true shapes=1$'} | Select-Object -Last 1
    } 5 'text placement confirmation from product diagnostics'
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData -Force | Out-Null
    $bmp=[Drawing.Bitmap]::new(160,120,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {$g=[Drawing.Graphics]::FromImage($bmp);try{$g.Clear([Drawing.Color]::Transparent)}finally{$g.Dispose()};$bmp.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png)}finally{$bmp.Dispose()}
    try {
        $env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData
        $env:SHOTKIT_UIA_DIAGNOSTICS_PATH=$diagnosticPath
        $proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru
    }
    finally {
        $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
        if($null -eq $oldDiagnosticPath){Remove-Item Env:SHOTKIT_UIA_DIAGNOSTICS_PATH -ErrorAction SilentlyContinue}else{$env:SHOTKIT_UIA_DIAGNOSTICS_PATH=$oldDiagnosticPath}
    }
    $main=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'} | Select-Object -First 1} $StartupTimeoutSeconds 'editor window'
    [void][ShotKitTextSmoke.Native]::ShowWindow($main.Handle,3);[void][ShotKitTextSmoke.Native]::SetForegroundWindow($main.Handle)
    $welcome=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1} 12 'startup notice'
    Button $welcome.Handle 'OK'
    $main=Wait-For {Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*'} | Select-Object -First 1} 10 'loaded editor'
    $client=[Activator]::CreateInstance([ShotKitTextSmoke.Rect]);if(-not [ShotKitTextSmoke.Native]::Client($main.Handle,[ref]$client)){throw 'Could not read editor size.'}
    $toolbar=[ShotKitTextSmoke.Native]::Children($main.Handle) | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'} | Select-Object -First 1
    if($null -eq $toolbar){throw 'EDITOR_NOT_READY: toolbar missing.'}
    $tb=[Activator]::CreateInstance([ShotKitTextSmoke.Rect]);if(-not [ShotKitTextSmoke.Native]::Client($toolbar.Handle,[ref]$tb)){throw 'Could not read toolbar size.'}
    $viewW=$client.Right;$viewH=$client.Bottom-$tb.Bottom;$fit=[Math]::Min($viewW/160.0,$viewH/120.0)
    $shownW=[int][Math]::Round(160*$fit,[MidpointRounding]::AwayFromZero);$shownH=[int][Math]::Round(120*$fit,[MidpointRounding]::AwayFromZero)
    $viewX=[int][Math]::Floor(($viewW-$shownW)/2.0);$viewY=$tb.Bottom+[int][Math]::Floor(($viewH-$shownH)/2.0)
    Button $main.Handle 'Text (T)'
    [void](Wait-For {Owned-Windows | Where-Object {$_.Title -like '*Werkzeug Text*'} | Select-Object -First 1} 5 'active Text tool')

    $original='smoke original';$changed='smoke edited'
    Canvas-Click 30 30
    $d=Text-Dialog 'new text dialog';$e=[ShotKitTextSmoke.Native]::Children($d.Handle) | Where-Object {$_.ClassName -eq 'Edit'} | Select-Object -First 1
    if($null -eq $e){throw 'TEXT_DIALOG_EDIT_CONTROL_MISSING.'}
    if(-not [ShotKitTextSmoke.Native]::SetEditText($e.Handle,$original)){throw 'TEXT_SET_FAILED: synchronous WM_SETTEXT returned zero for the test-owned dialog Edit control.'}
    $entered=[ShotKitTextSmoke.Native]::EditText($e.Handle)
    if($entered -cne $original){throw "TEXT_ENTRY_NOT_ACCEPTED: input control reads '$entered' before confirmation; expected '$original'."}
    Button $d.Handle 'OK'
    [void](Wait-For {@(@(Owned-Windows | Where-Object {$_.ClassName -eq 'ShotKitRustTextEntryDialog'}).Count -eq 0)} 2 'text creation dialog close')
    $placement=Wait-ForTextPlacement
    Write-Output "TEXT_PLACEMENT_STATUS: $placement"

    # Keep Text selected and click within the new object's hit bounds.
    [void](Wait-For {Owned-Windows | Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like '*Werkzeug Text*'} | Select-Object -First 1} 5 'Text tool remaining active after creation')
    # Click well inside the text bounds rather than near the top-left tolerance edge.
    Canvas-Click 40 40
    $d=Check-DialogText $original 'first reopen after creation'
    $e=[ShotKitTextSmoke.Native]::Children($d.Handle) | Where-Object {$_.ClassName -eq 'Edit'} | Select-Object -First 1
    if(-not [ShotKitTextSmoke.Native]::SetEditText($e.Handle,$changed)){throw 'TEXT_SET_FAILED: synchronous WM_SETTEXT returned zero while editing the existing text.'}
    $changedInput=[ShotKitTextSmoke.Native]::EditText($e.Handle)
    if($changedInput -cne $changed){throw "TEXT_EDIT_NOT_ACCEPTED: input control reads '$changedInput' before confirmation; expected '$changed'."}
    Button $d.Handle 'OK'
    Start-Sleep -Milliseconds 250
    Canvas-Click 40 40
    $d=Check-DialogText $changed 'reopen after saved text edit'
    Button $d.Handle 'Abbrechen'

    Button $main.Handle 'Zurück'
    Start-Sleep -Milliseconds 250
    Canvas-Click 40 40
    $d=Check-DialogText $original 'reopen after Undo'
    Button $d.Handle 'Abbrechen'
    Button $main.Handle 'Auswahl (V)'
    [void](Wait-For {Owned-Windows | Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like '*Werkzeug Select*'} | Select-Object -First 1} 5 'active Select tool for text double-click')
    Canvas-DoubleClick 40 40
    $d=Check-DialogText $original 'double-click on existing text with Select tool'
    Button $d.Handle 'Abbrechen'

    # Select the existing text, save a baseline, then change its size through
    # the actual native popup. Compare decoded PNG pixels, not file metadata.
    Canvas-Click 40 40
    Button $main.Handle 'Speichern'
    $baseline=Wait-For { try { $im=Read-Image $inputPath; if($im.Width -eq 160 -and $im.Height -eq 120 -and @($im.Pixels | Where-Object {$_.A -gt 0}).Count -gt 0){$im} } catch {} } 15 'baseline text export'
    Choose-FontSize48
    Confirm-RedoDiscard 1 # the saved text edit was undone; one redo entry remains
    $large=Save-AndWaitImage { param($image) -not (Test-SameImage $image $baseline) } 'font size 48 exported pixels'
    $baseYs=[System.Collections.Generic.List[int]]::new(); $largeYs=[System.Collections.Generic.List[int]]::new()
    for($y=0;$y -lt 120;$y++){for($x=0;$x -lt 160;$x++){
        if($baseline.Pixels[$y*160+$x].A -gt 0){$baseYs.Add($y)}
        if($large.Pixels[$y*160+$x].A -gt 0){$largeYs.Add($y)}
    }}
    if($baseYs.Count -eq 0 -or $largeYs.Count -eq 0){throw 'FONT_SIZE_TEXT_PIXELS_MISSING.'}
    $baseHeight=(($baseYs|Measure-Object -Maximum).Maximum) - (($baseYs|Measure-Object -Minimum).Minimum) + 1
    $largeHeight=(($largeYs|Measure-Object -Maximum).Maximum) - (($largeYs|Measure-Object -Minimum).Minimum) + 1
    if($largeHeight -lt $baseHeight+5){throw "FONT_SIZE_EXPORT_WRONG: baseline height=$baseHeight, size 48 height=$largeHeight."}
    Button $main.Handle 'Zurück'
    [void](Save-AndWaitImage { param($image) Test-SameImage $image $baseline } 'undo restoring original text pixels')
    Button $main.Handle 'Wiederholen'
    [void](Save-AndWaitImage { param($image) Test-SameImage $image $large } 'redo restoring size 48 text pixels')
    Write-Output "PASS: selected text size changed through native menu (export height $baseHeight to $largeHeight), with exact pixel Undo and Redo."

    Button $main.Handle 'Nummer (N)'
    [void](Wait-For {Owned-Windows | Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like '*Werkzeug Step*'} | Select-Object -First 1} 5 'active Number tool')
    Canvas-Click 80 80
    Button $main.Handle 'Speichern'
    $firstMarkerHash=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash
    $doubleClickTime=[ShotKitTextSmoke.Native]::GetDoubleClickTime()
    Start-Sleep -Milliseconds ([int]$doubleClickTime+100)
    Canvas-DoubleClick 80 80
    [void](Wait-For {Owned-Windows | Where-Object {$_.Handle -eq $main.Handle -and $_.Title -like '*Werkzeug Step*'} | Select-Object -First 1} 5 'Number tool remains active after canvas double-click')
    Button $main.Handle 'Speichern'
    $secondMarkerHash=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash
    if($secondMarkerHash -ceq $firstMarkerHash){throw 'DOUBLE_CLICK_MARK_NOT_SAVED: saved image did not change after the quick second Number click.'}
    Button $main.Handle 'Zurück'
    Button $main.Handle 'Speichern'
    $oneUndoHash=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash
    if($oneUndoHash -ceq $firstMarkerHash){throw 'DOUBLE_CLICK_CREATED_ONLY_ONE_MARK: one Undo removed all marks created by the quick double-click.'}
    Button $main.Handle 'Zurück'
    Button $main.Handle 'Speichern'
    $twoUndoHash=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash
    if($twoUndoHash -cne $firstMarkerHash){throw 'DOUBLE_CLICK_UNDO_DID_NOT_RESTORE_BASELINE: two Undo actions did not restore the saved single-marker image.'}
    Write-Output 'PASS: created and edited text, verified Undo, reopened existing text with a Select-tool double-click, and verified a quick Number double-click created two separately undoable saved markers while keeping Number active.'
}
catch {
    $failure=$_
    $lines=@(Get-Content -LiteralPath $diagnosticPath -ErrorAction SilentlyContinue | Select-Object -Last 80)
    if($lines.Count -gt 0){$failure=[Exception]::new("$($_ | Out-String)`nSHOTKIT_UIA_DIAGNOSTICS ($diagnosticPath):`n$($lines -join [Environment]::NewLine)")}
}
finally {
    if($mouseDown){try{[ShotKitTextSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($null -ne $proc -and -not $proc.HasExited){try{$proc.Kill();[void]$proc.WaitForExit(5000)}catch{}}
    if($oldAppData){$env:APPDATA=$oldAppData};if($oldLocalAppData){$env:LOCALAPPDATA=$oldLocalAppData}
    if($null -eq $oldDiagnosticPath){Remove-Item Env:SHOTKIT_UIA_DIAGNOSTICS_PATH -ErrorAction SilentlyContinue}else{$env:SHOTKIT_UIA_DIAGNOSTICS_PATH=$oldDiagnosticPath}
    if($null -eq $proc -or $proc.HasExited){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}

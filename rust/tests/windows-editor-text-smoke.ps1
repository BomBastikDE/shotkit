[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $StartupTimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
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
        delegate bool EnumProc(IntPtr h, IntPtr d);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr d);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr d);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
        [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out Rect r);
        [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out Rect r);
        [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
        [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
        static string Read(IntPtr h, bool cls) { var s=new StringBuilder(512); if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString(); }
        public static string ClassName(IntPtr h) { return Read(h,true); }
        public static string Title(IntPtr h) { return Read(h,false); }
        public static Win[] Windows(int wanted) { var a=new List<Win>(); EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)wanted)a.Add(new Win{Handle=h,Pid=(int)p,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
        public static Child[] Children(IntPtr parent) { var a=new List<Child>();EnumProc cb=(h,d)=>{a.Add(new Child{Handle=h,ClassName=Read(h,true),Text=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
        public static string EditText(IntPtr h) { return Read(h,false); }
        public static void SetEditText(IntPtr h,string s) { IntPtr p=Marshal.StringToHGlobalUni(s);try{SendMessageW(h,0x000C,IntPtr.Zero,p);}finally{Marshal.FreeHGlobal(p);} }
        public static void Click(IntPtr h) { SendMessageW(h,0x00F5,IntPtr.Zero,IntPtr.Zero); }
        public static bool Client(IntPtr h,out Rect r) { return GetClientRect(h,out r); }
        public static bool WindowRect(IntPtr h,out Rect r) { return GetWindowRect(h,out r); }
        public static int Owner(IntPtr h) { uint p;GetWindowThreadProcessId(h,out p);return (int)p; }
        public static int Id(IntPtr h) { return GetDlgCtrlID(h); }
    }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Text-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'; $inputPath = Join-Path $root 'input.png'
$oldAppData = $env:APPDATA; $oldLocalAppData = $env:LOCALAPPDATA; $proc = $null; $mouseDown = $false; $failure = $null
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
    [ShotKitTextSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true;Start-Sleep -Milliseconds 80
    [ShotKitTextSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
}
function Text-Dialog([string]$Description) {
    Wait-For { Owned-Windows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustTextEntryDialog' -and $_.Title -eq 'Text eingeben'} | Select-Object -First 1 } 8 $Description
}
function Check-DialogText([string]$Expected) {
    $d=Text-Dialog 'owned text entry dialog';
    if([ShotKitTextSmoke.Native]::Owner($d.Handle) -ne $proc.Id){throw 'TEXT_DIALOG_OWNER_MISMATCH.'}
    $edit=[ShotKitTextSmoke.Native]::Children($d.Handle) | Where-Object {$_.Visible -and $_.ClassName -eq 'Edit'} | Select-Object -First 1
    if($null -eq $edit){throw 'TEXT_DIALOG_EDIT_CONTROL_MISSING.'}
    $actual=[ShotKitTextSmoke.Native]::EditText($edit.Handle)
    if($actual -cne $Expected){throw "TEXT_DIALOG_INITIAL_CONTENT_WRONG: expected '$Expected', got '$actual'."}
    return $d
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData -Force | Out-Null
    $bmp=[Drawing.Bitmap]::new(160,120,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {$g=[Drawing.Graphics]::FromImage($bmp);try{$g.Clear([Drawing.Color]::Transparent)}finally{$g.Dispose()};$bmp.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png)}finally{$bmp.Dispose()}
    try {$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru}
    finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
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
    if($null -eq $e){throw 'TEXT_DIALOG_EDIT_CONTROL_MISSING.'};[ShotKitTextSmoke.Native]::SetEditText($e.Handle,$original);Button $d.Handle 'OK'
    [void](Wait-For {@(@(Owned-Windows | Where-Object {$_.ClassName -eq 'ShotKitRustTextEntryDialog'}).Count -eq 0)} 2 'text creation dialog close')

    # Keep Text selected and click within the new object's hit bounds.
    Canvas-Click 32 31
    $d=Check-DialogText $original
    $e=[ShotKitTextSmoke.Native]::Children($d.Handle) | Where-Object {$_.ClassName -eq 'Edit'} | Select-Object -First 1
    [ShotKitTextSmoke.Native]::SetEditText($e.Handle,$changed);Button $d.Handle 'OK'
    Start-Sleep -Milliseconds 250
    Canvas-Click 32 31
    $d=Check-DialogText $changed
    Button $d.Handle 'Abbrechen'

    Button $main.Handle 'Zurück'
    Start-Sleep -Milliseconds 250
    Canvas-Click 32 31
    $d=Check-DialogText $original
    Button $d.Handle 'Abbrechen'
    Write-Output 'PASS: created a text object, reopened it with the Text tool and verified initial text, saved an edit, reopened the edited content, and verified Undo restored the original text.'
}
catch {$failure=$_}
finally {
    if($mouseDown){try{[ShotKitTextSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($null -ne $proc -and -not $proc.HasExited){try{$proc.Kill();[void]$proc.WaitForExit(5000)}catch{}}
    if($oldAppData){$env:APPDATA=$oldAppData};if($oldLocalAppData){$env:LOCALAPPDATA=$oldLocalAppData}
    if($null -eq $proc -or $proc.HasExited){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}

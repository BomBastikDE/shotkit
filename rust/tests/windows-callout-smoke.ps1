[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $StartupTimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'This smoke test requires an interactive ephemeral GitHub Actions Windows runner or an explicitly opted-in local session.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) { throw "PREEXISTING_PROCESS: refusing to interact with '$imageName'." }

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitCalloutSmoke {
 public struct Rect { public int Left,Top,Right,Bottom; }
 public struct Point { public int X,Y; }
 public sealed class Win { public IntPtr H; public int Pid,Id; public string ClassName,Title; public bool Visible; }
 public static class N {
  delegate bool EnumProc(IntPtr h,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc cb,IntPtr d);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
  [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll",EntryPoint="SendMessageW",CharSet=CharSet.Unicode,SetLastError=true)] static extern IntPtr SendMessageText(IntPtr h,uint m,IntPtr w,[Out] StringBuilder l);
  [DllImport("user32.dll",SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,IntPtr w,IntPtr l,uint f,uint ms,out IntPtr r);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,uint x,uint y,uint data,UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int c);
  [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr h);
  [DllImport("user32.dll")] static extern IntPtr GetWindowLongPtrW(IntPtr h,int i);
  static string Read(IntPtr h,bool cls){var s=new StringBuilder(512);if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString();}
  static int Pid(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return (int)p;}
  public static Win[] Windows(int pid){var a=new List<Win>();EnumProc cb=(h,d)=>{int p=Pid(h);if(p==pid)a.Add(new Win{H=h,Pid=p,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static Win[] Children(IntPtr parent){var a=new List<Win>();EnumProc cb=(h,d)=>{a.Add(new Win{H=h,Id=GetDlgCtrlID(h),ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static int Owner(IntPtr h){return Pid(h);}
  public static long Style(IntPtr h){return GetWindowLongPtrW(h,-16).ToInt64();}
  public static string EditText(IntPtr h){long n=SendMessageW(h,0x000E,IntPtr.Zero,IntPtr.Zero).ToInt64();if(n<0||n>100000)throw new Exception("Invalid WM_GETTEXTLENGTH.");var s=new StringBuilder((int)n+1);long copied=SendMessageText(h,0x000D,(IntPtr)s.Capacity,s).ToInt64();if(copied<0||copied>s.Capacity-1)throw new Exception("Invalid WM_GETTEXT result.");return s.ToString(0,(int)copied);}
  public static void SetText(IntPtr h,string text){IntPtr p=Marshal.StringToHGlobalUni(text),r;try{if(SendMessageTimeoutW(h,0x000C,IntPtr.Zero,p,2,3000,out r)==IntPtr.Zero)throw new TimeoutException("WM_SETTEXT failed or timed out.");}finally{Marshal.FreeHGlobal(p);}}
  public static void Click(IntPtr h){IntPtr r;if(SendMessageTimeoutW(h,0x00F5,IntPtr.Zero,IntPtr.Zero,2,3000,out r)==IntPtr.Zero)throw new TimeoutException("BM_CLICK failed or timed out.");}
 }
}
'@

$root=Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Callout-Smoke-'+[Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata';$localAppData=Join-Path $root 'localappdata';$inputPath=Join-Path $root 'input.png'
$oldAppData=$env:APPDATA;$oldLocalAppData=$env:LOCALAPPDATA
$proc=$null;$mouseDown=$false;$failure=$null
function Owned-Windows { if($null -eq $proc -or $proc.HasExited){return @()};[ShotKitCalloutSmoke.N]::Windows($proc.Id) }
function Wait-For([scriptblock]$Probe,[int]$Seconds,[string]$What) {
    $until=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $until){if($null -ne $proc -and $proc.HasExited){throw "PROCESS_EXITED waiting for $What (code $($proc.ExitCode))."};$v=&$Probe;if($null -ne $v -and $v){return $v};Start-Sleep -Milliseconds 100}
    throw "TIMEOUT waiting for $What."
}
function Child-ById([IntPtr]$Parent,[int]$Id) {
    $items=@([ShotKitCalloutSmoke.N]::Children($Parent)|Where-Object {$_.Visible -and $_.Id -eq $Id})
    if($items.Count -ne 1){throw "CONTROL_IDENTITY id=$Id count=$($items.Count)."};$items[0]
}
function Click-Id([IntPtr]$Parent,[int]$Id){[ShotKitCalloutSmoke.N]::Click((Child-ById $Parent $Id).H)}
function Canvas-Click([int]$X,[int]$Y,[switch]$Double) {
    $client=[Activator]::CreateInstance([ShotKitCalloutSmoke.Rect]);$toolbar=@([ShotKitCalloutSmoke.N]::Children($main.H)|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'}|Select-Object -First 1)
    if($toolbar.Count -ne 1 -or -not [ShotKitCalloutSmoke.N]::GetClientRect($main.H,[ref]$client)){throw 'EDITOR_VIEWPORT_UNAVAILABLE.'}
    $tb=[Activator]::CreateInstance([ShotKitCalloutSmoke.Rect]);if(-not [ShotKitCalloutSmoke.N]::GetClientRect($toolbar[0].H,[ref]$tb)){throw 'TOOLBAR_RECT_UNAVAILABLE.'}
    $vh=$client.Bottom-$tb.Bottom;$fit=[Math]::Min($client.Right/320.0,$vh/240.0);$sw=[int][Math]::Round(320*$fit);$sh=[int][Math]::Round(240*$fit)
    $o=[Activator]::CreateInstance([ShotKitCalloutSmoke.Point]);if(-not [ShotKitCalloutSmoke.N]::ClientToScreen($main.H,[ref]$o)){throw 'EDITOR_ORIGIN_UNAVAILABLE.'}
    $sx=$o.X+[int][Math]::Floor(($client.Right-$sw)/2.0)+[int][Math]::Round(($X+0.5)*$sw/320.0);$sy=$o.Y+$tb.Bottom+[int][Math]::Floor(($vh-$sh)/2.0)+[int][Math]::Round(($Y+0.5)*$sh/240.0)
    if(-not [ShotKitCalloutSmoke.N]::SetForegroundWindow($main.H)){throw 'EDITOR_FOCUS_FAILED.'};Start-Sleep -Milliseconds 150
    $p=[Activator]::CreateInstance([ShotKitCalloutSmoke.Point]);$p.X=$sx;$p.Y=$sy
    if(-not [ShotKitCalloutSmoke.N]::SetCursorPos($sx,$sy)){throw 'CURSOR_FAILED.'};Start-Sleep -Milliseconds 80
    if([ShotKitCalloutSmoke.N]::Owner([ShotKitCalloutSmoke.N]::WindowFromPoint($p)) -ne $proc.Id){throw 'INPUT_TARGET_MISMATCH.'}
    $clicks=if($Double){2}else{1}
    for($i=0;$i -lt $clicks;$i++){
        [ShotKitCalloutSmoke.N]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true;Start-Sleep -Milliseconds 60
        [ShotKitCalloutSmoke.N]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
        if($i -eq 0 -and $Double){Start-Sleep -Milliseconds 100}
    }
}
function Text-Dialog([string]$What){Wait-For {Owned-Windows|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustTextEntryDialog'}|Select-Object -First 1} 8 $What}
function Edit-Control($Dialog){$e=@([ShotKitCalloutSmoke.N]::Children($Dialog.H)|Where-Object {$_.Visible -and $_.ClassName -eq 'Edit'});if($e.Count -ne 1){throw "EDIT_CONTROL_COUNT=$($e.Count)."};$e[0]}
function Confirm-Callout([string]$Text) {
    $d=Text-Dialog 'native multiline callout text dialog'
    if([ShotKitCalloutSmoke.N]::Owner($d.H) -ne $proc.Id){throw 'TEXT_DIALOG_OWNER_MISMATCH.'}
    $e=Edit-Control $d
    if(([ShotKitCalloutSmoke.N]::Style($e.H) -band 0x0004) -eq 0){throw 'CALLOUT_EDIT_IS_NOT_MULTILINE.'}
    if($Text.Length -gt 0){[ShotKitCalloutSmoke.N]::SetText($e.H,$Text)}
    $actual=[ShotKitCalloutSmoke.N]::EditText($e.H)
    if($actual -cne $Text){throw "CALLOUT_TEXT_MISMATCH expected=<$Text> actual=<$actual>."}
    Click-Id $d.H 1
    [void](Wait-For {-not @(Owned-Windows|Where-Object {$_.ClassName -eq 'ShotKitRustTextEntryDialog'}).Count} 4 'callout dialog close')
}
function Save-Hash([string]$PreviousHash,[string]$ExpectedHash) {
    Click-Id $main.H 0x5110
    Wait-For {
        try {
            if((Test-Path -LiteralPath $inputPath) -and (Get-Item -LiteralPath $inputPath).Length -gt 50){
                $hash=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash
                if((-not [string]::IsNullOrEmpty($ExpectedHash) -and $hash -ceq $ExpectedHash) -or ([string]::IsNullOrEmpty($ExpectedHash) -and $hash -cne $PreviousHash)){return $hash}
            }
        }catch{}
    } 15 'expected callout export hash'
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData -Force|Out-Null
    $bmp=[Drawing.Bitmap]::new(320,240,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try{$g=[Drawing.Graphics]::FromImage($bmp);try{$g.Clear([Drawing.Color]::Transparent)}finally{$g.Dispose()};$bmp.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png)}finally{$bmp.Dispose()}
    $initialHash=(Get-FileHash -LiteralPath $inputPath -Algorithm SHA256).Hash
    try{$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru}
    finally{$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    $main=Wait-For {Owned-Windows|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'}|Select-Object -First 1} $StartupTimeoutSeconds 'editor window'
    [void][ShotKitCalloutSmoke.N]::ShowWindow($main.H,3);[void][ShotKitCalloutSmoke.N]::SetForegroundWindow($main.H)
    $welcome=Wait-For {Owned-Windows|Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'}|Select-Object -First 1} 12 'startup notice'
    $ok=@([ShotKitCalloutSmoke.N]::Children($welcome.H)|Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and $_.Title -eq 'OK'}|Select-Object -First 1)
    if($ok.Count -ne 1){throw 'STARTUP_NOTICE_OK_MISSING.'};[ShotKitCalloutSmoke.N]::Click($ok[0].H)
    $main=Wait-For {Owned-Windows|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*'}|Select-Object -First 1} 10 'loaded editor'
    $calloutButton=Child-ById $main.H 0x510F
    if($calloutButton.ClassName -ne 'Button'){throw "CALLOUT_TOOL_CONTROL_NOT_BUTTON: $($calloutButton.ClassName)."}
    [ShotKitCalloutSmoke.N]::Click($calloutButton.H)
    [void](Wait-For {Owned-Windows|Where-Object {$_.H -eq $main.H -and $_.Title -like '*Werkzeug Callout*'}|Select-Object -First 1} 5 'Callout tool selected')
    $original="Sprechblase Zeile eins`r`nZeile zwei"
    Canvas-Click 48 42
    Confirm-Callout $original
    $createdHash=Save-Hash $initialHash $null
    if($createdHash -ceq $initialHash){throw 'CALLOUT_CREATION_NOT_SAVED: export still matches the blank fixture.'}

    Click-Id $main.H 0x5101
    [void](Wait-For {Owned-Windows|Where-Object {$_.H -eq $main.H -and $_.Title -like '*Werkzeug Select*'}|Select-Object -First 1} 5 'Select tool for callout double-click edit')
    Canvas-Click 75 65 -Double
    $d=Text-Dialog 'double-click edit dialog'
    $e=Edit-Control $d
    if([ShotKitCalloutSmoke.N]::Owner($d.H) -ne $proc.Id){throw 'DOUBLE_CLICK_DIALOG_OWNER_MISMATCH.'}
    $expectedInitial=$original
    $initial=[ShotKitCalloutSmoke.N]::EditText($e.H)
    if($initial -cne $expectedInitial){throw "DOUBLE_CLICK_DID_NOT_REOPEN_CALLOUT: expected=<$expectedInitial> actual=<$initial>."}
    $changed="Sprechblase bearbeitet`r`nzweite Zeile geändert"
    Confirm-Callout $changed
    $editedHash=Save-Hash $createdHash $null
    if($editedHash -ceq $createdHash){throw 'EDITED_CALLOUT_EXPORT_UNCHANGED.'}
    Click-Id $main.H 0x5112
    $undoHash=Save-Hash $editedHash $createdHash
    if($undoHash -cne $createdHash){throw 'CALLOUT_UNDO_DID_NOT_RESTORE_CREATED_STATE.'}
    Click-Id $main.H 0x5113
    $redoHash=Save-Hash $createdHash $editedHash
    if($redoHash -cne $editedHash){throw 'CALLOUT_REDO_DID_NOT_RESTORE_EDITED_STATE.'}
    Write-Output 'PASS: Callout toolbar ID 0x510F selected; native multiline text was created and saved; canvas double-click reopened the exact text; edited export changed; Undo restored the created export and Redo restored the edited export.'
}
catch{$failure=$_}
finally{
    if($mouseDown){try{[ShotKitCalloutSmoke.N]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($null -ne $proc -and -not $proc.HasExited){try{$proc.Kill();[void]$proc.WaitForExit(5000)}catch{}}
    if($null -ne $proc){$proc.Dispose()}
    $env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData
    if($null -eq $proc -or $proc.HasExited){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}

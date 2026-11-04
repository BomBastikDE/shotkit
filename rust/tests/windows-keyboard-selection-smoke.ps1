[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,
    [ValidateRange(5, 120)]
    [int] $StartupTimeoutSeconds = 20,
    [ValidateRange(5, 180)]
    [int] $CaptureTimeoutSeconds = 45
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
if ($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') { throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner.' }
if (-not [Environment]::UserInteractive) { throw 'Desktop session unavailable: this Windows runner is not interactive.' }

if (-not ('ShotKitKeyboardSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitKeyboardSmoke {
    public sealed class WindowInfo { public IntPtr Handle; public string ClassName; public string Title; public bool Visible; }
    public static class Native {
        private delegate bool EnumWindowsProc(IntPtr hwnd, IntPtr data);
        private delegate bool EnumChildProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
        [DllImport("user32.dll", SetLastError=true)] static extern bool CloseDesktop(IntPtr desktop);
        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool GetUserObjectInformationW(IntPtr handle, int index, StringBuilder text, uint length, out uint needed);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumWindowsProc callback, IntPtr data);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessageW(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
        [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint message, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr result);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
        public static string InputDesktopName() {
            IntPtr d=OpenInputDesktop(0,false,1); if(d==IntPtr.Zero)return null;
            try { var s=new StringBuilder(256); uint n; return GetUserObjectInformationW(d,2,s,(uint)(s.Capacity*2),out n)?s.ToString():null; }
            finally { CloseDesktop(d); }
        }
        static string Text(IntPtr h) { var s=new StringBuilder(512); GetWindowTextW(h,s,s.Capacity); return s.ToString(); }
        public static string WindowText(IntPtr h) {
            var s=new StringBuilder(Text(h)); EnumChildWindows(h,delegate(IntPtr c,IntPtr d){string t=Text(c);if(!String.IsNullOrWhiteSpace(t))s.Append(" ").Append(t);return true;},IntPtr.Zero);return s.ToString();
        }
        public static WindowInfo[] WindowsForProcess(int id) {
            var list=new List<WindowInfo>(); EnumWindows(delegate(IntPtr h,IntPtr d){uint p;GetWindowThreadProcessId(h,out p);if(p!=(uint)id)return true;
                var c=new StringBuilder(256);GetClassNameW(h,c,c.Capacity);list.Add(new WindowInfo{Handle=h,ClassName=c.ToString(),Title=Text(h),Visible=IsWindowVisible(h)});return true;},IntPtr.Zero);return list.ToArray();
        }
    }
}
'@
}

$inputDesktop = [ShotKitKeyboardSmoke.Native]::InputDesktopName()
if ([string]::IsNullOrWhiteSpace($inputDesktop)) { throw 'Could not open the interactive input desktop.' }
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Keyboard-Selection-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $testRoot 'appdata'; $localAppData = Join-Path $testRoot 'localappdata'; $saveFolder = Join-Path $testRoot 'captures'
$surfaceScript = Join-Path $testRoot 'surface.ps1'; $readyFile = Join-Path $testRoot 'surface-ready.txt'
$surfaceStdout = Join-Path $testRoot 'surface.stdout.log'; $surfaceStderr = Join-Path $testRoot 'surface.stderr.log'
$shotKit = $null; $surface = $null; $shiftDown = $false
$oldAppData = $env:APPDATA; $oldLocalAppData = $env:LOCALAPPDATA

function Get-OwnWindows([System.Diagnostics.Process] $p) { [ShotKitKeyboardSmoke.Native]::WindowsForProcess($p.Id) }
function Wait-ForCondition([scriptblock] $Condition, [int] $TimeoutSeconds, [string] $Description, [System.Diagnostics.Process] $OwnedProcess) {
    $deadline=[DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while([DateTime]::UtcNow -lt $deadline) {
        if($OwnedProcess.HasExited){throw "ShotKit exited before $Description (exit code $($OwnedProcess.ExitCode))."}
        $dialog=Get-OwnWindows $OwnedProcess | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -ne 'ShotKit Rust – Prototyp'} | Select-Object -First 1
        if($null -ne $dialog){throw "Unexpected ShotKit dialog: $($dialog.Title) $([ShotKitKeyboardSmoke.Native]::WindowText($dialog.Handle))"}
        $v=& $Condition; if($null -ne $v -and $v){return $v}; Start-Sleep -Milliseconds 100
    }
    throw "Timed out waiting for $Description."
}
function Send-Key([byte] $Key, [bool] $Down) {
    $flags=if($Down){0}else{2}; [ShotKitKeyboardSmoke.Native]::keybd_event($Key,0,$flags,[UIntPtr]::Zero)
}

try {
    New-Item -ItemType Directory -Path $appData,$localAppData,$saveFolder -Force | Out-Null
    $surfaceSource = @'
param([string]$ReadyFile)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies 'System.Windows.Forms.dll','System.Drawing.dll' -TypeDefinition @"
using System;
using System.Drawing;
using System.Windows.Forms;
using System.IO;
using System.Runtime.InteropServices;
namespace ShotKitKeyboardSurface {
 public sealed class Surface : Form {
  Bitmap image; string ready;
  [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  public Surface(string path) {
   try { SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch {}
   ready=path; Text="ShotKit Keyboard Selection Fixture"; StartPosition=FormStartPosition.Manual; Location=new Point(20,20);
   ClientSize=new Size(900,650); FormBorderStyle=FormBorderStyle.FixedSingle; MaximizeBox=false; MinimizeBox=false; ShowInTaskbar=false; DoubleBuffered=true; TopMost=true;
   image=new Bitmap(900,650);
   for(int y=0;y<650;y++) for(int x=0;x<900;x++) image.SetPixel(x,y,Color.FromArgb((x*29+y*17+(y/19)*53)&255,(x*7+y*31+(y/23)*97)&255,(x*19+y*11+(y/11)*41)&255));
  }
  protected override void OnShown(EventArgs e){base.OnShown(e);Point p=PointToScreen(Point.Empty);File.WriteAllText(ready,String.Format("{0},{1},{2},{3}",p.X,p.Y,ClientSize.Width,ClientSize.Height));}
  protected override void OnPaint(PaintEventArgs e){e.Graphics.DrawImageUnscaled(image,0,0);}
  protected override void Dispose(bool d){if(d&&image!=null)image.Dispose();base.Dispose(d);}
 }
}
"@
[System.Windows.Forms.Application]::Run([ShotKitKeyboardSurface.Surface]::new($ReadyFile))
'@
    [IO.File]::WriteAllText($surfaceScript,$surfaceSource,[Text.UTF8Encoding]::new($false))
    $powershellExe=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if(-not(Test-Path -LiteralPath $powershellExe -PathType Leaf)){throw 'Windows PowerShell 5.1 is required for the isolated WinForms fixture.'}
    $surfaceArgs='-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -ReadyFile "{1}"' -f $surfaceScript,$readyFile
    $surface=Start-Process -FilePath $powershellExe -ArgumentList $surfaceArgs -WorkingDirectory $testRoot -WindowStyle Hidden -RedirectStandardOutput $surfaceStdout -RedirectStandardError $surfaceStderr -PassThru
    Write-Output "MILESTONE: started keyboard fixture host PID=$($surface.Id); waiting for its topmost WinForms surface."
    try {
        $surfaceWindow=Wait-ForCondition -TimeoutSeconds 30 -Description 'the deterministic color fixture' -OwnedProcess $surface -Condition {
            [ShotKitKeyboardSmoke.Native]::WindowsForProcess($surface.Id) | Where-Object {$_.Title -eq 'ShotKit Keyboard Selection Fixture' -and $_.Visible} | Select-Object -First 1
        }
    }
    catch {
        $ownedWindows=@([ShotKitKeyboardSmoke.Native]::WindowsForProcess($surface.Id) | ForEach-Object {"HWND=0x$($_.Handle.ToInt64().ToString('X')) Class='$($_.ClassName)' Title='$($_.Title)' Visible=$($_.Visible)"}) -join '; '
        if(-not $ownedWindows){$ownedWindows='no enumerated top-level windows'}
        $stdout=if(Test-Path -LiteralPath $surfaceStdout){Get-Content -LiteralPath $surfaceStdout -Raw}else{''}
        $stderr=if(Test-Path -LiteralPath $surfaceStderr){Get-Content -LiteralPath $surfaceStderr -Raw}else{''}
        throw "Fixture startup failed. PID=$($surface.Id) Exited=$($surface.HasExited) ExitCode=$(if($surface.HasExited){$surface.ExitCode}else{'running'}); windows=[$ownedWindows]; stdout=[$stdout]; stderr=[$stderr]. Cause: $($_.Exception.Message)"
    }
    $ready=Wait-ForCondition -TimeoutSeconds 10 -Description 'fixture client coordinates' -OwnedProcess $surface -Condition {if(Test-Path -LiteralPath $readyFile){Get-Content -LiteralPath $readyFile -Raw}}
    $geometry=$ready.Trim().Split(',') | ForEach-Object {[int]$_}
    if($geometry.Count -ne 4 -or $geometry[2] -ne 900 -or $geometry[3] -ne 650){throw "Unexpected fixture geometry '$ready'."}

    $settingsPath=Join-Path (Join-Path $appData 'ShotKitRust') 'settings.json'
    New-Item -ItemType Directory -Path (Split-Path -Parent $settingsPath) -Force | Out-Null
    $settings=@{version=1;save_folder=$saveFolder;open_editor=$true;copy_to_clipboard=$false;auto_save=$true;scroll_direction='down'} | ConvertTo-Json
    [IO.File]::WriteAllText($settingsPath,$settings,[Text.UTF8Encoding]::new($false))
    try {$env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData;$shotKit=Start-Process -FilePath $ExePath -WorkingDirectory $testRoot -PassThru}
    finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    [void](Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'ShotKit main window' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustPrototype'} | Select-Object -First 1})
    $welcome=Wait-ForCondition -TimeoutSeconds $StartupTimeoutSeconds -Description 'ShotKit startup dialog' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1}
    if (-not [ShotKitKeyboardSmoke.Native]::PostMessageW($welcome.Handle,0x0111,[IntPtr]1,[IntPtr]::Zero)){throw 'Could not close the owned startup dialog.'}
    Start-Sleep -Milliseconds 250
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $overlay=Wait-ForCondition -TimeoutSeconds 10 -Description 'the area-selection overlay' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}

    # Losing mouse capture while a drag is active must cancel without output.
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($overlay.Handle)
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos(($geometry[0]+30),($geometry[1]+30)); Start-Sleep -Milliseconds 100
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero)
    try {
        [void][ShotKitKeyboardSmoke.Native]::SetCursorPos(($geometry[0]+130),($geometry[1]+110)); Start-Sleep -Milliseconds 150
        if(-not [ShotKitKeyboardSmoke.Native]::PostMessageW($overlay.Handle,0x001f,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'Could not post WM_CANCELMODE to the selection overlay.'}
        [void](Wait-ForCondition -TimeoutSeconds 10 -Description 'capture loss to cancel the selection overlay' -OwnedProcess $shotKit -Condition {
            if(-not (Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible})){ $true }
        })
    }
    finally { [ShotKitKeyboardSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero) }
    if(@(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File).Count -ne 0){throw 'Capture-loss cancellation unexpectedly produced a capture.'}
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $overlay=Wait-ForCondition -TimeoutSeconds 10 -Description 'a fresh area-selection overlay after capture loss' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}

    # A synchronously sent WM_CANCELMODE can be dispatched while GetMessageW
    # is blocked. Its completion must wake the overlay loop even as the HWND
    # is destroyed by the callback.
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($overlay.Handle)
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos(($geometry[0]+30),($geometry[1]+30)); Start-Sleep -Milliseconds 100
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero)
    try {
        [void][ShotKitKeyboardSmoke.Native]::SetCursorPos(($geometry[0]+130),($geometry[1]+110)); Start-Sleep -Milliseconds 150
        $sendResult=[IntPtr]::Zero
        if([ShotKitKeyboardSmoke.Native]::SendMessageTimeoutW($overlay.Handle,0x001f,[IntPtr]::Zero,[IntPtr]::Zero,2,5000,[ref]$sendResult) -eq [IntPtr]::Zero){throw 'Could not synchronously send WM_CANCELMODE to the selection overlay.'}
        [void](Wait-ForCondition -TimeoutSeconds 10 -Description 'synchronously sent WM_CANCELMODE to cancel the selection overlay' -OwnedProcess $shotKit -Condition {
            if(-not (Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible})){ $true }
        })
    }
    finally { [ShotKitKeyboardSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero) }
    if(@(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File).Count -ne 0){throw 'Synchronous capture-loss cancellation unexpectedly produced a capture.'}
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $overlay=Wait-ForCondition -TimeoutSeconds 10 -Description 'a fresh area-selection overlay after synchronous capture loss' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}

    # First prove Escape abandons a Shift-frozen manual selection without
    # producing output, then request a fresh overlay for the confirmed case.
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($overlay.Handle)
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos(($geometry[0]+30),($geometry[1]+30)); Start-Sleep -Milliseconds 100
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero)
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos(($geometry[0]+130),($geometry[1]+110)); Start-Sleep -Milliseconds 150
    Send-Key 0x10 $true; $shiftDown=$true
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 100
    Send-Key 0x10 $false; $shiftDown=$false
    Send-Key 0x1B $true; Start-Sleep -Milliseconds 80; Send-Key 0x1B $false
    $closed=Wait-ForCondition -TimeoutSeconds 10 -Description 'Escape to close the selection overlay' -OwnedProcess $shotKit -Condition {
        if(-not (Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible})){ $true }
    }
    if(@(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File).Count -ne 0){throw 'Escape cancellation unexpectedly produced a capture.'}
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $overlay=Wait-ForCondition -TimeoutSeconds 10 -Description 'a fresh area-selection overlay after Escape' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}

    $sendResult=[IntPtr]::Zero
    if([ShotKitKeyboardSmoke.Native]::SendMessageTimeoutW($overlay.Handle,0x0100,[IntPtr]0x1B,[IntPtr]::Zero,2,5000,[ref]$sendResult) -eq [IntPtr]::Zero){throw 'Could not synchronously send Escape to the selection overlay.'}
    [void](Wait-ForCondition -TimeoutSeconds 10 -Description 'synchronously sent Escape to close the selection overlay' -OwnedProcess $shotKit -Condition {
        if(-not (Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible})){ $true }
    })
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $overlay=Wait-ForCondition -TimeoutSeconds 10 -Description 'a fresh area-selection overlay after synchronous Escape' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}

    # Legacy parity: a real right-click aborts the overlay and produces no file.
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($overlay.Handle)
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos(($geometry[0]+30),($geometry[1]+30)); Start-Sleep -Milliseconds 100
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0008,0,0,0,[UIntPtr]::Zero)
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0010,0,0,0,[UIntPtr]::Zero)
    [void](Wait-ForCondition -TimeoutSeconds 10 -Description 'right-click to close the selection overlay' -OwnedProcess $shotKit -Condition {
        if(-not (Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible})){ $true }
    })
    if(@(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File).Count -ne 0){throw 'Right-click cancellation unexpectedly produced a capture.'}
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $overlay=Wait-ForCondition -TimeoutSeconds 10 -Description 'a fresh area-selection overlay after right-click' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}

    # Drag a known 200x150 rectangle over the synthetic surface. Shift remains
    # physically held across mouse-up; the overlay must stay alive for edits.
    $x1=$geometry[0]+80; $y1=$geometry[1]+60; $x2=$x1+200; $y2=$y1+150
    if (-not [ShotKitKeyboardSmoke.Native]::SetForegroundWindow($overlay.Handle)){throw 'Could not activate the selection overlay.'}
    if (-not [ShotKitKeyboardSmoke.Native]::SetCursorPos($x1,$y1)){throw 'Could not position at drag start.'}
    Start-Sleep -Milliseconds 120
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero)
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos($x2,$y2); Start-Sleep -Milliseconds 180
    Send-Key 0x10 $true; $shiftDown=$true
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 250
    $stillOpen=Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1
    if($null -eq $stillOpen){throw 'Selection overlay closed on mouse-up while Shift was held.'}
    if(@(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File).Count -ne 0){throw 'Capture was saved before keyboard confirmation.'}
    Send-Key 0x10 $false; $shiftDown=$false; Start-Sleep -Milliseconds 80

    # Two Right and one Down translate the rectangle by exactly (2,1). Shift
    # plus Right/Down expands only its right/bottom edges by one pixel each.
    foreach($key in @(0x27,0x27,0x28)) {Send-Key ([byte]$key) $true; Start-Sleep -Milliseconds 60; Send-Key ([byte]$key) $false; Start-Sleep -Milliseconds 80}
    Send-Key 0x10 $true; $shiftDown=$true
    foreach($key in @(0x27,0x28)) {Send-Key ([byte]$key) $true; Start-Sleep -Milliseconds 60; Send-Key ([byte]$key) $false; Start-Sleep -Milliseconds 80}
    Send-Key 0x10 $false; $shiftDown=$false; Start-Sleep -Milliseconds 80
    Send-Key 0x0D $true; Start-Sleep -Milliseconds 80; Send-Key 0x0D $false

    $png=Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'the confirmed auto-saved PNG' -OwnedProcess $shotKit -Condition {Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Select-Object -First 1}
    Add-Type -AssemblyName System.Drawing
    $bitmap=[Drawing.Bitmap]::new($png.FullName)
    try {
        $expectedWidth=201; $expectedHeight=151
        if($bitmap.Width -ne $expectedWidth -or $bitmap.Height -ne $expectedHeight){throw "Captured PNG is $($bitmap.Width)x$($bitmap.Height); expected ${expectedWidth}x${expectedHeight} after translation and edge expansion."}
        foreach($point in @(@(0,0),@(73,41),@(120,90),@(200,150))) {
            $x=$point[0]; $y=$point[1]; $sx=$x+82; $sy=$y+61
            $er=($sx*29+$sy*17+[int][Math]::Floor($sy/19)*53)%256
            $eg=($sx*7+$sy*31+[int][Math]::Floor($sy/23)*97)%256
            $eb=($sx*19+$sy*11+[int][Math]::Floor($sy/11)*41)%256
            $actual=$bitmap.GetPixel($x,$y)
            if($actual.R -ne $er -or $actual.G -ne $eg -or $actual.B -ne $eb){throw "Output pixel ($x,$y), source ($sx,$sy) was RGB=$($actual.R),$($actual.G),$($actual.B); expected RGB=$er,$eg,$eb."}
        }
    }
    finally {$bitmap.Dispose()}
    $editor=Wait-ForCondition -TimeoutSeconds 20 -Description 'the editor opened for the confirmed capture' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitRustEditor' -and $_.Visible} | Select-Object -First 1}

    # A plain drag without Shift must still finish immediately with the
    # original dimensions and source pixels.
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $plainOverlay=Wait-ForCondition -TimeoutSeconds 10 -Description 'the overlay for the ordinary mouse-drag regression case' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($plainOverlay.Handle)
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos(($geometry[0]+80),($geometry[1]+60)); Start-Sleep -Milliseconds 100
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero)
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos(($geometry[0]+280),($geometry[1]+210)); Start-Sleep -Milliseconds 150
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero)
    $allPngs=Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'both keyboard-corrected and ordinary mouse captures' -OwnedProcess $shotKit -Condition {
        $files=@(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File)
        if($files.Count -ge 2){$files}
    }
    $plainPng=@($allPngs | Where-Object {$_.FullName -ne $png.FullName}) | Select-Object -First 1
    if($null -eq $plainPng){throw 'The ordinary mouse drag did not produce a separate PNG.'}
    $plainBitmap=[Drawing.Bitmap]::new($plainPng.FullName)
    try {
        if($plainBitmap.Width -ne 200 -or $plainBitmap.Height -ne 150){throw "Ordinary mouse drag produced $($plainBitmap.Width)x$($plainBitmap.Height); expected 200x150."}
        foreach($point in @(@(0,0),@(73,41),@(199,149))) {
            $x=$point[0]; $y=$point[1]; $sx=$x+80; $sy=$y+60
            $er=($sx*29+$sy*17+[int][Math]::Floor($sy/19)*53)%256
            $eg=($sx*7+$sy*31+[int][Math]::Floor($sy/23)*97)%256
            $eb=($sx*19+$sy*11+[int][Math]::Floor($sy/11)*41)%256
            $actual=$plainBitmap.GetPixel($x,$y)
            if($actual.R -ne $er -or $actual.G -ne $eg -or $actual.B -ne $eb){throw "Ordinary mouse output pixel ($x,$y) was RGB=$($actual.R),$($actual.G),$($actual.B); expected RGB=$er,$eg,$eb."}
        }
    }
    finally {$plainBitmap.Dispose()}

    # A reverse manual drag ends inside its rectangle. Resize it with the
    # keyboard, then click at the unchanged pointer position. The click must
    # accept the edited rectangle, not the stale hover outline from before
    # the drag. A click also exercises the existing fresh-root confirmation.
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($surfaceWindow.Handle)
    Send-Key 0x7A $true; Send-Key 0x7A $false
    $clickOverlay=Wait-ForCondition -TimeoutSeconds 10 -Description 'the overlay for the keyboard-edit click case' -OwnedProcess $shotKit -Condition {Get-OwnWindows $shotKit | Where-Object {$_.ClassName -eq 'ShotKitSelectionOverlay' -and $_.Visible} | Select-Object -First 1}
    [void][ShotKitKeyboardSmoke.Native]::SetForegroundWindow($clickOverlay.Handle)
    $clickX=$geometry[0]+80; $clickY=$geometry[1]+60
    $dragX=$geometry[0]+280; $dragY=$geometry[1]+210
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos($dragX,$dragY); Start-Sleep -Milliseconds 100
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero)
    [void][ShotKitKeyboardSmoke.Native]::SetCursorPos($clickX,$clickY); Start-Sleep -Milliseconds 150
    Send-Key 0x10 $true; $shiftDown=$true
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 180
    Send-Key 0x10 $false; $shiftDown=$false
    Send-Key 0x10 $true; $shiftDown=$true
    Send-Key 0x27 $true; Start-Sleep -Milliseconds 60; Send-Key 0x27 $false
    Send-Key 0x10 $false; $shiftDown=$false; Start-Sleep -Milliseconds 100
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0002,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [ShotKitKeyboardSmoke.Native]::mouse_event(0x0004,0,0,0,[UIntPtr]::Zero)
    $clickPng=Wait-ForCondition -TimeoutSeconds $CaptureTimeoutSeconds -Description 'the capture accepted by clicking the keyboard-edited rectangle' -OwnedProcess $shotKit -Condition {
        $files=@(Get-ChildItem -LiteralPath $saveFolder -Filter '*.png' -File | Where-Object {$_.FullName -ne $png.FullName -and $_.FullName -ne $plainPng.FullName})
        if($files.Count -ge 1){$files | Select-Object -First 1}
    }
    $clickBitmap=[Drawing.Bitmap]::new($clickPng.FullName)
    try {
        if($clickBitmap.Width -ne 201 -or $clickBitmap.Height -ne 150){throw "Click after keyboard resize produced $($clickBitmap.Width)x$($clickBitmap.Height); expected the edited 201x150 rectangle."}
        foreach($point in @(@(0,0),@(73,41),@(200,149))) {
            $x=$point[0]; $y=$point[1]; $sx=$x+80; $sy=$y+60
            $er=($sx*29+$sy*17+[int][Math]::Floor($sy/19)*53)%256
            $eg=($sx*7+$sy*31+[int][Math]::Floor($sy/23)*97)%256
            $eb=($sx*19+$sy*11+[int][Math]::Floor($sy/11)*41)%256
            $actual=$clickBitmap.GetPixel($x,$y)
            if($actual.R -ne $er -or $actual.G -ne $eg -or $actual.B -ne $eb){throw "Click-confirmed output pixel ($x,$y) was RGB=$($actual.R),$($actual.G),$($actual.B); expected RGB=$er,$eg,$eb."}
        }
    }
    finally {$clickBitmap.Dispose()}
    Write-Output "PASS: Capture loss, Escape, and right-click cancelled without output; Shift-drag stayed open; arrows translated and Shift+arrows resized to ${expectedWidth}x${expectedHeight} with verified pixels; Enter confirmed. Plain mouse drag remained immediate at 200x150 with verified pixels on '$inputDesktop'."
}
finally {
    if($shiftDown){Send-Key 0x10 $false}
    foreach($owned in @($shotKit,$surface)) {
        if($null -ne $owned) {
            try {if (-not $owned.HasExited){$owned.Kill();if (-not $owned.WaitForExit(5000) -or -not $owned.HasExited){throw 'Owned process did not exit after termination request.'}}}
            catch {Write-Warning "Could not stop owned process: $($_.Exception.Message)"}
            $owned.Dispose()
        }
    }
    $env:APPDATA=$oldAppData; $env:LOCALAPPDATA=$oldLocalAppData
    if(Test-Path -LiteralPath $testRoot){Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue}
}

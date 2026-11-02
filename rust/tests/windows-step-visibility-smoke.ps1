[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $StartupTimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'This smoke test is restricted to an interactive ephemeral GitHub Actions Windows runner.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: refusing to interact with '$imageName'."
}

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitStepSmoke {
    public struct Rect { public int Left, Top, Right, Bottom; }
    public struct Point { public int X, Y; }
    public sealed class Window { public IntPtr Handle; public string ClassName, Title; public bool Visible; }
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
        [DllImport("user32.dll")] static extern bool ScreenToClient(IntPtr h, ref Point p);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
        [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
        [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int command);
        [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint data, UIntPtr extra);
        [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
        [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
        [DllImport("user32.dll", SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, IntPtr w, IntPtr l, uint flags, uint timeout, out UIntPtr result);
        [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr owner);
        [DllImport("user32.dll")] static extern bool CloseClipboard();
        [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint format);
        [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr memory);
        [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr memory);
        [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr memory);
        [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
        static string Read(IntPtr h, bool cls) { var s=new StringBuilder(512); if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString(); }
        public static string ClassName(IntPtr h) { return Read(h,true); }
        public static string Title(IntPtr h) { return Read(h,false); }
        public static uint ClipboardSequence() { return GetClipboardSequenceNumber(); }
        public static uint Owner(IntPtr h) { uint p; GetWindowThreadProcessId(h,out p); return p; }
        public static Window[] Windows(int wanted) {
            var a=new List<Window>(); EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)wanted)a.Add(new Window{Handle=h,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};
            EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();
        }
        public static Child[] Children(IntPtr parent) {
            var a=new List<Child>();EnumProc cb=(h,d)=>{a.Add(new Child{Handle=h,ClassName=Read(h,true),Text=Read(h,false),Visible=IsWindowVisible(h)});return true;};
            EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();
        }
        public static bool Client(IntPtr h,out Rect r) { return GetClientRect(h,out r); }
        public static bool ChildRect(IntPtr parent,IntPtr child,out Rect r) {
            r=new Rect();if(!GetWindowRect(child,out r))return false;
            Point a=new Point{X=r.Left,Y=r.Top},b=new Point{X=r.Right,Y=r.Bottom};
            if(!ScreenToClient(parent,ref a)||!ScreenToClient(parent,ref b))return false;
            r=new Rect{Left=a.X,Top=a.Y,Right=b.X,Bottom=b.Y};return true;
        }
        public static void Click(IntPtr h) { UIntPtr result; if(SendMessageTimeoutW(h,0x00F5,IntPtr.Zero,IntPtr.Zero,2,5000,out result)==IntPtr.Zero) throw new InvalidOperationException("Button click timed out or failed; an owned dialog may require attention."); }
        public static void CloseNotice(IntPtr h) { UIntPtr result; if(SendMessageTimeoutW(h,0x0010,IntPtr.Zero,IntPtr.Zero,2,5000,out result)==IntPtr.Zero) throw new InvalidOperationException("Startup notice close timed out."); }
        public static byte[] ReadDibV5() {
            if(!OpenClipboard(IntPtr.Zero))throw new InvalidOperationException("OpenClipboard failed after Ctrl+C.");
            IntPtr memory=IntPtr.Zero,p=IntPtr.Zero;
            try {
                memory=GetClipboardData(17);if(memory==IntPtr.Zero)throw new InvalidOperationException("CF_DIBV5 missing after Ctrl+C.");
                ulong size=GlobalSize(memory).ToUInt64();if(size<124||size>67108864)throw new InvalidOperationException("CF_DIBV5 size is invalid.");
                p=GlobalLock(memory);if(p==IntPtr.Zero)throw new InvalidOperationException("GlobalLock failed for CF_DIBV5.");
                byte[] b=new byte[(int)size];Marshal.Copy(p,b,0,b.Length);return b;
            } finally { if(p!=IntPtr.Zero)GlobalUnlock(memory);if(!CloseClipboard())throw new InvalidOperationException("CloseClipboard failed."); }
        }
    }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Step-Visibility-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'
$inputPath = Join-Path $root 'step-fixture.png'; $referencePath = Join-Path $root 'step-reference.png'; $process = $null; $failure = $null; $mouseDown = $false
$oldAppData = $env:APPDATA; $oldLocalAppData = $env:LOCALAPPDATA
$centers = @(
    @{X=72;Y=70}, @{X=232;Y=70}, @{X=392;Y=70}, @{X=552;Y=70},
    @{X=72;Y=230}, @{X=232;Y=230}, @{X=392;Y=230}, @{X=552;Y=230},
    @{X=72;Y=390}, @{X=232;Y=390}, @{X=392;Y=390}, @{X=552;Y=390}
)
$radius = 10.0 + 4.0 * 2.2

function Get-OwnedWindows {
    if ($null -eq $process -or $process.HasExited) { return @() }
    @([ShotKitStepSmoke.Native]::Windows($process.Id))
}
function Wait-For([scriptblock]$Probe,[int]$Seconds,[string]$Description) {
    $deadline=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $deadline) {
        if($null -ne $process -and $process.HasExited){throw "PROCESS_EXITED waiting for $Description (code $($process.ExitCode))."}
        $value=&$Probe;if($null -ne $value -and $value){return $value};Start-Sleep -Milliseconds 100
    }
    throw "TIMEOUT waiting for $Description."
}
function Get-Button([IntPtr]$Parent,[string]$Text) {
    $button=[ShotKitStepSmoke.Native]::Children($Parent) | Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and (($_.Text -replace '&','').Trim() -eq $Text)} | Select-Object -First 1
    if($null -eq $button){throw "BUTTON_NOT_FOUND: '$Text'."};return $button
}
function Click-Button([IntPtr]$Parent,[string]$Text) { $b=Get-Button $Parent $Text;[ShotKitStepSmoke.Native]::Click($b.Handle);Start-Sleep -Milliseconds 120 }
function Get-ViewPoint([int]$ImageX,[int]$ImageY) {
    $origin=[Activator]::CreateInstance([ShotKitStepSmoke.Point]);if(-not [ShotKitStepSmoke.Native]::ClientToScreen($main.Handle,[ref]$origin)){throw 'ClientToScreen failed.'}
    $sx=$origin.X+$viewX+[int][Math]::Round(($ImageX+0.5)*$shownW/640.0,[MidpointRounding]::AwayFromZero)
    $sy=$origin.Y+$viewY+[int][Math]::Round(($ImageY+0.5)*$shownH/480.0,[MidpointRounding]::AwayFromZero)
    $p=[Activator]::CreateInstance([ShotKitStepSmoke.Point]);$p.X=$sx;$p.Y=$sy;return $p
}
function Click-Canvas([int]$ImageX,[int]$ImageY) {
    if(-not [ShotKitStepSmoke.Native]::SetForegroundWindow($main.Handle)){throw 'Could not foreground owned editor.'}
    $p=Get-ViewPoint $ImageX $ImageY
    if(-not [ShotKitStepSmoke.Native]::SetCursorPos($p.X,$p.Y)){throw 'SetCursorPos failed.'}
    Start-Sleep -Milliseconds 100
    $target=[ShotKitStepSmoke.Native]::WindowFromPoint($p)
    if([ShotKitStepSmoke.Native]::Owner($target) -ne [uint32]$process.Id){throw "INPUT_TARGET_MISMATCH at image ($ImageX,$ImageY)."}
    [ShotKitStepSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true
    Start-Sleep -Milliseconds 60
    [ShotKitStepSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
    Start-Sleep -Milliseconds 100
}
function Read-Png([string]$Path) {
    $b=[Drawing.Bitmap]::new($Path)
    if($b.Width -ne 640 -or $b.Height -ne 480){$b.Dispose();throw "PNG_DIMENSIONS: expected 640x480, got $($b.Width)x$($b.Height)."}
    return $b
}
function Read-ClipboardBitmap {
    $dib=[ShotKitStepSmoke.Native]::ReadDibV5()
    $w=[BitConverter]::ToInt32($dib,4);$h=[BitConverter]::ToInt32($dib,8)
    $bpp=[BitConverter]::ToUInt16($dib,14);$compression=[BitConverter]::ToUInt32($dib,16)
    $redMask=[BitConverter]::ToUInt32($dib,40);$greenMask=[BitConverter]::ToUInt32($dib,44);$blueMask=[BitConverter]::ToUInt32($dib,48);$alphaMask=[BitConverter]::ToUInt32($dib,52)
    if([BitConverter]::ToUInt32($dib,0) -ne 124 -or $w -ne 640 -or [Math]::Abs($h) -ne 480 -or $bpp -ne 32 -or $compression -ne 3 -or
       $redMask -ne 16711680 -or $greenMask -ne 65280 -or $blueMask -ne 255 -or $alphaMask -ne 4278190080L){
        throw "CLIPBOARD_HEADER: expected 640x480 32bpp BI_BITFIELDS with RGBA masks, got ${w}x${h}, $bpp bpp compression=$compression masks=$redMask/$greenMask/$blueMask/$alphaMask."
    }
    $rows=[Math]::Abs($h);$rowBytes=$w*4
    if($dib.Length -lt 124+$rowBytes*$rows){throw 'CLIPBOARD_TRUNCATED: bitmap pixels are incomplete.'}
    $bitmap=[Drawing.Bitmap]::new($w,$rows,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $bounds=[Drawing.Rectangle]::new(0,0,$w,$rows)
        $locked=$bitmap.LockBits($bounds,[Drawing.Imaging.ImageLockMode]::WriteOnly,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            for($y=0;$y -lt $rows;$y++) {
                $sourceY=if($h -gt 0){$rows-1-$y}else{$y}
                [Runtime.InteropServices.Marshal]::Copy($dib,124+$sourceY*$rowBytes,[IntPtr]::Add($locked.Scan0,$y*$locked.Stride),$rowBytes)
            }
        } finally {$bitmap.UnlockBits($locked)}
        return $bitmap
    } catch { $bitmap.Dispose();throw }
}
function New-ExpectedImage([int]$Count) {
    $bitmap=[Drawing.Bitmap]::new($referencePath)
    $g=[Drawing.Graphics]::FromImage($bitmap)
    try {
        $g.SmoothingMode=[Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $g.TextRenderingHint=[Drawing.Text.TextRenderingHint]::AntiAlias
        $fill=[Drawing.SolidBrush]::new([Drawing.Color]::Red)
        $ring=[Drawing.Pen]::new([Drawing.Color]::White,[Math]::Max(2.0,$radius/8.0))
        $format=[Drawing.StringFormat]::new();$format.Alignment=[Drawing.StringAlignment]::Center;$format.LineAlignment=[Drawing.StringAlignment]::Center
        try { for($i=0;$i -lt $Count;$i++) {
            $c=$centers[$i];$x=[float]($c.X-$radius);$y=[float]($c.Y-$radius);$d=[float](2*$radius)
            $g.FillEllipse($fill,$x,$y,$d,$d);$g.DrawEllipse($ring,$x,$y,$d,$d)
            $number=[string]($i+1);$size=[float]($radius * $(if($number.Length -eq 1){1.05}else{[Math]::Min(1.7,1.8/$number.Length)}))
            $font=[Drawing.Font]::new('Segoe UI',$size,[Drawing.FontStyle]::Bold,[Drawing.GraphicsUnit]::Pixel)
            $brush=[Drawing.SolidBrush]::new([Drawing.Color]::White)
            try {$box=[Drawing.RectangleF]::new($x,$y,$d,$d);$g.DrawString($number,$font,$brush,$box,$format)} finally {$brush.Dispose();$font.Dispose()}
        }} finally {$fill.Dispose();$ring.Dispose();$format.Dispose()}
        return $bitmap
    } catch {$bitmap.Dispose();throw} finally {$g.Dispose()}
}
function Assert-StepPixels([Drawing.Bitmap]$Actual,[Drawing.Bitmap]$Expected,[int]$Count,[string]$Context) {
    if($Actual.Width -ne 640 -or $Actual.Height -ne 480){throw "${Context}_DIMENSIONS_INVALID."}
    for($i=0;$i -lt $Count;$i++) {
        $c=$centers[$i];$red=0;$white=0;$matched=0;$compared=0
        for($y=[int]($c.Y-15);$y -le [int]($c.Y+15);$y++) { for($x=[int]($c.X-15);$x -le [int]($c.X+15);$x++) {
            $p=$Actual.GetPixel($x,$y);$e=$Expected.GetPixel($x,$y);$dx=$x-$c.X;$dy=$y-$c.Y;$distance=[Math]::Sqrt($dx*$dx+$dy*$dy)
            if($distance -ge 10 -and $distance -le 15 -and $p.R -gt 180 -and $p.G -lt 100 -and $p.B -lt 100){$red++}
            if($distance -lt 11 -and $p.R -gt 200 -and $p.G -gt 150 -and $p.B -gt 150){$white++}
            if($distance -lt 13){$compared++;if([Math]::Abs($p.R-$e.R) -le 35 -and [Math]::Abs($p.G-$e.G) -le 35 -and [Math]::Abs($p.B-$e.B) -le 35){$matched++}}
        }}
        $background=if($c.X -lt 320){'dark'}else{'light'}
        if($red -lt 25){throw "${Context}_CIRCLE_MISSING: expected number $($i+1) on $background background; red interior/ring pixels=$red."}
        if($white -lt 5){throw "${Context}_GLYPH_MISSING: number $($i+1) on $background background has no visible high-contrast glyph pixels ($white)."}
        if($matched/$compared -lt 0.90){throw "${Context}_GLYPH_MISMATCH: number $($i+1) does not match its centered Segoe UI Bold reference; pixels matched=$matched/$compared."}
    }
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData -Force | Out-Null
    $fixture=[Drawing.Bitmap]::new(640,480,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $g=[Drawing.Graphics]::FromImage($fixture)
        $light=[Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(255,238,240,244))
        try {$g.Clear([Drawing.Color]::FromArgb(255,24,28,36));$g.FillRectangle($light,320,0,320,480)} finally {$light.Dispose();$g.Dispose()}
        $fixture.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png)
        $fixture.Save($referencePath,[Drawing.Imaging.ImageFormat]::Png)
    } finally {$fixture.Dispose()}

    $env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData
    try {$process=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru}
    finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    $main=Wait-For {Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'} | Select-Object -First 1} $StartupTimeoutSeconds 'editor window'
    [void][ShotKitStepSmoke.Native]::ShowWindow($main.Handle,3)
    [void][ShotKitStepSmoke.Native]::SetForegroundWindow($main.Handle)
    $welcome=Wait-For {Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1} 12 'startup notice'
    [void][ShotKitStepSmoke.Native]::SetForegroundWindow($welcome.Handle)
    [void](Get-Button $welcome.Handle 'OK');[ShotKitStepSmoke.Native]::CloseNotice($welcome.Handle)
    [void](Wait-For {if(-not (Get-OwnedWindows | Where-Object {$_.Handle -eq $welcome.Handle -and $_.Visible})) {$true}} 5 'startup notice dismissal')
    $main=Wait-For {Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*'} | Select-Object -First 1} 10 'loaded editor'
    $client=[Activator]::CreateInstance([ShotKitStepSmoke.Rect]);if(-not [ShotKitStepSmoke.Native]::Client($main.Handle,[ref]$client)){throw 'GetClientRect failed.'}
    $toolbar=[ShotKitStepSmoke.Native]::Children($main.Handle) | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'} | Select-Object -First 1
    if($null -eq $toolbar){throw 'EDITOR_NOT_READY: toolbar missing.'}
    $tr=[Activator]::CreateInstance([ShotKitStepSmoke.Rect]);if(-not [ShotKitStepSmoke.Native]::ChildRect($main.Handle,$toolbar.Handle,[ref]$tr)){throw 'Could not read toolbar bounds.'}
    $viewW=$client.Right;$viewH=$client.Bottom-$tr.Bottom;$fit=[Math]::Min($viewW/640.0,$viewH/480.0)
    $shownW=[int][Math]::Round(640*$fit,[MidpointRounding]::AwayFromZero);$shownH=[int][Math]::Round(480*$fit,[MidpointRounding]::AwayFromZero)
    if($shownW -lt 600 -or $shownH -lt 450){throw "EDITOR_CANVAS_TOO_SMALL: ${shownW}x${shownH}; cannot make a reliable 12-step pixel test."}
    $viewX=[int][Math]::Floor(($viewW-$shownW)/2.0);$viewY=$tr.Bottom+[int][Math]::Floor(($viewH-$shownH)/2.0)
    Click-Button $main.Handle 'Nummer (N)'
    for($i=0;$i -lt $centers.Count;$i++){Click-Canvas $centers[$i].X $centers[$i].Y}

    Click-Button $main.Handle 'Speichern'
    $png=Wait-For {try {$b=Read-Png $inputPath;$b.Dispose();$true}catch{$null}} 10 'saved PNG'
    $expected12=New-ExpectedImage 12
    $saved=Read-Png $inputPath
    try {Assert-StepPixels $saved $expected12 12 'PNG'} finally {$saved.Dispose()}

    Click-Button $main.Handle 'Zurück'
    Click-Button $main.Handle 'Speichern'
    $expected11=New-ExpectedImage 11
    $saved=Read-Png $inputPath
    try {Assert-StepPixels $saved $expected11 11 'UNDO_PNG'} finally {$saved.Dispose();$expected11.Dispose()}
    Click-Button $main.Handle 'Wiederholen'
    Click-Button $main.Handle 'Speichern'
    $saved=Read-Png $inputPath
    try {Assert-StepPixels $saved $expected12 12 'REDO_PNG'} finally {$saved.Dispose();$expected12.Dispose()}

    # Select an empty canvas point to give keyboard focus to the editor without
    # adding a thirteenth marker; the following comparison still checks 12.
    Click-Button $main.Handle 'Auswahl (V)'
    Click-Canvas 610 455
    $sequence=[ShotKitStepSmoke.Native]::ClipboardSequence()
    [void][ShotKitStepSmoke.Native]::SetForegroundWindow($main.Handle);Start-Sleep -Milliseconds 100
    if([ShotKitStepSmoke.Native]::GetForegroundWindow() -ne $main.Handle){throw 'EDITOR_FOCUS_FAILED before the Ctrl+C assertion.'}
    [ShotKitStepSmoke.Native]::keybd_event(0x11,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 30
    [ShotKitStepSmoke.Native]::keybd_event(0x43,0,0,[UIntPtr]::Zero);Start-Sleep -Milliseconds 80
    [ShotKitStepSmoke.Native]::keybd_event(0x43,0,2,[UIntPtr]::Zero)
    [ShotKitStepSmoke.Native]::keybd_event(0x11,0,2,[UIntPtr]::Zero)
    # EmptyClipboard can change the sequence before SetClipboardData completes.
    # Wait for the requested format itself, then still verify every pixel below.
    try { $clipboard=Wait-For {
        [uint32]$s=[ShotKitStepSmoke.Native]::ClipboardSequence()
        if($s -ne $sequence) {
            try { Read-ClipboardBitmap }
            catch {
                if($_.Exception.Message -notmatch 'CF_DIBV5 missing after Ctrl\+C|OpenClipboard failed after Ctrl\+C') { throw }
            }
        }
    } 5 'Ctrl+C CF_DIBV5 bitmap' } catch {
        $currentSequence=[ShotKitStepSmoke.Native]::ClipboardSequence()
        $windows=(Get-OwnedWindows | ForEach-Object {"$($_.ClassName):visible=$($_.Visible):title=$($_.Title)"}) -join ', '
        throw "CLIPBOARD_TIMEOUT: sequence before=$sequence after=$currentSequence; foreground=$([ShotKitStepSmoke.Native]::GetForegroundWindow()); owned windows=$windows; error=$($_.Exception.Message)"
    }
    $expected=New-ExpectedImage 12
    try {Assert-StepPixels $clipboard $expected 12 'CLIPBOARD'} finally {$clipboard.Dispose();$expected.Dispose()}
    Write-Output 'PASS: real editor clicks created step numbers 1–12 on dark and light halves; PNG and Ctrl+C CF_DIBV5 pixels matched centered, contrasting Segoe UI Bold references, and Undo/Redo removed/restored number 12.'
}
catch {$failure=$_}
finally {
    if($mouseDown){try{[ShotKitStepSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($null -ne $process -and -not $process.HasExited){try{$process.Kill();[void]$process.WaitForExit(5000)}catch{}}
    if($oldAppData){$env:APPDATA=$oldAppData};if($oldLocalAppData){$env:LOCALAPPDATA=$oldLocalAppData}
    if($null -eq $process -or $process.HasExited){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}

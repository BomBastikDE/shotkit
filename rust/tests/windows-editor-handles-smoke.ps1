[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,
    [ValidateRange(10, 120)]
    [int] $StartupTimeoutSeconds = 30,
    [ValidateNotNullOrEmpty()]
    [string] $PreviewScreenshotPath
)

$ErrorActionPreference = 'Stop'
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or -not [Environment]::UserInteractive) {
    throw 'This smoke test requires an interactive ephemeral GitHub Actions Windows runner.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: refusing to use or change '$imageName'."
}

Add-Type -AssemblyName System.Drawing
if (-not ('ShotKitHandlesSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitHandlesSmoke {
    public struct Rect { public int Left, Top, Right, Bottom; }
    public struct Point { public int X, Y; }
    public sealed class Window { public IntPtr Handle; public int Pid; public string ClassName, Title; public bool Visible; }
    public sealed class Button { public IntPtr Handle; public string Text; }
    public static class Native {
        private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr data);
        [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr data);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder s, int n);
        [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder s, int n);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr hwnd);
        [DllImport("user32.dll")] static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
        [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
        [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
        [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
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
        public static IntPtr FindToolbar(IntPtr parent) {
            IntPtr result=IntPtr.Zero; EnumProc cb=(h,d)=>{ if(IsWindowVisible(h) && Text(h,true)=="ShotKitRustEditorToolbar") { result=h; return false; } return true;}; EnumChildWindows(parent,cb,IntPtr.Zero); GC.KeepAlive(cb); return result;
        }
        public static bool ClientRect(IntPtr h,out Rect r) { return GetClientRect(h,out r); }
        public static void Click(IntPtr h) { SendMessageW(h,0x00F5,IntPtr.Zero,IntPtr.Zero); }
        public static int Owner(IntPtr h) { uint pid; GetWindowThreadProcessId(h,out pid); return (int)pid; }
    }
}
'@
}

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Editor-Handles-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'
$inputPath = Join-Path $root 'handles.png'
$oldAppData = $env:APPDATA; $oldLocalAppData = $env:LOCALAPPDATA
$proc = $null; $mouseDown = $false; $failure = $null; $previewCapturePath = $null
function Get-OwnedWindows { if ($null -eq $proc -or $proc.HasExited) { return @() }; @([ShotKitHandlesSmoke.Native]::Windows($proc.Id)) }
function Get-Main { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' } | Select-Object -First 1 }
function Wait-For([scriptblock]$Probe, [int]$Seconds, [string]$What) {
    $until=[DateTime]::UtcNow.AddSeconds($Seconds)
    while([DateTime]::UtcNow -lt $until) { if($proc.HasExited) { throw "PROCESS_EXITED while waiting for $What (code $($proc.ExitCode))." }; $v=&$Probe; if($v) { return $v }; Start-Sleep -Milliseconds 100 }
    throw "TIMEOUT waiting for $What."
}
function Click-Button([string]$Text) {
    $b=[ShotKitHandlesSmoke.Native]::Buttons($main.Handle) | Where-Object { ($_.Text -replace '&','').Trim() -eq $Text } | Select-Object -First 1
    if($null -eq $b) { throw "BUTTON_NOT_FOUND: $Text" }; [ShotKitHandlesSmoke.Native]::Click($b.Handle)
}
function Read-Image([string]$Path) {
    $bmp=[Drawing.Bitmap]::new($Path)
    try { $colors=[Array]::CreateInstance([Drawing.Color],$bmp.Width*$bmp.Height); for($y=0;$y -lt $bmp.Height;$y++){for($x=0;$x -lt $bmp.Width;$x++){$colors[$y*$bmp.Width+$x]=$bmp.GetPixel($x,$y)}}; return [pscustomobject]@{Width=$bmp.Width;Height=$bmp.Height;Pixels=$colors} }
    finally { $bmp.Dispose() }
}
function Map-Point([int]$x,[int]$length,[int]$origin,[int]$shown) { [int][Math]::Round($origin+($x+0.5)*$shown/$length,[MidpointRounding]::AwayFromZero) }
function Capture-HeldHandlePreview {
    # Capture the actual desktop pixels in the image viewport while the mouse
    # button is still down. This deliberately does not inspect an off-screen
    # bitmap or infer preview state from the window title.
    if([ShotKitHandlesSmoke.Native]::GetForegroundWindow() -ne $main.Handle){throw 'PREVIEW_CAPTURE_FOCUS_LOST.'}
    $origin=[Activator]::CreateInstance([ShotKitHandlesSmoke.Point])
    if(-not [ShotKitHandlesSmoke.Native]::ClientToScreen($main.Handle,[ref]$origin)){throw 'PREVIEW_CAPTURE_ORIGIN_FAILED.'}
    $client=[Activator]::CreateInstance([ShotKitHandlesSmoke.Rect])
    if(-not [ShotKitHandlesSmoke.Native]::ClientRect($main.Handle,[ref]$client)){throw 'PREVIEW_CAPTURE_CLIENT_RECT_FAILED.'}
    $left=$origin.X+$viewX; $top=$origin.Y+$viewY
    if($viewX -lt 0 -or $viewY -lt 0 -or ($viewX+$shownW) -gt $client.Right -or ($viewY+$shownH) -gt $client.Bottom){throw 'PREVIEW_CAPTURE_VIEWPORT_OUTSIDE_CLIENT.'}
    $cursor=[Activator]::CreateInstance([ShotKitHandlesSmoke.Point]); [void][ShotKitHandlesSmoke.Native]::GetCursorPos([ref]$cursor)
    if([ShotKitHandlesSmoke.Native]::Owner([ShotKitHandlesSmoke.Native]::WindowFromPoint($cursor)) -ne $proc.Id){throw 'PREVIEW_CAPTURE_INPUT_TARGET_MISMATCH.'}

    $bitmap=[Drawing.Bitmap]::new($shownW,$shownH,[Drawing.Imaging.PixelFormat]::Format24bppRgb)
    try {
        $graphics=[Drawing.Graphics]::FromImage($bitmap)
        try { $graphics.CopyFromScreen($left,$top,0,0,$bitmap.Size,[Drawing.CopyPixelOperation]::SourceCopy) }
        finally { $graphics.Dispose() }
        $script:previewCapturePath=Join-Path $root 'held-handle-preview.png'
        $bitmap.Save($script:previewCapturePath,[Drawing.Imaging.ImageFormat]::Png)
        if(-not [string]::IsNullOrWhiteSpace($PreviewScreenshotPath)){
            $requestedPath=[IO.Path]::GetFullPath($PreviewScreenshotPath)
            if(Test-Path -LiteralPath $requestedPath){throw "PREVIEW_SCREENSHOT_PATH_EXISTS: refusing to overwrite '$requestedPath'."}
            $requestedDirectory=[IO.Path]::GetDirectoryName($requestedPath)
            if(-not [string]::IsNullOrWhiteSpace($requestedDirectory) -and -not (Test-Path -LiteralPath $requestedDirectory -PathType Container)){
                New-Item -ItemType Directory -Path $requestedDirectory -Force | Out-Null
            }
            $bitmap.Save($requestedPath,[Drawing.Imaging.ImageFormat]::Png)
            $script:previewCapturePath=$requestedPath
        }

        # The first rectangle spans (30,25)-(70,60); the held resize should
        # replace its lower-right handle and yield (30,25)-(120,95). Probe
        # only the unique old/new edges, away from corners and the pointer.
        # Bands are scaled from image coordinates and allow for the 4 px
        # annotation stroke plus StretchBlt/DPI edge sampling. This is a live
        # screen-pixel regression probe for ghost/absent edges, not a golden
        # screenshot comparison; the saved-image pixel checks below remain
        # responsible for final document bounds.
        $scaleX=$shownW/160.0; $scaleY=$shownH/120.0
        $tolX=[Math]::Max(2,[int][Math]::Ceiling($scaleX*2.5)); $tolY=[Math]::Max(2,[int][Math]::Ceiling($scaleY*2.5))
        $oldRightX=Map-Point 70 160 0 $shownW; $oldBottomY=Map-Point 60 120 0 $shownH
        $newRightX=Map-Point 120 160 0 $shownW; $newBottomY=Map-Point 95 120 0 $shownH
        $countRed={ param($x0,$y0,$x1,$y1)
            $count=0
            for($yy=[Math]::Max(0,$y0);$yy -le [Math]::Min($bitmap.Height-1,$y1);$yy++){
                for($xx=[Math]::Max(0,$x0);$xx -le [Math]::Min($bitmap.Width-1,$x1);$xx++){
                    $pixel=$bitmap.GetPixel($xx,$yy)
                    if($pixel.R -gt 180 -and $pixel.G -lt 100 -and $pixel.B -lt 100){$count++}
                }
            }
            return $count
        }
        $oldRightTop=Map-Point 31 120 0 $shownH; $oldRightBottom=Map-Point 56 120 0 $shownH
        $oldBottomLeft=Map-Point 38 160 0 $shownW; $oldBottomRight=Map-Point 68 160 0 $shownW
        $newRightTop=Map-Point 31 120 0 $shownH; $newRightBottom=Map-Point 89 120 0 $shownH
        $newBottomLeft=Map-Point 32 160 0 $shownW; $newBottomRight=Map-Point 115 160 0 $shownW
        $oldRight=&$countRed ($oldRightX-$tolX) $oldRightTop ($oldRightX+$tolX) $oldRightBottom
        $oldBottom=&$countRed $oldBottomLeft ($oldBottomY-$tolY) $oldBottomRight ($oldBottomY+$tolY)
        $newRight=&$countRed ($newRightX-$tolX) $newRightTop ($newRightX+$tolX) $newRightBottom
        $newBottom=&$countRed $newBottomLeft ($newBottomY-$tolY) $newBottomRight ($newBottomY+$tolY)
        if($newRight -lt 5 -or $newBottom -lt 5){throw "LIVE_PREVIEW_NEW_EDGES_MISSING: red pixels right=$newRight bottom=$newBottom; screenshot=$script:previewCapturePath"}
        if($oldRight -gt 0 -or $oldBottom -gt 0){throw "LIVE_PREVIEW_OLD_EDGES_REMAIN: red pixels old-right=$oldRight old-bottom=$oldBottom; screenshot=$script:previewCapturePath"}
        # The selection frame uses RGB (0,120,215), three image pixels
        # outside the shape bounds. Verify its new and old exclusive edges
        # in the same held-drag capture, away from handles and the pointer.
        $countBlue={ param($x0,$y0,$x1,$y1)
            $count=0
            for($yy=[Math]::Max(0,$y0);$yy -le [Math]::Min($bitmap.Height-1,$y1);$yy++){
                for($xx=[Math]::Max(0,$x0);$xx -le [Math]::Min($bitmap.Width-1,$x1);$xx++){
                    $pixel=$bitmap.GetPixel($xx,$yy)
                    if($pixel.R -lt 40 -and $pixel.G -gt 90 -and $pixel.G -lt 150 -and $pixel.B -gt 180){$count++}
                }
            }
            return $count
        }
        $oldFrameRight=Map-Point 73 160 0 $shownW; $oldFrameBottom=Map-Point 63 120 0 $shownH
        $newFrameRight=Map-Point 123 160 0 $shownW; $newFrameBottom=Map-Point 98 120 0 $shownH
        $oldFrameRightCount=&$countBlue ($oldFrameRight-$tolX) $oldRightTop ($oldFrameRight+$tolX) $oldRightBottom
        $oldFrameBottomCount=&$countBlue $oldBottomLeft ($oldFrameBottom-$tolY) $oldBottomRight ($oldFrameBottom+$tolY)
        $newFrameRightCount=&$countBlue ($newFrameRight-$tolX) $newRightTop ($newFrameRight+$tolX) $newRightBottom
        $newFrameBottomCount=&$countBlue $newBottomLeft ($newFrameBottom-$tolY) $newBottomRight ($newFrameBottom+$tolY)
        if($newFrameRightCount -lt 5 -or $newFrameBottomCount -lt 5){throw "LIVE_PREVIEW_NEW_SELECTION_FRAME_MISSING: blue pixels right=$newFrameRightCount bottom=$newFrameBottomCount; screenshot=$script:previewCapturePath"}
        if($oldFrameRightCount -gt 0 -or $oldFrameBottomCount -gt 0){throw "LIVE_PREVIEW_OLD_SELECTION_FRAME_REMAINS: blue pixels old-right=$oldFrameRightCount old-bottom=$oldFrameBottomCount; screenshot=$script:previewCapturePath"}
        $handleY=Map-Point 29 120 0 $shownH
        $newHandleLeft=Map-Point 117 160 0 $shownW; $newHandleRight=Map-Point 122 160 0 $shownW
        $oldHandleLeft=Map-Point 67 160 0 $shownW; $oldHandleRight=Map-Point 72 160 0 $shownW
        $newHandleCount=&$countBlue $newHandleLeft ($handleY-$tolY) $newHandleRight ($handleY+$tolY)
        $oldHandleCount=&$countBlue $oldHandleLeft ($handleY-$tolY) $oldHandleRight ($handleY+$tolY)
        if($newHandleCount -lt 2 -or $oldHandleCount -gt 0){throw "LIVE_PREVIEW_SELECTION_HANDLE_WRONG: blue pixels new=$newHandleCount old=$oldHandleCount; screenshot=$script:previewCapturePath"}
        Write-Host "PASS: selection frame followed the held resize (blue edge pixels new-right=$newFrameRightCount new-bottom=$newFrameBottomCount; old-right=$oldFrameRightCount old-bottom=$oldFrameBottomCount)."
        Write-Host "PASS: held-drag screen preview replaced old lower-right edges with the new bounds (red edge pixels new-right=$newRight new-bottom=$newBottom; old-right=$oldRight old-bottom=$oldBottom)."
    }
    finally { $bitmap.Dispose() }
}
function Drag-Image([int]$x1,[int]$y1,[int]$x2,[int]$y2,[scriptblock]$WhileHeld=$null) {
    $o=[Activator]::CreateInstance([ShotKitHandlesSmoke.Point]); if(-not [ShotKitHandlesSmoke.Native]::ClientToScreen($main.Handle,[ref]$o)){throw 'Could not map editor origin.'}
    $sx=$o.X+(Map-Point $x1 160 $viewX $shownW); $sy=$o.Y+(Map-Point $y1 120 $viewY $shownH)
    $ex=$o.X+(Map-Point $x2 160 $viewX $shownW); $ey=$o.Y+(Map-Point $y2 120 $viewY $shownH)
    if(-not [ShotKitHandlesSmoke.Native]::SetForegroundWindow($main.Handle) -or [ShotKitHandlesSmoke.Native]::GetForegroundWindow() -ne $main.Handle){throw 'EDITOR_FOCUS_FAILED.'}
    if(-not [ShotKitHandlesSmoke.Native]::SetCursorPos($sx,$sy)){throw 'Could not place pointer at drag start.'}
    $p=[Activator]::CreateInstance([ShotKitHandlesSmoke.Point]); [void][ShotKitHandlesSmoke.Native]::GetCursorPos([ref]$p)
    if([ShotKitHandlesSmoke.Native]::Owner([ShotKitHandlesSmoke.Native]::WindowFromPoint($p)) -ne $proc.Id){throw 'INPUT_TARGET_MISMATCH at drag start.'}
    [ShotKitHandlesSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero); $script:mouseDown=$true; Start-Sleep -Milliseconds 120
    if(-not [ShotKitHandlesSmoke.Native]::SetCursorPos($ex,$ey)){throw 'Could not place pointer at drag end.'}; Start-Sleep -Milliseconds 350
    if($null -ne $WhileHeld){&$WhileHeld}
    [ShotKitHandlesSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero); $script:mouseDown=$false; Start-Sleep -Milliseconds 150
}
try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData -Force | Out-Null
    $fixture=[Drawing.Bitmap]::new(160,120,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {$g=[Drawing.Graphics]::FromImage($fixture); try{$g.Clear([Drawing.Color]::Transparent)}finally{$g.Dispose()}; $fixture.Save($inputPath,[Drawing.Imaging.ImageFormat]::Png)} finally{$fixture.Dispose()}
    try { $env:APPDATA=$appData; $env:LOCALAPPDATA=$localAppData; $proc=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru }
    finally { $env:APPDATA=$oldAppData; $env:LOCALAPPDATA=$oldLocalAppData }
    $main=Wait-For { Get-Main } $StartupTimeoutSeconds 'ShotKit editor'
    [void][ShotKitHandlesSmoke.Native]::ShowWindow($main.Handle,3); [void][ShotKitHandlesSmoke.Native]::SetForegroundWindow($main.Handle)
    $notice=Wait-For { Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1 } 10 'ShotKit startup notice'
    $ok=[ShotKitHandlesSmoke.Native]::Buttons($notice.Handle) | Where-Object { ($_.Text -replace '&','').Trim() -eq 'OK' } | Select-Object -First 1
    if($null -eq $ok){throw 'STARTUP_NOTICE_BUTTON_MISSING: expected owned OK button.'}
    [ShotKitHandlesSmoke.Native]::Click($ok.Handle)
    $main=Wait-For { Get-Main | Where-Object {$_.Title -like '*Strg+S speichern*'} | Select-Object -First 1 } 15 'loaded image editor'
    $client=[Activator]::CreateInstance([ShotKitHandlesSmoke.Rect]); [void][ShotKitHandlesSmoke.Native]::ClientRect($main.Handle,[ref]$client)
    # Native toolbar geometry is obtained from its owned child HWND.
    $toolbarHandle=[ShotKitHandlesSmoke.Native]::FindToolbar($main.Handle)
    if($toolbarHandle -eq [IntPtr]::Zero){throw 'EDITOR_NOT_READY: visible editor toolbar was not found.'}
    $tbRect=[Activator]::CreateInstance([ShotKitHandlesSmoke.Rect]); [void][ShotKitHandlesSmoke.Native]::ClientRect($toolbarHandle,[ref]$tbRect)
    $toolbarHeight=$tbRect.Bottom; $viewW=$client.Right; $viewH=$client.Bottom-$toolbarHeight
    $fit=[Math]::Min($viewW/160.0,$viewH/120.0); $shownW=[int][Math]::Round(160*$fit,[MidpointRounding]::AwayFromZero); $shownH=[int][Math]::Round(120*$fit,[MidpointRounding]::AwayFromZero)
    $viewX=[int][Math]::Floor(($viewW-$shownW)/2.0); $viewY=$toolbarHeight+[int][Math]::Floor(($viewH-$shownH)/2.0)
    Click-Button 'Rechteck (R)'
    [void](Wait-For { (Get-Main).Title -like '*Werkzeug Rectangle*' } 5 'Rectangle tool')
    # Create a rectangle. Mouse-up selects it while Rectangle remains active.
    Drag-Image 30 25 70 60
    [void](Wait-For { (Get-Main).Title -like '*Werkzeug Rectangle*' } 3 'Rectangle tool remaining active')
    # Start at the visible lower-right handle and resize, without selecting Select.
    Drag-Image 70 60 120 95 { Capture-HeldHandlePreview }
    [void](Wait-For { (Get-Main).Title -like '*Werkzeug Rectangle*' } 3 'Rectangle tool remaining active after handle drag')
    Click-Button 'Speichern'
    $saved=Wait-For { try { $im=Read-Image $inputPath; if($im.Width -eq 160 -and $im.Height -eq 120){$im} } catch { $null } } 15 'saved annotation image'
    $xs=[System.Collections.Generic.List[int]]::new(); $ys=[System.Collections.Generic.List[int]]::new(); $red=0
    for($y=0;$y -lt 120;$y++){for($x=0;$x -lt 160;$x++){$q=$saved.Pixels[$y*160+$x]; if($q.A -gt 0 -and $q.R -gt 200 -and $q.G -lt 80 -and $q.B -lt 80){$xs.Add($x);$ys.Add($y);$red++}}}
    if($red -lt 50){throw "HANDLE_RESIZE_PIXELS_MISSING: only $red red pixels saved."}
    $left=($xs|Measure-Object -Minimum).Minimum; $right=($xs|Measure-Object -Maximum).Maximum; $top=($ys|Measure-Object -Minimum).Minimum; $bottom=($ys|Measure-Object -Maximum).Maximum
    if([Math]::Abs($left-30) -gt 2 -or [Math]::Abs($top-25) -gt 2 -or [Math]::Abs($right-120) -gt 3 -or [Math]::Abs($bottom-95) -gt 3){throw "HANDLE_RESIZE_BOUNDS_WRONG: saved red bounds ($left,$top)-($right,$bottom), expected approximately (30,25)-(120,95)."}
    $inside=0; for($y=92;$y -le 98;$y++){for($x=117;$x -le 123;$x++){$q=$saved.Pixels[$y*160+$x]; if($q.A -gt 0 -and $q.R -gt 200 -and $q.G -lt 80 -and $q.B -lt 80){$inside++}}}
    if($inside -lt 3){throw "RESIZED_CORNER_PIXELS_MISSING: found $inside red pixels near resized lower-right corner."}
    Write-Host "PASS: Rectangle stayed active while its selected visible handle resized saved bounds to ($left,$top)-($right,$bottom)."
}
catch { $failure=$_ }
finally {
    if($mouseDown){try{[ShotKitHandlesSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    if($null -ne $proc -and -not $proc.HasExited){try{$proc.CloseMainWindow()|Out-Null; if(-not $proc.WaitForExit(5000)){ $proc.Kill(); [void]$proc.WaitForExit(5000) }}catch{}}
    if($oldAppData){$env:APPDATA=$oldAppData}; if($oldLocalAppData){$env:LOCALAPPDATA=$oldLocalAppData}
    if($null -eq $proc -or $proc.HasExited){Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue}
}
if($null -ne $failure){throw $failure}

# Interactive CI contract for normal file-import admission, without a product test bypass.
# Fixture: valid 10000x10000 RGBA8 PNG, complete zlib raster generated one row at a
# time. 16 MiB + 2*400000000 decoder bytes + 400000000 RGBA = 1216777216 bytes,
# exceeding the 939524096-byte (896 MiB) normal limit even with no retained imports.
# Dimensions and the 400 MB raster remain below the independent 30000/512 MiB limits.
# Repeated rejection checks cleanup, existing dirty pixels and observable undo/redo.
# This does not prove aggregate admission, retained-image accounting or process RSS.
# Fixture generation uses bounded row/compressed buffers; no huge Bitmap is constructed.
# Generation/validation have 30/45-second limits, UI run a 200-second polling limit.
# Polling/768-MiB private-byte guard is a regression tripwire, not an OS memory cap.
# A broken admission may allocate between polls; run only on an ephemeral CI desktop.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if ($PSVersionTable.PSVersion -lt [Version]'7.2') { throw 'PowerShell 7.2+ (pwsh, .NET ZLibStream) is required.' }
if (($env:GITHUB_ACTIONS -ne 'true') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'INTERACTIVE_WINDOWS_REQUIRED: ephemeral interactive Windows CI runner is required.'
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
namespace ShotKitImportBudgetSmoke {
 public struct Rect { public int Left,Top,Right,Bottom; }
 public struct Point { public int X,Y; }
 public sealed class Win { public IntPtr H; public string ClassName,Title; public bool Visible; }
 public sealed class Child { public IntPtr H; public int Id; public string ClassName,Text; public bool Visible; }
 public static class Native {
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,uint x,uint y,uint data,UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int cmd);
  // Complete PNG chunks, CRC32 and zlib stream; never allocate the decoded raster.
  static void Be(System.IO.Stream s,uint n){s.WriteByte((byte)(n>>24));s.WriteByte((byte)(n>>16));s.WriteByte((byte)(n>>8));s.WriteByte((byte)n);}
  static void Chunk(System.IO.Stream s,string type,byte[] data){
   Be(s,(uint)data.Length);byte[] name=Encoding.ASCII.GetBytes(type);s.Write(name,0,4);s.Write(data,0,data.Length);
   uint crc=0xffffffff;foreach(byte b in name){crc^=b;for(int k=0;k<8;k++)crc=(crc&1)!=0?(crc>>1)^0xedb88320:crc>>1;}
   foreach(byte b in data){crc^=b;for(int k=0;k<8;k++)crc=(crc&1)!=0?(crc>>1)^0xedb88320:crc>>1;}Be(s,crc^0xffffffff);
  }
  public static void BudgetPng(string path){
   var clock=System.Diagnostics.Stopwatch.StartNew();byte[] row=new byte[40001];
   // PNG filter byte=0; opaque black RGBA pixels (not an invalid/missing payload).
   for(int x=4;x<row.Length;x+=4)row[x]=255;
   using(var compressed=new System.IO.MemoryStream()){
    using(var z=new System.IO.Compression.ZLibStream(compressed,System.IO.Compression.CompressionLevel.Fastest,true)){
     for(int y=0;y<10000;y++){z.Write(row,0,row.Length);if(clock.Elapsed.TotalSeconds>30||compressed.Length>4194304)throw new Exception("FIXTURE_GENERATION_LIMIT");}
    }
    byte[] data=compressed.ToArray();if(data.Length>4194304)throw new Exception("FIXTURE_TOO_LARGE");
    // Verify full decompression length with bounded buffers, independently of app import.
    using(var input=new System.IO.MemoryStream(data))using(var z=new System.IO.Compression.ZLibStream(input,System.IO.Compression.CompressionMode.Decompress)){
     long count=0;int n;while((n=z.Read(row,0,row.Length))>0){count+=n;if(count>400010000||clock.Elapsed.TotalSeconds>45)throw new Exception("FIXTURE_VALIDATION_LIMIT");}
     if(count!=400010000)throw new Exception("FIXTURE_RASTER_LENGTH");
    }
    using(var file=System.IO.File.Create(path)){
     file.Write(new byte[]{137,80,78,71,13,10,26,10},0,8);
     using(var header=new System.IO.MemoryStream()){Be(header,10000);Be(header,10000);header.Write(new byte[]{8,6,0,0,0},0,5);Chunk(file,"IHDR",header.ToArray());}
     Chunk(file,"IDAT",data);Chunk(file,"IEND",new byte[0]);
    }
   }
  }
  delegate bool EnumProc(IntPtr h,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb,IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h,EnumProc cb,IntPtr d);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h,StringBuilder s,int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll",SetLastError=true)] static extern IntPtr SendMessageTimeoutW(IntPtr h,uint m,IntPtr w,IntPtr l,uint flags,uint timeout,out IntPtr result);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h,uint m,IntPtr w,IntPtr l);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k,byte scan,uint flags,UIntPtr extra);
  [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
  [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr owner);
  [DllImport("user32.dll")] static extern bool CloseClipboard();
  [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint format);
  [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr memory);
  [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr memory);
  [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr memory);
  [DllImport("user32.dll",SetLastError=true)] static extern IntPtr OpenInputDesktop(uint flags,bool inherit,uint access);
  [DllImport("user32.dll",SetLastError=true)] static extern bool CloseDesktop(IntPtr desktop);
  [DllImport("user32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool GetUserObjectInformationW(IntPtr obj,int index,StringBuilder info,uint length,out uint needed);
  static string Read(IntPtr h,bool cls){var s=new StringBuilder(512);if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString();}
  public static Win[] Windows(int pid){var a=new List<Win>();EnumProc cb=(h,d)=>{uint p;GetWindowThreadProcessId(h,out p);if(p==(uint)pid)a.Add(new Win{H=h,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static uint Owner(IntPtr h){uint p;GetWindowThreadProcessId(h,out p);return p;}
  public static Child[] Children(IntPtr parent){var a=new List<Child>();EnumProc cb=(h,d)=>{a.Add(new Child{H=h,Id=GetDlgCtrlID(h),ClassName=Read(h,true),Text=Read(h,false),Visible=IsWindowVisible(h)});return true;};EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();}
  public static string WindowText(IntPtr parent){var s=new StringBuilder(Read(parent,false));foreach(var c in Children(parent))if(!String.IsNullOrWhiteSpace(c.Text))s.Append(" ").Append(c.Text);return s.ToString();}
  public static bool SetEditText(IntPtr h,string value){IntPtr p=Marshal.StringToHGlobalUni(value),result;try{return SendMessageTimeoutW(h,0x000C,IntPtr.Zero,p,0x0002,3000,out result)!=IntPtr.Zero;}finally{Marshal.FreeHGlobal(p);}}
  public static string InputDesktopName(){IntPtr d=OpenInputDesktop(0,false,1);if(d==IntPtr.Zero)return null;try{var s=new StringBuilder(256);uint needed;if(!GetUserObjectInformationW(d,2,s,(uint)(s.Capacity*2),out needed))return null;return s.ToString();}finally{CloseDesktop(d);}}
  public static uint ClipboardSequence(){return GetClipboardSequenceNumber();}
  public static byte[] ReadDibV5(){if(!OpenClipboard(IntPtr.Zero))throw new InvalidOperationException("OpenClipboard failed.");IntPtr mem=IntPtr.Zero,ptr=IntPtr.Zero;try{mem=GetClipboardData(17);if(mem==IntPtr.Zero)throw new InvalidOperationException("CF_DIBV5 is absent.");ulong size=GlobalSize(mem).ToUInt64();if(size<124||size>67108864)throw new InvalidOperationException("CF_DIBV5 block size invalid: "+size);ptr=GlobalLock(mem);if(ptr==IntPtr.Zero)throw new InvalidOperationException("GlobalLock failed.");byte[] b=new byte[(int)size];Marshal.Copy(ptr,b,0,b.Length);return b;}finally{if(ptr!=IntPtr.Zero)GlobalUnlock(mem);if(!CloseClipboard())throw new InvalidOperationException("CloseClipboard failed.");}}
 }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Import-Budget-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'
$settingsDir = Join-Path $appData 'ShotKitRust'
$fixturePath = Join-Path $root 'existing-document.png'
$oversizedPath = Join-Path $root 'budget-10000x10000.png'
$oldAppData = $env:APPDATA; $oldLocalAppData = $env:LOCALAPPDATA
$mouseDown = $false
$runClock = [Diagnostics.Stopwatch]::StartNew()
$process = $null; $processStopped = $true; $failure = $null; $cleanupFailure = $null

$inputDesktop = [ShotKitImportBudgetSmoke.Native]::InputDesktopName()
if ($inputDesktop -cne 'Default') { throw "INTERACTIVE_DEFAULT_DESKTOP_REQUIRED: current input desktop is '$inputDesktop'." }

function Get-OwnedWindows {
    if ($null -eq $process -or $process.HasExited) { return @() }
    @([ShotKitImportBudgetSmoke.Native]::Windows($process.Id))
}
function Wait-For([scriptblock] $Probe, [string] $Description, [int] $Seconds = $TimeoutSeconds) {
    $end = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $end) {
        if ($runClock.Elapsed.TotalSeconds -gt 200) { throw 'TOTAL_RUNTIME_LIMIT: 200 seconds exceeded.' }
        if ($process.HasExited) { throw "PROCESS_EXITED while waiting for $Description (code $($process.ExitCode))." }
        $process.Refresh()
        if ($process.PrivateMemorySize64 -gt 768MB) { throw 'PRIVATE_MEMORY_GUARD: owned process exceeded 768 MiB.' }
        $value = & $Probe
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    $windows = (Get-OwnedWindows | ForEach-Object { "HWND=$($_.H) class='$($_.ClassName)' title='$($_.Title)' visible=$($_.Visible)" }) -join '; '
    throw "TIMEOUT waiting for $Description; owned windows: $windows"
}
function Send-CtrlC([IntPtr] $Editor) {
    if ([ShotKitImportBudgetSmoke.Native]::GetForegroundWindow() -ne $Editor) { throw 'EDITOR_NOT_FOREGROUND: refusing keyboard copy.' }
    [ShotKitImportBudgetSmoke.Native]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero)
    try {
        [ShotKitImportBudgetSmoke.Native]::keybd_event(0x43, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 80
        [ShotKitImportBudgetSmoke.Native]::keybd_event(0x43, 0, 2, [UIntPtr]::Zero)
    } finally { [ShotKitImportBudgetSmoke.Native]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero) }
}
function Get-EditorCopy([IntPtr] $Editor, [string] $What) {
    $before = [ShotKitImportBudgetSmoke.Native]::ClipboardSequence()
    Send-CtrlC $Editor
    $copy = $null; $sequence = $before; $end = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $end) {
        if ($process.HasExited) { throw "PROCESS_EXITED during $What clipboard export." }
        $sequence = [ShotKitImportBudgetSmoke.Native]::ClipboardSequence()
        if ($sequence -ne $before) {
            try { $copy = [ShotKitImportBudgetSmoke.Native]::ReadDibV5(); break }
            catch { if ($_.Exception.Message -notlike '*CF_DIBV5 is absent*' -and $_.Exception.Message -notlike '*OpenClipboard failed*') { throw } }
        }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq $copy) { throw "${What}_CLIPBOARD_TIMEOUT: editor did not publish CF_DIBV5." }
    if ($sequence -eq $before) { throw "${What}_CLIPBOARD_SEQUENCE_INVALID: clipboard did not change." }
    $header = [BitConverter]::ToUInt32($copy,0); $width = [BitConverter]::ToInt32($copy,4); $height = [BitConverter]::ToInt32($copy,8)
    if ($header -ne 124 -or $width -ne 160 -or [Math]::Abs([long]$height) -ne 120) { throw "${What}_DIMENSION_MISMATCH: CF_DIBV5 header=$header dimensions=${width}x${height}." }
    return [pscustomobject]@{ Bytes = $copy; Hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($copy)); Sequence = $sequence }
}
function Stop-OwnedProcess {
    if ($null -eq $process) { return }
    if (-not $process.HasExited) {
        foreach ($window in [ShotKitImportBudgetSmoke.Native]::Windows($process.Id)) { [void][ShotKitImportBudgetSmoke.Native]::PostMessageW($window.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) }
        if (-not $process.WaitForExit(5000)) { $process.Kill(); [void]$process.WaitForExit(5000) }
    }
    $script:processStopped = $process.HasExited
    $process.Dispose(); $script:process = $null
    if (-not $script:processStopped) { throw 'OWNED_PROCESS_CLEANUP_FAILED: the started ShotKit process is still running.' }
}


function Assert-OriginalPixels([byte[]] $Dib) {
    if ([BitConverter]::ToUInt16($Dib,14) -ne 32 -or $Dib.Length -lt (124+160*120*4)) { throw 'ORIGINAL_DIB_LAYOUT_INVALID.' }
    $height=[BitConverter]::ToInt32($Dib,8)
    for ($y=0; $y -lt 120; $y++) {
        $row=if ($height -lt 0) { $y } else { 119-$y }
        for ($x=0; $x -lt 160; $x++) {
            $i=124+($row*160+$x)*4
            if ($Dib[$i] -ne (($x*19+$y*13)%256) -or $Dib[$i+1] -ne (($x*11+$y*29)%256) -or $Dib[$i+2] -ne (($x*31+$y*7)%256) -or $Dib[$i+3] -ne 255) { throw "ORIGINAL_PIXEL_MISMATCH: ($x,$y)." }
        }
    }
}
function Click-OwnedButton([IntPtr] $Editor, [string] $Text) {
    if ([ShotKitImportBudgetSmoke.Native]::Owner($Editor) -ne [uint32]$process.Id) { throw 'EDITOR_OWNER_MISMATCH.' }
    $buttons = @([ShotKitImportBudgetSmoke.Native]::Children($Editor) | Where-Object { $_.Visible -and $_.ClassName -eq 'Button' -and ($_.Text -replace '&','').Trim() -eq $Text })
    if ($buttons.Count -ne 1 -or -not [ShotKitImportBudgetSmoke.Native]::IsWindowEnabled($buttons[0].H)) { throw "BUTTON_UNAVAILABLE: $Text" }
    if (-not [ShotKitImportBudgetSmoke.Native]::PostMessageW($buttons[0].H,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)) { throw "BUTTON_POST_FAILED: $Text" }
}
function Assert-History([IntPtr] $Editor, $Original, $Edited) {
    Click-OwnedButton $Editor 'Zurück'
    [void](Wait-For { Get-OwnedWindows | Where-Object { $_.H -eq $Editor -and -not $_.Title.StartsWith('* ') } } 'clean undo state')
    $undoCopy = Get-EditorCopy $Editor 'UNDO'
    if ($undoCopy.Hash -cne $Original.Hash) { throw 'UNDO_PIXELS_CHANGED_OR_HISTORY_LOST.' }
    Click-OwnedButton $Editor 'Wiederholen'
    [void](Wait-For { Get-OwnedWindows | Where-Object { $_.H -eq $Editor -and $_.Title.StartsWith('* ') } } 'dirty redo state')
    $redoCopy = Get-EditorCopy $Editor 'REDO'
    if ($redoCopy.Hash -cne $Edited.Hash) { throw 'REDO_PIXELS_CHANGED_OR_HISTORY_LOST.' }
}
function Edit-Document([IntPtr] $Editor) {
    Click-OwnedButton $Editor 'Verpixeln (B) – keine sichere Schwärzung'
    [void](Wait-For { Get-OwnedWindows | Where-Object { $_.H -eq $Editor -and $_.Title -like '*Werkzeug: Verpixeln*' } } 'pixelate tool')
    $client=[ShotKitImportBudgetSmoke.Rect]::new()
    if (-not [ShotKitImportBudgetSmoke.Native]::GetClientRect($Editor,[ref]$client)) { throw 'CLIENT_RECT_FAILED.' }
    $toolbars=@([ShotKitImportBudgetSmoke.Native]::Children($Editor) | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar' })
    if ($toolbars.Count -ne 1) { throw 'TOOLBAR_AMBIGUOUS.' }
    $tr=[ShotKitImportBudgetSmoke.Rect]::new()
    if (-not [ShotKitImportBudgetSmoke.Native]::GetClientRect($toolbars[0].H,[ref]$tr)) { throw 'TOOLBAR_RECT_FAILED.' }
    $vw=$client.Right; $vh=$client.Bottom-$tr.Bottom
    $fit=[Math]::Min($vw/160.0,$vh/120.0)
    $dw=[int][Math]::Round(160*$fit); $dh=[int][Math]::Round(120*$fit)
    $vx=[int][Math]::Floor(($vw-$dw)/2.0); $vy=$tr.Bottom+[int][Math]::Floor(($vh-$dh)/2.0)
    $origin=[ShotKitImportBudgetSmoke.Point]::new()
    if (-not [ShotKitImportBudgetSmoke.Native]::ClientToScreen($Editor,[ref]$origin)) { throw 'CLIENT_ORIGIN_FAILED.' }
    $start=[ShotKitImportBudgetSmoke.Point]::new(); $start.X=$origin.X+$vx+[int](24.5*$dw/160); $start.Y=$origin.Y+$vy+[int](20.5*$dh/120)
    $end=[ShotKitImportBudgetSmoke.Point]::new(); $end.X=$origin.X+$vx+[int](120.5*$dw/160); $end.Y=$origin.Y+$vy+[int](90.5*$dh/120)
    foreach ($point in @($start,$end)) {
        if ([ShotKitImportBudgetSmoke.Native]::WindowFromPoint($point) -ne $Editor) { throw 'DRAG_TARGET_MISMATCH.' }
    }
    if ([ShotKitImportBudgetSmoke.Native]::GetForegroundWindow() -ne $Editor) { throw 'DRAG_FOREGROUND_MISMATCH.' }
    if (-not [ShotKitImportBudgetSmoke.Native]::SetCursorPos($start.X,$start.Y)) { throw 'CURSOR_START_FAILED.' }
    [ShotKitImportBudgetSmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero); $script:mouseDown=$true
    try {
        Start-Sleep -Milliseconds 100
        if (-not [ShotKitImportBudgetSmoke.Native]::SetCursorPos($end.X,$end.Y)) { throw 'CURSOR_END_FAILED.' }
        Start-Sleep -Milliseconds 200
    } finally { [ShotKitImportBudgetSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero); $script:mouseDown=$false }
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData,$settingsDir -Force | Out-Null
    $settings = @{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'} | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $settingsDir 'settings.json'),$settings,[Text.UTF8Encoding]::new($false))

    $small = [Drawing.Bitmap]::new(160,120,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        for ($y=0; $y -lt 120; $y++) { for ($x=0; $x -lt 160; $x++) { $small.SetPixel($x,$y,[Drawing.Color]::FromArgb(255,($x*31+$y*7)%256,($x*11+$y*29)%256,($x*19+$y*13)%256)) } }
        $small.Save($fixturePath,[Drawing.Imaging.ImageFormat]::Png)
    } finally { $small.Dispose() }

    [ShotKitImportBudgetSmoke.Native]::BudgetPng($oversizedPath)
    $inputHash = (Get-FileHash -LiteralPath $oversizedPath -Algorithm SHA256).Hash

    $env:APPDATA = $appData; $env:LOCALAPPDATA = $localAppData
    try { $process = Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $fixturePath) -WorkingDirectory $root -PassThru; $processStopped = $false }
    finally { $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData }

    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $process -GetWindows { Get-OwnedWindows }
    $editor = Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*' } | Select-Object -First 1 } 'loaded fixture editor'
    if (-not [ShotKitImportBudgetSmoke.Native]::SetForegroundWindow($editor.H)) { throw 'EDITOR_FOCUS_FAILED.' }
    [void](Wait-For { [ShotKitImportBudgetSmoke.Native]::GetForegroundWindow() -eq $editor.H } 'editor foreground')
    [void][ShotKitImportBudgetSmoke.Native]::ShowWindow($editor.H,3)
    $originalCopy = Get-EditorCopy $editor.H 'ORIGINAL'
    Assert-OriginalPixels $originalCopy.Bytes
    $sourceHash = (Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash
    Edit-Document $editor.H
    [void](Wait-For { Get-OwnedWindows | Where-Object { $_.H -eq $editor.H -and $_.Title.StartsWith('* ') } } 'dirty edited title')
    $beforeCopy = Get-EditorCopy $editor.H 'EDITED'
    if ($beforeCopy.Hash -ceq $originalCopy.Hash) { throw 'EDIT_DID_NOT_CHANGE_PIXELS.' }
    Assert-History $editor.H $originalCopy $beforeCopy
    $originalTitle = (Get-OwnedWindows | Where-Object { $_.H -eq $editor.H }).Title
    $originalEditorHandle = $editor.H
    Write-Host "Owned PID=$($process.Id) editor HWND=$originalEditorHandle; fixture=$((Get-Item -LiteralPath $oversizedPath).Length) bytes; expected import peak=1216777216, normal limit=939524096."
    $firstAdmission = $null
    for ($attempt=1; $attempt -le 2; $attempt++) {
        # Use the editor's exact Open toolbar command so this exercises the existing-document path.
        if (-not [ShotKitImportBudgetSmoke.Native]::PostMessageW($editor.H,0x0111,[IntPtr]0x5141,[IntPtr]::Zero)) { throw 'OPEN_COMMAND_POST_FAILED.' }
        $openDialog = Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and @([ShotKitImportBudgetSmoke.Native]::Children($_.H) | Where-Object { $_.ClassName -eq 'Edit' -and $_.Visible }).Count -gt 0 } | Select-Object -First 1 } 'owned Open dialog'
        if ([ShotKitImportBudgetSmoke.Native]::Owner($openDialog.H) -ne [uint32]$process.Id) { throw 'OPEN_DIALOG_OWNER_MISMATCH: refusing to alter an unowned dialog.' }
        $edits = @([ShotKitImportBudgetSmoke.Native]::Children($openDialog.H) | Where-Object { $_.Visible -and $_.ClassName -eq 'Edit' })
        if ($edits.Count -ne 1) { throw "OPEN_FILENAME_CONTROL_AMBIGUOUS: expected one visible Edit, found $($edits.Count)." }
        [void][ShotKitImportBudgetSmoke.Native]::SetForegroundWindow($openDialog.H)
        [void](Wait-For { [ShotKitImportBudgetSmoke.Native]::GetForegroundWindow() -eq $openDialog.H } 'Open dialog foreground')
        if (-not [ShotKitImportBudgetSmoke.Native]::SetEditText($edits[0].H,$oversizedPath)) { throw 'OPEN_FILENAME_SET_FAILED.' }
        $openButton = @([ShotKitImportBudgetSmoke.Native]::Children($openDialog.H) | Where-Object { $_.Visible -and $_.ClassName -eq 'Button' -and $_.Id -eq 1 -and (($_.Text -replace '&','').Trim() -match '^(Open|Öffnen)$') })
        if ($openButton.Count -ne 1) { throw "OPEN_BUTTON_AMBIGUOUS: expected one owned Open button, found $($openButton.Count)." }
        if (-not [ShotKitImportBudgetSmoke.Native]::PostMessageW($openButton[0].H,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)) { throw 'OPEN_BUTTON_CLICK_POST_FAILED.' }

        $errorDialog = Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq 'Bild öffnen' } | Select-Object -First 1 } 'expected image import error' 10
        if ([ShotKitImportBudgetSmoke.Native]::Owner($errorDialog.H) -ne [uint32]$process.Id) { throw 'ERROR_DIALOG_OWNER_MISMATCH.' }
        $errorText = [ShotKitImportBudgetSmoke.Native]::WindowText($errorDialog.H)
        if ($errorText -notlike '*Bereits geöffnete Bilder bleiben erhalten.*') { throw "IMPORT_MESSAGE_NOT_ACTIONABLE: '$errorText'." }
        if ($errorText -match 'cannot reserve ([0-9]+) additional bytes: ([0-9]+) used, admission limit 939524096') {
            $admission = "$($Matches[1]):$($Matches[2])"
        } else { throw "WRONG_IMPORT_ERROR: expected normal-budget admission rejection, got '$errorText'." }
        if ($null -eq $firstAdmission) { $firstAdmission = $admission }
        elseif ($admission -cne $firstAdmission) { throw "BUDGET_RESERVATION_NOT_RELEASED: first=$firstAdmission, later=$admission." }
        [void][ShotKitImportBudgetSmoke.Native]::PostMessageW($errorDialog.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)
        [void](Wait-For { -not @(Get-OwnedWindows | Where-Object { $_.ClassName -eq '#32770' -and $_.Title -eq 'Bild öffnen' }).Count } 'expected import error close' 8)

        $editor = Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -eq $originalTitle } | Select-Object -First 1 } 'same original editor after rejected import'
        if (-not [ShotKitImportBudgetSmoke.Native]::SetForegroundWindow($editor.H)) { throw 'EDITOR_REFOCUS_FAILED after import error.' }
        [void](Wait-For { [ShotKitImportBudgetSmoke.Native]::GetForegroundWindow() -eq $editor.H } 'editor refocus after error')
        $afterCopy = Get-EditorCopy $editor.H 'AFTER_IMPORT'
        if ($beforeCopy.Hash -cne $afterCopy.Hash -or $beforeCopy.Bytes.Length -ne $afterCopy.Bytes.Length) {
            throw "DOCUMENT_CHANGED: CF_DIBV5 SHA256 before=$($beforeCopy.Hash), after=$($afterCopy.Hash); bytes before=$($beforeCopy.Bytes.Length), after=$($afterCopy.Bytes.Length)."
        }
        $editors = @(Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' })
        if ($editors.Count -ne 1 -or $editor.H -ne $originalEditorHandle) { throw 'EDITOR_IDENTITY_CHANGED after rejected import.' }
        Assert-History $editor.H $originalCopy $beforeCopy
        Write-Host "Budget rejection $attempt preserved dirty editor HWND=$($editor.H), pixels and undo/redo."
        if ((Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash -cne $sourceHash) { throw 'UNSAVED_SOURCE_WAS_WRITTEN.' }
    }
    $afterInputHash = (Get-FileHash -LiteralPath $oversizedPath -Algorithm SHA256).Hash
    if ($inputHash -cne $afterInputHash) { throw "INPUT_MODIFIED: SHA256 before=$inputHash after=$afterInputHash." }
    Write-Host "PASS: two 10000x10000 RGBA PNG imports rejected by 896-MiB normal admission; the same dirty editor, byte-identical CF_DIBV5 export and undo/redo states survived (SHA256 $($beforeCopy.Hash)); source files unchanged."
} catch { $failure = $_ }
finally {
    if ($mouseDown) { [ShotKitImportBudgetSmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero) }
    $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData
    if ($null -ne $process) { try { Stop-OwnedProcess } catch { $cleanupFailure = $_.Exception.Message } }
    if ($processStopped -and (Test-Path -LiteralPath $root)) { try { Remove-Item -LiteralPath $root -Recurse -Force } catch { if (-not $cleanupFailure) { $cleanupFailure = $_.Exception.Message } } }
    elseif (-not $processStopped) { $cleanupFailure = "Temporary fixture and app-data retained because the test-owned editor may still be running: $root" }
}
if ($cleanupFailure) { throw "CLEANUP_FAILED: $cleanupFailure; original failure: $failure" }
if ($failure) { throw $failure }

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(30, 120)][int] $TimeoutSeconds = 60,
    [switch] $WithoutUndo
)

$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'INTERACTIVE_WINDOWS_REQUIRED: an ephemeral Windows runner or dedicated local opt-in is required; this test controls mouse input and replaces clipboard contents.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw 'SHOTKIT_EXE_NOT_FOUND.' }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
foreach ($name in @([IO.Path]::GetFileNameWithoutExtension($ExePath), 'ShotKit', 'shotkit-rust-prototype') | Select-Object -Unique) {
    if (@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count -gt 0) { throw 'PREEXISTING_SHOTKIT_PROCESS: a dedicated test session is required.' }
}

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
namespace ShotKitHistorySmoke {
 public sealed class Win { public IntPtr H; public string ClassName, Title; public bool Visible; }
 public struct Rect { public int Left, Top, Right, Bottom; }
 public static class Native {
  delegate bool EnumProc(IntPtr h, IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h, EnumProc cb, IntPtr d);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
  static string Read(IntPtr h, bool cls) { var s=new System.Text.StringBuilder(512); if(cls)GetClassNameW(h,s,s.Capacity);else GetWindowTextW(h,s,s.Capacity);return s.ToString(); }
  static uint Owner(IntPtr h) { uint p; GetWindowThreadProcessId(h,out p); return p; }
  public static Win[] Windows(int pid) {
   var a=new List<Win>(); EnumProc cb=(h,d)=>{if(Owner(h)==(uint)pid)a.Add(new Win{H=h,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};
   EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();
  }
  public static Win[] Children(IntPtr parent) {
   var a=new List<Win>(); EnumProc cb=(h,d)=>{a.Add(new Win{H=h,ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h)});return true;};
   EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();
  }
  public static int OwnerPid(IntPtr h) { return (int)Owner(h); }
  [DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();
  [DllImport("user32.dll")] static extern IntPtr GetClipboardOwner();
  [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr h);
  [DllImport("user32.dll")] static extern bool CloseClipboard();
  [DllImport("user32.dll")] static extern bool EmptyClipboard();
  [DllImport("user32.dll")] static extern IntPtr GetClipboardData(uint format);
  [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr h);
  [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr h);
  [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr h);
  public static string Fingerprint(int pid) {
   if(Owner(GetClipboardOwner())!=(uint)pid || !OpenClipboard(IntPtr.Zero)) return null;
   try {
    // Large images publish CF_DIBV5 only; legacy CF_DIB/PNG are optional
    // and capped at 16 MiB by ShotKit's actual clipboard output path.
    IntPtr h=GetClipboardData(17); if(h==IntPtr.Zero)return null;
    ulong size=GlobalSize(h).ToUInt64();if(size<124 || size>200000124)throw new InvalidOperationException("CLIPBOARD_DIBV5_SIZE_INVALID.");
    IntPtr p=GlobalLock(h);if(p==IntPtr.Zero)return null;
    try {
     int header=Marshal.ReadInt32(p),w=Marshal.ReadInt32(p,4),height=Marshal.ReadInt32(p,8);
     if(header!=124 || w<=0 || height==0 || height==int.MinValue || Marshal.ReadInt16(p,12)!=1 ||
        Marshal.ReadInt16(p,14)!=32 || Marshal.ReadInt32(p,16)!=3 ||
        unchecked((uint)Marshal.ReadInt32(p,40))!=0x00ff0000u ||
        unchecked((uint)Marshal.ReadInt32(p,44))!=0x0000ff00u ||
        unchecked((uint)Marshal.ReadInt32(p,48))!=0x000000ffu ||
        unchecked((uint)Marshal.ReadInt32(p,52))!=0xff000000u)
      throw new InvalidOperationException("CLIPBOARD_DIBV5_BGRA_LAYOUT_INVALID.");
     int rows=Math.Abs(height),rowBytes=checked(w*4),n=checked(rowBytes*rows);
     if((ulong)n+124>size || unchecked((uint)Marshal.ReadInt32(p,20))!=(uint)n)
      throw new InvalidOperationException("CLIPBOARD_DIBV5_PIXEL_SIZE_INVALID.");
     var bytes=new byte[n];
     // Fingerprints always use top-down BGRA, respecting signed biHeight.
     if(height<0)Marshal.Copy(IntPtr.Add(p,124),bytes,0,n);
     else for(int y=0;y<rows;y++)Marshal.Copy(IntPtr.Add(p,checked(124+(rows-1-y)*rowBytes)),bytes,checked(y*rowBytes),rowBytes);
     using(var sha=System.Security.Cryptography.SHA256.Create())
      return w+"x"+Math.Abs(height)+":"+BitConverter.ToString(sha.ComputeHash(bytes)).Replace("-","");
    } finally { GlobalUnlock(h); }
   } finally { CloseClipboard(); }
  }
  public static void ClearOwnedClipboard(int pid) {
   if(Owner(GetClipboardOwner())!=(uint)pid)return;
   if(!OpenClipboard(IntPtr.Zero))throw new InvalidOperationException("OWNED_CLIPBOARD_CLEANUP_FAILED.");
   try { if(Owner(GetClipboardOwner())==(uint)pid && !EmptyClipboard())throw new InvalidOperationException("OWNED_CLIPBOARD_CLEANUP_FAILED."); }
   finally { CloseClipboard(); }
  }
  public struct Point { public int X,Y; }
 }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-History-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData=Join-Path $root 'appdata'; $localAppData=Join-Path $root 'localappdata'
$fixtureWidth=if($WithoutUndo){8000}else{6000}; $fixtureHeight=if($WithoutUndo){5000}else{4000}
$settingsDir=Join-Path $appData 'ShotKitRust'; $fixturePath=Join-Path $root ("${fixtureWidth}x${fixtureHeight}.bmp")
$oldAppData=$env:APPDATA; $oldLocalAppData=$env:LOCALAPPDATA
$process=$null; $processStopped=$true; $mouseDown=$false; $failure=$null; $cleanupFailure=$null
$editorHandle=[IntPtr]::Zero

function Get-OwnedWindows {
    if ($null -eq $process -or $process.HasExited) { return @() }
    @([ShotKitHistorySmoke.Native]::Windows($process.Id))
}
function Wait-For([scriptblock] $Probe, [string] $Description, [int] $Seconds = $TimeoutSeconds) {
    $end = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $end) {
        if ($process.HasExited) { throw "PROCESS_EXITED while waiting for $Description (code $($process.ExitCode))." }
        $value = & $Probe
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    throw "TIMEOUT waiting for $Description; owned window count=$(@(Get-OwnedWindows).Count)."
}
function Write-LargeBmp([string] $Path,[int] $Width = 6000,[int] $Height = 4000) {
    # Both fixture widths have 4-byte aligned 24bpp rows; no BMP padding is needed.
    $rowBytes=$Width*3; $pixelBytes=$rowBytes*$Height
    $stream=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try {
        $writer=[IO.BinaryWriter]::new($stream)
        try {
            $writer.Write([byte]0x42);$writer.Write([byte]0x4D);$writer.Write([uint32](54+$pixelBytes));$writer.Write([uint16]0);$writer.Write([uint16]0);$writer.Write([uint32]54)
            $writer.Write([uint32]40);$writer.Write([int32]$width);$writer.Write([int32]$height);$writer.Write([uint16]1);$writer.Write([uint16]24)
            $writer.Write([uint32]0);$writer.Write([uint32]$pixelBytes);$writer.Write([int32]2835);$writer.Write([int32]2835);$writer.Write([uint32]0);$writer.Write([uint32]0)
            $row=[byte[]]::new($rowBytes)
            for($x=0;$x -lt $width;$x++){$i=$x*3;$row[$i]=[byte]($x%256);$row[$i+1]=[byte](($x*3)%256);$row[$i+2]=[byte](($x*11)%256)}
            for($y=0;$y -lt $height;$y++){$writer.Write($row)}
            $writer.Flush()
        } finally { $writer.Dispose() }
    } finally { $stream.Dispose() }
    if((Get-Item -LiteralPath $Path).Length -ne (54+$pixelBytes)){throw 'FIXTURE_INVALID: generated BMP length was unexpected.'}
}
function Drag-Crop([IntPtr] $Editor,[int] $ShotKitProcessId,[int] $Width,[int] $Height) {
    Send-Command 0x510B
    Send-Command 0x5153
    [void](Read-Fingerprint) # queued command barrier; editor has applied fit before coordinate calculation
    $client=[ShotKitHistorySmoke.Rect]::new()
    if(-not [ShotKitHistorySmoke.Native]::GetClientRect($Editor,[ref]$client)){throw 'EDITOR_CLIENT_RECT_FAILED.'}
    $toolbar=@([ShotKitHistorySmoke.Native]::Children($Editor)|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'})
    if($toolbar.Count -ne 1){throw "EDITOR_TOOLBAR_AMBIGUOUS: found $($toolbar.Count)."}
    $tr=[ShotKitHistorySmoke.Rect]::new()
    if(-not [ShotKitHistorySmoke.Native]::GetClientRect($toolbar[0].H,[ref]$tr)){throw 'TOOLBAR_CLIENT_RECT_FAILED.'}
    $availableW=$client.Right; $availableH=$client.Bottom-$tr.Bottom
    $fit=[Math]::Min($availableW/[double]$Width,$availableH/[double]$Height)
    $displayW=[int][Math]::Round($Width*$fit);$displayH=[int][Math]::Round($Height*$fit)
    $viewX=[int][Math]::Floor(($availableW-$displayW)/2.0);$viewY=$tr.Bottom+[int][Math]::Floor(($availableH-$displayH)/2.0)
    $origin=[ShotKitHistorySmoke.Native+Point]::new()
    if(-not [ShotKitHistorySmoke.Native]::ClientToScreen($Editor,[ref]$origin)){throw 'EDITOR_COORDINATE_CONVERSION_FAILED.'}
    $sx=$origin.X+$viewX+[int][Math]::Round($displayW*0.10);$sy=$origin.Y+$viewY+[int][Math]::Round($displayH*0.10)
    $ex=$origin.X+$viewX+[int][Math]::Round($displayW*0.90);$ey=$origin.Y+$viewY+[int][Math]::Round($displayH*0.90)
    if(-not [ShotKitHistorySmoke.Native]::SetForegroundWindow($Editor)){throw 'EDITOR_FOCUS_FAILED.'}
    [void](Wait-For { [ShotKitHistorySmoke.Native]::GetForegroundWindow() -eq $Editor } 'editor foreground')
    if(-not [ShotKitHistorySmoke.Native]::SetCursorPos($sx,$sy)){throw 'CROP_START_POSITION_FAILED.'}
    $cursor=[ShotKitHistorySmoke.Native+Point]::new();[void][ShotKitHistorySmoke.Native]::GetCursorPos([ref]$cursor)
    $under=[ShotKitHistorySmoke.Native]::WindowFromPoint($cursor)
    if([ShotKitHistorySmoke.Native]::OwnerPid($under) -ne $ShotKitProcessId -or [ShotKitHistorySmoke.Native]::GetAncestor($under, 2) -ne $Editor){throw 'INPUT_TARGET_MISMATCH: crop start is not inside the selected editor.'}
    [ShotKitHistorySmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true;Start-Sleep -Milliseconds 100
    [void][ShotKitHistorySmoke.Native]::SetCursorPos($ex,$ey);Start-Sleep -Milliseconds 200
    [ShotKitHistorySmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
}

function Send-Command([int] $Id) {
    if ([ShotKitHistorySmoke.Native]::OwnerPid($editorHandle) -ne $process.Id) { throw 'EDITOR_OWNER_MISMATCH.' }
    if (-not [ShotKitHistorySmoke.Native]::PostMessageW($editorHandle,0x0111,[IntPtr]$Id,[IntPtr]::Zero)) { throw 'EDITOR_COMMAND_FAILED.' }
}
function Read-Fingerprint {
    # Copy is queued after previous commands on the same HWND. Require a fresh,
    # process-owned clipboard result; a stale bitmap cannot satisfy the assertion.
    $before=[ShotKitHistorySmoke.Native]::GetClipboardSequenceNumber()
    Send-Command 0x5111
    Wait-For {
        if ([ShotKitHistorySmoke.Native]::GetClipboardSequenceNumber() -eq $before) { return $null }
        [ShotKitHistorySmoke.Native]::Fingerprint($process.Id)
    } 'fresh owned clipboard image'
}
function Assert-Same([string] $Expected,[string] $Phase) {
    if ((Read-Fingerprint) -ne $Expected) { throw "IMAGE_OR_DIMENSIONS_CHANGED: $Phase" }
}
function Get-Prompt {
    @(Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq 'Rückgängig-Verlauf' })
}
function Answer-Prompt([int] $ButtonId,[int] $ExpectedUndo = 1,[int] $ExpectedRedo = 0,[switch] $ExpectWithoutUndo) {
    $dialogs=@(Wait-For { Get-Prompt } 'history eviction confirmation')
    if ($dialogs.Count -ne 1) { throw 'HISTORY_PROMPT_AMBIGUOUS.' }
    $dialog=$dialogs[0]
    # IDYES=6, IDNO=7; assert this is our editor's modal child, not another window.
    if ([ShotKitHistorySmoke.Native]::OwnerPid($dialog.H) -ne $process.Id -or [ShotKitHistorySmoke.Native]::GetWindow($dialog.H,4) -ne $editorHandle -or [ShotKitHistorySmoke.Native]::IsWindowEnabled($editorHandle)) { throw 'HISTORY_PROMPT_OWNER_OR_MODALITY_MISMATCH.' }
    $children=@([ShotKitHistorySmoke.Native]::Children($dialog.H))
    $expectedText="Für diese Bearbeitung werden $ExpectedUndo ältere Rückgängig-Schritte und $ExpectedRedo Wiederholen-Schritte entfernt. Möchten Sie fortfahren?"
    if ($ExpectWithoutUndo) {
        $expectedText="Das Speicherlimit reicht für diese Bearbeitung nicht aus. Der gesamte Rückgängig- und Wiederholen-Verlauf ($ExpectedUndo und $ExpectedRedo Schritte) wird gelöscht. Diese Bearbeitung kann nicht rückgängig gemacht werden. Möchten Sie fortfahren?"
    }
    if (@($children | Where-Object { $_.ClassName -eq 'Static' -and $_.Title -eq $expectedText }).Count -ne 1) { throw 'HISTORY_PROMPT_EVICTION_COUNT_MISMATCH.' }
    $buttons=@($children | Where-Object { $_.Visible -and $_.ClassName -eq 'Button' -and [ShotKitHistorySmoke.Native]::GetDlgCtrlID($_.H) -eq $ButtonId })
    if ($buttons.Count -ne 1) { throw 'HISTORY_PROMPT_BUTTON_MISSING.' }
    if (-not [ShotKitHistorySmoke.Native]::PostMessageW($buttons[0].H,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)) { throw 'HISTORY_PROMPT_ANSWER_FAILED.' }
    [void](Wait-For { -not [ShotKitHistorySmoke.Native]::IsWindow($dialog.H) -and [ShotKitHistorySmoke.Native]::IsWindowEnabled($editorHandle) } 'history prompt completion')
}
function Stop-OwnedProcess {
    if ($null -eq $process) { return }
    if (-not $process.HasExited) {
        foreach ($window in [ShotKitHistorySmoke.Native]::Windows($process.Id)) { [void][ShotKitHistorySmoke.Native]::PostMessageW($window.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) }
        if (-not $process.WaitForExit(3000)) { $process.Kill(); [void]$process.WaitForExit(5000) }
    }
    $script:processStopped=$process.HasExited
    $process.Dispose(); $script:process=$null
    if (-not $script:processStopped) { throw 'OWNED_PROCESS_CLEANUP_FAILED.' }
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData,$settingsDir -Force | Out-Null
    $settings=@{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'}|ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $settingsDir 'settings.json'),$settings,[Text.UTF8Encoding]::new($false))
    Write-LargeBmp $fixturePath $fixtureWidth $fixtureHeight
    $env:APPDATA=$appData; $env:LOCALAPPDATA=$localAppData
    try { $process=Start-Process -FilePath $ExePath -ArgumentList ('"'+$fixturePath+'"') -WorkingDirectory $root -PassThru; $processStopped=$false }
    finally { $env:APPDATA=$oldAppData; $env:LOCALAPPDATA=$oldLocalAppData }
    $notice=Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*' } | Select-Object -First 1 } 'owned startup notice'
    [void][ShotKitHistorySmoke.Native]::PostMessageW($notice.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)
    [void](Wait-For { -not [ShotKitHistorySmoke.Native]::IsWindow($notice.H) } 'startup notice close')
    $editor=Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title.IndexOf($fixturePath,[StringComparison]::OrdinalIgnoreCase) -ge 0 -and -not $_.Title.StartsWith('* ') } | Select-Object -First 1 } 'loaded fixture editor'
    $editorHandle=$editor.H
    $original=Read-Fingerprint
    if ($WithoutUndo) {
        if (-not $original.StartsWith('8000x5000:')) { throw 'WITHOUT_UNDO_ORIGINAL_DIMENSIONS_INVALID.' }
        # The first 160 MB RGBA snapshot cannot fit the default 128 MiB budget.
        # Even with no old Undo or Redo entries this requires explicit approval.
        Drag-Crop $editorHandle $process.Id 8000 5000
        Answer-Prompt -ButtonId 7 -ExpectedUndo 0 -ExpectedRedo 0 -ExpectWithoutUndo
        Assert-Same $original 'WithoutUndo No preserves all original pixels and dimensions'
        Send-Command 0x5112
        Assert-Same $original 'WithoutUndo No preserves empty Undo history'
        Send-Command 0x5113
        Assert-Same $original 'WithoutUndo No preserves empty Redo history'

        Drag-Crop $editorHandle $process.Id 8000 5000
        Answer-Prompt -ButtonId 6 -ExpectedUndo 0 -ExpectedRedo 0 -ExpectWithoutUndo
        $first=Read-Fingerprint
        $firstSize=($first.Split(':')[0]).Split('x'); $firstWidth=[int]$firstSize[0]; $firstHeight=[int]$firstSize[1]
        if ($firstWidth -lt 6200 -or $firstWidth -gt 6600 -or $firstHeight -lt 3800 -or $firstHeight -gt 4200 -or $first -eq $original) { throw 'WITHOUT_UNDO_CROP_DIMENSIONS_INVALID.' }
        if ([long]$firstWidth*$firstHeight*4 -ge 128*1024*1024) { throw 'WITHOUT_UNDO_NEXT_SNAPSHOT_MUST_FIT_BUDGET.' }
        Send-Command 0x5112
        Assert-Same $first 'WithoutUndo Yes leaves Undo without effect'
        Send-Command 0x5113
        Assert-Same $first 'WithoutUndo Yes leaves Redo without effect'

        # This predecessor is now approximately 102 MB, so a subsequent crop
        # must retain its normal Undo step without a history-loss prompt.
        Drag-Crop $editorHandle $process.Id $firstWidth $firstHeight
        [void](Wait-For { Get-OwnedWindows | Where-Object { $_.H -eq $editorHandle -and $_.Title -like '*Werkzeug Select*' } } 'normal undoable crop after WithoutUndo')
        $second=Read-Fingerprint
        $secondSize=($second.Split(':')[0]).Split('x')
        if ([int]$secondSize[0] -ge $firstWidth -or [int]$secondSize[1] -ge $firstHeight -or $second -eq $first) { throw 'CROP_AFTER_WITHOUT_UNDO_DID_NOT_COMMIT.' }
        if (@(Get-Prompt).Count -ne 0) { throw 'CROP_AFTER_WITHOUT_UNDO_UNEXPECTED_PROMPT.' }
        Send-Command 0x5112
        Assert-Same $first 'normal crop after WithoutUndo restores predecessor through Undo'
        Send-Command 0x5112
        Assert-Same $first 'WithoutUndo original remains absent after later normal Undo'
        Send-Command 0x5113
        Assert-Same $second 'normal crop after WithoutUndo remains redoable'
    } else {
    if (-not $original.StartsWith('6000x4000:')) { throw 'ORIGINAL_DIMENSIONS_INVALID.' }

    # Original RGBA allocation is 96 MB and fits the 128 MiB history budget.
    # The approximately 4800x3200 first crop adds ~61 MB on the next crop,
    # forcing removal of exactly the oldest (original) entry.
    Send-Command 0x510B
    Drag-Crop $editorHandle $process.Id 6000 4000
    [void](Wait-For { Get-OwnedWindows | Where-Object { $_.H -eq $editorHandle -and $_.Title.StartsWith('* ') } } 'first crop commit')
    $first=Read-Fingerprint
    $firstSize=($first.Split(':')[0]).Split('x'); $firstWidth=[int]$firstSize[0]; $firstHeight=[int]$firstSize[1]
    if ($firstWidth -lt 4500 -or $firstWidth -gt 5100 -or $firstHeight -lt 3000 -or $firstHeight -gt 3400) { throw 'FIRST_CROP_DIMENSIONS_INVALID.' }
    if (@(Get-Prompt).Count -ne 0) { throw 'FIRST_CROP_UNEXPECTED_PROMPT.' }
    Send-Command 0x5112
    Assert-Same $original 'first crop Undo restores the original and proves history exists'
    Send-Command 0x5113
    Assert-Same $first 'first crop Redo'

    Drag-Crop $editorHandle $process.Id $firstWidth $firstHeight
    Answer-Prompt 7
    Assert-Same $first 'No retains first crop pixels and dimensions'
    Send-Command 0x5112
    Assert-Same $original 'No retains original Undo history'
    Send-Command 0x5113
    Assert-Same $first 'Redo after declined crop'

    Drag-Crop $editorHandle $process.Id $firstWidth $firstHeight
    Answer-Prompt 6
    $second=Read-Fingerprint
    $secondSize=($second.Split(':')[0]).Split('x')
    if ([int]$secondSize[0] -ge $firstWidth -or [int]$secondSize[1] -ge $firstHeight -or $second -eq $first) { throw 'ACCEPTED_CROP_DID_NOT_COMMIT.' }
    Send-Command 0x5112
    Assert-Same $first 'Yes keeps immediate crop predecessor undoable'
    Send-Command 0x5112
    Assert-Same $first 'Yes evicts oldest original; second Undo is exhausted'
    Send-Command 0x5113
    Assert-Same $second 'accepted crop remains redoable after exhausted Undo'

    # With the oldest original evicted, Undo leaves no older Undo entry but
    # retains the accepted second crop as one Redo entry. A new crop therefore
    # requires confirmation specifically for discarding that Redo branch.
    Send-Command 0x5112
    Assert-Same $first 'Undo before branching crop restores first crop'
    Drag-Crop $editorHandle $process.Id $firstWidth $firstHeight
    Answer-Prompt -ButtonId 7 -ExpectedUndo 0 -ExpectedRedo 1
    Assert-Same $first 'No retains pixels and dimensions when only Redo would be discarded'
    Send-Command 0x5113
    Assert-Same $second 'No preserves the existing Redo branch for actual replay'
    }
} catch { $failure=$_ }
finally {
    $env:APPDATA=$oldAppData; $env:LOCALAPPDATA=$oldLocalAppData
    if ($mouseDown) { try { [ShotKitHistorySmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero) } catch {} }
    if ($null -ne $process) {
        try { [ShotKitHistorySmoke.Native]::ClearOwnedClipboard($process.Id) } catch { $cleanupFailure='OWNED_CLIPBOARD_CLEANUP_FAILED.' }
        try { Stop-OwnedProcess } catch { $cleanupFailure='OWNED_PROCESS_CLEANUP_FAILED.' }
    }
    if ($processStopped -and (Test-Path -LiteralPath $root)) { try { Remove-Item -LiteralPath $root -Recurse -Force } catch { $cleanupFailure='TEMPORARY_FIXTURE_CLEANUP_FAILED.' } }
    elseif (-not $processStopped) { $cleanupFailure='TEMPORARY_FIXTURE_RETAINED: owned process did not stop.' }
}
if ($failure) {
    # Do not emit host/user paths, fixture names from dialogs, or raw exception records.
    $message=$failure.Exception.Message.Replace($root,'[temporary fixture]').Replace($ExePath,'[ShotKit executable]')
    if ($cleanupFailure) { $message+='; '+$cleanupFailure }
    throw "SMOKE_FAILED: $message"
}
if ($cleanupFailure) { throw "CLEANUP_FAILED: $cleanupFailure" }
if ($WithoutUndo) {
    Write-Host 'PASS: WithoutUndo warning with 0/0 counts; No preserves all pixels and dimensions, Yes crops with no effective Undo/Redo, and a later smaller crop is normally undoable and redoable.'
} else {
    Write-Host 'PASS: crop confirmation No preserves pixels, dimensions and Undo history; Yes commits, evicts the oldest original, and keeps the immediate predecessor undoable and the accepted crop redoable; declining a new crop also preserves the existing Redo branch.'
}

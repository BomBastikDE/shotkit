[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'INTERACTIVE_WINDOWS_REQUIRED: ephemeral Windows runner or dedicated local opt-in is required.'
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
namespace ShotKitLargeImportSmoke {
 public sealed class Win { public IntPtr H; public string ClassName,Title; public bool Visible; }
 public sealed class Child { public IntPtr H; public int Id; public string ClassName,Text; public bool Visible; }
 public static class Native {
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

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Large-Import-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'
$settingsDir = Join-Path $appData 'ShotKitRust'
$fixturePath = Join-Path $root 'existing-document.png'
$oversizedPath = Join-Path $root 'oversized-30001x1.png'
$oldAppData = $env:APPDATA; $oldLocalAppData = $env:LOCALAPPDATA
$process = $null; $processStopped = $true; $failure = $null; $cleanupFailure = $null

$inputDesktop = [ShotKitLargeImportSmoke.Native]::InputDesktopName()
if ($inputDesktop -cne 'Default') { throw "INTERACTIVE_DEFAULT_DESKTOP_REQUIRED: current input desktop is '$inputDesktop'." }

function Get-OwnedWindows {
    if ($null -eq $process -or $process.HasExited) { return @() }
    @([ShotKitLargeImportSmoke.Native]::Windows($process.Id))
}
function Wait-For([scriptblock] $Probe, [string] $Description, [int] $Seconds = $TimeoutSeconds) {
    $end = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $end) {
        if ($process.HasExited) { throw "PROCESS_EXITED while waiting for $Description (code $($process.ExitCode))." }
        $value = & $Probe
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    $windows = (Get-OwnedWindows | ForEach-Object { "HWND=$($_.H) class='$($_.ClassName)' title='$($_.Title)' visible=$($_.Visible)" }) -join '; '
    throw "TIMEOUT waiting for $Description; owned windows: $windows"
}
function Send-CtrlC([IntPtr] $Editor) {
    if ([ShotKitLargeImportSmoke.Native]::GetForegroundWindow() -ne $Editor) { throw 'EDITOR_NOT_FOREGROUND: refusing keyboard copy.' }
    [ShotKitLargeImportSmoke.Native]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero)
    try {
        [ShotKitLargeImportSmoke.Native]::keybd_event(0x43, 0, 0, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 80
        [ShotKitLargeImportSmoke.Native]::keybd_event(0x43, 0, 2, [UIntPtr]::Zero)
    } finally { [ShotKitLargeImportSmoke.Native]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero) }
}
function Get-EditorCopy([IntPtr] $Editor, [string] $What) {
    $before = [ShotKitLargeImportSmoke.Native]::ClipboardSequence()
    Send-CtrlC $Editor
    $copy = $null; $sequence = $before; $end = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $end) {
        if ($process.HasExited) { throw "PROCESS_EXITED during $What clipboard export." }
        $sequence = [ShotKitLargeImportSmoke.Native]::ClipboardSequence()
        if ($sequence -ne $before) {
            try { $copy = [ShotKitLargeImportSmoke.Native]::ReadDibV5(); break }
            catch { if ($_.Exception.Message -notlike '*CF_DIBV5 is absent*') { throw } }
        }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq $copy) { throw "${What}_CLIPBOARD_TIMEOUT: editor did not publish CF_DIBV5." }
    if ($sequence -eq $before) { throw "${What}_CLIPBOARD_SEQUENCE_INVALID: clipboard did not change." }
    $header = [BitConverter]::ToUInt32($copy,0); $width = [BitConverter]::ToInt32($copy,4); $height = [BitConverter]::ToInt32($copy,8)
    if ($header -ne 124 -or $width -ne 64 -or [Math]::Abs([long]$height) -ne 48) { throw "${What}_DIMENSION_MISMATCH: CF_DIBV5 header=$header dimensions=${width}x${height}." }
    return [pscustomobject]@{ Bytes = $copy; Hash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($copy)); Sequence = $sequence }
}
function Stop-OwnedProcess {
    if ($null -eq $process) { return }
    if (-not $process.HasExited) {
        foreach ($window in [ShotKitLargeImportSmoke.Native]::Windows($process.Id)) { [void][ShotKitLargeImportSmoke.Native]::PostMessageW($window.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) }
        if (-not $process.WaitForExit(5000)) { $process.Kill(); [void]$process.WaitForExit(5000) }
    }
    $script:processStopped = $process.HasExited
    $process.Dispose(); $script:process = $null
    if (-not $script:processStopped) { throw 'OWNED_PROCESS_CLEANUP_FAILED: the started ShotKit process is still running.' }
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData,$settingsDir -Force | Out-Null
    $settings = @{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'} | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $settingsDir 'settings.json'),$settings,[Text.UTF8Encoding]::new($false))

    $small = [Drawing.Bitmap]::new(64,48,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        for ($y=0; $y -lt 48; $y++) { for ($x=0; $x -lt 64; $x++) { $small.SetPixel($x,$y,[Drawing.Color]::FromArgb(255,($x*31+$y*7)%256,($x*11+$y*29)%256,($x*19+$y*13)%256)) } }
        $small.Save($fixturePath,[Drawing.Imaging.ImageFormat]::Png)
    } finally { $small.Dispose() }

    $large = [Drawing.Bitmap]::new(30001,1,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try { for ($x=0; $x -lt 30001; $x++) { $large.SetPixel($x,0,[Drawing.Color]::FromArgb(255,($x*17)%256,($x*31)%256,($x*7)%256)) }; $large.Save($oversizedPath,[Drawing.Imaging.ImageFormat]::Png) }
    finally { $large.Dispose() }
    $largeCheck = [Drawing.Bitmap]::new($oversizedPath)
    try { if ($largeCheck.Width -ne 30001 -or $largeCheck.Height -ne 1) { throw "FIXTURE_INVALID: generated PNG is $($largeCheck.Width)x$($largeCheck.Height)." } }
    finally { $largeCheck.Dispose() }
    $inputHash = (Get-FileHash -LiteralPath $oversizedPath -Algorithm SHA256).Hash

    $env:APPDATA = $appData; $env:LOCALAPPDATA = $localAppData
    try { $process = Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $fixturePath) -WorkingDirectory $root -PassThru; $processStopped = $false }
    finally { $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData }

    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $process -GetWindows { Get-OwnedWindows }
    $editor = Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*' } | Select-Object -First 1 } 'loaded fixture editor'
    if (-not [ShotKitLargeImportSmoke.Native]::SetForegroundWindow($editor.H)) { throw 'EDITOR_FOCUS_FAILED.' }
    [void](Wait-For { [ShotKitLargeImportSmoke.Native]::GetForegroundWindow() -eq $editor.H } 'editor foreground')
    $originalTitle = $editor.Title
    $beforeCopy = Get-EditorCopy $editor.H 'BEFORE_IMPORT'

    # Use the editor's exact Open toolbar command so this exercises the existing-document path.
    if (-not [ShotKitLargeImportSmoke.Native]::PostMessageW($editor.H,0x0111,[IntPtr]0x5141,[IntPtr]::Zero)) { throw 'OPEN_COMMAND_POST_FAILED.' }
    $openDialog = Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and @([ShotKitLargeImportSmoke.Native]::Children($_.H) | Where-Object { $_.ClassName -eq 'Edit' -and $_.Visible }).Count -gt 0 } | Select-Object -First 1 } 'owned Open dialog'
    if ([ShotKitLargeImportSmoke.Native]::Owner($openDialog.H) -ne [uint32]$process.Id) { throw 'OPEN_DIALOG_OWNER_MISMATCH: refusing to alter an unowned dialog.' }
    $edits = @([ShotKitLargeImportSmoke.Native]::Children($openDialog.H) | Where-Object { $_.Visible -and $_.ClassName -eq 'Edit' })
    if ($edits.Count -ne 1) { throw "OPEN_FILENAME_CONTROL_AMBIGUOUS: expected one visible Edit, found $($edits.Count)." }
    [void][ShotKitLargeImportSmoke.Native]::SetForegroundWindow($openDialog.H)
    [void](Wait-For { [ShotKitLargeImportSmoke.Native]::GetForegroundWindow() -eq $openDialog.H } 'Open dialog foreground')
    if (-not [ShotKitLargeImportSmoke.Native]::SetEditText($edits[0].H,$oversizedPath)) { throw 'OPEN_FILENAME_SET_FAILED.' }
    $openButton = @([ShotKitLargeImportSmoke.Native]::Children($openDialog.H) | Where-Object { $_.Visible -and $_.ClassName -eq 'Button' -and $_.Id -eq 1 -and (($_.Text -replace '&','').Trim() -match '^(Open|Öffnen)$') })
    if ($openButton.Count -ne 1) { throw "OPEN_BUTTON_AMBIGUOUS: expected one owned Open button, found $($openButton.Count)." }
    if (-not [ShotKitLargeImportSmoke.Native]::PostMessageW($openButton[0].H,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)) { throw 'OPEN_BUTTON_CLICK_POST_FAILED.' }

    $errorDialog = Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq 'Bild öffnen' } | Select-Object -First 1 } 'expected image import error' 10
    if ([ShotKitLargeImportSmoke.Native]::Owner($errorDialog.H) -ne [uint32]$process.Id) { throw 'ERROR_DIALOG_OWNER_MISMATCH.' }
    $errorText = [ShotKitLargeImportSmoke.Native]::WindowText($errorDialog.H)
    if ($errorText -notmatch 'maximal 30000 Pixel je Seite') { throw "WRONG_IMPORT_ERROR: expected the 30000-pixel dimension message, got '$errorText'." }
    [void][ShotKitLargeImportSmoke.Native]::PostMessageW($errorDialog.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)
    [void](Wait-For { -not [ShotKitLargeImportSmoke.Native]::IsWindow($errorDialog.H) } 'expected import error close' 8)

    $editor = Wait-For { Get-OwnedWindows | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -eq $originalTitle } | Select-Object -First 1 } 'same original editor after rejected import'
    if (-not [ShotKitLargeImportSmoke.Native]::SetForegroundWindow($editor.H)) { throw 'EDITOR_REFOCUS_FAILED after import error.' }
    [void](Wait-For { [ShotKitLargeImportSmoke.Native]::GetForegroundWindow() -eq $editor.H } 'editor refocus after error')
    $afterCopy = Get-EditorCopy $editor.H 'AFTER_IMPORT'
    if ($beforeCopy.Hash -cne $afterCopy.Hash -or $beforeCopy.Bytes.Length -ne $afterCopy.Bytes.Length) {
        throw "DOCUMENT_CHANGED: CF_DIBV5 SHA256 before=$($beforeCopy.Hash), after=$($afterCopy.Hash); bytes before=$($beforeCopy.Bytes.Length), after=$($afterCopy.Bytes.Length)."
    }
    $afterInputHash = (Get-FileHash -LiteralPath $oversizedPath -Algorithm SHA256).Hash
    if ($inputHash -cne $afterInputHash) { throw "INPUT_MODIFIED: SHA256 before=$inputHash after=$afterInputHash." }
    Write-Host "PASS: importing 30001x1 PNG showed the expected size error; existing 64x48 document CF_DIBV5 export stayed byte-identical (SHA256 $($afterCopy.Hash)); input PNG SHA256 stayed $afterInputHash."
} catch { $failure = $_ }
finally {
    $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData
    if ($null -ne $process) { try { Stop-OwnedProcess } catch { $cleanupFailure = $_.Exception.Message } }
    if ($processStopped -and (Test-Path -LiteralPath $root)) { try { Remove-Item -LiteralPath $root -Recurse -Force } catch { if (-not $cleanupFailure) { $cleanupFailure = $_.Exception.Message } } }
    elseif (-not $processStopped) { $cleanupFailure = "Temporary fixture and app-data retained because the test-owned editor may still be running: $root" }
}
if ($failure) { throw $failure }
if ($cleanupFailure) { throw "CLEANUP_FAILED: $cleanupFailure" }

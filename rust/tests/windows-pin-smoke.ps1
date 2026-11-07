[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $TimeoutSeconds = 30
)

# Issue #99: Strg+T pins the current image (with annotations) as an
# always-on-top tool window; mouse wheel zooms, Esc closes, the editor stays.
# Isolated profile and fixture; interacts only with the owned ShotKit process.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if ($PSVersionTable.PSVersion.Major -lt 7 -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'INTERACTIVE_WINDOWS_REQUIRED: PowerShell 7 on an interactive Windows desktop is required.'
}
if ($env:GITHUB_ACTIONS -cne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') {
    throw 'LOCAL_OPT_IN_REQUIRED: use run-all.ps1 -AllowLocalInteractive in a dedicated test session.'
}
$resolvedExe = (Resolve-Path -LiteralPath $ExePath).Path
if (@(Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($resolvedExe)) -ErrorAction SilentlyContinue).Count -gt 0) {
    throw 'PREEXISTING_PROCESS: refusing to interact with an existing ShotKit instance.'
}
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitPinSmoke {
 public sealed class Win { public IntPtr H; public string ClassName, Title; public bool Visible; }
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
 public static class N {
  delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out Rect r);
  [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref Point p);
  public static int[] ClientSize(IntPtr h) { Rect r; if (!GetClientRect(h, out r)) throw new InvalidOperationException("GetClientRect failed."); return new[] { r.Right - r.Left, r.Bottom - r.Top }; }
  public static int[] ScreenPoint(IntPtr h, int x, int y) { var p = new Point { X = x, Y = y }; if (!ClientToScreen(h, ref p)) throw new InvalidOperationException("ClientToScreen failed."); return new[] { p.X, p.Y }; }
  [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr h, int index);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
  static string Text(IntPtr h, bool cls) { var b=new StringBuilder(512); if(cls)GetClassNameW(h,b,b.Capacity);else GetWindowTextW(h,b,b.Capacity); return b.ToString(); }
  public static Win[] Windows(int pid) { var a=new List<Win>(); EnumProc cb=(h,p)=>{uint id;GetWindowThreadProcessId(h,out id);if(id==(uint)pid)a.Add(new Win{H=h,ClassName=Text(h,true),Title=Text(h,false),Visible=IsWindowVisible(h)});return true;}; EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
 }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Pin-Smoke-' + [Guid]::NewGuid().ToString('N'))
$process = $null; $failure = $null
$fill = [Drawing.Color]::FromArgb(255, 34, 136, 221)

function Wait-Value([scriptblock]$Probe, [string]$What) {
    $end = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $end) {
        if ($process.HasExited) { throw "PROCESS_EXITED while waiting for $What" }
        $v = & $Probe
        if ($v) { return $v }
        Start-Sleep -Milliseconds 100
    }
    throw "TIMEOUT: $What"
}
function Owned { [ShotKitPinSmoke.N]::Windows($process.Id) }
function Pinned { Owned | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitPinnedImage' } | Select-Object -First 1 }
function Client-Size($Window) { return [ShotKitPinSmoke.N]::ClientSize($Window.H) }
function Screen-Pixel($Window, [int]$X, [int]$Y) {
    $p = [ShotKitPinSmoke.N]::ScreenPoint($Window.H, $X, $Y)
    $b = [Drawing.Bitmap]::new(1, 1)
    try { $g = [Drawing.Graphics]::FromImage($b); try { $g.CopyFromScreen($p[0], $p[1], 0, 0, [Drawing.Size]::new(1, 1)) } finally { $g.Dispose() }; return $b.GetPixel(0, 0) }
    finally { $b.Dispose() }
}
function Key([byte]$Code, [bool]$Down) { [ShotKitPinSmoke.N]::keybd_event($Code, 0, $(if ($Down) { 0 } else { 2 }), [UIntPtr]::Zero) }

try {
    $profile = Join-Path $root 'profile'
    $null = New-Item -ItemType Directory -Path (Join-Path $profile 'ShotKitRust')
    $settings = [ordered]@{ version = 1; open_editor = $true; copy_to_clipboard = $false; auto_save = $false; scroll_direction = 'down' }
    [IO.File]::WriteAllText((Join-Path $profile 'ShotKitRust\settings.json'), ($settings | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $fixture = Join-Path $root 'pin-fixture.png'
    $b = [Drawing.Bitmap]::new(120, 80, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try { $g = [Drawing.Graphics]::FromImage($b); try { $g.Clear($fill) } finally { $g.Dispose() }; $b.Save($fixture, [Drawing.Imaging.ImageFormat]::Png) } finally { $b.Dispose() }

    $s = [Diagnostics.ProcessStartInfo]::new($resolvedExe)
    $s.UseShellExecute = $false; $s.WorkingDirectory = $root
    $s.Environment['APPDATA'] = $profile; $s.Environment['LOCALAPPDATA'] = $profile
    $s.ArgumentList.Add($fixture)
    $process = [Diagnostics.Process]::Start($s)
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $process -GetWindows { Owned }
    $editor = Wait-Value { Owned | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title.Contains('pin-fixture.png') } | Select-Object -First 1 } 'editor with the fixture'
    [void](Wait-Value { [ShotKitPinSmoke.N]::SetForegroundWindow($editor.H) -and [ShotKitPinSmoke.N]::GetForegroundWindow() -eq $editor.H } 'editor in the foreground')

    # Strg+T pins the image.
    Key 0x11 $true; Key 0x54 $true; Key 0x54 $false; Key 0x11 $false
    $pin = Wait-Value { Pinned } 'pinned image window'
    $exStyle = [ShotKitPinSmoke.N]::GetWindowLongPtr($pin.H, -20).ToInt64()
    if (($exStyle -band 0x8) -eq 0) { throw "PIN_NOT_TOPMOST: WS_EX_TOPMOST missing (ex style 0x$('{0:X}' -f $exStyle))." }
    if (($exStyle -band 0x80) -eq 0) { throw "PIN_IN_TASKBAR: WS_EX_TOOLWINDOW missing (ex style 0x$('{0:X}' -f $exStyle))." }
    $size = Client-Size $pin
    if ($size[0] -ne 120 -or $size[1] -ne 80) { throw "PIN_SIZE: client $($size[0])x$($size[1]), expected 120x80 at 100 %." }
    $pixel = Wait-Value { $c = Screen-Pixel $pin 60 40; if ($c.ToArgb() -eq $fill.ToArgb()) { $c } } 'pinned pixels on screen'
    Write-Output "EVIDENCE: pinned window topmost, tool window, 120x80, screen pixel $pixel."

    # Mouse wheel up zooms by 1.25 (posted to the pinned window).
    [void][ShotKitPinSmoke.N]::PostMessageW($pin.H, 0x020A, [IntPtr](120 -shl 16), [IntPtr]::Zero)
    [void](Wait-Value { $z = Client-Size $pin; $z[0] -eq 150 -and $z[1] -eq 100 } 'zoomed pinned window 150x100')
    [void][ShotKitPinSmoke.N]::PostMessageW($pin.H, 0x020A, [IntPtr]((-120) -shl 16), [IntPtr]::Zero)
    [void](Wait-Value { $z = Client-Size $pin; $z[0] -eq 120 -and $z[1] -eq 80 } 'pinned window back at 120x80')

    # Esc closes the pinned window; the editor remains.
    [void][ShotKitPinSmoke.N]::PostMessageW($pin.H, 0x0100, [IntPtr]0x1B, [IntPtr]::Zero)
    [void](Wait-Value { -not [ShotKitPinSmoke.N]::IsWindow($pin.H) } 'pinned window closed by Esc')
    if (-not [ShotKitPinSmoke.N]::IsWindow($editor.H)) { throw 'EDITOR_CLOSED_WITH_PIN.' }
    if (@(Owned | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' }).Count -ne 0) { throw 'UNEXPECTED_DIALOG after closing the pin.' }
} catch { $failure = $_ }
finally {
    if ($process -and -not $process.HasExited) { try { $process.Kill(); [void]$process.WaitForExit(5000) } catch {} }
    if ($process) { $process.Dispose() }
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
}
if ($failure) { throw $failure }
Write-Output 'PASS: Strg+T pinned the image as a topmost tool window with exact pixels; wheel zoomed 1.25x and back; Esc closed it and the editor stayed open.'

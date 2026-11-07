[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $TimeoutSeconds = 30
)

# Issue #36: with "Mauszeiger mit aufnehmen" on, an F9 capture with automatic
# PNG output contains the real mouse cursor at its screen position; with the
# option off, the same spot shows only the background.
# Isolated profile, owned white fixture window, owned ShotKit process only.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if ($PSVersionTable.PSVersion.Major -lt 7 -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'INTERACTIVE_WINDOWS_REQUIRED: PowerShell 7 on an interactive Windows desktop is required.'
}
if ($env:GITHUB_ACTIONS -cne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') {
    throw 'LOCAL_OPT_IN_REQUIRED: use run-all.ps1 -AllowLocalInteractive in a dedicated test session.'
}
$resolvedExe = (Resolve-Path -LiteralPath $ExePath).Path
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitCursorCaptureSmoke {
 public sealed class Win { public IntPtr H; public string ClassName, Title; public bool Visible; }
 public static class N {
  delegate bool EnumProc(IntPtr h, IntPtr p);
  [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
  [StructLayout(LayoutKind.Sequential)] public struct CursorInfo { public int Size; public int Flags; public IntPtr Cursor; public Point Position; }
  [DllImport("user32.dll")] static extern bool GetCursorInfo(ref CursorInfo info);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern int ShowCursor(bool show);
  [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  public static bool CursorShowing(out int x, out int y) {
   var info = new CursorInfo { Size = Marshal.SizeOf<CursorInfo>() };
   if (!GetCursorInfo(ref info)) throw new InvalidOperationException("GetCursorInfo failed.");
   x = info.Position.X; y = info.Position.Y;
   return (info.Flags & 1) != 0 && info.Cursor != IntPtr.Zero;
  }
  static string Text(IntPtr h, bool cls) { var b=new StringBuilder(1024); if(cls)GetClassNameW(h,b,b.Capacity);else GetWindowTextW(h,b,b.Capacity); return b.ToString(); }
  public static Win[] Windows(int pid) { var a=new List<Win>(); EnumProc cb=(h,p)=>{uint id;GetWindowThreadProcessId(h,out id);if(id==(uint)pid)a.Add(new Win{H=h,ClassName=Text(h,true),Title=Text(h,false),Visible=IsWindowVisible(h)});return true;}; EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
  public static Win[] Children(IntPtr parent) { var a=new List<Win>(); EnumProc cb=(h,p)=>{a.Add(new Win{H=h,ClassName=Text(h,true),Title=Text(h,false),Visible=IsWindowVisible(h)});return true;}; EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
 }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Cursor-Capture-' + [Guid]::NewGuid().ToString('N'))
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$form = $null; $failure = $null; $cleanupErrors = @()

function Pump { [System.Windows.Forms.Application]::DoEvents() }
function Wait-Value([scriptblock]$Probe, [string]$What) {
    $end = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $end) {
        Pump
        $v = & $Probe
        if ($v) { return $v }
        Start-Sleep -Milliseconds 50
    }
    throw "TIMEOUT: $What"
}
function Windows([Diagnostics.Process]$Process) { if (-not $Process.HasExited) { [ShotKitCursorCaptureSmoke.N]::Windows($Process.Id) } }
function Capture-Png([bool]$WithCursor, [string]$Name) {
    $profile = Join-Path $root "$Name-profile"
    $folder = Join-Path $root "$Name-out"
    $null = New-Item -ItemType Directory -Path (Join-Path $profile 'ShotKitRust'), $folder
    $settings = [ordered]@{
        version = 1; save_folder = $folder; open_editor = $false; copy_to_clipboard = $false
        exclude_from_clipboard_history = $true; auto_save = $true; scroll_direction = 'down'; capture_cursor = $WithCursor
    }
    [IO.File]::WriteAllText((Join-Path $profile 'ShotKitRust\settings.json'), ($settings | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    $s = [Diagnostics.ProcessStartInfo]::new($resolvedExe)
    $s.UseShellExecute = $false; $s.WorkingDirectory = $root
    $s.Environment['APPDATA'] = $profile; $s.Environment['LOCALAPPDATA'] = $profile
    $p = [Diagnostics.Process]::Start($s); $owned.Add($p)
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $p -GetWindows { Windows $p }
    # Keep the white fixture in front and the cursor at its centre.
    $form.Activate(); Pump
    if (-not [ShotKitCursorCaptureSmoke.N]::SetCursorPos($script:cx, $script:cy)) { throw 'SetCursorPos failed.' }
    Start-Sleep -Milliseconds 300; Pump
    [ShotKitCursorCaptureSmoke.N]::keybd_event(0x78, 0, 0, [UIntPtr]::Zero)
    [ShotKitCursorCaptureSmoke.N]::keybd_event(0x78, 0, 2, [UIntPtr]::Zero)
    $notice = Wait-Value { Windows $p | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and (([ShotKitCursorCaptureSmoke.N]::Children($_.H) | ForEach-Object { $_.Title }) -join ' ').Contains('Aufnahme gespeichert') } | Select-Object -First 1 } "saved-capture notice ($Name)"
    $button = [ShotKitCursorCaptureSmoke.N]::Children($notice.H) | Where-Object { $_.Visible -and $_.ClassName -eq 'Button' } | Select-Object -First 1
    [void][ShotKitCursorCaptureSmoke.N]::PostMessageW($button.H, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)
    $png = Wait-Value { Get-ChildItem -LiteralPath $folder -Filter 'ShotKit-*.png' | Select-Object -First 1 } "saved PNG ($Name)"
    $p.Kill(); [void]$p.WaitForExit(5000)
    return $png.FullName
}
# Dark pixels in the 24 × 24 area right/below the hotspot (the arrow points
# up-left from its hotspot). The fixture is pure white, so any dark pixel there
# comes from the captured cursor.
function Dark-Pixels([string]$Path) {
    $b = [Drawing.Bitmap]::new($Path)
    try {
        $count = 0
        for ($y = $script:cy; $y -lt $script:cy + 24; $y++) {
            for ($x = $script:cx; $x -lt $script:cx + 24; $x++) {
                $c = $b.GetPixel($x - $script:originX, $y - $script:originY)
                if ($c.R -lt 128 -and $c.G -lt 128 -and $c.B -lt 128) { $count++ }
            }
        }
        return $count
    } finally { $b.Dispose() }
}

try {
    $null = New-Item -ItemType Directory -Path $root
    $screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
    $script:originX = $screen.X; $script:originY = $screen.Y
    $script:cx = $screen.X + [int]($screen.Width / 2); $script:cy = $screen.Y + [int]($screen.Height / 2)
    $form = [System.Windows.Forms.Form]::new()
    $form.FormBorderStyle = 'None'; $form.BackColor = [Drawing.Color]::White; $form.TopMost = $true
    $form.StartPosition = 'Manual'; $form.Bounds = $screen; $form.ShowInTaskbar = $false
    $form.Cursor = [System.Windows.Forms.Cursors]::Arrow
    $form.Show(); $form.Activate(); Pump
    # Without a mouse device Windows starts the display counter at -1 and hides
    # the cursor. The counter belongs to the thread owning the window under the
    # cursor, so raising it for the fixture thread makes the cursor visible over
    # the fixture only. This changes the test setup, not ShotKit.
    $counter = -1
    for ($i = 0; $i -lt 8 -and $counter -lt 0; $i++) { $counter = [ShotKitCursorCaptureSmoke.N]::ShowCursor($true) }
    Write-Output "EVIDENCE: fixture thread cursor display counter=$counter."
    if (-not [ShotKitCursorCaptureSmoke.N]::SetCursorPos($script:cx, $script:cy)) { throw 'SetCursorPos failed.' }
    Start-Sleep -Milliseconds 300; Pump
    $x = 0; $y = 0
    if (-not [ShotKitCursorCaptureSmoke.N]::CursorShowing([ref]$x, [ref]$y)) {
        throw 'NO_VISIBLE_CURSOR: Windows reports no visible mouse cursor on this desktop; the capture cannot contain one.'
    }

    $with = Dark-Pixels (Capture-Png $true 'with')
    $without = Dark-Pixels (Capture-Png $false 'without')
    Write-Output "EVIDENCE: dark pixels at the cursor position: with option=$with, without option=$without."
    if ($with -lt 10) { throw "CURSOR_MISSING: only $with dark pixels at the hotspot with the option on." }
    if ($without -ne 0) { throw "CURSOR_WITHOUT_OPTION: $without dark pixels at the hotspot with the option off." }
} catch { $failure = $_ }
finally {
    foreach ($p in $owned) {
        try { if (-not $p.HasExited) { $p.Kill() }; [void]$p.WaitForExit(5000) } catch { $cleanupErrors += "Process $($p.Id): $_" } finally { $p.Dispose() }
    }
    if ($form) { try { $form.Close(); $form.Dispose() } catch { $cleanupErrors += "Form: $_" } }
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue }
}
if ($cleanupErrors.Count -gt 0) { throw "CLEANUP_FAILED: $($cleanupErrors -join '; '). Test failure: $failure" }
if ($failure) { throw $failure }
Write-Output 'PASS: with "Mauszeiger mit aufnehmen" the automatic PNG contains the real cursor at its position; without it the spot stays white.'

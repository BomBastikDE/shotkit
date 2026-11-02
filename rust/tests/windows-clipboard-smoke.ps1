[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string] $ExePath,
    [ValidateRange(10, 120)]
    [int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') {
    throw 'This smoke test is restricted to an ephemeral GitHub Actions Windows runner because it uses the interactive system clipboard.'
}
if (-not [Environment]::UserInteractive) { throw 'NO_INTERACTIVE_DESKTOP: the Windows runner is not interactive.' }
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw "ShotKit EXE not found: $ExePath" }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw "PREEXISTING_PROCESS: refusing to interact with a process named '$imageName'."
}

if (-not ('ShotKitClipboardSmoke.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitClipboardSmoke {
  public struct Rect { public int Left; public int Top; public int Right; public int Bottom; }
  public struct Point { public int X; public int Y; }
  public sealed class Window { public IntPtr Handle; public string ClassName; public string Title; public bool Visible; }
  public static class Native {
    [StructLayout(LayoutKind.Sequential)] struct GuiThreadInfo { public uint cbSize; public uint flags; public IntPtr hwndActive, hwndFocus, hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret; public Rect rcCaret; }
    private delegate bool EnumProc(IntPtr hwnd, IntPtr data);
    private delegate bool ChildEnumProc(IntPtr hwnd, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr data);
    [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, ChildEnumProc cb, IntPtr data);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr hwnd, StringBuilder text, int max);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", SetLastError=true)] static extern bool GetGUIThreadInfo(uint threadId, ref GuiThreadInfo info);
    [DllImport("user32.dll")] static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll")] static extern short GetKeyState(int key);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr hwnd, ref Point point);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern uint RegisterClipboardFormatW(string name);
    [DllImport("user32.dll")] public static extern bool OpenClipboard(IntPtr owner);
    [DllImport("user32.dll")] public static extern bool CloseClipboard();
    [DllImport("user32.dll")] public static extern IntPtr GetClipboardData(uint format);
    [DllImport("kernel32.dll")] public static extern UIntPtr GlobalSize(IntPtr memory);
    [DllImport("kernel32.dll")] public static extern IntPtr GlobalLock(IntPtr memory);
    [DllImport("kernel32.dll")] public static extern bool GlobalUnlock(IntPtr memory);
    [DllImport("user32.dll")] static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll")] static extern bool IsClipboardFormatAvailable(uint format);
    [DllImport("user32.dll")] static extern uint EnumClipboardFormats(uint format);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClipboardFormatNameW(uint format, StringBuilder name, int max);
    [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    public static uint ClipboardSequence() { return GetClipboardSequenceNumber(); }
    public static string ControlKeyState() { return "GetAsyncKeyState="+GetAsyncKeyState(0x11)+", GetKeyState="+GetKeyState(0x11); }
    static string DescribeWindow(IntPtr hwnd) { return "HWND="+hwnd+" PID="+OwnerPid(hwnd)+" class='"+GetText(hwnd,false)+"' title='"+GetText(hwnd,true)+"'"; }
    public static string FocusReport(IntPtr target) {
      uint pid, tid=GetWindowThreadProcessId(target,out pid);
      GuiThreadInfo info=new GuiThreadInfo(); info.cbSize=(uint)Marshal.SizeOf(typeof(GuiThreadInfo));
      if(!GetGUIThreadInfo(tid,ref info)) return "target="+DescribeWindow(target)+" GetGUIThreadInfo failed error="+Marshal.GetLastWin32Error();
      return "target="+DescribeWindow(target)+" thread="+tid+" active=["+DescribeWindow(info.hwndActive)+"] keyboardFocus=["+DescribeWindow(info.hwndFocus)+"] capture=["+DescribeWindow(info.hwndCapture)+"]";
    }
    public static string ClipboardFormats() {
      uint png=RegisterClipboardFormatW("PNG"); var found=new List<string>();
      if(!OpenClipboard(IntPtr.Zero)) return "OpenClipboard failed; known-image-formats="+(png!=0 && IsClipboardFormatAvailable(png) ? "PNG " : "")+(IsClipboardFormatAvailable(17) ? "CF_DIBV5 " : "")+(IsClipboardFormatAvailable(8) ? "CF_DIB " : "")+(IsClipboardFormatAvailable(2) ? "CF_BITMAP" : "");
      try {
        uint format=0;
        while((format=EnumClipboardFormats(format))!=0) {
          string name;
          if(format==png) name="PNG";
          else if(format==17) name="CF_DIBV5";
          else if(format==8) name="CF_DIB";
          else if(format==2) name="CF_BITMAP";
          else if(format==1) name="CF_TEXT";
          else if(format==13) name="CF_UNICODETEXT";
          else { var b=new StringBuilder(256); int n=GetClipboardFormatNameW(format,b,b.Capacity); name=n>0 ? b.ToString() : "format"; }
          found.Add(name+"("+format+")");
        }
        return found.Count==0 ? "no formats enumerated" : String.Join(",",found.ToArray());
      } finally { CloseClipboard(); }
    }
    static string GetText(IntPtr h, bool title) { var b = new StringBuilder(512); if (title) GetWindowTextW(h,b,b.Capacity); else GetClassNameW(h,b,b.Capacity); return b.ToString(); }
    public static Window[] WindowsForProcess(int id) {
      var result = new List<Window>();
      EnumProc cb = delegate(IntPtr h, IntPtr d) { uint pid; GetWindowThreadProcessId(h,out pid); if(pid==(uint)id) result.Add(new Window { Handle=h, ClassName=GetText(h,false), Title=GetText(h,true), Visible=IsWindowVisible(h) }); return true; };
      EnumWindows(cb,IntPtr.Zero); GC.KeepAlive(cb); return result.ToArray();
    }
    public static string DescribeProcessWindows(int id) {
      var rows=new List<string>();
      EnumProc cb=delegate(IntPtr h,IntPtr d) {
        uint pid; GetWindowThreadProcessId(h,out pid); if(pid!=(uint)id) return true;
        var row=new StringBuilder(DescribeWindow(h)+" visible="+IsWindowVisible(h));
        if(String.Equals(GetText(h,false),"#32770",StringComparison.OrdinalIgnoreCase)) {
          var texts=new List<string>();
          ChildEnumProc child=delegate(IntPtr c,IntPtr ignored) { string t=GetText(c,true); if(!String.IsNullOrWhiteSpace(t)) texts.Add(t); return true; };
          EnumChildWindows(h,child,IntPtr.Zero); GC.KeepAlive(child);
          row.Append(" dialogText=[").Append(String.Join(" | ",texts.ToArray())).Append("]");
        }
        rows.Add(row.ToString()); return true;
      };
      EnumWindows(cb,IntPtr.Zero); GC.KeepAlive(cb);
      return rows.Count==0 ? "no top-level windows" : String.Join(" ; ",rows.ToArray());
    }
    public static uint OwnerPid(IntPtr h) { uint p; GetWindowThreadProcessId(h,out p); return p; }
    public static string WindowTitle(IntPtr h) { return GetText(h,true); }
    public static void ClickCanvas(IntPtr hwnd, int expectedPid) {
      if(OwnerPid(hwnd)!=(uint)expectedPid) throw new InvalidOperationException("Canvas HWND is not owned by the test process.");
      ShowWindow(hwnd,3); // SW_MAXIMIZE gives a stable, large canvas hit target.
      System.Threading.Thread.Sleep(200);
      Rect r; if(!GetClientRect(hwnd,out r)) throw new InvalidOperationException("GetClientRect failed for owned editor.");
      int width=r.Right-r.Left, height=r.Bottom-r.Top;
      if(width<400 || height<300) throw new InvalidOperationException("Owned editor client is too small for a reliable canvas click.");
      Point p=new Point { X=width/2, Y=(height*3)/4 };
      if(!ClientToScreen(hwnd,ref p) || !SetCursorPos(p.X,p.Y)) throw new InvalidOperationException("Could not position pointer over editor canvas.");
      if(WindowFromPoint(p)!=hwnd) throw new InvalidOperationException("Canvas hit target is not the owned editor HWND.");
      mouse_event(0x0002,0,0,0,UIntPtr.Zero);
      try { System.Threading.Thread.Sleep(60); }
      finally { mouse_event(0x0004,0,0,0,UIntPtr.Zero); }
      if(GetForegroundWindow()!=hwnd) throw new InvalidOperationException("Canvas click did not leave the owned editor in the foreground.");
    }
    public static byte[] ReadClipboardPng() {
      uint format=RegisterClipboardFormatW("PNG"); if(format==0) throw new InvalidOperationException("Could not register PNG clipboard format.");
      if(!OpenClipboard(IntPtr.Zero)) throw new InvalidOperationException("OpenClipboard failed.");
      IntPtr memory=IntPtr.Zero, ptr=IntPtr.Zero;
      try {
        memory=GetClipboardData(format); if(memory==IntPtr.Zero) throw new InvalidOperationException("PNG clipboard format is absent; alpha cannot be proven by an opaque fallback.");
        ulong size=GlobalSize(memory).ToUInt64(); if(size<8 || size>67108864) throw new InvalidOperationException("PNG clipboard block size is invalid or exceeds 64 MiB.");
        ptr=GlobalLock(memory); if(ptr==IntPtr.Zero) throw new InvalidOperationException("GlobalLock failed for clipboard PNG.");
        byte[] bytes=new byte[(int)size]; Marshal.Copy(ptr,bytes,0,bytes.Length); return bytes;
      } finally { if(ptr!=IntPtr.Zero) GlobalUnlock(memory); if(!CloseClipboard()) throw new InvalidOperationException("CloseClipboard failed."); }
    }
  }
}
'@
}
Add-Type -AssemblyName System.Drawing

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Clipboard-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'
$localAppData = Join-Path $root 'localappdata'
$inputPath = Join-Path $root 'transparent-input.png'
$process = $null
$processStartTicks = $null
$processStopped = $true
$oldAppData = $env:APPDATA
$oldLocalAppData = $env:LOCALAPPDATA
$failure = $null
$cleanupFailures = @()

try {
    New-Item -ItemType Directory -Path $root, $appData, $localAppData -Force | Out-Null
    $fixture = [Drawing.Bitmap]::new(32, 24, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $g = [Drawing.Graphics]::FromImage($fixture)
        try { $g.Clear([Drawing.Color]::FromArgb(0, 0, 0, 0)) } finally { $g.Dispose() }
        $fixture.SetPixel(7, 9, [Drawing.Color]::FromArgb(73, 19, 137, 211))
        $fixture.SetPixel(22, 16, [Drawing.Color]::FromArgb(255, 241, 37, 83))
        $fixture.Save($inputPath, [Drawing.Imaging.ImageFormat]::Png)
    } finally { $fixture.Dispose() }
    $source = [Drawing.Bitmap]::new($inputPath)
    $expectedPixels = [object[]]::new(32 * 24)
    try {
        $semi = $source.GetPixel(7,9); $opaque = $source.GetPixel(22,16); $clear = $source.GetPixel(2,2)
        if ($source.Width -ne 32 -or $source.Height -ne 24 -or $semi.A -ne 73 -or $semi.R -ne 19 -or $semi.G -ne 137 -or $semi.B -ne 211 -or $opaque.A -ne 255 -or $clear.A -ne 0) { throw 'FIXTURE_INVALID: generated PNG failed its alpha/color precondition.' }
        for ($y = 0; $y -lt 24; $y++) { for ($x = 0; $x -lt 32; $x++) { $expectedPixels[$y * 32 + $x] = $source.GetPixel($x, $y) } }
    } finally { $source.Dispose() }

    try {
        $env:APPDATA = $appData; $env:LOCALAPPDATA = $localAppData
        $process = Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $inputPath) -WorkingDirectory $root -PassThru
        $processStopped = $false
        $processStartTicks = $process.StartTime.ToUniversalTime().Ticks
    } finally { $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData }

    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $main = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($process.HasExited) { throw "PROCESS_EXITED during startup (code $($process.ExitCode))." }
        $main = [ShotKitClipboardSmoke.Native]::WindowsForProcess($process.Id) | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Strg+S speichern*' } | Select-Object -First 1
        if ($null -ne $main) { break }
        $notice = [ShotKitClipboardSmoke.Native]::WindowsForProcess($process.Id) | Where-Object { $_.Visible -and $_.Title -like '*Prototyp*' } | Select-Object -First 1
        if ($null -ne $notice) {
            # Dismiss only the known startup notice owned by this test process.
            if ([ShotKitClipboardSmoke.Native]::OwnerPid($notice.Handle) -ne [uint32]$process.Id) { throw 'NOTICE_OWNER_MISMATCH: refusing to click an unowned window.' }
            [void][ShotKitClipboardSmoke.Native]::SetForegroundWindow($notice.Handle)
            [void][ShotKitClipboardSmoke.Native]::SendMessageW($notice.Handle, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) # WM_CLOSE
        }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq $main) { throw 'EDITOR_TIMEOUT: owned image editor window not found.' }
    if (-not [ShotKitClipboardSmoke.Native]::SetForegroundWindow($main.Handle)) { throw 'EDITOR_FOCUS_FAILED: could not foreground owned editor.' }
    [ShotKitClipboardSmoke.Native]::ClickCanvas($main.Handle, $process.Id)
    Start-Sleep -Milliseconds 100
    if ([ShotKitClipboardSmoke.Native]::GetForegroundWindow() -ne $main.Handle) { throw 'EDITOR_FOCUS_FAILED: foreground window is not the owned editor.' }
    $sequenceBeforeCopy = [ShotKitClipboardSmoke.Native]::ClipboardSequence()
    $formatsBeforeCopy = [ShotKitClipboardSmoke.Native]::ClipboardFormats()
    $focusBeforeCopy = [ShotKitClipboardSmoke.Native]::FocusReport($main.Handle)
    # Send an actual Ctrl+C keystroke to the foreground editor window.
    [ShotKitClipboardSmoke.Native]::keybd_event(0x11,0,0,[UIntPtr]::Zero)
    try {
        Start-Sleep -Milliseconds 30
        $ctrlStateDuringCopy = [ShotKitClipboardSmoke.Native]::ControlKeyState()
        [ShotKitClipboardSmoke.Native]::keybd_event(0x43,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 80; [ShotKitClipboardSmoke.Native]::keybd_event(0x43,0,2,[UIntPtr]::Zero)
    }
    finally { [ShotKitClipboardSmoke.Native]::keybd_event(0x11,0,2,[UIntPtr]::Zero) }
    $ctrlStateAfterCopy = [ShotKitClipboardSmoke.Native]::ControlKeyState()

    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    $png = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($process.HasExited) { throw 'PROCESS_EXITED after Ctrl+C.' }
        $sequenceAfterCopy = [ShotKitClipboardSmoke.Native]::ClipboardSequence()
        if ($sequenceAfterCopy -eq $sequenceBeforeCopy) { Start-Sleep -Milliseconds 100; continue }
        try { $png = [ShotKitClipboardSmoke.Native]::ReadClipboardPng(); break } catch {
            if ($_.Exception.Message -notlike '*PNG clipboard format is absent*') { throw }
        }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq $png) {
        $sequenceAtTimeout = [ShotKitClipboardSmoke.Native]::ClipboardSequence()
        $formatsAtTimeout = [ShotKitClipboardSmoke.Native]::ClipboardFormats()
        $focusAtTimeout = [ShotKitClipboardSmoke.Native]::FocusReport($main.Handle)
        $ownedWindowsAtTimeout = [ShotKitClipboardSmoke.Native]::DescribeProcessWindows($process.Id)
        throw "CLIPBOARD_PNG_TIMEOUT: Ctrl+C did not publish PNG clipboard image data. sequenceBefore=$sequenceBeforeCopy sequenceAfter=$sequenceAtTimeout formatsBefore=[$formatsBeforeCopy] formatsAfter=[$formatsAtTimeout] focusBefore=[$focusBeforeCopy] focusAtTimeout=[$focusAtTimeout] ctrlStateDuring=[$ctrlStateDuringCopy] ctrlStateAfter=[$ctrlStateAfterCopy] ownedWindowsAndDialogs=[$ownedWindowsAtTimeout]"
    }
    if ($sequenceAfterCopy -eq $sequenceBeforeCopy) { throw 'CLIPBOARD_SEQUENCE_INVALID: the clipboard did not change after Ctrl+C.' }
    $stream = [IO.MemoryStream]::new([byte[]]$png)
    try {
        $image = [Drawing.Bitmap]::new($stream)
        try {
            $transparent = $image.GetPixel(2,2); $semi = $image.GetPixel(7,9); $opaque = $image.GetPixel(22,16)
            if ($image.Width -ne 32 -or $image.Height -ne 24) { throw "CLIPBOARD_DIMENSIONS_MISMATCH: got $($image.Width)x$($image.Height), expected 32x24." }
            if ($transparent.A -ne 0) { throw "ALPHA_FAILURE: transparent fixture pixel came back with alpha $($transparent.A)." }
            if ($semi.A -ne 73 -or $semi.R -ne 19 -or $semi.G -ne 137 -or $semi.B -ne 211) { throw "ALPHA_COLOR_FAILURE: semitransparent pixel expected A73 R19 G137 B211; got A$($semi.A) R$($semi.R) G$($semi.G) B$($semi.B)." }
            if ($opaque.A -ne 255 -or $opaque.R -ne 241 -or $opaque.G -ne 37 -or $opaque.B -ne 83) { throw "OPAQUE_COLOR_FAILURE: expected A255 R241 G37 B83; got A$($opaque.A) R$($opaque.R) G$($opaque.G) B$($opaque.B)." }
        } finally { $image.Dispose() }
    } finally { $stream.Dispose() }
    # Exercise ShotKit's actual clipboard import path (Ctrl+V), which invokes
    # clipboard::read_image and opens a second editor for the copied image.
    if (-not [ShotKitClipboardSmoke.Native]::SetForegroundWindow($main.Handle)) { throw 'PASTE_FOCUS_FAILED: could not refocus the owned source editor.' }
    [ShotKitClipboardSmoke.Native]::ClickCanvas($main.Handle, $process.Id)
    [ShotKitClipboardSmoke.Native]::keybd_event(0x11,0,0,[UIntPtr]::Zero)
    try { [ShotKitClipboardSmoke.Native]::keybd_event(0x56,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 80; [ShotKitClipboardSmoke.Native]::keybd_event(0x56,0,2,[UIntPtr]::Zero) }
    finally { [ShotKitClipboardSmoke.Native]::keybd_event(0x11,0,2,[UIntPtr]::Zero) }
    $pasteDeadline = [DateTime]::UtcNow.AddSeconds(5)
    $pasted = $null
    while ([DateTime]::UtcNow -lt $pasteDeadline) {
        if ($process.HasExited) { throw 'PROCESS_EXITED after Ctrl+V.' }
        $pasted = [ShotKitClipboardSmoke.Native]::WindowsForProcess($process.Id) | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title -like '*Neue Aufnahme*' } | Select-Object -First 1
        if ($null -ne $pasted) { break }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq $pasted) { throw 'CLIPBOARD_IMPORT_FAILURE: Ctrl+V did not open the copied clipboard image in a new owned editor.' }
    if ([ShotKitClipboardSmoke.Native]::OwnerPid($pasted.Handle) -ne [uint32]$process.Id) { throw 'PASTED_WINDOW_OWNER_MISMATCH: imported image editor is not owned by the test process.' }
    if (-not [ShotKitClipboardSmoke.Native]::SetForegroundWindow($pasted.Handle)) { throw 'PASTED_FOCUS_FAILED: could not focus the imported editor.' }
    [ShotKitClipboardSmoke.Native]::ClickCanvas($pasted.Handle, $process.Id)
    $focusDeadline = [DateTime]::UtcNow.AddSeconds(3)
    while ([DateTime]::UtcNow -lt $focusDeadline -and [ShotKitClipboardSmoke.Native]::GetForegroundWindow() -ne $pasted.Handle) { Start-Sleep -Milliseconds 50 }
    if ([ShotKitClipboardSmoke.Native]::GetForegroundWindow() -ne $pasted.Handle) { throw 'PASTED_FOCUS_FAILED: foreground window is not the imported editor.' }

    # Prove the imported editor itself contains the same image: copy from that
    # window, require another clipboard sequence change, then compare every RGBA pixel.
    $sequenceBeforeImportedCopy = [ShotKitClipboardSmoke.Native]::ClipboardSequence()
    [ShotKitClipboardSmoke.Native]::keybd_event(0x11,0,0,[UIntPtr]::Zero)
    try { [ShotKitClipboardSmoke.Native]::keybd_event(0x43,0,0,[UIntPtr]::Zero); Start-Sleep -Milliseconds 80; [ShotKitClipboardSmoke.Native]::keybd_event(0x43,0,2,[UIntPtr]::Zero) }
    finally { [ShotKitClipboardSmoke.Native]::keybd_event(0x11,0,2,[UIntPtr]::Zero) }
    $importedPng = $null
    $importedCopyDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([DateTime]::UtcNow -lt $importedCopyDeadline) {
        if ($process.HasExited) { throw 'PROCESS_EXITED after Ctrl+C from imported editor.' }
        $importedSequence = [ShotKitClipboardSmoke.Native]::ClipboardSequence()
        if ($importedSequence -eq $sequenceBeforeImportedCopy) { Start-Sleep -Milliseconds 100; continue }
        try { $importedPng = [ShotKitClipboardSmoke.Native]::ReadClipboardPng(); break } catch {
            if ($_.Exception.Message -notlike '*PNG clipboard format is absent*') { throw }
        }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq $importedPng) { throw 'IMPORTED_CLIPBOARD_PNG_TIMEOUT: Ctrl+C from imported editor did not publish PNG data.' }
    if ($importedSequence -eq $sequenceBeforeImportedCopy) { throw 'IMPORTED_CLIPBOARD_SEQUENCE_INVALID: clipboard did not change after copying from imported editor.' }
    $importedStream = [IO.MemoryStream]::new([byte[]]$importedPng)
    try {
        $importedImage = [Drawing.Bitmap]::new($importedStream)
        try {
            if ($importedImage.Width -ne 32 -or $importedImage.Height -ne 24) { throw "IMPORTED_DIMENSIONS_MISMATCH: got $($importedImage.Width)x$($importedImage.Height), expected 32x24." }
            for ($y = 0; $y -lt 24; $y++) {
                for ($x = 0; $x -lt 32; $x++) {
                    $expected = $expectedPixels[$y * 32 + $x]
                    $actual = $importedImage.GetPixel($x, $y)
                    if ($actual.A -ne $expected.A -or $actual.R -ne $expected.R -or $actual.G -ne $expected.G -or $actual.B -ne $expected.B) {
                        throw "IMPORTED_PIXEL_MISMATCH at ($x,$y): expected A$($expected.A) R$($expected.R) G$($expected.G) B$($expected.B), got A$($actual.A) R$($actual.R) G$($actual.G) B$($actual.B)."
                    }
                }
            }
        } finally { $importedImage.Dispose() }
    } finally { $importedStream.Dispose() }
} catch { $failure = $_ }
finally {
    $env:APPDATA = $oldAppData; $env:LOCALAPPDATA = $oldLocalAppData
    if ($null -ne $process) {
        try {
            if (-not $process.HasExited) {
                $actualPath = $process.MainModule.FileName
                $actualTicks = $process.StartTime.ToUniversalTime().Ticks
                if (-not [StringComparer]::OrdinalIgnoreCase.Equals([IO.Path]::GetFullPath($actualPath), [IO.Path]::GetFullPath($ExePath)) -or $actualTicks -ne $processStartTicks) { throw 'Refusing cleanup: process identity no longer matches test-owned executable and start time.' }
                $process.Kill()
                if (-not $process.WaitForExit(5000) -and -not $process.HasExited) { throw 'Test-owned editor did not exit after termination.' }
            }
            $processStopped = $process.HasExited
        } catch { $cleanupFailures += "Editor cleanup failed: $($_.Exception.Message)" }
        finally { $process.Dispose() }
    }
    if ($processStopped -and (Test-Path -LiteralPath $root)) {
        try { Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop } catch { $cleanupFailures += "Temporary directory cleanup failed: $($_.Exception.Message)" }
    } elseif (Test-Path -LiteralPath $root) { $cleanupFailures += "Temporary data retained because the owned editor may still be running: $root" }
}
if ($null -ne $failure) { throw $failure }
if ($cleanupFailures.Count -gt 0) { throw ('CLEANUP_FAILURE: ' + ($cleanupFailures -join '; ')) }
Write-Output 'PASS: Ctrl+C published PNG data with verified alpha/color pixels; Ctrl+V imported it into a second owned editor, and Ctrl+C from that editor produced a PNG whose every RGBA pixel matched the fixture.'

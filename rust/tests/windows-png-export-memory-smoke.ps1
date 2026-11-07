[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(30, 300)][int] $TimeoutSeconds = 90,
    [ValidateRange(1, 100)][int] $SampleIntervalMs = 10,
    [ValidateSet(1, 5)][int] $DocumentCount = 1,
    [switch] $ScrollHeight,
    [switch] $LargeAnnotated
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'PowerShell 7 (pwsh) is required.' }
if (($env:GITHUB_ACTIONS -ne 'true' -and $env:SHOTKIT_LOCAL_SMOKE_TEST -ne '1') -or $env:OS -ne 'Windows_NT' -or -not [Environment]::UserInteractive) {
    throw 'INTERACTIVE_WINDOWS_REQUIRED: an ephemeral Windows runner or dedicated local opt-in is required.'
}
if (-not (Test-Path -LiteralPath $ExePath -PathType Leaf)) { throw 'SHOTKIT_EXE_NOT_FOUND.' }
$ExePath = (Resolve-Path -LiteralPath $ExePath).Path
function Assert-NoExistingShotKitInstance {
    [Threading.Mutex] $existing = $null
    $exists = $false
    try {
        $exists = [Threading.Mutex]::TryOpenExisting('Local\ShotKitRustPrototype.RustOnly.SingleInstance.v1',[ref]$existing)
    }
    catch [UnauthorizedAccessException] { throw 'SINGLE_INSTANCE_MUTEX_ACCESS_DENIED: refusing to start when the existing instance mutex cannot be inspected.' }
    catch { throw 'SINGLE_INSTANCE_MUTEX_CHECK_FAILED: refusing to start when the existing instance mutex cannot be inspected.' }
    finally { if ($null -ne $existing) { $existing.Dispose() } }
    if ($exists) { throw 'PREEXISTING_SINGLE_INSTANCE_MUTEX: refusing to risk contacting a running ShotKit instance, including a renamed executable.' }
}
# Name checks alone cannot detect an already-running renamed ShotKit executable.
# This only opens/closes a handle; it never acquires, releases, or creates the mutex.
Assert-NoExistingShotKitInstance
foreach ($name in (@([IO.Path]::GetFileNameWithoutExtension($ExePath), 'ShotKit', 'shotkit-rust-prototype') | Select-Object -Unique)) {
    if (@(Get-Process -Name $name -ErrorAction SilentlyContinue).Count -gt 0) {
        throw "PREEXISTING_SHOTKIT_PROCESS: '$name' is already running; refusing single-instance or GUI interference."
    }
}

# ScrollHeight exercises the export of a synthetic 30,000-row result, not acquisition.
if ($ScrollHeight -and $LargeAnnotated) { throw 'INCOMPATIBLE_FIXTURES.' }
if ($LargeAnnotated -and $DocumentCount -ne 1) { throw 'ANNOTATED_FIXTURE_REQUIRES_ONE_DOCUMENT.' }
$width = if ($LargeAnnotated) { 8192 } elseif ($ScrollHeight) { 256 } else { 6000 }
$height = if ($LargeAnnotated) { 8192 } elseif ($ScrollHeight) { 30000 } else { 4000 }
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
namespace ShotKitPngExportMemorySmoke {
 public struct Rect { public int Left,Top,Right,Bottom; }
 public struct Point { public int X,Y; }
 public sealed class Win { public IntPtr H; public int Id, Pid; public string ClassName, Title; public bool Visible, Enabled; }
 [StructLayout(LayoutKind.Sequential)] struct CountersEx {
  public uint cb, PageFaultCount;
  public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage;
  public UIntPtr QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage, PrivateUsage;
 }
 public sealed class Stat { public ulong PrivateBytesMax, PrivateBytesFirst, PrivateBytesLast; public long Samples; }
 public static class Native {
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int cmd);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f,uint x,uint y,uint d,UIntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte k,byte s,uint f,UIntPtr e);
  delegate bool EnumProc(IntPtr h, IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr d);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr h, EnumProc cb, IntPtr d);
  [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, string text);
  [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern IntPtr ReadTimeoutText(IntPtr h, uint m, IntPtr w, System.Text.StringBuilder text, uint flags, uint timeout, out IntPtr result);
  [DllImport("psapi.dll", SetLastError=true)] static extern bool GetProcessMemoryInfo(IntPtr process, ref CountersEx counters, uint size);
  static string Read(IntPtr h, bool cls) {
   // Match the proven class-name buffer in windows-large-image-memory-smoke.ps1.
   // Keep long path-bearing window titles in a separate, larger buffer.
   if(cls) {
    var name=new System.Text.StringBuilder(512);int length=GetClassNameW(h,name,name.Capacity);
    if(length==0)return "[GetClassNameW failed; return=0; win32_error="+Marshal.GetLastWin32Error()+"]";
    return name.ToString();
   }
   var title=new System.Text.StringBuilder(32768);GetWindowTextW(h,title,title.Capacity);return title.ToString();
  }
  public static int OwnerPid(IntPtr h) { uint p; GetWindowThreadProcessId(h,out p);return (int)p; }
  static Win Describe(IntPtr h) { return new Win{H=h,Id=GetDlgCtrlID(h),Pid=OwnerPid(h),ClassName=Read(h,true),Title=Read(h,false),Visible=IsWindowVisible(h),Enabled=IsWindowEnabled(h)}; }
  public static Win[] Windows(int pid) {
   var a=new List<Win>();EnumProc cb=(h,d)=>{if(OwnerPid(h)==pid)a.Add(Describe(h));return true;};
   EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();
  }
  public static Win[] Children(IntPtr parent) {
   int pid=OwnerPid(parent);var a=new List<Win>();EnumProc cb=(h,d)=>{if(OwnerPid(h)==pid)a.Add(Describe(h));return true;};
   EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray();
  }
  public static string DiagnosticText(IntPtr h,int pid) {
   if(OwnerPid(h)!=pid)return "[window no longer owned by the launched process]";
   var text=new System.Text.StringBuilder(32768);IntPtr result;
   // WM_GETTEXT is read-only. ABORTIFHUNG | ERRORONEXIT bounds each request
   // to 100 ms; diagnostics must not block cleanup on an unresponsive UI.
   if(ReadTimeoutText(h,0x000D,(IntPtr)text.Capacity,text,0x22,100,out result)==IntPtr.Zero)
    return "[WM_GETTEXT failed/timed out; win32_error="+Marshal.GetLastWin32Error()+"]";
   return text.ToString();
  }
  public static bool SetEditText(IntPtr h,string text) { return SendMessageW(h,0x000C,IntPtr.Zero,text)!=IntPtr.Zero; }
  public static ulong PrivateBytes(Process process) {
   var c=new CountersEx();c.cb=(uint)Marshal.SizeOf(typeof(CountersEx));
   if(!GetProcessMemoryInfo(process.Handle,ref c,c.cb))throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
   return c.PrivateUsage.ToUInt64();
  }
 }
 public static class Sampler {
  static readonly object Gate=new object();static Thread Worker;static Process Process;static bool Stop;static string Phase, Error;static int Interval;
  static readonly Dictionary<string,Stat> Stats=new Dictionary<string,Stat>();
  public static void Start(int pid,int interval) {
   lock(Gate){if(Worker!=null)throw new InvalidOperationException("Sampler already started.");Process=System.Diagnostics.Process.GetProcessById(pid);Interval=interval;Phase="import";Stop=false;
    Worker=new Thread(()=>Run()){IsBackground=true,Name="ShotKit PNG export memory sampler"};Worker.Start();}
  }
  public static bool MarkAndWaitForSample(string phase,int timeoutMilliseconds) {
   lock(Gate){Phase=phase;}
   long deadline=Environment.TickCount64+timeoutMilliseconds;
   while(Environment.TickCount64<deadline){lock(Gate){Stat s;if(Error!=null)throw new InvalidOperationException(Error);if(Stats.TryGetValue(phase,out s)&&s.Samples>0)return true;if(Stop)return false;}Thread.Sleep(1);}
   return false;
  }
  static void Run() {
   while(true){string phase;lock(Gate){if(Stop)return;phase=Phase;}
    try { ulong priv=Native.PrivateBytes(Process);lock(Gate){Stat s;if(!Stats.TryGetValue(phase,out s)){s=new Stat{PrivateBytesFirst=priv};Stats.Add(phase,s);}s.Samples++;s.PrivateBytesLast=priv;if(priv>s.PrivateBytesMax)s.PrivateBytesMax=priv;} }
    catch(Exception e){lock(Gate){Error=e.GetType().Name+": "+e.Message;}return;}
    Thread.Sleep(Interval);
   }
  }
  public static Stat[] StopAndGet(out string[] names) {
   Thread t;lock(Gate){Stop=true;t=Worker;}
   if(t!=null&&!t.Join(5000))throw new InvalidOperationException("Memory sampler did not stop.");
   if(Process!=null)Process.Dispose();
   lock(Gate){if(Error!=null)throw new InvalidOperationException(Error);names=new string[Stats.Count];Stats.Keys.CopyTo(names,0);var a=new Stat[Stats.Count];for(int i=0;i<a.Length;i++)a[i]=Stats[names[i]];return a;}
  }
 }
 public static class Pixels {
  public static int Width=6000, Height=4000;
  public static bool ScrollPattern, LargePattern;
  public static int AnnotationState;
  static uint Next(ref uint state) { state^=state<<13;state^=state>>17;state^=state<<5;return state; }
  static void Row(byte[] row,ref uint state,int y) {
   for(int x=0;x<Width;x++){uint value=Next(ref state);if(ScrollPattern){uint gray=(uint)(32+((x/16*37+y/16*53)%192));value=gray|(gray<<8)|(gray<<16);}if(LargePattern){uint gray=(uint)(((x/16+y/16)%2==0)?40:220);value=gray|(gray<<8)|(gray<<16);}int i=x*4;row[i]=(byte)value;row[i+1]=(byte)(value>>8);row[i+2]=(byte)(value>>16);row[i+3]=255;}
  }
  static void Bounds(IntPtr scan0,int stride) { if(scan0==IntPtr.Zero||Math.Abs((long)stride)<Width*4)throw new InvalidOperationException("Invalid 32bpp bitmap row bounds."); }
  public static void Fill(IntPtr scan0,int stride) {
   Bounds(scan0,stride);uint state=0x51C1E27E;var row=new byte[Width*4];
   for(int y=0;y<Height;y++){Row(row,ref state,y);Marshal.Copy(row,0,IntPtr.Add(scan0,checked(y*stride)),row.Length);}
  }
  public static string Verify(IntPtr scan0,int stride,int tolerance) {
   Bounds(scan0,stride);uint state=0x51C1E27E;var expected=new byte[Width*4];var actual=new byte[expected.Length];
   using var hash=System.Security.Cryptography.IncrementalHash.CreateHash(System.Security.Cryptography.HashAlgorithmName.SHA256);
   long blurred=0;
   for(int y=0;y<Height;y++){Row(expected,ref state,y);Marshal.Copy(IntPtr.Add(scan0,checked(y*stride)),actual,0,actual.Length);hash.AppendData(actual);
    for(int i=0;i<actual.Length;i++){
     int x=i/4;
     if(LargePattern&&AnnotationState>0){
      bool inner=AnnotationState!=3&&x>=1088&&x<1984&&y>=1088&&y<1984;
      bool outer=AnnotationState!=3&&x>=960&&x<2112&&y>=960&&y<2112;
      bool blur=(AnnotationState==2||AnnotationState==3)&&x>=1472&&x<2624&&y>=1472&&y<2624;
      if(inner){if(actual[i]!=(i%4==3?255:0))throw new Exception("Redaction not opaque black at "+x+","+y);continue;}
      if(outer||blur){if(i%4==3&&actual[i]!=255)throw new Exception("Alpha changed in annotation");if(blur&&x>2200&&x<2500&&y>1700&&y<2400&&i%4==0&&actual[i]>50&&actual[i]<210)blurred++;continue;}
     }
     if(Math.Abs(actual[i]-expected[i])>(i%4==3?0:tolerance))throw new InvalidOperationException("RGBA mismatch at x="+(i/4)+", y="+y+", BGRA channel="+(i%4)+", expected="+expected[i]+", actual="+actual[i]);}
   }
   if(LargePattern&&(AnnotationState==2||AnnotationState==3)&&blurred<1000)throw new Exception("Gaussian blur has no mixed pixels outside redaction");
   return Convert.ToHexString(hash.GetHashAndReset());
  }
 }
}
'@

[ShotKitPngExportMemorySmoke.Pixels]::Width=$width
[ShotKitPngExportMemorySmoke.Pixels]::Height=$height
[ShotKitPngExportMemorySmoke.Pixels]::ScrollPattern=$ScrollHeight.IsPresent
[ShotKitPngExportMemorySmoke.Pixels]::LargePattern=$LargeAnnotated.IsPresent
$mouseDown=$false
$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Png-Export-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'; $temp = Join-Path $root 'temp'
$settingsDir = Join-Path $appData 'ShotKitRust'
$fixturePath = Join-Path $root "source-${width}x${height}.png"
$outputPath = Join-Path $root "exported-${width}x${height}.png"
$process = $null; $processStopped = $true; $failure = $null; $cleanupFailure = $null
$sampleNames = @(); $sampleStats = @(); $exportMs = 0L; $outputBytes = 0L

function Get-OwnedWindows {
    if ($null -eq $process -or $process.HasExited) { return @() }
    @([ShotKitPngExportMemorySmoke.Native]::Windows($process.Id))
}
function Protect-DiagnosticText([string] $Value) {
    $Value.Replace($root,'[temporary fixture]').Replace($ExePath,'[ShotKit executable]')
}
function Write-OwnedWindowDiagnostics([string] $Description) {
    if ($null -eq $process -or $process.HasExited) { Write-Host 'OWNED_WINDOW_DIAGNOSTICS: launched process is unavailable.'; return }
    $ownedPid = $process.Id
    $windows = @(Get-OwnedWindows)
    $foreground = [ShotKitPngExportMemorySmoke.Native]::GetForegroundWindow()
    Write-Host ("OWNED_WINDOW_DIAGNOSTICS: timeout waiting for {0}; pid={1}; top_level_count={2}; foreground_hwnd={3}; WM_GETTEXT timeout=100ms/control; all visibility states included." -f (Protect-DiagnosticText $Description),$ownedPid,$windows.Count,$foreground.ToInt64())
    foreach ($window in $windows) {
        Write-Host ('OWNED_WINDOW ' + ([ordered]@{pid=$window.Pid;hwnd=$window.H.ToInt64();class=$window.ClassName;title=(Protect-DiagnosticText $window.Title);visible=$window.Visible;enabled=$window.Enabled} | ConvertTo-Json -Compress))
        # EnumChildWindows includes descendants. Recheck each PID before any
        # text request, and never enumerate an unrelated process's windows.
        if ([ShotKitPngExportMemorySmoke.Native]::OwnerPid($window.H) -ne $ownedPid) { continue }
        foreach ($child in [ShotKitPngExportMemorySmoke.Native]::Children($window.H)) {
            if ($child.Pid -ne $ownedPid -or [ShotKitPngExportMemorySmoke.Native]::OwnerPid($child.H) -ne $ownedPid) { continue }
            $text = [ShotKitPngExportMemorySmoke.Native]::DiagnosticText($child.H,$ownedPid)
            Write-Host ('OWNED_CHILD ' + ([ordered]@{pid=$child.Pid;top_level_hwnd=$window.H.ToInt64();hwnd=$child.H.ToInt64();id=$child.Id;class=$child.ClassName;title=(Protect-DiagnosticText $child.Title);text=(Protect-DiagnosticText $text);visible=$child.Visible;enabled=$child.Enabled} | ConvertTo-Json -Compress))
        }
    }
}
function Wait-For([scriptblock] $Probe, [string] $Description, [int] $Seconds = $TimeoutSeconds) {
    $end = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $end) {
        if ($process.HasExited) { throw "PROCESS_EXITED while waiting for $Description (code $($process.ExitCode))." }
        $value = & $Probe
        if ($null -ne $value -and $value) { return $value }
        Start-Sleep -Milliseconds 100
    }
    try { Write-OwnedWindowDiagnostics $Description }
    catch { Write-Host ("OWNED_WINDOW_DIAGNOSTICS_FAILED: {0}" -f (Protect-DiagnosticText $_.Exception.Message)) }
    throw "TIMEOUT waiting for $Description; owned window count=$(@(Get-OwnedWindows).Count)."
}
function Require-Owner([IntPtr] $Handle) {
    if (-not [ShotKitPngExportMemorySmoke.Native]::IsWindow($Handle) -or [ShotKitPngExportMemorySmoke.Native]::OwnerPid($Handle) -ne $process.Id) { throw 'WINDOW_OWNER_MISMATCH.' }
}
function Mark-Phase([string] $Name) {
    if (-not [ShotKitPngExportMemorySmoke.Sampler]::MarkAndWaitForSample($Name,10000)) { throw "NO_MEMORY_SAMPLE: $Name." }
}
function Assert-Image([string] $Path, [string] $Format = 'PNG') {
    $image = [Drawing.Bitmap]::new($Path)
    try {
        $expectedFormat = if ($Format -eq 'PNG') { [Drawing.Imaging.ImageFormat]::Png } else { [Drawing.Imaging.ImageFormat]::Jpeg }
        if ($image.RawFormat.Guid -ne $expectedFormat.Guid -or $image.Width -ne $width -or $image.Height -ne $height) { throw 'IMAGE_FORMAT_OR_DIMENSION_MISMATCH.' }
        $data = $image.LockBits([Drawing.Rectangle]::new(0,0,$width,$height),[Drawing.Imaging.ImageLockMode]::ReadOnly,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try { $script:lastPixelHash=[ShotKitPngExportMemorySmoke.Pixels]::Verify($data.Scan0,$data.Stride,$(if ($Format -eq 'PNG') { 0 } else { 3 })) }
        finally { $image.UnlockBits($data) }
    } finally { $image.Dispose() }
}
function Stop-OwnedProcess {
    if ($null -eq $process) { return }
    if ($processStopped) { $process.Dispose(); $script:process = $null; return }
    if (-not $process.HasExited) {
        foreach ($window in [ShotKitPngExportMemorySmoke.Native]::Windows($process.Id)) { [void][ShotKitPngExportMemorySmoke.Native]::PostMessageW($window.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero) }
        if (-not $process.WaitForExit(5000)) { $process.Kill(); [void]$process.WaitForExit(5000) }
    }
    $script:processStopped = $process.HasExited
    $process.Dispose(); $script:process = $null
    if (-not $script:processStopped) { throw 'OWNED_PROCESS_CLEANUP_FAILED.' }
}

function Click-EditorButton([string] $Text) {
    Require-Owner $editor.H
    $buttons=@([ShotKitPngExportMemorySmoke.Native]::Children($editor.H) | Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and ($_.Title -replace '&','').Trim() -ceq $Text})
    if ($buttons.Count -ne 1) { throw "EDITOR_BUTTON_AMBIGUOUS: $Text." }
    if (-not [ShotKitPngExportMemorySmoke.Native]::PostMessageW($buttons[0].H,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)) { throw 'EDITOR_BUTTON_POST_FAILED.' }
}
function Draw-LargeAnnotation([byte] $Key,[string] $Tool,[int] $Low,[int] $High) {
    Require-Owner $editor.H
    if (-not [ShotKitPngExportMemorySmoke.Native]::SetForegroundWindow($editor.H)) { throw 'EDITOR_FOCUS_FAILED.' }
    [void](Wait-For {[ShotKitPngExportMemorySmoke.Native]::GetForegroundWindow() -eq $editor.H} 'editor foreground')
    [ShotKitPngExportMemorySmoke.Native]::keybd_event($Key,0,0,[UIntPtr]::Zero)
    [ShotKitPngExportMemorySmoke.Native]::keybd_event($Key,0,2,[UIntPtr]::Zero)
    [void](Wait-For {Get-OwnedWindows | Where-Object {$_.H -eq $editor.H -and $_.Title -like "*Werkzeug $Tool*"}} 'annotation tool')
    $client=[ShotKitPngExportMemorySmoke.Rect]::new()
    if (-not [ShotKitPngExportMemorySmoke.Native]::GetClientRect($editor.H,[ref]$client)) { throw 'CLIENT_RECT_FAILED.' }
    $toolbar=@([ShotKitPngExportMemorySmoke.Native]::Children($editor.H) | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'})
    if ($toolbar.Count -ne 1) { throw 'TOOLBAR_AMBIGUOUS.' }
    $tr=[ShotKitPngExportMemorySmoke.Rect]::new()
    if (-not [ShotKitPngExportMemorySmoke.Native]::GetClientRect($toolbar[0].H,[ref]$tr)) { throw 'TOOLBAR_RECT_FAILED.' }
    $side=[int][Math]::Round([Math]::Min($client.Right,$client.Bottom-$tr.Bottom))
    if ($side -lt 256) { throw 'VIEWPORT_TOO_SMALL_FOR_BOUNDED_PIXEL_MARGIN.' }
    $left=[int][Math]::Floor(($client.Right-$side)/2.0)
    $top=$tr.Bottom+[int][Math]::Floor(($client.Bottom-$tr.Bottom-$side)/2.0)
    $origin=[ShotKitPngExportMemorySmoke.Point]::new()
    if (-not [ShotKitPngExportMemorySmoke.Native]::ClientToScreen($editor.H,[ref]$origin)) { throw 'CLIENT_TO_SCREEN_FAILED.' }
    $a=[ShotKitPngExportMemorySmoke.Point]::new();$b=[ShotKitPngExportMemorySmoke.Point]::new()
    $a.X=$origin.X+$left+[int][Math]::Round(($Low+0.5)*$side/8192)
    $a.Y=$origin.Y+$top+[int][Math]::Round(($Low+0.5)*$side/8192)
    $b.X=$origin.X+$left+[int][Math]::Round(($High+0.5)*$side/8192)
    $b.Y=$origin.Y+$top+[int][Math]::Round(($High+0.5)*$side/8192)
    foreach ($point in @($a,$b)) {
        if ([ShotKitPngExportMemorySmoke.Native]::OwnerPid([ShotKitPngExportMemorySmoke.Native]::WindowFromPoint($point)) -ne $process.Id) { throw 'ANNOTATION_INPUT_TARGET_MISMATCH.' }
    }
    if (-not [ShotKitPngExportMemorySmoke.Native]::SetCursorPos($a.X,$a.Y)) { throw 'CURSOR_START_FAILED.' }
    [ShotKitPngExportMemorySmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true
    Start-Sleep -Milliseconds 100
    if (-not [ShotKitPngExportMemorySmoke.Native]::SetCursorPos($b.X,$b.Y)) { throw 'CURSOR_END_FAILED.' }
    Start-Sleep -Milliseconds 200
    [ShotKitPngExportMemorySmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
    [void](Wait-For {Get-OwnedWindows | Where-Object {$_.H -eq $editor.H -and $_.Title.StartsWith('* ')}} 'dirty annotation title')
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData,$temp,$settingsDir -Force | Out-Null
    $settings = @{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'} | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $settingsDir 'settings.json'),$settings,[Text.UTF8Encoding]::new($false))
    # Opaque random RGB avoids GDI alpha/premultiplication ambiguity while still
    # exercising a poorly compressible large PNG export. Transparent/partial
    # alpha is covered by the Rust PNG tests, not by this GUI smoke.
    $bitmap = [Drawing.Bitmap]::new($width,$height,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $data = $bitmap.LockBits([Drawing.Rectangle]::new(0,0,$width,$height),[Drawing.Imaging.ImageLockMode]::WriteOnly,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try { [ShotKitPngExportMemorySmoke.Pixels]::Fill($data.Scan0,$data.Stride) }
        finally { $bitmap.UnlockBits($data) }
        $bitmap.Save($fixturePath,[Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
    if ($LargeAnnotated) {
        $header=[byte[]]::new(26)
        $headerStream=[IO.File]::OpenRead($fixturePath)
        try { if ($headerStream.Read($header,0,26) -ne 26) { throw 'PNG_HEADER_TRUNCATED.' } } finally { $headerStream.Dispose() }
        if ($header[24] -ne 8 -or $header[25] -ne 6) { throw 'LARGE_FIXTURE_MUST_BE_RGBA8_PNG.' }
        $header=$null
    }
    Assert-Image $fixturePath
    $sourceHash = (Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash
    $fixtureBytes = (Get-Item -LiteralPath $fixturePath).Length
    if (Test-Path -LiteralPath $outputPath) { throw 'OUTPUT_ALREADY_EXISTS.' }

    # Only the launched child inherits this private settings/temp context.
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $ExePath; $start.UseShellExecute = $false; $start.WorkingDirectory = $root
    $paths=@($fixturePath)
    for ($i=1; $i -lt $DocumentCount; $i++) {
        $path=Join-Path $root "source-$i.png"
        Copy-Item -LiteralPath $fixturePath -Destination $path
        $paths+=$path
    }
    foreach ($path in $paths) { [void]$start.ArgumentList.Add($path) }
    $start.Environment['APPDATA'] = $appData; $start.Environment['LOCALAPPDATA'] = $localAppData
    $start.Environment['TEMP'] = $temp; $start.Environment['TMP'] = $temp
    $process = [Diagnostics.Process]::new(); $process.StartInfo = $start
    # A simultaneous foreign start can still race this check. The dedicated
    # interactive test-session requirement remains mandatory.
    Assert-NoExistingShotKitInstance
    if (-not $process.Start()) { throw 'PROCESS_START_FAILED.' }
    $processStopped = $false
    [ShotKitPngExportMemorySmoke.Sampler]::Start($process.Id,$SampleIntervalMs)
    Mark-Phase 'import'
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $process -GetWindows { Get-OwnedWindows }
    $editor = Wait-For { Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title.IndexOf($fixturePath,[StringComparison]::OrdinalIgnoreCase) -ge 0 -and -not $_.Title.StartsWith('* ')} | Select-Object -First 1 } 'loaded source title'
    $identities=@()
    foreach ($path in $paths) {
        $loaded=Wait-For { Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title.IndexOf($path,[StringComparison]::OrdinalIgnoreCase) -ge 0 -and -not $_.Title.StartsWith('* ')} | Select-Object -First 1 } 'all simultaneous document titles'
        Require-Owner $loaded.H
        $identities+=[pscustomobject]@{H=$loaded.H;Title=$loaded.Title;Path=$path}
    }
    if (@($identities.H | Select-Object -Unique).Count -ne $DocumentCount) { throw 'EDITOR_IDENTITIES_NOT_DISTINCT.' }
    Mark-Phase 'all_documents_open'
    $formats = if ($ScrollHeight) { @('PNG','JPEG') } else { @('PNG') }
    $combinedRejected=$false
    $stages=if ($LargeAnnotated) {@('Redact','Both','BlurOnly','RestoreBoth','UndoBlur','UndoRedact')} else {@('Original')}
    foreach ($stage in $stages) {
    if ($LargeAnnotated) {
        if ($stage -in @('BlurOnly','RestoreBoth') -and -not $combinedRejected) { continue }
        [void][ShotKitPngExportMemorySmoke.Native]::ShowWindow($editor.H,3)
        if ($stage -eq 'Redact') { Draw-LargeAnnotation 0x53 'Redact' 1024 2048 }
        elseif ($stage -eq 'Both') { Draw-LargeAnnotation 0x57 'GaussianBlur' 1536 2560 }
        elseif ($stage -eq 'BlurOnly') {
            # Inspect the actual post-failure document, not a previous export:
            # temporarily remove only its non-overlapping redaction object.
            # The remaining existing Gaussian object must survive and export.
            Draw-LargeAnnotation 0x56 'Select' 1280 1280
            [ShotKitPngExportMemorySmoke.Native]::keybd_event(0x2E,0,0,[UIntPtr]::Zero)
            [ShotKitPngExportMemorySmoke.Native]::keybd_event(0x2E,0,2,[UIntPtr]::Zero)
            Start-Sleep -Milliseconds 250
        }
        else { Click-EditorButton 'Zurück'; Start-Sleep -Milliseconds 250 }
        if ($stage -eq 'RestoreBoth') {
            # Undo the temporary deletion before the original history checks.
            # The next UndoBlur removes the original Gaussian object only.
            continue
        }
        [ShotKitPngExportMemorySmoke.Pixels]::AnnotationState=$(if ($stage -eq 'Both') {2} elseif ($stage -eq 'BlurOnly') {3} elseif ($stage -eq 'UndoRedact') {0} else {1})
    }
    foreach ($format in $formats) {
        $extension = if ($format -eq 'PNG') { 'png' } else { 'jpg' }
        $outputPath=Join-Path $root "exported-${width}x${height}-$stage.$extension"
        Require-Owner $editor.H
        # ID_SAVE_AS invokes the real GUI save dialog and extension-selected encoder.
        if (-not [ShotKitPngExportMemorySmoke.Native]::PostMessageW($editor.H,0x0111,[IntPtr]0x5140,[IntPtr]::Zero)) { throw 'SAVE_AS_COMMAND_FAILED.' }
        $dialog = Wait-For { Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and @([ShotKitPngExportMemorySmoke.Native]::Children($_.H) | Where-Object {$_.Visible -and $_.ClassName -eq 'Edit'}).Count -gt 0} | Select-Object -First 1 } 'owned Save As dialog'
        Require-Owner $dialog.H
        # Use the same visible Edit control as windows-save-original-smoke.ps1,
        # but fail on ambiguity instead of guessing a control or trying another path.
        $edits = @([ShotKitPngExportMemorySmoke.Native]::Children($dialog.H) | Where-Object {$_.Visible -and $_.ClassName -eq 'Edit'})
        if ($edits.Count -ne 1) { throw "SAVE_AS_FILENAME_AMBIGUOUS: found $($edits.Count) visible Edit controls." }
        Require-Owner $edits[0].H
        if (-not [ShotKitPngExportMemorySmoke.Native]::SetEditText($edits[0].H,$outputPath)) { throw 'SAVE_AS_FILENAME_SET_FAILED.' }
        $buttons = @([ShotKitPngExportMemorySmoke.Native]::Children($dialog.H) | Where-Object {$_.Visible -and $_.ClassName -eq 'Button' -and $_.Id -eq 1})
        if ($buttons.Count -ne 1) { throw "SAVE_AS_BUTTON_AMBIGUOUS: found $($buttons.Count) IDOK buttons." }
        Require-Owner $buttons[0].H
        if (-not [ShotKitPngExportMemorySmoke.Native]::SetForegroundWindow($dialog.H)) { throw 'SAVE_AS_FOCUS_FAILED.' }
        [void](Wait-For { [ShotKitPngExportMemorySmoke.Native]::GetForegroundWindow() -eq $dialog.H } 'Save As foreground' 10)
        Mark-Phase 'before_export_dialog_ready'
        Start-Sleep -Milliseconds 500
        Mark-Phase 'png_export'
        $exportTimer = [Diagnostics.Stopwatch]::StartNew()
        if (-not [ShotKitPngExportMemorySmoke.Native]::PostMessageW($buttons[0].H,0x00F5,[IntPtr]::Zero,[IntPtr]::Zero)) { throw 'SAVE_AS_ACCEPT_FAILED.' }
        if ($LargeAnnotated -and $stage -eq 'Both') {
            $outcome=Wait-For {
                $owned=@(Get-OwnedWindows)
                $errorWindow=$owned | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -eq 'ShotKit'} | Select-Object -First 1
                if ($null -ne $errorWindow) { return $errorWindow }
                $owned | Where-Object {$_.H -eq $editor.H -and $_.Title.IndexOf($outputPath,[StringComparison]::OrdinalIgnoreCase) -ge 0 -and -not $_.Title.StartsWith('* ')} | Select-Object -First 1
            } 'combined annotation save result'
            if ($outcome.ClassName -eq '#32770') {
                $combinedRejected=$true
                Require-Owner $outcome.H
                $text=(@([ShotKitPngExportMemorySmoke.Native]::Children($outcome.H) | ForEach-Object {[ShotKitPngExportMemorySmoke.Native]::DiagnosticText($_.H,$process.Id)}) -join ' ')
                if ($text -notlike '*Speichern fehlgeschlagen:*' -or $text -notmatch 'Bildbudget|Budget|budget') { throw "UNEXPECTED_SAVE_ERROR: $text" }
                if (Test-Path -LiteralPath $outputPath) { throw 'FAILED_SAVE_PUBLISHED_FILE.' }
                if (-not [ShotKitPngExportMemorySmoke.Native]::PostMessageW($outcome.H,0x0111,[IntPtr]1,[IntPtr]::Zero)) { throw 'SAVE_ERROR_DISMISS_FAILED.' }
                [void](Wait-For {Get-OwnedWindows | Where-Object {$_.H -eq $editor.H -and $_.Title.StartsWith('* ') -and [ShotKitPngExportMemorySmoke.Native]::IsWindowEnabled($_.H)}} 'retained dirty document after budget rejection')
                if ((Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash -cne $sourceHash) { throw 'SOURCE_CHANGED_AFTER_REJECTION.' }
                [ShotKitPngExportMemorySmoke.Pixels]::AnnotationState=1
                Assert-Image (Join-Path $root "exported-${width}x${height}-Redact.png")
                Write-Host "EXPECTED_BUDGET_REJECTION: $text; dirty editor retained; previous redacted file and source intact. BlurOnly export must independently prove the retained Gaussian object; restoring the redaction and UndoBlur/UndoRedact must prove retained history and pixels."
                continue
            }
        }
        # refresh_title runs only after the encoder and atomic write both succeed.
        [void](Wait-For { Get-OwnedWindows | Where-Object {$_.H -eq $editor.H -and $_.Title.IndexOf($outputPath,[StringComparison]::OrdinalIgnoreCase) -ge 0 -and -not $_.Title.StartsWith('* ') -and [ShotKitPngExportMemorySmoke.Native]::IsWindowEnabled($_.H)} } 'completed image export title')
        $exportTimer.Stop(); $exportMs = $exportTimer.ElapsedMilliseconds
        if (-not (Test-Path -LiteralPath $outputPath -PathType Leaf)) { throw 'PNG_OUTPUT_MISSING.' }
        $outputBytes = (Get-Item -LiteralPath $outputPath).Length
        if ($outputBytes -le 0) { throw 'PNG_OUTPUT_EMPTY.' }
        Mark-Phase 'post_export'
        Start-Sleep -Milliseconds 500
        Assert-Image $outputPath $format
        if ($LargeAnnotated -and $stage -eq 'BlurOnly') { Write-Host 'RETAINED_GAUSSIAN_OBJECT_VERIFIED: current post-rejection document exported with temporary redaction removal; mixed blur pixels and all outside pixels checked. The deletion must now be undone.' }
        if ($LargeAnnotated -and $stage -eq 'Redact') { $redactedPixelHash=$lastPixelHash }
        if ($LargeAnnotated -and $stage -eq 'UndoBlur' -and $lastPixelHash -cne $redactedPixelHash) { throw 'REDACTION_PIXELS_CHANGED_AFTER_BLUR_UNDO.' }
        if ((Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash -cne $sourceHash) { throw 'SOURCE_PNG_CHANGED.' }
        if ($process.HasExited) { throw "PROCESS_EXITED_AFTER_EXPORT: code $($process.ExitCode)." }
        $remaining=@(Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor'})
        if ($remaining.Count -ne $DocumentCount) { throw 'OPEN_DOCUMENT_COUNT_CHANGED_AFTER_SAVE.' }
        foreach ($identity in $identities) {
            Require-Owner $identity.H
            if ((Get-FileHash -LiteralPath $identity.Path -Algorithm SHA256).Hash -cne $sourceHash) { throw 'SOURCE_DOCUMENT_CHANGED.' }
            if ($identity.H -ne $editor.H -and @($remaining | Where-Object {$_.H -eq $identity.H -and $_.Title -ceq $identity.Title}).Count -ne 1) { throw 'OTHER_DOCUMENT_CHANGED_AFTER_SAVE.' }
        }
        Write-Host "EXPORT_PASS: format=$format dimensions=${width}x${height} simultaneous_documents=$DocumentCount; decoded pixels checked (annotation boundary margins when applicable); source hashes and editor identities preserved."
    }
    }
} catch { $failure = $_ }
finally {
    if ($mouseDown) { [ShotKitPngExportMemorySmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero) }
    try { $sampleStats = [ShotKitPngExportMemorySmoke.Sampler]::StopAndGet([ref]$sampleNames) } catch { if (-not $failure) { $failure = $_ } }
    if ($null -ne $process) { try { Stop-OwnedProcess } catch { $cleanupFailure = $_.Exception.Message } }
    if ($processStopped -and (Test-Path -LiteralPath $root)) { try { Remove-Item -LiteralPath $root -Recurse -Force } catch { $cleanupFailure = 'TEMPORARY_FIXTURE_CLEANUP_FAILED.' } }
    elseif (-not $processStopped) { $cleanupFailure = 'TEMPORARY_FIXTURE_RETAINED: owned process did not stop.' }
}
Write-Host "MEASUREMENTS: dimensions=${width}x${height} rgba_bytes=$([long]$width*$height*4) simultaneous_documents=$DocumentCount input_bytes=$fixtureBytes output_bytes=$outputBytes export_observed_ms=$exportMs sample_interval_ms=$SampleIntervalMs windows_version=$([Environment]::OSVersion.Version) exe_sha256=$((Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash)"
Write-Host 'PrivateUsage is process private committed memory, not working set or encoder-only allocation; the export phase includes dialog completion, image preparation, encoding, and atomic file writing.'
Write-Host 'Peaks between samples can be missed; scheduling can exceed the requested interval. Import has no prelaunch baseline. Measurements apply only to this build, host, fixture, and workload; no memory threshold is asserted.'
for ($i=0; $i -lt $sampleNames.Count; $i++) { $s=$sampleStats[$i]; Write-Host ('PHASE {0} samples={1} first_private_bytes={2} max_sampled_private_bytes={3} last_private_bytes={4}' -f $sampleNames[$i],$s.Samples,$s.PrivateBytesFirst,$s.PrivateBytesMax,$s.PrivateBytesLast) }
if ($failure) {
    $message = $failure.Exception.Message.Replace($root,'[temporary fixture]').Replace($ExePath,'[ShotKit executable]')
    if ($cleanupFailure) { $message += "; cleanup: $cleanupFailure" }
    throw "SMOKE_FAILED: $message"
}
if ($cleanupFailure) { throw "CLEANUP_FAILED: $cleanupFailure" }
foreach ($phase in @('import','all_documents_open','before_export_dialog_ready','png_export','post_export')) {
    $index = [Array]::IndexOf($sampleNames,$phase)
    if ($index -lt 0 -or $sampleStats[$index].Samples -lt 1) { throw "NO_MEMORY_SAMPLES: required phase '$phase' has no sample." }
}
Write-Host "PASS: GUI Save As completed with $DocumentCount simultaneous ${width}x${height} documents; full output pixel verification and unchanged source hashes."
if ($LargeAnnotated) { Write-Host 'ANNOTATED_LARGE_PASS: 8192x8192 RGBA import; redaction export, overlapping Gaussian blur save or checked budget rejection, UndoBlur redaction export and UndoRedact original export fully verified.' }
if ($ScrollHeight) { Write-Host 'SCENARIO_1: synthetic 256x30000 result exported as PNG and JPEG; this does not prove the actual scroll acquisition reaches 30000 pixels. JPEG grayscale 16x16 blocks have a maximum per-channel tolerance of 3; PNG is exact.' }
Write-Host 'SCOPE: this GUI smoke does not test A=0 or partial alpha; the existing Rust PNG round-trip tests cover transparent RGBA pixels.'

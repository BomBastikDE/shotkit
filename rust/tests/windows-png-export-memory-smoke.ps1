[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(30, 300)][int] $TimeoutSeconds = 90,
    [ValidateRange(1, 100)][int] $SampleIntervalMs = 10
)

$ErrorActionPreference = 'Stop'
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

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
namespace ShotKitPngExportMemorySmoke {
 public sealed class Win { public IntPtr H; public int Id, Pid; public string ClassName, Title; public bool Visible, Enabled; }
 [StructLayout(LayoutKind.Sequential)] struct CountersEx {
  public uint cb, PageFaultCount;
  public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage;
  public UIntPtr QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage, PrivateUsage;
 }
 public sealed class Stat { public ulong PrivateBytesMax, PrivateBytesFirst, PrivateBytesLast; public long Samples; }
 public static class Native {
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
  public const int Width=6000, Height=4000;
  static uint Next(ref uint state) { state^=state<<13;state^=state>>17;state^=state<<5;return state; }
  static void Row(byte[] row,ref uint state) {
   for(int x=0;x<Width;x++){uint value=Next(ref state);int i=x*4;row[i]=(byte)value;row[i+1]=(byte)(value>>8);row[i+2]=(byte)(value>>16);row[i+3]=255;}
  }
  static void Bounds(IntPtr scan0,int stride) { if(scan0==IntPtr.Zero||Math.Abs((long)stride)<Width*4)throw new InvalidOperationException("Invalid 32bpp bitmap row bounds."); }
  public static void Fill(IntPtr scan0,int stride) {
   Bounds(scan0,stride);uint state=0x51C1E27E;var row=new byte[Width*4];
   for(int y=0;y<Height;y++){Row(row,ref state);Marshal.Copy(row,0,IntPtr.Add(scan0,checked(y*stride)),row.Length);}
  }
  public static void Verify(IntPtr scan0,int stride) {
   Bounds(scan0,stride);uint state=0x51C1E27E;var expected=new byte[Width*4];var actual=new byte[expected.Length];
   for(int y=0;y<Height;y++){Row(expected,ref state);Marshal.Copy(IntPtr.Add(scan0,checked(y*stride)),actual,0,actual.Length);
    for(int i=0;i<actual.Length;i++)if(actual[i]!=expected[i])throw new InvalidOperationException("RGBA mismatch at x="+(i/4)+", y="+y+", BGRA channel="+(i%4)+", expected="+expected[i]+", actual="+actual[i]);}
  }
 }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Png-Export-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'; $temp = Join-Path $root 'temp'
$settingsDir = Join-Path $appData 'ShotKitRust'
$fixturePath = Join-Path $root 'source-6000x4000.png'; $outputPath = Join-Path $root 'exported-6000x4000.png'
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
function Assert-Png([string] $Path) {
    $image = [Drawing.Bitmap]::new($Path)
    try {
        if ($image.RawFormat.Guid -ne [Drawing.Imaging.ImageFormat]::Png.Guid -or $image.Width -ne 6000 -or $image.Height -ne 4000) { throw 'PNG_FORMAT_OR_DIMENSION_MISMATCH.' }
        $data = $image.LockBits([Drawing.Rectangle]::new(0,0,6000,4000),[Drawing.Imaging.ImageLockMode]::ReadOnly,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try { [ShotKitPngExportMemorySmoke.Pixels]::Verify($data.Scan0,$data.Stride) }
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

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData,$temp,$settingsDir -Force | Out-Null
    $settings = @{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'} | ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $settingsDir 'settings.json'),$settings,[Text.UTF8Encoding]::new($false))
    # Opaque random RGB avoids GDI alpha/premultiplication ambiguity while still
    # exercising a poorly compressible large PNG export. Transparent/partial
    # alpha is covered by the Rust PNG tests, not by this GUI smoke.
    $bitmap = [Drawing.Bitmap]::new(6000,4000,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $data = $bitmap.LockBits([Drawing.Rectangle]::new(0,0,6000,4000),[Drawing.Imaging.ImageLockMode]::WriteOnly,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try { [ShotKitPngExportMemorySmoke.Pixels]::Fill($data.Scan0,$data.Stride) }
        finally { $bitmap.UnlockBits($data) }
        $bitmap.Save($fixturePath,[Drawing.Imaging.ImageFormat]::Png)
    } finally { $bitmap.Dispose() }
    Assert-Png $fixturePath
    $sourceHash = (Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash
    $fixtureBytes = (Get-Item -LiteralPath $fixturePath).Length
    if (Test-Path -LiteralPath $outputPath) { throw 'OUTPUT_ALREADY_EXISTS.' }

    # Only the launched child inherits this private settings/temp context.
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $ExePath; $start.UseShellExecute = $false; $start.WorkingDirectory = $root
    [void]$start.ArgumentList.Add($fixturePath)
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
    $notice = Wait-For { Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'} | Select-Object -First 1 } 'owned startup notice'
    Require-Owner $notice.H
    if (-not [ShotKitPngExportMemorySmoke.Native]::PostMessageW($notice.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)) { throw 'STARTUP_NOTICE_CLOSE_FAILED.' }
    [void](Wait-For { -not [ShotKitPngExportMemorySmoke.Native]::IsWindow($notice.H) } 'startup notice close' 10)
    $editor = Wait-For { Get-OwnedWindows | Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title.IndexOf($fixturePath,[StringComparison]::OrdinalIgnoreCase) -ge 0 -and -not $_.Title.StartsWith('* ')} | Select-Object -First 1 } 'loaded source title'
    Require-Owner $editor.H
    # ID_SAVE_AS in toolbar.rs invokes the real GUI save dialog and PNG encode/write path.
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
    # refresh_title runs only after the encoder and atomic write both succeed.
    [void](Wait-For { Get-OwnedWindows | Where-Object {$_.H -eq $editor.H -and $_.Title.IndexOf($outputPath,[StringComparison]::OrdinalIgnoreCase) -ge 0 -and -not $_.Title.StartsWith('* ') -and [ShotKitPngExportMemorySmoke.Native]::IsWindowEnabled($_.H)} } 'completed PNG export title')
    $exportTimer.Stop(); $exportMs = $exportTimer.ElapsedMilliseconds
    if (-not (Test-Path -LiteralPath $outputPath -PathType Leaf)) { throw 'PNG_OUTPUT_MISSING.' }
    $outputBytes = (Get-Item -LiteralPath $outputPath).Length
    if ($outputBytes -le 0) { throw 'PNG_OUTPUT_EMPTY.' }
    Mark-Phase 'post_export'
    Start-Sleep -Milliseconds 500
    Assert-Png $outputPath
    if ((Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash -cne $sourceHash) { throw 'SOURCE_PNG_CHANGED.' }
    if ($process.HasExited) { throw "PROCESS_EXITED_AFTER_EXPORT: code $($process.ExitCode)." }
} catch { $failure = $_ }
finally {
    try { $sampleStats = [ShotKitPngExportMemorySmoke.Sampler]::StopAndGet([ref]$sampleNames) } catch { if (-not $failure) { $failure = $_ } }
    if ($null -ne $process) { try { Stop-OwnedProcess } catch { $cleanupFailure = $_.Exception.Message } }
    if ($processStopped -and (Test-Path -LiteralPath $root)) { try { Remove-Item -LiteralPath $root -Recurse -Force } catch { $cleanupFailure = 'TEMPORARY_FIXTURE_CLEANUP_FAILED.' } }
    elseif (-not $processStopped) { $cleanupFailure = 'TEMPORARY_FIXTURE_RETAINED: owned process did not stop.' }
}
Write-Host "MEASUREMENTS: dimensions=6000x4000 rgba_bytes=96000000 input_bytes=$fixtureBytes output_bytes=$outputBytes export_observed_ms=$exportMs sample_interval_ms=$SampleIntervalMs windows_version=$([Environment]::OSVersion.Version) exe_sha256=$((Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash)"
Write-Host 'PrivateUsage is process private committed memory, not working set or encoder-only allocation; the export phase includes dialog completion, image preparation, encoding, and atomic file writing.'
Write-Host 'Peaks between samples can be missed; scheduling can exceed the requested interval. Import has no prelaunch baseline. Measurements apply only to this build, host, fixture, and workload; no memory threshold is asserted.'
for ($i=0; $i -lt $sampleNames.Count; $i++) { $s=$sampleStats[$i]; Write-Host ('PHASE {0} samples={1} first_private_bytes={2} max_sampled_private_bytes={3} last_private_bytes={4}' -f $sampleNames[$i],$s.Samples,$s.PrivateBytesFirst,$s.PrivateBytesMax,$s.PrivateBytesLast) }
if ($failure) {
    $message = $failure.Exception.Message.Replace($root,'[temporary fixture]').Replace($ExePath,'[ShotKit executable]')
    if ($cleanupFailure) { $message += "; cleanup: $cleanupFailure" }
    throw "SMOKE_FAILED: $message"
}
if ($cleanupFailure) { throw "CLEANUP_FAILED: $cleanupFailure" }
foreach ($phase in @('import','before_export_dialog_ready','png_export','post_export')) {
    $index = [Array]::IndexOf($sampleNames,$phase)
    if ($index -lt 0 -or $sampleStats[$index].Samples -lt 1) { throw "NO_MEMORY_SAMPLES: required phase '$phase' has no sample." }
}
Write-Host 'PASS: actual GUI Save As exported a 6000x4000 PNG; every decoded RGBA channel matched the opaque synthetic fixture (A=255), and the source SHA256 remained unchanged.'
Write-Host 'SCOPE: this GUI smoke does not test A=0 or partial alpha; the existing Rust PNG round-trip tests cover transparent RGBA pixels.'

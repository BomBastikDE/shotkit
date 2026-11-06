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
$imageName = [IO.Path]::GetFileNameWithoutExtension($ExePath)
if (@(Get-Process -Name $imageName -ErrorAction SilentlyContinue).Count -gt 0) {
    throw 'PREEXISTING_SHOTKIT_PROCESS: refusing to risk interacting with an existing single-instance process.'
}

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Threading;
namespace ShotKitMemorySmoke {
 public sealed class Win { public IntPtr H; public string ClassName, Title; public bool Visible; }
 public struct Rect { public int Left, Top, Right, Bottom; }
 [StructLayout(LayoutKind.Sequential)] struct CountersEx {
  public uint cb, PageFaultCount;
  public UIntPtr PeakWorkingSetSize, WorkingSetSize, QuotaPeakPagedPoolUsage, QuotaPagedPoolUsage;
  public UIntPtr QuotaPeakNonPagedPoolUsage, QuotaNonPagedPoolUsage, PagefileUsage, PeakPagefileUsage, PrivateUsage;
 }
 public sealed class Stat { public ulong PrivateBytesMax, PrivateBytesLast, PeakWorkingSetBytesMax; public long Samples; }
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
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point p);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("psapi.dll", SetLastError=true)] static extern bool GetProcessMemoryInfo(IntPtr process, ref CountersEx counters, uint size);
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
  public static bool TryMemory(Process p,out ulong privateBytes,out ulong peakWorkingSet) {
   var c=new CountersEx();c.cb=(uint)Marshal.SizeOf(typeof(CountersEx));
   if(!GetProcessMemoryInfo(p.Handle,ref c,c.cb)){privateBytes=peakWorkingSet=0;return false;}
   privateBytes=c.PrivateUsage.ToUInt64();peakWorkingSet=c.PeakWorkingSetSize.ToUInt64();return true;
  }
  public struct Point { public int X,Y; }
 }
 public static class Sampler {
  static readonly object Gate=new object(); static Thread Worker; static Process Process; static bool Stop; static string Phase;
  static readonly Dictionary<string,Stat> Stats=new Dictionary<string,Stat>(); static int Interval;
  public static void Start(int pid,int interval) {
  lock(Gate){if(Worker!=null)throw new InvalidOperationException("Sampler already started.");Process=System.Diagnostics.Process.GetProcessById(pid);Interval=interval;Phase="import_and_idle";Stop=false;
    Worker=new Thread(()=>Run()){IsBackground=true,Name="ShotKit memory sampler"};Worker.Start();}
  }
  public static void Mark(string phase) { lock(Gate){Phase=phase;} }
  public static bool MarkAndWaitForSample(string phase,int timeoutMilliseconds) {
   lock(Gate){Phase=phase;}
   long deadline=Environment.TickCount64+timeoutMilliseconds;
   while(Environment.TickCount64<deadline){lock(Gate){Stat s;if(Stats.TryGetValue(phase,out s)&&s.Samples>0)return true;if(Stop)return false;}Thread.Sleep(1);}
   return false;
  }
  static void Run() {
   while(true){string phase;lock(Gate){if(Stop)return;phase=Phase;}
    try { ulong priv,ws;if(Native.TryMemory(Process,out priv,out ws)){lock(Gate){Stat s;if(!Stats.TryGetValue(phase,out s)){s=new Stat();Stats.Add(phase,s);}s.Samples++;s.PrivateBytesLast=priv;if(priv>s.PrivateBytesMax)s.PrivateBytesMax=priv;if(ws>s.PeakWorkingSetBytesMax)s.PeakWorkingSetBytesMax=ws;}} }
    catch(ObjectDisposedException){return;}catch(InvalidOperationException){return;}
    Thread.Sleep(Interval);
   }
  }
  public static Stat[] StopAndGet(out string[] names) {
   Thread t;lock(Gate){Stop=true;t=Worker;}if(t!=null)t.Join(5000);
   lock(Gate){names=new string[Stats.Count];Stats.Keys.CopyTo(names,0);var a=new Stat[Stats.Count];for(int i=0;i<a.Length;i++)a[i]=Stats[names[i]];return a;}
  }
 }
}
'@

$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Memory-Smoke-' + [Guid]::NewGuid().ToString('N'))
$appData = Join-Path $root 'appdata'; $localAppData = Join-Path $root 'localappdata'
$settingsDir = Join-Path $appData 'ShotKitRust'; $fixturePath = Join-Path $root '6000x4000.bmp'
$oldAppData = $env:APPDATA; $oldLocalAppData = $env:LOCALAPPDATA
$process = $null; $processStopped = $true; $mouseDown = $false; $failure = $null; $cleanupFailure = $null
$sampleNames = @(); $sampleStats = @(); $editorHandle = [IntPtr]::Zero

function Get-OwnedWindows {
    if ($null -eq $process -or $process.HasExited) { return @() }
    @([ShotKitMemorySmoke.Native]::Windows($process.Id))
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
function Write-LargeBmp([string] $Path) {
    # 24-bit uncompressed BMP: 6000 * 4000 * 3 = 72,000,000 pixel bytes.
    $width=6000; $height=4000; $rowBytes=$width*3; $pixelBytes=$rowBytes*$height
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
    if((Get-Item -LiteralPath $Path).Length -ne 72000054){throw 'FIXTURE_INVALID: generated BMP length was unexpected.'}
}
function Drag-Crop([IntPtr] $Editor,[int] $Pid) {
    $client=[ShotKitMemorySmoke.Rect]::new()
    if(-not [ShotKitMemorySmoke.Native]::GetClientRect($Editor,[ref]$client)){throw 'EDITOR_CLIENT_RECT_FAILED.'}
    $toolbar=@([ShotKitMemorySmoke.Native]::Children($Editor)|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditorToolbar'})
    if($toolbar.Count -ne 1){throw "EDITOR_TOOLBAR_AMBIGUOUS: found $($toolbar.Count)."}
    $tr=[ShotKitMemorySmoke.Rect]::new()
    if(-not [ShotKitMemorySmoke.Native]::GetClientRect($toolbar[0].H,[ref]$tr)){throw 'TOOLBAR_CLIENT_RECT_FAILED.'}
    $availableW=$client.Right; $availableH=$client.Bottom-$tr.Bottom
    $fit=[Math]::Min($availableW/6000.0,$availableH/4000.0)
    $displayW=[int][Math]::Round(6000*$fit);$displayH=[int][Math]::Round(4000*$fit)
    $viewX=[int][Math]::Floor(($availableW-$displayW)/2.0);$viewY=$tr.Bottom+[int][Math]::Floor(($availableH-$displayH)/2.0)
    $origin=[ShotKitMemorySmoke.Native+Point]::new()
    if(-not [ShotKitMemorySmoke.Native]::ClientToScreen($Editor,[ref]$origin)){throw 'EDITOR_COORDINATE_CONVERSION_FAILED.'}
    $sx=$origin.X+$viewX+[int][Math]::Round($displayW*0.10);$sy=$origin.Y+$viewY+[int][Math]::Round($displayH*0.10)
    $ex=$origin.X+$viewX+[int][Math]::Round($displayW*0.90);$ey=$origin.Y+$viewY+[int][Math]::Round($displayH*0.90)
    if(-not [ShotKitMemorySmoke.Native]::SetForegroundWindow($Editor)){throw 'EDITOR_FOCUS_FAILED.'}
    [void](Wait-For { [ShotKitMemorySmoke.Native]::GetForegroundWindow() -eq $Editor } 'editor foreground')
    if(-not [ShotKitMemorySmoke.Native]::SetCursorPos($sx,$sy)){throw 'CROP_START_POSITION_FAILED.'}
    $cursor=[ShotKitMemorySmoke.Native+Point]::new();[void][ShotKitMemorySmoke.Native]::GetCursorPos([ref]$cursor)
    $under=[ShotKitMemorySmoke.Native]::WindowFromPoint($cursor)
    if([ShotKitMemorySmoke.Native]::OwnerPid($under) -ne $Pid){throw 'INPUT_TARGET_MISMATCH: crop start is not owned by the launched ShotKit process.'}
    [ShotKitMemorySmoke.Native]::mouse_event(2,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$true;Start-Sleep -Milliseconds 100
    [void][ShotKitMemorySmoke.Native]::SetCursorPos($ex,$ey);Start-Sleep -Milliseconds 200
    [ShotKitMemorySmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero);$script:mouseDown=$false
}
function Stop-OwnedProcess {
    if($null -eq $process){return}
    if(-not $process.HasExited){foreach($window in [ShotKitMemorySmoke.Native]::Windows($process.Id)){[void][ShotKitMemorySmoke.Native]::PostMessageW($window.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)};if(-not $process.WaitForExit(5000)){$process.Kill();[void]$process.WaitForExit(5000)}}
    $script:processStopped=$process.HasExited
    $process.Dispose();$script:process=$null
    if(-not $script:processStopped){throw 'OWNED_PROCESS_CLEANUP_FAILED.'}
}

try {
    New-Item -ItemType Directory -Path $root,$appData,$localAppData,$settingsDir -Force | Out-Null
    $settings=@{version=1;save_folder=$null;open_editor=$true;copy_to_clipboard=$false;auto_save=$false;scroll_direction='down'}|ConvertTo-Json -Compress
    [IO.File]::WriteAllText((Join-Path $settingsDir 'settings.json'),$settings,[Text.UTF8Encoding]::new($false))
    Write-LargeBmp $fixturePath
    $env:APPDATA=$appData;$env:LOCALAPPDATA=$localAppData
    try {$process=Start-Process -FilePath $ExePath -ArgumentList ('"{0}"' -f $fixturePath) -WorkingDirectory $root -PassThru;$processStopped=$false}
    finally {$env:APPDATA=$oldAppData;$env:LOCALAPPDATA=$oldLocalAppData}
    [ShotKitMemorySmoke.Sampler]::Start($process.Id,$SampleIntervalMs)
    if(-not [ShotKitMemorySmoke.Sampler]::MarkAndWaitForSample('import_and_idle',10000)){throw 'NO_MEMORY_SAMPLE: import phase sampler did not return a sample.'}

    $notice=Wait-For {Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like '*Prototyp*'}|Select-Object -First 1} 'owned startup notice'
    if([ShotKitMemorySmoke.Native]::OwnerPid($notice.H) -ne $process.Id){throw 'STARTUP_NOTICE_OWNER_MISMATCH.'}
    if(-not [ShotKitMemorySmoke.Native]::PostMessageW($notice.H,0x0010,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'STARTUP_NOTICE_CLOSE_FAILED.'}
    [void](Wait-For {-not [ShotKitMemorySmoke.Native]::IsWindow($notice.H)} 'startup notice close' 10)
    # open_image_path sets current_path and refreshes the title only after read, decode, and display_image succeed.
    $editor=Wait-For {Get-OwnedWindows|Where-Object {$_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title.IndexOf($fixturePath,[StringComparison]::OrdinalIgnoreCase) -ge 0 -and -not $_.Title.StartsWith('* ')}|Select-Object -First 1} 'loaded BMP editor title readback'
    $editorHandle=$editor.H
    if([ShotKitMemorySmoke.Native]::OwnerPid($editorHandle) -ne $process.Id){throw 'EDITOR_OWNER_MISMATCH.'}
    if(-not [ShotKitMemorySmoke.Native]::SetForegroundWindow($editorHandle)){throw 'EDITOR_FOCUS_FAILED.'}
    if(-not [ShotKitMemorySmoke.Native]::PostMessageW($editorHandle,0x0111,[IntPtr]0x510B,[IntPtr]::Zero)){throw 'CROP_TOOL_ACTIVATION_FAILED.'}
    if(-not [ShotKitMemorySmoke.Sampler]::MarkAndWaitForSample('edit_and_history_commit',10000)){throw 'NO_MEMORY_SAMPLE: edit/history phase sampler did not return a sample.'}
    Drag-Crop $editorHandle $process.Id
    $editor=Wait-For {Get-OwnedWindows|Where-Object {$_.H -eq $editorHandle -and $_.Title.StartsWith('* ')}} 'undoable crop commit' 30

    if(-not [ShotKitMemorySmoke.Sampler]::MarkAndWaitForSample('undo_history',10000)){throw 'NO_MEMORY_SAMPLE: undo phase sampler did not return a sample.'}
    if(-not [ShotKitMemorySmoke.Native]::PostMessageW($editorHandle,0x0111,[IntPtr]0x5112,[IntPtr]::Zero)){throw 'UNDO_COMMAND_FAILED.'}
    [void](Wait-For {Get-OwnedWindows|Where-Object {$_.H -eq $editorHandle -and $_.Title.IndexOf($fixturePath,[StringComparison]::OrdinalIgnoreCase) -ge 0 -and -not $_.Title.StartsWith('* ')}} 'undo returned document to saved state' 30)
} catch { $failure=$_ }
finally {
    if($mouseDown){try{[ShotKitMemorySmoke.Native]::mouse_event(4,0,0,0,[UIntPtr]::Zero)}catch{}}
    try {$sampleStats=[ShotKitMemorySmoke.Sampler]::StopAndGet([ref]$sampleNames)} catch {if(-not $failure){$failure=$_}}
    if($null -ne $process){try{Stop-OwnedProcess}catch{$cleanupFailure=$_.Exception.Message}}
    if($processStopped -and (Test-Path -LiteralPath $root)){try{Remove-Item -LiteralPath $root -Recurse -Force}catch{$cleanupFailure='TEMPORARY_FIXTURE_CLEANUP_FAILED.'}}
    elseif(-not $processStopped){$cleanupFailure='TEMPORARY_FIXTURE_RETAINED: owned process did not stop.'}
}
if($failure){
    $message=$failure.Exception.Message
    if($root){$message=$message.Replace($root,'[temporary fixture]')}
    if($ExePath){$message=$message.Replace($ExePath,'[ShotKit executable]')}
    throw "SMOKE_FAILED: $message"
}
if($cleanupFailure){throw "CLEANUP_FAILED: $cleanupFailure"}
$requiredPhases=@('import_and_idle','edit_and_history_commit','undo_history')
foreach($phase in $requiredPhases){$index=[Array]::IndexOf($sampleNames,$phase);if($index -lt 0 -or $sampleStats[$index].Samples -lt 1){throw "NO_MEMORY_SAMPLES: required phase '$phase' has no sample."}}

 $exeHash=(Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash
Write-Host "MEASUREMENTS: format=BMP dimensions=6000x4000 input_bytes=72000054 sample_interval_ms=$SampleIntervalMs windows_version=$([Environment]::OSVersion.Version) exe_sha256=$exeHash"
Write-Host 'PrivateBytesMax is the largest sampled PROCESS_MEMORY_COUNTERS_EX.PrivateUsage value; brief peaks between samples can be missed.'
Write-Host 'process_peak_working_set_bytes is the native lifetime PeakWorkingSetSize observed during each phase; it is cumulative across phases and is resident-set peak, not total committed memory.'
for($i=0;$i -lt $sampleNames.Count;$i++){$s=$sampleStats[$i];Write-Host ('PHASE {0} samples={1} max_sampled_private_bytes={2} last_private_bytes={3} process_peak_working_set_bytes={4}' -f $sampleNames[$i],$s.Samples,$s.PrivateBytesMax,$s.PrivateBytesLast,$s.PeakWorkingSetBytesMax)}
Write-Host 'PASS: real GUI import, crop commit, and Undo completed; measurements describe this build, Windows host, BMP fixture, and workload only. No memory threshold is asserted.'

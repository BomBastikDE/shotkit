[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $TimeoutSeconds = 30
)

# Isolated, opt-in invocation on an ephemeral GitHub-hosted Windows runner:
# pwsh -File tests/windows-mapped-drive-smoke.ps1 -ExePath target/release/shotkit.exe
# Requires elevated admin, an interactive desktop and already-running local SMB.
# Never starts services, changes firewall/authentication settings, uses credentials,
# remaps a drive or touches a pre-existing share. Missing prerequisites FAIL.
# Native UI enumeration and per-child isolated profiles follow windows-instance-smoke.ps1.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'startup-dialog-assertions.ps1')
if ($PSVersionTable.PSVersion.Major -lt 7 -or $env:OS -ne 'Windows_NT') {
    throw 'WINDOWS_PWSH_REQUIRED: PowerShell 7 on Windows is required.'
}
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'HOSTED_RUNNER_REQUIRED: this test only permits an ephemeral GitHub-hosted Windows runner.'
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'SMB_ADMIN_REQUIRED: elevated administrator rights are required to create the temporary local share.'
}
foreach ($cmd in @('Get-SmbShare', 'New-SmbShare', 'Remove-SmbShare', 'Get-SmbShareAccess')) {
    $null = Get-Command $cmd -ErrorAction Stop
}
foreach ($service in @('LanmanServer', 'LanmanWorkstation')) {
    if ((Get-Service -Name $service).Status -ne 'Running') {
        throw "SMB_UNAVAILABLE: $service must already be running; no service was changed."
    }
}
$tcp = [Net.Sockets.TcpClient]::new()
try {
    if (-not $tcp.ConnectAsync('127.0.0.1', 445).Wait(3000) -or -not $tcp.Connected) {
        throw 'LOCAL_SMB_UNAVAILABLE: TCP 445 on loopback is not accepting connections.'
    }
} finally { $tcp.Dispose() }
if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) { throw 'RUNNER_TEMP_REQUIRED' }
# Reject UNC/device paths syntactically before any filesystem access to them.
$runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP)
$resolvedExe = [IO.Path]::GetFullPath($ExePath)
foreach ($path in @($runnerTemp, $resolvedExe)) {
    if ($path -notmatch '^[A-Za-z]:[\\/]') { throw 'LOCAL_PATH_REQUIRED: UNC and device paths are not permitted for the runner temp or EXE.' }
}
$runnerTemp = (Resolve-Path -LiteralPath $runnerTemp).Path
$resolvedExe = (Resolve-Path -LiteralPath $resolvedExe).Path
if ([IO.Path]::GetExtension($resolvedExe) -ine '.exe') { throw 'SHOTKIT_EXE_REQUIRED' }

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
namespace ShotKitMappedDriveSmoke {
 public sealed class Win { public IntPtr H; public string ClassName, Title; public bool Visible; public int Id; }
 public static class N {
  delegate bool EnumProc(IntPtr h, IntPtr p);
  [StructLayout(LayoutKind.Sequential, CharSet=CharSet.Unicode)]
  struct NetResource { public uint Scope, Type, DisplayType, Usage; public string LocalName, RemoteName, Comment, Provider; }
  [DllImport("mpr.dll", CharSet=CharSet.Unicode)] static extern int WNetAddConnection2W(ref NetResource n, string password, string user, uint flags);
  [DllImport("mpr.dll", CharSet=CharSet.Unicode)] public static extern int WNetCancelConnection2W(string name, uint flags, bool force);
  [DllImport("mpr.dll", CharSet=CharSet.Unicode)] static extern int WNetGetConnectionW(string local, StringBuilder remote, ref int length);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern uint QueryDosDeviceW(string name, StringBuilder target, int size);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] public static extern uint GetDriveTypeW(string root);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr OpenMutexW(uint access, bool inherit, string name);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr p);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll", SetLastError=true)] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
  [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern bool GetUserObjectInformationW(IntPtr h, int index, StringBuilder s, uint length, out uint needed);
  static string Text(IntPtr h, bool cls) { var b=new StringBuilder(4096); if(cls)GetClassNameW(h,b,b.Capacity);else GetWindowTextW(h,b,b.Capacity); return b.ToString(); }
  static Win Info(IntPtr h) { return new Win { H=h, ClassName=Text(h,true), Title=Text(h,false), Visible=IsWindowVisible(h), Id=GetDlgCtrlID(h) }; }
  public static Win[] Windows(int pid) { var a=new List<Win>(); EnumProc cb=(h,p)=>{uint id;GetWindowThreadProcessId(h,out id);if(pid==0||id==(uint)pid)a.Add(Info(h));return true;}; EnumWindows(cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
  public static Win[] Children(IntPtr parent) { var a=new List<Win>();EnumProc cb=(h,p)=>{a.Add(Info(h));return true;};EnumChildWindows(parent,cb,IntPtr.Zero);GC.KeepAlive(cb);return a.ToArray(); }
  public static string Desktop() { IntPtr d=OpenInputDesktop(0,false,1);if(d==IntPtr.Zero)return null;try{var s=new StringBuilder(256);uint n;return GetUserObjectInformationW(d,2,s,512,out n)?s.ToString():null;}finally{CloseDesktop(d);} }
  public static bool ExistingInstance() { IntPtr h=OpenMutexW(0x00100000,false,@"Local\ShotKitRustPrototype.RustOnly.SingleInstance.v1");if(h!=IntPtr.Zero){CloseHandle(h);return true;}int e=Marshal.GetLastWin32Error();if(e!=2)throw new Win32Exception(e);return false; }
  public static bool FreeLetter(string local) { var b=new StringBuilder(32768);int n=b.Capacity;int e=WNetGetConnectionW(local,b,ref n);if(e!=2250)return false;uint size=QueryDosDeviceW(local,b,b.Capacity);if(size!=0)return false;e=Marshal.GetLastWin32Error();if(e!=2)throw new Win32Exception(e);return true; }
  public static string Connection(string local) { var b=new StringBuilder(32768);int n=b.Capacity;int e=WNetGetConnectionW(local,b,ref n);if(e!=0)throw new Win32Exception(e);return b.ToString(); }
  public static int Map(string local,string remote) { var n=new NetResource {Type=1,LocalName=local,RemoteName=remote};return WNetAddConnection2W(ref n,null,null,4); }
 }
}
'@

if (-not [Environment]::UserInteractive -or [ShotKitMappedDriveSmoke.N]::Desktop() -cne 'Default') {
    throw 'NO_INTERACTIVE_DESKTOP: the active Default input desktop is required.'
}
foreach ($path in @($runnerTemp, $resolvedExe)) {
    if ([ShotKitMappedDriveSmoke.N]::GetDriveTypeW([IO.Path]::GetPathRoot($path)) -ne 3) {
        throw "LOCAL_FIXED_PATH_REQUIRED: test executable and runner temp must be on local fixed drives: $path"
    }
}
$appName = [IO.Path]::GetFileNameWithoutExtension($resolvedExe)
if ([ShotKitMappedDriveSmoke.N]::ExistingInstance() -or @(Get-Process -Name $appName -ErrorAction SilentlyContinue).Count -gt 0 -or
    @([ShotKitMappedDriveSmoke.N]::Windows(0) | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' }).Count -gt 0) {
    throw 'PREEXISTING_INSTANCE: refusing to interact with an existing ShotKit instance.'
}
$drive = $null
$psDriveNames = @(Get-PSDrive | ForEach-Object { $_.Name + ':' })
foreach ($code in 90..71) {
    $candidate = ([char]$code).ToString() + ':'
    if ($psDriveNames -notcontains $candidate -and [ShotKitMappedDriveSmoke.N]::FreeLetter($candidate)) { $drive = $candidate; break }
}
if ($null -eq $drive) { throw 'NO_FREE_DRIVE: no unused drive letter G: through Z: is available.' }
$token = [Guid]::NewGuid().ToString('N')
$shareName = 'ShotKitSmoke_' + $token
$description = 'ShotKit issue 55 isolated smoke ' + $token
$root = Join-Path $runnerTemp ('shotkit-mapped-drive-smoke-' + $token)
$sharePath = Join-Path $root 'share'
$remote = '\\localhost\' + $shareName
if ((Test-Path -LiteralPath $root) -or @(Get-SmbShare | Where-Object { $_.Name -ieq $shareName }).Count -gt 0) {
    throw 'RESOURCE_COLLISION: refusing to reuse existing resources.'
}
$profile = Join-Path $root 'profile'
$localProfile = Join-Path $root 'local-profile'
$temp = Join-Path $root 'temp'
$initialPath = Join-Path $root 'initial.png'
$fixturePath = Join-Path $sharePath 'mapped-fixture.png'
$mappedPath = $drive + '\mapped-fixture.png'
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$shareCreated = $false; $mappingCreated = $false; $rootCreated = $false
$failure = $null; $cleanupErrors = @(); $fixtureLock = $null

function Wait-Value([scriptblock]$Probe, [string]$What) {
    $end = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $end) {
        if ($primary -and $primary.HasExited) { throw "PRIMARY_EXITED while waiting for $What" }
        $v = & $Probe
        if ($v) { return $v }
        Start-Sleep -Milliseconds 100
    }
    throw "TIMEOUT: $What"
}
function Windows([Diagnostics.Process]$Process) {
    if (-not $Process.HasExited) { [ShotKitMappedDriveSmoke.N]::Windows($Process.Id) }
}
function Dialog([Diagnostics.Process]$Process, [string]$Title) {
    Windows $Process | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' -and $_.Title -like $Title } | Select-Object -First 1
}
function Click-Dialog([Diagnostics.Process]$Process, $Dialog, [int]$Id) {
    $button = Wait-Value { [ShotKitMappedDriveSmoke.N]::Children($Dialog.H) | Where-Object { $_.Visible -and $_.ClassName -eq 'Button' -and $_.Id -eq $Id } | Select-Object -First 1 } "dialog button $Id"
    if (-not [ShotKitMappedDriveSmoke.N]::PostMessageW($button.H, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'BUTTON_POST_FAILED' }
    $null = Wait-Value { @(Windows $Process | Where-Object { $_.H -eq $Dialog.H }).Count -eq 0 } 'dialog to close'
}
function Start-Owned([string]$Path) {
    $s = [Diagnostics.ProcessStartInfo]::new()
    $s.FileName = $resolvedExe; $s.WorkingDirectory = $root; $s.UseShellExecute = $false
    $s.Environment['APPDATA'] = $profile; $s.Environment['LOCALAPPDATA'] = $localProfile
    $s.Environment['TEMP'] = $temp; $s.Environment['TMP'] = $temp
    $s.ArgumentList.Add($Path)
    $p = [Diagnostics.Process]::new(); $p.StartInfo = $s
    if (-not $p.Start()) { throw 'PROCESS_START_FAILED' }
    $owned.Add($p)
    return $p
}
function Editor-Snapshot {
    @((Windows $primary | Where-Object { $_.ClassName -eq 'ShotKitRustEditor' } | Sort-Object { $_.H.ToInt64() } | ForEach-Object { "$($_.H):$($_.Title):$($_.Visible)" })) -join "`n"
}
function Assert-Warning($Warning) {
    $text = ([ShotKitMappedDriveSmoke.N]::Children($Warning.H) | ForEach-Object { $_.Title }) -join "`n"
    foreach ($required in @('Netzwerkserver', 'Anmeldeinformationen', $mappedPath)) {
        if ($text.IndexOf($required, [StringComparison]::OrdinalIgnoreCase) -lt 0) { throw "WARNING_TEXT_MISSING: $required; dialog text: $text" }
    }
    if ((Editor-Snapshot) -cne $baseline) { throw 'DOCUMENT_OPENED_BEFORE_CONSENT: editor set changed while the warning was open.' }
}
$primary = $null
try {
    $null = New-Item -ItemType Directory -Path $root
    $rootCreated = $true
    $null = New-Item -ItemType Directory -Path $sharePath, $profile, $localProfile, $temp
    Add-Type -AssemblyName System.Drawing
    foreach ($path in @($initialPath, $fixturePath)) {
        $bitmap = [Drawing.Bitmap]::new(37, 29)
        try { $bitmap.SetPixel(0, 0, [Drawing.Color]::Magenta); $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png) }
        finally { $bitmap.Dispose() }
    }
    # Only the current identity receives read access; no guest/Everyone or saved credentials.
    $null = New-SmbShare -Name $shareName -Path $sharePath -Description $description -ReadAccess $identity.Name -Temporary -CachingMode None -ErrorAction Stop
    $shareCreated = $true
    $access = @(Get-SmbShareAccess -Name $shareName)
    if ($access.Count -ne 1 -or $access[0].AccessControlType -ne 'Allow' -or $access[0].AccessRight -ne 'Read' -or
        ([Security.Principal.NTAccount]::new($access[0].AccountName)).Translate([Security.Principal.SecurityIdentifier]).Value -ne $identity.User.Value) {
        throw 'SHARE_ACCESS_MISMATCH: temporary share must grant only the current identity read access.'
    }
    if (@(Get-PSDrive | Where-Object { ($_.Name + ':') -ieq $drive }).Count -gt 0 -or -not [ShotKitMappedDriveSmoke.N]::FreeLetter($drive)) { throw 'DRIVE_RACE: chosen drive letter became occupied; refusing to replace it.' }
    $errorCode = [ShotKitMappedDriveSmoke.N]::Map($drive, $remote)
    if ($errorCode -ne 0) { throw "SMB_MAP_FAILED: Win32 $errorCode; no credentials, services or existing mappings will be changed." }
    $mappingCreated = $true
    if ([ShotKitMappedDriveSmoke.N]::Connection($drive) -ine $remote -or [ShotKitMappedDriveSmoke.N]::GetDriveTypeW($drive + '\') -ne 4) {
        throw 'DRIVE_REMOTE_REQUIRED: exact mapping and GetDriveTypeW=4 must both be proven.'
    }
    if ((Get-FileHash -LiteralPath $mappedPath).Hash -cne (Get-FileHash -LiteralPath $fixturePath).Hash) { throw 'MAPPED_FIXTURE_MISMATCH' }
    Write-Output "EVIDENCE: $drive => $remote; GetDriveTypeW=4 (DRIVE_REMOTE); mapped fixture hash matches local fixture."

    $primary = Start-Owned $initialPath
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $primary -GetWindows { Windows $primary }
    $null = Wait-Value { Windows $primary | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title.Contains($initialPath) } } 'initial editor'
    $baseline = Editor-Snapshot
    # Deny any successful image read during rejection. This does not instrument
    # network traffic or claim that no metadata/failed open was attempted.
    $fixtureLock = [IO.File]::Open($fixturePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    $decline = Start-Owned $mappedPath
    $warning = Wait-Value { Dialog $primary '*Netzwerkdatei aus zweiter Instanz*' } 'mapped-drive network warning for rejection'
    Assert-Warning $warning
    Click-Dialog $primary $warning 7
    $result = Wait-Value { Dialog $decline '*Datei nicht geöffnet*' } 'visible IPC rejection result'
    Click-Dialog $decline $result 2
    if (-not $decline.WaitForExit($TimeoutSeconds * 1000) -or $decline.ExitCode -ne 0) { throw 'DECLINE_EXIT_FAILED' }
    if ((Editor-Snapshot) -cne $baseline) { throw 'DECLINE_CHANGED_EDITORS: rejected file must not create or replace an editor.' }
    if (@(Windows $primary | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' }).Count -ne 0) { throw 'UNEXPECTED_DIALOG_AFTER_DECLINE' }
    $fixtureLock.Dispose(); $fixtureLock = $null

    $accept = Start-Owned $mappedPath
    $warning = Wait-Value { Dialog $primary '*Netzwerkdatei aus zweiter Instanz*' } 'mapped-drive network warning for consent'
    Assert-Warning $warning
    Click-Dialog $primary $warning 6
    if (-not $accept.WaitForExit($TimeoutSeconds * 1000) -or $accept.ExitCode -ne 0) { throw 'CONSENT_EXIT_FAILED' }
    $null = Wait-Value { Windows $primary | Where-Object { $_.Visible -and $_.ClassName -eq 'ShotKitRustEditor' -and $_.Title.Contains($mappedPath) } } 'editor showing the accepted mapped path'
    if (@(Windows $primary | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' }).Count -ne 0) { throw 'UNEXPECTED_DIALOG_AFTER_CONSENT' }
    Write-Output 'EVIDENCE: mapped-drive IPC warning contains path and credential warning; rejection preserved all editors; consent opened the mapped fixture and the sender exited successfully.'
} catch { $failure = $_ }
finally {
    if ($fixtureLock) { try { $fixtureLock.Dispose() } catch { $cleanupErrors += "Fixture lock: $_" } }
    foreach ($p in $owned) {
        try {
            if (-not $p.HasExited) { $p.Kill() }
            if (-not $p.WaitForExit(5000)) { throw 'owned process did not exit' }
        } catch { $cleanupErrors += "Owned process $($p.Id): $_" }
        finally { $p.Dispose() }
    }
    if ($mappingCreated) {
        try {
            if ([ShotKitMappedDriveSmoke.N]::Connection($drive) -ine $remote) { throw 'Mapping ownership changed; refusing to disconnect it.' }
            $errorCode = [ShotKitMappedDriveSmoke.N]::WNetCancelConnection2W($drive, 0, $false)
            if ($errorCode -ne 0) { throw "Exact non-forced mapping removal failed: Win32 $errorCode" }
            if (-not [ShotKitMappedDriveSmoke.N]::FreeLetter($drive)) { throw 'Mapping removal could not be confirmed.' }
            $mappingCreated = $false
        } catch { $cleanupErrors += "Mapping $drive : $_" }
    }
    if ($shareCreated) {
        try {
            $current = @(Get-SmbShare | Where-Object { $_.Name -ieq $shareName })
            if ($current.Count -ne 1 -or $current[0].Path -ine $sharePath -or $current[0].Description -cne $description) { throw 'Share ownership changed; refusing to remove it.' }
            $current[0] | Remove-SmbShare -Force -Confirm:$false -ErrorAction Stop
            if (@(Get-SmbShare | Where-Object { $_.Name -ieq $shareName }).Count -ne 0) { throw 'Share removal could not be confirmed.' }
            $shareCreated = $false
        } catch { $cleanupErrors += "Share $shareName : $_" }
    }
    # Retain local evidence on any cleanup failure; never delete data still shared.
    if ($rootCreated -and $cleanupErrors.Count -eq 0 -and -not $mappingCreated -and -not $shareCreated) {
        try {
            Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop
            if (Test-Path -LiteralPath $root) { throw 'Temporary root remains after removal.' }
        } catch { $cleanupErrors += "Temporary root: $_" }
    }
}
if ($cleanupErrors.Count -gt 0) {
    throw "CLEANUP_FAILED: $($cleanupErrors -join '; '); evidence retained at '$root'. Test failure: $failure"
}
if ($failure) { throw $failure }
Write-Output 'PASS: real DRIVE_REMOTE mapping, IPC rejection and consent verified; all owned processes, the exact mapping, temporary share and test files removed.'

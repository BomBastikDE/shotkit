[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $ExePath,
    [ValidateRange(10, 120)][int] $TimeoutSeconds = 30
)

# Issue #67, owner decision 2026-11-07: a save folder on a network drive is not
# checked for remnants at startup (no prompt, no server access). After the next
# successful save into that folder it is checked once per session.
#
# Isolated, opt-in invocation on an ephemeral GitHub-hosted Windows runner:
# pwsh -File tests/windows-temp-recovery-network-smoke.ps1 -ExePath target/release/shotkit.exe
# Requires elevated admin, an interactive desktop and already-running local SMB.
# Never starts services, changes firewall/authentication settings, uses credentials,
# remaps a drive or touches a pre-existing share. Missing prerequisites FAIL.
# Share setup and cleanup follow windows-mapped-drive-smoke.ps1.
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
namespace ShotKitTempRecoveryNetworkSmoke {
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
  [DllImport("user32.dll")] public static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode)] static extern IntPtr ReadText(IntPtr hwnd, uint message, IntPtr w, StringBuilder text, uint flags, uint timeout, out IntPtr result);
  [DllImport("user32.dll", SetLastError=true)] static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
  [DllImport("user32.dll")] static extern bool CloseDesktop(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern bool GetUserObjectInformationW(IntPtr h, int index, StringBuilder s, uint length, out uint needed);
  static string Text(IntPtr h, bool cls) {
   var b=new StringBuilder(8192);
   if(cls) { GetClassNameW(h,b,b.Capacity); return b.ToString(); }
   GetWindowTextW(h,b,b.Capacity);
   // GetWindowText cannot read another process's EDIT content; WM_GETTEXT is
   // a system message whose bounded output buffer Windows marshals.
   if(b.Length==0) { IntPtr r; ReadText(h,0x000d,new IntPtr(b.Capacity),b,2,1000,out r); }
   return b.ToString();
  }
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

if (-not [Environment]::UserInteractive -or [ShotKitTempRecoveryNetworkSmoke.N]::Desktop() -cne 'Default') {
    throw 'NO_INTERACTIVE_DESKTOP: the active Default input desktop is required.'
}
foreach ($path in @($runnerTemp, $resolvedExe)) {
    if ([ShotKitTempRecoveryNetworkSmoke.N]::GetDriveTypeW([IO.Path]::GetPathRoot($path)) -ne 3) {
        throw "LOCAL_FIXED_PATH_REQUIRED: test executable and runner temp must be on local fixed drives: $path"
    }
}
$appName = [IO.Path]::GetFileNameWithoutExtension($resolvedExe)
if ([ShotKitTempRecoveryNetworkSmoke.N]::ExistingInstance() -or @(Get-Process -Name $appName -ErrorAction SilentlyContinue).Count -gt 0 -or
    @([ShotKitTempRecoveryNetworkSmoke.N]::Windows(0) | Where-Object { $_.ClassName -eq 'ShotKitRustPrototype' }).Count -gt 0) {
    throw 'PREEXISTING_INSTANCE: refusing to interact with an existing ShotKit instance.'
}
$drive = $null
$psDriveNames = @(Get-PSDrive | ForEach-Object { $_.Name + ':' })
foreach ($code in 90..71) {
    $candidateLetter = ([char]$code).ToString() + ':'
    if ($psDriveNames -notcontains $candidateLetter -and [ShotKitTempRecoveryNetworkSmoke.N]::FreeLetter($candidateLetter)) { $drive = $candidateLetter; break }
}
if ($null -eq $drive) { throw 'NO_FREE_DRIVE: no unused drive letter G: through Z: is available.' }
$token = [Guid]::NewGuid().ToString('N')
$shareName = 'ShotKitRecovery_' + $token
$description = 'ShotKit issue 67 isolated smoke ' + $token
$root = Join-Path $runnerTemp ('shotkit-temp-recovery-network-' + $token)
$sharePath = Join-Path $root 'share'
$remote = '\\localhost\' + $shareName
if ((Test-Path -LiteralPath $root) -or @(Get-SmbShare | Where-Object { $_.Name -ieq $shareName }).Count -gt 0) {
    throw 'RESOURCE_COLLISION: refusing to reuse existing resources.'
}
$profile = Join-Path $root 'profile'
$localProfile = Join-Path $root 'local-profile'
$temp = Join-Path $root 'temp'
$candidateName = '.shotkit-4242-1700000000000000000-1.tmp'
$candidate = Join-Path $sharePath $candidateName
$young = Join-Path $sharePath '.shotkit-4242-1700000000000000000-2.tmp'
$decoy = Join-Path $sharePath 'x.shotkit-4242-1700000000000000000-3.tmp'
$owned = [Collections.Generic.List[Diagnostics.Process]]::new()
$shareCreated = $false; $mappingCreated = $false; $rootCreated = $false
$failure = $null; $cleanupErrors = @()

function Wait-Value([scriptblock]$Probe, [string]$What) {
    $end = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $end) {
        if ($script:current -and $script:current.HasExited) { throw "SHOTKIT_EXITED while waiting for $What" }
        $v = & $Probe
        if ($v) { return $v }
        Start-Sleep -Milliseconds 100
    }
    throw "TIMEOUT: $What"
}
function Windows([Diagnostics.Process]$Process) {
    if (-not $Process.HasExited) { [ShotKitTempRecoveryNetworkSmoke.N]::Windows($Process.Id) }
}
function Dialogs([Diagnostics.Process]$Process) {
    @(Windows $Process | Where-Object { $_.Visible -and $_.ClassName -eq '#32770' })
}
function Dialog-Text($Dialog) {
    ([ShotKitTempRecoveryNetworkSmoke.N]::Children($Dialog.H) | ForEach-Object { $_.Title }) -join "`n"
}
function Start-Owned {
    $s = [Diagnostics.ProcessStartInfo]::new()
    $s.FileName = $resolvedExe; $s.WorkingDirectory = $root; $s.UseShellExecute = $false
    $s.Environment['APPDATA'] = $profile; $s.Environment['LOCALAPPDATA'] = $localProfile
    $s.Environment['TEMP'] = $temp; $s.Environment['TMP'] = $temp
    $p = [Diagnostics.Process]::new(); $p.StartInfo = $s
    if (-not $p.Start()) { throw 'PROCESS_START_FAILED' }
    $owned.Add($p)
    $script:current = $p
    return $p
}
function Stop-Owned([Diagnostics.Process]$Process) {
    if (-not $Process.HasExited) { $Process.Kill() }
    if (-not $Process.WaitForExit(5000)) { throw 'OWNED_PROCESS_DID_NOT_EXIT' }
    $script:current = $null
}
function Saved-Pngs { @(Get-ChildItem -LiteralPath $sharePath -Force -File -Filter 'ShotKit-*.png') }
# F9 = screen under the mouse; with auto_save on and the editor off, ShotKit
# saves a PNG into the network save folder and reports it in a message box.
function Capture-AndConfirm([Diagnostics.Process]$Process, [int]$ExpectedPngs) {
    [ShotKitTempRecoveryNetworkSmoke.N]::keybd_event(0x78, 0, 0, [UIntPtr]::Zero)
    [ShotKitTempRecoveryNetworkSmoke.N]::keybd_event(0x78, 0, 2, [UIntPtr]::Zero)
    $notice = Wait-Value { Dialogs $Process | Where-Object { (Dialog-Text $_).Contains('Aufnahme gespeichert') } | Select-Object -First 1 } 'saved-capture notice'
    $text = Dialog-Text $notice
    if (-not $text.Contains($drive)) { throw "CAPTURE_NOT_ON_NETWORK_DRIVE: $text" }
    if ((Saved-Pngs).Count -ne $ExpectedPngs) { throw "SAVED_PNG_COUNT: expected $ExpectedPngs, got $((Saved-Pngs).Count)" }
    $button = Wait-Value { [ShotKitTempRecoveryNetworkSmoke.N]::Children($notice.H) | Where-Object { $_.Visible -and $_.ClassName -eq 'Button' } | Select-Object -First 1 } 'notice button'
    if (-not [ShotKitTempRecoveryNetworkSmoke.N]::PostMessageW($button.H, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)) { throw 'NOTICE_CLICK_FAILED' }
    $null = Wait-Value { @(Windows $Process | Where-Object { $_.H -eq $notice.H -and $_.Visible }).Count -eq 0 } 'notice to close'
}
function Recovery-Dialog([Diagnostics.Process]$Process) {
    Dialogs $Process | Where-Object { $_.Title -eq 'Temporäre Dateien nach einem Abbruch' } | Select-Object -First 1
}
function Assert-NoDialogFor([Diagnostics.Process]$Process, [int]$Seconds, [string]$Phase) {
    $end = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $end) {
        $open = Dialogs $Process
        if ($open.Count -gt 0) { throw "UNEXPECTED_DIALOG ($Phase): $(($open | ForEach-Object { $_.Title }) -join ', ')" }
        Start-Sleep -Milliseconds 100
    }
}
function Choose-Recovery([Diagnostics.Process]$Process, [int]$Choice, [string]$Phase) {
    $dialog = Wait-Value { Recovery-Dialog $Process } "recovery dialog ($Phase)"
    $text = Dialog-Text $dialog
    foreach ($required in @('1 Dateien', $candidateName, $drive, 'Netzwerkordner')) {
        if (-not $text.Contains($required)) { throw "RECOVERY_TEXT_MISSING ($Phase): '$required' in: $text" }
    }
    if ($text.Contains('-2.tmp') -or $text.Contains('x.shotkit')) { throw "RECOVERY_OFFERED_CONTROL ($Phase): $text" }
    # WM_COMMAND/BN_CLICKED, only to the dialog belonging to our process.
    if (-not [ShotKitTempRecoveryNetworkSmoke.N]::PostMessageW($dialog.H, 0x0111, [IntPtr]$Choice, [IntPtr]::Zero)) { throw 'RECOVERY_CLICK_FAILED' }
    $null = Wait-Value { (Dialogs $Process).Count -eq 0 } "recovery dialog to close ($Phase)"
}
function Assert-Controls([string]$Phase) {
    foreach ($path in @($young, $decoy)) {
        if (-not [IO.File]::Exists($path)) { throw "CONTROL_REMOVED ($Phase): $path" }
    }
}

$script:current = $null
try {
    $null = New-Item -ItemType Directory -Path $root
    $rootCreated = $true
    $null = New-Item -ItemType Directory -Path $sharePath, $profile, $localProfile, $temp
    # Only the current identity receives change access; no guest/Everyone or saved credentials.
    $null = New-SmbShare -Name $shareName -Path $sharePath -Description $description -ChangeAccess $identity.Name -Temporary -CachingMode None -ErrorAction Stop
    $shareCreated = $true
    $access = @(Get-SmbShareAccess -Name $shareName)
    if ($access.Count -ne 1 -or $access[0].AccessControlType -ne 'Allow' -or $access[0].AccessRight -ne 'Change' -or
        ([Security.Principal.NTAccount]::new($access[0].AccountName)).Translate([Security.Principal.SecurityIdentifier]).Value -ne $identity.User.Value) {
        throw 'SHARE_ACCESS_MISMATCH: temporary share must grant only the current identity change access.'
    }
    if (@(Get-PSDrive | Where-Object { ($_.Name + ':') -ieq $drive }).Count -gt 0 -or -not [ShotKitTempRecoveryNetworkSmoke.N]::FreeLetter($drive)) { throw 'DRIVE_RACE: chosen drive letter became occupied; refusing to replace it.' }
    $errorCode = [ShotKitTempRecoveryNetworkSmoke.N]::Map($drive, $remote)
    if ($errorCode -ne 0) { throw "SMB_MAP_FAILED: Win32 $errorCode; no credentials, services or existing mappings will be changed." }
    $mappingCreated = $true
    if ([ShotKitTempRecoveryNetworkSmoke.N]::Connection($drive) -ine $remote -or [ShotKitTempRecoveryNetworkSmoke.N]::GetDriveTypeW($drive + '\') -ne 4) {
        throw 'DRIVE_REMOTE_REQUIRED: exact mapping and GetDriveTypeW=4 must both be proven.'
    }

    # Synthetic remnant (exact name, older than 10 minutes) plus a young and a
    # foreign-named control that must never be offered or removed.
    foreach ($path in @($candidate, $young, $decoy)) { [IO.File]::WriteAllText($path, 'synthetic recovery fixture') }
    foreach ($path in @($candidate, $decoy)) { [IO.File]::SetLastWriteTimeUtc($path, [DateTime]::UtcNow.AddMinutes(-11)) }

    # Existing settings: no first-start notice. Save folder = mapped network drive.
    $settingsDir = Join-Path $profile 'ShotKitRust'
    $null = New-Item -ItemType Directory -Path $settingsDir
    $settings = [ordered]@{
        version = 1; save_folder = $drive + '\'; open_editor = $false; copy_to_clipboard = $false
        exclude_from_clipboard_history = $true; auto_save = $true; scroll_direction = 'down'
    }
    [IO.File]::WriteAllText((Join-Path $settingsDir 'settings.json'), ($settings | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
    Write-Output "EVIDENCE: $drive => $remote; GetDriveTypeW=4 (DRIVE_REMOTE); save folder and remnant fixture on the mapped drive."

    # Session 1: startup must neither prompt nor check the network folder.
    $first = Start-Owned
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $first -GetWindows { Windows $first }
    Assert-NoDialogFor $first 3 'after startup'
    if (-not [IO.File]::Exists($candidate)) { throw 'STARTUP_REMOVED_REMNANT' }
    Write-Output 'EVIDENCE: startup with a network save folder showed no dialog (no network prompt, no recovery prompt).'

    # First save into the network folder: recovery is offered once; "Später" keeps the file.
    Capture-AndConfirm $first 1
    Choose-Recovery $first 102 'after first save, Later'
    if (-not [IO.File]::Exists($candidate)) { throw 'LATER_REMOVED_REMNANT' }
    Assert-Controls 'after Later'

    # Second save in the same session: no further prompt for the same folder.
    Capture-AndConfirm $first 2
    Assert-NoDialogFor $first 3 'after second save in the same session'
    if (-not [IO.File]::Exists($candidate)) { throw 'SECOND_SAVE_REMOVED_REMNANT' }
    Write-Output 'EVIDENCE: the first save offered the remnant once; a second save in the same session did not prompt again.'
    Stop-Owned $first

    # Session 2: again no startup prompt; after a save, "Löschen" removes only the remnant.
    $second = Start-Owned
    Assert-NoStartupDialogs -TimeoutSeconds $TimeoutSeconds -OwnedProcess $second -GetWindows { Windows $second }
    Assert-NoDialogFor $second 3 'after second startup'
    Capture-AndConfirm $second 3
    Choose-Recovery $second 100 'after save in a new session, Delete'
    $null = Wait-Value { -not [IO.File]::Exists($candidate) } 'remnant removal'
    Assert-Controls 'after Delete'
    if ((Saved-Pngs).Count -ne 3) { throw 'DELETE_CHANGED_SAVED_CAPTURES' }
    Stop-Owned $second
    Write-Output 'EVIDENCE: a new session offered the remnant again after a save; Delete removed only the remnant; controls and saved captures remain.'
} catch { $failure = $_ }
finally {
    foreach ($p in $owned) {
        try {
            if (-not $p.HasExited) { $p.Kill() }
            if (-not $p.WaitForExit(5000)) { throw 'owned process did not exit' }
        } catch { $cleanupErrors += "Owned process $($p.Id): $_" }
        finally { $p.Dispose() }
    }
    if ($mappingCreated) {
        try {
            if ([ShotKitTempRecoveryNetworkSmoke.N]::Connection($drive) -ine $remote) { throw 'Mapping ownership changed; refusing to disconnect it.' }
            $errorCode = [ShotKitTempRecoveryNetworkSmoke.N]::WNetCancelConnection2W($drive, 0, $false)
            if ($errorCode -ne 0) { throw "Exact non-forced mapping removal failed: Win32 $errorCode" }
            if (-not [ShotKitTempRecoveryNetworkSmoke.N]::FreeLetter($drive)) { throw 'Mapping removal could not be confirmed.' }
            $mappingCreated = $false
        } catch { $cleanupErrors += "Mapping $drive : $_" }
    }
    if ($shareCreated) {
        try {
            $existing = @(Get-SmbShare | Where-Object { $_.Name -ieq $shareName })
            if ($existing.Count -ne 1 -or $existing[0].Path -ine $sharePath -or $existing[0].Description -cne $description) { throw 'Share ownership changed; refusing to remove it.' }
            $existing[0] | Remove-SmbShare -Force -Confirm:$false -ErrorAction Stop
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
Write-Output 'PASS: network save folder is not checked at startup; the first save per session offers remnants once; Delete removes only the remnant; share, mapping and files removed.'

# The caller passes only the unique, test-owned profile root. A transient lock
# may remain briefly after the verified Edge processes have exited; a persistent
# error is still reported to the caller after the bounded attempts.
function Remove-EdgeTestTree {
    param(
        [Parameter(Mandatory = $true)][string] $Path,
        [ValidateRange(1, 20)][int] $MaxAttempts = 10,
        [ValidateRange(0, 1000)][int] $DelayMilliseconds = 200,
        [scriptblock] $OnRetry = $null
    )
    $root = [IO.Path]::GetFullPath($Path).TrimEnd([IO.Path]::DirectorySeparatorChar)
    $temp = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([IO.Path]::DirectorySeparatorChar)
    if (-not [string]::Equals([IO.Path]::GetDirectoryName($root), $temp, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($root) -cnotmatch '^ShotKit-Browser-Scroll-[0-9a-f]{32}$') {
        throw 'Refusing to clean a path outside a uniquely named Edge test root.'
    }
    for ($attempt = 1; $attempt -le $MaxAttempts; $attempt++) {
        try {
            $rootItem = Get-Item -LiteralPath $root -Force -ErrorAction Stop
            if (-not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw 'Refusing to clean a non-directory or reparse-point test root.'
            }
            $links = @(Get-ChildItem -LiteralPath $root -Force -Recurse -ErrorAction Stop | Where-Object { $_.Attributes -band [IO.FileAttributes]::ReparsePoint })
            if ($links.Count -ne 0) { throw 'Refusing to clean a test tree containing a reparse point.' }
            Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction Stop
            return
        }
        catch {
            if ($attempt -eq $MaxAttempts) { throw }
            if ($null -ne $OnRetry) { & $OnRetry $attempt }
            if ($DelayMilliseconds -gt 0) { Start-Sleep -Milliseconds $DelayMilliseconds }
        }
    }
}

# Use Windows' argument parser rather than substring/regex matching. This is
# intentionally conservative: only one canonical --user-data-dir= argument
# with the literal owned path is accepted. Alternate switch spellings and
# duplicate profile switches are ambiguous and never establish ownership.
# Chromium also trims arguments and supports --single-argument; reject these
# noncanonical forms rather than infer ownership from a different interpretation.
# https://chromium.googlesource.com/chromium/src/+/main/base/command_line.cc
# CommandLineToArgvW allocations must be freed by LocalFree:
# https://learn.microsoft.com/windows/win32/api/shellapi/nf-shellapi-commandlinetoargvw
if (-not ('ShotKitEdgeCleanup.CommandLine' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
namespace ShotKitEdgeCleanup {
    public static class CommandLine {
        [DllImport("shell32.dll", CharSet=CharSet.Unicode, SetLastError=true, ExactSpelling=true)]
        private static extern IntPtr CommandLineToArgvW(string commandLine, out int count);
        [DllImport("kernel32.dll", SetLastError=true, ExactSpelling=true)]
        private static extern IntPtr LocalFree(IntPtr memory);
        public static bool HasExactProfile(string commandLine, string ownedProfile) {
            if (String.IsNullOrWhiteSpace(commandLine) || String.IsNullOrEmpty(ownedProfile) || commandLine.IndexOf('\0') >= 0) return false;
            int count;
            IntPtr arguments = CommandLineToArgvW(commandLine, out count);
            if (arguments == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
            try {
                if (count < 2) return false;
                string executable = Marshal.PtrToStringUni(Marshal.ReadIntPtr(arguments));
                if (String.IsNullOrEmpty(executable)) return false;
                bool found = false;
                for (int index = 1; index < count; index++) {
                    string argument = Marshal.PtrToStringUni(Marshal.ReadIntPtr(arguments, checked(index * IntPtr.Size)));
                    if (argument == null || argument != argument.Trim()) return false;
                    if (argument == "--") break; // subsequent arguments are positional
                    string switchText;
                    if (argument.StartsWith("--", StringComparison.Ordinal)) switchText = argument.Substring(2);
                    else if (argument.StartsWith("-", StringComparison.Ordinal) || argument.StartsWith("/", StringComparison.Ordinal)) switchText = argument.Substring(1);
                    else continue;
                    int equals = switchText.IndexOf('=');
                    string name = equals < 0 ? switchText : switchText.Substring(0, equals);
                    if (name.Equals("single-argument", StringComparison.OrdinalIgnoreCase)) return false;
                    if (!name.Equals("user-data-dir", StringComparison.OrdinalIgnoreCase)) continue;
                    if (found || !argument.StartsWith("--user-data-dir=", StringComparison.Ordinal)) return false;
                    if (!argument.Substring("--user-data-dir=".Length).Equals(ownedProfile, StringComparison.Ordinal)) return false;
                    found = true;
                }
                return found;
            }
            finally {
                if (LocalFree(arguments) != IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
            }
        }
    }
}
'@
}

# Discover only the caller's unique profile. Retain an opened OS handle before
# the second CIM check; System.Diagnostics.Process otherwise opens it lazily.
# OpenProcess is injectable for deterministic lifecycle-race contract tests.
function Get-ProfileEdgeProcesses {
    param(
        [scriptblock] $OpenProcess = { param($ProcessId) [Diagnostics.Process]::GetProcessById($ProcessId) },
        [switch] $IncludeUnverifiedSnapshots
    )
    $verified = [Collections.Generic.List[object]]::new()
    $candidates = @(Get-CimInstance Win32_Process -Filter "Name='msedge.exe'" -ErrorAction Stop |
        Where-Object { [ShotKitEdgeCleanup.CommandLine]::HasExactProfile($_.CommandLine, $edgeProfile) })
    foreach ($candidate in $candidates) {
        if ($null -eq $candidate.CreationDate) { throw 'Cannot verify an Edge process without a creation date.' }
        $known = $edgeOwnedHandles | Where-Object {
            $_.ProcessId -eq [int]$candidate.ProcessId -and $_.CreationDate -eq $candidate.CreationDate
        } | Select-Object -First 1
        if ($null -ne $known) {
            if (-not $known.Handle.HasExited) { [void]$verified.Add($candidate) }
            continue
        }
        $handle = $null
        try {
            try {
                $handle = & $OpenProcess ([int]$candidate.ProcessId)
                $nativeHandle = $handle.Handle # acquire and retain before revalidation
                # PowerShell property access can suppress a throwing getter and
                # return null. Never revalidate or register without a held handle.
                if ($null -eq $nativeHandle -or $nativeHandle -eq [IntPtr]::Zero) {
                    throw 'Cannot acquire a native handle for the profile-owned Edge candidate.'
                }
            }
            catch {
                # GetProcessById reports ArgumentException for an exited PID.
                # CIM may still return that stale row: never register or kill it.
                if ($_.Exception -is [ArgumentException] -or $_.Exception.InnerException -is [ArgumentException] -or
                    ($null -ne $handle -and $handle.HasExited)) { continue }
                $latest = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.ProcessId)" -ErrorAction Stop
                if ($null -eq $latest -or $latest.CreationDate -ne $candidate.CreationDate) { continue }
                throw
            }
            $confirmed = Get-CimInstance Win32_Process -Filter "ProcessId=$($candidate.ProcessId)" -ErrorAction Stop
            if ($null -eq $confirmed -or $confirmed.CreationDate -ne $candidate.CreationDate -or
                $confirmed.Name -ne 'msedge.exe' -or
                -not [ShotKitEdgeCleanup.CommandLine]::HasExactProfile($confirmed.CommandLine, $edgeProfile)) { continue }
            try {
                if ($handle.HasExited) { continue }
                if ($handle.ProcessName -ne 'msedge') { throw "Owned Edge process identity mismatch for PID $($candidate.ProcessId)." }
            }
            catch { if ($handle.HasExited) { continue }; throw }
            [void]$edgeOwnedHandles.Add([pscustomobject]@{
                ProcessId=[int]$candidate.ProcessId; CreationDate=$candidate.CreationDate; Handle=$handle
            })
            [void]$verified.Add($candidate)
            $handle = $null
        }
        finally { if ($null -ne $handle) { $handle.Dispose() } }
    }
    # The caller must rescan after termination. A stale row is not proof that
    # cleanup succeeded; persisting candidates remain a bounded cleanup failure.
    if ($IncludeUnverifiedSnapshots) { return $candidates }
    return $verified.ToArray()
}

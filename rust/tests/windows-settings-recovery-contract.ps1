[CmdletBinding()]
param([string] $ExePath)

# Experimental Issue #26 contract probe. Run manually with pwsh -File <this file>.
# Does not launch ShotKit or read
# its settings. All filesystem mutations stay in one fresh temporary directory.
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { throw 'Windows is required for this native filesystem contract probe.' }

if (-not ('ShotKitSettingsRecoveryContract.Native' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;
namespace ShotKitSettingsRecoveryContract {
    public static class Native {
        public const uint Read = 0x80000000, Write = 0x40000000, Delete = 0x00010000;
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern SafeFileHandle CreateFileW(string path, uint access, uint share,
            IntPtr security, uint disposition, uint attributes, IntPtr template);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool SetFileInformationByHandle(SafeFileHandle handle, int infoClass,
            IntPtr info, uint size);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool SetFilePointerEx(SafeFileHandle handle, long offset, out long position, uint method);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool ReadFile(SafeFileHandle handle, byte[] bytes, uint size, out uint read, IntPtr overlapped);

        public static SafeFileHandle OpenProtected(string path) {
            // FILE_SHARE_READ only: competing writers and deleters must fail.
            SafeFileHandle h = CreateFileW(path, Read | Delete, 1, IntPtr.Zero, 3, 0x80, IntPtr.Zero);
            int error = Marshal.GetLastWin32Error();
            if (h.IsInvalid) { h.Dispose(); throw new Win32Exception(error, "Open protected settings fixture"); }
            return h;
        }
        public static int CompetingOpenError(string path, uint access, uint disposition) {
            using (SafeFileHandle h = CreateFileW(path, access, 7, IntPtr.Zero, disposition, 0x80, IntPtr.Zero)) {
                int error = Marshal.GetLastWin32Error();
                return h.IsInvalid ? error : 0;
            }
        }
        public static int RenameWithoutReplace(SafeFileHandle handle, string target) {
            byte[] name = Encoding.Unicode.GetBytes(target);
            // FILE_RENAME_INFO: BOOLEAN, pointer-aligned HANDLE, DWORD, WCHAR[].
            int rootOffset = IntPtr.Size == 8 ? 8 : 4;
            int lengthOffset = rootOffset + IntPtr.Size;
            int nameOffset = lengthOffset + 4;
            int size = nameOffset + name.Length;
            IntPtr buffer = Marshal.AllocHGlobal(size);
            try {
                Marshal.Copy(new byte[size], 0, buffer, size); // ReplaceIfExists = FALSE; RootDirectory = NULL.
                Marshal.WriteInt32(buffer, lengthOffset, name.Length);
                Marshal.Copy(name, 0, IntPtr.Add(buffer, nameOffset), name.Length);
                bool ok = SetFileInformationByHandle(handle, 3, buffer, (uint)size); // FileRenameInfo
                int error = Marshal.GetLastWin32Error();
                return ok ? 0 : error;
            } finally { Marshal.FreeHGlobal(buffer); }
        }
        public static string ReadHeld(SafeFileHandle handle) {
            long position;
            if (!SetFilePointerEx(handle, 0, out position, 0)) throw new Win32Exception(Marshal.GetLastWin32Error());
            byte[] bytes = new byte[4096];
            uint read;
            if (!ReadFile(handle, bytes, (uint)bytes.Length, out read, IntPtr.Zero))
                throw new Win32Exception(Marshal.GetLastWin32Error());
            return Encoding.UTF8.GetString(bytes, 0, (int)read);
        }
    }
}
'@
}

function Assert-Contract([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw "CONTRACT_FAILED: $Message" }
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('shotkit-settings-contract-' + [Guid]::NewGuid().ToString('N'))
$held = $null
$created = $false
$evidence = [ordered]@{}
try {
    # GUID directory; refuse to touch anything if it already exists.
    Assert-Contract (-not [IO.Directory]::Exists($testRoot)) 'Temporary test directory already exists.'
    [void][IO.Directory]::CreateDirectory($testRoot)
    $created = $true
    $source = Join-Path $testRoot 'settings.json'
    $collision = Join-Path $testRoot 'settings.recovery-existing.json'
    $backup = Join-Path $testRoot 'settings.recovery-new.json'
    $oldData = '{"fixture":"original-invalid-settings"}'
    $collisionData = '{"fixture":"existing-recovery"}'
    $newData = '{"fixture":"separate-new-settings"}'
    $utf8 = [Text.UTF8Encoding]::new($false)
    [IO.File]::WriteAllText($source, $oldData, $utf8)
    [IO.File]::WriteAllText($collision, $collisionData, $utf8)
    $held = [ShotKitSettingsRecoveryContract.Native]::OpenProtected($source)
    Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($held) -ceq $oldData) 'Protected handle cannot read original bytes.'

    $writeError = [ShotKitSettingsRecoveryContract.Native]::CompetingOpenError($source, [ShotKitSettingsRecoveryContract.Native]::Write, 3)
    $deleteError = [ShotKitSettingsRecoveryContract.Native]::CompetingOpenError($source, [ShotKitSettingsRecoveryContract.Native]::Delete, 3)
    Assert-Contract ($writeError -eq 32) "Competing writer must fail with ERROR_SHARING_VIOLATION (32), got $writeError."
    Assert-Contract ($deleteError -eq 32) "Competing delete access must fail with ERROR_SHARING_VIOLATION (32), got $deleteError."
    Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($held) -ceq $oldData) 'Original bytes changed after competing access.'
    $evidence.competingWriterError = $writeError
    $evidence.competingDeleteError = $deleteError

    $collisionError = [ShotKitSettingsRecoveryContract.Native]::RenameWithoutReplace($held, $collision)
    Assert-Contract ($collisionError -in @(80, 183)) "Existing target must fail with FILE_EXISTS/ALREADY_EXISTS, got $collisionError."
    Assert-Contract ([IO.File]::Exists($source)) 'Collision removed the original path.'
    Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($held) -ceq $oldData) 'Collision changed held original bytes.'
    Assert-Contract ([IO.File]::ReadAllText($collision, $utf8) -ceq $collisionData) 'Collision overwrote existing recovery bytes.'
    $evidence.existingRecoveryCollisionError = $collisionError

    $renameError = [ShotKitSettingsRecoveryContract.Native]::RenameWithoutReplace($held, $backup)
    Assert-Contract ($renameError -eq 0) "Rename through the still-held handle failed: Win32 $renameError."
    Assert-Contract (-not [IO.File]::Exists($source)) 'Successful rename retained the old path.'
    Assert-Contract ([IO.File]::Exists($backup)) 'Successful rename did not create the recovery path.'
    Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($held) -ceq $oldData) 'Rename changed held bytes.'
    $evidence.renameWithHeldHandle = $true

    # Simulate another actor creating fresh settings after the original was moved.
    $fresh = [IO.File]::Open($source, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try {
        $newBytes = $utf8.GetBytes($newData)
        $fresh.Write($newBytes, 0, $newBytes.Length)
    } finally { $fresh.Dispose() }
    $recreatedError = [ShotKitSettingsRecoveryContract.Native]::RenameWithoutReplace($held, $source)
    Assert-Contract ($recreatedError -in @(80, 183)) "Recreated original target must reject replacement, got $recreatedError."
    $createError = [ShotKitSettingsRecoveryContract.Native]::CompetingOpenError($source, [ShotKitSettingsRecoveryContract.Native]::Write, 1)
    Assert-Contract ($createError -in @(80, 183)) "CREATE_NEW must reject the recreated original, got $createError."
    Assert-Contract ([IO.File]::ReadAllText($source, $utf8) -ceq $newData) 'Fresh settings were overwritten.'
    Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($held) -ceq $oldData) 'Recovery bytes changed after new settings creation.'
    $evidence.recreatedOriginalCollisionError = $recreatedError
    $evidence.createNewCollisionError = $createError

    $held.Dispose()
    $held = $null
    Assert-Contract ([IO.File]::ReadAllText($backup, $utf8) -ceq $oldData) 'Closed-handle recovery path has incorrect bytes.'
    Assert-Contract ([IO.File]::ReadAllText($collision, $utf8) -ceq $collisionData) 'Existing recovery did not survive all operations.'
    $evidence.allThreePayloadsPreserved = $true
} finally {
    if ($null -ne $held) { $held.Dispose() }
    if ($created) { [IO.Directory]::Delete($testRoot, $true) }
}

[ordered]@{
    result = 'PASS'
    scope = 'Experimental Windows filesystem contract only; no product integration or arbitrary-race proof.'
    pointerBits = [IntPtr]::Size * 8
    temporaryDirectoryRemoved = -not [IO.Directory]::Exists($testRoot)
    evidence = $evidence
} | ConvertTo-Json -Depth 4

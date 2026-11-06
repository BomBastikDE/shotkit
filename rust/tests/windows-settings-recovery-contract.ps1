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
    public sealed class RenameResult {
        public bool Success { get; set; }
        public int Error { get; set; }
    }
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
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool WriteFile(SafeFileHandle handle, byte[] bytes, uint size, out uint written, IntPtr overlapped);
        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool FlushFileBuffers(SafeFileHandle handle);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern uint GetFinalPathNameByHandleW(SafeFileHandle handle, StringBuilder path, uint size, uint flags);

        public static SafeFileHandle OpenProtected(string path) {
            // FILE_SHARE_READ only: competing writers and deleters must fail.
            SafeFileHandle h = CreateFileW(path, Read | Delete, 1, IntPtr.Zero, 3, 0x80, IntPtr.Zero);
            int error = Marshal.GetLastWin32Error();
            if (h.IsInvalid) { h.Dispose(); throw new Win32Exception(error, "Open protected settings fixture"); }
            return h;
        }
        public static SafeFileHandle CreatePreparedTemp(string path, byte[] bytes) {
            // CREATE_NEW never truncates an existing path. Retain WRITE for the
            // complete payload and flush, DELETE for handle-based rename, and
            // READ only for this probe's byte assertions. FILE_SHARE_READ denies
            // competing writers/deleters while this prepared handle remains held.
            SafeFileHandle h = CreateFileW(path, Read | Write | Delete, 1, IntPtr.Zero, 1, 0x80, IntPtr.Zero);
            int error = Marshal.GetLastWin32Error();
            if (h.IsInvalid) { h.Dispose(); throw new Win32Exception(error, "Create prepared settings temp"); }
            try {
                int offset = 0;
                while (offset < bytes.Length) {
                    byte[] remaining = new byte[bytes.Length - offset];
                    Buffer.BlockCopy(bytes, offset, remaining, 0, remaining.Length);
                    uint written;
                    if (!WriteFile(h, remaining, (uint)remaining.Length, out written, IntPtr.Zero))
                        throw new Win32Exception(Marshal.GetLastWin32Error(), "Write prepared settings temp");
                    if (written == 0 || written > remaining.Length)
                        throw new InvalidOperationException("Prepared settings write made no valid progress.");
                    offset = checked(offset + (int)written);
                }
                if (!FlushFileBuffers(h))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Flush prepared settings temp");
                return h;
            } catch { h.Dispose(); throw; }
        }
        public static int CompetingOpenError(string path, uint access, uint disposition) {
            using (SafeFileHandle h = CreateFileW(path, access, 7, IntPtr.Zero, disposition, 0x80, IntPtr.Zero)) {
                int error = Marshal.GetLastWin32Error();
                return h.IsInvalid ? error : 0;
            }
        }
        public static RenameResult RenameWithoutReplace(SafeFileHandle handle, string target) {
            byte[] name = Encoding.Unicode.GetBytes(target);
            // FILE_RENAME_INFO: BOOLEAN, pointer-aligned HANDLE, DWORD, WCHAR[].
            int rootOffset = IntPtr.Size == 8 ? 8 : 4;
            int lengthOffset = rootOffset + IntPtr.Size;
            int nameOffset = lengthOffset + 4;
            // Keep a UTF-16 NUL after the name for Win32 path conversion, but
            // exclude it from FileNameLength.
            int size = nameOffset + name.Length + 2;
            IntPtr buffer = Marshal.AllocHGlobal(size);
            try {
                Marshal.Copy(new byte[size], 0, buffer, size); // ReplaceIfExists = FALSE; RootDirectory = NULL.
                Marshal.WriteInt32(buffer, lengthOffset, name.Length);
                Marshal.Copy(name, 0, IntPtr.Add(buffer, nameOffset), name.Length);
                bool ok = SetFileInformationByHandle(handle, 3, buffer, (uint)size); // FileRenameInfo
                int error = Marshal.GetLastWin32Error();
                return new RenameResult { Success = ok, Error = ok ? 0 : error };
            } finally { Marshal.FreeHGlobal(buffer); }
        }
        public static string HeldPath(SafeFileHandle handle) {
            StringBuilder path = new StringBuilder(512);
            uint length = GetFinalPathNameByHandleW(handle, path, (uint)path.Capacity, 0);
            if (length == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
            if (length >= path.Capacity) {
                path = new StringBuilder(checked((int)length + 1));
                length = GetFinalPathNameByHandleW(handle, path, (uint)path.Capacity, 0);
                if (length == 0) throw new Win32Exception(Marshal.GetLastWin32Error());
                if (length >= path.Capacity) throw new InvalidOperationException("Held path changed during inspection.");
            }
            return path.ToString();
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

function Write-ContractDiagnostic($Result, [string] $Stage) {
    # Emit no absolute paths (including runner/owner profile directories).
    $diagnostic = [ordered]@{ stage = $Stage; nativeSuccess = $Result.Success; win32Error = $Result.Error }
    try {
        $actual = [ShotKitSettingsRecoveryContract.Native]::HeldPath($held)
        if ($actual.StartsWith('\\?\')) { $actual = $actual.Substring(4) }
        $parent = [IO.Path]::GetDirectoryName($actual)
        $diagnostic.heldTarget = if ([string]::Equals($parent, $testRoot, [StringComparison]::OrdinalIgnoreCase)) {
            [IO.Path]::GetFileName($actual)
        } else { '<outside isolated test directory>' }
    } catch { $diagnostic.heldTarget = '<unavailable>' }
    $files = @()
    try {
        if (([IO.File]::GetAttributes($testRoot) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'Unsafe diagnostic root.'
        }
        foreach ($path in [IO.Directory]::GetFileSystemEntries($testRoot)) {
            $entry = [ordered]@{ name = [IO.Path]::GetFileName($path) }
            $attributes = [IO.File]::GetAttributes($path)
            if (($attributes -band ([IO.FileAttributes]::Directory -bor [IO.FileAttributes]::ReparsePoint)) -ne 0) {
                $entry.sha256 = '<skipped directory or reparse point>'
            } else {
                $stream = $null
                $hasher = $null
                try {
                    # The protected source requests DELETE access; our read handle
                    # must share DELETE as well as READ/WRITE to inspect it safely.
                    $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read,
                        [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
                    $hasher = [Security.Cryptography.SHA256]::Create()
                    $entry.sha256 = [Convert]::ToHexString($hasher.ComputeHash($stream))
                    $entry.bytes = $stream.Length
                } catch { $entry.sha256 = '<unavailable>' }
                finally {
                    if ($null -ne $hasher) { $hasher.Dispose() }
                    if ($null -ne $stream) { $stream.Dispose() }
                }
            }
            $files += $entry
        }
    } catch { $diagnostic.filesUnavailable = $true }
    $diagnostic.files = $files
    Write-Host ('CONTRACT_DIAGNOSTIC: ' + ($diagnostic | ConvertTo-Json -Depth 4 -Compress))
}

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('shotkit-settings-contract-' + [Guid]::NewGuid().ToString('N'))
$held = $null
$prepared = $null
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

    $collisionResult = [ShotKitSettingsRecoveryContract.Native]::RenameWithoutReplace($held, $collision)
    $collisionError = $collisionResult.Error
    if ($collisionResult.Success -or $collisionError -notin @(80, 183)) {
        Write-ContractDiagnostic $collisionResult 'existing recovery collision'
    }
    Assert-Contract (-not $collisionResult.Success) 'Existing target rename unexpectedly returned native success.'
    Assert-Contract ($collisionError -in @(80, 183)) "Existing target must fail with FILE_EXISTS/ALREADY_EXISTS, got $collisionError."
    Assert-Contract ([IO.File]::Exists($source)) 'Collision removed the original path.'
    Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($held) -ceq $oldData) 'Collision changed held original bytes.'
    Assert-Contract ([IO.File]::ReadAllText($collision, $utf8) -ceq $collisionData) 'Collision overwrote existing recovery bytes.'
    $evidence.existingRecoveryCollisionError = $collisionError

    $renameResult = [ShotKitSettingsRecoveryContract.Native]::RenameWithoutReplace($held, $backup)
    $renameError = $renameResult.Error
    if (-not $renameResult.Success) { Write-ContractDiagnostic $renameResult 'new recovery rename' }
    Assert-Contract ($renameResult.Success) "Rename through the still-held handle returned native failure: Win32 $renameError."
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
    $recreatedResult = [ShotKitSettingsRecoveryContract.Native]::RenameWithoutReplace($held, $source)
    $recreatedError = $recreatedResult.Error
    if ($recreatedResult.Success -or $recreatedError -notin @(80, 183)) {
        Write-ContractDiagnostic $recreatedResult 'recreated original collision'
    }
    Assert-Contract (-not $recreatedResult.Success) 'Recreated target rename unexpectedly returned native success.'
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

    # Isolated candidate for a later normal save, separate from the existing
    # recovery assertions above. Both renames reuse the still-held handles.
    # This ordered probe does not establish crash durability or arbitrary-race safety.
    $saveData = '{"fixture":"complete-normal-save","values":[1,2,3]}'
    $saveBytes = $utf8.GetBytes($saveData)
    $saveEvidence = [ordered]@{}
    foreach ($scenario in @('success', 'foreign-create')) {
        $saveSource = Join-Path $testRoot ("settings.save-$scenario.json")
        $saveBackup = Join-Path $testRoot ("settings.save-$scenario.recovery.json")
        $saveTemp = Join-Path $testRoot ("settings.save-$scenario.tmp")
        [IO.File]::WriteAllText($saveSource, $oldData, $utf8)
        $held = [ShotKitSettingsRecoveryContract.Native]::OpenProtected($saveSource)
        $prepared = [ShotKitSettingsRecoveryContract.Native]::CreatePreparedTemp($saveTemp, $saveBytes)
        Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($prepared) -ceq $saveData) "$scenario prepared temp does not contain the complete payload."
        $preparedCreateError = [ShotKitSettingsRecoveryContract.Native]::CompetingOpenError($saveTemp, [ShotKitSettingsRecoveryContract.Native]::Write, 1)
        $preparedWriteError = [ShotKitSettingsRecoveryContract.Native]::CompetingOpenError($saveTemp, [ShotKitSettingsRecoveryContract.Native]::Write, 3)
        $preparedDeleteError = [ShotKitSettingsRecoveryContract.Native]::CompetingOpenError($saveTemp, [ShotKitSettingsRecoveryContract.Native]::Delete, 3)
        Assert-Contract ($preparedCreateError -in @(80, 183)) "$scenario prepared temp must reject CREATE_NEW, got $preparedCreateError."
        Assert-Contract ($preparedWriteError -eq 32) "$scenario prepared temp must reject competing WRITE access, got $preparedWriteError."
        Assert-Contract ($preparedDeleteError -eq 32) "$scenario prepared temp must reject competing DELETE access, got $preparedDeleteError."

        $saveBackupResult = [ShotKitSettingsRecoveryContract.Native]::RenameWithoutReplace($held, $saveBackup)
        if (-not $saveBackupResult.Success) { Write-ContractDiagnostic $saveBackupResult "$scenario save backup" }
        Assert-Contract ($saveBackupResult.Success -and $saveBackupResult.Error -eq 0) "$scenario could not preserve original through its protected handle: Win32 $($saveBackupResult.Error)."
        Assert-Contract (-not [IO.File]::Exists($saveSource)) "$scenario save backup retained the original name."
        Assert-Contract ([IO.File]::Exists($saveBackup)) "$scenario save backup path is missing."
        Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($held) -ceq $oldData) "$scenario save backup changed original bytes."

        if ($scenario -eq 'foreign-create') {
            # Deterministically interleave a foreign CREATE_NEW after backup and
            # before the prepared-handle rename. Closing it makes the collision
            # depend on NoReplace, rather than a sharing violation.
            $foreign = [IO.File]::Open($saveSource, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
            try {
                $foreignBytes = $utf8.GetBytes($newData)
                $foreign.Write($foreignBytes, 0, $foreignBytes.Length)
                $foreign.Flush($true)
            } finally { $foreign.Dispose() }
        }

        $saveResult = [ShotKitSettingsRecoveryContract.Native]::RenameWithoutReplace($prepared, $saveSource)
        if ($scenario -eq 'success') {
            if (-not $saveResult.Success) { Write-ContractDiagnostic $saveResult 'prepared save commit' }
            Assert-Contract ($saveResult.Success -and $saveResult.Error -eq 0) "Prepared save rename failed: Win32 $($saveResult.Error)."
            Assert-Contract ([IO.File]::Exists($saveSource)) 'Prepared save did not create the original name.'
            Assert-Contract (-not [IO.File]::Exists($saveTemp)) 'Prepared save retained its temporary name.'
        } else {
            if ($saveResult.Success -or $saveResult.Error -notin @(80, 183)) {
                Write-ContractDiagnostic $saveResult 'prepared save foreign collision'
            }
            Assert-Contract (-not $saveResult.Success) 'Prepared save unexpectedly replaced foreign settings.'
            Assert-Contract ($saveResult.Error -in @(80, 183)) "Prepared save must reject a foreign target with FILE_EXISTS/ALREADY_EXISTS, got $($saveResult.Error)."
            Assert-Contract ([IO.File]::ReadAllText($saveSource, $utf8) -ceq $newData) 'Prepared save changed foreign settings bytes.'
            Assert-Contract ([IO.File]::Exists($saveTemp)) 'Rejected prepared save lost its temporary name.'
        }
        Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($prepared) -ceq $saveData) "$scenario save changed the prepared payload."
        Assert-Contract ([ShotKitSettingsRecoveryContract.Native]::ReadHeld($held) -ceq $oldData) "$scenario save changed the protected original backup."
        $prepared.Dispose()
        $prepared = $null
        $held.Dispose()
        $held = $null
        Assert-Contract ([IO.File]::ReadAllText($saveBackup, $utf8) -ceq $oldData) "$scenario closed-handle save backup has incorrect bytes."
        if ($scenario -eq 'success') {
            Assert-Contract ([IO.File]::ReadAllText($saveSource, $utf8) -ceq $saveData) 'Committed normal-save bytes are incomplete or incorrect.'
        } else {
            Assert-Contract ([IO.File]::ReadAllText($saveTemp, $utf8) -ceq $saveData) 'Rejected normal-save temporary bytes were lost.'
            Assert-Contract ([IO.File]::ReadAllText($saveSource, $utf8) -ceq $newData) 'Foreign settings did not survive closing the save handles.'
        }
        $saveEvidence[$scenario] = [ordered]@{
            originalBackupPreserved = $true
            preparedPayloadPreserved = $true
            renameSuccess = $saveResult.Success
            renameError = $saveResult.Error
            createNewCollisionError = $preparedCreateError
            competingWriterError = $preparedWriteError
            competingDeleteError = $preparedDeleteError
        }
    }
    $evidence.preparedNormalSave = $saveEvidence
} finally {
    if ($null -ne $prepared) { $prepared.Dispose() }
    if ($null -ne $held) { $held.Dispose() }
    if ($created) {
        Assert-Contract (([IO.File]::GetAttributes($testRoot) -band [IO.FileAttributes]::ReparsePoint) -eq 0) 'Refusing cleanup through a reparse-point test root.'
        foreach ($path in [IO.Directory]::GetFileSystemEntries($testRoot)) {
            Assert-Contract (([IO.File]::GetAttributes($path) -band ([IO.FileAttributes]::Directory -bor [IO.FileAttributes]::ReparsePoint)) -eq 0) 'Refusing cleanup of an unexpected directory or reparse point.'
            [IO.File]::Delete($path)
        }
        [IO.Directory]::Delete($testRoot, $false)
    }
}

[ordered]@{
    result = 'PASS'
    scope = 'Experimental Windows filesystem contract only; no product integration or arbitrary-race proof.'
    pointerBits = [IntPtr]::Size * 8
    temporaryDirectoryRemoved = -not [IO.Directory]::Exists($testRoot)
    evidence = $evidence
} | ConvertTo-Json -Depth 4

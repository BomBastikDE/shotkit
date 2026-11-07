if ($PSVersionTable.PSVersion.Major -lt 7 -or $env:OS -ne 'Windows_NT') {
    throw 'This shared-log reader contract requires PowerShell 7 on Windows.'
}

. (Join-Path $PSScriptRoot 'shared-log-reader.ps1')
$root = Join-Path ([IO.Path]::GetTempPath()) ('ShotKit-Shared-Log-Contract-' + [Guid]::NewGuid().ToString('N'))
$path = Join-Path $root 'live.log'
$writer = $null
try {
    [void][IO.Directory]::CreateDirectory($root)
    [IO.File]::WriteAllText($path, "first`n", [Text.UTF8Encoding]::new($false))
    $writer = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Write,
        [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
    $writer.Seek(0, [IO.SeekOrigin]::End) | Out-Null
    $lines = @(Read-SharedLines -Path $path)
    if ($lines.Count -ne 1 -or $lines[0] -cne 'first') {
        throw 'The shared reader did not read the existing line while the append handle was open.'
    }
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes("second`n")
    $writer.Write($bytes, 0, $bytes.Length)
    $writer.Flush($true)
    $lines = @(Read-SharedLines -Path $path)
    if ($lines.Count -ne 2 -or $lines[1] -cne 'second') {
        throw 'The shared reader did not observe appended live-log content.'
    }
    $writer.Dispose()
    $writer = $null
    $locked = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $failed = $false
        try { [void](Read-SharedLines -Path $path -RetryCount 0) } catch { $failed = $true }
        if (-not $failed) { throw 'The reader silently accepted a file locked without sharing.' }
    } finally { $locked.Dispose() }
    Write-Output 'PASS: live log reads with an append handle open; an exclusive lock remains a failure.'
} finally {
    if ($null -ne $writer) { $writer.Dispose() }
    if ([IO.Directory]::Exists($root)) { [IO.Directory]::Delete($root, $true) }
}

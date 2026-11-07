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
    # Exercise the actual scroll fixture predicate without starting ShotKit or
    # touching the desktop. A one-line Outside proof must be recognized without
    # waiting for a second line: a wheel line would violate that scenario.
    $smokePath = Join-Path $PSScriptRoot 'windows-scroll-smoke.ps1'
    $tokens = $null
    $parseErrors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($smokePath, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count -ne 0) { throw "Scroll smoke script parse errors: $parseErrors" }
    $commands = @($ast.FindAll({
        param($node)
        $node -is [Management.Automation.Language.CommandAst] -and
        $node.GetCommandName() -eq 'Wait-ForCondition' -and
        $node.Extent.Text.Contains("-Description 'cursor clipping applied at the target activation barrier'")
    }, $true))
    if ($commands.Count -ne 1) { throw 'Could not uniquely locate the actual cursor clip wait predicate.' }
    $blocks = @($commands[0].CommandElements | Where-Object { $_ -is [Management.Automation.Language.ScriptBlockExpressionAst] })
    if ($blocks.Count -ne 1) { throw 'Cursor clip wait must contain exactly one condition scriptblock.' }
    $condition = $blocks[0].ScriptBlock.GetScriptBlock()
    $clipProofFile = Join-Path $root 'cursor-clip.proof'
    $active = 'active,708,111,709,512,708,511'
    foreach ($payload in @($active, ($active + "`n"), ($active + "`r`n"), ($active + "`nwheel,708,111,709,512,708,311`n"))) {
        [IO.File]::WriteAllText($clipProofFile, $payload, [Text.UTF8Encoding]::new($false))
        $observed = & $condition
        if ($observed -cne $active) {
            throw "Actual cursor clip predicate missed its complete first record: '$observed'."
        }
    }
    [IO.File]::WriteAllText($clipProofFile, 'active,708,111,709', [Text.UTF8Encoding]::new($false))
    if ($null -ne (& $condition)) { throw 'Incomplete cursor clip evidence was accepted.' }
    [IO.File]::WriteAllText($clipProofFile, "error,synthetic setup failure`n", [Text.UTF8Encoding]::new($false))
    $setupFailed = $false
    try { & $condition | Out-Null } catch { $setupFailed = $_.Exception.Message -like '*synthetic setup failure*' }
    if (-not $setupFailed) { throw 'Cursor clip setup errors must fail immediately.' }
    Write-Output 'PASS: live log reads with an append handle open; exclusive locks fail; actual cursor clip predicate accepts one complete record without a wheel line and rejects malformed/error evidence.'
} finally {
    if ($null -ne $writer) { $writer.Dispose() }
    if ([IO.Directory]::Exists($root)) { [IO.Directory]::Delete($root, $true) }
}

function Read-SharedLines {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $Path,
        [ValidateRange(0, 20)][int] $RetryCount = 10,
        [ValidateRange(0, 1000)][int] $RetryDelayMilliseconds = 50
    )

    for ($attempt = 0; $attempt -le $RetryCount; $attempt++) {
        $stream = $null
        $reader = $null
        try {
            if (-not [IO.File]::Exists($Path)) { return @() }
            $share = [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete
            $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
            $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8, $true)
            $lines = [Collections.Generic.List[string]]::new()
            while ($null -ne ($line = $reader.ReadLine())) { $lines.Add($line) }
            return $lines.ToArray()
        } catch {
            $cause = $_.Exception
            while ($null -ne $cause -and $cause -isnot [IO.IOException]) { $cause = $cause.InnerException }
            if ($null -eq $cause) { throw }
            $win32Error = $cause.HResult -band 0xFFFF
            if ($win32Error -notin @(32, 33) -or $attempt -ge $RetryCount) { throw }
            if ($RetryDelayMilliseconds -gt 0) { Start-Sleep -Milliseconds $RetryDelayMilliseconds }
        } finally {
            if ($null -ne $reader) { $reader.Dispose() }
            elseif ($null -ne $stream) { $stream.Dispose() }
        }
    }
}

function Read-SharedText {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string] $Path,
        [ValidateRange(0, 20)][int] $RetryCount = 10,
        [ValidateRange(0, 1000)][int] $RetryDelayMilliseconds = 50
    )
    $lines = @(Read-SharedLines -Path $Path -RetryCount $RetryCount -RetryDelayMilliseconds $RetryDelayMilliseconds)
    return ($lines -join [Environment]::NewLine)
}

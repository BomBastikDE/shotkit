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

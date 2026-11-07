# Dot-sourced by GUI smokes whose settings or file arguments suppress onboarding.
# The callback must enumerate only windows belonging to the test-owned process.
function Assert-NoStartupDialogs {
    param(
        [Parameter(Mandatory=$true)][Diagnostics.Process] $OwnedProcess,
        [Parameter(Mandatory=$true)][scriptblock] $GetWindows,
        [int] $TimeoutSeconds = 30
    )
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $readyAt = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $OwnedProcess.Refresh()
        if ($OwnedProcess.HasExited) {
            throw "STARTUP_PROCESS_EXITED: owned ShotKit process exited with code $($OwnedProcess.ExitCode)."
        }
        $windows = @(& $GetWindows)
        foreach ($window in $windows) {
            # The older multi-editor native helper calls this field Class.
            $classProperty = $window.PSObject.Properties['ClassName']
            if ($null -eq $classProperty) { $classProperty = $window.PSObject.Properties['Class'] }
            if ($null -eq $classProperty) { throw 'STARTUP_WINDOW_CLASS_MISSING: cannot verify owned windows.' }
            if ($window.Visible -and $classProperty.Value -eq '#32770') {
                throw "UNEXPECTED_STARTUP_DIALOG: title='$($window.Title)' PID=$($OwnedProcess.Id); settings/file startup must not show a dialog."
            }
            if ($classProperty.Value -in @('ShotKitRustPrototype', 'ShotKitRustEditor') -and $null -eq $readyAt) {
                $readyAt = [DateTime]::UtcNow
            }
        }
        # Watch beyond native window creation so a delayed modal cannot pass.
        if ($null -ne $readyAt -and ([DateTime]::UtcNow - $readyAt).TotalMilliseconds -ge 1000) { return }
        Start-Sleep -Milliseconds 50
    }
    throw 'STARTUP_WINDOW_TIMEOUT: owned ShotKit host/editor window was not created.'
}

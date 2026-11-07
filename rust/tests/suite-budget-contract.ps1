# Shared scheduler only: no desktop access, ShotKit discovery, or CI opt-out.
# The production caller owns the fixed CI budget; fixtures can use shorter budgets.
# A tree kill requires the direct child to still exist; descendants surviving
# an already exited parent cannot be identified reliably by this scheduler.
if (-not ('ShotKitSuiteBudget.Runner' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading.Tasks;
namespace ShotKitSuiteBudget {
    public sealed class Result {
        public string Output, Error, Status;
        public int ExitCode;
    }
    public static class Runner {
        private static async Task Pump(StreamReader reader, StringBuilder text) {
            char[] buffer = new char[1024];
            int count;
            while ((count = await reader.ReadAsync(buffer, 0, buffer.Length).ConfigureAwait(false)) > 0) {
                lock (text) { text.Append(buffer, 0, count); }
            }
        }
        public static Result Run(ProcessStartInfo start, Stopwatch suite, double budgetSeconds, int timeoutSeconds) {
            if (budgetSeconds > 0 && suite.Elapsed.TotalSeconds >= budgetSeconds)
                return new Result { Status = "SUITE_NOT_STARTED", Output = "", Error = "", ExitCode = -1 };
            using (var process = new Process { StartInfo = start }) {
                var output = new StringBuilder();
                var error = new StringBuilder();
                if (!process.Start()) throw new Exception("Could not start the test process.");
                var local = Stopwatch.StartNew();
                Task stdout = Pump(process.StandardOutput, output);
                Task stderr = Pump(process.StandardError, error);
                string status = "EXIT";
                try {
                    while (true) {
                        // Check the monotonic suite deadline even if the child just exited.
                        if (budgetSeconds > 0 && suite.Elapsed.TotalSeconds >= budgetSeconds) {
                            status = "SUITE_BUDGET_EXHAUSTED"; break;
                        }
                        if (process.HasExited) break;
                        if (local.Elapsed.TotalSeconds >= timeoutSeconds) { status = "TIMEOUT"; break; }
                        double remainingMs = (timeoutSeconds - local.Elapsed.TotalSeconds) * 1000;
                        if (budgetSeconds > 0) remainingMs = Math.Min(remainingMs, (budgetSeconds - suite.Elapsed.TotalSeconds) * 1000);
                        process.WaitForExit((int)Math.Max(1, Math.Min(100, remainingMs)));
                    }
                } finally {
                    try {
                        if (!process.HasExited) {
                            // Only this test's process tree; never a machine-wide process-name kill.
                            process.Kill(true);
                            if (!process.WaitForExit(5000)) throw new Exception("process tree did not stop within 5 seconds");
                        }
                    } catch (Exception ex) {
                        if (!process.HasExited) {
                            lock (error) { error.Append("\nPROCESS_CLEANUP_FAILED: " + ex.Message); }
                            if (status == "EXIT") status = "PROCESS_CLEANUP_FAILED";
                        }
                    }
                }
                // A child that inherits a pipe must never make output draining unbounded.
                // Pump stores characters incrementally, including a final unterminated line.
                bool drained = false;
                try { drained = Task.WaitAll(new Task[] { stdout, stderr }, 1000); }
                catch (AggregateException ex) {
                    lock (error) { error.Append("\nOUTPUT_READ_FAILED: " + ex.GetBaseException().Message); }
                    if (status == "EXIT") status = "OUTPUT_READ_FAILED";
                }
                if (budgetSeconds > 0 && suite.Elapsed.TotalSeconds >= budgetSeconds) status = "SUITE_BUDGET_EXHAUSTED";
                string outText, errText;
                lock (output) { outText = output.ToString(); }
                lock (error) { errText = error.ToString(); }
                if (!drained) errText += "\nOUTPUT_DRAIN_INCOMPLETE: an inherited output pipe remained open.";
                return new Result { Output = outText, Error = errText, Status = (!drained && status == "EXIT") ? "OUTPUT_DRAIN_INCOMPLETE" : status, ExitCode = process.HasExited ? process.ExitCode : -1 };
            }
        }
    }
}
'@
}

function Invoke-ShotKitSmokeSuite {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][object[]] $Cases,
        [Parameter(Mandatory)][string] $ScriptRoot,
        [Parameter(Mandatory)][string] $Pwsh,
        [string[]] $CommonArguments = @(),
        [Parameter(Mandatory)][Diagnostics.Stopwatch] $Clock,
        [ValidateRange(0, 86400)][double] $BudgetSeconds = 0,
        [switch] $AllowToolbarFailure,
        [switch] $IsCi,
        [switch] $LocalInteractive
    )
    $failures = [Collections.Generic.List[object]]::new()
    $exhausted = $false
    foreach ($case in $Cases) {
        if ($BudgetSeconds -gt 0 -and $Clock.Elapsed.TotalSeconds -ge $BudgetSeconds) { $exhausted = $true }
        if ($exhausted) {
            $message = 'NOT RUN: SUITE_BUDGET_EXHAUSTED; suite deadline reached before this scenario started.'
            $failures.Add([pscustomobject]@{ Name = $case.Name; Message = $message; NonBlocking = $false })
            Write-Host ("FAILURE: {0}: {1}" -f $case.Name, $message)
            continue
        }
        Write-Host ("--- {0} ---" -f $case.Name)
        try {
            $path = Join-Path $ScriptRoot $case.Script
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Test script missing: $($case.Script)" }
            $start = [Diagnostics.ProcessStartInfo]::new()
            $start.FileName = $Pwsh
            $start.UseShellExecute = $false
            $start.RedirectStandardOutput = $true
            $start.RedirectStandardError = $true
            $start.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
            $start.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
            foreach ($arg in @('-NoProfile', '-NonInteractive', '-File', $path) + $CommonArguments + $case.Extra) {
                [void] $start.ArgumentList.Add([string] $arg)
            }
            if ($LocalInteractive) { $start.Environment['SHOTKIT_LOCAL_SMOKE_TEST'] = '1' }
            $result = [ShotKitSuiteBudget.Runner]::Run($start, $Clock, $BudgetSeconds, $case.TimeoutSeconds)
            if ($result.Output) { Write-Host ($result.Output.TrimEnd()) }
            if ($result.Error) { Write-Host ($result.Error.TrimEnd()) }
            if ($result.Status -eq 'SUITE_NOT_STARTED') {
                $exhausted = $true
                throw 'NOT RUN: SUITE_BUDGET_EXHAUSTED; suite deadline reached before this scenario started.'
            }
            if ($result.Status -eq 'SUITE_BUDGET_EXHAUSTED') {
                $exhausted = $true
                throw 'SUITE_BUDGET_EXHAUSTED: overall suite deadline reached; running test process tree was stopped.'
            }
            if ($result.Status -eq 'TIMEOUT') { throw "TIMEOUT: test exceeded $($case.TimeoutSeconds) seconds; its process tree was stopped." }
            if ($result.Status -ne 'EXIT') { throw $result.Status }
            if ($result.ExitCode -ne 0) { throw "Test process exited with code $($result.ExitCode)." }
            Write-Host ("PASS: {0}" -f $case.Name)
        }
        catch {
            if ($BudgetSeconds -gt 0 -and $Clock.Elapsed.TotalSeconds -ge $BudgetSeconds) { $exhausted = $true }
            $message = $_.Exception.Message
            if ($exhausted -and $message -notlike '*SUITE_BUDGET_EXHAUSTED*') { $message = 'SUITE_BUDGET_EXHAUSTED: ' + $message }
            $optional = [bool]($case.Toolbar -and $AllowToolbarFailure -and -not $exhausted)
            $failures.Add([pscustomobject]@{ Name = $case.Name; Message = $message; NonBlocking = $optional })
            $kind = if ($optional) { 'NON-BLOCKING FAILURE' } else { 'FAILURE' }
            Write-Host ("{0}: {1}: {2}" -f $kind, $case.Name, $message)
            if ($optional -and $IsCi) { Write-Host ("::warning::Non-blocking toolbar smoke failed: {0}" -f $message) }
        }
    }
    # Include the last scenario/output/summary boundary: exhaustion is always blocking.
    if ($BudgetSeconds -gt 0 -and $Clock.Elapsed.TotalSeconds -ge $BudgetSeconds) {
        $exhausted = $true
        if (-not @($failures | Where-Object { $_.Message -like '*SUITE_BUDGET_EXHAUSTED*' }).Count) {
            $failures.Add([pscustomobject]@{ Name = 'Suite deadline'; Message = 'SUITE_BUDGET_EXHAUSTED: overall suite deadline reached.'; NonBlocking = $false })
        }
    }
    Write-Host ("Smoke test summary: {0} scenarios, {1} failures." -f $Cases.Count, $failures.Count)
    foreach ($failure in $failures) {
        Write-Host (" - {0} (non-blocking={1}): {2}" -f $failure.Name, $failure.NonBlocking, $failure.Message)
    }
    $blocking = @($failures | Where-Object { -not $_.NonBlocking })
    if ($blocking.Count -gt 0) { throw ("{0} blocking smoke test(s) failed." -f $blocking.Count) }
}
